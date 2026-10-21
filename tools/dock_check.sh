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
# No agent CLI is ever started here: the tab buttons are not pressed, and the
# typed command uses only shell builtins.
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

if [ "$failures" -gt 0 ]; then
    printf '%d dock check(s) failed; logs in %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'dock: ok (%s)\n' "$OUT"
