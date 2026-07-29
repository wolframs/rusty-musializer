#!/usr/bin/env bash
#
# Differential test: the frozen C shared preset store versus the Rust port.
#
# Four contracts, each of them one a hand-written port gets quietly wrong:
#
#   1. the scene tokens, which are *derived* from each scene's first persisted
#      setting key ("settings.loom.weight" -> "loom") rather than written down a
#      second time. A drift renames a scene in every store file ever written.
#   2. `preset_store_default_path`'s environment precedence, which decides where
#      a user's library lives.
#   3. the store document's exact JSON bytes, float formatting included. This is
#      the first harness to cover `musi_preset_store_serialize` at all.
#   4. `preset_store_merge`'s (imported, skipped) counts, whose identity rule is
#      "same scene and exactly equal values" and *not* the preset's name.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_preset_store.sh

set -euo pipefail

ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

echo "=== building the oracle's preset store (read-only) ==="
cc -O1 -std=c99 -D_POSIX_C_SOURCE=200809L \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/preset_store_oracle" \
    tests/differential/preset_store_oracle.c \
    "$ORACLE_SRC/preset_store.c" \
    "$ORACLE_SRC/scene_settings.c" \
    "$ORACLE_SRC/project_io.c" \
    "$ORACLE_SRC/project.c" \
    "$ORACLE_SRC/sha256.c" \
    "$ORACLE_SRC/event_timeline.c" \
    "$ORACLE_SRC/lyrics.c" \
    "$ORACLE_SRC/scene_routes.c" \
    -lm

echo "=== running both ==="
# The scratch store the oracle writes and reads back. Under our own build/, never
# anywhere near the operator's real library under $XDG_DATA_HOME.
"$OUT_DIR/preset_store_oracle" "$REPO_ROOT/$OUT_DIR/preset_store_scratch.json" \
    >"$OUT_DIR/preset_store_oracle.txt"
cargo run --quiet -p musializer-core --example preset_store_dump \
    >"$OUT_DIR/preset_store_rust.txt"

# The one place the two sides are *known* to spell things differently: C writes
# JSON numbers with `%.17g` (`project_io.c:48`), Rust with the shortest
# representation that round-trips. `project::io`'s own doc comment records that
# as a deliberate divergence — byte-identical JSON is an explicit non-goal, and
# the values are what a `.musi` promises. So every JSON number is normalized
# through a double on both sides before the comparison, which keeps the structure,
# the field order, the ids and the *values* under exact test while letting the
# spelling differ. Integers have no decimal point and are left alone.
normalize() {
    python3 -c '
import re, sys
pattern = re.compile(r"-?\d+\.\d+(?:[eE][-+]?\d+)?")
for line in open(sys.argv[1]):
    sys.stdout.write(pattern.sub(lambda m: repr(float(m.group(0))), line))
' "$1"
}

echo "=== comparing (structure and values exact; JSON number spelling normalized) ==="
if diff -u <(normalize "$OUT_DIR/preset_store_oracle.txt") \
           <(normalize "$OUT_DIR/preset_store_rust.txt"); then
    echo
    echo "PASS: scene tokens, path precedence, store bytes and merge counts match"
    echo "      the frozen C exactly ($(grep -c '^token ' "$OUT_DIR/preset_store_oracle.txt") \
scene tokens, $(grep -c ' preset ' "$OUT_DIR/preset_store_oracle.txt") presets, \
$(awk '/^store_bytes /{print length($0)-12}' "$OUT_DIR/preset_store_oracle.txt") bytes of store JSON)"
else
    echo
    echo "FAIL: the Rust preset store diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi
