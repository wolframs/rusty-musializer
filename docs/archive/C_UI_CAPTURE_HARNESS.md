# C UI capture harness (archived 2026-11-17)

Historical source inventory; removed tools are in the legacy C tree, not this repository.

## 10. Headless capture harness (historical; see `tools/headless_check.sh`)

**This application's gate is `tools/headless_check.sh`, and this section does
not describe it.** What follows is the C tree's two-script harness
(`tools/ui_fixture.sh`, `tools/ui_capture.sh`, `tools/ui_states.txt`), kept for
one reason: `fixtures/README.md` cites 10.1 as the provenance of the synthetic
capture fixture. Nothing here is a current instruction.

Two differences matter enough to state before anyone reads on:

- **`WAYLAND_DISPLAY` must be set to a name that cannot resolve, never unset.**
  10.4 below says "unset", and that is wrong: `wl_display_connect(NULL)` falls
  back to a hardcoded `"wayland-0"`, which is this operator's real socket, so
  unsetting the variable is *no isolation at all*. `AGENTS.md` records how that
  was proven and what it cost. Use `MZ_NO_WAYLAND`.
- **The gate renders on the GPU.** `tools/headless_check.sh` launches through
  VirtualGL (`vglrun -d egl0`) whenever it is installed — the normal path here,
  and around six times faster than Mesa's `llvmpipe`. It falls back to the CPU,
  prints which path it took, and `MZ_GL_LAUNCH=""` forces the CPU path so a
  capture check can be calibrated against both rasterizers. The fixture audio
  is `cargo run --bin make-fixture-wav`, not the `ffmpeg aevalsrc` recipe below.

`tools/UI_REVIEW.md:52-53` documents the C's conventional invocation:
`tools/ui_fixture.sh` then `tools/ui_capture.sh build/ui-review/shots`, with a
blessed reference set kept in `build/ui-review/reference/`
(`tools/UI_REVIEW.md:56`).

### 10.1 `tools/ui_fixture.sh` — build the fixture

`set -eu` at `:11`. `OUT=${1:-build/ui-review}` at `:13`.

Preflight (`:17-19`): `[ -x ./build/musializer ]` else "missing $APP - run
./nob build debug"; `command -v ffmpeg`; `command -v Xvfb`.

1. **Synthesizes a 40 s stereo WAV, only if absent** (`:23-29`):

   ```
   ffmpeg -loglevel error -y -f lavfi \
     -i "aevalsrc='0.45*sin(2*PI*100*t)*exp(-9*mod(t,0.5))
                 + 0.22*sin(2*PI*(300+180*sin(2*PI*0.08*t))*t)
                 + 0.12*sin(2*PI*1500*t)*exp(-30*mod(t,0.25))':d=40:s=44100:c=stereo" \
     -c:a pcm_s16le "$OUT/demo.wav"
   ```

   A 100 Hz decaying pulse, a vibrato'd ~300 Hz partial, and a 1500 Hz
   hi-hat-like transient.
2. An inline `python3 - "$OUT" <<'PY'` heredoc (`:31-61`) writes
   `demo.bridge.tsv`: header `MUSIALIZER_BRIDGE\t1`, then
   `AUDIO\t<sha256 of demo.wav>\t40000`, **8 base64-encoded `LYRIC` rows**
   (1500–36500 ms, deliberately mixed lengths, `:40-49`) and **3 `SECTION`
   rows** (spectrum 0–13000, loom 13000–26000, cadence 26000–40000, `:50-54`).
3. Builds the project **with the real application** rather than hand-writing
   `.musi` (`:63-72`):

   ```
   Xvfb ":$DISPLAY_NUM" -screen 0 1280x720x24 -nolisten tcp >"$OUT/.xvfb.log" 2>&1 &
   xvfb_pid=$!
   sleep 2
   DISPLAY=":$DISPLAY_NUM" PULSE_SERVER=/nonexistent/musializer-capture \
       "$APP" --mute "$OUT/demo.wav" \
       --analysis-bridge "$OUT/demo.bridge.tsv" \
       --save-project "$OUT/demo.musi" >"$OUT/fixture.log" 2>&1 || true
   ```

   then kills and waits on Xvfb (`:73-74`).
4. Postconditions (`:76-78`): `demo.musi` must exist, and
   `grep -c 'applied 8 lyrics, 3 scene sections' "$OUT/fixture.log"` must
   succeed.
5. Writes `reference.lyrics.txt` (3 authored lines, `:81-85`) and `fonts.tsv`
   (a `musializer.font-catalogue/v1` header plus 8 real families with script
   coverage, `:91-101`) so **no capture run ever contacts Google Fonts**.
6. Artifacts: `build/ui-review/{demo.wav, demo.bridge.tsv, demo.musi,
   reference.lyrics.txt, fonts.tsv, fixture.log, .xvfb.log}`.

### 10.2 `tools/ui_capture.sh` — photograph the states

`set -u` at `:19` — note **not** `-e`, deliberately, so one failing state does
not abort the run. Usage: `tools/ui_capture.sh OUTDIR [CATALOGUE]`, catalogue
defaulting to `tools/ui_states.txt` (`:23`).

Preflight (`:31-41`): OUTDIR required (exit **2**); `[ -x ./build/musializer ]`;
catalogue file exists; `command -v Xvfb`; `command -v ffmpeg`; and the fixture
master `$FIXTURE_DIR/demo.musi` must exist else "run tools/ui_fixture.sh".

Session isolation (`:56-61`): `unset WAYLAND_DISPLAY`; then
`PULSE_SERVER=/nonexistent/musializer-capture; export PULSE_SERVER`.

Per state — the loop is `while IFS='|' read -r name size args` (`:64`), with
comments and blanks skipped (`:65`) and `width=${size%x*}` /
`height=${size#*x}` (`:66-67`):

1. **A throwaway fixture copy.** `rm -rf "$WORKDIR"; mkdir -p "$WORKDIR"; cp
   "$MASTER" "$PROJECT"`, plus `cp -r "$FIXTURE_DIR/demo.assets"
   "$WORKDIR/demo.assets"` when present (`:75-80`). `WORKDIR="$OUTDIR/.fixture"`
   and `PROJECT="$WORKDIR/demo.musi"` (`:52-53`); the filename is preserved
   because the asset bundle is addressed relative to the project stem
   (`:72-74`). The literal token `PROJECT` in the catalogue args is substituted
   by `sed "s#PROJECT#$PROJECT#g"` (`:70`).
2. **One Xvfb per state**, geometry taken from the catalogue line:

   ```
   Xvfb ":$DISPLAY_NUM" -screen 0 "${width}x${height}x24" -nolisten tcp \
       >"$OUTDIR/.xvfb.log" 2>&1 &
   xvfb_pid=$!
   sleep 2
   ```

   Display number defaults to **77** (`MUSIALIZER_CAPTURE_DISPLAY`), depth 24.
3. Launch: `DISPLAY=":$DISPLAY_NUM" $APP --mute $resolved >"$log" 2>&1 &`
   (`:88`, with a deliberate `# shellcheck disable=SC2086` for word splitting).
4. **The wait is a blind `sleep "$SETTLE"`** (`:90`). There is **no polling of
   the window** and **no `xdotool`, `xwininfo`, or `wmctrl` anywhere in the
   repository**. `SETTLE=${MUSIALIZER_CAPTURE_SETTLE:-6}` (`:29`); the comment
   at `:26-28` notes that with `play=1` the captured playhead is roughly
   `time + SETTLE`.
5. **The frame grab is ffmpeg x11grab** — not ImageMagick `import`, not `xwd` —
   guarded by a liveness check (`:92-94`):

   ```
   if kill -0 "$app_pid" 2>/dev/null; then
       ffmpeg -loglevel error -y -f x11grab -video_size "${width}x${height}" \
           -i ":$DISPLAY_NUM.0" -frames:v 1 "$out" >>"$log" 2>&1
   ```

   If the application already exited it `wait`s, prints
   `FAIL $name (application exited early; see $log)`, and sets `status=1`
   (`:95-101`).
6. Teardown: `kill "$app_pid"` → `sleep 1` → `kill` / `wait` Xvfb (`:103-106`).
7. Verification: the PNG must exist else `FAIL $name (no capture)`
   (`:116-119`). The script also sha256-compares the per-state copy against the
   master digest captured once at `:54`
   (`master_before=$(sha256sum "$MASTER" | cut -d' ' -f1)`) and, if a state
   wrote the project, prints
   `ok   $name -> $out (WROTE the project; captures may drift)` (`:108-115`).

After the loop (`:122-126`): `rm -rf "$WORKDIR"`, and if the **master** digest
changed, `FAIL the master fixture changed during the run; captures are not
comparable` with `status=1`. Exits `status` (`:128`).

Artifacts: `$OUTDIR/<name>.png` (`:68`), `$OUTDIR/<name>.log` (application
stdout/stderr plus ffmpeg errors, `:69`), a shared `$OUTDIR/.xvfb.log`
overwritten each state (`:83`), and the transient `$OUTDIR/.fixture/`.
`<name>` is the first catalogue field verbatim.

### 10.3 `tools/ui_states.txt`

Format documented at `:1-4`: `name|WIDTHxHEIGHT|args`, `#` comments and blank
lines ignored, the token `PROJECT` substituted with the throwaway copy's path.
**41 states.** Sizes: 20 × `1280x720`, 11 × `960x640` (the
`SetWindowMinSize` floor), 5 × `1920x1080`, 5 × `1600x900`. Groups: entry and
landing (`:13-15`), default workspace (`:18-20`), expanded panels including
lyrics / tune / export / assist plus the cue-editing form and the armed Assist
prompt (`:23-46`), fullscreen stage (`:49-50`), a ten-scene framing sweep with
`play=1` (`:53-63`), timeline zoom 8× and 40× (`:67-68`), caption-style states
(`:69-70`), and the font browser using `fonts=consent` and
`fonts=build/ui-review/fonts.tsv` so nothing reaches the network (`:75-77`).
The application-side surface is `--ui-probe` (keys used: `size`, `panel`,
`time`, `play`, `fullscreen`, `lyric`, `assist`, `lyrics-file`, `zoom`,
`style`, `fonts`) plus `--project`, `--scene`, and `--mute`.

### 10.4 What a capture harness has to hold, whatever it is built from

The invariants below are why the C's harness worked, and `headless_check.sh`
keeps all of them. **One of them is stated wrongly here on purpose, so the
correction is not lost:** the C unset `WAYLAND_DISPLAY`, and that is not weaker
isolation, it is none — see the warning at the head of section 10.

- One display per state, at depth 24, sized to the state.
- `WAYLAND_DISPLAY` **set to a name that cannot resolve** (`MZ_NO_WAYLAND`),
  never unset. The C's `unset WAYLAND_DISPLAY` at `tools/ui_capture.sh:56` is
  the defect this line exists to flag.
- `PULSE_SERVER` pointed at a nonexistent path, and `--mute` on every launch —
  two independent guarantees that no test audio reaches a real sink.
- `PATH` stripped of `kdialog` and `zenity` for any probe that presses a control
  which could open a picker. This guard is per-call-site and is the one that was
  actually holding while the Wayland guard was dead for months.
- The project copied per state, and the master fixture's digest verified before
  and after the whole run, so a state that writes cannot silently invalidate the
  ones after it.
- The window parked at `(0, 0)`, so a capture needs no guesswork about
  compositor placement.
- A frame grab that does not need a window manager: the C used
  `ffmpeg -f x11grab -video_size WxH -i :NN.0 -frames:v 1 out.png`, needing only
  `Xvfb` and `ffmpeg` — no ImageMagick `import`, no `xdotool`.

The C's one genuinely fragile part was a blind `sleep` before the grab
(`MUSIALIZER_CAPTURE_SETTLE`, default 6 s, which also drifted the captured
playhead to `time + SETTLE`). A harness should wait on a readiness line from the
application instead.

---

