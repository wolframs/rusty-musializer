# Fluid Ink

Pick **Fluid Ink** in Scenes, play a track, and open Tune to direct it. Luminous
ink rolls through dark water in front of a warm light. Each kick fires a burst
of fresh ink that arrives white-hot, rolls into a mushroom ring and cools to its
colour within half a second. Sustained bass builds pressure that spins the whole
vessel, swells the light and makes the next burst bigger. Snares and hats flick
thin sparks in the complementary colour. In a quiet passage the ink thins out,
lingers and breathes, and the light dims to an ember.

Inspired by the visible [WebGL Fluid demo](https://webdev.teknium.io/d/gl/demos/webgl-fluid-ink/)
(ink with bloom and sunrays) and the fluid-stirred marble demos in the operator's
October 23 reference. The solver and display shader are original code, with no
JavaScript runtime, downloaded library or copied implementation. The numerical
method follows [Stam's Stable Fluids](https://doi.org/10.1145/311535.311548):
semi-Lagrangian advection, pressure projection and vorticity confinement.

## Direct it

`--scene fluid` and `--scene fluid-ink` select the same scene. Treatments name
`fluid`; the current `--vocabulary` describes its settings and bounds.

- **Flow speed** changes how quickly the water moves.
- **Curl** strengthens the small eddies. Transients sharpen them for a moment.
- **Ink size** sets the size of the bursts and the width of the two ribbons.
- **Ink life (s)** is pigment's half-life in track seconds. A quiet passage
  stretches it by up to 1.9x; heat always cools in 0.42 s.
- **Music response** scales every musical force: bursts, sprays, the pressure
  swirl and the ribbons' loudness push. Zero leaves only the two ambient
  ribbons, useful for a restrained section or a comparison on identical PCM.
- **Ink hue (deg)** turns the palette. The backing light turns with it, so it
  stays the complement of the ink.
- **Luminous ink** sets the bloom, the white-hot heat and the light shafts.
- **Surface light** sets the gloss: how far the dye's folds bend the light,
  the sheen on them and the rim light on fine filaments.

All eight keep their keys, bounds, defaults and precision from the scene's
introduction; what they do visually changed on 2026-10-24 (see below). They are
ordinary settings: editable in Tune, routable, preset-compatible, and saved in
projects and section tuning. A different `hue` per section is how a treatment
gives each section its own palette. A Threshold doorway can show Fluid Ink with
`beyond: 16`.

```json
{
  "schema": "musializer.treatment/v1",
  "audio": {"path": "song.mp3"},
  "scenes": {
    "fluid": {"settings": {"curl": 1.2, "lifetime": 4, "response": 1.1}}
  },
  "segments": [{"at": 0, "scene": "fluid"}]
}
```

For an agent's silent preview, use `tools/headless_run.sh` as usual:

```sh
tools/headless_run.sh --treatment ink.treatment.json --still 6 build/ink.png
tools/headless_run.sh --treatment ink.treatment.json --encoder nvenc --render build/ink.mp4
```

## What the music does

Every envelope advances once per fixed 60 Hz substep from the frame's audio,
never per drawn frame, so 30 Hz and 60 Hz replays stay bit-identical.

| Gesture | Detector | What you see |
| --- | --- | --- |
| Kick | Mean positive excursion of the lowest quarter of the bands over their trails, threshold 0.05, 0.2 s refractory (Clawd's measured kick detector) | A burst from a point on a ring around the frame, aimed past the centre and twisting alternately, so bursts spiral. Placement walks a golden-angle sequence, so consecutive bursts never land together. A small heat core inside the pigment winds into the leading ring as a white-hot edge. The light flares and the exposure lifts for ~0.3 s |
| Sustained bass | Pressure: a follower of bass times loudness, 3.2 s to build, 1.8 s to release | The vessel spins (divergence-free solid-body rotation), bursts grow, the light swells and the palette leans warmer by up to 34 degrees. The spin reverses each time pressure has fully released, so two drops do not turn alike |
| Transient that is not a kick | Spectral flux x 6 above 0.42, 0.11 s refractory | A thin, fast flick across the last burst in the complementary hue, mostly heat, plus a moment of sharper curl |
| Quiet | Breath: 1 minus the loudness follower | Bursts shrink to puffs (a kick's force scales with loudness), the ribbons puff on a six-second breath, the water thickens, ink lingers, and the light pulses slowly |

Bursts at full force would land as hard in a soft intro as in a drop, because
the detector reads excursion, not level. Scaling them by the loudness follower is
what keeps the intro quiet.

## The picture

The display is one fragment shader over one float texture, back to front:

- **Water**: a dark violet-navy gradient with a warm light behind the ink. The
  light wanders slowly; its position comes from the solver's clock, so a still
  and the video frame put it in the same place. Its halo runs from gold at the
  core to deep violet at the edge, because dim amber reads as brown.
- **Depth**: a smaller, blurred copy of the ink behind the real one, scaled
  toward the light and drifting against the swirl.
- **Ink**: diluted ink clears to water through a soft density threshold, which
  is what gives a cloud an edge and the frame negative space. Thicker ink is
  purer in colour. The ink transmits its own colour and shadows the rest, so
  the light behind it turns it to stained glass. The dye is lit as a glossy
  height field from the same light: broad slopes carry the shading and sheen,
  fine slopes only a rim light, or every ripple of the solve glitters.
- **Heat**: white-hot where a kick has just landed, confined to the ink.
- **Shafts**: 28 steps toward the light through the coarse ink. Only the
  light's core is a source, so ink silhouettes cut beams instead of averaging
  into fog; thin ink tints them and hot ink radiates streaks.
- **Bloom** from the pyramid, mostly from heat.
- ACES tone map, an elliptical vignette, and film grain on a cell sized to the
  frame and a 24 Hz tick of track time, so two renders of one moment agree and
  the grain survives supersampling.

The texture is an atlas: the 512x288 dye on top (RGB pigment, alpha = heat) and
a five-level box pyramid of it in a 144-row strip underneath. Bloom, shafts and
the depth layer read the pyramid through a four-tap cubic B-spline. One texture
keeps the pass to one sampler: a second would need an `unsafe` sampler bind, and
GPU mipmaps would need `GenTextureMipmaps`, which logs a line on every call. The
texture is RGBA32F because the shader takes surface normals from the dye's
gradient, and 8-bit steps facet a gentle slope at 4x magnification.

## History and the solver

The Rust core owns a 256x144 velocity grid and a 512x288 dye grid with four
channels: three of pigment and one of heat. Sixty fixed substeps per track
second advect velocity, confine its curl, solve pressure, and transport dye with
a midpoint backtrace and limited MacCormack correction. The correction keeps
thin folds from dissolving under repeated interpolation; its source-neighbour
bounds prevent negative dye and ringing. Walls have zero normal flow and zero
normal pressure gradient. The display crops 5 % so the walls, where pigment
piles up, stay out of frame.

Pressure is 20 sweeps of red-black over-relaxation (omega 1.72), warm-started
from the previous step's pressure at 0.8. That removes more divergence than the
first version's 32 cold Jacobi sweeps did (the core test now demands 50 %).

The palette is an analogous window around a centre hue that drifts by 26
degrees over 110 s and leans with pressure: ribbons at -20 and +40 degrees,
bursts cycling -45, +15 and +75, sprays at +180. Neighbouring hues mix into a
third vivid hue; the first version's lime, orchid and coral mixed into olive and
grey wherever they met.

Dye transport is split across up to six threads (`std::thread::scope`, 256 KiB
stacks), which pull bands of rows from one queue. Each cell reads only the
previous buffers, so the split cannot change a value; a core test runs one and
seven workers and requires identical dye and velocity. The velocity pass is too
small to be worth the spawns and runs on one thread.

No drawing pass advances history. Pause freezes it; still and clip replay
advances it even when intermediate frames are not drawn. A render cannot inherit
the preview's earlier ink. Live scrubbing keeps the current vessel and resumes
with the new audio, following the existing scene-clock discontinuity policy.

One update banks at most six substeps (0.1 s). The first version allowed 0.25 s,
so after one stalled preview frame the next ran fifteen substeps, stalled too,
and the preview spiralled. Every export at 10 fps and above is a whole number of
substeps under the cap, so its frames are unaffected.

Portrait covers the frame by cropping the same vessel, preserving round
vortices. A portrait composition can crop out a burst, so inspect a portrait
still before committing a whole export.

Scene id 15 and token `fluid` are appended; no existing id, token or setting
moves. The project and treatment structures are unchanged; both persistence
expectation files pin the eight rows. `state_version` is 2 since the 2026-10-24
rework (four dye channels and the musical state).

## Report line

`fluid ink:` names the solver, grid, steps and whether the dye uploaded and drew
through the shader, then `kicks=`, `sprays=`, `pressure=`, `breath=` and
`ink=` (the share of the vessel holding visible pigment). A dead kick detector
and a working one on a track with no kicks draw the same plausible frame; the
count is what tells them apart.

## Evidence

`tools/fluid_ink_check.sh` checks synthetic PCM through the production treatment,
project, still and render paths: music response against ambient on identical
PCM, seed and time; that the kick detector fired on the pulse fixture; opacity
and portrait structure; an exact project reopen; and a still and windowed export
against the full video's frame. `--negative-control` disables music response in
both treatments and must fail the music comparison.

2026-10-24 rework, measured on this machine (RTX 3090, release build, VirtualGL):

- Check: music versus ambient MAD 7.53, reopen exact, still/video 37.04 dB,
  full/clip 40.89 dB. Negative control: MAD 0.000, fails as required.
- Solver cost per 60 Hz substep (`examples/fluid_probe.rs`, 30 s of a real
  track): 9.6 ms before, single-threaded with three dye channels; 4.3 to 4.7 ms
  after, with four channels, at a machine load of 3 to 10. One worker costs
  16 ms, three 7.6 ms, five 5.5 ms.
- Preview, 600 frames at the default window (`--probe-frames 600`): before, 17
  and 6 frames past 25 ms (worst 27.7 and 49.9 ms). After, most runs 0 to 2
  (worst 17.7 to 45.4 ms). With five other agents loading the machine (load 12
  to 19), some runs still showed bursts of stalls, while Spectrum stayed clean
  at the same load: a threaded step waits for its slowest worker, and a busy
  machine parks workers. The band queue and the six-substep cap shorten those
  bursts; they do not remove them.

Core tests pin seed and replay determinism, audio influence, finite bounded dye
and velocity, pause, equivalent 30 and 60 Hz substeps, pressure removing half the
divergence, worker-count independence, a kick laying down heat that cools within
a second, response 0 ignoring kicks, and pressure building, breathing and
reversing the spin.

`crates/musializer-core/examples/fluid_probe.rs` replays decoded PCM through the
export's frame cadence, times every update and prints the envelopes per frame;
it is how the kick, pressure and breath numbers above were read off real
material. Aesthetic acceptance remains the operator's judgment.
