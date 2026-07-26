#!/usr/bin/env bash
#
# Differential test: the frozen C route evaluation versus the Rust port, over a
# sweep of source values for every interpolation curve and both clamp settings,
# against both a slider target and a toggle target.
#
# Three layers of hand-ported edge cases are checked here: the clamped/unclamped
# asymmetry in `musi_mapping_evaluate`, the descriptor-range clamp, and the toggle
# midpoint quantization. The source sweep deliberately runs outside 0..1 so the
# clamped and unclamped paths diverge where they should.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_routes.sh [TOLERANCE]

set -euo pipefail

TOLERANCE="${1:-1e-9}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

echo "=== building the oracle's route evaluation (read-only) ==="
cc -O1 -std=c99 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/routes_oracle" \
    tests/differential/routes_oracle.c \
    "$ORACLE_SRC/scene_routes.c" \
    "$ORACLE_SRC/scene_settings.c" \
    "$ORACLE_SRC/project.c" \
    "$ORACLE_SRC/sha256.c" \
    "$ORACLE_SRC/event_timeline.c" \
    "$ORACLE_SRC/lyrics.c" \
    -lm

echo "=== running both ==="
"$OUT_DIR/routes_oracle" >"$OUT_DIR/routes_oracle.txt"
cargo run --quiet -p musializer-core --example routes_dump >"$OUT_DIR/routes_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/routes_oracle.txt" "$OUT_DIR/routes_rust.txt" "$TOLERANCE" <<'PY'
import sys

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

def load(path):
    rows = []
    for line in open(path):
        p = line.split()
        if not p:
            continue
        # parameter curve clamp step have_eval eval have_mapped mapped
        rows.append((p[0], int(p[1]), int(p[2]), int(p[3]),
                     int(p[4]), float(p[5]), int(p[6]), float(p[7])))
    return rows

oracle, rust = load(oracle_path), load(rust_path)
failures = []
if len(oracle) != len(rust):
    failures.append("row count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

worst, worst_where = 0.0, None
for o, r in zip(oracle, rust):
    label = "%s curve=%d clamp=%d step=%d" % (o[0], o[1], o[2], o[3])
    if o[:4] != r[:4]:
        failures.append("row key mismatch: %s vs %s" % (o[:4], r[:4]))
        continue
    # Whether a value is produced at all is a boolean contract, compared exactly.
    if o[4] != r[4]:
        failures.append("%s: evaluate() success differs (oracle %d, rust %d)" % (label, o[4], r[4]))
    if o[6] != r[6]:
        failures.append("%s: output_value() success differs (oracle %d, rust %d)" % (label, o[6], r[6]))
    for name, idx, ok in (("evaluate", 5, o[4]), ("output_value", 7, o[6])):
        if not ok:
            continue
        delta = abs(o[idx] - r[idx])
        if delta > worst:
            worst, worst_where = delta, "%s %s" % (label, name)
        if delta > tolerance:
            failures.append("%s: %s differs by %.3g (oracle %.9g, rust %.9g)"
                            % (label, name, delta, o[idx], r[idx]))

print("rows compared:   %d" % len(oracle))
print("largest delta:   %.3g%s" % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance:       %.3g" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)
print("\nPASS: route evaluation matches the frozen C, including the clamp asymmetry")
print("      and the toggle midpoint quantization.")
PY
