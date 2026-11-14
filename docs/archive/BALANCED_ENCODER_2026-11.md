# Can Balanced exports keep their gradients? (H2, 2026-11-14)

Archived on 2026-11-14 as a closed record: the measurement behind keeping the
x264 Balanced arguments as they are. Read it before proposing a change to the
encoder arguments for gradients or grain; re-run it with
`tools/encoder_matrix.py` rather than quoting it after FFmpeg or the renderer
changes.

## The question

EX3 (`docs/DIVERGENCES.md`) renders exports at 16 bits and dithers once to
8 bits. High and Master keep most of that dither in the MP4; Balanced
(`-preset fast -crf 20`) flattens it again. The operator said yes to pursuing
it (2026-11-14: "I say yes to ... balanced exports") if the cost is reasonable.
The lead's rule for adopting a candidate at Balanced: the dark-material gradient
measure moves at least half way from today's Balanced to today's High, the
stretched crops show the steps gone or clearly reduced, SSIM does not drop, and
the file grows no more than about 15 % on bright and busy material and about
40 % on dark gradient material. High and Master take a candidate only under
10 % size.

## Method

`tools/encoder_matrix.py capture` renders an export through
`tools/headless_run.sh` with an `ffmpeg` in front of the real one that also
writes every piped frame losslessly (FFV1). `encode` then encodes those exact
frames with the application's argument list plus each candidate, video only.
The cell with today's arguments came out **byte-identical** to the
application's own MP4's video stream for all five Balanced captures, so the
matrix measures what an export would produce.

Material, 1920x1080, 30 fps, 10 s windows (4 s for the fixture):

- `door-threshold`: "Door, No Room" (`native.treatment.json`), 5–15 s, the
  closed door in haze, film finish.
- `door-constellation`: the same, 27–37 s, Constellation graded night-door,
  film finish.
- `clawd`: Clawd over `door.mp3` at 60 s, bright flat colour.
- `spectrum`: Spectrum at the drop, 198 s, busy.
- `fixture`: the gate's synthetic case, Constellation on the sweep fixture at
  4–8 s, no finish: a clean vignette gradient.

Measures, against the yuv420p frames handed to the encoder (the same
conversion FFmpeg does, written losslessly):

- **grad**: the gate's blur-residual (7 px box minus 31 px box, RMS) on the
  decoded Y plane, in Y codes, averaged over five frames, each over the darkest
  smooth 256x160 tile with a real gradient in it (picked on the reference).
- **blotch**: RMS of the encoding error after a 7 px blur, same tiles. Needed
  because under the film finish *grad* is wrong: the source is grain, the
  encoder's flattening lowers *grad*, and the visible defect is the grain
  turning into flat blocks (see `docs/TRAPS.md`, "The gate's gradient measure
  reads grain as detail"). On the clean fixture the two agree.
- SSIM (All) and PSNR per plane with FFmpeg's own filters; size; wall time.

## Balanced (x264): size change, grad, blotch, SSIM change ×10⁴, against today

| cell | fixture | door-threshold | door-constellation | clawd | spectrum |
| --- | --- | --- | --- | --- | --- |
| today (`fast`, CRF 20) | 0.77 MB · 0.125 · 0.163 | 4.01 MB · 0.247 · 0.338 | 7.17 MB · 0.277 · 0.278 | 8.58 MB | 4.58 MB · 0.237 · 0.034 |
| `-aq-mode 3` | +19 % · 0.111 · 0.145 · +3 | +60 % · 0.313 · +6 | +37 % · 0.247 · +7 | +9 % · +3 | −1 % · −1 |
| `-aq-mode 3 -aq-strength 1.2` | +37 % · 0.092 · 0.100 · +6 | +183 % · 0.254 · +14 | +94 % · 0.210 · +13 | +22 % · +6 | −5 % · −3 |
| `-aq-strength 1.2` | +12 % · 0.100 · 0.121 · +2 | +96 % · 0.287 · +8 | +41 % · 0.234 · +6 | +10 % · +2 | −5 % · −2 |
| `-aq-strength 1.3` | +21 % · 0.090 · 0.096 · +4 | +133 % · 0.268 · +10 | +63 % · 0.212 · +8 | +16 % · +3 | −7 % · −4 |
| `-tune film` | −4 % · 0.103 · 0.119 · +1 | +18 % · 0.336 · −0 | +16 % · 0.267 · −1 | +5 % · +1 | +4 % · +1 |
| `-tune film -aq-strength 1.2` | +24 % · 0.088 · 0.084 · +4 | +134 % · 0.285 · +9 | +69 % · 0.225 · +5 | +16 % · +2 | −1 % · −1 |
| `-tune grain` | −8 % · 0.143 · 0.212 · +2 | +38 % · 0.303 · +6 | +50 % · 0.283 · +5 | +16 % · +3 | +55 % · +9 |
| no dct-decimate, deadzones 6 | +19 % · 0.124 · 0.170 · +1 | +22 % · 0.300 · +5 | +7 % · 0.257 · +3 | +2 % · +1 | +1 % · +1 |
| `mbtree=0` | +108 % · 0.092 · 0.104 | +207 % · 0.262 | +72 % · 0.239 | +24 % | — |
| `-preset slow` (CRF 20) | −16 % · 0.100 · 0.108 · −1 | +47 % · 0.310 · −28 | +7 % · 0.258 · −25 | −14 % · −7 | — |
| CRF 18 | +27 % · 0.091 · 0.103 · +5 | +167 % · 0.264 · +13 | +93 % · 0.213 · +14 | +25 % · +8 | +18 % · +5 |
| High's arguments on these frames | +55 % · 0.074 · 0.058 · +11 | +522 % · 0.181 · +2 | +282 % · 0.154 · +3 | +38 % · +9 | +27 % · +10 |

Door columns give size, blotch, SSIM change (grad is not meaningful there);
Clawd has no dark region and Spectrum's blotch stays 0.026–0.036 in every
cell, so those two give size and SSIM change. Deblock −2 or off and
`ipratio/pbratio 1.1` changed nothing measurable. Half way to High is grad
≤ 0.0995 / blotch ≤ 0.110 on the fixture, blotch ≤ 0.260 on door-threshold and
≤ 0.216 on door-constellation. Real High exports (2x supersampled frames)
measured the same as High's arguments on Balanced frames: fixture 0.077 /
0.057, door blotch 0.190 and 0.150.

Encode time, door-threshold, median of three on a loaded machine: today
3.6 s, `-aq-mode 3` 3.7, `-aq-strength 1.3` 3.9, `-tune film` 3.4, CRF 18
3.5, `-preset slow` 5.4, High's arguments 9.6. AQ and CRF changes cost no time.

**By eye** (crops contrast-stretched about 30x and enlarged 3x): on the fixture,
today and `-aq-mode 3` show the same hard staircase; `-aq-strength 1.2`/`1.3`,
CRF 18 and the AQ-1.2 film mix break every step edge into dither, clearly
reduced but not High's smooth field; `-tune film` partly; `-preset slow` breaks
the edges up into smeared streaks. On both Door clips every Balanced cell,
including those at two to three times the bytes, is a patchwork of flat blocks
where the source has grain; only High's arguments keep the grain.

## High and Master: `-aq-mode 3`

| | fixture | door-threshold | door-constellation | clawd | spectrum |
| --- | --- | --- | --- | --- | --- |
| High → High + aq3 | +23 %, grad 0.077 → 0.074 | +20 %, blotch 0.190 → 0.168 | +24 %, 0.150 → 0.138 | +10 % | −1 % |
| Master → Master + aq3 | +18 %, 0.064 → 0.062 | +15 %, 0.111 → 0.103 | +17 %, 0.091 → 0.085 | +12 % | −1 % |

Every dark clip grows over the 10 % bar for a small gain.

## NVENC (`--encoder nvenc`, Balanced p1 CQ 28)

| | fixture | door-threshold | door-constellation | clawd | spectrum |
| --- | --- | --- | --- | --- | --- |
| today | 0.43 MB, grad 0.150 | 0.70 MB, blotch 0.375 | 4.98 MB, 0.375 | 5.44 MB | 6.26 MB |
| `-spatial-aq 1` | +65 %, 0.143 | +657 %, 0.276 | +30 %, 0.312 | +33 % | −4 % |
| `-spatial-aq 1 -aq-strength 12` | +120 %, 0.155 | +884 %, 0.275 | +63 %, 0.284 | +50 % | −5 % |
| `+ -temporal-aq 1` | +68 %, 0.143 | +660 %, 0.277 | +30 %, 0.311 | +33 % | −3 % |
| High (p4 CQ 23) → + spatial AQ | +80 %, 0.141 → 0.141 | +87 %, 0.276 → 0.254 | +28 %, 0.305 → 0.265 | +29 % | −5 % |

Spatial AQ barely touches the clean gradient and buys the Door clips' grain
back only with bytes. NVENC is for exports one only looks at; nothing adopted.

## Decision: nothing changes

No candidate meets the rule. On the film-finished Door material none reaches
half way within 40 %: `-aq-mode 3` gets door-threshold to 0.313 against 0.260
at +60 %; the closest within 40 % is dropping dct-decimate with deadzones 6
(0.300, +22 %). On the clean
gradient the candidates that do reach half way (`-aq-strength 1.3`, CRF 18,
`-tune film -aq-strength 1.2`, aq3 at strength 1.2) grow Clawd 16–25 %
against 15 % and the Door clips 63–183 % against 40 %. `-aq-mode 3` alone,
the candidate the builder named, moves the fixture 0.125 → 0.111 of the 0.074
target (27 % of the way) for 19 % more bytes.

What would change it, for whoever comes back: on clean gradients
`-aq-strength 1.3` is the cheapest real improvement (+21 % on the clean
fixture, +16 % on Clawd, no time cost), and it is the one to revisit if the size bar
moves; but it does not save grain. Grain under the film finish costs High's
bytes at any setting, so a film-finished video one keeps is a High export.
