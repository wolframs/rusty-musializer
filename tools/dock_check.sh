#!/usr/bin/env bash
#
# The dock gate (WX-1/2): a real terminal in the window, checked headless.
#
#   tools/dock_check.sh [OUTPUT_DIR]        # default build/dock-check
#
# Every launch goes through tools/headless_run.sh: private Xvfb, unreachable
# Wayland and audio endpoints, kdialog/zenity off PATH, --mute. The terminal's
# shell inherits all of that, so nothing typed into it can reach the
# operator's desktop or speakers either. SHELL is pinned to /bin/sh so the
# operator's rc files cannot change what the screen shows.
#
# No real agent CLI is ever started here: the terminal runs shell builtins,
# and every chat is pointed at tools/fake_agent.py through
# MUSIALIZER_AGENT_<NAME>.
#
# DOCK_BIN names a different binary, for a negative control without
# replacing the build.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/dock-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${DOCK_BIN:-$(realpath target/debug/musializer)}"
export SHELL=/bin/sh

FIXTURE="$OUT/fixture.wav"
cargo run --quiet --bin make-fixture-wav -- "$FIXTURE" 4 >/dev/null

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
dock_line() {
    grep -E '^dock: ' "$1" || true
}

# 1. The terminal runs what is typed into it, and holds the keyboard.
#    `$((6*7))` is evaluated by the shell, so "dock-42" on screen proves a
#    program ran; the typed line itself only ever shows the literal.
tools/headless_run.sh "$FIXTURE" \
    --ui-probe 'dock-run=echo dock-$((6*7)),dock-focus=1,size=1600x900' \
    --probe-frames 80 --probe-shot "$OUT/dock.png" >"$OUT/run.log" 2>&1 || true
line="$(dock_line "$OUT/run.log")"
printf '%s\n' "${line:0:240}"
case "$line" in
    *"open=true tabs=1"*) ;;
    *) fail "the dock did not open with one tab" ;;
esac
case "$line" in
    *"dock-42"*) ;;
    *) fail "the terminal never printed the shell's own output (dock-42)" ;;
esac
case "$line" in
    *"running keyboard=true"*) ;;
    *) fail "a focused terminal did not report holding the keyboard" ;;
esac
[ -s "$OUT/dock.png" ] || fail "no capture was written"

# 2. Negative control for keyboard ownership: the same run without focus
#    must report the keyboard free, or `keyboard=true` above proves nothing.
tools/headless_run.sh "$FIXTURE" \
    --ui-probe 'dock-run=echo idle,size=1600x900' \
    --probe-frames 60 >"$OUT/unfocused.log" 2>&1 || true
line="$(dock_line "$OUT/unfocused.log")"
case "$line" in
    *"keyboard=false"*) ;;
    *) fail "an unfocused terminal claimed the keyboard: ${line:0:160}" ;;
esac

# 3. Closed by default with a fresh configuration: no tab, no shell.
tools/headless_run.sh "$FIXTURE" --probe-frames 20 >"$OUT/closed.log" 2>&1 || true
line="$(dock_line "$OUT/closed.log")"
case "$line" in
    *"open=false tabs=0"*) ;;
    *) fail "the dock opened (or started a shell) without being asked: ${line:0:160}" ;;
esac

# 4. Nothing the dock started outlives the window.
sleep 0.5
if pgrep -f 'echo dock-\$\(\(6\*7\)\)' >/dev/null 2>&1; then
    fail "a dock shell survived its window"
fi

# 5. WX-3: the window follows a treatment. Started on one plan, the file is
#    rewritten mid-run to another, then to something that does not parse.
#    The second must apply at the playhead; the third must be refused and
#    leave the second on screen.
treatment() {
    printf '{"schema":"musializer.treatment/v1","audio":{"path":"fixture.wav"},"segments":[{"at":0,"scene":"%s"}]}\n' "$1"
}
treatment spectrum >"$OUT/live.treatment.json"
( sleep 2.5; treatment loom >"$OUT/live.treatment.json.tmp" && mv "$OUT/live.treatment.json.tmp" "$OUT/live.treatment.json"
  sleep 1.5; printf '{"schema": "musializer.treatment/v1", "segments": [' >"$OUT/live.treatment.json" ) &
writer=$!
tools/headless_run.sh --treatment "$OUT/live.treatment.json" \
    --ui-probe 'follow=1' --probe-frames 420 >"$OUT/follow.log" 2>&1 || true
wait "$writer" || true
follow_line="$(grep -E '^treatment follow: ' "$OUT/follow.log" || true)"
segments_line="$(grep -E '^scene segments: ' "$OUT/follow.log" || true)"
printf '%s\n%s\n' "${follow_line:0:200}" "$segments_line"
case "$follow_line" in
    *"applied=1 refused=1"*) ;;
    *) fail "the followed treatment did not apply once and refuse once" ;;
esac
case "$segments_line" in
    *"loom"*) ;;
    *) fail "the plan on screen is not the rewritten one (loom)" ;;
esac

# 6. WX-4: a native chat with each agent, against tools/fake_agent.py, which
#    speaks each CLI's protocol and echoes the message back. Then one
#    permission round trip each way: the fake asks before its tool, and its
#    reply says what the dock answered.
#    PATH inside a headless run is the dialog guard only, so the fake is
#    started through an absolute interpreter.
FAKE="$OUT/fake-agent"
printf '#!/bin/sh\nexec /usr/bin/python3 %s "$@"\n' "$(realpath tools/fake_agent.py)" >"$FAKE"
chmod +x "$FAKE"
chat() { # agent probe-extra -> the dock line
    env MUSIALIZER_AGENT_CLAUDE="$FAKE" MUSIALIZER_AGENT_CODEX="$FAKE" MUSIALIZER_AGENT_AGY="$FAKE" \
        tools/headless_run.sh "$FIXTURE" \
        --ui-probe "agent=$1,$2,size=1600x900" --probe-frames 100 \
        --probe-shot "$OUT/chat-$1.png" 2>&1 | grep -E '^dock: ' || true
}
for agent in claude codex agy; do
    line="$(chat "$agent" 'agent-say=make the bridge colder')"
    printf '%s\n' "${line:0:220}"
    case "$line" in
        *'replies=1 tools=1'*'busy=false'*'last="fake reply to: make the bridge colder"'*) ;;
        *) fail "the $agent chat did not complete a turn with a tool call" ;;
    esac
done
# Each protocol's own words: Claude's behavior, Codex's decision.
for case_ in claude:allow:allow claude:deny:deny codex:allow:accept codex:deny:decline; do
    IFS=: read -r agent answer wire <<<"$case_"
    line="$(chat "$agent" "agent-say=please ask first,agent-decide=$answer")"
    case "$line" in
        *"prompts=1"*"(permission: $wire)"*) ;;
        *) fail "the $agent chat's $answer did not reach the agent as $wire: ${line:0:200}" ;;
    esac
done
# Negative control for the round trip: unanswered, the agent must still be
# waiting, or "permission: allow" above could come from the fake alone.
line="$(chat claude 'agent-say=please ask first')"
case "$line" in
    *"prompts=1 busy=true"*) ;;
    *) fail "an unanswered prompt did not leave the turn waiting: ${line:0:200}" ;;
esac

if [ "$failures" -gt 0 ]; then
    printf '%d dock check(s) failed; logs in %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'dock: ok (%s)\n' "$OUT"
