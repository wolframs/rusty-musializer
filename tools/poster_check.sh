#!/usr/bin/env bash
#
# The poster-frame gate (WX-5b): one rendered frame per plan-lane run, from
# this track's own audio, drawn by a second renderer the preview never sees.
#
#   tools/poster_check.sh [OUTPUT_DIR]        # default build/poster-check
#
# Every launch goes through tools/headless_run.sh (private Xvfb, unreachable
# Wayland and audio, no dialogs, --mute). The fixture is synthetic: a 20 s
# make-fixture-wav track and a four-run treatment (clawd, threshold, stage,
# spectrum) — Clawd because it keeps light between frames, Threshold because it
# nests a second scene, so both are where a shared renderer would leak.
#
# Checks, each with the control that keeps it honest:
#
#   1. Determinism  two `poster-dump`s of the same recipes are byte-equal;
#                   control: the four posters of one run are pairwise distinct,
#                   so equality is not a property of blank tiles.
#   2. Isolation    paused inside the Clawd run and inside the Threshold run,
#                   the frame the posters are drawn in is byte-equal to the
#                   posters-off frame outside the plan lane, and 38 frames later the preview crop
#                   still is; control: the crops at the two moments differ.
#   3. Invalidation a followed-treatment edit to one section re-renders exactly
#                   that run (refreshed=1, one new dump, the others untouched);
#                   control: rewriting the file unchanged re-renders nothing.
#   4. Budget       unsettled and paused, every poster still arrives,
#                   step-worst <= the paused budget + the worst replay frame,
#                   and no more frames stall than with posters off.
#   5. GPU state    `gl framebuffer: mismatched-frames=0` with posters on.
#   6. In the lane  each run's slot, settled, matches its dumped poster upright
#                   better than flipped; control: the posters-off lane does
#                   not match at all.
#
# POSTER_BIN names a different binary, for a negative control without
# replacing the build. MUSIALIZER_CAPTURE_DISPLAY is not needed: headless_run
# picks a free display of its own.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/poster-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${POSTER_BIN:-$(realpath target/debug/musializer)}"

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 20 >/dev/null

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
line_of() { # prefix log
    grep -E "^$1" "$2" | head -n 1 || true
}
field() { # name line -> the value after name=
    sed -n "s/.*[[:space:]]$1=\\([^[:space:]]*\\).*/\\1/p" <<<"$2"
}

treatment() { # stage-settings-json -> the fixture treatment on stdout
    printf '{"schema":"musializer.treatment/v1","audio":{"path":"fixture.wav"},"segments":[%s]}\n' \
        "{\"at\":0,\"scene\":\"clawd\"},{\"at\":5,\"scene\":\"threshold\"},{\"at\":10,\"scene\":\"stage\",\"settings\":$1},{\"at\":15,\"scene\":\"spectrum\"}"
}
PLAN="$OUT/plan.treatment.json"
treatment '{"layout":0}' >"$PLAN"

run() { # name probe-keys frames -> log and capture under $OUT/name.*
    tools/headless_run.sh --treatment "$PLAN" --probe-frames "$3" \
        --ui-probe "$2" --probe-shot "$OUT/$1.png" >"$OUT/$1.log" 2>&1 || true
}

# 1. Determinism.
run dump-a "time=3.0,posters=settle,poster-dump=$OUT/dump-a" 30
run dump-b "time=3.0,posters=settle,poster-dump=$OUT/dump-b" 30
posters="$(line_of 'posters: ' "$OUT/dump-a.log")"
printf '%s\n' "${posters:0:240}"
[ "$(field ready "$posters")" = 4 ] || fail "a settled run did not make all four posters: $posters"
count_a="$(find "$OUT/dump-a" -name '*.png' 2>/dev/null | wc -l)"
[ "$count_a" = 4 ] || fail "the first dump holds $count_a posters, not 4"
for file in "$OUT"/dump-a/*.png; do
    other="$OUT/dump-b/$(basename "$file")"
    if [ ! -f "$other" ]; then
        fail "the second run has no $(basename "$file"): its recipe key moved between two runs"
    elif ! cmp -s "$file" "$other"; then
        fail "$(basename "$file") differs between two runs of the same recipe"
    fi
done
distinct="$(md5sum "$OUT"/dump-a/*.png 2>/dev/null | cut -d' ' -f1 | sort -u | wc -l)"
[ "$distinct" = 4 ] || fail "control: the four posters are not pairwise distinct ($distinct unique)"

# 2. Isolation, twice. Frame 2 is the frame the settle does its work in (the
#    lane's requests reach the service after frame 1), and it is before any
#    poster can be on screen, so the *whole* frame must equal the posters-off
#    frame: a draw that escaped the offscreen buffers would land here. It is
#    photographed by a 3-frame run, because `--probe-shot` reads the buffer
#    after the swap and so shows the frame before the last (measured: a
#    deliberate leak in frame 2 is invisible to a 2-frame shot). Frame 40
#    is long after, and its preview crop must still be equal: state the
#    service touched would persist into it. (The whole frame at 40 differs
#    once the lane draws its posters, which is the lane's check, not this.)
outside_lane_equal() { # a.png b.png log -> exit status: equal outside the plan lane
    python3 - "$1" "$2" "$3" <<'PY'
import re, sys
from PIL import Image, ImageChops
match = re.search(r"^scene runs:.* lane=(\d+),(\d+),(\d+),(\d+)", open(sys.argv[3]).read(), re.M)
x, y, w, h = map(int, match.groups())
images = [Image.open(path).convert("RGB") for path in sys.argv[1:3]]
for image in images:
    image.paste((0, 0, 0), (x, y, x + w, y + h))
sys.exit(0 if ImageChops.difference(*images).getbbox() is None else 1)
PY
}
crop_equal() { # a.png b.png log -> exit status: preview crops equal
    python3 - "$1" "$2" "$3" <<'PY'
import re, sys
from PIL import Image, ImageChops
match = re.search(r"pixels (\d+),(\d+),(\d+),(\d+)", open(sys.argv[3]).read())
x, y, w, h = map(int, match.groups())
crops = [Image.open(path).convert("RGB").crop((x, y, x + w, y + h)) for path in sys.argv[1:3]]
sys.exit(0 if ImageChops.difference(*crops).getbbox() is None else 1)
PY
}
for moment in 3.0 7.5; do
    for frames in 3 40; do
        run "off-$moment-$frames" "time=$moment,posters=off" "$frames"
        run "on-$moment-$frames" "time=$moment,posters=settle" "$frames"
        on="$(line_of 'posters: ' "$OUT/on-$moment-$frames.log")"
        [ "$(field rendered "$on")" = 4 ] ||
            fail "t=$moment frame $frames: the settled run rendered no posters, so it proves nothing: $on"
    done
    # Everything but the plan lane: since the lane draws posters (WX-5b
    #    merge), a poster finished in frame 2 is already on screen in it, and
    #    that is the lane's job rather than a leak. A draw escaping the
    #    offscreen buffers lands anywhere else in the frame and still fails.
    outside_lane_equal "$OUT/off-$moment-3.png" "$OUT/on-$moment-3.png" "$OUT/on-$moment-3.log" ||
        fail "t=$moment: outside the plan lane, the frame the posters were drawn in differs from posters off"
    crop_equal "$OUT/off-$moment-40.png" "$OUT/on-$moment-40.png" "$OUT/on-$moment-40.log" ||
        fail "t=$moment: the preview changed after the poster service ran (its state is not isolated)"
done
crop_equal "$OUT/off-3.0-40.png" "$OUT/off-7.5-40.png" "$OUT/off-3.0-40.log" &&
    fail "control: two different moments have equal preview crops, so the comparison cannot fail"

# 3. Invalidation, through a followed treatment. The writer waits for the
#    settled posters, then changes the stage section's layout (or, for the
#    control, rewrites the file byte for byte).
follow() { # name new-stage-settings
    treatment '{"layout":0}' >"$PLAN"
    ( sleep 3; treatment "$2" >"$PLAN.tmp" && mv "$PLAN.tmp" "$PLAN" ) &
    local writer=$!
    tools/headless_run.sh --treatment "$PLAN" --probe-frames 420 \
        --ui-probe "time=3.0,follow=1,posters=settle,poster-dump=$OUT/$1" >"$OUT/$1.log" 2>&1 || true
    wait "$writer" || true
}
follow edited '{"layout":2}'
follow same '{"layout":0}'
for name in edited same; do
    printf '%s: %s\n' "$name" "$(line_of 'treatment follow: ' "$OUT/$name.log" | cut -c1-120)"
    case "$(line_of 'treatment follow: ' "$OUT/$name.log")" in
        *"applied=1"*) ;;
        *) fail "$name: the rewritten treatment was not applied once" ;;
    esac
done
edited="$(line_of 'posters: ' "$OUT/edited.log")"
same="$(line_of 'posters: ' "$OUT/same.log")"
printf 'edited: %s\n' "${edited:0:240}"
[ "$(field refreshed "$edited")" = 1 ] && [ "$(field rendered "$edited")" = 5 ] ||
    fail "the edit did not re-render exactly one poster: $edited"
[ "$(field ready "$edited")" = 4 ] || fail "after the edit not every run is ready: $edited"
stage_files="$(find "$OUT/edited" -name 'stage-*.png' | wc -l)"
other_files="$(find "$OUT/edited" -name '*.png' ! -name 'stage-*.png' | wc -l)"
[ "$stage_files" = 2 ] && [ "$other_files" = 3 ] ||
    fail "the edit re-rendered the wrong runs: $stage_files stage dumps, $other_files others"
[ "$(field refreshed "$same")" = 0 ] && [ "$(field rendered "$same")" = 4 ] ||
    fail "control: an unchanged rewrite re-rendered posters: $same"

# 4. Budget: no settle, paused, long enough for every poster to arrive.
run budget-off "time=3.0,posters=off" 150
run budget-on "time=3.0" 150
on="$(line_of 'posters: ' "$OUT/budget-on.log")"
printf 'budget: %s\n' "${on:0:240}"
[ "$(field ready "$on")" = 4 ] || fail "unsettled, the posters did not all arrive in 150 frames: $on"
step_worst="$(field step-worst "$on" | tr -d 'ms')"
frame_worst="$(field frame-worst "$on" | tr -d 'ms')"
budget_paused="$(field budget "$on" | sed 's/.*\///; s/ms//')"
awk -v s="$step_worst" -v f="$frame_worst" -v b="$budget_paused" 'BEGIN { exit !(s <= b + f + 0.5) }' ||
    fail "step-worst ${step_worst}ms exceeds the paused budget ${budget_paused}ms plus one replay frame (${frame_worst}ms)"
stalls() { line_of 'frame budget: ' "$1" | sed -n 's/.* \([0-9]*\) of [0-9]* stalled.*/\1/p'; }
stalled_on="$(stalls "$OUT/budget-on.log")"
stalled_off="$(stalls "$OUT/budget-off.log")"
printf 'frame budget: on %s, off %s\n' "$(line_of 'frame budget: ' "$OUT/budget-on.log" | cut -c18-)" \
    "$(line_of 'frame budget: ' "$OUT/budget-off.log" | cut -c18-)"
[ -n "$stalled_on" ] && [ "$stalled_on" -le "${stalled_off:-0}" ] ||
    fail "posters stalled more frames ($stalled_on) than no posters ($stalled_off)"

# 6. The lane draws them, upright. Each run's poster slot in the settled
#    40-frame capture is matched against that run's dumped poster (the same
#    recipe, dumped by dump-a): scaled to the slot, searched over a few pixels
#    of placement, the upright poster must correlate better than the same
#    poster flipped, and well. The service stores posters top row first while a
#    render target's buffer is bottom row first; the lane was first written for
#    the second and drew every poster upside down (merge, WX-5b). Spectrum's
#    poster at 15 s is near-flat and proves nothing either way, so the three
#    with structure are measured. Control: the posters-off frame, with the
#    lane's plain run fills in the same slots, must not match.
slots_match() { # shot.png log dumpdir -> prints per run, exit status: all upright
    python3 - "$1" "$2" "$3" <<'PY'
import glob, re, sys
import numpy as np
from PIL import Image
shot_path, log_path, dump = sys.argv[1:4]
log = open(log_path).read()
x0, y0, w0, h0 = map(int, re.search(r"^scene runs:.* lane=(\d+),(\d+),(\d+),(\d+)", log, re.M).groups())
start, end = map(float, re.search(r"^scene runs:.* view=([\d.]+)\.\.([\d.]+)", log, re.M).groups())
shot = np.array(Image.open(shot_path).convert("L")).astype(float)
ok = True
for index, (name, at) in enumerate([("clawd", 0.0), ("threshold", 5.0), ("stage", 10.0)]):
    files = glob.glob(f"{dump}/{name}-*.png")
    if not files:
        print(f"  {name}: no dumped poster"); ok = False; continue
    poster = Image.open(files[0]).convert("L")
    left = x0 + w0 * (at - start) / (end - start)
    best = {"up": -2.0, "flipped": -2.0}
    h = h0 - 2
    for width in range(h * 16 // 9 - 6, h * 16 // 9 + 3):
        scaled = np.array(poster.resize((width, h), Image.BILINEAR)).astype(float)
        cols = int(width * 0.55)  # left of the fade into the run's fill
        for dx in range(-3, 4):
            for dy in range(-2, 3):
                x, y = int(round(left)) + 1 + dx, y0 + 1 + dy
                seen = shot[y + 4:y + h - 4, x + 4:x + cols].ravel()
                if seen.std() < 1.0:
                    continue
                for tag, image in (("up", scaled), ("flipped", scaled[::-1])):
                    r = np.corrcoef(image[4:h - 4, 4:cols].ravel(), seen)[0, 1]
                    best[tag] = max(best[tag], r)
    upright = best["up"] >= 0.6 and best["up"] > best["flipped"]
    ok &= upright
    print(f"  {name}: upright r={best['up']:.2f}, flipped r={best['flipped']:.2f}")
sys.exit(0 if ok else 1)
PY
}
echo "posters in the lane, settled:"
slots_match "$OUT/on-3.0-40.png" "$OUT/on-3.0-40.log" "$OUT/dump-a" ||
    fail "the lane does not draw each run's poster upright in its slot"
echo "control, posters off:"
slots_match "$OUT/off-3.0-40.png" "$OUT/on-3.0-40.log" "$OUT/dump-a" &&
    fail "control: the posters-off lane matched the posters, so the comparison cannot fail"

# 5. GPU state, on every run that drew posters.
for log in "$OUT"/dump-a.log "$OUT"/on-3.0-40.log "$OUT"/on-7.5-40.log "$OUT"/edited.log "$OUT"/budget-on.log; do
    case "$(line_of 'gl framebuffer: ' "$log")" in
        *"mismatched-frames=0 "*) ;;
        *) fail "$(basename "$log"): $(line_of 'gl framebuffer: ' "$log")" ;;
    esac
done

if [ "$failures" -ne 0 ]; then
    echo "poster check: $failures failure(s); evidence in $OUT"
    exit 1
fi
echo "poster check: all checks passed; evidence in $OUT"
