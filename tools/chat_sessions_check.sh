#!/usr/bin/env bash
# Saved agent chats, against tools/fake_agent.py: kept per project, reopened,
# resumed in the CLI, forgotten. A real agent is never started. MZ_GL_LAUNCH=""
# selects llvmpipe.
set -euo pipefail
cd "$(dirname "$0")/.."
CHAT_OUT="${1:-build/chat-sessions}"
mkdir -p "$CHAT_OUT"
CHAT_OUT="$(realpath "$CHAT_OUT")"
CHAT_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:92}"
if [[ ! "$CHAT_DISPLAY" =~ ^:([0-9]+)$ ]]; then
    echo 'chat_sessions_check: capture display must be :NUMBER' >&2
    exit 2
fi
CHAT_NUMBER="${BASH_REMATCH[1]}"
export MUSIALIZER_CAPTURE_DISPLAY="$CHAT_DISPLAY"
# An ancestor's server may already exist; every nested headless_run checks its
# ownership and refuses a foreign one. Otherwise this gate owns one server for
# its series, because xvfb-run's kill does not wait (docs/TRAPS.md).
if [ -e "/tmp/.X11-unix/X$CHAT_NUMBER" ] || [ -e "/tmp/.X$CHAT_NUMBER-lock" ]; then
    exec python3 tools/chat_sessions_check.py "$CHAT_OUT"
fi
Xvfb "$CHAT_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp >"$CHAT_OUT/xvfb.log" 2>&1 &
CHAT_XVFB=$!
chat_cleanup() {
    kill "$CHAT_XVFB" 2>/dev/null || true
    wait "$CHAT_XVFB" 2>/dev/null || true
}
trap chat_cleanup EXIT
for _ in $(seq 1 100); do
    [ -e "/tmp/.X11-unix/X$CHAT_NUMBER" ] && break
    sleep 0.05
done
python3 tools/chat_sessions_check.py "$CHAT_OUT"
