#!/usr/bin/env bash
#
# Differential test: the frozen C timeline view versus the Rust port, over an
# identical grid of view states, strip geometries and operation sequences.
#
# timeline_view is the single seconds<->pixel conversion the whole timeline strip
# goes through: the tick ladder, the playhead, the scrubber, the event markers, the
# lyric blocks and the waveform envelope's per-pixel-column bin range all call into
# it, deliberately, so that they cannot disagree about where a moment is
# (`timeline_view.h:6-11`). That design is also why a drift here is invisible to
# review and to a capture: if the view is wrong, every element on the strip is
# wrong *consistently*, so the strip still photographs as a self-coherent picture.
#
# The port's own unit tests were almost entirely property assertions --
# `is_finite()`, `span <= duration + 1e-9`, `span/step >= 2.0` -- and a genuinely
# wrong formula satisfies all three. The C's own test file pins 32 exact values at
# 1e-9, but hand-transcribing those risks a typo or, worse, a number copied from
# our own output. A differential comparison cannot be tautological, which is why
# this exists instead.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/timeline_view.c with all output going into our own build/
# directory. It never writes to the C tree and never invokes its `nob` build.
#
# Usage: tools/differential_timeline_view.sh [TOLERANCE]
#
# TOLERANCE is *relative*, against max(1, |oracle|, |rust|), and defaults to
# 1e-12. Relative rather than absolute because x_at legitimately returns tens of
# millions of pixels for a moment far outside a deeply zoomed window, where an
# absolute 1e-12 is finer than a double can represent.
#
# The measured difference across all 204953 compared values is exactly **zero** --
# every float agrees bit for bit, and `0` passes -- so the default is margin, not
# room the port needs. It is three orders tighter than the C's own 1e-9 because
# this module calls no libm function at all: every value comes out of comparisons
# and the four arithmetic operators on `double`, so there is no platform maths to
# differ in the last bits. Two mechanisms could still open a gap, and 1e-12 is
# sized to absorb exactly those and nothing larger:
#
#   - a compiler contracting `anchor - fraction*span` into an fnmadd on a machine
#     where FMA is in the baseline, which changes one rounding;
#   - an algebraically-equivalent reassociation of a conversion. Rewriting
#     seconds_at's `(x - left)/width*span` as `(x - left)*span/width` -- the sort
#     of tidy-up a later reader would call harmless -- was measured at a worst
#     relative delta of 5.7e-16, about 2.5 ULP. That is below the millisecond the
#     lyric bridge format round-trips and well below a pixel, so it is not a parity
#     bug; the three controls that *are* bugs came in at 0.0625 to 0.75 relative,
#     thirteen orders above the tolerance. Pass 0 to forbid reassociation too.
#
# The pass line prints the worst delta either way, so a run that stops being
# bit-identical is visible even while it still passes.
#
# Everything integral is compared exactly -- the `is_whole` flag, the view-kind and
# step indices, and the operation names -- because a difference there is a real bug
# rather than a rounding artifact. Non-finite values are printed as labels
# (`not_a_number`, `positive_infinity`, `negative_infinity`) by both sides rather
# than as `nan`/`inf`: those spellings *parse* as floats in Python and
# `abs(nan - nan) > tolerance` is False, so a column of them would pass
# unconditionally. That hole has cost this project a harness before.

set -euo pipefail

TOLERANCE="${1:-1e-12}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

for source in timeline_view.c timeline_view.h; do
    if [ ! -f "$ORACLE_SRC/$source" ]; then
        echo "error: oracle source $source not found in $ORACLE_SRC" >&2
        exit 1
    fi
done

echo "=== building the oracle's timeline view (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/timeline_view_oracle" \
    tests/differential/timeline_view_oracle.c \
    "$ORACLE_SRC/timeline_view.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/timeline_view_oracle" >"$OUT_DIR/timeline_view_oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example timeline_view_dump \
    >"$OUT_DIR/timeline_view_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/timeline_view_oracle.txt" "$OUT_DIR/timeline_view_rust.txt" \
    "$TOLERANCE" <<'PY'
import sys
from collections import Counter

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

# Mechanism divergences, asserted as named pairs rather than quietly excluded. The
# C takes a bare `Timeline_View *` and documents a null one as inert; Rust takes
# `&self`/`&mut self`, so a null view is not an input it refuses -- it is one it
# cannot express. An untested rejection path is where a difference hides, so each
# side states its half and both halves are pinned here.
EXPECTED_DIVERGENCES = {
    "null_view_x_at":               ("c_returns_left", "not_expressible"),
    "null_view_seconds_at":         ("c_returns_zero", "not_expressible"),
    "null_view_is_whole":           ("c_returns_true", "not_expressible"),
    "null_view_seconds_per_pixel":  ("c_returns_zero", "not_expressible"),
    "null_view_mutators":           ("c_no_op", "not_expressible"),
}


def load(path):
    """Every line is a record: a kind, then tokens. Nothing here needs a parser.

    `divergence` rows are pulled out into their own map, because their two sides
    are deliberately different and comparing them positionally would fail by
    design."""
    rows = []
    divergences = {}
    for number, line in enumerate(open(path), start=1):
        tokens = line.split()
        if not tokens:
            continue
        if tokens[0] == "divergence":
            divergences[tokens[1]] = tokens[2]
            continue
        rows.append((number, tokens))
    return rows, divergences


def classify(token):
    """Integers and strings compare exactly; only finite exponential-form floats
    get the tolerance. Both harnesses print every float with an exponent for
    exactly this reason, so a float can never be mistaken for an integer here.

    Non-finite values never reach this function as `nan` or `inf`: both harnesses
    spell them `not_a_number`, `positive_infinity` and `negative_infinity`, which
    `float()` rejects, so they land in the exact branch. The belt-and-braces check
    below catches the day someone prints a bare one anyway -- comparing NaN with a
    tolerance passes unconditionally, which would be a silent hole rather than a
    check."""
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


oracle, oracle_divergences = load(oracle_path)
rust, rust_divergences = load(rust_path)

failures = []
if len(oracle) != len(rust):
    failures.append("line count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

compared = 0
exact = 0
worst_relative = 0.0
worst_absolute = 0.0
worst_where = None
per_kind = Counter()

for (oracle_line, oracle_tokens), (_, rust_tokens) in zip(oracle, rust):
    kind = oracle_tokens[0]
    if kind != rust_tokens[0]:
        failures.append("line %d: record kind differs: %s vs %s"
                        % (oracle_line, kind, rust_tokens[0]))
        continue
    if len(oracle_tokens) != len(rust_tokens):
        failures.append("line %d (%s): field count differs: %d vs %d"
                        % (oracle_line, kind, len(oracle_tokens), len(rust_tokens)))
        continue
    per_kind[kind] += 1
    for column, (o, r) in enumerate(zip(oracle_tokens[1:], rust_tokens[1:]), start=1):
        o_type, o_value = classify(o)
        r_type, r_value = classify(r)
        compared += 1
        if o_type == "float" and r_type == "float":
            delta = abs(o_value - r_value)
            # Relative, because x_at legitimately returns tens of millions of
            # pixels for a moment far outside a deeply zoomed window, and an
            # absolute 1e-12 there is finer than a double can represent.
            scale = max(1.0, abs(o_value), abs(r_value))
            relative = delta / scale
            if relative > worst_relative:
                worst_relative = relative
                worst_where = "%s line %d column %d (oracle %.17g, rust %.17g)" % (
                    kind, oracle_line, column, o_value, r_value)
            worst_absolute = max(worst_absolute, delta)
            if relative > tolerance:
                failures.append(
                    "line %d (%s) column %d differs by %.3g (relative %.3g): "
                    "oracle %.17g, rust %.17g  |  oracle row: %s"
                    % (oracle_line, kind, column, delta, relative, o_value, r_value,
                       " ".join(oracle_tokens)))
        else:
            exact += 1
            if o != r:
                failures.append("line %d (%s) column %d differs exactly: "
                                "oracle %s, rust %s  |  oracle row: %s"
                                % (oracle_line, kind, column, o, r,
                                   " ".join(oracle_tokens)))

for name, (want_oracle, want_rust) in sorted(EXPECTED_DIVERGENCES.items()):
    got_oracle = oracle_divergences.get(name)
    got_rust = rust_divergences.get(name)
    compared += 2
    exact += 2
    per_kind["divergence"] += 1
    if got_oracle != want_oracle:
        failures.append("divergence %s: oracle said %r, expected %r"
                        % (name, got_oracle, want_oracle))
    if got_rust != want_rust:
        failures.append("divergence %s: rust said %r, expected %r"
                        % (name, got_rust, want_rust))
for name in sorted(set(oracle_divergences) | set(rust_divergences)):
    if name not in EXPECTED_DIVERGENCES:
        failures.append("divergence %s is reported but not pinned in this script"
                        % name)

print("records:          %d" % len(oracle))
for kind in sorted(per_kind):
    print("  %-14s  %d" % (kind, per_kind[kind]))
print("values compared:  %d  (%d of them exactly)" % (compared, exact))
print("largest delta:    %.3g absolute, %.3g relative%s"
      % (worst_absolute, worst_relative,
         "" if worst_where is None else "  at " + worst_where))
print("tolerance:        %.3g relative" % tolerance)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)

print("\nPASS: the Rust timeline view matches the frozen C view within tolerance.")
PY
