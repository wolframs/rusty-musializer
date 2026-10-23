# Metaball Garden and Voronoi Bloom

These are original Rust and embedded GLSL330 scenes inspired by the visible
Teknium demos, with no third-party source, assets or dependencies copied.

- [Molten metaballs](https://webdev.teknium.io/d/gl3/demos/three-raymarcher-metaballs/): the sculptural smooth unions and broad glossy reflections.
- [Paper loading states](https://webdev.teknium.io/d/paper/demos/paper-loading-states/): quiet, continuous organic motion.
- [Voronoi glass](https://webdev.teknium.io/d/code/demos/pts-voronoi-glass/) and [microscope](https://webdev.teknium.io/d/css3/demos/houdini-voronoi-microscope/): drifting cellular structure.

## The two looks

**Metaball Garden** (`metaballs`, scene 18) is a little sea-glass and orchid
sculpture floating above a velvet floor. Buoyant spheres pull into necks and
separate again. Bass increases their radius, track-relative loudness fills their
light, and treble strengthens the broad warm and cool studio reflections.
The default stays coloured and dimensional before music starts.

`flow` controls the sculpture clock; `count` selects 3–9 spheres; `size` changes
their mass; `response` scales the musical followers; `hue` turns the material;
`metal`, `glow` and `orbit` shape the surface and separation. Flow zero stops
motion while music can still change the sculpture. Response zero leaves ambient
motion and removes music from the followers once they decay.

**Voronoi Bloom** (`voronoi`, scene 20) is a field of petrol, sea-glass and amber
cells. Dark lead seams, directional bevels and etched inner contours make the
picture read as glass rather than flat noise. Twelve followed spectral regions
light different cells; bass lights their centres and track-relative loudness
provides the wider breathing light. Seeded sites drift gracefully without audio.

`cells` is density along the short axis, so a portrait retains the same glass
scale as a landscape. `edges` thickens the seams; `facet` controls the directional
face and bevel; `glow` lights the seams; `drift` sets the site excursion rate;
`flow`, `response` and `hue` serve the same purposes as in Metaballs.

## State and rendering contracts

Each core module exposes its named state with `new(seed: u64)`, `SceneState`,
and `descriptor()`. Each app module exposes `Resources { pub report: String, .. }`,
`Resources::load(&mut RaylibHandle, &RaylibThread) -> Self`, and:

```rust
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &NamedState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
)
```

Core has no GPU, filesystem or OS state. Only a finite positive scene delta
advances the clock or audio followers; pause freezes all history. A seed and
input sequence replay exactly without drawing. Bass uses shared `EffectInputs`;
loudness uses `TrackDynamics` when present and a bounded absolute fallback when
absent. Reports name `dyn=track` or `dyn=none` so the fallback cannot hide.
Drawing uploads only a few uniforms. No new unsafe blocks or render feedback.

Metaballs bounds the primary raymarch to 64 steps over at most nine spheres,
with four normal taps and analytic ground shadows; it never launches secondary
shadow or reflection marches. Voronoi uses nine nearest-site candidates and 25
edge candidates per pixel, independent of cell density, with integer lattice
hashes. Both use coordinates relative to the shorter image axis and output
opaque pixels. Both reject raylib's fallback shader by checking distinctive
uniform locations, visibly name an unavailable scene, and report a fallback
separately from `surface=shader`.

## Verification

The core tests exercise seeded replay, different seeds, ambient clock motion,
independent bass/treble response, track-relative light (Metaballs), separate
low/high spectral voices (Voronoi), response zero, and exact pause preservation.
Render evidence and performance measurements are recorded below. The integration
owner performs the final release build.

### 2026-10-23 leaf acceptance

- `cargo test -p musializer-core scenes::metaball_garden -- --nocapture`: 2 passed
  (repeated after the default mass/spacing adjustment).
- `cargo test -p musializer-core scenes::voronoi_bloom -- --nocapture`: 2 passed.
- `cargo build -p musializer-app`: passed against the parent's shared wiring.
  Only the four owned Rust leaves were formatted with standalone `rustfmt`.
- All application launches used `tools/headless_run.sh`, the debug binary,
  isolated state directories, private Xvfb/VirtualGL and the wrapper's mute and
  invalid audio-sink guards. Real decoded music remained the analyzer input.
- Real-music review: the existing local `you-opening-0-20.mp3` clip, seed 42,
  seconds 4–10, 1280×720, 30 fps, balanced quality, NVENC. Each output has 180
  video frames. The two `*-review.mp4` deliverables have **only a video stream**;
  their soundtracks were removed with `-map 0:v:0 -c:v copy -an` and `ffprobe`
  verified the absence of audio. No user audio or generated output is staged.
- Final landscape stills: `build/organic-scenes/metaballs-final.png` and
  `voronoi-final.png`. Portrait real-music stills:
  `metaballs-music-portrait.png` and `voronoi-music-portrait.png` (540×960).
  All four were pixel-inspected; every alpha byte is 255.
- Same seed/time, real music versus response zero: mean absolute RGB difference
  3.856/255 for Metaballs and 41.069/255 for Voronoi. These measure visible
  coupling, not aesthetic quality. Zero-response stills (`*-zero.png`) retain a
  complete ambient picture. The separate core tests establish frozen history
  on pause and repeatable input-sequence replay.
- Reports say `surface=shader` and `dyn=track`, with nonzero followed musical
  values. Final still logs: `metaballs-final.log`, `voronoi-final.log`; review
  logs: `metaballs-music.log`, `voronoi-music.log`.

The first physical Metaballs capture found a defect that compilation missed:
negative bases passed to GLSL `pow` made the sculpture black except for isolated
highlights. Using absolute values before the even powers fixed it. The initial
image is intentionally replaced by the corrected evidence, and the report is
still checked separately so a fallback could not earn visual acceptance.

Creative judgment: Metaballs now reads as a buoyant glazed sculpture, with
clearly merging necks, coloured studio reflections and a restrained floor shadow;
Voronoi reads as illuminated leaded glass, with a quiet structural drift and
individual notes lighting distinct cells. Both remain composed in portrait.
These are an implementation review, not operator listening acceptance.

Reference-browser limitation: all four assigned URLs were visited in a private
T3 tab created with `reuseExistingTab=false`. Metaball canvas pixels were inspected
successfully without reading source. T3 snapshots repeatedly returned
`Preview automation snapshot timed out after 15000ms.` The Pts canvas stayed
blank and Paper's canvases reported 0×0 even after an explicit viewport resize;
the microscope's visible labels were readable but its CSS picture could not be
captured. Those three effects therefore do not have a completed pixel review.
No third-party script, shader, CSS implementation or asset was read or copied.

Performance is bounded by the fixed shader loops, but this is not a portable
frame-rate guarantee. The two initial six-second exports ran concurrently on the
RTX 3090 and each took about 4.5 seconds wall including decode/startup/encoding;
that is end-to-end export evidence, not isolated GPU timing. A final Metaballs
export after widening its safe bounding sphere is timed in
`metaballs-render.time`. Preview's frame-budget line reports zero frames for
these offline exports and is not being used as a performance measurement.

The integration owner owns the required final `cargo build --release`, broad
checks, compact scene picker and release acceptance. This leaf task changes only
its assigned seven source/document files and gitignored evidence; no commit,
reset, stash or shared-file edit was performed.
