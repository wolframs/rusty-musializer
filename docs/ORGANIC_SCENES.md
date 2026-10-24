# Metaball Garden and Voronoi Bloom

Original Rust and embedded GLSL330 scenes. They take their look from Teknium
demos, worked out from what the demos show on screen. No third-party source,
asset or dependency was read or copied.

- [Molten SDF metaballs](https://webdev.teknium.io/d/gl3/demos/three-raymarcher-metaballs/):
  candy-finished goo beside a chrome ring.
- [Wobbly candy blobs](https://webdev.teknium.io/d/scene2/demos/tresjs-wobble-blobs/):
  sweets on a dark floor with sparkles and contact shadows.
- [Procedural clear-coated blob](https://webdev.teknium.io/d/scene/demos/three-tsl-node-material/)
  and the [Shader Park sculpture](https://webdev.teknium.io/d/fx/demos/shader-park-sdf-sculpture/):
  a coat over colour, and a thin-film sheen.
- [Perforated godrays lantern](https://webdev.teknium.io/d/gl3/demos/good-godrays-lattice/):
  light shafts cut by a lattice into haze.
- [Refractive glass](https://webdev.teknium.io/d/scene/demos/drei-transmission-glass/)
  and the [css-doodle patterns](https://webdev.teknium.io/d/fx/demos/css-doodle-patterns/):
  refraction, and a pattern built with intent rather than at random.

The 2026-10-24 rework replaced the first versions, which the operator judged
too plain: "didn't quite go all out with the scene design and all the bells
and whistles". The first Metaballs was four glossy spheres on a flat void. The
first Voronoi was flat teal and orange cells with no light or depth. Both still
use the same ids, tokens and settings descriptors (keys, bounds, defaults,
precision). Saved projects still open but **look different, on purpose**.
`state_version` is 2 for both.

## Metaball Garden (`metaballs`, scene 18)

**The look.** A molten candy sculpture in a dark plum studio. The main blobs
come in several sizes, from one big mother blob down to small ones. Satellite
droplets orbit further out. All of them merge through one smooth union. Each
blob keeps its own colour, and colours mix only where the goo meets. The main
mass uses a warm family (hot pink, tangerine, orchid, coral), because those mix
cleanly where lobes merge; pink and lime would turn grey. The droplets use cool
accents (aqua, lime, cornflower), so they read as separate sweets. The material
is a clear coat over colour:

- wrapped diffuse light, and thin necks and droplets that let light through;
- reflections of four rectangular softboxes, which make the window highlights
  of a photographed sweet;
- a sharp key highlight, and a rim light that flares on kicks.

A tilted chrome ring, with a thin-film sheen and three black glass beads, turns
slowly around the sculpture. The lacquer floor shows:

- a soft shadow from the key light;
- contact occlusion where the goo comes close to the floor;
- colour that each blob spills onto the floor below it;
- a shadow tinted by the translucent candy;
- a reflection that blurs with the height of what it reflects.

Behind it all is a cyclorama that glows behind the sculpture and falls to
black.

**What it does with the music.** It feels like mass, not a meter.

- **Kicks** use the bass-transient detector that Clawd measured on real mixes
  (`clawd::KICK_THRESHOLD`, the positive excursion of the lowest quarter of the
  bands over their trails). Each kick hits two damped springs:
  - The *squash* spring (3 Hz, underdamped) presses the sculpture flat toward
    the floor. It bulges sideways, then rebounds once into a stretch. The
    contact shadow tightens as it drops.
  - The *burst* spring (1.5 Hz) flings the droplets clear and pushes the main
    blobs apart, so the goo splits. It then pulls them back and they merge
    again with a wobble.
  - While both springs move, the jelly surface quivers. The ripple amplitude is
    `wobble()`, taken from the springs' speeds.
- **Sustained bass** swells every blob and loosens the goo, through a larger
  smooth-union radius.
- **Track-relative loudness** is followed twice:
  - A fast follower drives the key light and the magenta softbox.
  - A slow *section energy* follower changes the whole scene. In loud sections
    the blobs gather into one molten mass, the camera dollies in about 0.75
    units, and the clock runs at 130 %. In the track's own quiet the blobs
    drift apart into separate droplets, the clock slows to 70 %, and the studio
    dims.
- **Treble transients** use the same excursion measure over the top third of
  the spectrum. Each one strikes one or two star glints on the surface of a
  blob, toward the camera and the light. Glints flare in 40 ms and fade within
  0.6 s.

`examples/organic_probe.rs` measured this on *Door, No Room*: 468 kicks
(about 1.5 per second), around 950 glints, and squash peaks of 0.25 to 0.48.
The glint gate (`HAT_THRESHOLD` = 0.045) fires a few times a second in busy
sections and almost never in pads.

**Controls.** Keys, bounds and defaults are unchanged.

| Control | What it does now |
| --- | --- |
| `flow` | Speed of the sculpture's clock (orbits, ring, camera drift). 0 freezes motion; kicks still squash it. |
| `count` | Main blobs, 3 to 9. Up to six droplets join them. |
| `size` | Mass of every blob. The camera backs off for a large sculpture. |
| `response` | Scales every musical term. 0 keeps the ambient sculpture and removes kicks, glints, swell and section energy. |
| `hue` | Turns both palettes about the grey axis. |
| `metal` | 0 is candy under a clear coat. 1 is liquid metal: the studio reflected, tinted by the pigment and a thin film. Default 0.45 is a glossy candy. |
| `glow` | Translucency of thin parts, rim light, colour spilled on the floor, the tinted shadow, and glint size. |
| `orbit` | How far blobs and droplets spread, and the ring's radius. The camera backs off so the ring stays in frame. |

## Voronoi Bloom (`voronoi`, scene 20)

**The look.** A leaded stained-glass window with the sun behind it and haze in
front. The palette is composed, not scattered:

- a warm heart of gold, ruby, rose and pale amber around the window's centre;
- a field of cobalt, violet and emerald toward the rim, with rare pale blue.

The family comes from each pane's lattice cell, not from its moving site, so a
pane never changes colour as it drifts.

Each pane is rolled cathedral glass. It has a low ripple that bends the sun
behind it, density mottling, streaks along a direction set per pane, and seed
bubbles that focus a point of light inside a dark ring.

The lead came is a rounded bevel lit by the room, with solder blobs at the
joints, and the glass darkens slightly under its edge. In front of the window,
haze scatters the light that came through into coloured shafts radiating from
the sun. Dust drifts in the beams, and the lens blooms around the sun.

**What it does with the music.**

- **Twelve followed spectral voices** light the window like a rose window: bass
  at the centre, treble at the rim. Each pane takes the value between two
  voices from its distance to the centre, so a pane never jumps between them.
  A contrast curve keeps a sounding band apart from a quiet one, even in a
  dense mix.
- **Kicks** (the same detector):
  - flare the sun and pulse the shafts;
  - start a *bloom wave* from a beat seed, alternately the sun and a seeded
    pane. Each pane lights when the wave front reaches it and fades behind it,
    so a kick reads as a disc of light growing outward with a bright rim, as
    if the glass were growing.
- **The hardest kicks of a loud section** crack one pane near the light:
  - Nine zig-zag fractures and two rings spread from the impact.
  - Each fracture has a bright core and a darker seam, and light pours through.
  - The pane flares, then heals over 2.4 s.
  - The gate is a kick excursion of at least 0.13 (`SHATTER_KICK`) with section
    energy of at least 0.5 and a 4 s cooldown.
  - The probe counted 15 cracks across *Door, No Room*, all in loud sections
    and none in the breakdowns.
- **Bass** swells the sun and thickens the haze.
- **Section energy** works as in Metaballs. On loud sections the sun rises and
  brightens and the drift runs faster. In quiet sections the sun sinks and dims
  and the glass deepens to cobalt, with the dust in the last light.

**Controls.** Keys, bounds and defaults are unchanged.

| Control | What it does now |
| --- | --- |
| `flow` | Speed of the window's clock (site drift, the sun's path, the haze). |
| `cells` | Panes along the short axis, so a portrait keeps the scale of a landscape. |
| `edges` | Width of the lead came. Even 0 keeps a thin came; real glass is never seamless. |
| `response` | Scales every musical term. 0 keeps the ambient window with no voices, waves or cracks. |
| `hue` | Turns the glass palette. The sun stays warm. |
| `glow` | Strength of the shafts, the halation around the sun, and the dust. 0 is a clean window. |
| `drift` | How far each site wanders and how far the window pans. |
| `facet` | Glass texture: ripple refraction, mottling, streaks and seeds. 0 is smooth glass. |

## State and rendering contracts

Each core module has its named state with `new(seed: u64)`, implements
`SceneState`, and has a `descriptor()`. Core holds every piece of history: the
clocks, the followers, the springs, the wave and glint ring buffers, and the
crack. Each app module has `Resources { pub report: String, .. }`,
`Resources::load`, and `draw(d, resources, state, frame, boundary)`. It uploads
uniforms and keeps no time of its own, which is why a still, a clip and a full
export draw the same picture. The springs integrate at a fixed 1/240 s step, so
a preview at 60 fps and an export at 30 fps move them the same way (a unit test
pins this). Only a positive, finite scene delta advances state, so pause
freezes everything.

Core has no GPU, filesystem or OS state. Loudness uses `TrackDynamics` when
present and a bounded absolute fallback when it is absent. The reports say
`dyn=track` or `dyn=none`, so the fallback cannot hide.

The shaders run on fixed budgets:

- **Metaballs:** at most 112 primary steps over up to 15 spheres, a ring and
  three beads, inside a bounding sphere that core computes. The floor
  reflection uses 40 steps and the soft shadow 20.
- **Voronoi:** 9 nearest-site candidates, then 25 for the two nearest seams,
  plus 22 shaft samples of 9 candidates each.

Every hash is an integer hash, never `fract` of a large product. Every `pow`
has a non-negative base. Colours are lit in linear light, tone-mapped and
written opaque. Both shaders reject raylib's fallback shader by checking
distinctive uniform locations, and visibly name an unavailable scene.

The report lines name what the frame held, because a scene that stopped hearing
kicks still photographs as a plausible scene:

- **metaballs:** `blobs=6+6, kicks=N, glints=K live, squash=, burst=, light=, energy=, treble=, clock=, dyn=`
- **voronoi:** `kicks=N, waves=K live, crack=0.70s|none, sun=strength@(x,y), light=, bass=, energy=, clock=, dyn=`

Both lines keep `surface=shader`, which `tools/scene_gallery_check.py` checks
for.

## Verification (2026-10-24)

- **Core tests:** `cargo test -p musializer-core --lib -- scenes::metaball_garden scenes::voronoi_bloom`,
  10 passed. They cover:
  - seeded replay, and different seeds;
  - ambient motion, the bass swell, and the track-relative light;
  - a kick that squashes, splits, rebounds and settles;
  - a spring that does not depend on the replay frame rate;
  - glints that spawn and expire;
  - quiet sections that slow the clock while loud ones gather the blobs and
    dolly in;
  - low and high voices kept apart;
  - bloom waves that travel outward;
  - a crack only on a hard kick in a loud section, once per cooldown, and
    healing;
  - the sun rising with the section;
  - response zero, and exact pause preservation.

  The full `cargo test` passed.
- **`tools/scene_gallery_check.py`'s per-scene check** ran for these two scenes
  only, through a wrapper. It uses the same fixture and assertions; the tool has
  no scene filter.

  | | metaballs | voronoi |
  | --- | --- | --- |
  | response MAD | 18.3 | 60.9 |
  | motion MAD | 11.5 | 14.4 |
  | project reopen | identical | identical |
  | still vs video | 38.7 dB | 35.7 dB |
  | full vs clip | 42.9 dB | 40.2 dB |

  Its `--negative-control` fails both scenes with `music response missing (MAD 0.0000)`, as it must.
- **CPU against GPU:** the same 640x360 still through llvmpipe
  (`MZ_GL_LAUNCH=""`) and through the RTX 3090 agrees at 62.9 dB (Metaballs,
  mid-kick) and 63.4 dB (Voronoi, mid-crack).
- **Cost:** a 10 s windowed export at 1920x1080 High (2x supersampled) on the
  RTX 3090 took the following wall time, compared with Spectrum at 20.2 s:
  - Metaballs 22.0 s (+1.8 s), about 4.6 ms per 3840x2160 frame for the
    scene;
  - Voronoi 22.4 s (+2.2 s), about 5.6 ms per frame.

  Both are a small fraction of a 16.7 ms preview frame at full-window preview
  sizes. The live preview's `frame budget:` line (release build, 600 frames
  playing a loud passage) showed no stalls past 25 ms for either scene in an
  unloaded run, at the default 1280x720 window and at 1920x1080 in Full. That
  line is noisy while other renders share the GPU: Spectrum itself logged 6
  stalls in one such run.
- **Judged by eye** on contact sheets of 12 moments across *Door, No Room*
  (quiet intro, build, drops, breakdown, finale), sheets of beat sequences, a
  sweep of every control's extremes, and 540x960 portraits. The upgrade's
  before and after sheets are in the gitignored `build/scene-upgrade/organic/`.
  This is the implementation's own review, not the operator's listening
  acceptance.
