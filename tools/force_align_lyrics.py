#!/usr/bin/env python3
"""Refine lyric cue boundaries with an independent CTC forced aligner.

Whisper supplies text/order evidence, but its decoder timestamps are not an
acoustic word aligner: on produced singing the first token of a segment can be
seconds early. This helper teacher-forces the already-decided cue text through
TorchAudio's multilingual MMS_FA acoustic model. It never changes display text
or cue order. Low-scoring alignments retain their input timing and are marked
uncertain instead of silently replacing one guess with another.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
import unicodedata
from pathlib import Path
from typing import Any, Sequence

from analysis_io import (
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    duration,
    read_json,
    sha256_file,
)


ALIGNMENT_VERSION = "13"
MODEL_ID = "torchaudio.pipelines.MMS_FA"
MAX_CHUNK_SECONDS = 50.0
CHUNK_LEAD_PADDING_SECONDS = 0.75
CHUNK_TAIL_PADDING_SECONDS = 6.0
MINIMUM_ALIGNMENT_SCORE = 0.15
HIGH_INPUT_CONFIDENCE = 0.90
MINIMUM_HIGH_CONFIDENCE_ALIGNMENT_SCORE = 0.05
MAXIMUM_LOW_SCORE_START_SHIFT_SECONDS = 2.0
LARGE_SHIFT_MINIMUM_BOUNDARY_WORD_SCORE = 0.35
MINIMUM_BOUNDARY_WORD_SCORE = 0.02
INPUT_SPAN_TOLERANCE_SECONDS = 0.50
CUE_LEAD_SECONDS = 0.10
CUE_TAIL_SECONDS = 0.15

_SMALL_NUMBERS = (
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
    "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen",
    "sixteen", "seventeen", "eighteen", "nineteen",
)
_TENS = ("", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy",
         "eighty", "ninety")


def _number_words(value: int) -> list[str]:
    if value < 20:
        return [_SMALL_NUMBERS[value]]
    if value < 100:
        tens, unit = divmod(value, 10)
        return [_TENS[tens], *([_SMALL_NUMBERS[unit]] if unit else [])]
    return [str(value)]


def alignment_words(value: str) -> list[str]:
    """Normalize display text to the MMS_FA lowercase Roman alphabet."""
    folded = unicodedata.normalize("NFKD", value.casefold())
    folded = "".join(character for character in folded
                     if not unicodedata.combining(character))
    folded = folded.replace("&", " and ")
    result: list[str] = []
    for token in re.findall(r"[a-z']+|\d+", folded):
        if token.isdigit():
            number = int(token)
            result.extend(_number_words(number) if number < 100 else [])
        else:
            cleaned = token.strip("'")
            if cleaned:
                result.append(cleaned)
    return result


def alignment_groups(lines: Sequence[dict[str, Any]]) -> list[list[int]]:
    """Use one padded acoustic request per cue.

    Multi-cue requests are faster, but a long authored line can contain pauses
    and repeated words. In real tracks the CTC path then assigned its first word
    to a later phrase depending on which neighboring cues shared the request.
    Per-cue windows make every boundary independent of batching context.
    """
    return [[index] for index in range(len(lines))]


def alignment_is_trusted(
    score: float, input_confidence: Any, *, word_count: int = 1,
    start_shift_seconds: float = 0.0, first_word_score: float | None = None,
    end_shift_seconds: float = 0.0, last_word_score: float | None = None,
    acoustic_within_input: bool = False,
) -> bool:
    high_confidence_input = (
        isinstance(input_confidence, (int, float))
        and not isinstance(input_confidence, bool)
        and float(input_confidence) >= HIGH_INPUT_CONFIDENCE
    )
    acoustically_supported = (
        score >= MINIMUM_ALIGNMENT_SCORE
        or (high_confidence_input
            and score >= MINIMUM_HIGH_CONFIDENCE_ALIGNMENT_SCORE)
    )
    if not acoustically_supported:
        return False
    if (word_count > 1
            and (first_word_score is None
                 or first_word_score < MINIMUM_BOUNDARY_WORD_SCORE
                 or last_word_score is None
                 or last_word_score < MINIMUM_BOUNDARY_WORD_SCORE)):
        return False
    if (word_count > 1
            and abs(start_shift_seconds) > MAXIMUM_LOW_SCORE_START_SHIFT_SECONDS
            and not acoustic_within_input
            and (first_word_score is None
                 or first_word_score
                 < LARGE_SHIFT_MINIMUM_BOUNDARY_WORD_SCORE)):
        return False
    if (word_count > 1
            and abs(end_shift_seconds) > MAXIMUM_LOW_SCORE_START_SHIFT_SECONDS
            and not acoustic_within_input
            and (last_word_score is None
                 or last_word_score
                 < LARGE_SHIFT_MINIMUM_BOUNDARY_WORD_SCORE)):
        return False
    return True


def review_fallback_should_be_dropped(
    decision: dict[str, Any], input_confidence: Any,
) -> bool:
    """Drop only a jointly unsupported multi-word transcription.

    A low CTC score alone is not evidence that sung text is absent. Repeated
    syllables, long vowels, and dense accompaniment produced low scores for
    several lyrics that are plainly present. Treat review text as absent only
    when Whisper was also below the high-confidence threshold and the candidate
    contains enough words for the acoustic comparison to be useful.
    Single-word exclamations are retained because their CTC score is especially
    unstable.
    """
    score = float(decision.get("score", 0.0))
    first_word_score = float(decision.get("first_word_score", 0.0))
    last_word_score = float(decision.get("last_word_score", 0.0))
    word_count = int(decision.get("word_count", 0))
    confident_transcription = (
        isinstance(input_confidence, (int, float))
        and not isinstance(input_confidence, bool)
        and float(input_confidence) >= HIGH_INPUT_CONFIDENCE
    )
    return (
        (score < MINIMUM_ALIGNMENT_SCORE
         or first_word_score < MINIMUM_BOUNDARY_WORD_SCORE
         or last_word_score < MINIMUM_BOUNDARY_WORD_SCORE)
        and not confident_transcription
        and word_count > 1
    )


def mark_duplicate_review_alignments(
    lines: Sequence[dict[str, Any]], decisions: dict[int, dict[str, Any]],
) -> None:
    """Reject review candidates that snapped to the same acoustic phrase."""
    for left_index, left in enumerate(lines):
        left_words = alignment_words(str(left["text"]))
        if not left_words:
            continue
        left_decision = decisions.get(left_index, {})
        if left_decision.get("status") != "aligned":
            continue
        for right_index in range(left_index + 1, len(lines)):
            right = lines[right_index]
            if alignment_words(str(right["text"])) != left_words:
                continue
            right_decision = decisions.get(right_index, {})
            if right_decision.get("status") != "aligned":
                continue
            start_delta = abs(
                float(left_decision["acoustic_start_seconds"])
                - float(right_decision["acoustic_start_seconds"])
            )
            end_delta = abs(
                float(left_decision["acoustic_end_seconds"])
                - float(right_decision["acoustic_end_seconds"])
            )
            if start_delta > 0.25 or end_delta > 0.25:
                continue
            left_rank = (
                float(left.get("confidence") or 0.0),
                float(left_decision.get("score", 0.0)),
            )
            right_rank = (
                float(right.get("confidence") or 0.0),
                float(right_decision.get("score", 0.0)),
            )
            loser = left_index if left_rank < right_rank else right_index
            decisions[loser]["status"] = "duplicate_alignment"


def mark_order_ambiguous_alignments(
    lines: Sequence[dict[str, Any]], decisions: dict[int, dict[str, Any]],
    *, review_lane: bool,
) -> None:
    """Fall back from whichever acoustic move reverses the authored order."""
    while True:
        active: list[int] = []
        for index, line in enumerate(lines):
            decision = decisions.get(index, {})
            status = decision.get("status")
            if review_lane and (
                    status == "duplicate_alignment"
                    or (status != "aligned"
                        and review_fallback_should_be_dropped(
                            decision, line.get("confidence")))):
                continue
            active.append(index)

        reversed_pair: tuple[int, int] | None = None
        for left, right in zip(active, active[1:]):
            left_decision = decisions.get(left, {})
            right_decision = decisions.get(right, {})
            left_start = (
                float(left_decision["acoustic_start_seconds"])
                - CUE_LEAD_SECONDS
                if left_decision.get("status") == "aligned"
                else float(lines[left]["start_seconds"])
            )
            right_start = (
                float(right_decision["acoustic_start_seconds"])
                - CUE_LEAD_SECONDS
                if right_decision.get("status") == "aligned"
                else float(lines[right]["start_seconds"])
            )
            if right_start < left_start:
                reversed_pair = (left, right)
                break
        if reversed_pair is None:
            return

        movable = [index for index in reversed_pair
                   if decisions.get(index, {}).get("status") == "aligned"]
        if not movable:
            return
        loser = max(
            movable,
            key=lambda index: (
                abs(float(decisions[index]["acoustic_start_seconds"])
                    - float(lines[index]["start_seconds"])),
                -float(decisions[index].get("score", 0.0)),
            ),
        )
        decisions[loser]["status"] = "order_fallback"


def _validate_lane(value: Any, audio_duration: float) -> list[dict[str, Any]]:
    if not isinstance(value, dict) or value.get("schema_version") not in {
        "musializer.lyric-sync/v1", "musializer.lyric-review/v1",
    }:
        raise AnalysisValidationError("forced alignment needs a lyric sync or review lane")
    lines = value.get("lines")
    if not isinstance(lines, list):
        raise AnalysisValidationError("lyric lane lines must be an array")
    previous_start = -1.0
    for index, line in enumerate(lines):
        if not isinstance(line, dict) or not isinstance(line.get("text"), str):
            raise AnalysisValidationError(f"lyric line {index} is invalid")
        try:
            start = float(line["start_seconds"])
            end = float(line["end_seconds"])
        except (KeyError, TypeError, ValueError) as error:
            raise AnalysisValidationError(f"lyric line {index} has invalid timing") from error
        if (not math.isfinite(start) or not math.isfinite(end) or start < previous_start or
                start < 0.0 or end <= start or end > audio_duration):
            raise AnalysisValidationError(f"lyric line {index} violates timing bounds")
        previous_start = start
    return lines


def _load_audio(audio: Path) -> tuple[Any, int]:
    try:
        import soundfile as sf
        import torch
        import torchaudio.functional as audio_functional
    except ImportError as error:
        raise RuntimeError(
            "forced alignment requires torch, torchaudio, and soundfile") from error
    samples, sample_rate = sf.read(
        audio, dtype="float32", always_2d=True)
    waveform = torch.from_numpy(samples.mean(axis=1)).unsqueeze(0)
    if sample_rate != 16000:
        waveform = audio_functional.resample(waveform, sample_rate, 16000)
        sample_rate = 16000
    return waveform, sample_rate


def _align_group(
    waveform: Any, sample_rate: int, lines: Sequence[dict[str, Any]],
    indices: Sequence[int], model: Any, tokenizer: Any, aligner: Any,
    *, audio_duration: float,
) -> dict[int, dict[str, Any]]:
    import torch

    first = indices[0]
    last = indices[-1]
    clip_start = max(
        0.0,
        float(lines[first]["start_seconds"]) - CHUNK_LEAD_PADDING_SECONDS,
    )
    clip_end = min(audio_duration,
                   float(lines[last]["end_seconds"]) + CHUNK_TAIL_PADDING_SECONDS)
    if clip_end - clip_start > MAX_CHUNK_SECONDS:
        return {index: {"status": "window_too_long", "score": 0.0}
                for index in indices}
    start_sample = int(clip_start * sample_rate)
    end_sample = min(waveform.shape[-1], math.ceil(clip_end * sample_rate))
    clip = waveform[:, start_sample:end_sample]

    transcript = ["*"]
    owners: list[int | None] = [None]
    for index in indices:
        words = alignment_words(str(lines[index]["text"]))
        transcript.extend(words)
        owners.extend([index] * len(words))
    transcript.append("*")
    owners.append(None)
    if len(transcript) == 2:
        return {}

    device = next(model.parameters()).device
    with torch.inference_mode():
        emission, _ = model(clip.to(device))
    emission = emission[0].cpu()
    spans = aligner(emission, tokenizer(transcript))
    seconds_per_frame = (end_sample - start_sample) / sample_rate / emission.shape[0]

    per_line: dict[int, list[dict[str, float | str]]] = {}
    for word_text, owner, word_spans in zip(transcript, owners, spans):
        if owner is None or not word_spans:
            continue
        start = clip_start + min(span.start for span in word_spans) * seconds_per_frame
        end = clip_start + max(span.end for span in word_spans) * seconds_per_frame
        weighted = sum(float(span.score) * (span.end - span.start)
                       for span in word_spans)
        frames = sum(span.end - span.start for span in word_spans)
        per_line.setdefault(owner, []).append({
            "text": word_text,
            "start_seconds": start,
            "end_seconds": end,
            "score": weighted / frames,
        })

    result: dict[int, dict[str, Any]] = {}
    for index in indices:
        words = per_line.get(index, [])
        if not words:
            result[index] = {"status": "no_alignment", "score": 0.0}
            continue
        score = sum(float(word["score"]) for word in words) / len(words)
        acoustic_start = min(float(word["start_seconds"]) for word in words)
        acoustic_end = max(float(word["end_seconds"]) for word in words)
        input_start = float(lines[index]["start_seconds"])
        input_end = float(lines[index]["end_seconds"])
        acoustic_within_input = (
            acoustic_start >= input_start - INPUT_SPAN_TOLERANCE_SECONDS
            and acoustic_end <= input_end + INPUT_SPAN_TOLERANCE_SECONDS
        )
        result[index] = {
            "status": ("aligned" if alignment_is_trusted(
                           score, lines[index].get("confidence"),
                           word_count=len(words),
                           first_word_score=float(words[0]["score"]),
                           last_word_score=float(words[-1]["score"]),
                           acoustic_within_input=acoustic_within_input,
                           start_shift_seconds=(
                               acoustic_start - input_start),
                           end_shift_seconds=(acoustic_end - input_end))
                       else "unsupported_fallback"),
            "score": score,
            "first_word_score": words[0]["score"],
            "last_word_score": words[-1]["score"],
            "word_count": len(words),
            "word_alignments": words,
            "acoustic_within_input": acoustic_within_input,
            "acoustic_start_seconds": acoustic_start,
            "acoustic_end_seconds": acoustic_end,
        }
    return result


def refine(
    audio: Path, source: dict[str, Any], *, audio_duration: float,
    source_sha256: str,
) -> dict[str, Any]:
    try:
        import torch
        import torchaudio
    except ImportError as error:
        raise RuntimeError("forced alignment requires torch and torchaudio") from error
    if not torch.cuda.is_available():
        raise RuntimeError("forced alignment requires a CUDA device")
    lines = _validate_lane(source, audio_duration)
    waveform, sample_rate = _load_audio(audio)
    bundle = torchaudio.pipelines.MMS_FA
    model = bundle.get_model(with_star=True).to(torch.device("cuda")).eval()
    tokenizer = bundle.get_tokenizer()
    aligner = bundle.get_aligner()

    decisions: dict[int, dict[str, Any]] = {}
    for group in alignment_groups(lines):
        decisions.update(_align_group(
            waveform, sample_rate, lines, group, model, tokenizer, aligner,
            audio_duration=audio_duration,
        ))

    review_lane = source.get("schema_version") == "musializer.lyric-review/v1"
    if review_lane:
        mark_duplicate_review_alignments(lines, decisions)
    mark_order_ambiguous_alignments(
        lines, decisions, review_lane=review_lane)

    refined = json.loads(json.dumps(source))
    aligned_count = 0
    fallback_count = 0
    dropped: list[dict[str, Any]] = []
    start_deltas: list[float] = []
    end_deltas: list[float] = []
    kept_lines: list[dict[str, Any]] = []
    for index, line in enumerate(refined["lines"]):
        decision = decisions.get(index, {"status": "no_alignment", "score": 0.0})
        status = str(decision["status"])
        original_start = float(line["start_seconds"])
        original_end = float(line["end_seconds"])
        if status == "aligned":
            acoustic_start = float(decision["acoustic_start_seconds"])
            acoustic_end = float(decision["acoustic_end_seconds"])
            line["start_seconds"] = max(0.0, acoustic_start - CUE_LEAD_SECONDS)
            line["end_seconds"] = min(audio_duration, acoustic_end + CUE_TAIL_SECONDS)
            if line["end_seconds"] <= line["start_seconds"]:
                line["start_seconds"] = original_start
                line["end_seconds"] = original_end
                status = "collapsed_fallback"
            else:
                aligned_count += 1
                start_deltas.append(line["start_seconds"] - original_start)
                end_deltas.append(line["end_seconds"] - original_end)
        if status != "aligned":
            if review_lane and (
                    status == "duplicate_alignment"
                    or review_fallback_should_be_dropped(
                        decision, line.get("confidence"))):
                dropped.append({
                    "text": line["text"],
                    "input_start_seconds": original_start,
                    "input_end_seconds": original_end,
                    "reason": status,
                    "source_line_indices": line.get("source_line_indices", []),
                    "forced_alignment": decision,
                })
                continue
            fallback_count += 1
            line["uncertain"] = True
        line["forced_alignment"] = {
            **decision,
            "status": status,
            "input_start_seconds": original_start,
            "input_end_seconds": original_end,
        }
        kept_lines.append(line)

    refined["lines"] = kept_lines

    settings = {
        "model": MODEL_ID,
        "alignment_version": ALIGNMENT_VERSION,
        "maximum_chunk_seconds": MAX_CHUNK_SECONDS,
        "chunk_lead_padding_seconds": CHUNK_LEAD_PADDING_SECONDS,
        "chunk_tail_padding_seconds": CHUNK_TAIL_PADDING_SECONDS,
        "minimum_score": MINIMUM_ALIGNMENT_SCORE,
        "high_input_confidence": HIGH_INPUT_CONFIDENCE,
        "minimum_high_confidence_score": MINIMUM_HIGH_CONFIDENCE_ALIGNMENT_SCORE,
        "maximum_low_score_start_shift_seconds": MAXIMUM_LOW_SCORE_START_SHIFT_SECONDS,
        "large_shift_minimum_boundary_word_score": (
            LARGE_SHIFT_MINIMUM_BOUNDARY_WORD_SCORE),
        "minimum_boundary_word_score": MINIMUM_BOUNDARY_WORD_SCORE,
        "input_span_tolerance_seconds": INPUT_SPAN_TOLERANCE_SECONDS,
        "cue_lead_seconds": CUE_LEAD_SECONDS,
        "cue_tail_seconds": CUE_TAIL_SECONDS,
    }
    refined["timing_refinement"] = {
        "adapter": "tools/force_align_lyrics.py",
        "alignment_version": ALIGNMENT_VERSION,
        "model": MODEL_ID,
        "source_sha256": source_sha256,
        "audio_sha256": sha256_file(audio),
        "request_settings": settings,
        "request_settings_sha256": canonical_sha256(settings),
        "statistics": {
            "input_lines": len(lines),
            "aligned_lines": aligned_count,
            "fallback_lines": fallback_count,
            "dropped_review_lines": len(dropped),
            "mean_start_delta_seconds": (
                sum(start_deltas) / len(start_deltas) if start_deltas else None),
            "mean_end_delta_seconds": (
                sum(end_deltas) / len(end_deltas) if end_deltas else None),
        },
        "dropped_review_lines": dropped,
    }
    return refined


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("lyrics", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--duration", required=True, type=float)
    args = parser.parse_args(argv)
    try:
        audio_duration = duration(args.duration)
        if not args.audio.is_file() or not args.lyrics.is_file():
            raise AnalysisValidationError("audio and lyric lane must be files")
        source = read_json(args.lyrics)
        result = refine(
            args.audio, source, audio_duration=audio_duration,
            source_sha256=sha256_file(args.lyrics),
        )
        atomic_write_json(args.output, result)
    except (AnalysisValidationError, OSError, RuntimeError, ValueError) as error:
        print(f"Forced lyric alignment failed: {error}", file=sys.stderr)
        return 1
    stats = result["timing_refinement"]["statistics"]
    print(f"Forced alignment refined {stats['aligned_lines']} lyric cues "
          f"({stats['fallback_lines']} retained their input timing; "
          f"{stats['dropped_review_lines']} unsupported review cues dropped).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
