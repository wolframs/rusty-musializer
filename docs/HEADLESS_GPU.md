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

