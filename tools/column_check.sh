#!/usr/bin/env bash
#
# The column gate (WX-5d, build/redesign/wx5d-design.md §5, the Frame checks):
# the right column, checked headless.
#
#   tools/column_check.sh [OUTPUT_DIR]        # default build/column-check
#
# Every launch goes through tools/headless_run.sh: private Xvfb, unreachable
# Wayland and audio endpoints, kdialog/zenity off PATH, a private config
# directory, --mute. Destinations are given with `--ui-probe save-to=`, so no
# picker is ever asked for. The fixture is synthetic: make-fixture-wav audio
# and a three-section treatment. Every press aims at a rect from a report
# line (`top bar:`, `column:`, `render view:`), never at a pixel measured
# once, and every run is at --ui-scale 100 except the 150 % row.
#
# Checks, each with its control:
#   1. Tabs: a tab press by its `top bar:` rect sets `column: view=`; a press
#      on the empty stretch between the tabs and Still changes nothing; the
#      selected tab again closes the column.
#   2. The frame does not move: `preview frame:`, `transport:` and the band
#      (`column: band=`) are identical across all five views at 960x640,
#      1280x720, 1920x1080 and 2560x1440 at 150 %.
#   3. The accent census (`--primary 1`) in every view, both appearances —
#      Look also with its route editor open, Assist also with a card open —
#      with the two marks the report permits (the current lyric line's time
#      and tick, the route plot's live dot), and an accent slab in each named
#      rect passes only while the report names it; in the Render view the accent lies inside
#      the commit and the top bar's Render… holds none.
#   4. ui.json: a dragged width and the view survive a restart; a pre-WX-5d
#      file with `dock_width 480, dock_open true` opens a 480 column on Direct.
#   5. The dock in the column: `dock: rect=` is the column's; a focused
#      terminal holds the keyboard on Direct and releases it when the view
#      leaves Direct; a reply behind Lyrics lights the Direct tab
#      (`badge=direct:reply`) and the same reply on Direct does not; a
#      permission prompt behind Lyrics is `waiting` and posts one warning.
#   6. The Render view: a SIZE press by its rect changes `export config:`; a
#      press between two shape tiles changes nothing; the commit with
#      `save-to=` renders the file.
#   7. The progress screen (`progress-shot=N`): the ground is the surface
#      exactly, the bar's filled part is the accent exactly, the encoded
#      frame is drawn; at frame 0 the bar holds no accent. (The ground
#      test's control was the band layout's old screen until the WX-5d flip
#      deleted it; it is now the background perturbation in build A below.)
#   8. The Direct view's chat (§3.1), at 1920x1080: with no chat, a Start row
#      per agent inside the column (and none once a chat is open); your
#      bubble on the right, the agent's prompt card, chips and prose at the
#      left inset, the composer at the bottom with its send button inside;
#      the waiting prompt outlined in the warning tone with an ink Allow;
#      the census in both appearances; Allow reaches the agent and its reply
#      is drawn; a followed treatment rewritten mid-run is noted in the chat
#      as a notice, and an unrewritten one is not. The welcome screen, which
#      has no column, draws no dock (the control is every run above with a
#      track, whose `dock: rect=` is the column's).
#
# COLUMN_BIN names a different binary, for a negative control without
# replacing the build. MZ_GL_LAUNCH="" runs it on the CPU.
#
# NEGATIVE CONTROLS, run 2026-10-29 on the GPU path and on llvmpipe
# (MZ_GL_LAUNCH=""), each a binary built from a perturbed tree that was then
# restored with `git checkout` and shown clean; identical failures on both:
#   A (16 failures): the tab press calling nothing (check 1's five, the gap
#     control still green, and check 4's restart, whose save is a tab
#     press); the column 1 px wider on Lyrics (check 2 at all four sizes);
#     the commit drawn Neutral (check 3: census 0, no accent in the commit);
#     the keyboard release removed (check 5: keyboard=true after leaving);
#     the progress ground cleared to the old background (check 7).
#   B (8 failures): Render… left Primary while its view is open (census 2,
#     ~1900 accent px on Render…); the dock_width seed removed (check 4's
#     old file); the Direct dot never set (reply and waiting fail, the
#     on-Direct control still `none`); the bar filled in the level tone
#     instead of the accent (check 7's exact bar; the frame-0 control green).
#   C (4 failures, the Direct chat): the empty state not drawn (no Start
#     rows); your bubble left-aligned (gap 155 px from the right); Allow
#     drawn on the raised fill and the prompt outlined in the rule tone
#     (outline 0.00, Allow brightness 27 against >= 170); the window's apply
#     never noted — while the unrewritten-treatment control stayed green.
# The frame-0 control first failed on llvmpipe alone: a readback there
# returned the newest frame, not the one before (docs/TRAPS.md), so the
# probe now holds the encoder and photographs a still screen.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/column-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${COLUMN_BIN:-$(realpath target/debug/musializer)}"
export SHELL=/bin/sh

cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 6 >/dev/null
cat >"$OUT/show.treatment.json" <<'EOF'
{ "schema": "musializer.treatment/v1", "title": "Column Fixture",
  "audio": { "path": "fixture.wav" },
  "segments": [ { "at": 0, "scene": "spectrum" }, { "at": 2, "scene": "loom" },
                { "at": 4, "scene": "cadence" } ],
  "lyrics": [ { "at": 1.0, "until": 2.4, "text": "the first line" },
              { "at": 2.6, "until": 3.6, "text": "the line at the playhead" },
              { "at": 3.8, "until": 5.0, "text": "the line after it" } ] }
EOF
SHOW="$OUT/show.treatment.json"

failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
line_of() { # line_of KEY LOG
    grep -E "^$1: " "$2" | tail -n 1 | sed -E "s/^$1: +//" || true
}
# run NAME FRAMES PROBE [extra args]; state in $OUT/state-NAME unless STATE is set.
run() {
    local name="$1" frames="$2" probe="$3"
    shift 3
    MZ_STATE_DIR="${STATE:-$OUT/state-$name}" tools/headless_run.sh --treatment "$SHOW" \
        --ui-scale "${SCALE:-100}" --ui-probe "$probe" --probe-frames "$frames" \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}
# rect NAME LINE -> "x y w h" of `NAME@x,y,w,h` (a `*` may follow NAME).
rect() {
    python3 - "$1" "$2" <<'PY'
import re, sys
name, line = sys.argv[1], sys.argv[2]
m = re.search(r"(?:^|[ ;,=])" + re.escape(name) + r"\*?@(-?\d+),(-?\d+),(\d+),(\d+)", line)
if not m:
    m = re.search(r"(?:^| )" + re.escape(name) + r"=(-?\d+),(-?\d+),(\d+),(\d+)", line)
if not m:
    sys.exit(f"no rect for {name}")
print(*m.groups())
PY
}
centre() { # centre NAME LINE -> XxY
    local r
    r="$(rect "$1" "$2")" || return 1
    read -r x y w h <<<"$r"
    echo "$((x + w / 2))x$((y + h / 2))"
}
field() { # field KEY LINE -> the value of `KEY=value`
    sed -nE "s/.*(^| )$1=([^ ]*).*/\\2/p" <<<"$2"
}

# --- 1. Tabs -----------------------------------------------------------------
run base 30 "size=1280x720,time=3,column=direct"
bar="$(line_of 'top bar' "$OUT/base.log")"
col="$(line_of column "$OUT/base.log")"
printf 'column: %s\n' "${col:0:200}"
case "$col" in
    *"view=direct open=1 rect=922,46,358,674"*) ;;
    *) fail "the base run is not a 358 column on Direct at 1280x720: $col" ;;
esac
case "$bar" in
    *"tabs=direct*@"*";assist@"*) ;;
    *) fail "the top bar does not draw the four column tabs: ${bar:0:200}" ;;
esac
case "$bar" in
    *"export@"*) fail "Export is still a tab with the column layout" ;;
esac
for tab in lyrics look assist; do
    run "tab-$tab" 30 "size=1280x720,time=3,column=direct,click=$(centre "$tab" "$bar")"
    got="$(field view "$(line_of column "$OUT/tab-$tab.log")")"
    [ "$got" = "$tab" ] || fail "the $tab tab's press left the column on $got"
done
run tab-direct-again 30 "size=1280x720,time=3,column=direct,click=$(centre direct "$bar")"
got="$(field view "$(line_of column "$OUT/tab-direct-again.log")")"
[ "$got" = "direct" ] && fail "the selected tab's press did not close the column"
case "$(line_of column "$OUT/tab-direct-again.log")" in
    *"open=0"*) ;;
    *) fail "the selected tab's press left the column open" ;;
esac
read -r ax _ aw _ <<<"$(rect assist "$bar")"
read -r sx _ _ _ <<<"$(rect still "$bar")"
gap="$(((ax + aw + sx) / 2))x23"
run tab-gap 30 "size=1280x720,time=3,column=direct,click=$gap"
got="$(field view "$(line_of column "$OUT/tab-gap.log")")"
[ "$got" = "direct" ] || fail "a press at $gap, between the tabs and Still, changed the view to $got"

# --- 2. The frame does not move ------------------------------------------------
for size in 960x640 1280x720 1920x1080 2560x1440@150; do
    window="${size%@*}"
    scale=100
    [ "$size" != "$window" ] && scale="${size#*@}"
    reference=""
    for view in direct lyrics look assist render; do
        name="frame-$window-$view"
        SCALE=$scale run "$name" 24 "size=$window,time=3,column=$view"
        frame="$(line_of 'preview frame' "$OUT/$name.log")"
        transport="$(line_of transport "$OUT/$name.log")"
        band="$(field band "$(line_of column "$OUT/$name.log")")"
        width="$(rect rect "$(line_of column "$OUT/$name.log")" | cut -d' ' -f3)"
        got="$frame | $transport | band=$band | column-width=$width"
        if [ -z "$frame" ] || [ -z "$band" ]; then
            fail "$name reported no frame or band"
        elif [ -z "$reference" ]; then
            reference="$got"
            printf '%s: %s\n' "$size" "${frame:0:80} band=$band column=$width"
        elif [ "$got" != "$reference" ]; then
            fail "the frame moved at $size between direct and $view: $got"
        fi
    done
done

# --- 3. The accent census ------------------------------------------------------
# Look with its route editor open (the plot's live dot) and Assist with a
# card open (selection, never the accent) are views of their own here.
for theme in dark light; do
    for view in direct lyrics look look-route assist assist-open render; do
        name="census-$theme-$view"
        case "$view" in
            # At 1 s, where the treatment's first scene is the drawn one: the
            # probe opens a route on the scene frame 0 draws.
            look-route) probe="column=look,route=settings.spectrum.amplitude,time=1" ;;
            assist-open) probe="column=assist,assist=confirm" ;;
            *) probe="column=$view" ;;
        esac
        run "$name" 30 "size=1280x720,time=3,$probe" --ui-theme "$theme"
        if ! python3 tools/accent_census.py "$OUT/$name.png" "$OUT/$name.log" --primary 1 \
            >"$OUT/$name.census" 2>&1; then
            fail "census $theme $view: $(tail -n 3 "$OUT/$name.census" | tr '\n' ' ')"
        fi
    done
done
# The census's two permitted marks: the report is what admits them. The marks
# as drawn are too thin to prove that (12 px text never reaches the accent
# within the census's tolerance), so an accent slab goes into each named rect:
# with the naming line it passes, and with that line removed it must fail.
for pair in lyrics:time look-route:plot; do
    view="${pair%%:*}"
    key="${pair##*:}"
    name="census-dark-$view"
    if ! grep -qE "\b$key@" "$OUT/$name.log"; then
        fail "$view drew no $key@, so its mark is not in this census"
        continue
    fi
    python3 - "$OUT/$name.png" "$OUT/$name.log" "$key" "$OUT/$name.painted.png" <<'PY'
import re, sys
from PIL import Image
png, log, key, out = sys.argv[1:]
text = open(log, encoding="utf-8", errors="replace").read()
accent = re.search(r"^appearance:.* accent=#([0-9A-Fa-f]{6})", text, re.M).group(1)
fill = tuple(int(accent[i:i + 2], 16) for i in (0, 2, 4))
x, y, w, h = map(int, re.search(r"\b" + key + r"@(\d+),(\d+),(\d+),(\d+)", text).groups())
image = Image.open(png).convert("RGB")
image.paste(fill, (x + 4, y + 4, x + 24, y + 16))
image.save(out)
PY
    grep -vE '^(lyrics column|tune layout):' "$OUT/$name.log" >"$OUT/$name.unnamed.log"
    python3 tools/accent_census.py "$OUT/$name.painted.png" "$OUT/$name.log" --primary 1 \
        >"$OUT/$name.painted.census" 2>&1 ||
        fail "the census refuses a mark the report names, in $view: $(tail -n 1 "$OUT/$name.painted.census")"
    if python3 tools/accent_census.py "$OUT/$name.painted.png" "$OUT/$name.unnamed.log" --primary 1 \
        >"$OUT/$name.unnamed.census" 2>&1; then
        fail "the census passes $view without the report naming its mark, so the mark is not checked"
    fi
done
# Where the one accent is in the Render view: in the commit, not on Render….
accent_in() { # accent_in PNG LOG X Y W H -> count of accent pixels
    python3 - "$@" <<'PY'
import re, sys
import numpy as np
from PIL import Image
png, log, x, y, w, h = sys.argv[1], sys.argv[2], *map(int, sys.argv[3:])
text = open(log, encoding="utf-8", errors="replace").read()
accent = re.search(r"^appearance:.* accent=#([0-9A-Fa-f]{6})", text, re.M).group(1)
target = np.array([int(accent[i:i + 2], 16) for i in (0, 2, 4)])
pixels = np.asarray(Image.open(png).convert("RGB"), dtype=int)[y:y + h, x:x + w]
print(int((np.abs(pixels - target).max(axis=2) <= 18).sum()))
PY
}
for theme in dark light; do
    name="census-$theme-render"
    view_line="$(line_of 'render view' "$OUT/$name.log")"
    bar_line="$(line_of 'top bar' "$OUT/$name.log")"
    read -r cx cy cw ch <<<"$(rect commit "$view_line")"
    read -r rx ry rw rh <<<"$(rect 'render(view)' "$bar_line")"
    in_commit="$(accent_in "$OUT/$name.png" "$OUT/$name.log" "$cx" "$cy" "$cw" "$ch")"
    on_button="$(accent_in "$OUT/$name.png" "$OUT/$name.log" "$rx" "$ry" "$rw" "$rh")"
    printf 'render view %s: accent in the commit %s, on Render… %s\n' "$theme" "$in_commit" "$on_button"
    [ "$in_commit" -gt $((cw * ch / 2)) ] || fail "$theme: the commit is not the accent fill ($in_commit px)"
    [ "$on_button" -eq 0 ] || fail "$theme: Render… keeps the accent while its view is open ($on_button px)"
done

# --- 4. ui.json ----------------------------------------------------------------
STATE="$OUT/state-persist" run persist-1 30 \
    "size=1280x720,time=3,column=direct,column-width=500,click=$(centre lyrics "$bar")"
STATE="$OUT/state-persist" run persist-2 20 "size=1280x720,time=3"
case "$(line_of column "$OUT/persist-2.log")" in
    *"view=lyrics open=1 rect=780,46,500,674"*) ;;
    *) fail "a 500 column on Lyrics did not survive the restart: $(line_of column "$OUT/persist-2.log")" ;;
esac
mkdir -p "$OUT/state-old/config/musializer"
cat >"$OUT/state-old/config/musializer/ui.json" <<'EOF'
{"schema":"musializer.ui-preferences/v1","scale":"auto","sidebar_width":null,
 "inspector_width":null,"timeline_height":null,"lyric_lane_height":null,
 "lyric_tap_offset":null,"dock_open":true,"dock_width":480.0}
EOF
STATE="$OUT/state-old" run old-file 20 "size=1280x720,time=3"
case "$(line_of column "$OUT/old-file.log")" in
    *"view=direct open=1 rect=800,46,480,674"*) ;;
    *) fail "a pre-WX-5d dock_width 480 did not open a 480 column on Direct: $(line_of column "$OUT/old-file.log")" ;;
esac

# --- 5. The dock in the column -------------------------------------------------
run dock-focus 60 'size=1600x900,column=direct,dock-run=echo dock-$((6*7)),dock-focus=1'
dock="$(line_of dock "$OUT/dock-focus.log")"
col_rect="$(rect rect "$(line_of column "$OUT/dock-focus.log")" | tr ' ' ',')"
case "$dock" in
    *"dock-42"*"rect=$col_rect") ;;
    *) fail "the dock did not run in the column's rect ($col_rect): ${dock:0:120}…${dock: -40}" ;;
esac
case "$dock" in
    *"keyboard=true"*) ;;
    *) fail "a focused terminal on Direct did not hold the keyboard" ;;
esac
run dock-leave 60 'size=1600x900,column=direct,dock-run=echo leave,dock-focus=1,column-switch=lyrics'
case "$(line_of dock "$OUT/dock-leave.log")" in
    *"keyboard=false"*) ;;
    *) fail "leaving Direct did not release the keyboard: $(line_of dock "$OUT/dock-leave.log" | cut -c1-160)" ;;
esac
[ "$(field view "$(line_of column "$OUT/dock-leave.log")")" = "lyrics" ] ||
    fail "column-switch=lyrics did not reach Lyrics"
FAKE="$OUT/fake-agent"
printf '#!/bin/sh\nexec /usr/bin/python3 %s "$@"\n' "$(realpath tools/fake_agent.py)" >"$FAKE"
chmod +x "$FAKE"
agent_run() { # agent_run NAME VIEW SAY
    env MUSIALIZER_AGENT_CLAUDE="$FAKE" MUSIALIZER_AGENT_CODEX="$FAKE" MUSIALIZER_AGENT_AGY="$FAKE" \
        MZ_STATE_DIR="$OUT/state-$1" tools/headless_run.sh --treatment "$SHOW" --ui-scale 100 \
        --ui-probe "size=1600x900,column=$2,agent=claude,agent-say=$3" --probe-frames 120 \
        --probe-shot "$OUT/$1.png" >"$OUT/$1.log" 2>&1 || true
}
agent_run badge-away lyrics 'make the bridge colder'
agent_run badge-here direct 'make the bridge colder'
agent_run badge-wait lyrics 'please ask first'
for case_ in badge-away:reply badge-here:none badge-wait:waiting; do
    name="${case_%%:*}"
    want="${case_##*:}"
    got="$(field badge "$(line_of column "$OUT/$name.log")")"
    [ "$got" = "direct:$want" ] || fail "$name: the Direct tab's dot is $got, not direct:$want"
done
case "$(line_of dock "$OUT/badge-here.log")" in
    *"replies=1"*) ;;
    *) fail "the reply on Direct never arrived, so the badge control proves nothing" ;;
esac
case "$(line_of notices "$OUT/badge-wait.log")" in
    *"warning"*) ;;
    *) fail "a permission prompt behind Lyrics posted no warning" ;;
esac

# --- 6. The Render view --------------------------------------------------------
run render 30 "size=1280x720,time=3,column=render"
view_line="$(line_of 'render view' "$OUT/render.log")"
before="$(line_of 'export config' "$OUT/render.log")"
run render-720 30 "size=1280x720,time=3,column=render,click=$(centre 720p "$view_line")"
case "$(line_of 'export config' "$OUT/render-720.log")" in
    "1280x720 at"*) ;;
    *) fail "the 720p press did not reach the config: $(line_of 'export config' "$OUT/render-720.log")" ;;
esac
read -r tx ty tw th <<<"$(rect 16:9 "$view_line")"
run render-tile-gap 30 "size=1280x720,time=3,column=render,click=$((tx + tw + 3))x$((ty + th / 2))"
[ "$(line_of 'export config' "$OUT/render-tile-gap.log")" = "$before" ] ||
    fail "a press between two shape tiles changed the config"
mkdir -p "$OUT/renders"
run render-go 40 "size=1280x720,time=3,column=render,save-to=$OUT/renders/clip.mp4,click=$(centre commit "$view_line"),progress-shot=20" \
    --encoder "${MZ_ENCODER:-nvenc}" --render-window 1 2
[ -s "$OUT/renders/clip.mp4" ] || fail "the commit with save-to= wrote no video"

# --- 7. The progress screen ----------------------------------------------------
pixel_check() { # pixel_check PNG LOG MODE -> prints and exits nonzero on failure
    python3 - "$@" <<'PY'
import re, sys
import numpy as np
from PIL import Image
png, log, mode = sys.argv[1:]
text = open(log, encoding="utf-8", errors="replace").read()
look = re.search(r"^appearance:.* accent=#(\w{6}) surface=#(\w{6})", text, re.M)
hexrgb = lambda h: np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)])
accent, surface = hexrgb(look.group(1)), hexrgb(look.group(2))
image = np.asarray(Image.open(png).convert("RGB"), dtype=int)
line = re.search(r"^export progress:\s+(.*)$", text, re.M)
fields = dict(kv.split("=", 1) for kv in line.group(1).split()) if line else {}
# The ground: an opaque fill, so compared exactly, away from every drawn part.
ground = image[8, 8]
ground_ok = bool((ground == surface).all())
if mode == "ground":
    print(f"ground {tuple(ground)} surface {tuple(surface)}")
    sys.exit(0 if ground_ok else 1)
x, y, w, h = map(int, fields["bar"].split(","))
fill = int(fields["fill"])
bar = image[y:y + h, x:x + w]
accent_px = int((np.abs(bar - accent).max(axis=2) == 0).sum())
# Inside the fill, clear of the rounded ends: the accent exactly.
inner = image[y + 2:y + h - 2, x + 4:x + max(fill - 4, 4)]
inner_ok = fill > 8 and bool((np.abs(inner - accent).max(axis=2) == 0).all())
print(f"ground_ok={ground_ok} fill={fill} accent_px={accent_px} inner_exact={inner_ok} thumb={fields.get('thumb')}")
if mode == "zero":
    sys.exit(0 if accent_px == 0 else 1)
sys.exit(0 if ground_ok and inner_ok and fields.get("thumb") == "drawn" else 1)
PY
}
progress_png="$OUT/render-go-progress.png"
if [ -s "$progress_png" ]; then
    pixel_check "$progress_png" "$OUT/render-go.log" full || fail "the progress screen at frame 20 is not the themed one"
else
    fail "progress-shot=20 wrote no picture"
fi
run render-zero 40 "size=1280x720,time=3,column=render,save-to=$OUT/renders/zero.mp4,click=$(centre commit "$view_line"),progress-shot=0" \
    --encoder "${MZ_ENCODER:-nvenc}" --render-window 1 2
if [ -s "$OUT/render-zero-progress.png" ]; then
    pixel_check "$OUT/render-zero-progress.png" "$OUT/render-zero.log" zero ||
        fail "the bar holds accent at frame 0"
else
    fail "progress-shot=0 wrote no picture"
fi

# --- 8. The Direct view's chat (§3.1) ------------------------------------------
# Rects from the `dock:` line's direct=[…]; the fake agent from check 5.
direct_of() { # direct_of LOG -> the direct=[…] body
    line_of dock "$1" | sed -nE 's/.* direct=\[([^]]*)\].*/\1/p'
}
geometry() { # geometry DIRECT COLUMN-RECT MODE -> exits nonzero on failure
    python3 - "$@" <<'PY'
import re, sys
direct, column, mode = sys.argv[1], sys.argv[2], sys.argv[3]
cx, cy, cw, ch = map(int, column.split(","))
rects = {}
for kind, x, y, w, h in re.findall(r"(\w+)@(-?\d+),(-?\d+),(\d+),(\d+)", direct):
    rects.setdefault(kind, []).append(tuple(map(int, (x, y, w, h))))
problems = []
if mode == "empty":
    for agent in ("claude", "codex", "agy"):
        if agent not in rects:
            problems.append(f"no Start for {agent}")
        else:
            x, y, w, h = rects[agent][0]
            if not (cx < x and x + w <= cx + cw and cy < y < cy + ch):
                problems.append(f"{agent}'s Start lies outside the column")
else:
    for kind in ("user", "permission", "allow", "composer", "send"):
        if kind not in rects:
            problems.append(f"no {kind} drawn")
    if "user" in rects:
        x, y, w, h = rects["user"][0]
        right_gap = cx + cw - (x + w)
        # Your words sit on the right, as in every chat.
        if not (8 <= right_gap <= 22 and x > cx + cw * 0.12):
            problems.append(f"the user bubble is not right-aligned (x={x} w={w}, gap {right_gap})")
    for kind in ("permission", "tool", "assistant"):
        for x, y, w, h in rects.get(kind, []):
            if abs(x - (cx + 15)) > 3:
                problems.append(f"{kind} does not start at the column's left inset (x={x})")
    if "composer" in rects and "send" in rects:
        x, y, w, h = rects["composer"][0]
        sx, sy, sw, sh = rects["send"][0]
        if not (x <= sx and sx + sw <= x + w and y <= sy and sy + sh <= y + h):
            problems.append("the send button is not inside the composer")
        if y + h < cy + ch - 300:
            problems.append("the composer is not at the bottom of the chat")
print("; ".join(problems) or "ok")
sys.exit(1 if problems else 0)
PY
}
pixels() { # pixels PNG LOG DIRECT -> the permission card's outline and Allow's fill
    python3 - "$@" <<'PY'
import re, sys
import numpy as np
from PIL import Image
png, log, direct = sys.argv[1:]
text = open(log, encoding="utf-8", errors="replace").read()
rgb = lambda h: np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)])
# The warning and ink roles, from the palette's report line.
look = re.search(r"^appearance:\s+(\S+)", text, re.M).group(1)
warning = rgb(re.search(r"^appearance:.* warning=#(\w{6})", text, re.M).group(1))
image = np.asarray(Image.open(png).convert("RGB"), dtype=int)
rects = dict((k, tuple(map(int, v))) for k, *v in re.findall(r"(\w+)@(-?\d+),(-?\d+),(\d+),(\d+)", direct))
x, y, w, h = rects["permission"]
# The 1 px outline lands on a fraction of a pixel: classify each side's
# two-pixel band as "near the warning tone" rather than read one exact pixel.
near = lambda region: (np.abs(region - warning).max(axis=2) <= 40)
sides = [image[y:y + 2, x + 8:x + w - 8], image[y + h - 2:y + h, x + 8:x + w - 8],
         image[y + 8:y + h - 8, x:x + 2], image[y + 8:y + h - 8, x + w - 2:x + w]]
outline = min(near(side).any(axis=0 if side.shape[0] == 2 else 1).mean() for side in sides)
ax, ay, aw, ah = rects["allow"]
inside = image[ay + 6:ay + ah - 6, ax + 4:ax + 10]
ink = image[ay + ah // 2, ax + 4]
brightness = inside.mean()
print(f"outline={outline:.2f} allow-brightness={brightness:.0f}")
ok_outline = outline >= 0.9
ok_allow = brightness >= 170 if look == "black-amber" else brightness <= 90
sys.exit(0 if ok_outline and ok_allow else 1)
PY
}
direct_run() { # direct_run NAME PROBE [extra]
    local name="$1" probe="$2"
    shift 2
    env MUSIALIZER_AGENT_CLAUDE="$FAKE" MUSIALIZER_AGENT_CODEX="$FAKE" MUSIALIZER_AGENT_AGY="$FAKE" \
        MZ_STATE_DIR="$OUT/state-$name" tools/headless_run.sh --treatment "${TREATMENT:-$SHOW}" --ui-scale 100 \
        --ui-probe "size=1920x1080,time=3,column=direct${probe:+,$probe}" --probe-frames "${FRAMES:-120}" \
        --probe-shot "$OUT/$name.png" "$@" >"$OUT/$name.log" 2>&1 || true
}
# The welcome screen has no column: its right side is the recent-project list.
MZ_STATE_DIR="$OUT/state-welcome" tools/headless_run.sh --ui-scale 100 --size 1280x720 --probe-frames 20 \
    >"$OUT/welcome.log" 2>&1 || true
case "$(line_of dock "$OUT/welcome.log")" in
    *"direct=[start ]"*) ;;
    *) fail "the Direct view drew over the welcome screen: $(line_of dock "$OUT/welcome.log" | tail -c 80)" ;;
esac
direct_run direct-empty ""
column_rect="$(rect rect "$(line_of column "$OUT/direct-empty.log")" | tr ' ' ',')"
empty="$(direct_of "$OUT/direct-empty.log")"
printf 'direct (empty): %s\n' "${empty:0:160}"
case "$empty" in
    start\ *) geometry "$empty" "$column_rect" empty >/dev/null || fail "the empty state: $(geometry "$empty" "$column_rect" empty)" ;;
    *) fail "with no chat, the Direct view drew no invitation: ${empty:0:120}" ;;
esac
direct_run direct-ask 'agent=claude,agent-say=please ask first: make the bridge colder'
asked="$(direct_of "$OUT/direct-ask.log")"
printf 'direct (prompt): %s\n' "${asked:0:200}"
case "$asked" in
    chat\ *) ;;
    *) fail "with a chat open, the Direct view still drew the invitation" ;;
esac
geometry "$asked" "$column_rect" chat >"$OUT/direct-ask.geometry" ||
    fail "the chat's layout: $(cat "$OUT/direct-ask.geometry")"
pixels "$OUT/direct-ask.png" "$OUT/direct-ask.log" "$asked" >"$OUT/direct-ask.pixels" ||
    fail "the prompt card is not outlined in warning with an ink Allow: $(cat "$OUT/direct-ask.pixels")"
for theme in dark light; do
    name="direct-census-$theme"
    direct_run "$name" 'agent=claude,agent-say=please ask first: make the bridge colder' --ui-theme "$theme"
    python3 tools/accent_census.py "$OUT/$name.png" "$OUT/$name.log" --primary 1 >"$OUT/$name.census" 2>&1 ||
        fail "census on the Direct chat, $theme: $(tail -n 2 "$OUT/$name.census" | tr '\n' ' ')"
done
direct_run direct-allowed 'agent=claude,agent-say=please ask first: make the bridge colder,agent-decide=allow'
case "$(line_of dock "$OUT/direct-allowed.log")" in
    *"replies=1"*"(permission: allow)"*) ;;
    *) fail "Allow from the Direct view did not reach the agent" ;;
esac
case "$(direct_of "$OUT/direct-allowed.log")" in
    *"assistant@"*) ;;
    *) fail "the reply after Allow was not drawn" ;;
esac
# The window's own apply, in the chat: the followed treatment rewritten
# mid-run lands as a notice; without a rewrite there is none.
cp "$SHOW" "$OUT/live.treatment.json"
( sleep 3.5
  sed 's/"cadence"/"loom"/' "$OUT/live.treatment.json" >"$OUT/live.treatment.json.tmp" &&
      mv "$OUT/live.treatment.json.tmp" "$OUT/live.treatment.json" ) &
writer=$!
TREATMENT="$OUT/live.treatment.json" FRAMES=420 direct_run direct-applied 'agent=claude,follow=1'
wait "$writer" || true
TREATMENT="$OUT/show.treatment.json" FRAMES=420 direct_run direct-unapplied 'agent=claude,follow=1'
case "$(direct_of "$OUT/direct-applied.log")" in
    *"notice@"*) ;;
    *) fail "the window's treatment apply was not noted in the chat: $(line_of 'treatment follow' "$OUT/direct-applied.log")" ;;
esac
case "$(direct_of "$OUT/direct-unapplied.log")" in
    *"notice@"*) fail "a chat noted an apply that never happened" ;;
esac

if [ "$failures" -gt 0 ]; then
    printf '%d column check(s) failed; logs in %s\n' "$failures" "$OUT"
    exit 1
fi
printf 'column check: all checks passed; evidence in %s\n' "$OUT"
