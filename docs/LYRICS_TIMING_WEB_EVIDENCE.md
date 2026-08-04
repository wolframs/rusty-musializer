# Lyrics timing: web evidence for the trimmed benchmark (2026-08-04)

Companion to `LYRICS_TIMING_RESEARCH_PLAN.md`. The operator cut the plan's
seven-variant experiment matrix to two local lanes (whole-song
Qwen3-ForcedAligner; anchor→block MMS). This memo records the published and
community evidence that answers the cut questions, so nobody re-benches them
on local GPU time.

| # | Question | Answer | Confidence | Key source |
|---|---|---|---|---|
| 1 | Qwen3-ForcedAligner-0.6B on real songs | No community quality reports exist. Nightingale ships it as an experimental backend with automatic fallback to WhisperX; no music-vs-speech comparison, failure modes, or VRAM figures anywhere. The Qwen3-ASR tech report benchmarks the *ASR* model on singing (M4Singer 5.98% WER, full mixed songs 14.6% WER) but the aligner timestamp-accuracy eval is speech-only — the aligner is never validated on music. | High (absence is well-checked) | [Qwen3-ASR Technical Report](https://arxiv.org/html/2601.21337v1) 2026; [Nightingale README](https://github.com/rzru/nightingale/blob/master/README.md) 2026 |
| 2 | Two-grid overlapping Whisper vs VAD-only | No published/community evidence for a second shifted grid improving lyric recovery. The documented, accepted fix for repetition/hallucination loops is VAD segmentation + `condition_on_prev_text=False` (stable-ts, WhisperX). Overlap is a generic ASR heuristic, unbenchmarked for lyrics. | Medium | [jianfch/stable-ts](https://github.com/jianfch/stable-ts) 2026; [HN Nightingale thread](https://news.ycombinator.com/item?id=47422942) 2026-03 |
| 3 | Demucs before forced alignment: gain size | Measured, not huge. arXiv 2102.09202: Demucs vs Spleeter mean AE 0.31s vs 0.38s, median tied at 0.05s — no ablation against *no separation*. ISMIR 2025 LBD (Huang & Benetos, word-level MUSDB18 test set) finds accuracy varies substantially by separator and stresses robustness *without* assuming one. | Medium | [arXiv:2102.09202](https://arxiv.org/pdf/2102.09202) 2021; [ISMIR2025 LBD 412](https://ismir2025program.ismir.net/lbd_412.html) 2025-09 |
| 4 | Anchor→block beats whole-song CTC: independent confirmation | Partial. No direct replication of the anchor-spotting method found. HCLAS-X (MAE 0.16s vs prior-best 0.22s on Jamendo) corroborates the hierarchical "localize coarse, refine fine" family, but is cross-correlation-based with no public repo. | Low-Medium | [arXiv:2307.04377 HCLAS-X](https://arxiv.org/pdf/2307.04377) 2023 |
| 5 | Existing tool that already does known-text localization well | nomadkaraoke/python-lyrics-transcriber is archived (Dec 2025); its lyric-fetch + anchor-sequence + LLM-correction work moved into the maintained successor **nomadkaraoke/karaoke-gen** — worth studying for anchor-sequence design, though it is Whisper/AudioShake-based, not a global-lattice localizer. | Medium-High | [nomadkaraoke/karaoke-gen](https://github.com/nomadkaraoke) 2026 |

## Conclusions

1. Whole-song Qwen aligner lane: keep, but it is genuinely untested on music —
   the ASR model's singing benchmarks must not lend the aligner credibility.
2. Two-grid Whisper: stays cut. Adopt VAD + `condition_on_prev_text=False` in
   the production Whisper pass regardless of any chunking decision.
3. Demucs-mandatory: stays cut. Keep separation as an on-demand evidence lane;
   separator choice matters more than separation-vs-none is proven to.
4. Anchor→block MMS lane: keep, with lowered replication confidence — HCLAS-X
   supports the family, not the specific method.
5. Read nomadkaraoke/karaoke-gen's anchor-sequence design before building a
   global lattice from scratch; do not re-derive ISMIR 2025's separator
   comparison locally.
