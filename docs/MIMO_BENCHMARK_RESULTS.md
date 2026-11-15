# MiMo v2.5 description benchmark — results of the first live run

**Since 2026-11-15 the application's listening lane runs on Gemini 3.8
Flash** (operator's choice; `ASSIST_PIPELINE.md`, "Gemini 3.8 Flash through
OpenRouter"); MiMo stays selectable as experimental, and this file is the
evidence its suitability row cites. What it found about prompting, the cap and
cost carried over to the lane as a whole. The harness itself still targets
MiMo; pointing it at another model is more than one line (the model constant
in `matrix.py`, a price row in `cost.py` or its spending stop projects 0 USD,
and its fixed temperature 0.2, which Google advises against for Gemini 3 and
which, with the harness's `require_parameters`, narrows routing to the
endpoints that list `temperature`, Google AI Studio's).

Read this before changing how the application's semantic lane chunks, prompts or
shapes its request. The design, hypotheses and gates are in
[`MIMO_BENCHMARK_PLAN.md`](MIMO_BENCHMARK_PLAN.md); this file is what the run
said. Update it in the same commit as a re-score that changes a number here
(after the listening session fills in key, metre and instruments, the three
"abstains" rows below get numbers).

**Run:** 2026-11-14, `xiaomi/mimo-v2.5` at temperature 0.2, served by Venice
(the only OpenRouter provider for it that day). Scorer
`musializer.mimo-bench-scorers/v2`. Raw calls, scores and the report live in
the main checkout's gitignored `build/mimo-bench/` (`results/`,
`scores/report.json`); `tools/mimo_bench/run.py score` reprints the table.

## What was run, and why not all of it

The plan's 3 repeats of 13 cells (159 calls) were projected at 0.32–0.36 USD.
The first call cost 0.0106 USD, three times its projection: MiMo **reasons
before it answers**, 600–9 200 reasoning tokens per call, billed as output.
Audio itself is cheap: the recorded usage calibrates to **7.4 audio tokens per
second** (the plan bracketed 25–50). The full matrix would have cost well over
1 USD, past the 0.60 USD ceiling, so the run was cut to fit:

- **repeat 1 of all 13 cells** (53 calls), then
- **repeats 2–3 of the one-call cells** while the ceiling allowed: strict-checklist
  S0 ×3, casual-open S0 ×3, casual-checklist S0 ×2. strict-open S0 repeats 2
  and 3 came back billed with **no text** (null content) and failed. The
  spending stop then refused the next call (its reserve is the worst call seen,
  0.131 USD).

So most cells have **one sample**. Every "supported" below means "at n = 1,
and the two tracks agree where both were run", not more.

## Cost

| | USD |
| --- | --- |
| recorded `usage.cost` of the 58 stored calls | **0.4942** |
| two billed responses with null content (not stored by the harness at the time) | unknown, **≤ 0.133** |
| real cost of this job | **0.494 – 0.627** |

The upper bound: the key's `usage_daily` read 0.987 USD at the end; the
parallel video-generation stream's ledger records 0.36 USD on the same day;
0.987 − 0.36 − 0.494 = 0.133 is the most the two lost calls can have cost, if
nothing else used the key that day. **The 0.60 USD ceiling may have been
passed by up to 0.03 USD.** The harness now keeps such responses
(`failed_*.json`) and counts them, so that cannot recur.

Per call, where the money went:

| cell (repeat 1) | calls | USD | mean reasoning tokens |
| --- | --- | --- | --- |
| c20x05 S1 | 20 | 0.053 | 599 |
| c10x10 S1 | 10 | 0.035 | 850 |
| c05x20 S1 | 5 | **0.147** | 918 — one call ran away, see below |
| c01x100 S1 | 1 | 0.011 | 4 015 |
| constellation c01x100 S1 | 1 | 0.022 | 9 231 |
| constellation c05x20 S1 | 5 | 0.040 | 2 932 |

**Two failure modes cost more than everything planned.** One structured call
(shut-up-cat c05x20, chunk 2) emitted **65 536 tokens of whitespace** and
stopped on `length`: 0.131 USD, 27 % of all recorded spend. One casual-checklist
free-text answer ran to ~43 000 tokens (0.087 USD). Two calls returned null
content. The application's request (`tools/mimo_openrouter.py`) set no
`max_tokens` either, so the app was exposed to the same thing (capped since
2026-11-14; see item 1 below).

## The table

`lyr_med` median |Δ| seconds of quoted lyric moments; `lyr<2s` share within
±2 s; `form` boundary agreement F1 ±3 s; `conc/100` concrete statements per
100 words; `schema` valid structured calls; `agree` descriptor Jaccard across
repeats. `temp` tempo accept rate (chance: 32 % shut-up-cat, 27 % constellation).

```
cell                                            reps  temp  lyr_med  lyr<2s  form  conc/100  schema  agree
shut-up-cat/c20x05/strict-checklist/S1          1     0.15  0.64     0.78    0.71  5.85      1.00    -
shut-up-cat/c10x10/strict-checklist/S1          1     0.20  1.04     0.83    0.68  5.31      1.00    -
shut-up-cat/c05x20/strict-checklist/S1          1     0.20  0.95     0.70    0.52  5.56      0.80    -
shut-up-cat/c01x100/strict-checklist/S1         1     0.00  2.16     0.50    -     5.80      1.00    -
shut-up-cat/c01x100/strict-open/S0              1     -     2.54     0.00    0.18  3.64      -       -
shut-up-cat/c01x100/casual-open/S0              3     -     10.14    0.25    0.24  4.34      -       0.20
shut-up-cat/c01x100/strict-checklist/S0         3     0.00  3.48     0.35    0.29  8.08      -       0.34
shut-up-cat/c01x100/casual-checklist/S0         2     0.00  6.19     0.21    0.30  6.10      -       0.33
shut-up-cat/c01x100/strict-checklist/S2a        1     0.00  1.73     0.58    0.46  8.53      1.00    -
shut-up-cat/c01x100/strict-checklist/S2b        1     0.00  1.84     0.55    0.46  6.56      1.00    -
shut-up-cat/c01x100/strict-checklist/S3         1     0.00  1.73     0.58    0.44  7.74      1.00    -
constellation-whale/c01x100/strict-checklist/S1 1     0.00  4.44     0.14    0.21  6.32      1.00    -
constellation-whale/c05x20/strict-checklist/S1  1     0.00  1.64     0.64    0.47  5.73      1.00    -
```

Lyric fabrication rate (quoted phrases matching no truth line): c20x05 33 %,
c10x10 28 %, c05x20 4 %, c01x100 S1 0 %, constellation c01x100 22 %, c05x20
21 %; pooled over every cell 46 of 314 = 15 %. "Fabricated" overcounts: it
includes mishearings of real lines ("Shadow cat", "guilty of the clock"),
descriptions of chopped vocals ("through, through, through" — the track does
stutter its vocal), and, in free-text cells, non-lyric fragments the quote
parser picked up. Outright inventions ("Drop it low", "break it down") are a
minority.

The first scoring pass gave lyric medians of 26–32 s. That was a scorer
defect, not the model: a repeated chorus line was always matched to its
**last** occurrence. Scorer v2 matches the nearest equally good occurrence and
lets a quote straddle two aligned lines; every number here is v2.

## Hypotheses

| # | verdict at this sample size | evidence |
| --- | --- | --- |
| H1 finer chunking buys lyric timing, costs context | **timing: supported**; context: not shown as form loss, shows as fabrication | lyric median 0.64 / 1.04 / 0.95 s at 5 / 10 / 20 s chunks against 2.16 s whole; constellation 1.64 (20 s) vs 4.44 (whole). Form F1 is *higher* when chunked, but chunk edges every 5 s land near any ±3 s boundary by construction, so that number cannot show a context loss. The cost appears instead as fabrication: 28–33 % at 5–10 s chunks, 0 % whole |
| H2 finer chunking not worth its price | **not falsified** (supported by default) | no finer chunking wins a gate threshold (below). Price ratio 20×5 s : 1×100 s is **5×**, not the planned 13×: short chunks reason less (600 vs 4 000 tokens) |
| H3 casual/open more concrete than strict/checklist | **refuted** | casual-open 4.34 vs strict-checklist 8.08 per 100 words. **Specificity, not register, drives it**: checklist ≈ 1.5–2× open in both registers; casual beat strict only among open prompts (4.34 vs 3.64, 3 reps vs 1). Casual-checklist also produced the 43 000-token answer |
| H4 schema in the same turn suppresses detail | **borderline** | S1 = 5.80, S0 = 8.08 (3 reps): S1 is 72 % of S0, the gate threshold is 70 %. Against S0's own repeat 1 (6.73) it is 86 %. Two-turn reshapes kept the detail: S2a 8.53, S2b 6.56, S3 7.74 |
| H5 a cheap reformatter is not deterministic enough | **supported** | S3 over 5 identical inputs: field disagreement 4.6 % (just under 5 %), identical-output rate **0.6** (< 0.8). Worst field: instruments (0.4) |
| H6 tempo octave-ambiguous but not random; key weaker than instruments | **tempo half refuted; key half pending truth, leaning supported** | tempo accept 0–20 % per cell, at or below chance (32 % / 27 %). Caveat: the references disagree among themselves (measured pulse 56.25 bpm, librosa beat 112.35); MiMo's dominant 150 bpm is 4/3 × 112.35, a triplet-feel reading. Key claims for shut-up-cat scatter: A♯/B♭ minor ×9, B minor, F♯ minor, F minor, A minor, E minor, A major; instrument sets repeat (Jaccard 0.38–0.69). The local chroma estimate is E♭ major, which none of those is. On constellation MiMo's key answer cited the lyric "neither major, neither minor" as its evidence — exactly the trap `ground_truth/tracks.json` warns about |
| H7 the chunking answer replicates | **supported** | both tracks rank 20 s chunks above the whole excerpt on lyric timing |

## Decision gates

| gate | fires? | so |
| --- | --- | --- |
| G1 keep one call | **no** | 1×100 s is 1.52 s behind the best chunking on lyric median (threshold 1 s); instrument F1 abstains |
| G2 chunk the request | **no** | best improvement 1.52 s (threshold > 2 s) |
| G3 evocative register | **no** | casual/open is lower, not 30 % higher |
| G4 move to two-turn S2b | **no, by 2 points** | S1/S0 = 72 % (threshold < 70 %); re-test with repeats |
| G5 reject the separate reformatter | **yes** | identical-output rate 0.6 < 0.8 |
| G6 adopt the cheap reformatter | **no** | fails G5 |
| G7 never surface MiMo tempo | **yes** | accept rate ≤ chance + 10 points in every cell |
| G8 keep "no authoritative lyrics" | **yes** | 22 % on constellation in the app's own shape (whole track, schema); 0 % on shut-up-cat |
| G9 raise the cache-reuse bar | **partly** | descriptor Jaccard 0.20 (casual-open), 0.33–0.34 (checklists); the app's own shape (S1) has one repeat, so its stability is unmeasured |
| G10 chunking is track-dependent | **no** | the two tracks agree |

## What this means for the application's MiMo lane

Changes the gates and the run justify, for the lead to queue (none made here;
items 1, 2 and 7 landed on 2026-11-14, see `ASSIST_PIPELINE.md`, "The MiMo
feelings request", and `DIVERGENCES.md`, "The MiMo lane's request"):

1. ~~**Cap the completion.**~~ *Landed: `max_tokens` 16 000, failed runs never
   cached.* Send `max_tokens` (reasoning included; ~12 000 would
   have cut both runaways, 65 536 and 43 578 tokens, and no other call: the
   largest of the rest used 10 942) and treat
   `finish_reason: length` or null content as a failed run, never as a result.
   One uncapped call cost more than the rest of a chunked cell.
2. ~~**Show cost from completion tokens, not audio.**~~ *Landed: the
   confirmation's estimate, and the artifact's estimate beside the billed cost.
   The lane's own two capped calls on shut-up-cat cost 0.0052 and 0.0042 USD
   (2 407 and 2 044 completion tokens, 294 and 452 reasoning): this prompt
   reasons far less than the benchmark's.* A whole-track MiMo call
   costs about 0.01–0.02 USD, nearly all of it reasoning output; any estimate
   built on audio seconds is wrong by an order of magnitude in the other
   direction.
3. **Keep one whole-track call for now** (neither G1 nor G2 fires), and keep the
   checklist-style prompt: the specific checklist, not the casual register,
   is what makes the answer concrete.
4. **No MiMo tempo in the interface** (G7); measured analysis stays the only
   tempo authority.
5. **Keep the "do not claim authoritative lyrics" instruction** and no lyric
   field in `SemanticScore` yet (G8). If lyric moments are wanted later, the
   5–20 s chunked runs place them within ~1 s, at more fabrication.
6. **No separate reformatter model** (G5).
7. ~~**Define the numeric scales in the schema.**~~ *Landed: prompt v2 states
   them; an off-scale number fails the run.* Unprompted, `energy` came back
   as 78, 8 and 5 from three chunks of the same excerpt.

## What is not yet answered

- **Key, metre, instruments** abstain on both tracks until the operator's
  listening session fills `ground_truth/tracks.json`. That decides the key half
  of H6, instrument F1/recall for G1–G4, and whether MiMo's unanimous 4/4 is
  right (the shut-up-cat beat grid hears strong triplet subdivision).
- **Consistency of the app's own shape** (S1) and of every cell with one
  repeat: G9 and the H4 margin need repeats 2–3. Resuming costs roughly
  0.4–0.6 USD more at today's reasoning lengths, plus margin for a runaway;
  with a `max_tokens` cap in the harness that bound tightens, but a cap would
  change the condition the app runs under, so it belongs in a decision, not a
  silent edit.
- **Whether reasoning helps listening.** MiMo accepts `reasoning` settings; a
  low-effort arm would show whether the money buys accuracy. Not in the design.
- `xiaomi/mimo-v2.6` (flash and pro, both audio-capable) appeared on OpenRouter
  by 2026-11-14. Out of scope here (plan §9).

## The listening session that supplies the missing truth

Built in the existing browser Listening Lab (`tools/listening-lab`), one
session per excerpt, three questions each, answered by listening and picking:
which held chord sounds like home (the three most likely keys rendered under
the excerpt, plus "none of these", "no stable home note", "can't tell"); which
counted click fits (4/4, 3/4, 6/8, and 12/8 on shut-up-cat where the grid
hears triplets; plus "free", "can't tell"); and a three-state instrument
checklist from the scorer's lexicon with a free-text line. Answers save on
every click, so it can be stopped and resumed. It needs the Lab change that
lets a protocol hold eight tracks (landed with the session, `41fe23b`).

Start it (from the main checkout; **this plays the music**):

```sh
cd ~/Projects/rusty-musializer/tools/listening-lab && LISTENING_LAB_PROTOCOLS="$PWD/../../build/mimo-bench/truth-session/browser" npm run dev -- --host 127.0.0.1 --port 4179
```

then open `http://127.0.0.1:4179`. Afterwards, convert and re-score:

```sh
cd ~/Projects/rusty-musializer && python3 tools/mimo_bench/truth_convert.py && tools/mimo_bench/run.py score
```

The converter refuses (and writes nothing) while any answer is half-filled or
unanswered; `--allow-missing` converts what is complete. Rebuilding the session
(`build/lyrics-research-v2/.venv/bin/python tools/mimo_bench/truth_session.py
--force`) is refused once answers exist.
