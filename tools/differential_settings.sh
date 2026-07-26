#!/usr/bin/env bash
#
# Differential test: the frozen C scene-settings descriptor table versus the
# hand-transcribed Rust one.
#
# ~85 descriptors were typed by hand from ../musializer/src/scene_settings.c.
# Every field is a compatibility surface, and an out-of-range value silently
# becomes the default rather than being clamped, so a single mistyped bound
# surfaces much later as a scene quietly ignoring a saved setting. This compares
# every field exactly.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_settings.sh

set -euo pipefail

ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

if [ ! -f "$ORACLE_SRC/scene_settings.c" ]; then
    echo "error: oracle source not found at $ORACLE_SRC" >&2
    exit 1
fi

echo "=== building the oracle's settings table (read-only) ==="
cc -O1 -std=c99 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/settings_oracle" \
    tests/differential/settings_oracle.c \
    "$ORACLE_SRC/scene_settings.c" \
    "$ORACLE_SRC/project.c" \
    "$ORACLE_SRC/sha256.c" \
    "$ORACLE_SRC/event_timeline.c" \
    "$ORACLE_SRC/lyrics.c" \
    "$ORACLE_SRC/scene_routes.c" \
    -lm

echo "=== running both ==="
"$OUT_DIR/settings_oracle" >"$OUT_DIR/settings_oracle.txt"
cargo run --quiet -p musializer-core --example settings_dump >"$OUT_DIR/settings_rust.txt"

echo "=== comparing (exact) ==="
if diff -u "$OUT_DIR/settings_oracle.txt" "$OUT_DIR/settings_rust.txt"; then
    echo
    echo "PASS: every descriptor field matches the frozen C exactly"
    echo "      ($(grep -vc '^scene ' "$OUT_DIR/settings_oracle.txt") descriptors across 10 scenes)"
else
    echo
    echo "FAIL: the Rust settings table diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi
