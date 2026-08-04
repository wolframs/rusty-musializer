#!/usr/bin/env python3
"""Anchor-to-block forced alignment of authored lyrics with MMS_FA.

The production path makes Whisper the gatekeeper for localization: an authored
line that does not match Whisper evidence is omitted before the acoustic
aligner ever sees it, and a line that does match is refined only inside a
window derived from that same suspect evidence.

This lane keeps Whisper as *evidence* and removes it as *authority*, following
"Low Resource Audio-to-Lyrics Alignment From Polyphonic Music Recordings"
(arXiv:2102.09202): spot rare n-grams that occur exactly once in the authored
text and exactly once in the Whisper words, keep the largest monotonic subset
of those matches as anchors, cut the song into blocks at the anchors, and force
the complete consecutive text of each block through MMS_FA in one request.

Every alignable authored line reaches the acoustic stage, including the lines
in the initial block before the first anchor and the terminal block after the
last one. Those two blocks are the point of the exercise: dropping them is
precisely how the canary loses its two outro lines.

Run this under the installed alignment runtime, which is used read-only:

    ~/.local/share/musializer/lyrics-align/.venv/bin/python \\
        tools/lyrics_research/anchor_block_mms.py AUDIO WHISPER OUTPUT \\
        --duration 114.84 --reference-file results/<slug>/reference.txt

No audio output device is opened; samples are decoded with soundfile and go
straight to the model.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
import time
from pathlib import Path
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import lyric_align  # noqa: E402
from analysis_io import (  # noqa: E402
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    duration as validate_duration,
    read_json,
    sha256_file,
)
from force_align_lyrics import alignment_words  # noqa: E402
from research_io import RESEARCH_LANE_VERSION, alignable_lines  # noqa: E402


METHOD = "anchor-block-mms"
LANE_VERSION = "3"
MODEL_ID = "torchaudio.pipelines.MMS_FA"

# Anchor spotting. An n-gram is only an anchor when it is unique on *both*
# sides: a phrase repeated in the lyrics cannot say which chorus it belongs to,
# and a phrase Whisper emitted twice cannot say which occurrence is real.
ANCHOR_LENGTHS = (5, 4, 3)
# Two words is enough to be common in almost any lyric sheet, which is why the
# shortest anchor is three.
ANCHOR_MINIMUM_LENGTH = 3

# Block windows. The lead reaches back before the anchor so the anchored line's
# own onset is inside the window; the tail reaches past the next anchor's onset
# so the block's last line has room to end.
BLOCK_LEAD_SECONDS = 1.5
BLOCK_TAIL_SECONDS = 3.0
# One forward pass over the block. 90 s of 16 kHz audio is comfortable on a
# 24 GiB device; longer blocks are split rather than refused, because refusing
# would reintroduce the coverage failure this lane exists to remove.
MAX_BLOCK_SECONDS = 90.0
BLOCK_SPLIT_OVERLAP_SECONDS = 4.0

# Boundary padding, matching tools/force_align_lyrics.py so cue edges from the
# two lanes are directly comparable.
CUE_LEAD_SECONDS = 0.10
CUE_TAIL_SECONDS = 0.15

MINIMUM_ALIGNMENT_SCORE = 0.15
MINIMUM_LINE_SECONDS = 0.15


# --------------------------------------------------------------------------
# Evidence
# --------------------------------------------------------------------------


def evidence_tokens(
    whisper: dict[str, Any], unreliable: Sequence[tuple[float, float]],
) -> list[dict[str, Any]]:
    """Timed normalized tokens from the Whisper lane, loops excluded.

    A repetition loop is high-confidence and completely wrong, so its words are
    the worst possible anchors: the canary's loop repeats a genuine authored
    line 20 times across the outro, and anchoring on it would pin that line to
    the wrong occurrence and bury the two lines that follow it.
    """
    words = whisper.get("words")
    if not isinstance(words, list) or not words:
        # Models without DTW word timing still give line intervals; every token
        # of a line then shares the line's window.
        words = whisper.get("lines") if isinstance(whisper.get("lines"), list) else []
    tokens: list[dict[str, Any]] = []
    for word in words or []:
        if not isinstance(word, dict):
            continue
        try:
            start = float(word["start_seconds"])
            end = float(word["end_seconds"])
        except (KeyError, TypeError, ValueError):
            continue
        if not math.isfinite(start) or not math.isfinite(end) or end < start:
            continue
        middle = (start + end) / 2.0
        if any(low <= middle <= high for low, high in unreliable):
            continue
        for token in lyric_align.normalize_tokens(str(word.get("text", ""))):
            tokens.append({"token": token, "start": start, "end": end})
    return tokens


def authored_tokens(lines: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    tokens: list[dict[str, Any]] = []
    for position, line in enumerate(lines):
        for token in line["tokens"]:
            tokens.append({"token": token, "line_position": position})
    return tokens


def _ngram_index(tokens: Sequence[str], length: int) -> dict[str, list[int]]:
    index: dict[str, list[int]] = {}
    for start in range(len(tokens) - length + 1):
        key = " ".join(tokens[start:start + length])
        index.setdefault(key, []).append(start)
    return index


def spot_anchors(
    authored: Sequence[dict[str, Any]], evidence: Sequence[dict[str, Any]],
    *, lengths: Sequence[int] = ANCHOR_LENGTHS,
) -> list[dict[str, Any]]:
    """Rare n-gram matches, longest first, non-overlapping and monotonic."""
    authored_words = [item["token"] for item in authored]
    evidence_words = [item["token"] for item in evidence]
    candidates: list[dict[str, Any]] = []
    taken_authored: list[tuple[int, int]] = []
    taken_evidence: list[tuple[int, int]] = []
    for length in sorted({int(value) for value in lengths}, reverse=True):
        if length < ANCHOR_MINIMUM_LENGTH:
            continue
        left = _ngram_index(authored_words, length)
        right = _ngram_index(evidence_words, length)
        for key, positions in left.items():
            if len(positions) != 1:
                continue
            hits = right.get(key)
            if hits is None or len(hits) != 1:
                continue
            authored_start = positions[0]
            evidence_start = hits[0]
            authored_span = (authored_start, authored_start + length)
            evidence_span = (evidence_start, evidence_start + length)
            if any(authored_span[0] < end and start < authored_span[1]
                   for start, end in taken_authored):
                continue
            if any(evidence_span[0] < end and start < evidence_span[1]
                   for start, end in taken_evidence):
                continue
            taken_authored.append(authored_span)
            taken_evidence.append(evidence_span)
            candidates.append({
                "text": key,
                "length": length,
                "authored_token_index": authored_start,
                "evidence_token_index": evidence_start,
                "line_position": authored[authored_start]["line_position"],
                "start_seconds": float(evidence[evidence_start]["start"]),
                "end_seconds": float(
                    evidence[evidence_start + length - 1]["end"]),
            })
    candidates.sort(key=lambda item: item["authored_token_index"])
    return _monotonic_subset(candidates)


def _monotonic_subset(
    candidates: Sequence[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Heaviest subset whose times increase with authored position.

    A unique-on-both-sides match can still be wrong, and a wrong anchor is
    worse than a missing one because it mislocates a whole block. Contradictory
    anchors are resolved globally rather than by a local score: keep the
    heaviest increasing subsequence (weighted by n-gram length) and drop the
    rest. O(n^2) is ample — a song yields tens of candidates, not thousands.
    """
    count = len(candidates)
    if count == 0:
        return []
    best = [float(candidates[index]["length"]) for index in range(count)]
    previous = [-1] * count
    for index in range(count):
        for earlier in range(index):
            if (candidates[earlier]["start_seconds"]
                    < candidates[index]["start_seconds"]
                    and candidates[earlier]["end_seconds"]
                    <= candidates[index]["start_seconds"]
                    and best[earlier] + candidates[index]["length"] > best[index]):
                best[index] = best[earlier] + candidates[index]["length"]
                previous[index] = earlier
    tail = max(range(count), key=lambda index: best[index])
    chain: list[dict[str, Any]] = []
    while tail >= 0:
        chain.append(candidates[tail])
        tail = previous[tail]
    chain.reverse()
    return chain


# --------------------------------------------------------------------------
# Blocks
# --------------------------------------------------------------------------


def token_time_map(
    anchors: Sequence[dict[str, Any]], *, token_count: int,
    audio_duration: float,
) -> list[tuple[int, float]]:
    """Monotone control points mapping authored token index to seconds.

    Anchors pin *tokens*, not lines, and a rare n-gram often begins in the
    middle of a line — the canary's ``spark of who we are`` starts on the last
    word of one line. Treating the anchor's time as that line's onset would
    start the search window several seconds after the line begins and clip its
    first words. Interpolating between anchors instead gives every line an
    onset estimate that degrades gracefully rather than lying.

    The endpoints are deliberately loose bounds (zero and the track duration):
    an estimate that reaches back too far only widens a search window, while
    one that reaches back too little loses audio that a line needs.
    """
    points: list[tuple[int, float]] = []
    for anchor in anchors:
        start = int(anchor["authored_token_index"])
        points.append((start, float(anchor["start_seconds"])))
        points.append((start + int(anchor["length"]),
                       float(anchor["end_seconds"])))
    points.sort()
    monotone: list[tuple[int, float]] = []
    for index, seconds in points:
        if monotone and index == monotone[-1][0]:
            monotone[-1] = (index, min(monotone[-1][1], seconds))
            continue
        if monotone and seconds < monotone[-1][1]:
            seconds = monotone[-1][1]
        monotone.append((index, seconds))
    if not monotone or monotone[0][0] > 0:
        monotone.insert(0, (0, 0.0))
    if monotone[-1][0] < token_count:
        monotone.append((token_count, audio_duration))
    return monotone


def interpolate(points: Sequence[tuple[int, float]], index: int) -> float:
    if index <= points[0][0]:
        return points[0][1]
    if index >= points[-1][0]:
        return points[-1][1]
    for (left_index, left_time), (right_index, right_time) in zip(points, points[1:]):
        if left_index <= index <= right_index:
            if right_index == left_index:
                return left_time
            fraction = (index - left_index) / (right_index - left_index)
            return left_time + fraction * (right_time - left_time)
    return points[-1][1]


def build_blocks(
    lines: Sequence[dict[str, Any]], anchors: Sequence[dict[str, Any]],
    *, audio_duration: float, max_block_seconds: float = MAX_BLOCK_SECONDS,
) -> list[dict[str, Any]]:
    """Partition every authored line into an ordered, timed search block.

    Cuts fall at the lines where anchors begin, so consecutive unanchored lines
    stay together and are aligned as one ordered path. Each block searches from
    its first line's estimated onset to its last line's estimated offset, both
    padded. Lines before the first anchor form an initial block searching from
    zero; lines from the last anchor on form a terminal block searching to the
    end of the track. With no anchors at all the whole song is one block —
    still a search, never an omission.
    """
    line_count = len(lines)
    if line_count == 0:
        return []

    first_token: list[int] = []
    running = 0
    for line in lines:
        first_token.append(running)
        running += len(line["tokens"])
    token_count = running
    points = token_time_map(
        anchors, token_count=token_count, audio_duration=audio_duration)

    def onset(position: int) -> float:
        return interpolate(points, first_token[position])

    def offset(position: int) -> float:
        end = (first_token[position + 1] if position + 1 < line_count
               else token_count)
        return interpolate(points, end)

    cuts: list[int] = []
    for anchor in anchors:
        position = int(anchor["line_position"])
        if not cuts or cuts[-1] != position:
            cuts.append(position)

    spans: list[dict[str, Any]] = []
    if not cuts:
        spans.append({
            "first_line": 0, "last_line": line_count - 1,
            "window_start": 0.0, "window_end": audio_duration,
            "kind": "unanchored",
        })
    else:
        if cuts[0] > 0:
            spans.append({
                "first_line": 0, "last_line": cuts[0] - 1,
                "kind": "initial",
            })
        for index, position in enumerate(cuts):
            following = cuts[index + 1] if index + 1 < len(cuts) else None
            last_line = (following - 1) if following is not None else line_count - 1
            if last_line < position:
                continue
            spans.append({
                "first_line": position, "last_line": last_line,
                "kind": "terminal" if following is None else "anchored",
            })
    for span in spans:
        span.setdefault(
            "window_start",
            0.0 if span["first_line"] == 0
            else max(0.0, onset(span["first_line"]) - BLOCK_LEAD_SECONDS))
        span.setdefault(
            "window_end",
            audio_duration if span["last_line"] == line_count - 1
            else min(audio_duration, offset(span["last_line"]) + BLOCK_TAIL_SECONDS))

    blocks: list[dict[str, Any]] = []
    for span in spans:
        blocks.extend(_split_span(
            span, lines, max_block_seconds=max_block_seconds))
    for index, block in enumerate(blocks):
        block["index"] = index
    return blocks


def _split_span(
    span: dict[str, Any], lines: Sequence[dict[str, Any]],
    *, max_block_seconds: float,
) -> list[dict[str, Any]]:
    """Cut an over-long span into overlapping sub-blocks by token weight.

    Only reached when anchors are sparse. The split is recorded on every piece
    so a later reader can tell a proportional guess from a real anchor.
    """
    window_start = float(span["window_start"])
    length = float(span["window_end"]) - window_start
    if length <= max_block_seconds:
        return [{**span, "split": False}]
    positions = list(range(span["first_line"], span["last_line"] + 1))
    if len(positions) < 2:
        # One line cannot be cut further, and a CTC forward pass is quadratic
        # in window length: an unbounded window here is an out-of-memory abort,
        # not a slow answer. Keep the centre of the estimate and record that
        # the search space was clamped, so a miss is explainable.
        middle = (window_start + float(span["window_end"])) / 2.0
        half = max_block_seconds / 2.0
        return [{
            **span,
            "window_start": max(window_start, middle - half),
            "window_end": min(float(span["window_end"]), middle + half),
            "split": False,
            "oversized": True,
            "clamped": True,
        }]
    weights = [max(1, len(lines[position]["tokens"])) for position in positions]
    prefix = [0]
    for weight in weights:
        prefix.append(prefix[-1] + weight)
    total = float(prefix[-1])
    pieces = min(int(math.ceil(length / max_block_seconds)), len(positions))
    target = total / pieces

    groups: list[tuple[int, int]] = []
    group_start = 0
    for offset in range(len(positions)):
        last_line = offset == len(positions) - 1
        full = prefix[offset + 1] >= target * (len(groups) + 1)
        if last_line or (len(groups) < pieces - 1 and full):
            groups.append((group_start, offset))
            group_start = offset + 1
            if group_start >= len(positions):
                break

    result: list[dict[str, Any]] = []
    for first, last in groups:
        start = window_start + (prefix[first] / total) * length
        end = window_start + (prefix[last + 1] / total) * length
        result.append({
            **span,
            "first_line": positions[first],
            "last_line": positions[last],
            "window_start": max(
                window_start, start - BLOCK_SPLIT_OVERLAP_SECONDS),
            "window_end": min(
                float(span["window_end"]), end + BLOCK_SPLIT_OVERLAP_SECONDS),
            "split": True,
        })
    return result


# --------------------------------------------------------------------------
# Acoustics
# --------------------------------------------------------------------------


def _load_audio(audio: Path) -> tuple[Any, int]:
    """Mono 16 kHz samples. Decode only — nothing opens an output device."""
    try:
        import soundfile as sf
        import torch
        import torchaudio.functional as audio_functional
    except ImportError as error:
        raise RuntimeError(
            "the anchor/block lane requires torch, torchaudio and soundfile; "
            "run it with the lyrics-align virtual environment") from error
    samples, sample_rate = sf.read(audio, dtype="float32", always_2d=True)
    waveform = torch.from_numpy(samples.mean(axis=1)).unsqueeze(0)
    if sample_rate != 16000:
        waveform = audio_functional.resample(waveform, sample_rate, 16000)
        sample_rate = 16000
    return waveform, sample_rate


def align_block(
    waveform: Any, sample_rate: int, lines: Sequence[dict[str, Any]],
    block: dict[str, Any], model: Any, tokenizer: Any, aligner: Any,
    *, interior_stars: bool,
) -> dict[int, dict[str, Any]]:
    """Force the block's complete consecutive text through one CTC path.

    Aligning the lines together is what disambiguates a repeated phrase: the
    path has to pass through the block's earlier lines before it can reach a
    later one, so an ordered chorus cannot collapse onto a single occurrence.
    """
    import torch

    clip_start = float(block["window_start"])
    clip_end = float(block["window_end"])
    start_sample = int(clip_start * sample_rate)
    end_sample = min(waveform.shape[-1], math.ceil(clip_end * sample_rate))
    clip = waveform[:, start_sample:end_sample]
    if clip.shape[-1] < sample_rate // 10:
        return {position: {"status": "window_too_short", "score": 0.0}
                for position in range(block["first_line"], block["last_line"] + 1)}

    transcript: list[str] = ["*"]
    owners: list[int | None] = [None]
    for position in range(block["first_line"], block["last_line"] + 1):
        words = alignment_words(str(lines[position]["display"]))
        if not words:
            continue
        if interior_stars and len(transcript) > 1:
            # Songs put instrumental bars between lines. Without a star to
            # absorb them the path has to explain accompaniment with lyrics,
            # which drags a boundary across the gap.
            transcript.append("*")
            owners.append(None)
        transcript.extend(words)
        owners.extend([position] * len(words))
    transcript.append("*")
    owners.append(None)
    if len(transcript) <= 2:
        return {}

    device = next(model.parameters()).device
    with torch.inference_mode():
        emission, _ = model(clip.to(device))
    emission = emission[0].cpu()
    spans = aligner(emission, tokenizer(transcript))
    seconds_per_frame = (
        (end_sample - start_sample) / sample_rate / emission.shape[0])

    per_line: dict[int, list[dict[str, Any]]] = {}
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
            "score": weighted / frames if frames else 0.0,
        })

    result: dict[int, dict[str, Any]] = {}
    for position in range(block["first_line"], block["last_line"] + 1):
        words = per_line.get(position, [])
        if not words:
            result[position] = {"status": "no_alignment", "score": 0.0}
            continue
        score = sum(float(word["score"]) for word in words) / len(words)
        acoustic_start = min(float(word["start_seconds"]) for word in words)
        acoustic_end = max(float(word["end_seconds"]) for word in words)
        supported = (score >= MINIMUM_ALIGNMENT_SCORE
                     and acoustic_end - acoustic_start >= MINIMUM_LINE_SECONDS)
        result[position] = {
            "status": "aligned" if supported else "weak",
            "score": score,
            "first_word_score": words[0]["score"],
            "last_word_score": words[-1]["score"],
            "word_count": len(words),
            "acoustic_start_seconds": acoustic_start,
            "acoustic_end_seconds": acoustic_end,
            "word_alignments": words,
        }
    return result


# --------------------------------------------------------------------------
# Lane
# --------------------------------------------------------------------------


def order_violations(lines: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    """Timed lines whose start goes backwards against authored order."""
    violations: list[dict[str, Any]] = []
    previous: dict[str, Any] | None = None
    for line in lines:
        if line.get("start_seconds") is None:
            continue
        if previous is not None and line["start_seconds"] < previous["start_seconds"]:
            violations.append({
                "previous_reference_line_index": previous["reference_line_index"],
                "reference_line_index": line["reference_line_index"],
                "previous_start_seconds": previous["start_seconds"],
                "start_seconds": line["start_seconds"],
            })
        previous = line
    return violations


def align(
    audio: Path, whisper: dict[str, Any], reference_text: str, *,
    audio_duration: float, interior_stars: bool = True,
    max_block_seconds: float = MAX_BLOCK_SECONDS,
) -> dict[str, Any]:
    try:
        import torch
        import torchaudio
    except ImportError as error:
        raise RuntimeError(
            "the anchor/block lane requires torch and torchaudio") from error
    if not torch.cuda.is_available():
        raise RuntimeError("the anchor/block lane requires a CUDA device")

    started = time.monotonic()
    lines = alignable_lines(reference_text)
    if not lines:
        raise AnalysisValidationError("authored lyrics carry no alignable lines")

    unreliable = lyric_align.flag_unreliable_intervals(
        whisper.get("lines") if isinstance(whisper.get("lines"), list) else [])
    evidence = evidence_tokens(whisper, unreliable)
    authored = authored_tokens(lines)
    anchors = spot_anchors(authored, evidence)
    blocks = build_blocks(
        lines, anchors, audio_duration=audio_duration,
        max_block_seconds=max_block_seconds)

    waveform, sample_rate = _load_audio(audio)
    bundle = torchaudio.pipelines.MMS_FA
    model = bundle.get_model(with_star=True).to(torch.device("cuda")).eval()
    tokenizer = bundle.get_tokenizer()
    aligner = bundle.get_aligner()

    decisions: dict[int, dict[str, Any]] = {}
    owning_block: dict[int, int] = {}
    for block in blocks:
        block_started = time.monotonic()
        outcome = align_block(
            waveform, sample_rate, lines, block, model, tokenizer, aligner,
            interior_stars=interior_stars)
        block["runtime_seconds"] = time.monotonic() - block_started
        block["line_count"] = block["last_line"] - block["first_line"] + 1
        for position, decision in outcome.items():
            previous = decisions.get(position)
            # A split span overlaps; keep the better-supported copy.
            if previous is None or (float(decision.get("score", 0.0))
                                    > float(previous.get("score", 0.0))):
                decisions[position] = decision
                owning_block[position] = block["index"]

    lane_lines: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []
    for position, line in enumerate(lines):
        decision = decisions.get(position, {"status": "no_alignment", "score": 0.0})
        status = str(decision["status"])
        record: dict[str, Any] = {
            "reference_line_index": int(line["index"]),
            "line_position": position,
            "kind": line["kind"],
            "text": line["display"],
            "start_seconds": None,
            "end_seconds": None,
            "score": float(decision.get("score", 0.0)),
            "status": status,
            "uncertain": status != "aligned",
            "block_index": owning_block.get(position),
        }
        if status in {"aligned", "weak"}:
            record["start_seconds"] = max(
                0.0,
                float(decision["acoustic_start_seconds"]) - CUE_LEAD_SECONDS)
            record["end_seconds"] = min(
                audio_duration,
                float(decision["acoustic_end_seconds"]) + CUE_TAIL_SECONDS)
            if record["end_seconds"] <= record["start_seconds"]:
                record["start_seconds"] = None
                record["end_seconds"] = None
                record["status"] = "collapsed"
                record["uncertain"] = True
            record["word_alignments"] = decision.get("word_alignments", [])
            record["first_word_score"] = decision.get("first_word_score")
            record["last_word_score"] = decision.get("last_word_score")
        if record["start_seconds"] is None:
            unresolved.append({
                "reference_line_index": record["reference_line_index"],
                "text": record["text"],
                "reason": record["status"],
            })
        lane_lines.append(record)

    settings = {
        "anchor_lengths": list(ANCHOR_LENGTHS),
        "block_lead_seconds": BLOCK_LEAD_SECONDS,
        "block_tail_seconds": BLOCK_TAIL_SECONDS,
        "max_block_seconds": max_block_seconds,
        "block_split_overlap_seconds": BLOCK_SPLIT_OVERLAP_SECONDS,
        "cue_lead_seconds": CUE_LEAD_SECONDS,
        "cue_tail_seconds": CUE_TAIL_SECONDS,
        "minimum_alignment_score": MINIMUM_ALIGNMENT_SCORE,
        "minimum_line_seconds": MINIMUM_LINE_SECONDS,
        "interior_stars": interior_stars,
    }
    timed = [line for line in lane_lines if line["start_seconds"] is not None]
    return {
        "schema_version": RESEARCH_LANE_VERSION,
        "method": METHOD,
        "lane_version": LANE_VERSION,
        "audio": {
            "sha256": sha256_file(audio),
            "duration_seconds": audio_duration,
        },
        "lines": lane_lines,
        "unresolved": unresolved,
        "anchors": anchors,
        "blocks": blocks,
        "unreliable_evidence": [
            {"start_seconds": low, "end_seconds": high}
            for low, high in unreliable
        ],
        "statistics": {
            "authored_alignable_lines": len(lines),
            "timed_lines": len(timed),
            "aligned_lines": sum(1 for line in lane_lines
                                 if line["status"] == "aligned"),
            "weak_lines": sum(1 for line in lane_lines
                              if line["status"] == "weak"),
            "unresolved_lines": len(unresolved),
            "anchor_count": len(anchors),
            "block_count": len(blocks),
            "evidence_tokens": len(evidence),
            "authored_tokens": len(authored),
            "order_violations": len(order_violations(lane_lines)),
            "runtime_seconds": time.monotonic() - started,
        },
        "order_violations": order_violations(lane_lines),
        "provenance": {
            "adapter": "tools/lyrics_research/anchor_block_mms.py",
            "lane_version": LANE_VERSION,
            "model": MODEL_ID,
            "request_settings": settings,
            "request_settings_sha256": canonical_sha256(settings),
        },
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("whisper", type=Path,
                        help="lyrics.whisper.json from the baseline chunk")
    parser.add_argument("output", type=Path)
    parser.add_argument("--duration", required=True, type=float)
    parser.add_argument("--reference-file", type=Path,
                        help="authored lyrics; defaults to the embedded tag")
    parser.add_argument("--max-block-seconds", type=float,
                        default=MAX_BLOCK_SECONDS)
    parser.add_argument("--no-interior-stars", action="store_true",
                        help="do not let a star absorb instrumental bars "
                             "between lines")
    args = parser.parse_args(argv)
    try:
        audio_duration = validate_duration(args.duration)
        if not args.audio.is_file():
            raise AnalysisValidationError("audio file does not exist")
        if not args.whisper.is_file():
            raise AnalysisValidationError("whisper evidence does not exist")
        whisper = read_json(args.whisper)
        if args.reference_file is not None:
            reference_text = args.reference_file.read_text(encoding="utf-8")
        else:
            import external_analysis

            found = external_analysis.discover_reference_lyrics(args.audio)
            if found is None:
                raise AnalysisValidationError("no authored lyrics for this track")
            reference_text = found["text"]
        result = align(
            args.audio, whisper, reference_text,
            audio_duration=audio_duration,
            interior_stars=not args.no_interior_stars,
            max_block_seconds=args.max_block_seconds,
        )
        atomic_write_json(args.output, result)
    except (AnalysisValidationError, OSError, RuntimeError, ValueError,
            json.JSONDecodeError) as error:
        print(f"Anchor/block alignment failed: {error}", file=sys.stderr)
        return 1
    stats = result["statistics"]
    print(f"Anchor/block alignment timed {stats['timed_lines']}/"
          f"{stats['authored_alignable_lines']} authored lines "
          f"({stats['aligned_lines']} supported, {stats['weak_lines']} weak, "
          f"{stats['unresolved_lines']} unresolved) from "
          f"{stats['anchor_count']} anchors in {stats['block_count']} blocks "
          f"in {stats['runtime_seconds']:.1f} s.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
