#!/usr/bin/env bash
#
# The desktop-scale gate (DS, 2026-11-19): window sizes follow the scale the
# shell wants, so the first frame is drawn at the desktop's scale.
#
#   tools/desktop_scale_check.sh [OUT]      default OUT: build/desktop-scale-check
#
# The defect it pins: every window size was raw pixels while the shell is in
# scaled units. On a 125 % desktop the default 1280x720 window cannot hold the
# 960x640 logical minimum at 125 %, so Auto was clamped to 100 % until the
# window was enlarged by hand, and a scale chosen with Ctrl+plus in a small
# window changed nothing.
#
# A desktop's scale reaches the application as the X resource `Xft.dpi` (GLFW
# reads it once, at start). No other gate has one: Xvfb reports 96 dpi, which
# is why nothing here ever saw this. So this gate owns an Xvfb and sets the
# resource on it, and refuses a server it did not start: the resource is the
# whole display's, and setting it under another gate would move its windows.
# `-noreset`, because without it the server regenerates when `xrdb` (its only
# client) disconnects and the resource is gone before the application starts.
#
# Every assertion reads the `window:`, `ui layout:` and `fonts:` report lines.
# The window is addressed by what the report says of it, never by a size
# measured once.
#
# Checks:
#   1. scaled desktop, nothing named: the window opens at 1600x900
#      (`startup=scaled:`), the floor is 1200x800, the shell draws at 125 %
#      with a native 1.25 atlas (no fallback face, no non-native request), the
#      welcome screen is drawn and photographed at that size.
#   2. control, the same launch on a desktop with no scale: 1280x720,
#      `startup=default`, floor 960x640, 100 %. What moved check 1 is the
#      desktop's scale and nothing else.
#   3. `--size 1400x1000` on the scaled desktop opens as named
#      (`startup=explicit`), not as 1750x1250: a named size is raw pixels.
#   4. a saved 150 % on an unscaled desktop opens 1920x1080 and draws at 150 %.
#   5. a saved 150 % with `--size 1280x720`: the window is under the floor
#      (1440x960) and grows to it; the shell draws at 150 %.
#   6. `--ui-probe key=scale-up` (Ctrl+plus) in a 960x640 window: the window
#      grows to 1200x800, the shell draws at 125 %, and the saved preference
#      says 125.
#   7. the capture seams stay raw, on a scaled desktop and over a saved 150 %:
#      `--ui-scale 100 --ui-probe size=960x640` is 960x640 at 100 % from a
#      default first window; `--ui-scale 100` alone opens 1280x720;
#      `--ui-probe size=960x640` alone is 960x640 with the application's own
#      floor; `--size 800x600` stays 800x600 (at the 960x640 floor the limit
#      is a hint, as it always was).
#   8. a saved 200 % with `--size 1280x720` on the 1440p screen: the floor is
#      capped a rung down (1680x1120) but the window grows to the size that
#      holds 200 % (1920x1280), because nine tenths of the monitor has room.
#   9. real keys (XTest, tools/desktop_scale_input.py) on a German layout:
#      Ctrl+plus scales up (`plus=KEY_RIGHT_BRACKET minus=KEY_SLASH`);
#      plus, plus, minus ends at 125 %; plus, zero ends on Auto.
#  10. real keys on a US layout: Ctrl+= scales up, then Ctrl+minus back.
#  11. real keys with the pointer resting on the welcome Theme picker (its
#      rect from `welcome: theme@`): Ctrl+plus still scales.
#  12. a window manager that refuses the first window's resize
#      (tools/desktop_scale_wm.py): after the settle the shell lays out for
#      the 1280x720 it has (`adopted=1280x720`, 100 %), and the photograph is
#      that size. One that grants 1400x800 instead: 1400x800 at 125 %.
#
# NEGATIVE CONTROLS (2026-11-19), each a perturbed debug build copied aside
# and run as DESKTOP_SCALE_BIN, the tree restored with `git checkout` and
# shown clean. What each caught is recorded in
# docs/archive/FEATURE_PARITY_HISTORY.md, "DS".

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/desktop-scale-check}"
# Emptied only if this gate made it: a path typed wrong must not cost a
# directory that holds something else.
MARKER=.desktop-scale-check
if [ -e "$OUT" ] && [ ! -e "$OUT/$MARKER" ] && [ -n "$(ls -A "$OUT" 2>/dev/null)" ]; then
    echo "desktop_scale_check: $OUT exists, is not empty and was not made by this gate; refusing to empty it" >&2
    exit 2
fi
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
: >"$OUT/$MARKER"

export MZ_BIN="${DESKTOP_SCALE_BIN:-$(realpath target/debug/musializer)}"

REAL_BIN="$MZ_BIN"

for tool in Xvfb xrdb xdpyinfo setxkbmap; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "desktop_scale_check: $tool is not installed" >&2
        exit 2
    fi
done

CAPTURE_DISPLAY="${DESKTOP_SCALE_DISPLAY:-:94}"
if [[ ! "$CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'desktop_scale_check: DESKTOP_SCALE_DISPLAY must be :NUMBER' >&2
    exit 2
fi
CAPTURE_NUMBER="${BASH_REMATCH[1]}"
if [ -e "/tmp/.X11-unix/X$CAPTURE_NUMBER" ] || [ -e "/tmp/.X$CAPTURE_NUMBER-lock" ]; then
    echo "desktop_scale_check: $CAPTURE_DISPLAY exists; this gate sets a display-wide resource and only does that on a server it started (DESKTOP_SCALE_DISPLAY picks another)" >&2
    exit 2
fi
Xvfb "$CAPTURE_DISPLAY" -screen 0 2560x1440x24 -nolisten tcp -noreset >"$OUT/xvfb.log" 2>&1 &
CAPTURE_PID=$!
WM_PID=""
capture_cleanup() {
    if [ -n "$WM_PID" ]; then
        kill "$WM_PID" 2>/dev/null || true
        wait "$WM_PID" 2>/dev/null || true
    fi
    kill "$CAPTURE_PID" 2>/dev/null || true
    wait "$CAPTURE_PID" 2>/dev/null || true
}
trap capture_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
CAPTURE_READY=0
for _ in {1..100}; do
    if xdpyinfo -display "$CAPTURE_DISPLAY" >/dev/null 2>&1; then
        CAPTURE_READY=1
        break
    fi
    kill -0 "$CAPTURE_PID" 2>/dev/null || break
    sleep 0.05
done
if [ "$CAPTURE_READY" -ne 1 ]; then
    echo "desktop_scale_check: owned Xvfb did not become ready; see $OUT/xvfb.log" >&2
    exit 2
fi
# An answering display is not yet our display: two gates that both found the
# number free both start a server, one loses, and the loser must not set a
# resource on the winner's. The lock names the server that holds the display.
if ! kill -0 "$CAPTURE_PID" 2>/dev/null ||
    [ "$(tr -d '[:space:]' <"/tmp/.X$CAPTURE_NUMBER-lock" 2>/dev/null || true)" != "$CAPTURE_PID" ]; then
    echo "desktop_scale_check: $CAPTURE_DISPLAY is held by a server this gate did not start; see $OUT/xvfb.log" >&2
    exit 2
fi
# headless_run reuses this server because it is owned by an ancestor (us),
# and keeps every audio, dialog and config guard on that path.
export MUSIALIZER_CAPTURE_DISPLAY="$CAPTURE_DISPLAY"

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}

# desktop DPI: the resource a 125 % desktop publishes is 120; `none` removes it.
desktop() {
    xrdb -display "$CAPTURE_DISPLAY" -remove
    if [ "$1" != none ]; then
        printf 'Xft.dpi: %s\n' "$1" | xrdb -display "$CAPTURE_DISPLAY" -merge
    fi
}
layout() { setxkbmap -display "$CAPTURE_DISPLAY" "$1"; }

# run NAME SAVED_SCALE ARGS...   SAVED_SCALE: `-` for no preference file.
run() {
    local name="$1" saved="$2"
    shift 2
    local state="$OUT/state-$name"
    mkdir -p "$state/config/musializer"
    if [ "$saved" != - ]; then
        printf '{"schema":"musializer.ui-preferences/v1","scale":"%s","theme":"black-amber"}\n' "$saved" \
            >"$state/config/musializer/ui.json"
    fi
    if ! MUSIALIZER_UI_PREFERENCES="$state/config/musializer/ui.json" MZ_STATE_DIR="$state" \
        tools/headless_run.sh --hud=0 "$@" >"$OUT/$name.log" 2>&1; then
        fail "$name: the launch failed; see $OUT/$name.log"
    fi
}

# typed NAME KEYS POINTER ARGS...   real key events; POINTER `-` or X,Y in the window.
typed() {
    local name="$1" keys="$2" pointer="$3"
    shift 3
    [ "$pointer" = - ] && pointer=""
    MZ_BIN="$(realpath tools/desktop_scale_input.py)" DESKTOP_SCALE_REAL_BIN="$REAL_BIN" \
        DESKTOP_SCALE_KEYS="$keys" DESKTOP_SCALE_POINTER="$pointer" run "$name" - "$@"
}

line_of() { grep -a -m1 "^$1:" "$2" | sed "s/^$1: *//" || true; }
# field KEY LINE   the value of KEY= in a report line.
field() { printf '%s\n' "$2" | tr ' ' '\n' | grep -a -m1 "^$1=" | cut -d= -f2- || true; }

# expect NAME WINDOW STARTUP FLOOR DESKTOP SCALE [ADOPTED]
expect() {
    local name="$1" log="$OUT/$1.log"
    local window layout fonts got
    window="$(line_of window "$log")"
    layout="$(line_of 'ui layout' "$log")"
    fonts="$(line_of fonts "$log")"
    if [ -z "$window" ] || [ -z "$layout" ] || [ -z "$fonts" ]; then
        fail "$name: no window:, ui layout: or fonts: line; see $log"
        return
    fi
    got="${window%% *}"
    [ "$got" = "$2" ] || fail "$name: the window is $got, expected $2"
    got="$(field startup "$window")"
    [ "$got" = "$3" ] || fail "$name: startup=$got, expected $3"
    got="$(field floor "$window")"
    [ "$got" = "$4" ] || fail "$name: floor=$got, expected $4"
    got="$(field desktop-scale "$window")"
    [ "$got" = "$5" ] || fail "$name: desktop-scale=$got, expected $5"
    got="$(field monitor "$window")"
    [ "$got" = 2560x1440 ] || fail "$name: monitor=$got, expected the 2560x1440 screen this gate started"
    got="$(field adopted "$window")"
    [ "$got" = "${7:-none}" ] || fail "$name: adopted=$got, expected ${7:-none}"
    got="$(field scale "$layout")"
    [ "$got" = "$6" ] || fail "$name: the shell drew at $got %, expected $6 %"
    # The atlas the shell drew with is the native one for that scale: a
    # fallback face or a magnified 1x atlas also "draws at" the scale.
    local atlas
    atlas="$(printf '%d.%02d' "$(($6 / 100))" "$(($6 % 100))")"
    case "$fonts" in
    *FALLBACK*) fail "$name: a face fell back: $fonts" ;;
    esac
    case "$fonts" in
    *"scale=$atlas "*non-native-requests=0,*) ;;
    *) fail "$name: the font atlas is not the native $atlas one: $fonts" ;;
    esac
}

# saved NAME VALUE   the scale the run left in its preference file.
saved() {
    local got
    got="$(tr -d ' \n' <"$OUT/state-$1/config/musializer/ui.json" 2>/dev/null || true)"
    case "$got" in
    *"\"scale\":\"$2\""*) ;;
    *) fail "$1: the saved preference does not say $2: ${got:-no file}" ;;
    esac
}

scale_keys() {
    local window
    window="$(line_of window "$OUT/$1.log")"
    [ "$(field plus "$window")" = "$2" ] || fail "$1: plus=$(field plus "$window"), expected $2"
    [ "$(field minus "$window")" = "$3" ] || fail "$1: minus=$(field minus "$window"), expected $3"
}

png_size() {
    python3 -c 'import struct,sys
with open(sys.argv[1],"rb") as f: head=f.read(24)
print("%dx%d" % struct.unpack(">II", head[16:24]) if head[:8]==b"\x89PNG\r\n\x1a\n" else "not-a-png")' "$1" 2>/dev/null || echo unreadable
}

layout us

# --- 1. A scaled desktop, nothing named ----------------------------------------
desktop 120
run scaled-desktop - --probe-frames 24 --probe-shot "$OUT/scaled-desktop.png"
expect scaled-desktop 1600x900 scaled:1600x900 1200x800 1.25 125
grep -a -q '^welcome: *open-audio@' "$OUT/scaled-desktop.log" ||
    fail "scaled-desktop: the welcome screen did not report its Open audio control"
got="$(png_size "$OUT/scaled-desktop.png")"
[ "$got" = 1600x900 ] || fail "scaled-desktop: the photograph is $got, expected 1600x900"

# --- 2. Control: the same launch, a desktop with no scale ----------------------
desktop none
run plain-desktop - --probe-frames 24 --probe-shot "$OUT/plain-desktop.png"
expect plain-desktop 1280x720 default 960x640 1.00 100
got="$(png_size "$OUT/plain-desktop.png")"
[ "$got" = 1280x720 ] || fail "plain-desktop: the photograph is $got, expected 1280x720"

# --- 3. A named size is raw pixels ---------------------------------------------
desktop 120
run named-size - --size 1400x1000 --probe-frames 24
expect named-size 1400x1000 explicit 1200x800 1.25 125

# --- 4. A saved scale sizes the first window -----------------------------------
desktop none
run saved-150 150 --probe-frames 24
expect saved-150 1920x1080 scaled:1920x1080 1440x960 1.00 150

# --- 5. A window under the floor grows to it -----------------------------------
run under-floor 150 --size 1280x720 --probe-frames 24
expect under-floor 1440x960 explicit 1440x960 1.00 150

# --- 6. Ctrl+plus in a window too small for the next rung ----------------------
run ctrl-plus - --size 960x640 --ui-probe key=scale-up --probe-frames 70
expect ctrl-plus 1200x800 explicit 1200x800 1.00 125
saved ctrl-plus 125

# --- 7. The capture seams stay raw ---------------------------------------------
desktop 120
run seam - --ui-scale 100 --ui-probe size=960x640 --probe-frames 24
expect seam 960x640 default 960x640 1.25 100
desktop none
run seam-scale 150 --ui-scale 100 --probe-frames 24
expect seam-scale 1280x720 default 960x640 1.00 100
run seam-probe 150 --ui-probe size=960x640 --probe-frames 24
expect seam-probe 960x640 scaled:1920x1080 960x640 1.00 100
run seam-small - --size 800x600 --probe-frames 24
expect seam-small 800x600 explicit 960x640 1.00 100

# --- 8. A window grown for a scale holds it where the monitor has room ---------
run hold-200 200 --size 1280x720 --probe-frames 24
expect hold-200 1920x1280 explicit 1680x1120 1.00 200

# --- 9. Real keys on a German layout -------------------------------------------
layout de
typed de-plus plus - --size 960x640 --probe-frames 300
expect de-plus 1200x800 explicit 1200x800 1.00 125
scale_keys de-plus KEY_RIGHT_BRACKET KEY_SLASH
saved de-plus 125
typed de-plus-plus-minus plus,plus,minus - --size 960x640 --probe-frames 420
expect de-plus-plus-minus 1440x960 explicit 1200x800 1.00 125
saved de-plus-plus-minus 125
typed de-plus-zero plus,zero - --size 960x640 --probe-frames 360
expect de-plus-zero 1200x800 explicit 960x640 1.00 100
saved de-plus-zero auto

# --- 10. Real keys on a US layout ----------------------------------------------
layout us
typed us-equal equal - --size 960x640 --probe-frames 300
expect us-equal 1200x800 explicit 1200x800 1.00 125
scale_keys us-equal KEY_EQUAL KEY_MINUS
typed us-equal-minus equal,minus - --size 960x640 --probe-frames 360
expect us-equal-minus 1200x800 explicit 960x640 1.00 100
saved us-equal-minus 100

# --- 11. The pointer resting on the Theme picker -------------------------------
theme="$(grep -a -m1 '^welcome:' "$OUT/seam.log" | tr ' ' '\n' | grep -a -m1 '^theme@' | cut -d@ -f2 || true)"
if [[ "$theme" =~ ^([0-9]+),([0-9]+),([0-9]+),([0-9]+)$ ]]; then
    over="$((BASH_REMATCH[1] + BASH_REMATCH[3] / 2)),$((BASH_REMATCH[2] + BASH_REMATCH[4] / 2))"
    typed over-picker equal "$over" --size 960x640 --probe-frames 300
    expect over-picker 1200x800 explicit 1200x800 1.00 125
else
    fail "over-picker: the 960x640 welcome screen reported no theme@ rect: ${theme:-nothing}"
fi

# --- 12. A window manager that refuses, and one that grants less ---------------
with_wm() {
    DISPLAY="$CAPTURE_DISPLAY" DESKTOP_SCALE_CHECK_DISPLAY="$CAPTURE_DISPLAY" \
        WAYLAND_DISPLAY=musializer-desktop-scale-no-compositor \
        python3 tools/desktop_scale_wm.py "$@" >"$OUT/wm.log" 2>&1 &
    WM_PID=$!
    for _ in {1..100}; do
        grep -q '^ready' "$OUT/wm.log" 2>/dev/null && return 0
        kill -0 "$WM_PID" 2>/dev/null || break
        sleep 0.05
    done
    fail "the window manager did not start; see $OUT/wm.log"
    return 0
}
without_wm() {
    kill "$WM_PID" 2>/dev/null || true
    wait "$WM_PID" 2>/dev/null || true
    WM_PID=""
}
desktop 120
with_wm refuse
run wm-refuse - --probe-frames 90 --probe-shot "$OUT/wm-refuse.png"
without_wm
expect wm-refuse 1280x720 scaled:1600x900 1200x800 1.25 100 1280x720
got="$(png_size "$OUT/wm-refuse.png")"
[ "$got" = 1280x720 ] || fail "wm-refuse: the photograph is $got, expected 1280x720"
with_wm clamp 1400x800
run wm-clamp - --probe-frames 90
without_wm
expect wm-clamp 1400x800 scaled:1600x900 1200x800 1.25 125

if [ "$failures" -ne 0 ]; then
    printf 'desktop_scale_check: %d failure(s); evidence %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'PASS: the window follows the desktop scale, a saved scale and the real keys; evidence %s\n' "$OUT"
