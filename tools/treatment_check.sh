#!/usr/bin/env bash
#
# The directing gate: --treatment, --vocabulary, --contact-sheet, --still and
# the wrapper that runs them, checked end to end on a synthetic fixture.
#
#   tools/treatment_check.sh [OUTPUT_DIR]        # default build/treatment-check
#
# Every launch goes through tools/headless_run.sh, deliberately: that script is
# what a caller with no display is told to use (docs/MUSIC_VIDEO_CLI.md), so it
# is the thing under test as much as the flags are. Private Xvfb, unreachable
# Wayland and audio endpoints, kdialog/zenity off PATH, and --mute — which
# leaves the fixture PCM intact for the analyzer and the exported soundtrack.
#
# TREATMENT_BIN names a different binary, for running a negative control without
# replacing the build.
#
# What it asserts, and what each claim was checked against, is in
# tools/treatment_check.py.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/treatment-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${TREATMENT_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"

run() {
    tools/headless_run.sh "$@"
}

FIXTURE="$OUT/fixture.wav"
cargo run --quiet --bin make-fixture-wav -- "$FIXTURE" 12

# --- the vocabulary answers with no display at all ---------------------------
# Run straight on the binary with DISPLAY removed, not through the wrapper: the
# claim is that it needs no window, and Xvfb would supply one.
env -u DISPLAY WAYLAND_DISPLAY=musializer-treatment-check-no-compositor \
    "$MZ_BIN" --vocabulary > "$OUT/vocabulary.json"

# --- the treatments ------------------------------------------------------------
# Clawd in the middle on purpose: it is the scene that keeps light between
# frames, so it is the one where "a still from a multi-still pass" and "the same
# still on its own" could differ.
cat > "$OUT/full.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "title": "Treatment check",
  "notes": "written by tools/treatment_check.sh",
  "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 30, "quality": "balanced" },
  "seed": 2012,
  "caption_style": {
    "face": "space_grotesk", "box": "plate", "anchor": "top_center",
    "size_scale": 0.09, "text_rgba": "#ffe9c2"
  },
  "scenes": {
    "spectrum": {
      "settings": { "amplitude": 1.6 },
      "routes": [
        { "setting": "hue_swing", "source": "bass", "input": [0, 1], "output": [10, 110] }
      ]
    },
    "loom": { "routes": [ "loom.weight:rms:0:0:1:0.4:2.2:smoothstep" ] }
  },
  "segments": [
    { "at": 0, "scene": "spectrum", "note": "opening" },
    { "at": 3, "scene": "clawd", "settings": { "cats": 5, "terminal": false } },
    { "at": "0:07.5", "scene": "loom", "settings": { "weight": 1.5 } },
    { "at": 10, "scene": "spectrum", "settings": { "amplitude": 0.5 } }
  ],
  "lyrics": [
    { "at": 1, "until": 2.5, "text": "first line" },
    { "at": "0:04", "text": "second line" },
    { "at": 8, "text": "third line" }
  ],
  "events": [ { "at": 5 }, { "at": 9, "type": "custom", "value": 0.5 } ]
}
EOF

# The same plan with no `lyrics` or `events` key and a different third scene:
# applied over the saved project, it must keep that project's lines.
cat > "$OUT/look-only.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "segments": [
    { "at": 0, "scene": "pulse" },
    { "at": 6, "scene": "knot" }
  ]
}
EOF

# Transitions: an intro out of black, a burn off Clawd (the one scene that
# keeps light of its own, so its run-up is in play), a dip, and an outro. The
# same plan with cuts is the control the burn is judged against.
cat > "$OUT/moves.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 30, "quality": "balanced" },
  "seed": 2012,
  "segments": [
    { "at": 0, "scene": "clawd", "transition": { "kind": "iris", "seconds": 1.5 } },
    { "at": 5, "scene": "spectrum", "transition": { "kind": "burn", "seconds": 2 } },
    { "at": 9, "scene": "pulse", "transition": "dip" }
  ],
  "outro": { "kind": "fade", "seconds": 1.5 }
}
EOF
python3 - "$OUT/moves.treatment.json" "$OUT/cuts.treatment.json" <<'EOF'
import json, sys
plan = json.load(open(sys.argv[1]))
plan.pop("outro")
for segment in plan["segments"]:
    segment.pop("transition")
json.dump(plan, open(sys.argv[2], "w"), indent=2)
EOF
cat > "$OUT/bad-move.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "loom", "transition": "crossfade" } ]
}
EOF

# Refusals. Each is one mistake a caller would actually make.
cat > "$OUT/bad-setting.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "loom", "settings": { "wieght": 1.5 } } ]
}
EOF
cat > "$OUT/bad-range.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "loom", "settings": { "weight": 99 } } ]
}
EOF
cat > "$OUT/past-end.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "loom" }, { "at": "1:00", "scene": "atlas" } ]
}
EOF
cat > "$OUT/bad-digest.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav",
             "sha256": "0000000000000000000000000000000000000000000000000000000000000000" },
  "segments": [ { "at": 0, "scene": "loom" } ]
}
EOF

# --- one run: apply, save, sheet, two stills -----------------------------------
# Both stills are inside the Clawd section, and the first is before the second's
# three-second run-up begins. That is the only arrangement in which light left
# in the persistence buffer by the first capture could reach the second, so it
# is the only one that tests the claim below.
run --treatment "$OUT/full.treatment.json" \
    --save-project "$OUT/full.musi" \
    --contact-sheet "$OUT/sheet-segments.png" --sheet-at segments \
    --still 3.6 "$OUT/still-a.png" --still 0:07 "$OUT/still-b.png" \
    > "$OUT/full.log" 2>&1

# The default spread, restricted to a window.
run --treatment "$OUT/full.treatment.json" \
    --render-window 3 4 --contact-sheet "$OUT/sheet-window.png" --sheet-at even:6 \
    > "$OUT/window.log" 2>&1

# The second still on its own, in a fresh process: must be the same bytes.
run --treatment "$OUT/full.treatment.json" --still 7 "$OUT/still-b-alone.png" \
    > "$OUT/alone.log" 2>&1

# The frame the export encodes at that moment, for the "a still is the video
# frame" cross-check. x264 pinned: this compares pictures, and a comparison that
# changed encoder between machines would be measuring the encoder.
run --treatment "$OUT/full.treatment.json" --render-window 7 0.2 \
    --render "$OUT/at-seven.mp4" > "$OUT/export.log" 2>&1
ffmpeg -v error -y -i "$OUT/at-seven.mp4" -frames:v 1 "$OUT/at-seven.png"
# A neighbouring moment as the control the PSNR is judged against.
run --treatment "$OUT/full.treatment.json" --still 6.5 "$OUT/still-neighbour.png" \
    > "$OUT/neighbour.log" 2>&1

# --- transitions ----------------------------------------------------------------
# A still inside the burn in a pass with others, the same still alone, the frame
# an export of that moment encodes, and the same moment from the plan with cuts.
run --treatment "$OUT/moves.treatment.json" --save-project "$OUT/moves.musi" \
    --still 0.3 "$OUT/move-intro.png" --still 5.4 "$OUT/move-burn-early.png" \
    --still 5.8 "$OUT/move-burn.png" \
    --still 11.7 "$OUT/move-outro.png" > "$OUT/moves.log" 2>&1
run --treatment "$OUT/moves.treatment.json" --still 5.8 "$OUT/move-burn-alone.png" \
    > "$OUT/moves-alone.log" 2>&1
run --treatment "$OUT/moves.treatment.json" --render-window 5.8 0.2 \
    --render "$OUT/move-burn.mp4" > "$OUT/moves-export.log" 2>&1
ffmpeg -v error -y -i "$OUT/move-burn.mp4" -frames:v 1 "$OUT/move-burn-export.png"
run --treatment "$OUT/cuts.treatment.json" --still 0.3 "$OUT/cut-intro.png" \
    --still 5.8 "$OUT/cut-burn.png" --still 11.7 "$OUT/cut-outro.png" \
    > "$OUT/cuts.log" 2>&1
run --project "$OUT/moves.musi" --save-project "$OUT/moves-again.musi" \
    --still 5.8 "$OUT/move-burn-reopened.png" > "$OUT/moves-reopen.log" 2>&1

# The preview draws them too, and it is a different framebuffer: the window is
# multisampled and the scene sits in a panel, not at the origin. Slices off
# Clawd's cream ground onto Spectrum's dark one, held long enough to photograph.
cat > "$OUT/preview-move.treatment.json" <<'EOF'
{
  "schema": "musializer.treatment/v1",
  "audio": { "path": "fixture.wav" },
  "segments": [
    { "at": 0, "scene": "clawd" },
    { "at": 0.6, "scene": "spectrum", "transition": { "kind": "slices", "seconds": 8 } }
  ]
}
EOF
run --treatment "$OUT/preview-move.treatment.json" --probe-frames 120 \
    --probe-shot "$OUT/preview-move.png" > "$OUT/preview-move.log" 2>&1

# --- the saved project is an ordinary project ----------------------------------
run --project "$OUT/full.musi" --save-project "$OUT/full-again.musi" \
    > "$OUT/reopen.log" 2>&1
# And a look-only treatment over it keeps its lyrics and events.
run --project "$OUT/full.musi" --treatment "$OUT/look-only.treatment.json" \
    --save-project "$OUT/relooked.musi" > "$OUT/relook.log" 2>&1
# A treatment with no audio block directs the track the command line opened.
run "$FIXTURE" --treatment "$OUT/look-only.treatment.json" \
    --save-project "$OUT/positional.musi" > "$OUT/positional.log" 2>&1

# --- refusals: exit 1, name the place, write nothing ---------------------------
for name in bad-setting bad-range past-end bad-digest bad-move; do
    status=0
    run --treatment "$OUT/$name.treatment.json" \
        --save-project "$OUT/$name.musi" --contact-sheet "$OUT/$name.png" \
        > "$OUT/$name.log" 2>&1 || status=$?
    echo "$status" > "$OUT/$name.status"
done
# A still past the end of the track, with a valid treatment.
status=0
run --treatment "$OUT/full.treatment.json" --still 90 "$OUT/late.png" \
    > "$OUT/late.log" 2>&1 || status=$?
echo "$status" > "$OUT/late.status"
# Two tracks: the treatment names one, the command line already opened another.
cp "$FIXTURE" "$OUT/other.wav"
status=0
run "$OUT/other.wav" --treatment "$OUT/full.treatment.json" \
    --save-project "$OUT/two-tracks.musi" > "$OUT/two-tracks.log" 2>&1 || status=$?
echo "$status" > "$OUT/two-tracks.status"

python3 tools/treatment_check.py "$OUT"
echo "PASS: vocabulary, treatment apply/save/reopen, contact sheets, stills and refusals ($OUT)"
