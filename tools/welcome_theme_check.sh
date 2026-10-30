#!/usr/bin/env bash
# Welcome appearance inherits the same saved choices as Look's footer.
# Actual theme presses/restarts are checked in pixel_look_check.sh; welcome
# picker ownership and contrast have headless egui input tests in egui_theme.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-build/welcome-theme-check}"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
for theme in black-amber light-blue; do
    state="$OUT/$theme-state"
    mkdir -p "$state/config/musializer"
    printf '{"schema_version":1,"theme":"%s"}\n' "$theme" > "$state/config/musializer/ui.json"
    MZ_STATE_DIR="$state" tools/headless_run.sh --ui-scale 100 \
        --ui-probe size=960x640 --probe-frames 24 --probe-shot "$OUT/$theme.png" > "$OUT/$theme.log" 2>&1
    grep -q "^appearance: *$theme " "$OUT/$theme.log"
done
printf 'PASS: welcome restored dark and light; evidence %s\n' "$OUT"
