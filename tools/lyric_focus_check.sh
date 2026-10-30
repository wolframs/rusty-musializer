#!/usr/bin/env bash
# Native text-focus regression checks on one owned, silent capture display.
set -euo pipefail
cd "$(dirname "$0")/.."
FOCUS_OUT="${1:-build/review-fixes/focus}"
mkdir -p "$FOCUS_OUT"
FOCUS_OUT="$(realpath "$FOCUS_OUT")"
FOCUS_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:88}"
if [[ ! "$FOCUS_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'lyric_focus_check: capture display must be :NUMBER' >&2
    exit 2
fi
FOCUS_CAPTURE_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$FOCUS_CAPTURE_DISPLAY"
# An ancestor's server may already exist. Every nested headless_run checks its
# ownership and refuses a foreign server; this branch never attaches by itself.
if [ -e "/tmp/.X11-unix/X$FOCUS_CAPTURE_NUMBER" ] || [ -e "/tmp/.X$FOCUS_CAPTURE_NUMBER-lock" ]; then
    exec python3 tools/lyric_focus_check.py "$FOCUS_OUT"
fi
# xvfb-run kills without wait during cleanup. A tight series on one fixed
# display can see its previous server still exiting and correctly refuse it
# as foreign. Hold one gate-owned server; nested launches reuse only this
# ancestor's server, preserving all per-call audio/dialog/config guards.
Xvfb "$FOCUS_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$FOCUS_OUT/xvfb.log" 2>&1 &
FOCUS_CAPTURE_PID=$!
pixel_capture_cleanup() {
    kill "$FOCUS_CAPTURE_PID" 2>/dev/null || true
    wait "$FOCUS_CAPTURE_PID" 2>/dev/null || true
}
trap pixel_capture_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
export DISPLAY="$FOCUS_CAPTURE_DISPLAY"
FOCUS_CAPTURE_READY=0
for FOCUS_ATTEMPT in {1..100}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
        FOCUS_CAPTURE_READY=1
        break
    fi
    kill -0 "$FOCUS_CAPTURE_PID" 2>/dev/null || break
    sleep 0.05
done
if [ "$FOCUS_CAPTURE_READY" -ne 1 ]; then
    echo "lyric_focus_check: owned Xvfb did not become ready; see $FOCUS_OUT/xvfb.log" >&2
    exit 2
fi
FOCUS_GATE_STATUS=0
python3 tools/lyric_focus_check.py "$FOCUS_OUT" || FOCUS_GATE_STATUS=$?
exit "$FOCUS_GATE_STATUS"
