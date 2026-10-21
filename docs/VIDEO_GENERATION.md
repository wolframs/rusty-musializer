# VG: generated video as material (scoping, 2026-10-21)

The operator, 2026-10-21: video generation "is getting pretty good" and "would
be worth having as a tool too in the future, but first, I'd want to estimate
prices and available models". Then: consider only models that can do 1080p,
only the latest ones, offer users and agents **5 picks**, leave editing
models for later, and find out which models take images as input (a baseline
image, or first and last frames).

Nothing is built yet. This file is the shortlist and the reasoning behind it.
The queue row is VG in `FEATURE_PARITY_PLAN.md`.

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
