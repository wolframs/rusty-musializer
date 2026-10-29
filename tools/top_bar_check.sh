#!/usr/bin/env bash
#
# The top bar gate (WX-5c, docs/WX5C_DESIGN.md §4 and §8): the title row, its
# tabs and its three actions, checked headless.
#
#   tools/top_bar_check.sh [OUTPUT_DIR]        # default build/top-bar-check
#
# Every launch goes through tools/headless_run.sh: private Xvfb, unreachable
# Wayland and audio endpoints, kdialog/zenity off PATH, a private config
# directory, --mute. Destinations are given with `--ui-probe save-to=`, so no
# picker is ever asked for. The fixture is synthetic: a few seconds of
# make-fixture-wav audio and a three-section treatment that names an artist.
#
# Rects come from the `top bar:` report line, in logical units, and every run
# here is at --ui-scale 100 so they are pixels too. Nothing presses a
# coordinate measured once.
#
# Checks, each with its control:
#   1. the line names the title, the artist, the info and the save state;
#      a treatment without an artist draws none (the artist is never guessed);
#   2. each tab's press opens what it names (`panel:` and the starred tab);
#      a press between the tabs and Still changes nothing;
#   3. Still and Contact sheet write their files through the export path;
#      a destination in a folder that does not exist writes nothing;
#   4. the save word carries ink where the line says it is; an empty stretch
#      of the same bar carries none, and in fullscreen the bar is not drawn;
#   5. a treatment's artist saves a v2 .musi; without one the file is v1.
#
# TOP_BAR_BIN names a different binary, for a negative control without
# replacing the build.
#
# NEGATIVE CONTROLS, run 2026-10-29 on the GPU path:
#   - the tab press calling nothing instead of `activate_mode_tab`: 11
#     failures (each of the four tabs unselected and Direct still starred;
#     three panels left at none), with the gap press still green;
#   - the save word drawn in the bar's own surface colour: the ink check
#     reads 0 against 81 and fails, the empty-stretch control still 0.
# The same run on the CPU path (MZ_GL_LAUNCH="") reads the same 81 ink pixels
# and 0.89/0.00 fill shares, so the classifier is not calibrated to one
# rasterizer.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/top-bar-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${TOP_BAR_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"
# DROP AT MERGE (wx5-topbar): gives the bar its 46 px on the top bar's own
# branch. Rail's frame gives it 46 px natively, and the variable is then read
# by nothing.
export MZ_WX5C_TOP_BAR=46

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 6 >/dev/null

treatment() { # treatment FILE [artist]
    local artist=""
    [ -n "${2:-}" ] && artist="\"artist\": \"$2\","
    cat >"$1" <<EOF
{ "schema": "musializer.treatment/v1", "title": "Top Bar Fixture", $artist
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "spectrum" }, { "at": 2, "scene": "loom" },
                { "at": 4, "scene": "cadence" } ] }
EOF
}
treatment "$OUT/show.treatment.json" "Chaos Cat Familiar"
treatment "$OUT/plain.treatment.json"

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
line_of() { # line_of KEY LOG
    grep -E "^$1: " "$2" | tail -n 1 || true
}
run() { # run NAME TREATMENT FRAMES PROBE [extra args]
    local name="$1" show="$2" frames="$3" probe="$4"
    shift 4
    tools/headless_run.sh --treatment "$show" --ui-scale 100 \
        --ui-probe "size=1280x720,$probe" --probe-frames "$frames" \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}
# The centre of a rect named on the `top bar:` line: `centre LINE NAME`.
centre() {
    python3 - "$1" "$2" <<'PY'
import re, sys
line, name = sys.argv[1], sys.argv[2]
m = re.search(r"(?:^|[ ;,=])" + re.escape(name) + r"\*?@(\d+),(\d+),(\d+),(\d+)", line)
if not m:
    sys.exit(f"no rect for {name}")
x, y, w, h = map(int, m.groups())
print(f"{x + w // 2}x{y + h // 2}")
PY
}

# 1. The line, with and without an artist.
run base "$OUT/show.treatment.json" 40 "time=3"
base="$(line_of 'top bar' "$OUT/base.log")"
printf '%s\n' "${base:0:260}"
for want in 'title="Top Bar Fixture"' 'artist="Chaos Cat Familiar"' \
    'info="0:06 · 1080p 16:9 · 30 fps"' 'save=no-file' 'tabs=direct*@' 'render@'; do
    case "$base" in
        *"$want"*) ;;
        *) fail "the top bar line lacks $want" ;;
    esac
done
run plain "$OUT/plain.treatment.json" 30 "time=3"
case "$(line_of 'top bar' "$OUT/plain.log")" in
    *'artist=none'*) ;;
    *) fail "a treatment without an artist drew one" ;;
esac

# 2. The tabs, by click: the window's only mouse route to these panels.
for case_ in lyrics:lyrics export:export assist:assist look:none; do
    tab="${case_%%:*}"
    panel="${case_##*:}"
    point="$(centre "$base" "$tab")"
    run "tab-$tab" "$OUT/show.treatment.json" 30 "time=3,click=$point"
    got_panel="$(line_of panel "$OUT/tab-$tab.log")"
    got_bar="$(line_of 'top bar' "$OUT/tab-$tab.log")"
    case "$got_panel" in
        *" $panel") ;;
        *) fail "the $tab tab at $point left [$got_panel], wanted $panel" ;;
    esac
    case "$got_bar" in
        *"$tab*@"*) ;;
        *) fail "the $tab tab is not drawn selected after its press" ;;
    esac
    case "$got_bar" in
        *'direct*@'*) fail "Direct still reads selected after the $tab press" ;;
    esac
done
# Control: the gap between the tabs' group and Still. A press there must
# change nothing, or the presses above could be landing on anything.
gap="$(python3 - "$base" <<'PY'
import re, sys
line = sys.argv[1]
assist = re.search(r"assist\*?@(\d+),(\d+),(\d+),(\d+)", line).groups()
still = re.search(r"still@(\d+),(\d+),(\d+),(\d+)", line).groups()
right = int(assist[0]) + int(assist[2])
print(f"{(right + int(still[0])) // 2}x{int(still[1]) + int(still[3]) // 2}")
PY
)"
run tab-gap "$OUT/show.treatment.json" 30 "time=3,click=$gap"
case "$(line_of panel "$OUT/tab-gap.log")|$(line_of 'top bar' "$OUT/tab-gap.log")" in
    *' none|'*'direct*@'*) ;;
    *) fail "a press at $gap, between the tabs and Still, changed the panel" ;;
esac

# 3. Still and Contact sheet, through the export path.
run still "$OUT/show.treatment.json" 30 "time=3,click=$(centre "$base" still),save-to=$OUT/written-still.png"
grep -q '^export still: ' "$OUT/still.log" || fail "Still printed no export still: line"
[ -s "$OUT/written-still.png" ] || fail "Still wrote no file"
run sheet "$OUT/show.treatment.json" 30 "time=3,click=$(centre "$base" sheet),save-to=$OUT/written-sheet.png"
sheet_line="$(line_of 'contact sheet' "$OUT/sheet.log")"
printf '%s\n' "$sheet_line"
case "$sheet_line" in
    *'3 tiles from'*) ;;
    *) fail "Contact sheet did not write one tile per section (3)" ;;
esac
[ -s "$OUT/written-sheet.png" ] || fail "Contact sheet wrote no file"
python3 - "$OUT/written-sheet.png" "$OUT/written-still.png" <<'PY' || fail "a written file is not the image it should be"
import sys
from PIL import Image
sheet, still = Image.open(sys.argv[1]), Image.open(sys.argv[2])
assert still.size == (1920, 1080), still.size
# Three 480 px tiles in a row, plus gutters: wider than tall, under 2000 px.
assert 1000 < sheet.size[0] <= 2000 and sheet.size[1] < sheet.size[0], sheet.size
PY
# Controls: a folder that does not exist. Nothing may be written, and the
# sheet must say it refused rather than stay silent.
mkdir -p "$OUT/refused"
run still-refused "$OUT/show.treatment.json" 30 \
    "time=3,click=$(centre "$base" still),save-to=$OUT/refused/missing/still.png"
run sheet-refused "$OUT/show.treatment.json" 30 \
    "time=3,click=$(centre "$base" sheet),save-to=$OUT/refused/missing/sheet.png"
case "$(line_of 'contact sheet' "$OUT/sheet-refused.log")" in
    *'refused:'*) ;;
    *) fail "a sheet into a missing folder did not report its refusal" ;;
esac
if [ -n "$(find "$OUT/refused" -type f)" ]; then
    fail "a refused destination wrote a file: $(find "$OUT/refused" -type f)"
fi

# 4. The save word is drawn where the line says, and nowhere in fullscreen.
run fullscreen "$OUT/show.treatment.json" 30 "time=3,fullscreen=1"
case "$(line_of 'top bar' "$OUT/fullscreen.log")" in
    'top bar:'*'hidden (fullscreen)') ;;
    *) fail "the top bar line does not say it is hidden in fullscreen" ;;
esac
python3 - "$OUT/base.png" "$OUT/fullscreen.png" "$OUT/base.log" <<'PY' || fail "the save word's ink check failed"
import re, sys
from PIL import Image

base_png, full_png, log = sys.argv[1:4]
text = open(log, encoding="utf-8").read()
surface = tuple(int(re.search(r"^appearance:.* surface=#([0-9A-Fa-f]{6})", text, re.M)
                    .group(1)[i:i + 2], 16) for i in (0, 2, 4))
bar = re.search(r"^top bar:.*", text, re.M).group(0)
word = tuple(map(int, re.search(r" word=(\d+),(\d+),(\d+),(\d+)", bar).groups()))
group_x = int(re.search(r"tabs=direct\*?@(\d+)", bar).group(1))

def ink(png, box):
    """Pixels far from the bar's own fill: glyph cores, not anti-aliasing.
    Invariant under the ±1 a blend rounds to on either renderer."""
    image = Image.open(png).convert("RGB").crop(box)
    return sum(1 for p in image.get_flattened_data()
               if max(abs(a - b) for a, b in zip(p, surface)) > 40)

def fill_share(png, box):
    """Share of pixels that are the bar's surface exactly: an opaque fill,
    which every renderer writes bit-exactly."""
    image = Image.open(png).convert("RGB").crop(box)
    pixels = list(image.get_flattened_data())
    return sum(1 for p in pixels if p == surface) / len(pixels)

x, y, w, h = word
word_box = (x, y + 8, x + w, y + h - 8)
# The control strip: as wide as the word, in the empty stretch before the tabs.
empty_x = x + w + 24
empty_box = (empty_x, y + 8, min(empty_x + w, group_x - 4), y + h - 8)
word_ink, empty_ink = ink(base_png, word_box), ink(base_png, empty_box)
bar_fill = fill_share(base_png, (0, 2, group_x - 4, 40))
full_fill = fill_share(full_png, (0, 2, group_x - 4, 40))
print(f"save word: {word_ink} ink px at {word}; empty stretch {empty_ink}; "
      f"bar fill {bar_fill:.2f} drawn, {full_fill:.2f} in fullscreen")
assert word_ink >= 20, "no save word where the line says it is"
assert empty_ink == 0, "the classifier counts the bar's own fill as ink"
assert bar_fill > 0.6, "the bar's surface is not where the line says the bar is"
assert full_fill < 0.1, "something bar-coloured is drawn in fullscreen"
PY

# 5. The artist reaches the .musi, as v2, and only when there is one.
tools/headless_run.sh --treatment "$OUT/show.treatment.json" \
    --save-project "$OUT/saved/artist.musi" >"$OUT/save-artist.log" 2>&1 || true
tools/headless_run.sh --treatment "$OUT/plain.treatment.json" \
    --save-project "$OUT/saved/plain.musi" >"$OUT/save-plain.log" 2>&1 || true
case "$(head -c 200 "$OUT/saved/artist.musi" 2>/dev/null)" in
    '{"schema_version":"musializer.project/v2"'*'"title":"Top Bar Fixture","artist":"Chaos Cat Familiar"'*) ;;
    *) fail "a treatment's artist did not save as a v2 metadata.artist" ;;
esac
case "$(head -c 200 "$OUT/saved/plain.musi" 2>/dev/null)" in
    *'"artist"'*) fail "a project with no artist wrote one" ;;
    '{"schema_version":"musializer.project/v1"'*) ;;
    *) fail "a project with no artist was not written as v1" ;;
esac

if [ "$failures" -gt 0 ]; then
    printf '%d top bar check(s) failed; logs in %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'top bar: ok (%s)\n' "$OUT"
