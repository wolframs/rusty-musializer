# Fluid Ink

Pick **Fluid Ink** in Scenes, play a track, and open Tune to direct it. The
lime, turquoise, orchid and coral ink rolls into what is already in the vessel:
bass pushes one jet, track-relative loudness pushes another, and spectral
transients stir the third. A quiet track still leaves a slow, moving picture.

Inspired by the visible [WebGL Fluid demo](https://webdev.teknium.io/d/gl/demos/webgl-fluid-ink/)
in the operator's October 23 reference. The solver and display shader are original
code, with no JavaScript runtime, downloaded library or copied implementation.
The numerical method follows [Stam's Stable Fluids](https://doi.org/10.1145/311535.311548):
semi-Lagrangian advection, pressure projection and vorticity confinement.

## Direct it

`--scene fluid` and `--scene fluid-ink` select the same scene. Treatments name
`fluid`; the current `--vocabulary` describes its settings and bounds.

- **Flow speed** changes how quickly the water moves.
- **Curl** strengthens the small eddies.
- **Ink size** widens the moving jets.
- **Ink life (s)** is pigment's half-life in track seconds.
- **Music response** scales the musical forces and injections. Zero leaves the
  ambient flow, useful for a restrained section or a comparison on identical PCM.
- **Ink hue (deg)** turns the palette without changing the water.
- **Luminous ink** adds a soft local light around the pigment.
- **Surface light** brings out the folds in the dye.

All eight are ordinary settings: editable in Tune, routable, preset-compatible,
and saved in projects and section tuning. A Threshold doorway can show Fluid Ink
with `beyond: 16`.

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

## History and display

The Rust core owns a 256×144 velocity grid and a 512×288 RGB pigment grid.
Sixty fixed substeps per track second advect velocity, confine its curl, solve
pressure (32 Jacobi iterations), and transport dye with a midpoint backtrace and
limited MacCormack correction. The correction keeps thin folds from dissolving
under repeated interpolation; its source-neighbour bounds prevent negative dye
and ringing. Walls have zero normal flow and zero normal pressure gradient.

The GPU uploads a bounded, tone-mapped view of that pigment and shades it with
surface light, a local halo and hue rotation. A cached transfer table avoids
hundreds of thousands of exponentials per drawn frame. No drawing pass advances
history. Pause freezes it; still and clip replay advances it even when intermediate
frames are not drawn. A render cannot inherit the preview's earlier ink. Live
scrubbing keeps the current vessel and resumes with the new audio, following the
existing scene-clock discontinuity policy; it does not reconstruct all pigment
from the track's beginning. Use a still to inspect the exact exported frame.

Portrait covers the frame by cropping the same vessel, preserving round vortices.
It does not stretch a landscape simulation into tall ovals. This also means a
portrait composition can crop out an outer jet. A different aspect is a different
composition, so inspect a portrait still before committing a whole export.

Scene id 15 and token `fluid` are appended; no existing id, token or setting moves.
The project/treatment structures stay unchanged, using their existing scene-token
and settings-key extension points. Both persistence expectation files pin the
added rows. At introduction, Threshold's `beyond` maximum grew from 14 to 16;
15 still refuses self-nesting and every previously valid selection keeps its
meaning. The later [scene gallery](SCENE_GALLERY.md) raises that maximum to 22.
The scene-plan schema vocabulary follows the parser's registry.

## Evidence (2026-10-23)

`tools/fluid_ink_check.sh` checks synthetic PCM through the production treatment,
project, still and render paths. It compares music response against ambient on
identical PCM/seed/time, checks opacity and portrait structure, reopens the saved
project, and compares a still and windowed export with the full video's frame.
`--negative-control` disables music response in both treatments and must fail the
music comparison. It is part of `tools/verify.sh`'s full gate.

Core tests pin seed/replay determinism, actual audio influence, finite bounded
pigment, pause, equivalent 30/60 Hz substeps, and pressure reducing divergence.
The initial 16-iteration projection failed the compression test (47,706 residual
versus 56,051 before); the 32-iteration solve passes the required 20% reduction.

Generated artifacts live under `build/fluid-ink/` and `build/fluid-ink-check/`.
The release was entered by clicking its scene tile while the operator's real
track played, opened in place. After the transfer-table optimization, 300 preview
frames advanced 306 fluid steps, with zero dropped PCM frames and zero output
underruns; seven frames exceeded 25 ms (worst 32.8 ms) while other gates ran.
The earlier debug run fell behind; the release check establishes the delivered
binary's behavior, not a debug-build performance claim. No user audio was copied
into the repository. Aesthetic acceptance remains the operator's judgment.

The visible gate measured response-versus-ambient MAD 17.438, exact project
reopen pixels, still/video PSNR 38.61 dB and full/clip PSNR 40.41 dB. The disabled
response negative control is recorded in `build/fluid-negative-control.log`.
