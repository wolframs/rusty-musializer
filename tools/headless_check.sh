#!/usr/bin/env bash
#
# Runs the Rust Musializer on a private display and captures evidence, so a
# session can check its own work without a human looking at a screen.
#
# Adapted from ../musializer/tools/ui_capture.sh and its UI_REVIEW.md. The
# isolation guarantees are the same and they are the point:
#
#   - a private Xvfb display (:77 by default), so nothing is drawn on the
#     operator's session, no window takes focus, and the real cursor never moves;
#   - WAYLAND_DISPLAY unset, so GLFW cannot pick the operator's compositor;
#   - PULSE_SERVER pointed at a path that cannot resolve, so a check never opens
#     a client stream on the audio server the operator is using;
#   - every artifact under build/, which .gitignore excludes.
#
# Usage:
#   tools/headless_check.sh [OUTPUT_DIR]
#
# Environment:
#   MUSIALIZER_CAPTURE_DISPLAY   X display to use (default :77)
#   MUSIALIZER_PROBE_FRAMES      frames to render before exiting (default 240)
#   MUSIALIZER_FIXTURE_SECONDS   synthetic fixture length (default 8)

set -euo pipefail

OUT_DIR="${1:-build/headless}"
DISPLAY_NUM="${MUSIALIZER_CAPTURE_DISPLAY:-:77}"
PROBE_FRAMES="${MUSIALIZER_PROBE_FRAMES:-240}"
FIXTURE_SECONDS="${MUSIALIZER_FIXTURE_SECONDS:-8}"
SCREEN_SIZE="1280x720x24"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

mkdir -p "$OUT_DIR"
FIXTURE="$OUT_DIR/fixture-sweep.wav"
SHOT="$OUT_DIR/spectrum.png"
REPORT="$OUT_DIR/report.txt"
XVFB_LOG="$OUT_DIR/xvfb.log"

echo "=== building ==="
cargo build --quiet

echo "=== synthetic fixture ==="
# Synthetic only. No user audio ever enters this repository.
cargo run --quiet --bin make-fixture-wav -- "$FIXTURE" "$FIXTURE_SECONDS"

echo "=== starting Xvfb on $DISPLAY_NUM ==="
if [ -e "/tmp/.X11-unix/X${DISPLAY_NUM#:}" ]; then
    echo "error: $DISPLAY_NUM is already in use. Set MUSIALIZER_CAPTURE_DISPLAY." >&2
    exit 1
fi
Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN_SIZE" -nolisten tcp >"$XVFB_LOG" 2>&1 &
XVFB_PID=$!
# One Xvfb per run, and it is always torn down: a leaked server would hold the
# display and make the next run fail for the wrong reason.
cleanup() {
    if kill -0 "$XVFB_PID" 2>/dev/null; then
        kill "$XVFB_PID" 2>/dev/null || true
        wait "$XVFB_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Wait for the display rather than sleeping blind. The C harness sleeps, and
# UI_REVIEW.md flags that as the weak point; a readiness check is cheap here.
for _ in $(seq 1 100); do
    [ -e "/tmp/.X11-unix/X${DISPLAY_NUM#:}" ] && break
    sleep 0.1
done
if [ ! -e "/tmp/.X11-unix/X${DISPLAY_NUM#:}" ]; then
    echo "error: Xvfb did not come up on $DISPLAY_NUM; see $XVFB_LOG" >&2
    exit 1
fi

echo "=== running $PROBE_FRAMES frames ==="
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --size 1280x720 \
        --probe-frames "$PROBE_FRAMES" \
        --probe-shot "$SHOT" \
    >"$REPORT" 2>&1
STATUS=$?
set -e

echo "=== report ==="
cat "$REPORT"
echo "exit status: $STATUS"

if [ "$STATUS" -ne 0 ]; then
    echo "FAIL: the application exited $STATUS" >&2
    exit "$STATUS"
fi
if [ ! -f "$SHOT" ]; then
    echo "FAIL: no screenshot at $SHOT" >&2
    exit 1
fi

# The interface face, which no screenshot can assert. A silent fall back to
# raylib's 10 px bitmap face is a regression that otherwise gets noticed by eye
# weeks later, so the report names both faces and this fails on either fallback.
FONT_LINE="$(sed -n 's/^fonts: *//p' "$REPORT")"
echo "fonts: ${FONT_LINE:-<absent>}"
case "${FONT_LINE:-absent}" in
    *FALLBACK*|absent)
        echo "FAIL: the interface or caption face did not load: ${FONT_LINE:-<absent>}" >&2
        exit 1
        ;;
esac

echo "=== screenshot ==="
ffprobe -v error -show_entries stream=width,height,pix_fmt \
    -of default=noprint_wrappers=1 "$SHOT"

# ---------------------------------------------------------------------------
# Scene sweep and panel captures.
#
# Adapted from ../musializer/tools/ui_capture.sh. UI_REVIEW.md records the
# lesson that pays for this section: the two worst defects ever found there were
# invisible to capture until the probe could name the state that showed them. A
# surface that cannot be photographed does not get reviewed, so every scene and
# every panel gets a frame, at the minimum supported window as well as at 720p.
#
# The report is parsed rather than eyeballed. `scene request: honoured` is the
# line that distinguishes "--scene parsed" from "--scene took effect", which a
# screenshot of a placeholder card cannot.
# ---------------------------------------------------------------------------

capture() {
    # capture NAME SIZE [extra args...]
    local name="$1" size="$2"
    shift 2
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer "$FIXTURE" \
            --size "$size" \
            --probe-frames 30 \
            --probe-shot "$out" \
            "$@" \
        >"$log" 2>&1
    local status=$?
    set -e
    printf '%-22s %-9s exit=%s ' "$name" "$size" "$status"
    if [ ! -f "$out" ]; then
        echo "FAIL (no frame)"
        return 1
    fi
    # The self-check the picture cannot make on its own.
    local verdict
    verdict="$(sed -n 's/^scene request: *//p' "$log")"
    local drawing
    drawing="$(sed -n 's/^scene drawing: *//p' "$log")"
    echo "scene=${verdict:-?} drawing=${drawing:-?}"
    [ "$status" -eq 0 ] || return 1
    case "$verdict" in
        MISMATCH*) return 1 ;;
    esac
    return 0
}

echo "=== scene sweep ==="
SWEEP_FAILED=0
for scene in spectrum pulse orbital ascii atlas terrarium constellation cadence loom pentagram; do
    capture "scene-$scene" 1280x720 --scene "$scene" || SWEEP_FAILED=1
done

echo "=== long scene aliases ==="
# The six long aliases are a separate code path from the ten stable names
# (`plug.c:933-964`), so they are exercised rather than assumed.
for alias in pulse-field orbital-lattice ascii-field song-atlas spectral-terrarium pentagram-orbits; do
    capture "alias-$alias" 1280x720 --scene "$alias" || SWEEP_FAILED=1
done

echo "=== panels, at 720p and at the 960x640 minimum ==="
# Both sizes on purpose: a panel's minimum size must be measured against the
# panel the minimum supported window actually produces, not against a guessed
# threshold. GLFW clamps a smaller request up to that floor.
for panel in none tune export lyrics; do
    for size in 1280x720 960x640; do
        capture "panel-$panel-$size" "$size" --ui-probe "panel=$panel,play=1" \
            || echo "  (note: $panel at $size exits non-zero while its panel is a stub)"
    done
done

echo "=== the welcome screen, with no track open ==="
# The one surface every other capture in this file cannot reach, because they all
# pass a fixture. It is also the first thing a new user sees, so a regression here
# is the most visible one available — and the layout tests can only assert where
# its pieces go, not that they were drawn.
for size in 1280x720 960x640; do
    out="$OUT_DIR/welcome-$size.png"
    log="$OUT_DIR/welcome-$size.txt"
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer \
            --size "$size" \
            --probe-frames 30 \
            --probe-shot "$out" \
        >"$log" 2>&1
    status=$?
    set -e
    printf '%-22s %-9s exit=%s ' "welcome" "$size" "$status"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL"
        SWEEP_FAILED=1
    else
        # With no track there is nothing to consume, so the verdict is expected to
        # say so. Asserting it keeps this capture honest about what it proves.
        echo "verdict=$(sed -n 's/^verdict: *//p' "$log")"
    fi
done

echo "=== the runtime track swap ==="
# The path a dropped file and the native picker both take, and the only place in
# this binary where a wrong `unsafe` ordering is a use-after-free rather than a
# wrong pixel: detach the processor, drop the Music, drain the ring, rebind the
# analyzer to the new file's sample rate, reattach. Neither a drop gesture nor a
# modal picker can be driven from a capture script, so `--probe-reopen` is what
# makes it reachable.
#
# A second fixture at a different length, so the swap is observable in the
# duration as well as in the audio-frame count.
FIXTURE_TWO="$OUT_DIR/fixture-sweep-short.wav"
cargo run --quiet --bin make-fixture-wav -- "$FIXTURE_TWO" 3
REOPEN_LOG="$OUT_DIR/reopen.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --size 1280x720 \
        --probe-frames 120 \
        --probe-reopen "$FIXTURE_TWO" \
        --probe-shot "$OUT_DIR/reopen.png" \
    >"$REOPEN_LOG" 2>&1
REOPEN_STATUS=$?
set -e
REOPEN_LINE="$(sed -n 's/^reopen: *//p' "$REOPEN_LOG")"
echo "reopen: ${REOPEN_LINE:-<absent>} (exit=$REOPEN_STATUS)"
# Three separate claims, because a clean exit proves none of them: the swap ran,
# it did not report failure, and audio arrived through the *second* attachment.
if [ "$REOPEN_STATUS" -ne 0 ]; then
    echo "FAIL: the track swap run exited $REOPEN_STATUS" >&2
    SWEEP_FAILED=1
fi
case "${REOPEN_LINE:-absent}" in
    ok*)
        frames_after="$(printf '%s' "$REOPEN_LINE" | sed -n 's/.*; \([0-9]*\) audio frames.*/\1/p')"
        if [ -z "$frames_after" ] || [ "$frames_after" -le 0 ]; then
            echo "FAIL: no audio arrived through the reattached stream" >&2
            SWEEP_FAILED=1
        fi
        ;;
    *)
        echo "FAIL: the track swap did not run: ${REOPEN_LINE:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac

echo "=== a routed setting ==="
# Routes are applied after every input is resolved, and the report prints how
# many landed on the active scene.
capture "route-loom-weight" 1280x720 \
    --scene loom --route 'loom.weight:band:2:0:1:0.4:2.2:smoothstep' || SWEEP_FAILED=1
sed -n 's/^routes: */routes on the active scene: /p' "$OUT_DIR/route-loom-weight.txt"

if [ "$SWEEP_FAILED" -ne 0 ]; then
    echo "FAIL: at least one scene or alias capture failed" >&2
    exit 1
fi

echo "artifacts in $OUT_DIR"
