#!/usr/bin/env python3
"""Appearance-independent measurements of a crop of the chrome (WX-5a).

The gate's panel checks used to read absolute luma off a light interface: "the
darkest pixel is under 200, so text was drawn on the ~247 panel". Since WX-5a
the default appearance is dark, where the same test passes on a blank panel —
every pixel is under 200 — and a check that cannot fail is not a check. These
measure *against the crop's own background* instead, so one threshold means the
same thing in both appearances:

  ink PNG W:H:X:Y    the largest luma distance from the crop's modal luma.
                     The background is the mode even when text covers a large
                     share of the crop, because anti-aliased glyphs spread over
                     many values. In the light appearance this is exactly the
                     old "247 minus the darkest pixel"; in the dark one it is
                     "the brightest pixel minus 19".
  fill PNG W:H:X:Y   the modal luma itself: what the crop's background is.
  changed A B W:H:X:Y
                     how many pixels of the crop differ by 24 or more in luma
                     between two captures of the same layout: what a state
                     drew that the other did not. A focus ring is neutral since
                     WX-5a (it used to be the accent, and was found by chroma),
                     so it is found as the pixels it adds.

Crops use ffmpeg's `crop=W:H:X:Y` order so a call site can keep its numbers.
Luma is BT.601 on full-range RGB, rounded.
"""

import sys

import numpy as np
from PIL import Image


def crop(path, spec):
    width, height, x, y = (int(v) for v in spec.split(":"))
    image = np.array(Image.open(path).convert("RGB")).astype(float)
    region = image[y:y + height, x:x + width]
    if region.size == 0:
        raise SystemExit(f"chrome_probe: crop {spec} is outside {path}")
    luma = np.rint(region @ np.array([0.299, 0.587, 0.114])).astype(int)
    return luma


def modal(luma):
    values, counts = np.unique(luma, return_counts=True)
    return int(values[counts.argmax()])


def main(argv):
    if len(argv) == 5 and argv[1] == "changed":
        first, second = crop(argv[2], argv[4]), crop(argv[3], argv[4])
        if first.shape != second.shape:
            raise SystemExit("chrome_probe: the two captures are different sizes")
        print(int((np.abs(first - second) >= 24).sum()))
        return 0
    if len(argv) != 4 or argv[1] not in ("ink", "fill"):
        print(__doc__, file=sys.stderr)
        return 2
    luma = crop(argv[2], argv[3])
    background = modal(luma)
    if argv[1] == "fill":
        print(background)
    else:
        print(int(np.abs(luma - background).max()))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
