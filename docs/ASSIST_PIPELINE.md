# Assist and lyrics pipeline

This document explains the current automatic-analysis path as a maintainable
system. For installation, dependency discovery, commands, and privacy details,
use [`tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md). For the dated
experiment log and acceptance evidence behind the lyrics policy, use
[`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md).

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
- whisper.cpp transcription and rough word order;
- authored lyric discovery from an explicit sheet, sibling file, or embedded
  metadata;
- local MMS forced alignment for final lyric timing;
- optional Codex or explicitly authorized OpenRouter semantic review;
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

`AssistController::load_candidate` also checks that the digest identifies the
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
   final clock. CUDA-backed TorchAudio MMS forced alignment teacher-forces the
   decided cue text against the audio and produces line- and word-level acoustic
   evidence.
3. **Should a questionable cue survive?** Confidence is evidence, not authority.
   Weak alignment keeps a cue uncertain; it does not erase authored text. A
   no-reference cue is removed only with corroborating evidence such as weak
   Whisper plus weak alignment, or duplicate candidates claiming the same
   acoustic span.

```text
                     authored text available?
                       /                 \
                    yes                   no
                     |                     |
          preserve text and order    Whisper transcription
                     |                + conservative review
                     +----------+----------+
                                |
                       per-cue MMS alignment
                                |
                   strong evidence? -- no --> keep proposal,
                         |                    mark uncertain
                        yes
                         |
                replace/narrow timing only
```

The central implementation is
[`force_align_lyrics.py`](../tools/force_align_lyrics.py). Its output
`lyrics.aligned.json` contains the audit evidence used to construct the bridge;
the bridge contains the bounded result the Rust application needs.

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
| Final cue timing | MMS acoustic evidence when strong; otherwise marked provisional timing |
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

## Verification map

| Concern | Evidence |
| --- | --- |
| Timestamp-token parsing, cluster selection, alignment policy | [`tests/test_lyrics_timing.py`](../tests/test_lyrics_timing.py) |
| Assist state and mode authority parity | `tools/differential_assist_ui.sh` and core tests |
| Process start, timeout, cancellation, and reaping | runtime `process::assist` tests |
| Bridge bounds, identity, coverage, and staging | core bridge/candidate tests and app panel tests |
| Installed helper and local-lyrics assets | `python3 tools/musializer_doctor.py --require local_lyrics` |
| Support workflow | `tools/support_bundle_check.sh` |
| Full repository gate | `tools/verify.sh` |
| Two metadata plus two stripped-track acceptance | [`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md) and gitignored `build/lyrics-investigation` artifacts |

The dated investigation also records the controls that failed: restoring the old
special-token parser, weakening the coherent-cluster bound, enabling real DTW,
batching CTC cues, deleting every low-score cue, using a symmetric search window,
and retaining duplicate candidates on one acoustic span.
