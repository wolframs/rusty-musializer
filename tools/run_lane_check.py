#!/usr/bin/env python3
"""Read the plan lane's runs off a capture (WX-5b).

The `scene runs:` report line says how many internal-cut hairlines the lane
drew and how its sections are graded. A report line is a claim about a frame;
this reads the frame. Two measurements, both from the report's own `lane=` rect
and `view=` span, so nothing here is a coordinate measured off a screenshot:

  hairlines PNG REPORT
      Columns inside the lane where every row of the run body is exactly
      `ui_run_cut`. The hairline is an opaque whole-pixel fill, and opaque fills
      are the one thing every renderer this gate supports writes bit-exactly
      (AGENTS.md, "The headless gate runs on the GPU"), so the match has no
      tolerance. Prints the count.

  grade-step PNG REPORT
      The mean colour of the grade bar under the first and the second section,
      and the largest channel difference between the two. A grade change inside
      a run must show in the bar under it; two sections on one grade draw the
      same ramp over the same width and read equal. Prints the difference.

Usage: run_lane_check.py {hairlines|grade-step} PNG REPORT
"""

import re
import sys

import numpy as np
from PIL import Image

# `theme::palette::{DARK,LIGHT}.ui_run_cut`, kept as literals so a change on
# either side shows up here as a mismatch rather than agreeing with itself.
RUN_CUT = {"black-amber": (0x5A, 0x5E, 0x66), "light-blue": (0xFB, 0xFA, 0xF7)}

# The run body inside the lane: the run is inset 2 px top and bottom
# (`scene_plan_lane`), and its top 3 rows carry the now-tick and a transition
# edge, its bottom 3 the grade bar and the run's edge, all drawn after or over
# the hairline's rows. The body between them is where a hairline is alone.
BODY_INSET_TOP = 2 + 4
BODY_INSET_BOTTOM = 2 + 4


def parse(report):
    text = open(report, encoding="utf-8", errors="replace").read()
    runs = re.search(
        r"^scene runs:.*lane=(\d+),(\d+),(\d+),(\d+) view=([\d.]+)\.\.([\d.]+)", text, re.M
    )
    appearance = re.search(r"^appearance:\s+(\S+)", text, re.M)
    scale = re.search(r"^ui layout:\s+scale=(\d+)", text, re.M)
    if not runs or not appearance or not scale:
        raise SystemExit(f"run_lane_check: {report} lacks scene runs:/appearance:/ui layout:")
    lane = tuple(float(v) for v in runs.groups()[:4])
    view = (float(runs.group(5)), float(runs.group(6)))
    return lane, view, appearance.group(1), int(scale.group(1)) / 100.0


def hairlines(png, report):
    (x, y, w, h), _, appearance, scale = parse(report)
    image = np.array(Image.open(png).convert("RGB")).astype(int)
    top = int(round((y + BODY_INSET_TOP) * scale))
    bottom = int(round((y + h - BODY_INSET_BOTTOM) * scale))
    left, right = int(round(x * scale)), int(round((x + w) * scale))
    body = image[top:bottom, left:right]
    exact = (body == np.array(RUN_CUT[appearance])).all(axis=-1).all(axis=0)
    return int(exact.sum())


def grade_step(png, report):
    (x, y, w, h), (start, end), _, scale = parse(report)
    image = np.array(Image.open(png).convert("RGB")).astype(float)
    # The bar's middle row: the bar is the run's bottom 3 rows above its edge.
    row = int(round((y + 2 + (h - 4) - 2) * scale))

    def x_at(seconds):
        return (x + (seconds - start) / (end - start) * w) * scale

    means = []
    for section_start, section_end in ((0.0, 1.0), (1.0, 2.0)):
        left = int(round(x_at(section_start))) + 4
        right = int(round(x_at(section_end))) - 4
        means.append(image[row, left:right].mean(axis=0))
    return float(np.abs(means[0] - means[1]).max())


def main(argv):
    if len(argv) != 4 or argv[1] not in ("hairlines", "grade-step"):
        print(__doc__, file=sys.stderr)
        return 2
    measure = hairlines if argv[1] == "hairlines" else grade_step
    value = measure(argv[2], argv[3])
    print(value if isinstance(value, int) else f"{value:.1f}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
