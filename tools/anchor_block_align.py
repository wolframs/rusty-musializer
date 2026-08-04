#!/usr/bin/env python3
"""Anchor-to-block forced alignment of authored lyrics: the acoustic half.

Runs under the installed alignment runtime (torch + torchaudio + soundfile),
which is why it is a separate executable from ``external_analysis.py``. The
policy it applies lives in ``tools/lyric_anchor_block.py`` and needs no model,
so it can be tested and versioned on its own.

    ~/.local/share/musializer/lyrics-align/.venv/bin/python \\
        tools/anchor_block_align.py AUDIO WHISPER OUTPUT \\
        --duration 114.84 --reference-file lyrics.reference.txt \\
        --coarse lyrics.sync.json

Every block's complete consecutive text goes through MMS_FA in one CTC pass, so
a repeated phrase is disambiguated by its ordered neighbours instead of by a
local score. The initial block before the first anchor and the terminal block
after the last one are first-class: dropping them is precisely how the coverage
canary lost its two outro lines.

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

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lyric_anchor_block  # noqa: E402
from analysis_io import (  # noqa: E402
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    duration as validate_duration,
    read_json,
    sha256_file,
)
from force_align_lyrics import alignment_words  # noqa: E402

# Bump with any change to the acoustic request itself (model, window padding,
# star policy). The localization policy version in `lyric_anchor_block` covers
# the decisions made around it.
ALIGNMENT_VERSION = "1"
MODEL_ID = "torchaudio.pipelines.MMS_FA"
ADAPTER = "tools/anchor_block_align.py"


def _load_audio(audio: Path) -> tuple[Any, int]:
    """Mono 16 kHz samples. Decode only — nothing opens an output device."""
    try:
        import soundfile as sf
        import torch
        import torchaudio.functional as audio_functional
    except ImportError as error:
        raise RuntimeError(
            "anchor/block alignment requires torch, torchaudio and soundfile; "
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
        # `aligned` versus `weak` is recorded, never acted on as confidence:
        # the operator adjudication measured the score as uninformative about
        # correctness. Both statuses produce a cue; only cross-view
        # disagreement and abstention produce a review flag.
        supported = (score >= lyric_anchor_block.MINIMUM_ALIGNMENT_SCORE
                     and acoustic_end - acoustic_start
                     >= lyric_anchor_block.MINIMUM_LINE_SECONDS)
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


def align(
    audio: Path, whisper: dict[str, Any], reference_text: str,
    coarse_document: Any, *, audio_duration: float, interior_stars: bool = True,
    max_block_seconds: float = lyric_anchor_block.MAX_BLOCK_SECONDS,
) -> dict[str, Any]:
    try:
        import torch
        import torchaudio
    except ImportError as error:
        raise RuntimeError(
            "anchor/block alignment requires torch and torchaudio") from error
    if not torch.cuda.is_available():
        raise RuntimeError("anchor/block alignment requires a CUDA device")

    started = time.monotonic()
    plan = lyric_anchor_block.plan_localization(
        reference_text, whisper, audio_duration=audio_duration,
        max_block_seconds=max_block_seconds)
    lines = plan["lines"]

    waveform, sample_rate = _load_audio(audio)
    bundle = torchaudio.pipelines.MMS_FA
    model = bundle.get_model(with_star=True).to(torch.device("cuda")).eval()
    tokenizer = bundle.get_tokenizer()
    aligner = bundle.get_aligner()

    decisions: dict[int, dict[str, Any]] = {}
    owning_block: dict[int, int] = {}
    for block in plan["blocks"]:
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

    coarse = lyric_anchor_block.coarse_proposals(coarse_document)
    document = lyric_anchor_block.assemble_document(
        plan, decisions, owning_block, coarse, audio_duration=audio_duration)

    settings = {
        "anchor_lengths": list(lyric_anchor_block.ANCHOR_LENGTHS),
        "block_lead_seconds": lyric_anchor_block.BLOCK_LEAD_SECONDS,
        "block_tail_seconds": lyric_anchor_block.BLOCK_TAIL_SECONDS,
        "max_block_seconds": max_block_seconds,
        "block_split_overlap_seconds": lyric_anchor_block.BLOCK_SPLIT_OVERLAP_SECONDS,
        "cue_lead_seconds": lyric_anchor_block.CUE_LEAD_SECONDS,
        "cue_tail_seconds": lyric_anchor_block.CUE_TAIL_SECONDS,
        "minimum_alignment_score": lyric_anchor_block.MINIMUM_ALIGNMENT_SCORE,
        "minimum_line_seconds": lyric_anchor_block.MINIMUM_LINE_SECONDS,
        "review_disagreement_seconds": lyric_anchor_block.REVIEW_DISAGREEMENT_SECONDS,
        "repeated_collapse_seconds": lyric_anchor_block.REPEATED_COLLAPSE_SECONDS,
        "interior_stars": interior_stars,
    }
    document["audio"] = {
        "sha256": sha256_file(audio),
        "duration_seconds": audio_duration,
    }
    document["timing_refinement"] = {
        "adapter": ADAPTER,
        "alignment_version": ALIGNMENT_VERSION,
        "localization_policy": lyric_anchor_block.LOCALIZATION_POLICY,
        "localization_policy_version": (
            lyric_anchor_block.LOCALIZATION_POLICY_VERSION),
        "model": MODEL_ID,
        "audio_sha256": document["audio"]["sha256"],
        "request_settings": settings,
        "request_settings_sha256": canonical_sha256(settings),
        "statistics": {
            **document["statistics"],
            "runtime_seconds": time.monotonic() - started,
        },
    }
    document["statistics"]["runtime_seconds"] = time.monotonic() - started
    return document


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("whisper", type=Path,
                        help="lyrics.whisper.json evidence")
    parser.add_argument("output", type=Path)
    parser.add_argument("--duration", required=True, type=float)
    parser.add_argument("--reference-file", required=True, type=Path,
                        help="authored lyrics, display truth")
    parser.add_argument("--coarse", type=Path,
                        help="lyrics.sync.json, the independent coarse view "
                             "used for review flags and repeated-phrase "
                             "abstention")
    parser.add_argument("--max-block-seconds", type=float,
                        default=lyric_anchor_block.MAX_BLOCK_SECONDS)
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
        if not args.reference_file.is_file():
            raise AnalysisValidationError("authored lyrics file does not exist")
        whisper = read_json(args.whisper)
        coarse_document: Any = None
        if args.coarse is not None and args.coarse.is_file():
            coarse_document = read_json(args.coarse)
        result = align(
            args.audio, whisper,
            args.reference_file.read_text(encoding="utf-8"), coarse_document,
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
    print(f"Anchor/block localization timed {stats['matched_lines']}/"
          f"{stats['reference_lines']} authored lines "
          f"({stats['unresolved_lines']} unresolved, "
          f"{stats['abstained_lines']} abstained on repeated phrases, "
          f"{stats['review_flagged_lines']} flagged for review) from "
          f"{stats['anchor_count']} anchors in {stats['block_count']} blocks "
          f"in {stats['runtime_seconds']:.1f} s.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
