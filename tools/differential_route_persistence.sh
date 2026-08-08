#!/usr/bin/env bash
#
# Differential test: the frozen C's route persistence versus the Rust port of the
# six functions item G moved out of `project::model` into `core::scene::routes`.
#
# `mapping_is_constant`, `constant_mapping`, `mappings_supported`,
# `export_mappings`, `import_mappings` and `parse_route_spec` are checked
# together, because `.musi` v1 has one representation for "this parameter has a
# value" and the constant rule and the route rule are two halves of one decision:
# a slider value is a full-range RMS mapping with equal output endpoints, an audio
# route is the same struct with unequal ones, and a parameter is persisted as
# exactly one of the two. Splitting the checks would let a change make a parameter
# eligible as both without either half's test noticing.
#
# The round trip covers **every descriptor of every scene**: exported as
# constants, exported again with routes replacing slots in place, then imported
# back with every restored value and every restored route printed.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_route_persistence.sh [TOLERANCE]

set -euo pipefail

TOLERANCE="${1:-1e-9}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

echo "=== building the oracle's route persistence (read-only) ==="
cc -O1 -std=c99 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/route_persistence_oracle" \
    tests/differential/route_persistence_oracle.c \
    "$ORACLE_SRC/scene_routes.c" \
    "$ORACLE_SRC/scene_settings.c" \
    "$ORACLE_SRC/project.c" \
    "$ORACLE_SRC/sha256.c" \
    "$ORACLE_SRC/event_timeline.c" \
    "$ORACLE_SRC/lyrics.c" \
    -lm

echo "=== running both ==="
"$OUT_DIR/route_persistence_oracle" >"$OUT_DIR/route_persistence_oracle.txt"
cargo run --quiet -p musializer-core --example route_persistence_dump \
    >"$OUT_DIR/route_persistence_rust_full.txt"

# The Rust registry has scenes the frozen C does not (Phosphor Dream, id 10,
# added 2026-08-08), and `export_mappings` walks the whole registry because a
# .musi must carry every scene. So the dump prints the C-era mappings, a marker,
# then the rest.
#
# Split rather than relax. The section above the marker is still compared to the
# oracle line-for-line, so the persistence grammar stays a frozen contract; the
# section below is compared to a checked-in expectation, so a post-legacy
# mapping cannot drift either — it fails against a file a person has to update
# on purpose.
MARKER="--- post-legacy (no oracle) ---"
EXPECTED="tests/differential/route_persistence_post_legacy.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} !found' \
    "$OUT_DIR/route_persistence_rust_full.txt" >"$OUT_DIR/route_persistence_rust.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} found' \
    "$OUT_DIR/route_persistence_rust_full.txt" >"$OUT_DIR/route_persistence_rust_post.txt"

# `--` because the marker starts with a dash and grep would read it as flags.
if ! grep -qxF -- "$MARKER" "$OUT_DIR/route_persistence_rust_full.txt"; then
    echo "FAIL: the dump printed no '$MARKER' line." >&2
    exit 1
fi

echo "=== comparing the post-legacy section (exact, against the pinned file) ==="
if [ ! -f "$EXPECTED" ]; then
    echo "FAIL: $EXPECTED is missing. Copy" \
         "$OUT_DIR/route_persistence_rust_post.txt there and read it first." >&2
    exit 1
fi
if ! diff -u "$EXPECTED" "$OUT_DIR/route_persistence_rust_post.txt"; then
    echo
    echo "FAIL: a post-legacy scene's persisted mappings changed ('-' is the" \
         "pinned expectation, '+' is the code). These go into real .musi files." >&2
    exit 1
fi

echo "=== comparing the C-era section ==="
python3 - "$OUT_DIR/route_persistence_oracle.txt" \
          "$OUT_DIR/route_persistence_rust.txt" "$TOLERANCE" <<'PY'
import sys

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

def load(path):
    return [line.split() for line in open(path) if line.split()]

oracle, rust = load(oracle_path), load(rust_path)
failures = []
if len(oracle) != len(rust):
    failures.append("line count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

def numeric(token):
    try:
        return float(token)
    except ValueError:
        return None

worst, worst_where = 0.0, None
for number, (o, r) in enumerate(zip(oracle, rust), start=1):
    if len(o) != len(r):
        failures.append("line %d: field count differs\n    oracle: %s\n    rust:   %s"
                        % (number, " ".join(o), " ".join(r)))
        continue
    for column, (a, b) in enumerate(zip(o, r)):
        if a == b:
            continue
        # Parameter keys, tags and every enum discriminant are exact; only the
        # seven float columns of a mapping get a tolerance, and they are the only
        # tokens that parse as a float and differ.
        x, y = numeric(a), numeric(b)
        if x is None or y is None:
            failures.append("line %d column %d: %r vs %r\n    oracle: %s\n    rust:   %s"
                            % (number, column, a, b, " ".join(o), " ".join(r)))
            continue
        delta = abs(x - y)
        if delta > worst:
            worst, worst_where = delta, "line %d column %d" % (number, column)
        if delta > tolerance:
            failures.append("line %d column %d differs by %.3g (oracle %.9g, rust %.9g)"
                            % (number, column, delta, x, y))

print("lines compared:  %d" % len(oracle))
print("largest delta:   %.3g%s" % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance:       %.3g" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)
print("\nPASS: route persistence matches the frozen C — the parse grammar, the")
print("      constant spelling, every descriptor's export/import round trip, and")
print("      the pairing that keeps a parameter from persisting as both.")
PY
