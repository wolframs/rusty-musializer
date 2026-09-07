# Lyrics grouping review — 2026-09-07

The lyrics investigation is paused at the operator's request. Two performed-
phrase grouping decisions remain open; no reference has been changed for them.

## Listen and score

On a device connected to the same Tailscale network, open
[the prepared listening session](http://100.102.37.124:4180/).
It uses the existing Listening Lab. **Compare the timed lyrics** shows
synchronized one-phrase and two-phrase caption previews above the answer form.
Their timelines share the playhead, label cue edges in original-track time, and
mark the proposed gap. Click a cue to seek, use **Seek before gap**, or
**Replay comparison**. Then select **one phrase**, **two phrases**, or **not
sure**, and record confidence. Answers save
immediately. A second-phrase onset and a note are optional.

The same two audio questions are available in the existing Musializer protocol
runner. The synchronized comparison is currently a browser feature; the Rust
protocol runner shows the question without timed lyric alternatives:

```sh
MUSIALIZER_BINARY="$PWD/target/release/musializer" \
  build/listening-review/lyrics-grouping-20260907/run.sh
```

This command intentionally plays audio for the operator. Space pauses, R
replays, 1/2/3 answer, and N skips. Both choices concern the same performance;
there are no differently processed A/B audio versions. The portable bundle is
`build/listening-review/lyrics-grouping-20260907.zip`; it contains the audio,
protocols, launcher, instructions and source hashes. Extract it on another
device with Musializer and point `MUSIALIZER_BINARY` at that device's executable.

| Question | Original excerpt | Passage to judge | Combined audition |
| --- | --- | --- | --- |
| Groyper Idol | 25–31 s | 26.5–29.7 s: “If shit goes south / take out your gun” | 0–6 s |
| Floor Mechanics | 100–106 s | about 101.6–103.5 s: “I am / present” | 8–14 s |

The two six-second excerpts are joined with two seconds of silence. Decoded
source samples remain intact. `evidence.json` maps audition time back to the
source: add 25 seconds for Groyper; add 92 seconds for Floor Mechanics.
The preview uses saved, unadjudicated proposals: Groyper's two cues are
26.50–28.00 and 28.22–29.70 seconds (`03/candidate-performed-runtime/lyrics.performance.json`);
Floor's are 101.45–102.20 and 102.85–103.55 seconds
(`07/performed-reference-audio-v3/reference.json`), relative to the private
research root. Both interpretations use the same outer edges to isolate the
grouping choice. These are proposed boundaries, not a measured claim of vocal
silence; music continues underneath. If the timing prevents a judgment, use
**not sure** and add a note. The original model references remain unchanged.
Independent model listens disagree about whether each pause separates phrases.
The established rule remains separate sung echoes, but one continuous chop run.

## Answers and service

Browser answers append to
`build/listening-lab/answers/lyrics-grouping-20260907.answers.jsonl`.
Rust answers are beside the protocol as `grouping.answers.jsonl`.
Use either surface; question ids are `groyper` and `floor`. If both surfaces
are used, preserve both answer logs and resolve conflicting choices explicitly.

The browser server is the user service `musializer-lyrics-grouping-review`,
bound only to this machine's Tailscale address on port 4180. It remains available
when the coding session ends, while the machine is awake. Check or stop it with:

```sh
systemctl --user status musializer-lyrics-grouping-review
systemctl --user stop musializer-lyrics-grouping-review
```

To recreate the bundle in a fresh directory:

```sh
python3 tools/lyrics_grouping_session.py --output-dir build/listening-review/new-grouping
```

Point `LISTENING_LAB_PROTOCOLS` at its `browser/` directory when starting the
existing `tools/listening-lab` Vite server. The generator refuses to replace an
existing session. Music, generated bundles and answers are private build
artifacts, not committed assets.

## Research checkpoint

All ten actual normal Assist runs have finished. The final saved-candidate
comparison uses reference v3 and matcher v6; original receipts retain their
actual adapter versions (10 for tracks 01–07, 11 for 08–10). The full table is
in `build/lyrics-acceptance-2026-09-06/NORMAL_ASSIST_CHECKPOINT.md` and the
[investigation record](LYRICS_ASSIST_REPAIR_2026-09-06.md).
These are model-reference disagreements, not adjudicated accuracy. The ≤3%
per-track target with ±0.5-second edge tolerance remains unmet/unproven.
No research helper remains running. Resume after the operator's listening
judgments; preserve the existing frozen reference until adjudication is recorded.
