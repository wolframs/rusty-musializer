#!/usr/bin/env python3
"""Shared plumbing for the trimmed lyric-timing benchmark (research v2).

Two benchmark lanes are compared against today's pipeline:

``baseline``
    ``tools/external_analysis.py assist --mode lyrics`` exactly as the
    application runs it. Both the evidence input for the other lanes and the
    comparison baseline.
``qwen``
    Whole song plus complete authored lyrics through Qwen3-ForcedAligner in
    one pass, with no Whisper proposals involved.
``anchor-block-mms``
    Rare n-gram anchors between authored lyrics and the existing Whisper word
    evidence partition the song into blocks; each block's complete consecutive
    text is force-aligned with the already installed MMS_FA model.

Everything generated lands under the gitignored ``build/lyrics-research-v2/``.
Source audio is opened read-only and no audio output device is ever
initialized: this tree only decodes samples and writes JSON.
"""

from __future__ import annotations

import json
import os
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterator, Sequence

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from analysis_io import (  # noqa: E402  (path shim must precede the import)
    AnalysisValidationError,
    atomic_write_json,
    read_json,
    sha256_file,
)

RESEARCH_ROOT = ROOT / "build" / "lyrics-research-v2"
RESULTS_ROOT = RESEARCH_ROOT / "results"
WORKSPACE_ROOT = RESEARCH_ROOT / "workspace"
ANALYSIS_ROOT = ROOT / "build" / "analysis"

RESEARCH_LANE_VERSION = "musializer-research.lyric-timing/v1"

# Chunk order per track. Chunk 0 regenerates today's artifacts, because both
# other lanes consume its Whisper evidence and its aligned lane is the
# agreement reference.
METHODS = ("baseline", "qwen", "anchor-block-mms")

MUSIC = Path.home() / "Music"


@dataclass(frozen=True)
class Track:
    """One benchmark track. ``audio`` is read-only for the whole harness."""

    slug: str
    audio: Path
    duration_seconds: float
    note: str = ""
    # Non-gating tracks are reported separately: a crash or an empty lane is
    # information, not a harness failure.
    gating: bool = True
    # Authored reference-line indices that today's pipeline is known to omit,
    # and the time window a plausible recovery must land inside.
    expected_late_reference_lines: tuple[int, ...] = ()
    expected_late_window: tuple[float, float] | None = None
    extra: dict[str, Any] = field(default_factory=dict)


TRACKS: tuple[Track, ...] = (
    Track(
        slug="constellation-whale",
        audio=MUSIC / "Constellation Whale (Glitchpop).mp3",
        duration_seconds=114.84,
        note="coverage canary: Whisper loops from 90.0 s; sync omits the two "
             "outro lines",
        expected_late_reference_lines=(24, 25),
        expected_late_window=(90.0, 114.84),
    ),
    Track(
        slug="shut-up-cat",
        audio=MUSIC / "Shut Up Cat.mp3",
        duration_seconds=160.2,
    ),
    Track(
        slug="shipped-the-disposition",
        audio=MUSIC / "Shipped The Disposition.mp3",
        duration_seconds=227.9,
    ),
    Track(
        slug="a-cell-within-a-cell",
        audio=MUSIC / "A cell within a cell.MMXXVI Part 1 (DUDUDUMMM).mp3",
        duration_seconds=284.4,
        note="choir, partly non-English; stress case, reported separately",
        gating=False,
    ),
)


def track_by_slug(slug: str) -> Track:
    for track in TRACKS:
        if track.slug == slug:
            return track
    raise AnalysisValidationError(f"unknown track slug: {slug}")


def chunks() -> Iterator[tuple[Track, str]]:
    """Every (track, method) chunk in the order the driver runs them."""
    for track in TRACKS:
        for method in METHODS:
            yield track, method


# --------------------------------------------------------------------------
# Chunk directories, done markers
# --------------------------------------------------------------------------


def chunk_dir(track: Track, method: str) -> Path:
    return RESULTS_ROOT / track.slug / method


def track_dir(track: Track) -> Path:
    return RESULTS_ROOT / track.slug


def workspace_dir(track: Track) -> Path:
    return WORKSPACE_ROOT / track.slug


def done_marker(track: Track, method: str) -> Path:
    return chunk_dir(track, method) / "done.json"


def chunk_state(track: Track, method: str) -> dict[str, Any] | None:
    """The done marker, or None when the chunk still has to run.

    A marker is only written after every artifact is in place, so a run that
    is interrupted by suspend, a kill, or a power loss leaves the chunk
    pending and a rerun repeats it from the start. Nothing is lost.
    """
    marker = done_marker(track, method)
    if not marker.is_file():
        return None
    try:
        state = read_json(marker)
    except (OSError, json.JSONDecodeError):
        return None
    return state if isinstance(state, dict) else None


def write_done(
    track: Track, method: str, *, status: str, **payload: Any,
) -> None:
    atomic_write_json(done_marker(track, method), {
        "track": track.slug,
        "method": method,
        "status": status,
        "completed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        **payload,
    })


# --------------------------------------------------------------------------
# Authored lyrics
# --------------------------------------------------------------------------


def discover_reference(track: Track) -> dict[str, Any]:
    """Authored lyrics for a track, preferring the embedded ``lyrics-eng`` tag.

    Delegates to the production discovery path so the benchmark aligns exactly
    the text the application would. Tags are read, never written; the source
    MP3 is opened read-only by ffprobe.
    """
    import external_analysis

    reference = external_analysis.discover_reference_lyrics(track.audio)
    if reference is None:
        raise AnalysisValidationError(
            f"{track.slug} carries no authored lyrics (no sheet, sibling file, "
            "or embedded lyric tag)")
    return reference


def alignable_lines(reference_text: str) -> list[dict[str, Any]]:
    """Authored lines that carry alignment tokens, in authored order.

    Section headings, stage directions and delivery notes are not sung text
    and are not part of the coverage denominator.
    """
    import lyric_align

    return [line for line in lyric_align.classify_reference_lines(reference_text)
            if line["kind"] in {"lyric", "backing"} and line["tokens"]]


def cached_reference_path(track: Track) -> Path:
    return track_dir(track) / "reference.txt"


def ensure_reference_cache(track: Track) -> dict[str, Any]:
    """Materialize the authored text beside the results so lanes can reuse it.

    The MMS and Qwen lanes run under their own virtual environments. Handing
    them a file avoids re-probing the MP3 from three processes and pins every
    lane to the byte-identical reference.
    """
    reference = discover_reference(track)
    path = cached_reference_path(track)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".txt.tmp")
    temporary.write_text(reference["text"], encoding="utf-8")
    os.replace(temporary, path)
    atomic_write_json(track_dir(track) / "reference.json", {
        "track": track.slug,
        "audio": str(track.audio),
        "audio_sha256": sha256_file(track.audio),
        "duration_seconds": track.duration_seconds,
        "source": reference["source"],
        "sha256": reference["sha256"],
        "alignable_lines": len(alignable_lines(reference["text"])),
    })
    return reference


def load_reference_text(track: Track) -> str:
    path = cached_reference_path(track)
    if path.is_file():
        return path.read_text(encoding="utf-8")
    return discover_reference(track)["text"]


# --------------------------------------------------------------------------
# Seeding from the application's own analysis cache
# --------------------------------------------------------------------------


def seed_analysis_dir(track: Track) -> Path | None:
    """An existing ``build/analysis/<hash>/`` workspace for this exact audio.

    The application names those directories from its own project hash, which
    this harness has no way to recompute. Matching on the recorded audio
    SHA-256 finds the canary's existing artifacts without hard-coding a hash,
    and returns None for a track that has never been analyzed.
    """
    if not ANALYSIS_ROOT.is_dir():
        return None
    audio_sha = sha256_file(track.audio)
    for candidate in sorted(ANALYSIS_ROOT.iterdir()):
        measured = candidate / "measured.json"
        if not measured.is_file():
            continue
        try:
            document = read_json(measured)
        except (OSError, json.JSONDecodeError):
            continue
        if document.get("audio", {}).get("sha256") == audio_sha:
            return candidate
    return None


# --------------------------------------------------------------------------
# Small shared numerics
# --------------------------------------------------------------------------


def median(values: Sequence[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return float(ordered[middle])
    return float((ordered[middle - 1] + ordered[middle]) / 2.0)


__all__ = [
    "ANALYSIS_ROOT",
    "AnalysisValidationError",
    "METHODS",
    "RESEARCH_LANE_VERSION",
    "RESEARCH_ROOT",
    "RESULTS_ROOT",
    "ROOT",
    "TOOLS",
    "TRACKS",
    "Track",
    "WORKSPACE_ROOT",
    "alignable_lines",
    "atomic_write_json",
    "cached_reference_path",
    "chunk_dir",
    "chunk_state",
    "chunks",
    "discover_reference",
    "done_marker",
    "ensure_reference_cache",
    "load_reference_text",
    "median",
    "read_json",
    "seed_analysis_dir",
    "sha256_file",
    "track_by_slug",
    "track_dir",
    "workspace_dir",
    "write_done",
]
