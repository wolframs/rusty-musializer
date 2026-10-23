# Liquid Chrome and Iridescent Pool

Two original native material scenes, implemented in Rust and embedded GLSL 330.
They use no downloaded implementation, new dependencies or unsafe blocks.

Liquid Chrome (`chrome`, id 16) bends broad studio lights across silver folds.
Bass changes surface pressure, loudness swells secondary folds, and transients
add fine concentric deformation. Its small blue and warm reflected cards keep
it predominantly metallic; Hue rotates the restrained edge tint. Shine changes
the reflected lighting, Contrast deepens the folds, and Grain gives a restrained
static surface texture.

Iridescent Pool (`iridescent`, id 21) is a dark water surface carrying a varying
thin film. Three wavelength samples approximate optical interference; they are
an artistic material model rather than a spectral renderer. A transient launches
a travelling wave packet that bends the normal and the film's optical thickness.
Six packets are retained, in a fixed ring. A held onset launches one wave rather
than a new wave every frame. Shimmer changes the film's colour strength; Light
changes the grazing reflection; Spread changes its optical thickness range.

Both scenes use the shared `EffectInputs` definition of bass and
`TrackDynamics::level` for loudness when a profile exists. Their reports identify
`dynamics=track` or the explicit `absolute` fallback. Response zero leaves a useful
ambient material and prevents new audio deformation; an already moving response
relaxes and existing Pool rings finish their decay. Flow governs the slow material
clock; Pool's already launched ripples continue travelling even at Flow zero.

All drawing inputs come from seeded core state and settings. Invalid or zero
delta freezes the clock, envelopes, launch edge memory and wave ages. Drawing
never advances a clock, reads wall time or samples audio. Skipped draws therefore
lose no history. Coordinates use the short output axis; portrait output reveals
a taller portion of the surface without stretching the folds or rings.

## APIs

- `scenes::liquid_chrome::LiquidChromeState::new(seed: u64)` implements
  `SceneState`; `descriptor()` returns its version-1 descriptor. Read-only
  `phase()`, `clock()` and `drive()` expose the GPU inputs.
- `scenes::iridescent_pool::IridescentPoolState::new(seed: u64)` implements
  `SceneState`; `descriptor()` returns its version-1 descriptor. Read-only
  `phase()`, `clock()`, `drive()`, `ripples()` and `launches()` expose its inputs.
- Each app module exposes `Resources { pub report: String, .. }`,
  `Resources::load(&mut RaylibHandle, &RaylibThread) -> Self`, and
  `draw(&mut RaylibDrawHandle<'_>, &mut Resources, &NamedState,
  &SceneFrame<'_>, Rectangle)`.

Each shader must both pass raylib validity and expose its material-specific
uniform (`chromeShine` / `filmSpread`). Failure draws a notice naming the scene
and reports `shader=REFUSED`; successful drawing reports `shader=drawn`.

## Verification

The scene tests cover deterministic replay, distinct seeds, musical influence,
useful silence, zero response, and pause with changing audio and track time.
Chrome also checks track-relative loudness. Pool checks bounded travelling wave
history and a sustained onset producing exactly one launch.

```sh
cargo test -p musializer-core scenes::liquid_chrome
cargo test -p musializer-core scenes::iridescent_pool
```

Visual references were requested from the following pages. The implementation
session opened all three in its own T3 preview tab, but snapshots timed out twice
and the fallback browser was unavailable. A canvas capture returned black and
was not accepted as visual evidence. These are inspiration links, not a claim
that their rendering was successfully inspected:

- <https://webdev.teknium.io/d/paper/demos/paper-logo-liquid-metal/>
- <https://webdev.teknium.io/d/av/demos/regl-iridescent-puddle/>
- <https://webdev.teknium.io/d/av/demos/three-iridescence-foil/>

The shared debug app build passed. All seven scene tests passed, and scoped
`rustfmt --check` passed for the four Rust leaves. Strict core Clippy stopped
on eleven warnings in unowned existing modules (`unnecessary_map_or` and
`manual_is_multiple_of`); it reported none in these scene leaves.

Muted GPU captures used `tools/headless_run.sh`, with private state directories,
on both a synthetic fixture and an existing real-music excerpt. Both shaders
reported `shader=drawn` and `dynamics=track`. Landscape 1280x720 and portrait
720x1280 were inspected. Chrome reads as soft folded silver with sharp reflected
edges; Pool has dark gaps, more tightly spaced interference bands, grazing
highlights and transient ripple fronts. The first Pool capture had colour over
too much of the image; the final shader reduces coverage and increases the
optical thickness variation to restore dark water between coloured contours.

At seven seconds of the same real-music excerpt, Response zero versus default
produced the following RGB pixel differences (out of 255):

| Scene | Mean absolute difference | Pixels with a channel delta above 10 |
| --- | ---: | ---: |
| Chrome | 21.86 | 59.16% |
| Pool | 15.44 | 78.37% |

Capturing a ten-second frame alone and after an earlier seven-second capture
produced byte-identical PNGs for both scenes, proving drawing does not advance
the material history on the tested path. Receipts are in the gitignored
`build/material-scenes/audio-comparison.json` and `replay-comparison.json`.
The stills and logs are in the same directory. Four-second review MP4s used
`--encoder nvenc`; `chrome-review-silent.mp4` and `pool-review-silent.mp4` were
remuxed with `-an`, and ffprobe confirmed one video stream and no audio stream.

Live muted 180-frame runs entered each scene through `scene-pick` from Spectrum
with playback active. Both reported `family=materials`, visible Chrome and Pool
tiles, the selected scene's `shader=drawn`, zero dropped audio frames, zero output
underruns and zero dropped render frames. Screenshots and logs are
`chrome-live-picker.png` / `chrome-live.log` and `pool-live-picker.png` /
`pool-live.log`. The scene picker is the integration owner's implementation;
these runs exercise entry, not a full interaction/accessibility audit.

The integration owner performs the final release build; this leaf stream did
not run a competing release build. Final integrated project/picker gates remain
the integration owner's responsibility.
