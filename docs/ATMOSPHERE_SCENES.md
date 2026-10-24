# Aurora Veil and Neon Tunnel

Two original native scenes, the Atmospheres family. The first versions
(2026-10-23) were competent screensavers: Aurora read as a teal comb of
vertical lines over a wavy band, a spectrum barcode rather than a sky, and the
Tunnel was flat concentric hexagon wireframes. The operator's verdict on that
wave was that it "didn't quite go all out with the scene design". Both were
rebuilt on 2026-10-24 (SX5) from the same brief: depth, light, texture, motion
with intent, and a response to the music you can feel.

Inspiration, read for visible behaviour only, captured headless and muted
(`build/scene-upgrade/atmospheres/ref/`): the webdev.teknium.io catalogue's
[layered liquid hero](https://webdev.teknium.io/d/gl3/demos/shaders-com-liquid-hero/),
[Takram's golden hour over the Alps](https://webdev.teknium.io/d/scene2/demos/takram-atmosphere-sky/)
(haze, ridges), the [Paper god-rays hero](https://webdev.teknium.io/d/paper/demos/paper-god-rays-hero/),
the [shader-doodle audio-texture tunnel](https://webdev.teknium.io/d/av/demos/shader-doodle-audio/)
(a spectrum ring receding into its own trail), the
[threejs-toys neon tube](https://webdev.teknium.io/d/gl/demos/threejs-toys-cursors/) and
[three-good-godrays](https://webdev.teknium.io/d/gl3/demos/good-godrays-lattice/).
No third-party source, artwork, colormap or dependency was copied.

## Aurora Veil (`aurora`, id 17)

A night lake under the northern lights. The camera stands on the shore with
the horizon a quarter of the way up the frame at every aspect, drifting slowly
in yaw and pitch. Folded curtains of light stand over a mountain ridge, green
at the lower border, fading upward into crimson tops, mirrored in rippled
water; stars twinkle above, a faint galactic band gives the dark sky texture,
and the aurora rims the ridge crest and lights the snow.

**How the curtains are made.** Each curtain is a vertical sheet standing on a
curve in the ground plane, at one of three distances (far arc, middle,
overhead). A view ray is walked across the band of ground the sheet can occupy,
and the sheet's Gaussian thickness is integrated *exactly* over each linear
piece of the walk (an erf difference), so the sheet never bands however few
steps are taken. A ray grazing a fold collects a long path through the sheet,
so the bright fold lines of a real aurora fall out of the geometry instead of
being painted on. The curve's x coordinate is warped by depth, which lets it
double back into S-folds and hooks. World-vertical rays converge toward the
zenith by perspective alone. The same function is evaluated for the mirrored,
rippled ray under the horizon, which is what puts the aurora in the lake.

**What the music does.**

| Musical figure | What you see | Why |
| --- | --- | --- |
| Track-relative loudness, phrase-length follower (rise ~1 s, settle ~3 s) | How much of the sky is alight: one calm arc low over the ridge in a quiet passage; the middle curtain with the phrase; the overhead curtain only when the song is genuinely loud. Fold depth, crimson tops and the lake's glow follow | An aurora that changes shape every analyzer tick reads as a meter. Sections should read as weather |
| Quiet (low activity) | The lone arc breathes: a slow swell and settle | Quiet passages should feel alive, not empty |
| Bass (trailing low quarter, soft-kneed) | The lower border brightens and drops; a pink nitrogen fringe appears under it; the lake ripples harder | Weight. A real substorm's lower border does exactly this |
| Kicks (adaptive bass-transient detector) | A *flaming* wave: a band of light races up the rays from the border and fades; the whole sky lifts briefly | One legible accent per hit, at any distance |
| A strong kick while the sky is sparse (activity < 0.55), at most once per 9 s | A shooting star crosses the sky | A single streak reads as an event in a quiet passage; on every chorus hit it would be a screensaver |
| Treble | Ray shimmer speed and contrast; deeper star twinkle | The hats as glitter, through a phase accumulator so a rate change never jumps the pattern |
| Mids | A violet tint in the middle of the curtains | Colour that moves with the arrangement |

In the probe's replay of *Door, No Room* the detector fires on the track's
main accents rather than on the bass line under them (intervals between hits
cluster near 0.9 s and 1.8–1.9 s), and the shooting star appears seven times
in 5:22, all in quiet passages.

**Controls.** Keys, bounds, defaults and precision are unchanged (they are a
`.musi` surface pinned by `tests/differential/*_post_legacy.txt`); what changed
is what they look like.

| Control | Range | Default | Now |
| --- | --- | --- | --- |
| Flow | 0–2 | 0.5 | Speed of the folds travelling along the curtains and of the camera drift |
| Height | 0.2–1.5 | 0.8 | Altitude span of the curtains (the overhead curtain reaches 30 % higher) |
| Folds | 1–12, integer | 5 | Fold frequency along each curtain, in folds per screen width |
| Response | 0–2 | 1 | Scales every musical coupling. Below 1 it also lifts a floor under the activity, so 0 is a calm mid-activity sky (two curtains) rather than the near-empty sky of silence |
| Hue | −180–180° | 0 | Rotates the aurora's colour about the grey axis (green core at 0), including its light on the ridge and snow |
| Glow | 0–2 | 1 | Curtain brilliance and the soft halo around each sheet |
| Stars | 0–1 | 0.35 | Star field, galactic band and shooting-star brightness |
| Spread | 0.3–2 | 1 | Horizontal scale of the fold pattern: wider, lazier folds as it rises |

## Neon Tunnel (`tunnel`, id 19)

Flight through a dark, wet, polygonal tunnel. The camera flies along a gently
curving centre line and looks along its tangent, so the far end swings away and
the camera banks into each bend. Neon rings stand on the walls at fixed world
depths and are the scene's only lights: they light the panels around them,
glow in the haze of their own cross-section, and are mirrored in the glossy
walls and the puddled floor. Faint emissive panel seams form a grid that
carries the perspective and the speed where no ring is near; corner rails carry
light dashes running toward the camera faster than the walls. At the end, a
hot exit light throws god rays through the haze. Rings close to the lens
defocus into a soft glow of the same energy instead of a hard bar.

**The rings are the music.** The state keeps a 64-slot spectrum history at
1/60 s per slot, counted in track time so a 30 fps export and the 60 Hz preview
sample the same moments. It is uploaded each draw as an 18 × 64 RGBA8 texture
(16 band groups, a kick-flash column, a loudness column) and bound as the
quad's own `texture0`. Each ring shows the spectrum around its circumference —
bass on the floor, treble on the ceiling, mirrored left and right — and bulges
toward the camera where its band is loud. It shows that spectrum *as it was
when a wave fired from the camera reached it* (34 world units per second), so a
kick lights the nearest ring at once and then visibly races away down the
tunnel. The near ring being immediate is what makes the hit felt; the trail is
what makes it seen.

| Musical figure | What you see |
| --- | --- |
| Kick | A forward lunge (travel surge), a field-of-view punch, an exposure flash with the exit light, a small roll, rings smeared into streaks trailing away from the camera, and the flash wave running down the tunnel |
| Kick rate (leaky estimate, kicks per second) | Cruise speed: a busier groove flies faster, saturating so a fill cannot send the camera to warp |
| Loudness | Pace, haze and exit light; a quiet passage glides at half speed with the rings down to embers |
| Spectrum (history) | Ring brightness and shape around the circumference, per ring, delayed by its depth |
| Bass | The aperture breathes slightly wider |
| Treble | Rail dashes brighten and shift toward cyan |
| Travel | The palette leans up to ~25° either way over minutes, so the verse and the last chorus are not the same tunnel |

**Controls.** Keys, bounds, defaults and precision unchanged.

| Control | Range | Default | Now |
| --- | --- | --- | --- |
| Speed | 0–3 | 0.75 | Base travel speed; 0 parks travel and roll (the music still lights the rings) |
| Twist | −2–2 | 0.6 | Signed helical twist of the prism along its length, so travelling forward rolls the tunnel; also the direction of the kick roll |
| Sides | 3–12, integer | 6 | Polygon of the cross-section; the floor is a flat face at rest |
| Response | 0–2 | 1 | Scales every musical coupling. Below 1 the rings keep an ambient glow of their own, so 0 is a steadily lit tunnel |
| Hue | −180–180° | 0 | Rotates the palette (magenta floor, violet walls, cyan ceiling, cyan rails, violet exit) |
| Glow | 0–2 | 1 | Ring halos, haze and god rays |
| Depth | 0.5–3 | 1 | Ring density along the tunnel (spacing 1.7 / depth) |
| Width | 0.4–1.6 | 1 | Tunnel radius |

## State and rendering contracts

Both scenes keep the established shape: `core::scenes::<name>::<Name>State`
with `new(seed: u64)`, `SceneState`, a module `descriptor()` (now
`state_version: 2`), and an app module exposing `Resources::load` and `draw`.
State has no I/O, raylib, wall clock or ambient randomness; nonpositive or
nonfinite deltas leave every field unchanged; deltas are capped at 250 ms; the
seed and the input sequence determine everything. Rendering never advances
state and keeps nothing between draws, so a still needs no drawing warmup and
an export cannot inherit a preview's frames. Grain is an integer hash of a
pixel cell and a 24 Hz tick of *track* time.

`core::scenes::aurora_veil` also owns the family's shared musical vocabulary,
which Neon Tunnel imports: `Features::of` (track-relative level, trailing bass,
the bass-transient signal, flux, mids, treble), `KickDetector` (an adaptive bar
at 1.7× a 0.6 s running mean, never below 0.032, re-armed only after the signal
falls under 80 % of the bar, 0.2 s refractory, a leaky kick-rate estimate) and
`follow`. They were tuned with `crates/musializer-core/examples/atmosphere_probe.rs`,
which replays a decoded track through the export cadence and prints both
states' envelopes per frame.

Report lines: `aurora: shader=drawn, clock, bass, activity, mids, treble,
surge, kicks, flaming, meteors, dynamics=track|absolute` and `tunnel:
shader=drawn, distance, bass, loudness, mids, treble, surge, punch, cruise,
kicks, history=floor/ceiling, dynamics=…`. A detector that never fires and a
history that stays empty photograph as plausible frames, so the counts and the
history's newest floor and ceiling values are printed. Shader validity still
requires a scene-specific uniform (`auroraTime` / `tunnelTravel`) on top of
raylib's check; failure draws the labelled unavailable scene.

Shader portability: integer hashes only, no `pow` of a possibly negative base,
derivatives (`fwidth`) taken only in uniform control flow (the tunnel's
reflection is computed unconditionally for that reason), `textureLod` for the
history reads inside loops. No new `unsafe`.

## Verification, 2026-10-24

- `cargo test`: the whole workspace passes. The scenes' own 13 tests cover
  seeded replay, seed variation, music-driven envelopes, independent spectral
  evidence in the history (floor for bass, ceiling for treble), a kick's flash
  appearing near the front of the history and an older copy further down,
  history slots counted by track time (30 and 60 fps agree), one flaming wave
  per hit, one shooting star per cooldown in a sparse sky and none in silence,
  a held excursion firing once, pause freezing every field including under
  invalid deltas, Response 0 leaking no audio, Speed 0 parking travel, and the
  same relative phrase lighting quiet and loud masters identically.
- `tools/scene_gallery_check.py` (restricted to `aurora` and `tunnel`): pass.
  Response MAD 21.42 / 29.53 (was 18.84 / 25.60), motion MAD 14.43 / 20.49,
  project reopen identical, still against NVENC video frame 36.32 / 33.76 dB,
  full against clip 39.84 / 37.39 dB. The tunnel's still/video figure sits where
  the earlier tunnel (34.55) and Voronoi Bloom (33.49) did: thin saturated
  lines lose most in 4:2:0 chroma, and lowering the grain changed it by 0.01 dB.
- NVIDIA against Mesa llvmpipe (`MZ_GL_LAUNCH=""`), same still at 2:30 of *Door,
  No Room*: 54.19 dB (Aurora), 59.66 dB (Tunnel).
- Every control's extreme rendered as its own section of one treatment
  (`build/scene-upgrade/atmospheres/sweep/`): all draw, none collapse.
- Real time, release build, RTX 3090, *Door, No Room* from 2:20 (a loud
  passage), 600 live frames, panels hidden: 0 of 600 frames over the 25 ms
  budget at 1280×720, 1920×1080 and 2560×1440 for both scenes (worst 16.7–23.3
  ms; the previous versions measured 16.8–26.1 ms, one stall). A GPU-bound
  comparison, 600 frames exported at 2560×1440 High (5120×2880 internally),
  took 79.5 s for Aurora against 84.9 s before and 81.0 s for the Tunnel
  against 75.6 s before: about +9 ms per 15-megapixel frame for the tunnel,
  about 1 ms at 1080p, and no measurable change for Aurora.

Contact sheets, before and after, are under `build/scene-upgrade/atmospheres/`
(gitignored): `before/{aurora,tunnel}-door-sheet.png` and
`after/{aurora,tunnel}-door-sheet.png` (twelve moments of *Door, No Room*),
`after-portrait/`, `after-dark/` (a second track), `kick/` (a flaming wave
climbing over 0.8 s), `meteor/`, `v4/tunnel-sheet.png` (a drop: the flash wave
and the lunge), `llvmpipe/` and `sweep/`.

Operator aesthetic acceptance remains open.
