#!/usr/bin/env bash
# SX5 visual/export gate. Private Xvfb + unreachable Wayland/audio endpoints;
# --mute preserves real fixture PCM for the analyzer and exported soundtrack.
# Optional KNOT_BIN allows testing a negative-control binary without replacement.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-build/knot-check}"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
mkdir -p "$OUT/config"
export XDG_CONFIG_HOME="$OUT/config"
BIN="$(realpath "${KNOT_BIN:-target/debug/musializer}")"
GL=()
if [ "${MZ_GL_LAUNCH-unset}" != "" ] && command -v vglrun >/dev/null; then
    GL=("$(command -v vglrun)" -d "${MZ_VGL_DEVICE:-egl0}")
fi
# A short authored synthetic phrase: kick, bass, minor arpeggio and hats.
python3 - "$OUT/phrase.wav" <<'PY'
import math, struct, sys, wave
rate=44100
notes=[220,261.626,329.628,391.995,329.628,261.626,293.665,246.942]
with wave.open(sys.argv[1],'wb') as f:
    f.setparams((2,2,rate,0,'NONE','not compressed'))
    pcm=bytearray()
    for i in range(rate*8):
        t=i/rate; beat=t%0.5; eighth=t%0.25; note=notes[int(t*4)%8]
        kick=math.sin(2*math.pi*(48*beat+8*(1-math.exp(-beat*35))))*math.exp(-beat*18)*0.5
        bass=math.sin(2*math.pi*55*t)*(0.2+0.1*math.sin(t*math.pi))
        arp=(math.sin(2*math.pi*note*t)+0.3*math.sin(2*math.pi*note*3*t))*math.exp(-eighth*12)*0.2
        hat=(math.sin(t*70123)+math.sin(t*91371))*math.exp(-eighth*85)*0.07
        x=max(-1,min(1,kick+bass+arp+hat)); pcm.extend(struct.pack('<hh',int(x*30000),int(x*26000)))
    f.writeframes(pcm)
PY
launch() {
    xvfb-run -a -s '-screen 0 1600x1000x24' \
        env WAYLAND_DISPLAY=musializer-knot-check-no-compositor \
        PULSE_SERVER=unix:/tmp/musializer-knot-check-no-audio \
        "${GL[@]}" "$BIN" --mute "$@"
}
launch "$OUT/phrase.wav" --scene knot --hud=0 --size 1280x720 \
    --probe-frames 160 --probe-shot "$OUT/preview.png" > "$OUT/preview.log" 2>&1
launch "$OUT/phrase.wav" --scene knot --hud=0 --size 1280x720 --ui-probe play=0,time=0 \
    --probe-frames 4 --probe-shot "$OUT/idle.png" > "$OUT/idle.log" 2>&1
launch "$OUT/phrase.wav" --scene spectrum --hud=0 --size 960x640 \
    --ui-probe play=1,click=42x260 --probe-frames 16 --probe-shot "$OUT/minimum.png" > "$OUT/minimum.log" 2>&1
# Warm preview versus cold export: both must reconstruct identical scene state.
for mode in warm cold; do
    args=()
    if [ "$mode" = warm ]; then args=(--probe-frames 40); fi
    launch "$OUT/phrase.wav" --scene knot --render-window 1 4 \
        --resolution 960x540 --fps 30 "${args[@]}" \
        --render "$OUT/$mode.mp4" > "$OUT/$mode.log" 2>&1
done
launch "$OUT/phrase.wav" --scene openai-knot --render-window 1 1 \
    --resolution 360x640 --fps 30 --render "$OUT/portrait.mp4" > "$OUT/portrait.log" 2>&1
launch "$OUT/phrase.wav" --scene knot --render-window 1 1 \
    --route knot.size:rms:0:0:1:1.149:1.15 \
    --route knot.response:rms:0:0:1:1.999:2 \
    --route knot.pulse:rms:0:0:1:1.999:2 \
    --resolution 360x640 --fps 30 --render "$OUT/strong.mp4" > "$OUT/strong.log" 2>&1
# Matched response-off control: same PCM, time, seed, geometry and light clock.
launch "$OUT/phrase.wav" --scene knot --render-window 1 1 \
    --route knot.response:rms:0:0:1:0:0.000001 \
    --route knot.pulse:rms:0:0:1:0:0.000001 \
    --resolution 360x640 --fps 30 --render "$OUT/response-off.mp4" > "$OUT/response-off.log" 2>&1
# Identical musical frame without the new finish; route clamping yields exact zero.
launch "$OUT/phrase.wav" --scene knot --render-window 1 1 \
    --route knot.bloom:rms:0:0:1:-1:0 \
    --route knot.fringe:rms:0:0:1:-1:0 \
    --route knot.grain:rms:0:0:1:-1:0 \
    --resolution 360x640 --fps 30 --render "$OUT/finish-off.mp4" > "$OUT/finish-off.log" 2>&1
launch "$OUT/phrase.wav" --scene knot --route knot.polish:rms:0:0:1:0.25:0.8 \
    --save-project "$OUT/knot.musi" > "$OUT/save.log" 2>&1
launch --project "$OUT/knot.musi" --hud=0 --probe-frames 40 \
    --probe-shot "$OUT/reopened.png" > "$OUT/reopen.log" 2>&1
for mode in warm cold; do
    ffmpeg -v error -i "$OUT/$mode.mp4" -map 0:v:0 -f hash -hash sha256 - > "$OUT/$mode.hash"
done
cmp "$OUT/warm.hash" "$OUT/cold.hash"
ffmpeg -v error -i "$OUT/cold.mp4" -vf 'fps=4,scale=480:-1,tile=4x4' -frames:v 1 -y "$OUT/motion.png"
ffmpeg -v error -i "$OUT/portrait.mp4" -frames:v 1 -y "$OUT/portrait.png"
ffmpeg -v error -i "$OUT/response-off.mp4" -frames:v 1 -y "$OUT/response-off.png"
ffmpeg -v error -i "$OUT/strong.mp4" -frames:v 1 -y "$OUT/strong.png"
ffmpeg -v error -i "$OUT/finish-off.mp4" -frames:v 1 -y "$OUT/finish-off.png"
python3 tools/knot_check.py "$OUT"
echo "PASS: SX5 preview, idle, rhythmic motion, portrait, project/routes and warm/cold export ($OUT)"
