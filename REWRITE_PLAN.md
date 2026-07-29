# Rusty Musializer Rewrite Plan

## Status

The C repository is feature frozen. Implementation may begin.

Source reference:

- Repository: `../musializer`
- Frozen branch: `master` (not `main`)
- Frozen commit: **`9300af942bd00d8c85fc4e3c8c02cf2b6356764f`**
  (`9300af9`, 2026-07-26, "docs: index the planning, in-progress and review
  documents in one place"), working tree clean at freeze
- Rust repository: `../rusty-musializer`
- Initial target: Linux on the current Kubuntu workstation

Read `../musializer/CURRENT_FILE_POINTERS.md` before trusting any other
document in the C repository. It is the index, and it marks which documents
describe behaviour and which describe intent.

This is a rewrite powered by coding-agent parallelism and abundant token budget.
The objective is a fun, usable Rust Musializer, based on the hobby-C-project.

## Handoff: start here

If you are the session picking this up, you are the integration owner. Nothing
has been built yet.

State of `rusty-musializer` at handoff:

- Branch `master`, **zero commits**. `AGENTS.md`, its `CLAUDE.md` symlink, and
  this file are untracked. There is no Cargo workspace, no `.gitignore`, and no
  code.
- Everything below the vertical slice is unstarted. No note entry claims
  otherwise; the NOTE ENTRIES section at the bottom is the only record of what
  has actually happened, so read it before assuming a section describes
  reality.

Your first moves, in order, before any parallel work:

1. Read `AGENTS.md` here and `../musializer/CURRENT_FILE_POINTERS.md` there.
2. Commit the current documents so the fleet has a base commit to branch from.
   Add a `.gitignore` covering `target/`, audio, video, and caches first.
3. Do Phase 0's remaining inventory. It is small and it calibrates everything
   after it.
4. Build the vertical slice yourself with one runtime-focused agent. Phase 1 is
   a go/no-go gate on the binding choice; do not fan out six agents before a
   window opens and Spectrum reacts.
5. Land the shared contracts listed under "Shared contracts land first".
6. Only then brief agents A-F from the ownership map and let them run.

Do not start by auditing the C architecture again. The map exists: this file
for the plan, `CURRENT_FILE_POINTERS.md` for the documents, and the ownership
table below for the files.

## The vibe contract

The first useful Rust build should arrive quickly:

1. Open an audio track.
2. Play it through raylib.
3. Analyze its PCM.
4. Draw at least one audio-reactive scene.
5. Exit without leaking application-owned resources or abandoning child
   processes.

After that, parallel agents fill in the product until it is complete to replace the
C build for ordinary use.

Expected tempo:

| Milestone |
| --- |
| Cargo + raylib window + audio + one scene |
| Ten mechanically ported scenes |
| Mostly working Linux application |
| Projects, export, UI, and rough behavioral parity |
| Most valuable tests ported and the rough edges sanded down |

These are "well this sounds about right" targets, not delivery commitments.

## Scope

Rewrite first-party application and build code in Rust:

- application state and orchestration;
- audio analysis and realtime sample handoff;
- scene registry, state, update, and drawing code;
- workspace UI and editing state;
- `.musi` model, validation, loading, and saving;
- caption typography, including the imported caption face and its licence;
- content-addressed asset bundling for `audio`, `images`, and `fonts`;
- FFmpeg export supervision and transactional publication;
- Assist job supervision and result staging;
- font import job supervision and its bounded catalogue reader;
- CLI parsing;
- Linux build and launcher-facing executable.

Keep:

- **raylib as an external C library through Rust bindings. Not provisional.**
  raylib stays C. It is a third-party dependency that happens to be vendored,
  and vendoring is not authorship: `../musializer/thirdparty/raylib-5.5` sitting
  inside the frozen tree makes it *look* first-party, and it is not. It is not
  in scope, not on the ownership map, and not something a stubborn binding
  justifies replacing. Neither is any other upstream C in `thirdparty/`.
- FFmpeg as an external executable;
- the existing Python analysis helpers and their schemas;
- the frozen C binary and repository as the parity oracle.

Keep, but genuinely provisional — revisit when convenient:

- tinyfiledialogs through FFI if that is faster than selecting a Rust dialog
  crate.

## Explicit first-pass non-goals

Do not delay the rewrite for:

- live `.so`/DLL hot reload;
- Windows, macOS, OpenBSD, MinGW, or MSVC parity;
- exact pixel-identical frames;
- byte-identical JSON formatting where semantic compatibility is sufficient;
- porting the custom `nob` build system;
- eliminating all C from the dependency graph;
- porting every C test before running the application;
- signed packages, installers, auto-updates, or release automation;
- architectural perfection.

Hot reload is to  return later as session serialization plus process restart, or
as a much narrower scene-plugin boundary. Do not carry arbitrary Rust heap
objects across an unloaded dynamic-library boundary.

**HOWEVER:**
When encountering stuff that would need considerable "workarounds" in order to skip
rewriting it, the correct move is to *rewrite rather than implementing the workaround*.

That clause is about **first-party** code — the C this project wrote. It is not
a licence to absorb an upstream dependency. If raylib bindings fight you, the
answer is a different binding, a thinner wrapper, a small `unsafe` island, or a
question to the human. It is never "reimplement raylib in Rust", and the same
goes for FFmpeg and the Python helpers. An agent that finds itself porting
`thirdparty/` has misread this section.

## Non-negotiable behavior

Even a playful rewrite should retain a small set of real invariants:

- Preview and export use the same scene semantics.
- Audio callbacks do not allocate, block, perform file I/O, or touch UI state.
- Scene output is deterministic for the same project, seed, decoded PCM, and
  frame index, within reasonable floating-point and GPU tolerance.
- `.musi` input is bounded and validated before mutating application state.
- Existing projects are never silently rewritten by merely opening them.
- Saves and video exports publish transactionally; a failed or cancelled job
  does not destroy an existing destination.
- Measured audio, lyric timing, model interpretation, and manual events remain
  separate evidence lanes.
- Model output is staged for Apply/Discard and never mutates a project merely
  because a job finished.
- Every spawned child is explicitly finalized, killed when necessary, and
  waited/reaped. There are three families, not two: FFmpeg export, Assist
  analysis, and font import.
- An imported caption face is bundled content-addressed together with its
  licence file. A project whose caption face is `imported` without that asset,
  or that carries the asset without the face being `imported`, is invalid —
  captions must be reproducible from the file alone.
- Caption measurements are fractions of the frame, never pixel counts, so a
  project typeset against a preview window exports identically at any
  resolution.
- Analysis inputs that point outside the bundle stay session state and are
  never written into `.musi`. The chosen lyric sheet is the current case: it is
  an input to analysis, its words land in the project as cues anyway, and a
  bare absolute path would be the format's only non-bundled reference.
- Credentials, private audio, analysis caches, and generated video stay out of
  Git.

## Rewrite architecture

Start with three crates. Add more only when a real boundary demands it.

```text
rusty-musializer/
├── Cargo.toml
├── crates/
│   ├── musializer-core/       # no raylib: model, analysis, timelines, layout
│   ├── musializer-runtime/    # raylib, resources, processes, filesystem edges
│   └── musializer-app/        # binary, CLI, workspace UI, orchestration
├── resources/
├── schemas/
└── tests/
```

### `musializer-core`

Pure, deterministic, headlessly testable code:

- `AudioAnalyzer`, beat tracker, sample/frame calculations;
- scene-frame value types;
- lyrics and event timelines;
- scene settings and audio routes;
- project model and validation;
- render scheduling;
- caption, timeline, and workspace layout;
- semantic lanes and staged candidates.

No raylib handles, OS process handles, global mutable state, or filesystem side
effects.

### `musializer-runtime`

Unsafe and platform-sensitive work lives behind small safe APIs:

- raylib initialization and owned resource wrappers;
- realtime callback bridge and SPSC sample ring;
- decoded-wave access;
- FFmpeg process and frame pipe;
- Assist child-process supervision;
- atomic file publication and content-addressed asset storage;
- Linux-specific process-group behavior;
- optional tinyfiledialogs wrapper.

The goal is not zero `unsafe`; it is small, named, reviewable unsafe islands.

### `musializer-app`

Own one `App` value rather than recreating C's global `Plug *p`.

Suggested top-level state:

```text
App
├── Workspace
│   ├── Tracks
│   ├── ProjectController
│   └── EditorState
├── AudioEngine
├── SceneRuntime
├── RenderController
├── AssistController
├── ResourceStore
└── UiState
```

`RenderController` and `AssistController` should be explicit state machines.
Owning job types must expose finalization and cancellation; their `Drop`
implementations should make a best effort to prevent abandonment, while normal
control flow reports cleanup failures rather than hiding them.

Avoid solving borrow-checker friction with a forest of `Rc<RefCell<_>>`.
Controllers receive narrow mutable references or commands, and long-running
work communicates through bounded channels or polled job state.

## Dependency starting point

Prefer conservative, ordinary crates:

- `raylib`/`raylib-sys` matching the frozen raylib 5.5 behavior;
- `serde` and `serde_json`;
- `thiserror` for library errors;
- `anyhow` only at the binary/command boundary;
- `clap` for the CLI if it accelerates compatibility;
- `sha2` for asset identities;
- `tempfile` for tests and transactional staging;
- standard-library `std::process::Command` plus small platform extensions for
  child ownership;
- a bounded channel or a hand-written atomic SPSC ring for the realtime bridge.

### The raylib linking choice

Two ways to get the C library under the bindings:

1. let `raylib-sys` build the raylib it vendors; or
2. use its no-build mode and link a raylib 5.5 built from the frozen source.

**Human's stated preference, 2026-07-26: option 2, on maintenance grounds.**
Try it first. It is a preference, not a mandate — the vertical slice is still
the gate, and option 1 remains the fallback if 2 costs more than an afternoon.

The reasoning, stated accurately so nobody re-derives it wrongly: option 1 does
*not* expose the project to surprise raylib updates — `Cargo.lock` pins the
crate, and the raylib inside it moves only on a deliberate bump. The real
argument is version coupling. Under option 1 the raylib version is a side
effect of the binding crate's version, so a binding bugfix cannot be taken
without moving raylib under the renderer at the same time, and exactly 5.5 is
only available if some crate release happens to ship it. Option 2 decouples
them and pins the same C library the parity oracle was built against, which is
worth something while visual parity is still being judged.

What option 2 concretely requires:

- Vendor the raylib 5.5 **source** into this repository, from
  `../musializer/thirdparty/raylib-5.5/`. It is upstream source, not a build
  artifact, so copying it is allowed and normal.
- Do **not** link `../musializer/build/raylib/linux/libraylib.a`. That path is
  a gitignored artifact directory in the frozen repo; compiling against it
  would tie every Rust build to the C repo's build state and would break the
  moment that tree is cleaned. Build our own copy once.
- Check that the binding crate's no-build mode expects an API/ABI compatible
  with 5.5. A pregenerated-bindings mismatch is the likeliest way this option
  fails, and it fails loudly at link time rather than subtly at runtime.

Whichever lands, record it in `AGENTS.md` with the reason, and add a note entry
here.

## Fleet layout

The effective fleet is six agents plus one integration owner. More agents are
welcome only when they own genuinely disjoint files.

### Integration owner

Owns:

- workspace structure and shared types;
- `Cargo.toml` and dependency decisions;
- the application loop;
- merge order;
- compiling checkpoints;
- workstream handoffs;
- final behavior triage.

No other agent edits the root manifest or broad application state without
coordinating first.

### Agent A: core audio and timing

Port:

- sample ring;
- audio analyzer and FFT;
- beat tracker;
- render frame scheduling;
- track waveform and Song Atlas preprocessing;
- relevant deterministic tests.

### Agent B: project and editor model

Port:

- `.musi` data model;
- Serde codec plus compatibility defaults;
- strict validation;
- asset hashing, bundling, and project-relative resolution across the
  `audio`, `images`, and `fonts` categories;
- caption typography: the `caption_style` block, frame-fraction measurements,
  and the imported-face-plus-licence coupling;
- lyrics, events, presets, scene switches, and routes;
- transactional saves;
- model-level tests and fixtures.

`caption_style` is optional in v1: files authored before it exists take the
shipped defaults, which reproduce the appearance those files were authored
against. That is a compatibility default, not a missing field.

### Agent C: scenes 1-5

Port:

- Spectrum;
- Pulse Field;
- Orbital Lattice;
- ASCII Field;
- Song Atlas.

Keep formulas and draw-call order recognizable until the Rust application
works. Refactoring the artistry comes later.

### Agent D: scenes 6-10

Port:

- Spectral Terrarium;
- Constellation;
- Cadence;
- Loom;
- Pentagram Orbits.

Keep deterministic seeds, bounded state, lyric/semantic/event inputs, and scene
setting mappings.

### Agent E: runtime and process edges

Build:

- raylib resource ownership;
- audio callback bridge;
- FFmpeg process and raw-frame transport;
- Assist process supervision;
- font import process supervision, its nonce/staleness handling, and the
  bounded catalogue reader that parses the helper's manifest;
- cancellation and bounded shutdown;
- atomic publication;
- Linux process-group cleanup.

The font import job resolves `tools/google_fonts.py` through a small candidate
path list because the layout differs between an extracted distribution and a
`./build` source run. Reproduce that resolution rather than hardcoding one
path.

This agent owns most of the initial `unsafe` budget.

### Agent F: UI and product shell

Port:

- workspace layout and panels;
- scene browser and tuning inspector;
- timeline and transport, including strip zoom and pan;
- lyrics editor, its three-pane panel, and lyric-cue selection, dragging, and
  atomic bulk retiming in the lane;
- caption typography controls and the font import panel;
- Assist and Export panels, including the confirmation step's lyric-sheet row
  with Choose/Replace/Clear;
- notices, guards, and toolbar;
- CLI-to-application wiring.

Two layout rules the C repository already paid for: a panel that reserves
height it never draws steals it from the scene preview, so rows that only some
modes have are parameters rather than assumptions; and a panel's minimum size
must be measured against the panel the minimum supported window (960x640)
actually produces, not against a guessed threshold.

Start ugly-but-operable. Visual refinement follows functional integration.

## Source ownership map

Every `../musializer/src` file at the freeze commit, and who owns its Rust
successor. This is the distribution list; an unassigned C file is a bug in this
table, not a licence to improvise. Rust paths are destinations, not existing
files.

### Shared contracts land first

The integration owner writes these before Phase 2 opens, because more than one
workstream consumes them. Nobody else defines them, and no agent redesigns them
unilaterally.

| C source | Rust destination | Consumed by |
| --- | --- | --- |
| `scene.h`, `scene.c` | `core::scene` registry, `SceneFrame` | C, D, F |
| `scene_settings.c/.h`, `scene_settings_values.h` | `core::scene::settings` | C, D, F |
| `scene_routes.c/.h` | `core::scene::routes` | B, C, D, F |
| `scene_event_merge.c/.h` | `core::scene::events` | C, D |
| `scene_draw.h` | `runtime::draw` primitives | C, D, E |
| `track.h` | `app::Workspace` track model | A, B, F |

`scene_draw.h` exists because GL line primitives are implementation-defined and
commonly rasterize as a single pixel. Whatever the Rust equivalent is, both
scene agents call it rather than each inventing line drawing.

### Agent A — core audio and timing

`audio_analyzer.c/.h`, `beat_tracker.c/.h`, `sample_ring.c/.h`,
`song_atlas_map.c/.h`, `track_timeline.c/.h`, `track_identity.c/.h`,
`render_export.c/.h` → `crates/musializer-core/`.

`render_export` is the deterministic transport math — frame counts over decoded
audio frames — not the encoder. The FFmpeg process is Agent E's.

### Agent B — project and editor model

`project.c/.h`, `project_io.c/.h`, `lyrics.c/.h`, `event_timeline.c/.h`,
`preset_store.c/.h`, `semantic_lane.c/.h`, `analysis_bridge.c/.h`,
`analysis_candidate.c/.h`, `scene_switch.c/.h`, `caption_layout.c/.h`,
`editor_draft.c/.h`, `sha256.c/.h` → `crates/musializer-core/`.

`caption_layout` bounds cues before they reach the renderer; coordinate its
shape with Agent F, who draws the result.

### Agent C — scenes 1-5

`scene_spectrum.c`, `scene_pulse_field.c`, `scene_orbital_lattice.c` +
`scene_orbital_lattice_motion.c/.h`, `scene_ascii_field.c` + `ascii_art.c/.h`,
`scene_song_atlas.c` → `crates/musializer-core/scenes/` for the deterministic
halves, drawing in `musializer-app`.

### Agent D — scenes 6-10

`scene_spectral_terrarium.c`, `scene_constellation.c` +
`scene_constellation_motion.c/.h`, `scene_cadence.c` +
`scene_cadence_timing.c/.h`, `scene_loom.c` + `scene_loom_weave.c/.h`,
`scene_pentagram.c` → same split as Agent C.

The `*_motion`, `*_timing`, and `*_weave` modules are already raylib-free with
headless tests. Port them as pure modules and keep them that way; they are the
reason those scenes are testable at all.

### Agent E — runtime and process edges

`ffmpeg.h` + `ffmpeg_posix.c`, `font_catalogue.c/.h`, plus new Rust work with
no C counterpart: raylib ownership wrappers, the audio callback bridge, Assist
child supervision, atomic publication, Linux process-group cleanup, and the
optional tinyfiledialogs wrapper (`../musializer/thirdparty/`).

### Agent F — UI and product shell

`plug.c`, `plug.h`, `musializer.c`, `workspace_layout.c/.h`,
`timeline_layout.c/.h`, `timeline_view.c/.h`, `lyrics_editor_layout.c/.h`,
`lyrics_editor_ui.c/.h`, `lyric_lane_edit.c/.h`, `assist_ui_state.c/.h`,
`route_editor_state.c/.h`, `font_import_state.c/.h`, `ui_contrast.c/.h`,
`ui_notice.c/.h`, `ui_palette.h`, `ui_row_typography.c/.h`, `ui_theme.h`,
`ui_widgets.c/.h` → `crates/musializer-app/`.

`plug.c` is 8,682 lines and is the composition root; it is a source to
distribute from, not a file to port. Its state belongs in the `App` tree, and
the integration owner arbitrates anything that does not obviously land in one
panel. The `*_layout`, `*_state`, and `ui_contrast`/`ui_row_typography` modules
are raylib-free and headless-tested in C — keep them in `musializer-core` if
they stay pure.

### Deliberately not ported

`hotreload.h`, `hotreload_posix.c`, `hotreload_windows.c` (non-goal),
`ffmpeg_windows.c` (non-goal), `musializer.rc`, `utf8.xml` (Windows resources),
and the `nob` build system in `nob.c` / `src_build/`. Confirm before reviving
any of these; each is a stated non-goal above.

Nothing under `../musializer/thirdparty/` is on this map at all — not as work,
not as a non-goal, not as a decision. raylib and tinyfiledialogs are upstream
code the C project consumes, and the Rust project consumes them the same way.
The map covers `src/` because `src/` is what this project wrote.

## Briefing an agent

Each workstream prompt should carry: its rows from the map above, the crate it
writes into, the invariants from "Non-negotiable behavior" that touch it, the
instruction that `../musializer` is read-only, and its reporting duty.

Reporting duty is uniform: land coherent commits on your own branch or
worktree, and append a NOTE ENTRIES line for anything a later session would
otherwise rediscover — a module that turned out to be someone else's, a C
behavior that looked like a bug, an FFI shape that fought back. `[DONE]` when a
mapped module is ported with its tests, `[HURDLE]` when blocked, `[INFO]` for
everything else. Notes are how this file stays true; treat them as part of the
work, not paperwork after it.

## Working without a human in the loop

This rewrite runs unattended. That is workable because a rewrite against a
frozen oracle carries very little genuine underdetermination: the behaviour is
already decided, written down in C, and covered by 327 tests. Almost every
question that arises has an answer on disk that is faster and more accurate
than asking.

How to resolve hurdles, in the order to try them:

1. **Behavioural question — what should this do?** Read the C source and its
   tests. That is what the oracle is for. Do not infer from this plan when the
   code is one grep away, and do not ask.
2. **The C code looks wrong or contradictory.** It is frozen, so its behaviour
   is the specification whether or not it is correct. Reproduce it, note the
   suspicion, move on. Do not fix it there, and do not silently "improve" it
   here.
3. **A choice the oracle does not settle** — crate selection, module shape,
   naming, error type granularity. Pick the option that is cheapest to reverse,
   note the choice in one line, keep going. These are reversible; deliberation
   costs more than the occasional rework.
4. **An approach fails.** Take the fallback this plan names. Where it names
   none, invent one — there is usually a cruder mechanism that works (hand
   written FFI instead of bindgen, a polled flag instead of a channel, a stub
   panel instead of a finished one). Ship the cruder thing and note it.
5. **This plan is wrong.** Fix the plan in the same commit as the code that
   proves it wrong, and add a note saying what was corrected. The plan is a
   working document, not a contract with the human.
6. **Genuinely blocked.** Write the note, park that thread, and move to
   another workstream. There are six; a stall in one is not a stall overall.

Open questions go in NOTE ENTRIES, not into a message that stops work. The
notes are an asynchronous channel to the human, who reads them between
sessions. Nothing in this rewrite is destructive or outward-facing enough to
need a synchronous answer: the oracle is read-only, the Rust repository is
version controlled, and every mistake here is a revert away from undone.

The failure mode to avoid is not a wrong decision. It is a fleet idling on a
question that the C source answers.

## Collision policy

- Each agent receives explicit file/module ownership.
- Shared contracts land before dependent ports.
- Scene agents do not redesign core frame types independently.
- Leaf agents may add dependencies only through a request to the integration
  owner.
- Agents commit coherent checkpoints to their own branches/worktrees.
- The integration owner merges frequently and hands compiler errors back to the
  owner of the broken boundary.
- The frozen C repository is read-only. No agent "fixes parity" by modifying the
  oracle.

## Execution plan

### Phase 0: freeze and inventory

Freeze is announced and the commit is recorded above. Remaining inventory work:

1. Record the C binary's build profile and version output.
2. Run the existing C and Python tests once. Baseline at the freeze commit is
   327 C tests across 42 files in `tests/`, plus 11 Python adapter suites in
   `tests/adapters/`. `tests/e2e/` is manual by design — real Whisper, a live
   model request, the built binary under Xvfb — and must never be wired into an
   automated run, here or there.
3. Save only non-private synthetic fixtures and normalized expected outputs
   needed for differential checks.
4. Catalogue resources, schemas, scene names, CLI flags, and `.musi`
   compatibility fixtures. There are twelve schemas; `project-v1` and
   `font-import-v1` are the two the rewrite must satisfy directly. The CLI
   surface is wider than the sample commands below: `--project`, `--render`,
   `--render-window`, `--scene`, `--ascii-image`, `--event`, `--route`,
   `--mute`, `--version`, `-h/--help`, and a positional audio path. Routes are
   deliberately applied after every positional and `--project` input is
   resolved; preserve that ordering.
5. Do not begin another architecture audit. `CURRENT_FILE_POINTERS.md` is the
   index, and it already separates live plans, shipped history, and contracts.

Two cautions when reading the C repository's documents:

- `AGENTS.md` (with `CLAUDE.md` as a symlink to it) is gitignored by design and
  does not travel with a clone. It is present in the local working tree; do not
  assume a fresh checkout of the frozen commit will have it.
- Several documents describe intent, not behaviour. `EXTENSION_PLAN.md` is part
  roadmap and part changelog with decision gates D2, D5, D6 and D7 still
  unanswered; `cadence-overhauls-2026-07-26.md` is an unimplemented scratchpad.
  Porting either as though it described the frozen binary would invent
  behaviour the oracle does not have.

### Phase 1: vertical slice

The integration owner and runtime agent build one end-to-end path:

1. Create the Cargo workspace.
2. Open a raylib window.
3. Initialize audio.
4. Load and play a synthetic or ordinary audio file.
5. Feed samples through an allocation-free callback bridge.
6. Analyze samples in Rust.
7. Draw Spectrum.
8. Shut down cleanly.

This is the go/no-go gate for the binding and ownership approach. Do not port
ten scenes before it works.

The gate is self-checked, not reported upward for approval — nothing about it
needs a human, since both raylib options are already decided. But check it with
evidence rather than assertion: compiling is not the gate, and neither is a
process that exits 0. A window that opened, audio that advanced, and a Spectrum
that visibly responded to it are the gate.

`../musializer/tools/UI_REVIEW.md` documents how the C project verifies exactly
this without disturbing the operator: captures on a private Xvfb display (`:77`
by default), `WAYLAND_DISPLAY` unset, `PULSE_SERVER` pointed somewhere
unresolvable so a check never opens a stream on the audio server the human is
using, and every artifact under a gitignored directory. Adopt that shape early.
It is the difference between a rewrite that can check its own work and one that
needs a human to look at a screen for every claim.

If both raylib paths fail, the next move is a hand-rolled `extern "C"` surface
over a static raylib — the API needed for the slice is perhaps two dozen
functions, and bindgen is a convenience rather than a requirement. There is a
third option before there is a question.

### Phase 2: parallel core and scene port

Once shared `SceneFrame`, `SceneSettings`, and project types compile:

- Agents A and B port pure modules and tests.
- Agents C and D port the two scene halves.
- Agent E builds export and child supervision.
- Agent F creates an operable workspace around the vertical slice.

Compilation beats completeness. Stub unfinished panels explicitly rather than
blocking all integration.

### Phase 3: project and export parity

1. Open frozen C `.musi` fixtures.
2. Save and reopen them without losing editor-supported data.
3. Render a short MP4 through FFmpeg.
4. Verify frame count, stream types, dimensions, and duration with `ffprobe`.
5. Cancel an export and confirm an existing destination survives.
6. Fail FFmpeg startup and confirm the application recovers.
7. Run a full-versus-windowed render comparison with a tolerant visual metric.

### Phase 4: complete the useful UI

Land the panels in this order:

1. track open and transport;
2. scene selection;
3. tuning;
4. project open/save;
5. export;
6. lyrics;
7. Assist;
8. presets, route editing, and secondary notices.

Mouse-only parity is acceptable initially. Accessibility work should improve
the Rust application later, not block the first rewrite.

### Phase 5: cleanup after the fun part works

- Port high-value regression tests.
- Replace temporary FFI shortcuts where they are unpleasant.
- Audit every `unsafe` block and document its invariants.
- Add Clippy and formatting gates.
- Decide whether to retain tinyfiledialogs.
- Decide whether session-restoring restart is enough for development.
- Add native platform work only if someone actually wants to run it there.

## Parity ladder

Track progress by capabilities, not translated line counts.

- **P0 — Boots:** Cargo builds; window opens and closes.
- **P1 — Reacts:** audio plays; Spectrum reacts.
- **P2 — Performs:** all ten scenes work with settings.
- **P3 — Remembers:** existing supported `.musi` projects open, save, and reopen.
- **P4 — Exports:** deterministic short MP4 export works and cancels safely.
- **P5 — Edits:** timeline zoom/pan, lyrics and lane editing, events, routes,
  presets, and caption typography are usable.
- **P6 — Assists:** Python helpers launch, cancel, stage, apply, and clean up.
  Includes font import bundling a face and its licence into the project.
- **P7 — Replaces C for the user:** ordinary Linux sessions no longer need the
  C binary.

## Test strategy

Do not blindly transliterate every test first. Port tests in this order:

1. bounds, finite-number, and malformed-input tests;
2. `.musi` compatibility and atomic mutation tests;
3. analyzer, timeline, and deterministic scene-state tests;
4. render frame scheduling and cancellation tests;
5. process lifecycle tests;
6. layout tests;
7. remaining behavioral tests as bugs or uncertainty demand.

Use the C implementation for differential tests where practical:

- analyzer output for synthetic PCM;
- normalized `.musi` documents;
- scene setting defaults;
- frame scheduling;
- lyric/event ordering;
- one or two representative rendered windows compared with PSNR or another
  tolerant metric.

Do not require encoded MP4 files to be byte-identical.

## Definition of "working enough"

The first successful rewrite is allowed to call itself usable when, on Linux:

- it opens common audio formats through raylib;
- playback and seeking work;
- all ten scenes render without obvious corruption;
- existing editor-supported `.musi` projects open;
- projects save transactionally and reopen;
- MP4 export works and cancellation preserves an existing file;
- Assist jobs can be launched, cancelled, and reaped;
- the application survives malformed projects and missing optional tools;
- no known application-owned GPU, audio, heap, file, or child-process resource
  is abandoned on normal exit.

It need not yet have release packaging, perfect visual parity, hot reload, or
non-Linux support.

## Known traps

- The C composition root mixes audio, UI, rendering, persistence, jobs, and hot
  reload. Do not reproduce it as one enormous Rust module. `plug.c` is 8,682
  lines at the freeze commit, and the split plan there dropped its line-count
  goal after extraction removed ~670 lines while features added ~2,900. The
  lesson for the rewrite is a rule about where new state is allowed to land,
  not a cleanup task to schedule later.
- What actually worked in C was the opposite pattern: fourteen raylib-free
  modules with headless tests, which is why that suite grew from 175 to 327
  tests. `musializer-core` is the same bet with the language on its side.
  Prefer moving state out of the shell over moving drawing code between files.
- Safe Rust does not automatically reap child processes. Ownership policy is
  still required.
- Raylib callbacks and `rlgl` calls will require carefully contained FFI.
- Rust RAII wrappers must be dropped while the raylib audio/window context they
  depend on is still alive.
- Serde alone does not reproduce strict project semantics: input bounds,
  duplicate/unknown fields, finite numbers, compatibility defaults, and atomic
  destination replacement still need deliberate code.
- The decoded Wave remains the canonical export timeline unless the rewrite
  intentionally adopts and validates another decoder.
- Full-track decoded audio can consume gigabytes. Chunking is a later feature,
  not a rewrite prerequisite.
- Scene formulas are easy to port but visual drift is easy to misdiagnose.
  Compare meaningful frames at meaningful playhead positions.
- The Python helpers remain independent evidence-producing tools. Do not absorb
  their network/model behavior into the renderer. This now includes
  `google_fonts.py`; the application supervises it and reads a bounded
  manifest, and that is the whole boundary.
- Discovery rules that live in a helper must not be duplicated in the
  application. Only an explicitly chosen lyric sheet is passed as
  `--lyrics-file`; sibling `<stem>.lyrics.txt` discovery is left to the helper
  so the rule lives in one place. Two copies drift.
- A feature nobody can find is a feature nobody has. Several C behaviors are
  reachable only through the UI affordance that names them; porting the logic
  without the affordance ports a dead feature.

## Expected commands

The initial workspace should converge on:

```sh
cargo build
cargo run -- path/to/song.mp3
cargo test
cargo clippy --all-targets --all-features
cargo fmt --check
cargo run -- --project path/to/show.musi
cargo run -- path/to/song.mp3 --render output.mp4
```

Add a narrow differential-test command once fixtures exist. Avoid rebuilding
the frozen C project on every Rust iteration.

## Kickoff checklist

- [x] User announces feature freeze.
- [x] Frozen C commit recorded above.
- [ ] `AGENTS.md` expanded with the actual Cargo commands and crate map.
- [ ] Rust/raylib binding choice proven by the vertical slice.
- [x] Shared types assigned to the integration owner.
- [x] Six workstreams assigned disjoint files.
- [ ] Synthetic fixtures copied or regenerated without private material.
- [ ] P0 and P1 achieved before broad scene translation.

Then release the token goblins.

## NOTE ENTRIES

### Purpose

As work on this goes on, place terse notes about things that are important to keep
in the following section. This is to ensure that later agent sessions know which
parts have already been rewritten, tested or are causing problems.

Format of a note entry:
`- [DONE/INFO/HURDLE] Note text goes here, pointers to code sections too`

### Notes:

- [INFO] The human overhauled this planning file and added the NOTE ENTRIES section,
         and placed this example note here too!
- [INFO] 2026-07-26: freeze declared at `../musializer` `9300af9` on branch `master`,
         clean tree. This plan was drafted against an earlier state, and the last
         ~10 C commits before the freeze added things it did not describe. The
         sections above are now corrected; this note records what changed so a
         later session can tell amendment from original.
- [INFO] Font import landed in C after this plan was drafted: `src/font_catalogue.c`,
         `src/font_import_state.c`, `tools/google_fonts.py`,
         `schemas/font-import-v1.schema.json`, spawn/nonce handling around
         `src/plug.c:1560-1830`. It is a third supervised child family alongside
         FFmpeg and Assist. Owners: model/licence coupling to Agent B, process and
         manifest reader to Agent E, panel to Agent F.
- [INFO] Caption typography is authorable and saved: `caption_style` in
         `schemas/project-v1.schema.json`, optional in v1 with defaults that
         reproduce pre-existing files, all measurements as frame fractions. An
         imported face requires a bundled font asset plus its licence
         (`caption_font_asset`, six required fields, sha256 on both). Asset
         categories are now `audio`, `images`, `fonts`
         (`MUSI_PROJECT_ASSET_FONT` -> `fonts`, `src/project_io.c:1278`).
- [INFO] Lyric/timeline editing grew: strip zoom and pan, cue selection and
         dragging, atomic bulk retiming (`src/lyric_lane_edit.c`). P5 is a taller
         bar than the original plan assumed.
- [INFO] The Assist confirmation step now names the lyric sheet a timed-lyrics run
         will use, with Choose/Replace/Clear (`src/assist_ui_state.c`). The choice
         is session state on `Track` and deliberately never written to `.musi`.
- [INFO] `../musializer/CURRENT_FILE_POINTERS.md` is the C repository's document
         index and marks which docs describe behaviour versus intent. Read it before
         any other C doc. Note that `AGENTS.md`/`CLAUDE.md` there is gitignored and
         absent from a fresh clone, and that `EXTENSION_PLAN.md` (gates D2, D5, D6,
         D7 open) and `cadence-overhauls-2026-07-26.md` describe intent, not the
         frozen binary.
- [INFO] 2026-07-26: this file was made handoff-ready for a fresh orchestrating
         session. Added "Handoff: start here", the complete source ownership map,
         the shared-contracts-first list, and the briefing/reporting protocol. The
         map was verified mechanically against all 110 files in
         `../musializer/src` at the freeze commit — every one is assigned or
         listed as deliberately not ported.
- [INFO] `rusty-musializer` had zero commits at handoff, with `AGENTS.md`,
         `CLAUDE.md` (a symlink to it) and this file untracked. The first commit
         and a `.gitignore` are the incoming session's job. If git history here
         already shows commits, that step is done and this note is stale.
- [INFO] Human ruling, 2026-07-26: raylib stays C, full stop. It is an external
         dependency even though vendoring it in `../musializer/thirdparty/` makes
         it look like project source. The "HOWEVER, rewrite rather than work
         around" clause applies to first-party code only and now says so. This
         does *not* settle the Phase 1 binding choice — both options there keep
         raylib in C — it settles that the choice is between bindings, never
         between bindings and a port.
- [INFO] Human preference, 2026-07-26: try the no-build/link path for raylib
         (option 2 under "The raylib linking choice") before letting `raylib-sys`
         build its own. Reason is version decoupling from the binding crate and
         matching the parity oracle's 5.5, not fear of drive-by updates —
         `Cargo.lock` already handles that. Option 1 stays the fallback. Vendor
         the raylib source into this repo; never link the C repo's gitignored
         `build/` output.
- [HURDLE] Not resolved by this pass: the frozen C repo's own `plug.c` split plan
         still lists eight open tasks (1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.5, 2.6).
         Those are C-side cleanups the rewrite inherits as *structure to avoid
         reproducing*, not as work to port. Do not modify the oracle to close them.

### Session 1 (2026-07-26): Phase 0, Phase 1, shared contracts

- [DONE] Base commit landed. `.gitignore`, `AGENTS.md` (+`CLAUDE.md` symlink,
         preserved as a symlink in git), `REWRITE_PLAN.md`. The "zero commits"
         note above is now stale.
- [DONE] Phase 0 test baseline at the freeze commit: **327/327 C tests**
         (`../musializer/build/tests/musializer_tests`, prebuilt — the oracle was
         *not* rebuilt) and **137 Python tests + 15 subtests** across the 11 files
         in `tests/adapters/`. Build profile: `MUSIALIZER_TARGET_LINUX`, hotreload
         off, unbundle off, microphone off. `--version` prints `musializer 2026.07`.
- [DONE] Phase 0 inventory catalogue is in `docs/PHASE0_INVENTORY.md` (CLI
         grammar, all ten scenes' settings tables, both schemas, resources, env
         vars, capture harness). Read it before re-deriving any of that.
- [HURDLE] **This plan's CLI list was incomplete.** Eight flags were missing:
         `--save-project FILE`, `--analysis-bridge FILE`, `--auto-scenes`,
         `--resolution WxH`, `--fps N`, `--quality NAME`, `--reload-once`,
         `--ui-probe SPEC`. Also `--render-window` takes **two** argv words, not
         one. The route-ordering claim was verified correct
         (`../musializer/src/musializer.c:553-561`). Agent F owns the CLI and
         should work from `docs/PHASE0_INVENTORY.md` section 3, not from the
         Phase 0 prose above.
- [INFO] The C parser has **no unknown-flag diagnostic**: any unrecognized
         `--flag` falls through to the positional arm and is loaded as an audio
         path (`../musializer/src/musializer.c:546-550`). The Rust slice
         deliberately errors instead. Noted as an intentional divergence, not a
         parity bug — flag it to the human if strict parity is wanted.
- [INFO] Spot-checked the inventory's CLI section against
         `../musializer/src/musializer.c` directly, since Agent F codes against
         it: 18 long flags confirmed, all eight "missing" ones real,
         `--render-window` confirmed to consume two argv words (`i + 2 >= argc`),
         and route application confirmed to happen after the positional loop
         (`musializer.c:553`). The inventory is trustworthy.
- [INFO] One CLI detail worth Agent F's attention: a **positional argument
         dispatches on file extension** — `.musi` goes to `plug_load_project`,
         anything else to `plug_load_track` (`musializer.c:546-547`). So
         `--project` is not the only way to open a project, and `musializer
         show.musi` works. Easy to miss when porting the flags one by one.

#### The raylib binding decision — resolved, option 2

- [DONE] **Option 2 landed and is proven by the vertical slice.** raylib 5.5
         source is vendored at `vendor/raylib-5.5/` (copied from
         `../musializer/thirdparty/raylib-5.5/`), built by
         `crates/raylib-5-5-link/build.rs` with the oracle's exact flags
         (`-DPLATFORM_DESKTOP -D_GLFW_X11 -fPIC -DSUPPORT_FILEFORMAT_FLAC=1`,
         per `../musializer/src_build/nob_linux.c:98-150`), and linked under
         `raylib`/`raylib-sys` 5.5.1 in `nobuild` mode. Link deps are only
         `-lm -ldl -lpthread`, matching the oracle: GLFW under `_GLFW_X11`
         `dlopen`s X11 and GL rather than linking them.
- [INFO] The version-coupling argument for option 2 turned out to be *stronger*
         than the plan assumed: **`raylib-sys` 5.5.1 vendors raylib 5.6-dev**, not
         5.5.0. So option 1 would have silently put a different raylib under the
         renderer than the parity oracle uses.
- [INFO] The feared bindings/ABI mismatch is **not** a problem, and this was
         checked rather than hoped: the 5.5.0 and 5.6-dev headers differ by 18
         lines — a version string, one whitespace change, and one added function
         (`GetKeyName`). `rlgl.h` differs only in comments and one implementation
         bugfix; `rcamera.h` and `rgestures.h` are identical. No struct layout or
         signature changes, so bindings generated from 5.6-dev headers are
         ABI-compatible with a 5.5.0 library. `GetKeyName` is simply never called.
- [HURDLE] **bindgen cannot be switched off**, so the "pregenerated bindings"
         idea does not work. The safe `raylib` crate declares `raylib-sys`
         without `default-features = false` (`raylib-5.5.1/Cargo.toml:87-88`), and
         Cargo unifies features, so `bindgen` is on whenever `raylib` is in the
         graph. Setting `default-features = false` on our own entry does not undo
         it (verified with `cargo tree -e features -p raylib-sys`).
- [HURDLE] Ubuntu's `libclang1-21` ships `libclang.so` **without clang's resource
         directory**, so bindgen fails with `'stdarg.h' file not found`. Fixed
         with five ~12-line shim headers in `vendor/clang-builtin-shim/`, put on
         `CPATH` by `.cargo/config.toml`. `raylib-5-5-link/build.rs` removes
         `CPATH` before compiling raylib so the shims never shadow GCC's real
         headers. Rejected alternative: `BINDGEN_EXTRA_CLANG_ARGS` pointing at
         `/usr/lib/gcc/x86_64-linux-gnu/15/include`, which works but breaks on the
         next GCC bump with an error that does not mention GCC. Installing
         `clang`/`libclang-dev` is the better fix on a machine where root is
         available. See `vendor/clang-builtin-shim/README.md`.
- [INFO] `cargo build` therefore needs **no** environment setup. Verified from a
         deleted `target/`: 8.2 s.

#### Phase 1 gate — cleared with evidence

- [DONE] **P0 and P1 reached.** `tools/headless_check.sh` runs the binary on a
         private Xvfb `:77` with `WAYLAND_DISPLAY` unset and `PULSE_SERVER` pointed
         at an unresolvable path, per `../musializer/tools/UI_REVIEW.md`. Result:
         window opened, audio device initialized on miniaudio/ALSA (**not** the
         operator's PulseAudio), 190,560 audio frames through the callback bridge,
         104 bands, peak 0.95, clean shutdown, exit 0.
- [DONE] Reactivity was checked, not asserted. Captures at three playhead
         positions against a synthetic 110→3500 Hz sweep put the peak band at
         **36 (398 Hz) → 55 (1297 Hz) → 66 (2509 Hz)**, moving monotonically with
         the playhead. A picture alone would not have distinguished this from a
         stuck analyzer, which is why the report prints the peak-band range and
         the verdict separates "drew something" from "tracked the sweep".
- [INFO] The capture harness improves on the C one in one place: it waits for the
         X socket instead of `sleep 6`. `UI_REVIEW.md` flags the blind sleep as
         its weak point.
- [INFO] `take_screenshot` is unusable for a path with directories: raylib's
         `TakeScreenshot` runs the argument through `GetFileName` and writes to
         the working directory, so `build/x/y.png` lands in the repository root.
         Use `LoadImageFromScreen` + `ExportImage`.

#### Parity findings worth knowing before porting

- [INFO] **The analyzer reads the left channel only.** `analyzer_configure`
         (`../musializer/src/plug.c:438-449`) uses
         `AUDIO_ANALYZER_CHANNEL_SELECT` with `selected_channel = 0`, never
         `MIX`. The slice originally mixed both channels; corrected. A mix looks
         more principled and is a visible parity break for anything panned
         off-centre. Reproduced in `AudioAnalyzerConfig::preview`.
- [INFO] **The analyzer's sample rate is the source file's, not the device's.**
         `plug.c:660` passes `track->music.stream.sampleRate`, but raylib invokes
         stream processors *after* resampling to the device rate. On a 44.1 kHz
         file with a 48 kHz device — the common case — every frequency label the
         analyzer derives is skewed by 44100/48000. This is reproduced
         deliberately. It is a suspected oracle bug; **do not fix it in
         `../musializer`**, and do not "correct" it here without the human's call.
         `analyzer_configure(48000, 2)` is the no-track default
         (`plug.c:8368,8399`).
- [INFO] Band values are **normalized per frame** by the frame's own maximum
         (`audio_analyzer.c:204`, with `maximum` seeded at 1.0), so the loudest
         band always reads ~1.0 regardless of absolute level. A scene cannot read
         loudness from `bands`; that is what `rms`/`peak` on the frame are for.
         A test pins this so nobody "fixes" it.
- [INFO] The FFT's twiddle factor uses `+2π/length` (`audio_analyzer.c:47-48`),
         the inverse-transform sign convention. Harmless downstream because only
         `re² + im²` is read, and reproduced literally.
- [INFO] `raylib`'s own `attach_audio_stream_processor_to_music` was rejected on
         purpose: it routes every callback through a `LazyLock<Mutex<_>>` slot
         table, and taking a mutex on the audio thread violates the
         "callbacks do not block" invariant. `runtime::audio_bridge` calls
         `AttachAudioStreamProcessor` directly with a bare `extern "C"` callback
         over a lock-free SPSC ring. raylib 5.5's `AudioCallback` has no user-data
         pointer, which is why the ring must live in a `static` — the same shape
         the C uses (`plug.c:531-536`).

#### Shared contracts — landed

- [DONE] All six rows of "Shared contracts land first" now exist and compile:
         `core::scene` (`SceneId`, `SceneFrame`, `SceneAudioFrame`,
         `SemanticFrame`, `LyricCue`, `SceneState`, `SceneDescriptor`,
         `SceneInstance`), `core::scene::settings` (all ten scenes' descriptor
         tables with exact C bounds/defaults, plus snapshot legacy-count
         compatibility), `core::scene::routes` (sources, curves, mapping
         evaluation, route table), `core::scene::events` (records, merged view,
         semantic id namespacing), `runtime::draw` (the `tube` primitive and the
         non-owning default-texture wrappers). The `track.h` row is **not** done —
         see the open question below.
- [INFO] The C `Scene_Descriptor` splits into two halves in Rust: `SceneState`
         (deterministic `update`, in `musializer-core`, headlessly testable) and a
         drawing function in `musializer-app` where raylib is allowed. Spectrum is
         a `StatelessScene` because the C sets no `update` for it.
- [INFO] Agent A's `audio_analyzer.c` and `sample_ring.c` were **already ported**
         by the integration owner because the Phase 1 gate needed them. They are
         faithful line-by-line ports with 20 tests. Agent A's job on those two is
         to verify against the C suite and port the remaining C tests, not to
         rewrite them.
- [HURDLE] `scene_routes` cannot be finished until Agent B lands the `.musi`
         model: `scene_routes_export_mappings`/`import_mappings` and
         `scene_route_parse_spec` need the project codec's canonical names. The
         evaluation semantics (which the frame loop, the Tune readout and the
         transfer graph must share) are done and tested; the persistence half is
         Agent B's to complete against this module.

#### Differential harnesses against the frozen C

- [DONE] Four harnesses in `tools/differential_*.sh`, each compiling the relevant
         `../musializer/src/*.c` with output into our `build/`. The oracle was
         verified clean at `9300af9` after every run. Results: analyzer 104 bands
         to 4e-10; scene settings all 81 descriptors exact; route evaluation 380
         rows exact; event merge 12 cases exact. Copy this pattern for every pure
         module — a number to compare beats a paragraph of reasoning.
- [HURDLE] **`core::scene::events` was wrong on first write, and the differential
         harness is what caught it.** It had been written from
         `scene_event_merge.h`'s comment rather than the `.c`, and got *seven*
         things wrong. All are now fixed and pinned by tests:
         1. Semantic ids are namespaced by **XOR** with `0x8000000000000000`, not
            OR. XOR is one-to-one; OR maps every id that already has the high bit
            set onto itself, so two distinct semantic ids could collapse into one.
         2. A qualified id of **zero is avoided**, becoming `lane_bit | 1`.
         3. There is a **bounded collision probe**: if the qualified id is already
            used, it advances by the golden-ratio constant
            `0x9E3779B97F4A7C15`, skipping zero.
         4. The canonical sort key is **`(timestamp, type, id)`** —
            `type` sits *between* the other two (`scene_event_merge.c:6-17`).
            Omitting it reorders events sharing a timestamp.
         5. A record's **`id` must be non-zero** (`event_timeline.c:35`).
         6. A record's **`value_count` must be at least 1** — an event with no
            values is malformed (`event_timeline.c:37`).
         7. An **unknown `event_type` is rejected, not carried through**. The
            first draft deliberately passed unknown types along on the theory that
            dropping them would be worse; the oracle simply rejects them
            (`event_timeline.c:36`). That was inventing tolerance the oracle does
            not have.
         Also: each input lane must **already be strictly sorted with unique
         ids** (`event_timeline_validate`, `event_timeline.c:46-65`); the merge
         validates and rejects rather than sorting inputs into shape.
- [INFO] The lesson for every agent: **read the implementation, not the header
         comment.** Six of those seven errors came from trusting a well-written
         comment that described intent accurately but omitted the edge cases. The
         plan already says the code and its tests are authoritative and documents
         are not; this is what that costs when ignored.

#### Open questions for the human

- [HURDLE] **A deliberate divergence from the oracle needs your ruling.** Agent E
         found that `font_catalogue_parse` does not deliver the atomicity its own
         header promises (`font_catalogue.h:76-78`) and its own test asserts
         (`tests/test_font_catalogue.c:85-88`): it writes rows straight into the
         destination and only withholds `count`, so a mid-parse failure leaves the
         old count beside new rows — a silently corrupted font picker
         (`font_catalogue.c:196-206`). E chose to parse into a local and commit,
         which satisfies both the header and the C tests while *not* reproducing
         the clobber. That is the one place in this session where the Rust
         deliberately behaves better than the frozen C rather than identically.
         The standing rule is "reproduce it, note the suspicion, move on", so say
         if you would rather have bit-parity with the bug. Full note under Agent E
         below.

- [INFO] `track.h` → `app::Workspace` was deferred rather than guessed. The
         track model touches Agent A (waveform, atlas preprocessing), B (project
         binding) and F (tracks panel), and inventing it before the `.musi` model
         exists would mean Agent B reshaping it immediately. It is scheduled for
         after Agent B's first checkpoint.
- [INFO] The `.musi` fixture plan needs a decision. **There are no `.musi`
         fixtures in the frozen tree to copy** — zero in git, zero outside
         `build/`; the C suite builds compatibility fixtures inline by serializing
         a project and then textually deleting JSON blocks. So `fixtures/musi/` is
         deliberately empty and Agent B must generate fixtures the same way. If
         you have saved projects you would like used as real-world compatibility
         cases, say so and they can be reduced to synthetic equivalents.
- [INFO] Two schema asymmetries found in the inventory that look deliberate but
         are worth confirming: `caption_font_asset.licence_sha256` uses
         `^([0-9a-f]{64})?$` rather than the shared `$defs/sha256`, allowing an
         empty licence hash for a legitimately unlicensed user-disk import; and
         every schema `maxLength` counts **UTF-8 bytes**, not code points, so a
         Rust `chars().count()` check would accept documents the C rejects.
- [INFO] The version string exists three times in three spellings:
         `musializer 2026.07`, `Musializer 2026.07`, `musializer-2026.07`. The
         Rust build currently prints `musializer-rs <crate version>` and does not
         claim parity with any of them. Tell us which spelling is canonical when
         the CLI is finished.

#### Agent A — core audio and timing

- [DONE] All five mapped modules are ported with tests. `musializer-core` went
         from 101 to 132 passing tests; `cargo fmt`, `cargo clippy --all-targets`
         and `cargo test` are clean.
         `audio::beat_tracker` (10 tests) <- `src/beat_tracker.c/.h`;
         `audio::song_atlas_map` (14) <- `src/song_atlas_map.c/.h`;
         `timing::render_export` (23) <- `src/render_export.c/.h`;
         `timing::track_identity` (4) <- `src/track_identity.c/.h`;
         `timing::track_timeline` (12) <- `src/track_timeline.c/.h`.
         Every C test case in `tests/test_{beat_tracker,song_atlas_map,
         render_export,track_identity,track_timeline}.c` is represented.
- [DONE] **`analyzer.rs` and `sample_ring.rs` verified, not rewritten.** Eight
         assertions from `tests/test_audio_analyzer.c` and four from
         `test_sample_ring.c` had no Rust counterpart; they are appended as
         `mod c_suite_parity` in each file (taking those two modules from 11+5 to
         19+10 tests) and **all pass unmodified**. No
         divergence found in either port. The valuable ones a later session should
         not delete: antiphase `Mix` cancelling to *exact* zero
         (`test_audio_analyzer.c:81-104`, sharper than a silent-channel test), the
         `8*dt`/`3*dt` smoothing coefficients pinned directly
         (`:106-124`), and the circular-window identity where silence-then-tone
         must equal tone alone (`:143-164`, the only cover for the ring
         wraparound in `prepare_window`).
- [DONE] **Differential tests against the compiled C, not just behavioural
         ones.** `../musializer/src/{song_atlas_map,audio_analyzer,track_timeline,
         beat_tracker,render_export}.c` were compiled *unmodified* with `gcc` into
         a scratch harness under `/tmp` (nothing written to the oracle; its tree
         is still clean at `9300af9`), and its `%.9g` output is pinned as literals
         in four tests named `matches_the_c_oracle_*`. The atlas one compares 72
         slices x 28 bands x 3 scalars for a 1800 Hz mono sweep and a 440 Hz
         stereo one and agrees to < 1e-6. To reproduce, compile those five
         `.c` files plus a `main` into a directory outside both repositories.
         This also independently validates the already-landed analyzer port,
         since the atlas drives it.
- [INFO] **The Song Atlas spatial blur is in place, and it is asymmetric.**
         `atlas_map_smooth` writes `bands[band]` inside the same loop that reads
         `bands[band - 1]` (`song_atlas_map.c:89` vs `:102`), so the low-frequency
         neighbour is already filtered and the high-frequency one is not. It looks
         like a missing scratch buffer. Reproduced deliberately; the differential
         test is what pins it, because a "corrected" symmetric blur still produces
         a plausible terrain. Do not fix it in the oracle.
- [INFO] **`Song_Atlas_Slice.rms` is a mean square during the measuring pass**
         (`song_atlas_map.c:208`), not an RMS. The square root arrives later and
         is applied to the *ratio* against the loudest slice
         (`:115-116`), so the published value is
         `sqrt(meansquare_i/meansquare_max)` — a relative loudness in [0,1] that
         is not comparable across tracks. The misleading field name is kept.
- [INFO] **A third analyzer-configuration asymmetry, beyond the two Session 1
         recorded.** The offline atlas pass uses `AUDIO_ANALYZER_CHANNEL_MIX`
         (`song_atlas_map.c:162`) while the live preview uses `CHANNEL_SELECT`
         channel 0 (`plug.c:438-449`). The same track therefore has a different
         spectrum in the Song Atlas terrain than in every other scene. Agent C
         should not "unify" these.
- [INFO] `render_export_total_frames` reports `ERROR_FRAME_RATE` for a zero
         `frame_count` *and* for a zero `sample_rate` (`render_export.c:123`),
         neither of which is a frame-rate problem. Reproduced. If Agent F matches
         notice text against error classes, this is the one that will look wrong.
- [INFO] Three C error classes are unrepresentable in Rust and are therefore
         untestable rather than untested: out-of-range enum arguments to the
         render-config setters (a Rust `enum` cannot hold one, so the setters are
         infallible), and `ERROR_BUFFER` from the path and duration helpers now
         that they return `String`. `RenderExportError::OutputBufferTooSmall` is
         kept anyway, with a test pinning all eight message strings verbatim,
         because `render_export_result_string` (`render_export.c:105-115`) is
         user-visible text Agent F will want.
- [INFO] `sample_ring_init` also rejects `capacity > SIZE_MAX/2`
         (`sample_ring.c:8`), which `SampleRing::with_capacity` does not. Left
         alone: on a 64-bit host the only power of two above `SIZE_MAX/2` is
         `2^63`, and allocating `2^63` frames aborts in the allocator long before
         the check could matter. Noted in the file rather than fixed.
- [INFO] Two shape decisions a later session should not re-litigate.
         `build_waveform` and `SongAtlasMap::build` derive the frame count from
         `samples.len() / channel_count` instead of taking it separately, which
         makes C's `frame_count > SIZE_MAX/channel_count` overflow guards
         unrepresentable rather than dropped. `render_export::window_frames`
         returns a `Range<u64>` because C's start/end pair is already half-open.
- [HURDLE] Nothing blocked, but one small ask for the **integration owner**:
         `audio::analyzer::AnalyzerError` derives neither `Clone` nor `Copy`, so
         `SongAtlasMapError` — which wraps it with `#[from]` — cannot either. Two
         plain derives on `AnalyzerError` would fix it. Not done here because
         `analyzer.rs` is not this agent's file outside its `#[cfg(test)]` blocks.
- [INFO] `track.h` -> `app::Workspace` is still deferred (see the open questions
         above) and Agent A did **not** invent it. The waveform and atlas
         preprocessing it needs are free functions over `&[f32]` plus owned
         `Waveform`/`SongAtlasMap` values, so whoever lands the track model can
         hold those without either side reshaping the other.

#### Agent E (2026-07-26): runtime and process edges

- [DONE] All six modules under `crates/musializer-runtime/src/process/` are
         landed with **72 tests** passing (`cargo test -p musializer-runtime`),
         clippy and `fmt --check` clean. Nothing outside that directory was
         touched except this note.
         `process_group.rs` (9 tests) — bounded reaping and group signalling;
         `publish.rs` (11) — transactional publication;
         `ffmpeg.rs` (12 + 1 `#[ignore]`d real encode) — the encoder child;
         `font_import.rs` (23) — catalogue/manifest readers + job supervision,
         with `tests/test_font_catalogue.c` ported in full;
         `assist.rs` (12) — Assist supervision;
         `dialogs.rs` (2) — a deliberate stub, see below.
- [HURDLE] **Do not give the Python helpers their own process group from the
         parent.** This is the trap in this workstream. `os.setsid()` in
         `tools/external_analysis.py:307-310` fails with `EPERM` if the caller is
         already a process-group leader, so `CommandExt::process_group(0)` or a
         `pre_exec` `setsid` would make the helper raise `PermissionError` and die
         on startup. The child creates its own group (`--new-process-group`); the
         parent only signals it. `assist.rs`'s
         `cancelling_reaches_the_whole_tree_not_just_python` test fails if anyone
         changes this, and says so in its panic message.
         The consequence is a race the oracle also has: between `exec` and the
         helper's `setsid`, `kill(-pid, …)` addresses a group that does not exist
         and returns `ESRCH`. That is what the `kill(-pid)` → `kill(pid)` fallback
         (`plug.c:4112-4113`) is for; it is not defensive padding and must not be
         simplified away.
- [INFO] **Dependencies wanted, none added.** `libc` for `kill(2)`: `std` sends
         only `SIGKILL` to only one process, so `process_group.rs` declares
         `extern "C" { fn kill(pid: c_int, sig: c_int) -> c_int; }` by hand — the
         plan's "hand-written FFI instead of bindgen" fallback. It works and is
         one 4-line `mod sys`, so this is a preference, not a blocker.
         `tempfile` turned out **not** to be needed: transactional staging must
         write next to the destination (`rename(2)` is per-filesystem), which is
         a name-and-retry loop, not a temp-dir crate, and the tests use
         `std::env::temp_dir()` with `Drop` cleanup.
- [INFO] **Two `waitpid` behaviours got better for free.** `Child::try_wait()`
         caches the status, so the C's `ECHILD`-means-finished tolerance
         (`plug.c:3991`, `:4159`) is unnecessary rather than reimplemented, and a
         monotonic `Instant` replaces both the `clock_gettime` borrow-from-seconds
         fixup (`ffmpeg_posix.c:83-89`) and the "clock went backwards" guard
         (`font_import_state.c:31-35`).
- [INFO] Two C behaviours that look like bugs, reproduced with a comment rather
         than fixed:
         (1) `render_export_temporary_path` formats its nonce in **decimal**
         while `musi_project_temporary_path` uses **16 hex digits**
         (`render_export.c:287` vs `project_io.c:999`). Harmless, preserved
         because those names appear in messages users are told to look for.
         (2) `project_last_separator` treats `\` as a path separator on POSIX
         (`project_io.c:994`), so a Linux filename containing a backslash gets
         its temporary in the wrong directory. Exotic; reproduced and noted.
- [HURDLE] **One deliberate divergence, in `font_catalogue_parse`.** Its header
         promises the destination survives a failed parse
         (`font_catalogue.h:76-78`) and `tests/test_font_catalogue.c:85-88`
         asserts it, but the implementation writes each row straight into
         `destination->entries` and only withholds `count`
         (`font_catalogue.c:196-206`). A failure part-way through therefore leaves
         the **old count with new rows** — a silently corrupted picker. The Rust
         parses into a local and commits, which satisfies both the header and the
         C tests. Reproducing the clobber would have meant reproducing a bug
         nothing covers. Flag it if strict parity is wanted.
- [INFO] `ffmpeg_available` diverges in one detail: the C uses `access(X_OK)`,
         which consults the real uid/gid; without `libc` the Rust checks "is a
         file with an execute bit". A file the user cannot actually execute now
         passes the preflight and fails at spawn — which the C already tolerates
         by design, since it re-validates startup independently
         (`ffmpeg.h:12-13`). The `PATH` quirks are exact: unset or empty is
         `false`, and an empty element means `"."`.
- [INFO] The C's `transport_ok` from a failing `close(2)` on the frame pipe has
         no safe-Rust equivalent — `ChildStdin` has no fallible close. The flag is
         kept and cleared by a failed **frame write** instead, which is a stronger
         trigger for the same purpose: a broken transport never publishes.
- [INFO] `dialogs.rs` is a **stub on purpose** and is the one thing here that is
         not a port. It defines the call surface (the eight `tinyfd_*` sites are
         tabulated in its doc comment, with their titles and filters) and returns
         `DialogError::Unavailable` from all of it. Phase 5 owns the backend
         choice; `zenity`/`kdialog` as child processes is the cheapest option and
         would reuse this crate's existing machinery with no new dependency.
         Two rules for whoever implements it: **cancellation is `Ok(None)`, not an
         `Err`**, because every C call site treats `NULL` as "the user changed
         their mind"; and an `Err` from the unsaved-work confirmation
         (`plug.c:7248`) must **keep** the user's work, never discard it.
- [INFO] Boundary duplications to delete when their owners land, all marked in the
         code: `export_temporary_path` and `transport_duration_text` are Agent A's
         (`render_export.c`); `project_temporary_path` is Agent B's
         (`project_io.c`); `ExportConfig`/`validate` should become a `From`
         conversion over Agent A's `Render_Export_Config`.
         `publish_content_addressed` takes a **verifier callback** so hashing
         stays Agent B's `sha256` and this crate needs no digest dependency, and
         it publishes with `link(2)` rather than `rename(2)` on purpose: a digest
         collision must be detectable, not destructive.
         `find_assist_helper` lives in `font_import.rs` next to
         `find_font_helper` because the two C functions are identical apart from
         the env variable and the filename; one resolver, so they cannot drift.
- [INFO] `unsafe` inventory rows for `AGENTS.md` (Agent E cannot edit that file):
         | `runtime::process::process_group` | `std`'s `Child::kill` sends only
         `SIGKILL` to only one process, so `SIGTERM` and process-group delivery
         need `kill(2)`, and `libc` is not a dependency | The single `unsafe`
         block wraps a hand-declared `extern "C" fn kill(c_int, c_int) -> c_int`.
         Both arguments are passed by value, nothing is written through a pointer,
         and every caller passes a pid it owns as a live `std::process::Child` (or
         its negation). A wrong pid is a logic bug, not unsoundness |
         That is the only new `unsafe` in this workstream — the three child
         families otherwise run entirely on `std::process`.

#### Agent C (scenes 1-5, 2026-07-26)

- [DONE] **All five scenes ported, both halves.** Deterministic state and update in
         `crates/musializer-core/src/scenes/`: `spectrum.rs` (stateless — the C
         descriptor sets no `update`), `pulse_field.rs`, `orbital_lattice.rs` +
         `orbital_lattice/motion.rs`, `ascii_field.rs` + `ascii_field/ascii_art.rs`,
         `song_atlas.rs`. Drawing in `crates/musializer-app/src/scenes/`:
         `pulse_field.rs`, `orbital_lattice.rs`, `ascii_field.rs`, `song_atlas.rs`.
         **51 tests**, of which **21 are direct ports of C tests**: all 5 from
         `tests/test_scene_orbital_lattice_motion.c` and all 16 from
         `tests/test_ascii_art.c`. `cargo fmt`, `cargo clippy --all-targets` and
         `cargo test` are clean.
- [DONE] The two already-raylib-free C modules stayed that way, as the plan asks:
         `scene_orbital_lattice_motion.c` -> `scenes/orbital_lattice/motion.rs`,
         `ascii_art.c` -> `scenes/ascii_field/ascii_art.rs`. Their pure helpers that
         the *drawing* code also needs were put in `musializer-core` too rather than
         duplicated in the app crate: `ascii_field::{audio_band, main_color,
         seed_phase, color_byte, clamp01}` and `song_atlas::{render_sample_count,
         render_sample_index, render_distance, hash_unit}`. `main_color` is the whole
         ASCII colour pipeline and is now testable.
- [DONE] **Rendering was verified with pixels, not asserted.** `--scene` is Agent
         F's, so `main.rs` was patched *temporarily*, all four new scenes captured on
         a private Xvfb `:78`, and the patch reverted (`main.rs` is byte-identical to
         trunk; verified by md5 and `git diff`). Every scene exited 0 with the
         "tracked the sweep" verdict, and the captures show what they should: Pulse
         Field's rose stack with its centre bloom, Orbital Lattice's receding ring
         convoy with facets and swaying links, ASCII Field's waterfall showing the
         fixture's rising sweep as a diagonal ridge of ramp glyphs, and Song Atlas's
         lit terrain with the bass-to-treble hue gradient. `tools/headless_check.sh`
         still passes on the reverted tree.
- [HURDLE] **`song_atlas_map.h`'s slice type and render sampling had to be defined
         by Agent C**, because Agent A's `core::audio::song_atlas_map` is still a
         placeholder and the scene's own live ring stores the same slices. They are
         in `core::scenes::song_atlas`: `Slice`, `SongAtlasMap`, `BAND_COUNT`,
         `BASE_SLICES`, `MAX_DETAIL`, `MAX_SLICES`, `render_sample_count`,
         `render_sample_index`, `render_distance`, `SongAtlasMap::{is_valid,
         playhead, dynamics}`. **Agent A should build into this `SongAtlasMap` rather
         than declare a second slice type**; only `song_atlas_map_build` (the
         whole-track FFT) is missing and it is squarely Agent A's.
- [HURDLE] **Two pieces of shared 3D machinery are parked in Agent C's files and
         want hoisting into `musializer_runtime::draw`** next to `tube()`, which
         Agent C may not edit. Agent D's 3D scenes will need both:
         `app::scenes::orbital_lattice::SceneViewport` (the clip-the-GL-viewport-to-a-
         sub-rectangle dance from `scene_orbital_lattice.c:128-159,302-308`, plus the
         raylib 5.5 projection-aspect correction) and `app::scenes::song_atlas::{Batch,
         LineWidth}` (RAII `rlBegin`/`rlEnd` and `rlSetLineWidth`). Song Atlas already
         imports `SceneViewport` from the Orbital Lattice module, which is the wrong
         home for it. Also parked there: `color_brightness` and `color_to_hsv`, two
         one-line ffi wrappers the safe raylib API is missing.
- [INFO] **New `unsafe` islands, for the AGENTS.md inventory** (Agent C may not edit
         that file). All are rlgl/raylib ffi with `SAFETY:` comments, all in
         `musializer-app`: `SceneViewport` (rlgl viewport + framebuffer size +
         `rlDrawRenderBatchActive`, restored on drop), `Batch` (`rlBegin`/`rlVertex3f`/
         `rlColor4ub`/`rlEnd`, closed on drop), `LineWidth` (`rlSetLineWidth`, reset on
         drop), `color_brightness`/`color_to_hsv` (pure colour arithmetic), and
         `DefaultFont::get` (`GetFontDefault`, a non-owning handle that is never
         unloaded). The invariant in every drop-guard case is the same: the pair is
         closed by `Drop`, so an early return cannot leave rlgl in a scene's state.
- [INFO] **Spectrum's existing drawing half was checked against
         `scene_spectrum.c` line by line and no parity error was found.** It is
         unchanged. The one thing worth recording is that its `#[allow(clippy::
         needless_range_loop)]` is load-bearing for the same reason the new files
         keep index loops.
- [INFO] Three C functions called `orbital_clamp01`/`atlas_clamp01`/`ascii_clamp01`
         are **not the same function**. The motion module's and ASCII Field's reject
         non-finite input; `scene_orbital_lattice.c:10-15` and
         `scene_song_atlas.c:32-37` do not, so a NaN passes through both comparisons
         untouched. Reproduced faithfully rather than unified, and each carries a
         comment saying so — this is exactly the kind of thing a later session would
         "tidy" into a parity break.
- [INFO] Suspected oracle bug, reproduced, **not** fixed: `ascii_field_update`
         (`scene_ascii_field.c:186-195`) indexes the *trail* array by
         `column*bands_count/MAX_COLUMNS` and bounds it against `bands_count` rather
         than the trail array's own length, while the band beside it uses the very
         different `ascii_audio_band` index expression. The two arrays are always the
         same length in practice so nothing misbehaves, but the asymmetry looks
         accidental. Rust's version cannot read out of bounds either way.
- [INFO] Two behaviours that read as bugs and are not, both pinned by tests:
         Song Atlas's `song_atlas_update` runs its camera damping *on the same frame*
         as a discontinuity snap, one frame after setting the value it then smooths
         toward; and its onset flag is latched before the capture check, so the very
         first frame of a track consumes an onset it never scrolled far enough to
         show. Both are harmless and both are what the terrain was tuned against.
- [INFO] The event lane does not reach Agent C at all. Verified against the C: none
         of `scene_spectrum.c`, `scene_pulse_field.c`, `scene_orbital_lattice.c`,
         `scene_ascii_field.c` or `scene_song_atlas.c` reads `frame->events`, and no
         Agent C file mentions `EventRecord`. The `core::scene::events` corrections
         are Agent D's concern, not this workstream's.
- [INFO] The four new drawing modules carry a file-level `#![allow(dead_code)]`
         with the reason inline: nothing dispatches them yet because `main.rs` still
         calls Spectrum directly and the `SceneId` -> descriptor/draw registry is the
         integration owner's. Each `descriptor()` exists and is tested; wiring them
         into one table is a five-line change once someone owns it.

#### Agent D (2026-07-27): scenes 6-10

- [DONE] **All five scenes ported, both halves.** Deterministic state and update in
         `crates/musializer-core/src/scenes/`: `spectral_terrarium.rs`,
         `constellation.rs`, `cadence.rs`, `loom.rs`, `pentagram.rs`. Drawing in
         `crates/musializer-app/src/scenes/` under the same five names. **66 new
         tests**; `musializer-core` is at 121. `cargo fmt --check`, `cargo clippy
         --all-targets` and `cargo test` are clean, and `tools/headless_check.sh`
         still passes unchanged (Spectrum, 240 frames, peak band 14..103).
- [DONE] The three raylib-free modules are ported as pure submodules and their C
         tests came with them: `scenes/constellation/motion.rs` (from
         `scene_constellation_motion.c`, 2 C tests), `scenes/cadence/timing.rs`
         (from `scene_cadence_timing.c`, 5 C tests), `scenes/loom/weave.rs` (from
         `scene_loom_weave.c`, 5 C tests). They live in `scenes/<name>/<part>.rs`
         next to `scenes/<name>.rs`, which needs no `mod.rs` and mirrors the C's
         file split.
- [INFO] Each scene exposes a `pub const DESCRIPTOR: SceneDescriptor` with the C's
         `state_version` (terrarium 2, constellation 2, cadence 1, loom 2,
         pentagram 1), ready for whoever builds the registry.
- [INFO] **Nothing draws these yet.** `main.rs` still dispatches only Spectrum, so
         every app-side scene module carries `#![allow(dead_code)]` with a comment
         saying to remove it once a scene selector exists. The drawing halves are
         therefore **unverified visually** — that needs the `--scene` flag (Agent F)
         plus a capture. Do not read "clean clippy" as "looks right".

##### Needed from the integration owner (shared contracts)

- [HURDLE] `SemanticFrame` does not derive `PartialEq`, so a Loom test compares it
         field-by-field (`scenes/loom.rs`, `the_woven_fallback_answers_the_same_thing_twice`).
         Please add `PartialEq` (and `Copy` is already there).
- [HURDLE] Two drawing primitives every 3D scene needs are parked in
         `musializer-app::scenes::spectral_terrarium` because `runtime/**` is not
         Agent D's: **`SceneViewport`** (the `rlViewport` +
         `rlSetFramebufferWidth/Height` + projection-rescale block the C repeats in
         each 3D scene, so a narrow panel crops the world instead of stretching it)
         and **`draw_billboard_rec`** over the non-owning default texture. Agent C's
         Orbital Lattice and Song Atlas need the first one too, and Constellation
         already imports it across a module boundary that reads oddly. They belong
         in `runtime::draw` next to `tube()`.
- [HURDLE] Agent C's `CircleShader::set_radius`/`set_power` are private to
         `scenes::spectrum`, so three of my scenes go through the public
         `radius_location`/`power_location` fields via a local `set_circle` helper.
         Making those two methods `pub` would remove the helper.
- [INFO] `EventRecord::is_well_formed` (pre-fix) was laxer than the C's
         `event_record_is_valid`, which Constellation and the semantic lane both
         depend on, so `scenes::constellation::event_is_valid` is a local faithful
         copy. Once the tightened contract lands (non-zero id, `value_count >= 1`,
         known type) that function should collapse into a call to it.
- [INFO] `semantic_lane.c` is Agent B's and is still a stub, but Loom needs it now,
         so `scenes::loom::semantic_lane_sample` is a local faithful port. Collapse
         it into `core::project::semantic_lane` when B lands the real one.
- [INFO] `musializer-core` has no shared small-vector type, so
         `spectral_terrarium::Vec3`, `constellation::Vec3` and `pentagram::Vec2`
         are three tiny local definitions. Agent C's 3D scenes will make that four
         or five. A `core::math` with `Vec2`/`Vec3` would be worth ten minutes.

##### C behaviour that looks wrong, reproduced deliberately

- [INFO] **`semantic_lane_sample` rejects any view longer than
         `EVENT_TIMELINE_CAPACITY` (1024)**, but a scene receives the *merged* view
         whose capacity is `2 * 1024` (`semantic_lane.c` vs `scene_event_merge.h:8`).
         A project with more than 1024 merged events therefore **silently loses its
         semantic lane in Loom**, which falls back to measured audio. Suspected
         oracle bug; reproduced, tested
         (`a_merged_view_larger_than_the_lane_capacity_loses_the_lane`), not fixed.
- [INFO] `cadence_layout`'s fit loop shrinks `font_size` by 0.82 *after* the sixth
         failed attempt and then exits, so a line that cannot be made to fit is
         drawn 18% smaller than the slot widths it was measured at
         (`scene_cadence.c:141-164`). Only reachable with a pathological cue.
         Reproduced with the reason recorded at the site.
- [INFO] The four scene hash helpers are not the same function: Constellation,
         Pentagram and Spectral Terrarium add `0x9e3779b97f4a7c15` to the salt
         before mixing, **Cadence and Loom do not**. "Harmonizing" them would move
         every particle in Cadence and every glint in Loom. A test pins it.
- [INFO] Spectral Terrarium drops its simulation backlog rather than repaying it:
         once the catch-up loop saturates at eight fixed steps the leftover
         accumulator is reduced modulo the step (`:344-346`). Deliberate — it stops
         a stall becoming a burst — and now tested.
- [INFO] `pentagram_hop_ease`'s input can in principle be negative, and the C casts
         it through `uint64_t`, which is UB for negatives. `motion` and
         `orbit_offset` are both non-negative so it cannot happen; the Rust port
         makes that explicit with a `max(0.0)` instead of relying on the setting
         range. Behaviourally identical.
- [INFO] Confirmed rather than "fixed": Loom filling only part of the stage is
         correct, exactly as `../musializer/tools/UI_REVIEW.md` says. The reason is
         restated in the module doc of both Loom halves so the next reader does not
         re-diagnose it.

#### Agent F (UI and product shell)

- [DONE] **All eleven raylib-free UI modules are ported with their C tests**, one
         Rust module per C pair, one `#[test]` per C test case, same names:
         `workspace_layout` 9, `timeline_layout` 7, `lyrics_editor_layout` 6,
         `timeline_view` 14, `lyric_lane_edit` 14, `assist_ui_state` 11,
         `route_editor_state` 10, `font_import_state` 7, `contrast` 5, `notice` 8,
         `row_typography` 12 — **103 ported tests**. `musializer-core` is at 158
         passing, `musializer-app` at 62, `musializer-runtime` at 5.
- [DONE] **`--scene NAME` works and is proven by captures, not by a clean exit.**
         Done first, out of Phase 4's order, because Agents C and D were porting
         ten scenes and nothing could display them.
         `crates/musializer-app/src/scene_host.rs` is the hookup point: one line
         per scene in `descriptor()` and one match arm in `SceneRenderer::draw`,
         both marked `HOOKUP`. **A scene agent's whole integration is those two
         edits plus flipping `drawing_is_ported`.** Unported scenes draw a card
         naming the scene, its `--scene` flag, its C source and its owner — a
         black frame cannot distinguish "not ported" from "ported and broken".
- [DONE] **The full CLI is ported** in `crates/musializer-app/src/cli.rs` from
         `docs/PHASE0_INVENTORY.md` section 3: all eighteen flags, the four value
         grammars, `--ui-probe`'s eleven keys, the `-h/--help/--version` pre-pass,
         the ordered-action replay, the deferred routes, `--render-window`'s two
         argv words and its index-advance quirk, and the shared error flag that
         short-circuits later stages. 34 tests. `--route` spec parsing lives here
         rather than in `core::scene::routes`, whose persistence half is still
         Agent B's; **if a `ParameterMapping::parse_spec` lands there, `cli.rs`
         should delegate to it rather than keep a second grammar.**
- [INFO] Operable now: scene browser (all ten), transport, timeline
         zoom/pan/scrub, tuning inspector with per-descriptor sliders and a routed
         readout, notice tray, fullscreen, drag-and-drop track open.
         Stubbed-but-named: export, lyrics, assist, project open/save, presets,
         route editing. **A stub asks for no extra timeline height**, so opening
         one does not steal preview it will not use.
- [INFO] **Reading the captures found four defects no test would have caught.**
         Recorded because the lesson generalizes: (1) U+2026 is in the C's
         imported face's curated codepoint set but *not* in raylib's default font,
         so every truncated toolbar label rendered as `Pau?`, `Exp?` — substituted
         at the draw layer, and this goes away when a real UI face lands; (2) the
         toolbar reserved a guessed 200 px for the timecode, which forced every
         label through the ellipsis at the 960x640 minimum with the inspector open
         (a 440 px bar) — it now goes through the ported `TimelineBand`, which is
         the module written for exactly that collision, and the timecode relocates
         to the timeline header when the band says it cannot sit inline; (3) the
         diagnostic readout was drawn at the window origin, under the tracks
         panel, and overran the preview into the inspector; (4) 10-11 px labels
         rendered `:` and `.` as boxes in raylib's bitmap font.
- [INFO] `TimelineBand`'s `fits == false` does **not** mean it returned something
         that fits — it means it has already scaled to its floor and the caller
         must drop controls. A test asserting the wrong thing here failed for the
         right reason; it now pins the caller's obligation instead.
- [HURDLE] **`scene_settings_ui_layout` was effectively unassigned.** It lives in
         `scene_settings.c`, which the map puts in the shared-contracts bundle,
         but it is pure workspace geometry that only the shell reads. It landed in
         `crates/musializer-app/src/ui/shell_layout.rs` rather than being wedged
         into `core::scene::settings`, which C, D and F all compile against. Move
         it if the integration owner disagrees; nothing else consumes it.
         Related: `core::ui`'s module list is fixed by the ownership map and
         `ui/mod.rs` is not Agent F's to edit, so any *new* pure UI module has no
         home in `musializer-core`. `shell_layout` is app-side for that reason
         too, despite being raylib-free and headlessly tested.
- [INFO] **Dependency request: `clap`.** Not blocking — `cli.rs` is hand-rolled,
         complete, and tested, and it is arguably the *better* answer here because
         the oracle's grammar is idiosyncratic (order-sensitive application,
         two-word `--render-window`, a comma-separated sub-grammar in
         `--ui-probe`, no `--` marker) and clap would have to be fought into
         each of those. Recording the request rather than making it: **no crate is
         needed.**
- [INFO] Two agent-B-shaped contracts Agent F now depends on, both designed to be
         cheap to satisfy: `ui::lyric_lane_edit::LaneCues` (three required
         methods; **index order must be canonical cue order**, or the hit test's
         overlap preference and shift+click ranges break silently), and
         `ui::row_typography`'s `impl Fn(&str) -> f32` measurer in place of
         `Caption_Measure_Text`.
- [INFO] **The divergence on unknown flags is kept and now has a test.** The C
         loads `--typo` as an audio path (`musializer.c:546-550`); the Rust errors
         and says so. Same exit status, better diagnostic. Flag it if strict parity
         is wanted — it is one match arm.
- [INFO] Requested-but-unbuilt stages (`--render`, `--save-project`,
         `--analysis-bridge`, `--event`, `--project`, `--ascii-image`,
         `--reload-once`, and the `--ui-probe` keys that need stub panels) warn on
         stderr, raise a notice, **and set the exit status to 1**. Silently
         ignoring `--render` would let a script believe it produced a video.
- [DONE] `tools/headless_check.sh` extended along the lines of
         `../musializer/tools/ui_capture.sh`, keeping the Xvfb `:77` /
         `WAYLAND_DISPLAY` unset / unresolvable `PULSE_SERVER` isolation. It now
         sweeps all ten scenes, all six long `--scene` aliases, four panels at
         both 1280x720 and the 960x640 minimum, and one routed setting — 26
         captures, all exit 0. It parses **`scene request: honoured`** from the
         report, which is the line separating "`--scene` parsed" from "`--scene`
         took effect"; a screenshot of a placeholder card cannot make that
         distinction.
- [INFO] Left undone by Agent F, in the plan's order: project open/save (needs
         Agent B), export (needs Agent E), the lyrics three-pane editor and cue
         lane, the Assist panel and its confirmation step, presets, and the route
         editor UI. The *state* for the last four is already ported and tested in
         `core::ui`; what is missing is only the drawing and the wiring — which is
         the right way round, and the opposite of what porting `plug.c` wholesale
         would have produced.
- [INFO] Keyboard bindings so far, from `ui_theme.h:60-64` plus two of ours:
         Space toggles play, `F` fullscreen, `T` the tuning inspector, Tab and
         Shift+Tab cycle scenes. Mouse-only parity was the brief; these were free.
- [INFO] The UI draws with raylib's default font, not the C's imported face.
         `ui::widgets::measure` is the single place that changes when the real face
         lands, and it measures at zero spacing on purpose — that is what makes
         width linear in font size, which `row_typography::font_size` relies on to
         fit a row in one pass instead of searching.

#### Agent B — project and editor model

- [DONE] All thirteen assigned files are ported and none are stubbed:
         `project/{model,io,lyrics,event_timeline,preset_store,semantic_lane,
         analysis_bridge,analysis_candidate,scene_switch,caption_layout,
         editor_draft,sha256,assets}.rs`. **229 tests** across them, all passing,
         `cargo fmt`/`cargo clippy --all-targets` clean. Test counts per module:
         io 36, model 33, analysis_bridge 24, lyrics 23, scene_switch 22,
         preset_store 18, caption_layout 17, analysis_candidate 17, assets 13,
         event_timeline 8, sha256 6, semantic_lane 6, editor_draft 6.
- [HURDLE] **`musializer-core` still needs `serde`, `serde_json` and `sha2`.** The
         manifest was off limits, so both were worked around and the workarounds
         are shippable rather than temporary scaffolding:
         `sha256.rs` is a hand port of `sha256.c` (the C hand-rolls it too, so the
         port is checkable), and `io.rs` is a hand-rolled strict JSON writer plus a
         recursive-descent parser. Adding the dependencies is now optional, not
         blocking. If they are added, the sensible split is documented at the top
         of `io.rs`: derive the *shape*, keep every rule as explicit code.
         `#[serde(deny_unknown_fields)]` would cover two of the six rules that
         make this codec strict; the other four (byte-bounded strings at parse
         time, integers that reject `1.0`, arrays bounded before element parse,
         compatibility defaults) still need code either way.
- [INFO] The persistence half of `core::scene::routes` is done, in
         `project::model`: `mapping_is_constant`, `constant_mapping`,
         `mappings_supported`, `export_mappings`, `import_mappings`,
         `parse_route_spec`. It landed there rather than in `routes.rs` because
         that file is a shared contract Agent B does not own, and it needs only
         `routes`' public API. **What `routes` should absorb when convenient:** all
         six functions, plus `scene::settings` gaining `mapping_supported` /
         `export_mappings` / `import_mappings` (`scene_settings.c:407-467`), which
         are currently absent there. Move them wholesale — splitting the
         constant-mapping rule from the route rule would let one parameter persist
         as both, which is the exact ambiguity v1 cannot represent.
- [INFO] For Agent F: `caption_layout::CaptionLayout` is `Vec<CaptionLine>` (max 3)
         plus an `ellipsized` flag; each line carries `text`, measured `width`, and
         `centered_offset`. Nothing to interpret, no state across frames. The
         measure callback is `&mut dyn FnMut(&str) -> f32` and its unit is whatever
         the renderer measures in; the *fractions* stay in
         `model::CaptionStyle` and become a `max_width` before the call.
- [INFO] Verified, not assumed: the analysis bridge's scene names and enum order
         are identical to `SceneId::stable_name` and the registry order, so C's
         separate `Analysis_Scene` enum has **no** Rust counterpart and a test
         asserts the equivalence (`bridge_scene_names_match_the_registry`). The C
         comment at `analysis_candidate.c:43-44` claims this; now something checks it.
- [INFO] `scene::events::EventRecord::is_well_formed` is a **weaker** check than
         C's `event_record_is_valid` (`event_timeline.c:32-44`): it omits `id != 0`,
         the event-type range, and `value_count != 0`. `project::event_timeline::
         record_is_valid` is the complete rule and is what the model and codec use.
         Do not substitute one for the other; `is_well_formed` alone would admit an
         id-zero event into a `.musi`.
- [INFO] Two C parity details worth not "fixing": an integer field with a fraction
         (`"width": 1920.0`) is a **syntax** error, not a number error, because the
         integer parser stops at the digits and the object loop then chokes on the
         `.`; and one whole millisecond of scene-switch coverage drift is *not*
         reliably tolerated, because `20.001 - 20.0` is `0.001000000000001` and the
         C test is `> 0.001`. Both are reproduced with tests that say so.
- [INFO] `scene_switch_retarget`'s snapshot guard is incomplete and now has a test
         that documents it rather than a comment that hopes: same-control-count
         scenes with mutually in-range values carry silently. Capturing from the
         target scene remains the caller's responsibility.
- [INFO] The filesystem halves of `project_io.c` and `preset_store.c` are
         deliberately absent from `musializer-core`, which has no filesystem:
         `atomic_write`, `canonicalize_existing_file`, `existing_files_alias`,
         `project_copy_asset_transaction`, directory `fsync`, `mkdir -p`, and
         `sha256_file`. **Agent E owns these.** What is here for them to build on:
         every result enum with its granularity intact (`ProjectFileError` keeps
         `Sync`/`Publish`/`Durability` apart, and `Durability` is the one failure
         that must *not* delete the transaction file), `io::temporary_path`,
         `io::{directory_of, is_unambiguous_relative_path, relative_descendant_path,
         path_is_absolute}`, `assets::bundle_paths` and `assets::safe_extension`,
         and `sha256::Sha256` as a streaming hasher (C reads 64 KiB chunks;
         nothing about the digest depends on that).
- [INFO] `PresetLibrary` is in `preset_store.rs` and belongs in `scene::settings`
         beside the rest of that contract. It is there only because that module had
         no preset library yet. Also `preset_store::default_path` takes a
         `PathEnvironment` struct instead of reading `getenv`, which keeps the crate
         pure and makes the path policy testable — the C could only exercise it by
         mutating the process environment. Only the Linux branch exists; Windows and
         macOS are stated non-goals and are absent rather than half-written.
- [INFO] Fixtures are generated in tests, never committed, exactly as the C suite
         does it: serialize a project, then edit the text. `io.rs`'s
         `without_block` helper is the Rust equivalent of the C tests' JSON-block
         deletion. `fixtures/musi/` stays empty.
- [INFO] The "analysis inputs outside the bundle are session state" invariant needs
         no code: there is no `.musi` field for the chosen lyric sheet and none was
         added. Recorded here so a later session does not add one by analogy with
         `analysis_lanes`.
- [INFO] Deliberate strictness beyond the C, in one place only: an interior
         `U+0000` in lyric text is rejected, where C's NUL-terminated buffer would
         treat it as end-of-string and silently accept the prefix. Truncating a
         lyric on a byte the user cannot see is the worse failure. Flag it if strict
         parity matters more.

### Session 1 close-out: integration (2026-07-27)

All six workstreams merged. **P0, P1 and P2 reached; P3-P6 partly built but not
wired.** State of trunk:

- [DONE] **724 tests**, clippy clean, `cargo fmt --check` clean, and
         `tools/verify.sh` at 10 passed / 0 failed. That script is the one command
         to run first next session: fmt, build, clippy, tests, four differential
         harnesses against the frozen C, the headless window/audio/Spectrum gate,
         and a check that the oracle is still clean at `9300af9`.
- [DONE] **All ten scenes draw.** `app::scene_host` binds descriptor and drawing
         half; `--scene` honours all ten stable names and six long aliases.
         Verified by 26 captures on a private Xvfb, not by exit codes.
- [DONE] Merge order was A, E, C, D, F, B. Every merge conflicted only in this
         file's notes section, resolved by keeping all sections. **Zero source
         conflicts across six agents and ~40 files**, which is what the
         pre-scaffolded module tree bought.

#### What the integration owner had to arbitrate

- [DONE] **`SceneViewport` was ported twice**, by C and D, with different APIs, and
         Song Atlas imported it across a scene-module boundary. Hoisted to
         `runtime::draw`. The two ports disagreed on `Drop` versus an explicit
         `end()`; the canonical version has **both**, because a narrowed viewport is
         global renderer state and a scene that returns early would otherwise
         corrupt everything drawn after it in the frame, UI included.
- [DONE] **`SongAtlasMap` and `Slice` were defined twice**, by A and C — the one
         collision the scaffolding did not prevent, because C branched while A's
         module was still a placeholder. A's survives (it owns `build` and is
         differentially verified); C's scene-specific readers became free functions
         over A's type. A gained `from_slices` and `Slice::ZERO`.
- [DONE] Applied on request: `AnalyzerError: Clone + Copy`, `SemanticFrame:
         PartialEq`, `CircleShader` setters public. Dropped `tempfile` from
         `musializer-runtime` — Agent E established `rename(2)` cannot cross
         filesystems, so staging writes next to the destination and a temp-dir
         crate is the wrong tool.
- [HURDLE] **The ffmpeg test suite was failing about one run in three**, a
         different test each time. Root cause was the classic Linux fork/exec race:
         one test writes its fake-encoder script while another forks, the child
         inherits the write descriptor, and the first test's `exec` then fails with
         `ETXTBSY`. A module-wide mutex now spans script creation and spawning;
         eight consecutive runs pass. Deliberately test-only — `Encoder::start`
         must never retry `ETXTBSY`, because for a real user that means something
         is writing their ffmpeg binary. **Found by running the suite repeatedly
         rather than once**, which is worth doing again after any process work.

#### Independent confirmation worth noting

- [INFO] Agent B, working from the C without seeing the fix, independently
         reported that `scene::events::EventRecord::is_well_formed` was missing
         `id != 0`, the type range, and `value_count != 0` — the same three rules
         the differential harness had already caught. Two routes to the same
         finding is a good sign about both.

#### Next session: start here

1. Run `tools/verify.sh`. Then read the six `#### Agent` sections above; each ends
   with what its author deliberately left undone.
2. **Highest value first: wire what is already built.** The state for project
   open/save, export, lyrics, Assist, presets and the route editor is ported and
   tested — only drawing and wiring are missing, which is the right way round.
   Agent F's stubs name each one. That is P3 through P6 mostly by connection
   rather than by new code.
3. Absorb the persistence half of routes: Agent B left six functions in
   `project::model` (`mapping_is_constant`, `constant_mapping`,
   `mappings_supported`, `export_mappings`, `import_mappings`,
   `parse_route_spec`) that belong in `scene::routes`, plus `PresetLibrary` into
   `scene::settings`. Move them wholesale — splitting the constant rule from the
   route rule lets one parameter persist as both, the exact ambiguity v1 cannot
   represent.
4. Collapse the local copies agents wrote around each other: `scenes::loom`'s
   `semantic_lane_sample`, `scenes::constellation`'s `event_is_valid`, and the
   three small `Vec2`/`Vec3` copies (a `core::math` was requested twice).
5. `track.h` → `app::Workspace` is still deferred. Agent B's model has landed now,
   so the reason for deferring it is gone.
6. Add a differential harness for anything ported without one. The pattern is in
   `AGENTS.md`; it caught a seven-way error in a module that looked finished.

#### Known gaps in the evidence

- [INFO] **Cadence cannot be visually verified yet.** It typesets timed lyric
         words, the synthetic fixture has none, so captures show only its particle
         swarm. Not evidence of a defect, but not evidence of correctness either. A
         lyric fixture would close it.
- [INFO] Song Atlas draws its idle terrain because whole-track preprocessing is
         not yet run at track load. `SceneRenderer::atlas_map` is where the built
         map goes; `SongAtlasMap::build` already exists and is verified.
- [INFO] ASCII Field draws its procedural rolling spectrogram because
         `--ascii-image` is not wired to a grid. That is its other documented mode,
         not a degraded one.
- [INFO] Nothing has been pushed. All work is local commits on `master`.

### Session 2 (2026-07-27): the interface face and the welcome screen

Both items came from the operator looking at the running application, which is
worth recording on its own: the automated checks were green through a session in
which the entire interface rendered in raylib's 10 px bitmap face, and in which
the first screen a new user sees did not exist. Neither was a subtle defect. See
the new trap in `AGENTS.md`: *a surface nothing photographs does not get
reviewed*.

#### The interface face — landed

`runtime::font` ports the font half of `load_assets` (`plug.c:8060-8137`) plus
`ui_font`/`caption_face` (`plug.c:340-365`).

- **Space Grotesk** for the chrome, at the codepoint subset `ui_font_codepoint`
  defines, and **Alegreya** for captions, at the full curated set. Both faces are
  in `resources/fonts/` under SIL OFL 1.1, copied from the oracle's own
  `resources/fonts/`.
- The subset is built by *filtering the curated caption set*, exactly as the C
  does, rather than by expanding the ranges directly. That is what makes the
  interface face a strict subset of the caption face — so a string that renders in
  a button renders in a caption too. Pinned by a test.
- Loaded once into `Faces`, borrowed for the run. `ShellInput.fonts` carries it so
  no panel can measure a string with one face and draw it with another.
- **The fallback is a `WeakFont`, not a `Font`.** `GetFontDefault()` is a
  non-owning handle into raylib's static storage and `Font`'s `Drop` calls
  `UnloadFont`. Same hazard `runtime::draw` already carries a note about.
- **raylib-rs's safe `load_font_from_memory` cannot be used.** It takes the glyph
  set as a `&str` and passes `str::len()` — a byte count — as the codepoint count.
  Correct for ASCII, out-of-bounds for the curated set. Hence the ffi call and a
  new row in the `unsafe` inventory.

Two consequences worth knowing:

- The C's **third** face (Space Grotesk at the full caption set, for
  `MUSI_CAPTION_FACE_SPACE_GROTESK`) and **fourth** (a project's imported face)
  are deliberately not loaded. Both are selected by caption style, which nothing
  can reach yet, and a 64 px atlas of ~2,000 codepoints is not worth carrying for
  an unreachable selector. `Faces::caption()` is where they land.
- The `'\u{2026}'` → `"..."` substitution in `widgets::draw_button_label` survives
  but is now guarded on `!font.is_loaded()`. The real ellipsis is in the interface
  face's General Punctuation range, so truncated labels finally read as the oracle
  wrote them; the substitution is only reachable on the fallback face, where it is
  still needed.
- Cadence now draws with `fonts.caption()`, which is what the oracle hands it
  (`plug.c:1329` → `scene_cadence.c:449`). ASCII Field deliberately keeps raylib's
  default: `ascii_grid_font` (`scene_ascii_field.c:154-160`) says why — the
  bundled face is proportional and the built-in one is monospaced, which is what
  keeps ASCII samples legible after video encoding.

#### The welcome screen — landed

`Shell::draw_welcome` ports `preview_screen`'s `else` branch
(`plug.c:7769-7830`); geometry is in `shell_layout::WelcomeFrame` so it is
assertable rather than photographed.

This replaces the rewrite's own answer, which was to draw the full workspace with
eleven controls greyed out. The oracle's answer is better and it is the oracle: an
empty workspace makes a first-time user read every disabled control to discover
the one thing they can do.

The startup notice ("Drop an audio file on the window to begin") is **gone**. It
existed because the empty workspace said nothing; the welcome screen says it
properly. It was also actively harmful — being persistent it stayed in the tray
after a track loaded, and on the welcome screen it covered the supported-format
strip along the bottom edge. A capture is what showed that.

#### Runtime track loading — landed, and it was a real gap

The welcome screen prints "or drop audio anywhere in this window", and before this
session that gesture answered "Reopening is not wired up yet". The loop held the
`Music` by shared reference and could not replace it.

`open_track`/`close_track` in `main.rs` own the transition, and the ordering is
the whole reason they are functions rather than code at each site:

1. Load the new `Music` **first**, so a failure leaves the session as it was.
2. Detach the processor **before** the old `Music` drops, or raylib's per-stream
   list holds a callback for a freed stream.
3. Drain the ring **after** the detach — the only moment a consumer safely can,
   since nothing produces then — or the new track's first frames are analysed
   together with the old track's tail.
4. Rebind the analyzer from the **file's** sample rate, not the device's
   (`start_preview_track`, `plug.c:658-660`). Reading the device's rate shifts
   every band.

#### File dialogs — implemented, on the backend the stub recommended

`process::dialogs` was a stub that laid out three candidate backends and named
`kdialog`/`zenity` as child processes as the one fitting its existing machinery.
That is what landed. No new dependency; it is what tinyfiledialogs does on Linux
anyway.

**The display guard is the most important line in that file.** `kdialog` with no
reachable display does not fail politely — it aborts under `SIGABRT`, and Ubuntu
files an Apport crash report for it. This project did that to its operator once
already. `choose_backend` is pure so the guard is testable, and *no test in this
repository calls anything that could open a dialog* — a dialog opened by
`cargo test` would appear on the operator's real desktop.

#### Evidence

`tools/verify.sh`: 10 passed, 0 failed. 733 tests. Oracle clean at `9300af9`.

New in `tools/headless_check.sh`:

- **`fonts:` is parsed from the report and the run fails on either fallback.** The
  regression this closes is invisible to a screenshot review that nobody performs.
- **The welcome screen is captured** at 1280x720 and at the 960x640 minimum. Every
  other capture in that file passes a fixture, so this surface was unreachable.
- **`--probe-reopen PATH` swaps tracks halfway through a probe run**, which is the
  only way to reach the detach/drop/drain/rebind/reattach path from a script: a
  drop gesture and a modal picker can be driven by neither. It is a rewrite-only
  diagnostic, in the same spirit as `--probe-frames`. The check asserts three
  things a clean exit does not prove — the swap ran, it did not report failure,
  and audio arrived through the *second* attachment. Measured: 47,040 frames.

#### Known gaps in this session's evidence

- [INFO] **The `zenity` path is unit-tested but never executed**: zenity is not
         installed on this machine. `kdialog`'s argument syntax was verified for
         real, on a private Xvfb display, by checking that it opens and waits
         rather than exiting with a usage error.
- [INFO] **Clicking "Open audio" and dropping a file are not driven by any
         script.** `xdotool` is absent and installing packages on the operator's
         machine is not this session's call. What is proven is the code both
         gestures reach, through `--probe-reopen`.
- [INFO] The modal picker blocks the render loop while it is open, as
         tinyfiledialogs does in the C. The window stops repainting; that is
         expected, not a hang.

### Session 2 close-out (2026-07-27): the parity rule, and what is actually left

#### The standing rule, from the operator

> "The coding agent does what it must do to achieve the feature parity with the
> before-rewrite state. Which means: the alternatives must be found and
> implemented as need be."

Written up properly in `AGENTS.md` under **"Parity is the goal. A line-by-line
port is not the method"**, including the part that makes it safe: the *mechanism*
is negotiable, anything a user or a file can observe is not. Read that section
before the next feature, because the remaining work is where it starts to bite.

The reasoning behind it is worth keeping. The oracle is most informative for pure
logic — analysis, layout policy, settings tables, formats — and that is precisely
what went first. What remains is what is most entangled with the things this
rewrite deliberately does not reproduce: the single global `Plug *p`, hot reload,
tinyfiledialogs, and C idioms with no Rust shape. The oracle will keep getting
less useful. That is expected, not a sign anything has gone wrong.

#### What is left, checked rather than remembered

The next session's first job is a completion plan. This inventory is the input to
it, verified against both trees this session rather than recalled:

**The PCM → scene-settings coupling — the gnarly one, and it is further along
than it looks.** Three parts, two of them done:

- [DONE] The **engine**: `core::scene::routes` — `AnalysisSource`,
  `Interpolation`, `ParameterMapping::evaluate`/`output_value`/`is_valid_for`,
  `RouteSources::from_audio`, `RouteTable::add`/`remove`/`apply`. Differentially
  verified: 380 route rows exact against the C. Routes are already evaluated into
  a staged copy every frame in `main.rs`, and the inspector already shows a routed
  row in accent with a "routed" readout.
- [DONE] The **editor state**: `core::ui::route_editor_state` is ported.
- [TODO] The **editor UI** — and it needs no design work, which this note
  originally got wrong. It is **not** a new panel and it is **not** the toolbar's
  `+ Feel` / `+ Scene` / `+ Custom` row (those are manual *event* markers, a
  different feature). The route editor expands **the setting's own row inside the
  Tune inspector**, replacing its slider zone: `scene_route_editor_area_height`
  (`plug.c:5517-5523`) is 24+26+40+40+70+26+32+4 px, plus 24 more when the source
  is `band`, and `route_editor_targets` decides which row hosts it. Editing runs
  on a draft; Apply commits, Close discards, and a dirty draft participates in the
  close/context-change guards. All of that is specified in
  `route_editor_state.h:10-16` and already ported. What is missing is the drawing
  and the widget wiring, nothing more.
- [TODO] The **persistence half**. Six functions Agent B left in
  `project::model` belong in `scene::routes`: `mapping_is_constant`,
  `constant_mapping`, `mappings_supported`, `export_mappings`,
  `import_mappings`, `parse_route_spec`. Move them wholesale — splitting the
  constant rule from the route rule lets one parameter persist as both, which is
  an ambiguity the v1 format cannot represent.

So this is "draw an expanded inspector row over ported state" plus "move six
functions", not "port a coupling system", and not a design exercise either. Worth
knowing before it gets budgeted as the hardest item — on this reading it is a
mid-sized one.

The genuinely unported neighbour is the **manual event row** (`+ Feel`,
`+ Scene`, `+ Custom`, `Clear manual` with its confirm/undo states,
`plug.c:2834-2860`). Its policy is ported — `project::event_timeline`,
`project::semantic_lane`, `scene::events` — and the C's tooltips record a rule
worth keeping: manual markers carry one value, so the semantic lane skips them
and only Constellation reacts. Say that in the interface rather than implying
every scene responds.

**The rest, in rough order of how much is already ported underneath:**

| Feature | State underneath | What is missing |
| --- | --- | --- |
| Project open/save (`.musi`) | Agent B's model + codec, ported | Wiring: the four tracks-panel buttons, `--project`, `--save-project` |
| Export / FFmpeg | `process::ffmpeg`, `render_export`, `publish` — all ported and tested | The Export panel, and `--render` |
| Lyrics editor | `lyrics`, `lyric_lane_edit`, `lyrics_editor_layout`, `caption_layout` — ported | The three-pane panel and cue lane |
| Assist | `process::assist`, `assist_ui_state`, `analysis_bridge` — ported | The confirmation panel, `--analysis-bridge` |
| Presets | `preset_store` ported | UI |
| Caption face selector | `font_catalogue`, `font_import_state`, `process::font_import` — ported | The browser panel; `runtime::font` then loads 4 faces, not 2 |
| Song Atlas whole-track map | `SongAtlasMap::build` ported and verified | Run it at track load into `SceneRenderer::atlas_map` |
| `--ascii-image` | `ascii_art` ported | Decode → glyph grid → `ascii_field::draw`'s `Option` argument |
| `track.h` → `app::Workspace` | Agent B's model landed | The multi-track list itself; `App.track` is one `Option<String>` |
| Microphone capture | nothing | `MUSIALIZER_MICROPHONE` is a build flag in the C; decide whether it is in scope at all |

The pattern is consistent and worth stating plainly: **the raylib-free half is
almost entirely ported and tested; what is missing is nearly all panels and
wiring.** That is the good case — it is translation and housekeeping, and it is
why the coupling editor is the only item that needs real design.

#### Next action

Write the completion plan against the table above. Do not re-derive the
inventory; it was checked this session. Do read `AGENTS.md`'s parity section
first, because several of these will not have a faithful port available.

---

## COMPLETION PLAN (session 3 onward)

Supersedes the phase sketches above for everything that remains. Those were
written before the port existed; this one is written against it.

### The one fact that shapes everything

**The raylib-free half is almost entirely ported and tested. What is missing is
panels and wiring.** Every remaining feature has its policy, state machine,
process supervision or codec already in the tree with tests — usually
differentially verified. Almost nothing here is "port a subsystem"; it is "draw
the surface over the subsystem that is already there, and connect it to `main`".

That is why this plan is organised by *surface*, not by C file.

### Scope

**In:** everything in the feature table below.

**Out, deliberately, and recorded so it is not rediscovered as a gap:**

- **Microphone capture** (`MUSIALIZER_MICROPHONE`). Operator's call, 2026-07-27.
  It is a build-flag feature of the frozen binary, nothing else depends on it, and
  it needs a second audio path the bridge does not have. Parity is declared
  without it. If it ever comes back, `capture_screen` (`plug.c`) and a second
  `audio_bridge` producer are the whole of it.
- **Hot reload.** Already an explicit first-pass non-goal.
- **Windows and macOS.** Linux-first hobby rewrite.

### Dependency order

Three items unblock others; everything else is independent. Do not fan out until
the first band has landed.

```
Band 0 (integration owner, before any fan-out)
  W1  .musi open/save wiring          -> unblocks presets, route persistence, W2
  W2  track.h -> app::Workspace        -> unblocks the tracks panel, per-track state
  W3  pre-scaffold the module tree     -> prevents the mod.rs collisions of session 1

Band 1 (parallel, six agents)
  G  route editor row + persistence   (needs W1)
  H  export panel + --render
  I  lyrics editor panel
  J  assist panel + --analysis-bridge
  K  font browser + caption faces
  L  manual event row + presets       (needs W1, W2)

Band 2 (integration owner, after the merge)
  M  Song Atlas whole-track map at load
  N  --ascii-image -> glyph grid
  O  the parity gate
```

`W2` is the one that will hurt if it is skipped: `App.track` is a single
`Option<String>` and six panels want a real track list underneath. Doing it after
the fan-out means six agents each invent a shim.

### Working agreements for the fan-out

Session 1's fan-out produced zero source collisions and three duplicated types.
Keep what worked and close what did not:

1. **Pre-scaffold every module before branching.** `W3` creates each file below
   with a doc comment, its `pub mod` line already registered, and nothing else.
   Agents fill files; they never edit `mod.rs`. This is what prevented collisions
   last time.
2. **Only the integration owner edits the root manifest, `main.rs`,
   `scene_host.rs`, `theme.rs` or `widgets.rs`.** An agent that needs a widget or
   a colour *requests* it; adding one is a five-line merge conflict in the file
   every agent touches.
3. **Duplication is a merge cost, not an agent's problem.** Session 1 lost time to
   `SongAtlasMap`, `SceneViewport` and three `Vec2`/`Vec3` copies. `W3` therefore
   also lands the `core::math` that was requested twice, and the shared
   `runtime::draw` helpers, *before* the branch.
4. **Every agent appends a `#### Agent X` section to NOTE ENTRIES.** Merge
   conflicts there are resolved by keeping all sections.
5. **A panel that cannot host its own controls is not drawn at all.**
   `workspace_layout.h`'s rule; it is why invisible zero-height panels stole
   clicks in the C.

### Definition of done — applies to every item

An item is done when **all** of these hold. This is the checklist that would have
caught the font gap and the missing welcome screen.

- [ ] It draws, and `tools/headless_check.sh` **captures it** at 1280x720 *and*
      at the 960x640 minimum. A surface nothing photographs does not get reviewed.
- [ ] The capture is reachable: if the state needs a probe flag to reach, the flag
      exists. `--ui-probe` already carries `panel=`, `assist=`, `lyric=`, `style=`,
      `fonts=`, `lyrics-file=` and they currently report "not implemented" —
      honouring them *is* part of the item.
- [ ] The report prints **evidence, not existence**: a line a script can assert
      on, distinguishing "drew something" from "did the thing". Follow the
      `fonts:` and `reopen:` lines.
- [ ] Anything with a C counterpart that is pure gets a **differential harness**
      (`tests/differential/` + `crates/musializer-core/examples/` + `tools/`). The
      pattern found a seven-way error in a module that looked finished.
- [ ] `cargo clippy --all-targets` silent, `cargo fmt --check` clean, no new
      `unsafe` without a `SAFETY:` comment *and* a row in `AGENTS.md`.
- [ ] `tools/verify.sh` green, oracle still clean at `9300af9`.
- [ ] Divergences from the oracle recorded with their reason, per `AGENTS.md`'s
      parity section.

### The work

Each brief names the C sources, the Rust that already exists underneath, and what
is actually missing. Where the oracle does not settle a question, the brief says
so — those are the only places invention is required.

#### W1 — `.musi` open/save wiring  *(integration owner)*

- **C:** `project_io.c`, `project.c`, `plug.c:4634`, `:5049`, `:5180-5191`
- **Have:** `project::model`, `project::io`, `project::assets`, `project::sha256`,
  `process::publish` (transactional rename), `dialogs::filters::MUSIALIZER_PROJECT`
- **Missing:** the four tracks-panel buttons reaching it, `--project`,
  `--save-project`, and the unsaved-work guard on quit (`plug.c:7248`, which is
  `dialogs::confirm_warning` — note its contract: an `Err` must **keep** the work,
  never discard it).
- **Watch:** `export_mappings`/`import_mappings` move to `scene::routes` in G, not
  here. Do not duplicate them.

#### W2 — `track.h` → `app::Workspace`  *(integration owner)*

- **C:** `track.h`, `plug.c`'s `Tracks`/`current_track`
- **Have:** Agent B's model; the deferral reason is gone.
- **Missing:** the type itself. `App.track: Option<String>` becomes a real list
  with a selection, per-track settings, per-track routes, per-track editor drafts
  and `scene_seed_for_track` (which replaces `DEFAULT_SCENE_SEED`).
- **Why first:** the route editor keys its draft by `track_slot`, and the C's
  close/context-change guards are all per-track. Six panels want this underneath.

#### G — the route editor row, and route persistence

- **C:** `route_editor_state.c/h`, `scene_routes.c`, `plug.c:5517-5528`,
  `:6179-6215`
- **Have:** the engine (380 route rows differentially exact),
  `ui::route_editor_state`, live per-frame evaluation, the routed readout
- **Missing:** (a) the expanded inspector row — **not a new panel**; it replaces
  the setting's 30 px slider zone and makes the row
  `24+26+40+40+70+26+32+4` px tall, plus 24 when the source is `band`. Draft
  edits, Apply commits, Close discards, dirty participates in the guards.
  (b) Move six functions out of `project::model` into `scene::routes`:
  `mapping_is_constant`, `constant_mapping`, `mappings_supported`,
  `export_mappings`, `import_mappings`, `parse_route_spec`. **Move them
  wholesale** — splitting the constant rule from the route rule lets one parameter
  persist as both, an ambiguity the v1 format cannot represent.
- **Harness:** yes. Round-trip every descriptor through export/import against the C.

#### H — export panel and `--render`

- **C:** `render_export.c`, `ffmpeg_posix.c`, `plug.c:7132`, `:7157-7160`
- **Have:** `process::ffmpeg` (with the `ETXTBSY` test lock), `timing::render_export`,
  `process::publish`, `WorkspaceFrame::export_timeline_height`
- **Missing:** the panel, the save dialog, progress and cancel, and the
  fast-forward window (`render_start_frame`) that keeps windowed exports
  bit-identical to the same frames of a full render.
- **Evidence:** encode a short synthetic fixture in the headless check and assert
  frame count and determinism — the same export twice must be byte-identical.

#### I — lyrics editor panel

- **C:** `lyrics_editor_ui.c`, `lyrics_editor_layout.c`, `lyric_lane_edit.c`
- **Have:** all three ported, plus `caption_layout` and the caption face
- **Missing:** the three-pane editor, the cue lane, and text entry — which this
  codebase has none of yet. **Invention required:** a text input widget.
  `widgets.rs` has no caret, selection or clipboard. Design it once, in
  `widgets.rs`, via the integration owner.

#### J — assist panel and `--analysis-bridge`

- **C:** `analysis_bridge.c`, `analysis_candidate.c`, `assist_ui_state.c`,
  `plug.c:2143`, `:2176-2337`
- **Have:** all of it, including `process::assist` with the `setsid`/`EPERM`
  trap already handled and tested
- **Missing:** the confirmation panel naming the lyric sheet a run will use, with
  Choose/Replace/Clear; the importer; `--ui-probe assist=`.
- **Do not touch:** `process::assist`'s process-group handling. There is a test
  that fails loudly with the reason if anyone "simplifies" it.

#### K — font browser and the two missing faces

- **C:** `font_catalogue.c`, `font_import_state.c`, `plug.c:373-426`, `:1547+`
- **Have:** `ui::font_catalogue`-side state, `process::font_import`,
  `runtime::font`
- **Missing:** the browser panel, the once-per-run network consent (**deliberately
  not persisted** — consent to contact a third party is asked once per run, not
  remembered until someone withdraws it), and then `runtime::font` grows from two
  faces to four: Space Grotesk at the full caption set, and the project's imported
  face keyed by path. `Faces::caption()` is the seam.
- **Open question, unresolved:** Agent E's deliberate `font_catalogue_parse`
  atomicity divergence. Settle it here.

#### L — manual event row and presets

- **C:** `plug.c:2834-2880`, `event_timeline.c`, `semantic_lane.c`,
  `preset_store.c`
- **Have:** all the policy, plus `scene::events` (the module the header comment
  got wrong in seven ways — trust the `.c`)
- **Missing:** the `+ Feel` / `+ Scene` / `+ Custom` row with `Clear manual`'s
  confirm/undo states, and the preset UI.
- **Keep the honesty:** manual markers carry one value, so the semantic lane skips
  them and **only Constellation reacts**. The C's tooltip says so; say it too
  rather than implying every scene responds.

#### M, N — the two scene gaps  *(integration owner)*

- **M:** run `SongAtlasMap::build` at track load into `SceneRenderer::atlas_map`.
  Both sides exist and are verified; this is a call site. It belongs in
  `open_track`.
- **N:** `--ascii-image` → decode → `ascii_art`'s glyph grid → the `Option`
  argument `ascii_field::draw` already takes.

#### O — the parity gate

Parity is reached when, and only when:

- Every item above is done by the checklist.
- A `.musi` written by the frozen C opens here, and one written here opens in the
  frozen C, with no field lost. **Round-trip both directions.**
- The differential harnesses all pass, and every pure module ported without one
  has since acquired one.
- `tools/headless_check.sh` captures every panel and every scene, and asserts
  something about each beyond "a file exists".
- The out-of-scope list above is the complete set of differences, and it is in
  `AGENTS.md` where a user would find it.

### Known unknowns

Three things this plan cannot settle from the oracle, flagged rather than guessed:

1. **Text entry** (item I) is genuinely absent from this codebase. It is the one
   piece of real UI engineering left.
2. **The three version-string spellings** in the C, still unresolved from session
   1. Pick one at the gate.
3. **The `.musi` fixture strategy**, also open from session 1. A round-trip test
   against the C needs a fixture, and the repository rule is synthetic only —
   which is fine, but the fixture has to be *generated*, not committed from a real
   project.

---

## NOTE ENTRIES — session 3

### Band 0 is landed: W1, W2, W3

All three integration-owner items are done and committed. The fan-out is
unblocked. Two of the three changed the plan they were written from.

#### W2 — `app::Workspace` (commit `9e7c6b5`)

`track.h`'s `Track`/`Tracks` came across whole, minus the `Music`. **That
omission is the one design decision in it and it is settled, not open:**
raylib-rs's `Music` borrows the audio device, so a `Track` holding one makes the
whole model `Workspace<'audio>` and infects `App`, `ShellInput` and every panel.
It is also unnecessary — selecting a track in the C *stops* the outgoing stream
(`plug.c:5273`) and replays the incoming one from zero, so no playback state
survives a switch. `main.rs` keeps one `Music` for the current track and reopens
on switch; the cost is one `LoadMusicStream`, which is a streaming open.

Three `Option`s replace C sentinel pairs (`project_metadata_initialized`, the
ASCII cell/dimension bundle, every possibly-empty path). `song_atlas_map_attempted`
is deliberately *not* folded in: "no map" and "tried and failed" differ, and the
C distinguishes them to avoid retrying every frame.

`App` keeps a *pending* settings/routes pair — `p->scene_settings` and
`p->pending_scene_routes` — used with no track open, with the pending routes
handed to the first track that loads (`plug.c:852`). `App::settings()` is
`track_effective_scene_settings` and is the **edit** target as well as the read
target, because the C returns a non-const pointer and the inspector writes
through it.

Two parity fixes fell out:

- `DEFAULT_SCENE_SEED` was invented. It is now the oracle's
  `0x4D555349414C495A` (`plug.c:8401`), so a fresh track's scene state matches
  the C's for the same audio.
- the frame's `duration_seconds` now comes from the track, not the stream
  (`plug.c:1169`) — which is what survives a track added but not playing.

**`core::ui::scroll_list` is new and shared.** The tracks list scrolls, and the
oracle's geometry, momentum and scrollbar (`plug.c:5213-5382`) are policy, so
they are raylib-free and tested rather than living in the drawing code. The C
keeps `panel_scroll`/`panel_velocity`/`scrolling` as **function statics**, which
is why its list code can only ever serve one panel; per-list state here is the
divergence that lets the inspector and both browsers use the same policy. Two
oracle behaviours are reproduced rather than corrected and both look like bugs:
damping is per *frame* (`velocity *= 0.9` once per update, so 60 fps decays twice
as fast as 30), and the offset is clamped after integration while the velocity
is not (so a fling rests against the end rather than bouncing).

`UiRect::contains_point` was added because `contains` deliberately refuses an
empty inner rect, so a point expressed as a zero-size rect always read as
outside. Two callers had already reached for it.

**The capture is what caught the first attempt.** It passed every check in
`headless_check.sh` while silently dropping track 2, because the row stride was a
guess (`row_height + 4`) rather than the oracle's `item_size`. The report now
carries a `tracks:` line and the check asserts on it.

#### W1 — `.musi` open and save (commit `e30e336`)

`runtime::project_files` is the filesystem half that `project::io`'s own module
comment said was deliberately absent from the core crate. The rule worth knowing:
**a bundled asset must resolve back to the very same relative path it was stored
as.** `resolve_bundled_asset_path` canonicalizes the asset *and* the project
directory, recomputes the descendant path, and refuses anything that does not
match byte for byte (`project_io.c:874-884`). Without it "bundled" would only
mean "found". A test proves a symlink into the bundle is refused.

`app::project` is `build_project` / `save_project_to_path` / `open_project_path`.
Open verifies every digest before a `Track` exists and builds the whole `Track`
before anything is replaced; save publishes assets first and adopts the published
copies after. Neither leaves a half-state.

Three findings:

- **the tracks action row's labels were invented.** The oracle's are
  `"Open project"`, `"Add audio"`, `"Save"`, `"Save As"` (`plug.c:5165`), and
  there is **no "Close"** — the frozen C cannot close a single track. A `Close`
  button would have been an invented feature wearing parity's clothes.
- **`PresetLibrary` could only allocate ids, not restore them.** Hydrating a
  project through `push` would have renumbered every preset, quietly breaking any
  reference across a round trip. `PresetLibrary::restore` is the missing half.
- **the quit guard diverges, deliberately.** `tinyfd_messageBox` always answers
  because tinyfd falls back to a terminal prompt; `kdialog`/`zenity` may be
  absent. Refusing forever traps the user, quitting silently loses the work the
  guard exists to protect. So an unavailable dialog refuses the **first** request
  and says why in the tray and on stderr, and honours the second.

Still to hook in, by the agent who owns each: `confirm_close` weighs only dirty
projects — the open lyric draft (I), the open route edit (G), staged Assist
suggestions (J), a running analysis (J) and a running export (H) each add a line.
`autosave_is_due` takes an `editor_dirty` parameter for the same reason.

Autosave currently only writes the **current** track, because only it has a bound
stream and `build_project` needs the sample rate and channel count off it
(`plug.c:4304`). The C autosaves every track. Closing that means caching those
two numbers on the `Track` at load; it is a small, known gap.

#### W3 — the scaffold (this commit)

`ui/panels/` is one file per agent, every `pub mod` line already registered.
**No agent edits a `mod.rs`, `shell.rs`, `widgets.rs`, `theme.rs` or the root
manifest.** An agent that needs a widget or a colour requests it.

Three surfaces nest inside another agent's, and the call sites are defined rather
than left to be negotiated:

| Surface | Owner | Called from |
| --- | --- | --- |
| the route editor row | G | `tune.rs`'s per-setting row loop, already wired |
| the font browser pane | K | `lyrics.rs`'s three-pane editor |
| the manual event row | L | `events.rs`, from the timeline strip |

`route_editor_height` is asked **before** the row is measured, so a row that will
not fit is never drawn. That is the layout rule this repository has already paid
for, applied to a row rather than to a panel.

The font browser is a **pane inside the lyrics editor**, not a panel: the oracle
drives it with `p->lyric_editor.font_pane` (`plug.c:3786`). There is therefore no
`UiPanel` variant for it, and K does not need to touch `cli.rs`.

**`core::math` was not landed, and the plan line asking for it no longer
applies.** It was justified by scene agents duplicating `Vec2`/`Vec3`/`clamp01` in
session 1. Band 1 is panels; no Band 1 agent touches a scene file, so there is no
collision to prevent, and the eleven `clamp01` copies are in modules that are
already differentially verified. Consolidating them is a cleanup, not a
prerequisite, and doing it mid-fan-out would risk numeric parity for no benefit.

**A capture caught this item too.** The first split drew the "not built yet" box
into the panel's content rectangle and printed it over the timeline's ticks; the
oracle's box starts below the strip (`strip.y + strip.height + 28.0`). No test
would have found it.

#### Agent G — the route editor row, and route persistence

**(b) is complete and verified. (a) draws, is captured at both window sizes, and
edits a draft — but it cannot commit one**, because committing needs two
`ShellCommand` variants and the fan-out rules put `shell.rs` and `main.rs`
off-limits. What is needed to close that is listed under "Four seams" below, and
every one of them is a few lines in a file only the integration owner may touch.

##### (b) The six functions moved, and a seventh copy died with them

`mapping_is_constant`, `constant_mapping`, `mappings_supported`,
`export_mappings`, `import_mappings` and `parse_route_spec` (plus
`ROUTE_SPEC_MAX_BYTES`) are now in `core::scene::routes`, together, with their
tests. **No `pub use` re-export was left behind** — there are four call sites in
the tree and updating them was smaller than leaving two spellings of one home.
`MAX_MAPPINGS_PER_SCENE` and `capacity::PARAMETER` stayed in `project::model`,
because they are `.musi` schema bounds the project validator also reads.

**`cli.rs` had a second, independent implementation of the `--route` grammar.**
Its own doc comment asked for exactly this — "if a `ParameterMapping::parse_spec`
lands there, this should delegate to it rather than keep a second copy, a
duplicated grammar drifts" — so it now does. The two differed in ways nobody
would have found by reading: the CLI copy rejected an empty spec explicitly and
never bounded the parameter key against the mapping's 64-byte buffer. Both
reached the same answer, but only by accident of two other checks.

`tools/differential_route_persistence.sh` is the harness. **381 lines, zero
delta.** It checks the four things together rather than one at a time, which is
the point: `.musi` v1 has one representation for "this parameter has a value", so
a slider constant is a full-range RMS mapping with equal output endpoints and a
route is the same struct with unequal ones. The constant rule and the route rule
are two halves of one decision, and a harness that split them could pass while a
parameter became eligible as both. It dumps the parse grammar over 35 specs
(every optional-token permutation, every rejection the grammar owes, and the
64/65-byte key boundary), every descriptor of every scene exported as constants,
the same export with five routes replacing slots in place, the full import back,
and hand-built probes for the cases export cannot reach — a duplicate parameter,
an unknown one, and three mappings that are *almost* the canonical constant.

**Not wired into `tools/verify.sh`** — that file is shared. Add:
`tools/differential_route_persistence.sh` beside the other three.

##### (a) The row

The editor **area** is the oracle's exactly: `24+26+40+40+70+26+32+4` px, plus 24
for the band stepper, and the drawing advances its cursor by those same numbers in
that order. The row *around* it diverges and the reason is written at
`ROUTE_EDITOR_ROW_HEADER`: the C's setting row is 76 px with a 29 px header and
12 px of trailing rule space, so its expanded row is `area + 41`; this inspector's
row is 46 px with a 22 px header and a 4 px gap, so its expanded row is
`area + 26`. Copying the 41 would put 15 px of nothing under a block that already
ends in 4 px of its own.

Three things about the row that are behaviour rather than pixels:

- **The label and the readout are drawn for every row, expanded included**
  (`plug.c:6156-6190`). The first version drew them only for collapsed rows,
  which left the one row the user is working on as the only one that did not say
  what value it currently produces. A capture caught it.
- **A routed setting is not a slider.** It shows the summary and the live meter,
  and the whole zone is a hit target that opens the editor (`plug.c:6197-6215`).
  Before this, a routed row drew a slider that appeared to move on its own.
- **A dirty draft refuses the `~` button and says why** rather than being
  replaced (`plug.c:6174-6178`). That is the entire reason the draft exists.

##### Four seams, and the exact lines that close them

Each is marked `SEAM n:` in `ui/panels/tune.rs`, so `grep -n 'SEAM ' ` finds all
of them.

1. **The draft has nowhere to live.** It sits in a `thread_local RefCell` in
   `tune.rs` — the only global mutable state in the application crate, and it
   should not survive. Add `pub route_editor: RouteEditorState` and
   `route_editor_track: usize` to `Shell`; `with_editor`/`peek_editor` are the
   only two functions that touch it.
2. **Apply and Remove cannot commit.** They draw with the oracle's enable rules
   (Apply live only for a valid draft that is dirty or never committed; Remove
   only with a committed route behind it; Close reads `Discard` while dirty) and
   push `ShellCommand::NotImplemented` on click. They need
   `ShellCommand::ApplyRoute { scene, route: ParameterMapping }` and
   `ShellCommand::RemoveRoute { scene, parameter: String }`, handled in `main.rs`
   through `app.routes_mut()` followed by `mark_project_dirty` (`plug.c:5852`,
   `:5862`). Two lines here become the push; the rest is already written.
3. **The live source values are not in `ShellInput`.** `main.rs` builds a
   `RouteSources` for the frame loop and hands the shell only `rms`, so RMS reads
   live and the other four sources honestly report `no signal` — the C's own
   wording for an unavailable source. `pub route_sources: RouteSources<'a>` on
   `ShellInput` and `Shell::live_value` becomes one call. Until then the band
   source's meter and the transfer graph's live dot are dark, which is visible in
   the `route-editor-band-*` captures.
4. **`--ui-probe route=KEY` is applied from `tune.rs`, not `main.rs`.** The key
   is parsed and resolved in `cli.rs` (a typo fails the command line, which the
   headless check asserts on), but honouring it means re-reading `argv` through
   the same `parse_ui_probe`. Move it to `main.rs`'s probe block and both
   `apply_route_probe` and the `probe_applied` flag go.

Also still to hook in, and named in W1's note as G's line to add:
`confirm_close` in `main.rs` should weigh an open route edit.
`ui::panels::tune::route_edit_is_dirty()` exists for exactly that call and is
`#[allow(dead_code)]` until it is made.

##### The captures, and what they assert

`tools/headless_check.sh` grew a "the route editor row" section: three states —
collapsed, opened onto a committed band route, opened fresh on an unrouted
setting — at 1280x720 and at the 960x640 minimum, plus a negative control that a
mistyped `route=` key exits non-zero.

The evidence line is `route editor: <key> open row=<h>px source=<S> committed=<n>`,
printed from `tune.rs` because `Report::print` is in `main.rs`. The check asserts
the exact heights, 312 for the band draft and 288 for the fresh one, because a
24 px difference is what proves the editor opened onto the *right* draft rather
than merely opening. If `Report::print` ever takes this over, that is the wording.

##### Two things a later session would otherwise re-derive

- **`route_editor_height` is not handed the workspace**, so it cannot check the
  track slot the draft is keyed against. The slot is cached on the host at the
  top of `inspector`. Without that, a hidden draft belonging to another track
  reshapes this track's list — which `route_editor_targets` is track-scoped
  specifically to prevent (`route_editor_state.c:101-114`), and there is a test.
- **The C's `parse_route_spec` accepts a 64-byte parameter key and rejects 65**,
  because `MUSI_PROJECT_PARAMETER_CAPACITY` is 65 *with* the NUL. `capacity::PARAMETER`
  is 64 and the Rust check is `> capacity::PARAMETER`, which is the same boundary.
  It reads like an off-by-one and is not; the harness pins both sides of it.
---

#### Agent K — the font browser pane and the two missing faces

**Landed, except two call sites in shared files.** Everything below builds, tests
and is clippy/fmt clean; nothing in `shell.rs`, `main.rs`, `widgets.rs`,
`theme.rs`, `cli.rs` or any `mod.rs` was touched. The pane is written and tested
but **is not drawn yet**, because both of its entry points live in files this
agent may not edit. The two diffs are at the bottom of this note; until they are
applied `ui/panels/fonts.rs` carries a module-level `#![allow(dead_code)]` whose
comment says to delete it when they land.

##### `runtime::font` is four faces now

`Faces` carries the interface subset of Space Grotesk, Alegreya at the full
curated caption set, **Space Grotesk at the full caption set**, and a project's
imported face. `Faces::caption_for(CaptionFace)` is `caption_face`
(`plug.c:350-364`) including its fallback rule — a style naming a face this build
could not rasterize gets **Alegreya**, never raylib's default, because the bitmap
face has none of the curated coverage and would silently drop every accent.

Two things worth knowing:

- **`caption()` survived as the no-argument default** rather than becoming
  `caption(face)`. `scene_host.rs:308` calls it and that file is the integration
  owner's; a signature change there is a one-line diff nobody asked for. It now
  means "the caption default", and its doc says a scene drawing a *project's*
  captions must use `caption_for` with that track's style instead. That call site
  is a real, small parity gap: the cadence scene's overlay ignores the caption
  face the project selected.
- **The imported face is keyed by its path** (`plug.c:389-392`), and
  `load_imported` returns early when asked for the face it already holds. That
  early return is not an optimization — without it a per-frame call leaks a 64 px
  atlas every frame. The 32 MiB ceiling and the `.ttf` extension default are the
  oracle's (`plug.c:369`, `:411-415`); the size gate is split out as
  `imported_face_size_is_usable` because it is the half of that function reachable
  without a window.

No new `unsafe`. The four existing blocks now also carry heap bytes read from
disk rather than only `include_bytes!` arrays, so the `SAFETY:` comment and the
`AGENTS.md` row were both corrected to say why that is equally sound: raylib
copies out what it needs before returning and keeps no pointer into the buffer.

The report line grew to name all four slots, and the imported one reports its
**path** — "an imported face is loaded" and "the imported face this project
names is loaded" are different claims and only the second is worth a check.

##### The pane, and where its state lives

`ui/panels/fonts.rs` reproduces `draw_font_consent` and `draw_font_browser`
(`lyrics_editor_ui.c:617-833`): consent, loading, fetching, cancelling, failed,
and the browsing body with its search field, its scrolling family list, its
"N of M" count and its "Download and use" row.

Everything that is a decision went somewhere headless:

| Decision | Where |
| --- | --- |
| rectangles, the 420x150 threshold, how many rows fit | `core::ui::font_import_state::BrowserLayout` |
| query editing, scroll window, selection, consent | `core::ui::font_import_state::BrowserView` |
| the child, the digests, the catalogue | `runtime::process::font_import::FontImporter` |

`FontImporter` is the oracle's nine `font_service_*` functions plus
`poll_font_job` and both finish steps, as one object instead of reaches into a
global `Plug *p`. That is what makes the whole state machine testable with a fake
helper: **eleven new tests drive consent refusal, a missing helper, a browse, a
verified download, a download whose digests do not hold, a track that closed
mid-download, a second request while one is in flight, cancellation, a helper
that writes nothing, and a local family list — and not one of them touches the
network.**

`FontImporter::poll` stops one step short of the C: it hands back a
**digest-verified** manifest rather than writing the caption style itself.
Rasterizing needs `&mut Faces` and recording needs `&mut Workspace`, and the pane
holds both as shared references. The last two steps of `font_job_finish_fetch`
(`plug.c:1762-1798`) therefore happen in `main.rs`, and they must stay **one
step**: a project that names an imported face without carrying the asset fails
its own validation.

##### Divergences, with reasons

1. **`font_catalogue_parse` atomicity — settled, keeping Agent E's improvement.**
   The C withholds only `count` on a failed parse, so a refresh that fails after
   a good row leaves the old count beside new rows: the picker keeps listing N
   families and the first few are now from the failed refresh. Three reasons the
   local-`Vec`-then-commit stays: nothing observable diverges (a catalogue never
   reaches a `.musi`, a frame, a number or a command line, so `AGENTS.md`'s test
   says take the better option); the oracle's own header *and* its own test ask
   for the behaviour we implement, and the C passes that test only because every
   case it exercises fails before the first row is written; and Rust cannot leave
   a `Vec`'s length disagreeing with its contents without deliberate extra code to
   make the corruption possible. Recorded on `parse_replace` in full, with the
   counter-argument, and pinned by
   `a_refresh_that_fails_after_a_good_row_leaves_the_old_catalogue_whole`.
   **A differential harness over this function would disagree with the C on
   exactly one class of input — a malformed row after at least one good one — and
   that is expected.**
2. **The selection is a family name, not a catalogue index.** The C stores an
   index and its own header comment says why that is wrong: "A catalogue refresh
   can renumber every row, so the family name is re-resolved rather than the index
   being trusted across a reload" (`lyrics_editor_ui.h:68-71`). The implementation
   never does the re-resolution — `can_import` only bounds-checks
   (`lyrics_editor_ui.c:813-814`) — so a refresh between choosing and pressing
   downloads a *different family*. Storing the name is the comment's stated
   intent. Same class as (1): the comment is right and the code is not.
3. **The selected row uses the palette's accent, not `GetColor(0xE7ECFAFF)`.**
   That literal is not in `ui_palette.h`, so it is invisible to the contrast suite
   that header exists for — exactly the failure `theme.rs`'s module comment warns
   about. The accent/white pair *is* contrast-checked, and adding a palette entry
   would mean editing `theme.rs`.
4. **`--ui-probe fonts=PATH` is invented**
   (`FontImporter::load_catalogue_from_file`). Nothing in the oracle reaches the
   browsing state without contacting Google, and no check here may do that. It
   grants **no** consent: a probe that quietly answered the consent question would
   make the consent panel unreviewable, which is the exact failure the probe
   exists to prevent. There is a test for that.
5. **The helper is resolved once, at construction**, not re-probed per request. A
   browser that starts offering a feature halfway through a run because a file
   appeared is stranger than a consistent refusal.
6. **`current_exe()` stands in for `GetApplicationDirectory()`.** Same fact, and
   it keeps `FontImporter` constructible without a window.

##### The consent is not a preference, and must not become one

`BrowserView::network_allowed` starts false in every run, is never written
anywhere, and has no setter that takes a stored value. `plug.c:1542-1548` is
explicit about why — "'I wanted a font earlier' is not standing permission to
contact anyone later" — and the consent copy promises it in as many words:
"Musializer asks again next time it starts."
`network_consent_starts_refused_every_run_and_cannot_be_restored` fails if anyone
adds one.

##### What is unfinished, and what I need

**Not done, and honestly not done:**

- **The pane is not drawn and has no capture.** Its only caller is Agent I's
  lyrics editor. Nothing photographs it, so by the definition of done this item is
  incomplete — and this is precisely the "a surface nothing photographs does not
  get reviewed" trap, arrived at from the merge side rather than the code side.
  The capture belongs in `tools/headless_check.sh` once the pane is reachable, at
  1280x720 and 960x640, driven by `--ui-probe panel=lyrics,fonts=PATH` with a
  **generated** family list under `build/`.
- **`--ui-probe fonts=` still reports "not implemented"** in `main.rs`, and the
  branch that says so also covers four other agents' flags.
- **The caption face selector** — the `FACE` row's third choice and the
  "Import a face..." / "Remove" buttons (`lyrics_editor_ui.c:855-928`) — is in the
  caption style form, which is Agent I's file. `FontBrowser::clear_selection` and
  `Faces::clear_imported` are the halves it needs from here.
- **No differential harness.** `font_catalogue.c`'s readers were already ported
  with the C test suite in full by Agent E, and the one function worth comparing
  numerically is the one this note deliberately diverges on. A harness over
  `font_catalogue_filter` and `font_scripts_describe` would still be worth having.
- **The cadence scene's overlay ignores the project's caption face** (see above).

**Exact diffs needed from the integration owner.**

`crates/musializer-app/src/ui/shell.rs` — one field, so the pane's state survives
a frame:

```rust
 use crate::cli::UiPanel;
+use crate::ui::panels::fonts::FontBrowser;

 pub struct Shell {
     ...
     track_scroll: ScrollState,
+    /// The caption face browser's state and its importer (Agent K).
+    pub font_browser: FontBrowser,
 }

 // in Shell::new()
             track_scroll: ScrollState::new(),
+            font_browser: FontBrowser::default(),
```

then in `crates/musializer-app/src/ui/panels/fonts.rs`, drop the `browser`
parameter and the module-level `#![allow(dead_code)]`:

```rust
     pub(crate) fn font_browser_pane(
         &mut self,
         d: &mut RaylibDrawHandle<'_>,
         input: &ShellInput<'_>,
-        browser: &mut FontBrowser,
         area: UiRect,
         commands: &mut Vec<ShellCommand>,
     ) {
         let _ = commands;
-        draw_font_browser(&mut self.widgets, d, input, browser, area);
+        let Shell { widgets, font_browser, .. } = self;
+        draw_font_browser(widgets, d, input, font_browser, area);
     }
```

`crates/musializer-app/src/main.rs` — four changes:

```rust
 // 1. the per-frame poll, next to the other per-frame work, BEFORE shell.draw.
 //    A download that finished behind a closed pane still has to be reaped.
+        app.shell
+            .font_browser
+            .poll(app.workspace.current().is_some());
+        if let Some(manifest) = app.shell.font_browser.take_import() {
+            // The rest of `font_job_finish_fetch` (plug.c:1762-1798). One step:
+            // the face becomes the selected one in the same move that records
+            // the asset, or the project names a face it does not carry.
+            let path = std::path::PathBuf::from(&manifest.font_path);
+            if fonts.load_imported(&mut rl, &thread, &path) {
+                if let Some(track) = app.workspace.current_mut() {
+                    let style = &mut track.caption_style;
+                    style.face = CaptionFace::Imported;
+                    style.font = Some(FontAsset {
+                        // `path`/`licence_path` are written by the save, which
+                        // decides where in the bundle they land. Empty until then.
+                        path: String::new(),
+                        sha256: manifest.font_sha256.clone(),
+                        family: manifest.family.clone(),
+                        licence_path: String::new(),
+                        licence_sha256: manifest.licence_sha256.clone(),
+                        licence_name: manifest.licence_name.clone(),
+                    });
+                    track.caption_font_path = Some(path);
+                    track.caption_licence_path =
+                        Some(std::path::PathBuf::from(&manifest.licence_path));
+                    track.project_dirty = true;
+                    app.shell.notify(
+                        Severity::Success,
+                        "Caption face imported",
+                        &format!(
+                            "{} is now the caption face. It is licensed under {}, and both \
+                             the face and its licence are saved with the project.",
+                            manifest.family, manifest.licence_name
+                        ),
+                    );
+                }
+            } else {
+                app.shell.notify(
+                    Severity::Error,
+                    "Caption face not imported",
+                    "The downloaded face is not a font this build can rasterize.",
+                );
+            }
+        }

 // 2. honour `--ui-probe fonts=`, and drop `fonts=` from the "not implemented" list.
+        match probe.font_browser.as_ref() {
+            None => {}
+            Some(cli::FontBrowserProbe::Consent) => {}   // the default state
+            Some(cli::FontBrowserProbe::Catalogue(path)) => {
+                // Reads a family list from disk. Never contacts anybody, and
+                // deliberately does not grant consent on its own.
+                if let Err(error) = app.shell.font_browser.load_catalogue_from_file(path) {
+                    eprintln!("warning: --ui-probe fonts=: {error}");
+                }
+                app.shell.font_browser.allow_network();
+            }
+        }

 // 3. the report, next to `fonts:`.
+        println!("font browser:    {}", app.shell.font_browser.describe());

 // 4. an imported face survives a project open. In `open_project_path`'s
 //    caller, after the track is installed:
+        if let Some(path) = app.workspace.current().and_then(|t| t.caption_font_path.clone()) {
+            if !fonts.load_imported(&mut rl, &thread, &path) {
+                eprintln!("FONT: the project's imported caption face could not be rasterized");
+            }
+        }
```

`crates/musializer-app/src/ui/widgets.rs` — optional and cosmetic: move the
pane's `0x464F_4E54` namespace into `widgets::id` as `FONTS`. It cannot collide
today (a test in `fonts.rs` proves it against all six existing namespaces), so
this is tidiness, not a fix.

##### For whoever writes the capture

`--ui-probe panel=lyrics,fonts=consent` photographs the consent panel — the
question itself, which is the screen most worth reviewing, since it is the one
that decides whether a user understands what leaves their machine. `fonts=PATH`
photographs the browsing body with a generated list. Assert on the
`font browser:` report line, not on the pixels: `consent=not asked` versus
`consent=granted` and `families=N` are the two facts a screenshot cannot prove on
its own.
#### Agent L — the manual event row and the preset UI

Both surfaces are drawn and their policy is ported, tested and differentially
verified. **Neither is reachable yet**: `event_row` and `preset_block` are
complete functions with no call site, because their callers are `shell.rs` and
`tune.rs` and a leaf agent edits neither. The wiring is a `ShellCommand` pair, a
`ShellInput` field and three call sites, all listed in Agent L's report.

**The C's event row is not this rewrite's event row, deliberately.** The oracle
seats `Lyrics` / `Assist` / `Export` in it (`plug.c:2830`); this rewrite already
seats them in the toolbar. Drawing them in both places would put two controls in
charge of one open panel — which is the exact defect the C's own comment at
`plug.c:2884-2887` records having fixed. So the row is the three marker buttons
and `Clear manual`, laid out through the same `TimelineBand` at the same widths.

**The undo buffer is per track, where the C keeps one global pair.** In the
frozen C you can clear track A's lane, select track B, click `Undo clear`, and
`event_timeline_replace(&track->manual_events, &p->event_undo)` moves A's markers
onto B (`plug.c:2944`). Nothing in a `.musi` distinguishes the two designs, so by
`AGENTS.md`'s parity test it is a mechanism choice and the safe mechanism wins.
`Track` therefore carries one `ManualClear`, whose whole state machine lives in
`project::event_timeline` and is tested there against the `.c`.

**The honesty is a caption, not a tooltip.** A manual marker carries one value,
so `semantic_lane::sample` — which requires the four-value analysis payload —
skips it and only Constellation's generic event path reads it. The C says so on
hover; this rewrite has no tooltip widget, and a claim only a hover can reveal is
a claim no capture can review, so the row prints it under the buttons together
with the live marker count. That count is what a headless check should assert on:
it separates "the row drew" from "the row recorded".

**`--event` needed a pending lane, not a track.** The command-line actions run
before an input is resolved (`main.rs` step 3), so every `--event` lands with no
track open. `Workspace` now carries `p->event_timeline`/`p->next_event_id`'s
equivalent and hands it to the first track that becomes current, emptying it —
`plug.c:844-851` exactly, including that a *second* track inherits nothing.

##### `tools/differential_preset_store.sh` is new, and it found something

Four contracts against `preset_store.c`: the derived scene tokens, the
`default_path` environment precedence, the store document's bytes, and `merge`'s
(imported, skipped) counts. It passes — 11 scene tokens, 44 presets, 3660 bytes
of store JSON — but only after the comparison was told to normalize JSON number
*spelling*:

**C writes `.musi` and store numbers with `%.17g` (`project_io.c:48`); this
rewrite writes Rust's shortest round-tripping form.** `0.40000000596046448`
against `0.4000000059604645`. `project::io::write_f64`'s doc comment records that
as a deliberate divergence — "byte identical JSON is an explicit non-goal" — and
this harness is the first thing to actually exercise it. The values are
identical, every field name, field order and id matches byte for byte, and the C
reads either spelling back to the same float. **It is still worth a decision at
the parity gate**, because it is the one place a file written here is not the
file the oracle would have written, and the C's own preset comment leans on
old files round-tripping "byte-stable" (`plug.c:4240-4243`).

Two smaller notes from building it: the harness's `%.9g` emulation is worth
keeping right (nine significant digits is exactly `f32` round-trip, so equal
prints mean equal values), and the fixture generator is duplicated across the two
sides on purpose — a shared one hides the difference being looked for.

##### What is still missing, and who has to do it

- **`crates/musializer-runtime/src/preset_files.rs` exists but is not
  registered.** `lib.rs` is off-limits to a leaf agent. It needs one line:
  `pub mod preset_files;`. Verified green with the line applied, then reverted;
  until it is registered its six tests do not run and clippy does not see it.
- **The shared library is not loaded or saved at startup yet.** `preset_files`
  is the filesystem half and `preset_store::merge` is `shared_presets_adopt`
  (`plug.c:4244-4265`), but `App` has no `shared_presets` field and nothing calls
  either. The preset block therefore has no data source until `main.rs` grows
  one.
- **The row's `+ Scene` path is ported but unexercised**: `Track::record_scene_cue`
  is `plug.c:1979-2030` with tests, and nothing calls it until the row is wired.
- **No capture yet.** Neither surface is in `tools/headless_check.sh`, because
  neither is reachable. Add both when the call sites land — the marker count in
  the caption is the assertable line.
