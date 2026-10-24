# Liquid Chrome and Iridescent Pool

Two original material scenes in Rust and embedded GLSL 330. They use no
third-party source or assets, no new dependencies and no unsafe blocks. The
look was reworked on 2026-10-24: the first versions (commit 79d73ed) were grey
satin folds and rainbow noise, and the operator's verdict was that they did not
go all out. Inspiration, read for visible behaviour only, came from the
webdev.teknium.io catalogue: "Metal FX liquid chrome", "three-raymarcher molten
metaballs beside a chrome ring", "Paper Shaders liquid metal", "regl iridescent
feedback puddle" and "three.js thin-film iridescence".

Both scenes keep their id, token and every settings descriptor exactly as
before (`tests/differential/settings_post_legacy.txt`). What the controls *do*
changed with the look; the tables below are the current meaning. Saved
projects keep their values and draw the new look.

## Liquid Chrome (`chrome`, id 16)

A mercury sea in a dark studio, with a chrome ring standing in it. The sea is an
analytic height field and the ring one torus, marched together; one reflection
bounce is traced against the ring, and the ring's lower half sees the sea
through a cheap second bounce. Twelve softboxes stand round a dark cyclorama,
spaced so two or three are always in frame, with three gels whose colour the
Hue control turns. The camera circles the ring on the material clock (about
seven minutes a turn at the default Flow), and the ring mostly turns to meet it.
Metal reflections carry a 60 % blend of a per-channel normal split, which gives
thin spectral fringes on edges without the red rims a full channel swap paints
on bright lights.

| Music | Lane | What you see |
| --- | --- | --- |
| Bass (`EffectInputs::bass`) | heavy swell | sharp-crested waves rolling through the metal, carrying past the note |
| Kick (bass excursion over trails, Clawd's measured 0.05 gate, 0.25 s refractory) | impact | a wave front leaving the ring's whole footprint, bulges racing up both legs of the tube, the softboxes strobing, a 4.5 % lens punch |
| Other transients (onset edge or flux jump) | droplets | small ripple rings landing on the visible sea, held in world space |
| Track-relative loudness (`TrackDynamics::level`) | light and chop | brighter gels and seam, finer chop; a quiet passage settles into a glassy mirror |

| Control | Meaning now |
| --- | --- |
| Flow speed | the material clock: swell, molten wobble, camera orbit |
| Fold scale | the sea's spatial scale; swell height falls as it tightens so a fine sea stays a sea |
| Ripples | kick fronts, tube bulges and droplets |
| Music response | every audio lane; zero leaves the ambient sea and orbit |
| Highlight hue | turns the three gels |
| Shine | softbox and overhead panel intensity |
| Contrast | contrast about middle grey after the film curve |
| Grain | film grain, sized to the picture and ticked by track time at 24 Hz |

## Iridescent Pool (`iridescent`, id 21)

An oil film on dark, deep water, seen from a camera gliding over it. The film's
colour is thin-film interference for oil (n 1.47) on water (n 1.33), with the
phase flip on the top surface only, so the thinnest film is dark. It is summed
over 24 wavelengths whose linear-sRGB weights are baked from the CIE 1931
Gaussian fit of Wyman, Sloan and Shirley (JCGT 2013); coherence falls with
thickness so thick oil goes pastel. Thickness climbs from nothing at the edge of
each drifting, domain-warped patch to about a micron in its heart, so every
patch carries the Newton sequence as concentric bands. Light refracts down to a
pebble floor; the floor brightens where the surface above it is convex, so
every ring train draws caustic rings on the stones. The water and the oil
reflect a dusk dome, a soft window and two neon signs.

No frame persistence: the smear of the reference puddle comes from the flowing
warp instead, so still, clip and export agree by construction and the scene
needs no warmup or reset in `scene_host`.

| Music | Lane | What you see |
| --- | --- | --- |
| Kick (same detector as Chrome) | big drop near the middle | a ring train, a hole torn in the oil with a thickened rim that closes over seconds, caustic rings on the floor, a brief lift of the window |
| Other transients | rain | small drops anywhere in view that only prick the film |
| Bass | swell | a slow noise swell rolling every reflection (noise, because crossed sines focus their caustics into a mesh) |
| Track-relative loudness | chop and light | fine chop and a brighter glow round the window |

Twelve drops are kept; a new one replaces the drop with least wave left
(`Ripple::remaining`), so rain never cuts off a kick's rings.

| Control | Meaning now |
| --- | --- |
| Flow speed | the material clock: marbling drift, swell, camera glide |
| Film scale | the marbling's scale; the patch layout follows it only weakly, so coarse film is bigger swirls rather than clear water |
| Ripples | drop wave height |
| Music response | every audio lane; zero leaves the ambient pool |
| Film hue | turns the film's colours |
| Shimmer | film strength over the water's own reflection |
| Grazing light | the window, neon and glow; the dome stays, so zero is moody, not black |
| Ripple spread | how fast and wide drop rings travel, and how wide a kick tears the film |

## Determinism and APIs

All drawing inputs come from seeded core state and settings. Invalid or zero
delta freezes every clock, follower, launch memory and drop age; launched waves
finish even at Flow zero. Drawing never advances state, reads wall time or
samples audio. The only per-frame input outside core state is a 24 Hz tick of
track time (Chrome's grain, Pool's anti-banding dither). Drop origins are
computed in core from the same camera the shader uses and stored in world
space; `RING_CENTRE_XZ`, `CAMERA_HEIGHT` and `CAMERA_REACH` must stay equal to
the shader constants of the same names.

- `scenes::liquid_chrome::LiquidChromeState` (descriptor state version 2):
  `phase`, `clock`, `drive`, `pulse`, `camera_orbit`, `impacts`, `droplets`,
  `kicks`, `splashes`.
- `scenes::iridescent_pool::IridescentPoolState` (state version 2): `phase`,
  `clock`, `drive`, `pulse`, `camera_drift`, `ripples`, `launches`, `kicks`.
- Each app module exposes `Resources { pub report, .. }`, `Resources::load` and
  `draw`. A shader that fails to compile, or lacks `chromeShine`/`impacts` or
  `filmSpread`/`rings`, draws a notice and reports `shader=REFUSED`. Report
  lines name kicks, droplets or drops and how many are live, so a track that
  moves the surface but never launches anything says so.

## Verification (2026-10-24)

`cargo test -p musializer-core scenes::liquid_chrome` (9 tests) and
`scenes::iridescent_pool` (7 tests) cover replay, seeds, one front per kick
under a sustained excursion, weakest-slot replacement, drops landing where the
camera looks, Flow zero, pause, zero response and track-relative loudness.

Renderer agreement, one 960x540 still each through `tools/headless_run.sh`:
llvmpipe against the RTX 3090 is 46.7 dB for Chrome and 63.3 dB for Pool.

Cost, measured as a 10 s, 1920x1080 High (2x supersampled, so 3840x2160 drawn)
export window against Spectrum's 20.4 s: Chrome 23.6 s and Pool 26.5 s, about
11 ms and 20 ms of shader per 4K frame. Chrome confines its march to the slab
the waves can reach plus the ring's bounding sphere (43 ms before that); Pool
bakes its colour-matching table (57 ms before). Live muted 600-frame probes
showed no frame past the 25 ms stall line at the default preview or at a
1447x814 panel.

`tools/scene_gallery_check.py`, run for these two scenes: response, motion,
portrait, reopen (byte-identical) and full-versus-clip all pass. Pool's
still-versus-video-frame pair measures 31.0 dB (NVENC) and 32.0 dB (x264)
against the tool's 33 dB floor: the comparison is a PNG against a lossy
640x360 yuv420 frame, and the oil's saturated band edges are exactly what
chroma subsampling loses. Luma alone is 39.7 dB, and after a 1.5 px blur the
RGB pair is 38 to 42 dB, so it is the same picture.
