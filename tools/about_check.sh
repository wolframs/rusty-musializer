#!/usr/bin/env bash
# About: report-addressed presses and dark/light captures, isolated and muted.
set -euo pipefail
cd "$(dirname "$0")/.."
ABOUT_OUT="${1:-build/about/gpu}"
mkdir -p "$ABOUT_OUT"
ABOUT_OUT="$(realpath "$ABOUT_OUT")"
ABOUT_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:89}"
if [[ ! "$ABOUT_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'about_check: capture display must be :NUMBER' >&2
    exit 2
fi
ABOUT_CAPTURE_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$ABOUT_CAPTURE_DISPLAY"
# An ancestor's server may already exist. Every nested headless_run checks its
# ownership and refuses a foreign server; this branch never attaches by itself.
if [ -e "/tmp/.X11-unix/X$ABOUT_CAPTURE_NUMBER" ] || [ -e "/tmp/.X$ABOUT_CAPTURE_NUMBER-lock" ]; then
    exec python3 tools/about_check.py "$ABOUT_OUT"
fi
# xvfb-run kills without wait during cleanup. A tight series on one fixed
# display can see its previous server still exiting and correctly refuse it
# as foreign. Hold one gate-owned server; nested launches reuse only this
# ancestor's server, preserving all per-call audio/dialog/config guards.
Xvfb "$ABOUT_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$ABOUT_OUT/xvfb.log" 2>&1 &
ABOUT_CAPTURE_PID=$!
about_capture_cleanup() {
    kill "$ABOUT_CAPTURE_PID" 2>/dev/null || true
    wait "$ABOUT_CAPTURE_PID" 2>/dev/null || true
}
trap about_capture_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
export DISPLAY="$ABOUT_CAPTURE_DISPLAY"
ABOUT_CAPTURE_READY=0
for ABOUT_ATTEMPT in {1..100}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
        ABOUT_CAPTURE_READY=1
        break
    fi
    kill -0 "$ABOUT_CAPTURE_PID" 2>/dev/null || break
    sleep 0.05
done
if [ "$ABOUT_CAPTURE_READY" -ne 1 ]; then
    echo "about_check: owned Xvfb did not become ready; see $ABOUT_OUT/xvfb.log" >&2
    exit 2
fi
ABOUT_GATE_STATUS=0
python3 tools/about_check.py "$ABOUT_OUT" || ABOUT_GATE_STATUS=$?
exit "$ABOUT_GATE_STATUS"
