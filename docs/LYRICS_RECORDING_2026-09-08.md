# Recorded Timed lyrics failure, 2026-09-08

The elapsed counter is repaired. The recommended local timing path now
recovers performed phrases from overlapping Whisper/MMS crops. Whole-track
acoustic acceptance remains open under LT2.

## What was inspected

Private evidence is in `build/lyrics-recording-2026-09-08/`:

- `frames/12.png` through `frames/40.png`: one frame per second from the
  operator's `2026-09-08 21-20-09.mp4`. These show the local job starting and
  remaining at `00:00 elapsed` while playback advances.
- `later/` and `later.jpg`: supplementary five-second samples after 00:40,
  showing the completed result, Apply, Potential cues, and caption playback.
- `prior-thread.md`: all 259 projected messages from T3 thread
  `f185f01b-848d-4331-9870-b846e5ae2808`, exported read-only.
- `findings.json`: source artifact hashes, local generation/cache facts,
  accepted lines and unresolved evidence.
- `gemini-first-clip.json`, `gemini-0-25.json`, `gemini-58-30.json`: fresh
  audio-only observations, with exact model selection, clip hashes and raw
  responses. The source windows are 10–35, 0–25 and 58–88 seconds.

## Why the displayed counter stayed at zero

`AssistController::start` records an application-clock timestamp. The panel
subtracted it from the song's playback position and clamped negative values to
zero. This mixes two different clocks; pause and seeking would also corrupt
the result. A comment justified using playback time for deterministic captures,
and the old test only checked that synthetic capture arrangement.

The controller now publishes elapsed seconds from `AssistJob::elapsed()`, the
same monotonic supervisor clock used for the timeout. The panel reads that
value. Synthetic running probes explicitly supply their fixed 2:05 value.

The new regression starts a real supervised helper with no playback and checks
the published elapsed value against the supervisor. Removing the publication
makes it fail; the negative-control receipt is retained. A separate muted,
private-Xvfb run entered Timed lyrics and Start normally: the counter reached
00:02 during playback, advanced from 00:23 to 00:26 while paused at 1:29.770,
and reached 00:27 after seeking back to 0:02.099. Its helper deliberately waits;
this is a lifecycle/UI test, not an acoustic-analysis result.

## Why the previous work did not fix this run

The recording explicitly selects local Whisper + MMS. The current saved
settings also select the recommended local profile. The earlier session's
later work mainly implemented the separate opt-in Antigravity performed-phrase
path; those changes do not change which route the local button runs. Local
parser and repeated-occurrence repairs did land, but did not solve this case.

The exact recorded process wrote `build/analysis/8c37e77c14e25ace/`. Its manifest
reports freshly generated measured, Whisper, synchronization and alignment
artifacts. It uses aligner version 10, localization policy 8 and acoustic
alignment version 7. This is not reuse of the September 6 baseline cache.
The running release executable's hash also matched the on-disk release before
this repair was built.

That local result placed 13 of 26 written lines and left 13 unresolved, with
nine additional unplaced Whisper proposals. Potential cues are deliberately
excluded from preview/export captions. Their presence in the lane therefore
does not mean their words will appear while playing.

The local result also treats the intended opening phrase “I'M TURNING
NEURALESE” as an unflagged caption at 10.645–16.157 seconds. Whisper instead
reports “I really think so”; both fresh, independently cropped Gemini listens
report separate deliveries around 10 and 15 seconds. The full-clip reference
text includes intended production instructions and backing vocals that cannot
be assumed to have been performed. Later written lines combine lead and echo,
while global and local forced alignments disagree on their occurrence. Turning
these refusals into accepted cues would hide the problem rather than resolve
it.

The earlier session itself did not establish the 3% target. Its final normal
performed-route result for this track contained 44 phrases against 47 model
reference proposals, with nine missing, six extra and six timing disagreements.
Those are diagnostic disagreements, not adjudicated error rates. Successful
schema, staging, Apply and build checks established working integration, not
accurate transcription or timing.

## Gemini and remaining work

Fresh requests to `gemini-3.8-flash-high` succeeded through the official
authenticated Antigravity ACP audio route. The requests contained decoded WAV
audio and the blind discovery prompt, without the written sheet or proposed
timestamps. No audio device was opened. These three successful requests
establish availability at the time of the check, not remaining quota.

The opening observations support the mismatch above. They also differ on
chopped words and later wording; model confidence is not acceptance evidence.
Further work must distinguish matching the written sheet from detecting the
performance, validate disputed boundaries independently, and test the selected
normal route. User settings and the live analysis artifacts were not replaced.
The operator chose to keep normal Assist entirely local. Gemini is authorized
only for checking our work; it must not become the normal analysis route.

## Local repair and its limits

The operator selected entirely local normal Assist, with Gemini used only to
check our work. The recommended wording stage now also stays local when no
sheet exists; previously that case invoked Codex. Existing explicit remote
route overrides remain supported, and no user configuration was rewritten.

`local-repair/normal/` contains a full normal helper run against the exact
recorded audio, using the local execution snapshot. The new recovery stage
uses 26 overlapping crops, and produces 27 renderable captions: eight retained
written cues and 19 corroborated audio phrases. The baseline rendered 13.
Eighteen written lines remain unresolved and three coarse performed proposals
remain unplaced. More captions is a coverage observation, not an error rate.

Concrete improvements:

- The false opening “I'M TURNING NEURALESE” is preserved for review instead of
  rendered. Separate “I really think so” captions occur at 10.405–11.226 and
  15.500–16.328 seconds, followed by “Total mode collapse” at 19.083–20.730.
- “Read a book once in a while” has separate lead/echo captions at
  66.553–68.234 and 68.486–70.357, and again at 168.45 and 170.37 seconds.
- The first “Mandate…” caption ends around 72.75 instead of stretching through
  78.96. Incompatible written compound lines retain review records.

Additional blind Gemini receipts cover 35–53, 115–140, 128–153 and 160–185
seconds in `local-repair/gemini-*.json`. They support several recovered phrases
and exposed a false local match at 132 seconds for words actually sung near
148. Both CTC crops agreed on that wrong location. Checking against their
original Whisper intervals rejects it and a neighboring “New release” fragment;
a regression explicitly tests this failure. The final result has 27 captions,
compared with 29 in the intermediate version before this correction.

These checks also show remaining defects: a missing first chorus entry near
62 seconds, missing middle-chorus phrases, uncertain chopped vocals and a late
written lead/backing compound that still runs too long. Gemini itself disagrees
on processed words and phrase edges. No human-adjudicated whole-track error
rate or ten-track acceptance is claimed.

`local-ui/` replays the freshly generated local bridge through normal staging
and Apply in a muted private Xvfb session. It asserts local-only execution,
then checks the recovered caption in the playing preview. This verifies UI
integration; the separate normal helper run establishes acoustic provenance.
The earlier `clock-ui/` run verifies elapsed time through playback, pause and
seeking. The operator's running app and original analysis cache were preserved.

Validation: all eight `tools/verify.sh --quick` gates passed (1,735 Rust
tests passed, one ignored; 483 Python tests ran, two skipped). The isolated
Assist/routing suite passed. Removing the ASR occurrence guard in memory makes
the new regression fail; the on-disk code was untouched. A normal cache replay
reused measured, Whisper, synchronization and alignment artifacts. Receipts are
`local-repair/verify-complete.log`, `local-repair/headless-final.log`,
`local-repair/negative-control.log` and `local-repair/release.log`.
Acoustic work remains open regardless of software-gate results.
