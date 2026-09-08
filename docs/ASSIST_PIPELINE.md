# Assist and lyrics pipeline

This document explains the automatic-analysis path as a maintainable system: its
control flow, its trust boundaries, and the reasons its lyric policy is
deliberately more conservative than the code alone suggests. For installation,
dependency discovery, commands, and privacy details, use
[`tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md). For the dated
experiment log and acceptance evidence behind the lyrics policy, use
[`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md) and
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
calls for this stage. See [the recording investigation](LYRICS_RECORDING_2026-09-08.md)
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
[`LYRICS_TIMING_BENCHMARK_RESULTS.md`](LYRICS_TIMING_BENCHMARK_RESULTS.md)
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
| Execution snapshot: route resolution at Start, provenance, mid-job settings immutability | [`tests/test_assist_execution.py`](../tests/test_assist_execution.py), `core::assist::execution` tests, [`crates/musializer-runtime/examples/assist_canary_probe.rs`](../crates/musializer-runtime/examples/assist_canary_probe.rs) |
| Installed helper and local-lyrics assets | `python3 tools/musializer_doctor.py --require local_lyrics` |
| Support workflow | `tools/support_bundle_check.sh` |
| Full repository gate | `tools/verify.sh` |
| Two metadata plus two stripped-track acceptance | [`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md) and gitignored `build/lyrics-investigation` artifacts |

The dated investigation also records the controls that failed: restoring the old
special-token parser, weakening the coherent-cluster bound, enabling real DTW,
batching CTC cues, deleting every low-score cue, using a symmetric search window,
and retaining duplicate candidates on one acoustic span.
