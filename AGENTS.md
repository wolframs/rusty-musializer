# Repository guide for coding agents

The Rust rewrite of Musializer. The C repository is feature frozen. The Cargo
workspace exists and the Phase 1 vertical slice works: a window opens, audio
plays through an allocation-free callback bridge, and Spectrum reacts to it.

`CLAUDE.md` is a symlink to this file. Edit `AGENTS.md`.

## Commands

```sh
cargo build                        # no environment setup needed
cargo test                         # headless; no window, no audio device
cargo clippy --all-targets
cargo fmt --check

cargo run -- path/to/song.mp3      # the slice: window + audio + Spectrum
cargo run --bin make-fixture-wav -- build/x.wav 8   # synthetic fixture audio

tools/headless_check.sh            # the self-check: private Xvfb, evidence
```

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
vendor/
  raylib-5.5/         # upstream raylib source (third-party, not ours)
  clang-builtin-shim/ # five headers so bindgen can run; see its README
resources/shaders/    # first-party GLSL
docs/PHASE0_INVENTORY.md  # CLI grammar, settings tables, schemas, env vars
```

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

Do not add an `unsafe` block without a `SAFETY:` comment and a row here.

## Before implementation work

Read `REWRITE_PLAN.md`. Start with its "Handoff: start here" section, which
gives the incoming session its first moves in order; then the source ownership
map, which assigns every C file in the frozen tree to a workstream. The plan
also carries the frozen commit, the crate boundaries, the invariants that
survive the rewrite, and a NOTE ENTRIES section at the bottom recording what
has already been done, decided, or gone wrong.

Read the notes before assuming any section describes reality — the prose
describes the plan, the notes describe what happened. Add a note when you learn
something a later session would otherwise rediscover.

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

## Still to be filled in

- `track.h` → `app::Workspace`: deferred until Agent B's `.musi` model lands, so
  the track model is not invented twice.
- The persistence half of `core::scene::routes` (export/import mappings, spec
  parsing), which needs Agent B's codec.
- FFmpeg export, Assist and font-import supervision (Agent E).
- The workspace UI and the real CLI (Agent F) — work from
  `docs/PHASE0_INVENTORY.md` section 3, not from the plan's older flag list,
  which was missing eight flags.
