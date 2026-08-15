# Musializer Listening Lab

A local, agent-authored A/B listening workspace. Protocols point to audio files
already on this machine; the browser shows a zoomable PCM waveform, exact
millisecond time, question markers, audition windows, and agent-authored guided
feedback controls. Every selection appends a revision to a JSONL answer log
immediately; prose is an optional escape hatch rather than the default task.

## Run it

```sh
cd tools/listening-lab
npm install
npm run dev
```

Open `http://127.0.0.1:4178`. Vite listens on all interfaces, so the same port
is reachable over Tailscale when the host firewall permits it. There is no
authentication: expose it only to the local machine or a trusted private
network.

The example expects `build/fixture.wav`, which the main repository's headless
gate normally creates. A protocol whose audio is absent remains listed and
reports the missing path when selected; it does not make the server read a
fallback file.

## Add a listening test

Copy `protocols/template.listen.json.example` to
`protocols/<id>.listen.json`, then edit it. The filename must agree with `id`.
Paths may be absolute or relative to `protocols/`.

A question may name a reusable `feedback_templates` entry or carry an inline
`feedback` form. An agent can compose:

- `single`: one described decision;
- `multi`: evidence/reason chips, optionally limited with `max_selections`;
- `scale`: an ordered set of anchored judgments;
- `timestamps`: one-click capture of the current playhead; and
- `show_when`: reveal a field only for matching primary answers or prior field
  values.

Set `required: true` on decisive follow-ups. The rail calls a saved primary
answer **In progress** until all currently visible required controls are filled.
Changing an earlier answer removes now-hidden responses rather than leaving
stale contradictory data in the log. `feedback.note.collapsed` keeps the text
box behind an explicit “the controls missed something” disclosure.

The served protocol deliberately hides source paths and internal track ids.
When `blind` is true, track order is deterministically shuffled and the browser
receives only `Track A`, `Track B`, and so on. The private mapping is written
into each answer-log record for the agent who analyzes the results; it is never
returned by the browser APIs.

Answers land at:

```text
build/listening-lab/answers/<protocol-id>.answers.jsonl
```

Each click/save is a new revision. A reader should take the last record for
each `question_id`. Records include the primary `answer`, structured
`responses`, `complete` state, optional note, exact playhead, active blind
label, audition counts, timestamp, and the private label-to-track map.
The directory and new files are requested as mode `0700` and `0600`.

### Protocol shape

- `tracks`: one to four audio paths. Supported browser-oriented extensions are
  WAV, MP3, FLAC, OGG/Opus, M4A, and AAC; actual codec support is the browser's.
- `questions`: one to 100 time-anchored prompts.
- `at_seconds`: the marker/anchor in seconds; decimals are preserved.
- `window.pre` / `window.post`: replay span around the anchor.
- `tracks`: internal track ids available for that question.
- `kind`: `choice`, `scale`, or `text`. Choice/scale needs 2–7 `options`.
- `loop`: whether Replay window loops by default.
- `required`: presentation metadata; optional questions can still be answered.
- `feedback`: an inline guided form or the name of a `feedback_templates` form.
- `playback: "external"` plus `companion`: replace the browser player with a
  synchronized command/item/time handoff when another application owns the
  media or visuals.

Agents should prefer specific prompts with an observable decision. “Which is
better?” loses evidence; “Which places the first consonant closer to the visible
transient, and at what time?” can be acted on.

## Run the current CX-4 sessions

CX-4 compares blind visual settings, so the Rust protocol runner applies each
look while the browser acts as its guided feedback sheet. Start the lab, choose
`cx4-surprise-a-feedback`, and run the command displayed at the top of the page:

```sh
# terminal 1
cd tools/listening-lab
npm run dev

# terminal 2, from the repository root
cargo run -- --protocol build/protocols/cx4-surprise-a.protocol.json
```

Keep both surfaces on the same `qNN`. In Rust, use `B` to alternate a two-look
item and `N` to advance; record the verdict and the revealed reason/fit controls
in the browser. Then repeat with the `-b` pair. Do not open either `.key.json`
until both sessions are complete.

The two browser logs are:

```text
build/listening-lab/answers/cx4-surprise-a-feedback.answers.jsonl
build/listening-lab/answers/cx4-surprise-b-feedback.answers.jsonl
```

If the Rust protocol files are regenerated, rebuild their feedback sheets with:

```sh
cd tools/listening-lab
npm run import:cx4 -- ../../build/protocols/cx4-surprise-a.protocol.json \
  ../../build/protocols/cx4-surprise-b.protocol.json
```

## Playback controls

- Click/drag the waveform to seek; hover for an exact timestamp.
- Enter `mm:ss.mmm` or seconds in the Seek field and press Enter.
- Use ±10 ms and ±100 ms buttons for boundary work.
- Change waveform zoom from 10 to 400 pixels/second.
- Slow playback to 0.5× or 0.75× while preserving pitch where the browser can.
- Switch A/B without resetting the playhead; a playing track continues from
  the same time on the other candidate.
- Keyboard: Space play/pause, `R` replay window, comma/period ±10 ms,
  Shift+comma/period ±100 ms, brackets switch candidate, 1–7 answer.

For timing adjudication, prefer WAV, FLAC, or a constant-bitrate source.
WaveSurfer's documentation notes that variable-bitrate audio can put the decoded
waveform and media-element clock slightly out of alignment; do not claim
millisecond truth from a VBR MP3 without checking the source/container first.

## What the project previously asked humans to judge

The repository has two relevant precedents:

1. **CX-4 Surprise keepability.** On one sparse and one energetic track, Song
   Atlas and Cadence each receive five pre-CX-4 and five revised seeded looks,
   judged blind as `keep`, `interesting but needs fixing`, or `reject`. The
   bundled guided sheets then ask for track fit, preserved strengths, repair
   targets/distance, pairwise distinctness, decisive dimensions, and confidence
   only where each is relevant.
   Passing means at least two keeps and no more than one reject per scene, with
   consecutive presses visibly distinct. This is a *visual-settings* A/B and
   must still run through the Rust `*.protocol.json` runner; this audio-only lab
   cannot apply scene snapshots.
2. **Lyrics timing adjudication.** The operator previously listened to 23
   waveform-confirmed spot checks, choosing between anchor and baseline timing
   and recording the correct occurrence/time. This lab is a better surface for
   future timing candidate renders because it makes the waveform, exact time,
   repeated audition, and notes one durable interaction.

The unbuilt CX-4 tap checks were: calibrate the proposed −100 ms default, stamp
eight familiar lines and inspect the median residual; then compare 80, 120, and
200 ms visual flashes. Those require visual/tap instrumentation rather than two
audio files, but their concrete questions are preserved here for future mixed
media support.

## Why WaveSurfer.js

[WaveSurfer.js 7](https://wavesurfer.xyz/docs/) provides typed playback plus
official Regions, Timeline, and Hover plugins. Those directly implement the
question windows, marker rail, time ruler, and precise hover readout used here.
[Peaks.js](https://github.com/bbc/peaks.js/) was also evaluated; its overview /
zoom views and annotation model are excellent, but its Konva and waveform-data
peer stack is more machinery than this focused local player needs.

## Checks

```sh
npm test
npm run build
npm run test:e2e
```

The Playwright run creates its own short WAV and isolated protocol/answer
directories under the repository's gitignored `build/` tree. Chromium is
started with `--mute-audio`; no test changes the fixture PCM, desktop volume,
or system audio configuration.
