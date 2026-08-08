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
cargo run --quiet -p musializer-core --example settings_dump >"$OUT_DIR/settings_rust_full.txt"

# The tree has scenes the frozen C does not (Phosphor Dream, id 10, added
# 2026-08-08). The dump prints the C-era ten, a marker line, then the rest.
#
# Split rather than relax: the first section is still diffed against the oracle
# byte-for-byte, so every C-era bound stays a frozen contract, and the second is
# diffed against a checked-in expectation, so a post-legacy bound cannot drift
# either — it just fails against a file a person has to update on purpose
# instead of against a binary nobody can rebuild.
MARKER="--- post-legacy (no oracle) ---"
EXPECTED="tests/differential/settings_post_legacy.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} !found' \
    "$OUT_DIR/settings_rust_full.txt" >"$OUT_DIR/settings_rust.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} found' \
    "$OUT_DIR/settings_rust_full.txt" >"$OUT_DIR/settings_rust_post.txt"

# `--` because the marker starts with a dash and grep would read it as flags.
if ! grep -qxF -- "$MARKER" "$OUT_DIR/settings_rust_full.txt"; then
    echo "FAIL: the dump printed no '$MARKER' line. Either every scene is now in" \
         "the oracle (it is not) or the marker was renamed on one side only." >&2
    exit 1
fi

echo "=== comparing the C-era table (exact, against the frozen C) ==="
if ! diff -u "$OUT_DIR/settings_oracle.txt" "$OUT_DIR/settings_rust.txt"; then
    echo
    echo "FAIL: the Rust settings table diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi

echo "=== comparing the post-legacy table (exact, against the pinned file) ==="
if [ ! -f "$EXPECTED" ]; then
    echo "FAIL: $EXPECTED is missing. A post-legacy scene's descriptors have to" \
         "be pinned somewhere; copy $OUT_DIR/settings_rust_post.txt there and" \
         "read it before you commit it." >&2
    exit 1
fi
if ! diff -u "$EXPECTED" "$OUT_DIR/settings_rust_post.txt"; then
    echo
    echo "FAIL: a post-legacy scene's descriptors changed ('-' is the pinned" \
         "expectation, '+' is the code). These are still a .musi compatibility" \
         "surface: an out-of-range value silently becomes the default. If the" \
         "change is deliberate, update $EXPECTED and say why in the commit." >&2
    exit 1
fi

echo
echo "PASS: every descriptor field matches the frozen C exactly"
echo "      ($(grep -vc '^scene ' "$OUT_DIR/settings_oracle.txt") descriptors across 10 C-era scenes)"
echo "      plus $(grep -vc '^scene ' "$OUT_DIR/settings_rust_post.txt") pinned descriptors" \
     "in $(grep -c '^scene ' "$OUT_DIR/settings_rust_post.txt") post-legacy scene(s)"
