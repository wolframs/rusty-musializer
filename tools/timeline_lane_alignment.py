#!/usr/bin/env python3
"""Measure that every timed lane in the timeline band shares one time axis.

The timeline band stacks the scene-plan lane, the waveform strip and (when the
lyrics editor is open) the lyric cue lane over the same `TimelineView`. They are
drawn by three different modules, and each is handed its own rectangle: if one
of them is inset by a different amount, `x_at` maps the same second onto a
different column in that lane and the playhead lies about where a cue is. That
is a correctness bug, not a cosmetic one, and nothing else in this repository
can see it — the lanes still look self-coherent, because everything inside a
lane moves together.

So this reads the pixels. For each lane it measures:

  * the two columns of the shared frame (the leftmost and rightmost rule column
    inside the lane, ignoring the panel's own outline), which is the inset; and
  * the columns the playhead occupies, which is one moment in time.

All of them must agree across every lane in the frame. The lanes are found from
the seams `Shell::timeline_group_chrome` paints between them, so the check does
not hard-code a single window's geometry.

Usage:  timeline_lane_alignment.py PNG LANES [UI_SCALE_PERCENT]
        LANES is how many timed lanes the frame should contain (2 without the
        lyrics editor, 3 with it). UI_SCALE_PERCENT defaults to 100; the shell
        selects 150 on its own at 1440p, and every logical dimension below is
        multiplied by it.
"""

import sys

import numpy as np
from PIL import Image

# theme::rgba, as the framebuffer stores them.
TROUGH = (230, 230, 234)  # UI_LANE_TROUGH, the seam between two lanes
RULE = (210, 210, 214)  # UI_RULE, every lane border and the group frame
ACCENT = (0, 47, 167)  # ACCENT, the playhead

# theme::metric::LANE_GAP. A seam that is not exactly this tall means the two
# sides disagree about the gap, which is the defect the compile-time assertion
# in `shell.rs` covers for the one case it can see.
LANE_GAP = 5
# How far into a lane to look. Small, so the band stays inside the shortest lane
# in the frame (the cue lane, 22 px before it became resizable).
PROBE_ROWS = 6


def _runs(rows):
    out, start, prev = [], None, None
    for y in rows:
        if start is None:
            start = prev = y
        elif y == prev + 1:
            prev = y
        else:
            out.append((start, prev))
            start = prev = y
    if start is not None:
        out.append((start, prev))
    return out


def _match(image, colour, tolerance):
    return np.abs(image - np.array(colour)).max(axis=-1) <= tolerance


def _longest_run(mask_row):
    """The longest contiguous True run, as (start, end_exclusive, length)."""
    best = (0, 0, 0)
    start = None
    for x, on in enumerate(mask_row):
        if on and start is None:
            start = x
        elif not on and start is not None:
            if x - start > best[2]:
                best = (start, x, x - start)
            start = None
    if start is not None and len(mask_row) - start > best[2]:
        best = (start, len(mask_row), len(mask_row) - start)
    return best


def measure(path, lanes, ui_scale=100):
    image = np.array(Image.open(path).convert("RGB")).astype(int)
    height, width, _ = image.shape

    trough = _match(image, TROUGH, 4)
    # **Total** trough pixels in the row, not the longest contiguous run.
    #
    # The run version is the obvious way to write this and it is wrong, which
    # cost a gate round to find: the seam deliberately carries the tick columns
    # and the playhead through it (`Shell::timeline_group_chrome`), so a 1240 px
    # seam with eight ticks in it has no run longer than about 155 px and the
    # detector reported zero seams on a band that was drawing them correctly.
    # Counting pixels is immune to being interrupted, which is the property this
    # measurement actually needs.
    #
    # Not a fraction of the row either: with the Tune inspector open the band is
    # narrower than the window, so a row-wide fraction misses the seam entirely.
    floor = max(120, width // 6)
    seam_rows, spans = [], {}
    for y in range(height):
        columns = np.flatnonzero(trough[y])
        if columns.size >= floor:
            seam_rows.append(y)
            # First and last trough pixel bound the lane, which is what the
            # border search below is bracketed against. Interruptions inside do
            # not move either end.
            spans[y] = (int(columns[0]), int(columns[-1]) + 1)
    seams = _runs(seam_rows)

    if len(seams) != lanes - 1:
        return None, f"expected {lanes - 1} seam(s) between {lanes} lanes, found {len(seams)}: {seams}"
    # A seam is `LANE_GAP` logical pixels. At 100 % that is exact; above it the
    # shell's camera puts a fractional edge somewhere, so allow a pixel either
    # way. The load-bearing assertion is the alignment below — this one only
    # catches a lane that quietly stopped using the shared gap.
    expected_gap = LANE_GAP * ui_scale / 100.0
    for top, bottom in seams:
        measured_gap = bottom - top + 1
        if abs(measured_gap - expected_gap) > 1.0:
            return None, (
                f"seam {top}..{bottom} is {measured_gap} px, not LANE_GAP={LANE_GAP} "
                f"at {ui_scale}% ({expected_gap:g} px)"
            )

    probe = max(PROBE_ROWS, round(PROBE_ROWS * ui_scale / 100.0))
    # One probe band per lane: just above the first seam, and just below each.
    bands = [("lane1", seams[0][0] - probe, seams[0][0])]
    for index, (_, bottom) in enumerate(seams, start=2):
        bands.append((f"lane{index}", bottom + 1, bottom + 1 + probe))

    # The seam runs between the lane's two frame columns, so widening it by a
    # few pixels brackets them without reaching any other chrome.
    seam_start, seam_end = spans[seams[0][0]]
    search_lo = max(0, seam_start - 4)
    search_hi = min(width, seam_end + 4)

    rule = _match(image, RULE, 6)
    accent = _match(image, ACCENT, 30)
    measured = []
    for name, y0, y1 in bands:
        if y0 < 0 or y1 > height:
            return None, f"{name} probe band {y0}..{y1} falls outside the frame"
        # A column that is the rule colour on every row of the band is a
        # vertical border. Ticks qualify too, which is why only the outermost
        # two are taken: those are the lane's own edges.
        columns = [x for x in range(search_lo, search_hi) if rule[y0:y1, x].all()]
        if not columns:
            return None, f"{name} has no vertical border at all"
        counts = accent[y0:y1].sum(axis=0)
        peak = counts.max()
        if peak < y1 - y0:
            return None, f"{name} has no playhead: strongest accent column covers {peak}/{y1 - y0} rows"
        playhead = tuple(int(x) for x in np.flatnonzero(counts == peak))
        measured.append((name, y0, y1, min(columns), max(columns), playhead))

    return measured, None


def main(argv):
    if len(argv) not in (3, 4):
        print(__doc__, file=sys.stderr)
        return 2
    path, lanes = argv[1], int(argv[2])
    ui_scale = int(argv[3]) if len(argv) == 4 else 100
    measured, error = measure(path, lanes, ui_scale)
    if error is not None:
        print(f"FAIL: {path}: {error}", file=sys.stderr)
        return 1

    for name, y0, y1, left, right, playhead in measured:
        print(f"  {name} rows {y0}..{y1}: frame x={left}..{right} playhead x={list(playhead)}")

    reference = measured[0]
    failed = False
    for entry in measured[1:]:
        for index, label in ((3, "left edge"), (4, "right edge"), (5, "playhead columns")):
            if entry[index] != reference[index]:
                print(
                    f"FAIL: {path}: {entry[0]} {label} {entry[index]} != "
                    f"{reference[0]} {reference[index]}",
                    file=sys.stderr,
                )
                failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
