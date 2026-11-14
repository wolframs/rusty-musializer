# VG: generated video as material (scoping, 2026-10-21)

The operator, 2026-10-21: video generation "is getting pretty good" and "would
be worth having as a tool too in the future, but first, I'd want to estimate
prices and available models". Then: consider only models that can do 1080p,
only the latest ones, offer users and agents **5 picks**, leave editing
models for later, and find out which models take images as input (a baseline
image, or first and last frames).

**Both halves are built (2026-11-14):** any video file can be a section of
the video — see *Clips as material* below — and a video model can make one,
from the window or from a treatment, with the price shown first — see
*Generating a clip*. The queue row is VG in `FEATURE_PARITY_PLAN.md`.

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
clip…" in the Scenes flyout or a dropped file. Look's section card (F1c)
edits all four for the selected section, with the library as rows of
middle frames and a poster of the section's first frame
(`ui/panels/tune/cards/clip.rs`, gate `tools/look_cards_check.sh`, groups
`clip` and `captures`). The grammar is in
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
  blocking and checked by its handle (a bundled one beneath the project's
  folder, one recorded name at a time, none of them a symlink); FFmpeg reads that handle
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

### What stays resident, across all clips (F1b, 2026-11-14)

Each worker used to keep up to 192 MiB of frames and its FFmpeg until it had
gone unused for 900 clip draws, and the count stopped whenever another scene
drew; thirty one-second 1080p sections kept thirty decoders and their frames
to the end of the video. Now:

- **One budget per process** (`runtime::clip::ClipBudget`): at most 4 FFmpeg
  decoders and 512 MiB of kept frames, a lease's share being 128 MiB (15
  frames of 1080p). A decoder starts only with a lease; when the budget is
  full the least recently asked worker gives its lease back first. A worker
  asked nothing for 3 s gives it back on its own. Both keep the last frame
  served, so a paused picture stays.
- **A worker exists only while its clip is on screen or coming up**: drawn
  this frame, or played by a section within 0.5 s before and 2 s after the
  frame's time, judged after every frame whatever scene drew it
  (`ClipResources::end_frame`). An export's workers go 10 s after it stops
  drawing.
- **A clip verified once is not hashed again** while its inode, size,
  modification and change times are the same, so a worker started again for
  a clip coming back costs a decoder start, not a hash.
- The `clip:` line ends with `resident entries=N released=N, budget decoders
  N/4, frames N/512 MiB, workers N, evictions N, overruns N`.

Measured with thirty distinct one-second 1080p H.264 clips as sections 1-31
of a 40 s track, Spectrum before and after, resident memory of the process
and its child processes sampled every 0.2 s (export at 640x360 with NVENC; the
preview a 1200-frame scrub across the track, posters on):

| | Peak resident | Peak children | On the closing Spectrum (80-95 % of the run) |
| --- | --- | --- | --- |
| Export, before | 6186 MiB | 31 | 6186 MiB, 31 |
| Export, after | 558 MiB | 3 | 289 MiB, 1 (the encoder) |
| Preview, before | 4314 MiB | 31 | 4314 MiB, 31 |
| Preview, after | 486 MiB | 2 | 233 MiB, 0 |

The export took the same time (33.1 s before, 33.3 s after), and the preview
drew the same counts of exact, inexact and opening frames (405/74/421 before,
413/73/414 after). Played in real time, the old build reached only 6.8 s of
the track in 52 s, holding 19 child processes, most of them poster workers no
frame ever released.

## Generating a clip (built 2026-11-14, H4)

A section's clip can be made by one of the five picks through OpenRouter, from
a prompt and, when asked for, the frame the section starts on and the frame
the next one starts on — rendered here through the export's still path, look
applied. The pure half (the picks, the price list, the estimate, the request
and its identity, the provenance) is `core::generation`; the client, the job,
the store and the card are `musializer-app/src/generation/` and
`ui/panels/tune/cards/generate.rs`; the treatment's grammar is in
`docs/MUSIC_VIDEO_CLI.md`, the `.musi` member in `docs/PHASE0_INVENTORY.md`
6.2, the contract in `docs/ASSIST_PROVIDER_CONTRACTS.md` (`TC-VIDEO`).

- **The price first.** `GET /api/v1/videos/models` (public, keyless) is read
  into the store's `prices.json` with the time it was read: by the card's
  "Read today's prices", by a treatment run, by `--vocabulary`, when the copy
  is missing or older than 7 days. A missing or stale list, a model not on
  it, or a SKU this application does not read is **price unknown**, in those
  words, on the button and the command line; a run with a ceiling refuses
  it. The estimate is the listed rate for the request's tier, without audio,
  times the seconds (`core::generation::price`, unit rules in its header);
  token-priced models are marked `~` (the token formula is ByteDance's, not
  OpenRouter's). It is rounded up to the cent.
- **Consent per job.** In the window the button says "Generate for about
  $0.12"; its press arms it and the second, "Send to OpenRouter · $0.12", sends.
  The order carries the price shown, and a request whose price changed since
  is refused. A treatment run spends only with `--generate-clips --max-spend
  USD`, within the ceiling by the estimates of the jobs it submitted.
- **What leaves the machine,** said on the card and on each `generate:` line:
  the prompt, and the frames when asked for (PNG at the request's size). No
  audio, no file name, nothing naming a person; the request has no `user`,
  `session_id` or callback, and the client calls itself `musializer-video/1`.
  The key goes to curl on stdin, only to the API's own host.
- **Nothing paid twice.** A request is identified by a digest of everything
  that determines it (`request_sha256`, including the frames' bytes). A clip
  the project or the store already has for it is used without a call (the
  window looks in the open project first, as a treatment run does); a job
  submitted and not yet downloaded (a Cancel, a crash, a closed window) is
  recorded in the store first and polled again by the next press. From the
  lookup until the job id is on disk one caller owns the request (a lock per
  digest in the store), so two windows, or a window and a run, pay once.
  What happens when the answer to a submit is lost: *Billing safety* below.
- **Nothing half-imported.** The download goes to scratch, capped (1 GiB, and
  curl's `max-filesize` stops a stream that grows past it), refused when it
  names itself JSON or text, opened by its handle and probed exactly as
  "Import clip…" does, and only then kept and placed. A failure says why by
  name, and what OpenRouter reported for it when it did.
- **The store**: `$XDG_DATA_HOME/musializer/generated-clips/` — paid clips
  with a receipt each (their provenance and their bytes' digest), job records,
  intents (every send of a request not yet answered), one lock file per
  request, the price list and `spending.tsv`, the ledger: a line as each
  request leaves and one when its job ends, the possibly billed ones marked
  (`docs/PHASE0_INVENTORY.md` 6.7b). Saving a
  project bundles a placed clip like any other; a save writes only clips a
  section plays, so a clip left unplaced lives in the store.

### The API as found (read 2026-11-14)

Sources, read 2026-11-14: <https://openrouter.ai/docs/api_reference/videos.md>,
<https://openrouter.ai/docs/api/api-reference/video-generation/submit-a-video-generation-request.md>,
`…/poll-video-generation-status.md`, `…/download-generated-video-content.md`,
<https://openrouter.ai/docs/cookbook/video-generation/text-to-video.md>,
`…/image-to-video.md`, OpenRouter's `openrouter-video` skill on GitHub
(data URLs for local images), and the live `/api/v1/videos/models` (32 models,
28 kept: a model with no duration or an unreadable id is dropped).

- `POST /api/v1/videos` with `model`, `prompt`, `duration` (one of the model's
  `supported_durations`), `resolution` and `aspect_ratio` (or `size`),
  `generate_audio` (defaults to the model's capability: send `false`), and
  `frame_images: [{type: "image_url", image_url: {url}, frame_type:
  "first_frame" | "last_frame"}]`, where `url` may be `data:image/png;base64,…`.
  Answers `202 {id, polling_url, status: "pending"}`.
- `GET /api/v1/videos/{id}`: `pending`, `in_progress`, then `completed`,
  `failed`, `cancelled` or `expired`; `error` as a string; `usage.cost` in
  dollars once known; `unsigned_urls` (this API, key needed) and `urls`
  (signed storage, no key, until `expires_at`).
- `GET /api/v1/videos/{id}/content?index=0` streams the MP4 (the key needed).
- Errors are `{error: {code, message}}` with 400 (naming the supported
  values), 401, 402, 403, 404, 413, 429, 5xx.
- **There is no cancel call.** Video is not eligible for zero data retention
  (`provider.zdr: true` fails routing).

**Recognising a request it already accepted** (H4b, read 2026-11-14):
nothing. <https://openrouter.ai/docs/api/api-reference/video-generation/submit-a-video-generation-request.md>
lists no request header and no idempotency key or client reference for
`POST /videos`; its body's `session_id`, `user` and `trace` group requests for
observability and broadcast destinations and are not returned by any video
endpoint. <https://openrouter.ai/docs/api_reference/videos.md> documents
four endpoints and no listing ("The Video Generation API consists of four
endpoints"), tells a client to "record which end user owns each job ID", and
has an idempotency key only on webhook deliveries
(`X-OpenRouter-Idempotency-Key: <jobId>-<status>`, which needs a
`callback_url` reachable from the internet). The activity endpoint
(<https://openrouter.ai/docs/api/api-reference/analytics/get-user-activity-grouped-by-endpoint.md>)
gives per-day totals with no job ids and needs a management key. So a job
whose id never reached this application cannot be found by it; the person can
see whether it was billed at <https://openrouter.ai/activity>.
- Not used: `input_references`, `seed`, `callback_url` and webhooks,
  `previous_job_id` (continuation), provider routing.

### The gate and its negative controls (H4)

`tools/generate_check.sh` (run from `tools/clip_check.sh`) drives the whole
path against `tools/fake_openrouter.py` on loopback, with a canary key in the
private credentials file, and `tools/generate_card_check.py` presses the card
(what each asserts is at the top of each file). The job's own unit tests run it
against an in-process fake (`generation::job::tests`). Each control below was a
scratch binary (`GENERATE_BIN`), the source restored byte for byte (`cmp`
against a saved copy) and rebuilt, 2026-11-14:

| Perturbation | What failed |
| --- | --- |
| A run without the flag treated as one with an unlimited ceiling | "without the flag: the estimate … exit 1" (status 0), "no submit reached the provider", and the steps after it that expected the store empty |
| The card's first press sending at once (no arming) | "the first press arms it: Send and Back", "Send made one submit" (the job had gone already), the card's outcome |
| The frames rendered at the project's output size instead of the request's | "the frames sent are the stills of the frame before the section and the first after it, byte for byte" |
| The clip row's Generate… drawn disabled again | "a press on Generate… opens the block" |
| The store's lookup and the job record ignored (unit tests) | `a_job_submits_polls_downloads_verifies_and_is_found_again_for_nothing` (submitted again) and `a_stop_after_the_submit_says_it_may_be_billed_and_a_second_press_picks_it_up` |
| The codec dropping `cost_usd` on read | `tools/differential_project_io.sh` step 20: re-written into different bytes |

### Billing safety (H4b, 2026-11-14)

A review found nine ways the first build could make the person pay twice or
send a request they had not agreed to. The rule since: **the application never
sends a paid request the person has not agreed to at a stated price, and never
sends the same request twice without telling them the first may already have
been billed.**

- **One owner per request.** `Store::claim` takes `locks/<digest>.lock`
  (`flock`, given back with `File::unlock`) from the lookup until the job id
  is on disk; a second caller's stage reads "waiting: another window or run is
  sending this same request", then it polls the first one's job. A job found
  submitted or in flight is picked up; since H4d that costs a treatment run
  nothing against `--max-spend`, and `--max-spend 0` picks it up too (*The
  ceiling, redefined* below).
- **An intent before the POST.** `intents/<digest>.json` (digest, model,
  seconds, the time, the estimate; since H4c every unanswered send, see
  *The second review's fixes*) is written before the request leaves,
  replaced by the job record when the id arrives, and removed when the
  provider refused it (a 4xx other than 408) or the call never wrote a byte of
  it (curl's `size_request` is 0: no such host, connection refused). Anything
  else after the request left — a dropped connection, a cut-short or
  unreadable answer, a Cancel or a closed window mid-submit, a 5xx or 408 —
  reads:
  > The request was sent and its answer never arrived, so it may have been
  > billed (about $0.12) for a job this application cannot find: OpenRouter
  > has no way to look one up. Asking for it again would be a second request,
  > and you are asked first.
- **A left-over intent holds the request.** Nobody holds its lock, so it is a
  request that may have been accepted with an unknown job id. The window's
  next Send for it sends nothing; the notice ("An earlier send may have been
  billed") and the card read
  > This exact request was sent on 2026-11-14 21:13:16 UTC and its answer
  > never arrived, so OpenRouter may have accepted and billed it (about
  > $0.12). OpenRouter has no way to find a job whose id was lost; your
  > activity page at openrouter.ai/activity shows whether it was billed. Press
  > Generate again to send it anew: that is a second request, and it is billed
  > too.

  The button then reads "Generate again for about $0.12" and, armed, "Send
  again · $0.12 more"; only that second press sends it again. A treatment run
  prints the same first sentences and refuses: "This run does not send it
  again: add --resend-unconfirmed for a new send, which counts against
  --max-spend at its estimate ($0.12)"; with the flag the resend is a new send
  of that run, counted once (H4d).
- **A refused download keeps its paid job.** One ask downloads a finished
  clip up to three times; then "the finished clip was downloaded 3 times and
  refused each time (last: …); its job is kept, so Generate again downloads it
  again without paying again". A job is forgotten only when OpenRouter says it
  ended without a clip: failed, cancelled, unknown (404), or "the job expired
  at OpenRouter before its clip was downloaded; Generate again asks (and pays)
  anew".
- **Bounded answers.** Submit and poll carry curl's `max-filesize` (1 MiB),
  and an error body is read through the same cap: "the answer is larger than
  1 MiB".
- **The project's own clip.** The window places a clip the open project holds
  for the request: "… plays in section 2 of 3 (…). The project already holds
  the clip made for exactly this request, so nothing was sent."
- **The section it was for.** A job carries its section by identity (project:
  slot, audio file, project file; audio digest; cue id and span at Send) and
  whether a treatment was followed at any frame while it ran. Otherwise the
  clip is not placed: "… is in this project's clip library, not in a section:
  a treatment was followed while it was made, and the followed file decides
  what each section plays. Nothing paid for is lost: choose it in a Video Clip
  section's clip rows (a save keeps only clips a section plays), and it stays
  in …/clips, where Generate with the same settings finds it without paying
  again." Generate is refused while a treatment is followed.
- **No stand-in in a frame sent.** The review's case, a section ending on
  `"next"` while the next one is generated too, is refused when the treatment
  is parsed. A frame inside a transition whose showing side is a generated
  section not made yet (a short section fading in over all of its length
  after a generated one this run could not make) is refused by name: "its
  frame at 0:05.979 lies inside the transition at 0:04.000, which shows
  segments[1] (0:02.000).clip.generate, a generated section not made yet; the
  frame would be a stand-in, so nothing was sent for it. Make that section
  first, or give this one an image file for the frame".
- **The export's frames.** A run renders the frames under `--resolution`,
  `--fps` and `--quality` as the export applies them (a 24 fps treatment at
  `--fps 30` sends the picture at 119/30 s, not 95/24 s).
- **One project's card.** Switching projects forgets the generate block, and
  an armed Send belongs to the request it was armed for; outcomes, warnings
  and jobs are kept by project and section.

### The billing-safety checks and their negative controls (H4b)

`tools/generate_check.sh` groups `concurrent`, `lost`, `redownload`, `order`
and `fps` (`GENERATE_ONLY="…"` runs some), the card check's steps 4b, 5b and
6b, and unit tests in `generation::{job,client,window}` and the look cards.
The fake provider's markers: `[slow-submit]`, `[hold]`, `[lost-answer-once]`
(accepted, the connection dropped without an answer; `[lost-answers-N]` for the
first N, H4c), `[truncate-N]`,
`[expire]`, `[polls-N]`, and since H4c `[stale-poller]` (two pollers of one job hear
it expire at different times; release files `release-stale`, `-2`, `-3`). Every spending check asserts the provider's submit
count. Each control was a scratch binary or test run on a perturbed source,
restored byte for byte (`cmp` against a saved copy), 2026-11-14:

| Perturbation | What failed |
| --- | --- |
| `Store::claim` a no-op and `in_flight` always false | `two_callers_for_one_request_pay_once` (2 submits, not 1); gate concurrent: "two runs for one request paid 2 times", the second run generated instead of picking up |
| A job found submitted or in flight counted at 0 again, no `--max-spend 0` refusal | gate concurrent: `--max-spend 0` picked the in-flight job up (status 0); a $0.10 ceiling admitted the $0.12 job |
| Every failed submit "nothing" and its intent removed (as before) | gate lost: the lost answer not reported as possibly billed; the next run sent it again (2 submits, status 0) |
| The window's prepare ignoring a left-over intent | card check 5b: the next Send went out silently |
| The job forgotten after the refused downloads (as before) | `a_refused_download_keeps_its_job_and_the_next_ask_downloads_it_again` ("the paid job is kept"); gate redownload: the next run paid again (2 submits) |
| An expired job's record kept | gate redownload: "the expired job was not named or not forgotten" |
| `max-filesize` dropped from the JSON calls | `a_json_answer_is_bounded_on_the_wire_and_in_memory`: 8388610 bytes reached the disk. The in-memory cap on an error body has no observable difference once the wire cap holds and is not pinned separately |
| The window's lookup in the open project skipped | card 4b by hand: with an empty store the same Send made a submit and read "Estimated $0.12" instead of "nothing was sent" |
| Placement ignoring a followed treatment | `a_clip_is_placed_only_in_the_section_send_was_pressed_for`; card 6b by hand: the followed file's section 2 (Pulse) became Video Clip with the clip, the review's defect |
| The stand-in guard absent | gate order: section 3 was sent a first frame inside the fade from section 1's stand-in (2 submits) |
| The command line's output flags applied after the frames (as before); then only `--quality` left out | gate fps: the frames sent under `--fps 12 --quality high` are not the export's, both times |
| `CardState::forget` leaving the generate block; the card's view not filtered by project | `another_project_forgets_the_generate_block_and_its_armed_send`; `the_card_reads_only_the_current_projects_sections` |

Not pinned by a check of its own: the armed Send's request key (the card
already disarms on each control that changes the request; the key is the
second guard), and the window's Cancel of only the current project's job.

### The second review's fixes (H4c, 2026-11-14)

A review of H4b confirmed seven more defects, five of them here. The rule
since: **the application never sends a paid request the person has not
agreed to at a stated price; a ceiling the person set is never exceeded by
what the application knowingly commits, counting everything that may already
have been billed; and where the truth cannot be known, it says so and counts
it as spent.** The middle clause was replaced on 2026-11-15 (H4d, *The
ceiling, redefined* below): a ceiling bounds what the run itself sends, and
what may already have been billed is said beside it, not counted in it.

- **Durable records.** Every store record (intent, job, receipt, price list)
  is written through a sibling, synced, renamed and then its folder synced; a
  folder made for it is synced into its parent; a kept clip's bytes (curl
  never syncs them) are synced before the rename; each `spending.tsv` line is
  synced. A power loss right after OpenRouter accepted a request keeps the
  intent or the job that says so.
- **A record goes only if it is still the one read.** A caller that polled a
  job and heard it expired, failed, was cancelled or is unknown (404) removes
  the job record only under the request's lock and only if it still names
  that job (`Store::forget_job_if`): another caller polling the same job may
  have forgotten it already and a third submitted and recorded a new, paid
  one, which a blind removal lost — and the next Generate then paid a third
  time with no warning. The same after a kept clip (`forget_job_held`).
- **Every unanswered send stays, and counts** (`intents/<digest>.json` v2:
  every attempt with its time and estimate). A send agreed after a lost one
  used to replace it, so two lost $0.12 sends and `--resend-unconfirmed
  --max-spend 0.24` admitted a third, $0.36 against a count of $0.24. Now a
  send is added beside the earlier ones; it is taken off when its job id
  arrives, the provider refused it or it never left; all go only when the
  request's clip is in hand. A run counted their sum against the ceiling
  until H4d (now it reports it on its own line), and the words name them all:
  > This exact request was sent 2 times, first on 2026-11-14 22:29:38 UTC and
  > last on 2026-11-14 22:29:40 UTC, and no answer arrived, so OpenRouter may
  > have accepted and billed each of them (about $0.24 in all). OpenRouter has
  > no way to find a job whose id was lost; your activity page at
  > openrouter.ai/activity shows which were billed. Every send is in
  > …/generated-clips/spending.tsv.

  One send keeps the H4b sentence, and both end with "Every send is in
  …/spending.tsv." — in the window's notice and card and on the command line.
  Agreeing to send again names the newest send by its id: a send lost after
  the warning was shown makes the next press warn again instead of sending.
- **The plan comes first** (its accounting replaced by H4d; the forecast
  stays). A treatment run discovered a held send only when
  it reached its section: with a new $0.12 request in section 1, a held $0.12
  send in section 2 and `--max-spend 0.12`, section 1 went out first — $0.24
  against $0.12. Now a run looks at every generated section before it sends
  anything (frames rendered, request built; found, a job to pick up, sends
  never answered, new), adds up what is committed or possibly billed, decides
  in time order what the ceiling still admits, and prints the plan:
  > generate:        segments[1] (0:03.000).clip.generate refused: it would
  > pass the ceiling: $0.12 committed or possibly billed, $0.12 more, $0.12
  > allowed
  > generate:        plan: $0.12 possibly billed already, $0.00 to send or pick
  > up, $0.12 of $0.12 allowed; nothing has been sent yet

  and admitted sections as "plan: … is sent, $0.12", "picks up the job sent
  before, counting $0.12", "is sent again after 1 unanswered send, $0.12
  more", "waits for … to be made, counting $0.12". A section whose frames
  show a generated section made earlier in the run cannot have its request
  known before that one is made: it is counted at its estimate and looked at
  again then, as is any section a clip made since may show in, and every send
  is checked once more with what is known at that moment.
- **The ledger** (`spending.tsv`) gets a line before each request leaves
  ("sending, billed if accepted") and one when the job ends, marked ", possibly
  billed" or ", answer lost, possibly billed" where no cost was reported. A
  crash mid-submit leaves the first line alone, which reads as what it is.
  The ceiling arithmetic reads the intents, not the ledger: they hold what is
  unresolved now, the ledger everything that ever left.
- **Maxima round up.** A run prints `--max-spend` as given ("$0.125
  allowed") and what it has committed rounded up to the cent.

### The second review's checks and their negative controls (H4c)

Each control was run on a perturbed source and the source restored byte for
byte (`cmp` against a saved copy), 2026-11-14:

| Perturbation | What failed |
| --- | --- |
| `forget_job_held` removing whatever record is there (the job id not compared, as before) | gate stale: "the late poller removed job 2's record", and run D submitted again (3 submits, not 2) |
| The plan not counting held sends (`possibly_billed` left at 0, as when a held send was found only on reaching its section) | gate plan: "the plan did not count the held send before sending" — under `--max-spend 0.12` section 1 was submitted beside section 2's held $0.12 (1 submit, the review's defect) |
| `add_attempt` replacing the earlier sends (as before: one intent per request) | gate lost: "the earlier lost sends were not both counted" — the third send went out under `--max-spend 0.24` (3 submits, status 0); `every_unconfirmed_send_stays_and_counts` and `two_lost_answers_both_stay_until_the_clip_is_in_hand` |
| `write_private` without the folder's sync after the rename | `every_record_is_synced_with_its_folder`: the intent's folder `store/intents` missing from what was synced. A power loss itself cannot be staged in a test; the test shows the folder is opened and synced, not that the disk keeps it |

### The ceiling, redefined (H4d, 2026-11-15)

Three reviews in a row found the prior-exposure accounting wrong in a new way:
a job in flight left out of the sum, a job record hiding unanswered sends of
the same request, a section whose request was known only after earlier sends,
and store changes another process made after the plan. The lead replaced it
with a rule simple enough to keep: **`--max-spend N` is the most this run may
newly send.** The estimates of the requests the run itself submits never add
up to more than N; nothing else counts.

- **Kept where money leaves.** `generation::job::Ceiling` is a run-local total
  shared by the run's jobs. `Ceiling::take` adds a request's estimate, or
  refuses it, inside the job, under the request's lock (`Store::claim`) and
  immediately before its intent is written; a send that cannot have been
  billed (the provider refused it, it never left, its record could not be
  written) gives it back. No order of sections, no section known only later
  and no other process's change to the store reaches that check. An unknown
  price is refused there under a ceiling.
- **A pickup is free.** A job an earlier run, another run or the window sent
  is picked up for nothing, even under `--max-spend 0`.
- **A held request** is refused by name without `--resend-unconfirmed`; with
  it the resend is a new send of this run and counts once, at its estimate.
- **What may already have been billed is said, not counted**, on a line of its
  own before anything leaves:
  > generate:        before this run: about $0.24 may already have been billed
  > for 2 earlier sends of these requests (1 job to pick up, 1 send never
  > answered); this is not part of --max-spend. Every send is in
  > …/generated-clips/spending.tsv
- **The plan is a forecast.** For a run with a job to pick up, a held request
  and a new one under `--max-spend 0.12`:
  > generate:        segments[3] (0:04.000).clip.generate This exact request
  > was sent on 2026-11-14 23:37:16 UTC and its answer never arrived, … Every
  > send is in …/spending.tsv.
  > generate:        segments[3] (0:04.000).clip.generate refused: This run
  > does not send it again: add --resend-unconfirmed for a new send, which
  > counts against --max-spend at its estimate ($0.12)
  > generate:        plan: segments[1] (0:01.000).clip.generate picks up the
  > job sent before; nothing new is sent
  > generate:        plan: segments[5] (0:07.000).clip.generate is sent, $0.12
  > generate:        before this run: about $0.24 may already have been billed …
  > generate:        plan: this run would send 1 request, about $0.12 of its
  > $0.12 allowed, and pick up 1 job; nothing has been sent yet

  and at the end "this run sent 1 request, about $0.12 of its $0.12 allowed".
  A section the forecast does not expect to fit is still tried (what goes
  before it may come to less), and is refused where it would leave: "it would
  pass the ceiling: this run has sent $0.24, this request is $0.12 more, $0.24
  allowed (--max-spend counts what this run sends)". When a later fact moves
  the forecast the run says so: "the plan counted it as a new send; its
  request, known now, was sent before and never answered", "now a job to pick
  up: it was sent since the plan by another window or run, and picking it up
  costs this run nothing".
- **Removed:** the plan's `possibly_billed`, `committed` and `reserved_after`
  sums, the held sends counted into them, the unknown-held refusal of the
  whole run, the "$X committed or possibly billed" words and the gate's
  `plan` group (replaced by `ceiling`).

### The ceiling's checks and their negative controls (H4d)

`tools/generate_check.sh` group `ceiling` (a job in flight, a held send and a
new request under one estimate; the held one again with the flag; `--max-spend
0` with a job in flight; three new requests under two estimates; a deferred
section that turns out held), the rewritten `concurrent` checks (a pickup under
`--max-spend 0` and under a ceiling below the job's estimate; two runs at once
under their own ceilings), `lost` (a resend counted once), and
`generation::job::tests::a_runs_ceiling_counts_only_what_it_sends`. Each
control was run on a perturbed source and the source restored byte for byte
(`cmp` against a saved copy), 2026-11-15:

| Perturbation | What failed |
| --- | --- |
| `Ceiling::take` admitting everything (the ceiling not kept where a request leaves; the forecast alone) | gate ceiling: three new requests under two estimates made 3 submits (status 0); base: the second of two clips went out under $0.15; concurrent: run P of the pair sent 2 requests under its own $0.12 |
| A pickup counted against the ceiling again (as under H4b/H4c) | gate concurrent: `--max-spend 0` and the $0.10 run refused the job in flight (status 1); the pair made 1 submit while the runs said 1 + 1 |
| A resend counted with the earlier unanswered send (twice its estimate) | gate lost: `--max-spend 0.12 --resend-unconfirmed` refused (status 1), and so did the third send of the twice-lost request; gate ceiling: the agreed resend refused (0 submits) |
| What changed not said when a deferred section turns out held | gate ceiling: "a deferred held section was sent or not explained" (the request was refused, silently) |
| The estimate not given back when the provider refused the send | `a_runs_ceiling_counts_only_what_it_sends`: (0.12, 1) left counted after a 402 |

Not pinned by a check of its own: a forecast-over section that is sent after all
because what went before it came to less (staged only by a pickup appearing
between the plan and the send, which no fake marker times reliably). The fake
provider makes a clip once per job under a lock since H4d: two pollers of one
job downloaded it while it was being written and refused it (seen in the
first control's concurrent group, not a defect of the application).

### The same review's smaller fixes (H4d, 2026-11-15)

- **An intent that cannot be read holds the request.** `Store::intent` read a
  record past 64 KiB (or one that did not parse) as no intent: the next
  Generate sent with no warning and `add_attempt` replaced the whole history.
  Now such a record is `Unreadable`, holds the request in the window and in a
  run, with or without `--resend-unconfirmed`, and is never written over; the
  words name the file and say to look at openrouter.ai/activity and then
  remove it (or put back a readable copy) to send anew.
- **Three unanswered sends stop a request** (`MAX_UNANSWERED_SENDS`). A third
  lost answer in a row on one request is no coincidence (a proxy or firewall
  cutting the connection once the body has left, a gateway timeout shorter
  than the provider's accept), and every further send may be billed and lost
  the same way: "This exact request was sent 3 times and no answer ever
  arrived (about $0.36 in all, each possibly billed). Something is wrong
  between this machine and OpenRouter (…), so it is not sent again, by the
  window or by a run, with or without --resend-unconfirmed. … remove
  …/intents/<digest>.json to send it anew." Three also keeps the record far
  below its 64 KiB bound.
- **No send without its ledger line.** `Store::log_spending` ignored a
  failed open, write or sync of `spending.tsv` and the submit went ahead, so
  the person's record of what may have cost money could miss a send. Now the
  line written before a request leaves must reach the disk, or the request is
  not sent: "the line that records this send in the ledger could not be
  written (…), so it was not sent"; its attempt is taken off and a run's
  ceiling gets the estimate back. The line written when a job ends comes after
  the money left: a failure there is a warning, and the job record or intent
  still holds the request.

| Perturbation | What failed |
| --- | --- |
| An intent that cannot be read taken for none again (`Err(_) => Ok(None)` in `Store::intent`, as before) | gate intent: the oversized record let the agreed resend out (status 0, 1 submit) and so did the unparseable one; `an_unreadable_intent_and_too_many_unanswered_sends_hold_the_request` panicked at the first hold |
| No bound on unanswered sends | gate intent: the fourth send of a request with three lost answers went out (4 submits); the unit test: "three unanswered sends did not hold the request" |
| A failed ledger line ignored again before the send (as before) | gate ledger: with `spending.tsv` a folder the request went out (status 0, 1 submit), and the next step found nothing left to send; `an_unwritable_ledger_stops_the_send` |
FIXES_H4D

### What is not built

- **A `generated` transition** (a bridge across a cut as a transition kind).
  A generated section with `"first": "previous"` and `"last": "next"` is that
  bridge today, placed as a section; a transition kind would make it one word.
- **A multi-line prompt field.** The card's field is one line that scrolls;
  the whole prompt is shown wrapped beneath it once it outgrows the field.
- **Reference images, a seed, continuation, webhooks**, editing models,
  upscalers and the model's own sound (out of scope by the brief).
- **A window that follows a treatment with a generated section**: such a
  treatment is refused there by name and pointed at the batch run, because
  its frames are rendered first and the window's follow applies on every
  save.
- **Cancelling at the provider**: OpenRouter has no call for it.
- **Finding a job whose id was lost**: OpenRouter has no idempotency key and
  no job listing (read 2026-11-14, *The API as found*); a left-over intent is
  said and held, never resolved by the application.
- **Keeping an unplaced generated clip in a saved project**: a save writes
  only the clips a section plays; such a clip stays in the store.

### What a clip cost (H4's real calls)

Three calls on 2026-11-14, all `google/veo-3.1-lite`, 4 s, 720p 16:9, no
audio, placed in a copy of Door, No Room's native treatment rendered at
1280x720. The clips, the ledger and the sheets judged are in the main
checkout's `build/generated-clips/` (never committed).

| Call | Frames sent | Estimate | Reported (`usage.cost`) | Submit to download |
| --- | --- | --- | --- | --- |
| A, "the architecture opens" (3:50.397) | none | $0.12 | $0.12 | 35 s |
| B, a new 4 s section at 0:24.8 | first = previous, last = next | $0.12 | $0.12 | 58 s |
| C, "a shape appeared" (2:05.266) | first = previous, last = next | $0.12 | $0.12 | 59 s |

$0.36 in all; every estimate was the cost to the cent. Frame images cost
nothing extra on this model (no `reference_images` SKU). Each run after the
first found the clips already made and sent nothing for them.

What it looked like, judged from contact sheets across each section:

- **A** follows a text prompt closely (translucent panels folding open onto a
  field of amber lights). Behind the section's iris, under the night-door
  grade and with the lyric "the architecture opens" over it, it reads as the
  video's own picture: the grade pulls the model's neutral greys into the
  video's lavender. The best use of text only: staging a sung line literally.
- **B and C** start on the frame the video leaves and end on the one it
  arrives at, and both cuts disappear: the frames either side of each cut are
  indistinguishable at sheet size. In C (pentagram to atlas) the model
  dissolves the pentagram's constellation and grows the atlas's spiral out of
  the dark, under "a shape appeared … that neither of us could have made
  alone": the cut stops being a cut and becomes the line's event. In B the
  threshold's last frame was already its "beyond" constellation, so the clip
  bridges two similar pictures and fills the middle with an invented soft
  planet drifting across: seamless, but the prompt's doorway never shows. A
  bridge earns its price where the two pictures differ.

### The billing-safety fixes' real call (H4b)

One call on 2026-11-14 through the reworked submit, on a fixture treatment
under the worktree's `build/h4b-real/` (never committed): `google/veo-3.1-lite`,
4 s, 720p 16:9, no frames, no audio, a text prompt naming nobody. The key was
read by the application from the operator's credentials file
(`MUSIALIZER_ASSIST_CREDENTIALS`), never in argv, a log or the store.

| Call | Estimate | Reported | Submit to download |
| --- | --- | --- | --- |
| "a slow drift of soft light over calm water at dusk" | $0.12 | $0.12 | 44 s |

The intent was replaced by the job record and both were gone after the
download; a second run found the clip in the store and sent nothing. Never
tried for real: a lost answer, a concurrent second caller, a refused or
cut-short download, an expired job, an oversized answer (the fake provider
only).

## What the generation half needed from this (the seam, as planned)

The seam is a file. Whatever calls a model ends with an MP4 on disk, and
`crate::clips::import` (the app) turns it into a library entry with a digest
and a probe, and a placement puts it in a section — exactly what "Import
clip…" does. What generation adds, and where it plugs in:

- **"Generate…"** in the Scenes flyout's footer (while Video Clip is the
  scene) and in Look's section card beside its Import clip… opens the
  generate block (built: *Generating a clip* above).
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
| The old fallback restored (an unresolved clip path joined to the folder and used) | `a_clip_record_that_leaves_the_bundle_is_refused_not_read`: the symlink out of the bundle opened with no refusal |
| The import target filtered to clip sections again (a selected Spectrum section falls through to the track) | `an_import_lands_in_the_section_pointed_at` and `the_notice_says_where_it_landed`. By hand, playhead at 4.5 s in the clip section of spectrum/clip/spectrum, section 1 selected, a clip dropped: before, the plan stayed `spectrum, clip, spectrum`; after, `clip, clip, spectrum` |
| The hash's look at `stop` removed | `dropping_a_worker_cancels_its_hash`: the drop waited 13.1 s for a 3 GiB hash |
| ffprobe's group unregistered and its poll blind to `stop` (as before) | `dropping_a_worker_kills_its_probe`: the drop waited 20.0 s, the probe's deadline |
| The display matrix ignored (the coded size forced again) | `a_turned_clip_decodes_as_ffmpeg_shows_it` (the frames differ from FFmpeg's), and the gate's turned still: 12.4 dB against FFmpeg's frame 25 (inf with the fix), the clip line `of 320x180` |
| A lease always granted (no recall when the budget is full) | `a_budget_of_two_decoders_runs_two_whatever_the_clips` (three decoders) and `the_budget_bounds_kept_frames_and_idle_workers_give_back` (two leases past the bytes) |
| `end_frame` keeping every entry | the gate's release check: `resident entries=1 released=0, budget decoders 1/4, frames 42/512 MiB, workers 1` on the Spectrum still after the clip section |

### The last review's negative controls (G, 2026-11-14)

Same procedure (`cmp` against a saved copy after each):

| Perturbation | What failed |
| --- | --- |
| A bundled clip opened by its resolved name again (`open_at` calling `open` on the folder joined to the record) | `a_clip_swapped_out_of_its_bundle_after_the_open_is_never_read`: with the bundled file swapped for a symlink to an outside copy, the decoder returned frame 0 instead of "leaves the project's folder" |
| The save's bundling reading the clip by name again (no opening beneath its folder) | the same test: Save As into another project bundled the outside copy instead of refusing |
| The recorded length never brought in line with the probe (`confirm_lengths` not called) | look cards check 37, both halves: the card read `clip-at=0.0200` against playback's 0.5, and Own start pinned 0.02 and drew frame 1 where frame 2 had been (the review's defect reproduced), the record still 0.12 |
| The card's Import dispatching the generic import again (`ImportClipDialog`) | look cards check 38: with the plan off, section 4 kept b.mp4 and the track's own clip became c.mp4 (the review's defect reproduced) |
