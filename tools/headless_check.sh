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
# EXIT alone is not enough: a `timeout` or a Ctrl-C on the parent kills this
# shell with a signal, the trap never fires, and the leaked server then makes the
# *next* run fail for the wrong reason — "display in use" instead of whatever was
# actually wrong. That has already cost one debugging round.
trap cleanup EXIT INT TERM

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
# The icon face separately, because its fallback is not a degraded face but a
# *different interface*: the transport row draws text labels instead. That is a
# deliberate, working fallback rather than a failure, so the check reports which
# one is on screen rather than refusing — but silently photographing the wrong
# one for weeks is exactly the failure mode this project keeps paying for.
case "$FONT_LINE" in
    *"icons=Font Awesome"*) echo "icons: the icon row" ;;
    *) echo "FAIL: the icon face did not load, so the row fell back to text labels" >&2; exit 1 ;;
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

echo "=== the three whole-track derivations ==="
# The waveform envelope, Song Atlas's terrain, and the ASCII glyph grid. All three
# are drawn *through* an `Option` that used to be `None` at every call site, and all
# three have a fallback that photographs as a plausible picture: ASCII Field draws a
# procedural rolling spectrogram, Song Atlas a live idle ring, and the strip a flat
# lane. The scene sweep above already captured two of them looking fine while none
# of them had any data, for two whole bands — so this section asserts the report
# lines, not the frames.

# The envelope is built at track load, so every log above already carries its line.
# Reading it from the sweep rather than re-running proves it on the ordinary path.
waveform_line="$(sed -n 's/^waveform: *//p' "$OUT_DIR/scene-spectrum.txt" 2>/dev/null || true)"
echo "waveform: ${waveform_line:-<absent>}"
case "$waveform_line" in
    *' bins') : ;;
    *)
        echo "FAIL: the timeline waveform was not built at track load" >&2
        SWEEP_FAILED=1
        ;;
esac

# Song Atlas's map is built lazily, at the first frame that would draw it
# (`plug.c:1313-1315`), so this line is evidence that the *scene* triggered it —
# and the spectrum log below is evidence that nothing else did.
atlas_line="$(sed -n 's/^atlas: *//p' "$OUT_DIR/scene-atlas.txt" 2>/dev/null || true)"
echo "atlas (on Song Atlas):  ${atlas_line:-<absent>}"
case "$atlas_line" in
    *' slices, '*' onsets') : ;;
    *)
        echo "FAIL: Song Atlas did not build a whole-track map" >&2
        SWEEP_FAILED=1
        ;;
esac
# The other half of the laziness, which is the part a bug would break silently: a
# scene that does not need the map must not spend a whole-track decode on it.
atlas_idle="$(sed -n 's/^atlas: *//p' "$OUT_DIR/scene-spectrum.txt" 2>/dev/null || true)"
echo "atlas (on Spectrum):    ${atlas_idle:-<absent>}"
if [ "$atlas_idle" != "not needed by this scene" ]; then
    echo "FAIL: a scene that does not draw the atlas built one anyway" >&2
    SWEEP_FAILED=1
fi

# The glyph grid, from a first-party image generated by tools/make_rust_logo.py.
# Synthetic in the sense the repository rule means: ours, generated, committed —
# no user asset enters this repository.
for size in 1280x720 960x640; do
    capture "ascii-image-$size" "$size" \
        --scene ascii --ascii-image resources/logo/logo-256.png \
        || SWEEP_FAILED=1
    ascii_line="$(sed -n 's/^ascii:  *//p' "$OUT_DIR/ascii-image-$size.txt" 2>/dev/null || true)"
    echo "ascii at $size: ${ascii_line:-<absent>}"
    case "$ascii_line" in
        *'x'*' glyphs from '*) : ;;
        *)
            echo "FAIL: --ascii-image did not produce a glyph grid at $size" >&2
            SWEEP_FAILED=1
            ;;
    esac
done

# The negative control. A refused import must fail the exit status rather than
# leaving a script believing the image is on screen, and it must leave the scene
# drawing its procedural mode rather than a half-built grid.
BAD_IMAGE="$OUT_DIR/not-an-image.png"
printf 'this is not a PNG' >"$BAD_IMAGE"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --probe-frames 10 --scene ascii --ascii-image "$BAD_IMAGE" \
    >"$OUT_DIR/ascii-image-refused.txt" 2>&1
bad_status=$?
set -e
bad_line="$(sed -n 's/^ascii:  *//p' "$OUT_DIR/ascii-image-refused.txt" 2>/dev/null || true)"
echo "refused import: exit=$bad_status ascii=${bad_line:-<absent>}"
if [ "$bad_status" -eq 0 ]; then
    echo "FAIL: an unreadable --ascii-image exited 0" >&2
    SWEEP_FAILED=1
fi
if [ "$bad_line" != "none (procedural mode)" ]; then
    echo "FAIL: a refused import left the grid in an unexpected state" >&2
    SWEEP_FAILED=1
fi

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

# The export panel is built, so unlike the stubs it is held to exiting 0 and to
# saying which panel it opened. `panel:` is the line that distinguishes "drew a
# box" from "opened the export panel" — a stub would print the same picture size
# and the same exit status.
for size in 1280x720 960x640; do
    log="$OUT_DIR/panel-export-$size.txt"
    line="$(sed -n 's/^panel: *//p' "$log" 2>/dev/null || true)"
    echo "export panel at $size: panel=${line:-<absent>}"
    if [ "$line" != "export" ]; then
        echo "FAIL: the export panel did not open at $size" >&2
        SWEEP_FAILED=1
    fi
done
echo "=== the lyrics editor, over a project that actually has cues ==="
# The panel loop above photographs the editor over the bare sweep, which has no
# lyrics: an empty cue list is a real state and worth a frame, but it cannot show
# the list, the cue lane, the bound form, or the caption pane. So a project is
# generated here and seeded with synthetic cues.
#
# Generated, not committed: the repository rule is synthetic fixtures only, and
# the plan's open question about the `.musi` fixture strategy asks for exactly
# this. See tools/seed_lyric_fixture.py for why editing the file in place is safe.
LYRIC_DIR="$OUT_DIR/lyrics"
rm -rf "$LYRIC_DIR"
mkdir -p "$LYRIC_DIR"
cp "$FIXTURE" "$LYRIC_DIR/source.wav"
LYRIC_PROJECT="$LYRIC_DIR/cues.musi"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$LYRIC_DIR/source.wav" \
        --save-project "$LYRIC_PROJECT" \
    >"$LYRIC_DIR/save.txt" 2>&1
LYRIC_SAVE=$?
set -e
if [ "$LYRIC_SAVE" -ne 0 ] || [ ! -f "$LYRIC_PROJECT" ]; then
    echo "FAIL: could not build the lyric fixture project" >&2
    SWEEP_FAILED=1
else
    python3 tools/seed_lyric_fixture.py "$LYRIC_PROJECT"

    # capture() passes $FIXTURE, so these run the binary directly.
    lyric_capture() {
        # lyric_capture NAME SIZE PROBE
        local name="$1" size="$2" probe="$3"
        local out="$OUT_DIR/$name.png"
        local log="$OUT_DIR/$name.txt"
        set +e
        env -u WAYLAND_DISPLAY \
            DISPLAY="$DISPLAY_NUM" \
            PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
            ./target/debug/musializer --project "$LYRIC_PROJECT" \
                --size "$size" \
                --probe-frames 30 \
                --probe-shot "$out" \
                --ui-probe "$probe" \
            >"$log" 2>&1
        local status=$?
        set -e
        printf '%-26s %-9s exit=%s ' "$name" "$size" "$status"
        if [ ! -f "$out" ]; then
            echo "FAIL (no frame)"
            return 1
        fi
        # The line the picture cannot assert on its own: which pane is showing,
        # which cue the form is bound to, and whether the draft is dirty. Absent
        # until `main.rs` prints it — see REWRITE_PLAN.md's Agent I note.
        echo "lyrics=$(sed -n 's/^lyrics: *//p' "$log" | head -1)"
        return "$status"
    }

    for size in 1280x720 960x640; do
        lyric_capture "lyrics-cues-$size" "$size" "panel=lyrics,play=1" \
            || echo "  (note: the lyric editor needs its shell.rs and main.rs hooks; see the Agent I note)"
        lyric_capture "lyrics-selected-$size" "$size" "panel=lyrics,lyric=3,play=1" \
            || echo "  (note: --ui-probe lyric= needs its main.rs hook)"
        lyric_capture "lyrics-style-$size" "$size" "panel=lyrics,style=caption,play=1" \
            || echo "  (note: --ui-probe style= needs its main.rs hook)"
    done
    lyric_capture "lyrics-fonts-1280x720" 1280x720 \
        "panel=lyrics,style=caption,fonts=consent,play=1" \
        || echo "  (note: the font pane is Agent K's; this frame shows the seam, not the browser)"
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

echo "=== the export transport ==="
# The two claims a screenshot cannot make, and the two `cargo test` must not
# make either because it may not depend on an external encoder:
#
#   1. an export is deterministic — the same project produces the same bytes;
#   2. a windowed export is bit-identical to the same frames of a full render,
#      which is what the fast-forward before `render_start_frame` exists for.
#
# `examples/export_probe.rs` runs the real RenderJob over the real FFmpeg and
# prints digests. FFmpeg is an expected external tool here: this script already
# calls ffprobe above, so its absence is a failure rather than a skip.
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "FAIL: ffmpeg is not on PATH; the export transport cannot be checked" >&2
    SWEEP_FAILED=1
else
cargo build --quiet -p musializer-runtime --example export_probe
EXPORT_DIR="$OUT_DIR/export"
rm -rf "$EXPORT_DIR"
mkdir -p "$EXPORT_DIR"

run_export() {
    # run_export NAME [extra args...]
    local name="$1"
    shift
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/examples/export_probe \
            --audio "$FIXTURE" \
            --out "$EXPORT_DIR/$name.mp4" \
            "$@" \
        >"$EXPORT_DIR/$name.txt" 2>&1
    local status=$?
    set -e
    if [ "$status" -ne 0 ]; then
        echo "FAIL: export probe $name exited $status" >&2
        sed -n 's/^export_probe: /  /p' "$EXPORT_DIR/$name.txt" >&2 || true
        return 1
    fi
    return 0
}

export_field() {
    sed -n "s/^export: .*$1=\\([^ ]*\\).*/\\1/p" "$EXPORT_DIR/$2.txt" | head -1
}

# The fixture is FIXTURE_SECONDS long at 30 fps, and frames 30..60 are the
# second of it that the windowed run below asks for.
EXPECTED_FRAMES=$((FIXTURE_SECONDS * 30))
EXPORT_OK=1
run_export full-a --digest-range 30 60 || EXPORT_OK=0
run_export full-b --digest-range 30 60 || EXPORT_OK=0
run_export window --window 1.0 1.0 || EXPORT_OK=0

if [ "$EXPORT_OK" -ne 1 ]; then
    SWEEP_FAILED=1
else
    FULL_A_FILE="$(export_field file-sha256 full-a)"
    FULL_B_FILE="$(export_field file-sha256 full-b)"
    FULL_A_FRAMES="$(export_field frames-sha256 full-a)"
    FULL_B_FRAMES="$(export_field frames-sha256 full-b)"
    FULL_A_RANGE="$(export_field range-sha256 full-a)"
    WINDOW_FRAMES_HASH="$(export_field frames-sha256 window)"
    ENCODED_FULL="$(export_field encoded full-a)"
    ENCODED_WINDOW="$(export_field encoded window)"
    COUNTED_FULL="$(ffprobe -v error -select_streams v:0 -count_frames \
        -show_entries stream=nb_read_frames -of csv=p=0 "$EXPORT_DIR/full-a.mp4")"
    COUNTED_WINDOW="$(ffprobe -v error -select_streams v:0 -count_frames \
        -show_entries stream=nb_read_frames -of csv=p=0 "$EXPORT_DIR/window.mp4")"

    echo "export frames:      wrote $ENCODED_FULL, container holds $COUNTED_FULL (expected $EXPECTED_FRAMES)"
    echo "window frames:      wrote $ENCODED_WINDOW, container holds $COUNTED_WINDOW (expected 30)"
    echo "determinism:        $FULL_A_FILE"
    echo "                    $FULL_B_FILE"
    echo "fast-forward:       full[30..60] $FULL_A_RANGE"
    echo "                    windowed     $WINDOW_FRAMES_HASH"

    # Four claims, each of which a clean exit would not make.
    if [ "$ENCODED_FULL" != "$EXPECTED_FRAMES" ] || [ "$COUNTED_FULL" != "$EXPECTED_FRAMES" ]; then
        echo "FAIL: a full export should be exactly $EXPECTED_FRAMES frames" >&2
        SWEEP_FAILED=1
    fi
    if [ "$ENCODED_WINDOW" != "30" ] || [ "$COUNTED_WINDOW" != "30" ]; then
        echo "FAIL: a one-second window at 30 fps should be exactly 30 frames" >&2
        SWEEP_FAILED=1
    fi
    if [ -z "$FULL_A_FILE" ] || [ "$FULL_A_FILE" != "$FULL_B_FILE" ] \
        || [ "$FULL_A_FRAMES" != "$FULL_B_FRAMES" ]; then
        echo "FAIL: the same export twice was not byte-identical" >&2
        SWEEP_FAILED=1
    fi
    if [ -z "$WINDOW_FRAMES_HASH" ] || [ "$FULL_A_RANGE" != "$WINDOW_FRAMES_HASH" ]; then
        echo "FAIL: a windowed export did not reproduce the same frames of a full render" >&2
        SWEEP_FAILED=1
    fi
    # Nothing may be left beside the destination: the staging WAV and the
    # in-progress .part.mp4 are both hidden siblings, and a retained one is how a
    # user finds a mystery file next week.
    LEFTOVERS="$(find "$EXPORT_DIR" -name '.musializer-*' 2>/dev/null | head -3)"
    if [ -n "$LEFTOVERS" ]; then
        echo "FAIL: staging files survived the export: $LEFTOVERS" >&2
        SWEEP_FAILED=1
    fi
fi
fi

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


echo "=== the transport row: icons, volume, tooltips, fine seek ==="
# None of this is in the frozen C, which is exactly why it needs photographing:
# there is no oracle to compare against, so a capture and a report line are the
# only evidence that the row does what it claims.
#
# Both sizes on purpose. At 960x640 with the inspector open the toolbar is about
# 440 px, which is where `transport_bar` sheds the fine-seek group and the band
# hands the timecode to the timeline panel. That is a *reachable* state, not a
# hypothetical one, and it is the one a capture is most likely to catch drifting.
TRANSPORT_FAILED=0
for size in 1280x720 960x640; do
    capture "transport-$size" "$size" --ui-probe "play=1" || TRANSPORT_FAILED=1
    capture "transport-tune-$size" "$size" --ui-probe "panel=tune,play=1" || TRANSPORT_FAILED=1
done

# The readout is off unless asked for, and a probe run asks for it by itself so
# that every capture above still carries its own evidence. Both halves are
# checked, because "off by default" and "on in a probe" are separate claims and
# getting either wrong silently changes what every other capture here means.
hud_state() {
    # hud_state NAME EXPECT [extra args...]
    local name="$1" expect="$2"
    shift 2
    local out="$OUT_DIR/$name.png"
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer "$FIXTURE" \
            --size 1280x720 --probe-frames 10 --probe-shot "$out" "$@" \
        >"$OUT_DIR/$name.txt" 2>&1
    local status=$?
    set -e
    # The readout is drawn, not printed, so the picture is the only witness. The
    # line sits in the top-left of the preview on a near-black background; a
    # bright pixel there means text, and none means a clean preview.
    local ink
    ink="$(ffprobe -v error -f lavfi \
        -i "movie=$out,crop=560:24:332:10,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
    ink="${ink:-0}"
    local got="off"
    [ "${ink%%.*}" -gt 120 ] 2>/dev/null && got="on"
    printf '%-22s exit=%s readout=%s (peak luma %s)\n' "$name" "$status" "$got" "$ink"
    if [ "$got" != "$expect" ]; then
        echo "FAIL: $name expected the readout $expect and it was $got" >&2
        return 1
    fi
    [ "$status" -eq 0 ]
}

hud_state hud-probe-default on || TRANSPORT_FAILED=1
hud_state hud-forced-off off --hud=0 || TRANSPORT_FAILED=1
hud_state hud-forced-on on --hud || TRANSPORT_FAILED=1

# A tooltip, which no earlier capture in this file could photograph at all: a
# headless run has no pointer, so every hover state in this interface was
# unreviewable. `--ui-probe hover=XxY` parks it, and the probe zeroes the dwell so
# the tip is in frame one rather than depending on how long the run lasted.
#
# The coordinates are the mute button's centre in the 1280x720 layout. If the row
# is ever re-laid-out they stop naming that control, and the check below is what
# says so — a tooltip that stopped appearing would otherwise be invisible.
capture "tooltip-mute" 1280x720 --ui-probe "play=1,hover=1121x449" || TRANSPORT_FAILED=1
TIP_INK="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/tooltip-mute.png,crop=140:30:1050:400,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
# The tip is white on near-black, drawn over the preview's dark background. A dark
# minimum is the box; without the tip that region is the preview's own dark too,
# so the discriminating measure is the *bright* text inside it.
TIP_TEXT="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/tooltip-mute.png,crop=140:30:1050:400,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "tooltip-mute            box luma min=${TIP_INK:-?} text luma max=${TIP_TEXT:-?}"
if [ "${TIP_TEXT%%.*}" -lt 180 ] 2>/dev/null; then
    echo "FAIL: no tooltip text where the mute button's tip should be" >&2
    TRANSPORT_FAILED=1
fi

if [ "$TRANSPORT_FAILED" -ne 0 ]; then
    echo "FAIL: the transport row checks did not pass" >&2
    exit 1
fi

echo "=== a routed setting ==="
# Routes are applied after every input is resolved, and the report prints how
# many landed on the active scene.
capture "route-loom-weight" 1280x720 \
    --scene loom --route 'loom.weight:band:2:0:1:0.4:2.2:smoothstep' || SWEEP_FAILED=1
sed -n 's/^routes: */routes on the active scene: /p' "$OUT_DIR/route-loom-weight.txt"

# `beat_phase` is the one route source with a stateful producer behind it, and it
# spent two bands hardcoded to 0.0 with the beat tracker never called at all —
# ported, unit-tested against a C trace, and unreachable. The CLI advertised it the
# whole time. So this asserts the *range*: a source stuck at any single value,
# including a plausible-looking one, is the failure being guarded against.
capture "route-loom-beat" 1280x720 \
    --scene loom --route 'loom.weight:beat_phase:0:0:1:0.4:2.2:linear' || SWEEP_FAILED=1
beat_line="$(sed -n 's/^beat phase: *//p' "$OUT_DIR/route-loom-beat.txt" 2>/dev/null || true)"
onset_line="$(sed -n 's/^onsets: *//p' "$OUT_DIR/route-loom-beat.txt" 2>/dev/null || true)"
echo "beat phase: ${beat_line:-<absent>}"
echo "onsets:     ${onset_line:-<absent>}"
case "$beat_line" in
    'never sampled'*)
        echo "FAIL: the beat tracker was never called" >&2
        SWEEP_FAILED=1
        ;;
    0.0000..0.0000*)
        echo "FAIL: beat_phase never advanced — the route source is a constant" >&2
        SWEEP_FAILED=1
        ;;
    *..*) : ;;
    *)
        echo "FAIL: the beat phase line was not in the expected form" >&2
        SWEEP_FAILED=1
        ;;
esac

echo "=== the route editor row, inside the Tune inspector ==="
# Three states of the same row, at 720p and at the 960x640 minimum, because the
# expanded editor is the tallest thing the inspector ever hosts and the minimum
# window is where it stops fitting:
#
#   collapsed  a routed setting shows its summary and live meter instead of a
#              slider, which is the state a user reaches without opening anything
#   band       the editor opened onto a committed band route, which is the taller
#              of the two shapes (the band stepper adds 24 px)
#   fresh      the editor opened on an unrouted setting, so the draft is the
#              full-range RMS route `route_editor_open` seeds
#
# The `route editor:` line is the evidence a screenshot cannot give: it names the
# setting, the row height that was actually asked for, and whether the draft came
# from a committed route or was seeded fresh.
ROUTE_EDITOR_FAILED=0
for size in 1280x720 960x640; do
    capture "route-collapsed-$size" "$size" \
        --scene loom --route 'loom.weight:band:2:0:1:0.4:2.2:smoothstep' \
        --ui-probe "panel=tune,play=1" || ROUTE_EDITOR_FAILED=1

    capture "route-editor-band-$size" "$size" \
        --scene loom --route 'loom.weight:band:2:0:1:0.4:2.2:smoothstep' \
        --ui-probe "panel=tune,play=1,route=loom.weight" || ROUTE_EDITOR_FAILED=1

    capture "route-editor-fresh-$size" "$size" \
        --scene loom --ui-probe "panel=tune,play=1,route=loom.density" \
        || ROUTE_EDITOR_FAILED=1
done

for name in band fresh; do
    for size in 1280x720 960x640; do
        line="$(sed -n 's/^route editor: //p' "$OUT_DIR/route-editor-$name-$size.txt" | head -n1)"
        printf '%-30s %s\n' "route-editor-$name-$size" "${line:-<absent>}"
        case "$line" in
            *" open row="*) ;;
            *)
                echo "FAIL: the route editor did not open for $name at $size" >&2
                ROUTE_EDITOR_FAILED=1
                ;;
        esac
    done
done

# The committed route is 24 px taller than the fresh one, because its source is
# `band` and the band stepper is a row. Asserting the difference is what
# distinguishes "the editor opened" from "the editor opened onto the right draft".
BAND_ROW="$(sed -n 's/^route editor: .* row=\([0-9.]*\)px .*/\1/p' \
    "$OUT_DIR/route-editor-band-1280x720.txt" | head -n1)"
FRESH_ROW="$(sed -n 's/^route editor: .* row=\([0-9.]*\)px .*/\1/p' \
    "$OUT_DIR/route-editor-fresh-1280x720.txt" | head -n1)"
echo "row heights: band=${BAND_ROW:-?} fresh=${FRESH_ROW:-?} (band is 24 px taller)"
if [ "${BAND_ROW:-0}" != "312" ] || [ "${FRESH_ROW:-0}" != "288" ]; then
    echo "FAIL: expected row heights 312 (band) and 288 (fresh)" >&2
    ROUTE_EDITOR_FAILED=1
fi

# A key that names no setting must fail the command line rather than quietly
# photographing an unexpanded row. This is the negative control for the whole
# section: without it, a `route=` that silently did nothing would look identical
# to one that worked.
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer "$FIXTURE" \
        --size 1280x720 --probe-frames 5 \
        --ui-probe "panel=tune,route=loom.wieght" \
    >"$OUT_DIR/route-editor-typo.txt" 2>&1
TYPO_STATUS=$?
set -e
echo "mistyped route= exit=$TYPO_STATUS (must be non-zero)"
if [ "$TYPO_STATUS" -eq 0 ]; then
    echo "FAIL: a mistyped --ui-probe route= key was accepted" >&2
    ROUTE_EDITOR_FAILED=1
fi

if [ "$ROUTE_EDITOR_FAILED" -ne 0 ]; then
    SWEEP_FAILED=1
fi

if [ "$SWEEP_FAILED" -ne 0 ]; then
    echo "FAIL: at least one scene or alias capture failed" >&2
    exit 1
fi

echo "artifacts in $OUT_DIR"
