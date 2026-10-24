# Pixel Cats

Six pixel-art stagings of the lyric line, each its own scene, sharing one
implementation (`core::scenes::pixel_cats`) and one picker family, *Pixel
cats*. Post-legacy, 2026-10-24, ids 22–27, no oracle.

## Why

The operator asked a Claude model, in a plain chat, for a video for his
chiptune-gabber track "/\\_/\\ (meow!)". It used nothing but Python, numpy and
ffmpeg. Every frame was drawn at 320x180 with a hand-made 5x7 font and
procedural cats, upscaled 6x behind a CRT grid, and every section got a gag tied
to what its line says: a boot screen, a side-scroller over keycap rooftops, a
barbershop quartet, monk cats under a rose window, a gabber tunnel with a giant
cat face, a lyrics editor whose tags take over the text. His verdict: "an
absolute masterstroke". These scenes bring that craft into Musializer, so a
treatment, or the operator in the window, can make a video like it for any song
and render it deterministically.

The operator asked for the stagings to be "a new scene group with sub scenes".
So each staging is a scene with its own token, tile and settings rows, and the
six share code and a settings shape.

## The six

| Token | Name | The line | The music |
| --- | --- | --- | --- |
| `pixel-boot` | Boot Screen | Drops in as the boot logo and lands with a shine; a long line: first word as the logo, the rest typed under it. No line: `MEOW` | ASCII cat assembles after the logo lands, blinks, goes wide-eyed on a hard kick; logo bounces and `PRESS START` blinks on the beat |
| `pixel-shell` | Root Shell | Typed into `LYRICS.TXT` behind a line number, set large when it fits; earlier lines scroll above | The music writes a tag as a line starts and the tag restyles it: `[BELTED]` (loud) big, red, shaking on kicks; `[WHISPERED]` (quiet) fading; `[FALSETTO]` (bright) tall and wobbling; `[GABBER]` (loud with a kick storm) colour-cycling and bouncing. The cat's paws tap on hats and kicks |
| `pixel-run` | Side Scroller | The level banner over the HUD; the ground's keycaps spell it | The ground runs faster with loudness, keys light on kicks, the gallop swaps frames every half beat, the cat jumps on hi-hats, `+9` rises on each onset and the score counts onsets |
| `pixel-quartet` | Barbershop | Typed onto a marquee; the riser terminal echoes the sung word | Singers bob in pairs on the beat and open their mouths with the vocal, each on its own phase; the bass drum swells on kicks, cymbals jump on hats, barber poles spin with loudness, marquee bulbs chase the beat, notes rise on onsets |
| `pixel-chapel` | Rose Window | Laid on a parchment chant staff, each word under a square note at a pitch hashed from the word; the sung word's note lights | Glass, light beams and every cat's eyes glow with the vocal; the window turns; monks bob when the track is loud; notes drift up on onsets |
| `pixel-rave` | Gabber Tunnel | Runs as a subtitle; the sung word slams in at the top as each word arrives | Rings fly out faster on kicks; the face swells on kicks, sings with the vocal, goes wide-eyed on a hard kick and fires eye lasers on the hardest; words pop on onsets |

Some lines trigger their own gags in the shell. A line that says `sudo` or
`root` is answered `ACCESS GRANTED`. A line that says `logout`, `exit` or
`goodbye` prints a shutdown, puts the cat to sleep and leaves a login prompt
under a sleeping ASCII cat. A line that opens with a tag, such as
`[BELTED] I typed belted`, names its own tag. A tag that contains `KEY` turns the
type cyan and lifts the page.

Every staging follows three rules:

- **The line is the subject.** It types at about 22 characters a second at
  `pace` 1 and never takes more than the cue's first 55 %, so a still from the
  back half of a line shows all of it.
- **Hits move things; they do not flash them.** A kick shakes the frame,
  punches it in and splits the colour. Nothing flashes the whole frame (the
  source video's rule).
- **No line still makes a picture.** The shell shows an ASCII spectrum meter,
  the boot screen drops `MEOW`, the runner keeps running.

When the CRT is on, every pixel scene that is playing at the end of a track
switches off in the last 0.7 s. The picture squashes to a bright bar, the bar
shrinks to a dot, and the last encoded frame is dark.

## Settings: one shape for six scenes

`settings.pixel-<staging>.<name>`, stamped once per staging by the
`pixel_cats!` macro in `scene/settings.rs`, so the six cannot drift. Only the
default palette differs (the boot screen defaults to the handheld's greens).

| # | Name | Range | Default | What it does |
| --- | --- | --- | --- | --- |
| 0 | `palette` | 0–5 | 1 (boot 0) | 0 handheld green, 1 the staging's colour, 2 pocket grey, 3 arcade (black, magenta, cyan, white), 4 virtual red, 5 dusk. The four-shade palettes map the finished frame's luma |
| 1 | `hue` | −180–180 | 0 | Turns the frame, or the four shades |
| 2 | `crt` | 0–1 | 0.6 | Scanlines, aperture grid, neighbour glow and vignette; 0 is a plain nearest-neighbour upscale. Above 0, the set powers off at the end of the track |
| 3 | `response` | 0–2 | 1 | How far the music reaches the picture. At 0 the scene keeps time on a neutral 120 BPM clock and is byte-identical for any audio |
| 4 | `shake` | 0–2 | 1 | Kick shake, punch-in and colour split |
| 5 | `pace` | 0.5–3 | 1 | Typing speed |
| 6 | `pops` | toggle | on | Onset pops in the stagings that have them (rave, runner, quartet, chapel) |
| 7 | `glitch` | 0–1 | 0.5 | Row tears on a section's first frames, a new line, and the last eighth of every fourth bar |
| 8 | `pixels` | 120–320 | 180 | The canvas's short side: smaller is chunkier |
| 9 | `motion` | 0–2 | 1 | Scrolling, turning and idle animation |

`hue` is a circle for Tune's Surprise, like every scene's hue. A voiced line
(MV9-h) gives its voice's hue to the accent colour that marks the sung word.

## How it draws

**The canvas is drawn on the CPU.** A staging draws into a small RGB raster
(`pixel_cats::raster`) with integer placement: rectangles, Bresenham lines,
ellipses and triangles judged at pixel centres, and integer alpha blends. Then
one compositing pass applies shake, punch-in, colour split, glitch rows, the
palette, the hue and the power-off, and writes RGBA bytes. The app uploads
them into the top-left of one 1024x1024 texture, allocated once, and draws it
through `resources/shaders/glsl330/pixel_cats.fs`.

A frame is about 58 000 pixels. Drawing that on the CPU costs less than the
draw calls it would take to put the same picture through raylib, and it gives
three things a GPU path cannot:

- the frame is the same bytes on NVIDIA and on llvmpipe, so the gate's
  rasterizer differences cannot reach it;
- every staging's response to the music is a unit test that counts pixels;
- preview, still, clip and export agree, because each frame is one pure
  function of the state and the frame.

**The canvas size** (`canvas_size`) is the whole-number factor that puts the
short axis nearest `pixels`. With the default of 180, 1280x720, 1920x1080 and a
supersampled 3840x2160 all draw the same 320x180 picture; 1080x1920 draws
180x320, 1080x1080 draws 180x180 and 1080x1350 draws 180x225. The canvas covers
the output to within one factor's worth of pixels on each axis. It is drawn
centred and the remainder is black bezel. The layouts are written against the
canvas's width and height, and they recompose in portrait.

**The shader** picks each output pixel's canvas texel and never filters it.
It darkens the lower part of each canvas row and the right edge of each column,
in proportion to the scale (no grid at 1x, all of it from 4x). It lets the
neighbours' light into those gaps and vignettes. If the driver refuses the
shader, the canvas is drawn plain and the report says `crt=FALLBACK`.

**The state** (`PixelCatsState`) holds followers and events, replayed like every
stateful scene's:

- beats, counted from the tracker's phase wraps;
- kicks, from Clawd's measured detector on the low quarter of the bands;
- hi-hat ticks, from the same detector on the top quarter;
- onsets, and the recent ones that drive pops and jumps;
- a vocal level from the mids;
- track-relative loudness (`dyn=track`, or `none` when there is no profile);
- the scroll distance;
- the lines already sung, with their tags, for the shell.

Drawing never advances it. Random-looking choices hash the seed with track
time, never a frame counter or the wall clock.

## The report line

```text
pixel cats:      staging=shell palette=handheld canvas=320x180x8 line=complete 6 tag=ROOT glyphs=36 cats=1 pops=0 beats=13 kicks=11 ticks=6 onsets=2 kick=0.00 energy=0.12 dyn=track hit=shake(0,0)+split0+zoom1.000 glitch=0 power=on history=0 rows=0 crt=shader cpu=0.84ms
```

Several states look the same in a capture: a staging with no line and one whose
typewriter never started, a floor that should hold cats and an empty one, a
four-shade palette that silently fell back to colour, a CRT pass that was
refused. The line says which state the frame was in. The words after `power=`
are the staging's own evidence (scroll and jump, window and staves, face size
and lasers, history rows, logo state).

## Evidence

- **Core tests (23, `scenes::pixel_cats`).**
  - All six stagings share one settings shape and a `pixel-` token.
  - The canvas rule holds in every shape the export panel offers.
  - A line lands inside the first half of its cue.
  - Every staging draws its line, with cats, and draws a picture without one.
  - Every staging reacts to a synthetic groove and is byte-identical across
    loud and silent audio at `response` 0. That second half is the negative
    control for the first.
  - The same track draws the same bytes.
  - The shell's tags come from the music or from the line, and it keeps a
    history.
  - A four-shade palette leaves only its four shades.
  - The set powers off only with the CRT on.
  - Onsets pop in the four stagings that have pops, and the toggle stops each.
  - Every staging composes in portrait.
- **`tools/pixel_cats_check.py`**, through the app and `headless_run`.
  - Each staging draws its line, moves, and answers the music: response 1
    against 0 on the same moment, MAD 4.97–27.5.
  - `--negative-control` shuts the music out of both renders and the check
    fails, MAD 0.000.
  - Still against the encoded frame: 30.99 dB raw and 39.63 dB after a 2 px
    blur, against 9.75 / 11.47 dB for a neighbouring frame. The raw figure is
    limited by the codec, because one-pixel scanlines do not survive 4:2:0.
  - A second process draws the same still byte for byte.
  - A 9:16 still draws a 180x320 canvas.
- **`tools/scene_gallery_check.py --picker-only`.** All seven families, and
  every Pixel cats tile clicked at the 960x640 / 240 px sidebar minimum.
- **Differential.** `settings_post_legacy.txt` and
  `route_persistence_post_legacy.txt` gain only the 60 new rows, plus
  Threshold's `beyond` maximum moving 22 → 28 so the doorway can show a pixel
  scene. `differential_settings.sh` failed on a perturbed `pace` bound (all six
  rows flagged) and passed once reverted.
- **Speed.** `cargo run --release -p musializer-core --example
  pixel_cats_bench` times update, render and composite for one frame. At
  320x180 the mean is 0.61–0.94 ms, the rave 2.0 ms, and the worst frame
  3.6 ms. At 400x225 the mean is 0.93–1.42 ms and the rave 2.9 ms. The shader
  pass is one textured quad.
- **Live.** `build/pixel-scene/live_check.sh` runs muted release playback of
  the meow track from 0:50: 300 frames per scene at 1280x720, entered with
  `--scene`. Every scene had zero dropped PCM frames and zero output
  underruns, and the canvas took 0.57–1.85 ms of CPU. The first run of the
  runner had one 72 ms stall, under a load average of 24 from parallel
  builds. Two reruns on a quieter machine stayed inside the 16.7 ms frame.

Two songs were staged with all six scenes: the meow track on the source
video's own section map (`meow.treatment.json`, its sheet, and an NVENC render
of the whole track), and *Autoregressive Kitty* from its aligned lyrics. Both
are in `build/pixel-scene/` with the scripts that wrote their treatments.

## Not built

- The source video's progress bar that never reaches 100 %, its pause menu
  flipping on hi-hat ticks, its handheld console with a scene inside it, and its
  music box. The picker shows a family's tiles without scrolling at the minimum
  sidebar up to six scenes, which set the count; these would be the next
  stagings.
- Staging individual lines inside one section. As with Stage, cut one section
  per line (`docs/MUSIC_VIDEO_CLI.md`).
- Scene-level chord colour jumps (the video's C–A♭–E hue thirds). A treatment
  can give each section its own `hue` instead.
