#!/usr/bin/env python3
"""Count where the accent is drawn, and fail if it marks anything but *now*.

WX-5a made the accent mean one thing: the playhead, the section under it, and
the primary action (docs/WORKSTATION.md, WX-5). Before it, the same amber marked
the selected track, the selected scene, Auto, every slider, the waveform and
every panel heading, so it stopped meaning anything — and a capture of that
state looks like a perfectly plausible interface. The rule only holds if
something counts, so this reads the chrome's pixels.

Outside the scene preview (whose picture may legitimately be amber), every
pixel within a small distance of the accent must belong to one of:

  * the playhead: within a few pixels of the column the accent runs down
    through the timeline band;
  * the section under it: a *hollow* outline (an inset rectangle, so few of its
    bounding box's pixels are set) at the top of the timeline group, where the
    scene plan lane is;
  * a thin solid marker: the transport's scrub handle, or the bar beside the
    cue-list row the playhead is inside;
  * the primary action — Render — a solid button-sized fill, and only as many
    of them as the caller says the frame has (`--primary N`, default 0).

Anything else is a regression: a selected tile, a slider, a heading or a
waveform in the accent.

Usage: accent_census.py PNG REPORT [--expect-appearance NAME] [--primary N]

  REPORT is the run's report; its `appearance:` line gives the accent and its
  `preview frame:` line the scene panel to leave out. Exits 1 with the
  offending regions on a failure, 2 on bad usage.
"""

import re
import sys

import numpy as np
from PIL import Image

# How close a pixel must be to the accent, per channel. Opaque accent fills are
# exact on every renderer; the margin admits the playhead's anti-aliased
# triangle without admitting any other palette colour (the nearest, the
# warning orange, is more than 40 away on some channel in both appearances).
TOLERANCE = 18


def parse_report(path):
    text = open(path, encoding="utf-8", errors="replace").read()
    appearance = re.search(r"^appearance:\s+(\S+) accent=#([0-9A-Fa-f]{6})", text, re.M)
    preview = re.search(r"^preview frame:.*pixels (-?\d+),(-?\d+),(\d+),(\d+)", text, re.M)
    if not appearance:
        raise SystemExit("accent_census: the report has no appearance: line")
    accent = tuple(int(appearance.group(2)[i:i + 2], 16) for i in (0, 2, 4))
    rect = tuple(int(v) for v in preview.groups()) if preview else None
    return appearance.group(1), accent, rect


def components(mask):
    """4-connected components as lists of (y, x), without scipy."""
    height, width = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    out = []
    for y, x in zip(*np.nonzero(mask)):
        if seen[y, x]:
            continue
        stack, pixels = [(y, x)], []
        seen[y, x] = True
        while stack:
            cy, cx = stack.pop()
            pixels.append((cy, cx))
            for ny, nx in ((cy - 1, cx), (cy + 1, cx), (cy, cx - 1), (cy, cx + 1)):
                if 0 <= ny < height and 0 <= nx < width and mask[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    stack.append((ny, nx))
        out.append(pixels)
    return out


def census(png, report, primary=0):
    name, accent, preview = parse_report(report)
    image = np.array(Image.open(png).convert("RGB")).astype(int)
    height, width, _ = image.shape
    mask = (np.abs(image - np.array(accent)).max(axis=-1) <= TOLERANCE)
    if preview:
        x, y, w, h = preview
        mask[max(0, y - 2):y + h + 2, max(0, x - 2):x + w + 2] = False

    # The playhead: the column the accent runs furthest down. It crosses every
    # timed lane, so it is by far the tallest accent column in the chrome.
    column_counts = mask.sum(axis=0)
    playhead_x = int(column_counts.argmax())
    if column_counts[playhead_x] < 40:
        return name, [f"no playhead: the tallest accent column has {column_counts[playhead_x]} px"]
    rows = np.flatnonzero(mask[:, playhead_x])
    group_top, group_bottom = int(rows.min()), int(rows.max())
    scale = max(1.0, width / 1280.0) if width >= 1280 else 1.0

    allowed = np.zeros_like(mask)
    band = 7 + int(3 * scale)
    allowed[:, max(0, playhead_x - band):playhead_x + band + 1] = True
    rest = mask & ~allowed

    problems = []
    # The plan lane's bottom: 44 px since WX-5b (it was 24, and this was 34),
    # plus the 2 px inset and a margin.
    lane_bottom = group_top + int(50 * scale)
    shapes = []
    for pixels in components(rest):
        ys = [p[0] for p in pixels]
        xs = [p[1] for p in pixels]
        top, bottom, left, right = min(ys), max(ys), min(xs), max(xs)
        box_w, box_h = right - left + 1, bottom - top + 1
        shapes.append((pixels, top, bottom, left, right, box_w, box_h,
                       len(pixels) / float(box_w * box_h)))

    # The primary action first: a solid button-sized fill. Its label's
    # counters (the holes in an `o` or an `e`) are accent pixels cut off from
    # the fill by the glyph strokes, so anything inside a primary's box is the
    # primary.
    primaries = [
        shape for shape in shapes if shape[7] >= 0.8 and shape[5] >= 40 and shape[6] >= 16
    ]

    def inside_primary(top, bottom, left, right):
        return any(
            top >= p[1] and bottom <= p[2] and left >= p[3] and right <= p[4] for p in primaries
        )

    for pixels, top, bottom, left, right, box_w, box_h, fill in shapes:
        if any(pixels is p[0] for p in primaries) or inside_primary(top, bottom, left, right):
            continue
        hollow_in_plan_lane = (
            top >= group_top - 4 and bottom <= lane_bottom and (fill < 0.5 or box_h <= 2)
        )
        # A thin solid mark: the scrub bar's handle above the timeline, or the
        # bar at the left of the cue-list row the playhead is inside. A slider
        # knob is round and wider, a splitter is the height of a panel, and a
        # selected tile or heading is neither thin nor short.
        thin_marker = box_w <= 5 and box_h <= 40 * scale and fill >= 0.8
        if hollow_in_plan_lane or thin_marker:
            continue
        problems.append(
            f"{len(pixels)} accent px at x={left}..{right} y={top}..{bottom} "
            f"({box_w}x{box_h}, fill {fill:.2f})"
        )
    primaries = [f"{p[5]}x{p[6]} at {p[3]},{p[1]}" for p in primaries]
    if len(primaries) != primary:
        problems.append(
            f"{len(primaries)} primary-action fill(s) where the frame has {primary}: "
            + (", ".join(primaries) or "none")
        )
    return name, problems


def main(argv):
    args = argv[1:]
    options = {"--expect-appearance": None, "--primary": "0"}
    positional = []
    while args:
        head = args.pop(0)
        if head in options and args:
            options[head] = args.pop(0)
        elif head.startswith("--"):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            positional.append(head)
    if len(positional) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    png, report = positional
    expect = options["--expect-appearance"]
    name, problems = census(png, report, int(options["--primary"]))
    if expect and name != expect:
        print(f"FAIL: {png} was drawn in {name}, the check expected {expect}")
        return 1
    if problems:
        print(f"FAIL: the accent marks something other than now in {png} ({name}):")
        for problem in problems[:12]:
            print(f"  {problem}")
        return 1
    print(f"ok  {png} ({name}): the accent marks only now (playhead, section, markers, primary action)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
