#!/usr/bin/env python3
"""EX4-c: a late picture while the injected stall loses no audio.

Usage: audio_stall_check.py REPORT {gpu,cpu}

The CPU's cold startup may fill the ring before the first drain. The raw
boundaries on `audio stall:` isolate frames 60..119 without resetting or hiding
the lifetime counters. GPU playback must also retain its lifetime zero losses.
"""

import math
import pathlib
import re
import sys


def check(report, renderer):
    text = pathlib.Path(report).read_text()
    budget = re.search(
        r"^frame budget:\s+worst ([\d.]+)ms at frame (\d+), (\d+) of (\d+) stalled past ([\d.]+)ms$",
        text, re.M,
    )
    audio = re.search(
        r"^audio frames:\s+(\d+) consumed, (\d+) dropped, peak ring fill (\d+)%$",
        text, re.M,
    )
    output = re.search(r"^output underruns:\s+(\d+)$", text, re.M)
    window = re.search(
        r"^audio stall:\s+requested=(\d+)ms frames=(\d+)\.\.(\d+) observed=(\d+) "
        r"peak=(\d+)/(\d+) dropped=(\d+)\.\.(\d+) underruns=(\d+)\.\.(\d+)$",
        text, re.M,
    )
    if not all((budget, audio, output, window)):
        return ["missing or malformed frame/audio/stall diagnostic"], None

    requested, first, last, count, peak, capacity, db, da, ub, ua = map(int, window.groups())
    worst = float(budget[1])
    errors = []
    if (requested, first, last, count, capacity) != (120, 60, 119, 60, 8192):
        errors.append(f"wrong intervention/window: {window[0]}")
    if not math.isfinite(worst) or worst < 120 or budget.groups()[1:] != ("61", "1", "120", "25.0"):
        errors.append(f"the 120 ms intervention was not the single late frame: {budget[0]}")
    # These are the buffer capacities, not a percentage fitted to one driver.
    # Above one drain's 4096 frames, below the 8192-frame overflow boundary:
    # exactly the silent band EX4-c was written to observe. Callback phase makes
    # the occupancy stochastic; the intervention, bounds and zero deltas are not.
    if not 4096 < peak < capacity:
        errors.append(f"stall occupancy {peak}/{capacity} is outside the silent band (4096, 8192)")
    if da != db or int(audio[2]) != da:
        errors.append(f"the stall/recovery dropped frames: before={db} after={da} lifetime={audio[2]}")
    if (ub, ua, int(output[1])) != (0, 0, 0):
        errors.append(f"output starvation: before={ub} after={ua} lifetime={output[1]}")
    if int(audio[1]) == 0:
        errors.append("the analyzer consumed no PCM")
    if renderer == "gpu" and (db, da) != (0, 0):
        errors.append(f"GPU lifetime playback lost frames: {db}..{da}")
    detail = (peak, capacity, db, da, int(audio[3]))
    return errors, detail


def main(argv):
    if len(argv) != 3 or argv[2] not in ("gpu", "cpu"):
        print(__doc__, file=sys.stderr)
        return 2
    errors, detail = check(argv[1], argv[2])
    if detail:
        peak, capacity, before, after, lifetime_peak = detail
        print(f"EX4-c {argv[2]}: controlled peak={peak}/{capacity}, dropped={after - before}, underruns=0 required")
        if argv[2] == "cpu":
            print(
                "CPU cold-start limitation: "
                f"pre-control dropped={before}, lifetime peak={lifetime_peak}%. "
                "The silent-band assertion covers frames 60..119; startup is outside that window."
            )
    for error in errors:
        print(f"FAIL: {error}", file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main(sys.argv))
