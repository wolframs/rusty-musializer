#!/usr/bin/env bash
#
# Differential test: the frozen C beat tracker versus the Rust port, over a grid
# of tick rates and onset patterns plus a set of hand-built boundary cases.
#
# Why this module gets a harness. It feeds a documented route source --
# `--route parameter:beat_phase:...` is part of the CLI grammar -- so the phase is
# observable in a rendered MP4 and therefore parity-relevant rather than internal.
# And the port's strongest existing evidence was a *hand-transcribed* table:
# `matches_the_c_oracle_step_for_step` pastes eight `%.9g` phases captured from a
# scratch C build that no longer exists. That is the exact shape this project has
# a rule against -- a typo makes it wrong forever, a number copied from our own
# output makes it tautological forever, and neither can be re-derived without
# redoing the work by hand. This re-derives it from the oracle on every run, and
# takes it from 8 values to roughly 123000.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/beat_tracker.c with all output going into our own build/
# directory. It never writes to the C tree and never invokes its `nob` build.
#
# Usage: tools/differential_beat_tracker.sh [TOLERANCE]
#
# TOLERANCE is *relative*, against max(1, |oracle|, |rust|), and defaults to
# 1e-15. Relative because the input columns are echoed and one case feeds a time of
# 1e18, where an absolute tolerance is finer than a double can represent.
#
# The measured difference across every compared value is exactly **zero**, so the
# default is margin rather than room the port needs. It is four orders tighter than
# the analyzer's 1e-4 because this module calls exactly one libm function --
# `floor`, which is exact for every input and required to be -- and otherwise uses
# only comparisons and the four arithmetic operators on `double`. There is no
# platform maths here to differ in the last bits. 1e-15 is sized to absorb a
# compiler contracting `interval + (observed - interval)*weight` into an fma, which
# changes one rounding and nothing else. Pass 0 to forbid even that.
#
# The pass line prints the worst delta either way, so a run that stops being
# bit-identical is visible while it still passes.
#
# This harness found a real parity bug on its first run, which is recorded here
# because a harness that has never failed proves nothing. The C's
# `beat_tracker_update` returns false in two unrelated situations: it refused the
# input, having written nothing; or it computed a position inside [0, 1) that
# narrowed to a float of exactly 1.0, wrote that, and refused it afterwards
# (`beat_tracker.c:76-78`). `plug.c:1139-1144` keeps a local initialised to 0.0f,
# passes its address and uses whatever is in it -- so the phase the scene frame gets
# is 0.0 in the first case and 1.0 in the second, and `beat_phase` is a documented
# route source. The port returned `None` for both and collapsed them to 0.0. The
# outcome column below is three-valued for that reason, and the oracle tells the two
# apart with a sentinel initialiser rather than by inspecting internals.
#
# Everything integral is compared exactly -- the onset flag, the outcome name,
# `learned_intervals`, `has_onset`, the case names and step indices --
# because a difference there is a real bug rather than a rounding artifact.
# Non-finite values are printed as labels (`not_a_number`, `positive_infinity`,
# `negative_infinity`) by both sides rather than as `nan`/`inf`: those spellings
# *parse* as floats in Python and `abs(nan - nan) > tolerance` is False, so a
# column of them would pass unconditionally. That hole has cost this project a
# harness before, and it matters more here than usual, because feeding NaN in is
# one of the cases rather than an accident.

set -euo pipefail

TOLERANCE="${1:-1e-15}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

for source in beat_tracker.c beat_tracker.h; do
    if [ ! -f "$ORACLE_SRC/$source" ]; then
        echo "error: oracle source $source not found in $ORACLE_SRC" >&2
        exit 1
    fi
done

echo "=== building the oracle's beat tracker (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/beat_tracker_oracle" \
    tests/differential/beat_tracker_oracle.c \
    "$ORACLE_SRC/beat_tracker.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/beat_tracker_oracle" >"$OUT_DIR/beat_tracker_oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example beat_tracker_dump \
    >"$OUT_DIR/beat_tracker_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR/beat_tracker_oracle.txt" "$OUT_DIR/beat_tracker_rust.txt" \
    "$TOLERANCE" <<'PY'
import sys
from collections import Counter

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

# Mechanism divergences, asserted as named pairs rather than quietly excluded.
#
# Both are inputs neither side can hand the other: the C takes bare pointers and
# documents a null tracker or a null out-parameter as a refusal, and Rust takes
# `&mut self` and returns an enum.
#
# There was a third entry here, claiming the C's out-of-range write was a quirk the
# port deliberately did not reproduce. That claim was wrong: `plug.c:1139-1144`
# keeps a local, passes its address and uses whatever is in it, so the written 1.0
# reaches the scene frame and a documented route source with it. Writing the excuse
# down is what exposed it. The out-of-range value is now compared on every `step`
# row, which is where it belonged.
EXPECTED_DIVERGENCES = {
    "null_tracker":   ("c_returns_false", "not_expressible"),
    "null_phase_out": ("c_returns_false", "not_expressible"),
}

# What the oracle must actually *do* on the two paths Rust cannot express. Read from
# the running C rather than from its header comment, which is this project's rule;
# these three lines are what turn the divergences above from a claim into a
# measurement. A value here that stops matching means the oracle's behaviour was
# misdescribed, not that the port drifted.
EXPECTED_ORACLE_ONLY = {
    "null_tracker_returned": "0",
    "null_phase_returned": "0",
    # The out-parameter is untouched when the tracker pointer is null: the C bails
    # before writing. Seeded with 7.0 by the harness.
    "phase_untouched_by_null_tracker": 7.0,
}


def load(path):
    """Every line is a record: a kind, then tokens. Nothing here needs a parser.

    `divergence` and `oracle_only` rows are pulled into their own maps: the first
    because its two sides are deliberately different, the second because only one
    side emits it at all. Comparing either positionally would fail by design."""
    rows = []
    divergences = {}
    oracle_only = {}
    for number, line in enumerate(open(path), start=1):
        tokens = line.split()
        if not tokens:
            continue
        if tokens[0] == "divergence":
            divergences[tokens[1]] = tokens[2]
            continue
        if tokens[0] == "oracle_only":
            oracle_only[tokens[1]] = tokens[2]
            continue
        rows.append((number, tokens))
    return rows, divergences, oracle_only


def classify(token):
    """Integers and strings compare exactly; only finite exponential-form floats
    get the tolerance. Both harnesses print every float with an exponent for
    exactly this reason, so a float can never be mistaken for an integer here.

    Non-finite values never reach this function as `nan` or `inf`: both sides spell
    them `not_a_number`, `positive_infinity` and `negative_infinity`, which
    `float()` rejects, so they land in the exact branch. The check below catches
    the day someone prints a bare one anyway -- comparing NaN with a tolerance
    passes unconditionally, which is a silent hole rather than a check."""
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


oracle, oracle_divergences, oracle_only = load(oracle_path)
rust, rust_divergences, rust_only = load(rust_path)

failures = []
if len(oracle) != len(rust):
    failures.append("line count differs: oracle %d, rust %d" % (len(oracle), len(rust)))
if rust_only:
    failures.append("the Rust side emitted oracle_only rows: %s" % sorted(rust_only))

compared = 0
exact = 0
worst_relative = 0.0
worst_absolute = 0.0
worst_where = None
per_kind = Counter()
per_case = Counter()

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
    # Column 1 is the case name on every record kind. Counting cases separately is
    # what makes a whole missing case visible in the summary rather than only as a
    # line-count mismatch.
    per_case[oracle_tokens[1].rsplit("_", 2)[0] if oracle_tokens[1].startswith("grid_")
             else oracle_tokens[1]] += 1
    for column, (o, r) in enumerate(zip(oracle_tokens[1:], rust_tokens[1:]), start=1):
        o_type, o_value = classify(o)
        r_type, r_value = classify(r)
        compared += 1
        if o_type == "float" and r_type == "float":
            delta = abs(o_value - r_value)
            scale = max(1.0, abs(o_value), abs(r_value))
            relative = delta/scale
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

for name, want in sorted(EXPECTED_ORACLE_ONLY.items()):
    got = oracle_only.get(name)
    compared += 1
    per_kind["oracle_only"] += 1
    if isinstance(want, float):
        try:
            if abs(float(got) - want) > 0.0:
                failures.append("oracle_only %s: oracle wrote %s, expected exactly %r"
                                % (name, got, want))
        except (TypeError, ValueError):
            failures.append("oracle_only %s: oracle wrote %r, expected the number %r"
                            % (name, got, want))
    else:
        exact += 1
        if got != want:
            failures.append("oracle_only %s: oracle said %r, expected %r"
                            % (name, got, want))
for name in sorted(oracle_only):
    if name not in EXPECTED_ORACLE_ONLY:
        failures.append("oracle_only %s is reported but not pinned in this script"
                        % name)

print("records:          %d" % len(oracle))
for kind in sorted(per_kind):
    print("  %-14s  %d" % (kind, per_kind[kind]))
print("cases:            %d" % len(per_case))
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

print("\nPASS: the Rust beat tracker matches the frozen C tracker within tolerance.")
PY
