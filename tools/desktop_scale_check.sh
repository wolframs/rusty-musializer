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
#      with a native 1.25 atlas, the welcome screen is drawn and photographed
#      at that size.
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
#   7. `--ui-scale 100 --ui-probe size=960x640` on the scaled desktop: 960x640
#      at 100 % with the application's own floor. The capture seams stay raw.
#
# NEGATIVE CONTROLS (2026-11-19), each a perturbed debug build copied aside
# and run as DESKTOP_SCALE_BIN, the file restored from a saved copy and
# compared with `cmp`; A and B on the GPU path and on the CPU path
# (MZ_GL_LAUNCH=""):
#   A (checks 1 and 4): `startup_window` returning the default.
#   B (checks 5 and 6): the floor applied as a limit only, the window under
#     it left where it was.
#   C (check 7): the `--ui-scale` exemption removed.
# What each caught is recorded in docs/archive/FEATURE_PARITY_HISTORY.md, "DS".

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/desktop-scale-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${DESKTOP_SCALE_BIN:-$(realpath target/debug/musializer)}"

for tool in Xvfb xrdb xdpyinfo; do
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
capture_cleanup() {
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

line_of() { grep -a -m1 "^$1:" "$2" | sed "s/^$1: *//" || true; }
# field KEY LINE   the value of KEY= in a report line.
field() { printf '%s\n' "$2" | tr ' ' '\n' | grep -a -m1 "^$1=" | cut -d= -f2- || true; }

# expect NAME WINDOW STARTUP FLOOR DESKTOP SCALE
expect() {
    local name="$1" log="$OUT/$1.log"
    local window layout got
    window="$(line_of window "$log")"
    layout="$(line_of 'ui layout' "$log")"
    if [ -z "$window" ] || [ -z "$layout" ]; then
        fail "$name: no window: or ui layout: line; see $log"
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
    got="$(field scale "$layout")"
    [ "$got" = "$6" ] || fail "$name: the shell drew at $got %, expected $6 %"
}

png_size() {
    python3 -c 'import struct,sys
with open(sys.argv[1],"rb") as f: head=f.read(24)
print("%dx%d" % struct.unpack(">II", head[16:24]) if head[:8]==b"\x89PNG\r\n\x1a\n" else "not-a-png")' "$1" 2>/dev/null || echo unreadable
}

# --- 1. A scaled desktop, nothing named ----------------------------------------
desktop 120
run scaled-desktop - --probe-frames 24 --probe-shot "$OUT/scaled-desktop.png"
expect scaled-desktop 1600x900 scaled:1600x900 1200x800 1.25 125
fonts="$(line_of fonts "$OUT/scaled-desktop.log")"
case "$fonts" in
*scale=1.25*) ;;
*) fail "scaled-desktop: the font atlas is not the 1.25 one: $fonts" ;;
esac
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
saved="$(tr -d ' \n' <"$OUT/state-ctrl-plus/config/musializer/ui.json" 2>/dev/null || true)"
case "$saved" in
*'"scale":"125"'*) ;;
*) fail "ctrl-plus: the saved preference does not say 125: ${saved:-no file}" ;;
esac

# --- 7. The capture seams stay raw ---------------------------------------------
desktop 120
run seam - --ui-scale 100 --ui-probe size=960x640 --probe-frames 24
expect seam 960x640 scaled:1600x900 960x640 1.25 100

if [ "$failures" -ne 0 ]; then
    printf 'desktop_scale_check: %d failure(s); evidence %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'PASS: the window follows the desktop scale, a saved scale and Ctrl+plus; evidence %s\n' "$OUT"
