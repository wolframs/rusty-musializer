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
| When exactly is each line sung? | CTC forced alignment of the known words inside a known window (`torchaudio.pipelines.MMS_FA`, the runtime `tools/musializer_doctor.py` reports) | A wrong window gives a confident wrong answer. Check it against the song's own repetition |

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
repeats, and every still of a run comes out of one pass over the track.

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
| `notes` | Anything. Parsed and ignored: JSON has no comments, and a treatment wants them |
| `audio` | `{ "path", "sha256" }`. The path resolves against the treatment file's own directory. With it, `--treatment` alone opens the whole video; without it, the treatment directs the track the command line opened. `sha256` is optional — pin it when the file should outlive the session |
| `output` | `{ "resolution": "1920x1080", "fps": 30, "quality": "high" }`, each optional. `--resolution`, `--fps` and `--quality` on the command line override it |
| `seed` | The deterministic seed scenes are built from |
| `caption_style` | The `.musi` member names, each optional here (see below) |
| `scenes` | Per scene name: `settings` (the base tuning for every section of that scene) and `routes` |
| `segments` | The plan. Required |
| `lyrics` | Timed lines |
| `events` | Authored markers; Constellation flares on them |

### `segments`

Each has `at`, `scene`, and optionally `settings` and a `note`. Sections are
listed in playing order, the first starts at 0, and each runs until the next one
starts; the last runs to the end of the track. There is nothing to keep
contiguous by hand.

A section's tuning is the scene's defaults, then that scene's `scenes.<name>.settings`,
then the section's own `settings`. Settings are named, not positional:
`"weight"`, `"loom.weight"` and `"settings.loom.weight"` are one control. A
toggle takes `true`/`false` or `1`/`0`. An in-range value is snapped to the
control's own precision, which is what the Tune panel does — the number in the
file is the number the readout shows.

Two adjacent sections of the **same** scene keep the scene's running state and
only change its tuning, which is how one scene changes mid-phrase: a riser that
tightens, or a ground that goes from night to day on a bar line. A section of a
*different* scene starts that scene fresh.

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

### `caption_style`

`face` (`alegreya`, `space_grotesk`), `box` (`none`, `shadow`, `plate`),
`anchor` (nine positions), `size_scale`, `margin_scale`, `width_scale`,
`text_rgba`, `box_rgba`, and `effects` (`glow_strength`, `glow_radius`,
`glow_rgba`, `glow_pulse`, `glow_pulse_depth`, `glow_hue_drive`,
`glow_hue_range`, `shadow_blur`, `shadow_opacity`, `plate_roundness`). Colours
are `"#rrggbb"` or `"#rrggbbaa"`. Whatever is not stated is the shipped default;
with no block at all the track's own style is kept.

## What the report says

```text
treatment:       23 sections over 3:59.960 (9 scenes), 2 routes, 29 lyric lines, 8 events, caption style set, output 1280x720@30 balanced, seed 2012 — song.treatment.json
contact sheet:   22 tiles from 0:03.680 to 3:57.965 at 1280x720 (1x target), sheet.png
export still:    t=176.000 frame 5280 of 7199 at 1280x720 (1x target), bridge.png
```

A treatment that applied and one that silently applied nothing both exit 0, so
the counts are the evidence: check them against the file you wrote.

## What a director learns the hard way

Found while making the first video, each one from a contact sheet:

- **There is one caption style per project.** A caption anchored at the bottom
  collides with whatever a scene keeps at the bottom — Clawd's cats, Spectrum's
  bars, the terrarium's base. `top_center` is the anchor that is clear in every
  scene. A per-section style needs a `.musi` extension and is not built.
- **Cadence does not draw the caption overlay.** The lyric line *is* its
  picture. That makes it the title card and the sing-along chorus, and it means
  a Cadence section with no line on screen is only its strata.
- **Clawd's petal show needs about three seconds of kicks before it ignites.**
  It is gated on a kick detector with hysteresis, by design. To have it lit *at*
  a cut, start a Clawd section two bars earlier and change its tuning at the cut.
- **Phosphor's `titles` clips a long line.** Its built-in 5x7 face fits about
  twenty characters across; a full lyric line runs off both edges. Leave it off
  for long lines and let the caption carry them.
- **Push a setting to its end before trusting the scene.** Orbital Lattice at
  `nodes: 1.6` drew black squares around its brightest pearls — a depth-test
  defect in the halo pass that the default size had been hiding as texture
  since the scene was written. It is fixed (`AGENTS.md`, MV1), and it was found
  by an audition sheet, not by a test.
- **A still at a section's midpoint says nothing about how it arrives.** Sheet
  the first seconds of a section densely before trusting it.

## What is not built

Said here so nobody looks for a flag:

- Per-section or per-line caption styles, and a title lane separate from lyrics.
  A title today is a lyric line shown through Cadence.
- Transitions. A section change is a cut; there is no fade or crossfade, and no
  fade from or to black.
- An imported caption face in a treatment. `"face": "imported"` is refused by
  name; import the face in the Lyrics panel after `--save-project`.
- Per-section routes, and the caption effects' drive-tuning blocks.

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

Three negative controls were run when it was written, each on a scratch binary
(`TREATMENT_BIN`), each reverted byte-for-byte:

| Perturbation | The gate | The unit tests |
| --- | --- | --- |
| The persistence buffer is not reset between captures | the two-still equality fails | n/a — it needs a renderer |
| The frame outputs are not gated on an earlier error | `past-end: a sheet was rendered after the refusal` | n/a |
| A section's own `settings` are dropped | three applied-value checks fail | 4 of 28 fail |

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
