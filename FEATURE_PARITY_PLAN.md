# Rusty Musializer Feature-Parity Plan

This is the **single authoritative task list** for reaching feature parity with
the frozen C Musializer. If another document, source comment, old agent handoff or
ignored worktree names unfinished work, reconcile it here before acting on it.

`REWRITE_PLAN.md` is the historical design and session record. Its NOTE ENTRIES
remain valuable evidence, but its phase plans and agent handoffs are no longer a
live queue. `docs/PHASE0_INVENTORY.md` is a behavioral inventory of the C oracle,
not a task list. `AGENTS.md` contains repository rules and deliberate exclusions.

## Target and meaning of parity

- **Oracle:** `../musializer`, branch `master`, frozen commit
  `9300af942bd00d8c85fc4e3c8c02cf2b6356764f` (`9300af9`). It is read-only.
- **Audited Rust baseline:** `e32705c`, 2026-07-31.
- **Baseline evidence:** `tools/verify.sh` is 19 passed / 0 failed: thirteen
  differential harnesses plus the headless capture gate.
- **Feature parity means observable capability parity.** A C workflow must be
  available in Rust and its persistent or rendered result must retain the same
  meaning. Mechanisms, ownership, UI composition and implementation structure may
  improve when the result is equivalent or better.
- **Code-detail equality is not a goal.** Do not reproduce global state, hot
  reload architecture, tinyfiledialogs, unsafe pointer shapes or C source layout
  merely because the oracle uses them.
- **A difference is a parity gap** when it changes a `.musi` file, rendered video,
  analysis number, documented command, user workflow or durable edit.

### Deliberate exclusions

These do not block parity unless the operator changes the target:

- Microphone capture (`MUSIALIZER_MICROPHONE`)
- Hot reload, including functional `--reload-once`
- Non-Linux platforms, including Windows, macOS and OpenBSD; this is a
  Linux-first rewrite

Exact pixel identity, C source organization and reproducing known C defects are
also not goals. Any new deliberate product-level exclusion must be approved and
added both here and to `AGENTS.md`.

## Current fit

The following are already delivered and must stay green:

- All ten scene drawing implementations and deterministic scene state
- Audio playback, analyzer and beat tracker
- Per-scene settings, routes, route persistence and preset model/UI
- Multi-track workspace and `.musi` open/save with asset verification
- Bidirectional C/Rust `.musi` differential round trips and frame-lane boundary
  selection: 2,550 values, delta 0
- FFmpeg export, deterministic full/windowed rendering and publication
- Lyrics editor, caption-style editor and local/imported font runtime support
- Assist UI/state/controller and analysis-bridge import
- Manual event capture, clear/undo and scene-cue model
- Whole-track waveform, Song Atlas map and image-backed ASCII derivations
- All bottom-panel layouts and the headless scene/panel capture sweep

The remaining work is concentrated at integration boundaries and missing product
entry points. A green pure-module harness does not prove those boundaries.

## Ordered completion map

Work in order. P0 establishes the shared render path on which later assertions
depend. P1 makes authored project behavior real. P2 closes missing C workflows.
P3 packages the external product. P4 removes false status and proves the result.

```text
P0 project-aware frame path
  A1 frame lanes -> A2 lyric overlay -> A3 integration evidence
                  -> B1 automatic switching -> B2 cue semantics -> B3 evidence

P1 durable editing
  C1 dirty marking -> C2 draft/context guards -> C3 reset workflow
                   -> C4 all-track autosave

P2 missing entry points
  D1 drop dispatch -> D2 ASCII image UI
  D3 lyrics TSV import/export
  D4 timeline information and pan

P3 complete external bundle
  E1 support-file manifest -> E2 Assist -> E3 Google Fonts -> E4 dist/doctor

P4 honesty and gate
  F1 stale UI/status removal -> F2 stale handoff cleanup
  G1 integration gates -> G2 final feature audit
```

## P0 — make saved project content reach the renderer

### A1 — one project-aware `SceneFrame` path

- [x] Replace the preview and export uses of `..SceneFrame::idle(effective)` with
      one shared frame-building path.
- [x] At `time_seconds`, sample the current track's semantic lane with
      `project::semantic_lane::sample`.
- [x] Select the active lyric with `LyricsDocument::at_time`.
- [x] Merge manual and semantic events through `SceneEventMerge`; preserve the
      C ordering and semantic-id qualification already pinned by the differential
      harness.
- [x] Feed the same semantic, lyric and merged-event view to preview and export.
- [x] Preserve the no-track behavior: default semantics, no lyric, empty events.
- [x] Define failure behavior visibly. An invalid merge must be reported and use
      an empty frame view; it must not retain a prior track's events.

Observable closure:

- Semantic valence/tension/confidence modulates every scene that reads it.
- Cadence receives active lyrics instead of always drawing its ambient fallback.
- Loom receives its semantic lane instead of always relying on fallback data.
- Constellation receives both manual and model-derived event markers.
- Preview and export see identical project data at the same scene time.

### A2 — shared lyric captions

- [x] Port the application-side `draw_scene_lyric_overlay` composition using the
      existing `caption_layout` and `runtime::font` machinery.
- [x] Draw the shared overlay after every scene except Cadence, which owns its
      lyric composition just as the C app does.
- [x] Honor the project's caption anchor, width, size, margin, text/box colors,
      box mode and imported caption face in both preview and export.
- [x] Keep the overlay resolution-independent through the export pixel scale.
- [x] Add evidence for long UTF-8 text, the three-line/ellipsis contract, all box
      modes, a non-default anchor and an imported face.

### A3 — frame-boundary evidence

- [x] Generate a synthetic `.musi` containing lyrics, semantic events and manual
      events; do not commit user media.
- [x] Add a headless report line naming the active lyric id, semantic availability
      and merged event count.
- [x] Capture at least one shared-caption scene, Cadence, Loom and Constellation.
- [x] Export selected frames and assert that seeded project lanes change the
      output. Include a negative control that drops one lane and demonstrably
      changes the evidence.
- [x] Confirm the same fixture written by C and Rust produces equivalent lane
      selection at boundary timestamps.

Completion evidence (2026-08-01, `Complete project-aware scene rendering`, this
commit): `ProjectFrameLanes` is the shared preview/export constructor and owns a
fresh merge so an invalid lane logs its error and exposes zero events. The pure
suite pins no-track state, exact lyric/semantic boundaries, canonical ids/order,
UTF-8 three-line ellipsis, all box modes, a non-default anchor and 2x export
scaling. `tools/headless_check.sh` generates the `.musi` and bundled imported face,
captures Spectrum, Cadence, Loom and Constellation, reports
`lyric=1 semantic=available source=11 merged-events=4` in both preview and export
at `t=1.000`, and repeats both full outputs deterministically. Removing lyrics,
semantics or manual events changes both preview and export hashes and changes only
the named report fields. The expanded bidirectional project harness compares 510
values per path / 2,550 total at delta 0; its 1 ms-early lyric negative control
produced four boundary failures in each direct comparison (12 discrepancies).

## P0 — automatic scene plans and cue semantics

### B1 — drive automatic switching in preview and export

- [ ] Apply `SceneSwitchTimeline::update` before frame construction on both paths.
- [ ] Make `--auto-scenes` actually enable the imported/saved plan; reject an
      empty plan with the oracle's observable behavior.
- [ ] Replay accepted Assist section plans and enabled `.musi` scene plans.
- [ ] Apply a cue's captured settings snapshot before audio routes, matching the
      C precedence: base/cue settings first, per-frame routing second.
- [ ] Reset the switch cursor and cue-settings state on seek, track change,
      project open, export start/end and plan enable/disable.
- [ ] Keep preview and export scene selection identical for the same time.

### B2 — base-scene selection and manual `+ Scene` capture

- [ ] When a user selects a different base scene, set the pending-selection state,
      remember the previous base scene and mark the project dirty.
- [ ] Selecting a base scene while Auto scenes is enabled must disable playback of
      the plan, retain its cues and explain that result in the notice tray.
- [ ] The first manual scene cue recorded after time zero must backfill the former
      base scene from zero before inserting the new cue.
- [ ] Recording a cue must capture the selected scene's effective tuning snapshot,
      clear the pending flag and apply the cue immediately at the playhead.
- [ ] Tune edits made while a cue snapshot is active must update that cue rather
      than silently changing only the base scene settings.

### B3 — scene-plan evidence

- [ ] Add preview and export tests at every cue start/end boundary, including seek
      backward, fast-forward and a windowed export beginning after cue zero.
- [ ] Assert selected scene id and a cue-specific setting, not merely the enabled
      flag or the words "automatic scene plan".
- [ ] Prove a manual first cue at `t > 0` retains the prior base scene on
      `[0, t)`.
- [ ] Add a negative control that disables the call to `update` and is caught.

## P1 — durable edits, guards and autosave

### C1 — complete dirty marking

- [ ] Audit every command that mutates data serialized into `.musi` and route it
      through one dirty-marking policy.
- [ ] At minimum, mark dirty after setting changes, scene reset, base-scene
      selection, cue-snapshot tuning, caption-style edits, lyric import, ASCII
      clear/import and scene-plan enable/disable.
- [ ] Keep purely transient playback, selection, hover and panel state clean.
- [ ] Test that each durable mutation starts/restarts the 1.5-second autosave
      settle and participates in the quit guard.

### C2 — lyric and route draft context guards

- [ ] Expose one truthful `lyric_editor_has_unsaved_draft` query.
- [ ] Block or resolve dirty lyric/route drafts before track change, scene change,
      panel change, project open, save, export start, Assist apply and quit where
      the C workflow protects them.
- [ ] Include the active lyric draft in `confirm_close` and in autosave's
      `editor_dirty` argument.
- [ ] Never let Assist replace lyrics while a conflicting lyric draft is open.
- [ ] Give every refusal an actionable notice; do not silently discard or switch.
- [ ] Cover Apply, Discard and Cancel paths, not just the blocked path.

### C3 — reversible scene reset

- [ ] Replace immediate "Reset scene" with the C workflow: Reset, Confirm, then
      Undo reset.
- [ ] Preserve the pre-reset settings until another reset/context invalidates the
      undo snapshot.
- [ ] Mark reset and undo as durable edits and commit to an active cue snapshot
      when applicable.
- [ ] Capture all three button states.

### C4 — autosave every track

- [ ] Cache decoded sample rate and channel count on each `Track` so project
      serialization does not depend on the currently bound music stream.
- [ ] Autosave every due track, not only the current one.
- [ ] Refuse autosave while that track owns a dirty editor draft.
- [ ] Test two dirty tracks, a background track loaded from a project, failure of
      one save without suppressing another, and recovery after a failed save.

## P2 — missing C workflows and timeline affordances

### D1 — typed file-drop dispatch

- [ ] Dispatch dropped `.musi` files through project open.
- [ ] Dispatch PNG/JPEG/BMP through ASCII import and select ASCII Field.
- [ ] Continue dispatching supported audio formats through track load.
- [ ] Preserve the C behavior that an image dropped before audio is staged for the
      next track.
- [ ] Report unsupported/corrupt input by its attempted type.
- [ ] Add non-interactive probes or direct command tests for all three branches.

### D2 — ASCII image import and clear UI

- [ ] Add "Import image -> ASCII" to the scene browser with PNG/JPEG/BMP filters.
- [ ] Show "Clear image" only when the active track owns an image-backed grid.
- [ ] Import transactionally, select ASCII Field on success and mark the project
      dirty; a failed decode must preserve the previous grid.
- [ ] Clear path, digest, cells and dimensions together and mark dirty.
- [ ] Capture empty, populated, cleared and staged-with-no-track states.

### D3 — timed-lyrics TSV import/export

- [ ] Replace the disabled Lyrics Export/Import buttons with native save/open
      dialog commands.
- [ ] Use `LyricsDocument::bridge_export` and `bridge_import`; do not invent a
      second codec.
- [ ] Import transactionally, validate against track duration, mark dirty and
      preserve the old document on failure.
- [ ] Export must not dirty the project.
- [ ] Test cancel, overwrite confirmation/backend failure, invalid TSV, valid
      round trip and UTF-8 text.

### D4 — timeline content and navigation

- [ ] Draw the merged manual/semantic event markers over the waveform lane.
- [ ] Draw the lyric cue lane even when the Lyrics editor is closed.
- [ ] Add Shift-wheel pan and middle-drag pan with robust pointer-claim release.
- [ ] Keep wheel zoom anchored at the pointer and every lane aligned through the
      shared `TimelineView` conversion.
- [ ] Give tick labels an opaque backing so waveform amplitude cannot erase their
      contrast.
- [ ] Capture event colors, lyric spans, zoom, pan and an off-screen boundary.

## P3 — package the complete external product

### E1 — define and copy the support bundle

- [ ] Treat the C distribution's support-file list as the starting manifest, not
      `external_analysis.py` alone.
- [ ] Bring over and review the required Python modules, prompt, schemas and docs:
      `analysis_io.py`, `analyze_audio.py`, `external_analysis.py`,
      `google_fonts.py`, `import_whisper.py`, `lyric_align.py`,
      `mimo_openrouter.py`, `musializer_doctor.py`, the lyrics cleanup prompt and
      every schema those tools read or write.
- [ ] Preserve the helpers as independent tools; first-party application code
      remains Rust.
- [ ] Remove C-build assumptions and resolve resources relative to the installed
      bundle without requiring the C checkout.
- [ ] Add one authoritative Rust distribution-support manifest so install and
      archive paths cannot drift.

### E2 — make Assist runnable

- [ ] Verify all local/offline Assist modes without network credentials.
- [ ] Verify explicit Codex/OpenRouter paths only when opted in; never copy
      credentials into fixtures, logs or the repository.
- [ ] Exercise real helper discovery from source, installed launcher and extracted
      distribution layouts.
- [ ] Test cancellation, timeout, process-group cleanup, invalid bridge output,
      staging, Apply and Discard.
- [ ] Keep a fake helper for deterministic lifecycle coverage, but add at least one
      real support-bundle smoke test.

### E3 — make Google Fonts import runnable

- [ ] Package `google_fonts.py`, `analysis_io.py` and its schema dependencies.
- [ ] Keep network consent once-per-run and never persist it.
- [ ] Verify catalogue caching, host allow-list/redirect refusal, digest checks,
      licence publication, import into a project and project reopen.
- [ ] Keep a fully offline fake/helper fixture for the default verification gate.

### E4 — installation, doctor and distribution

- [ ] Provide the C app's equivalent self-contained distribution/archive workflow,
      including executable, launcher, MIME data, resources and support bundle.
- [ ] Make `musializer_doctor.py` or a Rust equivalent report FFmpeg, helper,
      Whisper/model and writable-data-path readiness.
- [ ] Test the per-user launcher from a directory other than the repository root.
- [ ] Test an extracted distribution on a clean path with no Cargo invocation and
      no dependency on `../musializer`.
- [ ] Document optional external dependencies and which features degrade without
      them.

## P4 — honest status, stale handoffs and final gate

### F1 — remove false unfinished status

- [ ] Stop `toggle_panel` from emitting `ShellCommand::NotImplemented` for the real
      Export, Lyrics and Assist panels; delete the obsolete command if no genuine
      use remains.
- [ ] Remove the scene-browser footer `* not ported yet` while every registered
      scene reports a ported drawing path.
- [ ] Replace stale "largest remaining gap" claims in status text with a pointer
      to this plan.
- [ ] Ensure intentionally unavailable actions name their real prerequisite
      (`helper missing`, `FFmpeg missing`, `no track`) rather than saying "stub".

### F2 — retire completed agent seams without changing behavior

This is centralized dangling-work cleanup, not feature parity by itself:

- [ ] Remove or rewrite obsolete "no caller yet", "Agent X must wire this" and
      "nothing dispatches this" comments whose call sites now exist.
- [ ] Remove stale module/function `allow(dead_code)` attributes that only existed
      for the completed fan-out. Keep intentional future-scene placeholder code
      explicitly documented if it remains.
- [ ] Reconcile the old duplicate `transport_duration_text` boundary note with the
      landed core timing module; deduplicate only with numeric evidence.
- [ ] Review the old requests to hoist shared scene draw helpers. Consolidate only
      where it reduces live duplication without perturbing verified formulas.
- [ ] Keep `REWRITE_PLAN.md` as append-only historical evidence; do not rewrite its
      old session statements to pretend they were never true.

### F3 — optional structural cleanup, explicitly not a parity blocker

- [ ] Consider moving `Workspace::assist` to `Shell` and deleting its four `Cell`
      fields. Do this only with the existing Assist differential and capture suite
      green before and after.
- [ ] Treat further ownership, module and math-helper consolidation as normal
      refactoring. It does not belong on the parity critical path unless it fixes
      an observable defect.
- [ ] Fix or formally close the FFmpeg test-helper `ETXTBSY` race. The historical
      report named
      `process::ffmpeg::tests::a_completed_encode_is_published_by_rename`; on
      2026-07-31 `tools/verify.sh --quick` instead failed
      `an_encoder_that_ignores_the_pipe_closing_is_killed_after_the_grace_period`
      at helper spawn with `ExecutableFileBusy`, after which that exact test
      passed in isolation. Treat this as a shared generated-executable race, not
      as one flaky assertion, and retain a regression stress test when fixed.

### G1 — integration gates that cover the discovered holes

- [ ] Add a seeded-project headless scenario that simultaneously proves lyrics,
      semantics, manual events, scene plans and cue settings reach preview.
- [ ] Run the same project through full and windowed export and compare selected
      frame hashes/state reports.
- [ ] Add a drop-dispatch test, lyric TSV test, autosave-multiple-tracks test and
      packaged-helper discovery test.
- [ ] Every new gate gets a negative control; record what the perturbation broke.
- [ ] Choose further differential harnesses by evidence weakness, not by counting
      modules. A property-only unit suite is the strongest candidate signal.

### G2 — final feature-parity audit

- [ ] Walk the C application surface from `docs/PHASE0_INVENTORY.md`: CLI, file
      types, panels, timeline gestures, projects, scenes, captions, Assist, fonts,
      presets, events, export, launcher and distribution.
- [ ] For every C capability, record `fit`, `improved mechanism`, `deliberate
      exclusion` or an open task id in the matrix below. No uncategorized row.
- [ ] Run `tools/verify.sh`; require 0 failures and a clean oracle at `9300af9`.
- [ ] Open a C-written `.musi` in Rust and a Rust-written `.musi` in C, including
      embedded ASCII image/font assets, lyrics, semantic/manual lanes, routes,
      presets and enabled scene cues.
- [ ] Verify an interactive session on the operator's real desktop separately from
      Xvfb: dialogs, drag/drop, fullscreen, audio controls and launcher startup.
- [ ] At completion, the deliberate exclusions above are the complete observable
      difference list. Update `README.md` and `AGENTS.md` from this file.

## Feature-surface ledger

| C feature family | Current Rust state | Closure |
| --- | --- | --- |
| Ten scenes and audio reaction | Fit for audio/whole-track inputs; saved semantic/lyric/event inputs are unwired | A1-A3 |
| Shared captions and Cadence lyrics | Models/layout/runtime exist; render integration missing | A1-A3 |
| Automatic scene plans | Model, persistence and UI labels exist; playback/export driver missing | B1-B3 |
| Manual scene cues | Capture model exists; base-selection/pending semantics incomplete | B2-B3 |
| Settings/routes/presets | Core fit; dirty/cue/reset workflow incomplete | B2, C1, C3 |
| `.musi` open/save | Differential round trip fit | C1-C4 guard/autosave completion |
| Multi-track workspace | Fit interactively | C2, C4 |
| Lyrics editing | Editor fit; document import/export and broad draft guards missing | C2, D3 |
| Manual/semantic events | Models and row fit; scene/timeline consumption missing | A1, D4 |
| Timeline | Waveform/zoom/scrub fit; lyric/events/pan missing | D4 |
| ASCII image mode | CLI/project rendering fit; GUI and typed drop missing | D1-D2 |
| Assist | UI/controller/bridge fit; shipped helper bundle absent | C2, E1-E2 |
| Google Fonts | UI/runtime fit; shipped helper bundle absent | E1, E3 |
| FFmpeg export | Transport/determinism fit; project lanes/scene plans missing | A1-B3 |
| CLI | Mostly fit; `--auto-scenes` ineffective; `--reload-once` excluded | B1-B3 |
| Linux launcher/MIME | Source install fit | E4 |
| Self-contained distribution/doctor | Missing | E4 |
| Microphone/hot reload/non-Linux | Deliberately excluded | none |

## Consolidated dangling-note ledger

This table resolves the old planning fragments so they are not rediscovered as
parallel task lists.

| Inherited note | Disposition |
| --- | --- |
| `REWRITE_PLAN.md` TODO: route editor UI and persistence | Completed and differentially covered; stale historical TODO, not active work |
| Completion-plan W1/W2 and G through O | Landed; retain NOTE ENTRIES as history. Remaining integration gaps have new A-G ids above |
| Agent I: Lyrics panel call sites | Landed; only TSV import/export and broad draft guards remain, D3/C2 |
| Agent J: Assist panel seams | Landed; real helper bundle and lyric-draft conflict remain, E2/C2 |
| Agent K: font-browser seams | Landed; helper packaging remains, E3; stale dead-code comments go to F2 |
| Agent L: event/preset call sites | Landed; frame/timeline consumption remains, A1/D4 |
| W1 note: autosave only current track | Active as C4 |
| `Workspace::assist` ownership deferral | Optional structural cleanup F3, not a parity requirement |
| Item O: add every possible harness | Reframed as risk-based evidence work in G1 |
| Recorded intermittent FFmpeg `ETXTBSY` test-helper race | Reproduced in a second test and retained as test-reliability cleanup in F3 |
| Source comments naming agents/integration-owner handoffs | Audit and remove as F2 after verifying each call site |
| Ignored `.claude/worktrees/**/REWRITE_PLAN.md` files | Historical worktree snapshots, not tracked plans and not sources of current tasks |

## How this file stays authoritative

- Start work by claiming one task id here and recording dependencies.
- When a task lands, check its boxes and append the commit plus evidence under the
  task. Do not add a second completion plan elsewhere.
- New C-only behavior discovered during implementation becomes a task here before
  it becomes code.
- A task may be removed from the critical path only as `completed`, `duplicate`,
  `not observable`, or an operator-approved deliberate exclusion, with the reason
  recorded here.
- Keep historical investigation and negative-control details in
  `REWRITE_PLAN.md` NOTE ENTRIES or focused code/test comments; keep the live queue
  here concise and current.
