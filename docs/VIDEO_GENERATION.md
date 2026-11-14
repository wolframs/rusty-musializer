# VG: generated video as material (scoping, 2026-10-21)

The operator, 2026-10-21: video generation "is getting pretty good" and "would
be worth having as a tool too in the future, but first, I'd want to estimate
prices and available models". Then: consider only models that can do 1080p,
only the latest ones, offer users and agents **5 picks**, leave editing
models for later, and find out which models take images as input (a baseline
image, or first and last frames).

**The first half is built (2026-11-14):** any video file can be a section of
the video — see *Clips as material* below. Generating one is not. The rest of
this file is the shortlist of models and the reasoning behind it. The queue
row is VG in `FEATURE_PARITY_PLAN.md`.

## Where the numbers come from

`tools/openrouter_video_prices.py --hd` reads OpenRouter's public, keyless
`/api/v1/videos/models`. The ordinary `/models` list prices video models at
zero; the real prices are only on this endpoint. It prints dollars per
second at 720p and 1080p, clip lengths and image inputs, and spends nothing.
**Re-run it before trusting any price below.** These were read on
2026-10-21, and video prices move monthly.

## The five picks

The latest model of each family that outputs 1080p or better, excluding
editors, upscalers and avatar models. All five take a **first and a last
frame**. That matters most for Musializer, because it can render exact
frames (`--still`). A generated clip can start on one section's frame and end
on the next one's, as a bridge between two renders rather than a separate
picture.

| Pick | $/s at 1080p (no audio) | Clip | Images in | Why it is on the list |
| --- | --- | --- | --- | --- |
| `google/veo-3.1-lite` | 0.05 | 4, 6, 8 s | first, last | Drafts and volume. A 10-shot pass costs under $4. 16:9 and 9:16 only |
| `minimax/hailuo-3` | 0.13 (2K) | 5–15 s | first, last, reference images | The cheapest 2K, every aspect including 1:1 and 21:9 |
| `alibaba/wan-3.0` | 0.20 | 2–30 s | first, last, reference images | The longest single clip; fixed seed |
| `black-forest-labs/flux-3-video` | 0.29 (4K 0.41) | 5–20 s | first, last; continues a video | Extends its own clip, which is the stitching problem solved by the model; up to 4K |
| `bytedance/seedance-2.0` | ~0.37 (estimate) | 4–15 s | first, last, references (images and video) | Strongest prompt adherence and multimodal references; up to 4K |

Seedance prices by token. The estimate uses ByteDance's published
width × height × fps / 1024 at 24 fps, which OpenRouter does not state.

**Swap-in:** `google/veo-3.1` (0.20/s, 0.40/s with audio, 4K, 8 s) replaces
FLUX 3 if one top-fidelity shot matters more than continuation.

**Left out:** Grok Imagine Video 1.5 and 1.5 Lite (first frame only),
HappyHorse 1.1, Wan 3.0 Prime and Veo 3.1 Fast (an older or variant tier of a
listed family), HeyGen Video (general but new and unpriced at 1080p), and
everything at 720p or below. That includes Kling 3.0, Runway Gen-4.5 and
Seedance 2.5, though Seedance 2.5's 30 s clips are tempting if 720p is ever
enough.

## What a video costs

No model makes more than 30 s per call. A full track is a sequence of
generations: Door, No Room (5:22) is roughly 20–40 calls. That comes to
about **$16 with Veo 3.1 Lite**, **$65 with Wan 3.0** and **$120 with
Seedance 2.0** at 1080p, before retries. The realistic use is shorter:
inserts, a Threshold "beyond", a cover image brought to life, bridges
between sections. That costs cents to a few dollars a section.

## Image inputs, across the 1080p field

| Images in | Models |
| --- | --- |
| first + last + references | Wan 3.0, Wan 2.7, Hailuo 3, Seedance 2.0 |
| first + last | Veo 3.1, Veo 3.1 Fast, Veo 3.1 Lite, FLUX 3 Video, Wan 3.0 Prime, Seedance 1.5 Pro |
| first + references | HappyHorse 1.0/1.1, Wan 2.6, Hailuo 2.3, HeyGen Video |
| first only | Grok Imagine Video 1.5, 1.5 Lite |

"References" is read from the pricing SKUs and the model description. The API
has no field for it, so check a model's own page before relying on it.

## When this becomes a tool

Not designed yet; recorded so the next session starts from the right
questions:

- **Spending needs consent.** It goes through the Assist provider contracts
  (`docs/ASSIST_PROVIDER_CONTRACTS.md`): an explicit route, a price estimate
  before the call, and the OpenRouter credential rules that already exist.
- **The output becomes a project asset,** content-addressed and bundled like
  audio, so a `.musi` and an export stay reproducible without calling the
  model again.
- **For agents, it is a treatment feature.** A section that plays a clip
  instead of a scene, or a `generated` transition between two sections'
  frames, so the dock's agents can direct it. For the operator it is a button
  that shows the price first.

## Clips as material (built 2026-11-14)

A section can play a video file: the scene `clip` (Video Clip, id 29), in a
treatment `{ "scene": "clip", "clip": "take.mp4" }` with `from`, `after` (hold,
loop, bounce) and `fit` (cover, contain), or in the window through "Import
clip…" in the Scenes flyout or a dropped file. The grammar is in
`docs/MUSIC_VIDEO_CLI.md`, the `.musi` members in `docs/PHASE0_INVENTORY.md`
6.2, the gate is `tools/clip_check.sh`.

- **The file is an asset**: content-addressed and bundled like the audio
  (`<stem>.assets/clips/<sha256>.<ext>`), so a project and its exports do not
  need the original path, and two sections of one file share one copy.
- **Which frame is arithmetic on time** (`core::project::clip`): the probe
  reads every frame's presentation timestamp, and the clip ends where its
  last frame's own packet duration says (a variable-rate clip may hold its
  final frame far longer than the gap before it); a video frame at track time `t`
  in a section shows the last clip frame presented at or before
  `from + (t - origin)`, folded by `after`. A still and the export's frame at
  `t` are therefore one frame, and the gate finds them bit-identical to the
  frame FFmpeg extracts from the clip.
- **Bounded before anything allocates**: the file is opened once without
  blocking and checked by its handle; FFmpeg reads that handle
  (`/proc/self/fd/0`); the probe refuses a size past 8K before listing a frame
  (a 53-byte y4m claiming 16000x9000: refused, ffprobe peaking at 53 MB); the
  frame list is read through a cap; a decoder's output is cut at one probed
  frame and FFmpeg runs with `-max_pixels`.
- **A clip that cannot be drawn says so**: missing, changed (digest), decoding
  differently from what the project recorded, truncated, or still opening — a
  card naming the clip and the reason, and a `clip:` report line.

### Decoding: one FFmpeg per clip, restarted on a jump

Measured 2026-11-14 on this machine (FFmpeg 8.0.1, 24 cores) with 1080p H.264
clips: the testsrc2 fixtures (20 s at 30 fps, 12 s at 23.976) and
`door-title.mp4` (10 s, keyframes up to 205 frames apart).

| | |
| --- | --- |
| Reading on, 1080p to RGBA through a pipe | 240-268 frames/s (8x a 30 fps clip) |
| First frame after starting FFmpeg at a position | 95-126 ms near a keyframe, up to 340 ms deep in a long GOP |
| Same with one decoder thread | 95 ms to 2.2 s: frame threading is what makes seeks fast |

So the choice was a decoder restarted on a jump rather than an intermediate
decoded once at import: an all-intra proxy would still pay the ~100 ms start
floor and cost disk on `/` (a 30 s 1080p clip at 480x270 is ~175 MB even as
YUV), and a frame cache of the whole clip does not fit memory (a 30 s 1080p
clip is 7.5 GB of RGBA). `runtime::clip::ClipWorker` owns one FFmpeg and a
thread per clip and role: it reads on for small steps (up to 30 frames),
restarts at a jump, reads six frames ahead while nothing new is asked, and for
a bounce's way back decodes a run of 24 frames ending at the frame and keeps
them. FFmpeg 8's accurate input seek drops every frame presented before the
position, so a restart for frame `k` is told the midpoint between `k-1` and
`k`, offset by the container's start (a clip whose sound starts first);
unit tests pin both against the frames read from the start.

The export, stills, contact sheets and posters wait for the exact frame. The
preview waits at most 8 ms and otherwise draws the nearest it has, counted on
the `clip:` line: playing 240 frames, 221 exact, 3 inexact (16 while the clip
opened); a 60-step drag forward over 14 s, 68 inexact, exact 8 frames after it
ended; a 30-step drag back, 28 inexact, exact at once.

## What the generation half needs from this

The seam is a file. Whatever calls a model ends with an MP4 on disk, and
`crate::clips::import` (the app) turns it into a library entry with a digest
and a probe, and a placement puts it in a section — exactly what "Import
clip…" does. What generation adds, and where it plugs in:

- **The disabled "Generate…"** in the Scenes flyout's footer (while Video Clip
  is the scene) is where the control goes; `rail.rs`'s `clip_footer` draws it.
- **First and last frames**: `--still SECONDS FILE` already renders the exact
  frame at a time; a generation for a section takes the still at the section's
  start (or the previous section's end) and the next section's start, which is
  the bridge idea above.
- **The asset's provenance**: a generated clip is an ordinary `ClipAsset`. A
  `.musi` has no field for the model, prompt and price that made it; that
  wants an optional member on the asset, written only when present, as the
  rest of the clip members are.
- **Consent and price**: the Assist provider contracts, as above. Nothing in
  the clip half calls a network service.
- **A `generated` transition** between two sections' frames would be a clip
  section of its own, generated from the two stills and placed across the cut.

### The gate's negative controls

`tools/clip_check.sh` (run from `tools/treatment_check.sh`). Each control was a
scratch binary (`CLIP_BIN`, GPU only), the source reverted byte for byte after
the build (`git diff --quiet` on the file), 2026-11-14:

| Perturbation | What failed |
| --- | --- |
| Frame index as clip time × 24 instead of the probed frame times | six of eight exact stills: the matched frame at 14-19 dB and a neighbour at `inf` (t = 3.4 and 6.9 land on the same frame either way) |
| `loop` and `bounce` swapped | the loop and both bounce stills, each matching a neighbour better than its own frame |
| The missing-clip card not drawn | "the missing-clip card has no ink" (the report line alone still named it) |
| The drift's clock hurried by loudness whatever `response` says | "response 0: the still over music differs from the still over silence" |
| Exports drawing in the preview's role (no wait for the exact frame) | every exact still (3.7 dB: the "Opening" card), and the clip line's placeholder count |

### The review fixes' negative controls (F1b, 2026-11-14)

Each new check was run against a perturbed source, watched failing, and the
source restored byte for byte (`cmp` against a saved copy):

| Perturbation | What failed |
| --- | --- |
| Bundling's source opened without `O_NONBLOCK` | both FIFO tests (`publish`, `project_files`): the save parked for the 5 s the test waits |
| The copy's byte cap ignored | `an_endless_source_stops_at_the_limit_and_publishes_nothing`: the endless source reached the digest instead of stopping at the cap |
| The last frame's recorded duration ignored (the gap before it again) | `the_last_frame_lasts_its_own_recorded_duration` (0.12 s, not 1.08) and `a_probe_reads_how_long_the_last_frame_lasts` against a real held-frame MP4 |
| A split copying the placement's `from` again | `splitting_a_clip_section_changes_no_moment_of_the_clip`, the section's own `from` (clip time 2 at 15 s, not 7) |
| The track's `from` read as every section's own | the same test, the track's-clip case |
