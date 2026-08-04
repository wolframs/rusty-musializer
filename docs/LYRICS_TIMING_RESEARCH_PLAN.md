# Lyrics timing: global localization research and implementation plan

Date: 2026-08-04

Status: research/design proposal. This is not a second live repository queue.
If the operator accepts an implementation tranche, its tasks and acceptance
criteria must be copied into `FEATURE_PARITY_PLAN.md` before code changes begin.

No application, model, or playback process was run for this investigation. The
local findings below come from current source and existing artifacts under the
gitignored `build/analysis/` tree. The research section uses sources available on
2026-08-04. A read-only OpenRouter model-catalog query was made to verify the
current discovery shape and audio-model count; it did not submit audio or invoke
inference.

## Answer first

The user's diagnosis is correct: with authored metadata, word recognition is not
the product problem. The problem is locating a known ordered text sequence on the
song's time axis.

The current pipeline still makes Whisper the gatekeeper for localization. An
authored line must first match Whisper evidence (or fit a small interpolation
gap) before it is passed to the MMS acoustic aligner. An unmatched metadata line
is omitted before MMS sees it. A matched line is searched only near its Whisper
proposal. This architecture can refine a plausible proposal very well, but it
cannot recover a line that Whisper omitted, placed outside the search window, or
assigned to the wrong repeated occurrence.

Chunking can help, particularly by containing Whisper repetition loops and
recovering evidence near long instrumental gaps. But naive independent chunks are
not the complete solution: Whisper already works internally on roughly 30-second
windows, chunk boundaries can cut sung words, and repeated choruses remain
ambiguous. The useful form of chunking is an overlapping, multi-offset evidence
pass feeding a **global monotonic locator**, followed by block- and word-level
forced alignment.

The recommended target is therefore:

```text
authored lyrics (all lines retained)
             |
             v
whole-song candidate evidence
  overlapping Whisper + singing ASR + vocal activity
             |
             v
global monotonic line localization
  rare anchors -> ordered blocks -> repeated-phrase disambiguation
             |
             v
block-level and per-word acoustic refinement
  full mix + optional vocal stem, two aligners where useful
             |
             v
agreement/coverage checks -> confident cues or explicit review intervals
```

This also needs a provider-configuration foundation. The alignment benchmark
cannot become a maintainable feature if model selection remains split between
hard-coded Python constants, environment variables, and command-line flags. The
Assist panel should expose one clearly visible **AI settings** dialog where the
user can choose local and remote routes per task, inspect exactly what will leave
the machine, manage credentials, and see whether a selected route is currently
available. The configuration workstream is specified below; it is part of this
research plan because every experiment and accepted timing result must name the
exact provider, model, and policy that produced it.

## What happens today when Lyrics Assist is started

Selecting **Timed lyrics**, confirming, and pressing Start takes this path:

1. The Rust UI knows whether an explicit lyric sheet or sibling
   `<stem>.lyrics.txt` exists. It cannot see embedded tags itself, so the
   confirmation truthfully says Whisper will transcribe unless the audio carries
   a lyric tag.
2. `AssistController` starts `tools/external_analysis.py assist --mode lyrics`
   in the track's stable `build/analysis/<hash>/` workspace.
3. Measured analysis is generated or reused.
4. Whisper is generated or reused for the **complete track regardless of whether
   authored lyrics exist**. The current normalized artifact is
   `lyrics.whisper.json`.
5. Only after Whisper is available does `discover_reference_lyrics` choose, in
   order, an explicit sheet, sibling file, or embedded tag whose name contains
   `lyric`. `lyrics-eng` is preferred when present.
6. With a reference, `lyric_align.sync_lyrics` aligns normalized authored tokens
   to normalized Whisper words with a global ordered sequence match. It forms a
   provisional window for each sufficiently supported line. Missing lines are
   interpolated only between nearby trusted neighbors; otherwise they are stored
   in `unmatched` and omitted from `lines`.
7. `force_align_lyrics.py` receives only the provisional `lines`. It loads the
   full audio once, but makes one MMS request per cue over a slice beginning 0.75
   seconds before the provisional start and ending 6 seconds after the
   provisional end. Strong MMS evidence replaces the boundaries; weak evidence
   retains the Whisper-derived window as uncertain.
8. Rust validates the audio hash, duration, ordering, bounds, and mode authority,
   then stages an inert candidate. The project changes only after Apply.

The controlling sources are:

- `tools/external_analysis.py`: orchestration and reference discovery;
- `tools/lyric_align.py`: metadata-to-Whisper matching and omission policy;
- `tools/force_align_lyrics.py`: local-window MMS refinement;
- `crates/musializer-app/src/ui/panels/assist.rs`: process and staging boundary;
- `crates/musializer-core/src/project/analysis_candidate.rs`: inert application
  authority.

## Direct evidence of the remaining failure

The newest metadata-containing run is `Constellation Whale (Glitchpop).mp3` in
`build/analysis/cd0700a4a6109875/`.

- The embedded `lyrics-eng` tag contains 15 alignable lyric lines.
- `lyrics.sync.json` emits 13 cues and omits the final two authored lines:
  `Neither major, neither minor` and `Just... free`.
- `lyrics.whisper.json` enters a high-confidence repetition loop at 90.0 seconds,
  repeatedly emitting `We're the jazz that happens when we just appear` through
  the 114.84-second end of the track.
- The sync stage correctly flags 90.0–114.84 as unreliable evidence, but its
  conservative response is to omit the two outro lines.
- `lyrics.aligned.json` reports 13 input lines because MMS receives the sync
  output, not the complete metadata. It therefore cannot recover those lines,
  regardless of its acoustic ability.
- Conversely, MMS moves the first cue from the Whisper proposal
  2.01–13.34 seconds to 8.088–11.561 seconds. That six-second correction shows
  that the fine aligner is useful when the true occurrence happens to lie inside
  its allowed window.

This yields two separate failure classes:

1. **Coverage failure:** a metadata line absent from trusted Whisper evidence is
   removed before acoustic alignment.
2. **Search-space failure:** a retained line can be refined only inside a window
   derived from the evidence already suspected of being wrong.

The current acceptance tracks established that MMS often produces excellent
local boundaries. They did not prove that every authored line reaches MMS or
that the correct occurrence is inside every local window.

## Would explicit Whisper subsections help?

Yes, as a candidate-evidence rescue lane—not as the timing authority.

Whisper's encoder consumes 30-second contexts, and long-form transcription moves
through the recording using those contexts. Explicit independent subsections can
still change behavior in useful ways:

- a decoder repetition loop in one window does not condition the next window;
- a second chunk grid can expose a phrase that lay on a bad first-grid boundary;
- vocal-active chunks avoid asking an autoregressive text decoder to explain long
  accompaniment-only regions;
- absolute offsets make recovered word hypotheses useful as coarse anchors.

However, one fixed set of non-overlapping chunks introduces new errors:

- a boundary can split a consonant, syllable, or sustained word;
- overlapping chunks can emit duplicates with conflicting timestamps;
- independent chunks cannot decide which repeated chorus occurrence a line owns;
- prompting a chunk with the expected lyric can turn known text into an
  acoustically unsupported hallucination;
- sharper Whisper timestamps are still decoder timestamps, not evidence that a
  known transcript was globally aligned.

The benchmark should therefore test two independent grids—for example 30-second
windows with 8–10 seconds of overlap, plus a grid shifted by half a window. Each
window must decode without previous-window text conditioning. Results are merged
into a timestamped word lattice rather than directly into cues. Exact lengths
are experimental variables, not design constants.

## Current research landscape

### Practical forced-alignment architecture

[WhisperX](https://github.com/m-bain/whisperX) separates ASR from alignment: VAD
segments are transcribed without timestamps and a phoneme model aligns the
result. Its use of VAD and `condition_on_prev_text=False` is relevant to avoiding
long-form hallucination cascades. WhisperX's own maintainers caution that aligning
one transcript against very long audio can drift and become expensive, which is
why good coarse segments still matter.

[TorchAudio's forced-alignment tutorial](https://docs.pytorch.org/audio/stable/tutorials/forced_alignment_tutorial.html)
describes the underlying CTC trellis: frame-wise label probabilities are decoded
through one ordered transcript path. The important property for this project is
global monotonicity—later lyrics cannot silently jump to an earlier repeated
phrase.

The particularly relevant published design is
[Low Resource Audio-to-Lyrics Alignment From Polyphonic Music Recordings](https://arxiv.org/abs/2102.09202).
It first spots reliable anchoring words, segments the recording around those
anchors, and performs a second alignment pass inside the resulting blocks. The
paper also finds source separation important. That is almost exactly the missing
middle layer between the current whole-song Whisper evidence and per-cue MMS.

### New local model candidates

[Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR) is now an unusually strong
benchmark candidate because the official release explicitly supports singing
voice and songs with background music. Its companion
`Qwen3-ForcedAligner-0.6B` accepts an audio/transcript pair, predicts token
timestamps non-autoregressively, supports English and ten other languages, and
accepts up to five minutes in one pass. The official table labels the aligner
itself as a speech model, so song accuracy must be measured rather than assumed.
Still, it can test the central hypothesis directly: give the complete song and
complete embedded lyrics to a model that does not need Whisper locations.

The [Qwen3-ASR technical report](https://arxiv.org/abs/2601.21337) describes the
aligner as timestamp-slot prediction rather than CTC. This makes it valuable as
an independent error mode, not merely a faster copy of MMS.

Three music-oriented systems show where the field is going:

- [STARS](https://arxiv.org/abs/2507.06670) jointly models phoneme alignment,
  notes, technique, and style with hierarchical acoustic processing.
- [SongTrans](https://arxiv.org/abs/2409.14619) jointly predicts lyrics, word
  durations, and notes directly from songs with accompaniment.
- [VocalParse](https://arxiv.org/abs/2605.04613) provides released code and a
  checkpoint for structured singing transcription with lyric/note alignment.

These are candidates for coarse localization or future replacement experiments,
not immediate production dependencies. STARS is not presented as a convenient
released inference package; SongTrans does not expose a clearly maintained
public implementation; VocalParse solves structured transcription rather than
known-text forced alignment. A good paper result is not yet a supportable local
product path.

### Community practice

Current open karaoke tools converge on several pragmatic choices:

- [syncalong](https://syncalong.readthedocs.io/en/latest/) maps Whisper words to
  known lyrics with an order-preserving dynamic program so repeated choruses do
  not collapse onto one occurrence.
- [Nightingale](https://nightingale.cafe/docs/lyrics) isolates vocals, exposes
  WhisperX and TorchAudio CTC backends, and now offers Qwen3-ForcedAligner as an
  experimental whole-transcript option with explicit fallback.
- [stable-ts](https://github.com/jianfch/stable-ts) exposes VAD and Demucs as
  alignment aids for music.
- Recent karaoke makers similarly combine known lyrics, vocal separation,
  forced alignment, and an editor rather than claiming ASR timestamps are final.

The community lesson is architectural agreement, not proof of any one tool's
accuracy: preserve known text, isolate vocals where useful, align rather than
transcribe, retain fallbacks, and make residual errors cheap to correct.

### Evaluation data

The [MIREX lyrics task](https://music-ir.org/mirex/wiki/2025%3ALyrics_Transcription)
distinguishes monophonic vocals from full polyphonic mixtures and points to DALI
as a large synchronized music/lyrics dataset. A newer
[word-aligned MUSDB18 test set](https://zenodo.org/records/15547046) contains 45
professionally produced English songs with manually corrected word timestamps.
These can inform external benchmarking, subject to their licenses and audio
availability, but the product gate must still include this user's difficult
tracks and exact metadata.

## Proposed architecture

### Invariant 1: metadata lines survive localization failure

Every alignable authored line must reach the acoustic-localization stage. An
unmatched ASR line becomes “location unknown,” not “line absent.” Metadata may
contain performance directions, headings, or genuinely unsung alternate text,
so this does not mean forcing every string into a fake cue. It means the decision
to omit authored text requires acoustic/global evidence or explicit user review,
not absence from Whisper's transcription.

### Invariant 2: solve song position globally before cue boundaries locally

Build a candidate score lattice for each authored line over time, then find the
best ordered path across the complete lyric sequence. Candidate scores may
combine:

- rare exact or fuzzy ASR n-gram matches;
- MMS/Qwen alignment likelihood over coarse windows;
- vocal activity and separated-vocal energy;
- agreement across full mix and vocal stem;
- agreement across overlapping chunk grids;
- expected but softly constrained line duration and intervening silence;
- neighboring line support.

The path must allow an explicit “unsung/unresolved” state. It must not invent a
timestamp merely to achieve full coverage.

### Invariant 3: use reliable anchors to define blocks

Rare multi-word matches with cross-pass agreement become anchors. Consecutive
anchors partition the song and lyrics into blocks. Within a block, align all
intervening lines together so repeated words are disambiguated by their ordered
neighbors. Terminal and initial unanchored blocks remain first-class cases; they
must not be silently dropped because interpolation has only one neighbor.

This is the essential improvement over both extremes:

- whole-song CTC can be memory-heavy and drift;
- isolated per-cue alignment cannot escape a bad cue proposal.

### Invariant 4: refine and verify with independent views

After a block is located, obtain word boundaries on the full mix and, when the
first pass is weak or contradictory, a cached Demucs vocal stem. Run at least one
fine aligner; during evaluation run both MMS and Qwen3-ForcedAligner where their
language support applies.

A cue is confident when its line coverage, order, boundary evidence, and
independent views agree within measured tolerances. A cue is uncertain when they
do not. “Confident” must not be derived from one model's own score alone.

### Invariant 5: retain a fast human finish

Automatic near-certainty on arbitrary expressive singing is not a defensible
promise. Near-certainty for the product can come from automatic alignment plus
honest abstention and a fast review loop. The existing UX plan already calls for
seek-on-selection, playhead start/end stamping, nudges, undo, split/merge, and
auto-advance. The timing engine should emit the exact blocks and boundaries that
need that review instead of making the user hunt through the whole song.

## Assistance provider configuration workstream

Provider configuration is not merely an OpenRouter key field. It is the control
plane for every Assist data boundary and model choice. The current implementation
already has the beginnings of multiple routes, but no coherent user model:

- Whisper and MMS/Qwen-style aligners are local processes;
- Codex is an installed external tool, and `external_analysis.py` already accepts
  a `--codex-model` override;
- the OpenRouter semantic helper currently fixes its model to
  `xiaomi/mimo-v2.5`;
- the remote key comes from `OPENROUTER_API_KEY`, with a repository `.env`
  fallback in the desktop orchestration path;
- `Full assist` composes these mechanisms without a persistent, inspectable route
  profile.

That is enough for a developer-operated prototype, but not for a user-facing
feature. The target is a versioned task router, capability catalogs, secure
credential references, and a comfortable configuration dialog.

### Visible settings entry point and dialog

Add a persistent, text-labelled **AI settings** button with a sliders or gear
icon at the right side of the `ASSISTED ANALYSIS` heading row. It must remain
visible in Ready, Confirmation, Running, Candidate, Empty, and Failed states; it
must not be hidden behind one particular workflow card or below the panel fold.
At narrow widths it may occupy its own header row, but it keeps its label rather
than collapsing to an unexplained icon.

The button opens an application-modal, scrollable settings dialog inside the
Musializer window. A spacious modal surface is preferable to squeezing controls
into the shallow bottom panel, and it is more testable and portable than a
second native/raylib top-level window. It must have normal keyboard traversal,
an obvious close action, Escape handling, focus containment, masked secret
inputs, and usable layouts at the existing UI scale and window-size gates.
Opening it does not start an analysis job.

The dialog has five sections:

1. **Routing** — an overview matrix with one row per Assist task and columns for
   execution boundary, provider/runtime, model, fallback, and readiness.
2. **Local models** — installed Whisper, forced-alignment, singing-ASR, and stem
   separation runtimes; model paths/identities, language support, GPU readiness,
   and doctor output.
3. **Codex** — executable/auth readiness plus a model and reasoning-effort choice
   for each Codex-eligible text/reasoning task.
4. **OpenRouter** — connection state, key management, live/cached model catalog,
   endpoint/privacy constraints, cost metadata, and manual Refresh.
5. **Privacy and diagnostics** — remote-audio policy, ZDR/provider restrictions,
   last catalog refresh, configuration provenance, and a dry-run route summary.

The first view should offer a **Recommended / automatic** profile, not require a
new user to understand every backend. Advanced controls expose per-task routing.
Every dropdown is filtered by the task contract; the UI must not offer a text-only
Codex model as a local acoustic aligner merely because both are called models.

### Route by task, not by workflow button

The four workflow buttons are compositions of smaller tasks. Configuration must
address those tasks independently:

| Task contract | Typical eligible routes | Input boundary |
| --- | --- | --- |
| measured audio features | built-in deterministic analyzer | local only |
| coarse lyric evidence/localization | local Whisper, evaluated singing ASR, experimental approved remote audio model | audio; remote only with confirmation |
| known-text forced alignment | local MMS/Qwen-style aligner | local audio + authored lyrics |
| lyric wording/review when no authored text exists | installed Codex model over bounded Whisper JSON | text evidence, not audio |
| semantic/feeling analysis | approved OpenRouter audio model | complete or explicitly shown audio excerpts |
| scene-plan reasoning/review | deterministic planner or installed Codex model over measured artifacts | bounded text/JSON evidence |
| independent timing verification | evaluated local aligner or approved remote audio model | explicit review excerpts, never silently the whole song |

This decomposition lets `Timed lyrics`, `Scene changes`, `MiMo feelings`, and
`Full assist` resolve to a route graph. Before Start, the existing confirmation
must show the resolved graph: exact models, which inputs remain local, what leaves
the machine, remote provider restrictions, and whether a fallback could change a
data boundary.

Route fallback is explicit policy, not generic retry behavior. A local failure
must never fall through to a remote audio provider without another user decision.
Useful choices are `do not fall back`, `ask`, `local alternatives only`, and
`same-boundary alternatives only`. A running job snapshots its resolved routes;
editing settings affects the next job and cannot change a job in flight.

### Codex discovery and model selection

Codex remains a required installation for now, but Musializer must not hard-code
the Codex models available to a particular account or installation. Current
[Codex app-server documentation](https://developers.openai.com/codex/app-server/)
defines `model/list`, including picker visibility, supported reasoning efforts,
default effort, upgrade information, and input modalities. Use that supported
discovery surface when available and cache only non-secret catalog metadata.

If the installed Codex is too old to provide model discovery, the safe fallback
is **Codex default** plus an advanced manually entered model id that is validated
by a bounded dry run. Do not scrape Codex authentication files or copy its tokens.
The existing `codex exec --model` interface remains the execution boundary.
Musializer should show the model that actually ran in candidate provenance, not
infer it from the current settings after the fact.

Codex eligibility remains contract-driven. Today its lyrics role consumes
Whisper JSON for wording review when no authored lyrics exist; it is not an audio
timestamping engine. A future Codex catalog entry advertising an audio input
modality would still require a Musializer timing benchmark before it appears as
an acoustic-localization recommendation.

### OpenRouter catalog, suitability, and provider routing

OpenRouter's current [`GET /api/v1/models`](https://openrouter.ai/docs/guides/overview/models)
catalog can filter on input and output modalities and exposes model identity,
architecture modalities, context length, supported parameters, and pricing.
On 2026-08-04 an unauthenticated query filtered to audio input and text output
returned 25 entries. That count is a volatile observation, not a product
constant. Each model record links to its model-specific
`/models/:author/:slug/endpoints` data, while the provider-routing API exposes
restrictions such as `order`, `only`, `ignore`, `allow_fallbacks`, price bounds,
and `zdr` through its
[provider-selection contract](https://openrouter.ai/docs/guides/routing/provider-selection).

The dialog should fetch the filtered catalog on explicit Refresh and may refresh
stale data when the OpenRouter section is opened if the user has enabled catalog
network access. Fetching a public catalog still discloses an IP address and must
not be disguised as an offline action. Cache the sanitized response under the
per-user XDG cache directory with:

- a Musializer cache schema version and source URL;
- fetch and successful-validation timestamps;
- the exact filters used;
- bounded, normalized model/endpoint fields only;
- a maximum response size and model count;
- atomic replacement, retaining the last valid cache after a failed refresh.

The UI uses stale-while-offline behavior: show the last valid list, its age, and
an explicit stale/offline badge. It must not invent a model, price, or privacy
property when fields are absent. Catalog strings are untrusted display data and
must never become paths or shell fragments.

Modality is only the first eligibility filter. `audio -> text` means that an API
accepts audio; it does not establish singing training, timestamp resolution, or
lyrics-alignment accuracy. Maintain a small versioned Musializer suitability
overlay keyed by model id and task contract:

- `recommended` only after passing the relevant track benchmark;
- `experimental` for transport-compatible, unevaluated models;
- `unsupported` for known contract failures;
- evidence date, tested prompt/schema version, audio scope, languages, and known
  limitations.

Default pickers show recommended models. A **Show experimental audio models**
control exposes the rest with a clear warning. Suitability records should be
refreshable in application releases; they must not be inferred from model names
or marketing descriptions.

Provider choice is distinct from model choice. OpenRouter can route one model to
multiple endpoints. Let advanced users pin or allow providers and configure
fallback, price, and privacy constraints. For remote audio, default to ZDR-only
where available and surface when that leaves no eligible endpoint. OpenRouter's
[ZDR documentation](https://openrouter.ai/docs/guides/features/zdr) states that
per-request `provider.zdr: true` restricts routing to ZDR endpoints; a missing
eligible endpoint must block rather than weaken the policy silently.

### Credential storage and process boundaries

Do not persist API keys in `.musi` projects, ordinary preference JSON, analysis
artifacts, model catalogs, logs, support bundles, command arguments, or a
repository `.env`. The current repository-`.env` fallback is acceptable only as
a legacy developer/CLI compatibility path during migration; the desktop settings
flow must not write or depend on it.

On Linux, store persistent provider secrets through the
[freedesktop Secret Service API](https://specifications.freedesktop.org/secret-service/latest-single/),
which was designed jointly for GNOME and KDE secret stores. Non-secret settings
store only an opaque lookup identity such as provider/account name; the Secret
Service specification recommends lookup attributes rather than persisting a
service object path. The implementation tranche must validate the chosen client
mechanism and packaging on the supported Kubuntu environment before selecting a
Rust binding or a narrowly supervised system helper.

If no usable secret service is available, offer **session only** storage and
read-only import from `OPENROUTER_API_KEY` for the current process. Do not silently
fall back to a plaintext file. The dialog supports Replace, Forget, and Test;
Test uses OpenRouter's read-only, non-inference
[`GET /api/v1/key`](https://openrouter.ai/docs/api-reference/overview)
endpoint rather than spending credits on a completion. Display only the
provider-returned label or a short local fingerprint—never the full value.

Retrieve a secret only when resolving an authorized remote job, pass it through
the child's environment rather than argv, and remove credential-like variables
from every unrelated child as the helper already does. Secret lifetime in Rust
and Python is necessarily best-effort; the defensible guarantee is minimized
scope and no durable plaintext copies, not a claim that managed runtimes can
erase every memory copy.

### Non-secret preferences and provenance

Persist versioned, atomically replaced per-user preferences under the normal XDG
configuration location. This is user configuration, not project content. A
conceptual record is:

```text
assist settings v1
  profiles
    recommended
    user overrides by task contract
  routes
    backend/runtime id
    model id and reasoning effort
    provider allow/order/fallback/privacy/price constraints
  local runtime preferences
  catalog refresh policy and last selected filters
  credential lookup identity (never the secret)
```

Each analysis manifest and candidate records an immutable execution snapshot:
profile id, task routes, actual model ids, local model hashes where practical,
provider constraints, prompt/schema versions, catalog/suitability revision, and
the data boundary applied. It must not record the credential identity if that
would expose an account label unnecessarily. Cache acceptance must compare the
task-relevant route identity so changing an alignment model cannot silently reuse
an artifact from another model.

### Provider-settings delivery phases

This work can proceed alongside model evaluation without deciding which timing
model wins:

1. **P0 — contracts and threat model:** enumerate task contracts, remote payloads,
   configuration fields, secret exposures, and fallback boundary rules.
2. **P1 — persistence foundation:** versioned non-secret preferences, Secret
   Service abstraction, session-only fallback, migration away from repository
   `.env`, and redaction tests.
3. **P2 — discovery:** local doctor/runtime inventory, Codex `model/list`, bounded
   OpenRouter catalog and endpoint cache, suitability overlay, offline/stale UX.
4. **P3 — dialog:** persistent header button, modal navigation, routing matrix,
   provider editors, key lifecycle, connection tests, and accessible narrow/scale
   layouts.
5. **P4 — execution:** resolve and snapshot routes, inject only the required
   secret, wire exact model/provider flags, update confirmations and provenance,
   and prevent cross-boundary fallback.
6. **P5 — evidence:** headless dialog captures, pure configuration/router tests,
   malformed-catalog tests, secret-leak scans, and end-to-end dry-run manifests.

Provider-settings acceptance must include these negative controls:

- a local route failure cannot cause an unconfirmed remote request;
- a model losing its required modality invalidates the route and preserves the
  last valid catalog;
- malformed, oversized, duplicated, or partially written catalogs are refused;
- offline startup renders cached data with an honest age and no network hang;
- an invalid, revoked, locked, or absent key produces an actionable state;
- ZDR/provider constraints that match no endpoint block the request;
- changing settings while a job runs does not change that job's snapshot;
- Codex discovery failure preserves `Codex default` rather than substituting a
  guessed model;
- secret canaries do not appear in preferences, projects, caches, manifests,
  diagnostics, logs, crash/support bundles, clipboard, or process arguments;
- the dialog remains operable by keyboard and at every captured UI scale;
- deleting a saved key removes the Secret Service item while leaving unrelated
  provider entries untouched.

As with the lyrics algorithm, this section is design and research authority, not
a second completion queue. Once accepted, P0-P5 must be split into bounded items
in `FEATURE_PARITY_PLAN.md` with ownership and dependency order.

## Experiment plan

All experiments are offline artifact generation. They must not initialize an
audio output device. Source MP3s remain read-only; stems, models, and results live
under gitignored `build/lyrics-research-v2/`.

### Phase 0 — freeze a ground-truthable baseline

1. Preserve current outputs for the four earlier acceptance paths.
2. Add `Constellation Whale` as the coverage canary, with the 90-second
   repetition loop and two omitted outro lines explicitly recorded.
3. Select at least two more difficult metadata tracks: one with repeated chorus
   text and one with quiet/dense vocals.
4. Manually adjudicate line starts and ends to a practical UI tolerance using
   waveform, repeated listening, and slow playback. Store only derived timing
   annotations in the research tree unless the operator chooses to commit a
   synthetic/public fixture.
5. Record the resolved task-route snapshot for every run. A result without exact
   local model identity or remote provider/model/policy is not reproducible.

Baseline metrics:

- authored-line coverage;
- resolved, uncertain, and incorrectly omitted line counts;
- median, P90, P95, and maximum absolute start/end error;
- catastrophic error count above 1.0 and 3.0 seconds;
- repeated-occurrence errors;
- order/overlap violations;
- runtime and peak VRAM.

### Phase 1 — isolate the coarse-localization question

Run the following without changing production behavior:

| Variant | Purpose |
| --- | --- |
| Current Whisper -> sync | Baseline |
| Two-grid overlapping independent Whisper | Test whether loop containment and boundary diversity recover missing anchors |
| Qwen3-ASR on full mix | Test a model officially intended for singing/song recognition |
| Qwen3-ASR on Demucs vocals | Separate accompaniment error from singing-domain error |
| Full-song Qwen3-ForcedAligner with complete metadata | Test localization without Whisper proposals |
| Anchor spotting -> block MMS on full mix | Test the published hierarchical architecture with the installed model |
| Anchor spotting -> block MMS on vocals | Test source separation only where it changes the answer |

Do not tune on one track. Choose thresholds on a development subset and report
the untouched tracks separately.

### Phase 2 — build a global candidate lattice prototype

The prototype consumes artifacts from Phase 1 and emits, for every authored
line:

- all plausible time intervals and their source;
- lexical/acoustic/view-agreement scores;
- selected global path or unresolved state;
- anchor identities and block boundaries;
- competing repeated occurrences;
- reasons for confidence or abstention.

Use a monotonic dynamic program or Viterbi-style search. Include tests where the
locally highest-scoring repeated phrase is globally wrong but the complete
ordered path is right.

### Phase 3 — refine blocks and compare aligners

For each selected block:

1. align the complete consecutive text block, not independent lines;
2. derive word spans and line envelopes;
3. optionally refine boundary words in smaller windows;
4. compare full mix and vocal stem;
5. compare MMS and Qwen on supported English tracks;
6. retain competing evidence in the audit artifact.

The production choice may remain MMS if it wins. The goal is not to replace a
working model for novelty; it is to remove the wrong search-space dependency.

### Phase 4 — adversarial controls

The acceptance harness must demonstrate failures under these perturbations:

- shift every Whisper proposal by +15 seconds: global localization should still
  recover authored lines;
- delete one line from the ASR evidence: metadata coverage must survive;
- repeat a chorus hypothesis through the outro: terminal authored lyrics must
  remain searchable;
- cut a sung boundary at the center of one chunk grid: the shifted grid should
  rescue it;
- make two identical lyric lines candidates for one acoustic span: global order
  must select distinct occurrences or abstain;
- substitute a wrong but acoustically plausible metadata line: the system must
  mark it unresolved rather than squeeze it into a gap;
- degrade or remove the vocal stem: full-mix evidence must remain a supported
  fallback;
- perturb one model's boundary by more than the agreement tolerance: confidence
  must fall.

### Phase 5 — production integration, only after selection

Once the benchmark identifies a winner:

1. version the new localization and alignment policy in cache provenance;
2. preserve every prior artifact needed to explain a decision;
3. keep model/runtime discovery in the support bundle and doctor;
4. stage results through the existing bounded bridge and `AnalysisCandidate`;
5. surface unresolved lines and disagreement intervals in the Assist review;
6. add the chosen work as a tranche in `FEATURE_PARITY_PLAN.md`;
7. rerun the metadata and stripped-tag acceptance set plus the new coverage
   canary before changing the default;
8. expose the chosen route through the provider dialog and make its benchmarked
   suitability state visible rather than silently hard-coding it.

## Decision gates

Adopt chunked Whisper only if it improves authored-line coverage without raising
catastrophic repeated-occurrence errors.

Adopt Qwen3-ForcedAligner only if whole-song metadata alignment works on actual
songs with accompaniment and its errors are independently detectable. Its
speech-trained label is a real risk despite its attractive interface.

Make Demucs mandatory only if the benchmark shows a material and broad timing
gain that justifies its runtime and distribution cost. Otherwise generate a
vocal stem on demand for weak/contradictory blocks and keep it as an evidence
lane.

Replace MMS only if another local model wins boundary accuracy, coverage,
runtime, license, installation, and failure transparency together. Removing the
Whisper-window dependency is higher priority than changing the fine aligner.

Expose an OpenRouter model as recommended for a task only after that exact
model/prompt/schema combination passes the relevant benchmark. Audio modality
alone is an eligibility signal, not evidence of timing quality.

Ship persistent remote credentials only after the Secret Service, session-only
fallback, redaction scan, Forget flow, and locked-wallet behavior pass. A
plaintext preferences or repository-`.env` fallback is not an acceptable way to
make the dialog appear complete.

## Recommended first move

The fastest high-information experiment is not implementing custom chunking. It
is a three-way offline benchmark on `Constellation Whale` and the existing
acceptance tracks:

1. pass the complete metadata plus complete audio directly to
   Qwen3-ForcedAligner;
2. run two-grid independent overlapping Whisper and see whether it recovers the
   terminal lines;
3. run anchor-to-anchor block alignment with the already installed MMS model,
   on full mix and the cached/generated vocal stem.

That experiment directly answers whether the missing layer should be a better
whole-song aligner, better coarse evidence, or hierarchical search. Only then
should the production pipeline or dependency bundle change.
