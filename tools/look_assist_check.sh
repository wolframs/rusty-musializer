#!/usr/bin/env bash
#
# The Look & Assist column gate (WX-5d, build/redesign/wx5d-design.md §3.3,
# §3.4 and §5): Look and Assist drawn in the right column, pressed and
# photographed headless, with MUSIALIZER_COLUMN_LAYOUT=1.
#
#   tools/look_assist_check.sh [OUTPUT_DIR]      # default build/look-assist-check
#
# LOOK_ASSIST_ONLY=look|assist|dialog|captures runs one group (the negative
# controls use it); LOOK_ASSIST_BIN names a different binary, so a control
# never replaces the build. Every launch goes through tools/headless_run.sh:
# private Xvfb, unreachable Wayland and audio, no file dialogs, a private
# config directory, --mute. Fixtures are synthetic (make-fixture-wav and a
# one-section treatment) or the redesign's copied project under
# build/redesign/fixture, itself copied again into the output directory so no
# run can write into it.
#
# Every point pressed comes from a report line (`tune layout:`, `assist
# column:`, the AI dialog's `focus-rect=`); nothing presses a pixel measured
# once. Runs are at --ui-scale 100 unless a capture says 150.
#
# Checks, each with its control:
#   look 1. `tune layout:` lists each descriptor of the drawn scene exactly
#           once, in groups whose counts add up;
#   look 2. a press at a quarter of Swarm spread's track writes exactly the
#           quarter value (0.5); a press between two groups changes nothing;
#   look 3. the changed dot: a moved row's left 8 px carry ink, a default
#           row's carry none (classifier: pixels far from the row's median,
#           invariant under ±1 rounding);
#   look 4. Nudge opens a trial that moved values; Revert puts `tune values:`
#           back bit for bit; Keep closes the trial and keeps the values;
#   look 5. `~` opens the route editor under its row; Apply commits the route
#           (the row turns routed); a press on the plot commits nothing;
#   assist 1. a card press opens it on Confirmation; a press between two
#           cards opens nothing;
#   assist 2. Cancel closes it again; Discard drops a staged candidate;
#           Cancel job is claimed by its own id in the running state;
#   assist 3. AI settings in the head opens the dialog;
#   dialog 1. the AI dialog's corner is rounded (not the surface colour) and
#           its header row is the dialog's own surface exactly (opaque fills,
#           compared exactly); the old square, raised skin fails both;
#   captures. Look (plain, and with a Surprise trial and the route editor
#           open), Assist in every state, and the dialog, at 960x640,
#           1280x720, 1920x1080 and 150 %, dark and light. Each capture's
#           view line must be printed and every rect it names must lie inside
#           the column the `column:` line reports.
#
# NEGATIVE CONTROLS, 2026-10-29, each run on the GPU and on llvmpipe
# (MZ_GL_LAUNCH=""), perturbed, watched fail, reverted byte for byte:
#   - the row's drag write removed (look_row): look 2 fails, 0.5 wanted,
#     1 read; the gap press stays green;
#   - the changed dot never drawn (widgets::slider_row's `changed` passed
#     false from look_row): look 3 reads 0 ink pixels on the moved row;
#   - Revert pushing no snapshot: look 4's bit-for-bit comparison fails;
#   - the card press not calling select_mode: assist 1 reads body=Ready;
#   - the dialog drawn with the pre-WX-5d frame and header: dialog 1 fails
#     both the corner and the header comparison.
# The GPU and CPU runs read the same verdicts and the same pixel counts on
# the opaque fills; see build/redesign/wx5d-look-assist-gate.md.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/look-assist-check}"
ONLY="${LOOK_ASSIST_ONLY:-all}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${LOOK_ASSIST_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"
export MUSIALIZER_COLUMN_LAYOUT=1

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 8 >/dev/null
cat >"$OUT/look.treatment.json" <<'EOF'
{ "schema": "musializer.treatment/v1", "title": "Look Fixture",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "cadence" } ] }
EOF
PROJECT_SRC="build/redesign/fixture"
if [ ! -d "$PROJECT_SRC" ]; then
    echo "error: $PROJECT_SRC is missing; copy the redesign fixture there first" >&2
    exit 2
fi
cp -a "$PROJECT_SRC" "$OUT/project"
PROJECT="$(ls "$OUT"/project/*.musi | head -n 1)"

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
pass() {
    printf 'ok:   %s\n' "$*"
}
wants() { # wants GROUP
    [ "$ONLY" = all ] || [ "$ONLY" = "$1" ]
}
line_of() { # line_of KEY LOG
    grep -E "^$1: " "$2" | tail -n 1 | sed -E "s/^$1: +//" || true
}
# run NAME SIZE PROBE [args...]: the Look fixture, the column on Look.
look_run() {
    local name="$1" size="$2" probe="$3"
    shift 3
    tools/headless_run.sh --treatment "$OUT/look.treatment.json" --ui-scale 100 \
        --ui-probe "size=$size,column=look,play=0${probe:+,$probe}" --probe-frames 24 \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}
# assist_run NAME SIZE PROBE [args...]: the copied project, the column on Assist.
assist_run() {
    local name="$1" size="$2" probe="$3"
    shift 3
    tools/headless_run.sh "$PROJECT" --ui-scale 100 \
        --ui-probe "size=$size,column=assist,time=95,play=0$probe" --probe-frames 24 \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}

# A small reader for the report lines: `field LINE EXPR` evaluates a Python
# expression over the parsed line (rects as (x, y, w, h) tuples).
field() {
    python3 - "$1" "$2" <<'PY'
import re, sys
line, expr = sys.argv[1], sys.argv[2]
def rect(name, text=line):
    m = re.search(r"(?:^|[ ;,=(])" + re.escape(name) + r"[*!~]*@(-?\d+),(-?\d+),(-?\d+),(-?\d+)", text)
    if not m:
        sys.exit(f"no rect for {name}")
    return tuple(int(v) for v in m.groups())
def centre(r):
    return f"{r[0] + r[2] // 2}x{r[1] + r[3] // 2}"
def rows():
    body = re.search(r" rows=(.*?) explore=", line).group(1)
    out = []
    for entry in body.split(");"):
        m = re.match(r"(settings\.[\w.-]+?)([*~]*)@(-?\d+),(-?\d+),(-?\d+),(-?\d+)\(track@(-?\d+),(-?\d+),(-?\d+),(-?\d+)", entry)
        if m:
            g = m.groups()
            out.append({"key": g[0], "flags": g[1], "row": tuple(map(int, g[2:6])), "track": tuple(map(int, g[6:10]))})
    return out
def row(key):
    return next(r for r in rows() if r["key"] == key)
print(eval(expr))
PY
}

# ---------------------------------------------------------------------------
if wants look; then
    echo "=== Look ==="
    look_run look-base 1280x720 ""
    BASE="$(line_of 'tune layout' "$OUT/look-base.log")"
    VALUES="$(line_of 'tune values' "$OUT/look-base.log")"
    if [ -z "$BASE" ]; then
        fail "look: no tune layout: line"
    else
        # 1. Every descriptor once.
        verdict="$(field "$BASE" "(lambda rs, g: 'ok' if len(rs) == len(set(r['key'] for r in rs)) == 12 == sum(int(x.split(':')[1]) for x in g) else f'{len(rs)} rows {len(set(r[\"key\"] for r in rs))} keys {g}')(rows(), re.search(r'groups=(\S+)', line).group(1).split(','))")"
        [ "$verdict" = ok ] && pass "look 1: twelve rows, twelve keys, groups add up" \
            || fail "look 1: $verdict"

        # 2. A quarter press writes the quarter value; a gap press nothing.
        point="$(field "$BASE" "(lambda t: f'{t[0] + t[2] // 4}x{t[1] + t[3] // 2}')(row('settings.cadence.swarm')['track'])")"
        want="$(field "$BASE" "(lambda t: round(0.0 + (t[2] // 4) / t[2] * 2.0, 2))(row('settings.cadence.swarm')['track'])")"
        look_run look-quarter 1280x720 "click=$point"
        got="$(line_of 'tune values' "$OUT/look-quarter.log" | awk '{print $3}')"
        python3 -c "import sys; sys.exit(0 if abs(float('$got') - float('$want')) < 1e-6 else 1)" \
            && pass "look 2: a press at $point wrote $got (wanted $want)" \
            || fail "look 2: a press at $point wrote [$got], wanted $want"
        gap="$(field "$BASE" "(lambda a, b: f'{b[\"track\"][0] + 10}x{(a[\"row\"][1] + a[\"row\"][3] + b[\"row\"][1]) // 2}')(row('settings.cadence.reveal'), row('settings.cadence.swarm'))")"
        look_run look-gap 1280x720 "click=$gap"
        [ "$(line_of 'tune values' "$OUT/look-gap.log")" = "$VALUES" ] \
            && pass "look 2 control: a press between groups at $gap changed nothing" \
            || fail "look 2 control: the gap press at $gap changed [$(line_of 'tune values' "$OUT/look-gap.log")]"

        # 3. The changed dot.
        look_run look-dot 1280x720 "tune-type=settings.cadence.scale:1.3"
        DOT="$(line_of 'tune layout' "$OUT/look-dot.log")"
        counts="$(python3 - "$OUT/look-dot.png" "$(field "$DOT" "row('settings.cadence.scale')['row']")" "$(field "$DOT" "row('settings.cadence.spacing')['row']")" <<'PY'
import sys
from PIL import Image
image = Image.open(sys.argv[1]).convert("RGB")
def ink(rect):
    x, y, w, h = eval(rect)
    pixels = [image.getpixel((px, py)) for px in range(x, x + 8) for py in range(y, y + h)]
    lum = sorted(sum(p) for p in pixels)
    median = lum[len(lum) // 2]
    # A pixel counts when it is far from the row's own ground: a hairline
    # or a 1-level rounding difference is not ink.
    return sum(1 for p in pixels if abs(sum(p) - median) > 120)
print(ink(sys.argv[2]), ink(sys.argv[3]))
PY
)"
        moved="${counts% *}"
        still="${counts#* }"
        if [ "$moved" -ge 8 ] && [ "$still" -eq 0 ]; then
            pass "look 3: the moved row's dot carries $moved ink pixels, the default row's $still"
        else
            fail "look 3: moved row $moved ink pixels (want >= 8), default row $still (want 0)"
        fi

        # 4. Nudge, Revert, Keep.
        nudge="$(field "$BASE" "centre(rect('nudge'))")"
        look_run look-nudge 1280x720 "tune-seed=7,click=$nudge"
        entry="$(line_of 'tune entry' "$OUT/look-nudge.log")"
        case "$entry" in
            *"audition Auditioning: Nudge"*) [ "$(line_of 'tune values' "$OUT/look-nudge.log")" != "$VALUES" ] \
                && pass "look 4: Nudge at $nudge opened a trial that moved values" \
                || fail "look 4: Nudge opened a trial that moved nothing" ;;
            *) fail "look 4: Nudge at $nudge left [$entry]" ;;
        esac
        look_run look-trial 1280x720 "tune-seed=3,tune-explore=surprise"
        TRIAL="$(line_of 'tune layout' "$OUT/look-trial.log")"
        TRIED="$(line_of 'tune values' "$OUT/look-trial.log")"
        revert="$(field "$TRIAL" "centre(rect('revert'))")"
        keep="$(field "$TRIAL" "centre(rect('keep'))")"
        look_run look-revert 1280x720 "tune-seed=3,tune-explore=surprise,click=$revert"
        [ "$(line_of 'tune values' "$OUT/look-revert.log")" = "$VALUES" ] && [ "$TRIED" != "$VALUES" ] \
            && pass "look 4: Revert at $revert restored the values bit for bit" \
            || fail "look 4: Revert left [$(line_of 'tune values' "$OUT/look-revert.log")], wanted [$VALUES]"
        look_run look-keep 1280x720 "tune-seed=3,tune-explore=surprise,click=$keep"
        case "$(line_of 'tune entry' "$OUT/look-keep.log")" in
            *"audition none"*) [ "$(line_of 'tune values' "$OUT/look-keep.log")" = "$TRIED" ] \
                && pass "look 4: Keep at $keep closed the trial and kept the values" \
                || fail "look 4: Keep changed the values" ;;
            *) fail "look 4: Keep at $keep left the trial open" ;;
        esac

        # 5. The route editor.
        route="$(field "$BASE" "(lambda r: f'{r[0] + r[2] // 2}x{r[1] + r[3] // 2}')(rect('route', text=next(e for e in line.split(');') if e.startswith('settings.cadence.beat'))))")"
        look_run look-route 1280x720 "click=$route"
        case "$(line_of 'tune layout' "$OUT/look-route.log")" in
            *"editor=settings.cadence.beat@"*) pass "look 5: ~ at $route opened the editor under Beat breathing" ;;
            *) fail "look 5: ~ at $route opened no editor" ;;
        esac
        look_run look-editor 1280x720 "route=settings.cadence.beat"
        EDITOR="$(line_of 'tune layout' "$OUT/look-editor.log")"
        apply="$(field "$EDITOR" "centre(rect('apply'))")"
        plot="$(field "$EDITOR" "centre(rect('plot'))")"
        look_run look-apply 1280x720 "route=settings.cadence.beat,click=$apply"
        look_run look-plot 1280x720 "route=settings.cadence.beat,click=$plot"
        beat_flags() { field "$(line_of 'tune layout' "$1")" "row('settings.cadence.beat')['flags'] or '-'"; }
        [ "$(beat_flags "$OUT/look-apply.log")" = "~" ] \
            && pass "look 5: Apply at $apply committed the route" \
            || fail "look 5: Apply at $apply left Beat breathing [$(beat_flags "$OUT/look-apply.log")]"
        [ "$(beat_flags "$OUT/look-plot.log")" = "-" ] \
            && pass "look 5 control: a press on the plot committed nothing" \
            || fail "look 5 control: a press on the plot committed a route"
    fi
fi

# ---------------------------------------------------------------------------
if wants assist; then
    echo "=== Assist ==="
    assist_run assist-base 1280x720 ""
    BASE="$(line_of 'assist column' "$OUT/assist-base.log")"
    if [ -z "$BASE" ]; then
        fail "assist: no assist column: line"
    else
        case "$BASE" in
            *"open=none body=Ready"*) pass "assist: idle, every card closed" ;;
            *) fail "assist: the idle column is not Ready: ${BASE:0:120}" ;;
        esac
        scenes="$(field "$BASE" "centre(rect('scenes'))")"
        assist_run assist-card 1280x720 ",click=$scenes"
        case "$(line_of 'assist column' "$OUT/assist-card.log")" in
            *"open=scenes body=Confirmation"*) pass "assist 1: the Scene changes card at $scenes opened on Confirmation" ;;
            *) fail "assist 1: the card press at $scenes left [$(line_of 'assist column' "$OUT/assist-card.log" | cut -c1-160)]" ;;
        esac
        gap="$(field "$BASE" "(lambda a, b: f'{a[0] + a[2] // 2}x{(a[1] + a[3] + b[1]) // 2}')(rect('lyrics'), rect('scenes'))")"
        assist_run assist-gap 1280x720 ",click=$gap"
        case "$(line_of 'assist column' "$OUT/assist-gap.log")" in
            *"open=none body=Ready"*) pass "assist 1 control: a press between cards at $gap opened nothing" ;;
            *) fail "assist 1 control: the gap press at $gap opened a card" ;;
        esac

        export MUSIALIZER_ASSIST_PROBE_LANES=lyrics
        assist_run assist-confirm 1280x720 ",assist=confirm"
        CONFIRM="$(line_of 'assist column' "$OUT/assist-confirm.log")"
        cancel="$(field "$CONFIRM" "centre(rect('cancel'))")"
        assist_run assist-cancel 1280x720 ",assist=confirm,click=$cancel"
        case "$(line_of 'assist column' "$OUT/assist-cancel.log")" in
            *"open=none body=Ready"*) pass "assist 2: Cancel at $cancel closed the card" ;;
            *) fail "assist 2: Cancel at $cancel left [$(line_of 'assist column' "$OUT/assist-cancel.log" | cut -c1-160)]" ;;
        esac
        assist_run assist-candidate 1280x720 ",assist=candidate"
        CANDIDATE="$(line_of 'assist column' "$OUT/assist-candidate.log")"
        discard="$(field "$CANDIDATE" "centre(rect('discard'))")"
        assist_run assist-discard 1280x720 ",assist=candidate,click=$discard"
        case "$(line_of 'assist column' "$OUT/assist-discard.log")" in
            *"body=Ready"*) pass "assist 2: Discard at $discard dropped the staged result" ;;
            *) fail "assist 2: Discard at $discard left [$(line_of 'assist column' "$OUT/assist-discard.log" | cut -c1-160)]" ;;
        esac
        unset MUSIALIZER_ASSIST_PROBE_LANES
        assist_run assist-running 1280x720 ",assist=running"
        RUNNING="$(line_of 'assist column' "$OUT/assist-running.log")"
        stop="$(field "$RUNNING" "centre(rect('cancel-job'))")"
        assist_run assist-stop 1280x720 ",assist=running,click=$stop"
        # Cancel job is index 12 in the Assist namespace: claimed id +1.
        case "$(line_of 'click probe' "$OUT/assist-stop.log")" in
            *"claimed=0x"*"0000000d") pass "assist 2: Cancel job at $stop was claimed by its own id" ;;
            *) fail "assist 2: Cancel job at $stop: [$(line_of 'click probe' "$OUT/assist-stop.log")]" ;;
        esac

        settings="$(field "$BASE" "centre(rect('settings'))")"
        assist_run assist-settings 1280x720 ",click=$settings"
        case "$(line_of 'assist settings' "$OUT/assist-settings.log")" in
            "section=routing "*) pass "assist 3: AI settings at $settings opened the dialog" ;;
            *) fail "assist 3: AI settings at $settings left the dialog closed" ;;
        esac
    fi
fi

# ---------------------------------------------------------------------------
if wants dialog; then
    echo "=== AI settings dialog ==="
    for theme in black-amber light-blue; do
        MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
            assist_run "dialog-$theme" 1280x720 "" --ui-theme "$theme"
        verdict="$(python3 - "$OUT/dialog-$theme.png" "$(line_of 'assist settings' "$OUT/dialog-$theme.log")" <<'PY'
import re, sys
from PIL import Image
image = Image.open(sys.argv[1]).convert("RGB")
m = re.search(r"focus-rect=(\d+),(\d+),(\d+),(\d+)", sys.argv[2])
if not m:
    print("no focus-rect on the assist settings line"); sys.exit()
cx, cy, cw, ch = map(int, m.groups())
# The close button is 14 px in from the dialog's right edge and 7 px below its
# top (DialogLayout::of); the dialog is centred in the window.
right, top = cx + cw + 14, cy - 7
left = image.width - right
surface = image.getpixel((left + 5, top + 300))   # the rail's left margin
header = [image.getpixel((x, top + 5)) for x in range(left + 8, left + 120)]
# A square frame's corner pixel is its hairline, the same colour as the left
# edge further down; a 4 px rounded corner leaves the scrim there instead.
corner = image.getpixel((left, top))
edge = image.getpixel((left, top + 200))
problems = []
if any(p != surface for p in header):
    problems.append(f"header {sorted(set(header))[:3]} is not the surface {surface}")
if corner == edge:
    problems.append(f"corner {corner} is the edge's colour: square")
print("ok" if not problems else "; ".join(problems))
PY
)"
        [ "$verdict" = ok ] && pass "dialog 1 ($theme): rounded corner, header on the surface" \
            || fail "dialog 1 ($theme): $verdict"
    done
fi

# ---------------------------------------------------------------------------
if wants captures; then
    echo "=== captures ==="
    inside() { # inside LOG VIEWKEY: every rect on the view line inside the column
        python3 - "$(line_of column "$1")" "$(line_of "$2" "$1")" <<'PY'
import re, sys
column, view = sys.argv[1], sys.argv[2]
if not view:
    print("no view line"); sys.exit()
m = re.search(r"rect=(-?\d+),(-?\d+),(-?\d+),(-?\d+)", column)
if not m:
    print("no column rect"); sys.exit()
x, y, w, h = map(int, m.groups())
bad = []
for r in re.finditer(r"(\w[\w.-]*)[*!~]*@(-?\d+),(-?\d+),(-?\d+),(-?\d+)", view):
    rx, ry, rw, rh = map(int, r.groups()[1:])
    # Scrolled rects may sit above or below the viewport; x must not.
    if rx < x or rx + rw > x + w:
        bad.append(f"{r.group(1)}@{rx},{ry},{rw},{rh}")
print("ok" if not bad else f"outside x {x}..{x + w}: " + " ".join(bad[:4]))
PY
    }
    for size in 960x640 1280x720 1920x1080 1920x1080@150; do
        dims="${size%@*}"
        scale=100
        [ "$size" != "$dims" ] && scale="${size#*@}"
        for theme in black-amber light-blue; do
            tag="${dims}-${scale}-${theme}"
            shoot_look() { # shoot_look NAME PROBE
                tools/headless_run.sh --treatment "$OUT/look.treatment.json" --ui-scale "$scale" \
                    --ui-theme "$theme" \
                    --ui-probe "size=$dims,column=look,play=0$2" --probe-frames 24 \
                    --probe-shot "$OUT/cap-$1-$tag.png" >"$OUT/cap-$1-$tag.log" 2>&1 || true
                verdict="$(inside "$OUT/cap-$1-$tag.log" 'tune layout')"
                [ "$verdict" = ok ] || fail "capture $1 $tag: $verdict"
            }
            shoot_assist() { # shoot_assist NAME PROBE [env...]
                env "${@:3}" tools/headless_run.sh "$PROJECT" --ui-scale "$scale" --ui-theme "$theme" \
                    --ui-probe "size=$dims,column=assist,time=95,play=0$2" --probe-frames 24 \
                    --probe-shot "$OUT/cap-$1-$tag.png" >"$OUT/cap-$1-$tag.log" 2>&1 || true
                verdict="$(inside "$OUT/cap-$1-$tag.log" 'assist column')"
                [ "$verdict" = ok ] || fail "capture $1 $tag: $verdict"
            }
            shoot_look look ""
            shoot_look look-trial ",tune-seed=3,tune-explore=surprise,route=settings.cadence.beat"
            shoot_assist assist-ready ""
            shoot_assist assist-confirm ",assist=confirm" MUSIALIZER_ASSIST_PROBE_LANES=lyrics
            shoot_assist assist-running ",assist=running"
            shoot_assist assist-candidate ",assist=candidate" MUSIALIZER_ASSIST_PROBE_LANES=lyrics
            shoot_assist assist-failed ",assist=failed"
            shoot_assist settings "" MUSIALIZER_ASSIST_SETTINGS_OPEN=routing
        done
    done
    count="$(ls "$OUT"/cap-*.png 2>/dev/null | wc -l)"
    [ "$count" -eq 64 ] && pass "captures: 64 pictures, every view line inside its column" \
        || fail "captures: $count pictures, wanted 64"
fi

echo
if [ "$failures" -eq 0 ]; then
    echo "PASS: look & assist column checks ($ONLY) — evidence in $OUT"
else
    echo "FAIL: $failures look & assist check(s) failed — evidence in $OUT" >&2
    exit 1
fi
