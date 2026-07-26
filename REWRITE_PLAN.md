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
