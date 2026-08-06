# Assist and lyrics pipeline

This document explains the current automatic-analysis path as a maintainable
system. For installation, dependency discovery, commands, and privacy details,
use [`tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md). For the dated
experiment log and acceptance evidence behind the lyrics policy, use
[`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md). For provider
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
- optional Codex or explicitly authorized OpenRouter semantic review, routed
  through a per-job task-contract graph resolved once at Start (see
  [`ASSIST_PROVIDER_CONTRACTS.md`](ASSIST_PROVIDER_CONTRACTS.md) §1 and §6);
- scene-plan construction and bridge serialization.

The stable output directory permits reuse, but cache provenance includes source
and audio identities plus the relevant model, policy, and settings versions.
Changing a timing policy must invalidate the artifact it changes.

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

```text
                     authored text available?
                       /                 \
                    yes                   no
                     |                     |
          preserve text and order    Whisper transcription
                     |                + conservative review
      Whisper words as evidence              |
                     |                       |
      rare unique n-grams -> anchors         |
                     |                       |
      ordered blocks (initial and            |
      terminal blocks included)              |
                     |                       |
      one CTC pass per block          per-cue MMS alignment
                     |                       |
      placed? -- no --> unresolved   strong evidence? -- no --> keep
         |              + flagged          |                    proposal,
        yes                               yes                   uncertain
         |                                 |
   repeated phrase the global       replace/narrow timing only
   order did not decide?
         |
        yes --> abstain: unresolved + flagged
```

The pure half of that policy is
[`lyric_anchor_block.py`](../tools/lyric_anchor_block.py) — anchors, blocks,
abstention, review flags and the coverage guard, all without a model. The
acoustic half is [`anchor_block_align.py`](../tools/anchor_block_align.py),
which runs under the installed alignment runtime. The no-reference lane stays in
[`force_align_lyrics.py`](../tools/force_align_lyrics.py). Either way the output
`lyrics.aligned.json` contains the audit evidence used to construct the bridge;
the bridge contains the bounded result the Rust application needs, and an
unresolved line is never in it.

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
  than its own, by more than the 3-second review tolerance, or when two
  identical lines collapse onto one acoustic phrase. Guessing an occurrence
  confidently is worse than saying so.
- **Review flags.** A flag is cross-view disagreement (coarse proposal versus
  block placement, > 3 s) or an unresolved line. It is **never** the aligner's
  own score: the 2026-08-04 operator adjudication measured median score 0.139
  on confirmed-correct lines against 0.142 on confirmed-wrong ones, and cues
  therefore carry `confidence: null` rather than a number that orders nothing.
- **The coarse lane is demoted, not deleted.** `lyrics.sync.json` is still
  written, because both the flags and the abstention are defined as
  disagreement with it. Its cache identity records `role: coarse_proposal`.

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
| Authored-line position in the song | Anchor→block CTC path over the whole ordered block; Whisper words are evidence for the anchors only |
| Final cue timing | MMS acoustic evidence when strong; otherwise marked provisional timing |
| Whether a line is placed at all | The acoustics and the global order — never the aligner's own score |
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
- Weak lyric evidence: retain timing as uncertain rather than manufacture
  confidence or silently delete a line.
- An authored line that cannot be located, or a repeated phrase whose occurrence
  the global order did not decide: an `unresolved` record and a review flag
  naming the line, never a cue placed on a guess and never an omission.

## Verification map

| Concern | Evidence |
| --- | --- |
| Timestamp-token parsing, cluster selection, alignment policy | [`tests/test_lyrics_timing.py`](../tests/test_lyrics_timing.py) |
| Coverage invariant, repeated-phrase abstention, review flags, Whisper flags, localization cache identity | [`tests/test_lyric_anchor_block.py`](../tests/test_lyric_anchor_block.py) |
| Four-track localization coverage through the production path | `tools/lyrics_research/run.py --method baseline` plus `scoreboard.py`; gitignored `build/lyrics-research-v2` artifacts |
| Assist state and mode authority parity | `tools/differential_assist_ui.sh` and core tests |
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
