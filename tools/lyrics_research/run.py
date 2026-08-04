#!/usr/bin/env python3
"""Resumable driver for the trimmed lyric-timing benchmark.

One (track, method) chunk per invocation. Results land atomically under the
gitignored ``build/lyrics-research-v2/results/<track>/<method>/`` and a done
marker is written last, so a machine that suspends, a killed process, or a
power loss costs at most the chunk that was in flight. A rerun repeats that
chunk and skips everything already finished.

    tools/lyrics_research/run.py list        # matrix and status
    tools/lyrics_research/run.py next        # run exactly one pending chunk
    tools/lyrics_research/run.py run --track constellation-whale --method qwen
    tools/lyrics_research/run.py all         # every pending chunk, in order

Chunk 0 of every track is ``baseline``: today's pipeline
(``tools/external_analysis.py assist --mode lyrics``), which is both the
evidence input for the anchor lane and the agreement reference for the
scoreboard. Where the application has already analysed a track, its existing
``build/analysis/<hash>/`` artifacts are copied in first, so the cache-aware
assist reuses them instead of re-running Whisper.

Nothing here initializes an audio output device: the harness decodes audio and
writes JSON. Source MP3s are opened read-only and their tags are never
stripped.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

from research_io import (  # noqa: E402
    METHODS,
    RESEARCH_ROOT,
    ROOT,
    TOOLS,
    AnalysisValidationError,
    Track,
    atomic_write_json,
    chunk_dir,
    chunk_state,
    chunks,
    ensure_reference_cache,
    read_json,
    seed_analysis_dir,
    track_by_slug,
    workspace_dir,
    write_done,
)


HERE = Path(__file__).resolve().parent
DEFAULT_MMS_PYTHON = (
    Path.home() / ".local/share/musializer/lyrics-align/.venv/bin/python")
DEFAULT_QWEN_PYTHON = RESEARCH_ROOT / ".venv" / "bin" / "python"
# Whisper on a long track is the slow step; the assist helper enforces its own
# per-stage bounds inside this one.
DEFAULT_TIMEOUT_SECONDS = 10800.0

SEED_ARTIFACTS = (
    "measured.json",
    "lyrics.whisper.json",
    "lyrics.whisper.whisper.raw.json",
    "lyrics.sync.json",
    "lyrics.aligned.json",
)
BASELINE_ARTIFACTS = (
    "lyrics.whisper.json",
    "lyrics.sync.json",
    "lyrics.aligned.json",
    "assist-manifest.json",
)


class ChunkFailure(RuntimeError):
    """A chunk could not produce its result; recorded, never raised past main."""


def _copy(source: Path, destination: Path) -> None:
    """Same-directory temporary plus rename, so a reader never sees a half copy."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp")
    shutil.copyfile(source, temporary)
    os.replace(temporary, destination)


def _child_environment() -> dict[str, str]:
    """Drop credential-like variables; no chunk in this harness needs one."""
    blocked = ("API_KEY", "TOKEN", "SECRET", "PASSWORD", "OPENROUTER")
    return {
        key: value for key, value in os.environ.items()
        if not any(marker in key.upper() for marker in blocked)
    }


def _run(
    command: Sequence[str], log: Path, *, timeout: float,
) -> subprocess.CompletedProcess[str]:
    log.parent.mkdir(parents=True, exist_ok=True)
    completed = subprocess.run(
        list(command), check=False, capture_output=True, text=True,
        timeout=timeout, env=_child_environment(), cwd=str(ROOT),
    )
    with log.open("a", encoding="utf-8") as handle:
        handle.write(f"$ {' '.join(command)}\n")
        handle.write(completed.stdout or "")
        handle.write(completed.stderr or "")
        handle.write(f"[exit {completed.returncode}]\n\n")
    return completed


# --------------------------------------------------------------------------
# Chunks
# --------------------------------------------------------------------------


def run_baseline(track: Track, options: argparse.Namespace) -> dict[str, Any]:
    """Today's pipeline, in a private workspace, seeded from any existing run."""
    destination = chunk_dir(track, "baseline")
    destination.mkdir(parents=True, exist_ok=True)
    workspace = workspace_dir(track)
    workspace.mkdir(parents=True, exist_ok=True)

    seed = seed_analysis_dir(track)
    seeded: list[str] = []
    if seed is not None:
        for name in SEED_ARTIFACTS:
            source = seed / name
            if source.is_file() and not (workspace / name).is_file():
                _copy(source, workspace / name)
                seeded.append(name)

    command = [
        sys.executable, str(TOOLS / "external_analysis.py"), "assist",
        str(track.audio), str(workspace),
        "--duration", f"{track.duration_seconds:.6f}",
        "--mode", "lyrics",
    ]
    completed = _run(command, destination / "run.log", timeout=options.timeout)
    if completed.returncode != 0:
        raise ChunkFailure(
            f"assist exited {completed.returncode}: "
            f"{(completed.stderr or '').strip()[-500:]}")

    present: list[str] = []
    for name in BASELINE_ARTIFACTS:
        source = workspace / name
        if source.is_file():
            _copy(source, destination / name)
            present.append(name)
    lane = destination / "lyrics.aligned.json"
    if not lane.is_file():
        lane = destination / "lyrics.sync.json"
    if not lane.is_file():
        raise ChunkFailure("assist produced neither an aligned nor a sync lane")
    _copy(lane, destination / "result.json")
    return {
        "command": command,
        "seeded_from": str(seed) if seed is not None else None,
        "seeded_artifacts": seeded,
        "artifacts": present,
        "result_source": lane.name,
    }


def _whisper_evidence(track: Track) -> Path:
    candidates = [
        chunk_dir(track, "baseline") / "lyrics.whisper.json",
        workspace_dir(track) / "lyrics.whisper.json",
    ]
    seed = seed_analysis_dir(track)
    if seed is not None:
        candidates.append(seed / "lyrics.whisper.json")
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise ChunkFailure(
        "no Whisper evidence for this track; run its baseline chunk first")


def run_anchor_block_mms(
    track: Track, options: argparse.Namespace,
) -> dict[str, Any]:
    destination = chunk_dir(track, "anchor-block-mms")
    destination.mkdir(parents=True, exist_ok=True)
    interpreter = options.mms_python
    if not interpreter.is_file():
        raise ChunkFailure(
            f"the alignment runtime is not installed at {interpreter}")
    whisper = _whisper_evidence(track)
    command = [
        str(interpreter), str(HERE / "anchor_block_mms.py"),
        str(track.audio), str(whisper), str(destination / "result.json"),
        "--duration", f"{track.duration_seconds:.6f}",
        "--reference-file", str(options.reference_file),
        "--max-block-seconds", f"{options.max_block_seconds:.3f}",
    ]
    if options.no_interior_stars:
        command.append("--no-interior-stars")
    completed = _run(command, destination / "run.log", timeout=options.timeout)
    if completed.returncode != 0:
        raise ChunkFailure(
            f"anchor/block alignment exited {completed.returncode}: "
            f"{(completed.stderr or '').strip()[-500:]}")
    return {
        "command": command,
        "whisper_evidence": str(whisper),
        "summary": (completed.stdout or "").strip(),
    }


def run_qwen(track: Track, options: argparse.Namespace) -> dict[str, Any]:
    destination = chunk_dir(track, "qwen")
    destination.mkdir(parents=True, exist_ok=True)
    interpreter = options.qwen_python
    if not interpreter.is_file():
        raise ChunkFailure(
            f"the research runtime is not installed at {interpreter}")
    command = [
        str(interpreter), str(HERE / "qwen_forced_aligner.py"),
        str(track.audio), str(destination / "result.json"),
        "--duration", f"{track.duration_seconds:.6f}",
        "--reference-file", str(options.reference_file),
    ]
    if options.qwen_model_dir is not None:
        command.extend(["--model-dir", str(options.qwen_model_dir)])
    completed = _run(command, destination / "run.log", timeout=options.timeout)
    if completed.returncode != 0:
        raise ChunkFailure(
            f"Qwen alignment exited {completed.returncode}: "
            f"{(completed.stderr or '').strip()[-500:]}")
    return {
        "command": command,
        "summary": (completed.stdout or "").strip(),
    }


RUNNERS = {
    "baseline": run_baseline,
    "qwen": run_qwen,
    "anchor-block-mms": run_anchor_block_mms,
}


def execute(track: Track, method: str, options: argparse.Namespace) -> bool:
    """Run one chunk. Returns True when it produced a result."""
    destination = chunk_dir(track, method)
    destination.mkdir(parents=True, exist_ok=True)
    print(f"[{track.slug}/{method}] starting", flush=True)
    started = time.monotonic()
    reference: dict[str, Any] | None = None
    detail: dict[str, Any] = {}
    status = "ok"
    error: str | None = None
    try:
        reference = ensure_reference_cache(track)
        options.reference_file = (
            RESEARCH_ROOT / "results" / track.slug / "reference.txt")
        detail = RUNNERS[method](track, options)
        result = read_json(destination / "result.json")
        if not isinstance(result, dict) or not isinstance(result.get("lines"), list):
            raise ChunkFailure("the chunk result is not a lane document")
    except (ChunkFailure, AnalysisValidationError, OSError, ValueError,
            subprocess.TimeoutExpired, json.JSONDecodeError) as failure:
        status = "failed"
        error = f"{type(failure).__name__}: {failure}"
    runtime = time.monotonic() - started

    atomic_write_json(destination / "chunk.json", {
        "track": track.slug,
        "method": method,
        "status": status,
        "error": error,
        "audio": str(track.audio),
        "duration_seconds": track.duration_seconds,
        "gating": track.gating,
        "note": track.note,
        "reference": None if reference is None else {
            "source": reference["source"], "sha256": reference["sha256"],
        },
        "runtime_seconds": runtime,
        "detail": detail,
    })
    # The done marker is written last and only after chunk.json is durable, so
    # its presence means the chunk really finished.
    write_done(track, method, status=status, runtime_seconds=runtime,
               error=error)
    print(f"[{track.slug}/{method}] {status} in {runtime:.1f} s"
          + (f" — {error}" if error else ""), flush=True)
    return status == "ok"


def pending(options: argparse.Namespace) -> list[tuple[Track, str]]:
    result: list[tuple[Track, str]] = []
    for track, method in chunks():
        if options.track and track.slug != options.track:
            continue
        if options.method and method != options.method:
            continue
        state = chunk_state(track, method)
        if state is None:
            result.append((track, method))
        elif state.get("status") != "ok" and options.retry_failed:
            result.append((track, method))
    return result


def command_list(options: argparse.Namespace) -> int:
    rows = []
    for track, method in chunks():
        state = chunk_state(track, method)
        if state is None:
            status, runtime = "pending", ""
        else:
            status = str(state.get("status", "?"))
            seconds = state.get("runtime_seconds")
            runtime = f"{float(seconds):.1f}s" if isinstance(seconds, (int, float)) else ""
        rows.append((track.slug, method, status, runtime,
                     "" if track.gating else "non-gating"))
    widths = [max(len(row[column]) for row in rows) for column in range(5)]
    header = ("track", "method", "status", "runtime", "")
    widths = [max(widths[column], len(header[column])) for column in range(5)]
    line = "  ".join(header[column].ljust(widths[column]) for column in range(5))
    print(line.rstrip())
    print("  ".join("-" * widths[column] for column in range(5)).rstrip())
    for row in rows:
        print("  ".join(row[column].ljust(widths[column])
                        for column in range(5)).rstrip())
    remaining = len(pending(options))
    print(f"\n{remaining} chunk(s) pending of {len(rows)}.")
    return 0


def command_next(options: argparse.Namespace) -> int:
    queue = pending(options)
    if not queue:
        print("No pending chunks. Every requested chunk already has a done "
              "marker; pass --retry-failed to repeat the failures.")
        return 0
    track, method = queue[0]
    return 0 if execute(track, method, options) else 1


def command_all(options: argparse.Namespace) -> int:
    queue = pending(options)
    if not queue:
        print("No pending chunks.")
        return 0
    failures = 0
    for track, method in queue:
        if not execute(track, method, options):
            failures += 1
    print(f"{len(queue) - failures}/{len(queue)} chunks produced a result.")
    return 1 if failures else 0


def command_run(options: argparse.Namespace) -> int:
    if not options.track or not options.method:
        print("run needs both --track and --method", file=sys.stderr)
        return 2
    track = track_by_slug(options.track)
    if options.method not in METHODS:
        print(f"method must be one of {', '.join(METHODS)}", file=sys.stderr)
        return 2
    state = chunk_state(track, options.method)
    if state is not None and not options.force:
        print(f"[{track.slug}/{options.method}] already done "
              f"({state.get('status')}); pass --force to repeat it.")
        return 0
    return 0 if execute(track, options.method, options) else 1


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("list", "next", "run", "all"))
    parser.add_argument("--track")
    parser.add_argument("--method", choices=METHODS)
    parser.add_argument("--force", action="store_true",
                        help="repeat a chunk that already has a done marker")
    parser.add_argument("--retry-failed", action="store_true",
                        help="treat failed chunks as pending again")
    parser.add_argument("--timeout", type=float,
                        default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--mms-python", type=Path, default=DEFAULT_MMS_PYTHON,
                        help="read-only interpreter for the installed MMS "
                             "alignment runtime")
    parser.add_argument("--qwen-python", type=Path,
                        default=DEFAULT_QWEN_PYTHON)
    parser.add_argument("--qwen-model-dir", type=Path)
    parser.add_argument("--max-block-seconds", type=float, default=90.0)
    parser.add_argument("--no-interior-stars", action="store_true")
    options = parser.parse_args(argv)
    options.reference_file = None
    try:
        if options.track:
            track_by_slug(options.track)
    except AnalysisValidationError as error:
        print(str(error), file=sys.stderr)
        return 2
    return {
        "list": command_list,
        "next": command_next,
        "run": command_run,
        "all": command_all,
    }[options.command](options)


if __name__ == "__main__":
    raise SystemExit(main())
