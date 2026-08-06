#!/usr/bin/env python3
"""Resumable driver for the MiMo v2.5 description benchmark. Dry run by default.

    tools/mimo_bench/run.py plan            # the whole design, requests and cost
    tools/mimo_bench/run.py prepare         # cut the excerpts (ffmpeg, no network)
    tools/mimo_bench/run.py list            # what has run and what is pending
    tools/mimo_bench/run.py next --live     # exactly one pending unit
    tools/mimo_bench/run.py all --live      # every pending unit, in order
    tools/mimo_bench/run.py run --cell <id> --repeat 1 --live
    tools/mimo_bench/run.py score           # offline scoring of whatever is stored

**Nothing opens a socket without ``--live``**, and ``--live`` additionally
requires the confirmation variable::

    MIMO_BENCH_LIVE=yes-send-audio-to-openrouter

Two separate gates, because this harness uploads the operator's music to a
third party: `TC-SEMANTIC` in ``docs/ASSIST_PROVIDER_CONTRACTS.md`` is an
``audio-leaves-machine`` contract and needs a per-job confirmation. A flag on
its own is one typo away from an accident.

The resume unit is one (cell, repeat), and inside it each call is stored the
moment it returns. A machine that suspends mid-cell loses at most the single
request that was in flight.

No audio output device is opened anywhere in this file: the only audio work is
reading excerpt bytes from disk, and ``prepare`` shelling out to ffmpeg.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable, Sequence

if __package__ in (None, ""):  # invoked as a script rather than as a module
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    __package__ = "mimo_bench"

from . import audio, cost, matrix, prompts, report, request_build, schema  # noqa: E402
from .bench_io import (  # noqa: E402
    BENCH_ROOT,
    EXCERPT_SECONDS,
    TRACKS,
    atomic_write_json,
    call_path,
    cell_dir,
    cell_state,
    completed_calls,
    stored_calls,
    track_by_slug,
    write_done,
)
from . import ground_truth, scorers  # noqa: E402

LIVE_CONFIRMATION_VARIABLE = "MIMO_BENCH_LIVE"
LIVE_CONFIRMATION_VALUE = "yes-send-audio-to-openrouter"
TRANSIENT_STATUSES = frozenset((429, 500, 502, 503, 504))
DEFAULT_MAX_ATTEMPTS = 3

Transport = Callable[..., Any]


class BenchFailure(RuntimeError):
    """One call could not be produced; recorded, never raised past main."""


# ---------------------------------------------------------------------------
# The two gates
# ---------------------------------------------------------------------------


def live_permitted(argv_live: bool, environment: dict[str, str] | None = None) -> tuple[bool, str]:
    env = os.environ if environment is None else environment
    if not argv_live:
        return False, "dry run (pass --live to send requests)"
    value = env.get(LIVE_CONFIRMATION_VARIABLE)
    if value != LIVE_CONFIRMATION_VALUE:
        return False, (
            f"--live was passed but {LIVE_CONFIRMATION_VARIABLE} is not set to "
            f"{LIVE_CONFIRMATION_VALUE!r}; refusing to send audio")
    if not env.get("OPENROUTER_API_KEY"):
        return False, "OPENROUTER_API_KEY is not set in the process environment"
    return True, "live"


# ---------------------------------------------------------------------------
# Transport
# ---------------------------------------------------------------------------


def submit(
    payload: dict[str, Any],
    *,
    transport: Transport = urllib.request.urlopen,
    sleeper: Callable[[float], None] = time.sleep,
    timeout: float = request_build.DEFAULT_TIMEOUT,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
) -> dict[str, Any]:
    api_key = os.environ.get("OPENROUTER_API_KEY")
    if not api_key:
        raise BenchFailure("OPENROUTER_API_KEY is not set in the process environment")
    encoded = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    for attempt in range(1, max_attempts + 1):
        request = urllib.request.Request(
            request_build.OPENROUTER_URL,
            data=encoded,
            headers={"Authorization": f"Bearer {api_key}",
                     "Content-Type": "application/json",
                     "X-Title": "musializer-mimo-bench"},
            method="POST",
        )
        try:
            with transport(request, timeout=timeout) as response:
                return json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            if error.code not in TRANSIENT_STATUSES or attempt == max_attempts:
                error.close()
                raise BenchFailure(f"OpenRouter returned HTTP {error.code}") from error
            error.close()
            sleeper(min(2 ** (attempt - 1), 8))
        except urllib.error.URLError as error:
            if attempt == max_attempts:
                raise BenchFailure(f"network error: {error}") from error
            sleeper(min(2 ** (attempt - 1), 8))
    raise BenchFailure("OpenRouter request did not complete")


# ---------------------------------------------------------------------------
# Building one call's request
# ---------------------------------------------------------------------------


def description_for(cell: matrix.Cell, repeat: int) -> str:
    """The turn-1 prose a dependent arm reshapes, from the stored S0 cell."""
    if not cell.depends_on:
        raise BenchFailure(f"{cell.id} declares no dependency")
    records = stored_calls(cell.depends_on, repeat)
    if not records:
        raise BenchFailure(
            f"{cell.id} needs {cell.depends_on} repeat {repeat}; run that first")
    records.sort(key=lambda record: record.get("call_index", 0))
    parts = [str(record.get("text") or "").strip() for record in records]
    joined = "\n\n".join(part for part in parts if part)
    if not joined:
        raise BenchFailure(f"{cell.depends_on} repeat {repeat} stored no text")
    return joined


def audio_bytes_for(cell: matrix.Cell, call: matrix.Call, *, required: bool) -> bytes:
    track = track_by_slug(cell.track_slug)
    if call.chunk is None:
        return b""
    path = audio.chunk_file(track, cell.chunking, call.chunk.index)
    if path.is_file():
        return path.read_bytes()
    if required:
        raise BenchFailure(
            f"{path} is missing; run `tools/mimo_bench/run.py prepare` first")
    return b""


def build_call(
    cell: matrix.Cell, call: matrix.Call, repeat: int, *, live: bool,
) -> tuple[dict[str, Any], dict[str, Any]]:
    if cell.shaping in ("S0", "S1"):
        return request_build.build_listen_request(
            cell, call, audio_bytes_for(cell, call, required=live))
    description = (description_for(cell, repeat) if live
                   else _placeholder_description(cell, repeat))
    if cell.shaping == "S2a":
        return request_build.build_second_turn_request(
            cell, call, description, audio_bytes_for(cell, call, required=live))
    if cell.shaping == "S2b":
        return request_build.build_second_turn_request(cell, call, description, None)
    if cell.shaping == "S3":
        return request_build.build_reformatter_request(cell, call, description)
    raise BenchFailure(f"unknown shaping arm: {cell.shaping}")


def _placeholder_description(cell: matrix.Cell, repeat: int) -> str:
    """What a dry run substitutes for a turn-1 answer it has not paid for."""
    try:
        return description_for(cell, repeat)
    except BenchFailure:
        return (f"<the stored free-text description from {cell.depends_on} "
                f"repeat {repeat}; not yet produced, so this dry run shows the "
                f"request shape with the text elided>")


# ---------------------------------------------------------------------------
# Executing one resumable unit
# ---------------------------------------------------------------------------


def execute_unit(
    cell: matrix.Cell,
    repeat: int,
    *,
    transport: Transport = urllib.request.urlopen,
    sleeper: Callable[[float], None] = time.sleep,
    force: bool = False,
) -> dict[str, Any]:
    track = track_by_slug(cell.track_slug)
    calls = matrix.calls_for(cell, track)
    already = set() if force else completed_calls(cell.id, repeat)
    cell_dir(cell.id, repeat).mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    failures: list[str] = []
    sent = 0
    for call in calls:
        if call.index in already:
            continue
        try:
            payload, identity = build_call(cell, call, repeat, live=True)
            response = submit(payload, transport=transport, sleeper=sleeper)
            text = request_build.completion_text(response)
            metadata = request_build.response_metadata(response)
        except (BenchFailure, ValueError, OSError, json.JSONDecodeError) as failure:
            failures.append(f"call {call.index}: {type(failure).__name__}: {failure}")
            break
        record = {
            "cell": cell.id,
            "repeat": repeat,
            "call_index": call.index,
            "kind": call.kind,
            "turn": call.turn,
            "probe_run": call.probe_run,
            "structured": call.structured,
            "audio_seconds": call.audio_seconds,
            "chunk_index": call.chunk.index if call.chunk else None,
            "chunk_start_seconds": call.chunk.start_seconds if call.chunk else None,
            "chunk_end_seconds": call.chunk.end_seconds if call.chunk else None,
            "estimated_text_input_tokens": identity.get("estimated_text_input_tokens"),
            "identity": identity,
            "request": request_build.redact(payload),
            "response": response,
            "text": text,
            "status": "ok",
            **metadata,
        }
        atomic_write_json(call_path(cell.id, repeat, call.index), record)
        sent += 1
    runtime = time.monotonic() - started
    status = "failed" if failures else "ok"
    write_done(cell.id, repeat, status=status, runtime_seconds=runtime,
               calls_sent=sent, calls_total=len(calls),
               errors=failures or None)
    return {"cell": cell.id, "repeat": repeat, "status": status,
            "calls_sent": sent, "errors": failures}


def pending(repeats: int, *, only_cell: str | None = None,
            retry_failed: bool = False) -> list[tuple[matrix.Cell, int]]:
    queue: list[tuple[matrix.Cell, int]] = []
    for cell, repeat in matrix.units(repeats):
        if only_cell and cell.id != only_cell:
            continue
        state = cell_state(cell.id, repeat)
        if state is None or (retry_failed and state.get("status") != "ok"):
            queue.append((cell, repeat))
    return queue


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------


def command_plan(options: argparse.Namespace) -> int:
    summary = matrix.matrix_summary(options.repeats)
    projection = cost.project(options.repeats)

    print("=== MiMo v2.5 description benchmark — dry run ===")
    print(f"harness           {summary['matrix_version']}")
    print(f"prompts           {summary['prompt_registry_version']}")
    print(f"output schema     {summary['schema_version']}  sha256={schema.SCHEMA_SHA256[:16]}")
    print(f"model under test  {matrix.MIMO_MODEL}")
    print(f"reformatter       {matrix.REFORMATTER_MODEL}  (arm S3 only, text-only)")
    print(f"temperature       {matrix.TEMPERATURE}")
    print(f"time frame policy {prompts.TIME_FRAME_POLICY}")
    print()

    print("--- tracks and excerpts ---")
    for track in TRACKS:
        print(f"  {track.slug:22s} {track.role:11s} "
              f"[{track.excerpt_start_seconds:.1f}, "
              f"{track.excerpt_start_seconds + EXCERPT_SECONDS:.1f}) s of "
              f"{track.duration_seconds:.1f} s   "
              f"{track.aligned_lines_in_excerpt} aligned lane lines")
        manifest = audio.load_manifest(track)
        state = "prepared" if manifest else "NOT PREPARED (run `prepare`)"
        print(f"  {'':22s} excerpt: {state}")
    print()

    print("--- ground truth readiness ---")
    abstaining = 0
    for track in TRACKS:
        truth = ground_truth.load(track)
        missing = truth.abstaining_dimensions()
        abstaining += len(missing)
        chance = scorers.tempo_chance_accept_rate(truth)
        print(f"  {track.slug:22s} measured_bpm={truth.measured_bpm} "
              f"excerpt_bpm={truth.excerpt_bpm if truth.excerpt_bpm is None else round(truth.excerpt_bpm, 2)} "
              f"sections={len(truth.sections)} "
              f"usable_lyric_truth={len(truth.lyrics)} "
              f"(of {track.aligned_lines_in_excerpt} in the lane; the difference "
              f"is lines the operator adjudicated as unlocatable)")
        print(f"  {'':22s} tempo candidates: "
              f"{[round(value, 2) for value in truth.excerpt_bpm_candidates]}; "
              f"a random BPM guess would be accepted {chance * 100:.0f} % of the "
              f"time — read accept_rate against that, not against zero")
        print(f"  {'':22s} abstaining: {', '.join(missing) if missing else 'none'}")
        for note in truth.missing_sources:
            print(f"  {'':22s} ! {note}")
    if abstaining:
        print(f"  {abstaining} dimension(s) will abstain until "
              f"tools/mimo_bench/ground_truth/tracks.json is filled in.")
    print()

    print("--- matrix ---")
    width = max(len(str(row["cell"])) for row in summary["rows"])
    print(f"  {'cell'.ljust(width)}  blocks              chunks  calls/rep  depends on")
    for row in summary["rows"]:
        print(f"  {str(row['cell']).ljust(width)}  {str(row['blocks']):18s}  "
              f"{row['chunks']:6d}  {row['calls_per_repeat']:9d}  "
              f"{row['depends_on'] or ''}")
    print(f"\n  {summary['cells']} cells x {summary['repeats']} repeats = "
          f"{summary['units']} resumable units, "
          f"{summary['total_calls']} API calls "
          f"({summary['audio_calls']} carrying audio, {summary['text_calls']} text-only), "
          f"{summary['audio_seconds']:.0f} audio seconds")
    print()

    print(cost.format_projection(projection))
    print()

    print("--- request bodies (audio redacted to its sha256) ---")
    dumps = _request_dumps(options)
    if options.request_dump:
        directory = Path(options.request_dump)
        for name, dump in dumps:
            atomic_write_json(directory / f"{name}.json", dump)
        print(f"  {len(dumps)} request bodies written to {directory}")
    else:
        for name, dump in dumps:
            print(f"\n  # {name}")
            for line in json.dumps(dump, ensure_ascii=False, indent=2).splitlines():
                print(f"  {line}")
    print()
    permitted, reason = live_permitted(False)
    print(f"network: {reason}. No socket was opened.")
    print(f"to execute for real: "
          f"{LIVE_CONFIRMATION_VARIABLE}={LIVE_CONFIRMATION_VALUE} "
          f"tools/mimo_bench/run.py all --live")
    return 0


def _request_dumps(options: argparse.Namespace) -> list[tuple[str, dict[str, Any]]]:
    """One representative request per cell, or every request with ``--all-requests``.

    The representative is the first call: repeats and later chunks differ only
    in which audio bytes are attached and in the declared span, both of which
    are already visible in the identity block.
    """
    dumps: list[tuple[str, dict[str, Any]]] = []
    for cell in matrix.cells():
        track = track_by_slug(cell.track_slug)
        calls = matrix.calls_for(cell, track)
        chosen = calls if options.all_requests else calls[:1]
        for call in chosen:
            payload, identity = build_call(cell, call, 1, live=False)
            name = f"{cell.id.replace('/', '__')}__call{call.index:03d}"
            dumps.append((name, request_build.request_dump(payload, identity)))
    return dumps


def command_prepare(options: argparse.Namespace) -> int:
    wanted = {cell.chunking_id for cell in matrix.cells()}
    for track in TRACKS:
        chunkings = [chunking for chunking in matrix.CHUNKINGS
                     if chunking.id in wanted
                     and any(cell.track_slug == track.slug
                             and cell.chunking_id == chunking.id
                             for cell in matrix.cells())]
        try:
            manifest = audio.prepare_track(track, chunkings, force=options.force)
        except audio.AudioPreparationError as error:
            print(f"{track.slug}: {error}", file=sys.stderr)
            return 1
        total = sum(len(entry["chunks"]) for entry in manifest["chunkings"].values())
        print(f"{track.slug}: excerpt {manifest['excerpt']['bytes']} bytes, "
              f"{total} chunk file(s) across {len(manifest['chunkings'])} chunking(s)")
        for chunking_id, entry in sorted(manifest["chunkings"].items()):
            excess = entry.get("frame_boundary_excess_seconds")
            print(f"  {chunking_id}: {len(entry['chunks'])} chunk(s), "
                  f"probed total {entry.get('probed_total_seconds')} s"
                  + (f", {excess:+.3f} s of frame-boundary overlap"
                     if isinstance(excess, float) else ""))
    return 0


def command_list(options: argparse.Namespace) -> int:
    rows: list[tuple[str, str, str, str]] = []
    for cell, repeat in matrix.units(options.repeats):
        state = cell_state(cell.id, repeat)
        if state is None:
            stored = len(completed_calls(cell.id, repeat))
            status = f"pending ({stored} stored)" if stored else "pending"
            runtime = ""
        else:
            status = str(state.get("status", "?"))
            seconds = state.get("runtime_seconds")
            runtime = f"{float(seconds):.1f}s" if isinstance(seconds, (int, float)) else ""
        rows.append((cell.id, f"r{repeat:02d}", status, runtime))
    widths = [max(len(row[column]) for row in rows) for column in range(4)]
    for row in rows:
        print("  ".join(row[column].ljust(widths[column]) for column in range(4)).rstrip())
    remaining = len(pending(options.repeats, retry_failed=options.retry_failed))
    print(f"\n{remaining} unit(s) pending of {len(rows)}.")
    return 0


def _require_live(options: argparse.Namespace) -> bool:
    permitted, reason = live_permitted(options.live)
    if not permitted:
        print(f"refusing to run: {reason}", file=sys.stderr)
        print("`plan` shows the whole matrix without sending anything.", file=sys.stderr)
    return permitted


def command_next(options: argparse.Namespace) -> int:
    if not _require_live(options):
        return 2
    queue = pending(options.repeats, only_cell=options.cell,
                    retry_failed=options.retry_failed)
    if not queue:
        print("No pending units.")
        return 0
    cell, repeat = queue[0]
    result = execute_unit(cell, repeat)
    print(f"[{cell.id} r{repeat:02d}] {result['status']} "
          f"({result['calls_sent']} call(s) sent)")
    for error in result["errors"]:
        print(f"  {error}", file=sys.stderr)
    return 0 if result["status"] == "ok" else 1


def command_all(options: argparse.Namespace) -> int:
    if not _require_live(options):
        return 2
    queue = pending(options.repeats, only_cell=options.cell,
                    retry_failed=options.retry_failed)
    if not queue:
        print("No pending units.")
        return 0
    failures = 0
    for cell, repeat in queue:
        result = execute_unit(cell, repeat)
        print(f"[{cell.id} r{repeat:02d}] {result['status']} "
              f"({result['calls_sent']} call(s) sent)", flush=True)
        for error in result["errors"]:
            print(f"  {error}", file=sys.stderr)
        if result["status"] != "ok":
            failures += 1
    print(f"{len(queue) - failures}/{len(queue)} units completed.")
    return 1 if failures else 0


def command_run(options: argparse.Namespace) -> int:
    if not options.cell or options.repeat is None:
        print("run needs --cell and --repeat", file=sys.stderr)
        return 2
    if not _require_live(options):
        return 2
    cell = matrix.cell_by_id(options.cell)
    state = cell_state(cell.id, options.repeat)
    if state is not None and not options.force:
        print(f"[{cell.id} r{options.repeat:02d}] already done "
              f"({state.get('status')}); pass --force to repeat it.")
        return 0
    result = execute_unit(cell, options.repeat, force=options.force)
    print(f"[{cell.id} r{options.repeat:02d}] {result['status']} "
          f"({result['calls_sent']} call(s) sent)")
    return 0 if result["status"] == "ok" else 1


def command_score(options: argparse.Namespace) -> int:
    document = report.score_all(options.repeats)
    path = report.write_report(document)
    print(report.format_report(document))
    records = []
    for cell in matrix.cells():
        for repeat in range(1, options.repeats + 1):
            records.extend(stored_calls(cell.id, repeat))
    calibration = cost.calibrate_from_usage(records)
    print(f"\naudio token rate from stored usage: {calibration}")
    print(f"report written to {path}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "command", nargs="?", default="plan",
        choices=("plan", "prepare", "list", "next", "run", "all", "score"))
    parser.add_argument("--repeats", type=int, default=matrix.DEFAULT_REPEATS)
    parser.add_argument("--cell")
    parser.add_argument("--repeat", type=int)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--retry-failed", action="store_true")
    parser.add_argument("--all-requests", action="store_true",
                        help="print every request body, not one per cell")
    parser.add_argument("--request-dump", type=Path,
                        help="write the dry-run request bodies to this directory")
    parser.add_argument(
        "--live", action="store_true",
        help=f"send requests; also requires {LIVE_CONFIRMATION_VARIABLE}="
             f"{LIVE_CONFIRMATION_VALUE}")
    parser.add_argument("--dry-run", action="store_true", default=True,
                        help=argparse.SUPPRESS)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    options = build_parser().parse_args(argv)
    BENCH_ROOT.mkdir(parents=True, exist_ok=True)
    return {
        "plan": command_plan,
        "prepare": command_prepare,
        "list": command_list,
        "next": command_next,
        "run": command_run,
        "all": command_all,
        "score": command_score,
    }[options.command](options)


if __name__ == "__main__":
    raise SystemExit(main())
