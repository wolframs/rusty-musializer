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
#                   posters-off frame, and 38 frames later the preview crop
#                   still is; control: the crops at the two moments differ.
#   3. Invalidation a followed-treatment edit to one section re-renders exactly
#                   that run (refreshed=1, one new dump, the others untouched);
#                   control: rewriting the file unchanged re-renders nothing.
#   4. Budget       unsettled and paused, every poster still arrives,
#                   step-worst <= the paused budget + the worst replay frame,
#                   and no more frames stall than with posters off.
#   5. GPU state    `gl framebuffer: mismatched-frames=0` with posters on.
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
    cmp -s "$OUT/off-$moment-3.png" "$OUT/on-$moment-3.png" ||
        fail "t=$moment: the frame the posters were drawn in differs from posters off"
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
