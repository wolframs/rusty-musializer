#!/usr/bin/env bash
# Pixel/CRT filter and appearance moved from the retired toolkit inspector.
# Every launch uses the isolated, process-muted launcher. Control points come
# from pixel look:; writes are checked against the actual next-frame model.
set -euo pipefail
cd "$(dirname "$0")/.."
PIXEL_OUT="${1:-build/wx5-editor/gpu}"
mkdir -p "$PIXEL_OUT"
PIXEL_OUT="$(realpath "$PIXEL_OUT")"
PIXEL_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:85}"
if [[ ! "$PIXEL_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'pixel_look_check: capture display must be :NUMBER' >&2
    exit 2
fi
PIXEL_CAPTURE_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$PIXEL_CAPTURE_DISPLAY"
# An ancestor's server may already exist. Every nested headless_run checks its
# ownership and refuses a foreign server; this branch never attaches by itself.
if [ -e "/tmp/.X11-unix/X$PIXEL_CAPTURE_NUMBER" ] || [ -e "/tmp/.X$PIXEL_CAPTURE_NUMBER-lock" ]; then
    exec python3 tools/pixel_look_check.py "$PIXEL_OUT"
fi
# xvfb-run kills without wait during cleanup. A tight series on one fixed
# display can see its previous server still exiting and correctly refuse it
# as foreign. Hold one gate-owned server; nested launches reuse only this
# ancestor's server, preserving all per-call audio/dialog/config guards.
Xvfb "$PIXEL_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$PIXEL_OUT/xvfb.log" 2>&1 &
PIXEL_CAPTURE_PID=$!
pixel_capture_cleanup() {
    kill "$PIXEL_CAPTURE_PID" 2>/dev/null || true
    wait "$PIXEL_CAPTURE_PID" 2>/dev/null || true
}
trap pixel_capture_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
export DISPLAY="$PIXEL_CAPTURE_DISPLAY"
PIXEL_CAPTURE_READY=0
for PIXEL_ATTEMPT in {1..100}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
        PIXEL_CAPTURE_READY=1
        break
    fi
    kill -0 "$PIXEL_CAPTURE_PID" 2>/dev/null || break
    sleep 0.05
done
if [ "$PIXEL_CAPTURE_READY" -ne 1 ]; then
    echo "pixel_look_check: owned Xvfb did not become ready; see $PIXEL_OUT/xvfb.log" >&2
    exit 2
fi
PIXEL_GATE_STATUS=0
python3 tools/pixel_look_check.py "$PIXEL_OUT" || PIXEL_GATE_STATUS=$?
exit "$PIXEL_GATE_STATUS"
