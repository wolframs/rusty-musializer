#!/usr/bin/env python3
"""Prove the waveform lane's tick labels sit on an opaque plate.

The C draws a `COLOR_UI_RAISED` rectangle under every tick timestamp
(`plug.c:3065-3080`) and its comment is a measurement, not a preference: the
waveform behind the labels is not a constant background. It runs from the raised
surface in a silent passage to a dense accent blue at full amplitude, where muted
ink measures about 1.16:1. The Rust port drew the label and dropped the plate.

That defect is nastier than a fixed contrast bug, which is why it needs a check
of its own: it **depends on the audio**. The labels are perfectly legible over a
quiet passage and unreadable over a loud one, so it comes and goes as the user
scrolls, and it photographs as fine in any capture that happened to land on a
quiet bar. A gate measuring overall contrast would pass.

So the assertion here is exact rather than a threshold: **no waveform pixel may
appear inside a tick label's plate**. With the plate that count is zero by
construction, because the plate repaints the box before the glyphs go down.
Without it, a label over a loud bar has the envelope's own columns between its
letters.

An exact zero is only worth having if the fixture can produce a non-zero, so this
also refuses to pass **vacuously**: at least one label must have the envelope
reaching its plate, otherwise every label is over silence and the check proved
nothing. That guard is the same idea as the layout harness refusing to pass when
it compared no non-finite columns.

Usage: timeline_tick_plate.py PNG
"""

import sys
from collections import defaultdict

try:
    from PIL import Image
except ImportError:  # pragma: no cover - the gate installs Pillow
    print("timeline_tick_plate.py needs Pillow", file=sys.stderr)
    raise

RAISED = (0xFF, 0xFF, 0xFF)

# The box judged is the **glyph run plus a one-pixel halo**, not the plate's own
# rectangle. Reconstructing the plate from `plug.c:3074-3079`'s `-3, -2, +6, +4`
# was the first attempt and it was wrong by three rows: those offsets are around
# the *text origin*, and a timestamp's tallest glyph starts a couple of pixels
# below it, so a box anchored on the ink and given the plate's height hangs off
# the bottom and reports the envelope under the plate as bleed *through* it.
#
# The halo is also the more honest question. What makes a label readable is
# whether there is anything between and immediately around its letters — not
# whether a rectangle of a particular size was drawn.
PLATE_PAD_X = 3
PLATE_PAD_Y = 1

# How far down from the lane's top edge a tick label can sit. The C puts it at
# `+4` (`plug.c:3072`); 30 covers that plus the plate and a 150 % scale.
LABEL_BAND_ROWS = 30

# A waveform column is the accent blue at some brightness and alpha over white,
# so every one of them is markedly bluer than it is red. Testing the *relation*
# rather than a colour value survives the peak-dependent brightness and alpha the
# envelope is drawn with — and, importantly, no grey antialiased glyph edge can
# satisfy it.
WAVEFORM_BLUE_MARGIN = 40

# Rows with at least this many waveform pixels are inside the lane. The envelope
# is one line per pixel column across the whole strip, so a real lane row has
# hundreds; nothing else in the chrome carries a blue band that wide.
MIN_WAVEFORM_ROW_PIXELS = 120

# Label ink. A 12 px glyph is antialiased, so most of its pixels are grey rather
# than `UI_INK` itself — the first version of this tool matched the ink colour
# within 24 and found *no labels at all* on a frame that plainly had six. What
# separates a glyph from the envelope is that a glyph is dark and neutral while
# the envelope is bright and blue, so that is what is tested.
INK_MAX_CHANNEL = 150

# A timestamp is `MM:SS.mmm` — nine glyphs, so its ink runs across tens of
# columns. Anything narrower is a stray dark pixel.
MIN_LABEL_COLUMNS = 20

# Glyphs within one label are a pixel or two apart; labels are a tick step apart.
LABEL_GAP = 8

# How far below a plate the envelope may be and still count the label as one the
# check has teeth on. Generous on purpose: the strip is 56 px and the envelope's
# amplitude is 43 % of that, so the label band sits near the top of the envelope's
# reach and the gap is a handful of pixels on ordinary audio.
EXPOSURE_ROWS = 12


def _is_waveform(pixel):
    red, green, blue = pixel
    return (
        blue > red + WAVEFORM_BLUE_MARGIN
        and blue > green + WAVEFORM_BLUE_MARGIN
        # `green > red` is what separates the envelope from the `custom` event
        # marker, and it cost a wrong failure to find. The envelope is `accent`
        # (0, 47, 167) brightened and alpha-blended over white, and every one of
        # those operations preserves `red < green`; the custom marker's purple
        # (151, 111, 241) is blue-dominant too but has `red > green`. Without
        # this the tool reported 27 "waveform" pixels inside a label that a
        # marker line was legitimately drawn across — markers draw after ticks in
        # the oracle as well (`plug.c:3086`).
        and green > red
    )


def _is_ink(pixel):
    return max(pixel) < INK_MAX_CHANNEL and not _is_waveform(pixel)


def _waveform_lane(pixels, width, height):
    """The row band the amplitude envelope occupies, found from the pixels.

    Located rather than passed in, because every hard-coded band height in this
    repository's checks has gone stale at least once, and a check that silently
    starts measuring the wrong rectangle passes forever.
    """
    dense = [
        y
        for y in range(height)
        if sum(1 for x in range(width) if _is_waveform(pixels[x, y]))
        >= MIN_WAVEFORM_ROW_PIXELS
    ]
    if not dense:
        return None
    runs = []
    run = [dense[0]]
    for y in dense[1:]:
        if y - run[-1] > 2:
            runs.append(run)
            run = []
        run.append(y)
    runs.append(run)
    longest = max(runs, key=len)
    return longest[0], longest[-1]


def _clusters(columns):
    out = []
    run = []
    for x in columns:
        if run and x - run[-1] > LABEL_GAP:
            out.append(run)
            run = []
        run.append(x)
    if run:
        out.append(run)
    return out


def measure(path):
    image = Image.open(path).convert("RGB")
    width, height = image.size
    pixels = image.load()

    lane = _waveform_lane(pixels, width, height)
    if lane is None:
        return None
    lane_top, lane_bottom = lane

    # Restricted to the lane's own top band. That is not a tidy-up: searching the
    # whole window found the sidebar's and the inspector's text, which sits on
    # the surface colour rather than the raised one — the same defect
    # `tools/timeline_lane_alignment.py` had, a search correctly restricted in
    # one axis and not the other.
    rows = defaultdict(list)
    for y in range(lane_top, min(height, lane_top + LABEL_BAND_ROWS)):
        for x in range(width):
            if _is_ink(pixels[x, y]):
                rows[y].append(x)

    boxes = []
    for y in sorted(rows):
        for cluster in _clusters(sorted(set(rows[y]))):
            if cluster[-1] - cluster[0] + 1 < MIN_LABEL_COLUMNS:
                continue
            for box in boxes:
                if cluster[0] <= box["x1"] + 2 and cluster[-1] >= box["x0"] - 2:
                    box["x0"] = min(box["x0"], cluster[0])
                    box["x1"] = max(box["x1"], cluster[-1])
                    box["y1"] = max(box["y1"], y)
                    break
            else:
                boxes.append(
                    {"x0": cluster[0], "x1": cluster[-1], "y0": y, "y1": y}
                )

    results = []
    for box in boxes:
        x0 = max(0, box["x0"] - PLATE_PAD_X)
        x1 = min(width - 1, box["x1"] + PLATE_PAD_X)
        y0 = max(0, box["y0"] - PLATE_PAD_Y)
        y1 = min(height - 1, box["y1"] + PLATE_PAD_Y)

        bleed = 0
        raised = 0
        total = 0
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                pixel = pixels[x, y]
                total += 1
                if _is_waveform(pixel):
                    bleed += 1
                elif pixel == RAISED:
                    raised += 1
        # Does the envelope reach this plate at all? Sampled in the rows
        # immediately under it: if it does, removing the plate would put those
        # pixels inside the box, so this label is one the check has teeth on.
        near = None
        for y in range(y1 + 1, min(height, y1 + 1 + EXPOSURE_ROWS)):
            if any(_is_waveform(pixels[x, y]) for x in range(x0, x1 + 1)):
                near = y - y1
                break
        results.append(
            {
                "x": x0,
                "y": y0,
                "width": x1 - x0 + 1,
                "bleed": bleed,
                "raised": raised / total if total else 0.0,
                "exposed": near,
            }
        )

    results.sort(key=lambda result: result["x"])
    return results


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    results = measure(argv[1])
    if results is None:
        print("FAIL: no waveform lane in the capture", file=sys.stderr)
        return 1
    if not results:
        print("FAIL: no tick label found to check", file=sys.stderr)
        return 1

    for result in results:
        print(
            "  label x={x:<5} y={y:<5} w={width:<4} bleed={bleed:<4} "
            "raised={raised:.3f} exposed={exposed}".format(**result)
        )
    bleed = sum(result["bleed"] for result in results)
    exposed = sum(1 for result in results if result["exposed"] is not None)
    print(f"  labels: {len(results)}  waveform bleed {bleed}  exposed {exposed}")

    status = 0
    if bleed != 0:
        print(
            f"FAIL: {bleed} waveform pixels inside a tick label's plate; "
            "the labels have no opaque backing",
            file=sys.stderr,
        )
        status = 1
    if exposed == 0:
        print(
            "FAIL: no label has the envelope reaching it, so this capture cannot "
            "tell a plate from its absence",
            file=sys.stderr,
        )
        status = 1
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv))
