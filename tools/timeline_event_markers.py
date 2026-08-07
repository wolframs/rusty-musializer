#!/usr/bin/env python3
"""Read the event markers off a capture: their colour, and which lane each is in.

The waveform lane draws one marker per merged manual/semantic event (D4). Two
things about it are worth checking from pixels rather than from a count, and
neither is visible in the report line:

  * **The type colour.** Four types draw in four colours (`event_type_color`,
    `plug.c:1521-1530`). A build that resolved every marker to one colour would
    still report the right number of markers, still draw a plausible timeline,
    and still pass every unit test — the colour only exists on screen.

  * **The lane.** A manual marker's head is a filled disc, a semantic marker's is
    a ring. That distinction carries information the colour cannot: the manual
    event row's `+ Feel` button records a *semantic-typed* event into the manual
    lane (`plug.c:2897`), so an amber marker may be either, and a build that lost
    `SceneEventMerge::lanes` would draw every head filled while reporting exactly
    the same per-lane counts, because the counts come from the same list the
    heads were supposed to. Reading the head shape is what closes that loop.

A ring is detected as the surface colour appearing *inside* a disc of the type
colour — `ui_raised` is opaque white, so a hole in a marker head is unambiguous
and needs no tolerance.

Usage: timeline_event_markers.py PNG [EXPECT]

  EXPECT is optional and, when given, is a comma-separated list of
  `lane:type` in left-to-right order, e.g.

      manual:lyric,semantic:semantic,manual:cue,semantic:semantic,manual:semantic

  Exits 1 if the observed markers do not match, 2 on bad usage.
"""

import sys
from collections import defaultdict

try:
    from PIL import Image
except ImportError:  # pragma: no cover - the gate installs Pillow
    print("timeline_event_markers.py needs Pillow", file=sys.stderr)
    raise

# `EventType::rgba` (`crates/musializer-core/src/scene/events.rs`), which is the
# port of `event_type_color`. Kept as the same four literals rather than derived,
# so a change to either side shows up here as a mismatch rather than agreeing
# with itself.
TYPE_COLOURS = {
    "lyric": (0xEC, 0x59, 0xBE),
    "semantic": (0xF2, 0xBE, 0x42),
    "cue": (0x3F, 0xDC, 0xAB),
    "custom": (0x97, 0x6F, 0xF1),
}

# `theme::rgba::UI_RAISED`. The lane's own surface, which is what a semantic
# marker's head is punched out with.
RAISED = (0xFF, 0xFF, 0xFF)

# The head is a disc of radius 5 (`plug.c:3099`), so a cluster is separated from
# its neighbour by more than a diameter. Markers closer together than this on
# screen are genuinely one visual cluster and are reported as one.
CLUSTER_GAP = 12

# Below this the pixels are the marker's *line*, which is drawn at 0.75 or 0.45
# alpha and so lands nowhere near the exact colour. Only the head is opaque.
MIN_HEAD_PIXELS = 12

# The manual event row draws a 4 px category swatch down the left edge of each of
# its buttons, in the *same four colours* — so a naive colour search finds three
# extra "markers" above the lane and reports nine where there are six. That was
# not predicted; it came out of running the tool.
#
# The discriminator is shape, not position: a marker head is a disc clipped at
# the lane's top edge, so it is about 10 columns by 5 rows — wider than it is
# tall. A swatch is 4 columns by the button's full height. Requiring width >= height
# separates them at every UI scale, which a hard-coded y band would not.
def _is_head(width, height):
    return width >= height


def _near(pixel, colour, tolerance=2):
    return all(abs(pixel[i] - colour[i]) <= tolerance for i in range(3))


def measure(path):
    image = Image.open(path).convert("RGB")
    width, height = image.size
    pixels = image.load()

    # Opaque hits only: the head is drawn at full alpha over the lane, the line
    # is not, so an exact match finds heads and ignores the stems.
    by_type = defaultdict(list)
    for y in range(height):
        for x in range(width):
            pixel = pixels[x, y]
            for name, colour in TYPE_COLOURS.items():
                if _near(pixel, colour):
                    by_type[name].append((x, y))
                    break

    markers = []
    for name, points in by_type.items():
        columns = defaultdict(list)
        for x, y in points:
            columns[x].append(y)
        for cluster in _clusters(sorted(columns)):
            count = sum(len(columns[x]) for x in cluster)
            if count < MIN_HEAD_PIXELS:
                continue
            rows = [y for x in cluster for y in columns[x]]
            centre_x = (cluster[0] + cluster[-1]) // 2
            top, bottom = min(rows), max(rows)
            centre_y = (top + bottom) // 2
            if not _is_head(cluster[-1] - cluster[0] + 1, bottom - top + 1):
                continue
            # A ring has the lane's surface at its own centre; a disc has the
            # type colour there. Sampled as a small box rather than one pixel so
            # a half-pixel centre at 150 % scale cannot flip the answer.
            hole = any(
                _near(pixels[x, y], RAISED, 0)
                for x in range(centre_x - 1, centre_x + 2)
                for y in range(centre_y - 1, centre_y + 2)
                if 0 <= x < width and 0 <= y < height
            )
            markers.append(
                {
                    "x": centre_x,
                    "y": top,
                    "type": name,
                    "lane": "semantic" if hole else "manual",
                    "pixels": count,
                }
            )

    markers.sort(key=lambda marker: marker["x"])
    return markers


def _clusters(columns):
    out = []
    run = []
    for x in columns:
        if run and x - run[-1] > CLUSTER_GAP:
            out.append(run)
            run = []
        run.append(x)
    if run:
        out.append(run)
    return out


def main(argv):
    if not 2 <= len(argv) <= 3:
        print(__doc__, file=sys.stderr)
        return 2

    markers = measure(argv[1])
    observed = [f"{marker['lane']}:{marker['type']}" for marker in markers]
    for marker in markers:
        print(
            "  marker x={x:<5} y={y:<5} {lane:<9} {type:<9} head={pixels}px".format(
                **marker
            )
        )
    print(f"  markers: {len(markers)} [{', '.join(observed) or 'none'}]")

    if len(argv) == 2:
        return 0

    expected = [part for part in argv[2].split(",") if part]
    if observed != expected:
        print(
            f"FAIL: expected [{', '.join(expected)}] but read [{', '.join(observed)}]",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
