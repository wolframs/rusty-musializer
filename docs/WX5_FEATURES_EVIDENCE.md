# Scene posters and inline artist: implementation evidence

2026-10-30, branch `t3/scene-posters-artist-edit`, based on `be507e1`.
Implementation checkpoint `3f6efcc`; gate improvements `277b09c`, `62de227`;
nested-display fix and its controls `ff97064`.
This branch is for lead review, not pushed or merged.

## What changed

The Scenes flyout shows a frame of each named scene using this track's audio,
lyrics, scene tuning, routes and look. Its requests share the plan lane's
poster board, isolated renderer, replay scheduling and 96-entry LRU. A picker
scene and a plan cue have distinct cache identities. The representative moment
is stable during playback: the track midpoint, or the displayable lyric midpoint
nearest it. This avoids invalidating a replay before it finishes.

Tiles are 84–104 logical pixels high, retaining the family column, two tile
columns and scrolling. All current families fit at 960x640. Names remain below
the picture. Pending says Rendering…, stale says Refreshing… over a dimmed
picture, and unavailable says No preview with the actual reason in its tooltip.
Every visible tile reports its draw state and image rectangle. Song Atlas
without built terrain reports `ready-idle` and names its live audio ring.

The top bar's artist slot opens an inline 128-byte Unicode draft. Enter
commits, Escape or clicking away cancels, and empty/whitespace clears. No
artist gets the quiet Add artist invitation. The slot remains reachable when
the bar is narrow. Drafts belong to their track; a track change discards one.
`TextEntrySurface::TopBarArtist` suppresses shell shortcuts, including Ctrl
chords, and the composition root prevents egui from draining its text into
the dock. Commits mark changed metadata dirty through a track-bound command.
There is no persisted schema flag or format change: the existing content-derived
schema makes an artist v2 and a cleared artist byte-identical v1 again.

The only launcher change lets `headless_run.sh` honor the assigned
`MUSIALIZER_CAPTURE_DISPLAY=:82`; without it automatic display selection stays. Nested gates reuse the assigned
server only when process ancestry proves that an ancestor gate owns it; sibling
process trees are refused. Two temporary wrapper faults (accepting every Xvfb,
and refusing even an owned Xvfb) each failed twice, once per renderer setting.
The wrapper bytes were restored exactly. Receipts: `build/wx5-launcher-good.log`,
`build/wx5-launcher-negative-permissive.log` and
`build/wx5-launcher-negative-rejects-owned.log`.
All launches remained muted and isolated. No models or real agents were run.

## Feature gate

`tools/wx5_features_check.sh` uses synthetic audio for assertions and a local
copy of `build/redesign/fixture/Door, No Room.musi` for the visual matrix. It
opens every family through `scene-family=`, checks all 29 scene identities,
compares every ready tile with its own dumped poster and the other visible
scenes, pins the minimum tile geometry, and presses a tile and a gap using
points derived from report rectangles. Pending and unavailable are explicit. The pending capture holds Fluid Ink
while playing for 30 frames, so the previous-buffer GPU screenshot seam cannot
photograph a frame before the state was drawn.

It checks ordinary scheduling while music plays, retaining exactly 3 ms / 8 ms
budgets and the allowed single-replay-frame overrun. On the positive GPU run,
step-worst was 15.7 ms and frame-worst 13.3 ms; on llvmpipe, 29.0 ms and 26.9 ms.
These are measurements, not maximum promises. The restored final scheduling
checks also passed with 0 stalled frames in both 180-frame playing runs. All
six Pixel Cats arrived.

Artist checks press the real slot and a gap, supply a draft through the named
input seam, and share the real Enter/Escape outcome handler. They check cancel,
commit, released focus, v2 autosave, and byte-equal v1 after clearing through
a second UI edit and autosave. Unit checks separately pin shortcut suppression,
commit/cancel commands, validation, source scene identity and exact v1 bytes.
The probe supplies draft input; it does not synthesize OS keyboard events.

Positive full feature gates passed on NVIDIA RTX 3090 / VirtualGL and on
llvmpipe (`MZ_GL_LAUNCH=""`). Logs and captures:
`build/wx5-features-gpu/`, `build/wx5-features-cpu/`.

## Negative controls

Temporary faults were built into separate local binaries. The complete feature
gate ran against each binary on both renderers. Every perturbed source was
restored and its SHA256 compared with its pre-fault bytes. A clean
`git status --porcelain` receipt is `build/wx5-restoration-receipt.txt`.
Restored `cargo test` passed again.

| Fault batch | What the checks caught | Receipts |
| --- | --- | --- |
| Invisible poster drawing; every tile falsely reports ready; 105 px ceiling; tile hit boxes extend into the gap and react to hover; artist slot takes its gap; artist ownership disabled; commit/cancel reversed; clearing retains `stuck` | 38 feature failures on GPU and 38 on CPU: poster-to-dump mismatches, false fallback states, changed geometry, gap selection, missing draft/focus and missing commit/cancel outcomes. The exact-v1 and text-focus unit checks also failed | `build/wx5-negative-gpu.log`, `build/wx5-negative-cpu.log`, `build/wx5-negative-schema.log`, `build/wx5-negative-focus.log` |
| Every recipe renders Spectrum; playing budget changed from 3 to 3000 ms; empty artist retains `stuck` with normal focus and commit | 32 feature failures on GPU and 32 on CPU, including 28 missing scene-specific dumps, both budget assertions and failed UI clear/v1 restoration. The forced-scene unit check failed on the first non-Spectrum request | `build/wx5-negative2-gpu.log`, `build/wx5-negative2-cpu.log`, `build/wx5-negative-forced-scene.log` |

The first batch exercised the image classifier on both rasterizers: a plausible
report without its picture did not pass. Repeating the image/state/geometry fault batch against the final held-pending
fixture produced 27 failures on each renderer (`build/wx5-negative-held-gpu.log`
and `build/wx5-negative-held-cpu.log`), including the held-state assertion.
The second exercised scene provenance
and actual UI clear persistence while its normal commit and focus still worked.

## Visual review

Both features were captured in dark and light at 960x640, 1280x720 and
1920x1080 on both renderers. Every PNG has its requested dimensions. All 24 final captures were inspected in four current GPU/CPU review montages;
the minimum-size scenes, artist, pending and unavailable surfaces were also
inspected individually. The six cat scenes remain recognizable and their
names readable; the inline artist fits without overlapping tabs or actions at
960x640. The project was copied before opening it; no file in `~/Music` was
opened. Artist drafts in the visual captures were never committed to that copy.

For each directory `build/wx5-features-gpu` and `build/wx5-features-cpu`:

- `dark-960x640-scenes.png`, `dark-960x640-artist.png`
- `dark-1280x720-scenes.png`, `dark-1280x720-artist.png`
- `dark-1920x1080-scenes.png`, `dark-1920x1080-artist.png`
- `light-960x640-scenes.png`, `light-960x640-artist.png`
- `light-1280x720-scenes.png`, `light-1280x720-artist.png`
- `light-1920x1080-scenes.png`, `light-1920x1080-artist.png`

Review montages in both renderer directories: `review-scenes.png` and
`review-artist.png`.

## Integration

The lead adds this step to `verify.sh` (not edited on this branch):

```sh
step "scene posters and inline artist (tiles, editing, v1 bytes)" tools/wx5_features_check.sh
```

Project I/O differential passed: 3061 values across 6 comparisons, maximum
float delta 0. Final `tools/verify.sh`: **17 passed, 1 failed**, solely the expected stale
code map (`build/wx5-verify-final.log`). The headless, dock, top-bar, column,
lyrics, look/assist, Fluid Ink, gallery, Pixel Cats and poster gates all passed.
`cargo fmt --check`, clippy and all tests passed. The final
`cargo build --release` completed successfully in 31.60 s
(`build/wx5-release.log`); no later code change or build followed. All owned test
processes were stopped. The code map was deliberately left
unchanged for integration. Clippy exits successfully; pre-existing warnings
remain in untouched code, with no warnings introduced by these features.

## CPU timing comparison against the base

The optional full CPU `poster_check.sh` passed data, determinism, invalidation,
isolation, framebuffer and upright-image checks, but its wall-clock comparison
failed: 20 stalled frames with posters versus 0 without, worst at frame 118.
The poster step itself retained its 8 ms budget plus one replay frame.

An isolated `git archive be507e1` was compiled and its relevant sources compared
byte for byte with that revision. The same synthetic track, plan, parked time,
150 frames and llvmpipe path were then run as paired controls:

| Build | Posters off | Posters on |
| --- | --- | --- |
| `be507e1` | 0 stalled, worst 16.9 ms | 4 stalled, worst 36.9 ms (frame 143) |
| Feature branch | 1 stalled, worst 49.6 ms (frame 130) | 1 stalled, worst 26.7 ms (frame 2) |

The base also violates the CPU no-extra-stalls comparison, so this is not a
newly established regression from the features. These runs show timing
variability; they do not establish its cause. The assertion was not loosened.
Receipts: `build/wx5-poster-cpu.log`, `build/wx5-cpu-timing.log`,
`build/wx5-cpu-timing/results.json`, and its `baseline-receipt.txt`.

The first full verification had two additional failures: the newly fixed nested
Xvfb collision and the dock's global process scan. The latter searches every
process tree and did not print a matching PID; cross-session contention was a
hypothesis, not a proven cause. The final full rerun passed the dock gate with
its source untouched.
