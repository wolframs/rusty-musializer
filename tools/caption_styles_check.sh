#!/usr/bin/env bash
#
# MV2's gate: named caption styles and the title lane, end to end on a
# synthetic fixture, through tools/headless_run.sh like every directing check.
#
#   tools/caption_styles_check.sh [OUTPUT_DIR]   # default build/caption-styles-check
#
# tools/treatment_check.sh runs it last. TREATMENT_BIN (or CAPTION_STYLES_BIN)
# names a different binary, for a negative control without replacing the build.
#
# What it asserts, and against what, is in tools/caption_styles_check.py: the
# saved project against the treatment text, where a caption landed by its
# colour, a title card against the same moment with no card, and the card's
# words as laid out against the words the treatment gave it.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/caption-styles-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${CAPTION_STYLES_BIN:-${TREATMENT_BIN:-$(realpath target/debug/musializer)}}"
export MZ_STATE_DIR="$OUT/state"

run() {
    tools/headless_run.sh "$@"
}

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 12 >/dev/null

# The base caption is magenta with no backing, so where a line landed is a
# count of magenta pixels; `shout` is yellow and states no anchor, so a shouted
# line in the `top` section is yellow *at the top* only if styles stack. Clawd
# owns the floor in the middle section, Cadence draws its own line in the last.
# The first card takes the project's artist, the second states its own.
cat > "$OUT/styled.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "title": "Caption styles check",
  "artist": "Gate Artist",
  "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 30, "quality": "balanced" },
  "seed": 2012,
  "caption_style": { "box": "none", "size_scale": 0.07, "text_rgba": "#ff2bd6" },
  "caption_styles": {
    "top":   { "anchor": "top_center" },
    "shout": { "text_rgba": "#ffd800", "size_scale": 0.09 },
    "card":  { "box": "plate" }
  },
  "titles": [
    { "at": 0, "until": 3.6, "title": "Gate Title", "handle": "@gatecat" },
    { "at": 8.2, "until": 11.5, "title": "Over Cadence", "artist": "Second Act", "style": "card" }
  ],
  "segments": [
    { "at": 0, "scene": "spectrum" },
    { "at": 4, "scene": "clawd", "settings": { "cats": 3 }, "caption_style": "top" },
    { "at": 8, "scene": "cadence" }
  ],
  "lyrics": [
    { "at": 1.0, "until": 3.0, "text": "base line here" },
    { "at": 5.0, "until": 6.4, "text": "top line here" },
    { "at": 6.6, "until": 7.8, "text": "loud line here", "style": "shout" }
  ]
}
EOF
# The control: the same video with no named style anywhere and no card.
python3 - "$OUT/styled.treatment.json" "$OUT/plain.treatment.json" <<'EOF'
import json, sys
plan = json.load(open(sys.argv[1]))
plan.pop("caption_styles")
plan["titles"] = []
for segment in plan["segments"]:
    segment.pop("caption_style", None)
for line in plan["lyrics"]:
    line.pop("style", None)
json.dump(plan, open(sys.argv[2], "w"), indent=2)
EOF
# A card nobody gave an artist: no `artist` anywhere.
python3 - "$OUT/styled.treatment.json" "$OUT/bare.treatment.json" <<'EOF'
import json, sys
plan = json.load(open(sys.argv[1]))
plan.pop("artist")
plan["titles"] = [{"at": 0, "until": 3.6, "title": "Gate Title"}]
json.dump(plan, open(sys.argv[2], "w"), indent=2)
EOF
cat > "$OUT/unknown-style.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "caption_styles": { "top": { "anchor": "top_center" } },
  "segments": [ { "at": 0, "scene": "clawd", "caption_style": "tpo" } ]
}
EOF

STILLS=(--still 2.0 "@-title.png" --still 5.7 "@-top.png" --still 7.2 "@-loud.png"
        --still 9.5 "@-cadence.png")
stills() { # stills NAME -> the still arguments with NAME's prefix
    for argument in "${STILLS[@]}"; do
        printf '%s\n' "${argument//@/$OUT/$1}"
    done
}
mapfile -t styled_stills < <(stills styled)
mapfile -t plain_stills < <(stills plain)

run --treatment "$OUT/styled.treatment.json" --save-project "$OUT/styled.musi" \
    "${styled_stills[@]}" > "$OUT/styled.log" 2>&1
# The first card's still alone, in a fresh process: the same bytes as from the
# four-still pass, and the report's last card is then this one.
run --treatment "$OUT/styled.treatment.json" --still 2.0 "$OUT/title-alone.png" \
    > "$OUT/title-alone.log" 2>&1
run --treatment "$OUT/plain.treatment.json" --save-project "$OUT/plain.musi" \
    "${plain_stills[@]}" > "$OUT/plain.log" 2>&1
# Reopen the saved project and save it again: nothing about the styles moves.
run --project "$OUT/styled.musi" --save-project "$OUT/resaved.musi" \
    > "$OUT/resaved.log" 2>&1
run --treatment "$OUT/bare.treatment.json" --still 2.0 "$OUT/bare-title.png" \
    > "$OUT/bare.log" 2>&1
# A look applied over the saved project that says nothing about lyrics, titles
# or styles keeps all three; one whose table drops a name a kept line uses is
# refused, because the project could not be opened again.
cat > "$OUT/look-only.treatment.json" <<'EOF2'
{ "schema": "musializer.treatment/v1", "segments": [ { "at": 0, "scene": "pulse" } ] }
EOF2
cat > "$OUT/drops-shout.treatment.json" <<'EOF2'
{ "schema": "musializer.treatment/v1", "caption_styles": { "top": {} },
  "segments": [ { "at": 0, "scene": "pulse" } ] }
EOF2
run --project "$OUT/styled.musi" --treatment "$OUT/look-only.treatment.json" \
    --save-project "$OUT/kept.musi" > "$OUT/kept.log" 2>&1
status=0
run --project "$OUT/styled.musi" --treatment "$OUT/drops-shout.treatment.json" \
    --save-project "$OUT/dropped.musi" > "$OUT/dropped.log" 2>&1 || status=$?
echo "$status" > "$OUT/dropped.status"
status=0
run --treatment "$OUT/unknown-style.treatment.json" --save-project "$OUT/unknown.musi" \
    --still 1 "$OUT/unknown.png" > "$OUT/unknown.log" 2>&1 || status=$?
echo "$status" > "$OUT/unknown.status"
env -u DISPLAY WAYLAND_DISPLAY=musializer-caption-styles-check-no-compositor \
    "$MZ_BIN" --vocabulary > "$OUT/vocabulary.json"

python3 tools/caption_styles_check.py "$OUT"
echo "PASS: named caption styles, section and line styles, title cards ($OUT)"
