# The GPU: NVENC exports and the VirtualGL gate

Moved out of `AGENTS.md` on 2026-10-29. `AGENTS.md` keeps the rules; this file keeps the measurements and the defects the GPU path surfaced.

## Use the GPU for exports you are only going to look at (2026-08-08)

This machine has an RTX 3090 and FFmpeg has `h264_nvenc` and `hevc_nvenc`. Pass
`--encoder nvenc` for **every export an agent makes to check its own work**.
Leave the default (`x264`) alone for anything the operator will keep or post:
x264 `slow` still wins on quality per byte.

**The headless gate's own MP4 renders pick NVENC automatically** (2026-08-08)
when a one-frame test encode succeeds, the same detect-and-fall-back shape as
VirtualGL; `MZ_ENCODER=x264` forces the CPU path and the `encoder:` line names
which one a run took. This is safe against the gate's own assertions because
both were measured rather than assumed: two NVENC encodes of the same frames
are **byte-identical** (same md5), and the still cross-check's matched pair
reads 45.64 dB under NVENC against 45.55 under x264, both ~5.6 dB above the
40 dB floor with the 21.5 dB negative control unchanged. `export_probe` keeps
its deliberate X264 pin — it digests the frames *fed to* the encoder, so the
encoder cannot move its numbers and swapping it would prove nothing. A check
that compares two exports by md5 still has to hold the encoder fixed; the
gate's determinism checks satisfy that by comparing two runs of the *same*
`$MZ_ENC` invocation, never an NVENC file against an x264 one.

```sh
cargo run -- --mute track.mp3 --scene phosphor --encoder nvenc --render /tmp/check.mp4
```

`--encoder` takes `x264` (default), `nvenc`, or `nvenc-hevc`. An unknown name is
**refused**, not ignored, because an export that quietly used a different encoder
is not something anyone notices until they compare file sizes. The `video
encoder:` report line names what a run would use.

**Measured on this machine, so nobody re-derives it.** 600 frames of 1080p:

| step | x264 `slow -crf 16` | `h264_nvenc p4 -cq 23` |
| --- | --- | --- |
| encode only | 5.66 s | **2.29 s** |
| file size | 26.8 MB | 25.9 MB |
| a real 20 s export, end to end | 59.0 s | 56.6 s |

Read the last row before reaching for this expecting a transformation. Those
three rows were measured **on the CPU rendering path**, before VirtualGL: the
encoder was about 4 % of an export's wall clock and the other 54 seconds was
rendering through Mesa's `llvmpipe`, because Xvfb has no GPU of its own. With the
rendering on the GPU as well (next section) that ratio inverts and the encoder
choice is a real share of the clock — which is why this rule exists. The end-to-end
row has not been re-measured under VirtualGL; do not quote it as a current number.

Two things not to get wrong:

- **NVENC's `-cq` is not x264's `-crf`** and the numbers do not transfer. The
  mapping lives in `ExportQuality::nvenc_cq`, offset rather than copied; copying
  them is what makes a GPU export look obviously worse and get blamed on the
  silicon.
- **`-b:v 0` is load-bearing.** Without it NVENC caps at its 2 Mbit default and a
  1080p frame of high-frequency ASCII turns to mush.
- **The 3090 cannot encode AV1.** `av1_nvenc` is listed by `ffmpeg -encoders`
  because the *driver* supports it; the encode silicon is Ada and later. There is
  deliberately no `--encoder av1`.

**The encoder is session state, not project state.** It is on `RenderRequest`,
not on `RenderExportConfig`, so a `.musi` carried to a machine with no NVIDIA
card cannot insist on `h264_nvenc`. It does not change what is rendered — every
encoder is fed byte-identical frames, and the determinism contract is about
frames — but it does change the *file*, so a check that compares two exports by
md5 must hold the encoder fixed.

## The headless gate runs on the GPU

**This is the normal state on this machine.** VirtualGL 3.1.4 is installed and
`tools/headless_check.sh` resolves `vglrun` at detection time and uses it, printing
which path it took on its `gpu:` line. Read a gate run assuming the GPU path; a
`gpu:` line saying otherwise is the thing that needs explaining, not the default.

**Measured 2026-08-27: 6 min 55 s, against ~45 min on `llvmpipe`.** The gate
launches the application dozens of times, several in loops, and on the CPU path
every frame is rasterized by Mesa — 23 s for a 240-frame Spectrum capture, 50 s
for Phosphor Dream. That was the whole of the gate's wall clock. It was never the
Rust (`[profile.dev] opt-level = 1` moved it by a quarter and no further) and it
is not the differential harnesses, which have not run inside the gate since
2026-08-16.

`MZ_GL_LAUNCH` overrides the detection either way (set it empty to force the CPU
path, which is how the two are compared and how a new capture check gets its
second renderer); `MZ_VGL_DEVICE` picks the VirtualGL device, default `egl0` — the
EGL back end, which is the one that needs no X server on the GPU side. A machine
without `vglrun` still passes, just slowly:

```sh
curl -LO https://github.com/VirtualGL/virtualgl/releases/download/3.1.4/virtualgl_3.1.4_amd64.deb
sudo dpkg -i virtualgl_3.1.4_amd64.deb
```

**"Nothing else changes" was written before anyone had run the GPU path**, and
the first complete run took four attempts. Three defects, all in the gate's own
instruments, none in the application, and each one is a template for the next
capture check somebody writes:

- **The launcher must be an absolute path.** Three probe families launch with
  `PATH` stripped to the no-dialog guard directory (the kdialog/zenity
  isolation), and a bare `vglrun` under that `PATH` is exit 127 — which
  photographed as three unrelated sections failing. `MZ_GL` now resolves
  through `command -v` at detection time; the guard's job is hiding dialog
  binaries from the app, not hiding the launcher from the shell.
- **Blends round renderer-dependently; opaque fills do not.** The chrome's
  translucent rule strokes (`UI_RULE` at 40–50 % over `UI_SURFACE`) land
  anywhere from distance 1 to 3 of `UI_LANE_TROUGH` depending on who rounds
  the blend, so the lane-seam detector's ±4 colour window filled the frame
  with phantom seams on NVIDIA (7 at 1280x720, **31** at 150 %) while every
  real seam is an opaque fill both rasterizers write **bit-exactly** —
  verified at 905/1995/1249 exact pixels per seam row across GPU, GPU@150 %
  and llvmpipe. The detector now matches the trough exactly, tolerance zero:
  exactness is the one classifier blend rounding cannot reach.
- **A 1px stroke at a fractional x splits its coverage.** llvmpipe snaps it
  into one full-dark column; NVIDIA splits it across two partial ones — the
  AI dialog's divider became 228/228 where the CPU wrote one 210 (the rail
  scan gained 1px of slack against its 14-vs-0 defect), and a lane border
  became (244,244,245)+(219,219,222), which no colour window around
  `UI_RULE`±6 can catch without swallowing the trough at distance 20. Border
  columns are now found by a **darkness ceiling** (every band row ≤ 238 on
  the brightest channel): every split leaves its majority column far below
  it, and the split is a function of x alone, so all lanes still see the
  identical column and the cross-lane equality keeps its teeth.

The general rule the three add up to: **a capture check calibrated on
bit-exact llvmpipe output is calibrated to an accident.** Fills may be
compared exactly; anything a hairline or a blend touches needs a classifier
that is invariant under ±1 rounding and coverage splitting, and the two
renderers must both be in the negative-control loop when such a check is
written (`MZ_GL_LAUNCH=""` forces the CPU path for exactly this).


## EX4-c: a lifetime peak is not the injected stall (2026-10-30)

The CPU failure **predates WX-5d**. Five accepted quiet trials per commit used
that commit's native gate prefix and native light preferences, the same synthetic
WAV (SHA-256 `e178c3dcb8f19c435fedc73615aa82b523a4789df0ebc2544d92338a071c0b93`),
`MZ_GL_LAUNCH=""`, and display `:83`:

| commit | worst frame, ms (five trials) | lifetime dropped | lifetime peak | output underruns | old assertion |
| --- | --- | --- | --- | --- | --- |
| pre-wave `3ec989e` | 123.8, 124.0, 124.0, 124.2, 124.3 | 2, 2, 2, 1, 1 | 100% in all five | 0 in all five | failed 5/5 |
| WX-5d `be507e1` | 124.1, 124.1, 124.1, 124.1, 124.1 | 1, 2, 2, 1, 2 | 100% in all five | 0 in all five | failed 5/5 |

The dropped column is the raw diagnostic counter, not a count of every lost
PCM frame: the tap callback stops after its first rejected push and discards
the remainder of that callback. Zero still means no rejected push; nonzero
magnitudes undercount the lost tail. No counter semantics changed here.
Every trial counted exactly one late frame, frame 61 of 120, against 25 ms.
Accepted load-1 ranges were 1.91–4.37 and 2.46–3.22 respectively on 24 cores.
A process/load sampler checked throughout: no neighbouring Xvfb, Rust compiler
or model-lock holder overlapped accepted trials. Two pre-wave attempts with
`:82` captures were discarded. Earlier exploratory comparisons are also
**discarded**: passing the newer preference shape to the old binary rejected
`column_open`, silently selected dark and posted a warning. The corrected
comparison asserts `appearance: light-blue` and no preference warning before
accepting a sample. The throwaway pre-wave worktree was removed afterwards.

A temporary, reverted trace located the occupancy: **8192 frames at frame 0**,
5056 at frame 1, 1440 at frame 2; **6720 at the injected stall, frame 60**.
Immediately before that sleep the ring held 960; 120 ms at 48 kHz added 5760,
so the stall peak was 6720/8192 = 82%, matching EX4-c's historical measurement.
Draining 4096 left 2624; the next drain cleared it. The ordinary CPU run also
reached 8160 before its first drain while steady frames remained 16.7 ms.
The evidence identifies the pre-drain startup epoch, not a particular GL call.
It does **not** show WX-5d consuming the 120 ms band's headroom.

The existing frame budget excludes initialization; the old ring peak and loss
counter included it. This was an assertion-scope accident exposed by llvmpipe's
cold startup, not an app playback regression. No UI, playback or buffer sizing
was changed. A private `audio stall:` report records raw boundaries and peak
occupancy from the existing intervention through recovery (frames 60..119).
The lifetime figures remain intact. Both epochs share one occupancy snapshot,
so a callback between two reads cannot make the window exceed its lifetime.

The checker pins requested 120 ms, frame 61 as the single late frame of 120,
25.0 ms budget, 60 observations, frames 60..119 and capacity 8192. It requires
`4096 < peak < 8192`: the lower boundary is one analyzer drain, the upper is
ring overflow, **not percentages fitted to a driver**. Callback phase makes
occupancy stochastic. GPU requires lifetime `dropped=0` and `underruns=0`.
CPU requires **zero additional drops during the intervention/recovery and
zero lifetime output underruns**, prints its pre-control loss count and names
cold startup as the limitation. Thus CPU proves the local silent-band example;
it does not claim lossless cold startup. All paths retain the lifetime peak
and require it to contain the measured window.

### Assertion changes and controls

| assertion | before | after | evidence/reason |
| --- | --- | --- | --- |
| intervention/budget | 120 ms, 25 ms; any nonzero late count | same times; finite worst ≥120 ms, exactly 1/120 at frame 61 | both commits' ten quiet runs |
| occupancy | lifetime peak rejected only 0% and 10–19% | window strictly (4096,8192), capacity pinned 8192; lifetime contains it and is ≤100% | drain/ring capacities and reverted frame trace |
| drops | shell substring `0 dropped` in lifetime | parsed numeric counters; GPU lifetime zero; CPU window delta zero and final lifetime equals window endpoint | startup losses versus 6720-frame stall peak |
| underruns | lifetime zero | lifetime and both window boundaries zero | unchanged zero-starvation meaning |
| analyzer consumption | not checked here | parsed count >0 | final GPU 101760 / CPU 108512 consumed; callback phase makes the exact total stochastic |
| timing/pixel tolerances | existing values | unchanged | no numeric tolerance widened |

**35 negative controls failed (17 per renderer plus GPU pre-control loss)**, with each diagnostic input
perturbed in place, checked, restored byte-for-byte, and the tracked working
diff compared before/after. Real 1 ms and 750 ms interventions used the same
muted launcher on CPU and GPU. Only their request metadata was relabelled 120
in the checker input: the 1 ms run fails timing/occupancy; the 750 ms run fails
overflow, loss delta and starvation (45 CPU / 49 GPU underruns in the final-code repeat). Further controls
reject zero PCM consumption, nonfinite frame time, missing window/peak, missing or inconsistent lifetime peak, shifted or
doubled late frames, requested 121 ms, 59 observations, capacity 4096, ten added
drops, loss after the window, a reset counter one underrun and GPU pre-control loss with no further delta. Ten drops also
exposes the old shell substring bug (`10 dropped` contains `0 dropped`). Both
positive inputs still pass after restoration. Temporary trace instrumentation
was restored byte-for-byte before the production seam was written.

The isolated launcher originally gained a branch-local fixed-display option.
Integration consolidates it to `MUSIALIZER_CAPTURE_DISPLAY=:N` for all callers;
see `DIVERGENCES.md` and `TRAPS.md`. Malformed assignments and foreign or
unverifiable occupied displays are refused. An ancestor gate's Xvfb may be
reused, with every isolation guard still applied. VirtualGL needs `cut` and
`hostname` in the guarded PATH; no dialog program is exposed.

### WX-6 appearance pilot and runtime cap

The shipping light-only, column-closed gate passed fully on the final code:

| final-code full gate | wall clock | renderer reports | controlled stall |
| --- | --- | --- | --- |
| GPU | 553.38 s / 9.22 min | 379 NVIDIA RTX 3090, no llvmpipe | 122.3 ms, 6720/8192, dropped 0..0, underruns 0..0 |
| CPU (`MZ_GL_LAUNCH=""`) | 584.80 s / 9.75 min | 379 llvmpipe LLVM 21.1.8, no NVIDIA | 124.6 ms, 6720/8192, dropped 2..2, underruns 0..0 |

Both reached the final protocol/resume/digest checks and exited 0, with exactly
one late frame at 61 in their controlled stall. CPU lifetime peak was still
100%. The CPU sampler recorded load 1.59–6.59 on 24 cores and no other capture,
compiler or model-lock holder. No other CPU check needed a change or exemption.
An earlier pre-snapshot-fix CPU run also passed, in 615.82 s (10.26 min).
All exports held the existing NVENC encoder choice; CPU/GPU here names the
GL rasterizer, not the encoder.

An exploratory GPU baseline took 676.90 s (11.28 min); the paired-UI pilot took
928.42 s (15.47 min), +37.16%, exceeding the one-third cap of 902.53 s.
Other sessions compiled/captured during parts of these runs, so they are
**loaded exploratory timings**, not the quiet comparison. A subsequent full
verification's GPU gate passed in 549.49 s (9.16 min). The final-code quiet GPU baseline passed in **553.38 s (9.22 min)**; its
one-third cap is **737.83 s (12.30 min)**. A descendant-aware sampler recorded
no other capture, compiler or model-lock holder during that baseline. The
unchanged candidate completed in **849.83 s (14.16 min), +53.57%**. Its
receipt is quiet throughout (load 1.24–4.00; baseline 2.11–3.94 on 24 cores),
with no neighbouring capture, compiler or model-lock holder. The pilot
tracked 53 owned Xvfb PIDs through cleanup. This confirms the cap failure;
the candidate needs **at least 112.00 s removed** to reach 737.83 s, as well
as corrected assertions. It exited 1 after reaching the final protocol
section; the baseline exited 0. All temporarily substituted tools were
restored byte-for-byte.
A completed 851.39 s pilot is excluded from that verdict: its sampler flagged
four Xvfb cleanup observations without sufficient ownership receipts. The
repeat retains known owned PIDs through reparenting and ignores dead zombies.
Interrupted attempts and a refused stale owned display are excluded too; the
stale lock was removed only after its recorded owned PID was verified dead.

The unmerged pilot repeats legacy UI pixels in light-blue and black-amber,
uses column-open frames, and leaves scene exports single-pass. It ran through
the final protocol section but failed several new assertions. Implementation
stopped at the runtime cap, and all pixel changes were restored out of the
branch. The candidate remains in the ignored review artifact
`build/wx6-evidence/wx6-pixels-unmerged.patch`; **WX-6 conversion is not done**.
No proposed pixel thresholds from that candidate ship here. In particular:

- Fixed Render/key crops no longer describe the frame. A Render crop landed in
  the former bottom band; the key crop sampled the Replace label instead of
  password bullets. Follow reported control rectangles and test the active
  credential field before claiming a privacy check.
- The dark hover's opaque fill `(29,33,39)` rounds to BT.601 luma 32; the first
  pilot incorrectly expected 33. Blend and hairline checks need contrast or
  coverage classifiers, not swapped light RGB constants. The open-column run
  hairline reported two when the report said three in both looks. The flyout
  accent census failed when its chosen playhead was occluded. These require
  geometry/visibility work, not relaxed thresholds.
- Embedded lane-alignment probes omitted the explicit appearance and looked
  for light seams in dark frames. Every environment boundary needs the native
  preferences and an appearance assertion.
- The default frame caps the lyric lane to **49 px**. The pilot called its
  grip dead and attributed it to a 50 px minimum. Integration's press/drag
  reproduction corrects that diagnosis: the actual minimum is **22 px**, and
  an upward drag shrinks the default lane to **30 px**, persists and releases.
  A downward request is capped at 49 by fixed-band slack; an expanded 330 px
  band permits 66. Growing the band through this grip remains a layout choice.
  `tools/lyric_grip_check.py` pins both directions.

To fit the cap, replay only pixel-bearing captures; do not duplicate discovery
rungs, input scripts and state/persistence suites. Then resolve the classifier,
visibility, field-targeting and grip issues above, and run CPU/GPU negative
controls for every changed pixel assertion. Default-frame checks should use
column open; retain a closed full-width band only for a documented geometry
contract that actually needs it. None of those placement decisions is claimed
implemented in the shipping gate. The concrete placement decisions for the
next attempt are:

| check family | frame to photograph | why |
| --- | --- | --- |
| Assist hover/review, route editor, Render summary/progress | default column open | their actual panel lives in the column; use its reported rectangles |
| AI modal divider, focus ring, tabs and masked credential field | default column open underneath the modal | the modal needs no full-width timeline; pin the field actually focused |
| timed-lane alignment, run hairlines, poster/accent census, rail/flyouts, event markers/ticks | default column open | all are contracts of the operator's time axis; derive its current bounds |
| lyric cue colour, clipping, alignment and resize | default column open, plus one explicitly enlarged band for the max-height request | test the default grip separately; do not use an enlarged band to hide the dead default grip |
| column toggle / closed-band layout itself | one explicitly column-closed capture | the closed layout is the subject, so retain and comment this exception |
| scene still/video comparisons and data response | authored output/preview contract, single appearance-independent pass | repeating chrome cannot add evidence about exported scene PCM/determinism |

These are placement decisions for the open WX-6 queue item; the unmerged
candidate has not satisfied them.

### Poster budget: five quiet repetitions

Five complete quiet GPU poster gates passed. Each budget-on and budget-off
run had **0/150 frames past 25 ms**, worst 16.7 ms; replay frame-worst was
2.2 ms, step-worst 9.2 ms (the existing 8 ms budget permits one replay plus
0.5 ms). Whole-gate times were 37.86–37.94 s. Accepted load-1 was 1.97–6.25
on 24 cores, with no neighbouring capture/compiler/model-lock holder during
any accepted sample. Three additional attempts that overlapped captures were
discarded and repeated. The contention symptom did not recur; no poster code
or budget changed.

Raw reports, process/load receipts, binary hashes, controls and rejected pilot
are retained under `build/wx6-evidence/` for the lead's local review. They are
ignored artifacts, not committed fixture data.

### Final verification and disposition

`tools/verify.sh` completed: **17 passed, only the expected stale code-map
check failed** (1340.31 s overall). After making the two occupancy epochs share
one snapshot, `tools/verify.sh --quick` completed with seven passes and the
same sole expected code-map failure, followed by the full final-code GPU and
CPU gates above. `cargo fmt --check` passed. Clippy exited 0; existing warnings
in untouched code remain under Rust 1.96.1 (see the verification log), and no
UI cleanup was mixed into this change. `docs/CODE_MAP.md` was not regenerated.

The final-code 1 ms / 750 ms launcher controls were repeated on both renderers,
then all 35 diagnostic perturbations were rerun from a clean committed tree;
each original positive input and the clean tree were restored byte-for-byte.
The fixed-display refusal controls cover both renderer environments as well.
The only application change is the private report seam; the extra launcher
change reserves :83 for isolated hand probes and supplies VirtualGL's shell
utilities. No app UI or poster implementation changed. The pre-wave throwaway
worktree is gone. The final handoff includes a release build and no owned
background process; its command result is reported with the branch commits.
