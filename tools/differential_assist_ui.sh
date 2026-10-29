#!/usr/bin/env bash
#
# Differential test: the frozen C's Assist panel policy versus the Rust port.
#
# `assist_ui_state.c` decides everything the Assist panel does before a pixel is
# drawn -- which of six bodies shows, how tall each one is, what the lyric
# reference row costs, where the mode grid breaks to two columns, why Start is
# unavailable, and the `<stem>.lyrics.txt` rule the Python helper uses. All of it
# came across by hand, and all of it is either a number or a sentence a reviewer
# would otherwise have to trust.
#
# The stem rule earns the harness on its own: it has to match Python `pathlib`'s,
# because the helper is Python, and a disagreement means the panel says "found"
# about a file the run never opens.
#
# Since WX-5d the dump ends in a `--- post-legacy (no oracle) ---` section: the
# right column's card layout over a width x body sweep, diffed against
# tests/differential/assist_ui_post_legacy.txt. Both halves must pass.
#
# NEGATIVE CONTROL, 2026-10-29: CARD_PADDING 13 -> 14 in assist_ui_state.rs
# (every card 2 px taller) fails the post-legacy diff on every column row and
# leaves the oracle half green; restored byte for byte.
#
# The oracle at ../musializer is READ-ONLY: this reads its source and writes only
# into our own build/.
#
# Usage: tools/differential_assist_ui.sh

set -euo pipefail

ORACLE_SRC="/home/wolfram/Projects/musializer/src"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="build/differential"
mkdir -p "$OUT_DIR"

if [ ! -f "$ORACLE_SRC/assist_ui_state.c" ]; then
    echo "error: oracle source not found at $ORACLE_SRC" >&2
    exit 1
fi

echo "=== building the oracle's Assist policy (read-only) ==="
cc -O1 -std=c11 \
    -I"$ORACLE_SRC" \
    -o "$OUT_DIR/assist_ui_oracle" \
    tests/differential/assist_ui_oracle.c \
    "$ORACLE_SRC/assist_ui_state.c" \
    -lm

echo "=== running both ==="
# The oracle still prints `layout` and `timeline` rows (`ui_layout` and
# `timeline_height`, assist_ui_state.c:219-270). WX-5d took them out of the
# comparison: the window lays Assist out in the right column, and the band
# they sized no longer exists. The C driver is not edited; its rows are
# filtered here, and the Rust dump no longer prints them. What replaced them
# is the post-legacy section below.
#
# The mode copy rows (badge, workflow, boundary, empty) are filtered too: the
# copy was rewritten on 2026-08-18 (e8e2aa6) so the lyric route says what it
# does, and this on-demand harness had been red on 13 of these 16 rows since.
# They are pinned in the post-legacy file now, by name, so the filter cannot
# swallow anything else: the expected count is asserted below.
"$OUT_DIR/assist_ui_oracle" >"$OUT_DIR/assist_ui_oracle_full.txt"
grep -vE '^(layout|timeline) |^mode [0-3] (badge|workflow|boundary|empty)\|' \
    "$OUT_DIR/assist_ui_oracle_full.txt" >"$OUT_DIR/assist_ui_oracle.txt"
dropped_copy="$(grep -cE '^mode [0-3] (badge|workflow|boundary|empty)\|' "$OUT_DIR/assist_ui_oracle_full.txt")"
if [ "$dropped_copy" -ne 16 ]; then
    echo "FAIL: expected to set aside 16 mode copy rows, found $dropped_copy" >&2
    exit 1
fi
cargo run --quiet -p musializer-core --example assist_ui_dump >"$OUT_DIR/assist_ui_rust_full.txt"

MARKER="--- post-legacy (no oracle) ---"
EXPECTED="tests/differential/assist_ui_post_legacy.txt"
if ! grep -qxF -- "$MARKER" "$OUT_DIR/assist_ui_rust_full.txt"; then
    echo "FAIL: the dump printed no '$MARKER' line; the column layout is unpinned" >&2
    exit 1
fi
awk -v m="$MARKER" '$0 == m {found=1; next} !found' \
    "$OUT_DIR/assist_ui_rust_full.txt" >"$OUT_DIR/assist_ui_rust.txt"
awk -v m="$MARKER" '$0 == m {found=1; next} found' \
    "$OUT_DIR/assist_ui_rust_full.txt" >"$OUT_DIR/assist_ui_rust_post.txt"

echo "=== comparing the policy (exact, against the frozen C) ==="
# Exact, not tolerant: every value here is an integer, a pixel constant or a
# string. There is no arithmetic in this module whose last bits could differ
# between libm and Rust's intrinsics.
if ! diff -u "$OUT_DIR/assist_ui_oracle.txt" "$OUT_DIR/assist_ui_rust.txt"; then
    echo
    echo "FAIL: the Rust Assist policy diverges from the frozen C (diff above:" \
         "'-' is the oracle, '+' is Rust)" >&2
    exit 1
fi

echo "=== comparing the column layout (exact, against the pinned file) ==="
if [ ! -f "$EXPECTED" ]; then
    echo "FAIL: $EXPECTED is missing. Copy $OUT_DIR/assist_ui_rust_post.txt there" \
         "and read it before you commit it." >&2
    exit 1
fi
if ! diff -u "$EXPECTED" "$OUT_DIR/assist_ui_rust_post.txt"; then
    echo
    echo "FAIL: the Assist column layout changed ('-' is the pinned expectation," \
         "'+' is the code). If that is deliberate, update $EXPECTED and say why." >&2
    exit 1
fi

echo
echo "PASS: the Assist panel policy matches the frozen C exactly"
echo "      ($(wc -l <"$OUT_DIR/assist_ui_oracle.txt") decisions compared)"
echo "      plus $(wc -l <"$EXPECTED") pinned column-layout rows"
