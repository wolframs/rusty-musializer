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
# GENERATE_ONLY="NAME ..." runs only those groups (base, concurrent, lost,
# redownload, order, fps, stale, card, canary), for a negative control.
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
#   - the section card: every control pressed with a press beside it, Send,
#     Cancel, no key, a followed treatment, the flyout, captures (see
#     tools/generate_card_check.py);
#   - the canary key is in no output file, log, store file or argv;
#   - lost (H4b): a submit the provider accepts and never answers is "may have
#     been billed", never "nothing"; the next run sends nothing and says so,
#     and --resend-unconfirmed sends it again only when the ceiling holds both
#     estimates;
#   - redownload (H4b): a finished job whose downloads come back cut short is
#     kept, and the next run downloads it again without a new submit; an
#     expired job is named and forgotten;
#   - order (H4b): a frame inside the blend from a generated section this run
#     could not make (a stand-in) is never sent: the request is refused by
#     name and the provider gets no submit for it;
#   - fps (H4b): under --fps 12 --quality high (supersampled) the frames sent
#     for a 24 fps balanced treatment are,
#     byte for byte, the stills at the export's own frames around the cuts
#     (47/12 s and 96/12 s);
#   - concurrent (H4b): two runs asking for one request while the first one's
#     submit is in flight pay once (the provider's submit count), a run with
#     --max-spend 0 sends nothing, and a job found in progress counts against
#     a ceiling at its estimate;
#   - stale (H4c): two runs poll one recorded job and learn it expired at
#     different times; between the two answers a third run submits a new job
#     for the request. The late poller leaves the new job's record alone, and
#     a fourth run picks that job up: two submits in all, not three;

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
only() {
    [ -z "${GENERATE_ONLY:-}" ] || [[ " $GENERATE_ONLY " == *" $1 "* ]]
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

if only base; then
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
fi # base

# Runs at the same time each get a state folder of their own (with the same
# key): headless_run.sh links its tools into it and two at once would race.
own_state() {
    mkdir -p "$OUT/state-$1"
    cp -a "$MZ_STATE_DIR/config" "$OUT/state-$1/"
    echo "$OUT/state-$1"
}
# Waits up to $2 tenths of a second for the command in $1 to succeed.
wait_for() {
    local tries="$2"
    while [ "$tries" -gt 0 ]; do
        eval "$1" && return 0
        sleep 0.1
        tries=$((tries - 1))
    done
    return 1
}

# --- 8. two callers for one request (H4b) -------------------------------------------
# The first run's submit takes 8 s at the fake; two more runs start while it is
# in flight. The lock in the store makes the second wait and pick the job up;
# --max-spend 0 sends nothing. Then, with the job recorded and held in
# progress, a ceiling below its estimate refuses it: it counts at $0.12, not 0.
if only concurrent; then
treatment "$OUT/twice.treatment.json" "two callers at once [slow-submit] [hold]" ""
submits="$(count submit)"
MZ_STATE_DIR="$(own_state a)" tools/headless_run.sh --treatment "$OUT/twice.treatment.json" \
    --generate-clips --max-spend 1.00 --save-project "$OUT/twice-a.musi" >"$OUT/twice-a.log" 2>&1 &
TWICE_A=$!
for _ in {1..600}; do
    [ "$(count submit)" -gt "$submits" ] && break
    sleep 0.05
done
MZ_STATE_DIR="$(own_state b)" tools/headless_run.sh --treatment "$OUT/twice.treatment.json" \
    --generate-clips --max-spend 1.00 --save-project "$OUT/twice-b.musi" >"$OUT/twice-b.log" 2>&1 &
TWICE_B=$!
MZ_STATE_DIR="$(own_state z)" tools/headless_run.sh --treatment "$OUT/twice.treatment.json" \
    --generate-clips --max-spend 0 --save-project "$OUT/twice-z.musi" >"$OUT/twice-z.log" 2>&1 &
TWICE_Z=$!
# The first run's job is recorded once it polls.
polls="$(count poll)"
for _ in {1..600}; do
    [ "$(count poll)" -gt "$polls" ] && break
    sleep 0.05
done
MZ_STATE_DIR="$(own_state w)" launch twice-w --treatment "$OUT/twice.treatment.json" --generate-clips --max-spend 0.10 \
    --save-project "$OUT/twice-w.musi" &
TWICE_W=$!
for _ in {1..400}; do
    [ -e "$OUT/twice-w.status" ] && break
    sleep 0.05
done
touch "$OUT/fake/release-hold"
status_a=0; wait "$TWICE_A" || status_a=$?
status_b=0; wait "$TWICE_B" || status_b=$?
status_z=0; wait "$TWICE_Z" || status_z=$?
wait "$TWICE_W" || true
rm -f "$OUT/fake/release-hold"
if [ "$(count submit)" = $((submits + 1)) ]; then
    pass "two runs for one request while its submit was in flight: the provider got one submit"
else
    fail "two runs for one request paid $(( $(count submit) - submits )) times; see $OUT/twice-a.log, $OUT/twice-b.log"
fi
if [ "$status_a" = 0 ] && grep -qF 'clip.generate generated' "$OUT/twice-a.log" \
    && [ "$status_b" = 0 ] && grep -qF 'clip.generate picked up' "$OUT/twice-b.log" \
    && grep -qF 'being sent by another window or run' "$OUT/twice-b.log"; then
    pass "the first run generated it; the second waited, said so, and picked the same job up"
else
    fail "the two runs did not share the job (status $status_a, $status_b); see $OUT/twice-a.log, $OUT/twice-b.log"
fi
if [ "$status_z" = 1 ] && grep -qF 'refused: --max-spend 0 sends nothing' "$OUT/twice-z.log"; then
    pass "--max-spend 0 sends nothing, even for a request another run is sending"
else
    fail "--max-spend 0 did not refuse (status $status_z); see $OUT/twice-z.log"
fi
if [ "$(status_of twice-w)" = 1 ] \
    && has twice-w 'refused: it would pass the ceiling: $0.00 committed this run, $0.12 more, $0.10 allowed'; then
    pass "a job found in progress counts against a \$0.10 ceiling at its \$0.12 estimate"
else
    fail "a job in progress was not counted at its estimate (status $(status_of twice-w)); see $OUT/twice-w.log"
fi
fi # concurrent

# --- 9. an accepted submit whose answer never arrives (H4b) ----------------------
if only lost; then
treatment "$OUT/lost.treatment.json" "a lost answer [lost-answer-once]" ""
submits="$(count submit)"
launch lost-1 --treatment "$OUT/lost.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/lost-1.musi"
if [ "$(status_of lost-1)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] \
    && has lost-1 'The request was sent and its answer never arrived, so it may have been billed (about $0.12)' \
    && ! has lost-1 'Nothing was sent'; then
    pass "a submit accepted and never answered: one submit, \"may have been billed (about \$0.12)\", never \"nothing\""
else
    fail "the lost answer was not reported as possibly billed (status $(status_of lost-1)); see $OUT/lost-1.log"
fi
launch lost-2 --treatment "$OUT/lost.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/lost-2.musi"
if [ "$(status_of lost-2)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] \
    && has lost-2 'and its answer never arrived, so OpenRouter may have accepted and billed it (about $0.12)' \
    && has lost-2 'add --resend-unconfirmed'; then
    pass "the next run sends nothing, says the first may have been billed, and names --resend-unconfirmed"
else
    fail "the next run did not hold the request (status $(status_of lost-2), $(count submit) submits); see $OUT/lost-2.log"
fi
launch lost-3 --treatment "$OUT/lost.treatment.json" --generate-clips --max-spend 0.20 --resend-unconfirmed \
    --save-project "$OUT/lost-3.musi"
if [ "$(status_of lost-3)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] \
    && has lost-3 'refused: it would pass the ceiling: $0.12 committed this run, $0.12 more, $0.20 allowed'; then
    pass "sending it again counts both estimates: refused under a \$0.20 ceiling"
else
    fail "--resend-unconfirmed did not count the earlier send (status $(status_of lost-3)); see $OUT/lost-3.log"
fi
launch lost-4 --treatment "$OUT/lost.treatment.json" --generate-clips --max-spend 0.30 --resend-unconfirmed \
    --save-project "$OUT/lost-4.musi"
if [ "$(status_of lost-4)" = 0 ] && [ "$(count submit)" = $((submits + 2)) ] \
    && has lost-4 'clip.generate generated' && ! ls "$MUSIALIZER_GENERATED_DIR/intents/"*.json >/dev/null 2>&1; then
    pass "with --resend-unconfirmed and room for both, it is sent once more and generated; no intent is left"
else
    fail "the agreed resend did not generate (status $(status_of lost-4)); see $OUT/lost-4.log"
fi
launch lost-flag --treatment "$OUT/lost.treatment.json" --resend-unconfirmed --save-project "$OUT/lost-flag.musi"
if [ "$(status_of lost-flag)" = 1 ] && has lost-flag '--resend-unconfirmed sends again only with --generate-clips'; then
    pass "--resend-unconfirmed alone is refused"
else
    fail "--resend-unconfirmed alone was not refused; see $OUT/lost-flag.log"
fi
# H4c: a resend whose answer is lost too. Both sends stay on record and count:
# $0.24 possibly billed, so a $0.24 ceiling admits no third ($0.36), and the
# words name both sends and their sum.
treatment "$OUT/lost2.treatment.json" "lost twice [lost-answers-2]" ""
submits="$(count submit)"
launch lost2-1 --treatment "$OUT/lost2.treatment.json" --generate-clips --max-spend 1.00 --save-project "$OUT/lost2-1.musi"
launch lost2-2 --treatment "$OUT/lost2.treatment.json" --generate-clips --max-spend 1.00 --resend-unconfirmed \
    --save-project "$OUT/lost2-2.musi"
if [ "$(status_of lost2-1)" = 1 ] && [ "$(status_of lost2-2)" = 1 ] && [ "$(count submit)" = $((submits + 2)) ] \
    && has lost2-2 'The request was sent and its answer never arrived, so it may have been billed (about $0.12)'; then
    pass "a resend agreed after a lost answer is lost too: two submits, each possibly billed"
else
    fail "the second lost answer was not reported (status $(status_of lost2-1), $(status_of lost2-2), $(( $(count submit) - submits )) submits); see $OUT/lost2-2.log"
fi
launch lost2-3 --treatment "$OUT/lost2.treatment.json" --generate-clips --max-spend 0.24 --resend-unconfirmed \
    --save-project "$OUT/lost2-3.musi"
if [ "$(status_of lost2-3)" = 1 ] && [ "$(count submit)" = $((submits + 2)) ] \
    && has lost2-3 'This exact request was sent 2 times' && has lost2-3 'may have accepted and billed each of them (about $0.24 in all)' \
    && has lost2-3 'spending.tsv' && has lost2-3 '$0.24 committed'; then
    pass "both lost sends count: a \$0.24 ceiling admits no third (\$0.36); the words name 2 sends, \$0.24 in all, and the ledger"
else
    fail "the earlier lost sends were not both counted (status $(status_of lost2-3), $(( $(count submit) - submits )) submits); see $OUT/lost2-3.log"
fi
launch lost2-4 --treatment "$OUT/lost2.treatment.json" --generate-clips --max-spend 0.36 --resend-unconfirmed \
    --save-project "$OUT/lost2-4.musi"
digest2="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["clips"][0]["generated"]["request_sha256"])' "$OUT/lost2-4.musi" 2>/dev/null || true)"
if [ "$(status_of lost2-4)" = 0 ] && [ "$(count submit)" = $((submits + 3)) ] && [ -n "$digest2" ] \
    && [ ! -e "$MUSIALIZER_GENERATED_DIR/intents/$digest2.json" ] \
    && [ "$(grep -aF "$digest2" "$MUSIALIZER_GENERATED_DIR/spending.tsv" | grep -cF 'answer lost, possibly billed')" = 2 ] \
    && [ "$(grep -aF "$digest2" "$MUSIALIZER_GENERATED_DIR/spending.tsv" | grep -cF 'sending, billed if accepted')" = 3 ]; then
    pass "with room for all three it is sent and made; the intent is cleared and the ledger keeps both lost sends"
else
    fail "the third send did not generate, or the record is wrong (status $(status_of lost2-4)); see $OUT/lost2-4.log and the store's spending.tsv"
fi
fi # lost

# --- 10. a finished clip that arrives cut short (H4b) ----------------------------
if only redownload; then
treatment "$OUT/cut.treatment.json" "a clip cut short [truncate-3]" ""
submits="$(count submit)"
contents="$(count content)"
launch cut-1 --treatment "$OUT/cut.treatment.json" --generate-clips --max-spend 1.00 --save-project "$OUT/cut-1.musi"
if [ "$(status_of cut-1)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] && [ "$(count content)" = $((contents + 3)) ] \
    && has cut-1 'the finished clip was downloaded 3 times and refused each time' \
    && has cut-1 'its job is kept, so Generate again downloads it again without paying again' \
    && ls "$MUSIALIZER_GENERATED_DIR/jobs/"*.json >/dev/null 2>&1; then
    pass "three cut-short downloads: named, bounded at 3, the paid job kept"
else
    fail "the cut-short clip was not retried and kept (status $(status_of cut-1)); see $OUT/cut-1.log"
fi
launch cut-2 --treatment "$OUT/cut.treatment.json" --generate-clips --max-spend 1.00 --save-project "$OUT/cut-2.musi"
if [ "$(status_of cut-2)" = 0 ] && [ "$(count submit)" = $((submits + 1)) ] && has cut-2 'clip.generate picked up'; then
    pass "the next run downloads the same job whole: no new submit"
else
    fail "the next run paid again or failed (status $(status_of cut-2), $(( $(count submit) - submits )) submits); see $OUT/cut-2.log"
fi
treatment "$OUT/expire.treatment.json" "a clip that expires [expire]" ""
# Other groups leave paid jobs whose downloads were refused (kept on purpose):
# the expired one must add none.
jobs_before="$(find "$MUSIALIZER_GENERATED_DIR/jobs" -name '*.json' 2>/dev/null | wc -l)"
launch expire --treatment "$OUT/expire.treatment.json" --generate-clips --max-spend 1.00 --save-project "$OUT/expire.musi"
jobs_after="$(find "$MUSIALIZER_GENERATED_DIR/jobs" -name '*.json' 2>/dev/null | wc -l)"
if [ "$(status_of expire)" = 1 ] && has expire 'the job expired at OpenRouter before its clip was downloaded; Generate again asks (and pays) anew' \
    && [ "$jobs_after" = "$jobs_before" ]; then
    pass "an expired job is named, with what asking again costs, and forgotten"
else
    fail "the expired job was not named or not forgotten; see $OUT/expire.log"
fi
fi # redownload

# --- 11. no stand-in in a frame sent (H4b) ---------------------------------------
# Section 1 is generated and refused (402), so it stays a stand-in. Section 2
# is two seconds long and fades in over all of them, so its last frame still
# shows section 1; section 3 starts on that frame and must not be sent.
if only order; then
cat >"$OUT/blend.treatment.json" <<EOF
{ "schema": "musializer.treatment/v1", "audio": { "path": "fixture.wav" },
  "output": { "resolution": "640x360", "fps": 24, "quality": "balanced" }, "seed": 7,
  "segments": [ { "at": 0, "scene": "spectrum" },
                { "at": 2, "scene": "clip", "clip": { "generate": {
                    "prompt": "refused first [refuse-402]", "model": "google/veo-3.1-lite", "seconds": 4 } } },
                { "at": 4, "scene": "pulse", "transition": { "kind": "fade", "seconds": 3 } },
                { "at": 6, "scene": "clip", "clip": { "generate": {
                    "prompt": "after the blend", "model": "google/veo-3.1-lite", "seconds": 4, "first": "previous" } } },
                { "at": 9, "scene": "spectrum" } ] }
EOF
submits="$(count submit)"
launch blend --treatment "$OUT/blend.treatment.json" --generate-clips --max-spend 1.00 --save-project "$OUT/blend.musi"
if [ "$(status_of blend)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] \
    && has blend 'segments[3] (0:06.000).clip.generate refused: its frame at 0:05.979 lies inside the transition at 0:04.000, which shows segments[1] (0:02.000).clip.generate, a generated section not made yet; the frame would be a stand-in, so nothing was sent for it'; then
    pass "a frame that would show a stand-in is refused by name; the provider got only the refused section's submit"
else
    fail "a stand-in frame was sent or not named (status $(status_of blend), $(( $(count submit) - submits )) submits); see $OUT/blend.log"
fi
fi # order

# --- 12. --fps governs the frames sent (H4b) -------------------------------------
if only fps; then
treatment "$OUT/fps.treatment.json" "fog at another frame rate" ""
submits="$(count submit)"
launch fps --treatment "$OUT/fps.treatment.json" --fps 12 --quality high --generate-clips --max-spend 1.00 \
    --save-project "$OUT/fps.musi"
number=$((submits + 1))
# At 12 fps the last frame before 4 s is frame 47 (3.9583 s) and the first at
# 8 s is frame 96; at the treatment's 24 they would be 95 and 192.
launch fps-still --treatment "$OUT/fps.treatment.json" --project "$OUT/fps.musi" --resolution 1280x720 --fps 12 --quality high \
    --still 3.9583 "$OUT/fps-before.png" --still 8.0417 "$OUT/fps-after.png"
if [ "$(status_of fps)" = 0 ] && [ "$(count submit)" = "$number" ] \
    && cmp -s "$OUT/fps-before.png" "$OUT/fake/submit-$number-first.png" \
    && cmp -s "$OUT/fps-after.png" "$OUT/fake/submit-$number-last.png"; then
    pass "under --fps 12 --quality high the frames sent are the export's own frames at the cuts, byte for byte"
else
    fail "the frames sent under --fps 12 --quality high are not the export's (status $(status_of fps)); see $OUT/fps-before.png, $OUT/fake/submit-$number-first.png"
fi
fi # fps

# --- 13. a late poller leaves a newer job alone (H4c) ----------------------------
# Run X submits job 1 and gives up waiting (a 1 s deadline), leaving its record.
# Runs A and B pick it up; the fake answers the first poll after release-stale
# "expired" and holds the other one's answer. The first forgets job 1; run C
# then submits job 2 and records it; then the held poller hears "expired" too.
# It must not remove job 2's record (it is not the job it polled), or run D
# would pay a third time without a word.
if only stale; then
treatment "$OUT/stale.treatment.json" "a job polled twice [stale-poller]" ""
submits="$(count submit)"
job_one="gen-vid-$((1789480000 + submits + 1))-AAAAAAAAAAAAAAAAAAA$(( (submits + 1) % 10 ))"
job_two="gen-vid-$((1789480000 + submits + 2))-AAAAAAAAAAAAAAAAAAA$(( (submits + 2) % 10 ))"
recorded() { grep -qF "\"$1\"" "$MUSIALIZER_GENERATED_DIR/jobs/"*.json 2>/dev/null; }
MUSIALIZER_VIDEO_DEADLINE_S=1 launch stale-x --treatment "$OUT/stale.treatment.json" --generate-clips --max-spend 1.00 \
    --save-project "$OUT/stale-x.musi"
if [ "$(status_of stale-x)" = 1 ] && [ "$(count submit)" = $((submits + 1)) ] && recorded "$job_one"; then
    pass "a run that stops waiting keeps its job's record (job 1)"
else
    fail "the first stale run did not leave job 1 recorded (status $(status_of stale-x)); see $OUT/stale-x.log"
fi
for name in a b; do
    MZ_STATE_DIR="$(own_state "stale-$name")" launch "stale-$name" --treatment "$OUT/stale.treatment.json" \
        --generate-clips --max-spend 1.00 --save-project "$OUT/stale-$name.musi" &
done
wait_for 'has stale-a "the model is making it" 2>/dev/null && has stale-b "the model is making it" 2>/dev/null' 900 \
    || fail "the two pollers of job 1 did not both start polling; see $OUT/stale-a.log, $OUT/stale-b.log"
touch "$OUT/fake/release-stale"
wait_for '[ -e "$OUT/stale-a.status" ] || [ -e "$OUT/stale-b.status" ]' 300 || fail "neither poller heard the expiry"
recorded "$job_one" && fail "the first to hear the expiry did not forget job 1"
MZ_STATE_DIR="$(own_state stale-c)" launch stale-c --treatment "$OUT/stale.treatment.json" \
    --generate-clips --max-spend 1.00 --save-project "$OUT/stale-c.musi" &
STALE_C=$!
wait_for '[ "$(count submit)" = $((submits + 2)) ] && recorded "$job_two" && has stale-c "the model is making it" 2>/dev/null' 900 \
    || fail "run C did not submit and record job 2; see $OUT/stale-c.log"
touch "$OUT/fake/release-stale-2"
wait_for '[ -e "$OUT/stale-a.status" ] && [ -e "$OUT/stale-b.status" ]' 300 || fail "the late poller never ended"
if [ "$(status_of stale-a)" = 1 ] && [ "$(status_of stale-b)" = 1 ] \
    && has stale-a 'the job expired at OpenRouter' && has stale-b 'the job expired at OpenRouter' \
    && recorded "$job_two"; then
    pass "both pollers of job 1 heard it expire, and the late one left job 2's record alone"
else
    fail "the late poller removed job 2's record, or a poller did not end on the expiry (status $(status_of stale-a), $(status_of stale-b)); see $OUT/stale-a.log, $OUT/stale-b.log"
fi
MZ_STATE_DIR="$(own_state stale-d)" launch stale-d --treatment "$OUT/stale.treatment.json" \
    --generate-clips --max-spend 1.00 --save-project "$OUT/stale-d.musi" &
STALE_D=$!
wait_for 'has stale-d "the model is making it" 2>/dev/null || [ "$(count submit)" -gt $((submits + 2)) ]' 900 || true
touch "$OUT/fake/release-stale-3"
wait "$STALE_C" || true
wait "$STALE_D" || true
if [ "$(count submit)" = $((submits + 2)) ] && [ "$(status_of stale-c)" = 0 ] && has stale-c 'clip.generate generated' \
    && [ "$(status_of stale-d)" = 0 ] && has stale-d 'clip.generate picked up'; then
    pass "run D picked job 2 up: two submits for the request, not three"
else
    fail "run D paid again or failed ($(( $(count submit) - submits )) submits, status $(status_of stale-c), $(status_of stale-d)); see $OUT/stale-d.log"
fi
fi # stale

# --- 6. the section card's Generate, pressed (tools/generate_card_check.py) -------
if [ "${GENERATE_SKIP_CARD:-}" != 1 ] && only card; then
    python3 -I tools/generate_card_check.py "$OUT" || FAILURES=$((FAILURES + 1))
fi

# --- 7. the key is nowhere it should not be --------------------------------------
if only canary; then
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
fi # canary

echo
if [ "$FAILURES" -gt 0 ]; then
    echo "generate_check: $FAILURES check(s) failed; artifacts in $OUT" >&2
    exit 1
fi
echo "generate_check: all checks passed; artifacts in $OUT"
