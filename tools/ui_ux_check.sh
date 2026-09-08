#!/usr/bin/env bash
# Targeted visual UX captures. Every app process is muted, denied the desktop
# audio sink, isolated from Wayland, and given private preferences/config.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${MUSIALIZER_UX_CHECK_OUT:-$ROOT/build/ux-review}"
mkdir -p "$OUT/config" "$OUT/state" "$OUT/data"
OUT="$(cd "$OUT" && pwd)"
TRACK="${MUSIALIZER_UX_CHECK_TRACK:-$OUT/source.wav}"

export WAYLAND_DISPLAY=musializer-ux-check-no-compositor
export PULSE_SERVER=unix:/nonexistent/musializer-ux-check
export MUSIALIZER_UI_PREFERENCES="$OUT/ui-preferences.json"
export XDG_CONFIG_HOME="$OUT/config"
export XDG_STATE_HOME="$OUT/state"
export XDG_DATA_HOME="$OUT/data"

BIN="$ROOT/target/debug/musializer"
if [[ "${MUSIALIZER_UX_SKIP_BUILD:-0}" != 1 ]]; then
    cargo build --quiet --bin musializer --bin make-fixture-wav
fi
if [[ -z "${MUSIALIZER_UX_CHECK_TRACK:-}" ]]; then
    # Reproducible default without a dependency on the author's music library.
    # PCM remains real; process mute below keeps the check silent.
    "$ROOT/target/debug/make-fixture-wav" "$TRACK" 30 > "$OUT/fixture.log"
fi

capture() {
    local name="$1" size="$2" theme="$3" probe="$4"
    shift 4
    xvfb-run --auto-servernum --server-args="-screen 0 ${size}x24 -nolisten tcp" \
        "$BIN" --mute --size "$size" --ui-scale 100 --ui-theme "$theme" \
        "$@" --ui-probe "$probe" --probe-frames 12 --probe-shot "$OUT/$name.png" \
        >"$OUT/$name.log" 2>&1
}

capture_welcome() {
    local name="$1" size="$2" theme="$3"
    xvfb-run --auto-servernum --server-args="-screen 0 ${size}x24 -nolisten tcp" \
        "$BIN" --mute --size "$size" --ui-scale 100 --ui-theme "$theme" \
        --probe-frames 12 --probe-shot "$OUT/$name.png" >"$OUT/$name.log" 2>&1 || \
        [[ -s "$OUT/$name.png" ]]
}

capture_welcome welcome-light-960 960x640 light-blue
capture_welcome welcome-dark-1280 1280x900 black-amber

for size in 960x640 1280x900; do
    suffix="${size%x*}"
    capture "workspace-light-$suffix" "$size" light-blue panel=none,time=24,play=1 "$TRACK"
    capture "tune-light-$suffix" "$size" light-blue panel=tune,time=24,play=0 "$TRACK"
    capture "lyrics-light-$suffix" "$size" light-blue panel=lyrics,time=24,play=0 "$TRACK"
    capture "editor-light-$suffix" "$size" light-blue panel=none,time=24,play=0 --editor-ui "$TRACK"
    capture "assist-dark-$suffix" "$size" black-amber panel=assist,time=24,play=0 "$TRACK"
    capture "export-dark-$suffix" "$size" black-amber panel=export,time=24,play=0 "$TRACK"
done

capture assist-failed-dark-960 960x640 black-amber panel=assist,assist=failed,time=24,play=0 "$TRACK"

printf 'UX captures: %s\n' "$OUT"
