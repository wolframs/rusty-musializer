#!/usr/bin/env python3
"""Check that a notice card the report names is on screen, in its severity.

The notice tray draws each card as a floating surface (WX-5 follow-up): the
appearance's raised fill, a hairline, a soft shadow, and a head of a 6 px dot
and a micro label in the severity's colour. The run's `notices:` line names
every card it drew — id, severity, card rect, dot centre, close box — in
logical pixels at `scale=`. This reads the card the line names, never a pixel
measured once, and checks two things in the capture:

  * the card's interior (inset past the hairline and the rounded corners) is
    mostly the floating surface, **exactly**: it is an opaque fill, which every
    renderer writes bit-exactly (docs/HEADLESS_GPU.md). Text covers the rest.
  * the dot is the severity's colour, **exactly**, on enough pixels to be a
    dot: a 6 px disc, opaque, no blend. Coverage at its rim splits differently
    per rasterizer, so the floor is well under the disc's area.

The shadow is a blend and is never read.

The colours are the palette's, pinned here rather than read from the report,
so a card drawn in the wrong colour fails instead of agreeing with itself:
`Palette::float_surface` (= `ui_raised`) and `Palette::notice_severity` in
crates/musializer-app/src/ui/theme.rs. The appearance comes from the report's
`appearance:` line.

Usage:
  notice_card_check.py PNG REPORT --severity NAME
      The newest card of that severity must be drawn, and must measure as one.
  notice_card_check.py PNG REPORT --severity NAME --absent-at OTHER_REPORT
      The control: REPORT names no card of that severity, and at the rect
      OTHER_REPORT gave that card, PNG shows no dot of its colour.

Exit 0 on pass, 1 on a failed check, 2 on bad usage.
"""

import re
import sys

import numpy as np
from PIL import Image

# `Palette::float_surface` and `Palette::notice_severity`, per appearance.
PALETTES = {
    "light-blue": {
        "float": (0xFF, 0xFF, 0xFF),
        "info": (0x62, 0x62, 0x67),
        "done": (0x16, 0x6E, 0x47),
        "warning": (0x97, 0x58, 0x00),
        "error": (0xC6, 0x28, 0x28),
    },
    "black-amber": {
        "float": (0x17, 0x1A, 0x1F),
        "info": (0x92, 0x96, 0xA0),
        "done": (0x7D, 0xB3, 0x8C),
        "warning": (0xEB, 0x85, 0x5E),
        "error": (0xF2, 0x70, 0x7F),
    },
}
# The interior must be at least this much floating surface. Measured 0.85-0.93
# on the gate's cards; a card that is not drawn reads the scene, 0.
FILL_FLOOR = 0.60
# A radius-3 disc is ~28 px at 100 %; the rim splits per renderer.
DOT_FLOOR = 12
# Past the hairline (1 px) and the 4 px corner radius.
INSET = 5.0


def line(report, prefix):
    for text in report.splitlines():
        if text.startswith(prefix):
            return text[len(prefix):].strip()
    return None


def cards(report):
    notices = line(report, "notices:")
    if notices is None:
        sys.exit(f"notice_card_check: the report has no notices: line")
    scale = float(re.search(r"scale=([\d.]+)", notices).group(1)) if "scale=" in notices else 1.0
    found = []
    for match in re.finditer(
        r"id=(\d+) (\w+) card=([\d.-]+),([\d.-]+),([\d.-]+),([\d.-]+) dot=([\d.-]+),([\d.-]+)",
        notices,
    ):
        found.append(
            {
                "id": int(match.group(1)),
                "severity": match.group(2),
                "card": tuple(float(v) for v in match.group(3, 4, 5, 6)),
                "dot": (float(match.group(7)), float(match.group(8))),
            }
        )
    return notices, scale, found


def appearance(report):
    value = line(report, "appearance:")
    if value is None:
        sys.exit("notice_card_check: the report has no appearance: line")
    name = value.split()[0]
    if name not in PALETTES:
        sys.exit(f"notice_card_check: unknown appearance {name}")
    return name


def dot_pixels(image, dot, scale, colour):
    cx, cy = dot[0] * scale, dot[1] * scale
    reach = int(round(5 * scale))
    x0, y0 = int(cx) - reach, int(cy) - reach
    box = image[max(y0, 0):int(cy) + reach + 1, max(x0, 0):int(cx) + reach + 1]
    return int((box == np.array(colour)).all(axis=2).sum())


def fill_fraction(image, card, scale, colour):
    x, y, w, h = (v * scale for v in card)
    inset = INSET * scale
    box = image[int(y + inset):int(y + h - inset), int(x + inset):int(x + w - inset)]
    if box.size == 0:
        return 0.0
    return float((box == np.array(colour)).all(axis=2).mean())


def main(argv):
    if len(argv) not in (5, 7) or argv[3] != "--severity":
        print(__doc__, file=sys.stderr)
        return 2
    png, report_path, severity = argv[1], argv[2], argv[4].lower()
    report = open(report_path, encoding="utf-8").read()
    look = PALETTES[appearance(report)]
    if severity not in look or severity == "float":
        print(f"notice_card_check: unknown severity {severity}", file=sys.stderr)
        return 2
    image = np.array(Image.open(png).convert("RGB")).astype(int)
    notices, scale, drawn = cards(report)
    mine = [card for card in drawn if card["severity"] == severity]

    if len(argv) == 7:
        if argv[5] != "--absent-at":
            print(__doc__, file=sys.stderr)
            return 2
        other = open(argv[6], encoding="utf-8").read()
        _, other_scale, other_cards = cards(other)
        theirs = [card for card in other_cards if card["severity"] == severity]
        if not theirs:
            print(f"notice_card_check: {argv[6]} names no {severity} card to aim at")
            return 1
        aim = theirs[0]
        dot = dot_pixels(image, aim["dot"], other_scale, look[severity])
        print(
            f"control: {len(mine)} {severity} card(s) in the report, "
            f"{dot} {severity} dot px at the other run's dot"
        )
        return 0 if not mine and dot == 0 else 1

    if not mine:
        print(f"no {severity} card in the notices: line ({notices})")
        return 1
    card = mine[0]
    fill = fill_fraction(image, card["card"], scale, look["float"])
    dot = dot_pixels(image, card["dot"], scale, look[severity])
    dot_floor = int(DOT_FLOOR * scale * scale)
    print(
        f"{severity} card id={card['id']} at {','.join(f'{v:.0f}' for v in card['card'])}: "
        f"fill {fill:.2f} (floor {FILL_FLOOR}), dot {dot} px (floor {dot_floor})"
    )
    return 0 if fill >= FILL_FLOOR and dot >= dot_floor else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
