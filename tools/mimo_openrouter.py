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
    read_json,
    string_list,
    text,
)


OPENROUTER_URL = "https://openrouter.ai/api/v1/chat/completions"
MODEL = "xiaomi/mimo-v2.5"
PROMPT_VERSION = "musializer-semantic-score/v1"
SCORE_SCHEMA_VERSION = "musializer.semantic-score/v1"
CACHE_SCHEMA_VERSION = "musializer.analysis-cache/v1"
TRANSIENT_STATUSES = frozenset((429, 502, 503))
DEFAULT_TIMEOUT = 600.0
DEFAULT_MAX_ATTEMPTS = 3
COVERAGE_TOLERANCE_SECONDS = 0.001

Transport = Callable[..., Any]


def _snapshot_audio(audio_path: Path) -> tuple[bytes, str]:
    """Read one immutable snapshot used for both submission and identity."""
    audio_bytes = audio_path.read_bytes()
    return audio_bytes, hashlib.sha256(audio_bytes).hexdigest()


SYSTEM_PROMPT = """You are a music-to-visual creative interpreter. Describe how the
provided music feels, not what can be measured with certainty. Do not claim
authoritative lyrics, beat times, or signal facts. Return only JSON matching
the supplied SemanticScore schema. Cover the full requested duration with
non-overlapping interpretive segments. Continuous values must use the ranges
declared by the schema."""


def _creative_schema(include_timing: bool) -> dict[str, Any]:
    properties: dict[str, Any] = {
        "summary": {"type": "string"},
        "moods": {"type": "array", "items": {"type": "string"}},
        "energy": {"type": "number", "minimum": 0, "maximum": 1},
        "tension": {"type": "number", "minimum": 0, "maximum": 1},
        "valence": {"type": "number", "minimum": -1, "maximum": 1},
        "motion": {"type": "array", "items": {"type": "string"}},
        "textures": {"type": "array", "items": {"type": "string"}},
        "imagery": {"type": "array", "items": {"type": "string"}},
        "palette": {"type": "array", "items": {"type": "string"}},
        "scene_cues": {"type": "array", "items": {"type": "string"}},
        "confidence": {"type": "number", "minimum": 0, "maximum": 1},
    }
    required = list(properties)
    if include_timing:
        properties = {
            "start_seconds": {"type": "number"},
            "end_seconds": {"type": "number"},
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
        "energy": clamp(raw["energy"], 0, 1, f"{field}.energy"),
        "tension": clamp(raw["tension"], 0, 1, f"{field}.tension"),
        "valence": clamp(raw["valence"], -1, 1, f"{field}.valence"),
        "motion": string_list(raw["motion"], f"{field}.motion"),
        "textures": string_list(raw["textures"], f"{field}.textures"),
        "imagery": string_list(raw["imagery"], f"{field}.imagery"),
        "palette": string_list(raw["palette"], f"{field}.palette"),
        "scene_cues": string_list(raw["scene_cues"], f"{field}.scene_cues"),
        "confidence": clamp(raw["confidence"], 0, 1, f"{field}.confidence"),
    }


def normalize_semantic_score(
    raw_score: Any,
    *,
    audio_sha256: str,
    audio_duration: float,
    request_settings: dict[str, Any],
    response_metadata: dict[str, Any] | None = None,
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
        "model": str(metadata.get("model") or MODEL),
        "provider": metadata.get("provider"),
        "prompt_version": PROMPT_VERSION,
        "request_settings": request_settings,
        "generation": {
            key: metadata[key]
            for key in ("id", "created", "usage")
            if key in metadata
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


def build_request(
    audio_bytes: bytes,
    *,
    audio_format: str,
    audio_duration: float,
    provider_order: list[str] | None = None,
    allow_fallbacks: bool = True,
    zero_data_retention: bool = False,
) -> tuple[dict[str, Any], dict[str, Any]]:
    measured_duration = duration(audio_duration)
    settings: dict[str, Any] = {
        "model": MODEL,
        "prompt_version": PROMPT_VERSION,
        "prompt_sha256": canonical_sha256(SYSTEM_PROMPT),
        "response_schema_version": SCORE_SCHEMA_VERSION,
        "model_output_schema_sha256": canonical_sha256(MODEL_OUTPUT_SCHEMA),
        "audio_duration_seconds": measured_duration,
        "audio_format": audio_format,
        "allow_fallbacks": allow_fallbacks,
        "zero_data_retention": zero_data_retention,
        "provider_order": provider_order or [],
    }
    provider: dict[str, Any] = {
        "allow_fallbacks": allow_fallbacks,
        "require_parameters": True,
    }
    if provider_order:
        provider["order"] = provider_order
    if zero_data_retention:
        provider["zdr"] = True
    payload = {
        "model": MODEL,
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
                "name": "musializer_semantic_score_v1",
                "strict": True,
                "schema": MODEL_OUTPUT_SCHEMA,
            },
        },
        "provider": provider,
        "temperature": 0.2,
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


def submit_request(
    payload: dict[str, Any],
    *,
    transport: Transport = urllib.request.urlopen,
    sleeper: Callable[[float], None] = time.sleep,
    now: Callable[[], float] = time.time,
    timeout: float = DEFAULT_TIMEOUT,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
) -> dict[str, Any]:
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
                parsed = json.loads(response.read().decode("utf-8"))
            if not _content(parsed):
                raise EOFError("OpenRouter returned an empty completion")
            return parsed
        except urllib.error.HTTPError as error:
            last_error = error
            if error.code not in TRANSIENT_STATUSES or attempt == max_attempts:
                error.close()
                raise RuntimeError(f"OpenRouter request failed with HTTP {error.code}") from error
            delay = _retry_delay(error.headers, attempt, now)
            error.close()
            sleeper(delay)
        except EOFError as error:
            last_error = error
            if attempt == max_attempts:
                raise RuntimeError("OpenRouter returned empty completions") from error
            sleeper(min(2 ** (attempt - 1), 8))
    raise RuntimeError("OpenRouter request failed") from last_error


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
    provider_order: list[str] | None = None,
    allow_fallbacks: bool = True,
    zero_data_retention: bool = False,
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
            provider_order=provider_order,
            allow_fallbacks=allow_fallbacks,
            zero_data_retention=zero_data_retention,
        )
    else:
        payload, settings = _prepared_request
    response = submit_request(payload, transport=transport, sleeper=sleeper)
    score = decode_model_score(response)
    normalized = normalize_semantic_score(
        score,
        audio_sha256=audio_hash,
        audio_duration=audio_duration,
        request_settings=settings,
        response_metadata=response,
    )
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
    parser.add_argument("--provider", action="append", dest="providers")
    parser.add_argument("--no-fallbacks", action="store_true")
    parser.add_argument("--zdr", action="store_true", help="require Zero Data Retention providers")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--request-dump", type=Path)
    args = parser.parse_args(argv)
    try:
        audio_snapshot = _snapshot_audio(args.audio)
        audio_bytes, audio_hash = audio_snapshot
        audio_format = (args.audio_format or args.audio.suffix.lstrip(".")).lower()
        payload, settings = build_request(
            audio_bytes,
            audio_format=audio_format,
            audio_duration=args.duration,
            provider_order=args.providers,
            allow_fallbacks=not args.no_fallbacks,
            zero_data_retention=args.zdr,
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
            provider_order=args.providers,
            allow_fallbacks=not args.no_fallbacks,
            zero_data_retention=args.zdr,
            _audio_bytes=audio_bytes,
            _prepared_request=(payload, settings),
        )
    except (OSError, ValueError, RuntimeError) as error:
        print(f"MiMo analysis failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
