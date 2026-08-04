# Lyrics timing: trimmed benchmark results (2026-08-04)

Companion to `LYRICS_TIMING_RESEARCH_PLAN.md` (design) and
`LYRICS_TIMING_WEB_EVIDENCE.md` (why the cut lanes stayed cut). Harness:
`tools/lyrics_research/`; raw artifacts under gitignored
`build/lyrics-research-v2/results/`. All four tracks used their embedded
`lyrics-eng` metadata, intact.

## Scoreboard

| track | method | coverage | omitted | unresolved | canary outro | vs-baseline dstart med/max | runtime |
|---|---|---|---|---|---|---|---|
| constellation-whale | baseline | 13/15 (87%) | 2 | 0 | 0/2 | — | 1.4 s |
| constellation-whale | qwen whole-song | 14/15 | 0 | 1 | 0/2 | 1.99 / 18.36 s | 7.9 s |
| constellation-whale | **anchor→block MMS** | **15/15** | 0 | 0 | **2/2** | 0.01 / 0.31 s | 5.3 s |
| shut-up-cat | baseline | 27/33 (82%) | 6 | 0 | — | — | 12.7 s |
| shut-up-cat | qwen whole-song | 18/33 (55%) | 0 | 15 | — | 6.51 / 46.19 s | 7.9 s |
| shut-up-cat | **anchor→block MMS** | **33/33** | 0 | 0 | — | 0.17 / 15.49 s | 5.6 s |
| shipped-the-disposition | baseline | 50/51 (98%) | 1 | 0 | — | — | 16.8 s |
| shipped-the-disposition | qwen whole-song | 24/51 (47%) | 0 | 27 | — | 4.63 / 33.53 s | 9.3 s |
| shipped-the-disposition | **anchor→block MMS** | **51/51** | 0 | 0 | — | 0.01 / 5.63 s | 6.5 s |
| a-cell-within-a-cell (stress) | baseline | 42/42 | 0 | 0 | — | — | 18.4 s |
| a-cell-within-a-cell (stress) | qwen whole-song | 32/42 (76%) | 0 | 10 | — | 50.04 / 88.45 s | 8.4 s |
| a-cell-within-a-cell (stress) | anchor→block MMS | 42/42, 1 order violation | 0 | 0 | — | 0.01 / 11.98 s | 6.5 s |

`dstart` is agreement with the baseline lane over shared lines — not ground
truth. Total GPU time for the whole matrix: under two minutes.

## Decision-gate outcomes (plan's "Decision gates" section)

- **Qwen3-ForcedAligner: fails its gate.** Coverage collapses on real songs
  (47–76% on the three longer tracks), with catastrophic mid-song drift
  (median 50 s on the choir track). The web evidence predicted this risk: the
  aligner was only ever validated on speech. Lane retired; harness kept.
- **Anchor→block MMS: passes.** 100% authored-line coverage on all four
  tracks with the already-installed model, recovering the canary's two
  Whisper-looped outro lines at plausible positions (90.7–95.7 s), and both
  failure classes from the incident (coverage, search-space) are gone. This
  removes the Whisper-window dependency, which the plan ranked above changing
  the fine aligner.
- **Chunked Whisper: stays cut** (no evidence it beats VAD +
  `condition_on_prev_text=False`; see the web-evidence memo).
- **Demucs-mandatory: stays cut** (on-demand evidence lane only).

## Operator adjudication (2026-08-04)

The operator listen-checked all 23 spot-check lines (waveform-confirmed where
close). Machine-readable verdicts:
`build/lyrics-research-v2/ground_truth_adjudication.json`.

- Of 21 adjudicated lines, the anchor lane is correct on 14, the baseline on
  3, both wrong on 2; two were left unadjudicated. Every baseline omission the
  anchor lane recovered was confirmed correct, including both canary outro
  lines and one where the operator's own intuition was wrong and the model
  right (`shut-up-cat` line 11, confirmed at 44.8 s in the waveform editor).
- The anchor lane's genuine failures are exactly the predicted classes:
  the repeated chorus phrase (`shut-up-cat` line 26 — both lanes chose a
  wrong occurrence; the correct response is abstention, per Invariant 2's
  unresolved state) and short one-word exclamations with weak acoustic
  evidence (`rawr.`, `Vinculum.`).
- The choir track's 170–185 s region is genuinely ambiguous (held choir
  notes under the lead) and is excluded as calibration data by operator call.
- **The aligner's own score does not separate right from wrong** — median
  score 0.139 on confirmed-correct vs 0.142 on confirmed-wrong lines; 9/16
  correct lines were flagged `weak` while 2/5 wrong lines reported `aligned`.
  This confirms the plan's Invariant 4. What *did* catch all five wrong lines
  is cross-lane disagreement (>3 s between the coarse proposal and the
  anchor-block placement) — 23 flags across 141 lines, a reviewable burden.
  The review UI must therefore flag disagreement and unresolved lines, not
  raw model score.

## The default flipped: production results (2026-08-04, tranche LT1)

The `baseline` lane *is* `tools/external_analysis.py assist --mode lyrics`, so
rerunning it after LT1 measures the production path. All four tracks:

| track | coverage | omitted | unresolved | order | canary | vs pre-LT1 anchor lane |
|---|---|---|---|---|---|---|
| constellation-whale | **15/15** | 0 | 0 | 0 | **2/2 in window** | dstart med 0.00 / max 0.48 |
| shut-up-cat | **33/33** | 0 | 0 | 0 | — | 0.00 / 3.27 |
| shipped-the-disposition | **51/51** | 0 | 0 | 0 | — | 0.01 / 16.01 |
| a-cell-within-a-cell (stress) | **42/42** | 0 | 0 | **0** | — | 0.01 / 12.44 |

Every line the operator adjudicated *correct* is reproduced to ≤0.21 s, and the
canary's outro lines are real cues at 90.72 s and 93.94 s against the
adjudicated 90.7 / 93.9. Review flags total 17 across 141 lines (1 / 5 / 2 / 9).

Three of the four open items above closed, one changed shape:

1. **The choir track's order violation is gone**, not pinned. It came from the
   pre-`--max-context 0` Whisper evidence; better anchors removed it.
2. **`condition_on_prev_text=False` is adopted; VAD is not.** whisper-cli 1.8.6
   has no `--no-context`, but `--max-context 0` reaches the same code path and
   alone contains the loop — the canary's outro transcribes at 90.6 s and its
   duplicate segments fall 22→0. Silero VAD, measured on the same track, keeps
   **0.4 s of 114.84 s** at the default threshold and 3 segments at 0.10: it
   rejects singing over accompaniment. It is opt-in via
   `MUSIALIZER_WHISPER_VAD_MODEL` and degrades to no VAD when absent.
3. **The pinned abstention case no longer reproduces, and that is the honest
   result.** With the loop contained, the coarse lane covers 33/33 and puts
   `shut-up-cat` line 26 at 108.3 s against the block placement's 110.0 s — the
   two views now agree about the *occurrence*, so the criterion correctly does
   not fire and the line keeps a cue. Both previously adjudicated positions
   (106.8 s and 122.3 s) were wrong; 110.0 s falls in the gap the surrounding
   verse lines leave, but nobody has listened to it. The abstention rule is
   pinned against the recorded pre-LT1 numbers in
   `tests/test_lyric_anchor_block.py`, where it fires on line 26 and on none of
   the other five occurrences of the same phrase.

One regression is worth naming: `shipped-the-disposition` line 41 (`rawr.`) moved
from 150.6 s to 134.6 s against a truth of 156.2 s, and carries **no** flag. It
is the weak one-word-exclamation class the adjudication predicted, and it shows
the limit of cross-view flagging — when both views derive from the same Whisper
evidence and both are wrong, their agreement says nothing.
