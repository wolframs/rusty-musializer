#!/usr/bin/env bash
#
# Everything that can check itself, in one command.
#
#   tools/verify.sh            # the full suite
#   tools/verify.sh --quick    # skip the headless capture (no Xvfb needed)
#   tools/verify.sh --jobs 1   # serialize non-visual checks for diagnosis
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
VERIFY_JOBS="${VERIFY_JOBS:-4}"

usage() {
    cat <<'EOF'
Usage: tools/verify.sh [--quick] [--jobs N]

  --quick    skip the private-Xvfb capture gate
  --jobs N   run at most N independent non-visual checks together (default: 4)
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --quick)
            QUICK=1
            ;;
        --jobs)
            shift
            VERIFY_JOBS="${1:-}"
            ;;
        --jobs=*)
            VERIFY_JOBS="${1#--jobs=}"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf 'unknown verify option: %s\n\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

case "$VERIFY_JOBS" in
    ''|*[!0-9]*|0)
        printf 'verify jobs must be a positive integer, got: %s\n' "$VERIFY_JOBS" >&2
        exit 2
        ;;
esac

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ORACLE="/home/wolfram/Projects/musializer"
ORACLE_COMMIT="9300af942bd00d8c85fc4e3c8c02cf2b6356764f"

FAILED=()
PASSED=0
VERIFY_LOG_DIR=$(mktemp -d "${TMPDIR:-/tmp}/musializer-verify.XXXXXX")
PARALLEL_NAMES=()
PARALLEL_PIDS=()
PARALLEL_LOGS=()
ACTIVE_PIDS=()
declare -A PARALLEL_STATUS=()

cleanup() {
    local status=$?
    trap - EXIT
    for pid in "${ACTIVE_PIDS[@]}"; do
        # Every queued command has its own session, so interruption reaches its
        # compiler/test descendants rather than leaving a cargo or Python child.
        kill -TERM -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
    done
    for pid in "${ACTIVE_PIDS[@]}"; do
        wait "$pid" 2>/dev/null || true
    done
    rm -rf -- "$VERIFY_LOG_DIR"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

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

reap_one() {
    local completed status=0
    wait -n -p completed "${ACTIVE_PIDS[@]}" || status=$?
    PARALLEL_STATUS["$completed"]=$status
    local remaining=()
    local pid
    for pid in "${ACTIVE_PIDS[@]}"; do
        [ "$pid" = "$completed" ] || remaining+=("$pid")
    done
    ACTIVE_PIDS=("${remaining[@]}")
}

queue_step() {
    local key="$1"
    local name="$2"
    shift 2
    local log="$VERIFY_LOG_DIR/$key.log"
    setsid --wait "$@" >"$log" 2>&1 &
    local pid=$!
    PARALLEL_NAMES+=("$name")
    PARALLEL_PIDS+=("$pid")
    PARALLEL_LOGS+=("$log")
    ACTIVE_PIDS+=("$pid")
    if [ "${#ACTIVE_PIDS[@]}" -ge "$VERIFY_JOBS" ]; then
        reap_one
    fi
}

finish_queued_steps() {
    while [ "${#ACTIVE_PIDS[@]}" -gt 0 ]; do
        reap_one
    done

    local index name pid log status
    for index in "${!PARALLEL_PIDS[@]}"; do
        name="${PARALLEL_NAMES[$index]}"
        pid="${PARALLEL_PIDS[$index]}"
        log="${PARALLEL_LOGS[$index]}"
        status="${PARALLEL_STATUS[$pid]}"
        printf '\n\033[1m=== %s ===\033[0m\n' "$name"
        [ ! -s "$log" ] || cat "$log"
        if [ "$status" -eq 0 ]; then
            PASSED=$((PASSED + 1))
            printf '\033[32mok\033[0m  %s\n' "$name"
        else
            FAILED+=("$name")
            printf '\033[31mFAILED\033[0m  %s\n' "$name"
        fi
    done
}

# Cheapest first.
step "code map is current" tools/code_map.py --check
step "cargo fmt --check" cargo fmt --check
step "cargo build" cargo build --quiet
step "cargo clippy" cargo clippy --all-targets --quiet

# These checks have disjoint output directories and do not open a window or an
# audio device. Keep their logs separate, cap concurrency for ordinary laptops,
# and report in registry order so a parallel run remains readable and stable.
queue_step "cargo-test" "cargo test" cargo test --quiet
queue_step "support-bundle" "support bundle (offline Assist)" tools/support_bundle_check.sh
queue_step "secret-canary" "secret canary (provider credentials)" tools/secret_canary_check.sh

# Differential harnesses against the frozen C. These are the evidence that the
# ports are faithful rather than merely plausible.
for harness in analyzer beat_tracker settings routes route_persistence event_merge assist_ui preset_store song_atlas_map ascii_art project_io timeline_view layout; do
    script="tools/differential_${harness}.sh"
    if [ ! -x "$script" ]; then
        printf '\n\033[31mFAILED\033[0m  missing executable differential harness: %s\n' "$script"
        FAILED+=("differential: $harness")
        continue
    fi
    queue_step "differential-$harness" "differential: $harness" "$script"
done
finish_queued_steps

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
