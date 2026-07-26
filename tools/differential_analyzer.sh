#!/usr/bin/env bash
#
# Differential test: the frozen C analyzer versus the Rust port, on identical
# synthetic PCM.
#
# The plan asks for differential tests against the C implementation where
# practical, and the analyzer is the most practical of all: it is pure, it has no
# raylib dependency, and its output is a vector of numbers. Comparing those is
# stronger evidence than any amount of reading the port.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/audio_analyzer.c with all output going into our own build/
# directory. It never writes to the C tree and never invokes its `nob` build.
#
# Usage: tools/differential_analyzer.sh [TOLERANCE]
#
# TOLERANCE defaults to 1e-4. It is not zero on purpose: both sides run the same
# arithmetic in the same order, but `sinf`/`cosf`/`logf` come from libm in C and
# from Rust's intrinsics, and those are permitted to differ in the last bits.
# Band *indices* are compared exactly, because those are integers and any
# difference there is a real bug.

set -euo pipefail

TOLERANCE="${1:-1e-4}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

if [ ! -f "$ORACLE_SRC/audio_analyzer.c" ]; then
    echo "error: oracle source not found at $ORACLE_SRC" >&2
    exit 1
fi

echo "=== building the oracle's analyzer (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/analyzer_oracle" \
    tests/differential/analyzer_oracle.c \
    "$ORACLE_SRC/audio_analyzer.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/analyzer_oracle" >"$OUT_DIR/oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example analyzer_dump >"$OUT_DIR/rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/oracle.txt" "$OUT_DIR/rust.txt" "$TOLERANCE" <<'PY'
import sys

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

def load(path):
    bands = []
    count = None
    for line in open(path):
        parts = line.split()
        if not parts:
            continue
        if parts[0] == "band_count":
            count = int(parts[1])
            continue
        index, first, end, log, smooth, smear = parts
        bands.append((int(index), int(first), int(end),
                      float(log), float(smooth), float(smear)))
    return count, bands

oracle_count, oracle = load(oracle_path)
rust_count, rust = load(rust_path)

failures = []
if oracle_count != rust_count:
    failures.append("band_count differs: oracle %s, rust %s" % (oracle_count, rust_count))
if len(oracle) != len(rust):
    failures.append("row count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

worst = 0.0
worst_where = None
for o, r in zip(oracle, rust):
    if o[0] != r[0]:
        failures.append("row index mismatch: %d vs %d" % (o[0], r[0]))
        continue
    # Bin boundaries are integers. Any difference is a real indexing bug, so
    # these are compared exactly rather than within a tolerance.
    if o[1] != r[1] or o[2] != r[2]:
        failures.append("band %d bin range differs: oracle [%d,%d) rust [%d,%d)"
                        % (o[0], o[1], o[2], r[1], r[2]))
    for name, oi, ri in (("logarithmic", 3, 3), ("smooth", 4, 4), ("smear", 5, 5)):
        delta = abs(o[oi] - r[ri])
        if delta > worst:
            worst, worst_where = delta, "band %d %s (oracle %.9g, rust %.9g)" % (
                o[0], name, o[oi], r[ri])
        if delta > tolerance:
            failures.append("band %d %s differs by %.3g: oracle %.9g, rust %.9g"
                            % (o[0], name, delta, o[oi], r[ri]))

print("bands compared:   %d" % len(oracle))
print("largest delta:    %.3g%s" % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance:        %.3g" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)

print("\nPASS: the Rust analyzer matches the frozen C analyzer within tolerance.")
PY
