#!/usr/bin/env python3
"""Seed a `.musi` project with manual and semantic events, for the marker gate.

The waveform lane draws the merged manual/semantic event view (D4). Nothing in
the ordinary headless sweep produces either lane: `--event` can record into the
manual lane from the command line, but the semantic lane is only ever written by
an Assist run, and the whole point of the markers is that a user can tell the two
apart at a glance. A capture with one lane populated cannot show that.

So this writes both, into a project the gate has already saved — the same
generate-don't-commit rule `tools/seed_lyric_fixture.py` follows, and for the
same reason: the repository takes synthetic fixtures only.

The timestamps are chosen, not arbitrary:

  * four manual events inside the first eight seconds, one of each type, so all
    four type colours are on screen at once and a colour regression cannot hide
    behind a fixture that only ever draws amber;
  * one of them is `semantic`-typed **in the manual lane** — which is what the
    `+ Feel` button records (`plug.c:2897`) — so the capture contains the exact
    case where type does not tell you the lane, and only the head shape does;
  * two semantic events at times that interleave with the manual ones, so the
    merge's sort has to re-pair the lane with its record rather than emitting one
    lane and then the other;
An off-track event is deliberately **not** seeded here, and finding out why was
worth the round trip: `Project::validate_event_lanes` rejects any lane holding a
`timestamp_seconds` past `audio.duration_seconds`, so a `.musi` cannot carry one
at all. The oracle's own bound (`plug.c:3089`) is therefore reachable only
through *live* recording, which is what the gate uses `--event` for.

Usage: seed_event_fixture.py PROJECT.musi [DURATION_SECONDS]
"""

import json
import sys

# Inside the fixture: one of each type, so all four colours draw.
MANUAL = [
    (0.75, 1, "lyric", [1.0]),
    (2.50, 2, "cue", [1.0]),
    (4.25, 3, "custom", [1.0]),
    # The crossing case: `+ Feel` records a *semantic-typed* event into the
    # manual lane, so this marker is amber like a real semantic one and must
    # still draw a filled head.
    (6.00, 4, "semantic", [1.0]),
]

# Interleaved with the manual times on purpose, so a lane list that was not
# permuted alongside the records would put the wrong head on the wrong marker.
SEMANTIC = [
    (1.60, 1, [0.62, 0.30, 0.20, 0.90]),
    (5.10, 2, [0.80, 0.55, -0.10, 0.75]),
]


def main(argv):
    if not 2 <= len(argv) <= 3:
        print(__doc__, file=sys.stderr)
        return 2
    path = argv[1]
    duration = float(argv[2]) if len(argv) == 3 else 8.0

    with open(path, "r", encoding="utf-8") as handle:
        project = json.load(handle)

    project["manual_events"] = [
        {
            "timestamp_seconds": t,
            "id": i,
            "type": kind,
            "values": values,
        }
        for (t, i, kind, values) in MANUAL
    ]
    project["semantic_events"] = [
        {
            "timestamp_seconds": t,
            "id": i,
            "type": "semantic",
            "values": values,
        }
        for (t, i, values) in SEMANTIC
    ]

    with open(path, "w", encoding="utf-8") as handle:
        json.dump(project, handle, indent=2)
        handle.write("\n")

    print(
        f"seeded {len(project['manual_events'])} manual and "
        f"{len(SEMANTIC)} semantic events into {path} (track {duration:g}s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
