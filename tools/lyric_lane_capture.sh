#!/usr/bin/env bash
# Photographs the lyric cue lane's LX1-d surfaces: the per-origin colour code,
# the overlap fan-out, the legend, the tooltip and the resizable lane.
#
# Separate from tools/headless_check.sh on purpose, and it is not a second gate:
# two other tranches are editing that script in this session, and a capture that
# cannot be run is worse than one that lives in its own file until it can be
# folded in. The block to add to the gate is in LX1-d's report.
#
# Isolation follows tools/headless_check.sh exactly — private Xvfb, no
# WAYLAND_DISPLAY, an unresolvable PULSE_SERVER, and --mute on every launch — with
# one addition it should also have: MUSIALIZER_UI_PREFERENCES points at a scratch
# file, so a capture measures the application rather than whatever timeline split
# the operator last dragged on this machine.
set -euo pipefail

OUT_DIR="${1:-build/lyric-lane}"
DISPLAY_NUM="${MUSIALIZER_CAPTURE_DISPLAY:-:78}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

echo "=== building ==="
cargo build --quiet
cargo run --quiet --bin make-fixture-wav -- "$OUT_DIR/source.wav" 8 >/dev/null

if xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1; then
    echo "error: $DISPLAY_NUM is already in use. Set MUSIALIZER_CAPTURE_DISPLAY." >&2
    exit 1
fi
Xvfb "$DISPLAY_NUM" -screen 0 1920x1200x24 -nolisten tcp >"$OUT_DIR/xvfb.log" 2>&1 &
XVFB_PID=$!
trap 'kill "$XVFB_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
    xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1 && break
    sleep 0.1
done

PREFS="$OUT_DIR/ui.json"
PROJECT="$OUT_DIR/overlaps.musi"

run() {
    # run LOGNAME -- ARGS...
    local name="$1"; shift; shift
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-lyric-lane" \
        MUSIALIZER_UI_PREFERENCES="$PREFS" \
        ./target/debug/musializer --mute "$@" >"$OUT_DIR/$name.txt" 2>&1
    local status=$?
    set -e
    return $status
}

echo "=== the fixture: overlapping cues in four provenances ==="
run save -- "$OUT_DIR/source.wav" --save-project "$PROJECT" || {
    echo "FAIL: could not save the fixture project" >&2
    exit 1
}

python3 - "$PROJECT" <<'PY'
import json, pathlib, sys

path = pathlib.Path(sys.argv[1])
project = json.loads(path.read_text())
duration = float(project["audio"]["duration_seconds"])

# One flat run, then a cluster of four live at once, then a pair. The cluster is
# what makes `stack rows` greater than 1; the pair is the case the fan-out
# deliberately leaves flat, so one capture shows both rules.
cues = [
    (0.30, 1.20, "user", "A line I placed myself"),
    (1.60, 2.60, "certain", "The aligner is sure about this one"),
    (2.20, 3.60, "ambiguous", "Coarse and fine disagree here"),
    (2.40, 4.20, "potential", "Could not be placed at all"),
    (2.80, 4.60, "user", "And this one covers all of them"),
    (5.20, 6.40, "certain", "Back to a single line"),
    (6.10, 7.00, "ambiguous", "Tail laps into the next"),
]
records = []
for index, (start, end, origin, text) in enumerate(cues):
    assert end <= duration, f"{end} past the {duration}s fixture"
    record = {
        "id": index + 1,
        "start_seconds": start,
        "end_seconds": end,
        "text": text,
    }
    if origin != "user":
        record["origin"] = origin
    records.append(record)

project["lyrics"] = {"next_id": len(records) + 1, "cues": records}
path.write_text(json.dumps(project, indent=2) + "\n")
print(f"seeded {len(records)} cues in {len({c[2] for c in cues})} provenances")
PY

capture() {
    # capture NAME SIZE PROBE [EXTRA...]
    local name="$1" size="$2" probe="$3"; shift 3
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-lyric-lane" \
        MUSIALIZER_UI_PREFERENCES="$PREFS" \
        ./target/debug/musializer --mute --project "$PROJECT" \
            --size "$size" \
            --probe-frames 30 \
            --probe-shot "$OUT_DIR/$name.png" \
            --ui-probe "$probe" \
            "$@" \
        >"$OUT_DIR/$name.txt" 2>&1
    local status=$?
    set -e
    printf '%-28s %-9s exit=%s ' "$name" "$size" "$status"
    sed -n 's/^lyrics: *//p' "$OUT_DIR/$name.txt" | head -1
    return $status
}

FAILED=0

echo "=== the lane at both heights, at both gate sizes ==="
printf '{"schema":"musializer.ui-preferences/v1","scale":"auto","sidebar_width":null,"inspector_width":null,"timeline_height":null}\n' >"$PREFS"
capture lane-base-1280x720 1280x720 "panel=lyrics" || FAILED=1
capture lane-base-960x640 960x640 "panel=lyrics,play=1" || FAILED=1

printf '{"schema":"musializer.ui-preferences/v1","scale":"auto","sidebar_width":null,"inspector_width":null,"timeline_height":null,"lyric_lane_height":66.0}\n' >"$PREFS"
capture lane-max-1280x720 1280x720 "panel=lyrics,play=1" || FAILED=1
capture lane-max-960x640 960x640 "panel=lyrics,play=1" || FAILED=1
capture lane-max-1920x1080 1920x1080 "panel=lyrics,play=1" || FAILED=1

echo "=== the tooltip, parked over the middle of the overlap cluster ==="
printf '{"schema":"musializer.ui-preferences/v1","scale":"auto","sidebar_width":null,"inspector_width":null,"timeline_height":null}\n' >"$PREFS"
# The hover point is *found*, not guessed: the lane's position depends on the
# window, the UI scale and the cue count, and a hard-coded pair is how
# `--ui-probe hover=` silently photographs the wrong thing.
read -r LANE_TOP LANE_BOTTOM HOVER_X HOVER_Y <<<"$(python3 - "$OUT_DIR/lane-base-1280x720.png" <<'PY'
import sys
from PIL import Image

BORDERS = {(0x1F, 0x7A, 0x4D), (0x25, 0x63, 0xC7), (0xD0, 0x7A, 0x00), (0x7C, 0x7C, 0x86)}
image = Image.open(sys.argv[1]).convert("RGB")
width, height = image.size
pixels = image.load()

# Every row that carries a block border, then the *first* contiguous run of them
# from the top: that is the cue lane. The later runs are the cue list's own
# per-row swatches and the legend, which share the palette on purpose.
rows = [y for y in range(height) if any(pixels[x, y] in BORDERS for x in range(0, width, 2))]
run = [rows[0]]
for y in rows[1:]:
    if y - run[-1] > 2:
        break
    run.append(y)
top, bottom = run[0], run[-1]
middle = (top + bottom) // 2

# Then a point *inside* a block rather than on its border: the widest run of
# non-white pixels across the lane's own x range on the middle row. Aiming at a
# border column is how the first attempt landed a hair outside the cue and
# photographed no tooltip at all.
lane_columns = [x for x in range(width) if any(pixels[x, y] in BORDERS for y in range(top, bottom + 1))]
lo, hi = min(lane_columns), max(lane_columns)
best, current = [], []
for x in range(lo, hi + 1):
    if pixels[x, middle] != (255, 255, 255):
        current.append(x)
    else:
        if len(current) > len(best):
            best = current
        current = []
if len(current) > len(best):
    best = current
print(top, bottom, (best[0] + best[-1]) // 2, middle)
PY
)"
echo "cue lane rows $LANE_TOP..$LANE_BOTTOM, hovering ${HOVER_X}x${HOVER_Y}"
capture lane-tip-1280x720 1280x720 "panel=lyrics,hover=${HOVER_X}x${HOVER_Y}" || FAILED=1

echo "=== the report lines ==="
for pair in \
    "lane-base-1280x720:stack rows 3" \
    "lane-base-960x640:stack rows 3" \
    "lane-max-1920x1080:stack rows 3" \
    "lane-base-1280x720:origins user=2 certain=2 ambiguous=2 potential=1" \
    "lane-base-1280x720:lane height 50" \
    "lane-max-1920x1080:lane height 66" \
    "lane-base-960x640:lane height 46" \
    "lane-tip-1280x720:tip on"; do
    name="${pair%%:*}"
    want="${pair##*:}"
    case "$(sed -n 's/^lyrics: *//p' "$OUT_DIR/$name.txt" | head -1)" in
        *"$want"*) ;;
        *) echo "FAIL: $name did not report '$want'" >&2; FAILED=1 ;;
    esac
done

echo "=== the sidebar keeps its tracks panel ==="
for name in lane-base-1280x720 lane-max-1280x720 lane-base-960x640 lane-max-960x640 lane-max-1920x1080; do
    line="$(sed -n 's/^chrome: *//p' "$OUT_DIR/$name.txt" | head -1)"
    printf '%-28s %s\n' "$name" "$line"
done

echo "=== the colour code reached the pixels ==="
# The lane's own band, measured for the four block colours. A capture that
# photographed one amber for everything cannot produce four distinct hues, and
# the `Potential` grey cannot be produced by any of the other three.
python3 - "$OUT_DIR" <<'PY'
import pathlib, sys
from PIL import Image

out = pathlib.Path(sys.argv[1])
wanted = {
    "user": (0x1F, 0x7A, 0x4D),
    "certain": (0x25, 0x63, 0xC7),
    "ambiguous": (0xD0, 0x7A, 0x00),
    "potential": (0x7C, 0x7C, 0x86),
}
failed = False
for name in ("lane-base-1280x720", "lane-max-1920x1080"):
    image = Image.open(out / f"{name}.png").convert("RGB")
    counts = image.getcolors(maxcolors=1 << 22) or []
    present = {colour for _, colour in counts}
    found = {}
    for token, base in wanted.items():
        # Rows below the top one are shaded by 20 % a step, so accept the ladder.
        for step in range(5):
            shade = tuple(int(channel * (0.8 ** step)) for channel in base)
            if shade in present:
                found[token] = step
                break
    print(f"{name}: block colours found {sorted(found)} at shade steps {found}")
    if len(found) != 4:
        print(f"FAIL: {name} drew {len(found)} of the 4 origin colours", file=sys.stderr)
        failed = True
sys.exit(1 if failed else 0)
PY
[ $? -ne 0 ] && FAILED=1

echo "=== the tooltip is outside the cue lane ==="
# The operator's hard requirement, measured rather than reasoned about: the tip
# must not cover the blocks the user is scanning. Both frames are paused at the
# same instant, so every pixel that moved between them belongs to the tooltip
# and the hover highlight.
python3 - "$OUT_DIR" "$LANE_TOP" "$LANE_BOTTOM" <<'PY'
import pathlib, sys
from PIL import Image
import numpy as np

out = pathlib.Path(sys.argv[1])
lane_top, lane_bottom = int(sys.argv[2]), int(sys.argv[3])
plain = np.array(Image.open(out / "lane-base-1280x720.png").convert("RGB")).astype(int)
tipped = np.array(Image.open(out / "lane-tip-1280x720.png").convert("RGB")).astype(int)
difference = np.abs(plain - tipped).sum(axis=2)
rows = np.where(difference.max(axis=1) > 40)[0]
if rows.size == 0:
    print("FAIL: the hover frame is identical to the plain one", file=sys.stderr)
    sys.exit(1)
print(f"changed rows {rows.min()}..{rows.max()}, cue lane rows {lane_top}..{lane_bottom}")

# Inside the lane only the hovered block's own fill may change, and it changes
# by the alpha step from 0.38 to 0.68 — a wash, not a plate. The tooltip is
# white on near-black ink, so its rows contain pixels that went *very* dark.
inside = [y for y in rows if lane_top <= y <= lane_bottom]
ink = [
    y
    for y in inside
    if (tipped[y].sum(axis=1) < 120).any() and not (plain[y].sum(axis=1) < 120).any()
]
if ink:
    print(f"FAIL: the tooltip plate covered lane rows {ink[:8]}", file=sys.stderr)
    sys.exit(1)
above = [y for y in rows if y < lane_top]
print(
    f"the tooltip drew {len(above)} rows above the lane and no plate inside it; "
    f"{len(inside)} lane rows changed, all of them the hover wash"
)
PY
[ $? -ne 0 ] && FAILED=1

# The resize grip is drawn, and drawn where a pointer can reach it.
#
# This check exists because the grip was invisible in every frame the
# application has ever rendered: `lane_resize_gesture` runs before the lane is
# filled, so the dimple it painted was covered by `widgets::fill(d, lane, ..)`
# on the very next line. Nothing caught it -- the gesture worked, the constant
# was right, `cargo test` was green, and the operator reported the resize as
# simply not working. A surface nothing photographs does not get reviewed.
echo "=== the resize grip is on screen ==="
python3 - "$OUT_DIR" <<'GRIP'
import pathlib, sys
from PIL import Image
import numpy as np

out = pathlib.Path(sys.argv[1])
failed = False
# Only the frames whose window can actually seat a taller lane draw a grip: a
# handle that cannot move invites a drag that does nothing, so at 1280x720 its
# *absence* is the assertion.
for name, expected in (("lane-max-1920x1080", True), ("lane-base-1280x720", False)):
    image = np.array(Image.open(out / f"{name}.png").convert("RGB")).astype(int)
    trough = np.abs(image - np.array([230, 230, 234])).max(axis=-1) <= 4
    seams = [y for y in range(image.shape[0]) if np.flatnonzero(trough[y]).size >= 300]
    if not seams:
        print(f"FAIL: {name}: no lane seams, so the lane could not be located", file=sys.stderr)
        failed = True
        continue
    top = seams[-1] + 1
    rule = np.abs(image - np.array([210, 210, 214])).max(axis=-1) <= 6
    bottom = next(
        (y for y in range(top + 12, min(top + 140, image.shape[0]))
         if np.flatnonzero(rule[y]).size > image.shape[1] * 0.75),
        None,
    )
    if bottom is None:
        print(f"FAIL: {name}: no bottom rule under the cue lane", file=sys.stderr)
        failed = True
        continue
    # Two 1 px lines just above the lane's bottom edge, centred, and narrow: a
    # full-width run is a border, not a handle.
    # The longest *contiguous* dark run in each of the last few rows, not every
    # dark column in them. Two things live down there besides the grip: the
    # playhead, which is one or two columns wherever the transport happens to
    # be, and a cue block's own border. Taking all dark columns made the run
    # look 1919 px wide because of the playhead; taking the longest run and then
    # demanding it be narrow, centred and contiguous excludes both.
    #
    # A row range rather than two exact rows, because the grip's 5 px and 3 px
    # offsets are logical and this frame is on the 1.25 scale rung.
    width = image.shape[1]
    found = []
    for y in range(bottom - 8, bottom):
        dark = image[y].max(axis=-1) < 225
        best, run, start = (0, 0), 0, 0
        for x, on in enumerate(dark):
            if on:
                if run == 0:
                    start = x
                run += 1
                if run > best[1]:
                    best = (start, run)
            else:
                run = 0
        start, length = best
        if length < 12 or length > width * 0.1:
            continue
        centre = start + length / 2.0
        if abs(centre - width / 2.0) <= width * 0.02:
            found.append((y, int(length)))
    if expected and len(found) < 2:
        print(f"FAIL: {name}: the resize grip is not on screen (found {found})", file=sys.stderr)
        failed = True
    elif not expected and found:
        print(f"FAIL: {name}: a grip on a lane that cannot grow ({found})", file=sys.stderr)
        failed = True
    else:
        state = f"grip rows {[y for y, _ in found]}" if found else "no grip, correctly"
        print(f"{name}: lane {top}..{bottom}, {state}")
sys.exit(1 if failed else 0)
GRIP
[ $? -ne 0 ] && FAILED=1

if [ "$FAILED" -ne 0 ]; then
    echo "FAIL: the lyric lane capture found a problem" >&2
    exit 1
fi
echo "OK: lyric lane captures in $OUT_DIR"
