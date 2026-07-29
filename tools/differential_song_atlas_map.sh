#!/usr/bin/env bash
#
# Differential test: the frozen C Song Atlas map versus the Rust port, on
# identical synthetic PCM.
#
# song_atlas_map is pure — interleaved PCM in, a bounded vector of slices out —
# so it is exactly the kind of module the project says a number should settle
# rather than a paragraph of review. The three passes (measure, smooth,
# normalize) are each order-sensitive in ways that still produce a plausible
# terrain when rearranged, and a plausible terrain is what review cannot tell
# apart from the right one.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/song_atlas_map.c and audio_analyzer.c with all output going
# into our own build/ directory. It never writes to the C tree and never invokes
# its `nob` build.
#
# Usage: tools/differential_song_atlas_map.sh [TOLERANCE]
#
# TOLERANCE defaults to 1e-9. The measured difference across all 29374 compared
# values is exactly **zero** — every float agrees bit for bit, because Rust's
# `sin`/`powf`/`sqrt` on this platform lower to the same libm the C links — so the
# default is margin against a libm that rounds differently, not room the port
# needs. Tighten it to 0 to assert bit-identity; anything looser than about 1e-6
# stops being a meaningful check, because the smoothing pass is a recursive filter
# and a real divergence grows along the track rather than staying in the last bits.
#
# Everything integral is compared exactly — slice counts, the `onset` flag, the
# helper functions' indices, and `valid` — because a difference there is a real
# bug rather than a rounding artifact. In particular a differing onset means the
# flux or loudness threshold landed on the other side of a boundary, which is
# visible in the terrain.

set -euo pipefail

TOLERANCE="${1:-1e-9}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

for source in song_atlas_map.c audio_analyzer.c; do
    if [ ! -f "$ORACLE_SRC/$source" ]; then
        echo "error: oracle source $source not found in $ORACLE_SRC" >&2
        exit 1
    fi
done

echo "=== building the oracle's song atlas map (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/song_atlas_map_oracle" \
    tests/differential/song_atlas_map_oracle.c \
    "$ORACLE_SRC/song_atlas_map.c" \
    "$ORACLE_SRC/audio_analyzer.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/song_atlas_map_oracle" >"$OUT_DIR/song_atlas_map_oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example song_atlas_map_dump \
    >"$OUT_DIR/song_atlas_map_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/song_atlas_map_oracle.txt" "$OUT_DIR/song_atlas_map_rust.txt" \
    "$TOLERANCE" <<'PY'
import sys

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])


def load(path):
    """Every line is a record: a kind, then tokens. Nothing here needs a parser."""
    rows = []
    for number, line in enumerate(open(path), start=1):
        tokens = line.split()
        if tokens:
            rows.append((number, tokens))
    return rows


def classify(token):
    """Integers and strings compare exactly; only finite exponential-form floats
    get the tolerance. Both harnesses print every float with an exponent for
    exactly this reason, so a float can never be mistaken for an integer here.

    Non-finite tokens deliberately fall through to the exact branch. Neither
    harness should ever emit one — every field is clamped into [0, 1] — and
    comparing NaN with a tolerance passes unconditionally, so treating it as a
    number would be a silent hole rather than a check. C spells it `nan` and Rust
    `NaN`, so the exact branch fails loudly instead."""
    try:
        return "int", int(token)
    except ValueError:
        pass
    try:
        value = float(token)
    except ValueError:
        return "str", token
    if value != value or value in (float("inf"), float("-inf")):
        return "str", token
    return "float", value


oracle = load(oracle_path)
rust = load(rust_path)

failures = []
if len(oracle) != len(rust):
    failures.append("line count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

compared = 0
exact = 0
worst = 0.0
worst_where = None
context = "(before the first fixture)"

for (oracle_line, oracle_tokens), (_, rust_tokens) in zip(oracle, rust):
    kind = oracle_tokens[0]
    if kind == "fixture":
        context = " ".join(oracle_tokens[1:])
    if kind != rust_tokens[0]:
        failures.append("line %d: record kind differs: %s vs %s"
                        % (oracle_line, kind, rust_tokens[0]))
        continue
    if len(oracle_tokens) != len(rust_tokens):
        failures.append("line %d (%s): field count differs: %d vs %d"
                        % (oracle_line, kind, len(oracle_tokens), len(rust_tokens)))
        continue
    label = context if kind == "slice" else kind
    for column, (o, r) in enumerate(zip(oracle_tokens[1:], rust_tokens[1:]), start=1):
        o_type, o_value = classify(o)
        r_type, r_value = classify(r)
        compared += 1
        if o_type == "float" and r_type == "float":
            delta = abs(o_value - r_value)
            if delta > worst:
                worst = delta
                worst_where = "%s line %d column %d (oracle %.10g, rust %.10g)" % (
                    label, oracle_line, column, o_value, r_value)
            if delta > tolerance:
                failures.append(
                    "%s line %d (%s) column %d differs by %.3g: oracle %.10g, rust %.10g"
                    % (label, oracle_line, kind, column, delta, o_value, r_value))
        else:
            exact += 1
            if o != r:
                failures.append("%s line %d (%s) column %d differs exactly: "
                                "oracle %s, rust %s"
                                % (label, oracle_line, kind, column, o, r))

slices = sum(1 for _, tokens in oracle if tokens[0] == "slice")
fixtures = sum(1 for _, tokens in oracle if tokens[0] == "fixture")
helpers = sum(1 for _, tokens in oracle if tokens[0].startswith("helper_"))

print("fixtures:         %d" % fixtures)
print("slices:           %d" % slices)
print("helper calls:     %d" % helpers)
print("values compared:  %d  (%d of them exactly)" % (compared, exact))
print("largest delta:    %.3g%s"
      % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance:        %.3g" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)

print("\nPASS: the Rust Song Atlas map matches the frozen C map within tolerance.")
PY
