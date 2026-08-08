# Repository guide for coding agents

The Rust rewrite of Musializer. The C repository is feature frozen.

**The application is built and runs.** All eleven scenes draw, `.musi` projects open
and save, video exports through FFmpeg, every bottom panel is real, and
`tools/verify.sh --differential` is 23 passed / 0 failed — thirteen
differential harnesses against the frozen C (opt-in since 2026-08-08; the
plain run is 10 passed and skips them), a headless capture gate, and the
assist-era gates (secret canary scan among them). The sole live completion queue
is `FEATURE_PARITY_PLAN.md`. It records the application-boundary gaps those gates
do not cover — currently the durable-edit guarantees (complete dirty marking,
all-track autosave, draft guards), the missing C entry points (file drop, ASCII
image import, lyrics TSV, timeline pan/markers), proving the copied support
bundle actually runs (Assist end-to-end, Google Fonts, doctor/dist), and the
UX0-B/UX0-C workflow-friction and product-opportunity backlog.

`CLAUDE.md` is a symlink to this file. Edit `AGENTS.md`.

## Who this is for (operator, 2026-08-07)

This is a creative tool. Its user is someone who has learned to get the most
out of AI music generation and is now looking for the next step: giving the
music a video form that is **enticing**, through a tool that is **exciting to
work with**. Build from that chair, not from the checkbox. Handling the work as
numb instruction-following produces the wrong artifact here even when every
item closes — the operator has watched agents do exactly that.

Concretely: before calling a task done, run the feature the way this person
would — music playing, entering from where they would enter — and ask what the
moment of response feels like. A control should read as an invitation, a result
should look like something worth posting, and an idle affordance should make
the user want to try it. If the honest answer to "what does this feel like" is
"nothing", the task is not done; say what is missing and either fix it or
record it. The evidence rules elsewhere in this file are the floor for
correctness, not a substitute for this judgment.

## Commands

```sh
tools/verify.sh                    # everything that can check itself, in order
tools/verify.sh --quick            # same, minus the headless capture

cargo build                        # no environment setup needed
cargo test                         # headless; no window, no audio device
cargo clippy --all-targets
cargo fmt --check

cargo run -- path/to/song.mp3      # the slice: window + audio + Spectrum
cargo run --bin make-fixture-wav -- build/x.wav 8   # synthetic fixture audio

tools/headless_check.sh            # the self-check: private Xvfb, evidence
tools/install-linux-launcher.sh    # per-user desktop entry; --uninstall removes it
tools/differential_*.sh            # one per ported pure module, vs the frozen C
                                   #   verify.sh runs all of them; see the table below
```

## Test audio must never reach the operator's headphones

At the start of any session that may run the application, scene probes or UI
tests, identify every command that can initialize playback and confirm its mute
mechanism **before launching it**. The required outcome is zero audible output
from this process, not quieter test audio.

- Pass `--mute` to every direct application invocation used for testing. It sets
  raylib's master output volume to zero while leaving decoded samples, the audio
  callback, analyzer input and scene PCM unchanged. Do not make a fixture silent
  or zero its samples to achieve quiet playback; that invalidates the test.
- Use `tools/headless_check.sh` for scene/UI automation when possible. It already
  points `PULSE_SERVER` at an unresolvable private path in addition to using the
  application's mute path, so it cannot reach the operator's real audio sink.
- `cargo test` and pure differential harnesses do not open an audio device. If a
  new test or helper does, give it an explicit per-process mute/null sink and
  document why the PCM path remains real.
- Never change the desktop, device or system-wide volume as test setup. That
  mutates the operator's session and can still race with another process. Silence
  must be scoped to the Musializer process under test.
- If a command cannot be proven silent without changing the PCM being tested, do
  not run it on the operator's session. Adapt it to `--mute`, isolate its sink, or
  use the existing Xvfb/headless path first.

## Differential testing against the oracle

Every script compiles the relevant `../musializer/src/*.c` with output going into
our own `build/`. The oracle is read, never written, and its own build directory
is never touched.

| Harness | Result |
| --- | --- |
| `differential_analyzer.sh` | 104 bands agree to 4e-10 (float print precision) |
| `differential_settings.sh` | all 81 descriptors match exactly |
| `differential_routes.sh` | 380 rows, including the clamp asymmetry |
| `differential_route_persistence.sh` | 381 lines: parse grammar and the both-ways pairing |
| `differential_event_merge.sh` | 12 cases, 15 merged events, ids and ordering exact |
| `differential_assist_ui.sh` | 341 policy decisions exact |
| `differential_preset_store.sh` | 11 scene tokens, 44 presets, 3660 bytes of store JSON |
| `differential_song_atlas_map.sh` | 912 slices, 29374 values, largest delta **0** |
| `differential_ascii_art.sh` | 53847 records, 329738 values, largest delta **0** |
| `differential_project_io.sh` | 2550 values, **both `.musi` round trips** plus frame-lane boundaries, largest delta **0** |
| `differential_timeline_view.sh` | 30865 records, 204953 values, largest delta **0** |
| `differential_layout.sh` | 27547 records, 527187 values, largest delta **0** |
| `differential_beat_tracker.sh` | 12352 records, 123522 values, largest delta **0** — **found a parity bug** |

Two of those harnesses now compare in **two sections**. The tree has a scene the
frozen C cannot have (Phosphor Dream, id 10, 2026-08-08), so `settings_dump` and
`route_persistence_dump` print the C-era rows, a `--- post-legacy (no oracle) ---`
marker, then the rest; the driver diffs the first half against the oracle exactly
and the second against `tests/differential/*_post_legacy.txt`. Both controls were
run: perturbing `settings.spectrum.trail`'s bound fails against the C, and
perturbing `settings.phosphor.dwell`'s fails against the pinned file. Do not
"simplify" this back to one diff — the point is that a post-legacy value is still
a `.musi` compatibility surface and still has to fail loudly when it moves.

**This is the pattern to copy for every pure module.** A number to compare beats
a paragraph of reasoning about whether a port is faithful, and it catches the
class of error review misses — the settings harness exists because ~81
descriptors were transcribed by hand and a single mistyped bound would surface
much later as a scene quietly ignoring a saved setting.

Compare integers exactly; give floats a tolerance, because libm and Rust's
intrinsics may differ in the last bits. Duplicate the fixture generator between
the C and Rust sides rather than sharing it — a shared generator can hide the
difference you are looking for.

**Build a negative control, and record what it caught.** A harness that has never
failed proves nothing, and the two most recent ones both earned this rule: the
Song Atlas harness was checked by moving the onset threshold 0.62 → 0.60, which
made 20 `onset` flags disagree, and by perturbing a loudness coefficient by 1e-7,
which made 17389 floats disagree with a smallest delta of 1.9e-9. Writing that
control also exposed a hole in the comparator itself — `nan` and `inf` *parse* as
floats in Python, and `abs(nan - nan) > tolerance` is `False`, so those columns had
been passing unconditionally. Perturb, watch it fail, revert byte-for-byte, and
prove the tree is clean.

**A unit test suite ported from the oracle is not a substitute, and there is now a
measurement rather than an opinion.** The `layout` harness was written because
`ui/timeline_view.rs` had **1** exact assertion where `test_timeline_view.c` pins 32,
and `workspace_layout`/`timeline_layout` were similarly range-heavy. Its two negative
controls both left the ports' own unit tests **fully green**:

| perturbation | the harness | the module's unit tests |
| --- | --- | --- |
| `>=` → `>` in `workspace_layout`'s mode ladder | 431 values fail | **9 passed** |
| `margin * 2.0` → `* 2.0625` in `timeline_layout` (1/16 px) | 14673 values fail | **7 passed** |

The second is the instructive one: the tightest unit expectation gives `scale` a 1e-3
tolerance, and 0.375 px spread over a 628 px row lands inside it. So the rule is not
"prefer harnesses"; it is that a **property assertion cannot pin a value**. `assert!(x
>= MIN)` and `assert!(rect.contains(inner))` are satisfied by wrong formulas, and a
layout error is invisible in a capture because everything moves together and still
looks self-coherent. Where the oracle pinned a number, pin the number — and prefer a
differential comparison, because a hand-transcribed expectation can be mistyped or
(worse) copied from our own output, and would then pass forever.

Both harnesses found **no disagreement**: the ports were right. That is the other half
of the point — a harness on a suspected module converts "we cannot tell" into "it is
correct", which is worth as much as finding a bug.

**And then the next one found an actual bug, in the place the argument predicted.**
`beat_tracker` was chosen for a harness because its port's strongest evidence was a
*hand-transcribed* eight-value table — `%.9g` phases pasted from a scratch C build
that no longer exists, unre-derivable without redoing the work. Replacing 8 pinned
values with 123522 compared ones surfaced this:

`beat_tracker_update` returns false for two unrelated reasons. It refuses bad input
*before* writing its out-parameter; or it computes a position inside `[0, 1)`,
narrows it to a float that lands on exactly `1.0`, **writes that**, and refuses it
afterwards (`beat_tracker.c:76-78`). `plug.c:1139-1144` keeps a
`float beat_phase = 0.0f;`, passes its address, and uses whatever is in it either
way — so the scene frame gets `0.0` in the first case and `1.0` in the second. The
port returned `Option<f32>`, had nowhere to put the `1.0`, and collapsed both to
`0.0`. `beat_phase` is a documented route source, so that is visible in a rendered
MP4.

Three things about how it was found are worth copying:

- **It surfaced from writing the excuse down.** The first version of the harness
  pinned this as an expected divergence, with a comment arguing it was an oracle
  quirk the port deliberately did not reproduce. Reading the C's *caller* to justify
  that sentence is what showed the sentence was false. Pin asymmetries as named
  pairs — and then go check the claim you wrote in the pair.
- **The `Option` was the disguise.** `None` reads as "no value", so two refusals
  with different observable results looked identical at the type level. The fix is a
  three-variant `BeatUpdate`, because the C's `(bool, out float)` pair carries
  information an `Option` cannot.
- **It is not a corner case.** The grid hits it **187 times in 12250 steps**: any
  tick that nearly divides the beat interval drifts the position a hair under a
  whole multiple, and 2⁻²⁵ below 1.0 is enough. A 0.3 s tick against the neutral
  0.5 s clock does it by the tenth step.

Its two other negative controls behaved like `layout`'s — **10 unit tests green**
against 3 and 732 failing harness values, for `>` → `>=` on the discontinuity
threshold and for an exclusive upper bound on the credible-gap range. Neither test
suite has a case for a gap of exactly 0.75 s or exactly 1.5 s, which is why.

Two smaller lessons from the same build, both from checks failing *before* the port
was suspected:

- **Echo the inputs, so a drift between the duplicated generators fails in a key
  column.** `3.4028235e38f` in C is a *float* literal widened to a `double`
  parameter — a different number from the same decimal parsed as a double. The two
  harnesses disagreed by 3.4e30 on the first run, in the echoed `time` column, and
  it was mine, not the port's.
- **Clippy's `excessive_precision` can be reporting a dead test.** A row feeding
  `0.0399999991` as "the float just below the 0.04 floor" rounds back *to* `0.04f`,
  so it duplicated the row above it and the boundary looked tested while going
  untested. The real value has to be computed (`nextafterf` / `from_bits`), not
  written as a decimal.

**Where the two sides cannot express the same rejection, pin the asymmetry as an
expected pair.** The C takes bare pointers and Rust takes slices, so each can refuse
an input the other cannot even represent: the C rejects a `NULL` buffer, and Rust
rejects a 15-byte buffer for a 2x2 image that the C has no length parameter to
notice. The `ascii_art` harness asserts those as four named pairs — including
*deliberately not* calling the C with the truncated buffer, since that would read
past the end — rather than quietly excluding the cases. An untested rejection path
is where a difference hides.

Adding one: put the C harness in `tests/differential/`, the Rust side in
`crates/musializer-core/examples/` (examples need no manifest entry, so they
never collide with a parallel agent), and the driver in `tools/`. Add the name to
`tools/verify.sh`'s loop and a row above.

`tools/headless_check.sh` is how this project checks its own work without
occupying the operator's session. It runs on Xvfb `:77` with `WAYLAND_DISPLAY`
pointed at an unresolvable socket name (**not** unset — see the trap below) and
`PULSE_SERVER` pointed somewhere unresolvable, and writes artifacts
under the gitignored `build/`. Read `../musializer/tools/UI_REVIEW.md` for the
reasoning; the isolation rules there are not optional.

**Check claims with evidence, not with a clean compile.** A build that succeeds
and a process that exits 0 prove almost nothing about a renderer. The slice
prints a report distinguishing "drew something" from "tracked the input", and
that distinction is the point.

## Crate map

```text
crates/
  raylib-5-5-link/    # builds + links raylib 5.5 from vendor/raylib-5.5
  musializer-core/    # no raylib: analysis, scene contracts, model, layout
  musializer-runtime/ # raylib, the audio bridge, processes, filesystem edges
  musializer-app/     # the binary, CLI, scene drawing, UI
                      #   ui/panels/ is one file per fan-out agent; nobody edits
                      #   another's, and nobody edits a mod.rs
vendor/
  raylib-5.5/         # upstream raylib source (third-party, not ours)
  clang-builtin-shim/ # five headers so bindgen can run; see its README
resources/shaders/    # first-party GLSL
resources/fonts/      # Space Grotesk + Alegreya + Font Awesome 4, SIL OFL 1.1
                      #   (third-party; each carries its own licence file)
resources/logo/       # generated by tools/make_rust_logo.py
packaging/linux/      # desktop entry and MIME package templates
docs/README.md            # documentation index and authority map
docs/CODE_ARCHITECTURE.md # crate boundaries, state ownership, primary data flows
docs/ASSIST_PIPELINE.md   # analysis/lyrics control flow, trust and timing policy
docs/PHASE0_INVENTORY.md  # CLI grammar, settings tables, schemas, env vars
```

Shaders and fonts are **embedded** with `include_str!`/`include_bytes!` rather than
read from `./resources/` at runtime, which is what the C does. The C's relative
paths are why its launcher has to `cd` into the project root before exec; an
interface that loses its font because it was started from the wrong directory is a
failure mode worth deleting rather than reproducing.

`musializer-core` must stay free of raylib handles, OS process handles, global
mutable state, and filesystem side effects. That constraint is what made the C
project's 327-test suite possible and it is the main bet of this rewrite.

## The raylib binding decision

**Option 2, decided and proven by the vertical slice.** raylib 5.5 source is
vendored here and built by `crates/raylib-5-5-link`, with `raylib`/`raylib-sys`
5.5.1 in `nobuild` mode supplying only the bindings.

The reason, recorded so nobody re-derives it: `raylib-sys` 5.5.1 vendors raylib
**5.6-dev**, so letting it build its own copy would put a different raylib under
the renderer than the parity oracle was built against. Compile flags mirror
`../musializer/src_build/nob_linux.c` exactly.

Two things about this that will otherwise waste a session:

- `bindgen` cannot be turned off. The safe `raylib` crate depends on
  `raylib-sys` without disabling its default features, and Cargo unifies
  features, so `bindgen` is on whenever `raylib` is in the graph.
- bindgen needs clang's builtin headers, which Ubuntu's `libclang1` does not
  ship. `vendor/clang-builtin-shim/` supplies them via `CPATH`, and
  `raylib-5-5-link/build.rs` strips `CPATH` before compiling raylib so those
  minimal headers never shadow GCC's real ones. Full reasoning in
  `vendor/clang-builtin-shim/README.md`.

## `unsafe` inventory

Small, named, reviewable islands. Every `unsafe` block carries a `SAFETY:`
comment stating why it holds. Current islands:

| Where | Why | Invariant |
| --- | --- | --- |
| `runtime::audio_bridge` | raylib's `AudioCallback` is a bare `extern "C"` fn with no user-data pointer, so the ring must be reachable from a `static`; the vendored output-underrun counter is exposed by one no-argument C function | The callback only touches a lock-free SPSC ring — no allocation, lock, or syscall. `attach`/`detach` are `unsafe` and document the stream-lifetime contract; the diagnostic getter locks inside C and returns only an integer |
| `runtime::draw` | raylib's default 1x1 texture is a non-owning handle, and the safe `Texture2D` would unload it on drop | The ffi draw wrappers take a `&mut impl RaylibDraw`, so an active drawing context is proven at compile time |
| `runtime::draw` colour helpers | `ColorFromHSV`/`ColorAlpha` are pure C arithmetic | No global state; safe to call anywhere |
| `runtime::draw::SceneViewport` / `FramebufferAudit` | the 3D panel clip is rlgl viewport state the safe API does not expose, and the size authority it reads has two sources that disagree | Four blocks, all pure getters plus the `rlViewport`/`rlSetFramebuffer*` pair `Drop` restores. `begin_with_screen` reads `GetRenderWidth`/`Height` on the default framebuffer and rlgl's cached pair only inside a render target, because `EndTextureMode` restores the first and not the second (`rcore.c:1109-1131`, `rcore.c:3537`); `FramebufferAudit::observe` reads both and writes nothing, so the report line can state the invariant the headless gate asserts |
| `runtime::process::process_group` | `std`'s `Child::kill` sends only `SIGKILL`, to only one process. `SIGTERM` and process-group delivery need `kill(2)`, and `libc` is not a dependency | One block wrapping a hand-declared `extern "C" fn kill(c_int, c_int) -> c_int`. Both arguments pass by value, nothing is written through a pointer, and every caller passes a pid it owns as a live `Child` (or its negation) |
| `runtime::font` | raylib-rs's safe `load_font_from_memory` takes the glyph set as a `&str` and passes `str::len()` — a **byte** count — as the codepoint count, so a multi-byte set makes raylib read past the array. And `GetFontDefault` is a non-owning handle that `Font`'s `Drop` would unload | Four blocks, all inside `rasterize` and `default_face`, and every face goes through them — the four built-in ones (including the icon face, whose codepoints are all above U+F000 and so are exactly the multi-byte case the wrapper gets wrong) and a project's imported one. `LoadFontFromMemory` gets both lengths from the slices themselves, and raylib copies out what it needs before returning, so a heap buffer read from disk is as safe as an `include_bytes!` array; the result goes straight into `Font::from_raw`, whose `Drop` is `UnloadFont`. The default face is wrapped in `WeakFont`, whose drop is a no-op. `GenTextureMipmaps` borrows the font's own texture field so the level count is written back where `SetTextureFilter` reads it |
| `runtime::decode::wave_samples` | raylib-rs's safe `Wave::load_samples` builds its slice as `(pointer, frameCount)` while `LoadWaveSamples` allocates `frameCount * channels` floats, so for any stereo track it hands back exactly half the decoded audio — silently | One block. The count comes from the wave's own format, the pointer is null-checked before a slice is formed, the slice is copied into a `Vec` inside the block, and the allocation goes straight back to `UnloadWaveSamples`. Nothing borrowed escapes, and the `ffi::Wave` it is called with is a bitwise copy of one whose owner outlives the call |
| `runtime::decode::image_pixels_rgba8` | reading a decoded image's pixels needs `Image::data`, and the safe alternative — `get_image_data` — forms a slice from `LoadImageColors` without null-checking the pointer, which is the third instance of the same defect | One block, and the one every image reader in the tree goes through: `image_rgba8` calls it and copies, the export's per-frame readback calls it and does not. `ImageFormat` converts to `UNCOMPRESSED_R8G8B8A8` first, and the length is `GetPixelDataSize` — the same function raylib itself sizes the buffer with — checked against `width * height * 4` before the slice is formed, so a format `ImageFormat` silently declined to convert is refused rather than misread. The pointer is null-checked, and the returned slice borrows the caller's `&mut Image` for its whole life, so `UnloadImage` cannot run while it is alive and no second reference to the allocation can exist |
| `runtime::font::rasterize_sdf` | `LoadFontFromMemory` hard-codes `FONT_DEFAULT`, so a signed-distance-field atlas has to be assembled from the three calls it makes internally — `LoadFontData(FONT_SDF)`, `GenImageFontAtlas`, `LoadTextureFromImage` — and raylib-rs wraps none of them | Five blocks in one function, each freeing what it owns on the way out: the glyph array and the rectangles come from raylib's allocator and go back to `UnloadFontData`/`MemFree` on every refusal path, the atlas image is unloaded immediately after the upload, and the assembled `raylib_sys::Font` goes straight to `Font::from_raw` whose `Drop` is `UnloadFont`. Both lengths are taken from the slices, and `glyphPadding` is set to the same constant handed to `GenImageFontAtlas`, because `DrawTextCodepoint` grows the source rect by it |
| `runtime::font::flush_render_batch` | the on-demand atlases are built *inside* a begin/end drawing pair, which nothing else here does, and `rlLoadTexture` binds a texture behind the batch's back | One block wrapping `rlDrawRenderBatchActive`, which takes no arguments, writes through no caller pointer and only submits raylib's own batch. Belt and braces rather than a known defect, and it runs only on the handful of frames that rebuild an atlas |
| `runtime::halo` | the caption glow halo and soft shadow are blurred offscreen mid-frame, and raylib texture modes do not nest: `EndTextureMode` always returns to the *default* framebuffer (`rcore.c:1110-1131`), so the safe guard would end the export's supersampled texture mode behind its back and redirect the rest of the frame to the screen | Six blocks. The batch is flushed and the active framebuffer captured (`rlGetActiveFramebuffer` plus the width/height pair `draw::SceneViewport` already trusts) before any GL call; each pass runs through by-value `BeginTextureMode` calls, which flush and rebind everything they touch; and the caller's framebuffer is restored on every exit path — `EndTextureMode` **plus an explicit `rlSetFramebufferWidth`/`Height`** for the screen, because `EndTextureMode` restores the viewport through `SetupViewport` (`rcore.c:1109-1131`, `rcore.c:3537`) but never rlgl's cached size pair, or a reconstructed `BeginTextureMode` for a render target, which reads only the id and the colour texture's dimensions and sets all three size authorities itself (`rcore.c:1079-1108`). `LoadRenderTexture`'s zero-id failure is checked before `RenderTexture2D::from_raw` takes ownership (its `Drop` is `UnloadRenderTexture`), and no pointer crosses the boundary anywhere. The shadow's luminance-as-alpha composite (`halo_mask.fs`) is safe code throughout |
| `runtime::assist::env` | `std::env::remove_var` is `unsafe` in edition 2024, and E1 in `docs/ASSIST_PROVIDER_CONTRACTS.md` requires the app to take `OPENROUTER_API_KEY` out of its own environment after importing it, so no child — `ffmpeg`, `kdialog`/`zenity`, `codex`, a Python helper — can inherit it by accident | One block, inside `import_session_credentials`, which is itself an `unsafe fn` documenting the contract. `musializer-app`'s `main` calls it as its **first statement**, before the window, the audio device, `cli::parse` and any thread, so no other thread can be reading the environment concurrently. The value is copied into an owned `String` before the removal and nothing reads the environment after it |
| `app::scenes::ascii_field` `DefaultFont` | `scene_ascii_field.c:154-160` deliberately draws through raylib's built-in font rather than the caption face; the safe wrapper has no way to borrow `GetFontDefault()`'s handle without a crate-private constructor. **Shared with `app::scenes::phosphor_dream`**, which needs the same monospaced face — one type rather than two identical ones, so this stays a single island | One block. `GetFontDefault` returns a non-owning handle that exists for as long as the window does; the newtype never calls `UnloadFont` on it |
| `app::scenes::song_atlas` `Batch`/`LineWidth`/`color_to_hsv` | `scene_song_atlas.c`'s immediate-mode terrain draw needs `rlBegin`/`rlVertex3f`/`rlColor4ub`/`rlEnd`/`rlSetLineWidth` and `ColorToHSV`, none of which raylib-rs exposes on its own `Color` | Six blocks. `Batch`/`LineWidth` are RAII guards whose `Drop` closes what `begin`/`set` opened, so every call happens inside an already-open drawing context that `self` proves; `color_to_hsv` is pure arithmetic over a by-value colour |
| `app::scenes::orbital_lattice` `color_brightness` | `ColorBrightness`, which the safe raylib API only exposes for images, not colours | One block. Pure arithmetic over a by-value colour, no global state |
| `app::scenes::cadence` `glyph_alpha_at` | `cadence_glyph_alpha_at` (`:184-194`) reads a loaded TTF glyph's CPU-side bitmap so particles condense onto the letterform | One `unsafe fn`, documented with a `# Safety` section: the caller (`glyph_ink`) proves `data` is non-null with a supported format and positive dimensions before calling, and clamps `(x, y)` inside `width * height` first |
| `core::audio::sample_ring` `SyncCell`/`push`/`pop` | The realtime audio callback must push/pop with no allocation, lock, or syscall, so the ring's per-slot storage needs interior mutability without `Mutex` | `unsafe impl Send + Sync for SyncCell`, plus the two index-guarded writes in `push`/`pop`. Synchronisation comes from the acquire/release pair on `head`/`tail`: a slot is only ever written by the producer while the consumer is proven not to be reading it, and only ever read by the consumer while the producer is proven not to be writing it |

Do not add an `unsafe` block without a `SAFETY:` comment and a row here.

## Traps this rewrite has already paid for

- **`WAYLAND_DISPLAY` must be *set to a name that cannot resolve*. Unsetting it
  is not weaker isolation — it is none at all, and this entry told you to unset
  it until 2026-08-08.** `wl_display_connect(NULL)` reads the variable and, when
  it is missing, falls back to a **hardcoded `"wayland-0"`** (the literal is in
  `libwayland-client.so.0`; check with `strings`). This operator's socket is
  `$XDG_RUNTIME_DIR/wayland-0`. So `env -u WAYLAND_DISPLAY` resolves to exactly
  the same compositor as changing nothing, and `DISPLAY=:77` alongside it changes
  nothing either, because Qt and GTK prefer Wayland when they can reach it.

  Proven rather than reasoned: a probe that calls `wl_display_connect(NULL)` and
  disconnects (mapping no surface, so it is safe to run) **connects** under
  `env -u WAYLAND_DISPLAY DISPLAY=:78`, and is refused under
  `WAYLAND_DISPLAY=musializer-no-such-display`. `XDG_RUNTIME_DIR` pointed
  somewhere empty also refuses, which is the belt to that braces.

  Use `MZ_NO_WAYLAND`, which `tools/headless_check.sh` and
  `tools/lyric_lane_capture.sh` define and pass at every launch. Never write
  `env -u WAYLAND_DISPLAY` again, in a script or in a two-line shell test.

  **Why it survived so long, which is the transferable part.** The guard was
  wrong at 46 call sites while every capture passed, because the application
  itself reaches Xvfb through X11 and never asks for Wayland — so a broken guard
  is *invisible until something spawns a GUI child*. It was found when an agent
  hand-ran a click probe onto the ASCII **Import** row without the separate
  `PATH` guard that keeps `kdialog` off the search path, and file dialogs opened
  on the operator's real screen. Two independent guards, one of them dead for
  months, and only the live one was holding. `headless_check.sh` now opens with
  an **isolation self-check** that connects unguarded (to prove the probe works
  at all) and then asserts the guarded environment is refused; a guard nothing
  tests is a guard you find out about like this.
- **A second guard keeps GUI children unreachable, and it is per-call-site.**
  `ENTRY_PATH_OVERRIDE`/`ENTRY_NO_DIALOG_PATH` in `tools/headless_check.sh` strip
  `kdialog` and `zenity` from `PATH` for probes that press a control which opens a
  picker. It lives in a shell variable at one call site, so **anything not routed
  through `entry_capture` silently loses it** — which is exactly what happened
  above. If you hand-run a probe that clicks anything, take the `PATH` guard with
  it.
- **Do not invoke `kdialog` with no display reachable.** It aborts with `SIGABRT`,
  which on Ubuntu summons an Apport "internal error" report for a crash you caused.
  `tools/rusty-musializer-launcher` guards against this by checking for a display
  before it reaches for a dialog and printing to stderr otherwise; keep that guard.

- **Never give the Python helpers their own process group from the parent.**
  `os.setsid()` in `external_analysis.py` fails with `EPERM` if the caller is
  already a group leader, so calling `process_group(0)` on the child would kill
  the helper at startup. A test in `runtime::process::assist` fails loudly with
  that explanation if anyone changes it, and the `ESRCH` fallback from
  `kill(-pid)` to `kill(pid)` covers the race that leaves. Do not simplify it away.
- **Read the `.c`, not the header comment.** `core::scene::events` was written
  from `scene_event_merge.h`'s comment and was wrong in seven ways, including
  using OR where the C uses XOR to namespace ids — which is not injective and
  could have collapsed two distinct events into one. The comment was accurate
  about intent and silent about every edge case.
- **`take_screenshot` cannot write to a subdirectory.** raylib's
  `TakeScreenshot` runs its argument through `GetFileName`, so `build/x/y.png`
  lands in the working directory. Use `LoadImageFromScreen` + `ExportImage`.
- **rustfmt destroys data tables.** Anything checked column-by-column against C
  needs `#[rustfmt::skip]`, or it becomes one argument per line and stops being
  checkable.
- **A surface nothing photographs does not get reviewed.** The interface ran for a
  whole session in raylib's 10 px bitmap face, and every capture in
  `tools/headless_check.sh` passed a fixture, so the welcome screen — the first
  thing a new user sees — was never in a frame at all. Both were found by the
  operator, not by the checks. When you add a screen, add its capture; when you
  add an asset the interface depends on, print whether it loaded.
- **A control nothing presses does not get verified either, and hovering it is
  not the same check.** Three of the export panel's four SIZE buttons could not
  be clicked for as long as the panel existed. Every gate was green: `panel:`
  said the panel opened, the ink gate said it drew, and `--ui-probe hover=` lit
  the 720p button correctly at 100 % and 150 %, because a hover highlight comes
  from `contains_point` and `contains_point` was never the problem. The press was
  being cashed by a *different widget with the same id*. `--ui-probe click=XxY`
  exists for this, it holds the press for three frames because the claim rule
  drops a press and release that share one, and its gate section includes a click
  into the 8 px gap between two buttons — without that, a probe that pressed
  nothing satisfies every other assertion, since a no-op leaves the defaults.
- **A widget id namespace may only be minted in `widgets::id`, and only in its
  `ALL` table.** `panels/events.rs` picked `7` and `8` as bare literals with a
  comment promising to fold them in at merge; they became `EXPORT` and `SEEK`.
  The row draws first, so it silently ate three of four export sizes and four of
  the preset picker's own controls. **Three collision tests were green the whole
  time** — one each in `widgets.rs`, `events.rs` and `lyrics.rs` — because each
  enumerated a different hand-written list and none of them contained the
  colliding pair; `widgets.rs`'s had six entries and predated `EXPORT`. A
  collision test over a hand-maintained subset proves nothing about the namespace
  you just added, so the list lives beside the constants and a second test names
  every constant individually.
- **raylib-rs's `load_font_from_memory` miscounts codepoints.** It takes the glyph
  set as `&str` and passes `str::len()`, a byte count, as the count of `i32`s. Fine
  for ASCII, wrong for anything else. `runtime::font` calls the ffi directly for
  this reason; do not "simplify" it back to the safe wrapper.
- **raylib-rs's `Wave::load_samples` miscounts samples, the same way.** It builds
  its slice as `(pointer, frameCount)`, but `LoadWaveSamples` allocates
  `frameCount * channels` floats. For any stereo track the safe wrapper hands back
  **half the decoded audio** with no error, which in an export would spread the
  first half of the track across the whole timeline. `decode::wave_samples`
  calls the ffi directly and takes the length from the wave's format. Assume the
  next raylib-rs wrapper that returns a slice has the same defect until checked.
- **That assumption paid off: `Image::get_image_data` is the third one.** It forms
  its slice from `LoadImageColors` and never null-checks the pointer, so an
  undecodable image is a null dereference rather than an error. Three wrappers in
  this family have now been checked and all three were wrong, which is why
  `runtime::decode` exists as one place with all of them — the fourth belongs there
  too. Every length in it comes from the format the loader reports.
- **A fallback that looks like content hides an unwired feature indefinitely.**
  Three whole-track derivations — the timeline envelope, Song Atlas's terrain and
  the ASCII glyph grid — were drawn through an `Option` that was `None` at every
  call site for two whole bands. Nothing failed: ASCII Field drew its procedural
  spectrogram, Song Atlas its live idle ring, and the strip a flat lane, and each
  photographed as a perfectly plausible picture. Captures did not catch it and
  could not. A report line naming what the surface *had* is what catches it, so a
  scene that can draw without its data must say which one it did.
- **`EndTextureMode` is not the inverse of `BeginTextureMode`, and the missing
  half is global for the rest of the session.** `BeginTextureMode` moves three
  size authorities (`rcore.c:1079-1107`); `EndTextureMode` restores the viewport
  and the projection through `SetupViewport` and resets `currentFbo`, but leaves
  rlgl's cached `framebufferWidth`/`Height` at the *target's* dimensions
  (`rcore.c:1109-1131`, `rcore.c:3537`). Nothing later resets it — not
  `BeginDrawing`, only a window resize. So one caption glow blur into a 188x50
  buffer made `draw::SceneViewport` scale every later panel boundary by 0.15,
  which drew a scene panel with nothing in it, or pinned the GL viewport to a
  small rect at GL's bottom-left origin and squeezed the whole interface into the
  corner. Both are *coherent* pictures, the process exits 0, and the ports' own
  unit tests are green, so the only thing that catches it is a report line
  stating the invariant — `gl framebuffer:` — which the headless gate asserts is
  zero. **Assume every raylib `End*` pairs with fewer setters than its `Begin*`
  and check the C, rather than assuming symmetry from the names.**

## Before implementation work

Read `FEATURE_PARITY_PLAN.md` first. It is the only current task list and carries
the dependency order, acceptance evidence and complete C-to-Rust feature ledger.
Claim and update work there; do not create another completion plan.

Read `REWRITE_PLAN.md` only for historical design reasoning and NOTE ENTRIES about
work that already happened. Its phase sketches, source-ownership map and agent
handoffs describe the completed fan-out and are not current instructions. Add
historical investigation detail there only when a later session would otherwise
rediscover it; add every live task to `FEATURE_PARITY_PLAN.md`.

## The behavioral oracle

`../musializer` is read-only. It is frozen at commit
`9300af942bd00d8c85fc4e3c8c02cf2b6356764f` (`9300af9`) on branch `master` —
note `master`, not `main`.

- Never modify the C repository as a side effect of work here, including to
  "fix parity". If the oracle looks wrong, say so; do not edit it.
- Read `../musializer/CURRENT_FILE_POINTERS.md` before trusting any other
  document there. It marks which documents describe behaviour and which
  describe intent, and the difference has been expensive in that repository
  before.
- `EXTENSION_PLAN.md` is part roadmap with open decision gates, and
  `cadence-overhauls-2026-07-26.md` is an unimplemented scratchpad. Neither
  describes the frozen binary.
- `../musializer/AGENTS.md` is gitignored there and absent from a fresh clone,
  though it is present in the local working tree.
- The code and its tests are authoritative about behaviour. Documents are not.

### The C is legacy (operator decision, 2026-08-03)

The operator declared the C project superseded: it was never distributed, has no
users, and this rewrite is now the canonical Musializer. What that changes, and
what it does not:

- **Backwards compatibility with the frozen C is no longer a requirement.** The
  `.musi` format and schemas may gain fields and semantics the C cannot read.
  Bump `schema_version` when they do, and keep opening every file *this*
  application has ever written — the compatibility contract now runs against our
  own releases, not the C's.
- **The oracle remains read-only and remains useful** for behaviour not yet
  ported. Nothing about how to read it changes.
- **Existing differential harnesses stay green as regression anchors.** They pin
  behaviour we chose to keep, not behaviour we are forbidden to change. Diverging
  from one is now a decision, not a bug — but it must be deliberate: update the
  harness to pin the new behaviour and record why, never let one fail quietly or
  drift by accident.
- `FEATURE_PARITY_PLAN.md` stays the task queue for completing capability parity
  (features a user would otherwise lose in the switch). New features beyond the
  C's ceiling need no parity justification at all.

## Parity is the goal. A line-by-line port is not the method

The target is **feature parity with the frozen C**, judged by what a user or a
file can observe. It is not fidelity to the C's structure, and there is no
requirement that a feature arrive as a 1:1 translation.

This matters more the further the rewrite goes, and the shape is familiar to
anyone who has done a language migration by hand: **the oracle gets less
informative as you go.** It is at its most useful for pure logic — analysis,
layout policy, settings tables, file formats — which is exactly the part that
went first. What is left is what is most entangled with the things this rewrite
deliberately does not reproduce: `plug.c`'s single global `Plug *p`, hot reload,
tinyfiledialogs, and C idioms with no good Rust shape. Expect the last stretch to
be substantially invention rather than translation.

**So when a faithful port is impossible, unavailable, or would be bad Rust: find
the alternative and implement it.** Do not stall, do not leave a stub that
explains what the oracle does instead, and do not stop to ask. Decide, build it,
and record the divergence and its reason. This repository has been agent-driven
since the fork, and the operator's standing instruction is that the agent does
what it must to reach parity.

### What is negotiable, and what is not

The distinction is the whole rule, because "find an alternative" must never
become "reinvent the semantics".

**Negotiable — the mechanism.** How a thing is built is yours. Precedents already
in the tree:

| The oracle | Here | Why |
| --- | --- | --- |
| tinyfiledialogs through FFI | `kdialog`/`zenity` as child processes | No new dependency, same thing tinyfd does on Linux, and it fits `process`'s existing supervise-and-reap machinery |
| Fonts and shaders read from `./resources/` | `include_bytes!`/`include_str!` | The C's relative paths are why its launcher must `cd` first; an interface that loses its font to the wrong working directory is worth deleting, not reproducing |
| One global `Plug *p` | `main.rs` owns resources; the shell returns `ShellCommand`s | Makes the shell drivable in a test, and is why there is no `Rc<RefCell<_>>` in this codebase |
| `GuiSetFont`-style implicit face | `Face` threaded explicitly | Makes the fallback reachable in a test |
| Hot reload | Not reproduced at all | An explicit first-pass non-goal |
| — | `--probe-frames`, `--probe-shot`, `--probe-reopen`, `--ui-probe hover=` | Invented, because nothing in the oracle can drive a headless check |
| Six text buttons in the transport row | Eleven icon controls, each with a tooltip and a text fallback | Operator request. Icons are square, so the row carries volume, fine seek and a readout toggle in less space than the six labels used |
| No volume control (only `--mute` at startup) | Mute button and slider on the transport row | Operator request. `--mute` now sets a *flag* rather than the device volume, so the button can undo it |
| `Full` hides the panels | `Full` also toggles the OS window | Operator request. Guarded off in probe runs: Xvfb has no window manager to restack the window, so a capture would photograph a size it did not ask for |
| The diagnostic readout is always drawn | Off by default; `H`, the row's button and `--hud` turn it on, and probe runs default it on | Operator request. It is a developer HUD, and a capture that carries its own evidence is why the line exists |
| One 64 px caption atlas, magnified to whatever the style asks for | An at-size atlas per drawn size, quantized up to a multiple of 8 and capped at 256 px, two cached | Operator request, 2026-08-03. A caption is drawn at `max(20 * pixel_scale, boundary.height * size_scale)` — past 400 px with the panels hidden, and further again under export supersampling — and a magnified bitmap atlas is a blur the C never had to answer for because nothing photographed it. Built over the codepoints the cue actually uses, so the atlas is 10--30 ms rather than the seconds the full 1,770-codepoint set would cost at that size |
| Cadence typesets from the same bitmap atlas | A signed-distance-field atlas and a first-party fragment shader (`resources/shaders/glsl330/sdf_text.fs`) | Operator request, 2026-08-03. Cadence animates per-glyph scale continuously, so *no* raster size is right for the whole animation. The particle field still samples the bitmap atlas's glyph images — an SDF glyph is a distance ramp, and the C's 96/255 ink threshold applied to one would scatter particles around the letterform instead of onto it |
| The rounded plate outlined by a sharp `DrawRectangleLinesEx` (`plug.c:1281-1285`) | `DrawRectangleRoundedLinesEx`, tracing the fill's own roundness | Operator request, 2026-08-03: a rounded box in a rectangular outline, called out by name. Roundness itself is now authorable (`effects.plate_roundness`, default the C's 0.12) |
| Two fixed swatch rows for INK and PLATE | The swatches plus a hand-built free picker (SV square, hue bar, alpha bar) | Operator request, 2026-08-03, overriding the C's "few and opinionated" stance. The format always stored arbitrary RGBA; only the picker was missing |
| No caption effects | `caption_style.effects`: audio-drivable glow (strength/radius/colour, RMS/bass/beat/flux/time pulse and hue drives), soft shadow, plate roundness | Operator request, 2026-08-03 — the first `.musi` extension past the frozen C. Resolution is pure per-frame math in `core::project::caption_effects`, so exports reproduce the preview's pulse exactly; a default block is never serialized, keeping pre-effects files byte-identical |
| The glow drawn as 17 additive re-draws of the caption offset in two rings (`GLOW_TAPS`) | An offscreen two-pass separable Gaussian over the glyph coverage (`runtime::halo`, `resources/shaders/glsl330/halo_blur.fs`), composited additively at reduced resolution | Operator feedback, 2026-08-04 (UX0-C11): finite taps at a growing radius resolve into visible discrete copies of the text — "gravitational lensing" — and 100 % strength read thin. A blur widens one halo instead; the buffer is sized from the drawn font size and radius in pixels, so export supersampling scales it with everything else, and it stays in luminance (white-on-opaque-black) so premultiplied edge bleed is unstateable. The blur runs with no scissor active, which is why the preview's scene clip moved from `main.rs` into `SceneRenderer::draw` |
| The soft shadow drawn as 9 normal-blended re-draws (`SHADOW_TAPS`) | The same `runtime::halo` blur, composited through `halo_mask.fs` — the buffer's luminance becomes coverage alpha in the shadow colour under normal blending | Operator-approved follow-up to UX0-C11, 2026-08-04: the tap table had the identical discrete-copy pathology, hidden only by a dark-on-dark fixture. A shadow is an occlusion, not a light, so the additive composite is wrong for it; the mask shader converts the one blurred buffer instead of blurring twice differently. `shadow_blur` 0 keeps the legacy single hard copy byte-exactly (pinned by the gate's hard-versus-legacy zero delta), and the shadow keeps its legacy decision of clipping to the caption box, unlike the glow, which spills on purpose |
| Five route sources (`project.h:60-67`) | A sixth, `time` — the same eight-second triangle clock as the caption Time drive, one definition in `scene::routes::time_triangle` | Operator request, 2026-08-04 (UX0-C15). Additive: every C-era token keeps its meaning, the route differential harnesses stay green, and the C simply cannot read a `time` route (recorded in `PHASE0_INVENTORY.md`) |
| Caption effect drives are bare choices | Each drive carries an optional `DriveTuning` — quiet/loud in→out windows, curve, clamp — edited in a mapping editor with a live meter and transfer graph | Operator feedback, 2026-08-04 (UX0-C14): the Tune inspector's mapping layer was the missing depth. The arithmetic is `ParameterMapping::evaluate_mapping`, factored out so caption tuning and scene routes share the pinned semantics; the editor is built from `ui/mapping_editor.rs`, the same componentry the route editor now draws with. A hue drive on an achromatic base also blends saturation (and value, for dark bases) in with the drive (UX0-C13), so a white or grey glow sweeps into colour instead of silently ignoring the control |
| — | BLUR/SHADE/ROUND drawn beside BACKING on the Style pane, not under "Effects"; tooltips across the caption panes and the Tune editor | Operator feedback, 2026-08-04 (UX0-C12, C16): backing softness is styling, not an effect, and the tooltip path (`widgets::hint`) already existed — the styling controls just never called it |
| One kind of lyric cue, all drawn in one amber | `CueOrigin` on every cue — user applied / AI certain / AI ambiguous / potential — colour-coded in the lane with a legend and tooltips | Operator request, 2026-08-06 (LX1). The C cannot tell a line placed by ear from one an aligner guessed at, so after an assist run nothing on screen said which placements to check. `at_time` refuses to hand a `Potential` cue to a frame and `cue_shadow` ignores proposals both ways, so provenance is load-bearing rather than decorative |
| A line the localizer could not place is dropped | It becomes a `Potential` cue parked at its coarse proposal, editable in the lane | Operator request, 2026-08-06 (LX1-f). The proposal window was parsed, bounded and drawn nowhere — the same defect class as a lane that never reaches a frame. A line with **no** proposed time stays review-only, because a block at 0:00 is a lie |
| Overlapping cues draw as one full-height rectangle | A cluster with three live cues fans into rows, each step 20 % darker, brightest on top | Operator request, 2026-08-06 (LX1-d). Row 0 is the cue `at_time` will actually resolve to, so brightness tracks "what you will see" rather than document order |
| A fixed 22 px cue lane | 50 px (2.25x), dragged from its bottom edge to 66 px (3x), persisted in `~/.config/musializer/ui.json` | Operator request, 2026-08-06, revised upward the same day after seeing 1.5x on a real track: the whole lane height is the click target, so the constant *is* the affordance. A drawn grip was asked for in the same message, reversing the original "no visible border" instruction — the first version was invisible in every frame because it was painted before the lane's own fill, which is why the resize read as broken. The ceiling is derived from what the editing form and the sidebar floor still need: 46 px at 640, 50 from 720, the full 66 from 791 up |
| The zoom readout sits between the waveform and the cue lane | It draws below all three timed lanes, and `open_panel` reports where | Operator request, 2026-08-06 (LX1-c). Band chrome is unchanged to the pixel, which the existing compile-time assertion still checks |
| A cue's boundary handle is a 2 px line, drawn only once the pointer is already inside the 5 px grab zone | Both ends of a hovered block show a grab band at the hit test's own width, the one under the pointer darkens and takes a `RESIZE_EW` cursor | Operator feedback, 2026-08-06 (LX2-a): the handles could only be found by zooming in until a block was wide enough to stumble onto one. An affordance you have to be standing on to see cannot be found by a pointer that never lands there — the same defect the lane's own resize had. The geometry is `hit_test`'s `true_left`/`true_right` and its two visibility flags, so a handle can never be painted where a press would not take it |
| A cue block is a bare coloured rectangle | It carries the start of the cue's own text, typed through the authored face, fading out before the block's edge | Operator request, 2026-08-06 (LX2-b). The 2.25x lane bought room for a click target and the same room reads as a label. A fade rather than an ellipsis, which would cost three characters of the little that fits, or a hard cut, which reads as a rendering fault. `widgets::draw_text_faded` is generic over a `TextFace` so authored text can use it — the chrome bank is Latin-only, and a cue drawn through it is UX0-A05 again |
| The wheel zooms only over the PCM strip | All three timed lanes take it, through `Shell::request_timeline_zoom` | Operator request, 2026-08-06 (LX2-c). They are one axis, and the lane a user is aiming a 2 s cue in is the one they want to zoom from. First claim of the frame wins, because one notch is reported to every caller and two acceptances would multiply the factor |
| — | `--ui-probe wheel=NOTCHES`, delivered on one frame wherever `hover=` parked the pointer, and a `timeline:` report line | Invented, LX2. Xvfb has no wheel any more than it has a pointer, so the binding above was unphotographable; three modules draw the three lanes against three rectangles, which is where a region test is wrong in a way that reads correctly in the source |
| Selecting a scene makes it the base scene and switches the retained plan off (`track_select_base_scene`, `plug.c:963-977`) | While the plan is enabled it retargets **one segment** — the one selected in the lane, else the one under the playhead — and the plan keeps driving | Operator bugs, 2026-08-06 (LX3-a). The side effect was two reports in one: the plan stopped, so the whole track previewed one scene, and with no cue driving every tuning edit fell through to the track-wide per-scene-kind table. `--scene` on the command line is *not* routed here; the CLI grammar is a documented contract and `--scene X` still means "start on X" |
| The Tune header names the scene | It names what a slider will move: `segment 3 of 6 (01:30.500)`, `base scene - 6 segments are paused`, or `base scene - this segment captured no tuning` | Operator, 2026-08-06 (LX3-b): *"I'm not sure which one gets the tuning applied to"*. A plan on screen and a base-scene edit is a state the interface had no sentence for |
| — | `--ui-probe scene-pick=NAME` and a `scene segments:` report line | Invented, LX3. Nothing headless could press a scene tile, so "the plan survived the click" and "the click switched the plan off" produced the same picture — both leave the picked scene on screen at that playhead |
| — | `--ui-probe click=XxY`, plus `export config:` and `click probe:` report lines | Invented, EX1. `hover=` proved a control lights up; nothing could prove it *takes* the press, and that is exactly where three of the export panel's four SIZE buttons had been dead since the panel was written. Injected at `Widgets`' pointer seam over three frames (press, hold, release), because raylib exposes no way to synthesize a button and the claim rule drops a press and release that share a frame |
| Four resolution buttons, all 16:9 (`render_export.c:31-32`) | An ASPECT row beside them: 16:9, 9:16, 1:1, 4:5, with the rung naming the **short** edge | Operator request, EX2. The pipeline already accepted any even geometry and `--resolution 1080x1920` already worked; only a way to ask for it was missing. The short-edge reading makes 16:9 byte-identical to the C's own table and makes "1080p" mean the same amount of picture in every shape |
| The caption sizes and margins from `boundary.height` (`plug.c:1219-1307`) | From `min(width, height)`, and its line cap preserves text *area* rather than line count | EX2. The C is right for its own frames — the height *is* the short edge on all four of its presets — and wrong for a 9:16 one, where it gives 78 % larger type in a 3.1x narrower measure and the three-line cap then discards ~60 % of a normal lyric. Landscape is unchanged as an equality, not as a claim |
| ASCII Field's live grid is a fixed 80x42 (`scene_ascii_field.c:260-261`) | The cell edge derives from the frame's short axis and the counts fill it | EX2. 80x42 min-fitted into 1080x1920 draws a band across 28 % of the height. The 1920x1080 MP4 is byte-identical before and after — same md5, not "looks the same" |
| The preview panel *is* the frame | The preview is framed to the export's aspect, with a one-pixel edge, and `preview frame:` reports both rects | EX2. Once an export can be 9:16 the panel stops being a picture of the output, and a user places captions against a shape their file will not have. It costs preview area at every aspect — with a bottom panel open the band is routinely 3:1 — and buys the only honest answer. The edge rule exists because a capture showed near-black scene against near-black surround with no visible seam |
| The route affordance is a `~` (`plug.c:6169-6186`) | A word — `Route` / `Routed` / `Editing` — with a *dynamic* tip naming the route it would open | Operator plan UX0-B08, 2026-08-07 (PX6). A tilde abbreviates nothing and has no meaning in an audio interface, so its tooltip was the only thing in the application that explained it — and a tooltip is a thing you have to already suspect before you hover it. A word is readable with the pointer parked somewhere else. Apply and Remove, when disabled, gained a hit target of their own purely so `hint` can say *why*: `disabled_button` returns no `ButtonState`, so a reason had nowhere to hang |
| The readout is text; a value moves only by dragging its slider | The readout is a chip that opens a text field (Enter commits, Escape reverts), the wheel over a row steps one unit of the descriptor's `precision` and Shift ten, and the label resets that one setting | Operator plan UX0-B09, 2026-08-07 (PX6). Clamping is load-bearing rather than cosmetic: `SceneSettings::set` **rejects** out-of-range values rather than clamping (`scene_settings.c:143-149`), so a raw typed 99 would vanish with no message at all. A leading `*` marks a setting moved from its default, which is also the answer to "what have I changed on this scene". A new `TextEntrySurface::TuneValue` suppresses global shortcuts while typing — the same registry UX0-A06 was fixed by |
| A slider drag writes an arbitrary float; only `precision == 0` rounds | Every value this panel writes is snapped to the descriptor's own `precision` | PX6, and **the one change here a `.musi` file can see**. The readout prints two places, so before this the number on screen was a rounded picture of the number in the file and a typed 1.23 differed from a dragged "1.23". `differential_settings.sh` covers the descriptor table, not slider output, and stays green untouched |
| A preset Apply overwrites the tuning, with no way back | Any exploratory gesture opens an audition: A/B, Revert and Keep, with Revert **bit-for-bit** | Operator plan UX0-C04, 2026-08-07 (PX6). An explicit snapshot A/B rather than hold-to-audition, because a held button cannot be compared against *while it is held* and what a user wants is to flip back and forth with the track playing. A `SettingsSnapshot` is 12 `f32`s copied back — the same operation loading a cue performs — so the revert is exact rather than close. The session is keyed to `(track_slot, scene, cue)` and ends when the target moves, so a base-scene snapshot can never be written into a segment it was not captured from (LX3) |
| — | Nudge and Surprise: bounded randomize, inside every descriptor's own range | Operator plan UX0-C07, 2026-08-07 (PX6). Randomness is **injected** (`RandomSource`, a seeded `SplitMix64`), never ambient, so `--ui-probe tune-seed=` makes a random feature photographable and the bounds are swept over 200 seeds x 10 scenes rather than over whatever a lucky run produced. Biased to be worth pressing: snapped to precision, 5 % clear of both ends (where the degenerate looks live), 75 % of sliders moved so the scene stays recognisable, toggles flipped 25 % of the time. Sliders draw triangular about the descriptor default; the five `-180..180` angle controls draw uniform, because hue is circular and has no designed centre to pull toward |
| — | `--ui-probe tune-seed=`, `tune-explore=`, `tune-type=`, plus `tune values:` and `tune entry:` | Invented, PX6. `click=` presses one control per run and is the only thing that proves a control is wired at all (EX1); it cannot state a claim about a *sequence*, and every UX0-C04 claim is one. The gate cross-checks the two: the same seed through the Surprise **button** and through the sequence probe must give byte-identical `tune values:`. That line prints shortest-round-trip floats, not the readout's two places, because "restored it exactly" and "restored it to two decimals" are different claims |
| No history of any kind; the welcome screen's right column is empty | A recent-project list there — name, relative age, folder, one click to reopen, a forget cross per row — persisted in a **second** per-user file, `recent.json` | UX0-C06. A second file rather than a field in `ui.json` *because* both stores are refused wholesale when they do not parse: folding them together would make one truncated write cost the operator their splits **and** their history. Three states are drawn, not two — no history, an unreadable history, and a history whose files have moved are different facts, and a blank column is indistinguishable from a broken one. `--project` on the command line records an entry only in a session run (`is_session_run`), or `tools/verify.sh` would append a dozen scratch fixtures to the operator's real config as a side effect of being run |
| Every dropped file is handed to the audio decoder | `classify_drop` sends `.musi` to project open, PNG/JPEG/BMP to ASCII import, everything else to audio — and the failure notice names the type that was *attempted* | D1. The else branch is still the oracle's (`plug.c:7559`) rather than a whitelist: the list of formats raylib can open is raylib's to know, and a fourth "unsupported" arm would start silently refusing files that work. The match is **case-insensitive**, which `IsFileExtension` is not — a `.PNG` off a camera is unambiguously an image, and sending it to the audio decoder to be reported as a corrupt song is a defect, not a contract |
| — | `--ui-probe drop=PATH`, and a `drop probe:` report line naming the branch the shell dispatched | Invented, D1. Xvfb has no drag-and-drop, so a three-arm branch on a file extension was unreachable from every check this repository has — the exact shape of EX1's SIZE row. The line records what `dropped_files` *did*, not what a reporter recomputed, so it cannot read green while the branch is dead |
| ASCII imagery is reachable only from `--ascii-image` and a drop | An Import row and a Clear row under the scene tiles, Clear drawn only when the current track owns an image-backed grid | D2. The footer reserves **both** heights either way, so importing an image cannot make ten scene tiles jump a row and clearing it cannot make them jump back. Clearing drops one `Option`, so path, digest, cells and dimensions cannot part ways — "together" is structural rather than a discipline four assignments have to keep |
| A cue-list row click selects and never seeks (`lyrics_editor_ui.c:1255`) | It selects **and seeks to the cue's start** | PX2 (UX0-B02). Checking a line meant reading its timecode off the row and scrubbing to it by hand, and the review counted ~360 precise clicks to time 60 lines. The start rather than the middle, so pressing play immediately answers "is this on the beat" |
| No way to stamp a time at all | `LyricTap`: arm a run, then one **Enter** per line, each press closing the line before it and opening the next at the playhead | PX2 (UX0-C03). The pairing is review 1.14's rule applied as a gesture rather than as a default, which is what makes a run of taps produce contiguous captions instead of cues with whatever durations they arrived with. **Enter by elimination**: `T`/`M`/`S`/`F`/`H`/Space/Tab/arrows are all shell globals fired unconditionally once no field has focus, and the shell reads the keyboard before any panel draws — a panel key shadowing one fires both. The cue id order is snapshotted at arming time, because a stamp *moves* a cue and the document sorts by start: a cursor walking canonical order hands out line 2 twice and never reaches line 1 |
| `begin_new` focuses the text field, and a focused field stands every transport key down | Tap mode and text entry are **mutually exclusive states** — arming blurs the field, focusing a field disarms the run | PX2 (UX0-B02). The review's finding was that the natural add→type→play→tap loop broke exactly at the tap. Rather than carve an exception into the focus rule, there is no key that means two things, so there is nothing to get wrong |
| No undo anywhere; Delete is one unconfirmed click | `LyricHistory` — 64 whole-document snapshots, one per drained batch | PX2 (UX0-B03). A 3 px accidental lane drag committed a 64-cue `ShiftMany` with no way back. **Snapshots rather than inverse edits, deliberately**: `split` allocates an id, `merge` destroys one, and `update`/`retime` overwrite the `CueOrigin` that `at_time` reads, so an inverse log that misses one yields a valid document quietly different from the user's. `revision` deliberately does not round-trip — an undo *is* a change |
| A fixed 0.1 s nudge pair per time row, no repeat (`:219-250`) | The transport's Ctrl/Shift ladder at a tenth of its scale (0.01/0.1/1.0), hold-to-repeat counted in frames, and labels that state the step the modifiers currently mean | PX2 (UX0-B04). A cue boundary is placed against a syllable and a playhead against a section, so the *shape* is shared and the numbers are not — a user who learned Ctrl on the transport does not learn it twice. A fixed "-0.1" label lies whenever Ctrl is down. Frames rather than seconds because a headless probe has no wall clock worth trusting |
| Times shown to the millisecond and typeable only in tenths | The readout is a text field; `parse_cue_timestamp` refuses rather than guessing | PX2 (UX0-B04). `-1:30`, `1e3`, `+30` and `1:60` are all refused, because a silently-clamped typed time is a control that lies about what it took. The parser is in `core` and the formatter in the app, so a round-trip test pins the pair |
| `lyrics_split`/`lyrics_merge` ported, tested and with no call site | Ctrl+B / Ctrl+J and two buttons; split divides the text at the space nearest the seam's position through the line | PX2 (UX0-B05). Splitting a long imported line is the commonest subtitle edit there is, and the only route was retyping both halves. Never returns an empty half, because `validate_text` refuses empty text and a refusal *after* the user committed is worse than a rough guess |
| Export/Import offered unconditionally (`:1084-1140`) | Export is disabled for a document `bridge_export` would refuse, and import is transactional through `import_bridge_document` | PX2 (D3). A dialog that ends in an error is worse than a button that says it cannot. Import re-bases the file's **own** duration onto this track's rather than adopting it — adopting would silently re-length the destination and put every cue past the real end out of the timeline's reach |
| The cue lane exists only while the lyrics editor is open | It draws in every state except Export and Assist, in slack the band already reserved | PX2 (D4). Timing is judged against the waveform and the scene plan, which stay on screen when the editor does not — so closing the editor to see more preview took the cue blocks with it. Growing the band instead would move `workspace_layout`'s sidebar guarantee at 720p, which is the arithmetic that already forced the manual event row out of the lyrics band |
| — | `--ui-probe lyric-tap=N` and `lyric-undo=1`, plus `tap …, last stamp …, history u/r, typing …` on the `lyrics:` report line | Invented, PX2. Xvfb has no keyboard any more than it has a wheel. An armed run and a disarmed one differ by one 11 px line of hint text; a stamp that landed leaves exactly the frame a refused one does, because the difference is in the cue spans. Both defects the first capture found — a tap leaving the draft spuriously dirty, and the undo probe racing the drain — were invisible to every unit test and to the picture |
| An event marker is coloured by type and says nothing about which lane it came from (`plug.c:3086-3100`) | Type keeps the colour, and the **lane is the head's shape** — filled disc for manual, ring for semantic, with a tooltip naming both | D4. The two axes cross, so the C's one channel cannot carry both: the manual event row's `+ Feel` button records `EventType::Semantic` into the **manual** lane (`plug.c:2897`), so an amber marker may be either. After an Assist run the question a user has is "did I put that there, or did the model?" — the same question `CueOrigin` answers in the cue lane (LX1). Shape rather than a second colour, because a second colour would have to fight the four type colours for the same channel and would make a lyric marker and a cue marker indistinguishable, which is the information the C chose to show |
| Only the wheel and a middle-drag move the timeline view | **Shift**-wheel pans it too, across all three timed lanes, through the same `request_timeline_zoom` seam | D4. One notch is a fixed 15 % of the *visible span*, so the gesture keeps its feel at every zoom — the same argument the 1.2x-per-notch zoom makes multiplicatively. Routed through the shell rather than each call site, which is what gives the lyric cue lane the gesture without that file changing |
| The export always covers the whole track; a window exists only as `--render-window` on the command line (`render_export.c`) | A CLIP row above SIZE — `Full track`, `In <- playhead`, `Out <- playhead` — over `core::timing::render_export::ClipSelection`, driving the existing windowed `RenderPlan`; `--render-window` now seeds the panel's clip so flag and panel are one state | PX3 (UX0-C01), post-legacy extension. `In` alone selects to the end of the track, so the common "from the drop onward" teaser is one click. `suggest_clip_path` names the file after the window so a teaser cannot overwrite a full render. Session-only: whether the clip belongs in `.musi` is an open operator decision recorded in the PX3 plan section |
| No still-frame output of any kind | `Save still`: the playhead frame as a PNG through the same offline renderer an encoded frame uses (`with_export_frame`/`draw_offline_frame`, shared with `ExportSession::step`) | PX3 (UX0-C10), post-legacy extension. One renderer rather than two, so analyzer feed, beat phase, scene plan, routed settings and the EX3 linear-light resolve cannot diverge between the still and the video — proven at 45.55 dB against the MP4's own frame, and the check earned its keep by catching the still shipping vertically mirrored (`rlReadScreenPixels` is bottom-row-first) |
| — | `--ui-probe save-to=PATH`, plus `export clip:` and `export still:` report lines | Invented, PX3. A headless run cannot answer a destination dialog, and "the CLIP row took the press" and "it drew a highlight" are different observations |
| — | `--ui-probe middle-drag=FROMxTO` and `wheel-shift=0|1`, plus `gesture=` and `markers=` on the `timeline:` line | Invented, D4. `click=` cannot reach the pan — it goes through `Widgets`' pointer seam and the pan reads `MOUSE_BUTTON_MIDDLE` from raylib directly, since it claims nothing from the bank. And a **stranded pointer claim is invisible in a picture**: the view sits where the hand left it either way, and the symptom is the *next* interaction misbehaving. `gesture=none` is the only way a capture can say the release was taken |
| Ten scenes, `COUNT_SCENES == 10` | **Eleven.** Phosphor Dream (`phosphor`, id 10) — a generative ASCII screensaver: ten procedural fields cycling on a dwell clock, dithered crossfades between character alphabets, a CRT of bloom, colour split, scanlines and a rolling refresh band | Operator request, 2026-08-08, adapted with permission from a third party's Python offline renderer (`OUTSIDE-DROPS/`). The first scene here with **no oracle at all**, so its evidence is its own tests rather than a diff. Appended at id 10, never inserted, so every C-era id keeps its meaning and every earlier `.musi` still resolves its scenes. `SCENE_COUNT` is 11 and `ORACLE_SCENE_COUNT` is 10; the harnesses read the second |
| — | Two-section differential dumps: the C-era scenes diffed against the frozen C byte-for-byte, then a `--- post-legacy (no oracle) ---` marker, then rows diffed against a checked-in expectation | The eleventh scene made three harnesses fail for a reason unrelated to what they test. Splitting rather than relaxing keeps *both* halves a contract: a C-era bound still fails against the C, and a post-legacy one fails against a file somebody has to update on purpose. `settings` and `route_persistence` split; `preset_store` instead scopes its **fixture** to `ORACLE_ALL`, because its divergence is interleaved inside one JSON line and a marker cannot cut it — the coverage that gives up is taken back by a named unit test |
| The shade blocks `░▒▓█` as font glyphs | 4x4 dither patterns drawn from geometry (`SHADE_PATTERNS`) | raylib's built-in face is Latin-1 and stops; the whole `blocks` alphabet is above U+2500. Bundling a monospace TTF is a new asset and licence for ~20 glyphs, and a bitmap glyph magnified under export supersampling is the blur the caption work already answered for once. The **dither** specifically, rather than a flat rectangle at matching coverage: a capture of Plasma drawn with flat rectangles came out as three featureless teal blobs, correct in coverage and no longer recognisable as a grid of characters |
| The source's filmic rolloff, applied to `frame + 0.9 * glow` | Not reproduced | Reaching the bloomed signal needs a second full-frame render target. Running the same curve on the bare cell value is not a cheaper approximation of it — it is a 40 % dimmer, which a capture caught. Additive blending saturating against white does approximately the same job at the top of the range |
| The source surfaces six hardcoded words out of the field | The `titles` toggle surfaces the **track's own lyric cue**, through a built-in 5x7 face, and defaults **off** | This application has authored cue timing and a document behind it. Putting a canned `BREATHE` over somebody's track is not a thing it should do on their behalf, and two word layers fighting for one frame is the wrong default even though the effect is worth keeping |
| — | `phosphor dream:` report line — field, alphabet, grid, `amp`/`bass`, mean and peak cell, bloom outcome | Invented. This scene draws a plausible frame in several wrong states: the wrong field, a grid collapsed to its floor, a bloom that never built, an audio coupling reading zero. None are distinguishable in a capture, and two of them were found by reading this line rather than by looking at the picture |
| — | `--protocol PATH` and a `*.protocol.json` drop arm: a **human-feedback protocol** — timed questions over a named track (path **and** sha256), items as bottom-anchored timeline pennants, a keyboard-first question card, answers appended to `*.answers.jsonl` as they land | Operator proposal, 2026-08-08 (HX). The loop it replaces — agent writes prose, operator holds it in their head, reports back in chat — cannot *blind* a comparison: a plan section naming "current" vs "proposed" has already unblinded it. The application is the only party that can apply variant `a` or `b` without saying which. `core::feedback` is the schema and it is strict in the `.musi` codec's sense; the wrong audio is refused by digest, like an ASCII asset. Quitting mid-session loses nothing: every answer is flushed before its handler returns |
| — | An A/B item's two tunings are shown only as "first look"/"second look", opener seeded per item; the exact variant order the app played is recorded **in the answers file at answer time**, never on screen | HX's blinding contract, structural rather than disciplinary: the card cannot name `a`/`b` because the drawing code never learns which is which beyond the order token it prints nowhere. The unblinding survives on disk for the agent that wrote the protocol |
| — | `--ui-probe protocol-flip=ID` / `protocol-answer=ID:CHOICE[+ID:CHOICE]`, and a `protocol:` report line | Invented, HX-5. Xvfb can neither hear the track nor press `2` — and items are addressed by **id, never by pixel**, because GX-1 is what a pixel-addressed probe costs. The gate answers two items headlessly, reads the JSONL back, asserts the recorded variant order equals the claimed one, and proves a wrong digest refuses with a nonzero exit |

**Not negotiable by accident — anything a user or a file can observe.** Since
the 2026-08-03 legacy decision these may change *deliberately* (with a schema
version bump or an updated harness and a recorded reason), but an unplanned
difference in any of them is still a bug, not a design choice:

- The `.musi` format and every schema in `docs/PHASE0_INVENTORY.md`. A project
  saved by any earlier build of *this* application must open here and back
  again; the C-interchange requirement is retired.
- Analysis numbers. The analyzer, beat tracker and band layout are checked
  against the C numerically; a "nicer" formulation that shifts a band is wrong.
- Settings semantics: descriptor keys, bounds, defaults, precision and clamping.
  81 descriptors, verified column-by-column, and a mistyped bound surfaces much
  later as a scene quietly ignoring a saved value.
- Export determinism. The same project must produce the same frames.
- The CLI grammar, including flag order effects and exit status.
- Anything with a differential harness. If a harness exists, it is the contract.

The test to apply: **if the difference is visible in a `.musi` file, a rendered
MP4, a number, or a documented command line, it is a parity bug. If it is visible
only in the source, take the better option and write down why.**

### The transport row is the first thing here that is not a port at all

Four operator-requested additions landed together, and they are worth reading as
one decision rather than four: an icon row, a volume control, tooltips, and fine
seek. The oracle has none of them.

**Icons cost discoverability, and it is paid for in two places rather than waved
at.** Every control carries a tooltip naming it *and its keyboard shortcut*
(`ui/icons.rs`), because once the labels went the tooltip became the only place a
binding is written down — there is a test asserting every bound control names its
key. And every control carries a **text fallback**: the icon face is the one face
whose absence cannot be approximated, since raylib's default has no Private Use
Area coverage and an icon drawn through it is an empty box. `Faces::icons_available`
is the seam, `describe()` reports which interface is on screen, and the headless
gate fails if the atlas silently stopped loading.

**The layout arithmetic is in `core::ui::transport_bar`, raylib-free, for the usual
reason** — and it earned that immediately. Its width sweep caught a defect no
capture would have: with the volume slider falling back from 96 px to 64 px,
greedy placement made **mute vanish and then reappear** as the window narrowed,
because dropping the slider freed enough room for it. The fix is to choose a whole
configuration from a ladder whose widths strictly decrease, which makes the defect
unstateable rather than fixed. The property to assert is *monotonicity in width*,
not "X is shed before Y": monotonicity is what makes resizing feel stable, and a
lucky ordering cannot satisfy it.

**A hover state that nothing can photograph is a hover state nobody reviews.**
A headless run has no pointer — `GetMousePosition` is the origin forever — so
`--ui-probe hover=XxY` parks it and the probe zeroes the tooltip dwell. Without
that, tooltips would have joined the welcome screen and the three `None` fallbacks
on the list of things this repository shipped unreviewed for weeks. The gate
measures the tip rather than trusting the exit status: peak luma inside the box
where the text should be.

**The readout HUD is off by default and probe runs turn it back on.** It is a
developer HUD, and leaving a frame counter over a music visualiser is the
interface addressing the wrong audience — but a capture that carries its own
evidence is the whole reason the line exists, so the default is context-dependent
rather than a constant. `--hud=0|1` overrides either way, and the gate checks all
three states by measuring the preview's top-left luma: 21 when clean, 236 when
drawn.

Two smaller things, both found by capture rather than by test:

- **A slider knob is centred on its value, so at 0 and 1 half of it hangs outside
  the rect.** At 960 px with the inspector open the full-volume knob touched the
  fullscreen button beside it. `widgets::SLIDER_KNOB_RADIUS` is public so a caller
  packing a slider against a neighbour can inset by it.
- **`--ui-probe hover=1121,449` silently fails**, because the spec is itself
  comma-separated. It is `hover=XxY`, matching `size=WxH`, and there is a test
  pinning that — the warning it produced named no key, so it cost a capture
  round-trip to diagnose.

### Missing beats pretending

A feature that is not built yet says so **in the interface**, by name, where the
control would be — see the disabled buttons and `ShellCommand::NotImplemented`.
A blank region is indistinguishable from a broken one, and a control that
silently does nothing is worse than one that explains itself. This is also what
makes an unfinished area show up in a capture instead of in a bug report.

## Persistent file storage (operator decisions, 2026-08-04)

- **Downloadable model weights** (aligners, separators, ASR): the app lets the
  user choose the directory. Default is `<install dir>/models/`; if that is not
  writable, fall back to a `musializer` directory in the user's home. Never a
  location the user was not shown.
- On this dev machine, research model downloads live under
  `~/.local/share/musializer/models/`, one subdirectory per model. The existing
  `~/.local/share/musializer/lyrics-align/` venv stays as is and nothing is ever
  installed into it.
- **Remote-provider credentials**: no OS/vendor wallet — kwallet/Secret Service
  rejected 2026-08-04 for cross-platform reasons. Persist to a 0600-permission
  credentials file under the per-user config directory (the `gh`/`aws` model),
  with session-only storage and env-var import as alternatives. Never in `.musi`
  files, preference JSON, argv, logs, analysis artifacts, or a repository `.env`.
- **Non-secret preferences**: versioned, atomically replaced, per-user config
  directory (XDG on Linux).

## Use the GPU for exports you are only going to look at (2026-08-08)

This machine has an RTX 3090 and FFmpeg has `h264_nvenc` and `hevc_nvenc`. Pass
`--encoder nvenc` for **every export an agent makes to check its own work**.
Leave the default (`x264`) alone for anything the operator will keep or post:
x264 `slow` still wins on quality per byte, and it is what every existing
md5-identity check in `tools/headless_check.sh` compares against.

```sh
cargo run -- --mute track.mp3 --scene phosphor --encoder nvenc --render /tmp/check.mp4
```

`--encoder` takes `x264` (default), `nvenc`, or `nvenc-hevc`. An unknown name is
**refused**, not ignored, because an export that quietly used a different encoder
is not something anyone notices until they compare file sizes. The `video
encoder:` report line names what a run would use.

**Measured on this machine, so nobody re-derives it.** 600 frames of 1080p:

| step | x264 `slow -crf 16` | `h264_nvenc p4 -cq 23` |
| --- | --- | --- |
| encode only | 5.66 s | **2.29 s** |
| file size | 26.8 MB | 25.9 MB |
| a real 20 s export, end to end | 59.0 s | 56.6 s |

Read the last row before reaching for this expecting a transformation. The
encoder is about 4 % of an export's wall clock right now; the other 54 seconds is
**rendering through Mesa's `llvmpipe` on the CPU**, because Xvfb has no GPU. Put
the *rendering* on the GPU (VirtualGL — see below) and the encoder's share, and
this rule, start to matter.

Two things not to get wrong:

- **NVENC's `-cq` is not x264's `-crf`** and the numbers do not transfer. The
  mapping lives in `ExportQuality::nvenc_cq`, offset rather than copied; copying
  them is what makes a GPU export look obviously worse and get blamed on the
  silicon.
- **`-b:v 0` is load-bearing.** Without it NVENC caps at its 2 Mbit default and a
  1080p frame of high-frequency ASCII turns to mush.
- **The 3090 cannot encode AV1.** `av1_nvenc` is listed by `ffmpeg -encoders`
  because the *driver* supports it; the encode silicon is Ada and later. There is
  deliberately no `--encoder av1`.

**The encoder is session state, not project state.** It is on `RenderRequest`,
not on `RenderExportConfig`, so a `.musi` carried to a machine with no NVIDIA
card cannot insist on `h264_nvenc`. It does not change what is rendered — every
encoder is fed byte-identical frames, and the determinism contract is about
frames — but it does change the *file*, so a check that compares two exports by
md5 must hold the encoder fixed.

## Put the headless gate on the GPU

`tools/headless_check.sh` launches the application 46 times, several in loops,
and every frame is rasterized by `llvmpipe` on the CPU: 23 s of CPU for a
240-frame Spectrum capture, 50 s for Phosphor Dream, across five to eight
threads. That is where the gate's wall clock goes. It is not the differential
harnesses (about a minute, and opt-in since 2026-08-08) and it is not the Rust —
`[profile.dev] opt-level = 1` moved it by a quarter and no further.

The gate uses `vglrun` automatically when it is installed and prints which path
it took (`gpu:` line). Nothing else changes: no application change, no raylib
change, no capture change, and a machine without it still passes, just slowly.

```sh
curl -LO https://github.com/VirtualGL/virtualgl/releases/download/3.1.4/virtualgl_3.1.4_amd64.deb
sudo dpkg -i virtualgl_3.1.4_amd64.deb
```

`MZ_GL_LAUNCH` overrides the detection either way (set it empty to force the CPU
path, which is how the two are compared); `MZ_VGL_DEVICE` picks the VirtualGL
device, default `egl0` — the EGL back end, which is the one that needs no X
server on the GPU side.

## Rules for this repository

- Preserve unrelated work in both repositories.
- Keep first-party application code in Rust. Retaining raylib as an external C
  dependency through Rust bindings is intentional and in scope, as is FFmpeg as
  an external executable and the existing Python helpers as independent tools.
- Do not copy credentials, `.env`, user audio, generated video, analysis
  caches, or build artifacts into this repository. Synthetic fixtures only.
- Prefer small compiling checkpoints, but optimize for a fast Linux-first hobby
  rewrite rather than release-engineering ceremony.
- Only the integration owner edits the root manifest or broad application
  state. Leaf agents request dependencies rather than adding them.

## Style

- Cite the oracle. Where a function reproduces C behaviour, name the C file and
  line in the doc comment. It is the difference between a port a later reader can
  check and one they have to trust.
- Say why, not what, in comments — especially where the oracle looks wrong. Those
  are the lines a future session will otherwise "fix" back into a parity bug.
- Data tables that are checked column-by-column against C carry
  `#[rustfmt::skip]`, because rustfmt explodes them into one argument per line
  and destroys the thing that makes them checkable.
- `cargo fmt` and a clippy-clean tree are the baseline; deviations are marked
  with `#[allow(...)]` and a reason.

## Not built, and not going to be

Recorded here rather than only in the plan, because this is where a user would
look for the difference list. Parity is declared *with* these, not despite them.

- **Microphone capture** (`MUSIALIZER_MICROPHONE`). Operator's call, 2026-07-27.
  A build-flag feature of the frozen binary; nothing else depends on it, and it
  needs a second audio path the bridge does not have.
- **Hot reload.** An explicit first-pass non-goal since the fork.
- **Non-Linux platforms, including Windows, macOS and OpenBSD.** Linux-first
  hobby rewrite.

## Still to be filled in

`FEATURE_PARITY_PLAN.md` is the authoritative ordered list. In short, the critical
path is: feed persisted project lanes into preview/export frames; drive automatic
scene plans and cue settings; complete dirty/draft/autosave behavior; restore the
remaining image, lyric-document and timeline workflows; package the full Python,
schema and prompt support bundle; then run the expanded integration gate.

Do not duplicate that checklist here. The deliberate exclusions immediately
above and the rules throughout this guide remain authoritative.
