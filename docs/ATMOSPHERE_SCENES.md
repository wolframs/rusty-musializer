# Aurora Veil and Neon Tunnel

Two original native scenes, inspired by the visible demos at
[React Bits backgrounds](https://webdev.teknium.io/d/bits/demos/reactbits-backgrounds/),
[Audio Borealis](https://webdev.teknium.io/d/av2/demos/audio-borealis-voice-card/),
[shader-doodle audio](https://webdev.teknium.io/d/av/demos/shader-doodle-audio/) and
[threecn effects](https://webdev.teknium.io/d/kits2/demos/threecn-fx-scenes/).
No third-party implementation, artwork or dependency was copied. The reference
pages were navigated in a separate T3 preview tab, without enabling sound or the
microphone. T3 snapshots timed out; isolated, muted Chromium captures supplied
visual reference, including the selected Aurora view.

Aurora Veil (`aurora`, id 17) layers four luminous folded curtains over a navy
sky. Narrow vertical filaments emerge from the folded hems; broad sheets and
soft halos carry the phrase beneath a restrained seeded star field. Bass opens
the folds, track-relative loudness lifts their height and brightness, and the
upper spectrum adds fine light. The default is green through cyan into blue.

Neon Tunnel (`tunnel`, id 19) flies through twisting polygonal neon ribs. The
aperture expands with bass, and successive ribs alternate bass, mid and treble
lighting. Bright tube centers sit inside saturated halos; near ribs pass out
of frame while a darker vanishing point remains visible. The default is a
cyan, blue and violet hexagonal spiral.

Both scenes normalize coordinates by the shorter output axis. Portrait keeps
the same shapes and fills its height; landscape gains width without stretching.
Response zero retains an animated ambient composition. Flow/Speed zero parks
travel while musical light can still move. Pause freezes all evolving state.

## Controls

| Aurora control | Range | Default | Effect |
| --- | --- | --- | --- |
| Flow | 0–2 | 0.5 | Curtain drift speed |
| Height | 0.2–1.5 | 0.8 | Vertical reach |
| Folds | 1–12, integer | 5 | Broad folding frequency |
| Response | 0–2 | 1 | Musical deformation and illumination |
| Hue | −180–180° | 0 | Palette rotation |
| Glow | 0–2 | 1 | Sheet brilliance and luminous hems |
| Stars | 0–1 | 0.35 | Star field brightness |
| Spread | 0.3–2 | 1 | Horizontal fold spread |

| Tunnel control | Range | Default | Effect |
| --- | --- | --- | --- |
| Speed | 0–3 | 0.75 | Forward travel |
| Twist | −2–2 | 0.6 | Signed depth twist |
| Sides | 3–12, integer | 6 | Rib polygon |
| Response | 0–2 | 1 | Aperture and spectral illumination |
| Hue | −180–180° | 0 | Palette rotation |
| Glow | 0–2 | 1 | Tube halos |
| Depth | 0.5–3 | 1 | Rib density in depth |
| Width | 0.4–1.6 | 1 | Base aperture |

## State and rendering contracts

`core::scenes::aurora_veil::AuroraVeilState` and
`core::scenes::neon_tunnel::NeonTunnelState` expose `new(seed: u64)`, implement
`SceneState`, and have a module-level `descriptor() -> SceneDescriptor`.
Their state is only a seeded phase, an accumulated f64 clock/distance and four
musical envelopes. There is no I/O, raylib, wall clock or global random source.
Nonpositive/nonfinite deltas leave every state field unchanged; long deltas
are capped at 250 ms. Seed and input sequence fully determine the result.

Both use shared `EffectInputs` bass from the trailing spectrum, and
`TrackDynamics::level` when the track profile exists. Absolute RMS is the
explicit fallback. Mid and high regions use `SceneAudioFrame::spectral_regions`.
Attack/release followers are slower for Aurora and faster for Tunnel. Preview
seeking retains current travel and adapts to the new music; offline replay
reconstructs from the start, including updates on frames that are not drawn.

Each app module exposes exactly:

```rust
pub struct Resources { /* shader */ pub report: String }
impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self;
}
pub fn draw(
    d: &mut RaylibDrawHandle<'_>, resources: &mut Resources,
    state: &NamedState, frame: &SceneFrame<'_>, boundary: Rectangle,
);
```

Aurora's state getters are `clock()`, `phase()`, `music()`; Tunnel's are
`distance()`, `phase()`, `music()`. Music is `[bass, loudness, mids, treble]`.
The renderer uploads these and the six display controls, then draws one quad.
Aurora has four analytic sheet iterations; Tunnel has 28 analytic projected
ribs. Neither shader ray marches, allocates feedback targets or runs a CPU
simulation. Rendering never advances state. No new unsafe code was added.

Validity requires both raylib's shader check and a scene-specific uniform
(`auroraTime` / `tunnelTravel`), because raylib can return its default shader on
compile failure. Failure draws a scene-named unavailable label; reports say
`shader=REFUSED`. Success says `shader=drawn`, includes all four envelopes and
names `dynamics=track` or `dynamics=absolute`.

## Verification, 2026-10-23

Eight focused core tests cover exact seeded replay, seed variation, music-driven
envelopes, independent spectral regions, pause including invalid deltas, ambient
Response zero, Speed zero, and matching light at equivalent positions in quiet
and loud tracks' own dynamics ranges. Commands:

```sh
cargo test -p musializer-core scenes::aurora_veil
cargo test -p musializer-core scenes::neon_tunnel
```

The integrated debug application built successfully. Only these four Rust leaves
were formatted. Normal `cargo clippy -p musializer-app --all-targets`
completed with existing warnings and none in these scene leaves. Strict Clippy (`-D warnings`) was blocked by 11 existing lints
outside these leaves, principally `unnecessary_map_or` and
`manual_is_multiple_of`; those shared files were left untouched.

All application launches used `tools/headless_run.sh`: private Xvfb, private
configuration, unreachable Pulse/Wayland, dialog guard, application `--mute`.
The real-track evidence used the existing local *Warmth Source (Really Warm)*;
no audio was copied into the repository. Artifacts are gitignored under
`build/atmosphere/`:

- `aurora-landscape.png`, `aurora-portrait.png`, `tunnel-landscape.png`,
  `tunnel-portrait.png`: inspected at 1280×720 and 720×1280, at 12 s.
- `aurora-ambient.png`, `tunnel-ambient.png`: Response zero, same track/seed/time
  as the matching active portrait/landscape captures. Mean absolute RGB
  difference is 12.20/255 for Aurora and 21.84/255 for Tunnel. Pixels with a
  channel changing by more than 8/255 comprise 64.8% and 71.1%, respectively.
- `tunnel-replay.png`: fresh process replay matches `tunnel-landscape.png`
  byte for byte (SHA-256
  `6cb543c3a8ee22e355d7c2bce854726970857bf5137a3e7e02d0c0d5d6d9c9a5`).
- `aurora-live.png`, `tunnel-live.png` and their logs: entered through
  `--ui-probe panel=none,scene-pick=aurora|tunnel,play=1`, 240 frames. Reports
  confirm the Atmosphere category and selected tile. Real decoded PCM consumed:
  191520 / 190560 frames, zero dropped PCM frames and zero output underruns.
  Worst frame: Aurora 25.0 ms (1/240 above the 25 ms gate), Tunnel 17.8 ms
  (0/240 above it), while the other scene reviews also ran. These are
  debug-build observations on the RTX 3090, not a release performance ceiling.
- `aurora-review.mp4`, `tunnel-review.mp4`: six-second NVENC reviews at 960×540,
  music passage 8–14 s. Audio was removed with `ffmpeg -an -c:v copy`;
  `ffprobe` confirms a video stream only. `*-motion.png` sheets inspect six
  successive moments of folding/travel.
- `reference-*.png`: visible reference captures, retained only in build output.

Creative judgment: Aurora reads as luminous fabric with room to breathe;
Tunnel reads as a crisp, forward-moving spiral. Their motion and composition
remain distinct at both aspect ratios. Operator aesthetic acceptance remains
open. The live UI captures also contain the existing isolated-run recovery
snapshot warning (no per-user state root); that warning is outside these scenes.
The parent integration stream owns full gates and the final required release
build; these leaf checks do not claim a completed release qualification.
