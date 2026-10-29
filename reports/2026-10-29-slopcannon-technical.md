# Slopcannon (slop.cc): how pricing, the model and the pipeline work

Read on 2026-10-29 from the live public sources: `api.slop.cc/v1/capabilities`,
`/v1/models`, `/v1/billing/packs`, `/openapi.json`, `slop.cc/llms.txt`, the
`slopcannon-music-video` skill and its references under
`slop.cc/.well-known/skills/`, `slop.cc/faq`, `slop.cc/terms` and
`sloptoberfest.slop.cc`. Items marked **unverified** have not been observed
yet; the first real quote and the first generation will settle them.

## The one model

- One generation model: **MiniMax Hailuo 3 ("H3")**, via a provider adapter.
  The assembly docs mention FAL cancellation, so the adapter is most likely
  fal.ai.
- `/v1/models` lists exactly one product: `slopcannon/quality-2k`. The only
  resolution is 2K. `fourKAvailable: false`. There is no cheaper draft tier.
  `create_project` accepts `480P`–`1080P` as `targetResolution`, but the
  generator offers only 2K, so that field buys nothing.
- Aspect ratios are 21:9, 16:9, 4:3, 1:1, 3:4 and 9:16. The ratio is fixed per
  project; a quote at any other ratio is refused with
  `409 project_aspect_ratio_mismatch`.
- Each generation is **one scene of 5–15 s**, with at most 1 scene per quote.
- Each scene takes up to **9 reference images** (`imageReferencesPerScene`),
  each ≤ 768 KiB. One image means one individual. Grids and lineups make the
  model spawn or morph figures.
- **Audio drives the mouth.** Each scene gets its exact slice of the master as
  the audio reference: mono, 24 kHz, 16-bit, starting at local 0, padded with
  silence to whole seconds. Lyrics are steered with H3 dialogue spans in the
  scene text: `At 00:03.420, LEAD (S1) sings, <d>[English]exact line</d>`.
- Model quirks the docs record:
  - The mouth follows **whatever is loudest in the slice**. A sound effect or
    hit under the first word drives the mouth instead of the vocal.
  - The model also invents sound for any action the prompt describes, and the
    mouth follows that sound. Keep loud action out of the vocal window.
  - The prose order beats the timestamp. "X happens, then LEAD sings" plays
    the line over X.
  - "Whisper", "mumble" and "hushed" produce a still mouth. Ask for clear
    articulation.
  - Readable lip sync needs one or two singers, medium shot or closer, facing
    camera. Groups of four or more cannot sync.
  - Unstated facing makes characters walk backwards. Every named subject gets
    `NAME (facing; motion vector)` in round brackets. Square brackets are
    parsed as reference names and break cast resolution.
  - Non-human characters with a front-only reference: keep the camera in
    front. Orbits and back views make the model invent the unseen side.
  - A delivered take's audio sometimes "steps": it repeats or skips a moment of
    the song. Assembly reports this as `lipSyncReviewSceneIds`; rerolling those
    scenes fixes it.

## Money

**Unit.** 1 Cannon Credit = $0.02778. Every pack has the same rate (278 ¢ per
100 credits): **there is no bulk discount**. Buy only what the next step
needs; the terms mention no expiry on credits.

| Pack | Credits | Finished 2K seconds |
| --- | --- | --- |
| $10 | 360 | 45 |
| $30 | 1,080 | 135 |
| $50 | 1,800 | 225 |
| $100 | 3,600 | 450 |
| $200 | 7,200 | 900 |
| $500 | 18,000 | 2,250 |

**Rate.** 8 credits per output second, which is **$0.222/s or $13.33/min**.

**What is billed (measured on the first real quotes, 2026-10-29).** The
quote's basis is `scene-duration`: `ceil(exact scene seconds × 8)`, with no
whole-second rounding per scene. A 6.53 s scene quoted 53 credits and a 5.3 s
scene quoted 43. `requestSeconds` (whole seconds, 5–15) is only the length
of the provider render and the silence padding of the audio slice; it isn't
billed. The docs' line calling it "the billing unit" is wrong, or out of date.

**Consequences:**

- The floor is 40 credits ($1.11) per generation, which is a 5 s scene. A
  short test costs the same as a real 5 s scene, so test with real scenes.
- A reroll costs the full scene again. Short scenes make cheap rerolls, so keep
  the risky lip-sync shots short and the safe instrumental shots long.
- **Neither the scene count nor where the boundaries fall changes the price**;
  only the total seconds do.

**Free:**

- planning cuts (`plan_cuts`; no provider call);
- song analysis (managed transcription);
- server scene-audio slicing;
- server assembly (`creditsPerOutputMinute: 0`, no hold);
- downloads, status checks and recovery;
- the take checker (`check_take`, a vision model looking for extra limbs,
  duplicate figures, or a mouth moving in a voice-over shot).

`direct_scenes` (the LLM that drafts scene text) and the browser Producer chat
cost no credits. They are metered against a "producer fair-use budget" whose
size is not published.

**Failures.** A job that is rejected or fails terminally releases its hold, and
moderation failures can be rerolled. A take that shows as finished but won't
download is a support ticket with the job ID, never a paid re-render.

**Flow of a charge.** A quote is exposure: it reserves nothing and is freely
repeatable. Submission reserves the quoted amount; completion debits it; failure
refunds it. Each generation's `settlement` block shows `reservedCredits`,
`debitedCredits` and `refundedCredits`, plus an optional `providerCostMicros`.
If that last field comes back filled, it shows Slopcannon's margin over
fal/MiniMax. `GET /v1/usage` is the full ledger.

**Storage.** 35 GiB of projects are included, plus a separate 100 GiB for
finished films. Each extra 10 GiB block costs 10 credits per 30 days. At the
limit, new work is refused with `413 STORAGE_LIMIT_EXCEEDED` before anything is
reserved.

**Payment.** Credit purchase is a human action: Stripe checkout at
`slop.cc/account#buy-credits`. The catalog also lists x402 (USDC on Base,
live). MPP is off. Our MCP key cannot buy credits.

## The pipeline (MCP or CLI, the same API underneath)

1. `create_project` fixes the aspect ratio.
2. **Rights.** The human confirms the rights statement
   (`rights-2026-08-12-v1`) for each upload. `create_upload_link` gives a page
   where the person ticks the statement and picks the file. One sign-off covers
   the session for files the same person directs.
3. **Upload** the master once (≤ 50 MiB), then run `start_song_analysis` for
   timed words (≤ 25 MB, `songAnalysisControlReady: true`).
4. **Plan.** `plan_cuts` scores scene boundaries against the word timings.
   Scenes are 5–15 s and must not cut through a word. Alternatively, write our
   own timed plan (schema 1: integer ms, contiguous from 0, an H3 span per
   visible line) and validate it with
   `validate-timed-plan.mjs --require-h3-vocals`. Speaker IDs (`S1`, `S2`)
   are fixed for the whole production.
5. `slice_scene_audio` cuts one audio chunk per scene on the server.
6. **Quote and approve, one scene at a time.** Each scene's quote carries the
   master as `songAssetId` and its chunk as `sourceAudioAssetId`. A
   `create_render_request` can declare the whole film first, which binds each
   scene to a single paid execution so a retry can't double-buy. The person
   approves the exact credit total.
7. **First scene alone**, reviewed by the person, before the rest are
   submitted. Up to 48 generations may be in flight; 4 is the default.
8. **Review.** `check_take` on every take, then `select_take` under the
   workspace revision. Deleting a take is permanent and refunds nothing.
9. **Assemble.** `start_assembly` with `clips` omitted uses the current
   selections. Each take is trimmed to its exact scene window, generated clip
   audio is dropped, and the master sits at PTS 0. An optional end-card PNG
   can be laid over the last scene. Captions are optional and added at
   assembly, never drawn into the footage.
10. **Download and publish.** `assembly_download_links`, then verify bytes and
    SHA-256. `create_publish_link` gives the person a link to publish on Slop
    TV, which produces the public film link the contest form needs.

The bundled CLI (`@slopcannon/cli` 0.3.0, node ≥ 22.13, ffmpeg) runs the same
flow from one `production.json` with a durable ledger
(`slopcannon video run --max-credits N`). It stops at the rights, approval,
budget and first-scene-review gates.

## Levers we can pull (untested)

- **Reuse one take in several windows.** An explicit assembly `clips` list
  is `[{assetId, startMs, endMs}]` and nothing in the docs forbids repeating an
  `assetId`. With `audioMode: "song"`, a chorus take could play again over a
  later chorus whose timing is identical. **Unverified**; test it with a free
  assembly once there are two takes.
- **Imported video as a clip.** The contest allows imported outside source
  assets (Musializer renders, for example), but the server assembly docs only
  speak of persisted takes. Whether an uploaded `source_video` asset can sit in
  the cut without being regenerated is **unverified**. The paid path is the
  documented video-to-video remix (`prepare_source_video`), which bills like
  any scene. The browser's Legacy Studio timeline may allow more; the docs say
  "unsupported cuts remain browser edits".
- **Shorter film.** The contest rules set no minimum length. Slopcannon has
  its own song trimmer (`slop.cc/tools/song-trimmer`), which keeps the cut
  "made in Slopcannon".

## Sloptoberfest constraints

- Due 2026-10-29 23:59 ET (2026-10-30 04:59 CET). Winners are announced
  2026-10-31.
- All creation and editing must happen in Slopcannon or through its MCP.
  Outside source assets may be imported.
- The submission needs a public film link through the official form; tweets
  don't count. Multiple entries are allowed.
- Prizes: $38.1k in USDC on Solana, plus 960 generation minutes counted at
  $13.33 each. StealthGPT's "CAN'T BELIEVE IT'S SLOP!" award ($2.5k) is judged
  by a sponsor.

## Budget for the current candidate

"Hyper Memetic Fitness (Bamboo)" runs 202 s.

| Cut | Billed (est.) | Credits | USD | Leaves for rerolls |
| --- | --- | --- | --- | --- |
| Full song (202.6 s) | 202.6 s | ~1,621 | ~$45 | $50 pack: ~179 credits |
| ~90 s trim | ~90 s | ~720 | ~$20 | $30 pack: ~360 credits |
