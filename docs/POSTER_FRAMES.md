# Poster frames (WX-5b)

One small rendered frame per **run** of the scene plan (consecutive sections
of one scene), drawn in the plan lane so a run reads as the picture it will
be, of *this* track at *that* moment. The lane is the reader; this document is
about the service behind it. The contract between the two is
`crates/musializer-app/src/posters.rs` (the seam) and the WX-5b design note.

## Where the code is

| What | Where |
| --- | --- |
| Which moment a run shows; the recipe and its key | `musializer-core/src/render/poster.rs` |
| The board: `view`, `enqueue`, `step`, the cache, `posters:` | `musializer-app/src/posters.rs` |
| A recipe built from a `Track` and a `RunSpan` | `posters/recipe.rs` |
| One poster's replay | `posters/replay.rs` |
| Channel 0 of the load-time decode | `posters/audio.rs`, `Track::poster_audio` |
| The gate | `tools/poster_check.sh` (a `tools/verify.sh` step) |

## How a poster is made

- **The moment** (`poster_moment`) is the run's midpoint, or the middle of
  the displayable lyric line nearest it, pushed out of the section's arrival
  transition, its glide, a dip reaching back from the next section and the
  outro, then snapped to a frame. It never reads the playhead, so nothing
  chases playback.
- **The replay** is the export's per-frame sequence (`with_export_frame`,
  `draw_offline_frame`) over the service's own analyzer, beat tracker,
  `SceneInstance` and a **second `SceneRenderer`**. It starts at the moment
  minus the analyzer pre-roll A (2.0 s) and the scene's run-up W
  (`feedback_warmup_seconds`, 3 s for Clawd, else 0); only the run-up and the
  target frame are drawn. A poster is therefore close to the export frame and
  not bit-identical with it. It is never written as a still.
- **The tuning** is the moment's section's snapshot over the track table, and
  for a threshold the doorway scene's nearest section, as
  `App::frame_settings` does it; routes are applied per frame. The run is one
  scene, so there is no automatic scene switch.
- **The draw** goes into a work `Canvas` (`runtime::offscreen`), then is
  copied with a positive source height into the poster's own `Canvas`, which
  therefore stores the picture **top row first**: the lane draws it as an
  ordinary texture. No bare `begin_texture_mode` anywhere, so `gl framebuffer:`
  stays at `mismatched-frames=0`. The copy then gets mipmaps and a trilinear
  filter, because the lane draws a 256-pixel poster about 80 logical pixels
  wide and a bilinear threefold minification breaks thin lines and small type.

## Identity and invalidation

`PosterRecipe` holds every input that changes the pixels: audio digest, scene,
seed, frame, fps, pre-roll, the section snapshot (and the doorway's), the
scene's routes, grade, pixel look, finish, caption style when a caption is
drawn, the displayable lyric cues inside the replay window and the voice
table, the two event lanes' revisions, the ASCII digest, whether Song Atlas's
terrain is built, export and poster size, and `POSTER_RENDERER_VERSION`.
`PosterKey` is FNV-1a over the recipe's `Debug` text, so a field added to the
recipe is in the key without anyone remembering to hash it.

The lane calls `view` per run every frame and the key is recomputed each time.
An edit moves the key; the run's old picture is offered as `Stale` (the lane
draws it at 45 %) until the new one replaces it. Undo, a followed treatment
and a tuning drag all invalidate the same way, with no edit events. A cached
poster is identified by `(track path, run's first cue id)`; a split or merge
that moves a run's first section makes it a new run.

## Scheduling and budget

- `step` runs last in the workspace drawing pair, after egui. It advances whole
  replay frames until the budget is spent: **3 ms while playing, 8 ms
  paused**, at least one frame per app frame, unbounded under
  `--ui-probe posters=settle`.
- Requests are served visible runs first, then by distance from the playhead.
- One job at a time. A job whose key moves mid-replay is dropped
  (`abandoned=`).
- Each step resolves every requested run's recipe **once** (the survey) and
  uses it to pick work, to check the running job and to count. A resolve is
  10–15 µs, almost all of it the `Debug` text the key hashes; resolving three
  times per run cost 3.6 ms a frame on Door, No Room's 27 runs, the survey
  0.4 ms (release, `idle-worst=`). The lane's own `view` is the other resolve
  per run per frame.
- **Held while playing:** Fluid Ink (below), and Cadence until the shared SDF
  atlas exists. Its first build is ~300 ms, once per session (again only when
  new letters appear). Paused, a Cadence poster pays it; playing, it waits,
  and the preview pays the same build the first time it shows Cadence. With
  that rule, playing Door, No Room stalled 13 and 13 frames of 600 with
  posters on, 13 and 14 with posters off: no measurable difference.
- **96 posters** stay on the GPU (LRU across tracks, ~0.3 MB each with the
  depth buffer). No disk cache.
- The second renderer loads before the frame loop (~45 ms, +15 MB resident),
  so its shader compiles never land in a counted frame. Not loaded for batch
  outputs or with `posters=off`.

### Per-scene caps (`recipe::scene_cap`)

Measured on the GPU path, release build, 256x144 (the full table is in the
WX-5b note, §4a). Most scenes cost 12–17 ms a poster; size is not the lever,
because the cost is analysis and scene state on the CPU.

| Scene | Default cost | Cap | Capped cost |
| --- | --- | --- | --- |
| Clawd | 184 ms (90 drawn run-up frames) | W = 1.0 s | 66–115 ms |
| Fluid Ink | 600 ms (~10 ms CPU simulation per frame) | A = 1.0 s, paused only | 300–340 ms |

Fluid Ink is over the 250 ms worst on purpose. A = 0.5 s showed young blobs
instead of the ink, and its cost is sliced one frame per app frame and never
spent while the song plays, so it arrives later and stalls nothing.


## The `posters:` line

```text
posters: requested=4 ready=4 stale=0 pending=0 unavailable=0 lru=4/96 size=256x144
  preroll=2.0s step-worst=11.4ms settle-worst=0.0ms frame-worst=4.2ms idle-worst=0.09ms budget=3ms/8ms
  rendered=4 refreshed=0 abandoned=0 poster-median=34.3ms poster-worst=52.1ms
  isolated=renderer#2 load=45ms/+14.6MB version=1 probe=none dump=none
```

`step-worst` is the longest budgeted step; `settle-worst` is the longest
settle step, which is unbounded on purpose. `frame-worst` is the longest
single replay frame, the most a step may overrun its budget by. `idle-worst`
is the longest step with no poster work: the recipe and key per requested run
that every frame pays once the lane is full. `refreshed`
counts posters that replaced an older picture: the path an edit takes.
`isolated=` reads `renderer#2`, `idle` (nothing loaded yet) or `FAILED`.

## Probes and knobs

- `--ui-probe posters=settle` finishes every requested poster in the next
  frame's step. `posters=off` switches the service off; every run reads
  `Unavailable`.
- `--ui-probe poster-dump=DIR` writes each finished poster as
  `DIR/<scene>-<run id>-<key>.png`, read back from the poster texture.
- `MUSIALIZER_POSTER_PREROLL` (seconds), `MUSIALIZER_POSTER_BOX` (`WxH`) and
  `MUSIALIZER_POSTER_BENCH=N` (render every poster N times and print a
  `poster bench:` line each, plus the decode time) are the measurement knobs.
  The per-scene caps override the pre-roll.

## The gate

`tools/poster_check.sh` runs on a 20 s synthetic track with four runs: Clawd,
Threshold, Stage, Spectrum. Each check has a negative control that was run
and seen to fail:

| Check | Assertion | Perturbation that failed it |
| --- | --- | --- |
| Determinism | two dumps of the same recipes are byte-equal | the scene seeded from the clock: 3 posters differ |
| Isolation | the frame the posters are drawn in equals posters off, and so does the preview crop 37 frames later, inside the Clawd and the Threshold runs | the finished poster also drawn to the screen: both moments fail |
| Invalidation | a followed-treatment edit to one section re-renders exactly that run (`refreshed=1`, one new dump) | the section tuning left out of the key: `refreshed=0` |
| Budget | `step-worst` ≤ the paused budget + `frame-worst`; no more stalled frames than posters off | the budget multiplied by 1000: 137 ms step, 1 stall |
| GPU state | `gl framebuffer: mismatched-frames=0` | a bare `BeginTextureMode`/`EndTextureMode` pair: 29 of 30 frames mismatched |

Two things about the isolation check cost a run to learn:

- `--probe-shot` reads the buffer after the swap, so it shows the frame
  **before** the last. The posters are drawn in frame 2 (the lane's requests
  reach the service after frame 1), so that frame is photographed by a
  3-frame run. A 2-frame shot passed with the leak in place.
- A whole-frame comparison at a late frame is the lane's business once the
  lane draws posters, so the late comparison is cropped to the preview.

The gate passes on both renderers (VirtualGL and `MZ_GL_LAUNCH=""`). Its
comparisons are exact equalities of whole images, which both rasterizers
reproduce run to run.

## Scene picker (WX-5 follow-up, 2026-10-30)

The Scenes flyout uses the same board, replay, renderer, LRU and budget. A
`PosterRequest::scene` covers the track with a forced scene and no plan-section
snapshot; the moment is the nearest lyric midpoint to the track midpoint, or
the track midpoint. It stays still during playback so work can finish. Scene
settings, routes and all other recipe inputs are the track's own.

Cache identity is `(track path, Picker(SceneId))`, separate from
`(track path, Run(cue_id))` even for equal numerical ids. Only visible tiles
request work. The picker shows the picture upright, fitted above the name;
pending, stale and unavailable have visible labels and tooltip explanations.
`scene picker: … posters=scene:state@x,y,w,h;…` reports each visible tile's
actual draw state. Song Atlas without its terrain reports `ready-idle` rather
than claiming the terrain was rendered. Dumps use
`<scene>-picker-<scene>-<key>.png`. `tools/wx5_features_check.sh` compares every
tile with its own dump and every other visible scene as controls.
