"""Paths, tracks, excerpt windows and done markers for the MiMo benchmark.

Source MP3s are read-only. Everything this harness produces lands under the
gitignored ``build/mimo-bench/``. No audio output device is ever initialized:
the only audio work is an ffmpeg decode into a excerpt file.
"""

from __future__ import annotations

import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from analysis_io import (  # noqa: E402  (path shim must precede the import)
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    read_json,
    sha256_file,
)

BENCH_ROOT = ROOT / "build" / "mimo-bench"
AUDIO_ROOT = BENCH_ROOT / "audio"
RESULTS_ROOT = BENCH_ROOT / "results"
SCORES_ROOT = BENCH_ROOT / "scores"

# The lyric-timing benchmark's workspace is where the adjudicated tracks'
# measured analysis and aligned lyric lanes already live. Reusing it is the
# whole reason those two tracks were chosen.
LYRICS_RESEARCH_ROOT = ROOT / "build" / "lyrics-research-v2"
ADJUDICATION_PATH = LYRICS_RESEARCH_ROOT / "ground_truth_adjudication.json"
GROUND_TRUTH_PATH = Path(__file__).resolve().parent / "ground_truth" / "tracks.json"

MUSIC = Path.home() / "Music"

EXCERPT_SECONDS = 100.0


@dataclass(frozen=True)
class Track:
    """One benchmark track and the single 100 s excerpt every axis reuses.

    ``excerpt_start_seconds`` was chosen to maximize the number of aligned
    lyric lines inside the window, because lyric positioning is the one
    dimension with second-accurate truth.
    """

    slug: str
    audio: Path
    duration_seconds: float
    excerpt_start_seconds: float
    role: str  # "primary" runs the whole matrix; "replication" runs two cells
    aligned_lines_in_excerpt: int
    note: str = ""

    @property
    def excerpt_end_seconds(self) -> float:
        return self.excerpt_start_seconds + EXCERPT_SECONDS

    @property
    def workspace(self) -> Path:
        return LYRICS_RESEARCH_ROOT / "workspace" / self.slug


TRACKS: tuple[Track, ...] = (
    Track(
        slug="shut-up-cat",
        audio=MUSIC / "Shut Up Cat.mp3",
        duration_seconds=160.16,
        excerpt_start_seconds=30.0,
        role="primary",
        aligned_lines_in_excerpt=25,
        note="English, dense vocal, 25 of 33 aligned lines fall inside the window",
    ),
    Track(
        slug="constellation-whale",
        audio=MUSIC / "Constellation Whale (Glitchpop).mp3",
        duration_seconds=114.84,
        excerpt_start_seconds=0.0,
        role="replication",
        aligned_lines_in_excerpt=15,
        note="all 15 aligned lines inside the window; two of them operator-adjudicated",
    ),
)


def track_by_slug(slug: str) -> Track:
    for track in TRACKS:
        if track.slug == slug:
            return track
    raise AnalysisValidationError(f"unknown track slug: {slug}")


def primary_track() -> Track:
    return track_by_slug("shut-up-cat")


def replication_track() -> Track:
    return track_by_slug("constellation-whale")


# ---------------------------------------------------------------------------
# Result directories and done markers
# ---------------------------------------------------------------------------


def cell_slug(cell_id: str) -> str:
    """A filesystem-safe directory name for a ``a/b/c/d`` cell id."""
    return cell_id.replace("/", "__")


def cell_dir(cell_id: str, repeat: int) -> Path:
    return RESULTS_ROOT / cell_slug(cell_id) / f"r{repeat:02d}"


def call_path(cell_id: str, repeat: int, call_index: int) -> Path:
    return cell_dir(cell_id, repeat) / f"call_{call_index:03d}.json"


def done_marker(cell_id: str, repeat: int) -> Path:
    return cell_dir(cell_id, repeat) / "done.json"


def cell_state(cell_id: str, repeat: int) -> dict[str, Any] | None:
    """The done marker, or ``None`` while the (cell, repeat) still has to run.

    Written last, after every call file is durable, so a suspend or a kill
    leaves the unit pending and a rerun resumes at the first missing call
    rather than repeating the ones already paid for.
    """
    marker = done_marker(cell_id, repeat)
    if not marker.is_file():
        return None
    try:
        state = read_json(marker)
    except (OSError, json.JSONDecodeError):
        return None
    return state if isinstance(state, dict) else None


def write_done(cell_id: str, repeat: int, *, status: str, **payload: Any) -> None:
    atomic_write_json(done_marker(cell_id, repeat), {
        "cell": cell_id,
        "repeat": repeat,
        "status": status,
        "completed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        **payload,
    })


def completed_calls(cell_id: str, repeat: int) -> set[int]:
    """Call indices already stored for this unit, so a resume skips them."""
    directory = cell_dir(cell_id, repeat)
    if not directory.is_dir():
        return set()
    found: set[int] = set()
    for path in directory.glob("call_*.json"):
        try:
            found.add(int(path.stem.split("_")[1]))
        except (IndexError, ValueError):
            continue
    return found


def stored_calls(cell_id: str, repeat: int) -> list[dict[str, Any]]:
    directory = cell_dir(cell_id, repeat)
    if not directory.is_dir():
        return []
    records: list[dict[str, Any]] = []
    for path in sorted(directory.glob("call_*.json")):
        try:
            record = read_json(path)
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(record, dict):
            records.append(record)
    return records


def repeats_present(cell_id: str) -> list[int]:
    directory = RESULTS_ROOT / cell_slug(cell_id)
    if not directory.is_dir():
        return []
    found: list[int] = []
    for child in sorted(directory.iterdir()):
        if child.is_dir() and child.name.startswith("r"):
            try:
                found.append(int(child.name[1:]))
            except ValueError:
                continue
    return found


def iter_result_dirs() -> Iterator[Path]:
    if RESULTS_ROOT.is_dir():
        yield from sorted(RESULTS_ROOT.iterdir())


__all__ = [
    "ADJUDICATION_PATH",
    "AUDIO_ROOT",
    "AnalysisValidationError",
    "BENCH_ROOT",
    "EXCERPT_SECONDS",
    "GROUND_TRUTH_PATH",
    "LYRICS_RESEARCH_ROOT",
    "RESULTS_ROOT",
    "ROOT",
    "SCORES_ROOT",
    "TOOLS",
    "TRACKS",
    "Track",
    "atomic_write_json",
    "call_path",
    "canonical_sha256",
    "cell_dir",
    "cell_slug",
    "cell_state",
    "completed_calls",
    "done_marker",
    "iter_result_dirs",
    "primary_track",
    "read_json",
    "repeats_present",
    "replication_track",
    "sha256_file",
    "stored_calls",
    "track_by_slug",
    "write_done",
]
