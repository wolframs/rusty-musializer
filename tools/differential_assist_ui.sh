#!/usr/bin/env bash
#
# Differential test: the frozen C's Assist panel policy versus the Rust port.
#
# `assist_ui_state.c` decides everything the Assist panel does before a pixel is
# drawn -- which of six bodies shows, how tall each one is, what the lyric
# reference row costs, where the mode grid breaks to two columns, why Start is
# unavailable, and the `<stem>.lyrics.txt` rule the Python helper uses. All of it
# came across by hand, and all of it is either a number or a sentence a reviewer
# would otherwise have to trust.
#
# The stem rule earns the harness on its own: it has to match Python `pathlib`'s,
# because the helper is Python, and a disagreement means the panel says "found"
# about a file the run never opens.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_assist_ui.sh

set -euo pipefail

ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

if [ ! -f "$ORACLE_SRC/assist_ui_state.c" ]; then
    echo "error: oracle source not found at $ORACLE_SRC" >&2
    exit 1
fi

echo "=== building the oracle's Assist policy (read-only) ==="
cc -O1 -std=c11 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/assist_ui_oracle" \
    tests/differential/assist_ui_oracle.c \
    "$ORACLE_SRC/assist_ui_state.c" \
    -lm

echo "=== running both ==="
"$OUT_DIR/assist_ui_oracle" >"$OUT_DIR/assist_ui_oracle.txt"
cargo run --quiet -p musializer-core --example assist_ui_dump >"$OUT_DIR/assist_ui_rust.txt"

echo "=== comparing (exact) ==="
# Exact, not tolerant: every value here is an integer, a pixel constant or a
# string. There is no arithmetic in this module whose last bits could differ
# between libm and Rust's intrinsics.
if diff -u "$OUT_DIR/assist_ui_oracle.txt" "$OUT_DIR/assist_ui_rust.txt"; then
    echo
    echo "PASS: the Assist panel policy matches the frozen C exactly"
    echo "      ($(wc -l <"$OUT_DIR/assist_ui_oracle.txt") decisions compared)"
else
    echo
    echo "FAIL: the Rust Assist policy diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi
