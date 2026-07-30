#!/usr/bin/env bash
#
# Everything that can check itself, in one command.
#
#   tools/verify.sh            # the full suite
#   tools/verify.sh --quick    # skip the headless capture (no Xvfb needed)
#
# Run this before handing work over and after every merge. It is deliberately
# ordered cheapest-first, so a formatting slip fails in seconds rather than after
# a five-minute encode.
#
# The oracle at ../musializer is read-only throughout, and the last step proves it:
# a non-clean oracle tree fails the run. Several stages compile C from it, with all
# output going into our own build/.

set -uo pipefail

QUICK=0
[ "${1:-}" = "--quick" ] && QUICK=1

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ORACLE="/home/wolfram/Projects/musializer"
ORACLE_COMMIT="9300af942bd00d8c85fc4e3c8c02cf2b6356764f"

FAILED=()
PASSED=0

step() {
    local name="$1"
    shift
    printf '\n\033[1m=== %s ===\033[0m\n' "$name"
    if "$@"; then
        PASSED=$((PASSED + 1))
        printf '\033[32mok\033[0m  %s\n' "$name"
    else
        FAILED+=("$name")
        printf '\033[31mFAILED\033[0m  %s\n' "$name"
    fi
}

# Cheapest first.
step "cargo fmt --check" cargo fmt --check
step "cargo build" cargo build --quiet
step "cargo clippy" cargo clippy --all-targets --quiet
step "cargo test" cargo test --quiet

# Differential harnesses against the frozen C. These are the evidence that the
# ports are faithful rather than merely plausible.
for harness in analyzer settings routes route_persistence event_merge assist_ui preset_store song_atlas_map ascii_art project_io timeline_view; do
    script="tools/differential_${harness}.sh"
    [ -x "$script" ] && step "differential: $harness" "$script"
done

if [ "$QUICK" -eq 0 ]; then
    if command -v Xvfb >/dev/null 2>&1; then
        step "headless gate (window, audio, Spectrum)" tools/headless_check.sh
    else
        printf '\n\033[33mskipped\033[0m  headless gate: Xvfb not installed\n'
    fi
fi

# Last, and non-negotiable: the parity oracle must be untouched. Every stage above
# only reads it, and this is what proves the whole run kept that promise.
printf '\n\033[1m=== oracle is read-only ===\033[0m\n'
oracle_dirty=$(git -C "$ORACLE" status --porcelain 2>/dev/null | wc -l)
oracle_head=$(git -C "$ORACLE" rev-parse HEAD 2>/dev/null)
if [ "$oracle_dirty" -ne 0 ]; then
    printf '\033[31mFAILED\033[0m  the oracle has %s uncommitted changes — it must never be modified\n' "$oracle_dirty"
    git -C "$ORACLE" status --short | head -20
    FAILED+=("oracle unmodified")
elif [ "$oracle_head" != "$ORACLE_COMMIT" ]; then
    printf '\033[31mFAILED\033[0m  the oracle is at %s, expected the freeze commit %s\n' \
        "${oracle_head:0:7}" "${ORACLE_COMMIT:0:7}"
    FAILED+=("oracle at freeze commit")
else
    PASSED=$((PASSED + 1))
    printf '\033[32mok\033[0m  clean at %s\n' "${oracle_head:0:7}"
fi

printf '\n\033[1m=== summary ===\033[0m\n'
printf '%d passed' "$PASSED"
if [ "${#FAILED[@]}" -eq 0 ]; then
    printf ', 0 failed\n'
    exit 0
fi
printf ', %d failed:\n' "${#FAILED[@]}"
for name in "${FAILED[@]}"; do
    printf '  - %s\n' "$name"
done
exit 1
