#!/usr/bin/env python3
"""Create cached semantic scores (the song's sections and how each feels)
through one hosted listening call on OpenRouter.

Model-independent: the cap, the failed-run receipts, the one-send rule, the
estimate and the stated scales hold for every model. What differs per model is
looked up by its slug (``MODEL_LABELS``). Until 2026-11-15 this file was
``tools/mimo_openrouter.py``; caches and receipts it wrote are still read.

The helper is opt-in and external to the renderer. It reads the API key only
from the process environment. ``--dry-run`` never opens a network connection
and emits a safely redacted request description.
"""

from __future__ import annotations

import argparse
import base64
import calendar
import email.utils
import fcntl
import hashlib
import json
import math
import os
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
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
# What an unrouted command line asks for. The application always passes the
# TC-SEMANTIC route's model (`--model`); since 2026-11-15 its recommended route
# names the same model (operator's choice, docs/DIVERGENCES.md).
MODEL = "google/gemini-3.8-flash"
# v2 (2026-11-14): the numeric scales are defined in the prompt and the
# schema, and an off-scale number fails the run instead of being clamped. A v1
# answer was produced without those definitions, so it is a different request
# (`prompt_sha256`, `model_output_schema_sha256`) and is regenerated, not
# reused; the document it produced is still a readable semantic-score/v1.
PROMPT_VERSION = "musializer-semantic-score/v2"
SCORE_SCHEMA_VERSION = "musializer.semantic-score/v1"
CACHE_SCHEMA_VERSION = "musializer.analysis-cache/v1"
# Retried, because OpenRouter says the request was not processed: 429 is a
# rate limit applied before routing.
TRANSIENT_STATUSES = frozenset((429,))
# Clear refusals: OpenRouter answers these before the request reaches a model,
# so nothing ran and nothing is billed. Its error list (openrouter.ai/docs,
# "Errors", read 2026-11-14) names 400 (bad or missing parameters), 401 (the
# key), 402 (too little credit) and 403 (the input flagged by moderation); 404
# (no such route or model), 405, 413 (body too large), 415 and 422 (a body it
# cannot take) refuse the request as sent. **Every other status** after the
# request left — 408, any 5xx, a Cloudflare 52x, a 3xx, a 4xx not listed —
# may come after the model ran: a failed run that may have been billed, with
# a receipt, never a silent second call (H4c, H4d; the confirmation the person
# accepted budgets one call).
REFUSED_STATUSES = frozenset((400, 401, 402, 403, 404, 405, 413, 415, 422))
DEFAULT_TIMEOUT = 600.0
DEFAULT_MAX_ATTEMPTS = 3
COVERAGE_TOLERANCE_SECONDS = 0.001
FAILED_RUN_SCHEMA_VERSION = "musializer.semantic-failed-run/v1"
# Receipts this helper reads: its own, and the ones it wrote as
# tools/mimo_openrouter.py (same fields, older name).
READABLE_FAILED_RUN_SCHEMAS = frozenset((
    FAILED_RUN_SCHEMA_VERSION, "musializer.mimo-failed-run/v1"))
# What a cached answer's provenance names as its maker. The second pair is the
# same helper under its old name: an answer it made is still this request's
# answer when the identity matches (a user who stays on MiMo is not billed
# again for a rename).
ADAPTER_PATH = "tools/semantic_openrouter.py"
SOURCE_KIND = "openrouter_semantic"
ACCEPTED_ADAPTERS = ((ADAPTER_PATH, SOURCE_KIND),
                     ("tools/mimo_openrouter.py", "mimo_openrouter"))
@dataclass(frozen=True)
class AudioEncoding:
    """How the track is re-encoded before it is sent, when it is.

    FFmpeg reads the snapshot taken for the hash and writes one stream with
    every tag dropped: no title, artist or cover art leaves with the audio.
    """
    format: str          # the `input_audio.format` sent
    codec: str
    sample_rate: int
    channels: int
    bitrate: str

    def identity(self) -> str:
        return (f"{self.format} {self.codec} {self.sample_rate} Hz "
                f"{self.channels} ch {self.bitrate}")


@dataclass(frozen=True)
class ModelProfile:
    """Everything about the request that is really per model, in one row.

    The rest of this file holds for any model. A model with no row gets
    ``UNLISTED``: the request shape this helper sent before rows existed, the
    common cap, and no dollar figure (never a guessed one).

    Prices are USD per million tokens as OpenRouter listed them on
    ``prices_listed_on``; every number used by the estimate has a twin in
    `crates/musializer-core/src/assist/cost.rs`, pinned by
    tests/test_cross_language_pins.py: change both or neither.
    """
    label: str
    # The completion cap, reasoning included (OpenRouter's `max_tokens`).
    max_completion_tokens: int
    # The estimate's token model.
    audio_tokens_per_second: float
    prompt_text_tokens: int
    typical_completion_tokens: tuple[int, int]
    prompt_usd: float | None = None
    audio_usd: float | None = None
    completion_usd: float | None = None
    prices_listed_on: str | None = None
    # None: the source file's bytes are sent as they are, in its own format.
    audio_encoding: AudioEncoding | None = None
    # None: not sent, the provider's default applies.
    temperature: float | None = 0.2
    # The largest request body this helper sends to the model; past it the run
    # stops before anything leaves. None: no local bound.
    max_request_bytes: int | None = None

    def priced(self) -> bool:
        return None not in (self.prompt_usd, self.audio_usd, self.completion_usd)

    def identity_extra(self) -> dict[str, Any]:
        """The request-shape fields a cache key carries beyond the common ones.

        Empty for the shape every answer before 2026-11-15 was made under, so
        those identities, and a MiMo user's cache, stay byte-identical.
        """
        if self.audio_encoding is None and self.temperature == 0.2:
            return {}
        return {
            "audio_encoding": (self.audio_encoding.identity()
                               if self.audio_encoding else "source"),
            "temperature": self.temperature,
        }


# The cap before rows existed, kept for MiMo and for any model with no row.
# Why 16 000: without a cap the provider's own ceiling, 65 536, applied, and in
# the 2026-11-14 benchmark one MiMo call emitted that many tokens of whitespace
# (0.131 USD) and another ran to 43 578. The largest legitimate answer used
# 10 942 on a 100-second excerpt; the cap leaves half again as much room and
# still cuts both runaways. docs/MIMO_BENCHMARK_RESULTS.md, "Cost".
DEFAULT_MAX_COMPLETION_TOKENS = 16_000
UNLISTED = ModelProfile(
    label="",
    max_completion_tokens=DEFAULT_MAX_COMPLETION_TOKENS,
    # MiMo's calibration: the only one there is for a model nobody measured.
    audio_tokens_per_second=7.4,
    prompt_text_tokens=200,
    typical_completion_tokens=(1_500, 11_000),
)

MODEL_PROFILES: dict[str, ModelProfile] = {
    # Calibrated from the 2026-11-14 benchmark: audio at 7.4 tokens per second
    # (recorded `usage.prompt_tokens`), the request's text about 200 tokens
    # (928 prompt tokens for 100 s, 740 of them audio), answers of 2 044 and
    # 2 407 on this lane's capped request, 5 110 on the benchmark's shape of
    # it, 10 942 the largest legitimate one. Venice, the only provider that
    # day, lists no audio price: audio bills as prompt tokens. MiMo reasons
    # before it answers, billed at the completion rate.
    "xiaomi/mimo-v2.5": ModelProfile(
        label="MiMo",
        max_completion_tokens=DEFAULT_MAX_COMPLETION_TOKENS,
        audio_tokens_per_second=7.4,
        prompt_text_tokens=200,
        typical_completion_tokens=(1_500, 11_000),
        prompt_usd=0.40, audio_usd=0.40, completion_usd=2.00,
        prices_listed_on="2026-11-14",
    ),
    # The lane's model since 2026-11-15 (operator). OpenRouter, 2026-11-15:
    # 0.825 USD per million input and audio tokens and 4.125 per million output,
    # thinking included (`internal_reasoning`), the highest of the endpoints a
    # base slug routes to (Vertex us; Vertex global and AI Studio list 0.75 and
    # 3.75; the flex and priority tiers are opt-in only). Google: 32 tokens per
    # second of audio; ZDR endpoints are Vertex's only.
    #
    # Measured on this request, 2026-11-15, three calls served by Google
    # (Vertex): 2 502 audio tokens for 100 s and 7 502 for 300 s (25 per
    # second; the estimate keeps Google's 32, the higher), 1 650 tokens of text
    # every time, answers of 2 231 and 1 915 tokens on the two 100 s excerpts
    # and 5 030 on 300 s, of them 641, 0 and 2 264 thinking. The typical span is
    # those answers widened for a long track; the cap is about three times the
    # 300 s answer, room for a long song at the default thinking level, and it
    # stops a runaway at 0.066 USD of output. docs/ASSIST_PIPELINE.md, "Gemini
    # 3.8 Flash through OpenRouter", has the sources.
    "google/gemini-3.8-flash": ModelProfile(
        label="Gemini",
        max_completion_tokens=16_000,
        audio_tokens_per_second=32.0,
        prompt_text_tokens=1_650,
        typical_completion_tokens=(1_500, 8_000),
        prompt_usd=0.825, audio_usd=0.825, completion_usd=4.125,
        prices_listed_on="2026-11-15",
        # Google downsamples audio to 16 kbps and one channel itself; MP3 at
        # 128 kbit/s keeps everything it hears and keeps a long track inside
        # Google AI Studio's 20 MB request limit (about 15 minutes).
        audio_encoding=AudioEncoding(format="mp3", codec="libmp3lame",
                                     sample_rate=44_100, channels=2, bitrate="128k"),
        # Not sent: Google asks for the default (1.0) on Gemini 3, below which
        # it may loop, and OpenRouter's Vertex endpoints (the ZDR ones) do not
        # list `temperature`, so `require_parameters` would leave no endpoint.
        temperature=None,
        max_request_bytes=19_500_000,
    ),
}


def model_profile(model: str) -> ModelProfile:
    return MODEL_PROFILES.get(model, UNLISTED)


def model_label(model: str) -> str:
    """The name a sentence uses for the model; an unlisted one is its slug."""
    return model_profile(model).label or model


def max_completion_tokens(model: str) -> int:
    return model_profile(model).max_completion_tokens


# The cost estimate, built from completion tokens as much as from audio: a
# reasoning model's answer is most of its bill. v1 (2026-11-15) replaces
# musializer.mimo-cost-estimate/v1, adding the audio price and per-model rows.
COST_ESTIMATE_VERSION = "musializer.semantic-cost-estimate/v1"

# Bytes of a failed answer's text kept in its receipt: enough to see what kind
# of failure it was (whitespace, prose, half a JSON object), never 65 k tokens.
FAILED_CONTENT_KEEP = 4096

Transport = Callable[..., Any]


def estimate_cost(model: str, audio_duration: float) -> dict[str, Any]:
    """What one call should cost, before it is sent: a range and a ceiling.

    The range is the measured (or, for a new row, stated) span of answer
    lengths; the ceiling is what a call that runs to the completion cap costs,
    which is the most it can. Prices are a dated table, not a live quote; an
    unlisted model gets token counts and no dollar figures rather than a guess.
    """
    profile = model_profile(model)
    seconds = duration(audio_duration)
    audio_tokens = math.ceil(seconds * profile.audio_tokens_per_second)
    text_tokens = profile.prompt_text_tokens
    input_tokens = audio_tokens + text_tokens
    low_tokens, high_tokens = profile.typical_completion_tokens
    estimate: dict[str, Any] = {
        "version": COST_ESTIMATE_VERSION,
        "model": model,
        "audio_seconds": seconds,
        "audio_tokens": audio_tokens,
        "input_tokens": input_tokens,
        "completion_tokens_typical": [low_tokens, high_tokens],
        "completion_tokens_max": profile.max_completion_tokens,
        "prices_usd_per_million": None,
        "prices_listed_on": None,
        "usd_typical": None,
        "usd_max": None,
    }
    if not profile.priced():
        return estimate

    def usd(completion_tokens: int) -> float:
        return round((audio_tokens * profile.audio_usd
                      + text_tokens * profile.prompt_usd
                      + completion_tokens * profile.completion_usd) / 1_000_000.0, 6)

    estimate.update({
        "prices_usd_per_million": {"prompt": profile.prompt_usd,
                                   "audio": profile.audio_usd,
                                   "completion": profile.completion_usd},
        "prices_listed_on": profile.prices_listed_on,
        "usd_typical": [usd(low_tokens), usd(high_tokens)],
        "usd_max": usd(profile.max_completion_tokens),
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


# The bounds the model is given (defect J1-4, 2026-11-16). A compliant answer
# always fits what the application can hold: the bridge takes 256 semantic
# cues and 2 047 bytes of summary each (`core::project::analysis_bridge`,
# SEMANTIC_CAPACITY, SUMMARY_MAX_BYTES; pinned by
# tests/test_cross_language_pins.py). 64 sections is a song cut every ten
# seconds for over ten minutes; 500 characters are at most 2 000 bytes.
SEGMENT_MAX = 64
SUMMARY_MAX_CHARACTERS = 500
BRIDGE_SEMANTIC_CAPACITY = 256
BRIDGE_SUMMARY_MAX_BYTES = 2047


def _creative_schema(include_timing: bool, bounded: bool = True) -> dict[str, Any]:
    properties: dict[str, Any] = {
        "summary": ({"type": "string", "maxLength": SUMMARY_MAX_CHARACTERS}
                    if bounded else {"type": "string"}),
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


def _output_schema(bounded: bool) -> dict[str, Any]:
    segments: dict[str, Any] = {
        "type": "array",
        "minItems": 1,
        "description": "A contiguous, ordered, non-overlapping partition from 0 through the full audio duration.",
        "items": _creative_schema(True, bounded),
    }
    if bounded:
        # In words, not as `maxItems`: Google's endpoints refuse the whole
        # request for that keyword (HTTP 400 "Provider returned error", found
        # by the first Start pressed through the window, 2026-11-16;
        # `maxLength` and `minItems` are accepted). `bridge_fit` enforces what
        # the application can hold before an answer is cached. The prompt is
        # left alone, so an answer asked under the unbounded schema keeps its
        # identity (`legacy_request_identity`).
        segments["description"] += f" At most {SEGMENT_MAX} segments."
    return {
        "type": "object",
        "additionalProperties": False,
        "required": ["global", "segments"],
        "properties": {
            "global": _creative_schema(False, bounded),
            "segments": segments,
        },
    }


MODEL_OUTPUT_SCHEMA: dict[str, Any] = _output_schema(True)
# The schema every answer from 2026-11-14 to 2026-11-16 was asked under, which
# stated no bounds. Its hash is in those answers' identity; an answer made under
# it that fits the bridge is the same answer the bounded request asks for, so it
# is still reused rather than paid for again (`legacy_request_identity`).
LEGACY_OUTPUT_SCHEMA_SHA256 = "1d1c6b45bacf6dda612875ab84979371bcf725d2241c07fda62e9442c50d25f1"


def legacy_request_identity(identity: dict[str, Any]) -> dict[str, Any]:
    """The same request as it was identified before the schema had bounds."""
    return {**identity, "model_output_schema_sha256": LEGACY_OUTPUT_SCHEMA_SHA256}


def _clean_text(value: str) -> str:
    """Drawable UTF-8: no NUL or other control character the bridge's reader
    refuses or a card cannot draw, no lone surrogate (JSON allows "\\ud800";
    UTF-8 cannot carry it)."""
    cleaned = value.encode("utf-8", "replace").decode("utf-8")
    return "".join(" " if (ord(char) < 32 or ord(char) == 127) else char for char in cleaned)


REPAIR_KINDS = ("joined", "shortened", "cleaned")


def bridge_fit(normalized: dict[str, Any],
               counts: dict[str, int] | None = None) -> tuple[dict[str, Any], list[str]]:
    """The answer as the bridge will take it, and the repairs that made it so.

    Everything the bridge's reader requires of the semantic lane is checked
    here, before the answer is cached (defect J1-4): an answer the later steps
    refuse was cached and then refused on every run, for good. Harmless
    repairs are made and named (a section whose ends round to the same
    millisecond joins its neighbour; a summary is shortened at a character
    boundary; an unprintable character becomes a space); an answer that no
    repair can fit raises, and is a failed run with a receipt.

    ``counts``, when given, receives how many of each repair were made
    (``REPAIR_KINDS``), which the staged card says in its own words (J3-3).
    """
    repairs: list[str] = []
    segments = [dict(segment) for segment in normalized.get("segments") or []]
    merged: list[dict[str, Any]] = []
    joined = 0
    for segment in segments:
        start_ms = round(float(segment["start_seconds"]) * 1000)
        end_ms = round(float(segment["end_seconds"]) * 1000)
        if end_ms <= start_ms:
            joined += 1
            if merged:
                merged[-1]["end_seconds"] = segment["end_seconds"]
            else:
                # The first section: the next one starts where this did.
                segment["_join_next"] = True
                merged.append(segment)
            continue
        if merged and merged[-1].pop("_join_next", False):
            segment["start_seconds"] = merged[-1]["start_seconds"]
            merged[-1] = segment
            continue
        merged.append(segment)
    if merged and merged[-1].pop("_join_next", False):
        raise AnalysisValidationError("the answer has no section a millisecond long")
    if joined:
        repairs.append(f"joined {joined} section{'s' if joined != 1 else ''} shorter than a "
                       "millisecond to the one before")
    if len(merged) > BRIDGE_SEMANTIC_CAPACITY:
        raise AnalysisValidationError(
            f"the answer has {len(merged)} sections, more than the "
            f"{BRIDGE_SEMANTIC_CAPACITY} the application can hold")
    shortened = cleaned = 0
    for segment in merged:
        summary = str(segment.get("summary", ""))
        text = _clean_text(summary)
        if text != summary:
            cleaned += 1
        encoded = text.encode("utf-8")
        if len(encoded) > BRIDGE_SUMMARY_MAX_BYTES:
            text = encoded[:BRIDGE_SUMMARY_MAX_BYTES - 3].decode("utf-8", "ignore").rstrip() + "..."
            shortened += 1
        segment["summary"] = text
    if cleaned:
        repairs.append(f"replaced unprintable characters in {cleaned} summar"
                       f"{'ies' if cleaned != 1 else 'y'}")
    if shortened:
        repairs.append(f"shortened {shortened} summar{'ies' if shortened != 1 else 'y'} to "
                       f"{BRIDGE_SUMMARY_MAX_BYTES} bytes")
    if counts is not None:
        counts.update({"joined": joined, "shortened": shortened, "cleaned": cleaned})
    if not repairs:
        return normalized, []
    fitted = {**normalized, "segments": merged}
    return fitted, repairs


# Float noise at a bound (1.0000000002) is the bound; anything further out is
# an answer on another scale.
SCALE_TOLERANCE = 1e-6


def _on_scale(raw: Any, name: str, field: str) -> float:
    """The value, when it lies on its stated scale; a schema failure otherwise.

    Not a clamp: `energy: 78` clamped to 1.0 would be drawn as the loudest
    passage of the song, and an answer on a 0-100 scale has its other numbers
    on that scale too. So the run fails, naming the field, rather than one
    field being dropped (docs/DIVERGENCES.md, "MiMo numeric scales", which
    holds for every model).
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
        "adapter": ADAPTER_PATH,
        "adapter_version": ADAPTER_VERSION,
        "source_kind": SOURCE_KIND,
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

    The per-model request shape is in it too (``ModelProfile.identity_extra``,
    the cap, and the format actually sent), so an answer made under another
    model's shape is never this one's. For MiMo and unlisted models it is the
    identity this helper wrote before model rows existed, byte for byte.
    """
    profile = model_profile(model)
    if profile.audio_encoding is not None:
        audio_format = profile.audio_encoding.format
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
        "max_tokens": profile.max_completion_tokens,
        **profile.identity_extra(),
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
    """The request body and its identity.

    ``audio_bytes`` are what is sent: the source file's bytes, or, for a model
    whose row names an encoding, what ``prepare_audio`` made of them.
    """
    profile = model_profile(model)
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
                    "format": settings["audio_format"],
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
        **({"temperature": profile.temperature}
           if profile.temperature is not None else {}),
        "max_tokens": profile.max_completion_tokens,
    }
    return payload, settings


def ffmpeg_executable() -> str:
    """FFmpeg as an absolute path: a bare name is no child under a private
    ``PATH`` (docs/TRAPS.md)."""
    found = shutil.which("ffmpeg")
    if not found:
        raise RuntimeError("FFmpeg was not found, and it is needed to encode the track; "
                           "nothing was sent")
    return str(Path(found).resolve())


# What FFmpeg needs from the helper's environment. An allowlist, so that the
# OpenRouter key (and any other credential the helper was started with) never
# reaches a child that only decodes audio.
FFMPEG_ENVIRONMENT = ("PATH", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR",
                      "LD_LIBRARY_PATH")
ENCODE_STDERR_BYTES = 4096


def ffmpeg_environment() -> dict[str, str]:
    return {name: os.environ[name] for name in FFMPEG_ENVIRONMENT if name in os.environ}


def encode_audio(audio_bytes: bytes, source_format: str, encoding: AudioEncoding,
                 *, workdir: Path | None = None, timeout: float = 600.0,
                 max_bytes: int | None = None) -> bytes:
    """The snapshot re-encoded as ``encoding`` says, with every tag dropped.

    FFmpeg reads a private copy of the snapshot rather than the source path,
    so the bytes sent are the bytes hashed even if the file changes meanwhile.
    The copy is an anonymous in-memory file handed over as FFmpeg's standard
    input and read through ``/proc/self/fd/0``: seekable, which an MP4/M4A with
    its index at the end needs, and with no name on disk, so nothing of the
    track is left behind when the parent kills this process group at its own
    deadline. The encoding is read in pieces and stopped at ``max_bytes``: a
    track too long to send is refused without first being held whole.
    """
    # FFmpeg probes the content and no file is written, so neither is used;
    # both stay in the signature the callers and the tests' stand-ins share.
    _ = (source_format, workdir)
    ffmpeg = ffmpeg_executable()
    with os.fdopen(os.memfd_create("semantic-encode"), "wb+") as source, \
            tempfile.TemporaryFile() as errors:
        source.write(audio_bytes)
        source.flush()
        source.seek(0)
        process = subprocess.Popen(
            [ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error",
             "-i", "/proc/self/fd/0", "-map", "0:a:0", "-vn", "-sn", "-dn",
             "-map_metadata", "-1", "-map_chapters", "-1",
             "-ac", str(encoding.channels), "-ar", str(encoding.sample_rate),
             "-c:a", encoding.codec, "-b:a", encoding.bitrate,
             "-id3v2_version", "0", "-write_id3v1", "0",
             "-f", encoding.format, "pipe:1"],
            stdin=source, stdout=subprocess.PIPE, stderr=errors,
            env=ffmpeg_environment())
        assert process.stdout is not None
        output = process.stdout.fileno()
        deadline = time.monotonic() + timeout
        pieces: list[bytes] = []
        total = 0
        refusal: str | None = None
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    refusal = "FFmpeg did not finish encoding the track in time"
                    break
                ready, _, _ = select.select([output], [], [], min(remaining, 1.0))
                if not ready:
                    continue
                piece = os.read(output, 1 << 16)
                if not piece:
                    break
                total += len(piece)
                if max_bytes is not None and total > max_bytes:
                    refusal = (f"the track encodes to more than {max_bytes / 1e6:.1f} MB, "
                               "past what one request may carry")
                    break
                pieces.append(piece)
        finally:
            if refusal is not None:
                process.kill()
            process.stdout.close()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        if refusal is not None:
            raise RuntimeError(f"{refusal}; nothing was sent")
        if process.returncode != 0 or not pieces:
            errors.seek(0)
            detail = errors.read(ENCODE_STDERR_BYTES).decode("utf-8", "replace")
            # FFmpeg names its input; the descriptor's path says nothing useful.
            detail = " ".join(detail.replace("/proc/self/fd/0", "the track").split())[:120]
            raise RuntimeError(f"FFmpeg could not encode the track ({detail or 'no output'}); "
                               "nothing was sent")
    return b"".join(pieces)


def prepare_audio(audio_bytes: bytes, source_format: str, model: str,
                  *, encoder: Callable[..., bytes] = encode_audio,
                  workdir: Path | None = None) -> bytes:
    """What is sent for this model: the snapshot itself, or its encoding."""
    profile = model_profile(model)
    if profile.audio_encoding is None:
        return audio_bytes
    # Base64 only grows the encoding, so one past the request's own limit can
    # never be sent; stopping there bounds what is held.
    return encoder(audio_bytes, source_format, profile.audio_encoding,
                   workdir=workdir, max_bytes=profile.max_request_bytes)


def oversize_reason(payload: dict[str, Any], model: str) -> str | None:
    """Why this body is too large to send to this model, or None.

    Checked before anything leaves, so a refusal costs nothing: a provider
    that refuses a large body after reading it (413) is a refusal too, but a
    gateway that times out on one is not, and either way the person waits.
    """
    limit = model_profile(model).max_request_bytes
    if limit is None:
        return None
    size = len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))
    if size <= limit:
        return None
    return (f"the track encodes to a {size / 1e6:.1f} MB request, past the "
            f"{limit / 1e6:.1f} MB one {model_label(model)} request may carry; nothing was sent")


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


# How long a sentence the person reads may be. The helper's words reach the
# Assist panel two ways: the panel reads the receipt and shows the message
# whole (its notice keeps 224 bytes, `core::assist::diagnosis::CAUSE_MAX_BYTES`
# + 64), and the job log carries it behind "External analysis failed: ", where
# the panel's fallback reader clips at 160 bytes (`CAUSE_MAX_BYTES`). So what
# happened and what it cost come first and fit in 160; the next step follows
# and the whole fits in 224. Both numbers are pinned by
# tests/test_cross_language_pins.py.
ESSENTIAL_MAX_BYTES = 160
MESSAGE_MAX_BYTES = 224
ELLIPSIS = "..."


def bounded(head: str, variable: str, tail: str, after: str = "") -> str:
    """``head + variable + tail + after``, sized for the panel.

    Only ``variable`` (a provider's words, a schema error, a reason) is
    shortened, at a character boundary and with an ellipsis, so that
    ``head + variable + tail`` fits in ``ESSENTIAL_MAX_BYTES``: the head says
    what happened and the tail what it cost, and both survive whatever a
    provider or a schema says. ``after`` is the next step, which follows.
    """
    room = (ESSENTIAL_MAX_BYTES - len(head.encode("utf-8")) - len(tail.encode("utf-8")))
    encoded = variable.encode("utf-8")
    if len(encoded) > room:
        keep = max(0, room - len(ELLIPSIS))
        variable = encoded[:keep].decode("utf-8", "ignore").rstrip() + ELLIPSIS
        if len(variable.encode("utf-8")) > room:
            variable = ""
    # The next step is this file's own fixed words, never a provider's.
    assert len(after.encode("utf-8")) <= MESSAGE_MAX_BYTES - ESSENTIAL_MAX_BYTES, after
    return f"{head}{variable}{tail}{after}"


class Refused(RuntimeError):
    """The request was refused before a model ran, or never left whole.

    Nothing was billed. ``kind`` says which refusal it was, for the receipt
    and the panel's headline; the message is the sentence the person reads,
    with the next step in it.
    """

    def __init__(self, message: str, *, kind: str, status: int | None = None,
                 detail: str | None = None, sent: bool = True) -> None:
        super().__init__(message)
        self.kind = kind
        self.status = status
        self.detail = detail
        # Whether the request reached OpenRouter at all (a refusal) or not
        # (no key, no connection): the sentence says "billed" or "sent".
        self.sent = sent


# What a refused request's sentence says about where the key came from. The
# application sets this beside the key (never the key itself): "settings" for
# the 0600 file AI settings writes, "environment" for the OPENROUTER_API_KEY
# the application was started with.
KEY_ORIGIN_VARIABLE = "MUSIALIZER_OPENROUTER_FROM"


def _provider_message(error: urllib.error.HTTPError) -> str | None:
    """OpenRouter's own sentence from an error body, bounded, or None."""
    try:
        raw = error.read(4096)
    except Exception:  # noqa: BLE001 - a body that cannot be read says nothing
        return None
    try:
        body = json.loads(raw.decode("utf-8", "replace"))
    except ValueError:
        return None
    message = None
    if isinstance(body, dict):
        inner = body.get("error")
        message = inner.get("message") if isinstance(inner, dict) else body.get("message")
    if not isinstance(message, str):
        return None
    return " ".join(message.split())[:300] or None


def privacy_constraints(settings: dict[str, Any] | None) -> list[str]:
    """The routing constraints a request carried, in the words a person set
    them with (AI settings: Privacy and Routing)."""
    settings = settings or {}
    named: list[str] = []
    if settings.get("zero_data_retention"):
        named.append("zero data retention")
    if settings.get("provider_only"):
        named.append("only " + ", ".join(map(str, settings["provider_only"])))
    if settings.get("provider_ignore"):
        named.append("not " + ", ".join(map(str, settings["provider_ignore"])))
    if settings.get("max_price"):
        named.append("a price limit")
    if settings.get("allow_fallbacks") is False:
        named.append("no fallbacks")
    return named


def refusal_message(status: int, settings: dict[str, Any] | None, model: str,
                    provider_message: str | None) -> tuple[str, str]:
    """``(kind, sentence)`` for a clear refusal: what happened, that nothing
    was billed, and what to do next."""
    billed = "; nothing was billed."
    if status == 401:
        origin = os.environ.get(KEY_ORIGIN_VARIABLE, "").strip()
        if origin == "environment":
            return "key_rejected", bounded(
                "OpenRouter refused the OPENROUTER_API_KEY this app was started with "
                "(HTTP 401)", "", billed, " Add a working key in AI settings \u2192 OpenRouter.")
        if origin == "settings":
            return "key_rejected", bounded(
                "OpenRouter refused the key saved in AI settings \u2192 OpenRouter (HTTP 401)",
                "", billed, " Replace it there, then start again.")
        return "key_rejected", bounded(
            "OpenRouter refused the API key (HTTP 401)", "", billed,
            " Replace it in AI settings \u2192 OpenRouter, then start again.")
    if status == 402:
        return "no_credit", bounded(
            "OpenRouter refused the request: the account needs credit (HTTP 402)", "",
            billed, " Add credit at openrouter.ai, then start again.")
    if status == 404:
        named = privacy_constraints(settings)
        if named:
            return "no_endpoint", bounded(
                f"No OpenRouter endpoint for {model_label(model)} fits ", ", ".join(named),
                f" (HTTP 404){billed}",
                " Relax it in AI settings, or refresh the model list there.")
        return "no_endpoint", bounded(
            "OpenRouter has no endpoint for ", model, f" (HTTP 404){billed}",
            " Refresh the model list in AI settings \u2192 OpenRouter.")
    said = f": {provider_message}" if provider_message else ""
    # A request the provider calls malformed is this application's fault (the
    # first real Start was one: a schema keyword Google refuses), and a person
    # who reads "refused" looks at their key and settings first.
    return "refused", bounded(f"OpenRouter refused the request (HTTP {status}", said,
                              f"){billed}",
                              " The request itself is at fault, not your key or settings.")


class SentUnanswered(RuntimeError):
    """The request left and no usable answer came back: it may have been billed.

    Any HTTP status that is not a clear refusal (``REFUSED_STATUSES``) or a
    rate limit — a 5xx, a 408, a gateway's 524 — a timeout or a dropped
    connection while waiting for the answer, an answer that does not read. Nothing tells this
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
    settings: dict[str, Any] | None = None,
    model: str = MODEL,
) -> dict[str, Any]:
    """Sends the request once, and again only where it was not processed.

    Retried: a 429 (OpenRouter's rate limit, before routing) and a failure
    before the request was sent whole (``urllib`` raises ``URLError`` only
    from connecting and sending; anything after that comes as the bare
    error). A clear refusal (``REFUSED_STATUSES``) is a plain error; everything
    else that follows the send is ``SentUnanswered``.
    """
    api_key = os.environ.get("OPENROUTER_API_KEY")
    if not api_key:
        raise Refused(
            "No OpenRouter key reached the listening step; nothing was sent. Add one in "
            "AI settings \u2192 OpenRouter, then start again.",
            kind="no_key", sent=False)
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
            said = _provider_message(error) if code in REFUSED_STATUSES else None
            error.close()
            if code in TRANSIENT_STATUSES and attempt < max_attempts:
                sleeper(_retry_delay(headers, attempt, now))
                continue
            if code in TRANSIENT_STATUSES:
                raise Refused(
                    f"OpenRouter rate-limited the request {attempt} times (HTTP {code}); "
                    "nothing was billed. Wait a minute, then start again.",
                    kind="rate_limited", status=code) from error
            if code in REFUSED_STATUSES:
                kind, message = refusal_message(code, settings, model, said)
                raise Refused(message, kind=kind, status=code, detail=said) from error
            raise SentUnanswered(
                f"OpenRouter answered HTTP {code} after the request was sent") from error
        except urllib.error.URLError as error:
            # Connecting or sending failed: the request never reached
            # OpenRouter whole, so nothing can have been processed.
            last_error = error
            if attempt == max_attempts:
                reason = " ".join(str(error.reason).split())
                raise Refused(
                    bounded("Could not reach OpenRouter (", reason, "); nothing was sent.",
                            " Check the connection, then start again."),
                    kind="unreachable", detail=reason, sent=False) from error
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


def completion_failure(response: dict[str, Any], model: str = MODEL) -> str | None:
    """Why this billed response is not an answer, or None when it may be one.

    The three shapes the benchmark met: a completion cut off at the token
    limit (``finish_reason: length``, 65 536 tokens of whitespace), a response
    billed with null content, and an empty one. Only ``stop`` is a finished
    answer; anything else (``content_filter``, ``error``, a missing reason) is
    named rather than parsed, because a half answer can still be valid JSON.
    """
    finish = _choice(response).get("finish_reason")
    content = _raw_content(response)
    name = model_label(model)
    if finish == "length":
        return f"{name}'s answer was cut off at the {max_completion_tokens(model)}-token cap"
    if content is None:
        return f"{name} returned no answer text"
    if not content.strip():
        return f"{name} returned an empty answer"
    if finish != "stop":
        label = finish if isinstance(finish, str) and finish else "none"
        return f"{name}'s answer did not finish (finish reason: {label[:24]})"
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


# The record written **before** a request leaves (defect J1-3, 2026-11-16).
#
# Cancel, a closed window and the parent's 40-minute deadline kill this process
# group; nothing else on disk would say that a request was in flight, the user
# read "Nothing was applied", and the next Start could pay again without a word.
# So: a durable record before the POST, removed when the call ends in an answer
# (the cache) or in a receipt (which says what it cost); one still there is a
# request that was sent and never answered. Possible charges a new attempt
# replaces are carried forward in its record and receipt (`earlier`), and
# cleared only when an answer is in hand (docs/TRAPS.md, "Replacing a record of
# possible spending erases a possible charge").
#
# One record per attempt (J3-1, 2026-11-16). The J1 record had one name per
# cache, and an attempt learned which record was "its own" by `stat` after
# publishing it: two helpers on one job folder, and A could adopt B's record
# and remove it when A ended, leaving B's request in flight with no word on
# disk. Now each attempt writes its own name (`<cache>.sent-<attempt>.json`,
# the attempt's identity in the name and in the record), holds an exclusive
# `flock` on it from before it is published until it is removed, and removes
# only that name. Another attempt tells a record in flight (locked) from one
# a killed helper left (the kernel released its lock) by trying the lock.
SENT_RECORD_SCHEMA = "musializer.semantic-request-sent/v2"
# v1 (J1, one name per cache) is still read: a helper killed under that build
# left one.
READABLE_SENT_RECORD_SCHEMAS = frozenset(("musializer.semantic-request-sent/v1",
                                          SENT_RECORD_SCHEMA))
EARLIER_KEEP = 16
ATTEMPT_PATTERN = re.compile(r"[0-9]{1,20}-[0-9a-f]{8}")


def sent_record_path(cache_path: Path, attempt: str | None = None) -> Path:
    """An attempt's record, or (no attempt) the one name the J1 build used."""
    if attempt is None:
        return cache_path.with_name(f"{cache_path.stem}.sent.json")
    return cache_path.with_name(f"{cache_path.stem}.sent-{attempt}.json")


def sent_record_paths(cache_path: Path) -> list[Path]:
    """Every record beside this cache, oldest attempt first, the J1 name
    first of all."""
    found: list[tuple[int, Path]] = []
    legacy = sent_record_path(cache_path)
    if legacy.exists():
        found.append((-1, legacy))
    prefix = f"{cache_path.stem}.sent-"
    try:
        names = os.listdir(cache_path.parent)
    except OSError:
        names = []
    for name in names:
        if not (name.startswith(prefix) and name.endswith(".json")):
            continue
        attempt = name[len(prefix):-len(".json")]
        if ATTEMPT_PATTERN.fullmatch(attempt):
            found.append((int(attempt.split("-")[0]), cache_path.with_name(name)))
    return [path for _, path in sorted(found)]


def _read_small_json(path: Path) -> Any:
    try:
        with path.open("rb") as handle:
            if os.fstat(handle.fileno()).st_size > 1 << 20:
                return None
            return json.loads(handle.read().decode("utf-8"))
    except (OSError, ValueError):
        return None


def _sync_directory(path: Path) -> None:
    try:
        directory = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(directory)
    except OSError:
        pass
    finally:
        os.close(directory)


@dataclass
class DeadRecord:
    """A record no helper holds any more, read through its own handle."""
    path: Path
    identity: tuple[int, int]
    record: dict[str, Any]


def _dead_records(cache_path: Path) -> list[DeadRecord]:
    """The records beside this cache whose attempt is over: no live helper
    holds their lock. A record still locked is a request in flight now and is
    neither read as an earlier charge nor removed. A record that cannot be
    read is left where it is (docs/TRAPS.md, "A record that cannot be read is
    not an empty record")."""
    dead: list[DeadRecord] = []
    for path in sent_record_paths(cache_path):
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError:
            continue
        with os.fdopen(fd, "rb") as handle:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                continue
            try:
                seen = os.fstat(fd)
                if seen.st_size > 1 << 20:
                    continue
                record = json.loads(handle.read().decode("utf-8"))
            except (OSError, ValueError):
                continue
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
        if isinstance(record, dict) and record.get("schema_version") in READABLE_SENT_RECORD_SCHEMAS:
            dead.append(DeadRecord(path, (seen.st_ino, seen.st_dev), record))
    return dead


def request_key(audio_sha256: Any, settings: Any) -> str | None:
    """The request's identity as one digest: the cache key the answer to it
    would carry (`canonical_sha256({"audio_sha256": ..., **settings})`), and
    what `semantic-status` names for the next Start (J3-2)."""
    if not isinstance(audio_sha256, str) or not audio_sha256 or not isinstance(settings, dict):
        return None
    return canonical_sha256({"audio_sha256": audio_sha256, **settings})


def _charge_what(record: dict[str, Any]) -> dict[str, Any]:
    """What a possible charge was for (J3-2): the audio, the request and the
    model, so the next confirmation can say whether it was this track and
    these settings, or something else that used the folder."""
    request = record.get("request") if isinstance(record.get("request"), dict) else None
    return {
        "audio_sha256": record.get("audio_sha256"),
        "request_key": record.get("request_key") or request_key(record.get("audio_sha256"),
                                                                 request),
        "model": record.get("model") or (request or {}).get("model"),
    }


def _charge_of_record(record: dict[str, Any]) -> dict[str, Any]:
    return {"kind": "unanswered", "at_utc": record.get("sent_utc"),
            "at_ns": record.get("sent_ns"), "usd_max": record.get("usd_max"),
            **_charge_what(record)}


def earlier_possible_charges(cache_path: Path, dead: list[DeadRecord] | None = None,
                             ) -> list[dict[str, Any]]:
    """Every attempt beside this cache that may have been billed and is not
    yet answered: a record a killed helper left, a receipt that says "may have
    been billed", and what each of them carried forward."""
    found: list[dict[str, Any]] = []
    for entry in _dead_records(cache_path) if dead is None else dead:
        found.extend(item for item in entry.record.get("earlier") or [] if isinstance(item, dict))
        found.append(_charge_of_record(entry.record))
    receipt = _read_small_json(failure_receipt_path(cache_path))
    if isinstance(receipt, dict) and receipt.get("schema_version") in READABLE_FAILED_RUN_SCHEMAS:
        found.extend(entry for entry in receipt.get("earlier") or [] if isinstance(entry, dict))
        if receipt.get("may_be_billed"):
            ceiling = (receipt.get("estimate") or {}).get("usd_max")
            found.append({"kind": "may_be_billed", "at_utc": receipt.get("failed_utc"),
                          "at_ns": receipt.get("failed_ns"),
                          "usd_max": (ceil_usd(ceiling, 3)
                                      if isinstance(ceiling, (int, float)) else None),
                          **_charge_what(receipt)})
    unique: dict[tuple[Any, Any], dict[str, Any]] = {}
    for entry in found:
        unique[(entry.get("kind"), entry.get("at_ns"))] = entry
    ordered = sorted(unique.values(), key=lambda entry: entry.get("at_ns") or 0)
    return ordered[-EARLIER_KEEP:]


def _remove_dead(dead: list[DeadRecord]) -> None:
    """Removes records whose charges this attempt now carries, each only
    while its name still holds the file read and no helper holds it."""
    for entry in dead:
        try:
            fd = os.open(entry.path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError:
            continue
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            continue
        try:
            seen = os.fstat(fd)
            if (seen.st_ino, seen.st_dev) == entry.identity:
                entry.path.unlink(missing_ok=True)
        except OSError:
            # Left in place, it is read again next time: counted twice at
            # worst, never lost.
            pass
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)


@dataclass
class SentRecord:
    """This attempt's record and the open handle whose lock says it is in
    flight. The lock goes with the process: a kill releases it."""
    path: Path
    handle: Any
    attempt: str


def _write_sent_record(cache_path: Path, *, settings: dict[str, Any], audio_sha256: str,
                       estimate: dict[str, Any], earlier: list[dict[str, Any]]) -> SentRecord:
    """Durably, before the request leaves, under this attempt's own name and
    locked before it is published, so no other attempt ever sees it unlocked
    while this one lives."""
    attempt = f"{time.time_ns()}-{os.urandom(4).hex()}"
    path = sent_record_path(cache_path, attempt)
    ceiling = estimate.get("usd_max")
    payload = json.dumps({
        "schema_version": SENT_RECORD_SCHEMA,
        "attempt": attempt,
        "sent_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "sent_ns": time.time_ns(),
        "audio_sha256": audio_sha256,
        "request": settings,
        # What it is about, for the next confirmation's words (J3-2).
        "request_key": request_key(audio_sha256, settings),
        "model": settings.get("model") or MODEL,
        "model_label": model_label(str(settings.get("model") or MODEL)),
        # The most it can cost, rounded up at the precision every sentence
        # shows it (`ceil_usd`).
        "usd_max": ceil_usd(ceiling, 3) if isinstance(ceiling, (int, float)) else None,
        "estimate": estimate,
        "pid": os.getpid(),
        "earlier": earlier,
    }, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8") + b"\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    handle = os.fdopen(fd, "wb")
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        handle.write(payload)
        handle.flush()
        os.fsync(fd)
        os.replace(temporary, path)
        _sync_directory(path.parent)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        handle.close()
        raise
    return SentRecord(path, handle, attempt)


def _drop_sent_record(record: SentRecord | None) -> None:
    """Removes this attempt's record (its own name, nobody else's), then gives
    the lock back before the handle is closed."""
    if record is None or record.handle.closed:
        return
    try:
        try:
            now = os.stat(record.path)
            mine = os.fstat(record.handle.fileno())
        except OSError:
            return
        if (now.st_ino, now.st_dev) == (mine.st_ino, mine.st_dev):
            record.path.unlink(missing_ok=True)
            _sync_directory(record.path.parent)
    finally:
        try:
            fcntl.flock(record.handle.fileno(), fcntl.LOCK_UN)
        finally:
            record.handle.close()


AUDIO_CHANGED_REASON = ("the track's audio file changed on disk during the job, so nothing "
                        "was sent. Open the file again, then start")


def failure_receipt_path(cache_path: Path) -> Path:
    return cache_path.with_name(f"{cache_path.stem}.failed.json")


def receipt_time_ns(receipt: dict[str, Any]) -> int | None:
    """When a failure receipt's attempt failed, in nanoseconds since the epoch.

    ``failed_ns`` since H4d; an older receipt has only ``failed_utc``, to the
    second, and is placed at the end of that second, so a tie reads as "the
    failure came after", which keeps its warning (the safe side).
    """
    value = receipt.get("failed_ns")
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    text = receipt.get("failed_utc")
    if isinstance(text, str):
        try:
            seconds = calendar.timegm(time.strptime(text, "%Y-%m-%dT%H:%M:%SZ"))
        except ValueError:
            return None
        return seconds * 1_000_000_000 + 999_999_999
    return None


def answer_time_ns(envelope: dict[str, Any]) -> int | None:
    """When a cached answer was written (``answered_ns``, H4d), or None for a
    cache written before that."""
    value = envelope.get("answered_ns") if isinstance(envelope, dict) else None
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    return None


def receipt_applies(receipt: dict[str, Any], envelope: dict[str, Any]) -> bool:
    """Whether a failure receipt describes an attempt that failed **after**
    the cached answer was made (H4d). Two helpers on one cache: one fails and
    writes its receipt, the other then succeeds; the receipt is older than the
    answer and does not describe it. A cache or a receipt with no time keeps
    the warning."""
    answered = answer_time_ns(envelope)
    failed = receipt_time_ns(receipt)
    return answered is None or failed is None or failed > answered


def _receipt_identity(cache_path: Path) -> tuple[int, int] | None:
    try:
        seen = os.stat(failure_receipt_path(cache_path))
    except OSError:
        return None
    return seen.st_ino, seen.st_dev


def _drop_receipt_if(cache_path: Path, identity: tuple[int, int] | None) -> None:
    """Removes the receipt only while it is still the one read at the start
    of this attempt: a receipt written since (another helper's failure, by a
    rename, so another inode) stays."""
    if identity is not None and _receipt_identity(cache_path) == identity:
        try:
            failure_receipt_path(cache_path).unlink(missing_ok=True)
        except OSError:
            pass


def _drop_older_receipt(cache_path: Path, answered_ns: int) -> None:
    """Removes the failure receipt beside a cache just written, if it failed
    before this answer was made; a receipt of a later failure stays.

    Read by its handle and removed only while the name still points at the
    file read (a newer receipt is written by a rename, so it is another
    inode): a failure that lands between the read and the removal keeps its
    receipt.
    """
    path = failure_receipt_path(cache_path)
    try:
        with path.open("rb") as handle:
            seen = os.fstat(handle.fileno())
            if seen.st_size > 1 << 20:
                return
            receipt = json.loads(handle.read().decode("utf-8"))
    except (OSError, ValueError):
        return
    if not isinstance(receipt, dict):
        return
    failed = receipt_time_ns(receipt)
    if failed is None or failed >= answered_ns:
        return
    try:
        now = os.stat(path)
    except OSError:
        return
    if (now.st_ino, now.st_dev) == (seen.st_ino, seen.st_dev):
        path.unlink(missing_ok=True)


def failure_message(reason: str, response: dict[str, Any] | None,
                    estimate: dict[str, Any] | None = None) -> str:
    """The reason, then what was billed: the reason is shortened, never the
    bill, so the amount is always inside the 160 bytes the panel keeps."""
    return bounded("", reason,
                   f". Nothing was saved; {_cost_clause(response or {}, estimate)}.")


def unanswered_message(reason: str, estimate: dict[str, Any] | None = None) -> str:
    """What the person reads when the request left and no answer came back.

    Under the 160 bytes the application's diagnosis keeps, reason first.
    """
    ceiling = (estimate or {}).get("usd_max")
    bound = (f" (at most {ceil_usd(ceiling, 3):.3f} USD)"
             if isinstance(ceiling, (int, float)) else "")
    return bounded("", reason, f"; it may have been billed{bound}. Nothing was saved or sent again.")


def _write_failure_receipt(
    cache_path: Path, *, reason: str, response: dict[str, Any] | None,
    settings: dict[str, Any], audio_sha256: str, detail: str | None = None,
    estimate: dict[str, Any] | None = None, message: str | None = None,
    may_be_billed: bool = False, kind: str | None = None,
    earlier: list[dict[str, Any]] | None = None,
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
        # The same, to the nanosecond: a receipt applies only to a cached
        # answer older than it (H4d), and two helpers can end in one second.
        "failed_ns": time.time_ns(),
        "reason": reason,
        # Which failure it was, for the panel's headline: `key_rejected`,
        # `no_credit`, `no_endpoint`, `refused`, `rate_limited`, `no_key`,
        # `unreachable`, `not_sent`, `unanswered` or `unusable_answer`.
        "kind": kind or ("unanswered" if may_be_billed else
                         "unusable_answer" if response else "not_sent"),
        "message": message or failure_message(reason, response, estimate),
        # The request left and nothing says whether the model ran: counted as
        # possibly billed, up to the estimate's maximum.
        "may_be_billed": may_be_billed,
        "detail": detail,
        "audio_sha256": audio_sha256,
        "request": settings,
        "request_key": request_key(audio_sha256, settings),
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
        # Earlier attempts that may have been billed, carried forward from the
        # record and receipt this attempt replaced (J1-3).
        "earlier": earlier or [],
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
        raise AnalysisValidationError("the completion is not valid JSON") from error


@dataclass
class _Attempt:
    """What one attempt has done so far, for the receipt an unexpected
    failure still owes (J3-6): whether it wrote its own, whether its request
    left, what came back."""
    audio_hash: str | None
    settings: dict[str, Any] | None = None
    estimate: dict[str, Any] | None = None
    earlier: list[dict[str, Any]] | None = None
    sent: SentRecord | None = None
    response: dict[str, Any] | None = None
    explained: bool = False


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
    encoder: Callable[..., bytes] = encode_audio,
    request_dump: Path | None = None,
    expected_sha256: str | None = None,
    _audio_bytes: bytes | None = None,
    _prepared_request: tuple[dict[str, Any], dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """One listening call for ``audio_path``, cached at ``cache_path``.

    Every way this ends in failure leaves a receipt beside the cache (J1-2,
    J3-6), because a child's own words die at the parent's ``_run`` ("python3
    exited with code 1"): the named failures write theirs where they happen,
    and anything else (a file removed before it was read, a disk that refuses
    the cache after the answer was billed) is written here, saying whether the
    request had left.
    """
    expected = (expected_sha256 or "").strip().lower() or None
    state = _Attempt(audio_hash=expected)
    try:
        return _analyze_to_cache(
            state, audio_path, cache_path, audio_duration=audio_duration,
            audio_format=audio_format, model=model, provider_order=provider_order,
            provider_only=provider_only, provider_ignore=provider_ignore,
            allow_fallbacks=allow_fallbacks, zero_data_retention=zero_data_retention,
            max_price=max_price, transport=transport, sleeper=sleeper, encoder=encoder,
            request_dump=request_dump, expected_sha256=expected_sha256,
            _audio_bytes=_audio_bytes, _prepared_request=_prepared_request,
        )
    except Exception as error:  # noqa: BLE001 - re-raised; only the receipt is added
        if not state.explained:
            fallback: dict[str, Any]
            try:
                fallback = request_settings(
                    model=model, audio_duration=audio_duration,
                    audio_format=(audio_format or audio_path.suffix.lstrip(".")).lower(),
                    provider_order=provider_order, provider_only=provider_only,
                    provider_ignore=provider_ignore, allow_fallbacks=allow_fallbacks,
                    zero_data_retention=zero_data_retention, max_price=max_price)
            except (ValueError, TypeError):
                fallback = {"model": model}
            _receipt_for_unexpected(cache_path, state, error, fallback)
        raise


def _receipt_for_unexpected(cache_path: Path, state: _Attempt, error: Exception,
                            fallback_settings: dict[str, Any]) -> None:
    """The receipt a failure nobody named still owes, by how far the attempt
    got: before the request left (nothing billed), after it left with no
    answer (may have been billed), or with an answer in hand (billed)."""
    reason = " ".join(str(error).split()) or type(error).__name__
    if isinstance(error, OSError) and error.strerror:
        reason = error.strerror
    settings = state.settings or fallback_settings
    try:
        if state.sent is not None and state.response is not None:
            _write_failure_receipt(
                cache_path, reason=f"The answer could not be saved ({reason})",
                response=state.response, settings=settings, audio_sha256=state.audio_hash,
                estimate=state.estimate, earlier=state.earlier)
        elif state.sent is not None:
            _write_failure_receipt(
                cache_path, reason=reason, response=None, settings=settings,
                audio_sha256=state.audio_hash, estimate=state.estimate,
                message=unanswered_message(f"The listening step failed ({reason})", state.estimate),
                may_be_billed=True, kind="unanswered", earlier=state.earlier)
        else:
            unread = state.settings is None and isinstance(error, OSError)
            message = (bounded("Could not read the track's audio file (", reason,
                               "); nothing was sent.",
                               " Check that the file is still there, then start again.")
                       if unread else
                       bounded("The listening step stopped before sending (", reason,
                               "); nothing was sent."))
            _write_failure_receipt(
                cache_path, reason=reason, response=None, settings=settings,
                audio_sha256=state.audio_hash, message=message, may_be_billed=False,
                kind="not_sent", earlier=state.earlier)
    except OSError:
        # The folder refuses even the receipt: the record, if any, still says
        # the request may have left.
        return
    # The receipt now says how this attempt ended.
    _drop_sent_record(state.sent)


def _analyze_to_cache(
    state: "_Attempt",
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
    encoder: Callable[..., bytes] = encode_audio,
    request_dump: Path | None = None,
    expected_sha256: str | None = None,
    _audio_bytes: bytes | None = None,
    _prepared_request: tuple[dict[str, Any], dict[str, Any]] | None = None,
) -> dict[str, Any]:
    audio_bytes = _audio_bytes if _audio_bytes is not None else audio_path.read_bytes()
    audio_hash = hashlib.sha256(audio_bytes).hexdigest()
    expected = (expected_sha256 or "").strip().lower() or None
    state.audio_hash = expected if expected is not None else audio_hash
    selected_format = (audio_format or audio_path.suffix.lstrip(".")).lower()
    if not selected_format:
        raise AnalysisValidationError("audio format is required")
    # What earlier attempts may have cost: it rides forward in whatever this
    # attempt writes (J1-3). The records and the receipt it was read from are
    # removed only once this attempt's own record or receipt carries it (a
    # kill in between must not lose it), and a record another helper still
    # holds is a request in flight, not an earlier charge (J3-1).
    dead = _dead_records(cache_path)
    earlier = earlier_possible_charges(cache_path, dead)
    earlier_receipt = _receipt_identity(cache_path)
    state.earlier = earlier
    sent: SentRecord | None = None

    def not_sent(reason: str, settings: dict[str, Any]) -> RuntimeError:
        """A run that stopped before anything left: its receipt says so, and
        that nothing can have been billed (the parent reads the sentence from
        the receipt, since a child's own words die at `_run`)."""
        message = bounded("", f"{reason[:1].upper()}{reason[1:]}", ".")
        state.explained = True
        _write_failure_receipt(
            cache_path, reason=reason, response=None, settings=settings,
            audio_sha256=audio_hash, message=message, may_be_billed=False,
            kind="not_sent", earlier=earlier)
        return RuntimeError(message)

    # The parent hashed the file when the job started; this snapshot is read
    # minutes later, after the measured step. A file replaced in between would
    # be sent and paid for under a hash the parent then refuses, so it is
    # refused here, before anything leaves. The receipt names the job's hash,
    # which is the one the parent looks it up by.
    if expected is not None and audio_hash != expected:
        reason = AUDIO_CHANGED_REASON
        audio_hash = expected
        raise not_sent(reason, request_settings(
            model=model, audio_duration=audio_duration,
            audio_format=selected_format, provider_order=provider_order,
            provider_only=provider_only, provider_ignore=provider_ignore,
            allow_fallbacks=allow_fallbacks,
            zero_data_retention=zero_data_retention, max_price=max_price))

    if _prepared_request is None:
        try:
            sent_bytes = prepare_audio(audio_bytes, selected_format, model,
                                       encoder=encoder, workdir=cache_path.parent)
        except RuntimeError as error:
            raise not_sent(str(error), request_settings(
                model=model, audio_duration=audio_duration,
                audio_format=selected_format, provider_order=provider_order,
                provider_only=provider_only, provider_ignore=provider_ignore,
                allow_fallbacks=allow_fallbacks,
                zero_data_retention=zero_data_retention,
                max_price=max_price)) from error
        payload, settings = build_request(
            sent_bytes,
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
    state.settings = settings
    if request_dump is not None:
        atomic_write_json(request_dump, redacted_request_dump(payload, settings, audio_hash))
    requested = str(settings.get("model") or model)
    oversize = oversize_reason(payload, requested)
    if oversize is not None:
        raise not_sent(oversize, settings)
    estimate = estimate_cost(requested, audio_duration)
    state.estimate = estimate
    # The last thing before the request leaves: from here a kill leaves this
    # record, which is the only word on disk that the request was sent.
    sent = _write_sent_record(cache_path, settings=settings, audio_sha256=audio_hash,
                              estimate=estimate, earlier=earlier)
    state.sent = sent
    # This record now carries what the earlier ones said: they are done with.
    # A receipt from an earlier attempt describes that attempt, not this one.
    _remove_dead(dead)
    _drop_receipt_if(cache_path, earlier_receipt)
    try:
        response = submit_request(payload, transport=transport, sleeper=sleeper,
                                  settings=settings, model=requested)
    except Refused as error:
        state.explained = True
        # Refused before a model ran, or never sent: nothing billed, and the
        # sentence says why and what to do. A receipt, because a child's own
        # words die at `_run` ("python3 exited with code 1").
        _write_failure_receipt(
            cache_path, reason=str(error), response=None, settings=settings,
            audio_sha256=audio_hash, estimate=estimate, message=str(error),
            may_be_billed=False, kind=error.kind,
            detail=error.detail, earlier=earlier)
        _drop_sent_record(sent)
        raise
    except SentUnanswered as error:
        state.explained = True
        message = unanswered_message(str(error), estimate)
        _write_failure_receipt(
            cache_path, reason=str(error), response=None, settings=settings,
            audio_sha256=audio_hash, estimate=estimate, message=message,
            may_be_billed=True,
            kind="timed_out" if isinstance(error.__cause__, TimeoutError) else "unanswered",
            earlier=earlier)
        _drop_sent_record(sent)
        raise FailedRun(message) from error

    state.response = response

    def failed(reason: str, detail: str | None = None) -> FailedRun:
        state.explained = True
        _write_failure_receipt(
            cache_path, reason=reason, response=response, settings=settings,
            audio_sha256=audio_hash, detail=detail, estimate=estimate, earlier=earlier)
        # The receipt now says what this call cost; the record is answered.
        _drop_sent_record(sent)
        return FailedRun(failure_message(reason, response, estimate))

    name = model_label(requested)
    reason = completion_failure(response, requested)
    if reason is not None:
        raise failed(reason)
    try:
        score = decode_model_score(response)
    except AnalysisValidationError as error:
        raise failed(f"{name}'s answer is not valid JSON", str(error)) from error
    try:
        normalized = normalize_semantic_score(
            score,
            audio_sha256=audio_hash,
            audio_duration=audio_duration,
            request_settings=settings,
            response_metadata=response,
            requested_model=requested,
            cost_estimate=estimate,
        )
    except AnalysisValidationError as error:
        detail = str(error)
        raise failed(f"{name}'s answer does not fit the schema ({detail[:60]})",
                     detail) from error
    # What the bridge requires, checked before anything is cached (J1-4): an
    # answer that cannot be used is a failed run with its cost, never a cache
    # the next run reuses and fails on again.
    repair_counts: dict[str, int] = {}
    try:
        normalized, repairs = bridge_fit(normalized, repair_counts)
    except AnalysisValidationError as error:
        detail = str(error)
        raise failed(f"{name}'s answer cannot be used: {detail}", detail) from error
    if repairs:
        normalized.setdefault("provenance", {})["repairs"] = repairs
        normalized["provenance"]["repair_counts"] = repair_counts
    cache_key = canonical_sha256({"audio_sha256": audio_hash, **settings})
    answered_ns = time.time_ns()
    envelope = {
        "schema_version": CACHE_SCHEMA_VERSION,
        "cache_key": cache_key,
        "request": {"audio_sha256": audio_hash, **settings},
        "raw_response": response,
        "normalized": normalized,
        # When this answer was made: a failure receipt older than it (another
        # helper's, on the same cache) does not describe it (H4d).
        "answered_ns": answered_ns,
    }
    atomic_write_json(cache_path, envelope)
    # The answer is in hand: the record of this request, and the earlier
    # possible charges it carried for the same cache, are done with.
    _drop_sent_record(sent)
    _drop_older_receipt(cache_path, answered_ns)
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
    parser.add_argument("--expect-sha256",
                        help="refuse, before sending, audio that does not hash to this")
    args = parser.parse_args(argv)
    if not args.dry_run and args.cache is None:
        parser.error("cache is required unless only inspecting help")
    try:
        audio_format = (args.audio_format or args.audio.suffix.lstrip(".")).lower()
        max_price = {
            name: value for name, value in (
                ("prompt", args.max_price_prompt),
                ("completion", args.max_price_completion),
                ("audio", args.max_price_audio),
            ) if value is not None
        }
        if args.dry_run:
            audio_bytes, audio_hash = _snapshot_audio(args.audio)
            payload, settings = build_request(
                prepare_audio(audio_bytes, audio_format, args.model),
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
            dump = redacted_request_dump(payload, settings, audio_hash)
            if args.request_dump:
                atomic_write_json(args.request_dump, dump)
            else:
                print(json.dumps(dump, ensure_ascii=False, indent=2))
            return 0
        # The audio is read and prepared (encoded, for a model that needs it)
        # inside, where every failure leaves the receipt the parent reads
        # (J3-6: a file removed or made unreadable before it was read here
        # printed its error and the panel read "python3 exited with code 1").
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
            request_dump=args.request_dump,
            expected_sha256=args.expect_sha256,
        )
    except Exception as error:  # noqa: BLE001 - the one line the log keeps; the receipt is written
        # The receipt was written inside; this is the line the job log keeps.
        print(f"Semantic analysis failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
