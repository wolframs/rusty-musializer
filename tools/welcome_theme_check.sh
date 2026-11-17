#!/usr/bin/env bash
# Welcome appearance inherits the same saved choices as Look's footer.
# Actual theme presses/restarts are checked in pixel_look_check.sh; welcome
# picker ownership and contrast have headless egui input tests in egui_theme.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-build/welcome-theme-check}"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
WELCOME_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:85}"
if [[ ! "$WELCOME_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'welcome_theme_check: capture display must be :NUMBER' >&2
    exit 2
fi
WELCOME_CAPTURE_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$WELCOME_CAPTURE_DISPLAY"
# An ancestor's server may already exist. Every nested headless_run checks its
# ownership and refuses a foreign server; this branch never attaches by itself.
if [ ! -e "/tmp/.X11-unix/X$WELCOME_CAPTURE_NUMBER" ] && [ ! -e "/tmp/.X$WELCOME_CAPTURE_NUMBER-lock" ]; then
    # xvfb-run kills without wait during cleanup. A tight series on one fixed
    # display can see its previous server still exiting and correctly refuse it
    # as foreign. Hold one gate-owned server; nested launches reuse only this
    # ancestor's server, preserving all per-call audio/dialog/config guards.
    Xvfb "$WELCOME_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
    WELCOME_CAPTURE_PID=$!
    welcome_capture_cleanup() {
        kill "$WELCOME_CAPTURE_PID" 2>/dev/null || true
        wait "$WELCOME_CAPTURE_PID" 2>/dev/null || true
    }
    trap welcome_capture_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM HUP
    export DISPLAY="$WELCOME_CAPTURE_DISPLAY"
    WELCOME_CAPTURE_READY=0
    for WELCOME_ATTEMPT in {1..100}; do
        if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
            WELCOME_CAPTURE_READY=1
            break
        fi
        kill -0 "$WELCOME_CAPTURE_PID" 2>/dev/null || break
        sleep 0.05
    done
    if [ "$WELCOME_CAPTURE_READY" -ne 1 ]; then
        echo "welcome_theme_check: owned Xvfb did not become ready; see $OUT/xvfb.log" >&2
        exit 2
    fi
fi
for theme in black-amber light-blue; do
    state="$OUT/$theme-state"
    mkdir -p "$state/config/musializer"
    printf '{"schema":"musializer.ui-preferences/v1","scale":"100","theme":"%s"}\n' "$theme" > "$state/config/musializer/ui.json"
    rm -f "$OUT/$theme.png"
    MUSIALIZER_UI_PREFERENCES="$state/config/musializer/ui.json" \
        MZ_STATE_DIR="$state" tools/headless_run.sh --ui-scale 100 \
        --ui-probe size=960x640 --probe-frames 24 --probe-shot "$OUT/$theme.png" > "$OUT/$theme.log" 2>&1
    grep -q "^appearance: *$theme " "$OUT/$theme.log"
    grep -q "^welcome: *open-audio@" "$OUT/$theme.log"
    test -s "$OUT/$theme.png"
done
printf 'PASS: welcome restored dark and light; evidence %s\n' "$OUT"
