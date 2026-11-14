#!/usr/bin/env bash
#
# The generation gate (VG H4, 2026-11-14): asking a video model for a clip,
# end to end, against tools/fake_openrouter.py. Never a real call: every
# launch is pointed at the fake on loopback (MUSIALIZER_OPENROUTER_URL), and a
# canary key sits in the private credentials file the application reads.
#
#   tools/generate_check.sh [OUTPUT_DIR]     # default build/generate-check
#
# Run from tools/clip_check.sh. Every launch goes through tools/headless_run.sh.
# GENERATE_BIN names another binary, for a negative control without replacing
# the build. GENERATE_SKIP_CARD=1 leaves out the window card's half.
#
# What it asserts:
#   - the estimate is printed before any request, and without the flag (or with
#     --generate-clips alone) the fake has had no submit at all, and the run
#     exits 1 naming each missing clip and the total;
#   - with the flag the clip is generated, imported and placed, and the saved
#     project carries its provenance; the frame sent first is, byte for byte,
#     the still of the frame before the section;
#   - a second run, and a run from the saved project with no flag, send
#     nothing; a changed prompt is a new request;
#   - the ceiling is honoured mid-run;
#   - each failure shape (a failed job with a bill, a refused submit, a
#     download that is not a video, one too large) is named and nothing is
#     imported or saved;
#   - the canary key is in no output file, log, store file or argv.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-build/generate-check}"
rm -rf "$OUT"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"

export MZ_BIN="${GENERATE_BIN:-$(realpath target/debug/musializer)}"
export MZ_STATE_DIR="$OUT/state"
export MUSIALIZER_GENERATED_DIR="$OUT/store"
export MUSIALIZER_VIDEO_POLL_MS=40
export MUSIALIZER_VIDEO_MAX_BYTES=$((4 * 1024 * 1024))
CANARY="sk-or-v1-musializer-canary-h4-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
export FAKE_EXPECT_KEY="$CANARY"
FAILURES=0

fail() {
    echo "FAIL: $*" >&2
    FAILURES=$((FAILURES + 1))
}
pass() {
    echo "ok:   $*"
}

# The capture display, owned for the whole series (as tools/clip_check.sh).
XVFB_PID=""
FAKE_PID=""
cleanup() {
    if [ -n "$FAKE_PID" ]; then
        kill "$FAKE_PID" 2>/dev/null || true
        wait "$FAKE_PID" 2>/dev/null || true
    fi
    if [ -n "$XVFB_PID" ]; then
        kill "$XVFB_PID" 2>/dev/null || true
        wait "$XVFB_PID" 2>/dev/null || true
    fi
    return 0
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
if [ -n "${MUSIALIZER_CAPTURE_DISPLAY:-}" ]; then
    if [[ ! "$MUSIALIZER_CAPTURE_DISPLAY" =~ ^:([0-9]+)$ ]]; then
        echo 'generate_check: capture display must be :NUMBER' >&2
        exit 2
    fi
    NUMBER="${BASH_REMATCH[1]}"
    for _ in {1..40}; do
        [ -e "/tmp/.X11-unix/X$NUMBER" ] || [ -e "/tmp/.X$NUMBER-lock" ] || break
        sleep 0.05
    done
    if [ ! -e "/tmp/.X11-unix/X$NUMBER" ] && [ ! -e "/tmp/.X$NUMBER-lock" ]; then
        Xvfb "$MUSIALIZER_CAPTURE_DISPLAY" -screen 0 "${MZ_SCREEN:-2560x1440x24}" -nolisten tcp \
            >"$OUT/xvfb.log" 2>&1 &
        XVFB_PID=$!
        for _ in {1..100}; do
            xdpyinfo -display "$MUSIALIZER_CAPTURE_DISPLAY" >/dev/null 2>&1 && break
            sleep 0.05
        done
    fi
fi

# --- the fake provider and the canary key ---------------------------------------
python3 -I tools/fake_openrouter.py "$OUT/fake" >"$OUT/fake.log" 2>&1 &
FAKE_PID=$!
for _ in {1..100}; do
    [ -s "$OUT/fake/port" ] && break
    sleep 0.05
done
[ -s "$OUT/fake/port" ] || { echo "generate_check: the fake provider did not start" >&2; exit 2; }
export MUSIALIZER_OPENROUTER_URL="http://127.0.0.1:$(cat "$OUT/fake/port")/api/v1"
mkdir -p "$MZ_STATE_DIR/config/musializer"
chmod 700 "$MZ_STATE_DIR/config/musializer"
(
    umask 077
    printf '{"schema":"musializer.assist-credentials/v1","entries":{"openrouter/default":{"secret":"%s"}}}\n' \
        "$CANARY" >"$MZ_STATE_DIR/config/musializer/credentials.json"
)

count() {
    python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' \
        "$OUT/fake/counts.json" "$1"
}
# A launch whose status and output go to files.
launch() {
    local name="$1"
    shift
    local status=0
    tools/headless_run.sh "$@" >"$OUT/$name.log" 2>&1 || status=$?
    echo "$status" >"$OUT/$name.status"
}
status_of() { cat "$OUT/$1.status"; }
has() { grep -qF -- "$2" "$OUT/$1.log"; }

# --- fixtures -------------------------------------------------------------------
cargo run --quiet --bin make-fixture-wav -- "$OUT/fixture.wav" 12
treatment() {
    # $1 file, $2 the generated section's prompt, $3 extra segments after it
    cat >"$1" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 24, "quality": "balanced" }, "seed": 7,
  "segments": [ { "at": 0, "scene": "spectrum" },
                { "at": 4, "scene": "clip", "clip": { "generate": {
                    "prompt": "$2", "model": "google/veo-3.1-lite", "seconds": 4,
                    "first": "previous", "last": "next" } } },
                { "at": 8, "scene": "pulse" }$3 ] }
EOF
}
treatment "$OUT/fog.treatment.json" "fog rolls across a field at dawn" ""

# --- 1. no flag: the estimate, the refusal, nothing sent -------------------------
launch bare --treatment "$OUT/fog.treatment.json" --save-project "$OUT/bare.musi"
if [ "$(status_of bare)" = 1 ] && has bare 'missing: 4 s of google/veo-3.1-lite at 720p 16:9, $0.12' \
    && has bare 'about $0.12 in all' && has bare 'Nothing was sent' && [ ! -e "$OUT/bare.musi" ]; then
    pass "without the flag: the estimate (\$0.12) and the total, exit 1, no project"
else
    fail "the run without the flag did not refuse as it should (status $(status_of bare)); see $OUT/bare.log"
fi
[ "$(count submit)" = 0 ] && pass "no submit reached the provider" || fail "a run without the flag submitted"
launch half --treatment "$OUT/fog.treatment.json" --generate-clips --save-project "$OUT/half.musi"
if [ "$(status_of half)" = 1 ] && has half '--generate-clips needs --max-spend USD' && [ "$(count submit)" = 0 ]; then
    pass "--generate-clips without --max-spend is refused before anything"
else
    fail "--generate-clips alone was not refused; see $OUT/half.log"
fi

# --- 2. with the flag: generated, placed, saved with its provenance --------------
launch made --treatment "$OUT/fog.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/made.musi"
if [ "$(status_of made)" = 0 ] && has made 'generated: fog rolls across a field at dawn (Veo 3.1 Lite).mp4' \
    && [ "$(count submit)" = 1 ] && [ -e "$OUT/made.musi" ]; then
    pass "with the flag: one submit, the clip generated and the project saved"
else
    fail "the generating run did not make the clip (status $(status_of made)); see $OUT/made.log"
fi
python3 -I - "$OUT/made.musi" "$OUT/fake/submit-1.json" <<'PY' && pass "the saved project carries the clip, placed in section 2, with its provenance" || fail "the saved project's clip or provenance is wrong"
import json, sys
project = json.load(open(sys.argv[1]))
body = json.load(open(sys.argv[2]))
clips = project["clips"]
assert len(clips) == 1, clips
record = clips[0]["generated"]
assert record["provider"] == "openrouter" and record["model"] == "google/veo-3.1-lite", record
assert record["prompt"] == "fog rolls across a field at dawn" and record["seconds"] == 4, record
assert (record["resolution"], record["aspect_ratio"]) == ("720p", "16:9"), record
assert record["estimate_usd"] == 0.12 and record["cost_usd"] == 0.12, record
assert len(record["first_sha256"]) == 64 and len(record["last_sha256"]) == 64, record
cues = project["scene_switches"]["cues"]
assert cues[1]["scene_name"] == "clip" and cues[1]["clip"]["clip"] == clips[0]["sha256"], cues[1]
assert clips[0]["path"].startswith("made.assets/clips/"), clips[0]["path"]
# What was sent: no audio, the shape, two frames, and nothing naming anyone.
assert body["generate_audio"] is False and body["duration"] == 4, body.keys()
assert [frame["frame_type"] for frame in body["frame_images"]] == ["first_frame", "last_frame"]
assert set(body) == {"model", "prompt", "duration", "resolution", "aspect_ratio", "generate_audio", "frame_images"}, set(body)
PY
# The first frame sent is the still of the frame before the section: frame 95
# at 24 fps (3.958 s), rendered at the request's 1280x720.
launch still --treatment "$OUT/fog.treatment.json" --project "$OUT/made.musi" --resolution 1280x720 \
    --still 3.9792 "$OUT/before.png" --still 8.0208 "$OUT/after.png"
if cmp -s "$OUT/before.png" "$OUT/fake/submit-1-first.png" && cmp -s "$OUT/after.png" "$OUT/fake/submit-1-last.png"; then
    pass "the frames sent are the stills of the frame before the section and the first after it, byte for byte"
else
    fail "the frames sent are not the video's own frames around the section (see $OUT/before.png, $OUT/fake/submit-1-first.png)"
fi
if grep -q 'auth=ok' "$OUT/fake/requests.log" && ! grep -q 'auth=bad' "$OUT/fake/requests.log" \
    && grep -q 'agent=musializer-video/1' "$OUT/fake/requests.log"; then
    pass "the key from the credentials file authorized every call; the client name is generic"
else
    fail "a call went without the key, or named the client otherwise: $OUT/fake/requests.log"
fi

# --- 3. nothing is paid twice ----------------------------------------------------
launch again --treatment "$OUT/fog.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/again.musi"
if [ "$(status_of again)" = 0 ] && has again 'found in the store, nothing sent' && [ "$(count submit)" = 1 ]; then
    pass "a second run finds the clip in the store and sends nothing"
else
    fail "the second run did not reuse the clip; see $OUT/again.log"
fi
mv "$MUSIALIZER_GENERATED_DIR" "$OUT/store-aside"
launch reopen --project "$OUT/made.musi" --treatment "$OUT/fog.treatment.json" --save-project "$OUT/reopen.musi"
if [ "$(status_of reopen)" = 0 ] && has reopen 'found in the project, nothing sent' && [ "$(count submit)" = 1 ]; then
    pass "from the saved project, with no flag and no store, nothing is sent"
else
    fail "reopening the project did not find its clip; see $OUT/reopen.log"
fi
mv "$OUT/store-aside" "$MUSIALIZER_GENERATED_DIR"
treatment "$OUT/fog2.treatment.json" "fog rolls across a field at dusk" ""
launch changed --treatment "$OUT/fog2.treatment.json" --save-project "$OUT/changed.musi"
launch changed-made --treatment "$OUT/fog2.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/changed.musi"
if [ "$(status_of changed)" = 1 ] && has changed 'missing' && [ "$(status_of changed-made)" = 0 ] \
    && [ "$(count submit)" = 2 ]; then
    pass "a changed prompt is a new request: refused without the flag, generated with it"
else
    fail "a changed prompt did not generate again; see $OUT/changed.log, $OUT/changed-made.log"
fi

# --- 4. the ceiling, mid-run -----------------------------------------------------
cat >"$OUT/two.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 24, "quality": "balanced" }, "seed": 7,
  "segments": [ { "at": 0, "scene": "spectrum" },
                { "at": 3, "scene": "clip", "clip": { "generate": {
                    "prompt": "first of two", "model": "google/veo-3.1-lite", "seconds": 4, "first": "previous" } } },
                { "at": 6, "scene": "pulse" },
                { "at": 9, "scene": "clip", "clip": { "generate": {
                    "prompt": "second of two", "model": "google/veo-3.1-lite", "seconds": 4, "first": "previous" } } } ] }
EOF
launch ceiling --treatment "$OUT/two.treatment.json" --generate-clips --max-spend 0.15 \
    --save-project "$OUT/ceiling.musi"
if [ "$(status_of ceiling)" = 1 ] && has ceiling 'segments[1] (0:03.000).clip.generate generated' \
    && has ceiling 'refused: it would pass the ceiling: $0.12 committed this run, $0.12 more, $0.15 allowed' \
    && [ "$(count submit)" = 3 ] && [ ! -e "$OUT/ceiling.musi" ]; then
    pass "the ceiling: the first clip made, the second refused by name, exit 1"
else
    fail "the ceiling was not honoured; see $OUT/ceiling.log"
fi

# --- 5. each failure, named, nothing imported -----------------------------------
for shape in fail-billed refuse-402 not-video too-large; do
    treatment "$OUT/$shape.treatment.json" "a field [$shape]" ""
    before="$(find "$MUSIALIZER_GENERATED_DIR/clips" -name '*.mp4' | wc -l)"
    launch "$shape" --treatment "$OUT/$shape.treatment.json" --generate-clips --max-spend 1.00 \
        --save-project "$OUT/$shape.musi"
    after="$(find "$MUSIALIZER_GENERATED_DIR/clips" -name '*.mp4' | wc -l)"
    case "$shape" in
        fail-billed) reason='the job failed: the provider refused the content. OpenRouter reported $0.05 for it.' ;;
        refuse-402) reason='the OpenRouter account has too little credit (402): Insufficient credits' ;;
        not-video) reason='the download is application/json, not a video' ;;
        too-large) reason='the download is larger than the 4194304 bytes a clip may be' ;;
    esac
    if [ "$(status_of "$shape")" = 1 ] && has "$shape" "$reason" && [ "$before" = "$after" ] \
        && [ ! -e "$OUT/$shape.musi" ]; then
        pass "$shape: named (\"$reason\"), nothing imported, nothing saved"
    else
        fail "$shape was not refused by name, or something was kept; see $OUT/$shape.log"
    fi
done
if ls "$MUSIALIZER_GENERATED_DIR/work" 2>/dev/null | grep -q .; then
    fail "a job left scratch behind: $(ls "$MUSIALIZER_GENERATED_DIR/work")"
else
    pass "no job left scratch behind"
fi

# --- 6. the key is nowhere it should not be --------------------------------------
# A slow job, so its curl can be caught running and its argv read.
treatment "$OUT/slow.treatment.json" "a long take [slow]" ""
tools/headless_run.sh --treatment "$OUT/slow.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/slow.musi" >"$OUT/slow.log" 2>&1 &
SLOW_PID=$!
seen_curl=0
for _ in {1..600}; do
    # Every curl on the machine and every musializer: their argv, read as
    # the process table shows it to anyone on this account.
    while read -r pid; do
        line="$(tr '\0' ' ' 2>/dev/null <"/proc/$pid/cmdline" || true)"
        case "$line" in
            *curl*--config*) seen_curl=1 ;;
        esac
        case "$line" in
            *"$CANARY"*) fail "the key is in the argv of process $pid" ;;
        esac
    done < <(pgrep -x curl; pgrep -f "$MZ_BIN")
    [ "$seen_curl" = 1 ] && [ "$(count poll)" -gt 40 ] && break
    sleep 0.02
done
touch "$OUT/fake/release"
wait "$SLOW_PID" || true
[ "$seen_curl" = 1 ] && pass "curl was seen running (--config -), and no argv held the key" \
    || fail "no curl was seen while the slow job ran"
if grep -rlaF "$CANARY" "$OUT" --exclude=credentials.json 2>/dev/null | grep -v '^$' >"$OUT/canary-hits.txt"; then
    fail "the key is in: $(tr '\n' ' ' <"$OUT/canary-hits.txt")"
else
    pass "the key is in no log, project, store file or capture"
fi

echo
if [ "$FAILURES" -gt 0 ]; then
    echo "generate_check: $FAILURES check(s) failed; artifacts in $OUT" >&2
    exit 1
fi
echo "generate_check: all checks passed; artifacts in $OUT"
