#!/usr/bin/env bash
#
# The Lyrics view in the right column (WX-5d §3.2 and §5), checked headless.
#
#   tools/lyrics_column_check.sh [OUTPUT_DIR]          # default build/lyrics-column-check
#   tools/lyrics_column_check.sh --quick [OUTPUT_DIR]  # the checks, no capture matrix
#
# Every launch goes through tools/headless_run.sh: private Xvfb, unreachable
# Wayland and audio endpoints, kdialog/zenity off PATH, a private config
# directory, --mute. The fixture is
# synthetic: make-fixture-wav audio and a treatment with timed lines, a gap
# between them, and a caption ink no scene here draws (#FF2BD6, no glow, no
# backing), so "caption ink in the preview" is a colour count.
#
# Every point pressed is derived from a report line (`lyrics column:`,
# `caption style:`, `top bar:`), at --ui-scale 100 so rects are pixels.
#
# Checks, each with its control:
#   1. a press on a row (`row@=`) selects that cue and seeks to it
#      (`selected=` and `current=` name it); a press below the last row
#      changes neither;
#   2. at the line's time `current=` names it and its tick (`tick=`) is the
#      accent exactly, an opaque 2 px column; at another time `current=`
#      names another cue;
#   3. a press on the Style item of the head's switch opens Style; a face chip,
#      an ink swatch and a place cell each change `caption style:`; a press in
#      the gap beside each changes nothing;
#   4. Style open in a gap between lines: `sample=#id` and caption ink in the
#      preview; Cues open at the same time: `sample=none` and no ink. A still
#      written through the top bar's Still at that time is byte-identical with
#      Style open and with Cues open (the export never carries the sample);
#   5. at 960x640 the card and at least four rows; the font browser draws its
#      browsing body (`browser=…:browsing`, not `too-small`) at 292 px.
# Then, unless --quick, every pane (Cues, Style, Effects, fonts) at 960x640,
# 1280x720, 1920x1080 and 1920x1080 at 150 %, in both appearances.
#
# LYRICS_COLUMN_BIN names a different binary, for a negative control without
# replacing the build.
#
# NEGATIVE CONTROLS, run 2026-10-29 and again 2026-10-30 on the merged tree, with
# --quick on the GPU path and on the CPU path (MZ_GL_LAUNCH=""), each perturbation reverted with `git checkout` and the
# file proven clean after. Every one failed exactly its own check, on both:
#   - a row press that selects nothing (`if false &&` on the row's click):
#     check 1, "left selected=#7" and "did not seek there"; the gap press
#     stayed green;
#   - the tick filled `ui_muted` instead of the accent: check 2, 0/40 exact;
#   - an ink swatch press ignored: check 3, the second swatch not chosen; the
#     three gap presses stayed green;
#   - the sample dropped in `caption.rs`: check 4, 0 ink pixels (394 GPU,
#     395 CPU when drawn), the Cues control 0 either way;
#   - `SceneRenderer::sample_caption` cloned instead of taken, so the sample
#     outlives the preview draw: check 4's still md5 differs (GPU and CPU);
#   - `BROWSER_MIN_WIDTH` back at the oracle's 420: check 5, `too-small`.

set -euo pipefail
cd "$(dirname "$0")/.."

QUICK=0
if [ "${1:-}" = "--quick" ]; then
    QUICK=1
    shift
fi
OUT="${1:-build/lyrics-column-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${LYRICS_COLUMN_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 40 >/dev/null

# Fourteen lines from 2 s, a gap from 19 to 30 s, four more after it.
python3 - "$OUT/show.treatment.json" <<'PY'
import json, sys
lines = [{"at": 2.0 + 1.2 * i, "until": 2.0 + 1.2 * i + 1.1,
          "text": f"line {i + 1} of the opening verse"} for i in range(14)]
lines += [{"at": 30.0 + 2.0 * i, "until": 31.8 + 2.0 * i,
           "text": f"late line {i + 1}, after the break"} for i in range(4)]
treatment = {
    "schema": "musializer.treatment/v1",
    "title": "Lyrics Column Fixture",
    "audio": {"path": "fixture.wav"},
    "segments": [{"at": 0, "scene": "loom"}],
    "lyrics": lines,
    "caption_style": {
        "face": "alegreya", "box": "none", "anchor": "top_center",
        "size_scale": 0.12, "text_rgba": "#ff2bd6",
        "effects": {"glow_strength": 0.0},
    },
}
json.dump(treatment, open(sys.argv[1], "w"), indent=1)
PY
SHOW="$OUT/show.treatment.json"
# Line 7 (index 6) runs 9.2..10.3; its id is 7.
LINE_TIME=9.5
LINE_ID=7
OTHER_TIME=4.5
GAP_TIME=24

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
line_of() { # line_of KEY LOG
    grep -E "^$1: " "$2" | tail -n 1 || true
}
run() { # run NAME SIZE PROBE [extra args]
    local name="$1" size="$2" probe="$3"
    shift 3
    tools/headless_run.sh --treatment "$SHOW" --ui-scale 100 --ui-theme dark --hud=0 \
        --ui-probe "size=$size,play=0,column=lyrics,$probe" --probe-frames 40 \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}
field() { # field LINE KEY -> the value after `KEY=` up to a space
    python3 - "$1" "$2" <<'PY'
import re, sys
m = re.search(r"(?:^|\s)" + re.escape(sys.argv[2]) + r"=(\S+)", sys.argv[1])
print(m.group(1) if m else "")
PY
}
centre() { # centre "x,y,w,h" -> XxY
    python3 - "$1" <<'PY'
import sys
x, y, w, h = map(int, sys.argv[1].split(","))
print(f"{x + w // 2}x{y + h // 2}")
PY
}
rect_of() { # rect_of LINE NAME -> the x,y,w,h after `NAME@` (a starred name too)
    python3 - "$1" "$2" <<'PY'
import re, sys
m = re.search(r"(?:^|[\s;,=])" + re.escape(sys.argv[2]) + r"\*?@(-?\d+,-?\d+,\d+,\d+)", sys.argv[1])
print(m.group(1) if m else "")
PY
}

# ---------------------------------------------------------------------------
# 1. A row press selects and seeks; a press below the last row does nothing.
run base 1280x720 "time=$LINE_TIME"
base="$(line_of 'lyrics column' "$OUT/base.log")"
printf '%s\n' "${base:0:300}"
[ -n "$base" ] || fail "no lyrics column: line with the switch on"
target_id=$((LINE_ID + 2))
target="$(python3 - "$base" "$target_id" <<'PY'
import re, sys
m = re.search(r"#" + sys.argv[2] + r":(\d+,\d+,\d+,\d+)", sys.argv[1])
print(m.group(1) if m else "")
PY
)"
if [ -z "$target" ]; then
    fail "row #$target_id is not among the drawn rows"
else
    run row 1280x720 "time=$LINE_TIME,click=$(centre "$target")"
    row="$(line_of 'lyrics column' "$OUT/row.log")"
    [ "$(field "$row" selected)" = "#$target_id" ] ||
        fail "a press on row #$target_id left selected=$(field "$row" selected)"
    [ "$(field "$row" current)" = "#$target_id" ] ||
        fail "a press on row #$target_id did not seek there (current=$(field "$row" current))"
fi
gap="$(python3 - "$base" <<'PY'
import re, sys
line = sys.argv[1]
rows = re.findall(r"#\d+:(\d+),(\d+),(\d+),(\d+)", line.split("row@=")[1].split(" ")[0])
x, y, w, h = map(int, rows[-1])
fx, fy, fw, fh = map(int, re.search(r"footer@(\d+),(\d+),(\d+),(\d+)", line).groups())
print(f"{x + w // 2}x{(y + h + fy) // 2}")
PY
)"
run row-gap 1280x720 "time=$LINE_TIME,click=$gap"
row_gap="$(line_of 'lyrics column' "$OUT/row-gap.log")"
[ "$(field "$row_gap" selected)" = "#$LINE_ID" ] && [ "$(field "$row_gap" current)" = "#$LINE_ID" ] ||
    fail "a press at $gap, below the last row, changed selected/current to $(field "$row_gap" selected)/$(field "$row_gap" current)"

# ---------------------------------------------------------------------------
# 2. The current line and its accent tick.
[ "$(field "$base" current)" = "#$LINE_ID" ] ||
    fail "at $LINE_TIME s current=$(field "$base" current), wanted #$LINE_ID"
tick="$(field "$base" tick)"
tick_report="$(python3 - "$OUT/base.png" "$OUT/base.log" "$tick" <<'PY'
import re, sys
from PIL import Image
image = Image.open(sys.argv[1]).convert("RGB")
report = open(sys.argv[2]).read()
accent = re.search(r"^appearance:\s+\S+ accent=#([0-9A-Fa-f]{6})", report, re.M).group(1)
want = tuple(int(accent[i:i + 2], 16) for i in (0, 2, 4))
if not sys.argv[3]:
    print("0/0 no tick")
    sys.exit()
x, y, w, h = map(int, sys.argv[3].split(","))
pixels = [image.getpixel((x + i, y + j)) for i in range(w) for j in range(h)]
exact = sum(1 for p in pixels if p == want)
print(f"{exact}/{len(pixels)} accent=#{accent}")
PY
)"
echo "tick: $tick_report"
case "$tick_report" in
    "0/0"*) fail "no tick= on the current row" ;;
    *)
        exact="${tick_report%%/*}"
        rest="${tick_report#*/}"
        total="${rest%% *}"
        [ "$exact" = "$total" ] || fail "the tick is not the accent exactly ($tick_report)"
        ;;
esac
run other 1280x720 "time=$OTHER_TIME"
other_current="$(field "$(line_of 'lyrics column' "$OUT/other.log")" current)"
case "$other_current" in
    "#$LINE_ID" | none | "") fail "at $OTHER_TIME s current=$other_current; wanted another cue" ;;
esac

# ---------------------------------------------------------------------------
# 3. The pane switch, and three Style presses with a gap control each.
seg_style="$(rect_of "$base" style)"
run to-style 1280x720 "time=$LINE_TIME,click=$(centre "$seg_style")"
[ "$(field "$(line_of 'lyrics column' "$OUT/to-style.log")" pane)" = style ] ||
    fail "a press on the head's Style item did not open Style"
run style 1280x720 "time=$LINE_TIME,lyrics-pane=style"
style="$(line_of 'caption style' "$OUT/style.log")"
printf '%s\n' "${style:0:240}"
press_style() { # press_style NAME POINT
    run "$1" 1280x720 "time=$LINE_TIME,lyrics-pane=style,click=$2"
    line_of 'caption style' "$OUT/$1.log"
}
grotesk="$(rect_of "$style" grotesk)"
after="$(press_style face "$(centre "$grotesk")")"
case "$after" in *"grotesk*@"*) ;; *) fail "a press on the Grotesk chip did not choose it" ;; esac
gap_point="$(python3 - "$style" <<'PY'
import re, sys
a = list(map(int, re.search(r"alegreya\*?@(\d+),(\d+),(\d+),(\d+)", sys.argv[1]).groups()))
b = list(map(int, re.search(r"grotesk\*?@(\d+),(\d+),(\d+),(\d+)", sys.argv[1]).groups()))
print(f"{(a[0] + a[2] + b[0]) // 2}x{a[1] + a[3] // 2}")
PY
)"
after="$(press_style face-gap "$gap_point")"
[ "$(field "$after" faces)" = "$(field "$style" faces)" ] || fail "a press between the face chips changed the face"
swatch="$(python3 - "$style" <<'PY'
import re, sys
ink = re.search(r"ink=(\S+)", sys.argv[1]).group(1).split(";")
print(ink[1].split("@")[1])
PY
)"
after="$(press_style ink "$(centre "$swatch")")"
case "$(field "$after" ink)" in *f2be42ff\*@*) ;; *) fail "a press on the second ink swatch did not choose it ($(field "$after" ink | cut -c1-40))" ;; esac
ink_gap="$(python3 - "$style" <<'PY'
import re, sys
ink = re.search(r"ink=(\S+)", sys.argv[1]).group(1).split(";")
a = list(map(int, ink[0].split("@")[1].split(",")))
b = list(map(int, ink[1].split("@")[1].split(",")))
print(f"{(a[0] + a[2] + b[0]) // 2}x{a[1] + a[3] // 2}")
PY
)"
after="$(press_style ink-gap "$ink_gap")"
[ "$(field "$after" ink)" = "$(field "$style" ink)" ] || fail "a press between two ink swatches changed the ink"
top_left="$(rect_of "$style" top-left)"
after="$(press_style place "$(centre "$top_left")")"
[ "$(field "$after" place)" = top-left ] || fail "a press on the top-left place cell left place=$(field "$after" place)"
place_gap="$(python3 - "$style" <<'PY'
import re, sys
r = list(map(int, re.search(r"top-right\*?@(\d+),(\d+),(\d+),(\d+)", sys.argv[1]).groups()))
inset = list(map(int, re.search(r"inset@(\d+),(\d+),(\d+),(\d+)", sys.argv[1]).groups()))
print(f"{(r[0] + r[2] + inset[0]) // 2 + 1}x{r[1] + r[3] // 2}")
PY
)"
after="$(press_style place-gap "$place_gap")"
[ "$(field "$after" place)" = "$(field "$style" place)" ] || fail "a press beside the place frame changed the place"

# ---------------------------------------------------------------------------
# 4. The sample caption: preview only.
ink_count() { # ink_count NAME -> magenta caption pixels inside the preview frame
    python3 - "$OUT/$1.png" "$OUT/$1.log" <<'PY'
import re, sys
from PIL import Image
image = Image.open(sys.argv[1]).convert("RGB")
m = re.search(r"^preview frame:.*pixels (\d+),(\d+),(\d+),(\d+)", open(sys.argv[2]).read(), re.M)
x, y, w, h = map(int, m.groups())
want = (0xFF, 0x2B, 0xD6)
count = 0
for j in range(y, y + h, 1):
    for i in range(x, x + w, 2):
        p = image.getpixel((i, j))
        if all(abs(p[k] - want[k]) <= 24 for k in range(3)):
            count += 1
print(count)
PY
}
run sample-style 1280x720 "time=$GAP_TIME,lyrics-pane=style"
run sample-cues 1280x720 "time=$GAP_TIME,lyrics-pane=cues"
s_line="$(line_of 'lyrics column' "$OUT/sample-style.log")"
c_line="$(line_of 'lyrics column' "$OUT/sample-cues.log")"
s_ink="$(ink_count sample-style)"
c_ink="$(ink_count sample-cues)"
echo "sample: style $(field "$s_line" sample) ink=$s_ink, cues $(field "$c_line" sample) ink=$c_ink"
case "$(field "$s_line" sample)" in "#"*) ;; *) fail "Style in a gap offered no sample" ;; esac
[ "$(field "$c_line" sample)" = none ] || fail "Cues offered a sample"
[ "$s_ink" -ge 150 ] || fail "the sample caption drew $s_ink ink pixels in the preview (want >= 150)"
[ "$c_ink" -le 10 ] || fail "with Cues open the preview carries $c_ink caption ink pixels (want <= 10)"
top="$(line_of 'top bar' "$OUT/sample-style.log")"
still="$(rect_of "$top" still)"
if [ -z "$still" ]; then
    fail "no still@ on the top bar line"
else
    run still-style 1280x720 "time=$GAP_TIME,lyrics-pane=style,save-to=$OUT/export-style.png,click=$(centre "$still")"
    run still-cues 1280x720 "time=$GAP_TIME,lyrics-pane=cues,save-to=$OUT/export-cues.png,click=$(centre "$still")"
    if [ ! -s "$OUT/export-style.png" ] || [ ! -s "$OUT/export-cues.png" ]; then
        fail "Still wrote no file ($(ls "$OUT"/export-*.png 2>/dev/null | wc -l) of 2)"
    else
        a="$(md5sum <"$OUT/export-style.png")"
        b="$(md5sum <"$OUT/export-cues.png")"
        echo "still md5: style ${a:0:12} cues ${b:0:12}"
        [ "$a" = "$b" ] || fail "the still differs with Style open: the sample reached the export path"
    fi
fi

# ---------------------------------------------------------------------------
# 5. 960x640: the card and four rows; the browser at 292 px.
run small 960x640 "time=$LINE_TIME"
small="$(line_of 'lyrics column' "$OUT/small.log")"
rows="$(field "$small" rows)"
shown="$(python3 - "$rows" <<'PY'
import sys
a, b = sys.argv[1].split("/")[0].split("..")
print(int(b) - int(a) + 1)
PY
)"
[ "$shown" -ge 4 ] || fail "960x640 shows $shown rows under the card (want >= 4)"
[ -n "$(field "$small" card)" ] || fail "960x640 drew no card"
printf 'musializer.font-catalogue/v1\t3\nInter\tsans-serif\tlatin,greek\nCaveat\thandwriting\tlatin\nJetBrains Mono\tmonospace\tlatin,cyrillic\n' \
    >"$OUT/catalogue.tsv"
run fonts-small 960x640 "time=$LINE_TIME,fonts=$OUT/catalogue.tsv"
browser="$(field "$(line_of 'lyrics column' "$OUT/fonts-small.log")" browser)"
echo "browser at 960x640: $browser"
case "$browser" in
    *,292,*:browsing) ;;
    *) fail "the font browser at 960x640 is $browser, wanted 292 px wide and browsing" ;;
esac

# ---------------------------------------------------------------------------
# The capture matrix: every pane at every size, both appearances.
if [ "$QUICK" = 0 ]; then
    for theme in dark light; do
        for spec in 960x640:100 1280x720:100 1920x1080:100 1920x1080:150; do
            size="${spec%%:*}"
            scale="${spec##*:}"
            for pane in cues style effects fonts; do
                case "$pane" in
                    fonts) probe="fonts=$OUT/catalogue.tsv" ;;
                    *) probe="lyrics-pane=$pane" ;;
                esac
                name="matrix-$theme-$size-$scale-$pane"
                tools/headless_run.sh --treatment "$SHOW" --ui-scale "$scale" --ui-theme "$theme" \
                    --hud=0 --ui-probe "size=$size,play=0,column=lyrics,time=$LINE_TIME,$probe" \
                    --probe-frames 40 --probe-shot "$OUT/$name.png" >"$OUT/$name.log" 2>&1 || true
                [ -s "$OUT/$name.png" ] || fail "no capture for $name"
                got="$(field "$(line_of 'lyrics column' "$OUT/$name.log")" pane)"
                [ "$got" = "$pane" ] || fail "$name drew pane=$got"
            done
        done
    done
    echo "matrix: $(ls "$OUT"/matrix-*.png | wc -l) captures in $OUT"
fi

if [ "$failures" -gt 0 ]; then
    echo "lyrics_column_check: $failures failure(s); evidence in $OUT"
    exit 1
fi
echo "lyrics_column_check: ok ($OUT)"
