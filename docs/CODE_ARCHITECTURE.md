# Code architecture

This guide answers two maintenance questions: where does a responsibility live,
and what path does data take before it reaches a visible frame or a saved project?
It describes the current Rust implementation, not the historical order in which
it was ported.

## Dependency direction

```text
                         external executables
                         FFmpeg, Python, models
                                  ^
                                  |
musializer-app ------> musializer-runtime ------> raylib 5.5
       |                       |
       +-----------------------+----> musializer-core

tools/*.py ---- analysis.bridge.tsv ----> app/runtime boundary ----> core model
```

`musializer-core` is the bottom of the application dependency graph. It owns
deterministic data, validation, policy, analysis, and layout. It has no raylib
handles, OS process handles, global mutable state, or filesystem side effects.
Both `musializer-runtime` and `musializer-app` depend on it; core does not depend
on either one.

## Crates and ownership

| Area | Owns | Does not own |
| --- | --- | --- |
| [`musializer-core`](../crates/musializer-core/src/lib.rs) | Audio analysis, project model and schemas, scene contracts and deterministic state, timing, routes, UI policy and layout | Windows, GPU resources, files, child processes |
| [`musializer-runtime`](../crates/musializer-runtime/src/lib.rs) | raylib adapters, decoded media, the audio callback bridge, fonts, project filesystem edges, dialogs, FFmpeg and helper supervision | Editor policy or top-level application state |
| [`musializer-app`](../crates/musializer-app/src/main.rs) | Composition root, active playback, workspace, command dispatch, scene drawing, panels, preview and export orchestration | Reusable pure policy that can live in core |
| [`raylib-5-5-link`](../crates/raylib-5-5-link/build.rs) | Building and linking the vendored raylib 5.5 source | Application behavior |
| [`tools`](../tools) | Independent measured/model evidence, adapters, verification drivers, support checks | Direct mutation of a live project |

The important distinction is not “backend versus frontend.” It is deterministic
state versus effects. Geometry belongs in core even though pixels are drawn in
the app. Process lifecycle belongs in runtime even though the app decides when a
button requests it.

## State ownership

[`main.rs`](../crates/musializer-app/src/main.rs) is the composition root. Its
frame loop owns resources whose lifetimes must be explicit: the raylib window and
audio device, active `Music`, analyzer, fonts, scene renderer, export session,
and Assist process controller.

[`Workspace`](../crates/musializer-app/src/workspace.rs) owns user-visible editor
state: tracks, current selection, lyrics, semantic and manual event lanes, scene
plans, routes, output settings, dirty state, and the process-free half of the
Assist session. A [`Track`](../crates/musializer-app/src/workspace.rs) is the
application-side aggregate assembled from validated core types.

[`Shell`](../crates/musializer-app/src/ui/shell.rs) owns interaction state. It
draws against a snapshot and emits `ShellCommand`s. Commands are handled after
the drawing pair closes, so a modal dialog, filesystem operation, or process
start never occurs while a raylib draw handle is live.

Long-running controllers deliberately stay outside core:

- [`AssistController`](../crates/musializer-app/src/ui/panels/assist.rs) owns the
  helper process and its artifacts; `Workspace::assist` owns drawable state.
- [`ExportSession`](../crates/musializer-app/src/ui/panels/export.rs) coordinates
  deterministic frames and the FFmpeg writer.
- Runtime process modules own child processes and guarantee termination/reaping.

## The preview frame

The central path is shared by interactive preview and deterministic export:

```text
decoded audio callback
        |
        v
audio_bridge ring --> AudioAnalyzer --> SceneAudioFrame
                                         |
project lyrics + semantic + manual ------+--> SceneFrame
events, scene plan, settings, routes      |        |
                                                  v
                                  scene_host + scenes/*
                                                  |
                                                  v
                                        preview or export image
```

1. [`runtime::audio_bridge`](../crates/musializer-runtime/src/audio_bridge.rs)
   copies real decoded samples from raylib's callback into a lock-free ring. Test
   muting changes only output volume; it must not change this PCM.
2. The frame loop drains interleaved samples into
   [`AudioAnalyzer`](../crates/musializer-core/src/audio/analyzer.rs) and advances
   analysis using the frame delta.
3. [`project_frame_lanes`](../crates/musializer-app/src/main.rs) calls
   [`ProjectFrameLanes::build`](../crates/musializer-core/src/project/frame_lanes.rs)
   to sample the active lyric and semantic cue and merge semantic/manual events.
4. Scene-plan selection, its settings snapshot, and routes are resolved before
   drawing. [`SceneFrame`](../crates/musializer-core/src/scene/mod.rs) is the
   complete per-frame contract passed to a scene.
5. [`scene_host`](../crates/musializer-app/src/scene_host.rs) owns the scene
   registry and dispatch. Pure scene state and formulas live under
   [`core::scenes`](../crates/musializer-core/src/scenes); raylib drawing lives
   under [`app::scenes`](../crates/musializer-app/src/scenes).

Preview and export must not grow separate interpretations of project time. The
shared `ProjectFrameLanes` boundary and
[`timing::render_export`](../crates/musializer-core/src/timing/render_export.rs)
exist to keep the frame contract and timestamp arithmetic common.

## Project persistence

The project format is parsed and serialized by
[`core::project::io`](../crates/musializer-core/src/project/io.rs). That module
owns schema behavior and round-trip semantics but performs no filesystem work.
[`runtime::project_files`](../crates/musializer-runtime/src/project_files.rs)
resolves project-relative assets, hashes files, and performs the filesystem
boundary. [`app::project`](../crates/musializer-app/src/project.rs) converts
between the persisted model and workspace tracks.

This separation is part of the parity argument: the format can be compared
value-by-value with the frozen C without opening a window or relying on a local
directory layout.

## External analysis

The renderer does not import model frameworks. [`external_analysis.py`](../tools/external_analysis.py)
coordinates measured analysis, Whisper, lyric alignment, optional hosted model
requests, scene planning, caching, and the final bridge artifact. The only
application boundary is the bounded bridge schema in
[`core::project::analysis_bridge`](../crates/musializer-core/src/project/analysis_bridge.rs).

A successful helper exit is insufficient. The app checks schema, bounds,
ordering, audio identity, duration, and lane authority, then creates an inert
[`AnalysisCandidate`](../crates/musializer-core/src/project/analysis_candidate.rs).
The interactive workflow changes a project only after explicit Apply. See
[`ASSIST_PIPELINE.md`](ASSIST_PIPELINE.md) for the complete path.

## Export and publication

[`ExportSession`](../crates/musializer-app/src/ui/panels/export.rs) advances the
same analyzer/scene pipeline on deterministic timestamps. Runtime code splits the
effectful work into three concerns:

- [`render_job`](../crates/musializer-runtime/src/process/render_job.rs) owns job
  state and cancellation.
- [`ffmpeg`](../crates/musializer-runtime/src/process/ffmpeg.rs) writes encoded
  video to an external FFmpeg process.
- [`publish`](../crates/musializer-runtime/src/process/publish.rs) stages output
  beside its destination and publishes it without assuming a cross-filesystem
  rename will work.

## Where to make a change

| Change | Begin at |
| --- | --- |
| Audio-analysis math or beat behavior | `musializer-core/src/audio`, then add or extend a differential harness |
| Project schema or `.musi` behavior | `musializer-core/src/project/io.rs` and `runtime/src/project_files.rs` |
| A scene's deterministic state/formula | `musializer-core/src/scenes/<scene>.rs` |
| A scene's raylib drawing | `musializer-app/src/scenes/<scene>.rs` and `scene_host.rs` |
| Panel policy or geometry | `musializer-core/src/ui`, then the corresponding `app/src/ui/panels` renderer |
| Top-level interaction or resource lifetime | `musializer-app/src/main.rs`, `workspace.rs`, or `ui/shell.rs` |
| Child process, dialog, or filesystem effect | `musializer-runtime/src/process` or another runtime adapter |
| Assist evidence or model integration | `tools/external_analysis.py` and the focused helper; keep the bridge boundary stable |
| Lyrics alignment policy | `tools/force_align_lyrics.py`, regression tests, and the investigation record |

## Correctness layers

No one check proves the whole application. The repository uses overlapping
layers:

1. Core unit tests pin local invariants and rejection paths.
2. Differential harnesses compare pure Rust behavior with the frozen C over
   large generated grids. Every new harness needs a perturb-and-revert negative
   control.
3. Python regression tests pin external-analysis parsing and timing policy.
4. `tools/support_bundle_check.sh` verifies that the distributable support tools
   work together without requiring a network request.
5. `tools/headless_check.sh` runs the real application under private Xvfb with an
   unreachable Pulse server and `--mute`, then inspects captures and report lines.
6. `tools/verify.sh` composes the repository-wide gate.

The [`AGENTS`](../AGENTS.md) guide is authoritative for silence, oracle, unsafe,
and negative-control requirements.
