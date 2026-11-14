#!/usr/bin/env bash
#
# The look cards gate (D, MV9-l / MV3-a): Look's section and video cards,
# pressed headless. column_check.sh calls it; it also runs alone:
#
#   tools/look_cards_check.sh [OUTPUT_DIR]    # default build/look-cards-check
#
# Every launch goes through tools/headless_run.sh (private Xvfb, unreachable
# Wayland and audio endpoints, dialog PATH guard, private config, --mute).
# Every press aims at a rect from the `look cards:` (or `lane head:`) report
# line, never at a pixel measured once; every control has a press into the
# gap beside it that must change nothing. Writes are read back from the
# autosaved .musi, not only from the report. The checks are listed at the
# top of tools/look_cards_check.py.
#
# LOOK_CARDS_BIN names another binary (a negative control without replacing
# the build); MZ_GL_LAUNCH="" runs on the CPU. LOOK_CARDS_ONLY=section|video|
# tables|follow|captures runs one group.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-build/look-cards-check}"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
# No fixed display asked for: every launch takes a free one of its own.
if [ -z "${MUSIALIZER_CAPTURE_DISPLAY:-}" ]; then
    exec python3 tools/look_cards_check.py "$OUT"
fi
DISPLAY_NAME="$MUSIALIZER_CAPTURE_DISPLAY"
if [[ ! "$DISPLAY_NAME" =~ ^:([0-9]+)$ ]]; then
    echo 'look_cards_check: capture display must be :NUMBER' >&2
    exit 2
fi
NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$DISPLAY_NAME"
# A caller's last launch may still be tearing its server down (xvfb-run
# kills without waiting): give it two seconds to go. One that stays is an
# ancestor's to reuse, and every nested headless_run checks its ownership and
# refuses a foreign one (docs/TRAPS.md).
for _ in {1..40}; do
    [ -e "/tmp/.X11-unix/X$NUMBER" ] || [ -e "/tmp/.X$NUMBER-lock" ] || break
    sleep 0.05
done
if [ -e "/tmp/.X11-unix/X$NUMBER" ] || [ -e "/tmp/.X$NUMBER-lock" ]; then
    exec python3 tools/look_cards_check.py "$OUT"
fi
# One gate-owned server for the whole series: xvfb-run kills without
# waiting, and a tight series on one fixed display would see the previous
# server still exiting and rightly refuse it as foreign.
Xvfb "$DISPLAY_NAME" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB_PID=$!
cleanup() {
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
export DISPLAY="$DISPLAY_NAME"
READY=0
for _ in {1..100}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
        READY=1
        break
    fi
    kill -0 "$XVFB_PID" 2>/dev/null || break
    sleep 0.05
done
if [ "$READY" -ne 1 ]; then
    echo "look_cards_check: owned Xvfb did not become ready; see $OUT/xvfb.log" >&2
    exit 2
fi
STATUS=0
python3 tools/look_cards_check.py "$OUT" || STATUS=$?
exit "$STATUS"
