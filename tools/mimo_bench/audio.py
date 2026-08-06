"""Excerpt materialization. ffmpeg only; no audio device is ever opened.

Every chunking of a track is cut from **one** canonical 100 s excerpt file, not
from the source MP3 separately, so the chunk-granularity axis really does vary
only the cut points. The canonical excerpt is re-encoded once (mono, fixed
bitrate) so it is byte-deterministic for a given source and window; the chunks
are then stream copies of it.

Source MP3s are opened read-only and their tags are never touched. Everything
written lands under the gitignored ``build/mimo-bench/audio/``.
"""

from __future__ import annotations

import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence

from .bench_io import (
    AUDIO_ROOT,
    EXCERPT_SECONDS,
    Track,
    atomic_write_json,
    read_json,
    sha256_file,
)
from .matrix import Chunking, chunk_spans

EXCERPT_BITRATE = "192k"
EXCERPT_SAMPLE_RATE = "44100"
EXCERPT_CHANNELS = "1"
MANIFEST_NAME = "manifest.json"


class AudioPreparationError(RuntimeError):
    """The excerpt or one of its chunks could not be produced."""


@dataclass(frozen=True)
class ChunkFile:
    index: int
    path: Path
    start_seconds: float
    end_seconds: float
    sha256: str
    bytes: int
    #: What ffprobe reports, which is not exactly the nominal span: a stream
    #: copy can only cut on an MP3 frame boundary (~26 ms), so a chunk runs a
    #: few tens of milliseconds long and consecutive chunks overlap slightly.
    #: Recorded rather than hidden, because it bounds how precisely a chunked
    #: condition could possibly place a lyric.
    probed_seconds: float | None = None


def excerpt_path(track: Track) -> Path:
    return AUDIO_ROOT / track.slug / "excerpt.mp3"


def chunk_dir(track: Track, chunking: Chunking) -> Path:
    return AUDIO_ROOT / track.slug / chunking.id


def manifest_path(track: Track) -> Path:
    return AUDIO_ROOT / track.slug / MANIFEST_NAME


def _ffmpeg() -> str:
    binary = shutil.which("ffmpeg")
    if binary is None:
        raise AudioPreparationError("ffmpeg is not on PATH")
    return binary


def _run(command: Sequence[str]) -> str:
    completed = subprocess.run(
        list(command), check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        raise AudioPreparationError(
            f"{command[0]} exited {completed.returncode}: "
            f"{(completed.stderr or '').strip()[-400:]}")
    return completed.stdout


def probe_seconds(path: Path) -> float | None:
    """The file's real duration, or None when ffprobe is unavailable."""
    binary = shutil.which("ffprobe")
    if binary is None:
        return None
    try:
        output = _run([
            binary, "-v", "error", "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1", str(path),
        ])
    except AudioPreparationError:
        return None
    try:
        return float(output.strip())
    except ValueError:
        return None


def prepare_excerpt(track: Track, *, force: bool = False) -> Path:
    """Cut and re-encode the one canonical 100 s excerpt for a track."""
    destination = excerpt_path(track)
    if destination.is_file() and not force:
        return destination
    if not track.audio.is_file():
        raise AudioPreparationError(f"source audio is missing: {track.audio}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp.mp3")
    _run([
        _ffmpeg(), "-y", "-hide_banner", "-loglevel", "error",
        "-ss", f"{track.excerpt_start_seconds:.3f}",
        "-i", str(track.audio),
        "-t", f"{EXCERPT_SECONDS:.3f}",
        "-vn", "-map_metadata", "-1",
        "-ac", EXCERPT_CHANNELS, "-ar", EXCERPT_SAMPLE_RATE,
        "-c:a", "libmp3lame", "-b:a", EXCERPT_BITRATE,
        str(temporary),
    ])
    temporary.replace(destination)
    return destination


def prepare_chunks(
    track: Track, chunking: Chunking, *, force: bool = False,
) -> list[ChunkFile]:
    """Split the canonical excerpt into one chunking's files, by stream copy."""
    source = prepare_excerpt(track, force=force)
    directory = chunk_dir(track, chunking)
    directory.mkdir(parents=True, exist_ok=True)
    spans = chunk_spans(track, chunking)
    files: list[ChunkFile] = []
    for span in spans:
        path = directory / f"chunk_{span.index:03d}.mp3"
        if force or not path.is_file():
            temporary = path.with_name(f".{path.name}.tmp.mp3")
            offset = span.start_seconds - track.excerpt_start_seconds
            _run([
                _ffmpeg(), "-y", "-hide_banner", "-loglevel", "error",
                "-ss", f"{offset:.3f}", "-i", str(source),
                "-t", f"{span.seconds:.3f}",
                "-c", "copy", str(temporary),
            ])
            temporary.replace(path)
        files.append(ChunkFile(
            index=span.index,
            path=path,
            start_seconds=span.start_seconds,
            end_seconds=span.end_seconds,
            sha256=sha256_file(path),
            bytes=path.stat().st_size,
            probed_seconds=probe_seconds(path),
        ))
    return files


def prepare_track(
    track: Track, chunkings: Sequence[Chunking], *, force: bool = False,
) -> dict[str, Any]:
    excerpt = prepare_excerpt(track, force=force)
    manifest: dict[str, Any] = {
        "track": track.slug,
        "source": str(track.audio),
        "source_sha256": sha256_file(track.audio),
        "excerpt": {
            "path": str(excerpt),
            "sha256": sha256_file(excerpt),
            "bytes": excerpt.stat().st_size,
            "start_seconds": track.excerpt_start_seconds,
            "seconds": EXCERPT_SECONDS,
            "bitrate": EXCERPT_BITRATE,
            "sample_rate": EXCERPT_SAMPLE_RATE,
            "channels": EXCERPT_CHANNELS,
        },
        "chunkings": {},
    }
    for chunking in chunkings:
        files = prepare_chunks(track, chunking, force=force)
        probed = [chunk.probed_seconds for chunk in files
                  if chunk.probed_seconds is not None]
        manifest["chunkings"][chunking.id] = {
            "chunks": [
                {
                    "index": chunk.index,
                    "path": str(chunk.path),
                    "sha256": chunk.sha256,
                    "bytes": chunk.bytes,
                    "start_seconds": chunk.start_seconds,
                    "end_seconds": chunk.end_seconds,
                    "probed_seconds": chunk.probed_seconds,
                }
                for chunk in files
            ],
            "nominal_total_seconds": sum(
                chunk.end_seconds - chunk.start_seconds for chunk in files),
            "probed_total_seconds": sum(probed) if probed else None,
            # The excess is duplicated audio at the cut points, not extra
            # content: a stream copy cannot split inside an MP3 frame.
            "frame_boundary_excess_seconds": (
                sum(probed) - EXCERPT_SECONDS if probed else None),
        }
    atomic_write_json(manifest_path(track), manifest)
    return manifest


def load_manifest(track: Track) -> dict[str, Any] | None:
    path = manifest_path(track)
    if not path.is_file():
        return None
    try:
        document = read_json(path)
    except (OSError, ValueError):
        return None
    return document if isinstance(document, dict) else None


def chunk_file(track: Track, chunking: Chunking, index: int) -> Path:
    return chunk_dir(track, chunking) / f"chunk_{index:03d}.mp3"


__all__ = [
    "AudioPreparationError",
    "ChunkFile",
    "EXCERPT_BITRATE",
    "chunk_dir",
    "chunk_file",
    "excerpt_path",
    "load_manifest",
    "manifest_path",
    "prepare_chunks",
    "prepare_excerpt",
    "prepare_track",
]
