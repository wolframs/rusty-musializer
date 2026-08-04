#!/usr/bin/env python3
"""One compact table across every benchmark track and method.

There is no adjudicated ground truth yet, so this scores what can be measured
without one:

* **coverage** — authored lines that received a timed cue, over the alignable
  authored lines. The canary's failure is a coverage failure, so this is the
  headline number.
* **unresolved / omitted** — a line the lane kept but could not place, versus a
  line the lane dropped before the acoustic stage. The research plan's first
  invariant is that the second number should be zero.
* **order / overlap violations** — cues that go backwards, or that run into
  their successor. Cheap, and they catch a lane that bought coverage by
  guessing.
* **canary check** — did the two authored outro lines today's pipeline omits
  get plausible times inside the loop window?
* **agreement with the baseline** — median and maximum absolute start/end delta
  over the lines both lanes timed. Agreement is not accuracy, but a lane that
  reproduces the baseline where the baseline is trusted and *also* covers what
  it omits is the interesting result.
* **runtime** — wall clock for the chunk, from its done marker.

    tools/lyrics_research/scoreboard.py

Writes ``build/lyrics-research-v2/scoreboard.json`` and ``scoreboard.txt`` and
prints the rendered table. Missing chunks are reported as missing rather than
silently skipped: a hole in the matrix is information.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

from research_io import (  # noqa: E402
    METHODS,
    RESEARCH_ROOT,
    TRACKS,
    AnalysisValidationError,
    Track,
    alignable_lines,
    atomic_write_json,
    chunk_dir,
    chunk_state,
    load_reference_text,
    median,
    read_json,
)


def _timings(document: dict[str, Any]) -> dict[int, tuple[float, float]]:
    """Reference line index -> (start, end) for every line the lane timed.

    Both the production ``musializer.lyric-sync/v1`` lane and the research
    lanes key their lines by the authored line index, which is what makes them
    comparable at all: text matching would quietly reward a lane for reflowing
    the lyrics.
    """
    result: dict[int, tuple[float, float]] = {}
    for line in document.get("lines") or []:
        if not isinstance(line, dict):
            continue
        index = line.get("reference_line_index")
        start = line.get("start_seconds")
        end = line.get("end_seconds")
        if not isinstance(index, int) or isinstance(index, bool):
            continue
        if not isinstance(start, (int, float)) or isinstance(start, bool):
            continue
        if not isinstance(end, (int, float)) or isinstance(end, bool):
            continue
        result[int(index)] = (float(start), float(end))
    return result


def _present(document: dict[str, Any]) -> set[int]:
    """Reference line indices the lane kept, timed or not."""
    present: set[int] = set()
    for line in document.get("lines") or []:
        if isinstance(line, dict) and isinstance(line.get("reference_line_index"), int):
            present.add(int(line["reference_line_index"]))
    return present


def _violations(
    order: Sequence[int], timings: dict[int, tuple[float, float]],
) -> tuple[int, int]:
    previous: tuple[float, float] | None = None
    backwards = 0
    overlaps = 0
    for index in order:
        current = timings.get(index)
        if current is None:
            continue
        if previous is not None:
            if current[0] < previous[0]:
                backwards += 1
            if current[0] < previous[1]:
                overlaps += 1
        previous = current
    return backwards, overlaps


def _agreement(
    lane: dict[int, tuple[float, float]],
    baseline: dict[int, tuple[float, float]],
) -> dict[str, Any]:
    shared = sorted(set(lane) & set(baseline))
    starts = [abs(lane[index][0] - baseline[index][0]) for index in shared]
    ends = [abs(lane[index][1] - baseline[index][1]) for index in shared]
    return {
        "compared_lines": len(shared),
        "median_start_delta_seconds": median(starts),
        "max_start_delta_seconds": max(starts) if starts else None,
        "median_end_delta_seconds": median(ends),
        "max_end_delta_seconds": max(ends) if ends else None,
    }


def score_chunk(
    track: Track, method: str, authored: Sequence[dict[str, Any]],
    baseline_timings: dict[int, tuple[float, float]] | None,
) -> dict[str, Any]:
    order = [int(line["index"]) for line in authored]
    row: dict[str, Any] = {
        "track": track.slug,
        "method": method,
        "gating": track.gating,
        "authored_alignable_lines": len(order),
        "status": "missing",
        "timed_lines": None,
        "coverage": None,
        "unresolved_lines": None,
        "omitted_lines": None,
        "order_violations": None,
        "overlap_violations": None,
        "canary": None,
        "agreement_vs_baseline": None,
        "runtime_seconds": None,
        "error": None,
    }
    state = chunk_state(track, method)
    if state is None:
        return row
    row["status"] = str(state.get("status", "?"))
    row["runtime_seconds"] = state.get("runtime_seconds")
    row["error"] = state.get("error")

    result_path = chunk_dir(track, method) / "result.json"
    if not result_path.is_file():
        return row
    try:
        document = read_json(result_path)
    except (OSError, json.JSONDecodeError) as error:
        row["error"] = f"unreadable result: {error}"
        return row
    if not isinstance(document, dict):
        row["error"] = "result is not a lane document"
        return row

    timings = _timings(document)
    present = _present(document)
    timed = [index for index in order if index in timings]
    row["timed_lines"] = len(timed)
    row["coverage"] = (len(timed) / len(order)) if order else None
    row["unresolved_lines"] = sum(
        1 for index in order if index in present and index not in timings)
    row["omitted_lines"] = sum(1 for index in order if index not in present)
    backwards, overlaps = _violations(order, timings)
    row["order_violations"] = backwards
    row["overlap_violations"] = overlaps

    if track.expected_late_reference_lines:
        low, high = track.expected_late_window or (0.0, track.duration_seconds)
        checks = []
        for index in track.expected_late_reference_lines:
            span = timings.get(index)
            checks.append({
                "reference_line_index": index,
                "timed": span is not None,
                "start_seconds": None if span is None else span[0],
                "end_seconds": None if span is None else span[1],
                "inside_window": bool(
                    span is not None and low <= span[0] <= high
                    and low <= span[1] <= high + 1e-6),
            })
        row["canary"] = {
            "window": [low, high],
            "expected": len(checks),
            "timed": sum(1 for check in checks if check["timed"]),
            "inside_window": sum(1 for check in checks if check["inside_window"]),
            "lines": checks,
        }

    if baseline_timings is not None and method != "baseline":
        row["agreement_vs_baseline"] = _agreement(timings, baseline_timings)
    return row


def build(only_track: str | None = None) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    notes: list[dict[str, Any]] = []
    for track in TRACKS:
        if only_track and track.slug != only_track:
            continue
        try:
            authored = alignable_lines(load_reference_text(track))
        except (AnalysisValidationError, OSError) as error:
            notes.append({"track": track.slug, "problem": str(error)})
            continue
        baseline_path = chunk_dir(track, "baseline") / "result.json"
        baseline_timings: dict[int, tuple[float, float]] | None = None
        if baseline_path.is_file():
            try:
                baseline_timings = _timings(read_json(baseline_path))
            except (OSError, json.JSONDecodeError):
                baseline_timings = None
        for method in METHODS:
            rows.append(score_chunk(track, method, authored, baseline_timings))
        if track.note:
            notes.append({"track": track.slug, "note": track.note})
    return {"schema_version": "musializer-research.scoreboard/v1",
            "rows": rows, "notes": notes}


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------


COLUMNS = (
    ("track", 23),
    ("method", 17),
    ("status", 8),
    ("coverage", 12),
    ("unres", 5),
    ("omit", 4),
    ("ord", 3),
    ("ovl", 3),
    ("canary", 10),
    ("dstart med/max", 15),
    ("dend med/max", 15),
    ("runtime", 8),
)


def _pair(low: Any, high: Any) -> str:
    if low is None or high is None:
        return "-"
    return f"{float(low):.2f}/{float(high):.2f}"


def render(scoreboard: dict[str, Any]) -> str:
    lines = [
        "  ".join(name.ljust(width) for name, width in COLUMNS).rstrip(),
        "  ".join("-" * width for _, width in COLUMNS).rstrip(),
    ]
    for row in scoreboard["rows"]:
        coverage = "-"
        if row["coverage"] is not None:
            coverage = (f"{row['timed_lines']}/{row['authored_alignable_lines']} "
                        f"{row['coverage'] * 100:.0f}%")
        canary = "-"
        if row["canary"]:
            canary = (f"{row['canary']['inside_window']}/"
                      f"{row['canary']['expected']} in win")
        agreement = row["agreement_vs_baseline"] or {}
        cells = (
            row["track"],
            row["method"],
            row["status"],
            coverage,
            "-" if row["unresolved_lines"] is None else str(row["unresolved_lines"]),
            "-" if row["omitted_lines"] is None else str(row["omitted_lines"]),
            "-" if row["order_violations"] is None else str(row["order_violations"]),
            "-" if row["overlap_violations"] is None else str(row["overlap_violations"]),
            canary,
            _pair(agreement.get("median_start_delta_seconds"),
                  agreement.get("max_start_delta_seconds")),
            _pair(agreement.get("median_end_delta_seconds"),
                  agreement.get("max_end_delta_seconds")),
            ("-" if row["runtime_seconds"] is None
             else f"{float(row['runtime_seconds']):.1f}s"),
        )
        lines.append("  ".join(
            str(cell).ljust(width) for cell, (_, width) in zip(cells, COLUMNS)
        ).rstrip())
    for row in scoreboard["rows"]:
        if row["error"]:
            lines.append(f"! {row['track']}/{row['method']}: {row['error']}")
    for note in scoreboard["notes"]:
        label = note.get("note") or note.get("problem")
        lines.append(f"# {note['track']}: {label}")
    lines.append("")
    lines.append("coverage = authored lines with a timed cue / alignable "
                 "authored lines; unres = kept but unplaced; omit = dropped "
                 "before alignment;")
    lines.append("ord/ovl = start-order and overlap violations; dstart/dend = "
                 "absolute agreement with this track's baseline lane, over the "
                 "lines both timed.")
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--track")
    parser.add_argument("--json-out", type=Path,
                        default=RESEARCH_ROOT / "scoreboard.json")
    parser.add_argument("--text-out", type=Path,
                        default=RESEARCH_ROOT / "scoreboard.txt")
    args = parser.parse_args(argv)
    scoreboard = build(args.track)
    rendered = render(scoreboard)
    atomic_write_json(args.json_out, scoreboard)
    args.text_out.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.text_out.with_name(f".{args.text_out.name}.tmp")
    temporary.write_text(rendered + "\n", encoding="utf-8")
    temporary.replace(args.text_out)
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
