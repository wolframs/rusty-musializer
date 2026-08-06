# MiMo v2.5 description benchmark

**Question.** How exactly can `xiaomi/mimo-v2.5` — the model behind the app's
*MiMo feelings* lane — describe music? Instruments, feel, lyric positioning,
sonic texture, and music-theoretical content (key, tempo, meter, harmony, form).
And which prompt, chunking and output-shaping choices make that description
usable by the application.

**Status.** Designed and built; **not yet run**. No live call has been made. The
harness is dry-run by default and needs two separate gates to send anything.

| | |
| --- | --- |
| harness | `tools/mimo_bench/`, driver `tools/mimo_bench/run.py` |
| tests | `tests/test_mimo_bench.py` (108 offline tests) |
| model under test | `xiaomi/mimo-v2.5`, temperature 0.2 |
| reformatter (arm S3 only) | `openai/gpt-4o-mini`, text only — **confirm slug and price before the live run** |
| output schema | `musializer.mimo-bench-description/v1`, sha256 `7837e23afb77fcfb…` |
| prompts | `musializer.mimo-bench-prompts/v1`, four ids, each hashed |
| matrix | 13 cells x 3 repeats = 39 resumable units, **159 API calls** |
| projected cost | **$0.32 – $0.36** (bracketed over the audio token rate) |
| contract | `TC-SEMANTIC`, boundary `audio-leaves-machine` (`docs/ASSIST_PROVIDER_CONTRACTS.md` §1) |

---

## 1. Hypotheses

Written before the run, so the result can contradict them.

| # | hypothesis | what would falsify it |
| --- | --- | --- |
| H1 | Finer chunking buys **lyric timing precision** and costs **whole-excerpt context** (form, arc). | 20x5 s has no better median lyric error than 1x100 s. |
| H2 | Finer chunking is **not** worth its price: 20 calls cost ~13x one call for the same 100 s of audio. | 20x5 s wins a decision-gate threshold on some dimension. |
| H3 | The **casual/open** register produces more *concrete, falsifiable* statements per 100 words than the strict/checklist one — the operator's prior finding, made measurable. | Concreteness density is equal or lower, or instrument recall drops. |
| H4 | Demanding the **schema in the same turn** (S1) suppresses detail relative to free prose (S0) at equal cost. | S1's concreteness and instrument recall match S0's. |
| H5 | A **separate cheap reformatter** (S3) is not deterministic enough for the application: field-level disagreement over 5 identical-input runs exceeds 5 %. | Disagreement ≤ 5 % and identical-output rate ≥ 0.8. |
| H6 | MiMo's **tempo** claims are octave-ambiguous but not random, and its **key** claims are weaker than its instrument claims. | Tempo accept rate at or below the ~30 % chance baseline. |
| H7 | The chunking answer **replicates** on a second track. | The two tracks rank the chunkings differently. |

---

## 2. Axes and the matrix

A full 4 x 2 x 2 x 4 cross is 64 conditions and, because a 20-chunk condition is
twenty calls, it multiplies into something nobody resumes after a suspend. The
design is **one factor at a time around a named centre**:

> **centre** = chunking `1x100 s`, register **strict**, specificity **checklist**,
> shaping **S1** (single turn, schema demanded with the audio).

Four blocks sweep one axis each. Cells shared by two blocks run **once**.

### Block 1 — chunk granularity, at fixed total duration

The *same* 100 s excerpt, cut four ways. Every chunk is declared with its
absolute source-track offset, so all four conditions are asked for the same clock.

| chunking | chunks | calls/repeat | audio s/repeat |
| --- | --- | --- | --- |
| `c20x05` | 20 x 5 s | 20 | 100 |
| `c10x10` | 10 x 10 s | 10 | 100 |
| `c05x20` | 5 x 20 s | 5 | 100 |
| `c01x100` | 1 x 100 s | 1 | 100 |

### Block 2 — prompt register x specificity

Free text (no schema), so schema pressure cannot confound the register effect.
The **content requested is identical** within a specificity level; only the
wording changes, and the casual prompts carry an actual `:)`.

| | open-ended | checklist |
| --- | --- | --- |
| **strict** | `strict-open` | `strict-checklist` |
| **casual** | `casual-open` | `casual-checklist` |

`strict-open` is verbatim the operator's prior "describe this audio track in
exquisite detail, so that a text-modality LLM can understand it".

### Block 3 — output shaping

All four arms use the same prompt (`strict-checklist`) and the same audio. Only
*where* and *by whom* the schema is imposed changes.

| arm | shape | calls | why it is in the matrix |
| --- | --- | --- | --- |
| `S0` | free text, no schema | 1 audio | the rich description the others reshape; also a Block 2 cell |
| `S1` | one turn, schema with the audio | 1 audio | what the application does today |
| `S2a` | two turns, same model, **audio resent** | 1 audio | chat completions are stateless — a real second turn pays for the audio twice |
| `S2b` | two turns, same model, **audio elided** | 1 text | the model reshapes its own words with no audio in context |
| `S3` | separate cheap text-only model | **5** text | the determinism probe: 5 runs on byte-identical input |

`S2a`, `S2b` and `S3` consume `S0`'s stored turn-1 text rather than re-listening.
That sharing is why the matrix is 53 calls per repeat and not over a hundred.

### Block 4 — replication

`constellation-whale`, `c01x100` and `c05x20`, arm `S1`. Two cells, six calls.

### The cell list

| cell | blocks | chunks | calls/repeat | depends on |
| --- | --- | --- | --- | --- |
| `shut-up-cat/c20x05/strict-checklist/S1` | chunking | 20 | 20 | |
| `shut-up-cat/c10x10/strict-checklist/S1` | chunking | 10 | 10 | |
| `shut-up-cat/c05x20/strict-checklist/S1` | chunking | 5 | 5 | |
| `shut-up-cat/c01x100/strict-checklist/S1` | chunking, shaping | 1 | 1 | |
| `shut-up-cat/c01x100/strict-open/S0` | prompt | 1 | 1 | |
| `shut-up-cat/c01x100/casual-open/S0` | prompt | 1 | 1 | |
| `shut-up-cat/c01x100/strict-checklist/S0` | prompt, shaping | 1 | 1 | |
| `shut-up-cat/c01x100/casual-checklist/S0` | prompt | 1 | 1 | |
| `shut-up-cat/c01x100/strict-checklist/S2a` | shaping | 1 | 1 | `…/S0` |
| `shut-up-cat/c01x100/strict-checklist/S2b` | shaping | 1 | 1 | `…/S0` |
| `shut-up-cat/c01x100/strict-checklist/S3` | shaping | 1 | 5 | `…/S0` |
| `constellation-whale/c01x100/strict-checklist/S1` | replication | 1 | 1 | |
| `constellation-whale/c05x20/strict-checklist/S1` | replication | 5 | 5 | |

**13 cells x 3 repeats = 39 units, 159 calls** (141 carrying audio, 18 text-only),
3 300 audio seconds. Three repeats because inter-run agreement *is* the score for
the dimensions that have no truth, and two runs give one pairwise number.

---

## 3. Tracks and the excerpt

Both are tracks the operator already adjudicated for the LT1 lyric-timing
benchmark, so second-accurate lyric truth already exists. Their MP3s are
read-only; the excerpts land in the gitignored `build/mimo-bench/audio/`.

| track | role | excerpt | aligned lines in window | usable lyric truth |
| --- | --- | --- | --- | --- |
| `shut-up-cat` | primary (all blocks) | `[30.0, 130.0)` s of 160.2 s | 25 | **22** |
| `constellation-whale` | replication | `[0.0, 100.0)` s of 114.8 s | 15 | **15** |

The windows were chosen by sweeping every 100 s window and taking the one with
the most aligned lyric lines. The gap between "in the lane" and "usable" is lines
the operator adjudicated as unlocatable (`true_start_seconds: null`, or listed
for spot-check and left `unadjudicated`); those are **dropped**, not scored,
because scoring a model against a placement the operator refused to certify
imports the exact error the adjudication exists to exclude.

Every chunking is cut from **one canonical re-encoded 100 s excerpt** (mono,
192 kbps), not from the source separately, so the granularity axis really does
vary only the cut points. Stream-copy splitting can only cut on an MP3 frame
boundary, so consecutive chunks overlap slightly; the manifest records it:

| chunking | probed total | frame-boundary overlap |
| --- | --- | --- |
| `c01x100` | 100.000 s | +0.000 s |
| `c05x20` | 100.024 s | +0.024 s |
| `c10x10` | 100.154 s | +0.154 s |
| `c20x05` | 100.415 s | +0.415 s |

21 ms per cut. That bounds how precisely a chunked condition could place a lyric
even in principle, and it is far below the 2 s scoring tolerance.

---

## 4. Scoring rubric

Three kinds of rule, and the distinction is the design. Lexicons and thresholds
are hashed into every score document (`lexicon_sha256`), so a number can always
be traced to the vocabulary that produced it.

### 4.1 Checked against a measurement or an adjudication

| dimension | truth source | metric | notes |
| --- | --- | --- | --- |
| **tempo** | `measured.json` `pulse_estimate` **plus** the ranked candidate set from re-running the repository's own estimator over the excerpt | `exact` / `octave` / `wrong` / `absent`, plus the matched reference and the ratio | Octave equivalence (x¼ … x4) is mandatory: the repository's own estimate for both excerpts is a sub-multiple of the felt tempo. **Read the accept rate against the chance baseline the harness prints — 32 % for `shut-up-cat`, 27 % for `constellation-whale`** — not against zero. |
| **form** | `measured.json` `summary.sections` boundaries | precision / recall / F1 at ±3 s | Reported as *agreement*, never correctness: the segmentation is itself an estimate. |
| **lyric positioning** | `lyrics.aligned.json`, overridden by `ground_truth_adjudication.json` | match rate, median \|Δ\| s, share within ±2 s and ±5 s, **fabrication rate**, line coverage | A quoted phrase is matched to a truth line by token-LCS *containment* ≥ 0.7 (containment, because short quotations are what the prompt asks for). < 3 tokens → `too_short`, not fabricated. No time given → `untimed`, kept out of the error statistics. |
| **time frame** | the chunk offsets | `absolute` vs `chunk-local`, and which fit better | Every prompt declares the absolute offset. Whether the model obeys is a compliance result. |

### 4.2 Checked against an operator-authored list

`tools/mimo_bench/ground_truth/tracks.json`. Nothing in this repository measures
these. **Every entry currently says `unadjudicated` and the scorers abstain**;
`run.py plan` prints "6 dimension(s) will abstain" until they are filled in.

| dimension | metric |
| --- | --- |
| **key** | `exact` / `parallel` (right tonic, wrong mode) / `relative` (relative major-minor) / `wrong` / `absent`, with enharmonic normalization |
| **meter** | `correct` / `wrong` / `absent`, after normalizing `common time` → `4/4` |
| **instruments** | precision, recall, F1 over a canonical lexicon, plus **`allowed_extra`** (neutral: neither helps recall nor hurts precision) and **`absent`** (hallucination canaries, reported on their own line) |

### 4.3 Not checkable — measured as consistency and concreteness

Feel and texture have no truth. Two things about them *can* be measured.

| metric | rule |
| --- | --- |
| **inter-run agreement** | mean pairwise Jaccard over the normalized descriptor vocabulary across the 3 repeats, plus the same for the instrument set, plus stdev/range of `energy`/`tension`/`valence` |
| **concreteness** | A sentence is **concrete** if it contains any of: (1) a number with a musical unit (bpm/Hz/kHz/dB/bars/beats), (2) a timestamp (`m:ss` or `NN s`), (3) an instrument-lexicon term, (4) a music-term-lexicon term, (5) a quoted phrase of ≥ 2 words. It is **generic** if it contains a generic-adjective term and none of the five. Anything else is **neutral** and is excluded from the ratio, so padding cannot improve the score by dilution. Reported as `concrete_per_100_words` and `concrete_share`. |

### 4.4 Machine usability

| metric | rule |
| --- | --- |
| **conformance** | per call: parses as JSON, validates against the schema |
| **determinism** (arm S3) | per schema field, the share of the 5 runs whose normalized value differs from the modal one; plus `identical_output_rate`. List order and numeric formatting are normalized away, so only real disagreement counts. |
| **cost** | recorded `usage` per call; `calibrate_from_usage` recovers the real audio token rate and collapses the projection bracket |

---

## 5. Cost

Audio bills as input tokens by duration, at a rate OpenRouter does not publish
per model, so the projection is a **bracket** (25–50 tokens/s) rather than a
single number nobody can check. After the first live run the recorded `usage`
gives the real rate.

Prices: MiMo $0.40/M in, $2.00/M out. `gpt-4o-mini` $0.15/M in, $0.60/M out.

```
projected cost, 3 repeats, 159 calls, 3300 audio seconds
  cell                                             calls  audio_s      USD low     USD high
  shut-up-cat/c20x05/strict-checklist/S1              60      300       0.1166       0.1196
  shut-up-cat/c10x10/strict-checklist/S1              30      300       0.0598       0.0628
  shut-up-cat/c05x20/strict-checklist/S1              15      300       0.0314       0.0344
  shut-up-cat/c01x100/strict-checklist/S1              3      300       0.0087       0.0117
  shut-up-cat/c01x100/strict-open/S0                   3      300       0.0098       0.0128
  shut-up-cat/c01x100/casual-open/S0                   3      300       0.0098       0.0128
  shut-up-cat/c01x100/strict-checklist/S0              3      300       0.0100       0.0130
  shut-up-cat/c01x100/casual-checklist/S0              3      300       0.0100       0.0130
  shut-up-cat/c01x100/strict-checklist/S2a             3      300       0.0100       0.0130
  shut-up-cat/c01x100/strict-checklist/S2b             3        0       0.0070       0.0070
  shut-up-cat/c01x100/strict-checklist/S3             15        0       0.0109       0.0109
  constellation-whale/c01x100/strict-checklist/S1      3      300       0.0087       0.0117
  constellation-whale/c05x20/strict-checklist/S1      15      300       0.0314       0.0344
  TOTAL                                              159     3300       0.3241       0.3571
```

The whole chunking block is 36 % of the cost for one axis, and that asymmetry is
itself the H2 result: **the same 100 s of audio costs 13x more at 20x5 s than at
1x100 s**, entirely in per-call prompt and completion overhead.

---

## 6. Decision gates

What result changes the application, and how. Each names the file it would change.

| gate | threshold | action |
| --- | --- | --- |
| **G1 — chunking** | `c01x100` within 0.05 instrument F1 **and** within 1 s median lyric error of the best finer chunking | Keep `tools/mimo_openrouter.py` sending the whole track in one call. |
| **G2 — chunking** | a finer chunking improves median lyric error by **> 2 s** and holds instrument F1 | Chunk the TC-SEMANTIC request at that granularity; add the chunk-offset header to `SYSTEM_PROMPT`. |
| **G3 — register** | casual/open beats strict/checklist on `concrete_per_100_words` by **> 30 %** with instrument recall no worse | Rewrite `SYSTEM_PROMPT` toward the evocative register and move the schema to a second turn. |
| **G4 — shaping** | `S1` concreteness < **70 %** of `S0`'s at equal instrument recall | Move the app to two-turn `S2b`: describe freely, then reshape with the audio elided. |
| **G5 — determinism** | `S3` field disagreement > **5 %** or identical-output rate < **0.8** | Reject the separate-reformatter strategy outright. Determinism is not negotiable for a lane that feeds an export. |
| **G6 — determinism** | `S3` passes G5 **and** costs < 50 % of `S2b` | Adopt the cheap reformatter, and pin its model id in the execution snapshot. |
| **G7 — tempo** | accept rate **≤ chance + 10 points** (i.e. ≤ ~42 %) | Never surface a MiMo tempo claim in the UI. `TC-MEASURED` stays the only tempo authority. |
| **G8 — lyrics** | fabrication rate > **20 %** | Keep the app's current "do not claim authoritative lyrics" instruction and do not add a lyric-moment field to `SemanticScore`. |
| **G9 — consistency** | descriptor Jaccard < **0.3** across three repeats | Raise the app's cache-reuse bar: a single MiMo run is not a stable artifact, and a re-run would visibly change the render. |
| **G10 — replication** | the two tracks rank the chunkings differently | Report the chunking result as track-dependent and do not change the app on it. |

---

## 7. Reproducibility and resumability

Every stored call carries, per §6 of `docs/ASSIST_PROVIDER_CONTRACTS.md`:
requested model **and** the model the response reported, the provider served,
prompt id + sha256, schema version + sha256, chunking id, chunk index and
absolute span, audio sha256 and byte count, temperature, the redacted request
body, the whole raw response, and `usage`. Scores additionally record the scorer
version, the lexicon hash and the ground-truth version.

Editing a prompt string changes its sha256 and therefore the identity of every
result it produced. That is intended: an edited prompt invalidates its results.

The resume unit is one `(cell, repeat)`. Inside it, each call is written the
moment it returns, and a rerun skips the calls already stored; the `done.json`
marker is written last. A machine that suspends mid-cell loses at most the one
request in flight.

---

## 8. Running it

```sh
# offline, no socket: the whole design, every request body, the cost
tools/mimo_bench/run.py plan

# cut the excerpts (ffmpeg only; reads ~/Music read-only, writes build/)
tools/mimo_bench/run.py prepare

# the live run — both gates are required
MIMO_BENCH_LIVE=yes-send-audio-to-openrouter tools/mimo_bench/run.py all --live

tools/mimo_bench/run.py list        # progress
tools/mimo_bench/run.py score       # offline, re-runnable, writes scores/report.json
```

**Before the live run:** fill in `tools/mimo_bench/ground_truth/tracks.json`
(key, meter, instruments for each 100 s excerpt) or key, meter and instruments
will abstain and three of the operator's five questions go unanswered. Confirm
`openai/gpt-4o-mini` is the intended reformatter and its price is current.

---

## 9. Out of scope

- **Other models.** This benchmarks MiMo v2.5, not a model bake-off. The
  reformatter is a fixed component of arm S3, not a competitor.
- **Lyric *transcription* accuracy.** LT1 owns wording; this measures only where
  a quoted phrase is claimed to occur.
- **Tracks without adjudicated lyric truth.** Adding a third track multiplies
  cost across every axis and buys one more sample of the noisiest dimension.
- **Provider variance.** Recorded (`provider_served`) but not an axis. Pinning a
  provider with `--only` would need its own design.
- **Changing the application.** This run produces evidence. Acting on it is a
  separate task, gated by §6, and belongs in `FEATURE_PARITY_PLAN.md`.
- **Reviving the outdated `audio-listening` skill.** Its findings are mined here
  (the two response styles are Block 2); the bundle itself stays retired.
