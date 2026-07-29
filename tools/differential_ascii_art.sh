#!/usr/bin/env bash
#
# Differential test: the frozen C `ascii_art` versus the Rust port, on identical
# synthetic RGBA8 pixel buffers.
#
# `ascii_art` is pure — pixels in, a grid of glyph cells out — so it is exactly
# the kind of module AGENTS.md says a number should settle rather than a
# paragraph of review. It was ported without a harness, and `--ascii-image` has
# since made its output reachable from the command line, which is what makes
# these numbers worth pinning down.
#
# Six surfaces are compared:
#
#   fit        ascii_art_fit_grid_dimensions over a 35 x 12 source/maximum sweep,
#              including zero dimensions, extreme aspect ratios and SIZE_MAX
#   layout     ascii_art_grid_layout, successes and rejections
#   populated  ascii_art_grid_is_populated
#   convert    ascii_art_convert_rgba8, every field of every cell over 36 cases
#              across 9 pixel fixtures, plus 10 rejection cases and the
#              atomicity of a rejected call
#   grid       plug.c:860-891's fit-then-convert composition (the --ascii-image
#              path) against the Rust `Grid::from_rgba8` that replaced it
#   anim       ascii_art_animated_glyph over cell x position x time x activity
#              x seed
#
# Result as of first run: 53847 records, 329738 compared values, largest float
# delta exactly 0 — the two sides agree bit for bit, including through
# `powf(x, 0.72f)`. That is stronger than the tolerance asks for; the tolerance
# stays because libm and Rust's intrinsics are permitted to differ and a future
# toolchain may take it up.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/ascii_art.c with all output going into our own build/
# directory. It never writes to the C tree and never invokes its `nob` build.
#
# Usage: tools/differential_ascii_art.sh [TOLERANCE]
#
# TOLERANCE defaults to 1e-6 and applies only to the two genuinely
# floating-point cell fields (`luminance`, `edge_strength`) and to
# `ascii_art_grid_layout`'s six floats, where it is scaled by magnitude because
# one case reaches 1e30. Everything else is compared EXACTLY: glyph codepoints,
# the four colour channels, edge orientations, grid dimensions and every boolean.
# A differing glyph is a real bug, not a rounding artifact.
#
# Note that `luminance` is a scaled integer either way — `display_luminance/255`
# — so its tolerance is not really absorbing arithmetic drift. What it protects
# against is only decimal formatting, because `display_luminance` runs through
# `powf(x, 0.72)` and a one-ULP difference there would move the integer by a
# whole 1/255 = 3.9e-3 and change the glyph. That would fail this tolerance
# loudly, which is the point.

set -euo pipefail

TOLERANCE="${1:-1e-6}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

if [ ! -f "$ORACLE_SRC/ascii_art.c" ]; then
    echo "error: oracle source not found at $ORACLE_SRC" >&2
    exit 1
fi

echo "=== building the oracle's ascii_art (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/ascii_art_oracle" \
    tests/differential/ascii_art_oracle.c \
    "$ORACLE_SRC/ascii_art.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/ascii_art_oracle" >"$OUT_DIR/ascii_art_oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example ascii_art_dump \
    >"$OUT_DIR/ascii_art_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/ascii_art_oracle.txt" "$OUT_DIR/ascii_art_rust.txt" "$TOLERANCE" <<'PY'
import sys
from collections import Counter

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

# Per record kind: how many leading tokens identify the record, then a
# (name, kind) pair per compared field. "i" is compared exactly, "f" within the
# tolerance, "fs" within a magnitude-scaled tolerance.
CELL_FIELDS = [
    ("glyph", "i"), ("red", "i"), ("green", "i"), ("blue", "i"), ("alpha", "i"),
    ("luminance", "f"), ("edge_strength", "f"), ("edge_orientation", "i"),
]
LAYOUT_FIELDS = [("ok", "i")] + [
    (name, "fs") for name in
    ("cell_width", "cell_height", "field_width", "field_height", "offset_x", "offset_y")
]

SCHEMA = {
    # key tokens                       compared fields
    "fit":       (4,  [("ok", "i"), ("columns", "i"), ("rows", "i")]),
    "layout":    (1,  LAYOUT_FIELDS),
    "populated": (2,  [("populated", "i")]),
    "convert":   (6,  [("ok", "i"), ("cell_count", "i")]),
    "cell":      (2,  CELL_FIELDS),
    "reject":    (1,  [("ok", "i"), ("guard0", "i"), ("guard1", "i")]),
    "grid":      (4,  [("ok", "i"), ("columns", "i"), ("rows", "i")]),
    "gridcell":  (2,  CELL_FIELDS),
    "anim":      (5,  [("glyph", "i")]),
}

# Divergences in mechanism, not in behaviour: the C takes bare pointers, the Rust
# takes slices, and each can express a rejection the other cannot. Asserted as an
# exact pair so the difference stays tested rather than merely described.
EXPECTED_DIVERGENCES = {
    "null_pixels":      ("c_rejects", "not_expressible"),
    "null_output":      ("c_rejects", "not_expressible"),
    "null_cell":        ("c_rejects", "not_expressible"),
    "truncated_pixels": ("c_cannot_detect", "rust_rejects"),
}


def load(path):
    records = []
    divergences = {}
    for line_number, line in enumerate(open(path), 1):
        parts = line.split()
        if not parts:
            continue
        kind = parts[0]
        if kind == "divergence":
            divergences[parts[1]] = parts[2]
            continue
        if kind not in SCHEMA:
            raise SystemExit("%s:%d: unknown record kind %r" % (path, line_number, kind))
        key_len, fields = SCHEMA[kind]
        body = parts[1:]
        if len(body) != key_len + len(fields):
            raise SystemExit("%s:%d: %s wants %d fields, got %d"
                             % (path, line_number, kind, key_len + len(fields), len(body)))
        key = (kind,) + tuple(body[:key_len])
        values = [int(token) if spec == "i" else float(token)
                  for (_, spec), token in zip(fields, body[key_len:])]
        records.append((kind, key, fields, values))
    return records, divergences


oracle, oracle_divergences = load(oracle_path)
rust, rust_divergences = load(rust_path)

failures = []
compared = 0
per_kind = Counter()
worst, worst_where = 0.0, None

if len(oracle) != len(rust):
    failures.append("record count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

for o, r in zip(oracle, rust):
    if o[1] != r[1]:
        failures.append("record key mismatch: %s vs %s" % (" ".join(o[1]), " ".join(r[1])))
        continue
    kind, key, fields, ovalues = o
    rvalues = r[3]
    label = " ".join(key)
    compared += len(ovalues)
    per_kind[kind] += len(ovalues)
    for (name, spec), ov, rv in zip(fields, ovalues, rvalues):
        if spec == "i":
            # Integers and enums are exact. A differing glyph, colour channel,
            # orientation or dimension is a real bug, never a rounding artifact.
            if ov != rv:
                failures.append("%s: %s differs: oracle %s, rust %s" % (label, name, ov, rv))
            continue
        scale = max(1.0, abs(ov), abs(rv)) if spec == "fs" else 1.0
        delta = abs(ov - rv)
        relative = delta/scale
        if relative > worst:
            worst, worst_where = relative, "%s %s (oracle %.9g, rust %.9g)" % (
                label, name, ov, rv)
        if relative > tolerance:
            failures.append("%s: %s differs by %.3g: oracle %.9g, rust %.9g"
                            % (label, name, delta, ov, rv))

for name, (want_oracle, want_rust) in sorted(EXPECTED_DIVERGENCES.items()):
    got_oracle = oracle_divergences.get(name)
    got_rust = rust_divergences.get(name)
    compared += 2
    per_kind["divergence"] += 2
    if got_oracle != want_oracle:
        failures.append("divergence %s: oracle said %r, expected %r"
                        % (name, got_oracle, want_oracle))
    if got_rust != want_rust:
        failures.append("divergence %s: rust said %r, expected %r"
                        % (name, got_rust, want_rust))

print("records compared:  %d" % min(len(oracle), len(rust)))
print("values compared:   %d" % compared)
for kind in sorted(per_kind):
    print("    %-10s %8d" % (kind, per_kind[kind]))
print("largest float delta: %.3g%s"
      % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance:           %.3g  (floats only; every integer field is exact)" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)

print("\nPASS: the Rust ascii_art matches the frozen C — every glyph, colour")
print("      channel, edge orientation and grid dimension exactly, and both")
print("      float fields within tolerance.")
PY
