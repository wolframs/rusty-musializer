#!/usr/bin/env bash
# Emit the CX-4 Surprise keepability session as HX protocol files.
#
#   tools/cx4_surprise_session.sh TRACK_A TRACK_B [OUT_DIR]
#
# Writes, per track: <OUT_DIR>/cx4-surprise-{a,b}.protocol.json (the blind
# session the application runs) and beside it a .key.json — the unblinding
# map from item id to sampler (pre-CX-4 "current" vs revised) and seed.
# DO NOT open the key before answering; the blind is the point.
#
# The audio is referenced by absolute path + sha256, never copied. Protocol
# files are session artifacts and belong under a gitignored directory
# (default build/protocols), not in the repository.
#
# To run a session afterwards:
#   cargo run -- --protocol <OUT_DIR>/cx4-surprise-a.protocol.json
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 TRACK_A TRACK_B [OUT_DIR]" >&2
    exit 2
fi
TRACK_A="$1"
TRACK_B="$2"
OUT_DIR="${3:-build/protocols}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
mkdir -p "$OUT_DIR"

# The freeze check first: if the frozen pre-CX-4 sampler no longer reproduces
# the old gate pin, every "current" label below would be a lie.
cargo run -q --example cx4_surprise_protocol -p musializer-core -- --self-check

emit() {
    local track_path="$1" track_index="$2" letter="$3"
    if [ ! -f "$track_path" ]; then
        echo "FAIL: $track_path does not exist" >&2
        exit 1
    fi
    local sha duration title
    sha="$(sha256sum "$track_path" | cut -d' ' -f1)"
    duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$track_path")"
    title="CX-4 Surprise keepability - $(basename "$track_path")"
    cargo run -q --example cx4_surprise_protocol -p musializer-core -- \
        --audio "$track_path" --sha256 "$sha" --duration "$duration" \
        --track "$track_index" --title "$title" \
        > "$OUT_DIR/cx4-surprise-$letter.protocol.json" \
        2> "$OUT_DIR/cx4-surprise-$letter.key.json"
    echo "wrote $OUT_DIR/cx4-surprise-$letter.protocol.json ($(basename "$track_path"))"
}

emit "$TRACK_A" 0 a
emit "$TRACK_B" 1 b

echo
echo "Session: cargo run -- --protocol $OUT_DIR/cx4-surprise-a.protocol.json"
echo "   then: cargo run -- --protocol $OUT_DIR/cx4-surprise-b.protocol.json"
echo "Answers append beside each protocol as *.answers.jsonl."
echo "Open the .key.json files only after both sessions."
