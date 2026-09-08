#!/usr/bin/env bash
# Palette review with real decoded PCM; every launch is muted and isolated.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="$PWD/build/dark-theme-review"
mkdir -p "$OUT/config" "$OUT/state" "$OUT/data"
export WAYLAND_DISPLAY=musializer-dark-no-compositor
export PULSE_SERVER=unix:/nonexistent/musializer-dark
export XDG_CONFIG_HOME="$OUT/config" XDG_STATE_HOME="$OUT/state" XDG_DATA_HOME="$OUT/data"
export MUSIALIZER_UI_PREFERENCES="$OUT/ui.json"
cargo build --quiet --bin musializer --bin make-fixture-wav
./target/debug/make-fixture-wav "$OUT/source.wav" 8 > "$OUT/fixture.log"
TRACK="${MUSIALIZER_UX_CHECK_TRACK:-$OUT/source.wav}"
capture() {
    local name="$1" theme="$2" probe="$3"
    shift 3
    xvfb-run --auto-servernum --server-args='-screen 0 1280x720x24 -nolisten tcp' \
        ./target/debug/musializer --mute --size 1280x720 --ui-scale 100 \
        --ui-theme "$theme" "$TRACK" --ui-probe "$probe" \
        --probe-frames 12 --probe-shot "$OUT/$name.png" "$@" > "$OUT/$name.log" 2>&1
}
capture tooltip-dark black-amber 'panel=none,time=2,play=1,hover=1083x389'
capture tooltip-light light-blue 'panel=none,time=2,play=1,hover=1083x389'
capture editor-dark black-amber 'panel=none,time=2,play=1' --editor-ui
python3 - "$OUT" <<'PY'
import pathlib, sys
from PIL import Image
out = pathlib.Path(sys.argv[1])
for name, fill in [('tooltip-dark', (48, 56, 42)),
                   ('tooltip-light', (20, 20, 20))]:
    im = Image.open(out / (name + '.png')).convert('RGB')
    pixels = list(im.crop((1026, 343, 1140, 365)).get_flattened_data())
    assert pixels.count(fill) > 1500, f'{name}: tooltip surface missing'
    assert sum(min(pixel) > 180 for pixel in pixels) > 20, f'{name}: tooltip text missing'
print('PASS: both tooltip surfaces and readable text captured')
PY
