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
#   - every application launch also passes --mute, which leaves decoded/analyzer
#     PCM intact while forcing raylib's process-local master output to zero;
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
# Large enough to exercise the 1440p scale rung. Individual windows still use
# their requested size, so the existing 720p/minimum captures stay comparable.
SCREEN_SIZE="2560x1440x24"

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
    ./target/debug/musializer --mute "$FIXTURE" \
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
case "$FONT_LINE" in
    *"ui=Space Grotesk (17 native sizes)"*"non-native-requests=0"*) : ;;
    *)
        echo "FAIL: the shell did not use the complete native-size UI font bank: $FONT_LINE" >&2
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

# Analyzer-ring drops and audible output starvation are different queues. The
# ordinary run must report neither, and the negative control below deliberately
# stalls only the main/refill thread long enough to drain raylib's streaming
# buffer. If the output counter stays at zero there, the instrumentation is not
# measuring the silence-producing path it claims to measure.
BASELINE_UNDERRUNS="$(sed -n 's/^output underruns: *//p' "$REPORT")"
echo "output underruns: ${BASELINE_UNDERRUNS:-<absent>}"
if [ "${BASELINE_UNDERRUNS:-missing}" != "0" ]; then
    echo "FAIL: ordinary playback reported output starvation" >&2
    exit 1
fi

echo "=== output-underrun negative control ==="
UNDERRUN_REPORT="$OUT_DIR/output-underrun-negative-control.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" \
        --size 960x640 \
        --probe-frames 120 \
        --ui-probe "play=1,audio-stall=750" \
    >"$UNDERRUN_REPORT" 2>&1
UNDERRUN_STATUS=$?
set -e
if [ "$UNDERRUN_STATUS" -ne 0 ]; then
    echo "FAIL: output-underrun negative control exited $UNDERRUN_STATUS" >&2
    cat "$UNDERRUN_REPORT"
    exit "$UNDERRUN_STATUS"
fi
NEGATIVE_UNDERRUNS="$(sed -n 's/^output underruns: *//p' "$UNDERRUN_REPORT")"
echo "negative-control underruns: ${NEGATIVE_UNDERRUNS:-<absent>}"
case "${NEGATIVE_UNDERRUNS:-missing}" in
    ''|*[!0-9]*|0)
        echo "FAIL: the forced output starvation was not detected" >&2
        cat "$UNDERRUN_REPORT"
        exit 1
        ;;
esac

# The stall band that leaves every other counter at zero (EX4).
#
# A stall between roughly 85 ms — the 4096-frame scratch buffer — and 170 ms —
# the 8192-frame ring and the raised output stream buffer — desynchronizes the
# picture from the music while `output underruns:` and the ring's `dropped`
# count both stay at 0. That is the band the operator's "rendering sometimes
# introduces little hiccups" lives in, and until now nothing in this report
# could distinguish it from a clean run. `peak ring fill` is the figure that
# can, and `frame budget:` names the frame it happened on.
#
# 120 ms is chosen to sit *inside* the band on purpose: the assertions below
# require `dropped=0` and `underruns=0`, so this fails if the new lines ever
# stop being the only ones that fire.
echo "=== the silent stall band ==="
STALL_REPORT="$OUT_DIR/frame-stall-negative-control.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" \
        --size 960x640 \
        --probe-frames 120 \
        --ui-probe "play=1,audio-stall=120" \
    >"$STALL_REPORT" 2>&1
STALL_STATUS=$?
set -e
if [ "$STALL_STATUS" -ne 0 ]; then
    echo "FAIL: the frame-stall control exited $STALL_STATUS" >&2
    cat "$STALL_REPORT"
    exit "$STALL_STATUS"
fi
STALL_BUDGET="$(sed -n 's/^frame budget: *//p' "$STALL_REPORT")"
STALL_AUDIO="$(sed -n 's/^audio frames: *//p' "$STALL_REPORT")"
STALL_UNDERRUNS="$(sed -n 's/^output underruns: *//p' "$STALL_REPORT")"
echo "stalled run:  $STALL_BUDGET"
echo "stalled run:  $STALL_AUDIO"
case "$STALL_BUDGET" in
    *"0 of 120 stalled"*|'')
        echo "FAIL: a forced 120 ms stall was not counted by 'frame budget:'" >&2
        exit 1
        ;;
esac
case "$STALL_AUDIO" in
    *"peak ring fill 0%"*|*"peak ring fill 1"[0-9]"%"|'')
        echo "FAIL: a forced 120 ms stall did not move 'peak ring fill'" >&2
        exit 1
        ;;
esac
# The half that makes the line worth having: at 120 ms the counters that already
# existed are still clean, so this run is indistinguishable from a healthy one
# without the two new figures.
case "$STALL_AUDIO" in
    *"0 dropped"*) : ;;
    *) echo "FAIL: 120 ms was outside the silent band (frames were dropped)" >&2; exit 1 ;;
esac
if [ "${STALL_UNDERRUNS:-1}" != "0" ]; then
    echo "FAIL: 120 ms was outside the silent band (underruns=$STALL_UNDERRUNS)" >&2
    exit 1
fi
# And a clean run must read clean, or the two lines are noise rather than signal.
CLEAN_BUDGET="$(sed -n 's/^frame budget: *//p' "$REPORT")"
echo "clean run:    ${CLEAN_BUDGET:-<absent>}"
case "${CLEAN_BUDGET:-absent}" in
    *"0 of "*" stalled"*) : ;;
    *) echo "FAIL: the ordinary run reported a stall: ${CLEAN_BUDGET:-<absent>}" >&2; exit 1 ;;
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
        ./target/debug/musializer --mute "$FIXTURE" \
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
    local underruns
    underruns="$(sed -n 's/^output underruns: *//p' "$log")"
    echo "scene=${verdict:-?} drawing=${drawing:-?} underruns=${underruns:-?}"
    [ "$status" -eq 0 ] || return 1
    [ "${underruns:-missing}" = "0" ] || return 1
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

# The synthetic track's startup transient crosses Spectrum's onset threshold by
# frame eight. The removed renderer pass used that boolean to paint a full-width
# white gradient across the reflection floor — the short flash reported in both
# preview and exports. Pin a quiet region far from the active bands. Negative
# control: restoring the old pass raises this crop's YMAX from 21 to 55.
#
# Pin the split on this capture: per-user timeline preferences are intentionally
# honoured elsewhere in the sweep, but letting one move a renderer negative
# control made this crop sample white timeline chrome instead of the scene.
#
# The requested height must be a height the band can actually take, because
# `Shell::resolved_timeline_height` clamps it up to
# `SCENE_SECTION_HEIGHT + EVENT_ROW_HEIGHT + 150` and a *silently clamped*
# request moves the split without moving this comment. 276 is that floor today
# (60 + 66 + 150); it was 270 until LX1-e spent a lane gap inside the scene
# section, and the six pixels the band gained put white chrome inside the old
# y=386 crop. To re-derive after a band constant moves: the preview's last row
# is `720 - band - 50`, so the crop is the 12 px ending two rows above it.
capture "spectrum-onset-floor" 1280x720 --scene spectrum --probe-frames 8 \
    --ui-probe "play=1,timeline-height=276" \
    || SWEEP_FAILED=1
SPECTRUM_ONSETS="$(sed -n 's/^onsets: *\([0-9][0-9]*\) .*/\1/p' "$OUT_DIR/spectrum-onset-floor.txt")"
SPECTRUM_FLOOR="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/spectrum-onset-floor.png,crop=200:12:900:380,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "spectrum onset floor: onsets=${SPECTRUM_ONSETS:-?} peak-luma=${SPECTRUM_FLOOR:-?}"
if [ "${SPECTRUM_ONSETS:-0}" -eq 0 ] 2>/dev/null \
    || [ "${SPECTRUM_FLOOR%%.*}" -gt 30 ] 2>/dev/null; then
    echo "FAIL: Spectrum's onset frame brightened the reflection floor" >&2
    SWEEP_FAILED=1
fi

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
    ./target/debug/musializer --mute "$FIXTURE" \
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

echo "=== scaled and user-sized workspace ==="
capture "ui-scale-125-1600x900" 1600x900 --ui-scale 125 \
    --ui-probe "panel=tune,play=1" || SWEEP_FAILED=1
capture "ui-scale-150-2560x1440" 2560x1440 --ui-scale 150 \
    --ui-probe "panel=tune,play=1" || SWEEP_FAILED=1
capture "ui-auto-2560x1440" 2560x1440 \
    --ui-probe "panel=tune,play=1" || SWEEP_FAILED=1
capture "ui-splits-1920x1080" 1920x1080 --ui-scale 125 \
    --ui-probe "panel=tune,play=1,sidebar=400,inspector=440,timeline-height=330" \
    || SWEEP_FAILED=1

scale_125="$(sed -n 's/^ui layout: *//p' "$OUT_DIR/ui-scale-125-1600x900.txt")"
scale_150="$(sed -n 's/^ui layout: *//p' "$OUT_DIR/ui-scale-150-2560x1440.txt")"
scale_auto="$(sed -n 's/^ui layout: *//p' "$OUT_DIR/ui-auto-2560x1440.txt")"
split_layout="$(sed -n 's/^ui layout: *//p' "$OUT_DIR/ui-splits-1920x1080.txt")"
echo "125% layout: ${scale_125:-<absent>}"
echo "150% layout: ${scale_150:-<absent>}"
echo "1440p Auto: ${scale_auto:-<absent>}"
echo "user splits: ${split_layout:-<absent>}"
case "$scale_125" in scale=125*) : ;; *) SWEEP_FAILED=1; echo "FAIL: 125% scale was not active" >&2 ;; esac
case "$scale_150" in scale=150*) : ;; *) SWEEP_FAILED=1; echo "FAIL: 150% scale was not active" >&2 ;; esac
case "$scale_auto" in scale=150*) : ;; *) SWEEP_FAILED=1; echo "FAIL: 1440p Auto did not select 150%" >&2 ;; esac
case "$split_layout" in
    *"sidebar=400"*"inspector=440"*"timeline=330"*) : ;;
    *) SWEEP_FAILED=1; echo "FAIL: the requested workspace splits were not active" >&2 ;;
esac

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

# review 1.4: `panel: export` only proves the panel object was told to open —
# it is the same line whether the panel drew its full body, its "too small"
# notice, or nothing at all, and it passed while both shipped captures were a
# blank white band. This measures the pixels instead, the same technique the
# tooltip gate below uses: crop where the panel's own text has to land, then
# read peak/trough luma via ffprobe's signalstats.
#
# `Shell::export_panel`'s content box always ends exactly `UI_PANEL_PADDING`
# (10 px) above the window's bottom edge, however tall the band above it is —
# so its position only needs deriving once per window height, not per band
# state. The Export band is now derived end-to-end from
# `export.rs::EXPORT_MIN_BAND_HEIGHT` (through both the automatic budget in
# `shell_layout.rs` and the persisted-split floor in `shell.rs`), so the body
# this crop lands on is the full control rows; the "EXPORT" header line sits
# within a few pixels of the same spot either way, so the crop also catches a
# regression back to the notice-or-blank states.
declare -A EXPORT_INK_CROP_Y=( [1280x720]=474 [960x640]=394 )
for size in 1280x720 960x640; do
    png="$OUT_DIR/panel-export-$size.png"
    crop_y="${EXPORT_INK_CROP_Y[$size]}"
    EXPORT_INK="$(ffprobe -v error -f lavfi \
        -i "movie=$png,crop=340:90:10:$crop_y,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
    echo "export panel ink at $size: darkest pixel=${EXPORT_INK:-<absent>} (blank fill reads ~247)"
    # 200 sits well below the panel's own near-white fill (`ui_surface`,
    # ~247) and well above every colour this panel actually draws with —
    # accent text, warning text, muted labels, button borders — so this is
    # the measurement the report line could not make.
    if [ -z "${EXPORT_INK:-}" ] || [ "${EXPORT_INK%%.*}" -gt 200 ] 2>/dev/null; then
        echo "FAIL: the export panel drew no ink at $size — panel: export but the box is blank" >&2
        SWEEP_FAILED=1
    fi
done

# The export panel's controls, pressed rather than looked at (EX1).
#
# Every check above this point asks whether the row was *drawn*, and the row was
# always drawn. Three of its four SIZE buttons shipped unable to take a click for
# as long as the panel has existed, because `panels/events.rs` had minted the
# namespace `7` as a bare literal and `7` is `widgets::id::EXPORT`: the manual
# event row draws first, so `+ Feel`, `+ Scene` and `+ Custom` claimed ids 0, 1
# and 2 and cashed the release before 720p, 1080p and 1440p were drawn. 2160p is
# index 3, has no counterpart in that row, and worked — which is why the symptom
# reached the operator as "I can't pick different sizes" rather than as a dead
# panel, and why a capture of any single state looked correct.
#
# `hover=` could not have caught it. The hover highlight is computed from the
# same `contains_point` that was never the problem, and it lit up correctly at
# both scales. Only a press separates "this control is under the pointer" from
# "this control receives the press", so the check is the press.
#
# Coordinates are the button centres at 1280x720 from `panel-export-1280x720.png`
# above, and each run asserts the *whole* configuration line, so a click that
# lands on a neighbour fails rather than passing on a partial match.
echo "=== the export panel takes a click ==="
click_export() {
    # click_export NAME POINT EXPECTED-CONFIG
    capture "click-export-$1" 1280x720 --ui-probe "panel=export,click=$2" || SWEEP_FAILED=1
    local got claimed
    got="$(sed -n 's/^export config: *//p' "$OUT_DIR/click-export-$1.txt")"
    claimed="$(sed -n 's/^click probe: *//p' "$OUT_DIR/click-export-$1.txt")"
    echo "click $1 at $2: [$got] via $claimed"
    if [ "$got" != "$3" ]; then
        echo "FAIL: clicking $1 gave [$got], expected [$3]" >&2
        SWEEP_FAILED=1
    fi
    case "$claimed" in
        *"claimed=nothing"*)
            echo "FAIL: the $1 click reached no control at all" >&2
            SWEEP_FAILED=1
            ;;
    esac
}
click_export 720p    116x542 "1280x720 at 30 fps, High, supersample 2x"
click_export 1440p   284x542 "2560x1440 at 30 fps, High, supersample 2x"
click_export 2160p   368x542 "3840x2160 at 30 fps, High, supersample 2x"
click_export 24fps   498x542 "1920x1080 at 24 fps, High, supersample 2x"
click_export master  377x589 "1920x1080 at 30 fps, Master, supersample 2x"
# The control-free gap between two SIZE buttons, so the gate proves it can tell a
# press that lands from one that does not. Without this row, a probe that
# silently clicked nothing would satisfy every assertion above by leaving the
# default configuration in place.
capture "click-export-gap" 1280x720 --ui-probe "panel=export,click=158x542" || SWEEP_FAILED=1
GAP_CLAIM="$(sed -n 's/^click probe: *//p' "$OUT_DIR/click-export-gap.txt")"
GAP_CONFIG="$(sed -n 's/^export config: *//p' "$OUT_DIR/click-export-gap.txt")"
echo "click gap at 158x542: [$GAP_CONFIG] via $GAP_CLAIM"
case "$GAP_CLAIM" in
    *"claimed=nothing"*) : ;;
    *)
        echo "FAIL: a click in the gap between two buttons claimed $GAP_CLAIM" >&2
        SWEEP_FAILED=1
        ;;
esac
if [ "$GAP_CONFIG" != "1920x1080 at 30 fps, High, supersample 2x" ]; then
    echo "FAIL: a click in the gap changed the export configuration to [$GAP_CONFIG]" >&2
    SWEEP_FAILED=1
fi

# The ASPECT row, and the preview framing that goes with it (EX2).
#
# The buttons are pressed rather than photographed, for the reason EX1 exists:
# the row draws identically whether or not its presses land. And the preview is
# checked through `preview frame:` rather than by pixel, because a scene framed
# to a 9:16 export inside a wide panel and a scene that simply failed to fill
# that panel are the same picture — which is exactly what ASCII Field's fixed
# grid looked like before this tranche.
echo "=== the export aspect row, and the framed preview ==="
click_export 9x16  605x589 "1080x1920 at 30 fps, High, supersample 2x"
click_export 1x1   675x589 "1080x1080 at 30 fps, High, supersample 2x"
click_export 4x5   745x589 "1080x1350 at 30 fps, High, supersample 2x"
# A rung change must keep the shape. This is the whole reason the two rows are
# one configuration rather than eight independent buttons.
capture "click-export-tall-2160p" 1280x720 \
    --ui-probe "panel=export,click=605x589" >/dev/null 2>&1 || true
TALL_THEN_2160="$(env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" --size 1280x720 --resolution 1080x1920 \
        --probe-frames 30 --ui-probe "panel=export,click=368x542" 2>&1 \
    | sed -n 's/^export config: *//p')"
echo "vertical then 2160p: [$TALL_THEN_2160]"
if [ "$TALL_THEN_2160" != "2160x3840 at 30 fps, High, supersample 2x" ]; then
    echo "FAIL: pressing 2160p on a vertical export did not stay vertical" >&2
    SWEEP_FAILED=1
fi

# The preview reports the rect the scene was drawn into, and its shape has to be
# the *export's*, not the panel's. Checked as a ratio rather than as pinned
# pixels so a layout change moves the panel without breaking this.
check_preview_frame() {
    # check_preview_frame NAME RESOLUTION EXPECTED-RATIO
    local name="$1" geometry="$2" expected="$3"
    env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer --mute "$FIXTURE" --size 1600x1000 \
            --resolution "$geometry" --probe-frames 20 --ui-probe "panel=none" \
        >"$OUT_DIR/preview-frame-$name.txt" 2>&1
    local line
    line="$(sed -n 's/^preview frame: *//p' "$OUT_DIR/preview-frame-$name.txt")"
    echo "preview at $geometry: ${line:-<absent>}"
    if [ -z "$line" ]; then
        echo "FAIL: no 'preview frame:' line for $geometry" >&2
        SWEEP_FAILED=1
        return
    fi
    # "panel WxH, framed WxH (kind)" -> the framed pair. Python rather than awk
    # because the three-argument `match()` is a gawk extension and this machine's
    # /usr/bin/awk is mawk, which fails with a syntax error rather than a wrong
    # answer — but only at the point the check runs, which is late.
    python3 - "$line" "$expected" "$geometry" <<'PYEOF' || SWEEP_FAILED=1
import re, sys
line, want, geometry = sys.argv[1], float(sys.argv[2]), sys.argv[3]
found = re.search(r"framed (\d+)x(\d+)", line)
if not found:
    print(f"FAIL: could not read the framed rect from: {line}", file=sys.stderr)
    sys.exit(1)
width, height = int(found.group(1)), int(found.group(2))
ratio = width / height
if not (want * 0.99 <= ratio <= want * 1.01):
    print(
        f"FAIL: {geometry} framed at {width}x{height} (ratio {ratio:.4f}), "
        f"wanted {want:.4f}",
        file=sys.stderr,
    )
    sys.exit(1)
PYEOF
}
check_preview_frame wide 1920x1080 1.7778
check_preview_frame tall 1080x1920 0.5625
check_preview_frame square 1080x1080 1.0

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
    ./target/debug/musializer --mute "$LYRIC_DIR/source.wav" \
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
            ./target/debug/musializer --mute --project "$LYRIC_PROJECT" \
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
        # until `main.rs` prints it, so the report is asserted as well as the PNG.
        local report
        report=$(sed -n 's/^lyrics: *//p' "$log" | head -1)
        echo "lyrics=$report"
        # Review 1.5 / 4.3: the seeded fixture is deliberately Greek+Cyrillic,
        # and for weeks the editor drew it as rows of `?` while every capture
        # passed. `missing=` is a direct count of glyphs the serving face could
        # not draw for the strings actually on screen — an inference-free
        # assertion where a question-mark-shaped pixel heuristic would guess.
        case "$report" in
            *"missing=0"*) ;;
            *)
                echo "FAIL: $name served authored text through a face missing glyphs — $report" >&2
                return 1
                ;;
        esac
        # `face=none` is a frame that drew no authored text at all (the style
        # and fonts panes) — honest, and not the defect. The defect is authored
        # text served by the Latin-only chrome bank or raylib's default.
        case "$report" in
            *"face=ui"* | *"face=default"*)
                echo "FAIL: $name drew cue text with the wrong face — $report" >&2
                return 1
                ;;
        esac
        return "$status"
    }

    for size in 1280x720 960x640; do
        lyric_capture "lyrics-cues-$size" "$size" "panel=lyrics,play=1" || SWEEP_FAILED=1
        lyric_capture "lyrics-selected-$size" "$size" "panel=lyrics,lyric=3,play=1" || SWEEP_FAILED=1
        lyric_capture "lyrics-style-$size" "$size" "panel=lyrics,style=caption,play=1" || SWEEP_FAILED=1
        # The free colour picker, which a click is otherwise the only way to
        # open. 960x640 is the size that matters twice over: it is the minimum
        # supported window, and it is the one whose caption form was 165 px —
        # inside the [152, 184) band where "Import a face..." was positioned and
        # then silently not drawn.
        lyric_capture "lyrics-picker-ink-$size" "$size" "panel=lyrics,picker=ink,play=1" \
            || SWEEP_FAILED=1
    done
    lyric_capture "lyrics-picker-plate-960x640" 960x640 \
        "panel=lyrics,picker=plate,play=1" || SWEEP_FAILED=1
    # The caption effects form and the glow colour picker inside it
    # (post-legacy, 2026-08-03). 960x640 for the same reason as the pickers:
    # the minimum window is where a control positioned-but-not-fitting hides.
    lyric_capture "lyrics-effects-960x640" 960x640 \
        "panel=lyrics,style=effects,play=1" || SWEEP_FAILED=1
    lyric_capture "lyrics-picker-glow-960x640" 960x640 \
        "panel=lyrics,picker=glow,play=1" || SWEEP_FAILED=1
    # The drive tuning editor (UX0-C14): a disclosure behind the effects form's
    # Tune buttons, so without its own probe it would join the list of surfaces
    # that shipped unphotographed.
    lyric_capture "lyrics-tune-pulse-960x640" 960x640 \
        "panel=lyrics,tune=pulse,play=1" || SWEEP_FAILED=1
    # One caption tooltip actually in a frame (UX0-C16). Probe runs suppress
    # tips unless `hover=` asks for one, so without this capture every tip the
    # caption panes gained would be unphotographed — the welcome-screen blind
    # spot again. 250x466 is the GLOW slider at 960x640; its tip box lands
    # above the bar, and the crop below sits inside the box where the bare
    # frame is plain panel (measured: bare 247/247, tip 20/239).
    lyric_capture "lyrics-tip-glow-960x640" 960x640 \
        "panel=lyrics,style=effects,play=1,hover=250x466" || SWEEP_FAILED=1
    TIP_STATS="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/lyrics-tip-glow-960x640.png,crop=85:10:165:436,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN,lavfi.signalstats.YMAX \
        -of csv=p=0 2>/dev/null | head -1)"
    TIP_MIN="${TIP_STATS%%,*}"
    TIP_MAX="${TIP_STATS##*,}"
    echo "caption tooltip:  box luma min=${TIP_MIN:-?} text luma max=${TIP_MAX:-?}"
    if [ -z "$TIP_STATS" ] \
        || [ "${TIP_MIN%%.*}" -ge 100 ] 2>/dev/null \
        || [ "${TIP_MAX%%.*}" -lt 180 ] 2>/dev/null; then
        echo "FAIL: no tooltip where the GLOW slider's tip should be" >&2
        SWEEP_FAILED=1
    fi
    lyric_capture "lyrics-fonts-1280x720" 1280x720 \
        "panel=lyrics,style=caption,fonts=consent,play=1" \
        || SWEEP_FAILED=1
    # The non-Latin line itself bound into the editable field (review 4.3):
    # cue 1 is the seeded Greek+Cyrillic lyric, so this frame fails unless the
    # field, not just the list, drew through the glyph-complete face.
    lyric_capture "lyrics-nonlatin-1280x720" 1280x720 \
        "panel=lyrics,lyric=1,play=1" \
        || SWEEP_FAILED=1
    # Positive pin: the frames that show the cue pane must have served it from
    # the caption atlas — `face=none` here would mean the rows silently stopped
    # reporting, which is the blindness review 4.3 exists to end.
    for name in lyrics-cues-1280x720 lyrics-nonlatin-1280x720; do
        case "$(sed -n 's/^lyrics: *//p' "$OUT_DIR/$name.txt" | head -1)" in
            *"face=caption"*) ;;
            *)
                echo "FAIL: $name did not serve cue text from the caption atlas" >&2
                SWEEP_FAILED=1
                ;;
        esac
    done

    # The report line first: `picker=` says which of the two colours the frame
    # is showing, and `picker=none` on the plain style frames is what proves the
    # disclosure is closed by default rather than stuck open.
    for pair in \
        "lyrics-picker-ink-1280x720:picker=ink" \
        "lyrics-picker-ink-960x640:picker=ink" \
        "lyrics-picker-plate-960x640:picker=plate" \
        "lyrics-picker-glow-960x640:picker=glow" \
        "lyrics-picker-glow-960x640:pane caption-effects" \
        "lyrics-effects-960x640:pane caption-effects" \
        "lyrics-effects-960x640:picker=none" \
        "lyrics-effects-960x640:tune=none" \
        "lyrics-tune-pulse-960x640:tune=pulse" \
        "lyrics-tune-pulse-960x640:pane caption-effects" \
        "lyrics-tune-pulse-960x640:picker=none" \
        "lyrics-style-1280x720:picker=none" \
        "lyrics-style-960x640:picker=none"; do
        name="${pair%%:*}"
        want="${pair##*:}"
        case "$(sed -n 's/^lyrics: *//p' "$OUT_DIR/$name.txt" | head -1)" in
            *"$want"*) ;;
            *)
                echo "FAIL: $name did not report $want" >&2
                SWEEP_FAILED=1
                ;;
        esac
    done

    # And then the pixels, because the report line only proves the panel was
    # *told* to open the picker — the same trap review 1.4 found in the export
    # panel, where `panel: export` printed happily over a blank white band.
    #
    # The measurement is specific to what a hue bar is: a full turn around the
    # chroma circle. Sixteen pixel columns at x=124 are the bar itself (the
    # timeline panel's content starts at x=20 — 10 px window padding plus 10 px
    # panel padding — and the picker puts its bar 104 px in), swept over the
    # bottom 220 rows, which covers the panel at both window heights. A ramp
    # through all six sectors makes U and V each span nearly the full byte; grey
    # chrome and a single accent fill cannot, whatever else they draw.
    picker_chroma() {
        # picker_chroma NAME WINDOW_HEIGHT -> "Uspread Vspread"
        local png="$OUT_DIR/$1.png" top=$(( $2 - 220 ))
        local raw
        raw="$(ffprobe -v error -f lavfi \
            -i "movie=$png,crop=16:220:124:$top,signalstats" \
            -show_entries frame_tags=lavfi.signalstats.UMIN,lavfi.signalstats.UMAX,lavfi.signalstats.VMIN,lavfi.signalstats.VMAX \
            -of csv=p=0 2>/dev/null | head -1)"
        [ -z "$raw" ] && { echo "0 0"; return; }
        IFS=, read -r umin umax vmin vmax <<<"$raw"
        echo "$(( umax - umin )) $(( vmax - vmin ))"
    }
    for pair in \
        "lyrics-picker-ink-1280x720:720:open" \
        "lyrics-picker-ink-960x640:640:open" \
        "lyrics-picker-plate-960x640:640:open" \
        "lyrics-picker-glow-960x640:640:open" \
        "lyrics-effects-960x640:640:closed" \
        "lyrics-style-1280x720:720:closed" \
        "lyrics-style-960x640:640:closed"; do
        name="${pair%%:*}"
        rest="${pair#*:}"
        height="${rest%%:*}"
        state="${rest##*:}"
        read -r u_spread v_spread <<<"$(picker_chroma "$name" "$height")"
        printf '%-30s %-6s chroma spread U=%s V=%s\n' "$name" "$state" "$u_spread" "$v_spread"
        # Measured: an open picker gives 251/252, the closed left column 67/34.
        # The two thresholds are a decade apart on purpose, and the closed case
        # is the negative control this gate carries with it — a bar drawn
        # unconditionally would fail it.
        if [ "$state" = open ]; then
            if [ "$u_spread" -lt 200 ] || [ "$v_spread" -lt 200 ]; then
                echo "FAIL: $name says the picker is open but drew no hue ramp" >&2
                SWEEP_FAILED=1
            fi
        elif [ "$u_spread" -gt 100 ] || [ "$v_spread" -gt 100 ]; then
            echo "FAIL: $name drew a hue ramp with the picker closed" >&2
            SWEEP_FAILED=1
        fi
    done
fi

echo "=== project-aware scene frames and shared captions ==="
# A generated project whose three authored lanes all have an observable reader:
# long UTF-8 lyrics reach the shared overlay, semantics modulate the scene, and
# manual plus semantic events reach Constellation through the canonical merge.
# Every application launch below is both process-muted and pointed at the private
# unreachable Pulse server; the WAV and decoded PCM remain unchanged.
if [ ! -f "$LYRIC_PROJECT" ]; then
    echo "FAIL: the seeded project-lane fixture is unavailable" >&2
    SWEEP_FAILED=1
else
FRAME_LANE_DIR="$OUT_DIR/frame-lanes"
rm -rf "$FRAME_LANE_DIR"
mkdir -p "$FRAME_LANE_DIR"

make_frame_lane_variant() {
    # make_frame_lane_variant NAME SCENE BOX [DROPPED_LANE] [EFFECTS]
    local name="$1" scene="$2" box="$3" dropped="${4:-}" effects="${5:-}"
    local directory="$FRAME_LANE_DIR/$name"
    mkdir -p "$directory"
    cp "$LYRIC_PROJECT" "$directory/cues.musi"
    cp -a "$LYRIC_DIR/cues.assets" "$directory/cues.assets"
    local args=("$directory/cues.musi" --scene "$scene" --box "$box")
    if [ -n "$dropped" ]; then
        args+=(--drop "$dropped")
    fi
    if [ -n "$effects" ]; then
        args+=(--effects "$effects")
    fi
    python3 tools/seed_lyric_fixture.py "${args[@]}" >"$directory/seed.txt"
}

frame_lane_capture() {
    # frame_lane_capture NAME
    local name="$1"
    local directory="$FRAME_LANE_DIR/$name"
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer --mute --project "$directory/cues.musi" \
            --size 1280x720 \
            --hud=0 \
            --probe-frames 12 \
            --probe-shot "$directory/preview.png" \
            --ui-probe "time=1,play=0" \
        >"$directory/preview.txt" 2>&1
    local status=$?
    set -e
    local lanes scene
    lanes="$(sed -n 's/^frame lanes: *//p' "$directory/preview.txt" | head -1)"
    scene="$(sed -n 's/^scene: *//p' "$directory/preview.txt" | head -1)"
    echo "$name: exit=$status scene=${scene:-<absent>} lanes=${lanes:-<absent>}"
    if [ "$status" -ne 0 ] || [ ! -f "$directory/preview.png" ] || [ -z "$lanes" ]; then
        return 1
    fi
}

# Required scene surfaces, plus the two remaining box modes. Cadence receives the
# lyric in its own scene composition and therefore intentionally has no shared box.
make_frame_lane_variant shared-caption spectrum plate
make_frame_lane_variant cadence cadence plate
make_frame_lane_variant loom loom plate
make_frame_lane_variant full constellation plate
make_frame_lane_variant box-none spectrum none
make_frame_lane_variant box-shadow spectrum shadow
# The caption effects, over the same scene/box pairs as their baselines so the
# only authored difference is the effects block itself.
make_frame_lane_variant fx-glow spectrum plate "" glow
make_frame_lane_variant fx-soft-shadow spectrum shadow "" soft-shadow
# The two features crossed: an authored glow (so the offscreen halo blur runs
# every frame) over a scene that builds a `draw::SceneViewport` (Constellation).
# Neither half alone can show the defect — the blur only corrupts rlgl's cached
# framebuffer size, and only a scene viewport reads it — and the crossing was
# missing because each feature had its own fixture. See the `gl framebuffer:`
# assertion below.
make_frame_lane_variant fx-glow-3d constellation plate "" glow
# Two hand-derived companions for the soft shadow (UX0-C11 follow-up):
# `fx-shadow-hard` is the soft-shadow fixture with *only* its blur zeroed, so
# the delta against fx-soft-shadow isolates exactly what the blur drew — the
# same colours hard versus blurred, nothing else authored differently. And
# `fx-shadow-legacy` is the same fixture with the whole effects block removed,
# so hard-versus-legacy pins that a zeroed blur degenerates byte-exactly to
# the legacy composition (asserted as exactly 0 below).
make_frame_lane_variant fx-shadow-hard spectrum shadow "" soft-shadow
make_frame_lane_variant fx-shadow-legacy spectrum shadow "" soft-shadow
python3 - "$FRAME_LANE_DIR/fx-shadow-hard/cues.musi" "$FRAME_LANE_DIR/fx-shadow-legacy/cues.musi" <<'EOF'
import json, pathlib, sys
hard = pathlib.Path(sys.argv[1])
document = json.loads(hard.read_text())
document["caption_style"]["effects"]["shadow_blur"] = 0.0
hard.write_text(json.dumps(document, indent=2) + "\n")
legacy = pathlib.Path(sys.argv[2])
document = json.loads(legacy.read_text())
del document["caption_style"]["effects"]
legacy.write_text(json.dumps(document, indent=2) + "\n")
EOF
for name in shared-caption cadence loom full box-none box-shadow fx-glow fx-glow-3d fx-soft-shadow fx-shadow-hard fx-shadow-legacy; do
    frame_lane_capture "$name" || SWEEP_FAILED=1
done

# The renderer's size invariant, stated rather than photographed.
#
# `BeginTextureMode` moves both the GL viewport and rlgl's cached framebuffer
# size; `EndTextureMode` restores only the first, through `SetupViewport`
# (`rcore.c:1109-1131`, `rcore.c:3537`). Nothing later in a frame resets the
# second. So every offscreen pass — the caption halo's blur, an export target —
# used to leave rlgl reporting the *buffer's* dimensions for the rest of the
# session, and `draw::SceneViewport` then scaled a panel boundary by a ratio
# like 0.15: a scene panel that drew nothing, or the entire interface squeezed
# into the window's bottom-left corner, persisting frame after frame.
#
# Asserted on the report line rather than on pixels, because both symptoms are
# *coherent* pictures — every element is exactly where the wrong arithmetic put
# it — and because the run still exits 0 throughout. The number is zero when the
# invariant holds and counts frames when it does not, so there is no threshold
# to tune. Checked over every variant, since a scene viewport plus any offscreen
# pass is enough and only `fx-glow-3d` crosses both on purpose.
for name in shared-caption cadence loom full box-none box-shadow fx-glow fx-glow-3d fx-soft-shadow fx-shadow-hard fx-shadow-legacy; do
    fb_line="$(sed -n 's/^gl framebuffer: *//p' "$FRAME_LANE_DIR/$name/preview.txt" | head -1)"
    printf '%-18s gl framebuffer: %s\n' "$name" "${fb_line:-<absent>}"
    case "$fb_line" in
        "gl="*" render="*" mismatched-frames=0 of "*) : ;;
        "")
            echo "FAIL: $name reported no gl framebuffer line" >&2
            SWEEP_FAILED=1
            ;;
        *)
            echo "FAIL: $name ended frames with rlgl's framebuffer size out of sync: $fb_line" >&2
            SWEEP_FAILED=1
            ;;
    esac
done

# Each effects variant differs from its baseline *only* in the authored
# effects block, and the parked frame is deterministic (asserted below for the
# preview), so a per-pixel difference isolates exactly what the effect drew.
# An average-luma crop cannot do this: the first version of this gate measured
# the caption's corner and read 134 → 136, because the crop was mostly chrome
# and the halo was diluted into signalstats noise.
caption_fx_delta() {
    # caption_fx_delta VARIANT BASELINE -> peak |difference| luma, whole frame
    ffprobe -v error -f lavfi \
        -i "movie=$FRAME_LANE_DIR/$1/preview.png[a];movie=$FRAME_LANE_DIR/$2/preview.png[b];[a][b]blend=all_mode=difference,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null \
        | head -1 | cut -d. -f1
}
GLOW_DELTA="$(caption_fx_delta fx-glow shared-caption)"
# The shadow's baseline is its own zero-blur twin, not the legacy box-shadow
# frame: the two differ *only* in `shadow_blur`, so the delta isolates exactly
# what the blur moved. Against the legacy frame the number would also carry the
# fixture's recoloured shadow, and a bright hard shadow alone would clear any
# threshold with the blur broken — the same trap the first fx-glow fixture
# (authored roundness) was rejected for.
SHADOW_DELTA="$(caption_fx_delta fx-soft-shadow fx-shadow-hard)"
# Standing negative control: with the blur zeroed, the effects path must
# degenerate byte-exactly to the legacy hard-shadow composition, so the
# zero-blur twin against the no-effects-block twin is asserted as exactly 0.
SHADOW_ZERO_DELTA="$(caption_fx_delta fx-shadow-hard fx-shadow-legacy)"
# The control: a frame diffed against itself must be exactly zero, or the
# instrument itself is broken and every assertion here is meaningless.
SELF_DELTA="$(caption_fx_delta shared-caption shared-caption)"
echo "caption fx delta: glow=$GLOW_DELTA soft-shadow=$SHADOW_DELTA shadow-zero=$SHADOW_ZERO_DELTA control=$SELF_DELTA"
if [ "${SELF_DELTA:-1}" -ne 0 ]; then
    echo "FAIL: the fx difference control is $SELF_DELTA, not 0" >&2
    SWEEP_FAILED=1
fi
if [ "${SHADOW_ZERO_DELTA:-1}" -ne 0 ]; then
    echo "FAIL: a zeroed shadow blur did not degenerate to the legacy composition (delta $SHADOW_ZERO_DELTA)" >&2
    SWEEP_FAILED=1
fi
# Measured 2026-08-04, after UX0-C11 replaced the 17-tap glow with the
# offscreen render-texture blur and the follow-up moved the soft shadow onto
# the same blur (`halo_mask.fs` composites the buffer's luminance as coverage
# alpha): glow 123 (was 125 with the taps — the peak barely moves because the
# halo core saturates the same pixels; what changed is the skirt, now one
# widening Gaussian instead of discrete text copies), soft shadow 152 against
# its zero-blur twin (blur 0.15 measures 131, 0.4 measures 152 — evidence
# under build/shadow-evidence/). The shadow's old number was 3-4, because the
# original fixture blurred a near-black shadow over a near-black scene corner;
# the seeder now authors a bright warm shadow (`box_rgba ffd27ae8`, blur 0.3)
# so the gate measures the blur rather than the fixture's modesty. The
# hard-versus-legacy degeneration control measured exactly 0 on first run.
# Both thresholds keep ~4x headroom under their measured values.
#
# Negative controls, re-run by hand for the RT-blur halo (2026-08-04): a copy
# of the fx-glow fixture — project *and* its `.assets` bundle — with
# `glow_strength` hand-edited to 0.0 measured **exactly 0** against the
# shared-caption baseline, captured in a separate run, so the zero also pins
# cross-run determinism. A radius sweep at strength 1.0 measured 169 / 135 /
# 95 peak delta at radius 0.08 / 0.3 / 0.6 (wider halo, same energy, lower
# peak), with captures under build/glow-evidence/. The original controls
# stand: an explicit all-default effects block measured exactly 0 against the
# no-block baseline, and the first fx-glow fixture, which authored plate
# roundness alongside the glow, was rejected because the reshaped corners
# alone would have cleared the glow threshold with the glow broken.
if [ -z "$GLOW_DELTA" ] || [ "$GLOW_DELTA" -lt 32 ]; then
    echo "FAIL: the authored glow changed the frame by ${GLOW_DELTA:-nothing} (< 32)" >&2
    SWEEP_FAILED=1
fi
if [ -z "$SHADOW_DELTA" ] || [ "$SHADOW_DELTA" -lt 32 ]; then
    echo "FAIL: the soft shadow changed the frame by ${SHADOW_DELTA:-nothing} (< 32)" >&2
    SWEEP_FAILED=1
fi
# The halo's own report line (UX0-C11). The pixel delta above proves *something*
# drew; this proves it was the blurred halo and not a fallback, and — the case a
# capture cannot carry — that the no-effects baseline was `off` rather than
# `unavailable`, which are the same picture with different meanings.
GLOW_HALO_LINE="$(sed -n 's/^caption halo: *//p' "$FRAME_LANE_DIR/fx-glow/preview.txt" | head -1)"
BASE_HALO_LINE="$(sed -n 's/^caption halo: *//p' "$FRAME_LANE_DIR/shared-caption/preview.txt" | head -1)"
echo "caption halo: fx-glow=[${GLOW_HALO_LINE:-<absent>}] baseline=[${BASE_HALO_LINE:-<absent>}]"
if [ "$GLOW_HALO_LINE" != "rt-blur last=blurred" ]; then
    echo "FAIL: the authored glow did not draw through the RT blur: ${GLOW_HALO_LINE:-<absent>}" >&2
    SWEEP_FAILED=1
fi
if [ "$BASE_HALO_LINE" != "rt-blur last=off" ]; then
    echo "FAIL: the no-effects baseline's halo state is not off: ${BASE_HALO_LINE:-<absent>}" >&2
    SWEEP_FAILED=1
fi

FULL_LANES="$(sed -n 's/^frame lanes: *//p' "$FRAME_LANE_DIR/full/preview.txt" | head -1)"
if [ "$FULL_LANES" != "lyric=1 semantic=available source=11 merged-events=4" ]; then
    echo "FAIL: the seeded frame did not carry all project lanes: $FULL_LANES" >&2
    SWEEP_FAILED=1
fi
IMPORTED_LINE="$(sed -n 's/^fonts: *//p' "$FRAME_LANE_DIR/full/preview.txt" | head -1)"
case "$IMPORTED_LINE" in
    *"imported="*"cues.assets/fonts/"*) : ;;
    *)
        echo "FAIL: the generated project did not rasterize its imported caption face: $IMPORTED_LINE" >&2
        SWEEP_FAILED=1
        ;;
esac

# Which atlas the caption was drawn from. A caption magnified out of the shared
# 64 px atlas and one rasterized at its drawn size are both perfectly plausible
# pictures of a caption, so this is a claim only the report line can carry:
# `sizes=[...]` non-empty means an at-size atlas served the frame, and `failed=0`
# with `overflow=0` means it did so without falling back on the way.
CAPTION_ATLAS_LINE="$(sed -n 's/^fonts: *//p' "$FRAME_LANE_DIR/shared-caption/preview.txt" | head -1)"
echo "caption atlas: ${CAPTION_ATLAS_LINE#*caption-atlas=}"
case "$CAPTION_ATLAS_LINE" in
    *"caption-atlas=(sizes=[]"*)
        echo "FAIL: the caption overlay drew from the shared 64px atlas: $CAPTION_ATLAS_LINE" >&2
        SWEEP_FAILED=1
        ;;
    *"caption-atlas=("*"failed=0"*"overflow=0"*) : ;;
    *)
        echo "FAIL: an at-size caption atlas was refused or overflowed: $CAPTION_ATLAS_LINE" >&2
        SWEEP_FAILED=1
        ;;
esac

# And which face typeset Cadence. The scene animates per-glyph scale, so it draws
# through a distance field rather than a raster atlas; `cadence-text=bitmap` here
# is the fallback, which is survivable and must not be silent.
CADENCE_TEXT_LINE="$(sed -n 's/^scene text: *//p' "$FRAME_LANE_DIR/cadence/preview.txt" | head -1)"
echo "cadence text: ${CADENCE_TEXT_LINE:-<absent>}"
case "${CADENCE_TEXT_LINE:-absent}" in
    "sdf-shader=compiled, cadence-text=sdf") : ;;
    *)
        echo "FAIL: Cadence did not typeset through the SDF path: ${CADENCE_TEXT_LINE:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac
# The scenes that are *not* Cadence must not have asked for it, which is what
# keeps a 200 ms SDF build off the startup path of the other nine.
SPECTRUM_TEXT_LINE="$(sed -n 's/^scene text: *//p' "$FRAME_LANE_DIR/shared-caption/preview.txt" | head -1)"
case "${SPECTRUM_TEXT_LINE:-absent}" in
    *"cadence-text=none") : ;;
    *)
        echo "FAIL: a non-Cadence frame built the SDF atlas: ${SPECTRUM_TEXT_LINE:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac

# All three box modes must paint differently. The composition unit test pins the
# layout rule; these hashes prove each mode reached the GPU draw path.
NONE_HASH="$(sha256sum "$FRAME_LANE_DIR/box-none/preview.png" | cut -d' ' -f1)"
SHADOW_HASH="$(sha256sum "$FRAME_LANE_DIR/box-shadow/preview.png" | cut -d' ' -f1)"
PLATE_HASH="$(sha256sum "$FRAME_LANE_DIR/shared-caption/preview.png" | cut -d' ' -f1)"
echo "caption boxes: none=$NONE_HASH shadow=$SHADOW_HASH plate=$PLATE_HASH"
if [ "$NONE_HASH" = "$SHADOW_HASH" ] || [ "$NONE_HASH" = "$PLATE_HASH" ] \
    || [ "$SHADOW_HASH" = "$PLATE_HASH" ]; then
    echo "FAIL: at least two caption box modes produced the same preview" >&2
    SWEEP_FAILED=1
fi

# Lane-removal negative controls. Each variant differs by one JSON lane only;
# report shape names what disappeared, while the hashes prove the loss is visible.
make_frame_lane_variant no-lyrics constellation plate lyrics
make_frame_lane_variant no-semantic constellation plate semantic
make_frame_lane_variant no-manual constellation plate manual
for name in no-lyrics no-semantic no-manual; do
    frame_lane_capture "$name" || SWEEP_FAILED=1
done
if [ "$(sed -n 's/^frame lanes: *//p' "$FRAME_LANE_DIR/no-lyrics/preview.txt" | head -1)" \
        != "lyric=none semantic=available source=11 merged-events=4" ]; then
    echo "FAIL: dropping lyrics did not remove only the active lyric" >&2
    SWEEP_FAILED=1
fi
if [ "$(sed -n 's/^frame lanes: *//p' "$FRAME_LANE_DIR/no-semantic/preview.txt" | head -1)" \
        != "lyric=1 semantic=unavailable source=0 merged-events=2" ]; then
    echo "FAIL: dropping semantics did not remove the semantic sample and events" >&2
    SWEEP_FAILED=1
fi
if [ "$(sed -n 's/^frame lanes: *//p' "$FRAME_LANE_DIR/no-manual/preview.txt" | head -1)" \
        != "lyric=1 semantic=available source=11 merged-events=2" ]; then
    echo "FAIL: dropping manual events did not remove only the manual half of the merge" >&2
    SWEEP_FAILED=1
fi

FULL_PREVIEW_HASH="$(sha256sum "$FRAME_LANE_DIR/full/preview.png" | cut -d' ' -f1)"
for name in no-lyrics no-semantic no-manual; do
    negative_hash="$(sha256sum "$FRAME_LANE_DIR/$name/preview.png" | cut -d' ' -f1)"
    echo "preview negative control $name: $negative_hash"
    if [ "$negative_hash" = "$FULL_PREVIEW_HASH" ]; then
        echo "FAIL: dropping $name did not change the parked preview" >&2
        SWEEP_FAILED=1
    fi
done

# Repeat the full parked preview byte-for-byte: the same seed, time and project
# must not depend on wall clock or mutable state inherited from another capture.
cp "$FRAME_LANE_DIR/full/preview.png" "$FRAME_LANE_DIR/full/preview-first.png"
frame_lane_capture full || SWEEP_FAILED=1
FULL_REPEAT_HASH="$(sha256sum "$FRAME_LANE_DIR/full/preview.png" | cut -d' ' -f1)"
if [ "$FULL_REPEAT_HASH" != "$FULL_PREVIEW_HASH" ]; then
    echo "FAIL: the parked seeded preview was not deterministic" >&2
    SWEEP_FAILED=1
fi

run_project_lane_export() {
    # run_project_lane_export FIXTURE_NAME RUN_NAME
    local fixture="$1" run="$2"
    local directory="$FRAME_LANE_DIR/$fixture"
    local output="$FRAME_LANE_DIR/$run.mp4"
    set +e
    env -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        ./target/debug/musializer --mute --project "$directory/cues.musi" \
            --render "$output" --render-window 1 0.2 \
            --resolution 640x360 --fps 30 --quality balanced \
        >"$FRAME_LANE_DIR/$run-export.txt" 2>&1
    local status=$?
    set -e
    if [ "$status" -ne 0 ] || [ ! -f "$output" ]; then
        echo "FAIL: project-lane export $run exited $status" >&2
        return 1
    fi
    ffmpeg -v error -y -i "$output" -frames:v 1 "$FRAME_LANE_DIR/$run-frame.png"
}

EXPORT_LANES_OK=1
run_project_lane_export full full-a || EXPORT_LANES_OK=0
run_project_lane_export full full-b || EXPORT_LANES_OK=0
run_project_lane_export no-lyrics no-lyrics || EXPORT_LANES_OK=0
run_project_lane_export no-semantic no-semantic || EXPORT_LANES_OK=0
run_project_lane_export no-manual no-manual || EXPORT_LANES_OK=0
if [ "$EXPORT_LANES_OK" -ne 1 ]; then
    SWEEP_FAILED=1
else
    FULL_EXPORT_HASH="$(sha256sum "$FRAME_LANE_DIR/full-a-frame.png" | cut -d' ' -f1)"
    FULL_EXPORT_REPEAT="$(sha256sum "$FRAME_LANE_DIR/full-b-frame.png" | cut -d' ' -f1)"
    echo "project export determinism: $FULL_EXPORT_HASH / $FULL_EXPORT_REPEAT"
    if [ "$FULL_EXPORT_HASH" != "$FULL_EXPORT_REPEAT" ]; then
        echo "FAIL: the seeded project export was not deterministic" >&2
        SWEEP_FAILED=1
    fi
    for name in no-lyrics no-semantic no-manual; do
        negative_hash="$(sha256sum "$FRAME_LANE_DIR/$name-frame.png" | cut -d' ' -f1)"
        echo "export negative control $name: $negative_hash"
        if [ "$negative_hash" = "$FULL_EXPORT_HASH" ]; then
            echo "FAIL: dropping $name did not change the exported frame" >&2
            SWEEP_FAILED=1
        fi
    done
    PREVIEW_PAYLOAD="$(sed -n 's/^frame lanes: *//p' "$FRAME_LANE_DIR/full/preview.txt" | head -1)"
    EXPORT_PAYLOAD="$(sed -n 's/^export frame lanes: t=[^ ]* //p' "$FRAME_LANE_DIR/full-a-export.txt" | head -1)"
    echo "preview/export frame data: preview=[$PREVIEW_PAYLOAD] export=[$EXPORT_PAYLOAD]"
    if [ "$PREVIEW_PAYLOAD" != "$EXPORT_PAYLOAD" ]; then
        echo "FAIL: preview and export saw different project lanes at t=1" >&2
        SWEEP_FAILED=1
    fi
fi
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
        ./target/debug/musializer --mute \
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

echo "=== command-line action ordering ==="
# Inputs are actions, not candidates for one eventual slot. The frozen C appends
# both positional audio files and leaves the first one current; this report line
# catches the old Rust reduction to `Option<Input>`, where only the second file
# survived long enough to be opened.
CLI_MULTI_LOG="$OUT_DIR/cli-multiple-inputs.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" "$FIXTURE_TWO" \
        --probe-frames 1 \
    >"$CLI_MULTI_LOG" 2>&1
CLI_MULTI_STATUS=$?
set -e
CLI_MULTI_TRACKS="$(sed -n 's/^tracks: *//p' "$CLI_MULTI_LOG")"
echo "multiple inputs: ${CLI_MULTI_TRACKS:-<absent>} (exit=$CLI_MULTI_STATUS)"
case "${CLI_MULTI_TRACKS:-absent}" in
    "2 open, current 0 "*) : ;;
    *)
        echo "FAIL: CLI inputs were not opened left-to-right: ${CLI_MULTI_TRACKS:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac
if [ "$CLI_MULTI_STATUS" -ne 0 ]; then
    echo "FAIL: the multiple-input CLI run exited $CLI_MULTI_STATUS" >&2
    SWEEP_FAILED=1
fi

# `--ascii-image` selects ASCII Field only after a successful import. A missing
# file must leave the scene selected immediately before it alone. The run still
# prints its report on failure, so this checks the state as well as the exit code.
CLI_ASCII_LOG="$OUT_DIR/cli-failed-ascii.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute --scene loom \
        --ascii-image "$OUT_DIR/cli-absent-image.png" \
        --probe-frames 1 \
    >"$CLI_ASCII_LOG" 2>&1
CLI_ASCII_STATUS=$?
set -e
CLI_ASCII_SCENE="$(sed -n 's/^scene: *//p' "$CLI_ASCII_LOG")"
echo "failed ASCII import: ${CLI_ASCII_SCENE:-<absent>} (exit=$CLI_ASCII_STATUS)"
case "${CLI_ASCII_SCENE:-absent}" in
    "loom (Loom)") : ;;
    *)
        echo "FAIL: a failed ASCII import changed the selected scene: ${CLI_ASCII_SCENE:-<absent>}" >&2
        SWEEP_FAILED=1
        ;;
esac
if [ "$CLI_ASCII_STATUS" -eq 0 ]; then
    echo "FAIL: a missing command-line ASCII image exited successfully" >&2
    SWEEP_FAILED=1
fi

# The C keeps parsing after an error but gates later durable side effects. This
# was a particularly dangerous mismatch: Rust returned failure *and* wrote the
# requested project, so a caller could not infer whether failure meant no output.
CLI_BLOCKED_PROJECT="$OUT_DIR/cli-error-must-not-save.musi"
CLI_BLOCKED_LOG="$OUT_DIR/cli-error-save.txt"
rm -f "$CLI_BLOCKED_PROJECT"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" --fps 0 \
        --save-project "$CLI_BLOCKED_PROJECT" \
    >"$CLI_BLOCKED_LOG" 2>&1
CLI_BLOCKED_STATUS=$?
set -e
echo "error-gated save: file=$([ -e "$CLI_BLOCKED_PROJECT" ] && echo PRESENT || echo absent) (exit=$CLI_BLOCKED_STATUS)"
if [ "$CLI_BLOCKED_STATUS" -eq 0 ] || [ -e "$CLI_BLOCKED_PROJECT" ]; then
    echo "FAIL: an earlier CLI error did not suppress --save-project" >&2
    SWEEP_FAILED=1
fi

REOPEN_LOG="$OUT_DIR/reopen.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" \
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
    ./target/debug/musializer --mute "$PROJECT_DIR/source.wav" \
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

# Render flags are configured before the save stage. The old Rust order wrote a
# valid project with the defaults and only changed the in-memory config after the
# file was already published, which an ordinary reopen could not distinguish.
CONFIG_PROJECT="$PROJECT_DIR/configured.musi"
CONFIG_LOG="$OUT_DIR/project-configured-save.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$PROJECT_DIR/source.wav" \
        --resolution 854x480 --fps 24 --quality master \
        --save-project "$CONFIG_PROJECT" \
    >"$CONFIG_LOG" 2>&1
CONFIG_STATUS=$?
set -e
if [ "$CONFIG_STATUS" -eq 0 ] && python3 - "$CONFIG_PROJECT" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    output = json.load(source)["output"]
assert output["width"] == 854
assert output["height"] == 480
assert output["fps_numerator"] == 24
assert output["fps_denominator"] == 1
assert output["quality"] == "master"
PY
then
    echo "saved render configuration: 854x480 at 24 fps, master"
else
    echo "FAIL: render flags were not applied before --save-project" >&2
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
    ./target/debug/musializer --mute --project "$PROJECT" \
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

# Routes are the one deliberately deferred argv family. They must be applied
# after project hydration, while the scene action after that project remains an
# immediate action. This run catches both halves of the ordering contract.
PROJECT_ROUTE_LOG="$OUT_DIR/project-route.txt"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute --project "$PROJECT" --scene loom \
        --route 'loom.weight:band:2:0:1:0.4:2.2:smoothstep' \
        --probe-frames 1 \
    >"$PROJECT_ROUTE_LOG" 2>&1
PROJECT_ROUTE_STATUS=$?
set -e
PROJECT_ROUTE_SCENE="$(sed -n 's/^scene: *//p' "$PROJECT_ROUTE_LOG")"
PROJECT_ROUTE_COUNT="$(sed -n 's/^routes: *//p' "$PROJECT_ROUTE_LOG")"
echo "project + scene + deferred route: scene=${PROJECT_ROUTE_SCENE:-<absent>} routes=${PROJECT_ROUTE_COUNT:-<absent>} (exit=$PROJECT_ROUTE_STATUS)"
if [ "$PROJECT_ROUTE_STATUS" -ne 0 ] \
    || [ "$PROJECT_ROUTE_SCENE" != "loom (Loom)" ] \
    || [ "$PROJECT_ROUTE_COUNT" != "1" ]; then
    echo "FAIL: project hydration, scene action, and deferred route targeted different states" >&2
    SWEEP_FAILED=1
fi

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
# The Assist panel.
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
assist_capture() {
    # assist_capture NAME SIZE EXPECT_BODY EXPECT_HELPER [extra --ui-probe keys]
    local name="$1" size="$2" expect="$3" helper="$4" extra="${5:-}"
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    # ASSIST_PLAY=0 parks the transport: the Running body's elapsed clock is
    # drawn from the transport time, so a deterministic capture needs it still.
    local spec="panel=assist,play=${ASSIST_PLAY:-1}"
    [ -n "$extra" ] && spec="$spec,$extra"
    local helper_env=()
    [ "$helper" = "missing" ] && helper_env=("MUSIALIZER_ASSIST_HELPER=$OUT_DIR/absent-assist-helper.py")
    set +e
    env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        "${helper_env[@]}" \
        ./target/debug/musializer --mute "$FIXTURE" \
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
        "missing:helper=not found"*) ;;
        source:*"/tools/external_analysis.py"*) ;;
        *)
            echo "FAIL: $name expected a $helper helper, got: ${line:-<absent>}" >&2
            return 1
            ;;
    esac
    return 0
}

# The panel draws from `ui/panels/assist.rs`, but reaching its states and
# reporting them needs the live `main.rs` and `shell.rs` seams. The report below
# verifies those seams rather than treating a drawn panel body as sufficient.
set +e
env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" --size 1280x720 --probe-frames 3 \
    >"$OUT_DIR/assist-probe.txt" 2>&1
set -e
ASSIST_WIRED=0
grep -q '^assist:' "$OUT_DIR/assist-probe.txt" && ASSIST_WIRED=1

if [ "$ASSIST_WIRED" -eq 0 ]; then
    echo "SKIPPED: the report has no 'assist:' line, so the Assist panel is not"
    echo "         reachable from a probe yet. Apply the main.rs and shell.rs"
    echo "         live application seams, then rerun."
fi

for size in 1280x720 960x640; do
    [ "$ASSIST_WIRED" -eq 0 ] && break
    # The real source bundle is the normal path: the workflow row is live and
    # the confirmation step can arm without an environment override.
    assist_capture "assist-ready-$size" "$size" Ready source || SWEEP_FAILED=1
    assist_capture "assist-confirm-$size" "$size" Confirmation source \
        "assist=confirm" || SWEEP_FAILED=1
    # And with an authored sheet chosen, which is the row the panel exists for.
    assist_capture "assist-sheet-$size" "$size" Confirmation source \
        "assist=confirm,lyrics-file=$ASSIST_SHEET" || SWEEP_FAILED=1
    # Negative control: a set-but-invalid override fails hard and must not fall
    # back to the source helper.
    assist_capture "assist-missing-$size" "$size" Ready missing || SWEEP_FAILED=1
    # Review 4.2: the three bodies a user actually has to read. Synthesized by
    # the probe, so no helper runs and no clock ticks.
    ASSIST_PLAY=0 assist_capture "assist-candidate-$size" "$size" Candidate source \
        "assist=candidate" || SWEEP_FAILED=1
    ASSIST_PLAY=0 assist_capture "assist-running-$size" "$size" Running source \
        "assist=running" || SWEEP_FAILED=1
    ASSIST_PLAY=0 assist_capture "assist-failed-$size" "$size" Empty source \
        "assist=failed" || SWEEP_FAILED=1
done

# The synthesized states must be the states they claim, not a body label over
# the wrong machinery: candidate means a staged, successful job; running means
# an unstaged live one; failed means the job reached Failed.
for size in 1280x720 960x640; do
    [ "$ASSIST_WIRED" -eq 0 ] && break
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-candidate-$size.txt")" in
        *"state=Succeeded"*"staged=true"*) ;;
        *) echo "FAIL: assist=candidate staged nothing at $size" >&2; SWEEP_FAILED=1 ;;
    esac
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-running-$size.txt")" in
        *"state=Running"*"staged=false"*) ;;
        *) echo "FAIL: assist=running is not running at $size" >&2; SWEEP_FAILED=1 ;;
    esac
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-failed-$size.txt")" in
        *"state=Failed"*) ;;
        *) echo "FAIL: assist=failed did not reach the failure state at $size" >&2; SWEEP_FAILED=1 ;;
    esac
done

# The blocked-Apply reason moves to its own full-width row when the panel is
# too narrow for the beside-slot (review 1.13); the two supported sizes never
# reach that branch, so it gets its own frame.
if [ "$ASSIST_WIRED" -eq 1 ]; then
    ASSIST_PLAY=0 assist_capture "assist-candidate-narrow" 800x640 Candidate source \
        "assist=candidate" || SWEEP_FAILED=1
fi

# ---------------------------------------------------------------------------
# Tranche P4: the resolved route graph on the confirmation.
#
# The confirmation is where the decision is made, so it is where the routes have
# to be legible. Three frames, because three answers must look different and
# only one of them was reachable before:
#
#   local     a Timed-lyrics job. Every task stays on this machine except the
#             Codex wording review, which sends bounded JSON; the heading says
#             so and no row is tinted as audio-leaving.
#   remote    a MiMo job with a stubbed credential in a private 0600 file. One
#             task sends the whole track, and both the heading and that row are
#             warning-tinted.
#   no-key    the same graph with no credential anywhere, which must refuse
#             *before* anything spawns and name the repair (§5 invariant 4).
#
# The credential is a fixture in a throwaway config directory, never a real key,
# and every run greps its own report for it. No socket is opened: nothing here
# presses Start.
# ---------------------------------------------------------------------------
ROUTE_CONFIG="$OUT_DIR/assist-routes-config"
ROUTE_STUB_KEY="sk-or-v1-P4STUB0000000000000000000000000000000000000000000000000000000000"
rm -rf "$ROUTE_CONFIG"
mkdir -p "$ROUTE_CONFIG/nokey" "$ROUTE_CONFIG/keyed/musializer" "$ROUTE_CONFIG/cache"
printf '{"schema":"musializer.assist-credentials/v1","entries":{"openrouter/default":{"secret":"%s","label":"Gate stub"}}}\n' \
    "$ROUTE_STUB_KEY" >"$ROUTE_CONFIG/keyed/musializer/credentials.json"
chmod 700 "$ROUTE_CONFIG/keyed/musializer"
chmod 600 "$ROUTE_CONFIG/keyed/musializer/credentials.json"

assist_routes_capture() {
    # assist_routes_capture NAME LANES CONFIG_HOME
    local name="$1" lanes="$2" config="$3"
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        XDG_CONFIG_HOME="$config" \
        XDG_CACHE_HOME="$ROUTE_CONFIG/cache" \
        MUSIALIZER_ASSIST_PROBE_LANES="$lanes" \
        ./target/debug/musializer --mute "$FIXTURE" \
            --size 1280x720 \
            --probe-frames 30 \
            --probe-shot "$out" \
            --ui-probe "panel=assist,play=0,assist=confirm" \
        >"$log" 2>&1
    local status=$?
    set -e
    local line
    line="$(sed -n 's/^assist routing:  *//p' "$log" | tail -1)"
    printf '%-28s exit=%s %s\n' "$name" "$status" "${line:-<no assist routing line>}"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL: $name did not produce a frame, or exited $status" >&2
        return 1
    fi
    # No capture, log or report may ever carry the fixture credential.
    if grep -q -- "$ROUTE_STUB_KEY" "$log"; then
        echo "FAIL: $name printed the stub credential" >&2
        return 1
    fi
    return 0
}

assist_routes_saturation() {
    # assist_routes_saturation NAME CROP (w:h:x:y). Mean chroma, which is how the
    # LT1 block measures its own tinted rows; a colourless crop reads near zero.
    ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/$1.png,crop=$2,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.SATAVG -of csv=p=0 2>/dev/null | head -1
}

if [ "$ASSIST_WIRED" -eq 1 ]; then
    assist_routes_capture assist-routes-local lyrics "$ROUTE_CONFIG/nokey" || SWEEP_FAILED=1
    assist_routes_capture assist-routes-remote mimo "$ROUTE_CONFIG/keyed" || SWEEP_FAILED=1
    assist_routes_capture assist-routes-nokey mimo "$ROUTE_CONFIG/nokey" || SWEEP_FAILED=1

    ROUTE_LOCAL="$(sed -n 's/^assist routing:  *//p' "$OUT_DIR/assist-routes-local.txt" | tail -1)"
    ROUTE_REMOTE="$(sed -n 's/^assist routing:  *//p' "$OUT_DIR/assist-routes-remote.txt" | tail -1)"
    ROUTE_NOKEY="$(sed -n 's/^assist routing:  *//p' "$OUT_DIR/assist-routes-nokey.txt" | tail -1)"

    # A lyrics job never sends audio, and its graph is the four local tasks plus
    # the text-only wording review.
    case "$ROUTE_LOCAL" in
        *"TC-COARSE=local-proc/whisper.cpp[local-only]"*"TC-ALIGN=local-proc/mms-ctc[local-only]"*"audio-leaves=false"*"blocks=none"*) ;;
        *) echo "FAIL: the local graph is not local: $ROUTE_LOCAL" >&2; SWEEP_FAILED=1 ;;
    esac
    # A MiMo job with a key resolves the OpenRouter route and blocks nothing.
    case "$ROUTE_REMOTE" in
        *"TC-SEMANTIC=openrouter/xiaomi/mimo-v2.5[audio-leaves-machine]"*"credential=file"*"blocks=none"*) ;;
        *) echo "FAIL: the remote graph did not resolve: $ROUTE_REMOTE" >&2; SWEEP_FAILED=1 ;;
    esac
    # And without one it refuses, naming the contract and the missing piece
    # rather than the word "blocked".
    case "$ROUTE_NOKEY" in
        *"credential=none"*"blocks=TC-SEMANTIC:No key"*) ;;
        *) echo "FAIL: a keyless remote job was not refused: $ROUTE_NOKEY" >&2; SWEEP_FAILED=1 ;;
    esac
    # No applied fallback anywhere in any of the three can widen a boundary.
    for name in local remote nokey; do
        case "$(sed -n 's/^assist routing:  *//p' "$OUT_DIR/assist-routes-$name.txt" | tail -1)" in
            *"ask-as-none=none"*) ;;
            *) echo "FAIL: assist-routes-$name reports an unexpected ask policy" >&2
               SWEEP_FAILED=1 ;;
        esac
    done
    # The remote frame has to *look* different from the local one, or the block
    # is drawing the same three lines whatever it resolved. Warning ink is the
    # measurable difference: the heading and the audio-leaving row are tinted.
    # `w:h:x:y`, the same grammar the LT1 review measurements use. The window
    # spans the route block in both frames: the lyric-reference row makes the
    # local panel taller, so the block starts a few pixels apart in the two.
    ROUTE_BLOCK_CROP=700:100:18:595
    ROUTE_SAT_LOCAL="$(assist_routes_saturation assist-routes-local "$ROUTE_BLOCK_CROP")"
    ROUTE_SAT_REMOTE="$(assist_routes_saturation assist-routes-remote "$ROUTE_BLOCK_CROP")"
    ROUTE_SAT_NOKEY="$(assist_routes_saturation assist-routes-nokey "$ROUTE_BLOCK_CROP")"
    printf 'assist routes ink: local-sat=%s remote-sat=%s nokey-sat=%s\n' \
        "${ROUTE_SAT_LOCAL:-?}" "${ROUTE_SAT_REMOTE:-?}" "${ROUTE_SAT_NOKEY:-?}"
    if ! awk -v a="${ROUTE_SAT_LOCAL:-0}" -v b="${ROUTE_SAT_REMOTE:-0}" \
        'BEGIN{exit !(b > a + 1)}'; then
        echo "FAIL: the audio-leaving graph is not drawn any differently" >&2
        SWEEP_FAILED=1
    fi
    if grep -rqs -- "$ROUTE_STUB_KEY" "$OUT_DIR"/assist-routes-*.txt; then
        echo "FAIL: the stub credential reached a gate report" >&2
        SWEEP_FAILED=1
    fi
fi

# ---------------------------------------------------------------------------
# Tranche LT1: unresolved lines and review flags, by name and time range.
#
# The localizer's failures are sparse and specific, so a count alone sends the
# user back through the whole song. The panel names them, and this is where that
# claim is checked rather than assumed.
#
# Three job folders, because three answers must look different:
#
#   flagged  two unresolved lines (one of them an abstention) and two cues whose
#            views disagree -- four flags, four named rows;
#   clear    an LT1 run that placed everything, which must say so in words: a
#            blank region is indistinguishable from a broken one;
#   legacy   a manifest from before this tranche, which must render exactly as
#            it always did. This is the requirement that "absence" and "zero"
#            are different answers, and it is also the SATAVG control below.
#
# The fixtures are files rather than a literal in the panel, and they reach the
# probe through MUSIALIZER_ASSIST_PROBE_DIR, because the review is read from
# `assist-manifest.json` plus a `lyric-sync-v1` document in the job folder. That
# is the same path a real finished job takes, so what is photographed here is
# the real ingestion and not a synthesized picture of it.
# ---------------------------------------------------------------------------
#   stale    review LT1-R, R1: a *Sections* run's manifest with the lyric
#            document of an earlier full run still beside it, which is what an
#            audio-keyed cache folder actually looks like. It must render as
#            `absent`; before the fix it drew the other run's four flags.
ASSIST_REVIEW_DIR="$OUT_DIR/assist-review"
rm -rf "$ASSIST_REVIEW_DIR"
mkdir -p "$ASSIST_REVIEW_DIR/flagged" "$ASSIST_REVIEW_DIR/clear" \
         "$ASSIST_REVIEW_DIR/legacy" "$ASSIST_REVIEW_DIR/stale" \
         "$ASSIST_REVIEW_DIR/navigate" "$ASSIST_REVIEW_DIR/parked"

cat >"$ASSIST_REVIEW_DIR/flagged/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "lyrics",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json",
                "sync": "/nonexistent/job/lyrics.sync.json"},
  "result_counts": {"lyrics": 2, "lyrics_unmatched": 2, "lyrics_unresolved": 2,
                    "lyrics_review_flags": 4, "sections": 2, "semantics": 2},
  "lyric_localization": {"policy": "anchor-block-mms", "policy_version": "3"}
}
JSON
cat >"$ASSIST_REVIEW_DIR/flagged/lyrics.aligned.json" <<'JSON'
{
  "schema_version": "musializer.lyric-sync/v1",
  "lane": "lyric_sync",
  "localization_policy": "anchor-block-mms",
  "localization_policy_version": "3",
  "lines": [
    {"reference_line_index": 0, "text": "we were never meant to stay",
     "start_seconds": 12.0, "end_seconds": 16.0, "review_flagged": true},
    {"reference_line_index": 1, "text": "and the lights came up anyway",
     "start_seconds": 16.0, "end_seconds": 21.0, "review_flagged": true}
  ],
  "unresolved": [
    {"reference_line_index": 25, "line_position": 25, "kind": "lyric",
     "text": "hold the note until it breaks", "reason": "no block placement",
     "abstained": false, "coarse_start_seconds": 90.6, "coarse_end_seconds": 94.2},
    {"reference_line_index": 30, "line_position": 30, "kind": "lyric",
     "text": "and again, and again",
     "reason": "repeated phrase could not be pinned", "abstained": true,
     "coarse_start_seconds": null, "coarse_end_seconds": null}
  ],
  "review_flags": [
    {"reference_line_index": 0, "text": "we were never meant to stay",
     "flag": "coarse_disagreement", "reason": "the two views differ by 21.6 s",
     "start_seconds": 12.0, "end_seconds": 16.0,
     "coarse_start_seconds": 33.6, "delta_seconds": -21.6},
    {"reference_line_index": 1, "text": "and the lights came up anyway",
     "flag": "coarse_disagreement", "reason": "the two views differ by 8.4 s",
     "start_seconds": 16.0, "end_seconds": 21.0,
     "coarse_start_seconds": 24.4, "delta_seconds": -8.4},
    {"reference_line_index": 25, "text": "hold the note until it breaks",
     "flag": "unresolved", "reason": "no block placement",
     "start_seconds": null, "end_seconds": null,
     "coarse_start_seconds": 90.6, "delta_seconds": null},
    {"reference_line_index": 30, "text": "and again, and again",
     "flag": "unresolved", "reason": "repeated phrase could not be pinned",
     "start_seconds": null, "end_seconds": null,
     "coarse_start_seconds": null, "delta_seconds": null}
  ],
  "statistics": {"reference_lines": 4, "matched_lines": 2, "estimated_lines": 0,
                 "unmatched_lines": 2, "reference_tokens": 30,
                 "unresolved_lines": 2, "abstained_lines": 1,
                 "review_flagged_lines": 4, "coarse_disagreement_lines": 2}
}
JSON
cat >"$ASSIST_REVIEW_DIR/clear/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "lyrics",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json"},
  "result_counts": {"lyrics": 2, "lyrics_unmatched": 0, "lyrics_unresolved": 0,
                    "lyrics_review_flags": 0, "sections": 2, "semantics": 2},
  "lyric_localization": {"policy": "anchor-block-mms", "policy_version": "3"}
}
JSON
cat >"$ASSIST_REVIEW_DIR/clear/lyrics.aligned.json" <<'JSON'
{
  "schema_version": "musializer.lyric-sync/v1",
  "lane": "lyric_sync",
  "localization_policy": "anchor-block-mms",
  "localization_policy_version": "3",
  "lines": [
    {"reference_line_index": 0, "text": "we were never meant to stay",
     "start_seconds": 12.0, "end_seconds": 16.0, "review_flagged": false},
    {"reference_line_index": 1, "text": "and the lights came up anyway",
     "start_seconds": 16.0, "end_seconds": 21.0, "review_flagged": false}
  ],
  "unresolved": [],
  "review_flags": [],
  "statistics": {"reference_lines": 2, "matched_lines": 2, "estimated_lines": 0,
                 "unmatched_lines": 0, "reference_tokens": 12,
                 "unresolved_lines": 0, "review_flagged_lines": 0}
}
JSON
# Pre-LT1: the manifest a job folder written before this tranche carries. No
# `lyrics_unresolved`, no `lyrics_review_flags`, no `lyric_localization`.
cat >"$ASSIST_REVIEW_DIR/legacy/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "lyrics",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json"},
  "result_counts": {"lyrics": 2, "lyrics_unmatched": 0, "sections": 2, "semantics": 2}
}
JSON
cat >"$ASSIST_REVIEW_DIR/legacy/lyrics.aligned.json" <<'JSON'
{
  "schema_version": "musializer.lyric-sync/v1",
  "lane": "lyric_sync",
  "lines": [{"reference_line_index": 0, "text": "we were never meant to stay",
             "start_seconds": 12.0, "end_seconds": 16.0}]
}
JSON

# Review LT1-R, R1. The manifest of a run with no lyrics lane, over the lyric
# document a previous run left in the same audio-keyed folder. The document is
# byte-identical to `flagged`'s, so anything the panel draws here came from the
# wrong run.
cat >"$ASSIST_REVIEW_DIR/stale/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "sections",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json"},
  "result_counts": {"lyrics": 0, "lyrics_unmatched": 0, "lyrics_unresolved": 0,
                    "lyrics_review_flags": 0, "sections": 2, "semantics": 0},
  "lyric_localization": null
}
JSON
cp "$ASSIST_REVIEW_DIR/flagged/lyrics.aligned.json" \
   "$ASSIST_REVIEW_DIR/stale/lyrics.aligned.json"

# Review LT1-R, R9. The `flagged` fixture above names lines no document in this
# repository has, which is right for the list and useless for the *navigation*:
# a row can only bind a cue that exists in the track the Lyrics panel edits. So
# this job folder flags one of `seed_lyric_fixture.py`'s own seeded lines by its
# exact text and time, and leaves one unresolved with a proposal — the two
# outcomes a press has to keep apart.
cat >"$ASSIST_REVIEW_DIR/navigate/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "lyrics",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json"},
  "result_counts": {"lyrics": 2, "lyrics_unmatched": 1, "lyrics_unresolved": 1,
                    "lyrics_review_flags": 2, "sections": 2, "semantics": 2},
  "lyric_localization": {"policy": "anchor-block-mms", "policy_version": "3"}
}
JSON
cat >"$ASSIST_REVIEW_DIR/navigate/lyrics.aligned.json" <<'JSON'
{
  "schema_version": "musializer.lyric-sync/v1",
  "lane": "lyric_sync",
  "localization_policy": "anchor-block-mms",
  "localization_policy_version": "3",
  "lines": [
    {"reference_line_index": 1, "text": "Every band a step above",
     "start_seconds": 1.65, "end_seconds": 2.65, "review_flagged": true}
  ],
  "unresolved": [
    {"reference_line_index": 9, "line_position": 9, "kind": "lyric",
     "text": "a line the sheet has and the song does not",
     "reason": "no block placement", "abstained": false,
     "coarse_start_seconds": 5.2, "coarse_end_seconds": 6.4}
  ],
  "review_flags": [
    {"reference_line_index": 1, "text": "Every band a step above",
     "flag": "coarse_disagreement", "reason": "the two views differ by 3.9 s",
     "start_seconds": 1.65, "end_seconds": 2.65,
     "coarse_start_seconds": 5.55, "delta_seconds": 3.9},
    {"reference_line_index": 9, "text": "a line the sheet has and the song does not",
     "flag": "unresolved", "reason": "no block placement",
     "start_seconds": null, "end_seconds": null,
     "coarse_start_seconds": 5.2, "delta_seconds": null}
  ],
  "statistics": {"reference_lines": 2, "matched_lines": 1, "estimated_lines": 0,
                 "unmatched_lines": 1, "reference_tokens": 14,
                 "unresolved_lines": 1, "abstained_lines": 0,
                 "review_flagged_lines": 2, "coarse_disagreement_lines": 1}
}
JSON

# Tranche LX1-f. Ten unresolved lines whose coarse proposals are all *inside*
# the probe bridge's 60 s lane, which is what `flagged` deliberately is not: its
# two proposals are 1:30.6 and nothing at all, so neither can be parked and the
# feature would have no capture at all.
#
# Ten because the list holds three named rows plus a tail, so this is the first
# fixture in the tree that has to scroll — and a scroll a headless run cannot
# reach is a scroll nobody reviews, which is why
# MUSIALIZER_ASSIST_PROBE_REVIEW_SCROLL exists.
{
    printf '%s\n' '{' \
        '  "schema_version": "musializer.lyric-sync/v1",' \
        '  "lane": "lyric_sync",' \
        '  "localization_policy": "anchor-block-mms",' \
        '  "localization_policy_version": "3",' \
        '  "lines": [],' \
        '  "unresolved": ['
    for index in $(seq 0 9); do
        start="$(( index * 5 + 5 ))"
        printf '    {"reference_line_index": %d, "line_position": %d, "kind": "lyric",\n' \
            "$index" "$index"
        printf '     "text": "a line nobody could pin, number %d", "reason": "no block placement",\n' \
            "$index"
        printf '     "abstained": false, "coarse_start_seconds": %d.0, "coarse_end_seconds": %d.5}' \
            "$start" "$(( start + 2 ))"
        [ "$index" -lt 9 ] && printf ','
        printf '\n'
    done
    printf '%s\n' '  ],' '  "review_flags": ['
    for index in $(seq 0 9); do
        start="$(( index * 5 + 5 ))"
        printf '    {"reference_line_index": %d, "text": "a line nobody could pin, number %d",\n' \
            "$index" "$index"
        printf '     "flag": "unresolved", "reason": "no block placement",\n'
        printf '     "start_seconds": null, "end_seconds": null, "coarse_start_seconds": %d.0}' \
            "$start"
        [ "$index" -lt 9 ] && printf ','
        printf '\n'
    done
    printf '%s\n' '  ],' \
        '  "statistics": {"reference_lines": 10, "matched_lines": 0, "unresolved_lines": 10,' \
        '                 "abstained_lines": 0, "review_flagged_lines": 10}' '}'
} >"$ASSIST_REVIEW_DIR/parked/lyrics.aligned.json"
cat >"$ASSIST_REVIEW_DIR/parked/assist-manifest.json" <<'JSON'
{
  "schema_version": "musializer.assist-manifest/v1",
  "mode": "lyrics",
  "artifacts": {"aligned": "/nonexistent/job/lyrics.aligned.json"},
  "result_counts": {"lyrics": 2, "lyrics_unmatched": 10, "lyrics_unresolved": 10,
                    "lyrics_review_flags": 10, "sections": 2, "semantics": 2},
  "lyric_localization": {"policy": "anchor-block-mms", "policy_version": "3"}
}
JSON

assist_review_capture() {
    # assist_review_capture NAME JOB_FOLDER [SIZE] [LANES] [PROBE_EXTRA]
    #
    # SIZE and LANES are review LT1-R: R2 needs the 960x640 window whose scissor
    # was eating the tail, and R11 needs the lyrics-only candidate, which
    # `probe_candidate` could not produce until it took a mode. PROBE_EXTRA is
    # R9's, and it is `hover=XxY`: a headless run has no pointer, so without it
    # every hover state on the review rows would be unphotographable — the blind
    # spot `--ui-probe hover=` was invented to close.
    local name="$1" dir="$2" size="${3:-1280x720}" lanes="${4:-}" extra="${5:-}"
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER \
        -u MUSIALIZER_ASSIST_PROBE_ACTIVATE_ROW \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        MUSIALIZER_ASSIST_PROBE_DIR="$dir" \
        MUSIALIZER_ASSIST_PROBE_LANES="$lanes" \
        MUSIALIZER_ASSIST_PROBE_REVIEW_SCROLL="${REVIEW_SCROLL:-}" \
        ./target/debug/musializer --mute "$FIXTURE" \
            --size "$size" \
            --probe-frames 30 \
            --probe-shot "$out" \
            --ui-probe "panel=assist,play=0,assist=candidate$extra" \
        >"$log" 2>&1
    local status=$?
    set -e
    local line
    line="$(sed -n 's/^assist review: *//p' "$log")"
    printf '%-28s exit=%s %s\n' "$name" "$status" "${line:-<no assist review line>}"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL: $name did not produce a frame, or exited $status" >&2
        return 1
    fi
    return 0
}

assist_review_navigate_capture() {
    # assist_review_navigate_capture NAME ROW
    #
    # Review LT1-R, R9: the frame *after* a review row is pressed. Over
    # `$LYRIC_PROJECT`, because a row can only bind a cue that already exists in
    # the track the Lyrics panel edits, and the bare sweep has none.
    #
    # The press arrives through MUSIALIZER_ASSIST_PROBE_ACTIVATE_ROW rather than
    # through the pointer: `--ui-probe hover=` parks a pointer and nothing in the
    # grammar releases a button, and the grammar lives in `cli.rs`. It goes
    # through the same `Shell::open_lyric_review_row` a real press does, so this
    # photographs the navigation and not a picture of it.
    local name="$1" row="$2"
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        MUSIALIZER_ASSIST_PROBE_DIR="$ASSIST_REVIEW_DIR/navigate" \
        MUSIALIZER_ASSIST_PROBE_LANES="lyrics" \
        MUSIALIZER_ASSIST_PROBE_ACTIVATE_ROW="$row" \
        ./target/debug/musializer --mute --project "$LYRIC_PROJECT" \
            --size 1280x720 \
            --probe-frames 30 \
            --probe-shot "$out" \
            --ui-probe "panel=assist,play=0,assist=candidate" \
        >"$log" 2>&1
    local status=$?
    set -e
    printf '%-28s exit=%s %s\n' "$name" "$status" \
        "$(sed -n 's/^assist review nav: *//p' "$log" | head -1)"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL: $name did not produce a frame, or exited $status" >&2
        return 1
    fi
    return 0
}

# Saturation rather than luma: the panel surface and its disabled buttons are
# achromatic, and the review rows are accent blue and warning orange. A crop that
# is bright *and* coloured can only be drawn review text -- a blank surface or a
# grey button reads near zero, which is what `legacy` measures below.
# Darkest pixel in a crop. The tail row is muted grey rather than coloured, so
# saturation cannot see it; what distinguishes "text here" from "panel surface"
# is that text has dark pixels in it at all (review LT1-R, R2).
assist_review_luma_min() {
    # assist_review_luma_min NAME CROP
    ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/$1.png,crop=$2,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1
}

assist_review_saturation() {
    # assist_review_saturation NAME CROP
    ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/$1.png,crop=$2,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.SATAVG -of csv=p=0 2>/dev/null | head -1
}

if [ "$ASSIST_WIRED" -eq 1 ]; then
    echo "--- LT1 review surface ---"
    assist_review_capture assist-review-flagged "$ASSIST_REVIEW_DIR/flagged" || SWEEP_FAILED=1
    assist_review_capture assist-review-clear "$ASSIST_REVIEW_DIR/clear" || SWEEP_FAILED=1
    assist_review_capture assist-review-legacy "$ASSIST_REVIEW_DIR/legacy" || SWEEP_FAILED=1
    assist_review_capture assist-review-stale "$ASSIST_REVIEW_DIR/stale" || SWEEP_FAILED=1
    assist_review_capture assist-review-tail "$ASSIST_REVIEW_DIR/flagged" 960x640 \
        || SWEEP_FAILED=1
    assist_review_capture assist-review-lyrics-only "$ASSIST_REVIEW_DIR/flagged" \
        1280x720 lyrics || SWEEP_FAILED=1

    # The counts, and the names. `line 26` and `1:30.6-1:34.2` are the whole
    # point: a run that printed only "2 unresolved" would pass a count check and
    # still leave the user hunting.
    #
    # `rows_drawn=`/`tail=` are review LT1-R (R5): the line used to describe the
    # parse, which cannot be clipped, while the panel below it was being cut.
    # `first=`/`parked=` are LX1-f. `parked=0/0/2` here on purpose: this fixture's
    # two unresolved lines propose 1:30.6 and nothing at all, against a 60 s
    # staged lane — so neither has a window this side can honour, and the
    # `parked` folder below is where a proposal actually reaches the timeline.
    REVIEW_FLAGGED_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-flagged.txt")"
    case "$REVIEW_FLAGGED_LINE" in
        "unresolved=2 flagged=4 listed=4 rows_drawn=4 tail=no first=0 omitted=0 \
parked=0/0/2 counts=document \
manifest=2/4 policy=anchor-block-mms | "*) ;;
        *)
            echo "FAIL: the flagged review did not report 2 unresolved, 4 flags and 4 rows:" >&2
            echo "      ${REVIEW_FLAGGED_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    # The row grammar, including the three LT1-R fixes a screenshot shows and a
    # count cannot: the proposal label (R8), the number/clock separator (R7) and
    # the reason that was parsed and never drawn (R6).
    for named in \
        'UNPLACED line 26 proposed 1:30.6-1:34.2 "hold the note until it breaks"' \
        'AMBIGUOUS line 31 not placed "and again, and again" (repeated phrase could not be pinned)' \
        'CHECK line 1 at 0:12.0-0:16.0 "we were never meant to stay" (views differ 21.6s)' \
        'CHECK line 2 at 0:16.0-0:21.0 "and the lights came up anyway" (views differ 8.4s)'
    do
        case "$REVIEW_FLAGGED_LINE" in
            *"$named"*) ;;
            *)
                echo "FAIL: the review list did not name: $named" >&2
                SWEEP_FAILED=1
                ;;
        esac
    done

    REVIEW_CLEAR_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-clear.txt")"
    case "$REVIEW_CLEAR_LINE" in
        "unresolved=0 flagged=0 listed=0 rows_drawn=0 tail=no first=0 omitted=0 \
parked=0/0/0 counts=document \
manifest=0/0 policy=anchor-block-mms | All lines placed, none flagged") ;;
        *)
            echo "FAIL: a run that placed every line did not say so: ${REVIEW_CLEAR_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac

    # Review LT1-R, R1. The Sections run whose folder still holds the previous
    # run's lyric document draws no review at all. This is the assertion that
    # would have caught the defect: before the fix this line named `line 26`.
    REVIEW_STALE_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-stale.txt")"
    case "$REVIEW_STALE_LINE" in
        "absent"*) ;;
        *)
            echo "FAIL: a run with no lyrics lane showed another run's review:" >&2
            echo "      ${REVIEW_STALE_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac

    # Review LT1-R, R2/R5. At 960x640 the panel gets less height than the review
    # block asks for, and the row that admits the truncation used to be the first
    # thing the scissor ate. The tail is now fitted before the names are, so at
    # this size it is the only review row there is.
    REVIEW_TAIL_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-tail.txt")"
    case "$REVIEW_TAIL_LINE" in
        *" tail=yes "*) ;;
        *)
            echo "FAIL: the 960x640 panel drew no tail row: ${REVIEW_TAIL_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    case "$REVIEW_TAIL_LINE" in
        *" listed=4 rows_drawn=0 "*) ;;
        *)
            echo "FAIL: the 960x640 panel did not report what it actually drew:" >&2
            echo "      ${REVIEW_TAIL_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    # And the pixels, because a report line is a claim about a frame. 20,602 is
    # the tail row inside the panel at 960x640; 20,615 is the strip between it
    # and the panel's own border, which must stay empty -- a row drawn there
    # would be one the scissor cuts mid-glyph.
    REVIEW_TAIL_INK="$(assist_review_luma_min assist-review-tail 520:14:20:602)"
    REVIEW_TAIL_CLEAR="$(assist_review_luma_min assist-review-tail 520:12:20:615)"
    echo "review tail at 960x640: ink=${REVIEW_TAIL_INK:-?} below=${REVIEW_TAIL_CLEAR:-?} (control)"
    if [ "${REVIEW_TAIL_INK%%.*}" -ge 200 ] 2>/dev/null; then
        echo "FAIL: the tail row is reported but not on screen at 960x640" >&2
        SWEEP_FAILED=1
    fi
    if [ "${REVIEW_TAIL_CLEAR%%.*}" -lt 200 ] 2>/dev/null; then
        echo "FAIL: a review row was drawn into the panel's clipped edge" >&2
        SWEEP_FAILED=1
    fi

    # Review LT1-R, R11. The same job folder staged as a lyrics-only run: one
    # lane line instead of three, which is the arrangement the review surface
    # actually ships in and the one nothing could photograph.
    REVIEW_LYRICS_ONLY_LINE="$(sed -n 's/^assist review: *//p' \
        "$OUT_DIR/assist-review-lyrics-only.txt")"
    case "$REVIEW_LYRICS_ONLY_LINE" in
        *"rows_drawn=4 tail=no"*) ;;
        *)
            echo "FAIL: the lyrics-only candidate drew no review:" >&2
            echo "      ${REVIEW_LYRICS_ONLY_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    case "$(sed -n 's/^assist: *//p' "$OUT_DIR/assist-review-lyrics-only.txt")" in
        *"mode=lyrics"*) ;;
        *)
            echo "FAIL: MUSIALIZER_ASSIST_PROBE_LANES=lyrics did not select the lyrics mode" >&2
            SWEEP_FAILED=1
            ;;
    esac

    # Absence is not zero. A pre-LT1 job folder must reach the same panel it
    # always reached, which is also why this capture is the saturation control.
    REVIEW_LEGACY_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-legacy.txt")"
    case "$REVIEW_LEGACY_LINE" in
        "absent"*) ;;
        *)
            echo "FAIL: a pre-LT1 manifest invented a review: ${REVIEW_LEGACY_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    # The named rows, at the panel's default scale. 20,588 300x82 is the block
    # below the Apply/Discard row at 1280x720; `clear` draws two shorter rows and
    # gets its own tighter crop.
    REVIEW_SAT_FLAGGED="$(assist_review_saturation assist-review-flagged 300:82:20:588)"
    REVIEW_SAT_CLEAR="$(assist_review_saturation assist-review-clear 300:36:20:632)"
    REVIEW_SAT_LEGACY_A="$(assist_review_saturation assist-review-legacy 300:82:20:588)"
    REVIEW_SAT_LEGACY_B="$(assist_review_saturation assist-review-legacy 300:36:20:632)"
    echo "review block saturation: flagged=${REVIEW_SAT_FLAGGED:-?} clear=${REVIEW_SAT_CLEAR:-?}" \
         "legacy=${REVIEW_SAT_LEGACY_A:-?}/${REVIEW_SAT_LEGACY_B:-?} (control)"
    if [ "${REVIEW_SAT_FLAGGED%%.*}" -lt 3 ] 2>/dev/null; then
        echo "FAIL: the named review rows were not drawn where the panel says they are" >&2
        SWEEP_FAILED=1
    fi
    if [ "${REVIEW_SAT_CLEAR%%.*}" -lt 2 ] 2>/dev/null; then
        echo "FAIL: the 'all lines placed' state drew nothing" >&2
        SWEEP_FAILED=1
    fi
    if [ "${REVIEW_SAT_LEGACY_A%%.*}" -ge 3 ] 2>/dev/null \
        || [ "${REVIEW_SAT_LEGACY_B%%.*}" -ge 2 ] 2>/dev/null; then
        echo "FAIL: the control is not a control; a pre-LT1 panel drew review text" >&2
        SWEEP_FAILED=1
    fi
    # And the three states are three pictures, not one picture three times.
    REVIEW_SHOTS="$(sha256sum "$OUT_DIR/assist-review-flagged.png" \
        "$OUT_DIR/assist-review-clear.png" "$OUT_DIR/assist-review-legacy.png" \
        | cut -d' ' -f1 | sort -u | wc -l)"
    if [ "$REVIEW_SHOTS" -ne 3 ]; then
        echo "FAIL: the three LT1 review captures are not three distinct frames" >&2
        SWEEP_FAILED=1
    fi
    # Review LT1-R, R1, as a frame rather than as a sentence: a run with no
    # lyrics lane must render **exactly** the panel a pre-LT1 job folder does,
    # and must not render the one its neighbouring document would produce.
    if ! cmp -s "$OUT_DIR/assist-review-stale.png" "$OUT_DIR/assist-review-legacy.png"; then
        echo "FAIL: a no-lyrics-lane manifest did not render the pre-LT1 panel" >&2
        SWEEP_FAILED=1
    fi
    if cmp -s "$OUT_DIR/assist-review-stale.png" "$OUT_DIR/assist-review-flagged.png"; then
        echo "FAIL: a no-lyrics-lane run drew the review of the run before it" >&2
        SWEEP_FAILED=1
    fi
    # And the lyrics-only frame is its own picture: one lane line, not three.
    if cmp -s "$OUT_DIR/assist-review-lyrics-only.png" "$OUT_DIR/assist-review-flagged.png"; then
        echo "FAIL: MUSIALIZER_ASSIST_PROBE_LANES=lyrics changed nothing on screen" >&2
        SWEEP_FAILED=1
    fi
    # The stale panel is the second saturation control: it holds `flagged`'s own
    # lyric document, so any coloured review text in the block is that document
    # being drawn by a run that never had a lyrics lane.
    REVIEW_SAT_STALE="$(assist_review_saturation assist-review-stale 300:82:20:588)"
    echo "review block saturation: stale=${REVIEW_SAT_STALE:-?} (control)"
    if [ "${REVIEW_SAT_STALE%%.*}" -ge 3 ] 2>/dev/null; then
        echo "FAIL: a run with no lyrics lane drew the previous run's review rows" >&2
        SWEEP_FAILED=1
    fi

    # -----------------------------------------------------------------------
    # Tranche LX1-f: proposals reach the lane, and the list scrolls.
    #
    # Two complaints, one fixture. Ten unresolved lines with in-range coarse
    # proposals: every one of them becomes a `Potential` cue on the timeline (the
    # 37-second hole), and ten of them do not fit in three rows (the "shows three
    # lines max, rest refers to file").
    # -----------------------------------------------------------------------
    echo "--- LX1-f: parked proposals and a scrolling review list ---"
    assist_review_capture assist-review-parked "$ASSIST_REVIEW_DIR/parked" || SWEEP_FAILED=1
    REVIEW_SCROLL=5 assist_review_capture assist-review-scrolled \
        "$ASSIST_REVIEW_DIR/parked" || SWEEP_FAILED=1

    REVIEW_PARKED_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-parked.txt")"
    echo "parked: ${REVIEW_PARKED_LINE:-<absent>}"
    case "$REVIEW_PARKED_LINE" in
        *"listed=10 rows_drawn=3 tail=yes first=0 omitted=0 parked=10/10/10 "*) ;;
        *)
            echo "FAIL: ten in-range proposals did not reach the staged lane:" >&2
            echo "      ${REVIEW_PARKED_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    # `parked=10/...` is a count of `CueOrigin::Potential` cues in the *staged
    # lane*, not of review rows, so the assertion above is already evidence that
    # the proposals reached the document Apply will publish.

    # And the scroll. `first=5` is the whole point: before LX1-f the list showed
    # rows 0..2 and told the user to go and read a JSON file for the rest.
    REVIEW_SCROLLED_LINE="$(sed -n 's/^assist review: *//p' "$OUT_DIR/assist-review-scrolled.txt")"
    echo "scrolled: ${REVIEW_SCROLLED_LINE:-<absent>}"
    case "$REVIEW_SCROLLED_LINE" in
        *"rows_drawn=3 tail=yes first=5 "*) ;;
        *)
            echo "FAIL: the review list did not scroll:" >&2
            echo "      ${REVIEW_SCROLLED_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac
    # A report line is a claim about a frame, so the two frames must differ.
    if cmp -s "$OUT_DIR/assist-review-parked.png" "$OUT_DIR/assist-review-scrolled.png"; then
        echo "FAIL: scrolling the review list changed nothing on screen" >&2
        SWEEP_FAILED=1
    fi
    # Both frames drew coloured review rows where the panel says they are. The
    # scrolled one is the interesting half: a list that scrolled its data and not
    # its pixels would pass the line check and fail here.
    REVIEW_SAT_PARKED="$(assist_review_saturation assist-review-parked 300:82:20:588)"
    REVIEW_SAT_SCROLLED="$(assist_review_saturation assist-review-scrolled 300:82:20:588)"
    echo "review block saturation: parked=${REVIEW_SAT_PARKED:-?} scrolled=${REVIEW_SAT_SCROLLED:-?}"
    if [ "${REVIEW_SAT_PARKED%%.*}" -lt 3 ] 2>/dev/null \
        || [ "${REVIEW_SAT_SCROLLED%%.*}" -lt 3 ] 2>/dev/null; then
        echo "FAIL: a scrolled review list drew no rows" >&2
        SWEEP_FAILED=1
    fi
    # The Open folder control, on the heading row's right edge. Disabled here --
    # a probe run never spawns a file manager, whatever the display says -- so
    # what this measures is that a *box* was drawn, which is the difference
    # between a control that explains itself and a blank corner.
    REVEAL_INK="$(assist_review_luma_min assist-review-parked 92:16:1168:589)"
    REVEAL_CONTROL="$(assist_review_luma_min assist-review-legacy 92:16:1168:589)"
    echo "review open-folder button: ink=${REVEAL_INK:-?} bare=${REVEAL_CONTROL:-?} (control)"
    if [ "${REVEAL_INK%%.*}" -ge 200 ] 2>/dev/null; then
        echo "FAIL: the Open folder control is not on screen" >&2
        SWEEP_FAILED=1
    fi
    if [ "${REVEAL_CONTROL%%.*}" -lt 200 ] 2>/dev/null; then
        echo "FAIL: the control is not a control; a pre-LT1 panel drew a folder button" >&2
        SWEEP_FAILED=1
    fi

    # -----------------------------------------------------------------------
    # Review LT1-R, R9: the route from a flagged row to where it gets fixed.
    #
    # Two things a capture can prove and a unit test cannot -- that the row is
    # *reachable* (it lights under the pointer and names itself) and that the
    # Lyrics panel really arrives with the cue bound. Which cue it bound is the
    # other way round, and lives in `assist.rs`'s own tests.
    # -----------------------------------------------------------------------
    echo "--- LT1-R R9: review row navigation ---"

    # Where the rows landed, published by the panel rather than measured off a
    # screenshot once and then trusted forever. A re-layout that moves them makes
    # the hover below miss, and this is the line that says so instead.
    REVIEW_ROWS_LINE="$(sed -n 's/^assist review rows: *//p' \
        "$OUT_DIR/assist-review-flagged.txt" | head -1)"
    echo "review rows: ${REVIEW_ROWS_LINE:-<absent>}"
    case "$REVIEW_ROWS_LINE" in
        'x=20 y=609 width=1240 pitch=15 rows=4 hint="Click a line to open it in Lyrics"') ;;
        *)
            echo "FAIL: the review rows are not where the hover below aims:" >&2
            echo "      ${REVIEW_ROWS_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
            ;;
    esac

    # 200x614 is the middle of the first row -- `y=609` less the 2 px the hit box
    # starts above the ink, plus half a 15 px pitch.
    assist_review_capture assist-review-hover "$ASSIST_REVIEW_DIR/flagged" \
        1280x720 "" ",hover=200x614" || SWEEP_FAILED=1
    # The tip is white on near-black over a panel that is otherwise 247 flat, so
    # one crop carries both halves: a dark minimum is the box, a bright maximum
    # is the text inside it. `assist-review-flagged` is the same frame without a
    # pointer and is the control.
    ROW_TIP="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/assist-review-hover.png,crop=300:16:480:581,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN,lavfi.signalstats.YMAX \
        -of csv=p=0 2>/dev/null | head -1)"
    ROW_TIP_CONTROL="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/assist-review-flagged.png,crop=300:16:480:581,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
    echo "review row tooltip: box/text=${ROW_TIP:-?} bare=${ROW_TIP_CONTROL:-?} (control)"
    if [ "${ROW_TIP%%,*}" -ge 100 ] 2>/dev/null \
        || [ "${ROW_TIP##*,}" -lt 180 ] 2>/dev/null; then
        echo "FAIL: no tooltip where the first review row's tip should be" >&2
        SWEEP_FAILED=1
    fi
    if [ "${ROW_TIP_CONTROL%%.*}" -lt 200 ] 2>/dev/null; then
        echo "FAIL: the control is not a control; a bare panel drew a tooltip" >&2
        SWEEP_FAILED=1
    fi
    # And the row itself lights. Two controls, because one is not enough here:
    # the *same* row in the frame with no pointer, and the row below it in this
    # frame -- a highlight that painted every row would pass the first alone.
    ROW_FILL="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/assist-review-hover.png,crop=400:12:700:608,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
    ROW_FILL_NEIGHBOUR="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/assist-review-hover.png,crop=400:12:700:623,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
    ROW_FILL_BARE="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/assist-review-flagged.png,crop=400:12:700:608,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
    echo "review row hover fill: row=${ROW_FILL:-?} neighbour=${ROW_FILL_NEIGHBOUR:-?}" \
         "bare=${ROW_FILL_BARE:-?} (controls)"
    if [ "${ROW_FILL%%.*}" -ge 245 ] 2>/dev/null; then
        echo "FAIL: the hovered review row drew no highlight" >&2
        SWEEP_FAILED=1
    fi
    if [ "${ROW_FILL_NEIGHBOUR%%.*}" -lt 245 ] 2>/dev/null \
        || [ "${ROW_FILL_BARE%%.*}" -lt 245 ] 2>/dev/null; then
        echo "FAIL: the row highlight is not a hover state; it painted a row nobody pointed at" >&2
        SWEEP_FAILED=1
    fi

    # The press, over a project that has cues. Row 1 is the CHECK line whose text
    # a seeded cue carries; row 0 is the unresolved one, which must land at the
    # proposal without borrowing somebody else's cue.
    if [ ! -f "$LYRIC_PROJECT" ]; then
        echo "SKIPPED: no lyric fixture project, so the R9 press has no cues to bind"
    else
        assist_review_navigate_capture assist-nav-cue 1 || SWEEP_FAILED=1
        assist_review_navigate_capture assist-nav-unplaced 0 || SWEEP_FAILED=1

        NAV_CUE_LINE="$(sed -n 's/^assist review nav: *//p' "$OUT_DIR/assist-nav-cue.txt" \
            | head -1)"
        if [ "$NAV_CUE_LINE" != "line 2 -> cue 2 at 0:01.6 (Lyrics panel)" ]; then
            echo "FAIL: pressing a CHECK row did not bind that line's own cue:" >&2
            echo "      ${NAV_CUE_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
        fi
        # The claim, checked against the panel that would have to honour it. This
        # line is the Lyrics panel's own, so it cannot agree with the Assist
        # panel by construction.
        NAV_CUE_LYRICS="$(sed -n 's/^lyrics: *//p' "$OUT_DIR/assist-nav-cue.txt" | head -1)"
        echo "review row press:   $NAV_CUE_LYRICS"
        case "$NAV_CUE_LYRICS" in
            *"pane cues"*"selected #2 at 00:01.650"*) ;;
            *)
                echo "FAIL: the Lyrics panel did not arrive bound to the pressed row's cue:" >&2
                echo "      ${NAV_CUE_LYRICS:-<absent>}" >&2
                SWEEP_FAILED=1
                ;;
        esac

        NAV_UNPLACED_LINE="$(sed -n 's/^assist review nav: *//p' \
            "$OUT_DIR/assist-nav-unplaced.txt" | head -1)"
        if [ "$NAV_UNPLACED_LINE" != "line 10 -> no cue; Lyrics panel at the proposed 0:05.2" ]
        then
            echo "FAIL: pressing an UNPLACED row did not land at its proposal:" >&2
            echo "      ${NAV_UNPLACED_LINE:-<absent>}" >&2
            SWEEP_FAILED=1
        fi
        # The half that matters most: a line with no cue must select nothing.
        # Binding a neighbouring cue would look right in every screenshot.
        NAV_UNPLACED_LYRICS="$(sed -n 's/^lyrics: *//p' "$OUT_DIR/assist-nav-unplaced.txt" \
            | head -1)"
        echo "review row press:   $NAV_UNPLACED_LYRICS"
        case "$NAV_UNPLACED_LYRICS" in
            *"selected none"*) ;;
            *)
                echo "FAIL: an unresolved line's row bound a cue that is not its own:" >&2
                echo "      ${NAV_UNPLACED_LYRICS:-<absent>}" >&2
                SWEEP_FAILED=1
                ;;
        esac
        # Two presses, two pictures. A press that changed nothing on screen would
        # otherwise pass every assertion above through the report line alone.
        if cmp -s "$OUT_DIR/assist-nav-cue.png" "$OUT_DIR/assist-nav-unplaced.png"; then
            echo "FAIL: the two review-row presses produced the same frame" >&2
            SWEEP_FAILED=1
        fi
    fi
fi

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

# The plan's interactive control lives in the Assist header. Seed a disabled
# two-cue plan, then capture both header states at a playhead in the second cue.
# `scene:` is the state beyond the label: disabled restores the saved base scene,
# while enabled drives the same preview frame through Loom.
AUTO_SCENE_DIR="$OUT_DIR/auto-scenes"
rm -rf "$AUTO_SCENE_DIR"
mkdir -p "$AUTO_SCENE_DIR"
if [ ! -f "$LYRIC_PROJECT" ]; then
    echo "FAIL: the generated project needed for the Auto-scenes gate is unavailable" >&2
    SWEEP_FAILED=1
else
    cp "$LYRIC_PROJECT" "$AUTO_SCENE_DIR/plan.musi"
    cp "$LYRIC_DIR/source.wav" "$AUTO_SCENE_DIR/source.wav"
    # The generated project publishes its audio under this asset root; preserve
    # that content-addressed path when the project document is copied.
    cp -a "$LYRIC_DIR/cues.assets" "$AUTO_SCENE_DIR/cues.assets"
    python3 tools/seed_lyric_fixture.py "$AUTO_SCENE_DIR/plan.musi" \
        --scene constellation --scene-plan >"$AUTO_SCENE_DIR/seed.txt"

    auto_scene_capture() {
        # auto_scene_capture NAME [extra application arguments]
        local name="$1"
        shift
        local shot="$AUTO_SCENE_DIR/$name.png"
        local log="$AUTO_SCENE_DIR/$name.txt"
        local probe="panel=assist,time=5,play=0"
        if [ "$name" = "zoomed" ]; then
            probe="panel=none,time=5,play=0,zoom=4"
        fi
        # LX3-a. The one gesture the operator's two reports were about: a scene
        # click while an automatic plan is running. The frozen C answers it by
        # making the scene the track's base and switching the plan off as a side
        # effect, which is what turned the whole track into one scene and sent
        # every later tuning edit into the shared per-scene table. Here it
        # retargets the segment under the playhead and the plan keeps driving.
        if [ "$name" = "retarget" ]; then
            probe="panel=tune,time=5,play=0,scene-pick=pentagram"
        fi
        set +e
        env -u WAYLAND_DISPLAY \
            DISPLAY="$DISPLAY_NUM" \
            PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
            ./target/debug/musializer --mute --project "$AUTO_SCENE_DIR/plan.musi" \
                "$@" \
                --size 1280x720 \
                --probe-frames 12 \
                --probe-shot "$shot" \
                --ui-probe "$probe" \
            >"$log" 2>&1
        local status=$?
        set -e
        local state scene segments
        state="$(sed -n 's/^auto scenes: *//p' "$log" | head -1)"
        scene="$(sed -n 's/^scene: *//p' "$log" | head -1)"
        segments="$(sed -n 's/^scene segments: *//p' "$log" | head -1)"
        echo "$name: exit=$status state=${state:-<absent>} scene=${scene:-<absent>} segments=${segments:-<absent>}"
        if [ "$status" -ne 0 ] || [ ! -f "$shot" ]; then
            return 1
        fi
    }

    auto_scene_capture disabled || SWEEP_FAILED=1
    auto_scene_capture enabled --auto-scenes || SWEEP_FAILED=1
    auto_scene_capture zoomed --auto-scenes || SWEEP_FAILED=1
    auto_scene_capture retarget --auto-scenes || SWEEP_FAILED=1

    DISABLED_STATE="$(sed -n 's/^auto scenes: *//p' "$AUTO_SCENE_DIR/disabled.txt" | head -1)"
    ENABLED_STATE="$(sed -n 's/^auto scenes: *//p' "$AUTO_SCENE_DIR/enabled.txt" | head -1)"
    DISABLED_SCENE="$(sed -n 's/^scene: *//p' "$AUTO_SCENE_DIR/disabled.txt" | head -1)"
    ENABLED_SCENE="$(sed -n 's/^scene: *//p' "$AUTO_SCENE_DIR/enabled.txt" | head -1)"
    ZOOMED_STATE="$(sed -n 's/^auto scenes: *//p' "$AUTO_SCENE_DIR/zoomed.txt" | head -1)"
    if [ "$DISABLED_STATE" != "disabled (2 cues)" ] \
        || [ "$DISABLED_SCENE" != "constellation (Constellation)" ]; then
        echo "FAIL: disabled Auto-scenes did not retain the plan and base scene" >&2
        SWEEP_FAILED=1
    fi
    if [ "$ENABLED_STATE" != "enabled (2 cues)" ] \
        || [ "$ENABLED_SCENE" != "loom (Loom)" ]; then
        echo "FAIL: enabled Auto-scenes did not drive the parked preview frame" >&2
        SWEEP_FAILED=1
    fi
    if [ "$ZOOMED_STATE" != "enabled (2 cues)" ]; then
        echo "FAIL: the zoomed scene-lane capture lost its retained plan" >&2
        SWEEP_FAILED=1
    fi

    # LX3-a, and the assertion is the *state*, not the scene. Both the old
    # behaviour and the new one leave Pentagram on screen at t=5 — one because
    # it became the base scene with the plan switched off, the other because the
    # segment the playhead is inside now carries it. The frame is a plausible
    # picture of either, which is why this gate reads three lines instead.
    RETARGET_STATE="$(sed -n 's/^auto scenes: *//p' "$AUTO_SCENE_DIR/retarget.txt" | head -1)"
    RETARGET_SCENE="$(sed -n 's/^scene: *//p' "$AUTO_SCENE_DIR/retarget.txt" | head -1)"
    RETARGET_SEGMENTS="$(sed -n 's/^scene segments: *//p' "$AUTO_SCENE_DIR/retarget.txt" | head -1)"
    ENABLED_SEGMENTS="$(sed -n 's/^scene segments: *//p' "$AUTO_SCENE_DIR/enabled.txt" | head -1)"
    RETARGET_SCOPE="$(sed -n 's/^tune scope: *//p' "$AUTO_SCENE_DIR/retarget.txt" | head -1)"
    echo "scene pick: state=$RETARGET_STATE scene=$RETARGET_SCENE segments=$RETARGET_SEGMENTS"
    if [ "$ENABLED_SEGMENTS" != "spectrum, loom" ]; then
        echo "FAIL: the untouched plan did not start as spectrum, loom" >&2
        SWEEP_FAILED=1
    fi
    if [ "$RETARGET_STATE" != "enabled (2 cues)" ]; then
        echo "FAIL: a scene click switched the running plan off (state=$RETARGET_STATE)" >&2
        SWEEP_FAILED=1
    fi
    if [ "$RETARGET_SEGMENTS" != "spectrum, pentagram" ]; then
        echo "FAIL: the scene click did not land on the playhead's segment alone (segments=$RETARGET_SEGMENTS)" >&2
        SWEEP_FAILED=1
    fi
    if [ "$RETARGET_SCENE" != "pentagram (Pentagram Orbits)" ]; then
        echo "FAIL: the retargeted segment is not the one being previewed" >&2
        SWEEP_FAILED=1
    fi
    # LX3-b, both branches of it. The seeded cues carry no snapshot, so the
    # untouched plan's header has to say so — an edit there really does go to
    # the shared per-scene table, which is the sentence that was missing. A
    # retarget captures one on the way through, so the same header then names
    # the segment. Prefix-matched on the retarget side because the segment
    # boundary is the fixture's own midpoint and is not a constant here.
    ENABLED_SCOPE="$(sed -n 's/^tune scope: *//p' "$AUTO_SCENE_DIR/enabled.txt" | head -1)"
    echo "tune scope: enabled='$ENABLED_SCOPE' retarget='$RETARGET_SCOPE'"
    if [ "$ENABLED_SCOPE" != "Editing: base scene - this segment captured no tuning" ]; then
        echo "FAIL: the Tune scope line did not name the segment's uncaptured state (scope=$ENABLED_SCOPE)" >&2
        SWEEP_FAILED=1
    fi
    case "$RETARGET_SCOPE" in
        "Editing: segment 2 of 2 ("*) ;;
        *)
            echo "FAIL: the Tune scope line did not name the retargeted segment (scope=$RETARGET_SCOPE)" >&2
            SWEEP_FAILED=1
            ;;
    esac
    if [ "$(sha256sum "$AUTO_SCENE_DIR/disabled.png" | cut -d' ' -f1)" \
        = "$(sha256sum "$AUTO_SCENE_DIR/enabled.png" | cut -d' ' -f1)" ]; then
        echo "FAIL: enabled and disabled Auto-scenes captures were identical" >&2
        SWEEP_FAILED=1
    fi
    # At this fixed 1280x720 layout, y=404 is inside the 24 px scene lane and
    # outside both its controls and the waveform. Saturation proves the two-cue
    # plan is actually painted; a plausible empty strip or a missing lane is
    # white here and reads zero. Disabled stays visible but deliberately muted.
    ENABLED_LANE_SAT="$(ffprobe -v error -f lavfi \
        -i "movie=$AUTO_SCENE_DIR/enabled.png,crop=1000:12:100:404,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.SATAVG -of csv=p=0 2>/dev/null | head -1)"
    DISABLED_LANE_SAT="$(ffprobe -v error -f lavfi \
        -i "movie=$AUTO_SCENE_DIR/disabled.png,crop=1000:12:100:404,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.SATAVG -of csv=p=0 2>/dev/null | head -1)"
    echo "scene lane saturation: enabled=${ENABLED_LANE_SAT:-?} disabled=${DISABLED_LANE_SAT:-?}"
    if [ "${ENABLED_LANE_SAT%%.*}" -lt 30 ] 2>/dev/null \
        || [ "${DISABLED_LANE_SAT%%.*}" -lt 10 ] 2>/dev/null; then
        echo "FAIL: the editable scene-plan lane was not visibly drawn in both states" >&2
        SWEEP_FAILED=1
    fi
fi

# `assist=confirm` without `panel=assist` must be refused rather than quietly
# arming a step in a panel nobody can see (`musializer.c:128-130`).
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$FIXTURE" \
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

echo "=== the AI settings dialog (AP3) ==="
# None of this is in the frozen C either, so a capture and a report line are the
# only evidence the surface does what it claims. Everything the dialog reads is
# a fixture written here: the settings file, the credentials file, both
# discovery caches and a doctor report. No network is reachable from any run —
# `MUSIALIZER_ASSIST_SETTINGS_KEY_TEST` stubs the one control that could open a
# socket, and `catalog.network_allowed` gates the other.
AI_DIR="$OUT_DIR/ai-settings"
AI_CANARY="sk-or-v1-MUSICANARY7Q4X2ZK9"
rm -rf "$AI_DIR"
mkdir -p "$AI_DIR/config" "$AI_DIR/cache/musializer" "$AI_DIR/corrupt" "$AI_DIR/loose"
AI_FAILED=0

python3 - "$AI_DIR" <<'PYFIXTURES'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
# A settings file with one per-task override, so the routing matrix has both a
# recommended row and an override row in the same picture.
(root / "config/assist.json").write_text(json.dumps({
    "schema": "musializer.assist-settings/v1",
    "active_profile": "custom",
    "profiles": [{"id": "custom", "label": "Custom", "routes": {
        "TC-COARSE": {"contract": "TC-COARSE", "route_type": "local-proc",
                       "runtime_id": "whisper.cpp", "model_id": "whisper.cpp",
                       "fallback": "ask"}}}],
    "catalog": {"network_allowed": True,
                 "last_filters": {"input_modalities": "audio", "output_modalities": "text"},
                 "last_refresh_utc": "2026-07-20T09:00:00Z"},
}, indent=2) + "\n")
(root / "corrupt/assist.json").write_text("{ broken")
# A credentials file with the loose mode the read path must refuse rather than
# repair. The secret is the canary, so a leak anywhere is greppable.
(root / "loose/credentials.json").write_text(json.dumps({
    "schema": "musializer.assist-credentials/v1",
    "entries": {"openrouter/default": {"secret": "sk-or-v1-MUSICANARY7Q4X2ZK9",
                                        "label": "Fixture"}},
}, indent=2) + "\n")
(root / "loose/credentials.json").chmod(0o644)

cache = root / "cache/musializer"
models = [
    {"id": "xiaomi/mimo-v2.5", "name": "MiMo v2.5", "context_length": 131072,
     "input_modalities": ["audio", "text"], "output_modalities": ["text"],
     "pricing": {"prompt": "0.0000004", "completion": "0.0000012", "audio": "0.0000090"}},
    {"id": "acme/never-measured", "name": "Acme Audio 1", "context_length": 32000,
     "input_modalities": ["audio"], "output_modalities": ["text"],
     "pricing": {"prompt": "0.0000010", "completion": None, "audio": None}},
    # Every optional field absent, so "not reported" is in the picture rather
    # than only in a test.
    {"id": "acme/quiet", "name": "Acme Quiet", "context_length": None,
     "input_modalities": ["audio"], "output_modalities": ["text"],
     "pricing": {"prompt": None, "completion": None, "audio": None}},
]
(cache / "openrouter-models-v1.json").write_text(json.dumps({
    "schema_version": "musializer.openrouter-catalog/v1",
    "source_url": "https://openrouter.ai/api/v1/models",
    "fetched_at_utc": "2026-07-20T09:00:00Z", "validated_at_utc": "2026-07-20T09:00:00Z",
    "filters": {"input_modalities": "audio", "output_modalities": "text"},
    "model_count": len(models), "unfiltered_model_count": 340, "models": models,
}, indent=2, sort_keys=True) + "\n")
(cache / "codex-models-v1.json").write_text(json.dumps({
    "schema_version": "musializer.codex-model-catalog/v1",
    "source": "codex app-server model/list", "fetched_at_utc": "2026-08-04T18:00:00Z",
    "codex_bin": "codex", "model_count": 2, "models": [
        {"id": "gpt-5.3-codex", "model": "gpt-5.3-codex", "display_name": "GPT-5.3 Codex",
         "is_default": True, "hidden": False, "default_reasoning_effort": "medium",
         "supported_reasoning_efforts": [{"reasoning_effort": e} for e in ("low", "medium", "high")],
         "input_modalities": ["text"]},
        {"id": "gpt-5.3-codex-mini", "model": "gpt-5.3-codex-mini",
         "display_name": "GPT-5.3 Codex mini", "is_default": False, "hidden": False,
         "default_reasoning_effort": "low",
         "supported_reasoning_efforts": [{"reasoning_effort": "low"}],
         "input_modalities": ["text"]},
    ],
}, indent=2, sort_keys=True) + "\n")
(cache / "doctor.json").write_text(json.dumps({
    "schema_version": "musializer.doctor/v1", "runtimes": {
        "whisper": {"state": "available", "path": "/usr/local/bin/whisper-cli",
                     "version": "whisper-cli 1.8.6",
                     "model_path": "/opt/models/ggml-large-v3.bin", "model_sha256": "ab12cd34ef56",
                     "language_support": "multilingual (99 languages)", "gpu_ready": True,
                     "remediation": None},
        "mms_ctc_aligner": {"state": "available", "path": "/opt/lyrics-align/bin/python",
                             "version": "torchaudio 2.6.0", "model_path": "/opt/models/mms-ctc",
                             "model_sha256": None,
                             "language_support": "romanized text normalized to the MMS_FA alphabet",
                             "gpu_ready": False, "remediation": None},
        # Deliberately missing, so the picture carries a remediation string.
        "stem_separator": {"state": "unavailable", "path": None, "version": None,
                            "model_path": None, "model_sha256": None, "language_support": None,
                            "gpu_ready": None,
                            "remediation": "pip install demucs into the alignment venv"},
    },
}, indent=2, sort_keys=True) + "\n")
PYFIXTURES

# ai_capture NAME SIZE [VAR=VALUE ...] -- [extra musializer args]
#
# The clock is pinned so a cache age badge is the same number in every run, and
# the key test is stubbed so no capture can reach the network.
ai_capture() {
    local name="$1" size="$2"
    shift 2
    local extra_env=()
    while [ $# -gt 0 ] && [ "$1" != "--" ]; do extra_env+=("$1"); shift; done
    [ "${1:-}" = "--" ] && shift
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -u WAYLAND_DISPLAY -u MUSIALIZER_ASSIST_HELPER \
        DISPLAY="$DISPLAY_NUM" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        MUSIALIZER_ASSIST_SETTINGS="$REPO_ROOT/$AI_DIR/config/assist.json" \
        MUSIALIZER_ASSIST_CREDENTIALS="$REPO_ROOT/$AI_DIR/config/credentials.json" \
        XDG_CACHE_HOME="$REPO_ROOT/$AI_DIR/cache" \
        MUSIALIZER_ASSIST_SETTINGS_NOW=2026-08-05T12:00:00Z \
        MUSIALIZER_ASSIST_SETTINGS_KEY_TEST=no-key \
        MUSIALIZER_ASSIST_DISCOVERY=off \
        "${extra_env[@]}" \
        ./target/debug/musializer --mute "$FIXTURE" \
            --size "$size" \
            --probe-frames 20 \
            --probe-shot "$out" \
            "$@" \
        >"$log" 2>&1
    local status=$?
    set -e
    printf '%-30s %-9s exit=%s\n' "$name" "$size" "$status"
    if [ ! -f "$out" ] || [ "$status" -ne 0 ]; then
        echo "FAIL: $name produced no frame, or exited $status" >&2
        return 1
    fi
    # E4/E9: no capture, and no log, may ever contain a credential.
    if grep -q "MUSICANARY" "$log"; then
        echo "FAIL: $name leaked the credential canary into its report" >&2
        return 1
    fi
    return 0
}

ai_report() {
    # ai_report LOG PREFIX -- print the dialog's own evidence line
    sed -n "s/^$2//p" "$OUT_DIR/$1.txt" | head -1
}

# 1. The entry point, in every panel state the probe can reach and at the
#    narrowest supported window. The button is drawn from the heading row before
#    the body branches, so "present in all six states" is a report assertion
#    rather than six pictures somebody looked at.
for state in ready confirm running candidate failed; do
    spec="panel=assist,play=0"
    [ "$state" = "ready" ] || spec="$spec,assist=$state"
    ai_capture "ai-entry-$state" 1280x720 -- --ui-probe "$spec" || AI_FAILED=1
    line="$(sed -n 's/^assist settings button: //p' "$OUT_DIR/ai-entry-$state.txt" | head -1)"
    echo "  entry $state: ${line:-<absent>}"
    case "$line" in
        'visible=true label="AI settings"'*) ;;
        *) echo "FAIL: the AI settings button is missing in the $state body" >&2; AI_FAILED=1 ;;
    esac
    case "$line" in
        *icons=on*) ;;
        *) echo "FAIL: the $state capture drew the entry point without its icon face" >&2; AI_FAILED=1 ;;
    esac
done
ai_capture "ai-entry-narrow" 960x640 -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
NARROW_ENTRY="$(sed -n 's/^assist settings button: //p' "$OUT_DIR/ai-entry-narrow.txt" | head -1)"
echo "  entry narrow: ${NARROW_ENTRY:-<absent>}"
case "$NARROW_ENTRY" in
    'visible=true label="AI settings"'*) ;;
    *) echo "FAIL: the AI settings button is missing at the minimum window" >&2; AI_FAILED=1 ;;
esac

# 2. Each of the five sections, plus one at 150% and one at the minimum window.
for section in routing local codex openrouter privacy; do
    ai_capture "ai-$section" 1280x720 \
        "MUSIALIZER_ASSIST_SETTINGS_OPEN=$section" \
        "MUSIALIZER_ASSIST_SETTINGS_DOCTOR=$REPO_ROOT/$AI_DIR/cache/musializer/doctor.json" \
        -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
    got="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-$section.txt" | head -1)"
    echo "  $got"
    case "$got" in
        "section=$section "*) ;;
        *) echo "FAIL: ai-$section opened on the wrong section: $got" >&2; AI_FAILED=1 ;;
    esac
done
# One section at 150%. 1920 physical is 1280 logical, which is the smallest
# window that keeps 150% above the shell's own logical minimum — 1280 at 150%
# would be 853 logical and the scale is refused.
ai_capture "ai-local-150" 1920x1080 MUSIALIZER_ASSIST_SETTINGS_OPEN=local \
    "MUSIALIZER_ASSIST_SETTINGS_DOCTOR=$REPO_ROOT/$AI_DIR/cache/musializer/doctor.json" \
    -- --ui-scale 150 --ui-probe "panel=assist,play=0" || AI_FAILED=1
ai_capture "ai-openrouter-small" 960x640 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
# And the two honest degradations, named in the report rather than left to the
# eye: below 882 px of dialog the section list becomes a top row of tabs and the
# routing matrix stacks each contract onto two lines. Both are decided from the
# window size, and a capture cannot tell a deliberate stack from a broken table.
ai_capture "ai-routing-narrow" 800x640 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
for pair in "ai-routing:sections=rail routing=matrix" "ai-routing-narrow:sections=tabs routing=stacked"; do
    want="${pair#*:}"
    got="$(sed -n 's/^assist settings: //p' "$OUT_DIR/${pair%%:*}.txt" | head -1)"
    case "$got" in
        *"$want"*) echo "  ${pair%%:*}: $want" ;;
        *) echo "FAIL: ${pair%%:*} expected $want, got: $got" >&2; AI_FAILED=1 ;;
    esac
done

# 2c. The bottom of the two tallest sections (AP3-R S10).
#     `MUSIALIZER_ASSIST_SETTINGS_SCROLL` used to be applied at open, in front of
#     a clamp against a content height nothing had measured yet, so every value
#     clamped to zero and **no section bottom had ever been photographed**. It is
#     applied on the first frame that has a measurement now, which is why the
#     assertion reads the last report line rather than the first.
#
#     OpenRouter gets a catalog long enough to fill its own list, because the
#     three-model fixture every other check uses fits on one screen — a bottom
#     capture of a section that never scrolls is a check that cannot fail.
python3 - "$AI_DIR" <<'PYBIG'
import json, pathlib, sys
cache = pathlib.Path(sys.argv[1]) / "bigcache/musializer"
cache.mkdir(parents=True, exist_ok=True)
models = [
    {"id": f"vendor{i // 4}/audio-model-{i:02d}", "name": f"Vendor {i // 4} Audio {i:02d}",
     "context_length": 32000 + i, "input_modalities": ["audio", "text"],
     "output_modalities": ["text"],
     "pricing": {"prompt": "0.0000004", "completion": "0.0000012", "audio": "0.0000090"}}
    for i in range(20)
]
(cache / "openrouter-models-v1.json").write_text(json.dumps({
    "schema_version": "musializer.openrouter-catalog/v1",
    "source_url": "https://openrouter.ai/api/v1/models",
    "fetched_at_utc": "2026-07-20T09:00:00Z", "validated_at_utc": "2026-07-20T09:00:00Z",
    "filters": {"input_modalities": "audio", "output_modalities": "text"},
    "model_count": len(models), "unfiltered_model_count": 340, "models": models,
}, indent=2, sort_keys=True) + "\n")
PYBIG
for section in local openrouter; do
    bottom_env=("MUSIALIZER_ASSIST_SETTINGS_OPEN=$section"
                "MUSIALIZER_ASSIST_SETTINGS_DOCTOR=$REPO_ROOT/$AI_DIR/cache/musializer/doctor.json"
                MUSIALIZER_ASSIST_SETTINGS_SCROLL=4000)
    [ "$section" = "openrouter" ] &&
        bottom_env+=("XDG_CACHE_HOME=$REPO_ROOT/$AI_DIR/bigcache-root")
    [ "$section" = "openrouter" ] && rm -rf "$AI_DIR/bigcache-root" &&
        mkdir -p "$AI_DIR/bigcache-root" &&
        cp -r "$AI_DIR/bigcache/musializer" "$AI_DIR/bigcache-root/musializer"
    ai_capture "ai-$section-bottom" 1280x720 "${bottom_env[@]}" \
        -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
    BOTTOM="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-$section-bottom.txt" | tail -1)"
    SCROLLED="$(printf '%s\n' "$BOTTOM" | sed -n 's/.*scroll=\([0-9]*\).*/\1/p')"
    OVERFLOW="$(printf '%s\n' "$BOTTOM" | sed -n 's/.*overflow=\([0-9]*\).*/\1/p')"
    echo "  $section bottom: scroll=${SCROLLED:-?} of overflow=${OVERFLOW:-?}"
    # The bottom, whatever the section's height. A section that fits entirely is
    # at its bottom at zero, and asserting only `scroll > 0` could not tell that
    # from the dead seam this replaces — which is why the overflow is reported.
    case "$BOTTOM" in
        *"at-bottom=true"*) ;;
        *) echo "FAIL: the $section capture is not at the bottom of its section" >&2
           AI_FAILED=1 ;;
    esac
    if [ "${OVERFLOW:-0}" -gt 0 ] 2>/dev/null; then
        if [ "${SCROLLED:-0}" -le 0 ] 2>/dev/null; then
            echo "FAIL: the scroll seam is dead, so the bottom of $section is unphotographed" >&2
            AI_FAILED=1
        fi
        # And the picture really moved: the top of the body differs from the
        # unscrolled capture of the same section.
        BOTTOM_DELTA="$(ffprobe -v error -f lavfi \
            -i "movie=$OUT_DIR/ai-$section.png,crop=830:400:302:90[a];movie=$OUT_DIR/ai-$section-bottom.png,crop=830:400:302:90[b];[a][b]blend=all_mode=difference,signalstats" \
            -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
        echo "  $section bottom: max luma difference against the unscrolled capture = ${BOTTOM_DELTA:-?}"
        python3 -c "import sys; sys.exit(0 if float('${BOTTOM_DELTA:-0}') > 20.0 else 1)" \
            || { echo "FAIL: the $section capture scrolled by a number but not by pixels" >&2; AI_FAILED=1; }
    else
        echo "  $section bottom: the section fits without scrolling at this window"
    fi
done
# At least one of the two really does overflow, or the pair above would be
# asserting nothing about the seam that was dead.
if ! grep -h '^assist settings: ' "$OUT_DIR/ai-local-bottom.txt" "$OUT_DIR/ai-openrouter-bottom.txt" \
    | grep -qv 'overflow=0 '; then
    echo "FAIL: neither section overflowed, so the scroll seam is untested" >&2
    AI_FAILED=1
fi

# 3. The modal really is modal. With the dialog open the timeline strip is not
#    drawn at all, because the lyrics cue field reads raylib's keyboard directly
#    and would otherwise receive keys typed into the masked field. The strip is
#    the only thing that prints `assist settings button:`, so its absence from a
#    dialog run is the assertion.
if grep -q '^assist settings button:' "$OUT_DIR/ai-routing.txt"; then
    echo "FAIL: the Assist panel drew underneath the modal, so its text field could take keys" >&2
    AI_FAILED=1
else
    echo "  modal: the timeline strip is not drawn while the dialog is open"
fi

# 4. Keyboard traversal, photographed. A headless run has no keyboard any more
#    than it has a pointer, so the probe applies the Tab steps.
ai_capture "ai-focus-8" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    MUSIALIZER_ASSIST_SETTINGS_TAB=8 -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
FOCUS_LINE="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-focus-8.txt" | tail -1)"
echo "  focus: $FOCUS_LINE"
case "$FOCUS_LINE" in
    *"focus=8/"*) ;;
    *) echo "FAIL: eight Tab steps did not land on control 8: $FOCUS_LINE" >&2; AI_FAILED=1 ;;
esac

# ai_focus_ring LOG PNG BASELINE_PNG -- assert a ring was drawn where the report
# says the focus is.
#
# The crop comes from the report's own `focus-rect=` rather than from a
# hand-measured rectangle (AP3-R S4). A pinned crop is a second, silent claim
# about where a control is, and it is wrong the moment a column width changes —
# which this tranche changed. Reading the box the dialog says it drew makes the
# check about the ring rather than about the layout.
ai_focus_ring() {
    local log="$1" shot="$2" baseline="$3" what="$4"
    local line rect x y w h focused plain
    line="$(sed -n 's/^assist settings: //p' "$OUT_DIR/$log.txt" | tail -1)"
    case "$line" in
        *"focus-visible=true"*) ;;
        *) echo "FAIL: $what left the focus off screen: $line" >&2; return 1 ;;
    esac
    rect="$(printf '%s\n' "$line" | sed -n 's/.*focus-rect=\([0-9,]*\).*/\1/p')"
    if [ -z "$rect" ] || [ "$rect" = "none" ]; then
        echo "FAIL: $what reported no focus rectangle: $line" >&2
        return 1
    fi
    IFS=, read -r x y w h <<EOF
$rect
EOF
    x=$((x - 6)); y=$((y - 6)); w=$((w + 12)); h=$((h + 12))
    focused="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/$shot.png,crop=$w:$h:$x:$y,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.UAVG -of csv=p=0 2>/dev/null | head -1)"
    plain="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/$baseline.png,crop=$w:$h:$x:$y,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.UAVG -of csv=p=0 2>/dev/null | head -1)"
    echo "  $what: rect=$rect chroma focused=${focused:-?} unfocused=${plain:-?}"
    python3 -c "import sys; sys.exit(0 if float('${focused:-0}') - float('${plain:-0}') > 1.0 else 1)" \
        || { echo "FAIL: no focus ring where $what says the focus is" >&2; return 1; }
    return 0
}
ai_focus_ring ai-focus-8 ai-focus-8 ai-routing "eight Tab steps" || AI_FAILED=1

# 4b. Scroll follows focus (AP3-R S4). The last control of Local models is below
#     the fold with a doctor report loaded — the reviewer measured tabs 7, 8 and
#     9 as byte-identical pictures, because the ring was being drawn off screen
#     and clipped away. The ring has to be *in* the picture, which is what the
#     crop from `focus-rect` asserts.
AI_LOCAL_RING="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-local.txt" | tail -1 \
    | sed -n 's|.*focus=[0-9]*/\([0-9]*\).*|\1|p')"
ai_capture "ai-local-last" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=local \
    "MUSIALIZER_ASSIST_SETTINGS_DOCTOR=$REPO_ROOT/$AI_DIR/cache/musializer/doctor.json" \
    "MUSIALIZER_ASSIST_SETTINGS_TAB=$((AI_LOCAL_RING - 1))" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
ai_focus_ring ai-local-last ai-local-last ai-local "the last Local models control" || AI_FAILED=1
case "$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-local-last.txt" | tail -1)" in
    *"scroll=0 "*)
        echo "FAIL: tabbing to the last control did not scroll it into view" >&2
        AI_FAILED=1 ;;
    *) echo "  scroll followed the focus to the bottom of Local models" ;;
esac

# 4c. Keyboard traversal at the narrow, stacked layout (LYRICS_TIMING_RESEARCH_PLAN.md
#     negative control: "the dialog remains operable by keyboard and at every
#     captured UI scale"). 4/4b prove Tab at 1280 and scroll-follows-focus at
#     1280; 2b proves the 800px width renders the stacked routing table; neither
#     proves Tab still lands a visible ring once the table has actually
#     stacked. Same eight-step probe as 4, same baseline pattern as 4b, just at
#     the narrow width against the unfocused `ai-routing-narrow` capture.
ai_capture "ai-focus-narrow" 800x640 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    MUSIALIZER_ASSIST_SETTINGS_TAB=8 -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
FOCUS_NARROW_LINE="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-focus-narrow.txt" | tail -1)"
echo "  focus (narrow): $FOCUS_NARROW_LINE"
case "$FOCUS_NARROW_LINE" in
    *"focus=8/"*) ;;
    *) echo "FAIL: eight Tab steps at 800x640 did not land on control 8: $FOCUS_NARROW_LINE" >&2
       AI_FAILED=1 ;;
esac
ai_focus_ring ai-focus-narrow ai-focus-narrow ai-routing-narrow \
    "eight Tab steps at the narrow stacked layout" || AI_FAILED=1

# 5. Escape closes. The dialog prints nothing once closed, and the panel behind
#    it starts drawing again — which is the same seam as check 3, run backwards.
ai_capture "ai-escape" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    MUSIALIZER_ASSIST_SETTINGS_ESCAPE=1 -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
if grep -q '^assist settings: section=' "$OUT_DIR/ai-escape.txt"; then
    echo "FAIL: Escape did not close the dialog" >&2
    AI_FAILED=1
elif ! grep -q '^assist settings button:' "$OUT_DIR/ai-escape.txt"; then
    echo "FAIL: Escape closed the dialog but the Assist panel never came back" >&2
    AI_FAILED=1
else
    echo "  escape: the dialog closed and the panel behind it drew again"
fi
# And an unsaved edit gets one explicit confirm step rather than a silent
# discard: the same single Escape leaves it open and says so.
ai_capture "ai-escape-dirty" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    MUSIALIZER_ASSIST_SETTINGS_DIRTY=1 MUSIALIZER_ASSIST_SETTINGS_ESCAPE=1 \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
DIRTY_LINE="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-escape-dirty.txt" | head -1)"
echo "  escape with edits: ${DIRTY_LINE:-<closed>}"
case "$DIRTY_LINE" in
    *"dirty=true confirm-close=true"*) ;;
    *) echo "FAIL: Escape discarded unsaved edits without asking" >&2; AI_FAILED=1 ;;
esac

# 5b. Enter activates, and Save writes. Without the activate seam the whole
#     write path would be unreachable from a headless run and "Enter activates"
#     would only ever be a unit test. Its own copy of the settings file, so the
#     write cannot change what every other capture above reads.
cp "$AI_DIR/config/assist.json" "$AI_DIR/config/assist-save.json"
ai_capture "ai-save" 1280x720 \
    "MUSIALIZER_ASSIST_SETTINGS=$REPO_ROOT/$AI_DIR/config/assist-save.json" \
    MUSIALIZER_ASSIST_SETTINGS_OPEN=routing MUSIALIZER_ASSIST_SETTINGS_DIRTY=1 \
    MUSIALIZER_ASSIST_SETTINGS_ACTIVATE=1 -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
SAVE_LINE="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-save.txt" | head -1)"
echo "  save: ${SAVE_LINE:-<absent>}"
case "$SAVE_LINE" in
    *"dirty=false"*) ;;
    *) echo "FAIL: Enter on the focused Save button did not commit the edit" >&2; AI_FAILED=1 ;;
esac
if ! grep -q '"TC-ALIGN"' "$AI_DIR/config/assist-save.json"; then
    echo "FAIL: Save did not write the override to the settings file" >&2
    AI_FAILED=1
else
    echo "  save: the TC-ALIGN override reached the settings file"
fi

# 6. The masked field. Two runs, two different credentials of the *same* length:
#    if one character of either reached a pixel the crops would differ, because
#    A and W are nowhere near the same width in Space Grotesk. The mask is a
#    function of the length alone, so the two crops must be byte-identical.
ai_capture "ai-key-a" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    "MUSIALIZER_ASSIST_SETTINGS_KEY=$AI_CANARY" -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
ai_capture "ai-key-b" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    "MUSIALIZER_ASSIST_SETTINGS_KEY=WWWWWWWWWWWWWWWWWWWWWWWWWWW" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
for half in a b; do
    ffmpeg -loglevel error -y -i "$OUT_DIR/ai-key-$half.png" \
        -vf "crop=830:34:302:216" "$OUT_DIR/ai-key-$half-crop.png"
done
KEY_A="$(sha256sum "$OUT_DIR/ai-key-a-crop.png" | cut -d' ' -f1)"
KEY_B="$(sha256sum "$OUT_DIR/ai-key-b-crop.png" | cut -d' ' -f1)"
KEY_INK="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-key-a-crop.png,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
echo "  masked field: a=${KEY_A:0:16} b=${KEY_B:0:16} darkest=${KEY_INK:-?}"
if [ "$KEY_A" != "$KEY_B" ]; then
    echo "FAIL: two different keys of the same length drew different pixels" >&2
    AI_FAILED=1
fi
# Two blank boxes would also match, which would make the check prove nothing.
if [ "${KEY_INK:-255}" -ge 120 ] 2>/dev/null; then
    echo "FAIL: the masked field drew nothing at all, so the match is vacuous" >&2
    AI_FAILED=1
fi
case "$(sed -n 's/^assist settings key: //p' "$OUT_DIR/ai-key-a.txt" | head -1)" in
    *'masked-len=25'*) echo "  masked field: 25 bullets for a 27-character key" ;;
    *) echo "FAIL: the masked report line does not describe a masked field" >&2; AI_FAILED=1 ;;
esac

# 7. The stale badge, with its age. An absent cache is "never fetched", which is
#    a different sentence from an empty catalog and is checked as one.
CATALOG_LINE="$(sed -n 's/^assist settings discovery: //p' "$OUT_DIR/ai-openrouter.txt" | head -1)"
echo "  discovery: $CATALOG_LINE"
case "$CATALOG_LINE" in
    'catalog=stale age="16 days ago" models=3'*) ;;
    *) echo "FAIL: the catalog badge is not the stale one the fixture pins" >&2; AI_FAILED=1 ;;
esac
ai_capture "ai-no-cache" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    "XDG_CACHE_HOME=$REPO_ROOT/$AI_DIR/empty-cache" -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
case "$(sed -n 's/^assist settings discovery: //p' "$OUT_DIR/ai-no-cache.txt" | head -1)" in
    'catalog=never fetched'*'codex=never fetched'*) echo "  no cache: never fetched, not empty" ;;
    *) echo "FAIL: an absent cache was not reported as never fetched" >&2; AI_FAILED=1 ;;
esac

# 8. A corrupt settings file is an error state, not a silent reset — and it says
#    so **on Routing**, which is where a user configuring routes is standing
#    (AP3-R S1).
#
#    The fixture deliberately opens on `routing` rather than on `privacy`: the
#    independent review measured a corrupt-file routing capture as *pixel
#    identical* to a clean one, because the matrix drew the built-in defaults and
#    said nothing anywhere on the page. The two assertions below are the two
#    halves of the fix — a load error reported wherever you are standing, and a
#    matrix drawn explicitly disabled underneath it.
ai_capture "ai-corrupt" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    "MUSIALIZER_ASSIST_SETTINGS=$REPO_ROOT/$AI_DIR/corrupt/assist.json" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
case "$(sed -n 's/^assist settings stores: //p' "$OUT_DIR/ai-corrupt.txt" | head -1)" in
    'settings=error: '*) echo "  corrupt settings: reported as an error" ;;
    *) echo "FAIL: a corrupt settings file was not reported as an error" >&2; AI_FAILED=1 ;;
esac
CORRUPT_LINE="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-corrupt.txt" | tail -1)"
case "$CORRUPT_LINE" in
    *"load-error=true"*) echo "  corrupt settings: the banner state is reported on routing" ;;
    *) echo "FAIL: routing does not report the failed load: $CORRUPT_LINE" >&2; AI_FAILED=1 ;;
esac
# Every routing control disabled means the ring is only Close plus the five
# section tabs. A number, so "explicitly disabled" is not a matter of opinion.
case "$CORRUPT_LINE" in
    *"focus=0/6 "*) echo "  corrupt settings: the routing matrix offers no tabstop" ;;
    *) echo "FAIL: a corrupt file left routing controls focusable: $CORRUPT_LINE" >&2; AI_FAILED=1 ;;
esac
for crop_name in ai-routing ai-corrupt; do
    ffmpeg -loglevel error -y -i "$OUT_DIR/$crop_name.png" \
        -vf "crop=1040:700:120:20" "$OUT_DIR/$crop_name-crop.png"
done
CORRUPT_DELTA="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-routing-crop.png[a];movie=$OUT_DIR/ai-corrupt-crop.png[b];[a][b]blend=all_mode=difference,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "  corrupt settings: max luma difference against a clean routing capture = ${CORRUPT_DELTA:-?}"
python3 -c "import sys; sys.exit(0 if float('${CORRUPT_DELTA:-0}') > 40.0 else 1)" \
    || { echo "FAIL: a failed settings load is invisible on Routing" >&2; AI_FAILED=1; }
if [ "$(cat "$AI_DIR/corrupt/assist.json")" != "{ broken" ]; then
    echo "FAIL: the corrupt settings file was overwritten rather than reported" >&2
    AI_FAILED=1
fi

# 9. A loose-permission credentials file is refused, and the refusal names the
#    mode and the fix. Refused, not repaired: the mode on disk is untouched.
ai_capture "ai-loose" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    "MUSIALIZER_ASSIST_CREDENTIALS=$REPO_ROOT/$AI_DIR/loose/credentials.json" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
case "$(sed -n 's/^assist settings stores: //p' "$OUT_DIR/ai-loose.txt" | head -1)" in
    *'credentials=refused(0644)'*) echo "  loose credentials: refused, mode reported" ;;
    *) echo "FAIL: a 0644 credentials file was not refused" >&2; AI_FAILED=1 ;;
esac
LOOSE_MODE="$(stat -c '%a' "$AI_DIR/loose/credentials.json")"
if [ "$LOOSE_MODE" != "644" ]; then
    echo "FAIL: the loose credentials file was repaired (now $LOOSE_MODE) instead of refused" >&2
    AI_FAILED=1
fi

# 9b. §5 invariant 4: READY means every required piece is present (AP3-R S3).
#     The fixture is the reviewer's: a stored, well-permissioned key and an
#     OpenRouter route with **no model chosen**. That reported `Ready` — the one
#     direction a readiness badge must never be wrong in.
python3 - "$AI_DIR" <<'PYREADY'
import json, os, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "ready"
root.mkdir(parents=True, exist_ok=True)
(root / "assist.json").write_text(json.dumps({
    "schema": "musializer.assist-settings/v1",
    "active_profile": "overrides",
    "profiles": [{"id": "overrides", "label": "Overrides", "routes": {
        "TC-SEMANTIC": {"contract": "TC-SEMANTIC", "route_type": "openrouter",
                         "runtime_id": "openrouter", "fallback": "none"}}}],
}, indent=2) + "\n")
credentials = root / "credentials.json"
credentials.write_text(json.dumps({
    "schema": "musializer.assist-credentials/v1",
    "entries": {"openrouter/default": {"secret": "sk-or-v1-MUSICANARY7Q4X2ZK9",
                                        "label": "Fixture"}},
}, indent=2) + "\n")
credentials.chmod(0o600)
os.chmod(root, 0o700)
# A credentials file whose *entry* is schema-invalid while the file is perfectly
# good JSON. It reported "not valid JSON" with no path, entry or fix.
entry = pathlib.Path(sys.argv[1]) / "badentry"
entry.mkdir(parents=True, exist_ok=True)
bad = entry / "credentials.json"
bad.write_text(json.dumps({
    "schema": "musializer.assist-credentials/v1",
    "entries": {"openrouter/default": {"secret": 12345}},
}, indent=2) + "\n")
bad.chmod(0o600)
os.chmod(entry, 0o700)
PYREADY
ai_capture "ai-ready-no-model" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    "MUSIALIZER_ASSIST_SETTINGS=$REPO_ROOT/$AI_DIR/ready/assist.json" \
    "MUSIALIZER_ASSIST_CREDENTIALS=$REPO_ROOT/$AI_DIR/ready/credentials.json" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
READY_LINE="$(sed -n 's/^assist settings routing: //p' "$OUT_DIR/ai-ready-no-model.txt" | tail -1)"
echo "  readiness: $(printf '%s\n' "$READY_LINE" | tr ' ' '\n' | grep TC-SEMANTIC)"
case "$(sed -n 's/^assist settings stores: //p' "$OUT_DIR/ai-ready-no-model.txt" | head -1)" in
    *'credentials=file('*) ;;
    *) echo "FAIL: the readiness fixture has no usable credential, so it tests nothing" >&2
       AI_FAILED=1 ;;
esac
# The *reason*, not just "blocked". A negative control that removed the
# model check still read `[blocked]`, because this fixture's catalog also
# leaves no eligible endpoint — so the check passed while the thing it was
# written for was disabled. Naming the first missing piece is what makes it
# a check.
case "$READY_LINE" in
    *'TC-SEMANTIC=openrouter/openrouter/not chosen[No model chosen]'*) ;;
    *) echo "FAIL: a route with a key but no model does not name the missing model" >&2
       AI_FAILED=1 ;;
esac

# 9c. A schema-invalid credentials *entry* is diagnosed, not called invalid JSON,
#     and it names the entry (AP3-R S12). The file is left exactly as it is.
ai_capture "ai-bad-entry" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=openrouter \
    "MUSIALIZER_ASSIST_CREDENTIALS=$REPO_ROOT/$AI_DIR/badentry/credentials.json" \
    -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
case "$(sed -n 's/^assist settings stores: //p' "$OUT_DIR/ai-bad-entry.txt" | head -1)" in
    *'credentials=unusable(schema entry=openrouter/default)'*)
        echo "  bad credentials entry: named, with the fault separated from 'not JSON'" ;;
    *) echo "FAIL: a schema-invalid credentials entry was not diagnosed" >&2; AI_FAILED=1 ;;
esac
if ! grep -q '"secret": 12345' "$AI_DIR/badentry/credentials.json"; then
    echo "FAIL: the unreadable credentials file was rewritten rather than reported" >&2
    AI_FAILED=1
fi

# 9d. A job in flight is stated where the settings promise is made (AP3-R S11).
#     The dialog says routing changes apply to the next job; the banner is what
#     makes that promise about something.
for state in running ready; do
    spec="panel=assist,play=0"
    [ "$state" = "ready" ] || spec="$spec,assist=$state"
    ai_capture "ai-job-$state" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
        -- --ui-probe "$spec" || AI_FAILED=1
    GOT="$(sed -n 's/^assist settings: //p' "$OUT_DIR/ai-job-$state.txt" | tail -1 \
        | sed -n 's/.*\(job-running=[a-z]*\).*/\1/p')"
    WANT="job-running=false"
    [ "$state" = "running" ] && WANT="job-running=true"
    echo "  job banner: $state -> ${GOT:-<absent>}"
    if [ "$GOT" != "$WANT" ]; then
        echo "FAIL: the dialog reports $GOT with the panel in the $state state" >&2
        AI_FAILED=1
    fi
done
JOB_DELTA="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-job-ready.png,crop=840:40:300:85[a];movie=$OUT_DIR/ai-job-running.png,crop=840:40:300:85[b];[a][b]blend=all_mode=difference,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "  job banner: max luma difference at the top of the body = ${JOB_DELTA:-?}"
python3 -c "import sys; sys.exit(0 if float('${JOB_DELTA:-0}') > 40.0 else 1)" \
    || { echo "FAIL: the running-job banner is reported but not drawn" >&2; AI_FAILED=1; }

# 9e. A readiness badge carries its whole remediation in a tooltip (AP3-R S9).
#     The pill is 96 px and ellipsizes; the sentence a user has to act on is
#     longer than that in every interesting case. A headless run has no pointer,
#     so the hover is parked by the probe — the same seam the transport row's
#     tooltips are photographed through.
ai_capture "ai-badge-hint" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
    MUSIALIZER_ASSIST_SETTINGS_HOVER=1 \
    -- --ui-probe "panel=assist,play=0,hover=1088x355" || AI_FAILED=1
BADGE_INK="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-badge-hint.png,crop=700:120:420:330,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
BADGE_PLAIN="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-routing.png,crop=700:120:420:330,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
BADGE_DELTA="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-routing.png,crop=700:120:420:330[a];movie=$OUT_DIR/ai-badge-hint.png,crop=700:120:420:330[b];[a][b]blend=all_mode=difference,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "  badge tooltip: darkest hovered=${BADGE_INK:-?} unhovered=${BADGE_PLAIN:-?} delta=${BADGE_DELTA:-?}"
python3 -c "import sys; sys.exit(0 if float('${BADGE_DELTA:-0}') > 40.0 else 1)" \
    || { echo "FAIL: hovering the No key badge drew no tooltip" >&2; AI_FAILED=1; }

# 10. Nothing above opened a socket. Every run stubbed the key test, and the
#     refresh tools were never spawned — a live call in a capture would make the
#     gate depend on OpenRouter being up, and would send an IP address from a
#     check that claims to be offline.
if grep -rl 'openrouter.ai' "$OUT_DIR"/ai-*.txt | grep -q . ; then
    if grep -h 'openrouter.ai' "$OUT_DIR"/ai-*.txt | grep -qv 'assist settings'; then
        echo "FAIL: a capture log mentions a live OpenRouter request" >&2
        AI_FAILED=1
    fi
fi
echo "  network: every run stubbed the key test and left catalog refresh unpressed"

# 11. Defect A (operator, 2026-08-05). The section rail's buttons sat flush
#     against the vertical divider while keeping a comfortable gap to the
#     dialog's left border. Both gaps must be the same.
#
#     Measured twice on purpose: the dialog reports the geometry it drew, and
#     the *capture* is then scanned for the same two numbers. A report line
#     alone would pass if the buttons were drawn somewhere other than their
#     layout rect, which is precisely the class of error a layout constant
#     cannot rule out.
echo "  --- defect A: the section rail's two gaps ---"
for pair in "ai-routing:rail" "ai-routing-narrow:tabs"; do
    log="${pair%%:*}"
    want="${pair#*:}"
    CHROME="$(sed -n 's/^assist settings chrome: //p' "$OUT_DIR/$log.txt" | head -1)"
    echo "  $log: ${CHROME:-<absent>}"
    case "$CHROME" in
        "mode=$want "*) ;;
        *) echo "FAIL: $log did not draw the $want arrangement" >&2; AI_FAILED=1; continue ;;
    esac
    case "$CHROME" in
        *symmetric=true*) ;;
        *) echo "FAIL: $log draws the section list with unequal gaps" >&2; AI_FAILED=1 ;;
    esac
done
# And the same two numbers off the pixels, for the rail. The first rail button
# is the selected one, so it is a dark fill against the dialog's light surface
# and its edges are unambiguous; the divider is a single darker column.
python3 - "$OUT_DIR/ai-routing.png" \
    "$(sed -n 's/^assist settings chrome: //p' "$OUT_DIR/ai-routing.txt" | head -1)" <<'PYRAIL' || AI_FAILED=1
import subprocess, sys

png, chrome = sys.argv[1], sys.argv[2]
fields = dict(part.split("=", 1) for part in chrome.split() if "=" in part)
tab = [float(v) for v in fields["first-tab"].split(",")]
row = int(tab[1] + tab[3] / 2.0)

probe = subprocess.run(
    ["ffprobe", "-v", "error", "-select_streams", "v:0",
     "-show_entries", "stream=width,height", "-of", "csv=p=0:s=,", png],
    capture_output=True, text=True).stdout.strip()
width, height = (int(v) for v in probe.split(","))
raw = subprocess.run(
    ["ffmpeg", "-v", "error", "-i", png, "-f", "rawvideo", "-pix_fmt", "gray", "-"],
    capture_output=True).stdout
scan = raw[row * width:(row + 1) * width]

# The dialog's own surface is the brightest thing on this row; the workspace
# behind the scrim is much darker. Find the surface, then the button inside it.
surface = max(scan)
if surface < 200:
    print(f"FAIL: row {row} of {png} carries no dialog surface (max luma {surface})")
    raise SystemExit(1)
left = next(x for x in range(width) if scan[x] >= surface - 4)
# The dialog's border is one darker column immediately left of its fill, and it
# is what `dialog.x` names -- so both gaps are measured to a border column and
# the two numbers are comparable with each other and with the report.
dialog_x = left - 1
button_left = next(x for x in range(left, width) if scan[x] < surface - 4)
button_right = button_left
while scan[button_right + 1] < surface - 4:
    button_right += 1
divider = next(x for x in range(button_right + 1, width) if scan[x] < surface - 4)

gap_left = button_left - dialog_x
gap_right = divider - (button_right + 1)
print(f"  ai-routing pixels: dialog-border={dialog_x} button={button_left}..{button_right} "
      f"divider={divider} gap-left={gap_left}px gap-right={gap_right}px")
if gap_left != gap_right:
    print(f"FAIL: the rail is inset {gap_left}px on the left and {gap_right}px on the right")
    raise SystemExit(1)
if gap_left < 4:
    print(f"FAIL: the rail has no gap at all ({gap_left}px)")
    raise SystemExit(1)
# And the capture agrees with what the dialog said it drew.
for name, measured in (("gap-left", gap_left), ("gap-right", gap_right)):
    if abs(float(fields[name]) - measured) > 0.5:
        print(f"FAIL: reported {name}={fields[name]} against {measured} in the picture")
        raise SystemExit(1)
PYRAIL

# 12. Defect B (operator, 2026-08-05). `Show experimental: off` displayed
#     TC-SEMANTIC's model with an orange `experimental` marker, because the
#     picker re-inserted the selected id after the filter dropped it. The
#     ruling: the toggle governs what any picker OFFERS, the configured model is
#     still shown, and the row says in words which of the two it is.
echo "  --- defect B: the experimental filter ---"
python3 - "$AI_DIR" <<'PYEXPERIMENTAL'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "experimental"
root.mkdir(parents=True, exist_ok=True)
# Nothing but the toggle differs between the two, so the pair isolates it.
for name, show in (("off", False), ("on", True)):
    (root / f"assist-{name}.json").write_text(json.dumps({
        "schema": "musializer.assist-settings/v1",
        "active_profile": "recommended",
        "catalog": {"show_experimental": show},
    }, indent=2) + "\n")
PYEXPERIMENTAL
for state in off on; do
    ai_capture "ai-experimental-$state" 1280x720 MUSIALIZER_ASSIST_SETTINGS_OPEN=routing \
        "MUSIALIZER_ASSIST_SETTINGS=$REPO_ROOT/$AI_DIR/experimental/assist-$state.json" \
        -- --ui-probe "panel=assist,play=0" || AI_FAILED=1
    PICKERS="$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-$state.txt" | head -1)"
    echo "  experimental $state: $(printf '%s' "$PICKERS" | grep -o 'TC-SEMANTIC{[^}]*}')"
    echo "  experimental $state: $(printf '%s' "$PICKERS" | grep -o 'starved=\[[^]]*\]')"
done
# With the toggle off: nothing offered, the configuration still readable, and
# the marker says which of the two it is.
case "$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-off.txt" | head -1)" in
    *'TC-SEMANTIC{configured=xiaomi/mimo-v2.5 status=experimental marker="not offered: experimental" offers=[]}'*)
        echo "  toggle off: the picker offers nothing and the row says why" ;;
    *) echo "FAIL: an experimental model is still offered with the toggle off" >&2; AI_FAILED=1 ;;
esac
case "$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-off.txt" | head -1)" in
    *'starved=[TC-SEMANTIC]'*) ;;
    *) echo "FAIL: the contract with no offerable model is not named" >&2; AI_FAILED=1 ;;
esac
# With it on: the same model is offered, and the marker changes meaning.
case "$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-on.txt" | head -1)" in
    *'TC-SEMANTIC{configured=xiaomi/mimo-v2.5 status=experimental marker="configured: experimental" offers=[xiaomi/mimo-v2.5,acme/never-measured,acme/quiet]}'*)
        echo "  toggle on: three models offered, marker names the configuration" ;;
    *) echo "FAIL: the toggle does not refill the picker" >&2; AI_FAILED=1 ;;
esac
case "$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-on.txt" | head -1)" in
    *'starved=[]'*) ;;
    *) echo "FAIL: a contract is still starved with the toggle on" >&2; AI_FAILED=1 ;;
esac
# The sentence has to be *drawn*, not merely reported: a warning nothing
# photographs is a warning nobody reads. Comparing the two captures would not
# show it — the ON case draws its own warning above the matrix and shifts every
# row down, so *everything* differs. So the dialog reports the band it drew the
# sentence into, and the band is measured for ink.
STARVED_BAND="$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-off.txt" \
    | head -1 | sed -n 's/.*starved-band=\([0-9]*\),\([0-9]*\),\([0-9]*\),\([0-9]*\).*/\1:\2:\3:\4/p')"
if [ -z "$STARVED_BAND" ]; then
    echo "FAIL: the dialog reports no band for the \"No recommended model\" sentence" >&2
    AI_FAILED=1
else
    SB_X="${STARVED_BAND%%:*}"; SB_REST="${STARVED_BAND#*:}"
    SB_Y="${SB_REST%%:*}";      SB_REST="${SB_REST#*:}"
    SB_W="${SB_REST%%:*}";      SB_H="${SB_REST#*:}"
    STARVED_INK="$(ffprobe -v error -f lavfi \
        -i "movie=$OUT_DIR/ai-experimental-off.png,crop=$SB_W:$SB_H:$SB_X:$SB_Y,signalstats" \
        -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
    echo "  no-offerable-model sentence: band ${SB_W}x${SB_H}+${SB_X}+${SB_Y} darkest luma = ${STARVED_INK:-?}"
    python3 -c "import sys; sys.exit(0 if float('${STARVED_INK:-255}') < 200.0 else 1)" \
        || { echo "FAIL: the \"No recommended model\" sentence is reported but not drawn" >&2
             AI_FAILED=1; }
fi
# And with the toggle on, no band is claimed at all.
case "$(sed -n 's/^assist settings pickers: //p' "$OUT_DIR/ai-experimental-on.txt" | head -1)" in
    *starved-band=none*) ;;
    *) echo "FAIL: a starved-model sentence is claimed with the toggle on" >&2; AI_FAILED=1 ;;
esac

# 13. Defect C (operator, 2026-08-05). "codex not on PATH" was false: a GUI
#     process started from a desktop entry inherits the session manager's
#     minimal PATH, and codex lives in an npm global prefix that is on the login
#     shell's PATH only. Every rung of the replacement ladder is exercised
#     against a fixture, with `HOME` and `PATH` under this script's control so
#     the result does not depend on what is installed on the machine.
echo "  --- defect C: finding codex ---"
CODEX_DIR="$OUT_DIR/codex-discovery"
rm -rf "$CODEX_DIR"
mkdir -p "$CODEX_DIR/home/.local/npm-global/bin" "$CODEX_DIR/onpath" \
         "$CODEX_DIR/elsewhere" "$CODEX_DIR/emptyhome"
for stub in "$CODEX_DIR/home/.local/npm-global/bin/codex" "$CODEX_DIR/onpath/codex" \
            "$CODEX_DIR/elsewhere/codex"; do
    printf '#!/bin/sh\nexit 0\n' >"$stub"
    chmod 755 "$stub"
done
# A login shell that puts codex somewhere nothing here has heard of. `sh -l`
# sources $HOME/.profile, which is the most portable of the login-shell files.
mkdir -p "$CODEX_DIR/shellhome"
printf 'export PATH="%s:$PATH"\n' "$REPO_ROOT/$CODEX_DIR/elsewhere" \
    >"$CODEX_DIR/shellhome/.profile"
python3 - "$AI_DIR" "$REPO_ROOT/$CODEX_DIR" <<'PYCODEX'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "codex"
root.mkdir(parents=True, exist_ok=True)
# TC-WORDING routes through Codex by default, so its readiness badge is the
# user-visible consequence of every answer below.
(root / "assist.json").write_text(json.dumps({
    "schema": "musializer.assist-settings/v1",
    "active_profile": "recommended",
}, indent=2) + "\n")
(root / "assist-override-missing.json").write_text(json.dumps({
    "schema": "musializer.assist-settings/v1",
    "active_profile": "recommended",
    "local_runtimes": {"codex_bin": f"{sys.argv[2]}/nowhere/codex"},
}, indent=2) + "\n")
PYCODEX

# codex_capture NAME HOME PATH DISCOVERY [VAR=VALUE ...]
#
# `env -i` on purpose: the point of every case below is what this process's own
# environment does and does not carry, so inheriting the operator's would make
# the answers depend on their shell.
codex_capture() {
    local name="$1" home="$2" path="$3" discovery="$4" settings="$5"
    shift 5
    local out="$OUT_DIR/$name.png"
    local log="$OUT_DIR/$name.txt"
    set +e
    env -i -u WAYLAND_DISPLAY \
        DISPLAY="$DISPLAY_NUM" \
        PATH="$path" \
        HOME="$REPO_ROOT/$home" \
        SHELL=/bin/sh \
        XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" \
        PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
        XDG_CACHE_HOME="$REPO_ROOT/$AI_DIR/cache" \
        MUSIALIZER_ASSIST_SETTINGS="$REPO_ROOT/$settings" \
        MUSIALIZER_ASSIST_CREDENTIALS="$REPO_ROOT/$AI_DIR/config/credentials.json" \
        MUSIALIZER_ASSIST_SETTINGS_NOW=2026-08-05T12:00:00Z \
        MUSIALIZER_ASSIST_SETTINGS_KEY_TEST=no-key \
        MUSIALIZER_ASSIST_DISCOVERY="$discovery" \
        MUSIALIZER_ASSIST_SETTINGS_OPEN=codex \
        "$@" \
        ./target/debug/musializer --mute "$FIXTURE" \
            --size 1280x720 --probe-frames 300 --probe-shot "$out" \
            --ui-probe "panel=assist,play=0" \
        >"$log" 2>&1
    local status=$?
    set -e
    printf '%-30s exit=%s\n' "$name" "$status"
    [ -f "$out" ] && [ "$status" -eq 0 ]
}

# C1: on PATH. The rung that always worked, kept as the control.
codex_capture "ai-codex-path" "$CODEX_DIR/emptyhome" \
    "$REPO_ROOT/$CODEX_DIR/onpath:/usr/bin:/bin" off "$AI_DIR/codex/assist.json" || AI_FAILED=1
# C2: **the operator's case.** A desktop entry's PATH, and codex in an npm
#     global prefix. This is the run that used to say "codex not on PATH".
codex_capture "ai-codex-wellknown" "$CODEX_DIR/home" \
    "/usr/bin:/bin" off "$AI_DIR/codex/assist.json" || AI_FAILED=1
# C3: nowhere this script has heard of, reachable only through the login shell.
#     Discovery is left on for this one — it is the rung being tested — and the
#     shell is `/bin/sh` against a fixture `$HOME`, never the operator's.
codex_capture "ai-codex-loginshell" "$CODEX_DIR/shellhome" \
    "/usr/bin:/bin" on "$AI_DIR/codex/assist.json" || AI_FAILED=1
# C4: genuinely absent. The searched list is the whole point of the picture.
codex_capture "ai-codex-absent" "$CODEX_DIR/emptyhome" \
    "/usr/bin:/bin" on "$AI_DIR/codex/assist.json" || AI_FAILED=1
# C5: a set-but-missing override fails loudly and does **not** fall back to the
#     copy sitting on PATH.
codex_capture "ai-codex-override-missing" "$CODEX_DIR/home" \
    "$REPO_ROOT/$CODEX_DIR/onpath:/usr/bin:/bin" on \
    "$AI_DIR/codex/assist-override-missing.json" || AI_FAILED=1

for expect in \
    "ai-codex-path:via=PATH:Ready" \
    "ai-codex-wellknown:via=well-known:Ready" \
    "ai-codex-loginshell:via=login-shell:Ready" \
    "ai-codex-absent:codex=absent:codex not found" \
    "ai-codex-override-missing:override-missing:codex_bin path is wrong" ; do
    log="${expect%%:*}"
    rest="${expect#*:}"
    want="${rest%%:*}"
    ready="${rest#*:}"
    GOT="$(sed -n 's/^assist settings codex: //p' "$OUT_DIR/$log.txt" | tail -1)"
    echo "  $log: ${GOT:-<absent>}"
    case "$GOT" in
        *"$want"*) ;;
        *) echo "FAIL: $log expected $want" >&2; AI_FAILED=1 ;;
    esac
    # `sed`, not `tr ' ' '\n' | grep`: the model label is `Codex default`, and
    # splitting on spaces cut the field in half — the check then compared the
    # wrong half and failed for a reason that had nothing to do with codex.
    WORDING="$(sed -n 's/^assist settings routing: //p' "$OUT_DIR/$log.txt" | tail -1 \
        | sed -n 's/.*\(TC-WORDING=[^[]*\[[^]]*\]\).*/\1/p')"
    echo "    readiness: $WORDING"
    case "$WORDING" in
        *"[$ready]"*) ;;
        *) echo "FAIL: $log reports TC-WORDING as $WORDING, wanted [$ready]" >&2; AI_FAILED=1 ;;
    esac
done
# The sentence the defect is named after must be gone everywhere.
if grep -rq 'not on PATH' "$OUT_DIR"/ai-codex-*.txt; then
    echo "FAIL: a run still claims codex is 'not on PATH'" >&2
    AI_FAILED=1
fi
# The override case never even looked past the override, which is what "loud
# failure, not a fallback" means. One searched entry, and it is the override.
case "$(sed -n 's/^assist settings codex: //p' "$OUT_DIR/ai-codex-override-missing.txt" | tail -1)" in
    *"searched=1"*) echo "  override: refused without falling back to the copy on PATH" ;;
    *) echo "FAIL: a set-but-missing override fell through to discovery" >&2; AI_FAILED=1 ;;
esac
# The absent case lists where it looked, in the report and in the picture. The
# list is what a user compares against where they know their binary is.
SEARCHED="$(sed -n 's/^assist settings codex searched: //p' "$OUT_DIR/ai-codex-absent.txt" | tail -1)"
echo "  searched: $SEARCHED"
for place in "PATH: /usr/bin" ".local/npm-global/bin" ".volta/bin" ".asdf/shims" \
             "/opt/homebrew/bin" "login shell"; do
    case "$SEARCHED" in
        *"$place"*) ;;
        *) echo "FAIL: the searched list never mentions $place" >&2; AI_FAILED=1 ;;
    esac
done
# Drawn, not only reported: the found and absent captures must differ in the
# Executable block, and the absent one must carry more rows than the found one.
CODEX_DELTA="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/ai-codex-wellknown.png,crop=840:110:300:160[a];movie=$OUT_DIR/ai-codex-absent.png,crop=840:110:300:160[b];[a][b]blend=all_mode=difference,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "  codex block: max luma difference between found and absent = ${CODEX_DELTA:-?}"
python3 -c "import sys; sys.exit(0 if float('${CODEX_DELTA:-0}') > 40.0 else 1)" \
    || { echo "FAIL: the Executable block draws the same thing found or not" >&2; AI_FAILED=1; }
# §5 rule 6 survives all of it: discovery failure yields exactly `Codex
# default`, never a guessed model id.
for log in ai-codex-absent ai-codex-override-missing; do
    case "$(sed -n 's/^assist settings routing: //p' "$OUT_DIR/$log.txt" | tail -1)" in
        *'TC-WORDING=codex/codex/Codex default'*) ;;
        *) echo "FAIL: $log invented a Codex model id" >&2; AI_FAILED=1 ;;
    esac
done
echo "  model fallback: exactly \"Codex default\" in every unresolved run"

if [ "$AI_FAILED" -ne 0 ]; then
    echo "FAIL: the AI settings dialog checks did not pass" >&2
    exit 1
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
    ./target/debug/musializer --mute "$FIXTURE" \
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
        ./target/debug/musializer --mute "$FIXTURE" \
            --size 1280x720 --probe-frames 10 --probe-shot "$out" "$@" \
        >"$OUT_DIR/$name.txt" 2>&1
    local status=$?
    set -e
    # The readout is drawn, not printed, so the picture is the only witness. The
    # line sits in the top-left of the preview on a near-black background; a
    # bright pixel there means text, and none means a clean preview.
    local ink
    ink="$(ffprobe -v error -f lavfi \
        -i "movie=$out,crop=760:24:390:10,signalstats" \
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
capture "tooltip-mute" 1280x720 --ui-probe "play=1,hover=1121x480" || TRANSPORT_FAILED=1
TIP_INK="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/tooltip-mute.png,crop=140:40:1050:420,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMIN -of csv=p=0 2>/dev/null | head -1)"
# The tip is white on near-black, drawn over the preview's dark background. A dark
# minimum is the box; without the tip that region is the preview's own dark too,
# so the discriminating measure is the *bright* text inside it.
TIP_TEXT="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/tooltip-mute.png,crop=140:40:1050:420,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "tooltip-mute            box luma min=${TIP_INK:-?} text luma max=${TIP_TEXT:-?}"
if [ "${TIP_TEXT%%.*}" -lt 180 ] 2>/dev/null; then
    echo "FAIL: no tooltip text where the mute button's tip should be" >&2
    TRANSPORT_FAILED=1
fi

# The same interaction after both axes are transformed by 125%. This is a hit
# target check, not only another picture: if pointer conversion stays in physical
# coordinates while the widgets move to logical ones, the parked pointer misses
# Mute and this crop contains no bright tooltip text.
capture "tooltip-mute-125" 1600x900 --ui-scale 125 \
    --ui-probe "panel=tune,play=1,hover=978x599" || TRANSPORT_FAILED=1
TIP_TEXT_125="$(ffprobe -v error -f lavfi \
    -i "movie=$OUT_DIR/tooltip-mute-125.png,crop=140:45:900:535,signalstats" \
    -show_entries frame_tags=lavfi.signalstats.YMAX -of csv=p=0 2>/dev/null | head -1)"
echo "tooltip-mute-125        text luma max=${TIP_TEXT_125:-?}"
if [ "${TIP_TEXT_125%%.*}" -lt 180 ] 2>/dev/null; then
    echo "FAIL: the 125% pointer transform missed the mute button" >&2
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
    ./target/debug/musializer --mute "$FIXTURE" \
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

echo "=== the timed lanes share one time axis ==="
# LX1-e. The timeline band stacks the scene-plan lane, the waveform strip and —
# when the lyrics editor is open — the lyric cue lane over the same
# `TimelineView`. Three modules draw them and each is handed its own rectangle,
# so a lane inset by a different amount maps the same second onto a different
# column and the playhead lies about where a cue is. Nothing else here can see
# that: everything inside a lane moves together, so every capture still looks
# self-coherent. This reads the pixels — each lane's outermost frame column and
# the columns its playhead occupies — and requires all of them to agree.
#
# It found two real defects on its first run, both off-by-one tick bounds in
# `shell.rs::timeline_strip`: the tick at the view's last visible second painted
# a rule one column outside the lane's right border, and the tick at its first
# did the same on the left at 150 % scale. Its negative control was a 2 px inset
# added to the waveform strip, which moved that lane's left edge 10 -> 12 and
# split the cue lane's playhead into two columns, while all 1301 unit tests
# stayed green.
LANE_ALIGNMENT_FAILED=0
lane_alignment() {
    # lane_alignment NAME LANES [UI_SCALE]
    local name="$1" lanes="$2" scale="${3:-100}"
    local png="$OUT_DIR/$name.png"
    if [ ! -f "$png" ]; then
        echo "  $name: <no capture>"
        return 0
    fi
    printf '%-30s lanes=%s scale=%s\n' "$name" "$lanes" "$scale"
    if ! python3 tools/timeline_lane_alignment.py "$png" "$lanes" "$scale"; then
        LANE_ALIGNMENT_FAILED=1
    fi
}
for size in 1280x720 960x640; do
    # No panel and Tune leave the band at two lanes; Lyrics adds the cue lane.
    lane_alignment "panel-none-$size" 2
    lane_alignment "panel-tune-$size" 2
    lane_alignment "panel-export-$size" 2
    lane_alignment "panel-lyrics-$size" 3
    # The seeded project, which is the only frame with cue blocks in the lane.
    lane_alignment "lyrics-cues-$size" 3
done
# The operator's own display. The shell picks 150 % on its own at 1440p, and a
# 1 px logical border straddles two columns there — which is how the left-hand
# tick bound was caught.
lane_alignment "ui-scale-150-2560x1440" 2 150
lane_alignment "ui-auto-2560x1440" 2 150
if [ "$LANE_ALIGNMENT_FAILED" -ne 0 ]; then
    echo "FAIL: the timed lanes do not share one time axis" >&2
    SWEEP_FAILED=1
fi

echo "=== the timeline says what it knows: markers, zoom, pan ==="
# D4. Three things land here and they share one fixture, because they are one
# question: can a user navigate a track by looking at the timeline?
#
#   1. The merged manual/semantic event markers. `SceneEventMerge` has reached
#      `SceneFrame` since P0, and `EventType::rgba` was ported for "the timeline
#      markers" by name — but the lane never drew one, for two whole bands. It is
#      the fallback-that-looks-like-content defect again: a timeline with no
#      markers and a timeline whose markers were never wired look identical.
#
#   2. Shift-wheel and middle-drag pan. Both change the view, and so does zoom,
#      so a capture alone cannot tell the three apart. The discriminator is the
#      **span**: a pan holds it and a zoom does not. That is why this section
#      asserts numbers off `timeline:` rather than measuring pixels for the
#      gesture half.
#
#   3. The tick labels' opaque backing. The C draws a COLOR_UI_RAISED plate under
#      every timestamp (`plug.c:3065-3080`) and its comment says why: the
#      waveform behind them runs from the raised surface to dense accent blue, so
#      muted ink over a loud bar measures about 1.16:1. The port drew the label
#      and dropped the plate, which makes the defect *depend on the audio* — the
#      labels are legible over quiet passages and vanish over loud ones, so it
#      appears and disappears as the user scrolls.
#
# The markers are read from pixels rather than trusted from the report, and the
# reason is specific: `timeline:` counts markers per lane from the same list the
# heads are drawn from, so a build that lost the lane distinction would report
# the right counts *and* draw every head identically. Only reading the head shape
# back off the framebuffer closes that. See tools/timeline_event_markers.py.
TIMELINE_D4_FAILED=0
D4_DIR="$OUT_DIR/timeline-d4"
rm -rf "$D4_DIR"
mkdir -p "$D4_DIR"
cp "$FIXTURE" "$D4_DIR/source.wav"
D4_PROJECT="$D4_DIR/events.musi"
set +e
env -u WAYLAND_DISPLAY \
    DISPLAY="$DISPLAY_NUM" \
    PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
    ./target/debug/musializer --mute "$D4_DIR/source.wav" \
        --save-project "$D4_PROJECT" \
    >"$D4_DIR/save.txt" 2>&1
D4_SAVE=$?
set -e
if [ "$D4_SAVE" -ne 0 ] || [ ! -f "$D4_PROJECT" ]; then
    echo "FAIL: could not build the D4 event fixture project" >&2
    TIMELINE_D4_FAILED=1
else
    python3 tools/seed_event_fixture.py "$D4_PROJECT" 8

    d4_capture() {
        # d4_capture NAME PROBE [EXTRA ARGS...]
        local name="$1" probe="$2"
        shift 2
        set +e
        env -u WAYLAND_DISPLAY \
            DISPLAY="$DISPLAY_NUM" \
            PULSE_SERVER="unix:/nonexistent/musializer-headless-check" \
            ./target/debug/musializer --mute --project "$D4_PROJECT" \
                --size 1280x720 \
                --probe-frames 30 \
                --probe-shot "$OUT_DIR/d4-$name.png" \
                --ui-probe "$probe" "$@" \
            >"$OUT_DIR/d4-$name.txt" 2>&1
        local status=$?
        set -e
        if [ "$status" -ne 0 ]; then
            echo "FAIL: d4-$name exited $status" >&2
            TIMELINE_D4_FAILED=1
        fi
    }

    d4_line() { rg --no-filename '^timeline:' "$OUT_DIR/d4-$1.txt" || true; }

    d4_expect() {
        # d4_expect NAME PATTERN WHY
        local name="$1" pattern="$2" why="$3"
        local line
        line="$(d4_line "$name")"
        if printf '%s' "$line" | rg -q -- "$pattern"; then
            printf '  %-22s %s\n' "$name" "$why"
        else
            echo "FAIL: d4-$name: $why" >&2
            echo "      wanted /$pattern/ in: $line" >&2
            TIMELINE_D4_FAILED=1
        fi
    }

    # --- the markers ------------------------------------------------------
    # Whole track. All four type colours and both lanes are on screen at once,
    # which is what makes a colour collapse or a lost lane visible in one frame.
    d4_capture markers-whole "play=0,time=3"
    d4_expect markers-whole \
        'markers=manual:4 semantic:2 off-screen:0 off-track:0' \
        "six markers, four manual and two semantic"

    # The pixels. The expected list is in time order and its **last** entry is
    # the case that matters most: `manual:semantic` is a semantic-*typed* event
    # in the manual lane, which is what the `+ Feel` button records. It draws the
    # same amber as the two real semantic markers and must still have a filled
    # head, so a build that inferred the lane from the colour — or from the id's
    # high bit, which the merge's collision probe clears — fails here and only
    # here.
    if ! python3 tools/timeline_event_markers.py "$OUT_DIR/d4-markers-whole.png" \
        "manual:lyric,semantic:semantic,manual:cue,manual:custom,semantic:semantic,manual:semantic"
    then
        echo "FAIL: the event markers' colours or lanes are wrong" >&2
        TIMELINE_D4_FAILED=1
    fi

    # Zoomed 4x onto 2.25..4.25: four of the six fall outside the window and must
    # be **skipped**, not clamped to the edge. Clamping would pile them onto the
    # two border columns and read as a cluster that is not in the track — which
    # is a picture, not an error, so only the count says which happened.
    d4_capture markers-zoom "play=0,time=3,zoom=4"
    d4_expect markers-zoom \
        'markers=manual:2 semantic:0 off-screen:4 off-track:0' \
        "four markers off-screen at 4x, and skipped rather than clamped"

    # Off the *track*, which is a different bound (`plug.c:3089`) and is only
    # reachable live: `Project::validate_event_lanes` refuses a lane holding a
    # timestamp past the audio duration, so a .musi cannot carry one. 38 s on an
    # 8 s fixture.
    d4_capture markers-offtrack "play=0,time=3" --event "cue:38:99:1"
    d4_expect markers-offtrack \
        'markers=manual:4 semantic:2 off-screen:0 off-track:1' \
        "an event past the track end is counted and drawn nowhere"

    # The tooltip is the only place the head shapes are written down, so it is
    # load-bearing rather than decorative. Parked on the first marker's head.
    d4_capture markers-hover "play=0,time=3,hover=127x470"
    d4_expect markers-hover \
        'hover=\[manual lyric  ' \
        "a marker's tooltip names its lane and its type"

    # --- zoom versus pan --------------------------------------------------
    # Both notches are delivered at the same pointer position from the same 4x
    # view, so the *only* difference is the modifier. A bare notch multiplies the
    # span (2.000 -> 1.388); a Shift notch leaves it at exactly 2.000 and slides
    # the window. Asserting the span is what makes "it panned" a claim rather
    # than an impression.
    d4_capture wheel-zoom "play=0,time=3,zoom=4,hover=640x470,wheel=2"
    d4_expect wheel-zoom '5\.760x  2\.556\.\.3\.944' \
        "a bare notch zooms, and the span changed"

    d4_capture wheel-pan "play=0,time=3,zoom=4,hover=640x470,wheel=2,wheel-shift=1"
    d4_expect wheel-pan '4\.000x  1\.650\.\.3\.650' \
        "Shift-wheel up pans earlier and holds the span"
    d4_expect wheel-pan 'free-view' \
        "a wheel pan suspends playback-follow, as a drag pan does"

    d4_capture wheel-pan-back "play=0,time=3,zoom=4,hover=640x470,wheel=-2,wheel-shift=1"
    d4_expect wheel-pan-back '4\.000x  2\.850\.\.4\.850' \
        "Shift-wheel down pans later by the same amount"

    # A whole-track view has nowhere to pan. Accepting the notch anyway would
    # light the Follow button over a view that never moved, so the refusal has to
    # leave `free-view` off — which is the half a picture cannot show.
    d4_capture wheel-pan-whole "play=0,time=3,hover=640x470,wheel=2,wheel-shift=1"
    d4_expect wheel-pan-whole '1\.000x  0\.000\.\.8\.000  gesture=none  ' \
        "a Shift notch over a whole-track view is refused and does not free the view"

    # --- middle-drag pan, and the claim it must release --------------------
    # `--ui-probe middle-drag=` exists because `click=` cannot reach this: the
    # pan reads MOUSE_BUTTON_MIDDLE from raylib directly rather than through the
    # widget bank's pointer seam, since it claims nothing from the bank.
    #
    # `gesture=none` after the release is the assertion that matters. A stranded
    # claim is invisible in a picture — the view sits exactly where the hand left
    # it either way — and only shows up as the *next* interaction behaving
    # strangely, which is a bug report rather than a gate failure.
    d4_capture drag-pan "play=0,time=3,zoom=4,hover=640x470,middle-drag=900x400"
    d4_expect drag-pan '4\.000x  3\.044\.\.5\.044' \
        "a leftward middle-drag moves the window later and holds the span"
    d4_expect drag-pan 'free-view  gesture=none' \
        "the drag suspended follow and released its claim"

    d4_capture drag-pan-back "play=0,time=3,zoom=4,hover=640x470,middle-drag=400x900"
    d4_expect drag-pan-back '4\.000x  1\.456\.\.3\.456  free-view  gesture=none' \
        "and the reverse drag is symmetric, to the millisecond"

    # Released *outside the window*, which is the case the plan calls out by
    # name. The release must still be taken: a gesture that only ends when the
    # pointer is over its own lane strands the claim exactly when a user's hand
    # overshoots, which is the common way to end a fast drag.
    d4_capture drag-offwindow "play=0,time=3,zoom=4,hover=640x470,middle-drag=900x-400"
    d4_expect drag-offwindow 'free-view  gesture=none' \
        "a drag released off-window does not strand the pointer claim"

    d4_capture drag-pan-whole "play=0,time=3,hover=640x470,middle-drag=900x400"
    d4_expect drag-pan-whole '1\.000x  0\.000\.\.8\.000  gesture=none' \
        "a middle-drag over a whole-track view begins no gesture"

    # --- the tick labels' plate -------------------------------------------
    # Measured, not eyeballed. The plate is COLOR_UI_RAISED — opaque white — so a
    # label drawn without one has the waveform's own colours immediately behind
    # its glyphs. This reads the rectangle where a label sits and requires it to
    # be dominated by the raised surface.
    #
    # Its negative control is the thing that was actually shipped: deleting the
    # single `draw_rectangle_rec` call takes the bleed from **0 to 1444 pixels**
    # on this fixture — every one of the seven labels goes from 0 to about 200,
    # and the raised fraction behind the glyphs halves from ~0.45 to ~0.21 —
    # while all **1368 unit tests stay green**. A plate is not a value any
    # assertion in the tree pins, and it cannot be: it is white on white
    # everywhere the audio is quiet.
    if ! python3 tools/timeline_tick_plate.py "$OUT_DIR/d4-markers-whole.png"; then
        echo "FAIL: the tick labels have no opaque backing" >&2
        TIMELINE_D4_FAILED=1
    fi
fi
if [ "$TIMELINE_D4_FAILED" -ne 0 ]; then
    echo "FAIL: the timeline's markers, pan or tick labels regressed" >&2
    SWEEP_FAILED=1
fi

# Every successful application run prints this after its last frame, so this is
# broader than the first Spectrum capture above: welcome, every scene, every
# panel, preview and export all have to keep shell text on native atlases. The
# `UiFonts` type makes raw-face bypasses compile errors; this runtime half catches
# a new fractional design size that still reaches the bank but would otherwise be
# quantized without anybody noticing.
FONT_REPORTS="$(rg --no-filename '^fonts:' "$OUT_DIR" --glob '*.txt' || true)"
FONT_REPORT_COUNT="$(printf '%s\n' "$FONT_REPORTS" | sed '/^$/d' | wc -l)"
echo "native UI font reports: $FONT_REPORT_COUNT"
if [ "$FONT_REPORT_COUNT" -eq 0 ]; then
    echo "FAIL: no application run reported UI font usage" >&2
    SWEEP_FAILED=1
fi
if printf '%s\n' "$FONT_REPORTS" | rg -q 'non-native-requests=[1-9][0-9]*'; then
    echo "FAIL: a shell label requested a scaled/non-native UI font size" >&2
    printf '%s\n' "$FONT_REPORTS" | rg 'non-native-requests=[1-9][0-9]*' >&2
    SWEEP_FAILED=1
fi
if printf '%s\n' "$FONT_REPORTS" | rg -v -q 'ui=Space Grotesk \(17 native sizes\).*non-native-requests=0'; then
    echo "FAIL: an application run did not report the complete native-size UI bank" >&2
    printf '%s\n' "$FONT_REPORTS" | rg -v 'ui=Space Grotesk \(17 native sizes\).*non-native-requests=0' >&2
    SWEEP_FAILED=1
fi

if [ "$SWEEP_FAILED" -ne 0 ]; then
    echo "FAIL: at least one scene or alias capture failed" >&2
    exit 1
fi

echo "artifacts in $OUT_DIR"
