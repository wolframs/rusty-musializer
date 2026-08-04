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

## Open before production default flips

1. **Boundary adjudication.** 13 lines across three tracks disagree with the
   baseline by >3 s (`build/lyrics-research-v2/spot-check-list.txt`; the other
   10 entries are recovered lines the baseline omitted entirely). One is the
   predicted repeated-occurrence case (`shut-up-cat` line 26, a repeated
   chorus line, 15.5 s apart). These need operator ears, not more compute.
2. The single order violation on the choir stress track needs a look during
   integration.
3. Production integration also adopts VAD + `condition_on_prev_text=False`
   for the Whisper evidence pass (accepted community fix for the repetition
   loop) and the plan's Invariant 1: unmatched authored lines surface as
   "location unknown" in review, never silently dropped.
