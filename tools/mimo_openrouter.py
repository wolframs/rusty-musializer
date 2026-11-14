#!/usr/bin/env python3
"""Create cached MiMo-V2.5 semantic scores through OpenRouter.

The helper is opt-in and external to the renderer. It reads the API key only
from the process environment. ``--dry-run`` never opens a network connection
and emits a safely redacted request description.
"""

from __future__ import annotations

import argparse
import base64
import email.utils
import hashlib
import json
import math
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable

from analysis_io import (
    ADAPTER_VERSION,
    AnalysisValidationError,
    audio_hash,
    atomic_write_json,
    canonical_sha256,
    clamp,
    duration,
    number,
    read_json,
    string_list,
    text,
)


OPENROUTER_URL = "https://openrouter.ai/api/v1/chat/completions"
MODEL = "xiaomi/mimo-v2.5"
# v2 (2026-11-14): the numeric scales are defined in the prompt and the
# schema, and an off-scale number fails the run instead of being clamped. A v1
# answer was produced without those definitions, so it is a different request
# (`prompt_sha256`, `model_output_schema_sha256`) and is regenerated, not
# reused; the document it produced is still a readable semantic-score/v1.
PROMPT_VERSION = "musializer-semantic-score/v2"
SCORE_SCHEMA_VERSION = "musializer.semantic-score/v1"
CACHE_SCHEMA_VERSION = "musializer.analysis-cache/v1"
# Retried, because OpenRouter says the request was not processed: 429 is a
# rate limit applied before routing. A 502, 503, 504 or 408 comes after the
# request was sent, possibly after the model ran, and may have been billed: it
# is a failed run the person decides about, never a silent second call (H4c;
# the confirmation they accepted budgets one call).
TRANSIENT_STATUSES = frozenset((429,))
MAY_BE_BILLED_STATUSES = frozenset((408, 500, 502, 503, 504))
DEFAULT_TIMEOUT = 600.0
DEFAULT_MAX_ATTEMPTS = 3
COVERAGE_TOLERANCE_SECONDS = 0.001
FAILED_RUN_SCHEMA_VERSION = "musializer.mimo-failed-run/v1"

# The completion cap, reasoning included (OpenRouter's `max_tokens`; MiMo bills
# its reasoning as completion tokens and counts it against this limit). Without
# it the provider's own ceiling, 65 536, applied: in the 2026-11-14 benchmark
# one call emitted that many tokens of whitespace (0.131 USD) and another ran
# to 43 578. The largest legitimate answer used 10 942, on a 100-second excerpt;
# this lane sends whole tracks, so the cap leaves half again as much room and
# still cuts both runaways. docs/MIMO_BENCHMARK_RESULTS.md, "Cost".
MAX_COMPLETION_TOKENS = 16_000
# The cost estimate, built from completion tokens rather than audio length: a
# whole-track MiMo call is nearly all reasoning output (600-9 200 reasoning
# tokens per benchmark call, billed at the completion rate), while audio is
# 7.4 tokens per second at the prompt rate. Every number below is pinned
# against its Rust copy in `crates/musializer-core/src/assist/cost.rs` by
# tests/test_cross_language_pins.py; change both or neither.
COST_ESTIMATE_VERSION = "musializer.mimo-cost-estimate/v1"
# USD per million tokens as OpenRouter listed them on 2026-11-14 (Venice, the
# only provider for this model that day). No separate audio price is listed;
# audio bills as prompt tokens. A model not in this table has no estimate.
PRICES_USD_PER_MILLION: dict[str, tuple[float, float]] = {
    "xiaomi/mimo-v2.5": (0.40, 2.00),
}
PRICES_LISTED_ON = "2026-11-14"
# Calibrated from the benchmark's recorded `usage.prompt_tokens`.
AUDIO_TOKENS_PER_SECOND = 7.4
# This request's text as prompt tokens: the 2026-11-14 capped call reported
# 928 prompt tokens for 100 s, 740 of them audio at the rate above.
PROMPT_TEXT_TOKENS = 200
# The span of whole-excerpt answers seen so far: 2 044 and 2 407 on this
# lane's capped request (two calls, 2026-11-14), 5 110 on the benchmark's
# shape of it, 10 942 the largest legitimate one. The ceiling is the cap.
TYPICAL_COMPLETION_TOKENS = (1_500, 11_000)
# Bytes of a failed answer's text kept in its receipt: enough to see what kind
# of failure it was (whitespace, prose, half a JSON object), never 65 k tokens.
FAILED_CONTENT_KEEP = 4096

Transport = Callable[..., Any]


def estimate_cost(model: str, audio_duration: float) -> dict[str, Any]:
    """What one call should cost, before it is sent: a range and a ceiling.

    The range is the measured span of answer lengths; the ceiling is what a
    call that runs to the completion cap costs, which is the most it can.
    Prices are a dated table, not a live quote; an unlisted model gets token
    counts and no dollar figures rather than a guess.
    """
    seconds = duration(audio_duration)
    input_tokens = math.ceil(seconds * AUDIO_TOKENS_PER_SECOND) + PROMPT_TEXT_TOKENS
    low_tokens, high_tokens = TYPICAL_COMPLETION_TOKENS
    estimate: dict[str, Any] = {
        "version": COST_ESTIMATE_VERSION,
        "model": model,
        "audio_seconds": seconds,
        "input_tokens": input_tokens,
        "completion_tokens_typical": [low_tokens, high_tokens],
        "completion_tokens_max": MAX_COMPLETION_TOKENS,
        "prices_usd_per_million": None,
        "prices_listed_on": None,
        "usd_typical": None,
        "usd_max": None,
    }
    prices = PRICES_USD_PER_MILLION.get(model)
    if prices is None:
        return estimate
    prompt_price, completion_price = prices

    def usd(completion_tokens: int) -> float:
        return round((input_tokens * prompt_price
                      + completion_tokens * completion_price) / 1_000_000.0, 6)

    estimate.update({
        "prices_usd_per_million": {"prompt": prompt_price, "completion": completion_price},
        "prices_listed_on": PRICES_LISTED_ON,
        "usd_typical": [usd(low_tokens), usd(high_tokens)],
        "usd_max": usd(MAX_COMPLETION_TOKENS),
    })
    return estimate


class FailedRun(RuntimeError):
    """A billed answer that is not a result: cut off, empty, or off-schema.

    Its message is the one sentence the user reads (behind the helper's
    ``External analysis failed:`` prefix, clipped by the application at 160
    bytes, so the reason comes first and the cost second). A failed run is
    never written to the cache and never staged; its receipt sits beside the
    cache path instead.
    """


def _snapshot_audio(audio_path: Path) -> tuple[bytes, str]:
    """Read one immutable snapshot used for both submission and identity."""
    audio_bytes = audio_path.read_bytes()
    return audio_bytes, hashlib.sha256(audio_bytes).hexdigest()


# Each numeric field's range and meaning, stated once and used twice: in the
# prompt and as the schema's `description`. The benchmark's own schema gave
# `energy` no range, and three chunks of one excerpt came back 78, 8 and 5.
NUMERIC_SCALES: dict[str, tuple[float, float, str]] = {
    "energy": (0.0, 1.0,
               "how much force and activity the music has: 0 is silence or "
               "stillness, 0.5 a steady moderate drive, 1 full intensity "
               "(loud, dense, driving)"),
    "tension": (0.0, 1.0,
                "unresolved pull: 0 is settled and at rest, 0.5 some "
                "expectation, 1 maximum suspense, dissonance or build-up"),
    "valence": (-1.0, 1.0,
                "emotional colour: -1 is dark, sad or menacing, 0 neutral or "
                "mixed, 1 bright, happy or triumphant"),
    "confidence": (0.0, 1.0,
                   "how sure you are of this description: 0 a guess, 1 certain"),
}


def _scale_text(name: str) -> str:
    low, high, meaning = NUMERIC_SCALES[name]
    return f"a number from {low:g} to {high:g}; {meaning}"


SYSTEM_PROMPT = """You are a music-to-visual creative interpreter. Describe how the
provided music feels, not what can be measured with certainty. Do not claim
authoritative lyrics, beat times, or signal facts. Return only JSON matching
the supplied SemanticScore schema. Cover the full requested duration with
non-overlapping interpretive segments; start_seconds and end_seconds count
seconds from the start of this audio, from 0 to the audio duration.
Every number must lie inside its range. Never use percentages, 0-10 or 0-100
scales:
""" + "\n".join(f"- {name}: {_scale_text(name)}." for name in NUMERIC_SCALES)


def _creative_schema(include_timing: bool) -> dict[str, Any]:
    properties: dict[str, Any] = {
        "summary": {"type": "string"},
        "moods": {"type": "array", "items": {"type": "string"}},
        "energy": {"type": "number", "minimum": 0, "maximum": 1,
                          "description": _scale_text("energy")},
        "tension": {"type": "number", "minimum": 0, "maximum": 1,
                           "description": _scale_text("tension")},
        "valence": {"type": "number", "minimum": -1, "maximum": 1,
                           "description": _scale_text("valence")},
        "motion": {"type": "array", "items": {"type": "string"}},
        "textures": {"type": "array", "items": {"type": "string"}},
        "imagery": {"type": "array", "items": {"type": "string"}},
        "palette": {"type": "array", "items": {"type": "string"}},
        "scene_cues": {"type": "array", "items": {"type": "string"}},
        "confidence": {"type": "number", "minimum": 0, "maximum": 1,
                              "description": _scale_text("confidence")},
    }
    required = list(properties)
    if include_timing:
        properties = {
            "start_seconds": {"type": "number", "minimum": 0,
                              "description": "seconds from the start of this audio"},
            "end_seconds": {"type": "number", "minimum": 0,
                            "description": "seconds from the start of this audio, at most its duration"},
            **properties,
        }
        required = list(properties)
    return {
        "type": "object",
        "additionalProperties": False,
        "required": required,
        "properties": properties,
    }


MODEL_OUTPUT_SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["global", "segments"],
    "properties": {
        "global": _creative_schema(False),
        "segments": {
            "type": "array",
            "minItems": 1,
            "description": "A contiguous, ordered, non-overlapping partition from 0 through the full audio duration.",
            "items": _creative_schema(True),
        },
    },
}


# Float noise at a bound (1.0000000002) is the bound; anything further out is
# an answer on another scale.
SCALE_TOLERANCE = 1e-6


def _on_scale(raw: Any, name: str, field: str) -> float:
    """The value, when it lies on its stated scale; a schema failure otherwise.

    Not a clamp: `energy: 78` clamped to 1.0 would be drawn as the loudest
    passage of the song, and an answer on a 0-100 scale has its other numbers
    on that scale too. So the run fails, naming the field, rather than one
    field being dropped (docs/DIVERGENCES.md, "MiMo numeric scales").
    """
    low, high, _meaning = NUMERIC_SCALES[name]
    value = number(raw, f"{field}.{name}")
    if value < low - SCALE_TOLERANCE or value > high + SCALE_TOLERANCE:
        raise AnalysisValidationError(
            f"{field}.{name} is {value:g}, outside {low:g} to {high:g}")
    return max(low, min(high, value))


def _creative(raw: Any, field: str) -> dict[str, Any]:
    if not isinstance(raw, dict):
        raise AnalysisValidationError(f"{field} must be an object")
    required = (
        "summary", "moods", "energy", "tension", "valence", "motion",
        "textures", "imagery", "palette", "scene_cues", "confidence",
    )
    missing = [name for name in required if name not in raw]
    if missing:
        raise AnalysisValidationError(f"{field} missing fields: {', '.join(missing)}")
    return {
        "summary": text(raw["summary"], f"{field}.summary"),
        "moods": string_list(raw["moods"], f"{field}.moods"),
        "energy": _on_scale(raw["energy"], "energy", field),
        "tension": _on_scale(raw["tension"], "tension", field),
        "valence": _on_scale(raw["valence"], "valence", field),
        "motion": string_list(raw["motion"], f"{field}.motion"),
        "textures": string_list(raw["textures"], f"{field}.textures"),
        "imagery": string_list(raw["imagery"], f"{field}.imagery"),
        "palette": string_list(raw["palette"], f"{field}.palette"),
        "scene_cues": string_list(raw["scene_cues"], f"{field}.scene_cues"),
        "confidence": _on_scale(raw["confidence"], "confidence", field),
    }


def normalize_semantic_score(
    raw_score: Any,
    *,
    audio_sha256: str,
    audio_duration: float,
    request_settings: dict[str, Any],
    response_metadata: dict[str, Any] | None = None,
    requested_model: str = MODEL,
    cost_estimate: dict[str, Any] | None = None,
) -> dict[str, Any]:
    audio_duration = duration(audio_duration)
    audio_sha256 = audio_hash(audio_sha256)
    if not isinstance(raw_score, dict):
        raise AnalysisValidationError("model output must be a JSON object")
    if "global" not in raw_score or not isinstance(raw_score.get("segments"), list):
        raise AnalysisValidationError("model output requires global and segments")
    if not raw_score["segments"]:
        raise AnalysisValidationError("segments must cover the full audio duration")

    normalized_segments: list[dict[str, Any]] = []
    previous_end = 0.0
    for index, raw_segment in enumerate(raw_score["segments"]):
        field = f"segments[{index}]"
        if not isinstance(raw_segment, dict):
            raise AnalysisValidationError(f"{field} must be an object")
        start = clamp(raw_segment.get("start_seconds"), 0, audio_duration, f"{field}.start_seconds")
        end = clamp(raw_segment.get("end_seconds"), 0, audio_duration, f"{field}.end_seconds")
        if index == 0:
            if abs(start) > COVERAGE_TOLERANCE_SECONDS:
                raise AnalysisValidationError("segments leave a gap before the first segment")
            start = 0.0
        else:
            boundary_error = start - previous_end
            if boundary_error < -COVERAGE_TOLERANCE_SECONDS:
                raise AnalysisValidationError(f"{field} overlaps the preceding segment")
            if boundary_error > COVERAGE_TOLERANCE_SECONDS:
                raise AnalysisValidationError(f"{field} leaves a gap after the preceding segment")
            start = previous_end
        if end <= start:
            raise AnalysisValidationError(f"{field} has an empty or reversed interval")
        normalized_segments.append({
            "start_seconds": start,
            "end_seconds": end,
            **_creative(raw_segment, field),
        })
        previous_end = end

    if abs(previous_end - audio_duration) > COVERAGE_TOLERANCE_SECONDS:
        raise AnalysisValidationError("segments leave a gap after the final segment")
    normalized_segments[-1]["end_seconds"] = audio_duration

    metadata = response_metadata or {}
    provenance = {
        "adapter": "tools/mimo_openrouter.py",
        "adapter_version": ADAPTER_VERSION,
        "source_kind": "mimo_openrouter",
        "audio_sha256": audio_sha256,
        "schema_version": SCORE_SCHEMA_VERSION,
        # §6: `model_id` is OBSERVED, not inferred. OpenRouter reports the
        # model it actually served, which can differ from the requested slug
        # when a variant is routed; the request is only the fallback for a
        # response that reported nothing.
        "model": str(metadata.get("model") or requested_model),
        "requested_model": requested_model,
        "provider": metadata.get("provider"),
        "prompt_version": PROMPT_VERSION,
        "request_settings": request_settings,
        "generation": {
            **{key: metadata[key]
               for key in ("id", "created", "usage")
               if key in metadata},
            # What was expected before sending, beside what the provider
            # reported after: the two numbers a user compares.
            **({"estimate": cost_estimate,
                "reported_cost_usd": reported_cost_usd(metadata)}
               if cost_estimate is not None else {}),
        },
    }
    return {
        "schema_version": SCORE_SCHEMA_VERSION,
        "lane": "semantic_interpretation",
        "audio": {"sha256": audio_sha256, "duration_seconds": audio_duration},
        "provenance": provenance,
        "global": _creative(raw_score["global"], "global"),
        "segments": normalized_segments,
    }


def request_settings(
    *,
    model: str = MODEL,
    audio_duration: float,
    audio_format: str,
    provider_order: list[str] | None = None,
    provider_only: list[str] | None = None,
    provider_ignore: list[str] | None = None,
    allow_fallbacks: bool = True,
    zero_data_retention: bool = False,
    max_price: dict[str, float] | None = None,
) -> dict[str, Any]:
    """The identity of one request, as both the cache key and the provenance.

    Every field a user can change is in here, which is what makes tranche P4's
    rule work: an artifact produced under a different model or a different
    provider policy has a different identity and is regenerated rather than
    reused (docs/ASSIST_PROVIDER_CONTRACTS.md §5 rule 7). The three price
    bounds and the two provider allow/deny lists were previously not part of it
    at all, so changing them silently reused the old answer.
    """
    return {
        "model": model,
        "prompt_version": PROMPT_VERSION,
        "prompt_sha256": canonical_sha256(SYSTEM_PROMPT),
        "response_schema_version": SCORE_SCHEMA_VERSION,
        "model_output_schema_sha256": canonical_sha256(MODEL_OUTPUT_SCHEMA),
        "audio_duration_seconds": duration(audio_duration),
        "audio_format": audio_format,
        "allow_fallbacks": allow_fallbacks,
        "zero_data_retention": zero_data_retention,
        "provider_order": provider_order or [],
        "provider_only": provider_only or [],
        "provider_ignore": provider_ignore or [],
        "max_price": dict(sorted((max_price or {}).items())),
        # Part of the identity: an answer produced under another cap (or none)
        # is not this request's answer (§5 rule 7).
        "max_tokens": MAX_COMPLETION_TOKENS,
    }


def build_request(
    audio_bytes: bytes,
    *,
    audio_format: str,
    audio_duration: float,
    model: str = MODEL,
    provider_order: list[str] | None = None,
    provider_only: list[str] | None = None,
    provider_ignore: list[str] | None = None,
    allow_fallbacks: bool = True,
    zero_data_retention: bool = False,
    max_price: dict[str, float] | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    measured_duration = duration(audio_duration)
    settings = request_settings(
        model=model,
        audio_duration=measured_duration,
        audio_format=audio_format,
        provider_order=provider_order,
        provider_only=provider_only,
        provider_ignore=provider_ignore,
        allow_fallbacks=allow_fallbacks,
        zero_data_retention=zero_data_retention,
        max_price=max_price,
    )
    provider: dict[str, Any] = {
        "allow_fallbacks": allow_fallbacks,
        "require_parameters": True,
    }
    if provider_order:
        provider["order"] = provider_order
    if provider_only:
        provider["only"] = provider_only
    if provider_ignore:
        provider["ignore"] = provider_ignore
    # Sent as OpenRouter states it, and never weakened here: if the constraint
    # leaves no endpoint the API refuses and that refusal is reported verbatim
    # (§5 invariant 4). The application already refuses, before spawning, every
    # constraint that is unsatisfiable without asking a provider.
    if zero_data_retention:
        provider["zdr"] = True
    if max_price:
        provider["max_price"] = dict(sorted(max_price.items()))
    payload = {
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": f"{SYSTEM_PROMPT}\nAudio duration: {measured_duration:.6f} seconds."},
                {"type": "input_audio", "input_audio": {
                    "data": base64.b64encode(audio_bytes).decode("ascii"),
                    "format": audio_format,
                }},
            ],
        }],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "musializer_semantic_score_v2",
                "strict": True,
                "schema": MODEL_OUTPUT_SCHEMA,
            },
        },
        "provider": provider,
        "temperature": 0.2,
        "max_tokens": MAX_COMPLETION_TOKENS,
    }
    return payload, settings


def redacted_request_dump(payload: dict[str, Any], settings: dict[str, Any], audio_sha256: str) -> dict[str, Any]:
    # Round-trip copy prevents accidental mutation of the real request.
    redacted = json.loads(json.dumps(payload))
    for message in redacted.get("messages", []):
        for part in message.get("content", []):
            if part.get("type") == "input_audio":
                part["input_audio"]["data"] = f"<base64 omitted; sha256={audio_sha256}>"
    return {
        "dry_run": True,
        "url": OPENROUTER_URL,
        "timeout_seconds": DEFAULT_TIMEOUT,
        "headers": {"Authorization": "Bearer <redacted>", "Content-Type": "application/json"},
        "settings": settings,
        "cost_estimate": estimate_cost(
            str(settings.get("model") or MODEL), settings["audio_duration_seconds"]),
        "payload": redacted,
    }


def _retry_delay(headers: Any, attempt: int, now: Callable[[], float]) -> float:
    value = headers.get("Retry-After") if headers is not None else None
    if value:
        try:
            return max(0.0, float(value))
        except ValueError:
            parsed = email.utils.parsedate_to_datetime(value)
            return max(0.0, parsed.timestamp() - now())
    return min(2 ** (attempt - 1), 8)


def _content(response: dict[str, Any]) -> str:
    try:
        content = response["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as error:
        raise AnalysisValidationError("OpenRouter response has no completion") from error
    if isinstance(content, str):
        return content.strip()
    if isinstance(content, list):
        return "".join(
            item.get("text", "") for item in content if isinstance(item, dict)
        ).strip()
    raise AnalysisValidationError("OpenRouter completion content is not text")


class SentUnanswered(RuntimeError):
    """The request left and no usable answer came back: it may have been billed.

    A 5xx or 408 from the gateway, a timeout or a dropped connection while
    waiting for the answer, an answer that does not read. Nothing tells this
    helper whether the model ran, so the run fails, says it may have been
    billed up to the cap, and is not sent again by itself (H4c).
    """


def submit_request(
    payload: dict[str, Any],
    *,
    transport: Transport = urllib.request.urlopen,
    sleeper: Callable[[float], None] = time.sleep,
    now: Callable[[], float] = time.time,
    timeout: float = DEFAULT_TIMEOUT,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
) -> dict[str, Any]:
    """Sends the request once, and again only where it was not processed.

    Retried: a 429 (OpenRouter's rate limit, before routing) and a failure
    before the request was sent whole (``urllib`` raises ``URLError`` only
    from connecting and sending; anything after that comes as the bare
    error). Everything else that follows the send is ``SentUnanswered``.
    """
    api_key = os.environ.get("OPENROUTER_API_KEY")
    if not api_key:
        raise RuntimeError("OPENROUTER_API_KEY is not set in the process environment")
    encoded = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    last_error: BaseException | None = None
    for attempt in range(1, max_attempts + 1):
        request = urllib.request.Request(
            OPENROUTER_URL,
            data=encoded,
            headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
            method="POST",
        )
        try:
            with transport(request, timeout=timeout) as response:
                # An empty or cut-off answer is returned, not retried: it was
                # billed, and a silent retry triples a failure's cost before
                # anyone sees it. `completion_failure` names it instead.
                body = response.read()
        except urllib.error.HTTPError as error:
            last_error = error
            code = error.code
            headers = error.headers
            error.close()
            if code in MAY_BE_BILLED_STATUSES:
                raise SentUnanswered(
                    f"OpenRouter answered HTTP {code} after the request was sent") from error
            if code not in TRANSIENT_STATUSES or attempt == max_attempts:
                raise RuntimeError(f"OpenRouter request failed with HTTP {code}") from error
            sleeper(_retry_delay(headers, attempt, now))
            continue
        except urllib.error.URLError as error:
            # Connecting or sending failed: the request never reached
            # OpenRouter whole, so nothing can have been processed.
            last_error = error
            if attempt == max_attempts:
                raise RuntimeError(
                    f"could not reach OpenRouter ({error.reason}); nothing was sent") from error
            sleeper(_retry_delay(None, attempt, now))
            continue
        except TimeoutError as error:
            raise SentUnanswered(
                f"OpenRouter's answer did not arrive within {timeout:g} s") from error
        except OSError as error:
            # http.client's RemoteDisconnected, a reset or a cut-short read:
            # the request had left.
            raise SentUnanswered(
                f"the connection dropped before OpenRouter answered ({type(error).__name__})") from error
        except Exception as error:  # noqa: BLE001 - http.client.IncompleteRead and kin
            raise SentUnanswered(
                f"OpenRouter's answer was cut short ({type(error).__name__})") from error
        try:
            return json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as error:
            raise SentUnanswered("OpenRouter's answer could not be read") from error
    raise RuntimeError("OpenRouter request failed") from last_error


def _choice(response: dict[str, Any]) -> dict[str, Any]:
    choices = response.get("choices") if isinstance(response, dict) else None
    if isinstance(choices, list) and choices and isinstance(choices[0], dict):
        return choices[0]
    return {}


def _raw_content(response: dict[str, Any]) -> str | None:
    """The answer text, or None when the response carries none at all."""
    message = _choice(response).get("message")
    content = message.get("content") if isinstance(message, dict) else None
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(
            item.get("text", "") for item in content if isinstance(item, dict)
            and isinstance(item.get("text"), str))
    return None


def completion_failure(response: dict[str, Any]) -> str | None:
    """Why this billed response is not an answer, or None when it may be one.

    The three shapes the benchmark met: a completion cut off at the token
    limit (``finish_reason: length``, 65 536 tokens of whitespace), a response
    billed with null content, and an empty one. Only ``stop`` is a finished
    answer; anything else (``content_filter``, ``error``, a missing reason) is
    named rather than parsed, because a half answer can still be valid JSON.
    """
    finish = _choice(response).get("finish_reason")
    content = _raw_content(response)
    if finish == "length":
        return f"MiMo's answer was cut off at the {MAX_COMPLETION_TOKENS}-token cap"
    if content is None:
        return "MiMo returned no answer text"
    if not content.strip():
        return "MiMo returned an empty answer"
    if finish != "stop":
        label = finish if isinstance(finish, str) and finish else "none"
        return f"MiMo's answer did not finish (finish reason: {label[:24]})"
    return None


def reported_cost_usd(response: dict[str, Any]) -> float | None:
    """What OpenRouter says this call cost, when it says so (``usage.cost``)."""
    usage = response.get("usage") if isinstance(response, dict) else None
    cost = usage.get("cost") if isinstance(usage, dict) else None
    if isinstance(cost, (int, float)) and not isinstance(cost, bool) and cost >= 0:
        return float(cost)
    return None


def ceil_usd(value: float, places: int) -> float:
    """Dollars rounded **up** at the precision shown, for a maximum.

    A bound printed at three decimals is a bound only if it rounds up: a
    capped call on 100 s is 0.032376 USD, and a nearest rounding read "at
    most 0.032 USD". Float noise below a millionth of the last place shown is
    not rounded up (0.033 stays 0.033). The same rule as
    ``core::assist::cost::ceil_thousandths``; both sides pin 100 s.
    """
    scale = 10 ** places
    return max(0.0, math.ceil(value * scale - 1e-6)) / scale


def _cost_clause(response: dict[str, Any], estimate: dict[str, Any] | None = None) -> str:
    cost = reported_cost_usd(response)
    if cost is not None:
        return f"OpenRouter billed {cost:.4f} USD for it"
    ceiling = (estimate or {}).get("usd_max")
    if isinstance(ceiling, (int, float)):
        return f"OpenRouter reported no cost for it (at most {ceil_usd(ceiling, 3):.3f} USD)"
    return "OpenRouter reported no cost for it"


def failure_receipt_path(cache_path: Path) -> Path:
    return cache_path.with_name(f"{cache_path.stem}.failed.json")


def failure_message(reason: str, response: dict[str, Any] | None,
                    estimate: dict[str, Any] | None = None) -> str:
    return f"{reason}. Nothing was saved; {_cost_clause(response or {}, estimate)}."


def unanswered_message(reason: str, estimate: dict[str, Any] | None = None) -> str:
    """What the person reads when the request left and no answer came back.

    Under the 160 bytes the application's diagnosis keeps, reason first.
    """
    ceiling = (estimate or {}).get("usd_max")
    bound = (f" (at most {ceil_usd(ceiling, 3):.3f} USD)"
             if isinstance(ceiling, (int, float)) else "")
    return f"{reason}; it may have been billed{bound}. Nothing was saved or sent again."


def _write_failure_receipt(
    cache_path: Path, *, reason: str, response: dict[str, Any] | None,
    settings: dict[str, Any], audio_sha256: str, detail: str | None = None,
    estimate: dict[str, Any] | None = None, message: str | None = None,
    may_be_billed: bool = False,
) -> dict[str, Any]:
    """The record of a failed run, beside (never in) the cache.

    It keeps what explains the failure and what it cost: the finish reason,
    the provider's ``usage`` and cost, the generation id, and the start of the
    answer text. The text is clipped, since a runaway answer is 65 k tokens.
    """
    response = response if isinstance(response, dict) else {}
    content = _raw_content(response)
    receipt = {
        "schema_version": FAILED_RUN_SCHEMA_VERSION,
        # When it failed: an earlier answer still in the cache is staged with
        # this receipt named beside it (H4c), and "when" is half of that.
        "failed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "reason": reason,
        "message": message or failure_message(reason, response, estimate),
        # The request left and nothing says whether the model ran: counted as
        # possibly billed, up to the estimate's maximum.
        "may_be_billed": may_be_billed,
        "detail": detail,
        "audio_sha256": audio_sha256,
        "request": settings,
        "finish_reason": _choice(response).get("finish_reason"),
        "generation": {
            key: response[key]
            for key in ("id", "created", "model", "provider", "usage")
            if key in response
        },
        "estimate": estimate,
        "reported_cost_usd": reported_cost_usd(response),
        "content_characters": None if content is None else len(content),
        "content_start": None if content is None else content[:FAILED_CONTENT_KEEP],
    }
    atomic_write_json(failure_receipt_path(cache_path), receipt)
    return receipt


def decode_model_score(response: dict[str, Any]) -> Any:
    content = _content(response)
    if not content:
        raise AnalysisValidationError("OpenRouter returned an empty completion")
    try:
        return json.loads(content)
    except json.JSONDecodeError as error:
        raise AnalysisValidationError("MiMo completion is not valid JSON") from error


def analyze_to_cache(
    audio_path: Path,
    cache_path: Path,
    *,
    audio_duration: float,
    audio_format: str | None = None,
    model: str = MODEL,
    provider_order: list[str] | None = None,
    provider_only: list[str] | None = None,
    provider_ignore: list[str] | None = None,
    allow_fallbacks: bool = True,
    zero_data_retention: bool = False,
    max_price: dict[str, float] | None = None,
    transport: Transport = urllib.request.urlopen,
    sleeper: Callable[[float], None] = time.sleep,
    _audio_bytes: bytes | None = None,
    _prepared_request: tuple[dict[str, Any], dict[str, Any]] | None = None,
) -> dict[str, Any]:
    audio_bytes = _audio_bytes if _audio_bytes is not None else audio_path.read_bytes()
    audio_hash = hashlib.sha256(audio_bytes).hexdigest()
    selected_format = (audio_format or audio_path.suffix.lstrip(".")).lower()
    if not selected_format:
        raise AnalysisValidationError("audio format is required")
    if _prepared_request is None:
        payload, settings = build_request(
            audio_bytes,
            audio_format=selected_format,
            audio_duration=audio_duration,
            model=model,
            provider_order=provider_order,
            provider_only=provider_only,
            provider_ignore=provider_ignore,
            allow_fallbacks=allow_fallbacks,
            zero_data_retention=zero_data_retention,
            max_price=max_price,
        )
    else:
        payload, settings = _prepared_request
    receipt_path = failure_receipt_path(cache_path)
    # A receipt from an earlier attempt describes that attempt, not this one.
    receipt_path.unlink(missing_ok=True)
    estimate = estimate_cost(str(settings.get("model") or model), audio_duration)
    try:
        response = submit_request(payload, transport=transport, sleeper=sleeper)
    except SentUnanswered as error:
        message = unanswered_message(str(error), estimate)
        _write_failure_receipt(
            cache_path, reason=str(error), response=None, settings=settings,
            audio_sha256=audio_hash, estimate=estimate, message=message,
            may_be_billed=True)
        raise FailedRun(message) from error

    def failed(reason: str, detail: str | None = None) -> FailedRun:
        _write_failure_receipt(
            cache_path, reason=reason, response=response, settings=settings,
            audio_sha256=audio_hash, detail=detail, estimate=estimate)
        return FailedRun(failure_message(reason, response, estimate))

    reason = completion_failure(response)
    if reason is not None:
        raise failed(reason)
    try:
        score = decode_model_score(response)
    except AnalysisValidationError as error:
        raise failed("MiMo's answer is not valid JSON", str(error)) from error
    try:
        normalized = normalize_semantic_score(
            score,
            audio_sha256=audio_hash,
            audio_duration=audio_duration,
            request_settings=settings,
            response_metadata=response,
            requested_model=str(settings.get("model") or model),
            cost_estimate=estimate,
        )
    except AnalysisValidationError as error:
        detail = str(error)
        raise failed(f"MiMo's answer does not fit the schema ({detail[:60]})",
                     detail) from error
    cache_key = canonical_sha256({"audio_sha256": audio_hash, **settings})
    envelope = {
        "schema_version": CACHE_SCHEMA_VERSION,
        "cache_key": cache_key,
        "request": {"audio_sha256": audio_hash, **settings},
        "raw_response": response,
        "normalized": normalized,
    }
    atomic_write_json(cache_path, envelope)
    return envelope


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("cache", type=Path, nargs="?")
    parser.add_argument("--duration", required=True, type=float)
    parser.add_argument("--audio-format")
    # The model is a flag rather than a constant as of tranche P4: it is
    # resolved from the TC-SEMANTIC route in the execution snapshot, and the
    # default below is only what an unrouted command line falls back to.
    parser.add_argument("--model", default=MODEL)
    parser.add_argument("--provider", action="append", dest="providers",
                        help="preferred provider slug, in order; repeatable")
    parser.add_argument("--only", action="append", dest="only",
                        help="restrict routing to these provider slugs; repeatable")
    parser.add_argument("--ignore", action="append", dest="ignore",
                        help="never route to these provider slugs; repeatable")
    parser.add_argument("--max-price-prompt", type=float)
    parser.add_argument("--max-price-completion", type=float)
    parser.add_argument("--max-price-audio", type=float)
    parser.add_argument("--no-fallbacks", action="store_true")
    parser.add_argument("--zdr", action="store_true", help="require Zero Data Retention providers")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--request-dump", type=Path)
    args = parser.parse_args(argv)
    try:
        audio_snapshot = _snapshot_audio(args.audio)
        audio_bytes, audio_hash = audio_snapshot
        audio_format = (args.audio_format or args.audio.suffix.lstrip(".")).lower()
        max_price = {
            name: value for name, value in (
                ("prompt", args.max_price_prompt),
                ("completion", args.max_price_completion),
                ("audio", args.max_price_audio),
            ) if value is not None
        }
        payload, settings = build_request(
            audio_bytes,
            audio_format=audio_format,
            audio_duration=args.duration,
            model=args.model,
            provider_order=args.providers,
            provider_only=args.only,
            provider_ignore=args.ignore,
            allow_fallbacks=not args.no_fallbacks,
            zero_data_retention=args.zdr,
            max_price=max_price,
        )
        if args.dry_run:
            dump = redacted_request_dump(payload, settings, audio_hash)
            if args.request_dump:
                atomic_write_json(args.request_dump, dump)
            else:
                print(json.dumps(dump, ensure_ascii=False, indent=2))
            return 0
        if args.request_dump:
            atomic_write_json(args.request_dump, redacted_request_dump(payload, settings, audio_hash))
        if args.cache is None:
            parser.error("cache is required unless only inspecting help")
        analyze_to_cache(
            args.audio,
            args.cache,
            audio_duration=args.duration,
            audio_format=audio_format,
            model=args.model,
            provider_order=args.providers,
            provider_only=args.only,
            provider_ignore=args.ignore,
            allow_fallbacks=not args.no_fallbacks,
            zero_data_retention=args.zdr,
            max_price=max_price,
            _audio_bytes=audio_bytes,
            _prepared_request=(payload, settings),
        )
    except (OSError, ValueError, RuntimeError) as error:
        print(f"MiMo analysis failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
