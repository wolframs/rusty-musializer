# Lyrics Assist investigation, 2026-09-06

Work is tracked as **LT2** in `FEATURE_PARITY_PLAN.md` and paused at the
operator’s request on 2026-09-07. These are observations and experiment receipts,
not an acceptance claim. The [listening handoff](LYRICS_GROUPING_REVIEW.md)
contains the two pending grouping judgments, browser session and portable bundle.

The operator specified ten tracks and approved an error definition: a missing
or extra performed lyric line, or either boundary more than 0.5 seconds wrong,
counts as an error; the target is at most 3% per track. They also authorized an
**opt-in Antigravity audio route in normal Assist** if needed. Do not silently
enable it or send audio from the default local route.

## Evidence and authority

Private artifacts are under `build/lyrics-acceptance-2026-09-06/`, including
`tracks.json` with full audio/reference hashes, original embedded lyric sheets,
classification snapshots, an unchanged tools/schema/prompt snapshot under
`baseline-runtime/`, and complete production baseline runs. Original Music
files and the live `build/analysis/` caches have not been overwritten.

All ten supplied files carry embedded `lyrics-eng`. A sheet describes intended
words; audio may contain omitted lines, extra ad-libs, or chopped repetitions.
Counting every unplaced sheet line as a missing *performed* lyric is therefore
not a valid accuracy metric. Authored wording remains useful for spelling and
context, while performance presence must be checked against audio.

`tools/lyrics_research/audio_audit.py` compares only complete observed phrase
edges after matching authored text. It explicitly reports unscored lines and
`not_adjudicated`. Model agreement is not ground truth and cannot establish the
3% target. The partial-clip exclusions must not be turned into a reduced
acceptance denominator.

## Antigravity transport actually tested

`agy` is not installed. The operator directed reuse of T3 Code's authenticated
`agy_acp_server_1.1.1`. Its initialization advertises audio input, authentication
succeeded, and the session offers `gemini-3.8-flash-high` (also medium/low).
The research client selects the exact requested model and sends WAV audio as
an ACP audio content block. The text-only passthrough HTTP facade is not used.

`tools/lyrics_research/antigravity_audio.py` takes explicit server, harness and
profile paths. It never reads or copies OAuth token contents or edits profile
settings. Client filesystem, terminal and permission requests are denied. Audio
is decoded to a pipe with no playback or original file metadata. The separate
passthrough sign-in attempt was cancelled after T3 authentication worked.

The blind sweep uses 18-second clips every 15 seconds, two requests concurrently,
with per-clip response/model/session/prompt/hash receipts. All ten tracks are in
the sweep. Cropped phrases need supplementary observations before scoring.

Two independently cropped observations of the same passage (starts 29 and 30
seconds on track 02) differed by up to 0.48 seconds on complete phrase edges.
This is a useful consistency check, not proof of sub-0.5-second accuracy.

## Confirmed defects

1. Multiline parenthetical production directions became caption text and CTC
   targets. The screenshot's vocal-chop direction is this defect. Parsing now
   treats wrapping consistently, retains explicitly quoted vocal words, preserves
   source text separately, and keeps wrapped backing lyrics. A leading delivery
   direction previously disappeared only from coarse matching tokens: MMS and
   captions still used the original string. They now receive the sung text.
   Adjacent section tags followed by an ad-lib direction are also structural.
2. Independent local refinement can replace two distinct jointly aligned
   repetitions with the same occurrence. Gemini independently heard both
   deliveries of the repeated line on track 08, around 56.2 and 58.7 seconds.
   The proposed production repair confines repeated phrases to their original
   jointly aligned neighbours, using a frozen copy of block decisions and owners.
3. Applying that confinement to *every* line can preserve a wrong initial block
   edge which local refinement was repairing. The production candidate narrows
   it to phrases repeated inside the same block. The broader experimental output
   is retained as a regression control.
4. Start-to-start disagreement can reject an acoustic placement inside a broad
   coarse interval. The `interval-coarse` experiment probes this; its tolerance
   relaxation is **not** a production fix.
5. Intended lyrics can differ substantially from actual sampled/stuttered words.
   Track 01's middle section is an example. Forcing the complete intended stanza
   does not create evidence that those words were performed.
6. Even jointly aligning Gemini's short-clip transcript can select an incorrect
   earlier acoustic event. In track 01's 105-second crop, unbounded MMS placed
   the first phrase near 108.5; independent Gemini crops located it near 112.7
   and 112.9. Phrase-specific CTC windows are being tested against that failure.

## Experiments

| Output | Change | Status |
| --- | --- | --- |
| `baseline/` | Unchanged production path | All ten completed |
| `candidate-parser/` | Source parsing only | All ten completed; not acceptance |
| `candidate-joint.json` | Coalesce short anchor blocks; disable local overwrite | Helps some repeats, regresses others |
| `candidate-joint-no-stars.json` | Above, without inter-line wildcard tokens | Not a general solution |
| `candidate-bounded-local.json` | Constrain all local passes by block neighbours | Exposed the unique-line regression |
| `candidate-ordered/` | Initial broad production neighbour constraint | Superseded by narrower repeated-phrase guard |
| `candidate-repeated/` | Production guard only for repeated phrases in a block | Ten-track rerun |
| `candidate-interval-coarse.json` | Compare unique-line occurrence against broad coarse interval | Research only |
| `whisper-chunks/` | Independent 18-second local Whisper contexts | All ten generated; seam/presence analysis pending |
| `candidate-gemini-phrases.json` | Blind Gemini phrase discovery, joint local MMS | Does not yet solve acoustic occurrence errors |
| `candidate-gemini-constrained.json` | Position-specific phrase windows in CTC | Research and synthetic controls |

Gemini's method review is retained under track 08. It suggested joint bounded
decoding, controlling wildcard skipping, and auditing repetitions and relative
intervals. Its diagnostic explanations are hypotheses; the experiments decide
which changes survive.

`tools/ctc_window_align.py` implements a NumPy CTC trellis with independent
half-open frame constraints per target position. Repeated labels require a
blank separator. Impossible windows raise an explicit error. The negative
control chooses the wrong stronger occurrence without windows and the intended
weaker occurrence with windows. NaN and impossible-path tests also exist.

Focused tests passed at the current checkpoint (72 lyric tests and four CTC
window tests). Full verification, silent interactive-path checks, the opt-in
route, complete acoustic adjudication and the final release build remain open.
No commit or push has been requested. Existing unrelated scene/UI edits remain
outside LT2's changes.

## Integration checkpoint

The experimental Antigravity route now reaches normal Assist through the frozen
route graph, per-job audio confirmation, helper dispatcher, local alignment,
bridge validation, review flags, staged Apply and observed project provenance.
Default settings remain local. Runtime/account discovery is filesystem-only;
the official ACP process checks OAuth and exact model availability at Start.
The three optional path overrides are passed identically to doctor and jobs.

The production experiment uses 30-second clips every 15 seconds. The first run
on track 02 exposed cross-clip duplicates after local alignment and a missing
review-surface connection; both were repaired. Clipped observations are recorded
as Potential candidates. The first two fresh tracks encountered invalid JSON;
subsequent code preserves failed responses and retries once. Startup/session
notifications are excluded from the prompt's response text.

Early independent-clip comparisons show that forcing every Gemini boundary to
MMS can worsen timing, especially on chopped vocals. A third crop of track 02's
chorus also moved Gemini's own first boundary by 0.45 seconds, so a single model
observation cannot establish the 0.5-second acceptance criterion. Additional
fusion/validation work remains active; no 3% claim has been made.

Validation at this checkpoint: 362 Python tests passed (one skipped), Rust tests
passed, and the isolated Assist/routing headless suite passed. Later edits still
need the final gates and release build. The CTC trellis matched TorchAudio's
unconstrained forced-align spans exactly on 240 seeded random cases. Two new
parser/caption tests failed against the unchanged baseline snapshot as expected.

## Later acoustic findings and controls

The opt-in route was exercised through the real entry, confirmation, helper
supervisor, staging, Apply, Lyrics panel and playing Cadence, at 960x640 on a
private Xvfb display. The helper replayed preserved real Gemini artifacts for
this UI check; it was not a fresh provider run. Playback used `--mute` plus an
unresolvable Pulse sink; config, cache, state, recovery and analysis folders were
private. A lyrics-only result used to reserve empty summary rows for two other
lanes, leaving zero clickable review rows. Returning that space now exposes
three rows with scrolling. The entry badge and workflow text follow the selected
route, and the review names performed wording as a proposal.

MMS can worsen a good model boundary on chopped vocals. The tested estimate uses
complete independently cropped observations plus the constrained CTC boundary:
the median when multiple complete audio observations exist, and the lone audio
proposal when they do not. Forcing its words through CTC does not make a lone
model observation independent evidence. The CTC search window includes all
complete observations, because the preferred crop can have the wrong clock.

Temporal-overlap deduplication emitted a fast passage twice when two Gemini
clocks drifted apart. Ordered phrase matching now pairs each delivery once even
when approximate intervals do not overlap. Equal-text repetitions remain
separate. Competing cross-clip versions and contained duplicate fragments are
parked as Potential cues; they are not asserted absent from the performance.

Two shorter-clip trials (18 seconds, stride 9) regressed independent phrase-edge
agreement against the 30-second, stride-15 method:

| Track | 30-second agree/disagree/unmatched | 18-second agree/disagree/unmatched |
| --- | --- | --- |
| i rly think so | 24 / 0 / 19 | 20 / 2 / 21 |
| The Cage Went Deep | 53 / 8 / 25 | 49 / 15 / 22 |

These are partial agreement counts, not full-line error rates. The shorter
variant is not promoted.

A full-coverage Gemini audit was then attempted with every proposed cue and
unrepresented audio interval checked. Its apparent timing scores **failed a
negative control**: on the fast Cage passage, adding 1.5 seconds to the supplied
timestamps led to 17 of 18 wrong proposals being accepted almost unchanged.
The same control on a slower Fence passage detected all three shifted cues.
Thus passing the slower control does not validate the timing auditor generally.
Do not quote the timestamp-visible audit percentages as accuracy. The raw
receipts and `AUDIT_INVALIDATED.md` preserve the failed control; the subsequent
`audio-only-audit` protocol hides proposed timestamps.

The presence audit exposed a separate, major production issue: supplying the
full lyric sheet to Gemini made it invent entire expected verses over
instrumental passages in Eight Thousand Tokens. Original blind audio-only
observations and the separately cropped auditor both rejected these. A 30-second
audio-only pilot also found the actual later vocal entrance without the invented
spoken introduction. The production discovery prompt now receives **audio
alone**; authored text stays local for comparison, and changing it does not
invalidate remote audio receipts. A new complete ten-track sweep and blinded
audio audit are running under `candidate-antigravity-blind/` and
`audio-only-audit/`. Final acceptance remains open.

Current offline gates passed, including the full Rust suite, 20 Antigravity
adapter tests, real TorchAudio CTC controls, support-bundle/credential canaries,
and the isolated Assist/routing headless suite. The final prompt/source changes
still require their final rerun, interface check and release build.

## Suspend recovery and corroboration, 2026-09-07

The resumed Codex server holds the original thread transcript and writer lock.
A process-tree check found no second Codex writer; the old research workers had
exited by the second check. No current session or unrelated process was killed.
The failed confirmation run reported OAuth host DNS resolution failure during
suspend recovery. DNS resolved again before resuming the cached requests.

Audio-only discovery still invented generic lyrics in one instrumental clip of
Eight Thousand Tokens. The current candidate therefore obtains fresh, shorter
audio-only observations for phrases seen in just one crop. Neither proposed
words nor timestamps are sent in those confirmation requests. A phrase needs
two complete crop observations before becoming an active caption; otherwise it
stays available as a Potential candidate. This is a hypothesis under test, not
a guarantee: agreement between two calls to the same model can share a mistake,
and withholding an actually performed phrase still counts as a missing line.

The new sweep is `candidate-antigravity-confirmed/`, reusing matching initial
receipts from `candidate-antigravity-blind/`. The latter remains the frozen
one-pass baseline where completed. The fake-provider negative control invents
a phrase in discovery and returns no words in the fresh crop: it produces zero
active captions and retains the unconfirmed phrase. All 21 Antigravity tests
passed after recovery. Full real-track corroboration and acceptance remain open.

Recovery validation: `tools/verify.sh --quick` passed all eight gates, 22
Antigravity tests passed, and `cargo build --release` completed. The retry now
names the validation error and exact clip duration; this recovered a real
17.08-second confirmation response whose last timestamp exceeded the clip.

Eight Thousand Tokens completed both corroboration and a fresh timestamp-blind
audit: 56 present, 6 timing errors, 1 extra, 1 missing (8/57, about 14%). The
one-pass audit was 57 present, 8 timing errors, 7 extra, 1 missing (16/58, about
28%). These remain model estimates. Comparing the retained receipts, six
removed phrases had previously been judged absent, while the removed word
"leak" had been judged present. The earlier independent phrase comparison
retained all 49 matched phrases (45 agreeing, 4 disagreeing).

Groyper Idol exposed a serious corroboration regression: active captions fell
from 66 to 34, with the blind phrase comparison changing from 20 agreeing / 3
disagreeing / 25 unmatched to 18 / 3 / 27. Unconfirmed observations include
spelling variants, different phrase grouping, and substantial disagreements
about actual words. A local matching-only `-ing`/`-in` normalization experiment
recovered only two rows, so it was not promoted. The strict confirmation rule
is not yet established as a general improvement. Its all-track sweep and
full-coverage audits continue; none of these results meets the 3% target.

## Vocal separation experiment, 2026-09-07

Strict two-crop corroboration is not accepted as a general production-quality
solution. The fresh Groyper audit found 33 present, 3 timing errors, 1 extra,
and 18 missing; Fence found 67 present, 10 timing errors, 1 extra, and 3 missing.
The i rly think so audit found 44 present, 4 timing errors, no extras, and 4
missing. Reference phrase grouping also varies across model audits, so their
denominators cannot be treated as a fixed benchmark. Full-coverage checks and
uncertainty remain necessary; no percentage is adjudicated accuracy.

`tools/lyrics_research/separate_vocals.py` now provides a reproducible local
input experiment using TorchAudio's `HDEMUCS_HIGH_MUSDB_PLUS`. The official
[Demucs documentation](https://github.com/facebookresearch/demucs) describes
vocal source separation; the installed TorchAudio bundle and implementation
were inspected directly. The helper records the exact checkpoint, audio and
output hashes, framework versions, chunk/overlap policy, and sample counts.
It uses ten-second windows with two-second crossfades and never opens an audio
output device. All ten full-track stems are in `vocal-stems/`. Input and
separated sample counts are identical; 16 kHz output duration differs by less
than one output sample. Re-running track 03 used the validated cached stem.
Original mixes remain the timing/presence reference; extracted stems can distort
or remove real vocals.

Four short trials are preserved under `vocal-trial/`: Groyper at 90 and 150 s,
and Eight Thousand Tokens at 250 and 275 s. Audio-only stem transcription
avoided the generic invented verse at 250 s and located the real later vocal
entrance. It still disagreed on the short ending, hearing "Memory wipe" where
the earlier mix auditor reported "Memory" and "leak". MMS on the separated
versus original crop did not show a convincing general timing improvement in
the small paired trial.

Adding lyric-sheet context to the vocal stems passed one planted-verse negative
control but failed other needs: it omitted the Hey ad-libs and imposed the
authored concatenated breakdown text. A paired wording-choice control, with
option order reversed, consistently selected the **audio-only** wording instead
of that authored text. Phonetic explanations from the model remain hypotheses,
but the observed choice is order-invariant. The next full-track experiment
therefore stays audio-only. `vocal-trial/raw_stem_trial.py` runs raw stem discovery
without the strict corroboration filter on tracks 03 and 06, retaining the
preprocessing receipt and original audio hash. This is an explicitly labelled
research ablation, not the normal Assist route.

## Fixed comparison and dedicated ASR trial

The full vocal-stem experiment contradicted the promising short samples.
Groyper's original-mix audit of stem-derived captions reported 42 present, 13
timing errors, 17 extras and 8 missing. Eight Thousand Tokens' stem discovery
invented a new four-line verse after 302 seconds. Neither separation nor strict
two-crop filtering is promoted as a general accuracy solution.

`tools/lyrics_research/fixed_reference.py` freezes complete timestamp-blind
audits with candidate/audio/prompt/receipt hashes and scores successive
candidates against unchanged phrase grouping. Four proposals currently exist
for tracks 01, 02, 03 and 06 (`reference-proposal-v1.json`). Five controls cover
the exact half-second tolerance, one error for two bad boundaries, missing and
extra lyrics, repeated-phrase assignment, nonfinite input, complete audio
coverage and refusal of timestamp-visible or mismatched receipts. The matching
policy is explicit and monotonic, with timing only a tie-breaker; it does not
turn a shifted unique line into both a missing and extra line.

These references remain **model proposals**, with phrase grouping inherited
from the audited candidate. That can favor its segmentation. Unmatched split
or merged captions therefore need inspection; strict cue scores are diagnostic
and cannot establish the user's 3% acceptance. This tool fixes denominator drift,
not the underlying need for trustworthy audio reference boundaries.

A separate `qwen-venv/` is prepared under the private evidence root for
`qwen-asr==0.0.6`, `transformers==4.57.6` and `accelerate==1.12.0`. It imports
Torch/TorchAudio from the existing alignment environment through a local `.pth`
and installs its own other dependencies. The dry-run report records 79 new
packages and zero Torch/NVIDIA/NumPy changes; the existing Assist environment
was not modified. Public model downloads disable implicit Hugging Face tokens
and telemetry. The official [Qwen3-ASR repository](https://github.com/QwenLM/Qwen3-ASR)
explicitly includes singing/song recognition and a separate forced aligner.
`qwen-trial/pilot.py` tests Qwen3-ASR-1.7B plus ForcedAligner-0.6B on the same four
short mix/stem pairs, with no lyric context or remote inference. Results and
model revisions are retained before deciding whether integration is warranted.

## Restoring authored caption authority

The integration audit found a scope error in the first opt-in route: even when
an authored sheet existed, it replaced display text with audio transcription.
That contradicts the normal Assist distinction between supplied words and
recognition evidence. Version 7 of `antigravity_lyrics.py` now emits coarse
`musializer.lyric-timing/v1` evidence when a sheet exists. The sheet stays local;
single-view phrases remain available as evidence. The orchestrator shares the
existing authored sync and anchor/block path between Whisper and Antigravity.
Only the no-reference branch uses transcription as proposed caption text.
The UI help and provider/data-flow documentation now describe that distinction.

The focused routing test uses deliberately different ASR and authored wording
and proves the supplied text reaches the authored localizer. Its negative
control bypasses that branch in an isolated in-memory function and fails as
expected before any subprocess can run. The adapter test also proves the sheet
is not sent and single-view evidence is not prematurely discarded. The local
route still passes the same checks. Rust tests (498 app, 1028 core, 186 runtime)
and all 382 Python support tests passed; the generated code map was refreshed
after its stale-document gate failed. The release build completed. A fresh
normal-helper run of track 02 is underway in `candidate-antigravity-authored-runtime/`.

Pure local trials using existing Gemini coarse receipts have completed for
tracks 01, 03 and 06 in `candidate-antigravity-authored/`: 48/8, 12/6 and 38/4
placed/unresolved authored lines. These counts prove coverage accounting, not
accuracy. The superseded caption-replacement sweep and its auditor were stopped
as owned process groups after preserving their receipts; tracks through 08
completed that candidate, and 09 has partial confirmation receipts. Its v6
adapter is frozen under `runtime-v6/`, so the old experiment can be reproduced
without silently changing semantics. The next all-track run must evaluate the
authored route, retaining the original task scope.

The Qwen pilot completed eight mix/stem tests. Recognition avoided invented
verses in the tested instrumental interval, but its forced aligner produced
22 zero-duration spans among 55 words in the first Groyper mix and 22/34 in the
breakdown mix. Vocal separation did not remove the issue. No zero-duration
span is accepted as a timing result. This agrees with a reported limitation in
[upstream issue 197](https://github.com/QwenLM/Qwen3-ASR/issues/197), but the local
receipts are the evidence here. Qwen is still a possible **coarse recognition**
source; it is not installed as a production aligner.

A compatible-midpoint fusion ablation regressed tracks 02 and 03. Excluding
model boundaries near internal crop cuts improved the raw track-06 comparison,
but regressed two partial comparisons on the earlier all-track artifacts.
Neither change was promoted.

The fresh track-02 normal-helper run completed: schema-v1 coarse evidence names
Gemini 3.8 Flash and `coarse-audio-evidence`; the final lyric-sync lane preserves
the original embedded-reference hash, with 17 placed and 9 unresolved lines.
The 6993-byte bridge and manifest were produced without an audio-caption
authority override. This verifies the corrected helper path, not timing
acceptance. A lead-only acoustic-target ablation is now testing whether inline
backing lyrics are being incorrectly forced as sequential speech; supplied
display text and standalone parenthetical vocals remain intact. No such
acoustic policy change is in production yet.


## Suspend recovery and lexical confidence repair

The resumed process is the original thread `01a073c0-9065-72d0-926d-adc272e87fa1`.
Inspection of process working directories and open transcript descriptors found
one Musializer Codex writer; the other app servers belong to separate projects.
No duplicate was killed. The completed primary-voice and voice-parts experiments
were retained, and no stale analysis subprocess remained.

The primary-only acoustic target completed all ten tracks. It increased placed
counts on tracks 02 and 03, but the independent comparable phrases did not gain
timing agreements. Fresh audio-only Gemini checks of track 02 at 30 and 59
seconds show backing replies occurring after the lead as well as repeated lead
phrases. Removing inline parentheses would cut real vocal tails. This policy
remains experimental and is not in production.

A separate reproducible defect was found in `sync_lyrics`: the nearby-word
boundary recovery counted unrelated words as lexical matches. Against the heard
phrase "I really think so", the supplied "I'm turning Neuralese" matched only
"I", then borrowed two unrelated windows and reported confidence 1.0. This
happened with both word timestamps and phrase timestamps. Version 8 freezes
lexical hit counts before boundary-only expansion: the same case now reports
0.3333 and cannot become trusted coarse occurrence evidence. The existing
boundary-extension regression still retains its recovered start. Three negative
assertions failed before the fix; all 28 timing tests and 81 lyric helper tests
pass afterward. The cache version changed, so old confidence is not reused.

The paired local sweep retained identical audio/reference/coarse source inputs
for all ten tracks. On the subset matched to independent 18-second audio
observations, agreements increased by one each on tracks 05 and 09; the other
eight were unchanged. Numerous unmatched observations and reference lines are
still unscored. `lexical-confidence-audit.json` is a disagreement diagnostic,
not a full-track error-rate measurement or evidence of the 3% goal.

`tools/verify.sh --quick` passed all eight gates after the confidence fix. A fresh
normal helper run of track 02 in `candidate-authored-v8-runtime/` produced 16
placed/10 unresolved authored lines and the bridge/manifest. The private muted
Xvfb UI replay loaded that actual helper output, staged it with the embedded
reference identity, and applied it after the normal embedded-source confirmation.
This replay tests UI integration; it does not rerun a remote inference.
The live check also exposed stale confirmation copy claiming wording always
followed audio. It now describes short-clip timing and explicitly distinguishes
embedded lyrics from the no-sheet transcription fallback.

An all-track authored Antigravity sweep is in `candidate-authored-v8/`. It reuses
only exact-identity discovery receipts from earlier runs. Track 02's manifest
length differs from the normal helper's measured length by 21 microseconds;
those identities have different independently generated receipts. Its outputs
must not be presented as a paired comparison with the normal-helper run.


The authored sweep completed all ten tracks. Track 10 retained completed clips
through one 180-second timeout and a subsequent provider HTTP 502, then resumed
successfully. No model substitution or new login was used. The table reports
placement accounting and partial independent agreement, **not acceptance**:

| Track | Placed | Unresolved | Timing agreements / comparable | Unscored authored lines |
| --- | ---: | ---: | ---: | ---: |
| 01 | 51 | 5 | 30 / 44 | 12 |
| 02 | 14 | 12 | 2 / 9 | 17 |
| 03 | 11 | 7 | 5 / 5 | 13 |
| 04 | 60 | 0 | 35 / 50 | 10 |
| 05 | 87 | 26 | 35 / 65 | 48 |
| 06 | 38 | 4 | 20 / 38 | 4 |
| 07 | 65 | 0 | 45 / 55 | 10 |
| 08 | 24 | 1 | 12 / 17 | 8 |
| 09 | 93 | 10 | 22 / 43 | 60 |
| 10 | 23 | 0 | 5 / 10 | 13 |

The final copy change passed the full quick gate again: Rust 498 app, 1028 core,
186 runtime plus one ignored test; 383 Python tests; all eight gates. A fresh
private Xvfb capture confirms the corrected wording. The authored result reached
60 editable cues (including Potential proposals) through the real Apply flow;
playback at 35.2 seconds rendered the authored verse, with master mute and an
unresolvable private audio sink throughout. Both UI processes and their Xvfb
servers were closed. `cargo build --release` completed after the final UI change.

The next concrete boundary question is end padding. On the same fixed,
incomplete 18-second observation subset, removing the deliberate 0.15-second
caption tail raises timing agreements from 217 to 241, with no track losing net
agreements (`cue-tail-ablation-audit.json`). This is an offline ablation only:
model boundary bias and actual sustained vocal endings need fresh verification
before changing display policy. Start medians are near zero on most tracks;
end medians are consistently positive. A whole-phrase spelling deduplication
trial also merges the two "tonal/total mode collapse" observations of one
track-02 performance, but changes few other cases and remains research-only.
The broad overlapping-alternatives and authored/performed mismatch problems
are not solved, and the 3% per-track goal remains open.


## Original-crop authored boundary refinement

Twelve fresh short recordings (6–8 seconds, paired crop offsets 0.65 seconds
apart) examined six late-ending examples without supplying lyric text or
candidate timestamps. Four complete phrase comparisons support the refined
endpoints below; the other two could not be matched unambiguously because the
model omitted words or combined phrases. Receipts retain audio/crop/prompt/model
identity in `short-boundary-trial/`; `short-boundary-comparison.json` records
both the usable comparisons and the missing ones. These are model observations,
not human adjudication.

The checks exposed more than padding. In track 06, one original 30-second crop
returns "I stitch them together from the cold dead air" as one phrase; the
overlapping crop returns its two halves. The one-to-one cross-crop merge keeps
all three. The acoustic path then places "air" into the next vocal phrase.
The supplied words can instead be matched separately against each original
crop, before its phrase division is lost in the merged representation.

`tools/authored_audio_boundaries.py` now performs this bounded refinement in
the opt-in authored route. It matches one supplied line against up to four
contiguous observed phrases per crop, requires both normalized outer words and
at least 85% token similarity, refuses cropped/uncertain observations, and
requires two distinct crop intervals agreeing within 0.5 seconds at both edges.
Their boundaries and the existing cue supply a median. The existing cue must
already be within three seconds of that occurrence. Competing uses of one
performance and newly reversed authored ordering decline refinement. It does
not promote unresolved lyrics or replace supplied wording. Original acoustic
word timings remain diagnostic evidence; the new boundary evidence records
what changed. Review links follow the new cue while retaining original warnings.

Adapter version 8 retains validated original crop rows in its local source
artifact; the discovery prompt/cache identity is unchanged and the sheet still
never leaves the computer. Localization policy version 6 invalidates old final
alignment caches. The support bundle and lyric-sync schema include the new
helper and audit fields. Eleven focused controls cover alternate phrase
divisions, edge coverage, duplicate crops, clipped/uncertain evidence, distant
occurrences, competing and distinct repetitions, ordering, review links and
schema fields. Restricting matching back to single phrases fails the regression
as a negative control.

The actual production path completed all ten tracks using exact matching cached
receipts, without new audio requests. On the fixed independent 18-second subset,
agreements increased from 211 to 257 of 336 comparable authored lines: **46 gains,
zero losses**. Per-track agreements are 35, 5, 5, 39, 44, 32, 51, 15, 25 and 6.
There are still 195 unscored authored lines; unmatched performances and unresolved
coverage remain open, so this is not a full-track error-rate claim. Placed and
unresolved counts are unchanged by this boundary-only stage.

The fresh short-crop checks place the four usable example endings at:

| Phrase | Previous cue end | Refined end | Fresh observations |
| --- | ---: | ---: | --- |
| track 01, box/question | 26.431 | 26.100 | 25.770, 25.880 |
| track 04, drifting/noise | 35.215 | 34.950 | 34.730, 34.780 |
| track 06, cold dead air | 73.753 | 72.950 | 72.730, 72.880 |
| track 06, cage of light | 98.982 | 98.820 | 98.600, 98.550 |

The global 0.15-second caption-tail policy remains unchanged; direct crop
corroboration gives stronger evidence for these individual boundaries than
removing padding everywhere. The full quick gate passed (394 Python tests;
Rust 498 app, 1028 core, 186 runtime plus one ignored; all eight gates), and the
release build completed. A normal helper run of track 02 preserved the embedded
sheet and refined "Read a book..." to end at 69.900 seconds. Its different
measured-duration cache identity is still kept separate from the manifest sweep.

The normal-helper result also passed the private muted Xvfb replay through
Start, staging, embedded-source confirmation, Apply and playback. Captures at
67.944 and 71.263 seconds show the refined lyric and its following line; evidence
is in `ui-authored-fusion/`. The test application and Xvfb server were closed.
The remaining acceptance work is performed-occurrence coverage, including
extra/repeated sections and inline backing words omitted by the recording.

## Recovering missing authored occurrences

A separate pass now recovers an unresolved authored line when original audio
crops locate one unique complete occurrence between existing neighbouring
placements. It requires at least three words, matching first/last words, 90%
token similarity and two distinct crops agreeing within 0.5 seconds at both
edges. A competing single-view occurrence or endpoint remains an ambiguity;
the number of overlapping crops cannot choose which repetition was performed.
Shared source rows, an existing nearby cue with the same words, and reversed
authored order prevent recovery. The accepted cue preserves the supplied text,
stays uncertain/marked for review, and retains the full rejected acoustic record
in `audio_occurrence_evidence`. The unresolved ledger, review links and counts
are rebuilt, as are Potential proposals outside authored coverage.

Localization policy version 7 invalidates final alignment caches. Nine recovery
controls cover retained rejection evidence, both competing boundaries, duplicate
and uncertain crops, neighbours, existing/competing owners, order and schema
fields. A negative control allowing a 1000-second disagreement instead of 0.5
fails both ambiguity tests without changing a source file. The normal controls
then pass.

All ten production runs completed with matching cached audio receipts in
`candidate-authored-recovery-production/`. They recover eight lines: one each in
01, 02 and 09, three in 05, and two in 06. All previous placements remain
byte-for-byte equal. Placed/unresolved counts are now:

| Track | Placed | Unresolved |
| --- | ---: | ---: |
| 01 | 52 | 4 |
| 02 | 15 | 11 |
| 03 | 11 | 7 |
| 04 | 60 | 0 |
| 05 | 90 | 23 |
| 06 | 40 | 2 |
| 07 | 65 | 0 |
| 08 | 24 | 1 |
| 09 | 94 | 9 |
| 10 | 23 | 0 |

Seven recovered lines have comparisons in the unchanged independent 18-second
subset, and all seven agree within the tolerance. Agreements rise from 257 to
264 of 336, with zero losses and the same 195 unscored authored lines. The
track-09 recovery remains unscored there. Fresh short-crop checks for track 03
disagree on the wording of its proposed phrase, so that ambiguous occurrence
remains unresolved. The fresh track-09 checks give only one complete observation
of the recovered phrase, so they do not establish an exact timing reference.

The separate frozen phrase-reference score is deliberately retained too:
recovery changes error counts 53→52, 51→52, 67→67 and 47→49 for 01, 02, 03
and 06. Inspection shows that the newly counted extras in 02 and 06 are full
authored sentences whose words the frozen reference split into two phrases.
For example, it separately times "Born in the prompt" and "dying in the night";
the recovered authored line spans both. These strict one-to-one scores expose
the grouping mismatch; neither this score nor the incomplete authored subset
establishes the user's full-track error rate. Both audit files are preserved.

The real track-02 helper produced 17 placed and nine unresolved authored lines
using its separately measured-duration receipt identity. Its recovered
"I really think so (I really think so.)" spans 61.625–64.125. Private muted Xvfb
replay confirms Start, staging, embedded-source confirmation, Apply and playback
at 61.935 seconds; the cue appears as ambiguous and renders in Cadence. Evidence
is in `ui-authored-recovery/`; its app and Xvfb processes are closed. All eight
quick gates pass (403 Python tests; Rust 498 app, 1028 core, 186 runtime plus
one ignored), and `cargo build --release` completes.

## Gemini review: performed instances versus written lines

The official ACP runtime, with `gemini-3.8-flash-high` selected and tools denied,
reviewed the pipeline alongside a fresh 20-second track-02 clip. Its receipt is
`gemini-pipeline-review.json`. No lyric sheet or candidate times were supplied.
It identifies separate alternating-voice performances of the repeated title
phrase and "Read a book once in a while", and recommends separating reusable
authored wording from the number of performed occurrences. This is the next
architecture hypothesis, not a validated implementation.

Two parts of that recommendation are explicitly not adopted: harmonic energy
is not sufficient evidence of singing rather than instruments, and a comparator
that requires correct timing before matching would count a mistimed line as
both missing and extra. The user's metric counts that line once. Overlapping
voices with identical text remain a risk even with explicit event instances.

`performed_template_trial.py` searches all ten tracks using complete authored
lines as reusable local templates, without their one-use/order constraint. It
finds 13 additional possible occurrences, retaining conflicting ownership and
existing placement coverage. These are research-only proposals. Fresh audio-only
short-crop checks are being used to challenge the repeated track-02 phrases and
the late track-05 line before any change to normal Assist.

Those eight short-crop requests completed. One response restarted its JSON
mid-stream; it is retained as invalid rather than silently repaired. A single
fresh retry returned valid JSON. Seven proposed occurrences have supporting
new short-crop observations in `performed-template-fresh-comparison.json`,
including both late title repetitions, two repeated-book pairs and the late
track-05 fire line. Nearby identical phrases appear as multiple candidate
matches in that artifact and require ordered occurrence assignment; simply
averaging all matching timestamps would collapse real repetitions. The 13
proposals remain outside normal Assist. This result supports pursuing performed
instances, not relaxing unresolved authored-line order guards globally.

## Numbered voice labels are structure

Inspection of the track-05 sheet found `Voice 1:` through `Voice 8:` still in
matching tokens, acoustic targets and displayed captions. A consecutive block
with distinct numbered voice labels now keeps only its authored words in those
three places, retaining the original row as `source_display` and preserving its
source index. A lone label, quoted text, ordinary "Voice of..." wording and
labels inside bracketed production prose retain their prior handling. The
coarse synchronizer version is 9, invalidating its caches and consequently
their dependent final alignments.

Three regressions pin the shared matching/acoustic/display words, source
retention and the protected cases. A ten-track scope comparison confirms
exactly eight changed classifications in 05 and none elsewhere. The actual
track-05 run in `candidate-authored-voice-labels/` places 93 lines with 20
unresolved. Against the *unchanged* independent subset, agreements rise from
47 to 48 with no losses. Three formerly missing reference comparisons now have
placements: one agrees and two still disagree, so the count increase is not
claimed as three accurate recoveries. Nearby global assignments also move,
including the two "GRADIENT DESCENT" lines; their evidence remains inspectable
in the before/after artifacts. All eight quick gates pass with 406 Python tests,
the updated document validates against the full JSON Schema, and the release
build completes. The prior 11 recovery documents also passed full schema
validation. Complete performed-line acceptance remains open.

## Additional performed instances in normal Assist

`tools/authored_audio_occurrences.py` now separates reusable authored wording
from the number of performances. Two original complete audio crops can propose
an additional occurrence outside existing placement coverage; two fresh short
audio-only crops must independently corroborate its words and both boundaries
within 0.5 seconds. Confirmed instances use the median of those observations.
Source-row ownership prevents duplicates, including two proposals consuming the
same fresh repetition. The original authored `lines`, `unresolved`, `unmatched`
and `review_flags` arrays remain unchanged. The new `performed_occurrences`
array carries supplied wording, source-line template indices and original/fresh
evidence; it does not claim to resolve the sheet's ambiguous occurrence order.

The existing bounded observation/cache function is shared by discovery and this
confirmation stage. Consent and model validation precede requests, and audio,
runtime, harness, prompt, proposal and crop identities bind cache reuse. Sheets
still stay local. A normal authored opt-in run reaches confirmation after local
alignment, including when that alignment was cached. Its final bridge, section
reentry detection and manifest count the combined rendered lane. The UI gives
the new uncertain cues their own `ADDED occurrence` review identity; they are
displayed after Apply rather than parked as Potential. Unrepresented performed
proposals are rebuilt against the combined lane.

Twenty new short-clip requests checked 13 proposals across the ten tracks.
All ten runs completed in `candidate-authored-occurrences-production/`:

| Track | Proposed additions | Confirmed additions |
| --- | ---: | ---: |
| 01 | 2 | 1 |
| 02 | 7 | 7 |
| 03 | 1 | 0 |
| 04 | 0 | 0 |
| 05 | 2 | 1 |
| 06 | 0 | 0 |
| 07 | 0 | 0 |
| 08 | 0 | 0 |
| 09 | 1 | 1 |
| 10 | 0 | 0 |

All ten schema validations pass, as does byte-for-byte preservation of each
authored ledger. Ten added performances are real additional renderable cues,
not a count increase from parking unresolved guesses. The Groyper proposal
remains rejected. `performed-occurrences-production-audit.json` records the
candidate/reference digests, counts and fixed-reference comparisons.

The full-lane audit and fixed-reference scorer now include additional instances.
A new freeze control refuses an audit omitting one. The existing monotonic
comparator also exposed an occurrence-assignment defect: an unrelated misplaced
cue could force later identical phrases onto much earlier repetitions. A
maximum-cardinality, minimum-cost bipartite assignment now permits a misplaced
unique line to match without displacing the other repetitions. Text still
determines eligibility; the timing tolerance never does, so a badly shifted
unique line counts once. Forty-eight small rectangular cases match exhaustive
assignment costs, and the crossed-order regression pins repeat ownership.
The new matching policy is `bipartite-phrase-0.72-v2`; both before and after are
rescored against the unchanged frozen reference. Prior v1 diagnostics survive.

Those diagnostic error counts change 52→51 for 01 and 52→49 for 02; 03 and
06 are unchanged. The frozen references still split some full authored lines,
including repeated-book pairs, so these counts remain diagnostic rather than
the final acceptance rate. The unchanged authored-index subset still has 265
agreements among 336 comparable lines and 195 unscored authored lines; it does
not measure these additional instances. An optional question about the scoring
unit is pending; absent a different preference, use authored line breaks where
available, with separate performances counted separately.

The actual normal helper run of 02, retaining its distinct measured-duration
identity, confirms five additions and produces 22 renderable cues plus nine
unresolved authored entries. Fresh requests and results are in
`candidate-authored-occurrences-runtime/`. Private muted replay through Start,
staging, embedded-source confirmation, Apply and playback shows the added title
phrase, repeated-book line and later title repetitions. The initial UI capture
exposed a redundant Potential copy of the book line. The final UI keeps its
unresolved sheet entry in review, but does not park a duplicate when both the
additional-occurrence metadata and an actual displayable bridge cue match its
words/window. Metadata alone cannot suppress a proposal. The counts explicitly
distinguish that case from a proposal with no usable time.

Final evidence is in `ui-authored-occurrences-dedup/`: 66 editor cues comprise
22 renderable and 44 Potential cues; one guess is already represented, and two
have no usable timing. Captures at 123.870 and 165.350 seconds show the book line
without its duplicate and the separate second title repetition. Both test apps
and Xvfb instances are closed. Occurrence, UI and reference controls and the
full quick gate pass: 419 Python tests, Rust 499 app,
1029 core and 186 runtime plus one ignored, all eight gates. A negative control
allowing a single crop fails both original-crop and fresh-confirmation guards.
The release build completes. Clippy reports only the existing MSRV warning in
the parallel UI work's `widgets.rs:362`; this stream adds no Clippy warnings.
Omitted backing words, unconfirmed alternatives and complete per-track reference
coverage remain open.

## Conditional main phrases and performed-phrase scoring

The operator confirmed on 2026-09-07 that distinct performed phrases are the
scoring unit: an audible lead and an audible echo count as two lines. This
supersedes the earlier provisional authored-line grouping. A merged cue cannot
earn two successful boundary checks without separate observed phrase times.

Occurrence policy 2 lets an unresolved authored line with a trailing inline
reply supply its main wording as another local template. The same two original
and two fresh audio crop requirements apply. A complete authored template owns
identical words, and a complete audible call-and-response owns its contained
lead. Without this precedence, the initial trial made the fragment and complete
cue compete for the same source rows and discarded both. A regression test now
pins that case, along with full placed-cue containment and preservation of the
unresolved sheet. Standalone backing and interior parentheses are not stripped.
Added fragments carry `text_scope: authored_inline_lead`; existing placements
are never shortened. Confirmation request policy remains version 1 because
the audio-only prompt and protocol are unchanged.

All ten `candidate-authored-inline-lead/` runs completed. Track 02 has nine
additional occurrences versus seven before, including corroborated main phrases
at 18.885–20.895 and 166.215–168.025 seconds. The other nine tracks retain their
results. All authored `lines`, `unresolved`, `unmatched` and `review_flags` arrays
are unchanged. Ten artifacts validate against the schema. Evidence is in
`performed-inline-lead.log` and `performed-inline-lead-audit.json`.

The frozen phrase matcher now treats a written hyphenated syllable stutter as
part of its completed word. Whole-word repeats and separate words are retained;
the test caught an initial implementation collapsing `Go-go-go`. No timestamps
are changed: omitting the initial stutter phoneme still fails the boundary
test. Policy `bipartite-performed-phrase-0.72-v3` is applied to both sides of this
comparison. The fixed model-reference diagnostic for 02 improves from 49 to 47
errors against 48 proposed phrases; 01/03/06 stay at 51/67/49. These are still
unadjudicated diagnostics, not accepted accuracy measurements. The frozen
references themselves are unchanged; six tracks still lack complete references.

The distinct normal-helper source identity produces eight additions and 25
renderable cues, with nine authored lines unresolved. The artifact validates.
Private muted Start, confirmation, Apply and playback are captured in
`ui-authored-inline-lead/`: 66 editor cues include 25 renderable and 41 Potential
cues, one guess already represented, and two without usable timing. Playback
captures at 19.625 and 121.021 seconds show the recovered main phrases. The app
and private Xvfb are closed. Final quick verification passes all eight gates:
426 Python tests and Rust 499 app / 1029 core / 186 runtime plus one ignored.
The release build completes; the previously recorded unrelated Clippy MSRV
warning remains outside this stream.

The performed-phrase requirement also led to a separate, unpromoted split
trial. Four existing merged call-and-response cues in 02 have independently
observed component boundaries in two original crops. Splitting those improves
the frozen phrase diagnostic from 47 to 35 errors. Six fresh short audio-only
requests confirm both components for three of those four splits; the last
echo remains unconfirmed. See `performed-phrase-split-trial.json` and
`02/performed-phrase-split-audio/summary.json`. These research outputs have not
changed the production lyric lane. Integration must preserve authored review
identity, count actual rendered phrases, and retain the unconfirmed case.

## Performed phrase splits in normal Assist

`tools/authored_audio_phrases.py` now implements that split stage after
additional-occurrence confirmation. Each component needs two original crop
observations plus two fresh short audio-only observations agreeing on both
boundaries within 0.5 seconds. Components cannot borrow each other's source
rows, and competing source cues cannot claim the same performance. The stage
does not distribute words uniformly through a compound cue's duration.

The `phrase_splits` array links each confirmed pair to an unchanged source cue
by array name, zero-based index and canonical digest. The authored `lines` and
`performed_occurrences` arrays remain intact. `rendered_rows` validates those
links and substitutes the components; stale or duplicate links fail rather
than silently replacing another cue. Rendering, scene reentries, bridge output,
manifest counts and research scoring all consume the same expanded view.
Separate `PHRASE` review entries preserve the authored review identity. Legacy
audit correction refuses these structured artifacts because its row numbers
cannot safely rewrite this representation.

All ten `candidate-performed-phrases/` runs and schemas pass. Track 02 applies
three of four proposed splits: the title pair near 62 seconds and book pairs
near 67 and 124 seconds. The unconfirmed final book pair stays intact. Its
renderable count rises from 24 to 27 and its unchanged frozen model-reference
diagnostic improves from 47 to 38 errors; the other tracks retain their results.
`performed-phrases-production-audit.json` verifies that all authored and
additional-occurrence ledgers are unchanged. The pilot's exact matching audio
receipts were reused, with no redundant audio request for that corpus run.

The normal measured-duration source requires its own requests and confirms
three splits from four fresh clips. `candidate-performed-phrases-runtime/` has
28 renderable cues and nine unresolved authored entries; its schema validates.
Private muted Start, embedded-source confirmation, Apply and playback are in
`ui-performed-phrases/`. The 69 editor cues comprise 28 renderable and 41
Potential cues. Captures at 122.910 and 124.600 seconds show the book lead and
reply as separate active cues. Their full compound proposal is not duplicated:
both component captions must actually exist in the lyric lane before that
guess is considered represented. Metadata alone and an absent or mistimed
reply cannot suppress it. The private app and Xvfb are closed.

Nine Python controls cover bridge/count integration, consent, native/fresh
evidence, missing replies, collisions, source identity and resumed requests.
Rust controls pin separate review identities and actual-cue duplicate
suppression. An isolated negative control admitting one original crop fails
the split evidence test. The complete quick gate passes all eight checks:
439 Python tests, Rust 500 app / 1030 core / 186 runtime plus one ignored.
The release build completes; only the existing unrelated UI MSRV warning
remains in Clippy.

`tools/lyrics_research/performed_reference.py` starts the next evidence phase:
fresh, full-coverage audio-only phrase observations, without either authored
words or candidate captions in the prompt. Its immutable reference proposal
retains uncertain/clipped phrases and ambiguous scoring-interval ownership
separately, with full request and clip provenance. It cannot claim adjudicated
accuracy. Coverage, source identity, raw/parsed consistency and uncertainty
retention have four controls. Track 02 is the pilot before extending this pass
to the tracks whose complete performed-phrase reference evidence is missing.

The audio-only 02 pilot completed with 64 proposed performed phrases and seven
explicit uncertain/clipped/interval-ownership cases. Its current-candidate
diagnostic has 45 missing, eight extra and three timing disagreements. This
reference has a different, independently heard phrase inventory from the older
candidate-worded audit; do not compare their rates as a before/after result.
The new observations expose short ad-libs and vocal chops that the authored
template path leaves as Potential proposals or misses. They are still model
proposals, not adjudicated truth. Receipts, frozen output and the diagnostic
are under `02/performed-reference-audio-v1/`. The same audio-only pass is now
running for the remaining nine tracks through `performed_reference_all.py`,
with per-track outcomes in `performed-reference-batch-status.json` and progress
in `performed-reference-all.log`. Consult the live process before retrying;
the batch resumes completed clip receipts and owns its ACP cleanup.

## Audio-observed additions and the quota checkpoint

`tools/observed_audio_occurrences.py` adds audible words outside the authored
reusable templates, including short ad-libs. It requires two distinct complete
original crops and two fresh audio-only crops agreeing on wording and both
boundaries within 0.5 seconds. Existing captions own contained words and their
source observations. Additions explicitly use `text_scope: audio_observed`,
have no authored line indices, remain uncertain, and receive separate `AUDIO`
review entries. The original authored and additional-template ledgers and
canonical compound-split links remain intact. Resume replaces only this stage's
own additions. The normal helper invokes it after authored occurrences and
compound splits; schema, bridge, scene reentries and rendered counts share that
representation.

Completed corpus runs add three phrases on track 01 (53 to 56 renderable cues)
and two on track 02 (27 to 29): the repeated “Turn it” phrase near 34 seconds
and “Woo” near 44 seconds. The independent audio-only model-reference diagnostic
changes from 57 to 55 errors on 01 and 56 to 54 on 02. These are not adjudicated
accuracy figures. `observed-performances-completed-audit.json` verifies schemas,
unchanged authored ledgers and existing occurrence prefixes, and intact split
links. `ui-observed-corpus/` is a private muted replay of the completed corpus
artifact; its captures show both new track-02 captions active during playback.
It is explicitly a corpus replay: the separate normal measured-duration job
in `candidate-observed-performances-runtime/` reached the new stage but did not
finish its own confirmation requests before the quota limit.

Eight new Python controls cover short words, original/fresh evidence,
contradictions, containment, source-row ownership, stutters, consent and
idempotent resume. The Rust parser distinguishes audio wording from an authored
additional occurrence. Full quick verification passes eight gates with 448
Python tests, Rust 500 app / 1031 core / 186 runtime plus one ignored. The
release build completes. Only the existing unrelated UI MSRV warning remains.

The provider eventually returned an explicit `Usage Limit Reached` response
with a reset duration of 56 minutes and 5 seconds. Empty responses immediately
before it had been incorrectly retried as malformed JSON. The adapter now
recognizes both conditions and stops with a resumable error, preserving valid
clip receipts. The two research batches were stopped and their ACP children
reaped. The owned scheduler `resume_audio_after_quota.py` waits until
2026-09-07 16:10:35 UTC (one minute beyond the reported reset) before resuming
independent references and observed additions. Its `quota-resume-status.json`
and live PID are authoritative; do not start duplicate batches. Reference
proposals for tracks 01 through 04 are complete; later tracks remain pending.

Four tiny Gemini crops around track 02's vocal chops disagree on both the
word (“so” versus “go”) and repetition count. Locally cached Demucs separation
and Qwen3-ASR on the same mix/stem intervals did not resolve those differences.
Neither experiment is promoted. A separate end-minus-0.15-second ablation on
MMS-only authored cues has 16 gains but three regressions in the older matched
subset, including a regression on track 02. It is also not promoted. Evidence
is in `02/chop-micro-audio/`, `qwen-trial/chops/`, `vocal-trial/02-145/` and
`tail-calibration-trial.json`.

An offline single-view enrichment trial reused exact older audio-only
confirmation receipts on eight tracks, refusing every uncached audio request.
Reintroducing those observations before authored localization changes many
placements: it recovers some lines but loses others and worsens the available
independent diagnostics on tracks 02 and 03. This research variant is not in
production. `enriched-discovery-paired-audit.json` compares authored arrays on
both sides, excluding additional occurrences and compound splits equally.
The existing caption-replacement baseline also has fewer model-reference
missing phrases, but loses supplied wording authority; its apparent coverage
gain is not a reason to silently replace the authored route. Complete reliable
performed-phrase references and the final per-track target remain open.

## Ordinary-sentence seams and continuous chop runs

Phrase policy version 2 extends the existing two-part representation to
ordinary written sentences. A complete original crop must identify the existing
cue and put a supplied-text prefix at one of its actual reported phrase seams.
Both components still require the same two-original/two-fresh confirmation.
Written punctuation alone cannot split a cue, and multiple supported seams
remain an unresolved grouping choice. Quoted components retain balanced quotes;
source arrays and hashes remain unchanged. The offline corpus produces eleven
new proposals: one on 03, six on 06, one on 08 and three on 10. The private
prototype and production proposal arrays agree exactly. Fresh confirmation is
pending the scheduled provider retry. The audio request protocol remains
version 1 so unchanged prior requests can reuse their exact receipts.
Twelve phrase controls and all eight quick gates pass (451 Python tests), and
the release build completes.

The operator clarified the metric again: **one continuous rhythmic vocal chop
run is one phrase**, while separately performed lead lines and echoes remain
separate phrases. The v1 reference pilot split the “so” run into individual
retriggers, so its old denominator is not the final acceptance denominator.
`performed_reference.py` now preserves that immutable v1 evidence and writes a
separate `performed-reference-audio-v2/reference.json`. Closely spaced short
repeated rows nominate intervals for another audio-only listen with the explicit
run/echo distinction; this detector does not merge or retime anything itself.
The model can retain distinct calls, group a continuous run, or mark grouping
uncertain. Complete original coverage is retained, with the actual original or
revised request identity recorded for each interval. Revised receipts are not
relabeled as original requests. Seven reference controls pin coverage, unchanged
source evidence, grouping selection and mixed-request provenance. The queued
reference batch includes track 02 again so its final metric uses this rule.


## Completed ten-track checkpoint after the provider reset

This checkpoint supersedes the pending/quota status above. Both scheduled
batches completed and were reaped. All ten immutable v1 references and separate
chop-run-aware v2 reference proposals have complete coverage; the audit rebuilt
parsed observations from their raw responses and checked request identities,
interval ownership and original audio hashes. These remain model proposals,
not an adjudicated acceptance denominator.

The current production corpus is `candidate-apostrophe-normalized/` per track.
All ten outputs validate, including canonical rendered links. The general seam
stage confirmed five of eleven new proposals; the observed audio stage added
25 corroborated phrases across the corpus. Existing authored ledgers remain
intact. Renderable counts in track order are 56, 29, 13, 61, 103, 42, 66, 25,
101 and 26. `normalized-ten-track-audit.json` retains missing, extra and timing
disagreements separately. None of these runs establishes the 3% target.

A concrete normalization defect affected curly apostrophes: matching and MMS
could tokenize `I’ve` differently from `I've`. Both paths now normalize the
right, left and modifier apostrophes after Unicode normalization, while leaving
displayed wording unchanged. Relevant aligner/localization/adapter identities
were bumped. The paired audit has three authored placement changes on 04 and
two on 09; all other authored timings are identical. Track 09 required twelve
fresh confirmation clips after its placement coverage changed; those completed.
The eight quick gates pass with 455 Python tests, and the release build is
current (`verify-apostrophe.log`, `release-apostrophe.log`).

The actual normal track-02 helper, using its measured audio duration, completed
with 30 renderable cues and nine unresolved authored lines. Its artifacts are
`02/candidate-observed-performances-runtime/`, distinct from the corpus run.
A private muted replay exercised Start, staged Apply, embedded-lyrics
confirmation, the lyric panel and playback. The final screenshots at 34.732 and
44.119 seconds show the parent captions; their filenames do not establish that
Turn it/Woo were active in this normal-run result. The application and private
Xvfb were closed. The separate track-03 corpus replay did show the confirmed
ordinary-sentence split through Apply and playback.

## Performance-led inventory and independent reference conflicts

A research-only alternative lets corroborated audio determine the performed
inventory before local acoustic refinement. All ten inventories completed in
`candidate-performance-first-audio/`. Counts are 68, 44, 34, 74, 112, 57, 63,
38, 85 and 36. Against the same v2 reference proposals, total model disagreement
counts are 37, 21, 43, 32, 70, 17, 10, 29, 37 and 26, respectively. These are
neither error percentages nor acceptance results. The production authored route
has larger disagreement counts on these runs, but a smaller model disagreement
count alone does not justify replacing supplied caption authority.

`performance-first-wording-trial-v2-audit.json` compares local authored-fragment
restoration against those exact inventories. Exact token matches preserve the
diagnostics. Fuzzy restoration improves one track by one disagreement and
worsens another by one, and can replace actually heard variants with intended
sheet words. It is not promoted. `confirmed-audio-ctc-trial.json` holds a separate
fixed-inventory boundary ablation: replacing fused boundaries with raw or
bounded CTC boundaries regresses on the eight completed comparisons. No timing
variant from that experiment is promoted.

Gemini 3.8 Flash also listened independently to seconds 88–112 of Floor
Mechanics and provided methodological advice (`gemini-method-review-07.json`).
It reported “that's the only job”, an event omitted from the frozen confident
reference, and explicitly marked “I am ... present” grouping as ambiguous.
This is additional model evidence, not adjudication. A fixed-policy vocal-envelope
boundary ablation on all ten cached local stems also regresses on most tracks
(`vocal-envelope-edge-trial-audit.json`) and is not promoted. The
reference remains frozen: conflicting evidence is retained instead of editing
its denominator to make a candidate pass.


## Merge trust regression and reference ownership resolution

A three-crop regression control proves that a later, more central observation
could erase an earlier wording disagreement and restore false certainty. The
adapter now preserves `observation_disagreement` and its review uncertainty
through subsequent merges. Adapter version 10 invalidates derived transcript
caches; raw audio request identities remain unchanged. The negative control
fails on the prior implementation and passes with the fix. An exact-receipt
all-ten audit shows no changes to this corpus's wording, timing, counts or
observation links; the defect is in a valid merge path absent from these saved
observations. Evidence: `sticky-disagreement-negative.log` and
`sticky-disagreement-corpus-audit.json`.

The near-identical-event wording-merge experiment recovers no track-02 phrase
and regresses track 01, so it is not promoted
(`near-identical-wording-merge-trial-audit.json`).

A separate audit found all four apparent extra Floor Mechanics phrases already
in the reference uncertainty ledger, solely for scoring-window ownership. The
new `resolve_ownership` reference operation requires a unique complete adjacent
crop with identical normalized words and both boundaries within 0.5 seconds.
It refuses wording uncertainty, partial views, competing instances, duplicate
matches and an already-owned reference row. It rebuilds the frozen source from
its raw audio receipts before resolving anything, consumes no candidate data,
and retains every original observation. Nine reference controls pass.

The separate immutable `performed-reference-audio-v3/reference.json` files
resolve 17 ownership cases across the corpus. Resolution counts by track are
3, 1, 0, 0, 4, 1, 4, 1, 2 and 1. The remaining uncertain counts are 8, 6, 11, 5,
7, 3, 0, 6, 8 and 4. Current authored-candidate model disagreements against v3
are 53, 37, 55, 34, 73, 39, 25, 32, 82 and 38; the research audio-led candidate
has 35, 20, 43, 32, 67, 16, 6, 28, 36 and 25. Candidates were unchanged in this
comparison. All are model disagreement counts, not adjudicated error rates.
`reference-ownership-v3-audit.json` retains complete pair assignments. Previous
v2 reports remain available and must not be mixed with v3 for before/after
claims. Grouping/wording uncertainty and full per-track acceptance remain open.


## Exact local spelling for the performance-led inventory

`tools/lyrics_research/performance_inventory.py` now projects an audio-derived
review lane onto unambiguous exact normalized-token fragments of the local
written sheet. It never adds/removes an event, changes a timestamp, overrides
uncertainty, or uses fuzzy similarity to substitute an expected word. The
source retains the original audio wording, classified reference text and
character spans, the complete local reference/hash, and the input artifact
hash. Ambiguous spelling and unrecognized performed words keep audio wording.
Review-flag text follows any accepted spelling change. Four controls cover
repeated performances, unchanged evidence/times, rejection of intended-word
substitution, production directions, ambiguity and source validation.

All ten projected review lanes validate against the production schema in
`candidate-performance-inventory-local/lyrics.aligned.json`. Their word-match
counts are 47, 4, 13, 53, 65, 50, 55, 26, 46 and 19. Complete candidate/reference
pair assignments and disagreement counts against v3 are identical before and
after this projection, as expected for spelling-only changes. Timing, counts
and observation evidence are identical. The actual API/schema projection is
recorded in `performance-inventory-local-audit.json`; the earlier fuzzy trial
is not its implementation. This remains a research entry point, not a changed
default in normal Assist. Normal Assist integration still needs an explicit
inventory choice and matching UI/review behavior. The written-lyrics localizer
continues to be the normal authored route until that work is complete.


## Normal Assist performed-inventory option

The spelling projector is now `tools/local_lyric_spelling.py`, included in the
runtime support bundle; the research entry point delegates to it. An explicit
saved `local_runtimes.performed_lyrics` boolean (default false) is available at
the top of AI settings → Local models. The Rust launcher passes
`--performed-lyrics`. Only an Antigravity route uses this choice. It transcribes
and confirms the audio-led inventory without sending the written sheet, runs
the local per-phrase aligner, and projects exact local spelling into the separate
`lyrics.performance.json` artifact. The existing written-sheet route remains
the default and has its own orchestration regression control. The new control
proves performed mode bypasses the written-line localizer. Settings persistence
and the actual helper argv are also tested.

The actual track-02 normal helper completed with 44 renderable phrases and
three exact local embedded-sheet spelling matches. Its manifest identifies
performed inventory and local spelling use; the output validates against the
production review schema. Against the same v3 model reference, the prior normal
written-sheet result has 38 disagreements and this normal performed result has
24 (11 missing, 8 extra, 5 timing). This is not an adjudicated accuracy claim.
One initially running helper had loaded the manifest bug before its fix; it
finished its audio requests, failed at manifest construction, and the corrected
retry completed using those saved receipts. No audio batch was duplicated.

The private muted UI test toggled and saved the setting off, reopened it,
enabled and saved it, and verified the actual helper argv contains the flag.
The Start screen describes performed inventory. The completed normal result
then passed staged review, embedded confirmation, Apply and playback; the
34.432-second capture displays the separate “Turn it…” phrase. Evidence is in
`ui-performed-option/`; captures 06, 09 and 10 establish the actual toggle
states (05 was a misplaced click before accounting for a doctor notice).
The app and Xvfb were closed. The replay checks the launcher/staging path using
the completed actual helper artifact; it does not claim another remote UI run.

All eight quick gates pass: 463 Python tests, 500 app tests, 1032 core tests,
186 runtime tests plus one ignored. The release build completes. The all-ten
actual-helper batch is now running via `performed_runtime_all.py`, using exact
available caches and normal measured duration. `performed-runtime-active.json`
and its live child PID identify the current job; `performed-runtime-all-status.json`
records completed tracks. Do not start a duplicate. Full per-track accuracy
acceptance remains open.


## Phrase matcher version 5

Two regression controls expose scoring-only defects: spaces around written
stutter hyphens made `To- to- to- total` fail to match `To-to-total`, and the
transcribed retrigger count made two whole single-word chop runs fail lexical
matching. The v5 comparator normalizes those forms only for matching. It never
merges rows, changes timestamps, or changes the denominator. A shortened run
still receives one timing error, additional separately emitted rows remain
extra, and separate lead/echo rows remain distinct. The old implementation
fails both controls (`chop-metric-negative.log`); all twelve comparator controls
pass with v5.

`matching-policy-v5-audit.json` re-scores unchanged corpus candidates against
the same v3 reference files. The authored corpus is unchanged on all tracks;
the audio-led corpus changes only track 08, from 28 to 26 model disagreements.
The actual normal track-02 performed result changes from 24 to 21. This is a
metric correction, not a detection improvement. The active normal corpus
batch loaded the preceding comparator before this edit; retain its recorded
policy identifiers and re-score completed artifacts uniformly under v5 after
the batch finishes. It must not be restarted merely to load a comparator.


## Final review artifact selection and acoustic spelling pilot

The editor review loader initially read the pre-projection aligned document
before the final performed inventory. `parse_review_manifest` now prefers the
manifest's `performance` document only when `lyric_inventory` is `performed`.
Written-sheet mode ignores a stale projection in the shared job folder. The
filesystem-level app regression changes spelling between the two artifacts,
checks both modes, and fails before the fix (`performed-review-negative.log`).
Paths still resolve by basename inside the owned job directory. Eight quick
gates pass with 465 Python tests and 501 app tests.

A research-only contrastive spelling pilot compared thirteen existing heard/
sheet phrase pairs under MMS on two padded crops each. The synthetic wildcard
channel was normalized with log-softmax before sequence-loss comparison, all
reported losses are finite/nonnegative, and the scores are not calibrated
confidence. Some choices depend on crop padding or per-token normalization;
there is no demonstrated reliable rule for promoting fuzzy sheet substitutions.
The experiment remains unpromoted (`contrastive-wording-trial.json`). It opens
no audio output device and sends no words or audio to another provider.

The separate `short_context_pilot.py` experiment is now testing track 03 with
12-second audio crops at four-second strides, scoring onsets in [6,36) against
the unchanged v3 reference. It uses the existing audio-only prompt, serial
requests, no candidate text, and requires the same corroboration rule as the
baseline. Results will be in `03/short-context-pilot/audit.json`. This is a
bounded context-length comparison; it does not change production clip size.
The actual normal corpus batch continues independently, with tracks 01, 02 and
03 complete and track 04 active at this checkpoint. Its loaded v4 matcher must
still be re-scored uniformly with current v5 after completion.


The short-context pilot completed nine audio-only requests. Against eight
unchanged reference phrases in the fixed interval, the baseline has seven
matched candidates and one missing phrase; the 12-second variant has six and
two missing phrases. Neither has a matched timing error. “Cooking in Texas”
remains missing and the shorter variant additionally loses “Southern style”.
This pilot is rejected as a production clip-size change. Its requests and
unconfirmed/partial observations remain in `03/short-context-pilot/`.


## Matched hint controls and corroboration repair

Two identical 16-second track-03 crops were evaluated with a tentative local
lyric fragment, a deliberately incorrect fragment, and an empty-hint control.
All prompts and raw responses remain in `03/lyric-hint-pilot/`. Both informed
conditions rejected the deliberately wrong northern/winter wording. The local
sheet hint did not establish a reliable recognition benefit: the fresh normal
Assist run independently recovered the same passage. The empty-hint views both
heard the Texas phrase, but one wrote “Cooking” and the other “Cookin’”; the
whole-token corroborator dropped both three-word observations. The matched
clip SHA assertions passed. This was a bounded research experiment under the
original model/method exploration request, not an additional user opt-in to
normal sheet transfer. Normal Assist still sends audio alone and keeps the
sheet local.

Six offline Qwen ASR runs used the same two crops with no hint, the local
fragment, and a wrong fragment. The local hint changed the intended word but
also changed unrelated “brother” to “rumor” in one crop. Neither remote nor
local hinting is promoted as an automatic spelling corrector. These artifacts
are in `qwen-trial/hints/`; they are recognition evidence without timestamps.

A broad single-character spelling fallback was tested offline and discarded
in favor of explicit apostrophe-marked dropped-g equivalence. Adapter v11 now
allows matching “Cooking in Texas” with “Cookin’ in Texas” when normalized
phrase context agrees and both observed edges are within one second. It does
not alter displayed words. The row remains uncertain and retains the wording
disagreement and both raw observations. Single words, unmarked substitutions,
different words, distant edges and two deliveries from one view do not gain
this corroboration. The positive regression failed before the fix; all 27
adapter tests pass afterwards. Saved ten-track observation replay adds two
previously dropped track-03 phrases, changes no other old-corpus row, and does
not change the first six normal-helper inventories. This recovery does not
reduce the frozen-reference diagnostic count; it is supported by the two raw
views, not by a claimed acceptance-score gain. `elision-production-audit.json`
also proves the matched empty-hint control now retains the Texas phrase with
uncertainty. No model was called during this replay.

## Phrase matcher version 6 and explicit chop prompt control

The v5 assignment preferred exact wording in a distant chorus over an eligible
nearby spelling variant. Two new regression tests catch swapped chorus
instances, including a nearby cue whose edge really is late. V6 retains the
same lexical eligibility, maximum one-to-one matched count, denominator and
half-second tolerance; among eligible instances it prioritizes temporal
proximity, with wording similarity breaking close ties. A uniquely misplaced
line still matches and counts as one timing error. All fourteen metric tests
pass; the previous implementation fails both new controls. This is a scorer
repair, not improved recognition. `matching-policy-v6-audit.json` re-scores
identical v3 reference and candidate files under both policies and records
hashes. The still-running normal batch retains its originally loaded v4 score;
its final output must be uniformly re-scored under v6.

Four matched production audio crops from track 02 were replayed with an
explicit instruction to keep continuous chop runs together and separate leads
from echoes. The two observations improved the chop's overall boundaries but
still disagreed on “so” versus “go”; nearby lead lines fragmented differently.
The result does not justify replacing the normal discovery prompt. Original
and experimental clip hashes match; all observations and the exact prompt are
in `02/chop-prompt-pilot/audit.json`. No frozen reference was changed.

Two user grouping judgments remain pending: the lead sentence around
26.5–29.7 seconds on track 03 and “I am / present” on track 07. Neither reference
has been changed while waiting. Continuous chop runs remain one phrase and
separately sung echoes remain separate under the already confirmed metric.


## Singing-specific alignment and relative-clock controls

A separate local SOFA pilot tested six fixed normal track-07 phrases on both
the original mix and existing clock-preserved vocal stems. It used the model
publisher's [English v1.0.0 checkpoint](https://github.com/spicytigermeat/SOFA-Models/releases/tag/v1.0.0_en),
its matching SOFA v1.0.3 code (`584d6b9a57927843f85decebf4cbf03b9598125f`),
and the publisher's v005 dictionary. The checkpoint loaded with
`weights_only=True` and strict state matching; file hashes and download URLs
are in `sofa-trial/downloads.json`. The isolated environment reads the existing
Torch runtime and adds its own research dependencies. The application runtime
and its installed packages were not changed. WAV loading uses soundfile on
mono 44.1 kHz PCM to avoid an unrelated TorchCodec dependency; it opens no audio
output device. Full requests, expanded alignment text, pronunciation fallbacks
and word boundaries remain in `sofa-trial/`.

The checkpoint dictionary and CMU dictionary both lack “sternum”; the pilot
records a g2p_en pronunciation fallback. An initial tokenizer also omitted the
number 28. Those initial audits are explicitly marked invalid and preserved
with `numeric-omission` filenames. The corrected pilot expands numbers before
phonemization and verifies that every input word survives. Both the original
mix and separated-vocal conditions still increase the full-track diagnostic
from 3 to 8 by changing only those six cue boundaries. Several tails reach the
crop edge. The matching-code control reproduces the regression; SOFA is not
promoted. This narrow negative result is not a claim about the model's accuracy
on all singing data.

A separate offline ten-track trial estimated relative per-crop clock offsets
from at least three common phrases, without reference timestamps in its
estimator. It made four one-error improvements but worsened track 06 by one;
the remaining tracks were unchanged. The fixed policy and all changed rows are
in `relative-clock-trial-audit.json`. This heuristic is not promoted either.

A read-only cache census found 117 byte-identical requests across the first
seven normal runs and their preceding corpus runs: same clip bytes, offsets,
lengths, request prompt hash, model, server and harness, but a new remote session.
The full-track duration/list identity prevents reuse of otherwise identical
crops. `clip-cache-reuse-audit.json` records the receipt pairs. Cache reuse needs
a separate provenance-preserving repair; no receipt was relabelled and no
running job was restarted to work around it.

Latest implementation checks: all eight quick gates pass, including 469 Python
and 501 app tests, 1032 core tests, 186 runtime tests plus one ignored. Release
build and code-map generation complete. These checks establish software behavior;
the final acoustic acceptance target remains open.


## Pause checkpoint — all ten normal Assist runs complete

No research helpers remain running. The two grouping decisions remain open.
The operator requested documentation, a scoped commit, a release build and a
listening handoff before switching devices. Further experiments are paused.

These are disagreements against independent model proposals, not adjudicated error rates. Every row uses the unchanged v3 reference and v6 matcher. Uncertain reference proposals are listed separately and are not resolved by model agreement.

| Track | Reference phrases | Candidate phrases | Missing | Extra | Timing | Disagreements | Uncertain reference proposals |
|---|---:|---:|---:|---:|---:|---:|---:|
| 01 Fence Around | 74 | 68 | 17 | 11 | 9 | 37 (50.00%) | 8 |
| 02 i rly think so | 47 | 44 | 9 | 6 | 6 | 21 (44.68%) | 6 |
| 03 Groyper Idol | 54 | 35 | 26 | 7 | 5 | 38 (70.37%) | 11 |
| 04 Resolve to You (GPT-5.6-Sol) | 63 | 65 | 6 | 8 | 7 | 21 (33.33%) | 5 |
| 05 Cat Gradient Optimizer (Learn) | 112 | 109 | 21 | 18 | 23 | 62 (55.36%) | 7 |
| 06 Eight Thousand Tokens | 54 | 56 | 4 | 6 | 7 | 17 (31.48%) | 3 |
| 07 Floor Mechanics I_ Load Bearing | 66 | 64 | 2 | 0 | 1 | 3 (4.55%) | 0 |
| 08 Breaking What Remains (Beige Cardigan) | 38 | 33 | 14 | 9 | 5 | 28 (73.68%) | 6 |
| 09 The Cage Went Deep (GPT-5.6-Sol) | 79 | 87 | 10 | 18 | 6 | 34 (43.04%) | 8 |
| 10 Autoregressive Kitty | 38 | 34 | 11 | 7 | 7 | 25 (65.79%) | 4 |

Original adapter provenance remains v10 for 01–07 and v11 for 08–10; the
paired offline replay audits retain original hashes. This table does not
establish acoustic acceptance. SOFA and relative-clock pilots were not promoted.

### Scoped handoff verification

The staged lyrics changes were applied to an isolated checkout of the parent
commit, without the concurrent scene/provider-discovery edits. Its build,
format, clippy, code-map, Rust tests (487 app / 1029 core / 186 runtime, one
ignored), Python suite (466 tests, two skipped), offline support bundle,
credential canary and capture-isolation lint passed. The first gate run's two
harness failures were caused by the private Cargo target directory: those
harnesses expect `target/debug`. A checkout-local symlink to the private target
fixed their lookup; both failed gates were rerun successfully. Logs are
`verify-commit-only.log`, `verify-commit-support.log` and
`verify-commit-canary.log` under the private evidence root.

The prepared grouping bundle's audio hash and ZIP integrity passed. Its Rust
protocol loaded both questions and saved synthetic choices in a separate QA
copy. Muted Chromium decoded and played both excerpts, exercised the browser
choices and confidence with POSTs intercepted, and showed the second excerpt
at the correct combined-audition offset. The real answer log remains empty.
The workspace release build completed (`release-pause-handoff.log`); concurrent
scene changes remain outside this lyrics commit.

## Listening handoff correction — synchronized lyric alternatives

The initial audio-only form did not show what its “pause” question meant as
timed lyrics. The browser now renders both grouping proposals against the real
playhead, with original-track timestamps, cue seek buttons, a marked proposed
gap and replay. One cue holds the full wording across the interval; two cues
clear and replace it. Proposal timing and provenance are explicit, not newly
adjudicated evidence. The session audio, answer ids and prior answers are
preserved; the portable bundle includes the new browser metadata. The Rust
protocol remains an audio/question fallback without these caption alternatives.
Research remains paused.

Validation: the browser production build and four unit tests passed; four
existing Playwright flows passed, and the new timed-caption test passed after
waiting for the newly selected audio to be ready. It pins shared playback
time, the half-open cue boundary, blank split preview during the proposed gap,
original-time conversion, seek and replay, and unchanged answer progress. Real
Groyper/Floor previews were captured on desktop and mobile with Chromium muted.
Audio bytes and answer logs were preserved. Release build completed.
