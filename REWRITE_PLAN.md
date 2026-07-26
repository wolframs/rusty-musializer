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