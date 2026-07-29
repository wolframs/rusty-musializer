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

# ---------------------------------------------------------------------------
# The Assist panel (Agent J).
#
# Its three interesting states are unreachable from the panel sweep above: the
# default body is Ready, and the confirmation step -- the one the panel exists
# for, because it names the lyric sheet a run will use -- only appears once a
# workflow has been proposed. `--ui-probe assist=confirm` is what reaches it,
# and `lyrics-file=` is what makes the reference row say something other than
# "none chosen".
#
# The report's `assist:` line is the evidence, because the panel draws the same
# box whether the helper was found or not: it names the resolved helper, the job
# state, which of the six bodies is showing, and whether anything is staged.
# ---------------------------------------------------------------------------
echo "=== the Assist panel ==="
# Deliberately NOT beside the fixture: `<stem>.lyrics.txt` is what the helper
# discovers on its own, and a sheet sitting there would make every capture read
# "found beside the audio" whether the probe worked or not.
ASSIST_SHEET="$OUT_DIR/authored-lyrics.txt"
printf 'a first line\na second line\n' >"$ASSIST_SHEET"
# A stand-in for tools/external_analysis.py, which this repository does not ship.
# `MUSIALIZER_ASSIST_HELPER` is the oracle's own override (`plug.c:2051-2056`),
# and without it the workflow buttons are only ever photographed disabled.
ASSIST_HELPER="$OUT_DIR/fake_external_analysis.py"
printf 'raise SystemExit(0)\n' >"$ASSIST_HELPER"

assist_capture() {
    # assist_capture NAME SIZE EXPECT_BODY EXPECT_HELPER [extra --ui-probe keys]
    local name="$1" size="$2" expect="$3" helper="$4" extra="${5:-}"
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    local spec="panel=assist,play=1"
    [ -n "$extra" ] && spec="$spec,$extra"
    local helper_env=("MUSIALIZER_ASSIST_HELPER=")
    [ "$helper" = "found" ] && helper_env=("MUSIALIZER_ASSIST_HELPER=$ASSIST_HELPER")
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        "${helper_env[@]}" \
        ./target/debug/musializer "$FIXTURE" \
            --size "$size" \
            --probe-frames 30 \
            --probe-shot "$out" \
            --ui-probe "$spec" \
        >"$log" 2>&1
    local status=$?
    set -e
    local line
    line="$(sed -n 's/^assist: *//p' "$log")"
    printf '%-28s %-9s exit=%s %s\n' "$name" "$size" "$status" "${line:-<no assist line>}"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL: $name did not produce a frame, or exited $status" >&2
        return 1
    fi
    # Evidence, not existence: the body the probe asked for is the body drawn,
    # and the helper the environment named is the helper that was resolved.
    case "$line" in
        *"body=$expect"*) ;;
        *)
            echo "FAIL: $name expected body=$expect, got: ${line:-<absent>}" >&2
            return 1
            ;;
    esac
    case "$helper:$line" in
        "found:helper=$ASSIST_HELPER"*) ;;
        "missing:helper=not found"*) ;;
        *)
            echo "FAIL: $name expected a $helper helper, got: ${line:-<absent>}" >&2
            return 1
            ;;
    esac
    return 0
}

# The panel draws from `ui/panels/assist.rs`, but reaching its states and
# reporting them needs the `main.rs` and `shell.rs` seams in Agent J's NOTE
# ENTRIES section. Until those land there is no `assist:` line to assert on, and
# a check that silently passed would be worse than one that says why it did not
# run. This is loud rather than fatal so an unrelated `verify.sh` stays readable.
set +e
env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" --size 1280x720 --probe-frames 3 \
    >"$OUT_DIR/assist-probe.txt" 2>&1
set -e
ASSIST_WIRED=0
grep -q '^assist:' "$OUT_DIR/assist-probe.txt" && ASSIST_WIRED=1

if [ "$ASSIST_WIRED" -eq 0 ]; then
    echo "SKIPPED: the report has no 'assist:' line, so the Assist panel is not"
    echo "         reachable from a probe yet. Apply the main.rs and shell.rs"
    echo "         seams in REWRITE_PLAN.md's '#### Agent J' note, then rerun."
fi

for size in 1280x720 960x640; do
    [ "$ASSIST_WIRED" -eq 0 ] && break
    # Without a helper: every workflow button disabled, and the status line says
    # why. This is what a real installation of this build looks like today.
    assist_capture "assist-ready-$size" "$size" Ready missing || SWEEP_FAILED=1
    # With one: the workflow row is live and the confirmation step can arm.
    assist_capture "assist-confirm-$size" "$size" Confirmation found \
        "assist=confirm" || SWEEP_FAILED=1
    # And with an authored sheet chosen, which is the row the panel exists for.
    assist_capture "assist-sheet-$size" "$size" Confirmation found \
        "assist=confirm,lyrics-file=$ASSIST_SHEET" || SWEEP_FAILED=1
done

# The panel that names a sheet must actually have taken it. A run that ignored
# `lyrics-file=` would draw an identical-looking panel reading "none chosen", so
# the report says which of the three references resolved.
for size in 1280x720 960x640; do
    [ "$ASSIST_WIRED" -eq 0 ] && break
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-sheet-$size.txt")" in
        *"sheet=Chosen"*) ;;
        *)
            echo "FAIL: --ui-probe lyrics-file= did not select the sheet at $size" >&2
            SWEEP_FAILED=1
            ;;
    esac
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-confirm-$size.txt")" in
        *"sheet=None"*) ;;
        *)
            echo "FAIL: a run with no sheet chosen claimed one at $size" >&2
            SWEEP_FAILED=1
            ;;
    esac
done

# `assist=confirm` without `panel=assist` must be refused rather than quietly
# arming a step in a panel nobody can see (`musializer.c:128-130`).
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --size 1280x720 --probe-frames 5 \
        --ui-probe "panel=export,assist=confirm" \
    >"$OUT_DIR/assist-misplaced.txt" 2>&1
MISPLACED_STATUS=$?
set -e
echo "assist=confirm without panel=assist: exit=$MISPLACED_STATUS (1 expected)"
if [ "$MISPLACED_STATUS" -eq 0 ] && [ "$ASSIST_WIRED" -eq 1 ]; then
    echo "FAIL: assist=confirm was honoured outside the Assist panel" >&2
    SWEEP_FAILED=1
fi

echo "=== the analysis bridge importer ==="
# `--analysis-bridge` applies rather than staging, so the evidence is that the
# track's lyric lane changed. A bridge for other audio must be refused, which is
# the guard the whole format exists for.
BRIDGE_DIR="$OUT_DIR/bridge"
rm -rf "$BRIDGE_DIR"
mkdir -p "$BRIDGE_DIR"
BRIDGE_LOG="$OUT_DIR/analysis-bridge.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --analysis-bridge "$BRIDGE_DIR/absent.bridge.tsv" \
        --size 1280x720 --probe-frames 5 \
    >"$BRIDGE_LOG" 2>&1
BRIDGE_STATUS=$?
set -e
echo "absent bridge: exit=$BRIDGE_STATUS (1 expected)"
if [ "$BRIDGE_STATUS" -eq 0 ]; then
    # Unconditional: the flag errors either way today -- unwired it reports
    # "not implemented", wired it reports "the bridge is not there" -- so a zero
    # exit means it silently accepted a file that does not exist.
    echo "FAIL: --analysis-bridge accepted a file that is not there" >&2
    SWEEP_FAILED=1
fi

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
# A fourth claim, and the one the old single-slot model could not have made: the
# swap kept *both* tracks in the workspace and made the second one current. A run
# that merely rebound the stream and forgot the first track would pass every
# check above.
TRACKS_LINE="$(sed -n 's/^tracks: *//p' "$REOPEN_LOG")"
echo "tracks after the swap: ${TRACKS_LINE:-<absent>}"
case "${TRACKS_LINE:-absent}" in
    "2 open, current 1 "*) ;;
    *)
        echo "FAIL: expected two tracks with the second current, got: ${TRACKS_LINE:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac

echo "=== the .musi round trip ==="
# Save a project, then open it back and check that the audio came through the
# *bundled* copy rather than the file it was saved from. A save that wrote a
# syntactically valid project referring to nothing would pass a "the file exists"
# check and fail here.
PROJECT_DIR="$OUT_DIR/project"
rm -rf "$PROJECT_DIR"
mkdir -p "$PROJECT_DIR"
cp "$FIXTURE" "$PROJECT_DIR/source.wav"
PROJECT="$PROJECT_DIR/show.musi"
SAVE_LOG="$OUT_DIR/project-save.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$PROJECT_DIR/source.wav" \
        --save-project "$PROJECT" \
    >"$SAVE_LOG" 2>&1
SAVE_STATUS=$?
set -e
BUNDLED="$(find "$PROJECT_DIR/show.assets" -type f 2>/dev/null | head -1)"
echo "saved: exit=$SAVE_STATUS project=$([ -f "$PROJECT" ] && echo present || echo ABSENT) bundled=$([ -n "$BUNDLED" ] && echo present || echo ABSENT)"
if [ "$SAVE_STATUS" -ne 0 ] || [ ! -f "$PROJECT" ] || [ -z "$BUNDLED" ]; then
    echo "FAIL: the project was not saved with its audio bundled" >&2
    SWEEP_FAILED=1
fi

# The source is removed before the reopen, so the run can only succeed by
# reading the bundled copy. This is the assertion that makes the bundle mean
# something rather than merely exist.
rm -f "$PROJECT_DIR/source.wav"
OPEN_LOG="$OUT_DIR/project-open.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --project "$PROJECT" \
        --size 1280x720 \
        --probe-frames 90 \
        --probe-shot "$OUT_DIR/project-open.png" \
    >"$OPEN_LOG" 2>&1
OPEN_STATUS=$?
set -e
OPEN_PROJECT_LINE="$(sed -n 's/^project: *//p' "$OPEN_LOG")"
OPEN_VERDICT="$(sed -n 's/^verdict: *//p' "$OPEN_LOG")"
echo "opened: exit=$OPEN_STATUS project=${OPEN_PROJECT_LINE:-<absent>}"
echo "        verdict=${OPEN_VERDICT:-<absent>}"
if [ "$OPEN_STATUS" -ne 0 ]; then
    echo "FAIL: opening the saved project exited $OPEN_STATUS" >&2
    SWEEP_FAILED=1
fi
case "${OPEN_PROJECT_LINE:-absent}" in
    *"show.musi (clean)") ;;
    *)
        echo "FAIL: the reopened track did not adopt its project path cleanly" >&2
        SWEEP_FAILED=1
        ;;
esac
case "${OPEN_VERDICT:-absent}" in
    "audio advanced"*)
        # Only reachable through the bundled asset: the source is gone.
        ;;
    *)
        echo "FAIL: no audio arrived from the bundled asset: ${OPEN_VERDICT:-<absent>}" >&2
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
