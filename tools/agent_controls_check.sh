#!/usr/bin/env bash
# The composer's model, effort and access controls, against tools/fake_agent.py.
# Presses go through egui's pointer events (`--ui-probe agent-press=`); a real
# agent is never started. MZ_GL_LAUNCH="" selects llvmpipe.
set -euo pipefail
cd "$(dirname "$0")/.."
AGENT_OUT="${1:-build/agent-controls}"
mkdir -p "$AGENT_OUT"
AGENT_OUT="$(realpath "$AGENT_OUT")"
AGENT_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:91}"
if [[ ! "$AGENT_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'agent_controls_check: capture display must be :NUMBER' >&2
    exit 2
fi
AGENT_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$AGENT_DISPLAY"
# An ancestor's server may already exist; every nested headless_run checks its
# ownership and refuses a foreign one. Otherwise this gate owns one server for
# its series, because xvfb-run's kill does not wait (docs/TRAPS.md).
if [ -e "/tmp/.X11-unix/X$AGENT_NUMBER" ] || [ -e "/tmp/.X$AGENT_NUMBER-lock" ]; then
    exec python3 tools/agent_controls_check.py "$AGENT_OUT"
fi
Xvfb "$AGENT_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$AGENT_OUT/xvfb.log" 2>&1 &
AGENT_XVFB=$!
agent_cleanup() {
    kill "$AGENT_XVFB" 2>/dev/null || true
    wait "$AGENT_XVFB" 2>/dev/null || true
}
trap agent_cleanup EXIT
for _ in $(seq 1 100); do
    [ -e "/tmp/.X11-unix/X$AGENT_NUMBER" ] && break
    sleep 0.05
done
python3 tools/agent_controls_check.py "$AGENT_OUT"
