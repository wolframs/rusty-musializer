#!/usr/bin/env python3
"""Normalize common Whisper JSON into Musializer's lyric timing sidecar."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any

from analysis_io import (
    ADAPTER_VERSION,
    AnalysisValidationError,
    audio_hash,
    atomic_write_json,
    clamp,
    duration,
    number,
    read_json,
    sha256_file,
    text,
)


SCHEMA_VERSION = "musializer.lyric-timing/v1"


def _confidence(item: dict[str, Any]) -> float | None:
    candidate = item.get("confidence", item.get("probability", item.get("p")))
    if candidate is None:
        return None
    return clamp(candidate, 0.0, 1.0, "confidence")


def _seconds(item: dict[str, Any], field: str) -> tuple[Any, Any]:
    if "startMs" in item or "endMs" in item:
        return (
            number(item.get("startMs"), f"{field}.startMs") / 1000.0,
            number(item.get("endMs"), f"{field}.endMs") / 1000.0,
        )
    offsets = item.get("offsets")
    if offsets is not None:
        if not isinstance(offsets, dict):
            raise AnalysisValidationError(f"{field}.offsets must be an object")
        return (
            number(offsets.get("from"), f"{field}.offsets.from") / 1000.0,
            number(offsets.get("to"), f"{field}.offsets.to") / 1000.0,
        )
    return (
        item.get("start", item.get("start_seconds")),
        item.get("end", item.get("end_seconds")),
    )


def _timed_text(
    item: dict[str, Any], audio_duration: float, field: str, corrected: bool = False
) -> dict[str, Any] | None:
    if not isinstance(item, dict):
        raise AnalysisValidationError(f"{field} entry must be an object")
    raw_start, raw_end = _seconds(item, field)
    start = clamp(raw_start, 0, audio_duration, f"{field}.start")
    end = clamp(raw_end, 0, audio_duration, f"{field}.end")
    value = text(item.get("word", item.get("text", "")), f"{field}.text")
    if not value or end <= start:
        return None
    return {
        "start_seconds": start,
        "end_seconds": end,
        "text": value,
        "confidence": _confidence(item),
        "corrected": bool(item.get("corrected", corrected)),
    }


# whisper.cpp emits controls such as `[_BEG_]` *and* timestamp tokens such as
# `[_TT_700]`. The previous pattern required an underscore immediately before
# `]`, so every timestamp token was joined onto the segment's final word and
# stretched that word to the coarse segment boundary.
_SPECIAL_TOKEN = re.compile(r"^\[_.*\]$")
# whisper.cpp reports experimental DTW time as one centisecond-resolution
# acoustic moment per decoded token, not as an interval. A small symmetric pad
# turns the first/last moments of a BPE word into a usable evidence window when
# a caller explicitly opts into DTW. The normal singing path uses heuristic
# token *onsets*: its interval ends routinely stretch through long instrumental
# gaps, while onsets stayed close to the audible words in the real-track audit.
_DTW_TOKEN_PAD_SECONDS = 0.10
_TOKEN_TAIL_MAX_SECONDS = 0.50


def _aggregate_remotion_tokens(
    tokens: Any, audio_duration: float, field: str, *, prefer_dtw: bool = False,
) -> list[dict[str, Any]]:
    """Join whisper.cpp BPE tokens into display words.

    whisper.cpp marks most new lexical words with leading whitespace. Tokens
    without it (word pieces and punctuation) extend the current word. Special
    timestamp/control tokens are excluded.
    """

    if not isinstance(tokens, list):
        raise AnalysisValidationError(f"{field} must be an array")
    groups: list[list[dict[str, Any]]] = []
    current: list[dict[str, Any]] = []
    for index, token_item in enumerate(tokens):
        token_field = f"{field}[{index}]"
        if not isinstance(token_item, dict):
            raise AnalysisValidationError(f"{token_field} must be an object")
        token_text = text(token_item.get("text", ""), f"{token_field}.text")
        if not token_text or _SPECIAL_TOKEN.fullmatch(token_text):
            continue
        raw_start, raw_end = _seconds(token_item, token_field)
        start = clamp(raw_start, 0, audio_duration, f"{token_field}.start")
        end = clamp(raw_end, 0, audio_duration, f"{token_field}.end")
        # Token heuristics sometimes report an end before the onset (especially
        # at a segment boundary). The onset and token text remain useful; give
        # that token a minimal provisional interval instead of erasing it.
        if end <= start:
            end = min(audio_duration, start + 0.01)
        if end <= start:
            continue
        normalized = {
            "start_seconds": start,
            "end_seconds": end,
            "text": token_text.strip(),
            "confidence": _confidence(token_item),
            "corrected": False,
        }
        raw_dtw = token_item.get("t_dtw")
        if (isinstance(raw_dtw, (int, float)) and not isinstance(raw_dtw, bool)
                and raw_dtw >= 0):
            normalized["_dtw_center_seconds"] = clamp(
                raw_dtw / 100.0, 0.0, audio_duration,
                f"{token_field}.t_dtw")
        starts_word = bool(str(token_item.get("text", ""))[:1].isspace())
        if current and starts_word:
            groups.append(current)
            current = []
        current.append(normalized)
    if current:
        groups.append(current)

    words: list[dict[str, Any]] = []
    for group in groups:
        confidences = [part["confidence"] for part in group if part["confidence"] is not None]
        confidence = sum(confidences) / len(confidences) if confidences else None
        centers = [part["_dtw_center_seconds"] for part in group
                   if "_dtw_center_seconds" in part]
        if prefer_dtw and centers:
            start = max(0.0, min(centers) - _DTW_TOKEN_PAD_SECONDS)
            end = min(audio_duration, max(centers) + _DTW_TOKEN_PAD_SECONDS)
        else:
            onsets = [part["start_seconds"] for part in group]
            start = min(onsets)
            raw_end = max(part["end_seconds"] for part in group)
            end = min(raw_end, max(onsets) + _TOKEN_TAIL_MAX_SECONDS)
        combined = {
            "start_seconds": start,
            "end_seconds": end,
            "text": "".join(part["text"] for part in group).strip(),
            "confidence": confidence,
            "corrected": False,
        }
        if combined["text"] and combined["end_seconds"] > combined["start_seconds"]:
            words.append(combined)
    return words


def normalize_whisper(
    raw: Any,
    *,
    audio_sha256: str,
    audio_duration: float,
    model: str = "unknown",
    corrected_lines: list[dict[str, Any]] | None = None,
    prefer_dtw: bool = False,
) -> dict[str, Any]:
    audio_duration = duration(audio_duration)
    audio_sha256 = audio_hash(audio_sha256)
    if not isinstance(raw, (dict, list)):
        raise AnalysisValidationError("Whisper input must be a JSON object or caption array")

    # Remotion's generated caption JSON is an array of startMs/endMs entries.
    # When used as the primary input it is treated as word/token evidence;
    # authoritative line captions belong in corrected_lines.
    if isinstance(raw, list):
        segments: list[Any] = []
        caption_words = raw
        transcription: list[Any] = []
    else:
        segments = raw.get("segments", [])
        caption_words = raw.get("words", [])
        transcription = raw.get("transcription", [])
    if not isinstance(segments, list):
        raise AnalysisValidationError("segments must be an array")
    if not isinstance(caption_words, list):
        raise AnalysisValidationError("words must be an array")
    if not isinstance(transcription, list):
        raise AnalysisValidationError("transcription must be an array")

    words: list[dict[str, Any]] = []
    lines: list[dict[str, Any]] = []
    for index, segment in enumerate(segments):
        line = _timed_text(segment, audio_duration, f"segments[{index}]")
        if line:
            lines.append(line)
        segment_words = segment.get("words", []) if isinstance(segment, dict) else []
        if not isinstance(segment_words, list):
            raise AnalysisValidationError(f"segments[{index}].words must be an array")
        for word_index, word in enumerate(segment_words):
            normalized = _timed_text(
                word, audio_duration, f"segments[{index}].words[{word_index}]"
            )
            if normalized:
                words.append(normalized)

    # Some wrappers and Remotion caption exports place timed units at the root.
    if not words:
        for index, word in enumerate(caption_words):
            normalized = _timed_text(word, audio_duration, f"words[{index}]")
            if normalized:
                words.append(normalized)

    # @remotion/install-whisper-cpp preserves whisper.cpp's millisecond
    # offsets and nested BPE tokens in transcription[].
    for index, segment in enumerate(transcription):
        field = f"transcription[{index}]"
        line = _timed_text(segment, audio_duration, field)
        if line:
            lines.append(line)
        if not isinstance(segment, dict):
            raise AnalysisValidationError(f"{field} must be an object")
        words.extend(
            _aggregate_remotion_tokens(
                segment.get("tokens", []), audio_duration, f"{field}.tokens",
                prefer_dtw=prefer_dtw,
            )
        )

    if corrected_lines is not None:
        if not isinstance(corrected_lines, list):
            raise AnalysisValidationError("corrected lines must be a caption array")
        lines = []
        for index, line in enumerate(corrected_lines):
            normalized = _timed_text(
                line, audio_duration, f"corrected_lines[{index}]", corrected=True
            )
            if normalized:
                normalized["corrected"] = True
                lines.append(normalized)

    words.sort(key=lambda item: (item["start_seconds"], item["end_seconds"]))
    lines.sort(key=lambda item: (item["start_seconds"], item["end_seconds"]))
    return {
        "schema_version": SCHEMA_VERSION,
        "lane": "lyrics",
        "audio": {"sha256": audio_sha256, "duration_seconds": audio_duration},
        "provenance": {
            "adapter": "tools/import_whisper.py",
            "adapter_version": ADAPTER_VERSION,
            "source_kind": "whisper_import",
            "audio_sha256": audio_sha256,
            "schema_version": SCHEMA_VERSION,
            "model": model,
            "provider": None,
            "prompt_version": None,
            "request_settings": {},
            "generation": {},
        },
        "words": words,
        "lines": lines,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("whisper_json", type=Path)
    parser.add_argument("audio", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--duration", required=True, type=float)
    parser.add_argument("--model", default="unknown")
    parser.add_argument("--corrected-lines", type=Path)
    parser.add_argument("--prefer-dtw", action="store_true")
    args = parser.parse_args(argv)
    try:
        corrected = read_json(args.corrected_lines) if args.corrected_lines else None
        normalized = normalize_whisper(
            read_json(args.whisper_json),
            audio_sha256=sha256_file(args.audio),
            audio_duration=args.duration,
            model=args.model,
            corrected_lines=corrected,
            prefer_dtw=args.prefer_dtw,
        )
        atomic_write_json(args.output, normalized)
    except (OSError, ValueError) as error:
        print(f"Whisper import failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
