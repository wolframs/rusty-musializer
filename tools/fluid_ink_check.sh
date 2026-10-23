#!/usr/bin/env bash
# Fluid Ink's visible contract. Every app launch goes through the private,
# muted headless wrapper; PCM stays real. Artifacts are synthetic, under build/.
# --negative-control makes both treatments unresponsive: the music check MUST fail.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="${FLUID_CHECK_OUT:-build/fluid-ink-check}"
NEGATIVE=0
if [ "${1:-}" = "--negative-control" ]; then NEGATIVE=1; fi
mkdir -p "$OUT"
cargo run --quiet --bin make-fixture-wav -- "$OUT/pulse.wav" 6
python3 - "$OUT" "$NEGATIVE" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1]).resolve()
for name, response in [('music', 0 if sys.argv[2] == '1' else 1), ('ambient', 0)]:
    doc = dict(schema='musializer.treatment/v1', audio=dict(path=str(out/'pulse.wav')),
               output=dict(resolution='960x540', fps=30, quality='balanced'), seed=17,
               scenes=dict(fluid=dict(settings=dict(response=response))),
               segments=[dict(at=0, scene='fluid')])
    (out/f'{name}.treatment.json').write_text(json.dumps(doc))
PY
tools/headless_run.sh --treatment "$OUT/music.treatment.json" \
    --still 4 "$OUT/music.png" --render "$OUT/full.mp4" \
    --save-project "$OUT/fluid.musi" > "$OUT/music.log" 2>&1
tools/headless_run.sh --treatment "$OUT/ambient.treatment.json" \
    --still 4 "$OUT/ambient.png" > "$OUT/ambient.log" 2>&1
tools/headless_run.sh "$OUT/fluid.musi" --still 4 "$OUT/reopen.png" \
    --render-window 4 1 --render "$OUT/clip.mp4" > "$OUT/reopen.log" 2>&1
tools/headless_run.sh --treatment "$OUT/music.treatment.json" \
    --resolution 540x960 --still 4 "$OUT/portrait.png" > "$OUT/portrait.log" 2>&1
ffmpeg -v error -i "$OUT/full.mp4" -vf 'select=eq(n\,120)' -frames:v 1 -y "$OUT/full-4.png"
ffmpeg -v error -i "$OUT/clip.mp4" -frames:v 1 -y "$OUT/clip-4.png"
python3 - "$OUT" <<'PY'
import math, sys
from pathlib import Path
import numpy as np
from PIL import Image
out = Path(sys.argv[1])
def read(name):
    im = Image.open(out/name).convert('RGBA')
    a = np.asarray(im)
    assert np.all(a[:,:,3] == 255), f'{name}: translucent export'
    return a[:,:,:3].astype(float)
def compare(a, b, floor):
    mse = np.mean((read(a)-read(b))**2)
    psnr = 10*math.log10(255**2/mse) if mse else math.inf
    print(f'{a} / {b}: {psnr:.2f} dB')
    assert psnr > floor, f'{a}/{b}: lost simulation history'
music = read('music.png')
mad = np.mean(np.abs(music-read('ambient.png')))
print(f'Music response versus ambient, identical PCM/seed/time: MAD {mad:.3f}')
assert mad > 3.0, 'audio never reached the fluid forces'
assert music.std() > 20, 'flat frame'
assert np.mean(music.max(axis=2)>35) > 0.12, 'nearly empty frame'
compare('music.png', 'reopen.png', 60)
compare('music.png', 'full-4.png', 35)
compare('full-4.png', 'clip-4.png', 35)
portrait = read('portrait.png')
assert portrait.shape == (960, 540, 3)
assert portrait.std() > 20, 'portrait lost the ink'
log = (out/'music.log').read_text()
assert 'dye=uploaded, display=shader' in log, 'GPU path silently fell back'
print('PASS: music response, persistent replay, project reopen, still/video, clip, opaque output and portrait')
PY
