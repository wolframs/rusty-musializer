#!/usr/bin/env bash
#
# Offline product smoke for the external support bundle. No command here opens
# an audio device: make-fixture-wav only writes PCM, and FFmpeg only decodes it.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# The helper list is read out of the manifest rather than repeated here. A
# second hand-maintained copy is what DX8 was: this script already compiled two
# helpers the manifest omitted, so the bundle it smoke-tested was not the bundle
# a distribution would contain.
MANIFEST=crates/musializer-runtime/src/support.rs
mapfile -t HELPERS < <(sed -n 's|^\s*"\(tools/[a-z0-9_]*\.py\)",$|\1|p' "$MANIFEST")
if [ "${#HELPERS[@]}" -lt 10 ]; then
    printf '%s\n' "no Python helpers were read from $MANIFEST" >&2
    exit 1
fi
for helper in "${HELPERS[@]}"; do
    python3 -m py_compile "$helper"
done

# Offline by construction: the one test in tests/ that opens a socket is gated
# behind MUSIALIZER_LIVE_CATALOG_TEST=1 and skips here (assist audit B3).
python3 -m unittest discover -s tests -p 'test_*.py'

SUPPORT_CHECK_DIR=$(mktemp -d)
trap 'rm -rf -- "$SUPPORT_CHECK_DIR"' EXIT HUP INT TERM

FIXTURE="$SUPPORT_CHECK_DIR/fixture.wav"
BRIDGE="$SUPPORT_CHECK_DIR/analysis/analysis.bridge.tsv"

if [ -x target/debug/make-fixture-wav ]; then
    target/debug/make-fixture-wav "$FIXTURE" 2
else
    cargo run --quiet --bin make-fixture-wav -- "$FIXTURE" 2
fi

# Scene changes is the fully local Assist path: real FFmpeg decode, NumPy
# measured analysis, deterministic planning, and bridge publication.
python3 tools/external_analysis.py assist \
    "$FIXTURE" "$SUPPORT_CHECK_DIR/analysis" \
    --duration 2 --mode sections --bridge "$BRIDGE"

cargo run --quiet -p musializer-core --example analysis_bridge_check -- "$BRIDGE"

# The other authority boundaries must build their plans without starting a
# model or making a network request.
for mode in lyrics mimo all; do
    python3 tools/external_analysis.py assist \
        "$FIXTURE" "$SUPPORT_CHECK_DIR/dry-$mode" \
        --duration 2 --mode "$mode" --timeout 600 --dry-run \
        >"$SUPPORT_CHECK_DIR/$mode.json"
    grep -q '"credentials": "environment only; omitted"' \
        "$SUPPORT_CHECK_DIR/$mode.json"
done

python3 tools/musializer_doctor.py --json --require preview \
    >"$SUPPORT_CHECK_DIR/doctor.json"

# Negative control: the native parser must catch a helper artifact whose
# contract header was removed.
tail -n +2 "$BRIDGE" >"$SUPPORT_CHECK_DIR/missing-header.tsv"
if cargo run --quiet -p musializer-core --example analysis_bridge_check -- \
    "$SUPPORT_CHECK_DIR/missing-header.tsv" >/dev/null 2>&1; then
    printf '%s\n' "negative control failed: headerless bridge was accepted" >&2
    exit 1
fi

printf '%s\n' "support bundle: source discovery, local Assist, dry runs, doctor, and Rust bridge import passed"
