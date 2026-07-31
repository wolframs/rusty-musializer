# Repository guide for coding agents

The Rust rewrite of Musializer. The C repository is feature frozen.

**The application is built and runs.** All ten scenes draw, `.musi` projects open
and save, video exports through FFmpeg, every bottom panel is real, and
`tools/verify.sh` is 19 passed / 0 failed — thirteen differential harnesses
against the frozen C plus a headless capture gate. The sole live completion queue
is `FEATURE_PARITY_PLAN.md`. It records the application-boundary gaps those gates
do not cover, including project lanes not reaching `SceneFrame`, automatic scene
plans not being driven, and the missing external support bundle.

`CLAUDE.md` is a symlink to this file. Edit `AGENTS.md`.

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
| `differential_ascii_art.sh` | 39263 cells, 329738 values, largest delta **0** |
| `differential_project_io.sh` | 1650 values, **both `.musi` round trips**, largest delta **0** |
| `differential_timeline_view.sh` | 30865 records, 204953 values, largest delta **0** |
| `differential_layout.sh` | 27547 records, 527187 values, largest delta **0** |
| `differential_beat_tracker.sh` | 12352 records, 123522 values, largest delta **0** — **found a parity bug** |

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
unset and `PULSE_SERVER` pointed somewhere unresolvable, and writes artifacts
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
| `runtime::audio_bridge` | raylib's `AudioCallback` is a bare `extern "C"` fn with no user-data pointer, so the ring must be reachable from a `static` | The callback only touches a lock-free SPSC ring — no allocation, lock, or syscall. `attach`/`detach` are `unsafe` and document the stream-lifetime contract |
| `runtime::draw` | raylib's default 1x1 texture is a non-owning handle, and the safe `Texture2D` would unload it on drop | The ffi draw wrappers take a `&mut impl RaylibDraw`, so an active drawing context is proven at compile time |
| `runtime::draw` colour helpers | `ColorFromHSV`/`ColorAlpha` are pure C arithmetic | No global state; safe to call anywhere |
| `runtime::process::process_group` | `std`'s `Child::kill` sends only `SIGKILL`, to only one process. `SIGTERM` and process-group delivery need `kill(2)`, and `libc` is not a dependency | One block wrapping a hand-declared `extern "C" fn kill(c_int, c_int) -> c_int`. Both arguments pass by value, nothing is written through a pointer, and every caller passes a pid it owns as a live `Child` (or its negation) |
| `runtime::font` | raylib-rs's safe `load_font_from_memory` takes the glyph set as a `&str` and passes `str::len()` — a **byte** count — as the codepoint count, so a multi-byte set makes raylib read past the array. And `GetFontDefault` is a non-owning handle that `Font`'s `Drop` would unload | Four blocks, all inside `rasterize` and `default_face`, and every face goes through them — the four built-in ones (including the icon face, whose codepoints are all above U+F000 and so are exactly the multi-byte case the wrapper gets wrong) and a project's imported one. `LoadFontFromMemory` gets both lengths from the slices themselves, and raylib copies out what it needs before returning, so a heap buffer read from disk is as safe as an `include_bytes!` array; the result goes straight into `Font::from_raw`, whose `Drop` is `UnloadFont`. The default face is wrapped in `WeakFont`, whose drop is a no-op. `GenTextureMipmaps` borrows the font's own texture field so the level count is written back where `SetTextureFilter` reads it |
| `runtime::decode::wave_samples` | raylib-rs's safe `Wave::load_samples` builds its slice as `(pointer, frameCount)` while `LoadWaveSamples` allocates `frameCount * channels` floats, so for any stereo track it hands back exactly half the decoded audio — silently | One block. The count comes from the wave's own format, the pointer is null-checked before a slice is formed, the slice is copied into a `Vec` inside the block, and the allocation goes straight back to `UnloadWaveSamples`. Nothing borrowed escapes, and the `ffi::Wave` it is called with is a bitwise copy of one whose owner outlives the call |
| `runtime::decode::image_rgba8` | reading a decoded image's pixels needs `Image::data`, and the safe alternative — `get_image_data` — forms a slice from `LoadImageColors` without null-checking the pointer, which is the third instance of the same defect | One block. `ImageFormat` converts to `UNCOMPRESSED_R8G8B8A8` first, and the length is `GetPixelDataSize` — the same function raylib itself sizes the buffer with — checked against `width * height * 4` before the slice is formed, so a format `ImageFormat` silently declined to convert is refused rather than misread. The pointer is null-checked, the `Image` owning the allocation is still in scope (its `Drop` is `UnloadImage`), and `to_vec` copies before the block ends |

Do not add an `unsafe` block without a `SAFETY:` comment and a row here.

## Traps this rewrite has already paid for

- **Unset `WAYLAND_DISPLAY`, not just `DISPLAY`, before testing anything that can
  open a window or a dialog.** This operator's session is Wayland
  (`XDG_SESSION_TYPE=wayland`), so `DISPLAY= somecommand` isolates nothing: Qt and
  GTK fall straight through to `WAYLAND_DISPLAY` and draw on the real desktop.
  This has already leaked a `kdialog` error box onto the operator's screen mid-session.
  `tools/headless_check.sh` gets this right with `env -u WAYLAND_DISPLAY`; the rule
  applies just as much to a two-line shell test as to a capture run.
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

**Not negotiable — anything a user or a file can observe.** These are parity
bugs, not design choices:

- The `.musi` format and every schema in `docs/PHASE0_INVENTORY.md`. A project
  saved by the C must open here and back again.
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
