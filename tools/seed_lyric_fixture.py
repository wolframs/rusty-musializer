#!/usr/bin/env python3
"""Puts synthetic lyric cues into a generated `.musi`, so the lyrics editor has
something to photograph.

The repository rule is synthetic fixtures only, and `REWRITE_PLAN.md` records the
open question this answers: a round-trip fixture has to be *generated*, not
committed from a real project. So `tools/headless_check.sh` saves a project from
the synthetic sweep, this script writes cues into it, and the capture opens the
result.

Editing the file in place is safe because every digest in a `.musi` is over an
*asset* — the audio, the imported face, the licence — and never over the
document. The lyrics array carries none, which is why the project still opens and
validates afterwards. If that ever stops being true, the capture will say so: the
open path verifies every digest before a `Track` exists.
"""

import json
import pathlib
import sys

# Six lines, six cues: enough that the list scrolls at the 960x640 minimum and
# does not at 1280x720, which is the difference the two captures exist to show.
LINES = [
    "Hold the line, the sweep is rising",
    "Every band a step above",
    "Count the pulse, it never lies",
    "Down again to where we started",
    "And the spectrum keeps its shape",
    "One more turn before it ends",
]


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} PROJECT.musi", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    project = json.loads(path.read_text())
    duration = float(project["audio"]["duration_seconds"])
    cues = []
    for index, text in enumerate(LINES):
        start = 0.4 + index * 1.25
        end = start + 1.0
        if end > duration:
            break
        cues.append(
            {
                "id": index + 1,
                "start_seconds": round(start, 3),
                "end_seconds": round(end, 3),
                "text": text,
            }
        )
    if not cues:
        print("the fixture is too short to hold a cue", file=sys.stderr)
        return 1
    project["lyrics"] = {"next_id": len(cues) + 1, "cues": cues}
    path.write_text(json.dumps(project, indent=2))
    print(f"seeded {len(cues)} lyric cues into {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
