#!/usr/bin/env python3
"""Whole-song forced alignment of authored lyrics with Qwen3-ForcedAligner.

This lane tests the central hypothesis of the research plan directly: hand the
complete audio and the complete authored lyrics to a model that predicts
timestamp slots for a known transcript, with no Whisper proposal anywhere in
the path. Nothing here reads ``lyrics.whisper.json``; the only inputs are the
MP3 and the embedded ``lyrics-eng`` tag.

Qwen3-ForcedAligner-0.6B is non-autoregressive: it inserts timestamp slots
after each word and predicts every index in one pass, so the whole song fits a
single request up to its documented five-minute cap. Every benchmark track is
inside that cap, so this lane deliberately has no splitting logic — splitting
would reintroduce the boundary question the lane exists to avoid.

The official table labels the aligner a *speech* model. Whether it survives
singing with accompaniment is exactly what is being measured, so a weak or
absent result is a finding rather than a harness failure.

Run under the research virtual environment:

    build/lyrics-research-v2/.venv/bin/python \\
        tools/lyrics_research/qwen_forced_aligner.py AUDIO OUTPUT \\
        --duration 114.84 --reference-file results/<slug>/reference.txt

No audio output device is opened; samples are decoded and go to the model.
"""

from __future__ import annotations

import argparse
import json
import math
import os
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
    sha256_file,
)
from research_io import RESEARCH_LANE_VERSION, alignable_lines  # noqa: E402


METHOD = "qwen"
LANE_VERSION = "1"
DEFAULT_MODEL_DIR = (
    Path.home() / ".local/share/musializer/models/Qwen3-ForcedAligner-0.6B")
MODEL_DIR_ENVIRONMENT = "MUSIALIZER_QWEN_ALIGNER_DIR"
LANGUAGE = "English"
SAMPLE_RATE = 16000
# The published single-pass cap. Every benchmark track is inside it; a longer
# track is refused loudly rather than silently truncated. Note that the
# installed ``qwen_asr`` package carries a stricter 180 s constant
# (``MAX_FORCE_ALIGN_INPUT_SECONDS``), but only its *ASR* path chunks on it —
# ``Qwen3ForcedAligner.align`` enforces nothing. Two benchmark tracks are past
# that constant, so a degraded result there is a finding about the model, not a
# harness bug.
MAX_AUDIO_SECONDS = 300.0
PACKAGE_SOFT_LIMIT_SECONDS = 180.0

CUE_LEAD_SECONDS = 0.0
CUE_TAIL_SECONDS = 0.0


def resolve_model_dir(explicit: Path | None = None) -> Path:
    if explicit is not None:
        return explicit
    configured = os.environ.get(MODEL_DIR_ENVIRONMENT)
    return Path(configured).expanduser() if configured else DEFAULT_MODEL_DIR


def load_audio(audio: Path) -> Any:
    """Mono float32 at 16 kHz as a numpy array. Decode only, never play."""
    try:
        import soundfile as sf
        import torch
        import torchaudio.functional as audio_functional
    except ImportError as error:
        raise RuntimeError(
            "the Qwen lane requires torch, torchaudio and soundfile") from error
    samples, sample_rate = sf.read(audio, dtype="float32", always_2d=True)
    waveform = torch.from_numpy(samples.mean(axis=1))
    if sample_rate != SAMPLE_RATE:
        waveform = audio_functional.resample(
            waveform.unsqueeze(0), sample_rate, SAMPLE_RATE).squeeze(0)
    return waveform.numpy()


def build_transcript(
    lines: Sequence[dict[str, Any]],
) -> tuple[str, list[int]]:
    """One flat transcript plus the authored line each word belongs to.

    The aligner takes a single ordered transcript, so line structure has to be
    carried alongside rather than in the text: a newline would become an
    unpredictable token and a per-line request would defeat the whole-song
    point of this lane.
    """
    words: list[str] = []
    owners: list[int] = []
    for position, line in enumerate(lines):
        for word in str(line["display"]).split():
            words.append(word)
            owners.append(position)
    return " ".join(words), owners


def _predict_with_package(
    model_dir: Path, samples: Any, transcript: str,
) -> tuple[str, list[dict[str, Any]]]:
    """Qwen's own ``qwen_asr`` wrapper, which is what the release ships.

    ``Qwen3ForcedAligner.align`` takes the audio as a ``(waveform, rate)``
    pair, so the decoded samples never touch a file or a URL, and returns one
    item per word with seconds already converted from the 80 ms slot indices.
    """
    import torch
    from qwen_asr import Qwen3ForcedAligner

    aligner = Qwen3ForcedAligner.from_pretrained(
        str(model_dir), dtype=torch.bfloat16,
        device_map="cuda:0" if torch.cuda.is_available() else "cpu",
    )
    result = aligner.align(
        audio=(samples, SAMPLE_RATE), text=transcript, language=LANGUAGE)[0]
    return "qwen_asr.Qwen3ForcedAligner", [
        {
            "text": str(item.text),
            "start_seconds": _seconds(item.start_time),
            "end_seconds": _seconds(item.end_time),
        }
        for item in result
    ]


def _predict_with_transformers(
    model_dir: Path, samples: Any, transcript: str,
) -> tuple[str, list[dict[str, Any]]]:
    """The ``-hf`` conversion, once Transformers carries the architecture.

    Kept as a second path because the two model repositories are packaged
    differently and whichever one is installed should just work; the lane
    records which backend produced a result.
    """
    import torch
    from transformers import AutoModelForTokenClassification, AutoProcessor

    processor = AutoProcessor.from_pretrained(str(model_dir))
    model = AutoModelForTokenClassification.from_pretrained(
        str(model_dir), dtype=torch.bfloat16,
        device_map="cuda:0" if torch.cuda.is_available() else "cpu",
    )
    aligner_inputs, word_lists = processor.prepare_forced_aligner_inputs(
        audio=samples, transcript=transcript, language=LANGUAGE)
    aligner_inputs = aligner_inputs.to(model.device, model.dtype)
    with torch.inference_mode():
        outputs = model(**aligner_inputs)
    predicted = processor.decode_forced_alignment(
        logits=outputs.logits,
        input_ids=aligner_inputs["input_ids"],
        word_lists=word_lists,
        timestamp_token_id=model.config.timestamp_token_id,
    )[0]
    return "transformers.AutoModelForTokenClassification", [
        {
            "text": str(item["text"]),
            "start_seconds": _seconds(item.get("start_time")),
            "end_seconds": _seconds(item.get("end_time")),
        }
        for item in predicted
    ]


def predict(
    model_dir: Path, samples: Any, transcript: str,
) -> tuple[str, list[dict[str, Any]]]:
    try:
        import qwen_asr  # noqa: F401
    except ImportError:
        return _predict_with_transformers(model_dir, samples, transcript)
    return _predict_with_package(model_dir, samples, transcript)


def map_words_to_lines(
    predicted: Sequence[dict[str, Any]], owners: Sequence[int],
    expected_words: Sequence[str],
) -> dict[int, list[dict[str, Any]]]:
    """Attribute each predicted word span to an authored line.

    The processor decides its own word segmentation, so a positional map is
    only safe when the counts agree. Otherwise fall back to the same monotonic
    token matcher the production sync stage uses, which cannot reorder words
    and therefore cannot move a span to an earlier line.
    """
    per_line: dict[int, list[dict[str, Any]]] = {}
    if len(predicted) == len(expected_words):
        for owner, item in zip(owners, predicted):
            per_line.setdefault(owner, []).append(item)
        return per_line

    expected_tokens: list[str] = []
    expected_owner: list[int] = []
    for owner, word in zip(owners, expected_words):
        for token in lyric_align.normalize_tokens(word):
            expected_tokens.append(token)
            expected_owner.append(owner)
    predicted_tokens: list[str] = []
    predicted_owner: list[int] = []
    for index, item in enumerate(predicted):
        for token in lyric_align.normalize_tokens(str(item.get("text", ""))):
            predicted_tokens.append(token)
            predicted_owner.append(index)
    pairs = lyric_align.align_tokens(expected_tokens, predicted_tokens)
    seen: set[tuple[int, int]] = set()
    for reference_index, evidence_index in pairs:
        owner = expected_owner[reference_index]
        item_index = predicted_owner[evidence_index]
        if (owner, item_index) in seen:
            continue
        seen.add((owner, item_index))
        per_line.setdefault(owner, []).append(predicted[item_index])
    return per_line


def _seconds(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    result = float(value)
    return result if math.isfinite(result) else None


def align(
    audio: Path, reference_text: str, *, audio_duration: float,
    model_dir: Path,
) -> dict[str, Any]:
    if audio_duration > MAX_AUDIO_SECONDS:
        raise AnalysisValidationError(
            f"track is {audio_duration:.1f} s; the aligner accepts "
            f"{MAX_AUDIO_SECONDS:.0f} s in one pass")
    if not model_dir.is_dir():
        raise AnalysisValidationError(
            f"Qwen3-ForcedAligner weights are not installed at {model_dir}; "
            f"set {MODEL_DIR_ENVIRONMENT} or --model-dir")
    started = time.monotonic()
    lines = alignable_lines(reference_text)
    if not lines:
        raise AnalysisValidationError("authored lyrics carry no alignable lines")
    transcript, owners = build_transcript(lines)
    expected_words = transcript.split()
    samples = load_audio(audio)
    backend, predicted = predict(model_dir, samples, transcript)
    per_line = map_words_to_lines(predicted, owners, expected_words)

    lane_lines: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []
    for position, line in enumerate(lines):
        words = [word for word in per_line.get(position, [])
                 if word["start_seconds"] is not None
                 and word["end_seconds"] is not None]
        record: dict[str, Any] = {
            "reference_line_index": int(line["index"]),
            "line_position": position,
            "kind": line["kind"],
            "text": line["display"],
            "start_seconds": None,
            "end_seconds": None,
            "score": None,
            "status": "no_alignment",
            "uncertain": True,
            "word_alignments": words,
        }
        if words:
            start = max(0.0, min(float(word["start_seconds"])
                                 for word in words) - CUE_LEAD_SECONDS)
            end = min(audio_duration, max(float(word["end_seconds"])
                                          for word in words) + CUE_TAIL_SECONDS)
            if end > start:
                record["start_seconds"] = start
                record["end_seconds"] = end
                record["status"] = "aligned"
                record["uncertain"] = False
            else:
                record["status"] = "collapsed"
        if record["start_seconds"] is None:
            unresolved.append({
                "reference_line_index": record["reference_line_index"],
                "text": record["text"],
                "reason": record["status"],
            })
        lane_lines.append(record)

    violations: list[dict[str, Any]] = []
    previous: dict[str, Any] | None = None
    for line in lane_lines:
        if line["start_seconds"] is None:
            continue
        if previous is not None and line["start_seconds"] < previous["start_seconds"]:
            violations.append({
                "previous_reference_line_index": previous["reference_line_index"],
                "reference_line_index": line["reference_line_index"],
                "previous_start_seconds": previous["start_seconds"],
                "start_seconds": line["start_seconds"],
            })
        previous = line

    settings = {
        "language": LANGUAGE,
        "sample_rate": SAMPLE_RATE,
        "backend": backend,
        "max_audio_seconds": MAX_AUDIO_SECONDS,
        "cue_lead_seconds": CUE_LEAD_SECONDS,
        "cue_tail_seconds": CUE_TAIL_SECONDS,
        "whole_song_single_pass": True,
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
        "order_violations": violations,
        "statistics": {
            "authored_alignable_lines": len(lines),
            "timed_lines": len(timed),
            "aligned_lines": sum(1 for line in lane_lines
                                 if line["status"] == "aligned"),
            "weak_lines": 0,
            "unresolved_lines": len(unresolved),
            "predicted_words": len(predicted),
            "transcript_words": len(expected_words),
            "order_violations": len(violations),
            "past_package_soft_limit": audio_duration > PACKAGE_SOFT_LIMIT_SECONDS,
            "runtime_seconds": time.monotonic() - started,
        },
        "provenance": {
            "adapter": "tools/lyrics_research/qwen_forced_aligner.py",
            "lane_version": LANE_VERSION,
            "backend": backend,
            "model": str(model_dir),
            "model_name": model_dir.name,
            "request_settings": settings,
            "request_settings_sha256": canonical_sha256(settings),
        },
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--duration", required=True, type=float)
    parser.add_argument("--reference-file", type=Path,
                        help="authored lyrics; defaults to the embedded tag")
    parser.add_argument("--model-dir", type=Path)
    args = parser.parse_args(argv)
    try:
        audio_duration = validate_duration(args.duration)
        if not args.audio.is_file():
            raise AnalysisValidationError("audio file does not exist")
        if args.reference_file is not None:
            reference_text = args.reference_file.read_text(encoding="utf-8")
        else:
            import external_analysis

            found = external_analysis.discover_reference_lyrics(args.audio)
            if found is None:
                raise AnalysisValidationError("no authored lyrics for this track")
            reference_text = found["text"]
        result = align(
            args.audio, reference_text, audio_duration=audio_duration,
            model_dir=resolve_model_dir(args.model_dir),
        )
        atomic_write_json(args.output, result)
    except (AnalysisValidationError, OSError, RuntimeError, ValueError,
            KeyError, json.JSONDecodeError) as error:
        print(f"Qwen forced alignment failed: {error}", file=sys.stderr)
        return 1
    stats = result["statistics"]
    print(f"Qwen whole-song alignment timed {stats['timed_lines']}/"
          f"{stats['authored_alignable_lines']} authored lines "
          f"({stats['unresolved_lines']} unresolved) from "
          f"{stats['predicted_words']} predicted words in "
          f"{stats['runtime_seconds']:.1f} s.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
