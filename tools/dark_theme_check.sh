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
# The readout button is read from the bare frame's `transport:` line (WX-5c:
# the row was rebuilt, and the old 1083x389 had been measured once by hand).
capture tooltip-bare black-amber 'panel=none,time=2,play=1'
read -r RX RY RW RH <<<"$(sed -n 's/^transport:.* readout=\([0-9]*\),\([0-9]*\),\([0-9]*\),\([0-9]*\).*/\1 \2 \3 \4/p' "$OUT/tooltip-bare.log")"
HOVER="$((RX + RW / 2))x$((RY + RH / 2))"
capture tooltip-dark black-amber "panel=none,time=2,play=1,hover=$HOVER"
capture tooltip-light light-blue "panel=none,time=2,play=1,hover=$HOVER"
capture editor-dark black-amber 'panel=none,time=2,play=1' --editor-ui
python3 - "$OUT" "$((RX + RW / 2))" "$RY" <<'PY'
import pathlib, sys
from PIL import Image
out = pathlib.Path(sys.argv[1])
cx, top = int(sys.argv[2]), int(sys.argv[3])
# The dark fill is WX-5a's tooltip surface; (48, 56, 42) was the
# pre-5a palette, stale since this script left the gate.
for name, fill in [('tooltip-dark', (38, 42, 49)),
                   ('tooltip-light', (20, 20, 20))]:
    im = Image.open(out / (name + '.png')).convert('RGB')
    # The tip sits centred over the button and ends a few pixels above it.
    pixels = list(im.crop((cx - 57, top - 29, cx + 57, top - 7)).get_flattened_data())
    assert pixels.count(fill) > 1500, f'{name}: tooltip surface missing'
    assert sum(min(pixel) > 180 for pixel in pixels) > 20, f'{name}: tooltip text missing'
print('PASS: both tooltip surfaces and readable text captured')
PY
