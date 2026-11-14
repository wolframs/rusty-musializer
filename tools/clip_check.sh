#!/usr/bin/env bash
#
# The clip gate (VG, 2026-11-14): a section that plays a video file, checked
# end to end on synthetic clips through the export's own still path.
#
#   tools/clip_check.sh [OUTPUT_DIR]        # default build/clip-check
#
# Run from tools/treatment_check.sh, so every caller of that gate runs this one.
# Every launch goes through tools/headless_run.sh (private Xvfb, --mute, the
# dialog PATH guard); the clips are FFmpeg's testsrc2, which burns a frame
# counter into the picture, made here and never committed.
#
# CLIP_BIN names a different binary, for a negative control without replacing
# the build. CLIP_RENDERERS chooses the GL paths the exactness stills are taken
# on: "gpu cpu" by default (the GPU through VirtualGL, and llvmpipe with
# MZ_GL_LAUNCH=""), per docs/HEADLESS_GPU.md.
#
# What it asserts, and against what, is in tools/clip_check.py.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/clip-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${CLIP_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"
RENDERERS="${CLIP_RENDERERS:-gpu cpu}"

# A fixed display (MUSIALIZER_CAPTURE_DISPLAY) gets one gate-owned Xvfb for the
# whole series, killed and waited for on exit: launch after launch on one
# fixed display otherwise finds the previous server still exiting and rightly
# refuses it as foreign (docs/TRAPS.md, "Sequential fixed-display launches").
# A server already there after two seconds is an ancestor's, which every
# headless_run checks before it reuses it. As tools/look_cards_check.sh.
if [ -n "${MUSIALIZER_CAPTURE_DISPLAY:-}" ]; then
    if [[ ! "$MUSIALIZER_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
        echo 'clip_check: capture display must be :NUMBER' >&2
        exit 2
    fi
    NUMBER="${BASH_REMATCH[1]}"
    for _ in {1..40}; do
        [ -e "/tmp/.X11-unix/X$NUMBER" ] || [ -e "/tmp/.X$NUMBER-lock" ] || break
        sleep 0.05
    done
    if [ ! -e "/tmp/.X11-unix/X$NUMBER" ] && [ ! -e "/tmp/.X$NUMBER-lock" ]; then
        Xvfb "$MUSIALIZER_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp \
            >"$OUT/xvfb.log" 2>&1 &
        XVFB_PID=$!
        cleanup() {
            kill "$XVFB_PID" 2>/dev/null || true
            wait "$XVFB_PID" 2>/dev/null || true
        }
        trap cleanup EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM HUP
        READY=0
        for _ in {1..100}; do
            if xdpyinfo -display "$MUSIALIZER_CAPTURE_DISPLAY" >/dev/null 2>&1; then
                READY=1
                break
            fi
            kill -0 "$XVFB_PID" 2>/dev/null || break
            sleep 0.05
        done
        if [ "$READY" -ne 1 ]; then
            echo "clip_check: owned Xvfb did not become ready; see $OUT/xvfb.log" >&2
            exit 2
        fi
    fi
fi

run() {
    tools/headless_run.sh "$@"
}
# A launch that is expected to be refused: its status goes to a file.
refused() {
    local name="$1"
    shift
    local status=0
    run "$@" > "$OUT/$name.log" 2>&1 || status=$?
    echo "$status" > "$OUT/$name.status"
}

ff() {
    ffmpeg -nostdin -v error -y "$@"
}

# --- fixtures -------------------------------------------------------------------
cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 12
ff -f lavfi -i anullsrc=r=44100:cl=stereo -t 12 -c:a pcm_s16le "$OUT/silent.wav"
# 23.976 fps, 16:9, a keyframe every second: frame times no 24 fps guess hits.
ff -f lavfi -i testsrc2=size=640x360:rate=24000/1001 -t 6.006 -c:v libx264 -g 24 \
    -pix_fmt yuv420p "$OUT/ntsc.mp4"
# Square at 25 fps, for cover against contain.
ff -f lavfi -i testsrc2=size=360x360:rate=25 -t 4 -c:v libx264 -g 25 -pix_fmt yuv420p \
    "$OUT/square.mp4"
# A variable rate: sixty frames at 30 fps, then thirty at 15.
ff -f lavfi -i testsrc2=size=640x360:rate=30 -frames:v 90 \
    -vf "setpts='if(lt(N,60),N/30,2+(N-60)/15)/TB'" -fps_mode passthrough \
    -c:v libx264 -g 30 -pix_fmt yuv420p "$OUT/vfr.mp4"
# A phone clip: 320x180 pixels and a display matrix that turns them a quarter,
# so a player shows it 180x320, upright.
ff -f lavfi -i testsrc2=size=320x180:rate=25 -t 2 -c:v libx264 -g 25 -pix_fmt yuv420p \
    "$OUT/flat.mp4"
ff -display_rotation:v:0 90 -i "$OUT/flat.mp4" -c copy "$OUT/turned.mp4"
# Hostile: the first half of a clip (its index is at the end), and 53 bytes
# claiming a 16000x9000 picture.
head -c "$(( $(stat -c %s "$OUT/ntsc.mp4") / 2 ))" "$OUT/ntsc.mp4" > "$OUT/truncated.mp4"
printf 'YUV4MPEG2 W16000 H9000 F30:1 Ip A1:1 C420jpeg\nFRAME\n\0\0\0\0' > "$OUT/huge.y4m"

# --- treatments -----------------------------------------------------------------
# The music is switched off in every exactness treatment: punch, pulse, drift,
# shake and tint at 0 leave the clip frame as it is, so a still can be compared
# with the frame FFmpeg extracts from the clip.
QUIET='"scenes": { "clip": { "settings": { "punch": 0, "pulse": 0, "drift": 0, "shake": 0, "tint": 0 } } }'
# At the clips' own size, so a still samples the clip texel for texel and is
# compared with FFmpeg's frame unscaled: two different downscalers would only
# measure each other.
cat > "$OUT/exact.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 30, "quality": "balanced" }, "seed": 7,
  $QUIET,
  "segments": [ { "at": 0, "scene": "spectrum" },
                { "at": 1, "scene": "clip", "clip": "ntsc.mp4" },
                { "at": 7.5, "scene": "clip", "clip": { "path": "vfr.mp4", "from": 0.5 } } ] }
EOF
# The same clip three times from 5 s in, so each outlasts it after a second.
cat > "$OUT/overrun.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 30, "quality": "balanced" }, "seed": 7,
  $QUIET,
  "segments": [ { "at": 0, "scene": "spectrum" },
                { "at": 1, "scene": "clip", "clip": { "path": "ntsc.mp4", "from": 5, "after": "hold" } },
                { "at": 4, "scene": "clip", "clip": { "path": "ntsc.mp4", "from": 5, "after": "loop" } },
                { "at": 7, "scene": "clip", "clip": { "path": "ntsc.mp4", "from": 5, "after": "bounce" } } ] }
EOF
cat > "$OUT/fit.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "320x180", "fps": 30, "quality": "balanced" }, "seed": 7,
  $QUIET,
  "segments": [ { "at": 0, "scene": "clip", "clip": "square.mp4" },
                { "at": 6, "scene": "clip", "clip": { "path": "square.mp4", "from": 0, "fit": "contain" } } ] }
EOF
# The turned clip at its displayed size, so the still samples it texel for
# texel against the frame FFmpeg shows.
cat > "$OUT/turned.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "180x320", "fps": 25, "quality": "balanced" }, "seed": 7,
  $QUIET,
  "segments": [ { "at": 0, "scene": "clip", "clip": "turned.mp4" } ] }
EOF
# A clip section, then the video moves on: once Spectrum draws, the clip's
# decoder, its frames and its worker are gone, whatever scene is drawing.
cat > "$OUT/release.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "320x180", "fps": 30, "quality": "balanced" }, "seed": 7,
  "segments": [ { "at": 0, "scene": "clip", "clip": "ntsc.mp4" }, { "at": 2, "scene": "spectrum" } ] }
EOF
# The music on (defaults), at response 0 and 1, over the fixture and silence.
for response in 0 1; do
    for audio in fixture silent; do
        cat > "$OUT/response-$response-$audio.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "$audio.wav" },
  "output": { "resolution": "320x180", "fps": 30, "quality": "balanced" }, "seed": 7,
  "scenes": { "clip": { "settings": { "response": $response, "shake": 1 } } },
  "segments": [ { "at": 0, "scene": "clip", "clip": { "path": "ntsc.mp4", "after": "loop" } } ] }
EOF
    done
done
# Refusals: an unreadable clip, a huge claim, a missing file.
for bad in truncated huge missing; do
    case "$bad" in
        truncated) file="truncated.mp4" ;;
        huge) file="huge.y4m" ;;
        missing) file="nowhere.mp4" ;;
    esac
    cat > "$OUT/bad-$bad.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "spectrum" }, { "at": 2, "scene": "clip", "clip": "$file" } ] }
EOF
done

# --- runs -----------------------------------------------------------------------
# Exact frames: 23.976 at clip times on and near frame edges, then the variable
# rate. Each still in its own process too would be slower and prove less: the
# stills of one pass come out of one replay, as an export's frames do.
EXACT_TIMES="1.5 2.0 3.4 6.0 6.9 7.6 9.5 10.1"
for renderer in $RENDERERS; do
    args=()
    for t in $EXACT_TIMES; do
        args+=(--still "$t" "$OUT/exact-$renderer-$t.png")
    done
    if [ "$renderer" = cpu ]; then
        MZ_GL_LAUNCH="" run --treatment "$OUT/exact.treatment.json" "${args[@]}" \
            > "$OUT/exact-$renderer.log" 2>&1
    else
        run --treatment "$OUT/exact.treatment.json" "${args[@]}" > "$OUT/exact-$renderer.log" 2>&1
    fi
done
# One still per process as well, for the report line's frame index.
for t in 6.0 9.5; do
    run --treatment "$OUT/exact.treatment.json" --still "$t" "$OUT/alone-$t.png" \
        > "$OUT/alone-$t.log" 2>&1
done

run --treatment "$OUT/overrun.treatment.json" \
    --still 2.5 "$OUT/hold.png" --still 5.5 "$OUT/loop.png" --still 8.5 "$OUT/bounce.png" \
    --still 9.5 "$OUT/bounce-late.png" > "$OUT/overrun.log" 2>&1

run --treatment "$OUT/fit.treatment.json" \
    --still 2.0 "$OUT/cover.png" --still 8.0 "$OUT/contain.png" > "$OUT/fit.log" 2>&1

run --treatment "$OUT/turned.treatment.json" --still 1.0 "$OUT/turned.png" > "$OUT/turned.log" 2>&1
run --treatment "$OUT/release.treatment.json" --still 1.0 "$OUT/release-clip.png" \
    --still 8.0 "$OUT/release-after.png" > "$OUT/release.log" 2>&1

for response in 0 1; do
    for audio in fixture silent; do
        run --treatment "$OUT/response-$response-$audio.treatment.json" \
            --still 4.25 "$OUT/response-$response-$audio.png" \
            > "$OUT/response-$response-$audio.log" 2>&1
    done
done

# Two exports of the same window, the encoder held fixed (x264, the default):
# the same frames, by FFmpeg's per-frame digests.
for take in a b; do
    MZ_ENCODER=x264 run --treatment "$OUT/exact.treatment.json" --render-window 5.8 1.2 \
        --encoder x264 --render "$OUT/export-$take.mp4" > "$OUT/export-$take.log" 2>&1
    ff -i "$OUT/export-$take.mp4" -map 0:v -f framemd5 "$OUT/export-$take.framemd5"
done

# Saved with the clip, its source deleted, reopened from the bundle; then the
# bundled copy deleted too, reopened onto the placeholder.
mkdir -p "$OUT/project"
cp "$OUT/ntsc.mp4" "$OUT/project/take.mp4"
cp "$OUT/fixture.wav" "$OUT/project/fixture.wav"
sed 's/"ntsc.mp4"/"take.mp4"/; s/"vfr.mp4"/"take.mp4"/' "$OUT/exact.treatment.json" \
    > "$OUT/project/take.treatment.json"
run --treatment "$OUT/project/take.treatment.json" --save-project "$OUT/project/take.musi" \
    --still 6.0 "$OUT/project-before.png" > "$OUT/project-save.log" 2>&1
rm "$OUT/project/take.mp4"
run --project "$OUT/project/take.musi" --still 6.0 "$OUT/project-reopened.png" \
    > "$OUT/project-reopen.log" 2>&1
rm "$OUT"/project/take.assets/clips/*
run --project "$OUT/project/take.musi" --still 6.0 "$OUT/project-missing.png" \
    > "$OUT/project-missing.log" 2>&1

for bad in truncated huge missing; do
    refused "bad-$bad" --treatment "$OUT/bad-$bad.treatment.json" --still 3 "$OUT/bad-$bad.png"
done

python3 tools/clip_check.py "$OUT" $RENDERERS

# VG H4: asking a video model for a clip, against a fake provider on loopback,
# on this gate's display.
GENERATE_BIN="${CLIP_BIN:-}" tools/generate_check.sh "$(dirname "$OUT")/generate-check"
echo "PASS: clip frames, overruns, fit, a turned clip, release, response 0, export determinism, bundle, placeholder, refusals ($OUT)"
