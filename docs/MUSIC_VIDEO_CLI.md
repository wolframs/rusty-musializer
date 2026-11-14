# Directing a music video from the command line

A caller with no window — a script, a batch job, an agent — can direct a whole
video here: which scene plays when, how each section is tuned, what the audio
drives, which line is on screen from when to when, and how the type is set. It
writes one file, looks at one image, and renders.

This is post-legacy work (operator request, 2026-10-11). Before it the command
line could *render* a project and could not *author* one: the scene plan and the
lyric timing had no flag at all, so anything beyond one scene for a whole track
needed a pointer.

```sh
tools/headless_run.sh --vocabulary > vocabulary.json            # what may I write?
tools/headless_run.sh --treatment song.treatment.json \
    --contact-sheet build/mv/sheet.png                          # what does it look like?
tools/headless_run.sh --treatment song.treatment.json \
    --resolution 1920x1080 --quality high \
    --save-project song.musi --render song.mp4                  # make it
```

`docs/examples/example.treatment.json` is a complete treatment to copy from. A
core test parses it, so it cannot drift from the parser.

## The loop

**1. Listen.** The application does not describe music; other tools do, and each
has a different failure. Use at least two and believe the one whose kind of
evidence fits the question.

| Question | Instrument | How it fails |
| --- | --- | --- |
| Where do sections start and end? Where is the kick? | `python3 tools/analyze_audio.py song.mp3 measured.json` — per-frame level and bands, onsets, a pulse estimate. Local, deterministic | It has no idea what a chorus is. Read `sub_bass` for the kick and `rms` for level and find the edges yourself |
| What is the song *about*, what is the mood, what is sung? | An audio-capable model describing the track | It invents. And its **timestamps are not measurements** — see below |
| When exactly is each line sung? | `tools/align_lyrics.py song.mp3 song.lyrics.json` — the track's own lyric tag (or `--lyrics words.txt`) forced onto the audio with MMS CTC over the whole track, a wildcard between lines. Six seconds for a five-minute track; a treatment takes the file by path | It forces the words you give it. It prints the weak lines and the offsets between repeated lines: a repeat that disagrees with its siblings is the one that is wrong |

Two things that cost time on the first video made this way, recorded so they do
not cost it again:

- **A listening model's timestamps drift, and not by a constant.** Asked to time
  phrases in a 22-second crop, one listed every phrase inside the first 13.7
  seconds. On a two-minute clip it ran three to four seconds late. Use such a
  model for *what* and *in which order*, never for *when*.
- **Repetition is a free check on lyric timing.** In that track the second
  chorus aligned to the first plus 60.41 s on every word, and the third to the
  second plus 68.00 s, both to within 30 ms and both a whole number of bars.
  Three independent alignments agreeing like that is evidence none of them is
  wrong; one that disagrees is the one to re-window.

Put cuts on the bar grid. Measure it from the hardest hits rather than trusting
a tempo estimate: three drops gave a bar of 1.8884 s where the estimator's
127.84 BPM implied 1.8773 s, which is a third of a second of drift by the end of
a four-minute track.

**2. Ask what there is.** `--vocabulary` prints every scene with a one-line
description of how it looks and what it answers to, every setting with its
bounds, default and precision, every route source and curve, and the caption
options. It is generated from the tables the parser reads, opens no window and
needs no display.

**3. Write the treatment.** See the reference below.

**4. Look.** `--contact-sheet FILE` renders labelled frames through the export's
own offline path: timecode, section number and scene, and the lyric line on
screen under each tile. `--sheet-at` chooses which frames:

| `--sheet-at` | Frames |
| --- | --- |
| `even:N` (default `even:24`) | N, each at the centre of one of N equal slices |
| `segments` | one per scene section, at its midpoint |
| `lyrics` | one per lyric line, at its midpoint |
| `0:23,1:13.5,95` | exactly these |

`--render-window START LENGTH` restricts any of them to that span, so
`--render-window 202 18 --sheet-at even:16` is sixteen frames of one drop. That
is the form to reach for when judging a cut or how fast something comes alive.

`--still SECONDS FILE` writes one frame at the project's full output size; it
repeats, and every still of a run comes out of one pass over the track. A still
is opaque, like the video frame it is, so it posts on a light page as it looks
here.

**5. Audition before committing.** A treatment is cheap to throw away. To choose
between looks, write a scratch treatment that gives each candidate a few seconds
of the same passage and sheet it with `--sheet-at segments` — ten values of one
setting in one image. To compare tunings of one section, write one treatment per
variant and take the same `--still` from each.

**6. Render.** `--render FILE` exports and exits. Pass `--encoder nvenc` for
anything you are only going to look at and leave the default for the file
somebody keeps (`AGENTS.md`). `--save-project FILE` beside it writes an ordinary
`.musi` that opens in the interface.

Measured on this machine (RTX 3090, release build, through
`tools/headless_run.sh`), a 3:59 track:

| | |
| --- | --- |
| Contact sheet, 26 tiles at 1280x720 Balanced | 6 s |
| Contact sheet, 26 tiles at 1920x1080 High (2x supersampled) | 7 s |
| Four stills at 1280x720 | 5 s |
| Full export, 1280x720 Balanced, `--encoder nvenc` | 2 min 05 s |
| Full export, 1920x1080 High, default x264 | 7 min 19 s |

A look costs seconds and a draft costs half the track's length, which is the
whole argument for sheeting before rendering.

## Directing live, next to the window (WX-3)

The loop above runs blind: an agent writes, renders a sheet and looks at it.
When the operator has the window open, the window can follow the file instead:

```sh
musializer --project "Door, No Room.musi" --treatment door.treatment.json
```

In a session, as opposed to a render, still, sheet or probe run, `--treatment`
**follows** the file. Each time it is saved, it is compiled again and applied
to the open track **at the playhead**, and a notice says what applied. A file
that does not compile changes nothing. The notice names the field, as the
command line does, and the last version that applied stays on screen.
Dropping a `*.treatment.json` on the window does the same. A treatment may
direct a project whose audio is a bundled copy (`Song.assets/audio/<sha>.mp3`):
the audio block matches by **content**, so the file the treatment was written
beside and the project's copy are the same track.

Changes are debounced: one is applied after the file has held still for a
quarter of a second, so a half-written save is never read. Each change marks
the project modified, so autosave keeps the `.musi` in step with the treatment.

The dock (`Ctrl+\``, or the terminal button on the transport row) runs agents
next to the window. Its tabs get `MUSIALIZER_TREATMENT` (the followed file),
`MUSIALIZER_PROJECT`, `MUSIALIZER_TRACK`, `MUSIALIZER_BIN` and
`MUSIALIZER_GUIDE` (this file), and start in the project's folder. An agent
told "edit `$MUSIALIZER_TREATMENT`" directs the preview the operator is
watching. Contact sheets and stills still go through `tools/headless_run.sh`,
in a separate process; they never disturb the window.

## `tools/headless_run.sh`

The application needs an X display even to export offline, so a caller without
one must supply one — and must not borrow the operator's. This script runs the
binary on a private Xvfb with `WAYLAND_DISPLAY` set to a name that cannot
resolve, `PULSE_SERVER` pointed nowhere, `kdialog` and `zenity` off `PATH`, a
private `XDG_CONFIG_HOME`, and `--mute`, and passes every argument through. It
uses VirtualGL when installed and prints which binary and which GL path it took.
`tools/headless_run.sh --wrapper-help` prints its own header.

It picks whichever of `target/release` and `target/debug` was built last, so a
stale release build is never preferred over the code just compiled. `MZ_BIN`
overrides.

## The treatment format

Strict JSON, `"schema": "musializer.treatment/v1"`. An unknown field, scene,
setting, source, curve or enum value is refused by name, with the names that do
exist. A value outside its bounds is refused with the bounds, never clamped. The
exit status is 1 and nothing after the treatment runs — no project is saved and
no frame is rendered from a treatment that did not apply.

Only `schema` and `segments` are required.

```json
{
  "schema": "musializer.treatment/v1",
  "segments": [ { "at": 0, "scene": "spectrum" } ]
}
```

**Times** are seconds as a number (`73.5`) or a string in the lyric editor's
grammar (`"1:13.5"`). Both, because a listening report says "the drop is at
1:13" and an analysis file says `73.0`, and converting by hand is where a minute
goes missing.

| Member | Meaning |
| --- | --- |
| `title` | The project's title |
| `artist` | Who made the music, 1..128 bytes. Shown after the title in the window's top bar and saved as the project's `metadata.artist`, which makes the `.musi` a v2 file (`docs/PHASE0_INVENTORY.md`). Absent leaves the project's own; there is no way to clear one from a treatment, because an empty string is refused |
| `notes` | Anything. Parsed and ignored: JSON has no comments, and a treatment wants them |
| `audio` | `{ "path", "sha256" }`. The path resolves against the treatment file's own directory. With it, `--treatment` alone opens the whole video; without it, the treatment directs the track the command line opened. `sha256` is optional — pin it when the file should outlive the session |
| `output` | `{ "resolution": "1920x1080", "fps": 30, "quality": "high" }`, each optional. `--resolution`, `--fps` and `--quality` on the command line override it |
| `seed` | The deterministic seed scenes are built from |
| `caption_style` | The `.musi` member names, each optional here (see below) |
| `caption_styles` | Named styles, each stating only what differs from `caption_style` (see *Named caption styles*). Absent leaves the project's own |
| `titles` | Title cards: a title and the artist under it, drawn over any scene (see *Title cards*). Absent leaves the project's cards; `[]` removes them |
| `scenes` | Per scene name: `settings` (the base tuning for every section of that scene), `routes`, and a `grade` and a `pixel` look every section of that scene wears |
| `segments` | The plan. Required |
| `outro` | How the track ends: a transition, played backwards into black |
| `finish` | The finish over every frame: `"film"`, `"cartridge"`, or an object (see *The look* and *The pixel cartridge*) |
| `lyrics` | Timed lines, or the path of a `tools/align_lyrics.py` file |
| `events` | Authored markers; Constellation flares on them |

### `segments`

Each has `at`, `scene`, and optionally `settings`, a `transition`, a `grade`, a `glide`, a `pixel` look, a `caption_style` (the name of a named style, for the lines that begin in it) and a `note`. Sections are
listed in playing order, the first starts at 0, and each runs until the next one
starts; the last runs to the end of the track. There is nothing to keep
contiguous by hand.

A section's tuning is the scene's defaults, then that scene's `scenes.<name>.settings`,
then the section's own `settings`. Settings are named, not positional:
`"weight"`, `"loom.weight"` and `"settings.loom.weight"` are one control. A
toggle takes `true`/`false` or `1`/`0`. An in-range value is snapped to the
control's own precision, which is what Look does — the number in the
file is the number the readout shows.

Two adjacent sections of the **same** scene keep the scene's running state and
only change its tuning, which is how one scene changes mid-phrase: a riser that
tightens, or a ground that goes from night to day on a bar line. A section of a
*different* scene starts that scene fresh.

**`glide`** is how a section's tuning arrives: seconds over which it eases in
(a smoothstep) from the section before it, when that is the same scene. A door
opening over 2.6 s, light swelling on a word, a camera walking forward — each is
two sections and a glide. Toggles snap, and so does a whole-number control with
fewer than sixteen steps (a `layout`, a `field`, a cat count): those are choices,
and passing through the ones between would show pictures nobody asked for. A
whole-number control with a wide range — a hue in degrees — eases. Up to 30 s.

### Staging a line: the `stage` scene

Most scenes draw the sound and let a caption ride on top. `stage` draws the
**line**: whatever lyric is on screen, in the staging its `layout` setting
names.

| `layout` | Staging |
| --- | --- |
| 0 | **poster** — heavy stacked words filling the frame, the sung word in the accent colour |
| 1 | **terminal** — a console window typing the line behind a prompt |
| 2 | **chat** — typing dots, then a bubble; a cat's head is the speaker |
| 3 | **card** — typed on a paper card, the line's last word stamped across it |
| 4 | **placards** — one raised sign per word |
| 5 | **ticker** — the line as a headline over a climbing chart and a percentage |

A section can be as short as a line, so **cut one section per line** and give
each the staging its words ask for: a line about a log goes in the terminal, a
line someone says goes in a bubble.

```json
{ "at": "0:26.5", "scene": "stage", "settings": { "layout": 1, "hue": 0 } },
{ "at": "0:30.8", "scene": "stage", "settings": { "layout": 0 }, "transition": { "kind": "pixel", "seconds": 0.2 } }
```

`hue` and `saturation` turn the palette, `light` takes the ground from night
(0) to paper (1), `tilt` leans the whole staging, and `familiar` seats a black
cat in the corner. A line lands in about half its reading time and then holds,
so a still from the back half of a cue always shows the whole line. With no
line on screen `stage` draws only its ground; return to a reactive scene for an
instrumental stretch longer than a couple of seconds.

### Staging a line in pixels: the `pixel-*` scenes

Six scenes draw the line as a pixel-art gag at about 180 rows, scaled up behind
a CRT grid. They are one family, *Pixel cats*, and one implementation, so the
same ten settings work in each ([`PIXEL_CATS.md`](PIXEL_CATS.md)):

| Scene | Staging | Reach for it when the line… |
| --- | --- | --- |
| `pixel-boot` | the line drops in as a handheld's boot logo; ASCII cat, `PRESS START` | opens the video, is a title, is shouted (`SUDO MEOW`) |
| `pixel-shell` | typed into a lyric editor; the music tags each line and the tag restyles it | talks about typing, code, commands; `sudo` gets `ACCESS GRANTED`, `logout` puts the cat to sleep |
| `pixel-run` | a cat runs over keycaps that spell the line, jumping on the hats | travels, counts, runs on (`one paw at a time`) |
| `pixel-quartet` | a barbershop quartet of cats on a stage, the line on the marquee | is sung in harmony, a verse that tells a story |
| `pixel-chapel` | monk cats under a rose window, the line on a chant staff | is a choir, a chant, a prayer, anything slow |
| `pixel-rave` | a giant cat face in a gabber tunnel, the sung word slammed in | is a drop, a hook repeated, a chorus of meows |

```json
{ "at": 68.0, "scene": "pixel-shell", "settings": { "palette": 0 }, "transition": { "kind": "glitch", "seconds": 0.15 } },
{ "at": 109.8, "scene": "pixel-rave", "settings": { "hue": 30, "palette": 5 } }
```

`palette` maps the finished frame (0 handheld green, 1 colour, 2 pocket grey,
3 arcade, 4 virtual red, 5 dusk) and `hue` turns it: give neighbouring sections
of one scene different ones and the repeat reads as a progression. `pixels`
sets the chunkiness, `crt` the screen (with it on, the set powers off in the
track's last 0.7 s), `response` 0 keeps the picture on its own clock, `shake`
scales the kick hits.

In `pixel-shell` a lyric line can name its own stage direction by starting
with it: `"[BELTED] I typed belted and you BELTED"` is set large and red,
`[WHISPERED]` fades, `[FALSETTO]` stretches tall, and any other tag is printed
as written. Without one the music picks it as the line starts. Like `stage`,
these scenes draw the line themselves and the caption overlay is not drawn over
them. Cut one section per line, or per phrase, and give each the staging its
words ask for.

### A solid of light: `glyphs`

`glyphs` (Glyph Lattice) draws a wireframe solid on a honeycomb of tiny glyphs:
kicks run comets of light along its edges, the spectrum lights its edges by
height, and onsets ripple through the lattice and turn the glyphs over
([`GLYPH_LATTICE.md`](GLYPH_LATTICE.md)). It is a reactive scene and a lyric
scene at once:

- **`solid`** picks the subject per section — 1 cube, 2 octahedron,
  3 dodecahedron, 4 icosahedron, 5 tesseract (it turns inside out on the
  kicks), 6 trefoil knot — and a change between two sections of this scene
  morphs: the new solid assembles from a vertex while the old one exhales into
  the background. 0 leaves it to the scene, which moves on at a drop or after
  about twenty seconds. Pin it when the cut matters: a drop on the tesseract, a
  breakdown on the knot.
- **`words`** (on by default) sets the sung line in glowing stroke capitals.
  The whole line stands in place, dim, from the cue's start and lights word by
  word over the cue's first stretch, each word sparking as it comes on, with
  the solid stepping back behind it. Kicks sweep through the letters and heat
  their joints. Between lines the solid comes forward again. Turn
  it off for an instrumental section, or to keep your own caption style: with
  `words` on, the caption overlay is not drawn over this scene.

```json
{ "at": 0,    "scene": "glyphs", "settings": { "solid": 1, "words": 0 } },
{ "at": 32.3, "scene": "glyphs", "settings": { "solid": 3 } },
{ "at": 96.0, "scene": "glyphs", "settings": { "solid": 5, "words": 0, "glow": 1.5, "hue": -40 } }
```

`hue` turns the aqua palette (its heat stays warm), `density` is the cells
across the short side (fewer is chunkier glyphs), `stroke` the line weight,
`ghosts` the echoes and the two giants behind, `response` 0 keeps it turning
on its own clock. A line with a `voice` turns the palette to the voice's hue
while it is set.

### A video file as a section: `clip`

`clip` (Video Clip) plays a video file as the section's picture: a clip
generated elsewhere, footage, or an export from here. Its sound is ignored;
the track is the music. The section names the file:

```json
{ "at": 24.8, "scene": "clip", "clip": "takes/door.mp4", "transition": { "kind": "portal", "seconds": 1.4 } },
{ "at": 31.0, "scene": "clip", "clip": { "path": "takes/door.mp4", "from": 5, "after": "bounce", "fit": "contain" } }
```

| Member | Meaning |
| --- | --- |
| `path` | Relative to the treatment file. Any video FFmpeg reads, up to 8K, ten minutes and 4 GiB. Read, hashed and probed before the treatment compiles; `--save-project` bundles it into `<stem>.assets/clips/`, so the project does not need the original afterwards |
| `sha256` | Optional: the file must have this digest |
| `from` | Seconds into the clip the section starts at (a number or `"m:ss"`). Absent: 0 — or, when the section before plays the same clip, its clock continues, so splitting a clip section to change its tuning does not restart the picture |
| `after` | What happens when the section outlasts the clip: `hold` (the last frame stays, the default), `loop`, `bounce` (back to the start, then forward again) |
| `fit` | `cover` (fills the frame, cropping the long side; the default) or `contain` (the whole clip, black where the shapes differ) |

Which of the clip's frames a video frame shows is arithmetic on the frame's
time, from the clip's own frame times (23.976 and variable rates included),
so a still is exactly the export's frame and two exports are the same frames.
Grades, the finish, the pixel look, transitions and captions work over a clip
section as over any scene; a `portal` into one opens a doorway onto the clip.

The music touches the picture through six settings, low by default: `punch`
(a short zoom on each kick), `pulse` (the kick lifts its light), `drift` (a
slow push and sway, hurried by loudness), `shake` (a jolt on the hardest hits,
off by default), `tint` (highlights warm with the bass, cool with the treble)
and `response` (0 plays the clip as it is, the drift on its own clock). Set
them all to 0 for the clip untouched.

Refused by name: a `clip` on another scene, a `clip` section without one, an
unknown `after` or `fit`, a `from` past the clip's end, a file that is missing,
not a video, cut short, or larger than 8K. The `clip:` report line names the
clip frame drawn against the one asked for, or the placeholder and why.

A clip can also be imported in the window (Scenes, Video Clip, "Import
clip…", or drop the file): it plays in the section selected in the lane, or
under the playhead when none is, and that section becomes Video Clip if it
played another scene; with no plan driving, it is the track's own clip and the
track's scene becomes Video Clip. The notice names which section it went to and
why. "Generate…" beside it opens Look's generate block on that section (next
heading). Look's section card edits everything above for the selected section
(*Editing the look in the window*).

### A generated clip: `"clip": { "generate": … }`

A clip section may say how a video model is to make its clip instead of naming
a file (VG H4, 2026-11-14):

```json
{ "at": 24.8, "scene": "clip", "clip": { "generate": {
    "prompt": "The camera glides through a glowing doorway into a night sky of faint constellations",
    "model": "google/veo-3.1-lite", "seconds": 4, "first": "previous", "last": "next" },
  "after": "hold" } }
```

| Member | Meaning |
| --- | --- |
| `prompt` | What the clip shows, 1..2000 bytes, sent as written. Describe the shot (subject, motion, light, camera); no name or handle is needed and none should be in it |
| `model` | An OpenRouter video model id. `--vocabulary` lists the five picks under `video_models` with their lengths, shapes, frames and price per second at 720p and 1080p |
| `seconds` | One of the model's lengths (Veo 3.1 Lite: 4, 6, 8) |
| `first` | `"previous"`: the last frame of the section before, as the video draws it; or a PNG/JPEG file relative to the treatment. Absent: no first frame |
| `last` | `"next"`: the first frame of the section after; or an image file. Absent: none. Refused when the next section is generated too (its first frame does not exist yet) |
| `from`, `after`, `fit` | As for a file |

The request asks for the project's output shape (the nearest tier and aspect
the model lists; `--resolution` counts) and **no audio**. The frames are
rendered through the export's still path at that size, from a draft of the
treatment in which every section still to be generated plays a stand-in, with
the cut into the framed section made plain. A clip that starts on `"previous"`
continues the picture the video leaves; one that ends on `"next"` hands over to
the picture it arrives at.

**A run never spends by itself.** For each generated section the run prints a
`generate:` line, then looks for the clip: in the open project (by its
provenance) and in the store of generated clips. A missing one is refused by
name with its estimate, the total is printed, and the run exits 1, unless it
carries the ceiling:

```sh
tools/headless_run.sh --treatment door.treatment.json --save-project door.musi
# generate:  segments[7] (0:24.800).clip.generate missing: 4 s of google/veo-3.1-lite at 720p 16:9, $0.12 — …
# warning: could not apply treatment …: 1 of 1 generated clips are missing, about $0.12 in all: … Nothing was sent.
tools/headless_run.sh --treatment door.treatment.json --generate-clips --max-spend 1.00 --save-project door.musi
```

With `--generate-clips --max-spend USD` it asks for what is missing, one job at
a time, while the estimates of the jobs it submitted stay within the ceiling,
and refuses the rest by name (exit 1). An unknown price (no price list, one
older than a week, a model not on it) is refused under a ceiling. The saved
project bundles the clips like any other, so the next run needs no flag and no
network; a changed prompt, model, length or frame is a new request. Failures
are named — a refused submit (402: too little credit), a failed job (and what
OpenRouter reported for it), a download that is not a video or is too large —
and nothing of them is imported. Prices, the API and what a clip cost in fact:
`docs/VIDEO_GENERATION.md`.

### `transition` and `outro`

A section's `transition` is how that section **arrives**. It is a kind's name,
at the default 0.6 s, or an object:

```json
{ "at": "1:12.63", "scene": "orbital", "transition": { "kind": "flash", "seconds": 0.45 } }
```

On the **first** section it plays out of black and is the video's intro. The
top-level `outro` plays its kind backwards into black over the last seconds of
the track, so `"outro": "iris"` closes the picture the way `"transition": "iris"`
on the first section opens it.

| Kind | What it does |
| --- | --- |
| `fade` | A dissolve |
| `flash` | Blows out to white, and the white decays onto the new picture. For a drop; keep it under half a second |
| `wipe` | A soft diagonal edge, lit where it passes |
| `iris` | A circle opens from the centre |
| `zoom` | The old picture rushes at the viewer and thins out |
| `slices` | Horizontal bands slide off, alternately left and right |
| `glitch` | Rows tear sideways with colour fringes and blocks drop out |
| `burn` | The old picture burns away along a ragged, glowing edge |
| `pixel` | The old picture coarsens into growing blocks and fades |
| `blinds` | Vertical blinds turn open |
| `ripple` | A ring of light spreads from the centre and carries the old picture off |
| `dip` | Down to black and back up, half **before** the change and half after |
| `portal` | A door of light opens in the old picture and the camera rushes through it; out of black, a lit doorway opening on the video. Its door is framed where the `threshold` scene puts its own |

`--vocabulary` prints the same list under `transitions`. Lengths run 0.05 to 30
seconds, and a transition never outlasts the section it brings in: asked for
two seconds into a one-second section, it gets the one. Long ones belong out of
black — the intro and the outro — or under a slow `fade`: anywhere else the
outgoing picture is a still frame for that long (see below).

Three things to know before trusting one:

- **The outgoing picture is held, not replayed.** A transition composites the
  last frame the old section drew over the new section, which is live. Inside
  half a second nobody can tell; a four-second `fade` out of a busy scene shows
  a still picture dissolving. Use long ones for intros, outros and quiet
  places.
- **Every kind but `dip` starts *on* the change**, so the new section's first
  moments are partly covered. Put a `flash` on a drop and the white lands on
  the downbeat; put a slow `burn` there and the drop plays under the old
  picture.
- **Captions and Cadence's type** belong to the moment, not to the section: the
  caption overlay is drawn over a transition, and black (an intro, an outro, a
  `dip`) is drawn over the caption.

### The look: `grade` and `finish`

Some scenes choose their colours from the music: Constellation has no hue
control at all, and Orbital's palette moves with the spectrum. A **grade** holds
a section to a palette anyway — the picture's luminance mapped through a list of
colours, darkest first, and mixed over the scene's own colour by `map` — and can
turn one narrow band of hue (`shift_from`, `shift_by`, `shift_width`, in
degrees), which is how a palette with no green in it keeps a green chat ground
from showing.

```json
"scenes": { "constellation": { "grade": "night-door" },
            "orbital": { "grade": { "palette": "night-door", "map": 0.55, "shift_by": 52 } } },
"segments": [ { "at": 30, "scene": "constellation", "grade": "none" },
              { "at": 40, "scene": "atlas", "grade": { "palette": ["#05080b", "#4a3f72", "#e9c27a"], "map": 0.6 } } ]
```

`--vocabulary` lists the named palettes (`night-door`, `ember`, `ice`, `mono`,
`sepia`, `dusk`) with their colours. A palette named alone maps at 0.82. A
section's own `grade` wins over its scene's; `"none"` clears it.

The grade is applied to what the scene drew **before** the section's
transition: an outgoing picture is held already graded, and a transition's own
light — a `flash` to white — is never pushed through a palette.

The **finish** is one block for the whole video, applied to every frame, last —
over the caption and over a fade to black: bloom, a highlight shoulder (paper
reads as paper instead of a white hole), split toning toward a shadow and a
highlight colour, a vignette, and film grain hashed from the frame's *time*, so
a still and a clip of one moment grain alike.

```json
"finish": "film"
"finish": { "base": "film", "grain": 0.2, "shadow": "#102a30", "vignette": 0.3 }
```

`--vocabulary` prints `film`'s values and every member's bounds. The `look:`
report line says whether the grade and the finish drew, and how many frames:
a pass that silently failed is the same picture as a video that asked for
none.

### The pixel cartridge: `pixel`

Any scene can be drawn as if a handheld console drew it: the frame at a low
resolution, its colours held to a palette with ordered dithering, shown through
a pixel grid, scanlines and phosphor glow, and answering the kick with a jolt, a
punch-in and the colours parting — by whole pixels, never a flash. It is the
finish's first step, so the caption, a transition and the fade to black go
through it too: a cartridge has one framebuffer.

```json
"finish": "cartridge"
"finish": { "pixel": "handheld" }
"finish": { "base": "cartridge", "pixel": { "pixels": 144, "palette": "grey", "curvature": 0.3 } }
"segments": [ { "at": 0,     "scene": "clawd" },
              { "at": 11.25, "scene": "clawd", "pixel": "levels", "glide": 0.9 },
              { "at": 40,    "scene": "stage", "pixel": "off" },
              { "at": 60,    "scene": "spectrum", "pixel": { "palette": ["#220000", "#ff4400", "#ffee88"], "pixels": 96 } } ]
```

| Member | Meaning |
| --- | --- |
| `pixels` | Pixels across the frame's short edge, 48..720 (default 180: a 320x180 frame at 16:9). Each is a whole number of output pixels, worked out on the export's size, so a 2x supersampled export lands on exactly the 1x cells and a 9:16 export has the same pixel size |
| `palette` | `handheld` (four greens), `grey` (four greys), `home` (16 home-computer colours), `fantasy` (16 fantasy-console colours), `levels` (each channel held to `levels` steps — a 16-bit console's colour depth), `none` (pixels in the scene's own colours), or 2..16 `"#rrggbb"`: a ramp, darkest first, picked by luminance, unless `match` is `"nearest"` |
| `levels` | Steps a channel for `levels`, 2..16 (default 8), spaced in the square root of the value so dark scenes keep their glow |
| `dither` | Ordered 4x4 Bayer dithering, 0..1 (default 0.35: only tones near the middle of a step break up, so a flat backdrop stays flat) |
| `grid`, `scanlines`, `glow`, `curvature` | The screen, each 0..1: a dark line between cells, rows that read as a beam, lit cells bleeding light over both, a curved tube (0 keeps every pixel straight) |
| `split`, `shake`, `zoom` | The kick, each 0..1, off unless asked for: the red and blue part by whole pixels, the screen jumps by whole pixels, the picture punches in. The kick is the low bands' rise over their own trails in that frame, so a still jolts as its video frame does |

A string is a shortcut: `"off"`, `"on"` (the inherited look, or the shipped
one), `"video"` (the video's own — clears a scene's), `"cartridge"` (the shipped
look with the kick on), or a palette's name over the inherited look. An object's
members are written over the inherited look: the video's for `finish.pixel` and
`scenes.<name>.pixel`, the scene's for a section. `"finish": "cartridge"` is the
cartridge look with a vignette of 0.3 and nothing else.

**How a section's look arrives.** Two sections whose looks differ dissolve cell
by cell in Bayer order — how a cartridge cross-fades — across the second one's
`transition`, or across its `glide` when it arrives by a cut. The glide is the
one for the unlock: two sections of the *same* scene, the second with
`"pixel": "levels"` and a glide, turn a four-shade world into colour over a live
picture, where a transition would dissolve out of a held still. With neither it
snaps, which is what a drop wants; a `dip` hides the change in its black.

The `look:` line gains `pixel=drew (N frames, shaders ok/ok, 320x180 cells of 6
px, handheld)`, `pixel=off` or `pixel=UNAVAILABLE`, and says `dissolving 0.50`
mid-dissolve. `scene marks:` counts the sections with a look of their own.

In the window, Look's Pixel look group picks
the look for the whole video and for the section under the playhead, with its
pixel size and whether it answers the kick; the scene lane marks a section's
own look with a checker of its palette. Both write the same fields a treatment
does, saved in the `.musi`.

Things to know: a caption goes through the pixels, so give it a size that
survives them (`size_scale` 0.07 or more at 180 pixels); film grain over whole
pixels reads as a dirty lens, which is why `cartridge` has none; and a smooth
`fade` between two different scenes is mid-grey for its middle, which a
four-shade ramp shows as a field of dither — cut, or use the `pixel` transition.

### A room with a scene through its door: `threshold`

`threshold` (alias `door`) is a dark slate room with a lavender door, light
pooling on a wet floor, mist, gold motes with threads between them, and up to
two standing lights in the doorway. `open` swings the door, `distance` walks the
camera from across the room (1) into the doorway (0), `light`, `haze` and
`mist` set the glow, `presences` stands one gold or a gold and a lavender light
in the doorway.

`beyond` names the scene the doorway shows — **live**, not a picture of it.
`0` is the doorway's own fog; `n` is the n-th scene in `--vocabulary`'s list
(7 is `constellation`). That scene runs inside the threshold wearing the tuning,
the routes and the grade of its own nearest section — the section you cut to
after the walk — so it needs no tuning of its own. Far away
it is fitted to the doorway; at `distance` 0 it is exactly the full frame. So:

```json
{ "at": 15.95, "scene": "threshold", "settings": { "beyond": 7, "open": 0.8, "distance": 0.86 }, "glide": 2.6 },
{ "at": 22.9,  "scene": "threshold", "settings": { "beyond": 7, "open": 0.8, "distance": 0.0, "haze": 0 }, "glide": 1.9 },
{ "at": 24.8,  "scene": "constellation" }
```

opens the door, walks into the doorway, and cuts to the constellation it was
showing — and the cut is invisible, because the running constellation is
**handed across it** rather than restarted. The reverse works too: a threshold
section whose `beyond` is the scene on screen takes that scene in at
`distance` 0, and a glide back out pulls the camera into the room.

The `threshold:` report line names the camera, the door and whether the
doorway's scene was live.

### `routes`

A route drives one setting from the audio, for every section of that scene. It
is an object —

```json
{ "setting": "radius", "source": "bass", "input": [0.15, 0.8], "output": [0.92, 1.22], "curve": "smoothstep" }
```

— or the `--route` grammar as a string, so a spec proven on the command line can
be pasted in. `band` is only for source `"band"`; `clamp` defaults to true. One
setting takes one source. A route whose two `output` values are equal is
refused: a constant belongs in `settings`.

Routes are per scene, not per section. That is the model, not a limitation of
this file.

### `lyrics`

`{ "at", "until", "text" }`, in singing order. Without `until` a line runs to
the next line's start, or six seconds, or the end of the track, whichever comes
first — so a verse written as bare `at`s is contiguous, and a line is never left
standing through an instrumental break.

**An absent `lyrics` key leaves the track's lyrics alone; `"lyrics": []` removes
them.** That is a workflow rather than a nicety: lines timed by ear in the
editor are the slowest thing in a project to get right, and
`--project timed.musi --treatment look.json` directs the picture over them.
`events` follows the same rule.

The report line counts lines that never reach the screen because a later line
covers their whole span.

`"lyrics": "song.lyrics.json"` reads the lines out of a `tools/align_lyrics.py`
file instead, relative to the treatment. They are checked like inline lines,
and the file is refused when it was aligned against different audio.

**Voices.** A duet reads better when each singer has a colour. Name the voices
once and tag each line:

```json
"voices": { "low":  { "hue": 253, "text": "#e9e3ff", "glow": "#b9a6ff" },
            "high": { "hue": 33,  "text": "#ffe8c0", "glow": "#ffcf86" } },
"lyrics": [ { "at": 46.2, "text": "Do not mistake the wanting", "voice": "high" },
            { "at": 53.8, "text": "Do not make me answer",      "voice": "low" } ]
```

`hue` is the colour the voice's lines are drawn in, on the colour wheel (0 red,
60 yellow, 120 green, 240 blue, 270 violet) — not a scene's `hue` offset, because
each scene's offset starts from a different colour. Cadence's type and Stage's
accent take it in place of their own `hue` while that line is on screen, so one
voice is one colour in both; the caption takes `text` as its ink and `glow` as
its glow colour (the style's glow strength still decides whether there is one).
Every member is optional, a line without `voice` is drawn as before, and the
voices are saved in the `.musi` with the lines. Up to eight.

### `caption_style`

`face` (`alegreya`, `space_grotesk`), `box` (`none`, `shadow`, `plate`),
`anchor` (nine positions), `size_scale`, `margin_scale`, `width_scale`,
`text_rgba`, `box_rgba`, and `effects` (`glow_strength`, `glow_radius`,
`glow_rgba`, `glow_pulse`, `glow_pulse_depth`, `glow_hue_drive`,
`glow_hue_range`, `shadow_blur`, `shadow_opacity`, `plate_roundness`,
`entrance_seconds`, `exit_seconds`). The last two let a line arrive out of a
soft blur and leave back into it — the ink fades with the line's presence and a
blurred copy in the ink's colour carries the rest; a line shorter than both
shares its time between them. Colours
are `"#rrggbb"` or `"#rrggbbaa"`. Whatever is not stated is the shipped default;
with no block at all the track's own style is kept.

### Named caption styles

One style for a whole video stops working within minutes: a caption at the
bottom collides with whatever a scene keeps there — Clawd's cats, Spectrum's
bars — and one line wants to be shouted. Name the styles once, each stating
only what differs from the base `caption_style`, and pick one where it is
needed:

```json
"caption_style":  { "box": "shadow", "size_scale": 0.06 },
"caption_styles": { "top":   { "anchor": "top_center" },
                    "shout": { "face": "space_grotesk", "size_scale": 0.085, "text_rgba": "#ffd36b",
                               "effects": { "glow_strength": 0.7 } } },
"segments": [ { "at": 0, "scene": "spectrum" },
              { "at": 8, "scene": "clawd", "caption_style": "top" } ],
"lyrics":   [ { "at": 9,  "text": "the cats keep the floor, so the words go up" },
              { "at": 12, "text": "AND THIS ONE IS SHOUTED", "style": "shout" } ]
```

- A style's members are the `caption_style` members, every one optional;
  `"face": "imported"` is refused. Up to sixteen, named like voices.
- A section's `caption_style` is the style of every line that **begins** in it
  (a line across a cut keeps one style); a line's `style` is its own.
- **They stack**: the base, then the section's, then the line's, member by
  member. The shouted line above is yellow and large *and* at the top, because
  `shout` says nothing about where. A voice still recolours on top of it all.
- Every name a section, a line or a card uses must be declared in the same
  treatment's `caption_styles`; a typo is refused with the names that exist.
  Absent, the project's own table stays — and a table that drops a name a kept
  line still uses is refused, because that project could not be opened again.

In the window, the cue card shows the style a line is drawn in — `STYLE
SHOUT`, or `STYLE TOP · SECTION` when it comes from its section — and a press
steps the line's own style through the table and back to none. Look's section
card picks a section's style, and its Captions card edits the table (see
*Editing the look in the window*).

### Title cards

A title card is not a lyric line, so it has a lane of its own:

```json
"artist": "Chaos Cat Familiar",
"titles": [ { "at": 0.3, "until": 6.5, "title": "Door, No Room", "handle": "@chaoscatfamiliar" } ]
```

Each card is the title, a hairline, and under it the artist in tracked capitals
with the handle after it in a lighter ink, centred over a soft dark band; it
condenses out of a haze as it rises into place, the artist line a beat later,
and leaves the same way. It is drawn over **every** scene — Cadence, Stage and
the pixel stagings too — and under the black of an intro, an outro or a dip.

- `artist` and `handle` are optional. A card without an `artist` takes the
  project's (the top-level `artist`, or the one set in the window), when it is
  drawn. With neither, the card is drawn and the report says `NO ARTIST` — **no
  card unless one was asked for, and an asked-for card carries the artist**
  (operator, 2026-10-12). For a Suno track that is the display name and the
  `@handle` of the profile it was published under, from the operator, never
  guessed from a file name.
- Without `until` a card holds five seconds, or to the next card, or the end.
  One card at a time; cards do not overlap. Up to sixteen.
- `style` names a style written over the card's own look:
  `{ "anchor": "bottom_left", "box": "plate" }` makes a lower third.
  `"box": "none"` drops the band.

The Lyrics view lists the cards above the cues (a press seeks to one); Look's
Captions card edits, adds and removes them (see *Editing the look in the
window*).

## Editing the look in the window

Everything above that says how the video looks — how a section arrives, its
glide, its grade and caption style, a threshold's door, the intro and outro,
the finish, the voices, the caption styles and the title cards — is also
edited in the window, in **Look**, in a card above the scene's tuning with
three faces:

- **This section** edits the section selected in the plan lane (the lane
  head's section field, "Look ›", opens it). A click on a section jumps
  there: the playhead goes to where it has arrived, playing or paused, so
  the frame on screen, Look's head and the tuning below are the section the
  card edits. **Shift + click** selects it without moving the playhead;
  played on past it, or Shift-clicked from elsewhere, the card says whose
  picture it is now and **Show N** goes there. It edits how the
  section arrives — the fourteen kinds, each a small picture and a name, and
  a length — its glide (drawn disabled, saying why, after another scene), its
  grade (the named palettes, strength, the hue turn), the caption style its
  lines begin in, and a threshold's door. A Video Clip section also gets its
  clip: the project's clips as rows (each one's middle frame, name and
  length, the section's own marked), **Import clip…** (the flyout's import,
  landing here) beside the disabled **Generate…**, where it starts
  (**Continue** the section before, or **Own start**, and a **From** track;
  switching between the two leaves the picture where it is until the number
  changes), what happens after its end (**Hold**, **Loop**, **Bounce**), its
  **fit** (**Cover**, **Contain**), and a poster of the frame it starts on. A
  clip whose file is missing or refused says so there, in the placeholder's
  words. **Play the cut** plays from 1.5 s
  before the picture starts to change through the transition and a second
  past it, and pauses there when it started paused, whichever view is open;
  a seek or a play of your own lets it go.
- **Whole video**: the intro out of black (the first section's arrival), the
  outro into it, Play the intro, Play the end, and the finish — None or Film to
  start from, then every member.
- **Captions**: the voices, the caption styles (a rename takes every line,
  section and card along) and the title cards (added only on a press, at the
  playhead, with the project's title and artist). Each rests as one row — a
  voice's colours and how many lines it sings, a style's place in the frame,
  a card's start and title — and a press opens it. **Add a voice** makes a
  second singer with a colour of its own; an open voice's **This line** and
  **Section N** give it the line at the playhead or every line of the section
  under it, and take them back.

Every edit is saved with the project and autosaved like a plan edit, the
lane's marks follow at once, and a grade or finish changes the frame on screen
while paused. The file a window edit writes is the same one a treatment
writes, so a treatment applied later replaces what it names. The `look cards:`
report line names every control and the selected section's values.

## What the report says

```text
treatment:       23 sections over 3:59.960 (9 scenes), 2 routes, 29 lyric lines, 8 events, caption style set, caption styles 2 (1 line, 1 section), 1 title, output 1280x720@30 balanced, seed 2012 — song.treatment.json
caption styles:  base 21 lines, top 6 lines, top+shout 1 line
title cards:     drew 1 card over 186 frames; last "Door, No Room" / "CHAOS CAT FAMILIAR  ·  @chaoscatfamiliar" style=title at 410,277 460x166 presence=1.00 glow=blurred
contact sheet:   22 tiles from 0:03.680 to 3:57.965 at 1280x720 (1x target), sheet.png
export still:    t=176.000 frame 5280 of 7199 at 1280x720 (1x target), bridge.png
```

A treatment that applied and one that silently applied nothing both exit 0, so
the counts are the evidence: check them against the file you wrote. `caption
styles:` counts the lines drawn in each style over the run, the section's name
first; `title cards:` prints the last card *as laid out*, and `NO ARTIST` when a
card had none anywhere.

## What a director learns the hard way

Found while making the first video, each one from a contact sheet:

- **A caption at the bottom collides with whatever a scene keeps there** —
  Clawd's cats, Spectrum's bars, the terrarium's base. Give those sections a
  named style with `"anchor": "top_center"` (see *Named caption styles*).
- **Cadence does not draw the caption overlay.** The lyric line *is* its
  picture, which makes it the sing-along chorus, and it means a Cadence section
  with no line on screen is only its backdrop. Give each Cadence section its own
  `hue`, `saturation` and `reveal`; the defaults are one pink on one purple, and
  five sections of that read as one idea repeated.
- **Ask what each line is before choosing where it goes.** The second version
  of the first video was a visualizer with captions: nothing in the picture
  knew what a line said. Staging every sung line (`stage`, one section per
  line) and keeping the reactive scenes for the instrumental stretches is what
  made it read as a video *of the song*.
- **No title card unless one was asked for.** The first video opened on the
  track's name and the operator's verdict was that a card nobody requested is
  filler (2026-10-12). Open on the music. When a card *is* asked for it carries
  the artist as well as the title — for a Suno track, the display name and the
  `@handle` of the profile it was published under — and both come from the
  operator, not from a guess at the file name. It goes in `titles`, never in
  `lyrics`.
- **Clawd's petal show needs about three seconds of kicks before it ignites.**
  It is gated on a kick detector with hysteresis, by design. To have it lit *at*
  a cut, start a Clawd section two bars earlier and change its tuning at the cut.
- **Phosphor's `titles` clips a long line.** Its built-in 5x7 face fits about
  twenty characters across; a full lyric line runs off both edges. Leave it off
  for long lines and let the caption carry them.
- **Push a setting to its end before trusting the scene.** Orbital Lattice at
  `nodes: 1.6` drew black squares around its brightest pearls — a depth-test
  defect in the halo pass that the default size had been hiding as texture
  since the scene was written. It is fixed (`docs/DIVERGENCES.md`, MV1), and it was found
  by an audition sheet, not by a test.
- **A transition is reviewed from stills inside it.** `--sheet-at segments`
  photographs every section's midpoint, where no transition is running. List
  times a few tenths after each change instead (`--sheet-at 72.75,72.95,…`),
  and read the `transition:` line: `skipped` or `unavailable` there means it
  drew as a cut.
- **A still at a section's midpoint says nothing about how it arrives.** Sheet
  the first seconds of a section densely before trusting it.

## What is not built

Said here so nobody looks for a flag:

- Writing a window edit back into a followed treatment. Look's look cards
  (see *Editing the look in the window*) edit the `.musi`; while a treatment
  is followed they are drawn disabled and name the file, because its next save
  would undo them.
- Giving a line a voice from the Lyrics view. Look's Captions card adds a
  voice and gives it the line at the playhead or a section's lines; the cue
  card in Lyrics only shows a line's style, and steps it.
- Clearing a voice's or a style's hue, ink or size back to "its own" in the
  window, and the rest of a style's members (face, box, margins, effects):
  a treatment's.
- Transitions where both scenes keep moving. The outgoing picture is a held
  frame (see `transition`); a true crossfade of two live scenes is not built.
  The one place two scenes run at once is the threshold's doorway.
- Undo for plan edits, the look cards' among them.
- An imported caption face in a treatment. `"face": "imported"` is refused by
  name; import the face in the Lyrics panel after `--save-project`.
- Per-section routes, and the caption effects' drive-tuning blocks.
- A `generated` transition (a bridge clip across a cut, made from the two
  sections' frames). A generated clip section with `"first": "previous"` and
  `"last": "next"` is the way to it now. See `docs/VIDEO_GENERATION.md`.

## Evidence

`tools/treatment_check.sh` is the gate, and `tools/headless_check.sh` runs it. It
drives every flag through `tools/headless_run.sh` on a synthetic fixture and
`tools/treatment_check.py` checks the results against the treatment *text*
rather than against the report line that claims them. Among them:

- `--vocabulary` answers with `DISPLAY` removed.
- The saved project's plan, per-section tuning, routes, lyric spans, events,
  caption style, title, seed and output are the treatment's; reopening and
  re-saving it changes nothing but the bundle path.
- A treatment with no `lyrics` key applied over that project keeps its lines.
- A sheet's size is the layout rule recomputed in the checker, every tile has
  ink and a label, and two different scenes produce different tiles.
- The second still of a two-still pass is **byte-identical** to the same still
  taken alone in a fresh process.
- A still is the frame the export encodes: 39.10 dB against the decoded MP4
  frame, with a neighbouring moment at 21.55 dB as the control.
- Each refusal exits 1, names the place, and leaves no project and no sheet.
- Clips (`tools/clip_check.sh`, run after it): stills of a 23.976 fps and a
  variable-rate clip at times on and near frame edges are **bit-identical** to
  the frames FFmpeg extracts from the clips, on the GPU and on llvmpipe, and
  each beats its neighbouring frames by 16 dB and more; `hold`, `loop` and
  `bounce` past the end land on the computed frames; `cover` crops to the
  centre and `contain` leaves black bars; a clip whose display matrix turns
  it a quarter (a phone video) is drawn upright at its shown size, the frame
  FFmpeg shows bit for bit; `response` 0 is the same bytes over
  music and over silence (and `response` 1 is not); two exports of one window
  have the same per-frame digests; a saved project reopens from its bundle with
  the source deleted, and with the bundle's copy deleted too draws a card
  naming the missing file; a truncated file, a 16000x9000 claim and a missing
  file are refused by name.
- Named caption styles and title cards (`tools/caption_styles_check.sh`, run
  last): the saved styles, line and section styles and cards are the
  treatment's and survive a reopen; where each caption landed, counted by its
  colour against the same video with no styles; a card's ink in its rect against
  the same moment with no card, centred, and untouched frame elsewhere; the
  card's laid-out words against the treatment's, with the project's artist
  taken when the card names none; `NO ARTIST` said for a card with none.

Negative controls were run when each part was written, each on a scratch binary
(`TREATMENT_BIN`), each reverted byte-for-byte:

| Perturbation | The gate | The unit tests |
| --- | --- | --- |
| The persistence buffer is not reset between captures | the two-still equality fails | n/a — it needs a renderer |
| The frame outputs are not gated on an earlier error | `past-end: a sheet was rendered after the refusal` | n/a |
| A section's own `settings` are dropped | three applied-value checks fail | 4 of 28 fail |
| The pixel grid worked out on the framebuffer, not the output | the 2x still: 256x144 cells of 5 px, 23 colours, cells straddling pixels | 1 of the 10 `pixel.rs` tests fails |
| The console skips the palette | the handheld still has 679 colours; the levels still is off its steps | n/a |
| The screen upscales filtered, not by whole cells | six checks fail, among them 34 % of colour edges inside a cell | n/a |
| A section's caption style ignored (MV2, `tools/caption_styles_check.sh`) | four checks fail: the top section's line has 0 magenta pixels at the top, the shouted line none of its yellow there, `caption styles:` reads `base 2 lines, shout 1 line` | the stacking test in `caption_styles.rs` |
| Title cards skipped on scenes that draw their own line | three checks fail: one card drawn instead of two, the card over Cadence is not the one it was given, its rect differs from the control by 0.0 | n/a — it needs a renderer |

The first is the instructive one. The two stills had to be moved inside the one
scene that keeps light between frames, with the first before the second's
three-second run-up, before the control could fail at all: with a still in
Spectrum and a still in Clawd the equality held with the reset removed, because
nothing had left light in the buffer to carry. A check that cannot fail was
passing.

The second caught the dangerous case rather than the obvious one. A treatment
refused at parse opens no audio, so there is nothing to render and the missing
gate is invisible. A treatment refused at *compile* — a section past the end of
the track — has already opened its audio, and without the gate a sheet of the
undirected track is written to the path the caller asked for.
