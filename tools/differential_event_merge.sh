#!/usr/bin/env bash
#
# Differential test: the frozen C event merge versus the Rust port.
#
# This harness exists because the Rust merge was first written from the header
# comment alone and got four things wrong: OR instead of XOR for namespacing
# semantic ids, no zero-avoidance, no collision probe, and a sort key missing
# `type`. Agents C and D read events through that contract. The lesson is the
# plan's own: read the implementation, not the header comment.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_event_merge.sh

set -euo pipefail

ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

echo "=== building the oracle's event merge (read-only) ==="
cc -O1 -std=c99 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/event_merge_oracle" \
    tests/differential/event_merge_oracle.c \
    "$ORACLE_SRC/scene_event_merge.c" \
    "$ORACLE_SRC/event_timeline.c" \
    -lm

echo "=== running both ==="
"$OUT_DIR/event_merge_oracle" >"$OUT_DIR/event_merge_oracle.txt"
cargo run --quiet -p musializer-core --example event_merge_dump >"$OUT_DIR/event_merge_rust.txt"

# The C prints its Event_Timeline_Result enum value while Rust prints a different
# error shape, so the OK marker is normalized on both sides. Every OK case, which
# is all of them, is then compared byte for byte — ids and ordering included.
normalize() { sed -E 's/result nonzero\([^)]*\)/result NONOK/; s/result 0 /result OK /; s/result [1-9][0-9]* /result NONOK /' "$1"; }

echo "=== comparing (exact) ==="
if diff -u <(normalize "$OUT_DIR/event_merge_oracle.txt") \
           <(normalize "$OUT_DIR/event_merge_rust.txt"); then
    echo
    echo "PASS: merged ids and canonical ordering match the frozen C exactly"
    echo "      ($(grep -c '^case ' "$OUT_DIR/event_merge_oracle.txt") cases, \
$(grep -vc '^case ' "$OUT_DIR/event_merge_oracle.txt") merged events)"
else
    echo
    echo "FAIL: the Rust event merge diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi
