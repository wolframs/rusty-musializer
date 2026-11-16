# Repository guide for coding agents

`CLAUDE.md` is a symlink to this file; edit `AGENTS.md`. This file holds the
rules. Detail, history and measurements live in the docs named below.

Rusty Musializer is a shipped, still-growing creative tool and the sole
implementation authority. The earlier C (`../musializer`) is legacy since
2026-08-03 (`docs/archive/C_PORT_HISTORY.md`): frame work as "what should this
application do", never "what did the C do", and never preserve a C defect.

What is built is answered by the code, `cargo test` and `tools/verify.sh`, never
by an inventory in a doc — every such list here went stale within weeks.
`FEATURE_PARITY_PLAN.md` is the one live queue: read it before implementation
work, claim and update tasks there, never start another plan.

## Who this is for (operator, 2026-08-07)

The user is someone good at AI music generation who wants to give the music an
**enticing** video form through a tool that is **exciting to work with**. Numb
checkbox work produces the wrong artifact even when every item closes.

Before calling a task done, run the feature as that person would — music
playing, entering where they would enter — and judge the moment of response. A
control should read as an invitation, a result should look worth posting, an
idle affordance should make them want to try it. If it feels like nothing, it is
not done: fix it or record what is missing. The evidence rules below are the
floor, not a substitute.

## Commands

```sh
tools/verify.sh                    # everything that can check itself, in order
tools/verify.sh --quick            # same, minus the headless capture
cargo build / cargo test           # test is headless: no window, no audio device
cargo clippy --all-targets && cargo fmt --check
cargo run -- path/to/song.mp3      # window + audio + Spectrum
cargo run --bin make-fixture-wav -- build/x.wav 8   # synthetic fixture audio

tools/headless_check.sh            # the self-check gate: private Xvfb, evidence
tools/headless_check.sh --skip-render  # Assist/routing checks only
tools/headless_run.sh ARGS...      # any hand launch: isolated, muted, on the GPU
tools/treatment_check.sh           # directing gate: --treatment, sheets, stills
tools/poster_check.sh              # poster frames gate
tools/column_check.sh              # the right column: views, frame, census, dock, render
tools/agent_controls_check.sh      # the composer's model, effort and access, against a fake agent
tools/chat_sessions_check.sh       # saved chats: reopen, resume, per project, forget
tools/agent_images_check.sh        # pictures in the chat: found, drawn in their colours, large
tools/align_lyrics.py song.mp3 song.lyrics.json   # time known lyrics (MMS CTC)
tools/install-linux-launcher.sh    # desktop entry; --uninstall removes it
cargo build --release              # ALWAYS last in a work stream
```

**End every work stream with `cargo build --release`** (operator, 2026-08-25).
The operator's manual look runs the release binary; a debug-only build hands
them a stale application.

## Silence: test audio never reaches the operator's headphones

Before launching anything that can open playback, identify its mute path. The
requirement is zero audible output from this process, not quieter audio.

- Direct test invocations take `--mute` (master volume 0; decoded samples,
  callback, analyzer and scene PCM unchanged). Never silence or zero a fixture.
- Every hand launch — render, still, contact sheet, probe — goes through
  `tools/headless_run.sh`: private Xvfb, `WAYLAND_DISPLAY`/`PULSE_SERVER`
  guards, dialog `PATH` guard, private `XDG_CONFIG_HOME`, `--mute`, VirtualGL.
  Hand-typed command lines here have got each of those wrong at least once.
- Scene/UI automation goes through `tools/headless_check.sh` (Xvfb `:77`).
- `cargo test` and differential harnesses open no device. A new test that does
  needs an explicit per-process mute/null sink.
- Never change desktop, device or system volume. If a command cannot be proven
  silent without altering the PCM under test, do not run it on the operator's
  session.

## Evidence

- A clean compile and exit 0 prove almost nothing about a renderer. Report
  lines must separate "drew something" from "tracked the input". A surface that
  can draw without its data must report which it did — a fallback that looks
  like content hides an unwired feature indefinitely.
- A surface nothing photographs is not reviewed; add a capture with every new
  screen, and print whether every new asset loaded.
- A control nothing presses is not verified, and hover is not press: use
  `--ui-probe click=XxY`, including a click into a gap as the control.
- A property assertion cannot pin a value (`x >= MIN` passes wrong formulas).
  Where a number exists, pin the number, preferably differentially.
- Every new check gets a negative control: perturb, watch it fail, revert
  byte-for-byte, prove the tree clean, record what it caught.
- Capture checks: opaque fills may be compared exactly; anything a blend or
  hairline touches needs a classifier invariant under ±1 rounding and coverage
  splitting. Put both renderers in the negative-control loop
  (`MZ_GL_LAUNCH=""` forces the CPU path).
- Headless probes are invented seams (`--probe-*`, `--ui-probe ...=`). Address
  items by id from report lines, never by a pixel measured once.

## Differential harnesses

`tools/differential_*.sh` compile `../musializer/src/*.c` into our `build/` and
compare against `crates/musializer-core/examples/*_dump.rs`. Table, lessons and
negative controls: `docs/DIFFERENTIAL_TESTING.md`.

- On demand, not in `verify.sh` (since 2026-08-16). Run one when you touch a
  module it pins. A failure is still a broken contract.
- Integers exact, floats with tolerance; `nan`/`inf` must fail explicitly
  (`abs(nan - nan) > tol` is `False`).
- Duplicate the fixture generator per side, never share it. Echo inputs into key
  columns so generator drift fails visibly. Compute boundary floats
  (`from_bits`/`nextafterf`), never type them as decimals.
- Pin asymmetric rejections as named expected pairs, then go check the claim
  written in the pair — that is how the `beat_tracker` parity bug was found.
- `settings` and `route_persistence` dumps print C-era rows, a
  `--- post-legacy (no oracle) ---` marker, then rows diffed against
  `tests/differential/*_post_legacy.txt`. Keep both halves.
- Diverging from a harness is a decision: update it to pin the new behaviour
  and record why.
- New harness: C side in `tests/differential/`, Rust side in
  `crates/musializer-core/examples/`, driver in `tools/`, row in the doc.

## Crate map

```text
crates/raylib-link/        builds + links raylib 6.0 from vendor/raylib-6.0
crates/musializer-core/    no raylib: analysis, scene contracts, model, layout
crates/musializer-runtime/ raylib, audio bridge, processes, filesystem edges
crates/musializer-app/     binary, CLI, scene drawing, UI (ui/panels/: one file per agent)
vendor/raylib-6.0/         upstream raylib (third-party)
vendor/clang-builtin-shim/ headers so bindgen runs; see its README
resources/shaders/ fonts/ logo/   embedded via include_str!/include_bytes!
packaging/linux/           desktop entry and MIME templates
docs/README.md             doc index and authority map
docs/CODE_ARCHITECTURE.md  crate boundaries, state ownership, data flows
docs/PHASE0_INVENTORY.md   CLI grammar, settings tables, schemas, env vars
docs/MUSIC_VIDEO_CLI.md    directing from the command line
docs/ASSIST_PIPELINE.md    analysis/lyrics control flow and trust policy
```

- `musializer-core` stays free of raylib handles, OS process handles, global
  mutable state and filesystem side effects.
- Shaders and fonts are embedded, never read from `./resources/` at runtime.
- raylib 6.0 is vendored and built by `crates/raylib-link`; `raylib`/`raylib-sys`
  6.0.0 run in `nobuild` mode. Rust 1.92 (egui 0.35). See
  `docs/RAYLIB_6_UPGRADE.md`. `bindgen` cannot be turned off (Cargo feature
  unification). It needs clang builtin headers Ubuntu lacks, supplied by
  `vendor/clang-builtin-shim/` via `CPATH`; `raylib-link/build.rs` strips
  `CPATH` before compiling raylib.

## `unsafe`

Small, named islands. Grep `docs/UNSAFE.md` for the module before touching one.
Every block carries a `SAFETY:` comment and a row in
`docs/UNSAFE.md`. Never add one without both.

## Traps already paid for

One line each. These are the rules; `docs/TRAPS.md` has the failure each one
caused and how to recognise its symptom. When a capture looks plausible but a
number disagrees, grep it there before debugging from scratch.

- **`WAYLAND_DISPLAY`**: set it to an unresolvable name (`MZ_NO_WAYLAND`),
  never unset it — unset falls back to a hardcoded `wayland-0`, the operator's
  compositor. Never write `env -u WAYLAND_DISPLAY`.
- **Auto scale changes logical card width.** Pin route geometry at an explicit
  scale, and compare both drawn reports; see `docs/TRAPS.md`.
- **Sequential fixed-display launches must reap Xvfb.** `xvfb-run` kills without waiting; tight gates own one server and `kill` + `wait` it. Never loosen the foreign-owner refusal. See `docs/TRAPS.md`.
- **A nested gate already holds `MUSIALIZER_CAPTURE_DISPLAY`.** Reuse only
  an Xvfb owned by a calling ancestor; refuse other process trees. Keep every
  isolation guard on both the reuse and new-server paths. See `docs/TRAPS.md`.
- **The dialog `PATH` guard** (`ENTRY_NO_DIALOG_PATH`) is per call site; a hand
  probe that clicks anything must carry it.
- **`kdialog` with no display** aborts with `SIGABRT` and summons Apport. Keep
  the launcher's display check.
- **Never give the Python helpers their own process group** from the parent
  (`os.setsid()` then fails with `EPERM`). Keep the `ESRCH` fallback.
- **Read the `.c`, not the header comment.**
- **`take_screenshot` ignores subdirectories**; use `LoadImageFromScreen` +
  `ExportImage`.
- **rustfmt destroys data tables**: `#[rustfmt::skip]` on anything checked
  column by column.
- **Widget id namespaces are minted only in `widgets::id`'s `ALL` table.** A
  collision test over a hand-written subset proves nothing.
- **Empty clipboard reads** go through `runtime::clipboard::text`; raylib-rs 6.0 dereferences GLFW's null result.
- **raylib-rs slice wrappers are wrong until checked**: `load_font_from_memory`,
  `Wave::load_samples` and `Image::get_image_data` all were. Image and sample
  readers go through `runtime::decode`, fonts through `runtime::font`.
- **`EndTextureMode` leaves rlgl's cached framebuffer size at the target's.**
  Assume every raylib `End*` undoes less than its `Begin*`; check the C. The
  `gl framebuffer:` report line asserts it.
- **Shader hashes are integer** (`uint` multiply-xorshift), never `fract` of a
  large product, which tears along the lattice.
- **`Faces::caption_at` lends one face at a time**; a second live borrow panics.
- **`rlBlitFramebuffer` takes far corners, not extents.**
- **`BLEND_ALPHA` eats destination alpha**, and a still is written opaque, so
  no PNG shows it: read `still alpha:` (`tools/still_alpha.py`), not the file's
  alpha plane. Full-frame washes use `BLEND_ALPHA_PREMULTIPLY`. See
  `docs/TRAPS.md`.
- **`--probe-shot` photographs the frame before the last** on the GPU and the
  last on llvmpipe; photograph a state held still for two frames.
- **A modal masks late chrome too.** Keep its pointer occluder through the notice tray; restore real input only for the overlay. See `docs/TRAPS.md`.
- **A stranded pointer claim is invisible in a picture**; the next interaction
  misbehaves. `gesture=none` on `timeline:` proves the release was taken.
- **`--ui-probe hover=XxY`**, not a comma; the spec is comma-separated.
- **A slider knob hangs outside its rect at 0 and 1**; inset neighbours by
  `widgets::SLIDER_KNOB_RADIUS`.
- **`rlDisableDepthTest` is immediate, geometry is batched**: bracket the
  toggle with a flush.
- **Cross-commit probes use native preference shapes** and assert `appearance:`; a rejected newer field can silently select the dark fallback.
- **Quiet timing excludes only owned process descendants**; nested gates start Xvfb servers too.
- **A lifetime ring peak includes startup**; injected-stall assertions use raw counter boundaries, and zero counters are parsed numerically.
- **`--ui-probe route=` opens on the scene frame 0 draws**; read `plot@` in
  the report before believing the editor is open.
- **`click=` never reaches egui**; press dock controls with `agent-press=`.
- **A probe run's column view persists** in `MZ_STATE_DIR`; a run that leaves Direct gets its own, or the next chat draws nothing.
- **A check on a name says nothing about the file you then open**: open once without blocking, ask the handle, read through a cap, and feed FFmpeg by stdin. See `docs/TRAPS.md`.
- **A small file can claim a huge picture**: budget from the header bytes before any decoder allocates, and refuse a format with no header reader.
- **A notice longer than the queue holds is shortened, not refused**: put what must survive first, and go through `Shell::notify`, not `notices.push`. See `docs/TRAPS.md`.
- **A stream seeked to exactly its end reads 0:00**: anything that parks the playhead "at the end" stops short of it.
- **`grep` here is ugrep and skips a file with a NUL byte silently** (`-c` prints nothing); use `grep -a` or `rg` when an empty answer decides something. See `docs/TRAPS.md`.
- **Reaping the encoder does not close its pipe**: a process the child started still holds the read end. Stop the writer before the kill, join it only after the reap, and bound the join.
- **A report's rect can be below the fold**: a `click=` past a scrolling view's `viewport=` is clipped and reads as a dead control. Check the viewport or make the window taller. See `docs/TRAPS.md`.
- **An edit's autosave posts a notice over the preview**: mask the `notices:` cards from both runs before comparing preview pixels.
- **FFmpeg's `-ss` counts from the container's start and drops frames before it**: seek to the midpoint between frames, plus the video's offset. See `docs/TRAPS.md`.
- **A video FFmpeg must seek in can't be piped**: hand it the open file as stdin and read `/proc/self/fd/0`, with `-nostdin`. See `docs/TRAPS.md`.
- **A still compared with FFmpeg's frame at another size measures two scalers**: compare at the source size. See `docs/TRAPS.md`.
- **A saved-file check must outlast autosave's 1.5 s settle**: give it the edit frame plus about 150 frames, and pair every "nothing was written" check with a control that writes.
- **An XTest driver focuses the mapped, window-sized child and ignores X errors**: the last root child can be unmapped, and Xlib's default handler then exits the driver.
- **Seek strictly inside a span, not onto its boundary**: a seek to a section's start can land a sample early, in the section before.
- **Bundling, hashing and copying an asset open it like the decoder does**: non-blocking, regular file, the kind's import cap, hashed while copied (`publish::open_source`). See `docs/TRAPS.md`.
- **A worker's drop must reach its start-up**: the hash and the probe check `stop`, and every child registers its group where the drop can kill it. See `docs/TRAPS.md`.
- **Release per-clip resources from the hook that runs after every frame, and bound their sum per process**: a count that only one scene advances stops when the video moves on. See `docs/TRAPS.md`.
- **Give a `flock` back with `File::unlock` before dropping the file**: a child mid-spawn on another thread holds the lock until it execs. See `docs/TRAPS.md`.
- **A poster's recipe must hold everything its replay reads**: the clip was missing from it, so lane posters did not go stale after a clip edit. See `docs/TRAPS.md`.
- **Own start pins the exact clip time**: rounding it to the readout moves the picture a frame. See `docs/TRAPS.md`.
- **A containment check on a name is a check on names too**: open a bundled file beneath its folder, one `O_NOFOLLOW` name at a time, and read only that handle. See `docs/TRAPS.md`.
- **A recorded number that playback recomputes is corrected in one place, not read beside it**: the card pinned with a clip's stale recorded length. See `docs/TRAPS.md`.
- **A press is not yet a click**: anything a drag must not do happens on the release, and `click-hold=` photographs a press before it resolves. See `docs/TRAPS.md`.
- **The gate's gradient measure reads film grain as detail**: an encoder that flattens grain lowers it, so under a finish read the blurred encoding error instead. See `docs/TRAPS.md`.
- **A child started by bare name is no child under `headless_run.sh`**: its `PATH` is private, so resolve the program to an absolute path. See `docs/TRAPS.md`.
- **A gate's EXIT trap stops at the first `wait` on a child it killed** under `set -e`: give every `kill`/`wait` in a cleanup its own `|| true`, or the Xvfb outlives the gate. See `docs/TRAPS.md`.
- **A reasoning model with no `max_tokens` stops at the provider's ceiling** (65,536 for MiMo, all whitespace once): send a cap on every hosted call, and treat `length` as a failure even when the text parses. See `docs/TRAPS.md`.
- **A helper's own reason dies at `_run`**, which reports only "exited with code N": a child that knows why it failed writes a receipt, checked for schema and audio hash, that the parent reads. See `docs/TRAPS.md`.
- **A failed paid submit is not a free one**: write an intent before the POST and clear it only on a 4xx or a `size_request` of 0. See `docs/TRAPS.md`.
- **Two launches at once need their own `MZ_STATE_DIR`**: `headless_run.sh`'s `ln -sf` races and one dies with "Already exists". See `docs/TRAPS.md`.
- **A save writes only the clips a section plays**: an unplaced clip is not in the saved `.musi`; look in the store. See `docs/TRAPS.md`.
- **A `--ui-probe` spec over 256 bytes is refused whole**: the run goes on with no probe; use relative paths. See `docs/TRAPS.md`.
- **A rename is atomic, not durable**: sync the folder after it, a new folder into its parent, and a file another program wrote before renaming it. See `docs/TRAPS.md`.
- **A removal after a wait removes whatever is there now**: remove a record under its lock and only while it is still the record you read. See `docs/TRAPS.md`.
- **A ceiling checked one request at a time bounds only what was seen so far**: keep the bound where the act happens, checked and increased under the act's own lock, and report rather than count a sum another process can change. See `docs/TRAPS.md`.
- **A record that cannot be read is not an empty record**: absent, read and there-but-unreadable are three answers; the third holds what the record guards until a person looks. See `docs/TRAPS.md`.
- **Replacing a record of possible spending erases a possible charge**: append, and clear only when the thing paid for is in hand. See `docs/TRAPS.md`.
- **A gateway error is not proof the request was not processed**: retry only a 429 or a request that never left whole. See `docs/TRAPS.md`.
- **An upper bound printed at a precision is rounded up**: show a ceiling the person typed as given. See `docs/TRAPS.md`.
- **A native-key driver that types on a clock races the first frame**: wait for the mapped window and a settle past frame 0 (or a painted ready mark), and give the run frames enough to outlast the keys. See `docs/TRAPS.md`.
- **A parameter one provider lists can leave a ZDR route with no endpoint**: Gemini's Vertex endpoints do not list `temperature`, so keep a model's quirks in its row, not in the shared request. See `docs/TRAPS.md`.
- **A cached catalog older than a model cannot say the model is gone**: its absence there is "not looked since", not a refusal. See `docs/TRAPS.md`.
- **`headless_run.sh` isolates config, not cache**: a probe reads the operator's `~/.cache/musializer`, so set an absolute `XDG_CACHE_HOME` when a capture must not depend on it. See `docs/TRAPS.md`.
- **A temporary folder outlives a helper that is killed**: a private copy that must not survive has no name (`os.memfd_create`, read through `/proc/self/fd/0`), and a child that only decodes gets an allowlisted environment. See `docs/TRAPS.md`.
- **A saved project's bytes move with the playhead**: a save writes `metadata.playhead_seconds` and a seek does not dirty the project, so mask it when comparing saved files. See `docs/TRAPS.md`.
- **A digest taken once describes the file at that moment**: keep the file's stamp beside it and check it before anything is spent. See `docs/TRAPS.md`.
- **A request in flight leaves no word on disk unless one is written before it leaves**: a kill runs no `finally`. See `docs/TRAPS.md`.
- **An answer cached before the later steps check it is refused forever**: check what the reader requires before caching, and set aside what it refuses. See `docs/TRAPS.md`.
- **A schema keyword one provider refuses fails the whole request** (Google refuses `maxItems`): after any change to what a hosted model is sent, make one real request before it reaches a release; no fake can know. See `docs/TRAPS.md`.
- **`LoadRenderTexture` only logs `rlFramebufferComplete`**: check completeness yourself on any target built from parts. RGBA16F is not a deeper RGBA8, because it does not clamp.

## Judge by what a user or a file can observe

Mechanism is negotiable; observable behaviour changes only on purpose.

- Before changing anything that looks odd, grep `docs/DIVERGENCES.md` for it
  (see Required lookups).
- When a faithful port is impossible, unavailable or bad Rust: decide, build the
  alternative, and record the divergence and its reason in
  `docs/DIVERGENCES.md`. Do not stall, stub or ask. An unrecorded divergence
  gets "fixed" back into a defect by the next session.
- These change only deliberately, with a schema bump or an updated harness and
  a recorded reason: the `.musi` format and every schema in
  `docs/PHASE0_INVENTORY.md` (every file this application ever wrote must open
  and round-trip; bump `schema_version`); analysis numbers; settings semantics
  (keys, bounds, defaults, precision, clamping); export determinism; the CLI
  grammar including flag order and exit status; anything with a harness.
- The test: visible in a `.musi`, an MP4, a number or a documented command line
  → deliberate or a bug. Visible only in source → take the better option and
  write down why.
- Features past the C need no parity justification. Scene ids and persisted
  indices append, never insert or reorder. A `.musi` extension never writes its
  default, so older files stay byte-identical.
- **Missing beats pretending**: an unbuilt feature says so in the interface, by
  name, where the control would be (a disabled control with its reason,
  `ShellCommand::NotImplemented`). Never a blank region or a silent no-op.

## The oracle and outside references

- `../musializer` is read-only, frozen at `9300af9` on `master` (not `main`).
  Never modify it or its build directory. If it looks wrong, say so. Its code and
  tests are authoritative about its behaviour, its documents are not; read its
  `CURRENT_FILE_POINTERS.md` first.
- **Outside inspiration: Le Biniou** (operator, 2026-08-27).
  `~/Projects/lebiniou` and `~/Projects/lebiniou-data` are GPLv2, not ours.
  Read them for effect ideas (the feedback family: blur/fade/zoom of the previous
  frame, colormap cycling, warps, beat-gated pulses). Reimplement from visible
  behaviour; never translate source, copy images or colormaps, or depend on
  them. The same rule covers any reference look (e.g. modal.com for Glyph
  Lattice): from captured images only, no code, assets or brand.

## Required lookups

Your own reasoning won't stand in for these. Each file records a decision or a
measurement that can't be derived from the code, and each was written because a
session that reasoned it out instead got it wrong. Grepping counts as looking.
Not grepping doesn't.

| Before you… | Run | Why reasoning is not enough |
| --- | --- | --- |
| change behaviour that looks wrong, odd, inconsistent or unlike the C | `grep -n -i '<module, setting, flag or feature>' docs/DIVERGENCES.md` | ~140 rows, most are operator decisions with dates. Code that looks like a bug is often a recorded choice, and "fixing" it reverts the operator |
| add, move or edit an `unsafe` block, or call raylib ffi | `grep -n '<module>' docs/UNSAFE.md` | The invariants live there, not in the types. Three raylib-rs wrappers looked safe and were wrong |
| touch raylib render targets, blends, shaders, fonts, `rlgl` state or the probe/report seams | `grep -n -i '<function or symptom>' docs/TRAPS.md` | Every entry produced a plausible picture with every gate green |
| edit a module | `ls tools/differential_*.sh`, then `grep -n '<module>' docs/DIFFERENTIAL_TESTING.md` | If a harness pins it, the harness is the contract, and the module's own unit tests have stayed green against wrong formulas |
| write or loosen a pixel/colour assertion, or a GPU-vs-CPU comparison | read `docs/HEADLESS_GPU.md` | Thresholds calibrated on one rasterizer are calibrated to an accident |
| answer "why is it like that" | `grep -rn -i '<thing>' docs/archive/FEATURE_PARITY_HISTORY.md docs/DIVERGENCES.md` | The reason is recorded; a guess can't be checked |

When you diverge, invent or hit a new trap, write the row before you finish.
Divergences go in `docs/DIVERGENCES.md`. Traps go in `docs/TRAPS.md`, plus one
line under the traps list above.

- History, not instructions: `docs/archive/` (`FEATURE_PARITY_HISTORY.md`
  answers "why is it like that"), `REWRITE_PLAN.md` (at the root only because
  source comments cite it). Put expensive-to-rediscover investigation there,
  live tasks in `FEATURE_PARITY_PLAN.md`.

## GPU

Measurements and the defects the GPU path surfaced: `docs/HEADLESS_GPU.md`.

- The headless gate runs on the GPU through VirtualGL; its `gpu:` line names
  the path, and a CPU line is the thing to explain. `MZ_GL_LAUNCH` overrides
  (empty forces `llvmpipe`), `MZ_VGL_DEVICE` picks the device (default `egl0`).
  The launcher must resolve to an absolute path, because probes strip `PATH`.
- Pass `--encoder nvenc` for every export you only look at; leave the default
  x264 for anything the operator keeps or posts. The gate picks NVENC itself
  (`MZ_ENCODER=x264` forces CPU). Unknown encoder names are refused.
- NVENC `-cq` is not x264 `-crf` (mapping in `ExportQuality::nvenc_cq`);
  `-b:v 0` is load-bearing; the 3090 cannot encode AV1. The encoder is session
  state (`RenderRequest`), never project state. An md5 comparison of two exports
  must hold the encoder fixed.

## Persistent storage (operator, 2026-08-04)

- Model weights: a directory the user chooses. Default `<install dir>/models/`,
  falling back to a `musializer` directory in the user's home. Never a location
  the user was not shown.
- On this machine, research downloads go under
  `~/.local/share/musializer/models/<model>/`. Never install into the
  `~/.local/share/musializer/lyrics-align/` venv.
- Hugging Face ids land in `~/.cache/huggingface/hub`, a symlink onto the
  `Bigger SSD`. If `mountpoint -q "/run/media/wolfram/Bigger SSD"` fails, run
  `udisksctl mount -b '/dev/disk/by-label/Bigger\x20SSD'`.
- Credentials: no OS wallet. A 0600 file under the per-user config directory,
  session-only storage, or env import. Never in `.musi`, preference JSON, argv,
  logs, analysis artifacts or a repository `.env`.
- Non-secret preferences: versioned, atomically replaced, XDG config directory.

## Repository rules

- Preserve unrelated work in both repositories.
- First-party code is Rust. raylib through bindings, FFmpeg as an executable and
  the Python helpers as independent tools are intended.
- Never commit credentials, `.env`, user audio, generated video, analysis caches
  or build artifacts. Synthetic fixtures only.
- Parallel agents: one file each under `ui/panels/`; nobody edits another's or a
  `mod.rs`. Only the integration owner edits the root manifest or broad
  application state; leaf agents request dependencies.
- Small compiling checkpoints at hobby pace, not release ceremony.

## Style

- Where code reproduces C behaviour, cite the C file and line.
- Comments say why, especially where the oracle looks wrong or a choice looks
  fixable.
- `cargo fmt` and clippy clean; deviations carry `#[allow(...)]` with a reason.

## Not built, and not going to be

Microphone capture (operator, 2026-07-27), hot reload, non-Linux platforms.
