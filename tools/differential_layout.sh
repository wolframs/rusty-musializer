#!/usr/bin/env bash
#
# Differential test: the frozen C workspace and timeline layout policy versus the
# Rust ports, over a dense sweep of window sizes.
#
# Both modules are pure — scalars in, rectangles and flags out — so a number can
# settle them rather than a paragraph of review. They earn a harness more than
# most: their ported tests are mostly *property* assertions ("this rect is inside
# that one"), and a property survives a genuinely wrong formula. A layout drift is
# also nearly invisible in a screenshot, because everything moves together and
# still looks plausible, and these two modules decide where every panel and
# control in the workspace is placed.
#
# The oracle's own suites carry 11 (test_timeline_layout.c: 10 EXPECT_NEAR plus one
# EXPECT_EQ_SIZE) and 18 (test_workspace_layout.c: 18 EXPECT_NEAR) exact value
# expectations, against 41 and 42 property assertions. This compares 527187
# values, and none of them can be mistyped or copied from our own output.
#
# Negative control, and what it caught. Both perturbations left the ports' own
# unit tests fully green, which is the whole argument for this harness:
#
#   1. workspace_layout.rs, the mode ladder's `>=` changed to `>` -- the exact
#      off-by-one this harness exists for. 431 discrepancies, every one a
#      `tracks_mode` flag, first at a 240x383 sidebar. `cargo test -p
#      musializer-core ui::workspace_layout` still reported 9 passed.
#   2. timeline_layout.rs, `margin*2.0` changed to `margin*2.0625` -- one sixteenth
#      of a pixel of padding. 14673 discrepancies across `scale`,
#      `controls_width`, `controls.width`, `clear.x` and `clear.width`, smallest
#      relative delta 8.1e-4. `cargo test -p musializer-core ui::timeline_layout`
#      still reported 7 passed, because its tightest expectation gives `scale` a
#      1e-3 tolerance and 0.375 px spread over a 628 px row lands inside it.
#
# Both were reverted byte-for-byte and the checksums re-verified.
#
# WX-5c (2026-10-28): the sidebar split is retired, and with it the `sidebar`
# and `action_row` records and four of the six divergence pairs. The window
# stopped calling `workspace_sidebar_layout` — the 320 px sidebar became a
# 52 px rail with flyouts — and a harness that pins code nothing calls proves
# nothing. Control 1 above was a sidebar perturbation and went with it.
# `UiRect` and `TimelineBand` are still the window's and still compared with
# the C.
#
# The rail has no oracle, so it is pinned the way `settings_dump` pins the
# scenes the C cannot have: the dump prints a `--- post-legacy (no oracle) ---`
# marker and then `WorkspaceRail::layout` over a window sweep, and that half is
# diffed exactly against `tests/differential/layout_post_legacy.txt` (1388
# records: the window grid at six band heights, every rail height from 0 to
# 260, each icon-ladder rung a sixteenth and a pixel either side, and ten
# degenerate cases). Its two negative controls, run 2026-10-28:
#
#   3. RAIL_WIDTH 52 -> 52.0625 (one sixteenth of a pixel): 1215 of the 1388
#      records differ — every accepted one; the 173 refusals print sentinels.
#   4. the flyout's top moved down 1 px: 1215 of 1388 differ, the same set.
#
# Unlike the controls above, the unit tests caught these too (3 and 1 of the
# module's tests failed), because WorkspaceRail's tests pin numbers at the
# photographed windows rather than properties — which is the lesson those
# controls taught, applied. The harness still earns its place: it is the sweep,
# and the pinned file is what a deliberate change has to be argued past.
# Both were reverted byte-for-byte (md5 re-verified) and the harness passed.
#
# The oracle at ../musializer is READ-ONLY. This script compiles
# ../musializer/src/workspace_layout.c and ../musializer/src/timeline_layout.c
# with all output going into our own build/ directory. It never writes to the C
# tree and never invokes its `nob` build.
#
# Usage: tools/differential_layout.sh [TOLERANCE]
#
# TOLERANCE defaults to 1e-9 and is *relative* — delta/max(1, |oracle|, |rust|) —
# because these layouts are swept out to 3e38 and an absolute epsilon is
# meaningless there. The measured worst case across all compared values is exactly
# **zero**: every float agrees bit for bit, because both sides do the same f32
# arithmetic in the same order with no libm call anywhere. So the default is
# margin against a compiler that contracts differently, not room the port needs.
# Tighten it to 0 to assert bit-identity.
#
# Everything integral is compared exactly, and that is the important half here:
# `ok`, `tracks_mode`, `timecode_inline` and `fits` are the decisions the drawing
# code branches on. A differing flag is not a rounding artifact — it is a panel
# drawn where it should not be, or a timecode printed through a button.
#
# Non-finite columns are routed to an exact string comparison after
# normalisation, never through the tolerance. AGENTS.md records why: `nan` and
# `inf` parse as floats in Python, and `abs(nan - nan) > tolerance` is False, so
# a tolerance branch would pass those columns unconditionally.

set -euo pipefail

TOLERANCE="${1:-1e-9}"
ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

for source in workspace_layout.c timeline_layout.c; do
    if [ ! -f "$ORACLE_SRC/$source" ]; then
        echo "error: oracle source $source not found in $ORACLE_SRC" >&2
        exit 1
    fi
done

echo "=== building the oracle's layout modules (read-only, output into $OUT_DIR) ==="
cc -O2 -std=c99 -Wall -Wextra \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/layout_oracle" \
    tests/differential/layout_oracle.c \
    "$ORACLE_SRC/workspace_layout.c" \
    "$ORACLE_SRC/timeline_layout.c" \
    -lm

echo "=== running the oracle ==="
"$OUT_DIR/layout_oracle" >"$OUT_DIR/layout_oracle.txt"

echo "=== running the Rust port ==="
cargo run --quiet --release -p musializer-core --example layout_dump \
    >"$OUT_DIR/layout_rust_full.txt"

# The C-era records, then the post-legacy rail after the marker (WX-5c).
MARKER="--- post-legacy (no oracle) ---"
EXPECTED="tests/differential/layout_post_legacy.txt"
if ! grep -qxF -- "$MARKER" "$OUT_DIR/layout_rust_full.txt"; then
    echo "FAIL: the dump printed no '$MARKER' line" >&2
    exit 1
fi
awk -v m="$MARKER" '$0 == m {found=1; next} !found' \
    "$OUT_DIR/layout_rust_full.txt" >"$OUT_DIR/layout_rust.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} found' \
    "$OUT_DIR/layout_rust_full.txt" >"$OUT_DIR/layout_rust_post.txt"

echo "=== comparing the post-legacy rail (exact, against the pinned file) ==="
if [ ! -f "$EXPECTED" ]; then
    echo "FAIL: $EXPECTED is missing. Copy $OUT_DIR/layout_rust_post.txt there" \
         "and read it before you commit it." >&2
    exit 1
fi
if ! diff -u "$EXPECTED" "$OUT_DIR/layout_rust_post.txt" >"$OUT_DIR/layout_post_legacy.diff"; then
    head -40 "$OUT_DIR/layout_post_legacy.diff"
    echo
    echo "FAIL: the rail layout changed: $(grep -c '^+rail' "$OUT_DIR/layout_post_legacy.diff") of" \
         "$(grep -c '^rail' "$EXPECTED") pinned records differ ('-' is the pinned" \
         "expectation, '+' is the code). If the change is deliberate, update" \
         "$EXPECTED and say why in the commit." >&2
    exit 1
fi
echo "post-legacy rail: $(grep -c '^rail' "$EXPECTED") records match the pinned file"

echo "=== comparing ==="
python3 - "$OUT_DIR/layout_oracle.txt" "$OUT_DIR/layout_rust.txt" "$TOLERANCE" <<'PY'
import sys
from collections import Counter

oracle_path, rust_path, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])

# Field layout per record kind: (key token count, [(name, spec)]).
#   "i" -> compared exactly. Integers, enums and every boolean.
#   "f" -> compared within a relative tolerance, unless either side is
#          non-finite, in which case the normalised token must match exactly.
# The key tokens identify the case and are compared exactly as strings; they are
# indices and labels only, never floats, so a print-format difference between
# printf("%.9e") and Rust's "{:.9e}" cannot desynchronise the pairing.
RECT = [("x", "f"), ("y", "f"), ("width", "f"), ("height", "f")]


def prefixed(prefix, fields):
    return [("%s.%s" % (prefix, name), spec) for name, spec in fields]


SCHEMA = {
    "rect_table": (1, RECT),
    "rect": (2, [("a_finite", "i"), ("a_empty", "i"),
                 ("b_finite", "i"), ("b_empty", "i"),
                 ("contains_ab", "i"), ("contains_ba", "i"),
                 ("overlaps_ab", "i"), ("overlaps_ba", "i")]
                + prefixed("isect_ab", RECT) + prefixed("isect_ba", RECT)),
    "control_set": (2, [("width", "f")]),
    "timeline": (1, [("in_x", "f"), ("in_y", "f"),
                     ("in_width", "f"), ("in_height", "f"), ("in_margin", "f"),
                     ("in_set", "i"), ("in_count", "i"),
                     ("in_clear", "f"), ("in_timecode", "f"),
                     ("ok", "i"), ("scale", "f"), ("controls_width", "f")]
                    + prefixed("controls", RECT) + prefixed("clear", RECT)
                    + prefixed("timecode", RECT)
                    + [("timecode_inline", "i"), ("fits", "i")]),
}

# How many leading fields are the case's echoed *inputs*. A failure quotes them,
# so the message names the window size rather than a record number -- and a
# mismatch in one of them means the two duplicated generators have drifted, which
# is a harness bug rather than a port bug.
INPUT_FIELDS = {"timeline": 9}

# Divergences in mechanism, not in behaviour: the C takes bare pointers and a bare
# enum, so it can refuse an input the Rust signatures cannot express at all.
# Asserted as an exact pair so the difference stays tested rather than described.
EXPECTED_DIVERGENCES = {
    "null_band_out":       ("c_rejects", "not_expressible"),
    "null_control_widths": ("c_rejects", "not_expressible"),
}


def normalise(token):
    """C spells them `nan`/`-nan`/`inf`, Rust `NaN`/`inf`. Fold the spellings so a
    non-finite column can be compared exactly and meaningfully: nan still fails
    against a number, and +inf still fails against -inf."""
    lowered = token.lower()
    if lowered in ("nan", "-nan", "+nan"):
        return "nan"
    return lowered


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
        records.append((kind, key, fields, body[key_len:]))
    return records, divergences


oracle, oracle_divergences = load(oracle_path)
rust, rust_divergences = load(rust_path)

failures = []
compared = 0
integral = 0
nonfinite = 0
per_kind = Counter()
worst, worst_where = 0.0, None

if len(oracle) != len(rust):
    failures.append("record count differs: oracle %d, rust %d" % (len(oracle), len(rust)))

for name, (want_c, want_rust) in sorted(EXPECTED_DIVERGENCES.items()):
    got_c = oracle_divergences.pop(name, "<missing>")
    got_rust = rust_divergences.pop(name, "<missing>")
    if (got_c, got_rust) != (want_c, want_rust):
        failures.append("divergence %s: expected (%s, %s), got (%s, %s)"
                        % (name, want_c, want_rust, got_c, got_rust))
for leftover in sorted(set(oracle_divergences) | set(rust_divergences)):
    failures.append("unexpected divergence %r: neither side may add one silently"
                    % leftover)

for o, r in zip(oracle, rust):
    if o[1] != r[1]:
        failures.append("record key mismatch: %s vs %s" % (" ".join(o[1]), " ".join(r[1])))
        continue
    kind, key, fields, otokens = o
    rtokens = r[3]
    inputs = INPUT_FIELDS.get(kind, 0)
    label = " ".join(key)
    if inputs:
        label += " [in: %s]" % " ".join(otokens[:inputs])
    compared += len(fields)
    per_kind[kind] += len(fields)
    for (name, spec), otoken, rtoken in zip(fields, otokens, rtokens):
        if spec == "i":
            # Integers, enums and every boolean. `ok`, `timecode_inline` and
            # `fits` are the decisions the drawing code branches on, so a
            # difference is a real bug, never a rounding artifact.
            integral += 1
            if int(otoken) != int(rtoken):
                failures.append("%s: %s differs: oracle %s, rust %s"
                                % (label, name, otoken, rtoken))
            continue
        ovalue, rvalue = float(otoken), float(rtoken)
        finite = ovalue == ovalue and rvalue == rvalue \
            and abs(ovalue) != float("inf") and abs(rvalue) != float("inf")
        if not finite:
            # Never through the tolerance: abs(nan - nan) > tol is False, which
            # would pass this column unconditionally.
            nonfinite += 1
            if normalise(otoken) != normalise(rtoken):
                failures.append("%s: %s differs (non-finite): oracle %s, rust %s"
                                % (label, name, otoken, rtoken))
            continue
        scale = max(1.0, abs(ovalue), abs(rvalue))
        delta = abs(ovalue - rvalue)/scale
        if delta > worst:
            worst = delta
            worst_where = "%s: %s (oracle %s, rust %s)" % (label, name, otoken, rtoken)
        if delta > tolerance:
            failures.append("%s: %s differs by %.3g relative: oracle %s, rust %s"
                            % (label, name, delta, otoken, rtoken))

print("records compared:      %d" % len(oracle))
for kind in sorted(per_kind):
    print("  %-14s %7d records, %8d values"
          % (kind, sum(1 for rec in oracle if rec[0] == kind), per_kind[kind]))
timeline_values = per_kind["timeline"] + per_kind["control_set"]
workspace_values = per_kind["rect"] + per_kind["rect_table"]
print("workspace_layout:      %d values" % workspace_values)
print("timeline_layout:       %d values" % timeline_values)
print("values compared:       %d" % compared)
print("  integral, exact:     %d   (ok, timecode_inline, fits, ...)" % integral)
print("  non-finite, exact:   %d   (routed away from the tolerance on purpose)" % nonfinite)
print("  float, tolerance:    %d" % (compared - integral - nonfinite))
print("largest rel. delta:    %.3g%s"
      % (worst, "" if worst_where is None else "  at " + worst_where))
print("tolerance (relative):  %.3g" % tolerance)
print("divergences pinned:    %d" % len(EXPECTED_DIVERGENCES))
if nonfinite == 0:
    # The hole AGENTS.md records: nan and inf parse as floats, and
    # abs(nan - nan) > tolerance is False. If the sweep stops producing them the
    # exact branch has gone dead and nobody would notice.
    print("\nFAIL: no non-finite column was compared; the exact branch is dead")
    sys.exit(1)

if failures:
    print("\nFAIL: %d discrepancies" % len(failures))
    for line in failures[:25]:
        print("  " + line)
    if len(failures) > 25:
        print("  ... and %d more" % (len(failures) - 25))
    sys.exit(1)

print("\nPASS: the Rust workspace and timeline layouts match the frozen C layouts.")
PY
