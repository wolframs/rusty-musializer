# Assist and lyrics pipeline

This document explains the automatic-analysis path as a maintainable system: its
control flow, its trust boundaries, and the reasons its lyric policy is
deliberately more conservative than the code alone suggests. For installation,
dependency discovery, commands, and privacy details, use
[`tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md). For the dated
experiment log and acceptance evidence behind the lyrics policy, use
[`LYRICS_TIMING_INVESTIGATION.md`](archive/LYRICS_TIMING_INVESTIGATION.md) and
[`LYRICS_TIMING_INCIDENT_2026-08-17.md`](LYRICS_TIMING_INCIDENT_2026-08-17.md).
For provider
task contracts, credential storage, the codex discovery ladder, and the
execution-snapshot schema recorded in every job's provenance, use
[`ASSIST_PROVIDER_CONTRACTS.md`](ASSIST_PROVIDER_CONTRACTS.md) and
[`PHASE0_INVENTORY.md`](PHASE0_INVENTORY.md).

## Design contract

Assist is an evidence import pipeline, not an in-process model feature.

- Python tools may measure audio, invoke local models, and make explicitly
  authorized hosted requests.
- Their output is untrusted until Rust validates the bridge and its audio
  identity.
- A completed interactive job stages an inert candidate. It does not edit the
  project.
- The user reviews the result and confirms Apply; Discard drops it.
- Each mode authorizes a fixed set of lanes, checked both while preparing and
  applying a candidate.

These constraints let model integrations change without pulling GPU frameworks,
network clients, or their failure modes into the renderer.

## Opt-in Antigravity audio transcription

The default lyric route is local. In Assist settings, enable **Show experimental**,
select the `antigravity` route for coarse lyric localization, and choose Gemini
3.8 Flash. Starting Timed lyrics or Full assist then confirms audio transfer for
that job. The complete recording is sent as overlapping clips of at most 30
seconds. By default, lyric sheets stay local and retain caption authority: feeding the complete
intended text to the remote model caused hallucinated verses over instrumental passages. Opening settings or
running the doctor does not authenticate or send audio.

An explicit **Local models → Antigravity: detect performed phrases** choice
(`local_runtimes.performed_lyrics`, default off; helper `--performed-lyrics`)
uses corroborated audio to determine the phrase inventory even when a written
sheet exists. Local occurrence-constrained alignment refines those phrases.
`tools/local_lyric_spelling.py` then restores only a unique exact normalized-token
match from the local sheet, preserving the audio wording and reference spans in
provenance. It never inserts unperformed written lines or substitutes fuzzy
expected words. Unmatched wording remains an audio proposal. The option affects
only the Antigravity route; the written-sheet localizer remains available with
it off. Both choices stage results for review and Apply. The manifest records
`lyric_inventory` and the sheet's `local_exact_spelling` use, and the projection
is written separately as `lyrics.performance.json`.

`tools/antigravity_audio.py` uses the official ACP executable and its saved OAuth
profile. It discovers T3 Code's active Linux runtime and requires exactly one
signed-in profile, or explicit `local_runtimes.antigravity_server`,
`antigravity_harness`, and `antigravity_profile` paths. Equivalent helper flags
and `MUSIALIZER_ANTIGRAVITY_*` environment overrides exist. An explicit missing
path fails; it never falls back to another account. No OAuth token is read,
copied into Musializer settings, or exposed to the interface.

Each clip starts a separate ACP session. The adapter checks advertised audio
support, the exact offered model and the acknowledged selection. Client tools
and permission requests are denied. Audio is decoded to WAV in a pipe, with no
playback or original metadata. Responses and clip identities are cached under
the private job folder; changing the recording, prompt, model, runtime
or harness invalidates their reuse. Changing a local lyric sheet does not. Invalid responses retain a diagnostic receipt
and receive one bounded retry. Cancellation stays within the Assist process tree.

With an authored sheet, discovery produces coarse phrase evidence for the same
anchor/block localizer used by the local route. Every alignable authored line
reaches that acoustic stage; a single-view phrase is not discarded before it
can help locate a supplied line. Captions retain authored wording. Unplaced
lines and unrepresented performed phrases remain visible for review.

Consecutive distinct numbered `Voice N:` labels are treated as source
structure, retaining their original rows while sending only the words after
the labels to matching, acoustic alignment and captions. Isolated or quoted
uses are preserved.

For this opt-in route, the original crop observations also refine boundaries
after localization. Matching happens locally against supplied words, separately
within each crop, so a sentence split into two phrases in one observation can
corroborate a whole sentence in another. Both outer words must bound the observed
phrase, two complete crops must agree within 0.5 seconds on both edges, and the
existing cue must already select that occurrence. Their times and the existing
cue supply a median; no interior word times are invented. Competing uses of one
performance and new backwards cue ordering decline refinement. Authored text,
unresolved lines, original acoustic word evidence and review warnings survive.

A separate recovery pass can place an unresolved supplied line when two complete
crops agree on its occurrence between the existing neighbouring lines. It
requires both outer words and at least 90% token similarity, with three or more
words. A competing single-crop occurrence or boundary still makes the result
ambiguous; sampling one repetition more often must not make it win. Existing
placements and other missing lines cannot claim the same observation rows, and
recovery cannot reverse authored order. Recovered cues retain the original
unresolved acoustic record and stay marked for review. Unmatched performed
proposals are rebuilt after recovery so the same performance does not remain
in the Potential lane as well. This does not yet resolve differences between
the sheet's repetition count and the performed repetition count.

Additional performances are represented separately in `performed_occurrences`.
Complete authored wording is a reusable local template, so the recording can
perform it more times than the sheet lists it. Two original crops must propose
an occurrence outside existing placement coverage. Two fresh short audio-only
crops must then corroborate its words and both boundaries within 0.5 seconds.
Competing ownership of source rows declines the addition. The authored ledger
is preserved; these instances do not resolve or consume its ambiguous lines.
An unresolved line with a trailing parenthesized reply can also supply its
main phrase as a template. This requires the same original and fresh audio
confirmation; it does not assume the reply was performed. A complete authored
template takes precedence over the same main-phrase wording, and an audible
complete call-and-response takes precedence over its contained fragment.
Such additions carry `text_scope: authored_inline_lead`; the full written line
remains unresolved. Existing placed lines are never shortened by this policy.

Both lists form the rendered lyric lane, scene-plan lyric reentries, bridge and
manifest counts. Additional instances are uncertain cues with distinct
"ADDED occurrence" review entries, not non-rendering Potential proposals.
Unrepresented performed proposals are rebuilt against the combined lane. The
source artifact retains original and fresh crop evidence and request identity;
confirmation requests still receive no authored text. Full-lane audit tools
score both lists.

Confirmed compound cues can additionally have `phrase_splits`: two separately
performed phrases, each supported by two original and two fresh audio crops.
The original authored or additional-occurrence record stays intact. Each split
names its source array, index and canonical digest; rendering refuses a stale
or duplicate source link. The rendered lane substitutes the component phrases
for the compound cue, and the bridge, scene reentries, counts and audit tools
use that same view. Separate `PHRASE` review entries name the components without
replacing the authored-line review identity. If either reply boundary remains
uncertain, the original compound cue stays in place. No timestamps are inferred
by dividing the compound cue's duration.
The same rule also covers ordinary written sentences containing two performed
phrases. A complete crop must first identify the existing cue and place a
supplied-text prefix at a reported phrase boundary. Both components then need
the same original and fresh crop agreement. A comma alone, one crop's grouping,
or multiple competing seams cannot split the caption. Authored spelling is
retained, including balanced quotation marks on quoted components.

Performed wording absent from the authored templates can also enter
`performed_occurrences`, explicitly marked `text_scope: audio_observed` with
no authored line indices. Two distinct original crops and two fresh audio-only
crops must agree on the words and both boundaries within 0.5 seconds. This
includes isolated short ad-libs. A word already contained in a placed phrase,
or observations already owned by another placed cue, cannot create an extra
caption. These uncertain cues have separate `AUDIO` review entries; they do
not rewrite or resolve an authored line. Their raw confirmation observations
and request identity remain in `observed_occurrence_analysis`. Resuming this
stage replaces its own additions while preserving authored occurrences and
compound split links.

An empty provider response or explicit quota limit stops audio requests with
a resumable error. It is not treated as malformed lyric JSON and retried as a
transcription repair. Completed clip receipts remain available for the next
attempt under their exact audio, model, prompt and runtime identity.

Without a sheet, this route proposes **performed wording** and does not invoke
Codex wording review. Overlapping observations and fresh short confirmation
crops support phrase presence; incomplete or unconfirmed phrases stay Potential.
Local MMS/CTC uses phrase-specific windows to inspect the acoustic boundaries.
Multiple audio observations and the acoustic result provide a median boundary
estimate. The observation and acoustic evidence remain separate, and uncertain
cues appear in review. Only this no-reference result declares audio transcription
as caption authority. Apply preserves the observed provider model in project
provenance in both modes.

This route remains experimental. The ten-track investigation and limitations are
recorded in [the dated repair notes](LYRICS_ASSIST_REPAIR_2026-09-06.md). Agreement
between model observations is not an adjudicated timing error rate.

## End-to-end control flow

```text
Assist panel
    |
    v
AssistSession request -- after drawing --> AssistController
                                            |
                                            v
                                     runtime AssistJob
                                            |
                                            v
                              tools/external_analysis.py
                               |       |       |       |
                           measured  Whisper  align  plan
                               \       |       |      /
                                analysis.bridge.tsv
                                            |
                                            v
                              parse + identity validation
                                            |
                                            v
                                  AnalysisCandidate
                                     |           |
                                   Apply       Discard
                                     |
                                     v
                         track lanes + provenance
```

### 1. UI policy

[`core::ui::assist_ui_state`](../crates/musializer-core/src/ui/assist_ui_state.rs)
owns modes, start guards, confirmation states, lane authority, status copy, and
layout. [`app::ui::panels::assist`](../crates/musializer-app/src/ui/panels/assist.rs)
draws that policy and places requests on `AssistSession`.

The frame loop drains the request only after drawing closes. This is where the
application may safely open a picker, start or stop a child, read artifacts, or
mutate a track.

### 2. Process supervision

`AssistController` resolves `tools/external_analysis.py`, chooses a stable
per-track workspace under `build/analysis`, and builds an `AssistSpec`.
[`runtime::process::assist`](../crates/musializer-runtime/src/process/assist.rs)
owns the child, artifact paths, forty-minute deadline, cancellation, process-tree
termination, and reaping.

The helper creates its own process group. Do not make it a process-group leader
from the Rust parent: its `os.setsid()` would fail with `EPERM`. The runtime tests
pin this otherwise non-obvious lifecycle requirement.

### 3. Evidence production

[`external_analysis.py`](../tools/external_analysis.py) is cache-aware and writes
intermediate evidence plus a final TSV bridge. Depending on mode it coordinates:

- deterministic measured analysis from FFmpeg-decoded PCM and NumPy;
- whisper.cpp transcription and rough word order, decoded with no cross-segment
  text conditioning (`--max-context 0`);
- authored lyric discovery from an explicit sheet, sibling file, or embedded
  metadata;
- anchor→block localization plus local MMS forced alignment for final lyric
  timing when authored text exists, and per-cue MMS refinement when it does not;
- local transcript retention when no authored sheet is found; Codex wording
  review requires an explicit route override;
- optional explicitly authorized OpenRouter semantic review, routed
  through a per-job task-contract graph resolved once at Start (see
  [`ASSIST_PROVIDER_CONTRACTS.md`](ASSIST_PROVIDER_CONTRACTS.md) §1 and §6);
- scene-plan construction and bridge serialization.

The stable output directory permits reuse, but cache provenance includes source
and audio identities plus the relevant model, policy, and settings versions.
Changing a timing policy must invalidate the artifact it changes.

### The feelings request (MiMo 2026-11-14; Gemini 3.8 Flash since 2026-11-15)

`tools/semantic_openrouter.py` (until 2026-11-15 `tools/mimo_openrouter.py`)
sends the whole track once, with the checklist-style prompt the MiMo benchmark
kept ([`MIMO_BENCHMARK_RESULTS.md`](archive/MIMO_BENCHMARK_RESULTS.md)), and the guards
below. They were built on MiMo and hold for every model; what is really per
model is one row of `MODEL_PROFILES` (Rust twin `core::assist::cost::MODELS`),
and the lane's model since 2026-11-15 is Gemini 3.8 Flash (next section). The
sentences quoted here name MiMo because they were written on it; the helper
now names the model the request asked for ("Gemini's answer was cut off …").

- **A completion cap.** `max_tokens: 16000`, reasoning included, part of the
  request identity. Without it the provider's 65 536 applied. The cap is per
  model now; both rows hold 16 000.
- **Failed runs.** A response that did not finish with `stop`, carries null or
  empty text, is not JSON, or does not fit the schema is never cached and never
  staged. A `musializer.semantic-failed-run/v1` receipt (written as
  `musializer.mimo-failed-run/v1` until 2026-11-15; both are read) goes beside the cache
  (`semantic.cache.failed.json`), and the job fails with one sentence naming the
  reason and what OpenRouter billed for it (or, unreported, the most it can
  have cost). A billed empty answer is not retried.
- **One call, as confirmed** (H4c). The request is sent again only where
  OpenRouter says it was not processed: a 429, or a failure in connecting or
  sending. Any other status that is not a clear refusal (400, 401, 402, 403,
  404, 405, 413, 415, 422; H4d: a 524, a 418 or a redirect was a plain error
  with no word on cost), a timeout or a dropped connection
  while waiting, or an answer that does not read may have been billed: a
  failed run with a receipt (`may_be_billed: true`) and the sentence
  "OpenRouter answered HTTP 502 after the request was sent; it may have been
  billed (at most 0.033 USD). Nothing was saved or sent again." Whether to run
  it again is the person's call, so the displayed maximum stays one call's.
- **A failed regeneration beside a kept answer** (H4c). A receipt beside an
  accepted cache means the latest attempt failed after that answer was made
  (the helper removes the receipt as an attempt starts). Since H4d only a
  receipt that failed after the answer (`failed_ns` against the cache's
  `answered_ns`) is read so, and a success removes a receipt older than
  itself: two helpers on one cache, one failing and the other then
  succeeding, labelled the new answer "the earlier answer … which was kept". Decided by the trust
  policy above: the earlier answer is still this request's answer — the same
  audio, the same request identity and route, validated when it was made —
  so it is staged, and discarding it would only make the person pay again.
  It is never staged silently: the manifest carries `semantic_latest_failure`
  and the Assist panel raises a warning, "The latest MiMo attempt failed"
  ("The latest listening attempt failed" since 2026-11-15),
  with the helper's words, e.g. "The latest MiMo attempt failed on 2026-11-14
  22:37 UTC: MiMo's answer was cut off at the 16000-token cap. Nothing was
  saved; OpenRouter billed 0.1312 USD for it. The feelings staged here are the
  earlier answer, made on 2026-11-14 18:53 UTC, which was kept." A receipt for
  other settings says "(with other settings)". It stays until the next attempt,
  so every reuse says it.
- **Stated scales.** The prompt and the model's schema give energy, tension
  (0–1), valence (−1–1) and confidence (0–1) a range and a meaning. A value
  outside its range fails the run, naming the field, rather than being clamped
  or dropped: the bridge needs all four, and an answer on another scale has its
  other numbers on it too. Prompt `musializer-semantic-score/v2`; a v1 cache is
  regenerated, and its document still opens.

The cost is estimated from completion tokens as well as audio
(`musializer.semantic-cost-estimate/v1` since 2026-11-15, before it
`musializer.mimo-cost-estimate/v1`; `core::assist::cost` and its pinned Python
twin): the confirmation shows a range and the cap's ceiling under the semantic
route (the ceiling rounded up at the thousandth shown, so a capped MiMo call on
100 s, 0.032376 USD, reads "at most 0.033 USD", in the confirmation and in a
failure sentence alike), and the artifact's `provenance.generation` and the manifest's
`semantic_cost` keep that estimate beside the provider's `usage` and cost.
Since 2026-11-15 the line reads "Cost: about 0.013–0.040 USD, at most 0.073 USD
(16000-token cap)" (Gemini, 200 s); the earlier "Estimated cost: …, at most …
(answer capped at 16000 tokens)" was cut off with an ellipsis at the default
window width, and now fits in both appearances (capture of 2026-11-15, 1280×720).

**Negative controls (H4c, 2026-11-14)**, each run on a perturbed source and
the source restored byte for byte (`cmp` against a saved copy):

| Perturbation | What failed |
| --- | --- |
| The confirmation's maximum formatted unrounded again (`{:.3}` of `usd_max`) | `at_most_is_rounded_up_never_down`: "at most 0.032 USD" for 100 s |
| The helper's failure clause without `ceil_usd` | `test_a_failed_run_without_a_reported_cost_names_the_ceiling` and `test_at_most_is_rounded_up_as_the_rust_side_rounds_it` |
| The reuse path not reading the receipt (`semantic_failure = None`) | `test_the_earlier_answer_is_staged_with_the_failure_named` and `test_a_gateway_failure_on_regeneration_says_it_may_have_been_billed`: no `semantic_latest_failure` in the manifest. The panel's reader is pinned by `an_earlier_semantic_answer_staged_after_a_failure_says_so`; that `stage_finished_job` chains its notice is not pinned by a test of its own |
| 502 and 503 retried again, and no status counted as possibly billed | `test_a_gateway_failure_after_sending_is_one_call_that_may_be_billed`, every status: two sends where one was confirmed, or "request failed with HTTP 504" with no word on cost |
| (H4d, 2026-11-15) A success no longer removing an older receipt | `test_a_failure_older_than_the_answer_is_not_its_word`: "B's success removed A's older receipt" |
| (H4d) The reuse path reading a receipt older than the answer (`receipt_applies` skipped) | the same test: `semantic_latest_failure` named "an older failure" beside a newer answer |
| (H4d, 2026-11-15) Only H4c's five statuses (408, 500, 502, 503, 504) counted as possibly billed again | `test_a_gateway_failure_after_sending_is_one_call_that_may_be_billed`: 524, 520, 418 and 409 ended as "OpenRouter request failed with HTTP …", no receipt, no word on billing |
| (Gemini, 2026-11-15) `temperature: 0.2` sent to Gemini again | `test_the_request_is_shaped_for_gemini`, `test_the_identity_carries_the_shape_and_the_cap` |
| (Gemini) `identity_extra` no longer empty for MiMo's shape | `test_mimos_request_is_byte_for_byte_what_it_was`: the identity hash moved, so every MiMo cache would be paid for again |
| (Gemini) no local request-size limit | `test_a_request_too_large_for_the_model_is_never_sent`: a 20 MB body was sent |
| (Gemini) both tag guards dropped (`-map_metadata -1` and `-id3v2_version 0`) | `RealEncoding.test_tags_never_leave_with_the_audio`: the canary title and artist in an ID3 header. Dropping either guard alone stays green, as it should: each suffices |
| (Gemini) Rust's Gemini completion price 4.125 → 4.0 | `test_the_table_is_the_same_table` (cross-language pins) |
| (Gemini) the confirmation's old long sentence restored | `the_line_is_short_enough_for_the_column` |
| (Gemini) the recommended route back on MiMo | `the_old_default_moves_to_gemini_and_a_stored_mimo_route_stays` |
| (Gemini) the stale-catalog rule switched off | `a_catalog_older_than_the_model_cannot_say_it_is_gone` |
| (Gemini) the `.musi` semantic lane hard-coded to `xiaomi-mimo-v2.5` again | `the_semantic_lane_names_the_model_that_ran` |
| (Gemini) the third workflow's label back to "MiMo feelings" | `tools/differential_assist_ui.sh`, the post-legacy diff |

### Gemini 3.8 Flash through OpenRouter (2026-11-15)

**The decision.** Operator, 2026-11-15: MiMo was "the first model that had
*any* usable *music* training", and "the gemini flash series has since
superseded it in usefulness by a lot"; asked whether to measure first:
"Gemini Flash 3.8, no competition ;)". So `google/gemini-3.8-flash` is the
recommended `TC-SEMANTIC` model on his judgment, **not on a benchmark run
here**; MiMo stays selectable as experimental. Row in `DIVERGENCES.md`, "The
listening lane's model".

**What was read, 2026-11-15.**

- OpenRouter's live lists: `https://openrouter.ai/api/v1/models`,
  `https://openrouter.ai/api/v1/models/google/gemini-3.8-flash/endpoints`,
  `https://openrouter.ai/api/v1/endpoints/zdr`. Input text, image, video, file,
  audio; context 1 048 576; output up to 65 536. Listed price 0.825 USD per
  million input and audio tokens, 4.125 per million output and per million
  `internal_reasoning`. That is the dearest endpoint a base slug routes to
  (Vertex `us`); Vertex `global` and Google AI Studio list 0.75 / 3.75, and the
  `flex` (0.375) and `priority` (1.35) tiers are opt-in only. **ZDR endpoints
  are Vertex's only**, so with the lane's default `zdr: true` it goes to
  Vertex. Vertex endpoints do **not** list `temperature` among their
  parameters (AI Studio does).
- Audio input, `https://openrouter.ai/docs/features/multimodal/audio`:
  `{"type": "input_audio", "input_audio": {"data": <base64>, "format": …}}`,
  base64 only (no URLs); Chat Completions takes any format the provider does.
  No size limit or billing rule stated there.
- Routing, `https://openrouter.ai/docs/features/provider-routing`: `zdr: true`
  routes only to ZDR endpoints; `require_parameters: true` excludes an
  endpoint that does not support every parameter sent; `max_price` is a hard
  filter; service tiers need `service_tier` or a `:floor`/`:nitro` slug.
- Reasoning, `https://openrouter.ai/docs/use-cases/reasoning-tokens`: Gemini 3
  uses Google's `thinkingLevel`, `reasoning.effort` maps onto it; reasoning
  tokens are billed as output and count against `max_tokens` (a run out of
  budget ends with `finish_reason: length`, billed); reported in
  `usage.completion_tokens_details.reasoning_tokens`.
- Structured output, `https://openrouter.ai/docs/features/structured-outputs`:
  per endpoint; `strict` enforcement varies by provider. All seven Gemini 3.8
  Flash endpoints list `structured_outputs` and `response_format`.
- Google, `https://aistudio.google.com/docs/audio` (the old
  `ai.google.dev/gemini-api/docs/audio` redirects there): 32 tokens per second
  of audio, up to 9.5 h per prompt, **20 MB per request in total** inline,
  audio downsampled to 16 kbps and mixed to one channel. Vertex,
  `https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/audio-understanding`:
  about 8.4 h or 1 M tokens per prompt, 256 MB per inline part. Gemini 3
  guide, `https://ai.google.dev/gemini-api/docs/gemini-3`: keep temperature at
  its default 1.0 ("setting it below 1.0 may lead to unexpected behavior, such
  as looping"); thinking level defaults to high.

**What the application sends** (the row in `MODEL_PROFILES`):

- the track **re-encoded** by FFmpeg from the snapshot that was hashed: MP3,
  128 kbit/s, 44.1 kHz, two channels, every tag and chapter dropped (no title,
  artist or cover leaves with the audio). Google hears 16 kbps mono anyway;
  128 kbit/s keeps a track of about 15 minutes inside AI Studio's 20 MB, which
  a non-ZDR route can reach. A WAV source sent as it is would have been 70 MB
  of base64 for five minutes. The audio hash, and so the cache key and the
  bridge's identity, stays the source file's. FFmpeg reads the snapshot from
  an anonymous in-memory file handed over as its standard input (seekable,
  so an M4A with its index at the end is read; no name on disk, so nothing of
  the track is left behind when the parent kills the helper at its deadline),
  gets an environment of seven named variables and so never the OpenRouter
  key, and its output is read in pieces and stopped once it passes the
  request's own size limit ("the track encodes to more than 19.5 MB, past
  what one request may carry; nothing was sent"). Review of 2026-11-15; the
  three controls: the limit ignored, the environment inherited, the input
  read as a pipe, each failing its test in `tests/test_semantic_openrouter.py`
  (`RealEncoding`). Not done, on purpose: an answer cached by this build is
  not reused by an older build (the rule is that newer builds read older
  files);
- **no `temperature`**: Google's advice, and on the ZDR route a `temperature`
  with `require_parameters: true` leaves no endpoint (a 404 refusal);
- no `reasoning` setting: the model's default thinking, which is the Gemini
  the operator chose on;
- `max_tokens: 16000`, the same prompt v2, schema and `provider` block as before;
- a local refusal, before anything leaves, of a body over 19.5 MB ("the track
  encodes to a 21.3 MB request, past the 19.5 MB one Gemini request may carry;
  nothing was sent"), with a receipt that says nothing was billed.

The request identity carries `audio_encoding` ("mp3 libmp3lame 44100 Hz 2 ch
128k"), `temperature: null`, the cap and `audio_format: "mp3"`, so a MiMo answer
is never served as Gemini's. MiMo's identity and body are byte for byte what
`tools/mimo_openrouter.py` built at `2d5d1b6` (pinned by hash), so a user who
stays on MiMo reuses every cached answer.

**The cap: 16 000, and why.** Three real calls (below) used 1 915–2 231
completion tokens on 100 s and 5 030 on 300 s, of which 0–2 264 were thinking.
Answer length grows with the number of sections, so a 10-minute track should
want about 10 000; 16 000 leaves room for that at the default thinking level,
and stops a runaway at 16 000 × 4.125 = 0.066 USD of output against the 0.27
USD an uncapped 65 536 would cost.

**The estimate** (`musializer.semantic-cost-estimate/v1`), from the row: audio
at Google's 32 tokens per second (Vertex counted 25.0 on this request; the
higher rate is kept for a bound), 1 650 tokens of text (measured, every call),
answers of 1 500–8 000 tokens typical and the cap as the ceiling, all at the
0.825 / 4.125 list price (calls were served at 0.75 / 3.75). For a whole
track: 200 s, about 0.013–0.040 USD, at most 0.073; 300 s, about 0.015–0.042,
at most 0.076 (0.075281 rounded up).

**The real calls**, through the path the application spawns (`semantic_openrouter.py
AUDIO CACHE --duration … --model google/gemini-3.8-flash --no-fallbacks --zdr`,
the recommended route's flags), on the two benchmark excerpts (approved
2026-11-14) and, for whole-track length, the three excerpts joined to 300 s as
a WAV (cat, whale, cat). Ledger kept in the worktree's `build/gemini-calls/`.

| audio | prompt tokens (audio) | completion (thinking) | finish | reported USD | estimate USD |
| --- | --- | --- | --- | --- | --- |
| shut-up-cat excerpt, 100 s | 4 152 (2 502) | 2 231 (641) | stop | 0.01148025 | 0.010–0.037, ≤ 0.071 |
| constellation-whale excerpt, 100 s | 4 152 (2 502) | 1 915 (0) | stop | 0.01029525 | 0.010–0.037, ≤ 0.071 |
| the three joined, 300 s | 9 152 (7 502) | 5 030 (2 264) | stop | 0.0257265 | 0.015–0.042, ≤ 0.076 |

Total 0.0475 USD of the 0.25 allowed. All three were served by Google
(Vertex; the first took 21 s end to end), valid JSON on the first try, on scale, contiguous to the
second. A whole song fits one request with room to spare: 7 502 audio tokens
for 300 s against a 1 M context, 4.8 MB of MP3.

**What the answers looked like** — two excerpts and one joined file, read
against what `build/mimo-bench/` knows (the adjudicated lyric onsets; its
measured sections are a 12 s grid and say little about form). Two excerpts are
not a benchmark, and nothing below is one.

- *constellation-whale* (0–100 s): seven sections, and they sit where the
  lyrics say the song changes: verse from 10.5 s (first line 8.1 s), a build
  from 23.5 s, a drop at 38–55.5 s exactly where the vocals stop (33–55 s), the
  second verse from 55.5 s (line at 55.1 s), a second build at 73 s, a final
  chorus from 88 s. Energy follows loudness closely (correlation 0.82 with each
  section's RMS level); tension peaks in the two builds (0.75, 0.80) and drops
  in the drops, which is what a build is.
- *shut-up-cat* (30–130 s of the track): six sections. The hook section at
  10–22.5 s matches the "Shut up Cat" refrains (8–21 s), the change at 22.5 s
  the switch to "I play it tough" / "queen of the sky" (23.3 s, 28.6 s), and
  33–58 s the repeated hooks. It calls 58–83 s an "introspective half-time
  breakdown" at energy 0.45 while the waveform there is only 2–4 dB quieter
  than its neighbours, and puts the last return at 83 s where the vocal comes
  back at 91.6 s (it may be hearing the build before it). Correlation with
  loudness 0.49.
- *joined, 300 s*: it found both joins (100.0 s exactly, and 199 s for 200 s).
  But the third part is the first part again, and it was segmented
  differently (one 61-second "climax" against 0–24, 24–58 the first time) and
  described as a different moment of the song: the same audio heard twice in
  one call does not get the same sections. Energy against loudness 0.87.

Valence stays positive on both excerpts (0.10–0.88), which fits two bright
tracks and leaves the dark half of the scale untested.

**A user with MiMo settings or answers** sees this:

- *No `TC-SEMANTIC` route stored* (the common case, the operator's own file):
  the next Feelings or Full assist job is on Gemini; the confirmation names
  it and shows its estimate, and the previous MiMo answer in the track's
  `semantic.cache.json` does not match the new identity, so the job pays once
  for a Gemini answer, which replaces it. The project's applied lane keeps its
  MiMo cues until the new candidate is applied.
- *A stored MiMo route* (chosen in AI settings): stays on MiMo, marked
  experimental in the picker; its cached answers are reused, nothing is paid
  again. The receipts and caches the old helper wrote still read.
- *A cached OpenRouter catalog older than 2026-11-15* (the operator's is
  2026-09-01, before Gemini 3.8 Flash existed): it does not list the model, and
  until this change that refused the job ("has no eligible endpoint"). Now it
  reads "Catalog older than this model; refresh it" and does not refuse.
  Nothing refreshes it by itself: the settings field `refresh_on_open` is
  saved but not wired in this build (the control says "Refresh on open: not
  wired"), so the list is refreshed only by **AI settings → OpenRouter →
  Refresh now**, with network access on. Since J1-5 (2026-11-16) the Start
  confirmation says so too, in one quiet line under the routes: "The saved
  model list is older than google/gemini-3.8-flash. Refresh it in AI
  settings → OpenRouter → Refresh now."

Not tried for real: a refusal or failure from Gemini (every failure path is
tested against synthetic responses only), a track long enough to hit the size
limit, a route without ZDR (AI Studio), and any thinking level other than the
default.

### Starting Feelings safely (J1, 2026-11-16)

Before anyone had pressed Start against the real service, a trace of the
route found seven defects; six are fixed here (the seventh, how a staged
answer is reviewed before Apply, is another stream's). The rule they share:
the user never pays for what the application could have known would be thrown
away, and whenever money may have been spent they are told so, with the
amount, where they are looking.

**A track file replaced after loading.** The panel compared the bridge's hash
with the digest taken at load, never checked again; the helper hashed the file
as it was, paid, and the result was then refused. The digest now carries the
file's stamp (length, modification and change time, inode;
`project_files::FileStamp`). Start and the open confirmation `stat` the file
(the card at most every two seconds); a moved stamp is re-hashed before it is
believed, and only different bytes refuse Start, before any route is resolved:
"This track's audio file changed on disk after it was loaded, so no result
could match it. Open the file again (or reopen the project), then start.
Nothing was sent." The verified digest rides to the helper
(`--expect-audio-hash`), and on to the listening helper (`--expect-sha256`),
so a change after the press is refused where the bytes are read ("the track's
audio file changed on disk after Start, so nothing was measured or sent. Open
the file again, then start"). A result for another file that turns out to be
this one changed says so instead of "Bridge audio does not match". Other
holders of the load-time digest, not changed here: saving re-hashes while it
copies and refuses a mismatch (`publish::open_source`); reopening a `.musi`
verifies; `--treatment`'s audio check, a generated clip's placement identity
(`generation/window.rs`) and a poster's recipe key compare against the cached
digest and would trust a stale one.

**Every failure of the listening step leaves its reason.** A refused key, no
credit, no endpoint, a rate limit after the retries and an unreachable network
were plain errors, and the panel read "A step of the analysis failed / python3
exited with code 1". Each now writes a receipt (`kind`, `may_be_billed`, a
message whose first 160 bytes say what happened and what it cost, the next
step after it, 224 at most); the panel reads the one written since the job
started and titles the notice by what to do. The words:

| Failure | Notice title | Sentence |
| --- | --- | --- |
| 401, key from AI settings | OpenRouter refused the key | OpenRouter refused the key saved in AI settings → OpenRouter (HTTP 401); nothing was billed. Replace it there, then start again. |
| 401, key from the environment | OpenRouter refused the key | OpenRouter refused the OPENROUTER_API_KEY this app was started with (HTTP 401); nothing was billed. Add a working key in AI settings → OpenRouter. |
| 402 | The OpenRouter account needs credit | OpenRouter refused the request: the account needs credit (HTTP 402); nothing was billed. Add credit at openrouter.ai, then start again. |
| 404 with privacy constraints | No OpenRouter endpoint fits these settings | No OpenRouter endpoint for Gemini fits zero data retention, no fallbacks (HTTP 404); nothing was billed. Relax it in AI settings, or refresh the model list there. |
| 404 without | No OpenRouter endpoint fits these settings | OpenRouter has no endpoint for google/gemini-3.8-flash (HTTP 404); nothing was billed. Refresh the model list in AI settings → OpenRouter. |
| 400, 403, 405, 413, 415, 422 | OpenRouter refused the request | OpenRouter refused the request (HTTP 403: *OpenRouter's words, shortened*); nothing was billed. |
| 429 after three tries | OpenRouter is rate-limiting requests | OpenRouter rate-limited the request 3 times (HTTP 429); nothing was billed. Wait a minute, then start again. |
| no connection | OpenRouter could not be reached | Could not reach OpenRouter (*reason*); nothing was sent. Check the connection, then start again. |
| no key reached the helper | The listening request was not sent | No OpenRouter key reached the listening step; nothing was sent. Add one in AI settings → OpenRouter, then start again. |
| timeout after sending | The listening request may have been billed | OpenRouter's answer did not arrive within 600 s; it may have been billed (at most 0.073 USD). Nothing was saved or sent again. |
| an answer that cannot be used | The listening answer could not be used | Gemini's answer does not fit the schema (…). Nothing was saved; OpenRouter billed 0.0131 USD for it. |
| FFmpeg missing, no cached measurement | A tool the analysis needs could not run | could not start FFmpeg, which measuring the track needs: it is not installed or not on PATH. Install FFmpeg, then start again; nothing was sent. |

The key's origin reaches the helper as `MUSIALIZER_OPENROUTER_FROM`, set by the
runtime beside the key and never without it. The schema-failure and
may-be-billed sentences shorten the reason, never the amount, so the bill
survives the panel's 160-byte clip. The measured step (`analyze_audio.py`)
writes `measured.failed.json` (`musializer.step-failed/v1`), which the
orchestrator raises; a missing NumPy says so too. Other child steps whose
words still die at `_run`, none on the Feelings path: whisper.cpp and its
FFmpeg decode (they name the tool and the code), the Codex review, the
anchor/block and forced aligners, the Antigravity aligner. ffprobe's failure
is swallowed on purpose (no embedded lyrics).

**A request that left is recorded.** (J3 below gave each attempt its own
record, put a read warning behind the user and changed these sentences; J4
replaced `earlier` and the log with the ledger; the words here are J1's.) Cancel, a closed window and the deadline
kill the helper's process group, and nothing on disk said a request was in
flight. The helper writes `semantic.cache.sent.json` before the request
leaves, durably, and removes its own (only while it is still its own) once an
answer or a receipt says how the call ended; a new attempt carries the
possible charges it replaces in its record and receipt (`earlier`) until an
answer is in hand. The panel reads a record written since the job started:

- Cancel: "Analysis cancelled after the request was sent" — "The analysis was
  stopped at your request, but the listening request had already been sent to
  OpenRouter and may have been billed (at most 0.073 USD). Nothing was applied
  to this project." (a warning, no longer an info).
- The deadline: "The helper ran for 40 minutes without finishing and was
  stopped; the listening request had already been sent to OpenRouter and may
  have been billed (at most 0.073 USD)."
- A helper that died with no receipt: "The listening request may have been
  billed" — "The helper stopped before an answer came, after the listening
  request had already been sent to OpenRouter and may have been billed (at
  most 0.073 USD). Nothing was saved."
- Closing the window: the quit dialog's line reads "Cancel the running
  analysis job; the listening request had already been sent to OpenRouter and
  may have been billed (at most 0.073 USD)." (pinned by no test of its own:
  `confirm_close` opens a dialog, which no gate can press).
- The next confirmation for the track, before Start: "An earlier listening
  request for this track was sent on 2026-11-16 10:02 UTC and never answered;
  it may have been billed (at most 0.073 USD)." (several: "3 earlier listening
  requests for this track may have been billed (at most 0.219 USD together);
  the last was on …"). Pressing Start after reading it is the agreement to
  send again. No ceiling or ledger is built here.

**An answer the result cannot hold is never cached.** The schema now states 64
sections and 500-character summaries (the bridge holds 256 and 2 047 bytes).
The section bound is written in the segments' description ("At most 64
segments."), not as `maxItems`: Google's endpoints refuse a request whose
schema carries that keyword (HTTP 400, "Provider returned error"), while
`maxLength` and `minItems` are accepted. Found by the first Start pressed
through the window against the real service (2026-11-16, the lead): the press
failed unbilled, three real calls isolated the keyword (without `maxLength`
still refused; without `maxItems` accepted; without both accepted), the
corrected request was accepted, and a second press through the window staged
seven sections for the benchmark's 100-second excerpt at 0.012 USD; a third
press reused that answer and said so before and after ("Cost: nothing. A saved
answer for this track and these settings, from 2026-11-16 14:57 UTC, is
reused, so this run sends nothing to OpenRouter."). The whole hunt cost about
0.05 USD. The test `test_the_schema_states_bounds_a_compliant_answer_always_fits`
pins that the schema carries no `maxItems`; its control is the keyword put back.
Before caching, `bridge_fit` repairs what is harmless (a section whose ends
round to the same millisecond joins its neighbour, a long summary is shortened
at a character, unprintables become spaces), names it in the manifest
(`semantic_repairs`), and fails the run with its cost where no repair fits
("Gemini's answer cannot be used: the answer has 300 sections, more than the
256 the application can hold. Nothing was saved; …"). The Python bridge
writer refuses what the Rust reader refuses (pinned), a cache the later steps
refuse is set aside (`semantic.cache.refused-<ns>.json`) by the helper or by
the panel when the bridge's semantic rows are what failed ("… The listening
answer was the part that did not fit and was set aside, so the next Start asks
again and may cost again."), and a cache that does not fit is never reused.
The bounded schema moves `model_output_schema_sha256`; answers made under the
unbounded one (2026-11-14 to 2026-11-16) are accepted when they fit, so none
is paid for again. On the owner's machine, 2026-11-16, the main checkout's
`build/analysis` held one listening cache, a MiMo answer under the v1 prompt
and schema, which no build since v2 reuses anyway; the 2026-11-15 Gemini calls
are a ledger in a worktree, not caches the application reads.

**The confirmation says what the model list knows.** The evaluator's
non-blocking "Catalog older than this model" now reaches the confirmation in
one quiet line: "The saved model list is older than google/gemini-3.8-flash.
Refresh it in AI settings → OpenRouter → Refresh now." A missing key no longer
offers to "route this task locally" for a contract with no local route: "…
no key is configured. Add one in AI settings → OpenRouter."

**A saved answer is said before Start.** `external_analysis.py
semantic-status` answers through the one reuse decision the run makes
(`saved_semantic_answer`), reading the job folder only. The panel asks it off
the frame thread when the card opens, with a snapshot of the listening route
and never the key, and while the answer is unknown the paid wording stands.
When it says saved, the cost line becomes "Cost: nothing. A saved answer for
this track and these settings, from 2026-11-15 18:53 UTC, is reused, so this
run sends nothing to OpenRouter." After the run the manifest carries
`semantic_reused` (`answered_utc`, ISO 8601, and `words`), which the staged
card reads (`stage_semantic_reuse`) and the staged notice appends: "Reused the
answer saved on 2026-11-15 18:53 UTC; nothing was sent or billed this time."
The data-boundary sentence
above it still says what the job is authorized to send.

**Through the window.** `tools/headless_check.sh --skip-render` presses Start
against a stand-in helper and pins every sentence above that the panel draws,
in the dark appearance (`assist notes:`, `assist notice:`); its captures are
`build/headless/j1-feelings/*.png`.

**Negative controls (J1, 2026-11-16)**, each perturbed, seen to fail, and
restored byte for byte (`cmp` against a saved copy):

| Perturbation | What failed |
| --- | --- |
| The stamp check always "unchanged" | `a_file_replaced_after_loading_is_told_apart_from_one_that_was_touched`, `start_refuses_a_file_replaced_after_loading_before_anything_is_spent`, `a_result_for_a_file_changed_since_loading_says_so` |
| Both helpers' expected-digest checks off | `AChangedFileIsNeverPaidFor` (2 tests) |
| A refusal writes no receipt | `EveryFailureLeavesItsReason` (every test reading a receipt) |
| `failure_message` unbounded again | `test_the_bill_survives_the_longest_schema_failure` and one more |
| The measured step's receipt not read | `test_the_parent_raises_the_steps_words` |
| No sent record written | `test_the_record_is_on_disk_before_the_request_leaves_and_a_kill_leaves_it`, `test_a_new_attempt_carries_the_earlier_possible_charge_until_an_answer` |
| Earlier charges not carried | `test_a_new_attempt_carries_the_earlier_possible_charge_until_an_answer` |
| A stop ignoring the sent record | `a_stop_after_the_request_left_says_it_may_have_been_billed` |
| `bridge_fit` skipped before caching | `AnAnswerThatCannotBeUsedIsNeverCached` (3 tests) |
| The unbounded schema's identity not accepted | `test_a_fitting_legacy_answer_is_reused` |
| The panel never blaming the semantic rows | `a_refused_listening_answer_is_set_aside_and_another_lane_leaves_it` |
| The older-list line not drawn | `a_model_list_older_than_the_model_is_said_quietly_on_the_confirmation` |
| "Route this task locally" always offered | `a_missing_credential_blocks_a_remote_route_and_names_the_repair` |
| `semantic-status` always "none" | `TheConfirmationKnowsASavedAnswer` (2 tests) |
| The saved answer not drawn | `a_saved_answer_is_said_before_start_and_after_the_run` |
| `semantic_reused` written as `{made_utc}` instead of `{answered_utc, words}` | `test_saved_means_the_run_sends_nothing_and_none_means_it_does` |
| Gate: the poll path ignoring receipt and record, Start skipping the stamp check | j1-no-credit, j1-cancel, j1-deadline, j1-changed (twice: the helper ran for a changed file) |
| Gate: the confirmation's three notes dropped | j1-saved (3), j1-catalog (2), j1-billed (3) |

Not tried for real: any refusal or failure from OpenRouter (every path runs
against a fake transport or a stand-in helper), a key rejected by the real
service, the real `semantic-status` from the window (the gate's stand-in
answers it; the real one is pinned by the Python tests), and a quit through
the real dialog with a request in flight (the gate's dialogs are guarded).

### Seven more defects on the Feelings path (J3, 2026-11-16)

A review of J1 and J2 confirmed seven defects; all seven were real and are
fixed here. Nothing sent to the model changed (request body, schema and
provider block are byte for byte J1's).

**One record per attempt (J3-1).** The J1 record had one name per cache, and
an attempt learned which record was "its own" by `stat` after publishing it:
with two helpers on one job folder, A could adopt B's record and remove it
when A ended, and a B killed mid-request then left no word on disk. Each
attempt now writes `semantic.cache.sent-<ns>-<8 hex>.json` (`attempt` inside
too), holds an exclusive `flock` on it from before it is published (written
to a temporary name, locked, synced, renamed) until it removes it, and removes
only that name. Another attempt reads a record as an earlier charge only when
it can take the lock (the kernel gave a killed helper's lock back); a locked
record is a request in flight and is neither read nor removed. The J1 name and
`…sent/v1` are still read. The panel reads every record name the helper can
write (`diagnosis::is_sent_record_name`, at most 64).

**Every failure leaves a receipt (J3-6).** `main` read the audio before the
part that writes receipts. The file is now read inside `analyze_to_cache`,
and a catch-all there writes the receipt any failure nobody named still owes,
by how far the attempt got. Found by reading `main` and everything before the
send: the audio read ("Could not read the track's audio file (No such file or
directory); nothing was sent. Check that the file is still there, then start
again."), an empty format, an unwritable `--request-dump` ("The listening
step stopped before sending (…); nothing was sent."), any fault after the
request left with no answer ("The listening step failed (…); it may have been
billed (at most 0.033 USD). Nothing was saved or sent again."), and a cache
that cannot be written after the answer was billed ("The answer could not be
saved (No space left on device). Nothing was saved; OpenRouter billed 0.0106
USD for it."). Removing the earlier receipt and records is best-effort and can
no longer fail an attempt that has written its record. Left: an argv the
parser refuses (exit 2) writes no receipt, because the cache path may be what
was refused; the application builds the argv, pinned by
`tests/test_helper_argv.py`.

**"Cost: nothing" only for the settings the run sends (J3-4).** The status
query was cached under `ContractSnapshot::compact()`, which leaves out the
provider block. The key is now the digest of the exact snapshot bytes the
query hands the helper (`status_key`), so any change to model, order, only,
ignore, fallbacks, price limits, fallback policy or ZDR asks again. And Start
compares the routes it resolves with the ones the card was drawn for
(`boundary_confirmed` aside): settings written while the card was open (the
dialog, another window) refuse Start with "The AI settings changed after this
card was drawn, so it may not say what Start would do. It is drawn again now;
read it, then start. Nothing was sent." and the card is resolved and asked
again. Both directions are pinned through the window.

**What was changed in the answer is said (J3-3).** The manifest counts the
repairs too (`semantic_repair_counts`, from the cache's provenance for a new
answer, from `bridge_fit` at reuse for an answer cached before repairs were
recorded), and the staged card draws notes under its head: the reused-answer
line (read since J2, never drawn) and "Fitted before staging: 2 sections
shorter than a millisecond joined to neighbours, their words and marks
dropped; 1 section's words shortened to fit." (cleaned characters: "unprintable
characters replaced in 3 sections' words"; a J1 manifest that names repairs
without counting: "The application changed this answer to fit before staging
it.").

**A row belongs to its answer's track (J3-5).** With another track current, a
row press switches to the answer's track (the track list's lyric-draft guard
first) and seeks with `ShellCommand::SeekOnTrack`, which `main` applies only
while that track is current, so a refused switch moves nothing. The card
notes "This answer is for other.wav; pressing a row switches to that track.",
the row's hint reads "Switch to other.wav and listen from 0:24", and an answer
whose track is closed moves nothing ("The track this answer is for is no
longer open, so its rows move nothing."). The switch starts playback, as the
track list does. The timeline's ghosts were already drawn only for the
answer's own track; Put back acts on the track Apply wrote to, and its line
names it when another is current ("3 earlier cues kept in Song A").

**An earlier charge says what it was for (J3-2).** The sent record and the
receipt carry `audio_sha256`, `request_key` (the request as one digest: the
cache key its answer would carry) and `model`; `semantic-status` names the
next Start's `request_key`. The confirmation's line, against what this Start
would send:

| Relation | Line (one record) |
| --- | --- |
| same request | A listening request for this track and these settings was started on 2026-11-16 10:02 UTC and never answered; it may have been sent and billed (at most 0.073 USD). Start sends the same request again; this note then moves to the job folder's log. |
| same request, saved answer | … This Start reuses a saved answer and sends nothing; this note then moves to the job folder's log. |
| other settings | A listening request for this track with other settings was started on … and never answered; it may have been sent and billed (at most 0.073 USD). After Start this note moves to the job folder's log. |
| other audio | A listening request for other audio that was at this track's path was started on … |
| settings not known yet | A listening request for this track was started on … |
| a J1 record | A listening request that used this track's job folder was started on … |
| a may-be-billed receipt | A listening request for this track and these settings, on 2026-11-15 20:00 UTC, may have been billed (at most 0.073 USD). … |
| several | 3 listening requests for this track may have been sent and billed (at most 0.219 USD together); the last was on …. … |

The J1 time filter (anything older than the cache's modification time counts
as answered) is gone: an answering attempt removes its own record, so a record
still there was never answered.

**Words after a stop, and the way out (J3-7).** The record is written before
the first byte leaves, so a helper killed just after writing it sent nothing.
The stops now read: Cancel, "Analysis cancelled during the listening request"
— "The analysis was stopped at your request, but the listening request may
already have been sent to OpenRouter and billed (at most 0.073 USD). Nothing
was applied to this project."; the deadline, "… and was stopped; the listening
request may already have been sent to OpenRouter and billed (at most 0.073
USD)."; a helper that died with no receipt, "The helper stopped before an
answer came; the listening request may already have been sent to OpenRouter
and billed (at most 0.073 USD). Nothing was saved."; the quit dialog, "Cancel
the running analysis job; the listening request may already have been sent to
OpenRouter and billed (at most 0.073 USD)." The way out: the confirmation
shows each possible charge once, saying what happens to the note, and the
Start pressed after reading it puts it in the job folder's log, so it is not
said again. Nothing is carried forward. **Replaced by J4** (below): J3's log
took every charge on disk at the listening step, and at the helper's start
too, whether the card had shown it or not; J4's ledger takes only the ids the
card drew, and holds every possible charge from the moment it is known.

Report lines: `assist feelings:` gains `track=current|other:N|gone` and
`notes@`; `assist feelings notes:` (the notes as drawn, or `none`);
`assist feelings nav:` gains `track=` and `switch=yes|refused`;
`assist routing: stale-confirmation …` when Start refuses a stale card. Probe
seam: `MUSIALIZER_ASSIST_PROBE_STAGE_TRACK=N` stages the bridge for track N.
Gates: `tools/headless_check.sh --skip-render` (J1 block: `billed-same`,
`billed-settings`, `billed-audio`, `billed-behind`, `settings-nothing-to-paid`,
`settings-paid-to-nothing`, and the new stop words in `cancel` and `deadline`);
`tools/look_assist_check.sh` feelings 6 (repairs and reuse notes) and 7 (a row
of another track's answer).

**Negative controls (J3, 2026-11-16)**, each perturbed, seen to fail, and
restored byte for byte (`cmp` against a saved copy):

| Perturbation | What failed |
| --- | --- |
| `_dead_records` and `_remove_dead` ignoring the lock (every record dead) | `test_two_helpers_on_one_folder_never_remove_each_others_record`, `test_a_record_in_flight_is_not_an_earlier_charge` |
| `is_sent_record_name` accepting only the J1 name | `every_attempts_record_is_read_and_only_records_are` |
| The catch-all receipt not written | `NoFailureEndsWithoutAReceipt` (5 tests) |
| `request_key` not written into the record | `test_the_record_and_the_status_name_the_same_request_by_one_digest` |
| `ChargeRelation` never `SameRequest` | `an_earlier_charge_says_whether_it_was_this_request`; gate: billed-same (2) |
| The status key built from `compact()` again | `a_saved_answer_is_never_said_for_other_provider_settings`; gate: settings-nothing-to-paid (the card said "Cost: nothing" after the change) and settings-paid-to-nothing |
| Start's stale-card check off | gate: settings-nothing-to-paid and settings-paid-to-nothing (the refusal never came, and the stand-in run started: "the run started for settings the card did not show") |
| The manifest's counts not read (`semantic_repairs` ignoring `semantic_repair_counts`) | `the_repairs_made_to_an_answer_are_said_in_the_cards_words`; gate: feelings 6 (the uncounted sentence drawn) |
| `_repair_counts` always empty | `RepairsAreCountedForTheCard` (2 tests) |
| `AnswerTrack::of` always `Current` | `a_staged_answer_belongs_to_its_track_and_says_so_elsewhere`; gate: feelings 7 twice (no note; no switch, and `SeekOnTrack` kept the current track's playhead at 30.000 instead of moving it) |
| The log not filtered by the panel (since J4 the helper's `pending_charges` filters, and the panel draws what `semantic-status` names) | `a_request_sent_and_never_answered_is_said_before_the_next_start`; gate: billed-behind (the logged receipt's line drawn again: "… on 2026-11-16 11:00 UTC, may have been billed …") |
| `acknowledge_possible_charges` appending nothing (the function is gone since J4; its controls are J4's) | `test_an_earlier_possible_charge_is_logged_once_at_the_next_start` and four more in `ARequestThatLeftIsRecorded` |
| `sent_clause` back to "had already been sent" | `a_stop_after_the_request_left_says_it_may_have_been_billed`; gate: cancel, deadline |

Not tried for real: any of this against OpenRouter (fake transports, real
`SIGKILL`ed helper processes in the tests, a stand-in helper in the gates);
a Start pressed through the window that runs the real orchestrator's
acknowledgement (the gate calls the same function on the folder, then draws
the card); a settings change through the AI settings dialog itself (the gate
writes `assist.json` while the card is open); two application windows on one
job folder.

### One ledger of possible charges (J4, 2026-11-16)

A third review, of J3's diff, found six defects, all in how possible charges
are kept; the lead read each in the code and decided the design built here.
Nothing sent to the model changed (`build_request`, the schema, the prompt and
the provider block are J3's byte for byte; the body pins in
`tests/test_semantic_openrouter.py` hold unchanged).

Five of the six were one fault, three reviews old: a possible charge was
derived from files that get replaced. (1) The may-be-billed receipt was one
file per job folder: with two helpers, the second's receipt replaced the
first's, or its answer removed it as older, and the first charge was in no
record. (3) Start acknowledged every charge on disk at the listening step,
including one that appeared after the card was drawn and was never shown.
(4) A kill between publishing the receipt and removing the sent record left
both, under different `(kind, at_ns)`, and one request was counted twice.
(5) Only the newest 16 charges were kept, and every dead record was removed.
(6) `os.write`'s count was ignored, so a short write followed by a good
`fsync` lost lines, and the records were removed anyway. The sixth, (2): "Cost:
nothing" bound nothing; a saved answer gone or replaced between the card and
the press made the run pay.

**The ledger.** `semantic.cache.charges.jsonl` (the J3 file, its lines now
`musializer.semantic-charge/v2`; PHASE0 §6.7f) holds every possible charge of
the job folder, one line per event, keyed by the attempt's id (the sent
record's `attempt`; the receipt and the cached answer carry it too):

- `possible`, appended **before** the file that told of it can be replaced or
  removed: by the helper, just before it writes a may-be-billed receipt
  (`by: receipt`); by a sweep, for a dead sent record or a file an earlier
  build left, just before it removes the record (`by: sweep`).
- `answered`, by a sweep, for a dead record whose attempt ended without a
  possible charge: its answer is the cache (a kill after writing the answer,
  before removing the record), or its receipt says it was refused or billed a
  stated amount.
- `acknowledged`, by the Start pressed after the confirmation showed it, for
  exactly the ids the confirmation drew (`by: start`).

Pending = possible − acknowledged − answered. Every append writes the whole
buffer (`os.write` until every byte is out), `fsync`s, syncs the folder, under
`semantic.cache.charges.lock`, and only then is a source removed; an append
that fails is a `LedgerError` and the source stays: a may-be-billed attempt
keeps its record (lock given back, so the next sweep reads it), a sweep
removes nothing. A ledger that cannot be read (an error, not a regular file,
over 16 MiB) is reported unreadable, never read as empty; a torn last line (a
crash mid-append) is skipped, its source was never removed, and the next
append starts on a line of its own. A receipt is now only a message: another
helper's receipt or answer may replace it and nothing is lost. J3's
per-attempt records and their `flock` are kept; no job-folder run lock is
added.

The sweep runs at the helper's start and at the orchestrator's listening step.
It no longer acknowledges anything: J3's helper-start acknowledgement put
behind the user charges nobody had shown them. An earlier build's receipt
beside the cache is removed by the next attempt only after a sweep succeeded
(it may be the only word of its charge); a success removes an older receipt
under the same condition.

**What earlier builds left is counted once.** v1 and v2 sent records, J1 and
J3 receipts, J1's `earlier` lists and J3's log lines are read under the id the
J3 log already used for them: `unanswered@<sent_ns>` for a record or an
`earlier` entry, `may_be_billed@<failed_ns>` for a receipt (a receipt with
only `failed_utc`, which J3 could not count, is placed at the end of its
second). A J3 log line is that charge, possible and acknowledged. A J3
receipt and a J3 record of the same attempt left by a kill cannot be told
from two attempts (neither names the other), so they count as two: the safe
side, and only for files a J3 build left in a window of milliseconds. The
operator's real job folder from the current build
(`build/analysis/050c5e1b3af2a397/` in the main checkout: a cache, the J3 lock,
no log, no record, no receipt) reads, from a copy, as no pending charge and a
readable ledger.

**The card says what Start will be held to.** `semantic-status` (read-only)
names every pending charge with its id (`pending_charges`), whether the
ledger could be read (`ledger`), and the saved answer's `cache_key`. The
panel no longer reads the folder for charges: it draws what the status names,
and where it draws the card it keeps what it drew (the saved answer's key
when it drew "Cost: nothing", the ids its charge line covers). Start, which
the frame loop handles after that drawing, hands both to the run
(`--reuse-answer KEY`, `--acknowledge-charge ID` per id; neither is secret,
and only well-formed ids reach the argv). The status is asked again whenever
a card opens anew (after a dismissal, a Start, a finished job), so a charge
that appeared while a card was open stays pending and is drawn by the next
card. A card with no status answer yet promised nothing and showed no charge:
Start then acknowledges nothing and binds nothing.

**Nothing is truncated (J4-5).** The card shows every pending charge: one
sentence for one, and for several their count and summed ceiling ("3
listening requests for this track may have been sent and billed (at most
0.219 USD together); the last was on …"), so every charge it covers was shown
and all of them are acknowledged. The status answer may be up to 1 MiB
(`STATUS_OUTPUT_LIMIT`, was 64 KiB), about 3 000 charges, and is read while the
query runs (read after its exit, an answer past the pipe's 64 KiB blocked the
child until the 15-second timeout); an answer larger than that is no answer, the card keeps the paid wording and shows no charge line,
and nothing is acknowledged, so no charge is lost.

**"Cost: nothing" binds the run (J4-2).** With `--reuse-answer`, the
orchestrator finds the saved answer through the same `saved_semantic_answer`
as before; when there is none (gone, or replaced by an answer for other
settings), it writes a receipt and sends nothing. An answer of another key the
run would still reuse (one for the same settings, made under the bounded
schema, replacing one from before it) is reused: nothing is sent either way,
the staged notice says when that one was made, and the job log says it is not
the one the card named (see the negative controls). The reverse, a
card that showed a cost and an answer that has appeared since, reuses it and
says so, as before ("Reused the answer saved on …; nothing was sent or billed
this time."). The order at the listening step: the reuse check, then the
acknowledgement (a refused run leaves the charges pending, so the next card,
which shows what a new request costs, shows them too), then the sweep.

**Every sentence the user reads, new or changed:**

| Where | Words |
| --- | --- |
| notice, the saved answer gone | title "The saved answer is no longer there" — "The saved answer the card offered to reuse is no longer there, so nothing was sent or billed. Open the card again: it now says what a new request would cost." |
| notice, the ledger refusing the acknowledgement | title "The listening request was not sent" — "Could not write the job folder's charge log (No space left on device), so the earlier requests the card showed were not put behind you; nothing was sent." |
| card, the ledger unreadable | "This track's record of earlier listening requests could not be read, so this card may not show every request that may have been billed." |
| card, several charges, the last with no time | "2 listening requests that used this track's job folder may have been sent and billed (at most 0.146 USD together). After Start …" (was "…; the last was." with nothing after "was") |

Every other sentence is J3's and still true: "this note then moves to the job
folder's log" is what the acknowledgement does. "Open the card again: it now
says what a new request would cost" is true because the job's end forgets the
status, and the card, opened again, asks it.

Report lines: `assist commitment: reuse=<key|-> charges=<ids|->` (what Start
carries); `assist charges:` gains `ids=`; `assist reuse:` gains `charges=` and
`ledger=`; `assist notes:` gains `ledger-unreadable=`. Gate:
`tools/headless_check.sh --skip-render`, J1 block: the stand-in's
`semantic-status` answers through the real `pending_charges`, and its run
acknowledges through the real `acknowledge_charges` and refuses with the real
receipt writer; `billed-look` and `billed-press` (a killed helper's record and
a may-be-billed receipt drawn as "2 listening requests … (at most 0.146 USD
together)", a third request killed after the card's status was taken, the
press carrying exactly the two drawn ids, read from the stand-in's argv);
`billed-behind` (the next card says only the third); `answer-gone` (the card
drew "Cost: nothing", the press carried its key, the notice above).

Tests: `tests/test_semantic_openrouter.py`, `TheLedgerKeepsEveryCharge` (two
real concurrent helpers for finding 1, real helpers `SIGKILL`ed at each point
of a may-be-billed ending and after an answer or a refusal for finding 4,
forty charges for 5, a seven-byte `os.write` and a full disk for 6, an
unreadable and a torn ledger, every legacy file at once, the gone and the
replaced answer for 2); the J3 classes rewritten for the ledger;
`tests/test_helper_argv.py` (the two flags, ids refused); Rust
`diagnosis` (`status_charges`, `is_charge_id`), `runtime::process::assist`
(`the_cards_commitment_rides_along_and_nothing_else`) and the panel
(`start_carries_what_the_card_drew_and_nothing_else`);
`tests/test_cross_language_pins.py` (record schemas and ids on both sides).

**Negative controls (J4, 2026-11-16)**, each perturbed, seen to fail, and
restored byte for byte (`cmp` against a saved copy; the tree clean after):

| Perturbation | What failed |
| --- | --- |
| One `os.write`, its count ignored | `test_a_short_write_is_finished_not_lost` |
| The record removed though the ledger refused the line | `test_a_may_be_billed_ending_the_ledger_refuses_keeps_its_record` |
| The `possible` line appended after the receipt, not before | `test_a_kill_anywhere_in_a_may_be_billed_ending_is_one_charge` (killed at the receipt: no charge of that kind), `test_a_second_helpers_answer_does_not_erase_the_first_ones_charge` |
| A receipt's charge under its legacy id, not its attempt | `test_a_kill_anywhere_in_a_may_be_billed_ending_is_one_charge` (two charges for one request) |
| A dead record of the cache's attempt not read as answered | `test_a_kill_after_an_answer_or_a_refusal_is_no_charge` |
| Start acknowledging every pending charge, not the ids given | `test_start_acknowledges_exactly_the_charges_the_card_showed`, `test_a_start_with_nothing_shown_acknowledges_nothing` |
| Pending kept to the newest sixteen | `test_every_charge_is_kept_however_many` |
| An unreadable ledger read as empty | `test_a_ledger_that_cannot_be_read_is_said_not_read_as_empty` |
| A sweep removing records the ledger refused | `test_a_ledger_that_cannot_be_written_keeps_every_source` |
| J1 `earlier` lists not read | `test_files_left_by_the_j1_and_j3_builds_are_counted_once` |
| J3 log lines not read | the same |
| The reuse key ignored by the run | `test_a_saved_answer_gone_since_the_card_was_drawn_is_not_paid_for`, `test_the_command_line_carries_the_commitment` |
| A free answer of another key refused | `test_another_answer_for_the_same_settings_is_reused_not_refused` |
| The acknowledgement's refusal swallowed | `test_a_ledger_that_cannot_be_written_keeps_every_source` |
| The status naming no charges | `test_the_record_and_the_status_name_the_same_request_by_one_digest` |
| `is_charge_id` (Rust) accepting anything | `a_charge_id_is_an_attempt_or_a_legacy_id_and_nothing_else` |
| A status charge with no time dropped | `every_pending_charge_is_counted_and_summed_up` |
| The runtime not filtering ids | `the_cards_commitment_rides_along_and_nothing_else` |
| The status answer not read while the query runs | `a_long_status_answer_is_read_whole_and_a_too_long_one_is_none` |
| Start's commitment not filtered by track | `start_carries_what_the_card_drew_and_nothing_else` |
| Gate: Start carrying no charges and no key (both at once) | billed-press ("the card drew unanswered@…,may_be_billed@…, Start carried <none>"), billed-behind (all three drawn again), answer-gone (the stand-in's "the run was not given the card's key (got None)") |

One control first came back missed: the run compared the card's key with the
answer it found and refused on a mismatch, and comparing by presence alone
failed no test. The only accepted answer of another key is one for the same
settings (made under the bounded schema, replacing one from before it); it
costs nothing, and refusing it would have said "it now says what a new
request would cost" over a card that would say "Cost: nothing". So the run
refuses only a send: such an answer is reused, its own date is said, the log
names the replacement, and a test pins it (the control above).

Not tried for real: any of this against OpenRouter (fake transports; real
helper processes in the tests, killed with `SIGKILL`; a stand-in helper in the
gate, which calls the real ledger functions); a disk that fills during a real
append (simulated by a refusing or short `os.write`); two application windows
on one job folder (two helper processes are what the folder sees, and those
are tested); a status answer over 1 MiB.

**The real presses on the J4 build (the lead, 2026-11-16)**, through the
window (headless, the operator's settings copied, his key read by the app;
about 0.07 USD in all): Start on the saved answer reused it and sent nothing;
Start with the answer set aside made one call and staged it, with no sent
record and no ledger left; then a helper killed with `SIGKILL` half a second
after its sent record appeared. The window said "The listening request may
have been billed … (at most 0.071 USD). Nothing was saved."; the next
confirmation carried "A listening request for this track and these settings
was started on 2026-11-16 18:38 UTC and never answered; it may have been sent
and billed (at most 0.071 USD). Start sends the same request again; this note
then moves to the job folder's log."; Start carried exactly that attempt's id
(`assist commitment: reuse=- charges=<attempt>`), the ledger took an
`acknowledged` and a swept `possible` line for it, the record was removed, a
new answer was staged, and the card after that warned of nothing.

**A receipt is never replaced before its charges are kept (the fourth review,
2026-11-16).** One loss was left: a may-be-billed receipt an earlier build
wrote, with its `earlier` list, is the only word of those charges until a
sweep has run, and a Start the orchestrator refuses (the saved answer gone,
the ledger refusing the acknowledgement) wrote its own receipt over it before
any sweep. `_write_failure_receipt` now sweeps first, whoever calls it; when
the ledger refuses the sweep, the outgoing receipt's legacy charges ride in
the new receipt's `earlier`, the J1 shape every reader still reads, until a
sweep can take them. Pinned by
`test_a_refusal_does_not_replace_the_only_word_of_an_earlier_charge`, written
first and seen to fail both ways (ledger writable, ledger a directory) with
"[] != ['unanswered@1', 'may_be_billed@2']". The same review's two lesser
findings are queued in `FEATURE_PARITY_PLAN.md` (GX-5).

**Start stays on screen when the card grows.** That press found the one thing
the gate had stepped around with a taller window: the confirmation is brought
into view when it opens, and the warning arrives with the status query a
moment later, 45 px of it, so at 1280x720 Start and Cancel sat below the
column's fold exactly when the card had something to say about money. A
confirmation that grows while open is now brought into view again
(`AssistColumn::shown_height`; only the confirmation, so a staged list that
grows under a press does not jump away from the row pressed). The gate's
`billed-fold` runs the warned card at 1280x720 and requires Start to end above
the status strip. Negative control: with the growth ignored it fails ("Start
is below the column's fold (y, height, status strip: 703 30 696)"); restored
from a saved copy, it reads 643 30 696.

### Local performed-phrase recovery (2026-09-08)

The recommended Timed lyrics workflow stays local, including tracks without
an authored sheet. Its built-in `TC-WORDING/local-transcript` stage retains
Whisper text as uncertain evidence; it never calls a text model. Explicit
Codex and Antigravity overrides remain available.

For authored tracks, `local_lyric_recovery.py` runs after anchor/block alignment.
It transcribes overlapping 30-second crops, starting at offsets 0 and 5 seconds
of each 15-second step, then aligns each crop's recognized text with MMS. The
written sheet supplies phrase templates and spelling, not evidence of presence.
A caption requires identical normalized words in at least two distinct crops,
with both acoustic edges agreeing within 350 ms. An ASR interval guard rejects
CTC words more than one second outside the recognizer's own phrase interval:
two CTC passes can agree on the same wrong occurrence. Explicit stutters retain
the first syllable's onset; repeated complete phrases retain distinct times.

Corroborated audio can retime a written cue or replace an incompatible compound
cue with separate performed phrases. Displaced written lines remain unresolved
with their original identity and proposed timing. Added captions carry
`audio_observed`, unknown confidence and review flags. This is a proposal policy,
not independent acoustic acceptance: shared Whisper/MMS errors remain possible.

Crop receipts are cached by audio, decoder/model settings, model and executable
hashes, and crop span. Recovery policy version 3 invalidates old aligned results
while preserving reusable crop evidence. The normal helper invokes no Gemini
calls for this stage. See [the recording investigation](archive/LYRICS_RECORDING_2026-09-08.md)
for the measured change and remaining failures. First runs cost additional local
ASR work; changing only recovery policy can reuse the crop cache.

### 4. Validation and staging

[`analysis_bridge::parse`](../crates/musializer-core/src/project/analysis_bridge.rs)
accepts a bounded, ordered schema containing an audio digest and any of three
lanes: lyrics, sections, and semantics. The Python writer validates its own
output, but the Rust parser independently checks it because the helper boundary
is untrusted.

`load_candidate` (`app::ui::panels::assist`) also checks that the digest identifies the
selected audio and that duration agrees within the bounded decoder/container
tail. [`AnalysisCandidate::prepare`](../crates/musializer-core/src/project/analysis_candidate.rs)
retains only authorized lanes and validates their project-level invariants.

Only then does the panel show a staged result. Apply rechecks lane authority and
updates the target track; Discard clears the candidate without changing editor
content. The batch `--analysis-bridge FILE` path is intentionally different: it
applies immediately because it has no interactive review step.

### Looking at a staged feelings answer (J2, 2026-11-16)

A paid listening answer used to stage as "Feeling cues: 3 → 9" and nothing
else, and Apply replaced the lane blind and for good. The model's sections are
an opinion (in the lead's three real calls one song was cut well, another had
a slightly quieter stretch called a quiet breakdown and a hook's return placed
eight seconds early), so looking before applying is the feature:

- **The candidate keeps what the review needs.** `AnalysisCandidate::prepare`
  carries each semantic cue's end and the model's words beside the event lane
  as `FeelingSection`s (`feeling_sections()`); the lane, and what Apply writes,
  are unchanged. No `.musi`, bridge or Apply change was needed.
- **The staged card lists the sections**
  (`app::ui::panels::assist::feelings`): a head `SECTIONS · N` with the marks'
  names, an energy strip of the whole song (a column per section as tall as
  its energy, the one under the playhead in ink), then one row per section:
  its span, a line of the model's words (all of them, wrapped to six lines,
  when the row is open, with how sure the model was), and three marks — energy
  a solid bar, tension a comb on the same scale, mood (valence, −1…1) a stem
  from a centre line ending in a dot. Five rows show; the list scrolls inside
  the card (the wheel over the rows moves the list, then the column), and the
  row under the playhead is tinted and comes into view as the song plays.
- **A row is a place in the song.** Pressing it opens it and seeks 50 ms into
  the section (`FeelingSection::seek_seconds`, the TRAPS rule on seeking onto
  a boundary), paused or playing. The strip is not a control.
- **The staged cues are on the timeline** while they are staged for the
  current track: a dashed line standing on a thin ring at the lane's bottom,
  beside the placed cues they would replace (solid lines, heads at the top)
  and the user's own markers (filled heads). Discard takes them away with the
  candidate.
- **Apply says what it does first**: "Apply replaces the 3 earlier feeling
  cues with the 11 from this answer. Your own markers, the scene plan and the
  lyrics stay as they are. You can put the earlier cues back afterwards." Each
  clause is what `apply_candidate_to_track` does (the semantic lane whole, the
  lyric lane and the plan only when staged, `manual_events` never), pinned by
  `a_feelings_apply_touches_neither_markers_nor_plan_and_steps_back_exactly`.
- **One step back.** An Apply that replaces feeling cues remembers the lane and
  its `.musi` analysis-lane record (`FeelingsStepBack` on the session); the card
  that applied offers **Put back** ("3 earlier cues kept"). It is offered while
  the lane is what that Apply wrote on the track it wrote it to: another such
  Apply replaces the memory with its own, a closed project or any change to the
  lane (another revision or other cues: an edit, an import, a reopen, a
  followed treatment) ends it, and the card then says "Earlier cues gone ·
  project closed" or "· edited since". Putting back restores the cues and the
  record in its old place (or removes the record the Apply added) and dirties
  the project, as the Apply did, so autosave writes it; the gate compares the
  saved file byte for byte with the one before Apply. No general undo.
- **A reused answer** is said in the head when the job's manifest says so. The
  key is the other agent's of the same wave; the reader takes
  `semantic_reused` (true, or an object with `answered_utc`/`made_utc` or
  `words`), `semantic_reuse` and `semantic_answer`/`semantic_cache` with
  `reused: true` (`semantic_reuse_words`). Nothing depends on it.

Report lines: `assist feelings:` (rows by id with rects and spans, `now=`,
`open=`, `first=`, `shown=`), `assist feelings nav:` (a press and its seek),
`staged feelings:` (staged, ghosts drawn, placed cues replaced),
`assist step back:` (`state=offered|gone:closed|gone:edited|none`, the card,
the button's rect). Probe seams: `MUSIALIZER_ASSIST_PROBE_STAGE=bridge|armed|applied`
stages `MUSIALIZER_ASSIST_PROBE_DIR`'s `result.bridge.tsv` against the current
track through `load_candidate` (armed: Apply's first press taken; applied: its
second press's request sent, so the run continues after a real Apply);
`MUSIALIZER_ASSIST_PROBE_FEELINGS_OPEN=N` and `…_SCROLL=N` for captures;
`tools/feelings_fixture.py` writes the job folder from a real answer or an
invented one. The gate is `tools/look_assist_check.sh`'s `feelings` group.

Left: the energy strip is not pressable; the step back covers the feeling
cues only, also after a Full assist Apply (the lyrics and plan it replaced
are not remembered); at 960x640 with a row open the Apply row needs a scroll.

## Lyrics decision tree

The production path separates three questions that one model should not be
allowed to answer implicitly:

1. **What are the words?** Prefer authored display text from an explicit lyric
   sheet, sibling file, or embedded metadata. Without a reference, Whisper plus
   review supplies provisional cue text and order.
2. **Where in the song are they sung?** Whisper timestamps are proposals, not the
   final clock, and since tranche LT1 they are not the search space either.
   With authored text the song is localized globally first — rare n-gram
   anchors partition it into ordered blocks — and CUDA-backed TorchAudio MMS
   then forces each block's complete consecutive text through one CTC path.
   Without authored text the transcription *is* the text, so the older per-cue
   MMS refinement still applies.
3. **Should a questionable cue survive?** Confidence is evidence, not authority.
   An authored line the acoustics could not place becomes `unresolved` and
   named for review; it is never dropped. A no-reference cue is removed only
   with corroborating evidence such as weak Whisper plus weak alignment, or
   duplicate candidates claiming the same acoustic span.
4. **Did the performance add words the sheet never contained?** Short Whisper
   spans outside every authored placement are retained as a separate
   `performed_candidates` lane. They may be Suno ad-libs, generated repeats or
   background vocals, but they may also be ASR errors. The application parks
   each as a non-rendering `Potential` cue and labels it `HEARD`; only a human
   promotion can make it a published caption. Long tail segments and known
   repetition-loop intervals are discarded rather than presented as vocals.

```text
                     authored text available?
                       /                 \
                    yes                   no
                     |                     |
       preserve text and order      Whisper transcription
              |                     + conservative review
    rare anchors + trusted blocks             |
              |                       per-cue MMS alignment
    unconditioned global challenger           |
              |                        strong evidence?
    occurrence agreement or rare anchor?     /       \
          /                 \              yes       no
        yes                 no              |         |
   bounded cue       unresolved + flag    refine   uncertain proposal
```

The pure half of that policy is
[`lyric_anchor_block.py`](../tools/lyric_anchor_block.py) — anchors, blocks,
abstention, review flags and the coverage guard, all without a model. The
acoustic half is [`anchor_block_align.py`](../tools/anchor_block_align.py),
which runs under the installed alignment runtime. The no-reference lane stays in
[`force_align_lyrics.py`](../tools/force_align_lyrics.py). Either way the output
`lyrics.aligned.json` contains the audit evidence used to construct the bridge;
the bridge contains the bounded result the Rust application needs, and an
unresolved line is never in it. Performed candidates also stay outside the
bridge's accepted cue list; Rust reads them from the review artifact and parks
them with `Potential` origin so Apply cannot turn unreviewed ASR prose into a
rendered lyric.

### Anchor→block localization (LT1)

The previous path made Whisper the *authority*: a line it missed was omitted
before MMS ever saw it, and a line it kept was searched only near its own
proposal. Both classes were measured — the coverage canary lost its two outro
lines to a 90-second repetition loop. The benchmark in
[`LYRICS_TIMING_BENCHMARK_RESULTS.md`](archive/LYRICS_TIMING_BENCHMARK_RESULTS.md)
selected anchor→block, and it is now the production default.

- **Coverage.** Every alignable authored line reaches the acoustic stage,
  including the block before the first anchor and after the last. An
  unlocatable line becomes an `unresolved` record with a named reason; the
  helper refuses to write a lane that lost one (`validate_full_coverage`).
- **Abstention.** A repeated authored phrase abstains when the coarse
  Whisper-derived view puts it nearer a *sibling* occurrence's block placement
  than its own, or when two identical lines collapse onto one phrase. Any
  occurrence-scale fine/coarse dispute, backwards path without a deciding
  margin, or unanchored section that disagrees with the unconditioned global
  path also abstains. Guessing an occurrence confidently is worse than saying so.
- **Review flags.** A flag is cross-view disagreement, a rare anchor resolving
  disagreement with the unconditioned path, or an unresolved line. It is
  **never** the aligner's
  own score: the 2026-08-04 operator adjudication measured median score 0.139
  on confirmed-correct lines against 0.142 on confirmed-wrong ones, and cues
  therefore carry `confidence: null` rather than a number that orders nothing.
- **The coarse lane is demoted, not deleted.** `lyrics.sync.json` is still
  written for candidate windows, flags and abstention. Estimated and sub-0.8
  rows are review-only; they cannot narrow acoustic search. Its cache identity
  records `role: coarse_proposal`.

### Whisper evidence pass

whisper-cli 1.8.6 has no `--no-context` flag — the library's `params.no_context`
is never wired to an argument — but `--max-context 0` reaches the same place:
`whisper.cpp:7097` skips history conditioning entirely when `n_max_text_ctx` is
zero. Assist passes it on every run. On the canary that alone removed the
repetition loop and transcribed the first outro line at 90.6 s; across the four
benchmark tracks duplicate segments fell 31→13, 27→11 and 22→0, with the choir
track gaining evidence (32→55 segments).

Its VAD (`--vad`/`--vad-model`) is available and **off by default**. Measured on
2026-08-04, Silero v6.2.0 rejects sung vocals over accompaniment almost
completely: the canary kept one 0.4-second segment at the default 0.50
threshold and three at 0.10. `MUSIALIZER_WHISPER_VAD_MODEL` enables it for an
operator who wants it; a path that is not a readable file logs the reason and
continues without VAD rather than failing the job. Whichever ran is recorded in
the lane's `request_settings` (`text_conditioning`, `vad_model_sha256`), so a
lane decoded under the other policy is regenerated rather than reused.

## Why Whisper timing alone was insufficient

The investigation found several independent failure classes rather than one bad
constant:

- whisper.cpp timestamp control tokens were initially parsed as text, stretching
  a preceding word to a segment boundary;
- DTW provenance could claim a path that flash attention had disabled, while real
  DTW degraded badly on repeated singing;
- ordinary Whisper token onsets were useful but token ends often inherited a
  coarse segment tail;
- global repeated-word matching could choose one distant occurrence and stretch
  a cue across tens of seconds;
- interpolation could place missing lines in gaps too small to contain them;
- even correctly parsed Whisper onsets were often early, sometimes by several
  seconds.

Parser and matching repairs remain necessary, but an independent acoustic
alignment stage is what makes cue boundaries defensible.

## Forced-alignment policy

This section describes the **no-reference** lane. With authored text the
anchor→block localizer above replaces it, and batching is not optional there:
aligning a block's lines together is precisely what disambiguates a repeated
phrase by its ordered neighbours.

MMS runs one acoustic request per cue. Batched repeated lyrics can change which
occurrence the aligner selects according to neighboring cues, so batching is not
an equivalent optimization.

The search window is asymmetric: it opens 0.75 seconds before the proposal and
extends 6 seconds after it. That encodes the measured direction of Whisper's
singing error while limiting the chance that an earlier repeated phrase steals
the cue. Boundary replacement is conservative:

- strong acoustic evidence may replace provisional boundaries;
- a broad cue may be narrowed to a supported acoustic span inside it;
- large boundary moves require support from the relevant boundary word;
- weak decisions keep input timing and become uncertain;
- low CTC score alone never deletes sung text.

Policy constants and accepted output can change together only with regression
evidence and a new policy/cache version.

## Authority and provenance

| Artifact or value | Authority |
| --- | --- |
| Authored lyric text and ordering | Explicit sheet, sibling file, or embedded metadata |
| No-reference text and ordering | Whisper proposal plus conservative review |
| Authored-line position in the song | Policy v2 consensus: trusted section-bounded block CTC, a coarse-local boundary refinement, and an unconditioned global challenger for every section without its own rare anchor; the candidates remain auditable |
| Final cue timing | Bounded MMS acoustic evidence supported by the coarse occurrence; an occurrence-scale disagreement never becomes a cue |
| Whether a line is placed at all | Independent-view agreement and the global authored order — never the aligner's own score or raw coverage |
| Authored-text source | Exact explicit/sibling/embedded source, digest and alignable-line count in the manifest and staged UI; embedded metadata is never visually anonymous |
| Sections and semantic cues | Measured/model evidence bounded by bridge validation |
| Audio identity | SHA-256 carried by the bridge and verified by Rust |
| Project mutation | Rust `AnalysisCandidate` plus explicit Apply |

Hosted audio-language models are semantic auditors, not timing oracles. During
the acceptance investigation they were useful for phrase presence, order, and
count; their absolute timestamps compressed musical time and candidate timestamp
prompts anchored to the suggestion. Remote output therefore cannot directly
replace a cue boundary.

## Failure behavior

Failure should remain visible at the narrowest boundary that can explain it:

- Dependency discovery and CUDA/model readiness: `musializer_doctor.py`.
- Helper/model diagnostics and intermediate artifacts: the stable analysis
  workspace and job log.
- Process failure, timeout, or cancellation: runtime job state surfaced by the
  Assist panel.
- Invalid schema, wrong audio, duration mismatch, or excess lane authority: Rust
  rejects the bridge and stages nothing.
- A semantic answer (any model) cut off at its cap, empty, unparseable, off-schema or off-scale:
  a failed run before any bridge exists; the job's failure line names the
  reason and the billed cost, and the receipt keeps the provider's `usage`.
- A semantic request answered by any status other than a clear refusal, a timeout
  or a dropped connection after it left: a failed run that says it may have been billed up
  to the cap; never sent again by itself.
- A semantic request too large for its model, or a track FFmpeg cannot encode:
  stopped before anything leaves, with a receipt that says nothing was sent.
- A semantic request refused (key, credit, endpoint, rate limit) or never sent
  (no connection, no key): a receipt that says nothing was billed and what to
  do next, which the panel reads for its notice (J1).
- A semantic request killed after it left (Cancel, a closed window, the
  deadline): the sent record survives, every stop says it may have been
  billed, and the next confirmation says so before Start (J1).
- A track file changed since loading: Start refused before anything is spent;
  a change after the press refused by the helpers before anything leaves (J1).
- Authorized mode producing no changes: a truthful terminal no-change result,
  not an empty candidate.
- Weak boundary evidence within one occurrence: retain timing as uncertain.
  Fine/coarse disagreement above eight seconds, or a backwards authored-order
  placement, is `unresolved` with both proposals retained — never a cue painted
  amber and shipped anyway.
- Coarse-conditioned section and local paths are one evidence family, not two.
  Without a rare anchor in that section they must agree with the unconditioned
  global path; estimated and low-confidence coarse rows cannot narrow a window.
- An authored line that cannot be located, or a repeated phrase whose occurrence
  the global order did not decide: an `unresolved` record and a review flag
  naming the line, never a cue placed on a guess and never an omission.

## Verification map

| Concern | Evidence |
| --- | --- |
| Timestamp-token parsing, cluster selection, alignment policy | [`tests/test_lyrics_timing.py`](../tests/test_lyrics_timing.py) |
| Coverage invariant, section/coarse windows, occurrence/order abstention, source provenance, review flags, Whisper flags, localization cache identity | [`tests/test_lyric_anchor_block.py`](../tests/test_lyric_anchor_block.py) |
| Four-track localization coverage through the production path | `tools/lyrics_research/run.py --method baseline` plus `scoreboard.py`; gitignored `build/lyrics-research-v2` artifacts |
| Assist state and mode authority | core tests, anchored by `tools/differential_assist_ui.sh` (341 policy decisions; run deliberately, not part of `verify.sh`) |
| Process start, timeout, cancellation, and reaping | runtime `process::assist` tests |
| Bridge bounds, identity, coverage, and staging | core bridge/candidate tests and app panel tests |
| Semantic request (MiMo and Gemini rows): cap, failed runs, cost estimate, numeric scales, Gemini's request shape and encoding, MiMo's unchanged identity | [`tests/test_semantic_openrouter.py`](../tests/test_semantic_openrouter.py) (fake OpenRouter; one real FFmpeg encode of a synthetic tone), the `SemanticCostEstimate` pins in [`tests/test_cross_language_pins.py`](../tests/test_cross_language_pins.py), `core::assist::cost` tests |
| The Feelings Start path (J1): stale file, receipts for every failure, the sent record, bridge fit and legacy answers, the older model list, the saved-answer query | `tests/test_semantic_openrouter.py`, `tests/test_assist_orchestrator.py`, `tests/test_cross_language_pins.py` (`ListeningReceipt`, `BridgeLimits`), `tests/test_helper_argv.py`, `core::assist::diagnosis` and `ui::panels::assist` tests, `process::assist` tests, and the `J1` section of `tools/headless_check.sh` |
| The default model's move and settings inheritance, the stale-catalog rule | `core::assist::execution` tests (`the_old_default_moves_to_gemini_and_a_stored_mimo_route_stays`, `a_catalog_older_than_the_model_cannot_say_it_is_gone`) |
| Execution snapshot: route resolution at Start, provenance, mid-job settings immutability | [`tests/test_assist_execution.py`](../tests/test_assist_execution.py), `core::assist::execution` tests, [`crates/musializer-runtime/examples/assist_canary_probe.rs`](../crates/musializer-runtime/examples/assist_canary_probe.rs) |
| Installed helper and local-lyrics assets | `python3 tools/musializer_doctor.py --require local_lyrics` |
| Support workflow | `tools/support_bundle_check.sh` |
| Full repository gate | `tools/verify.sh` |
| Two metadata plus two stripped-track acceptance | [`LYRICS_TIMING_INVESTIGATION.md`](archive/LYRICS_TIMING_INVESTIGATION.md) and gitignored `build/lyrics-investigation` artifacts |

The dated investigation also records the controls that failed: restoring the old
special-token parser, weakening the coherent-cluster bound, enabling real DTW,
batching CTC cues, deleting every low-score cue, using a symmetric search window,
and retaining duplicate candidates on one acoustic span.
