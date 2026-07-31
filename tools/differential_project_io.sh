#!/usr/bin/env bash
#
# Differential test: the `.musi` codec, driven in *both* directions against the
# frozen C.
#
# This is the parity gate's central requirement, and the plan states it as a
# single sentence: "A `.musi` written by the frozen C opens here, and one written
# here opens in the frozen C, with no field lost. Round-trip both directions."
#
# One direction is not enough. A field that is *parsed and then dropped on
# re-write* survives "C writes, Rust reads" — the read sees it — and survives
# "Rust writes, C reads" — what was never written is never missed. It dies only in
# the composition, so the composition is what this script holds:
#
#   1. C writes    -> Rust reads
#   2. Rust writes -> C reads
#   3. C writes    -> Rust reads -> Rust re-writes -> C reads      (the real test)
#   4. Rust writes -> C reads    -> C re-writes    -> Rust reads   (its mirror)
#
# plus a fifth check that the two independently written fixtures agree in the
# first place, because a comparison of two dumps of the *same* mistake proves
# nothing.
#
# # Values, not bytes
#
# C writes JSON numbers with `%.17g`; Rust writes its shortest round-tripping
# form, so the same double is spelled `0.23449999999999999` in one file and
# `0.2345` in the other and the two files differ in length. That difference is
# **settled as not a parity bug** (see `io::write_f64`'s doc comment): nothing in
# the oracle ever hashes or byte-compares a `.musi` — every `sha256` in the C
# project is over an *asset* — so the C reads `0.2345`, gets the same double it
# wrote, and has no way to notice. Export determinism is where bit-identity is
# required, and an MP4 is not a `.musi`.
#
# So the comparator below parses both dumps and compares **values**: strings,
# enums, booleans and integers exactly, floats within a tolerance. It does not
# round the C's output to fewer digits, which would hide a real loss rather than
# tolerate a spelling. Both dump programs print numbers at 17 significant digits,
# which is lossless for a double, so a value that survives compares at delta
# exactly zero.
#
# The loss worth hunting is not in the doubles: `%.17g` and shortest-round-trip
# are both lossless for a `f64`. It is in the fields the model stores as `f32`,
# where the path is `f32 -> f64 -> text -> f64 -> f32`. The fixture loads those
# fields deliberately — `0.1f`, `1.0f/3.0f`, `+/-FLT_MAX`, `-0.0f`, `65504.0f` —
# and the mapping `output_min` on the second scene is exactly `(double)0.4f`.
#
# The oracle at ../musializer is READ-ONLY. This script compiles its sources with
# all output going into our own build/ directory. It never writes to the C tree
# and never invokes its `nob` build.
#
# Usage: tools/differential_project_io.sh [TOLERANCE]
#
# TOLERANCE is relative and defaults to 1e-15, a few ULP of a double. It is not
# zero only because glibc's `%.17g` and Rust's `{:.16e}` need not break an exact
# rounding tie the same way; every observed delta so far is 0.

set -euo pipefail

TOLERANCE="${1:-1e-15}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential/project_io"
mkdir -p "$OUT_DIR"

for source in project_io.c project.c sha256.c event_timeline.c lyrics.c \
              scene_routes.c scene_settings.c scene_event_merge.c semantic_lane.c; do
    if [ ! -f "$ORACLE_SRC/$source" ]; then
        echo "error: oracle source not found at $ORACLE_SRC/$source" >&2
        exit 1
    fi
done

ORACLE="$OUT_DIR/project_io_oracle"

echo "=== building the oracle's codec (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$ORACLE" \
    tests/differential/project_io_oracle.c \
    "$ORACLE_SRC/project_io.c" \
    "$ORACLE_SRC/project.c" \
    "$ORACLE_SRC/sha256.c" \
    "$ORACLE_SRC/event_timeline.c" \
    "$ORACLE_SRC/lyrics.c" \
    "$ORACLE_SRC/scene_routes.c" \
    "$ORACLE_SRC/scene_settings.c" \
    "$ORACLE_SRC/scene_event_merge.c" \
    "$ORACLE_SRC/semantic_lane.c" \
    -lm

echo "=== building the Rust side ==="
cargo build --quiet --release -p musializer-core --example project_io_dump

# `cargo run` would re-check the manifest on every one of the six invocations and
# interleave its own output; the built binary is invoked directly instead.
RUST="target/release/examples/project_io_dump"
if [ ! -x "$RUST" ]; then
    echo "error: expected the built example at $RUST" >&2
    exit 1
fi

# Each step names the direction it is exercising, so a nonzero exit from either
# program says which half of which round trip refused the other's output. The
# error type and the field are the finding in that case, and both programs name
# them on stderr.
step() {
    local what="$1"
    shift
    if ! "$@"; then
        echo >&2
        echo "FAIL: $what" >&2
        echo "  The command above exited nonzero. If it was a parse, the error it" >&2
        echo "  named on stderr *is* the finding: one codec refused the other's" >&2
        echo "  output. Report the error type and the field; do not loosen either" >&2
        echo "  codec to make this pass." >&2
        exit 1
    fi
}

echo "=== step 1: the C writes its fixture ==="
step "the C could not build or write its fixture" \
    "$ORACLE" build "$OUT_DIR/c.musi" >"$OUT_DIR/c_wrote.txt"

echo "=== step 2: Rust writes its fixture ==="
step "Rust could not build or write its fixture" \
    "$RUST" build "$OUT_DIR/rust.musi" >"$OUT_DIR/rust_wrote.txt"

echo "=== step 3: Rust reads the C's file ==="
step "Rust refused a .musi written by the frozen C" \
    "$RUST" read "$OUT_DIR/c.musi" >"$OUT_DIR/rust_read_c.txt"

echo "=== step 4: the C reads Rust's file ==="
step "the frozen C refused a .musi written by Rust" \
    "$ORACLE" read "$OUT_DIR/rust.musi" >"$OUT_DIR/c_read_rust.txt"

echo "=== step 5: Rust re-writes what it read from the C ==="
step "Rust could not re-write the project it parsed from the C's file" \
    "$RUST" rewrite "$OUT_DIR/c.musi" "$OUT_DIR/rust_from_c.musi" \
    >"$OUT_DIR/rust_rewrote_c.txt"

echo "=== step 6: the C reads Rust's re-write (the composition) ==="
step "the frozen C refused Rust's re-write of the C's own project" \
    "$ORACLE" read "$OUT_DIR/rust_from_c.musi" >"$OUT_DIR/c_read_rust_from_c.txt"

echo "=== step 7: the C re-writes what it read from Rust ==="
step "the C could not re-write the project it parsed from Rust's file" \
    "$ORACLE" rewrite "$OUT_DIR/rust.musi" "$OUT_DIR/c_from_rust.musi" \
    >"$OUT_DIR/c_rewrote_rust.txt"

echo "=== step 8: Rust reads the C's re-write (the mirror composition) ==="
step "Rust refused the C's re-write of Rust's own project" \
    "$RUST" read "$OUT_DIR/c_from_rust.musi" >"$OUT_DIR/rust_read_c_from_rust.txt"

echo "=== comparing ==="
python3 - "$OUT_DIR" "$TOLERANCE" <<'PY'
import os
import sys

out_dir, tolerance = sys.argv[1], float(sys.argv[2])


def load(name):
    """One dump, as an ordered list of (key, raw value).

    The dump is one value per line, `key value`, with strings bracketed and
    percent-escaped so no value can contain the separator or a newline. Order is
    kept because it is part of the contract: a reordered dump means a reordered
    array, which is a difference and not a formatting choice.
    """
    path = os.path.join(out_dir, name)
    rows = []
    for number, line in enumerate(open(path, encoding="utf-8"), start=1):
        line = line.rstrip("\n")
        if not line:
            continue
        key, separator, value = line.partition(" ")
        if not separator:
            sys.exit("%s:%d: no value on the line: %r" % (name, number, line))
        rows.append((key, value))
    if not rows:
        sys.exit("%s is empty; the dump programs always print a full project" % name)
    return rows


def as_float(text):
    """The value as a float, or None if it is not a number at all.

    A bracketed string is never treated as a number, which is what keeps a title
    that happens to spell `nan` out of the numeric path.
    """
    if text.startswith("["):
        return None
    try:
        return float(text)
    except ValueError:
        return None


class Comparison:
    def __init__(self, label, left_name, right_name):
        self.label = label
        self.left_name = left_name
        self.right_name = right_name
        self.left = load(left_name)
        self.right = load(right_name)
        self.failures = []
        self.compared = 0
        self.numeric = 0
        self.worst_absolute = 0.0
        self.worst_relative = 0.0
        self.worst_where = None
        self.run()

    def fail(self, text):
        self.failures.append(text)

    def run(self):
        left_keys = [key for key, _ in self.left]
        right_keys = [key for key, _ in self.right]
        if left_keys != right_keys:
            missing = [k for k in left_keys if k not in set(right_keys)]
            extra = [k for k in right_keys if k not in set(left_keys)]
            self.fail("the key sequences differ: %d lines vs %d lines"
                      % (len(left_keys), len(right_keys)))
            for key in missing[:10]:
                self.fail("  field lost in %s: %s" % (self.right_name, key))
            for key in extra[:10]:
                self.fail("  field only in %s: %s" % (self.right_name, key))
            if not missing and not extra:
                for index, (a, b) in enumerate(zip(left_keys, right_keys)):
                    if a != b:
                        self.fail("  first reordering at line %d: %s vs %s"
                                  % (index + 1, a, b))
                        break
            return

        for (key, left_value), (_, right_value) in zip(self.left, self.right):
            self.compared += 1
            left_number = as_float(left_value)
            right_number = as_float(right_value)

            # A `.musi` cannot hold a non-finite number: both codecs reject one at
            # parse time and again at validation. So a nan or an inf reaching a dump
            # is a failure in its own right -- and it has to be checked explicitly,
            # because `abs(nan - nan) > tolerance` is False and `abs(x - nan)` is
            # nan, so both would otherwise pass unconditionally. (That hole was
            # found in an earlier harness here; see AGENTS.md.)
            nonfinite = False
            for name, number, raw in ((self.left_name, left_number, left_value),
                                      (self.right_name, right_number, right_value)):
                if number is not None and (number != number or number in (
                        float("inf"), float("-inf"))):
                    self.fail("%s: %s is %s, which no valid project can hold"
                              " (the other side has %s)"
                              % (name, key, raw,
                                 right_value if name == self.left_name else left_value))
                    nonfinite = True
            if nonfinite:
                # Comparing against a non-finite is meaningless, and letting it into
                # the delta tracking would silently poison the reported maximum.
                continue

            if left_value == right_value:
                # Identical spellings: true for strings, enums, booleans, integers
                # and -- because both sides print 17 significant digits -- for
                # every float that survived. Still measured below as a zero delta.
                if left_number is not None:
                    self.numeric += 1
                    self.measure(key, 0.0, 0.0, left_number, right_number)
                continue

            if left_number is None or right_number is None:
                self.fail("%s differs: %s has %s, %s has %s"
                          % (key, self.left_name, left_value, self.right_name, right_value))
                continue

            self.numeric += 1
            absolute = abs(left_number - right_number)
            scale = max(1.0, abs(left_number), abs(right_number))
            relative = absolute / scale
            self.measure(key, absolute, relative, left_number, right_number)
            if relative > tolerance:
                self.fail("%s differs by %.3g (relative %.3g): %s has %.17g, %s has %.17g"
                          % (key, absolute, relative, self.left_name, left_number,
                             self.right_name, right_number))

    def measure(self, key, absolute, relative, left_number, right_number):
        if absolute > self.worst_absolute or self.worst_where is None:
            self.worst_absolute = absolute
            self.worst_relative = relative
            self.worst_where = "%s (%.17g vs %.17g)" % (key, left_number, right_number)

    def report(self):
        print()
        print("--- %s" % self.label)
        print("    %-34s %s" % ("wrote the reference dump:", self.left_name))
        print("    %-34s %s" % ("compared against:", self.right_name))
        print("    %-34s %d" % ("values compared:", self.compared))
        print("    %-34s %d" % ("of which numeric:", self.numeric))
        print("    %-34s %.3g" % ("largest absolute float delta:", self.worst_absolute))
        print("    %-34s %.3g" % ("largest relative float delta:", self.worst_relative))
        if self.worst_absolute > 0.0:
            print("    %-34s %s" % ("at:", self.worst_where))
        if self.failures:
            print("    FAIL: %d discrepancies" % len(self.failures))
            for line in self.failures[:25]:
                print("      " + line)
            if len(self.failures) > 25:
                print("      ... and %d more" % (len(self.failures) - 25))
        else:
            print("    PASS")
        return not self.failures


comparisons = [
    # The premise: two fixtures built independently, in two languages, describing
    # the same project. Without this, every comparison below could be two dumps of
    # the same transcription mistake.
    Comparison("0. the two independently built fixtures agree",
               "c_wrote.txt", "rust_wrote.txt"),
    Comparison("1. C writes -> Rust reads",
               "c_wrote.txt", "rust_read_c.txt"),
    Comparison("2. Rust writes -> C reads",
               "rust_wrote.txt", "c_read_rust.txt"),
    Comparison("3. C writes -> Rust reads -> Rust re-writes -> C reads",
               "c_wrote.txt", "c_read_rust_from_c.txt"),
    Comparison("4. Rust writes -> C reads -> C re-writes -> Rust reads",
               "rust_wrote.txt", "rust_read_c_from_rust.txt"),
]

ok = True
for comparison in comparisons:
    ok = comparison.report() and ok

sizes = {}
for name in ("c.musi", "rust.musi", "rust_from_c.musi", "c_from_rust.musi"):
    sizes[name] = os.path.getsize(os.path.join(out_dir, name))

print()
print("--- the files themselves")
for name, size in sizes.items():
    print("    %-20s %6d bytes" % (name, size))
print("    The C's two files and Rust's two files are each expected to be")
print("    byte-identical to their own side, and the two sides' files are")
print("    expected to differ in length: %d vs %d bytes is the `%%.17g` versus"
      % (sizes["c.musi"], sizes["rust.musi"]))
print("    shortest-round-trip spelling, settled as not a parity bug.")


def same_bytes(left, right):
    with open(os.path.join(out_dir, left), "rb") as handle:
        a = handle.read()
    with open(os.path.join(out_dir, right), "rb") as handle:
        b = handle.read()
    return a == b


# Each codec re-writing a project it parsed from the *other* must reproduce its
# own original bytes. This is not the parity requirement -- that is the value
# comparison above -- but it is free evidence that neither codec's writer depends
# on anything its own parser did not recover.
for label, left, right in (("Rust", "rust_from_c.musi", "rust.musi"),
                           ("the C", "c_from_rust.musi", "c.musi")):
    if same_bytes(left, right):
        print("    %s re-wrote the other side's project into its own original bytes."
              % label)
    else:
        print("    FAIL: %s re-wrote %s and did not reproduce %s." % (label, left, right))
        ok = False

print()
if not ok:
    print("FAIL: the .musi codec does not round-trip against the frozen C.")
    sys.exit(1)

total = sum(comparison.compared for comparison in comparisons)
worst = max(comparison.worst_absolute for comparison in comparisons)
print("PASS: %d values compared across 5 comparisons, largest float delta %.3g."
      % (total, worst))
print("      A .musi written by the frozen C opens here and one written here opens")
print("      in the frozen C, and every field survives all four steps of both")
print("      round trips.")
PY
