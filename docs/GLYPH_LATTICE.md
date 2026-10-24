# Glyph Lattice

A wireframe solid lit on a fixed honeycomb of tiny glyphs. Post-legacy,
2026-10-24, id 28, token `glyphs` (alias `glyph-lattice`), in the *Geometry*
family. No oracle.

## Why, and the boundary

The operator saw a hero animation on modal.com and said: *"the style is super
cool, I'd want it"*, and *"I wouldn't want to steal it 1:1"*. The lead
captured it headlessly as three images (a still, a close-up and a 13-frame
motion sheet). As observed: a cube seen corner-on, rasterised onto a fixed
hexagonal lattice of pixel-art glyph icons. Cells inside the cube's thick
strokes light up, brightness falls off from each stroke's centre, a warm
bloom pools where strokes cross, glyphs flicker on their own phases, the
solid turns slowly, and faint ghost copies of other shapes sit behind in a
darker tone. Monochrome green on black.

This scene is built from that visible behaviour only, by the rule the
repository already applies to Le Biniou (`AGENTS.md`, "Outside inspiration").
None of the site's JavaScript, shaders or SVG icons was fetched or read. The
glyph set is this scene's own and is drawn from geometry in the shader. No
brand appears. The default palette is aqua with amber heat, not the
reference's green. The scene also has to be a music video scene rather than a
screensaver, which the reference does not attempt.

## The picture

**The lattice.** Offset rows of cells, `density` (default 120) cells across
the frame's short side. The pitch is a fraction of the short side, not a
pixel count, so a 720p preview, a 1080p export and a supersampled one light
the same cells and only the glyphs get sharper (`field::Lattice`). At 1080p a
cell is about 9 px.

**A stroke lights cells.** The solid is projected in perspective and every edge
drawn as a thick stroke. A cell's level is a smoothstep of its distance from
the stroke's centre line, so the tier a cell lands in is a measured distance,
not a texture:

| Tier | Where | Glyphs |
| --- | --- | --- |
| edge | the outer third of a stroke | seed dot, pinhole ring, dash |
| body | between edge and core | ring, tri-spoke, bullseye, triangle |
| core | the centre line | mostly discs, some filled hexagons and bold crosses |
| hot | crossings and pulses | four-point spark, six-spoke star |

Each tier is a mix. An integer hash of the cell picks within it, and about a
third of the core keeps line glyphs, so a stroke reads as glyphs side by side
rather than a honeycomb grid (all-hexagon cores tiled into one in the first
renders). A steady per-cell brightness jitter, squared so a few glyphs flare,
gives the stroke its sparkle. Glyphs are signed-distance shapes with a
one-pixel edge, and they stay crisp at any size.

**Light.** Strokes are summed across and maxed within, so a polyline's joints
stay even while two strokes over one cell pool into heat (sums of squares, so
a stroke's soft edge adds little). Two strokes meeting warm a cell, three or
more make it hot. A tight halo and a wide pool are the lit field and its heat
blurred on the lattice itself. A soft aura sits behind the solid. Heat turns
the glyphs into sparks and warms the bloom toward amber.

**Ghosts.** Three echoes of the solid as it was 0.08, 0.19 and 0.33 s ago,
from a track-time angle history, so a fast turn or a kick's lurch smears.
The solid the scene just left exhales into the background, growing as it
fades. Two other solids, large and slow, turn at the frame's long ends (left
and right in landscape, top and bottom in portrait). All of them are drawn in
a small, grey ghost glyph.

## The solids

`solid` names one per section: 1 cube, 2 octahedron, 3 dodecahedron,
4 icosahedron, 5 tesseract, 6 trefoil knot. 0 (the default) runs a cycle that
opens on the cube, in a seeded order. The cycle moves on at a drop (a kick at
loud level after two seconds of quiet) or after about 22 s on one solid.

A change morphs, including a cut between two sections of this scene with
different `solid` values (the scene instance is kept, so the state sees the
setting move). The new solid assembles outward from a vertex over 1.1 s, hop
by hop along its edges, with a spark at each growing tip, while the old one
exhales into the ghosts over 1.8 s. Played from the top of a track, the
first solid assembles as the video opens. Picked while paused, or entered
mid-track, it is simply there.

Dense solids draw thinner strokes (`Form::stroke_scale`) so their faces stay
open. The tesseract turns through the fourth axis with perspective in w, so its
inner cube swells through the outer one, and kicks push it. The knot is one
stroke, so its joints never heat. Its three self-crossings are found in
projection and given their own heat.

**The star tetrahedron was in the first set and is gone.** Seen down a body
diagonal, or side-on with one upright, it is a hexagram, a symbol rather than
a solid. Tilting it did not keep it off that pose. A cuboctahedron does the
same down its three-fold axis. The dodecahedron has no such view.

## What the music does

| Musical figure | What you see | Why |
| --- | --- | --- |
| Kick (the Atmospheres' adaptive detector) | A comet: a bright head with a streak behind it, running a seeded walk along the edges from a vertex (never doubling back, never taking an edge twice). A hard kick fires two. The spin lurches about five degrees | A hit you watch travel instead of a flash. The first version flooded the pulse out along every edge at once, which turned a 30-edge icosahedron white on every kick |
| No kick for 2.6 s | An ambient comet | Quiet passages and `response` 0 still have light moving on the edges |
| Track-relative loudness (`TrackDynamics`) | The turn (about 4 degrees a second quiet, 17 loud), stroke weight, echo strength, the giants' presence | A verse is a thin, slow solid and a chorus a heavy one, on quiet and loud masters alike |
| Spectrum, 16 groups | Each edge's brightness and weight by its height on the solid: lower edges the bass, upper ones the treble. Along its length on the knot | The solid shows which part of the mix is moving, and that turns with it |
| Bass | The halo, the pool and the aura swell | Weight |
| Treble | Glyph flicker depth | Hats as glitter |
| Hat ticks (the same detector on the top quarter of the bands) and onsets | A ripple from the centre: a faint ring of light crosses the lattice, and every glyph it passes turns over to another of its tier | Real mixes rarely reach the absolute onset lane (16 times in the first 67 s of *Door, No Room*), so the hats drive it, at most once every half second |
| The lyric line (`words`) | Set in stroke capitals, standing in place in the ghost tone from the cue's first frame, then lit word by word over the cue's first 38 % (at most 1.15 s), each word igniting hot and cooling; kicks sweep left to right through it and heat its joints; it dissolves into the ghosts when its cue ends | See below |

`response` scales every coupling. Toward 0 loudness leans to a neutral middle,
so 0 is a steady mid-weight solid on the ambient clock, byte-identical for any
audio.

## The line

With `words` on (the default), the current lyric line is set in a stroke font
authored for this scene: capitals on a 4-by-6 grid with corners cut at 45
degrees, so letters share the solids' faceted look (`strokes.rs`). Accented
Latin-1 letters fold to their capitals (`é` → `E`, `ß` → `SS`). Anything
else is dropped, never drawn as a box. The block wraps to the row count that
makes the letters largest: up to six rows, using portrait's height. It tilts
gently in depth and swells a little on kicks. While a line is set, the solid
steps back into the ghost tone, 30 % larger. Between lines it comes forward
again.

**The reveal lays the whole line out first.** From the cue's first frame
every word stands in its final place in the dim ghost tone. The words then
light in reading order, each over 0.12 s with a flash of heat that cools over
about a third of a second, so a word sparks as it is sung. The first version
grew stroke after stroke, and a long row could show a lone `I` and half an
`N` at its left end, outside the block above it: a stray mark, not a word
arriving.

**The letters glow like the solids' edges.** A letter's junctions (a stroke
ending on or crossing another, as in T, E, H, X) and its sharp corners (as
in L, V, M; the 45-degree cut corners are not) gather warm light, the way a
solid's vertices do, and run hot on the kicks (`strokes::joints`). Each word
breathes with a band of the spectrum, the line's first word low and its last
high. Letter strokes also write their own halo source, because they are
thinner than a solid's edges and would otherwise throw less glow. Halo and
joint heat scale with letter size, so a long line in small letters stays
sharp instead of going soft between rows. The caption overlay is not drawn over this scene while `words` is on.
A line with a voice turns the palette to the voice's hue (MV9-h; the
palette's own hue is 174 degrees).

## Controls

| # | Name | Range | Default | What it does |
| --- | --- | --- | --- | --- |
| 0 | `solid` | 0–6 | 0 | 0 the cycle; 1 cube, 2 octahedron, 3 dodecahedron, 4 icosahedron, 5 tesseract, 6 knot. A persisted index |
| 1 | `words` | toggle | on | The lyric line in the lattice |
| 2 | `spin` | 0–2 | 1 | Turn rate, drift and the kick lurch; 0 holds the pose |
| 3 | `size` | 0.5–1.6 | 1 | The solid's size (radius 0.32 of the short side at 1) |
| 4 | `density` | 60–200 | 120 | Cells across the short side |
| 5 | `stroke` | 0.5–2 | 1 | Stroke weight |
| 6 | `glow` | 0–2 | 1 | Halo, pool and aura |
| 7 | `ghosts` | 0–1 | 0.6 | Echoes and giants (the exhaling solid is always drawn) |
| 8 | `flicker` | 0–1 | 0.5 | Glyph twinkle and blink, scaled by the treble |
| 9 | `hue` | −180–180 | 0 | Turns the aqua palette; the heat stays warm |
| 10 | `saturation` | 0–1.5 | 1 | Palette saturation |
| 11 | `response` | 0–2 | 1 | How far the music reaches the picture |

## How it draws

`core::scenes::glyph_lattice` owns the state: followers, the kick and hat
detectors, comets, ripples, the angles and their history, the solid and its
morph, and the remembered line. Everything is integrated from the frame's
track-time delta. Non-positive or non-finite deltas change nothing.

`field::render` is a pure function of the state, the frame and the output's
aspect. It projects the solids, ghosts and line and rasterises them into the
lattice. Each stroke only visits the rows and columns its capsule can reach.
It then blurs the bloom on the lattice and packs two RGBA8 texels per cell:
`(lit, heat, ghost, wash)` and `(halo, warmth, pool)`. The app uploads them
into one 1024×512 atlas allocated once (`scenes::glyph_lattice`) and draws the
boundary through `glyph_lattice.fs`. The shader finds a pixel's cell (the
nearer of the two rows around it), draws that cell's glyph, interpolates the
bloom between cell centres, and adds the ground, the faint socket every cell
shows, flicker, vignette and grain.

Shader rules: integer hashes only. No derivatives, because the glyph changes
from cell to cell and `fwidth` there is undefined, so the edge width is the
pixel's size in cell units, a uniform. Flicker and grain read track time,
never a frame counter. No new `unsafe`.

## The report line

```text
glyphs: shader=drawn form=cube(cycle) assembled=1.00 previous=none lattice=217x143 pitch=8.5px lit=dim 528/body 743/core 2031/hot 141 ghost=7167 echoes=3 giants=2 waves=2 ripples=1 washed=0 bloom=0.81 line=none level=0.91 bass=0.78 treble=0.27 kicks=104 pulses=125 onsets=99 dyn=track cpu=2.34ms
```

A lattice with no core, a detector that never fired, comets that never ran, a
line that never drew and a lost track profile each still draw a plausible
honeycomb, so the line names what the frame held. `line=` gives the text,
strokes drawn of total, and `live` or `dissolving`. `cpu=` is the field's
time, a measurement and never an input.

## Evidence

- **Core tests (24, `scenes::glyph_lattice`).**
  - The lattice depends on the aspect, not the pixels.
  - Every cell is its own nearest cell.
  - Seeded replay draws the same cells.
  - `response` 0 is byte-identical for a loud groove and silence, with ambient
    pulses still firing.
  - Kicks fire comets that light cells.
  - A new solid assembles while the old one exhales.
  - The cycle moves on at a drop.
  - Bass and treble light opposite halves of the cube.
  - On a cue's first frame the whole line is laid out in the ghost tone and
    no word has lit. It lights word by word and dissolves.
  - Each letter's joints are where its strokes meet and its corners turn.
  - `words` off never sets it.
  - Onsets ripple and wash cells.
  - Pause freezes everything.
  - A quiet and a loud master read the same.
  - Every solid draws in 16:9, 9:16 and 1:1.
  - A fresh state shows its solid, and the top of a track builds it.
  - Shape tests: edge counts, hops, walks that never repeat an edge, rotations.
  - Font tests: every letter inside its cell, accent folding, wrapping.
- **`tools/scene_gallery_check.py --only glyphs`**:
  - Response MAD 10.22, motion MAD 10.56.
  - Project reopen identical.
  - Still against its NVENC video frame 42.10 dB luma; full against clip
    43.56 dB.
  - The report line is read: tiers, ghosts, pulses, kicks, ripples,
    `dyn=track`.
  - The full `tools/verify.sh` passed on `:80`, 13 of 13.
  - All six Geometry tiles click at the 960×640 / 240 px minimum.
  - `--negative-control` fails with MAD 0.0000.
- **Differential.** `settings_post_legacy.txt` and
  `route_persistence_post_legacy.txt` gain only the twelve rows, the counts
  and `beyond`'s maximum (28 → 29). `differential_settings.sh` failed on a
  `density` bound perturbed 200 → 201 and passed once reverted.
- **Live**, release build, RTX 3090, *Door, No Room* from 2:20 (loud), 600
  frames, panels hidden:
  - 0 of 600 frames past the 25 ms budget at 1280×720, 1920×1080 and
    2560×1440 (worst 19.0, 17.7 and 20.9 ms).
  - Zero dropped PCM frames, zero output underruns.
  - The field took about 2.4 ms of CPU.

Contact sheets and stills are under `build/glyphs/` (gitignored):

- *Door, No Room*, *Casually Superhuman* and *Ache*, twelve moments each
  (`v7-sheet.png`, `v15-*.png`, `v16-Ache.png`).
- 9:16 (`v7-portrait.png`, `v20-portrait.png`).
- The lyric line on its real timing (`v10-words.png`, `v11-50.png`).
- A morph between two sections (`v21-morph.png`).
- A line's reveal at 0.1 s steps (`v28-reveal.png`), and text frames beside
  the loud cube (`v27-sheet.png`, `v27-portrait.png`).
- Comets over one second (`v13-motion.png`).
- The reference beside a frame (`v9-compare.png`).

## Not built

- A waveform or spectrum polyline as the subject, and other shapes
  rasterised from a section's own geometry.
- Pixel-art bitmap glyphs like the reference's. These are smooth shapes
  with a one-pixel edge: crisp at every export size rather than at one.
- Frame persistence (`runtime::feedback`). The echoes are analytic, from
  the angle history, so a still needs no warmup.
- Staging individual lines differently inside one section.
