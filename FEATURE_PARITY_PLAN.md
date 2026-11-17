# Rusty Musializer — the live queue

**This file carries only what is open.** It kept the name it was born with, but
it stopped being a parity plan a long time ago: the C was declared legacy on
2026-08-03 and the application passed its ceiling in the waves that followed.
Nothing here needs a parity justification, and "the C does it this way" is not an
argument for or against any item below.

If another document, source comment, old agent handoff or ignored worktree names
unfinished work, reconcile it here before acting on it. There is one queue.

Open now: HY housekeeping, LT2 acoustic acceptance, WX-6 appearance coverage,
and the ordered remainders below. Built work awaiting a human look is separate.
History and operator wording: [FEATURE_PARITY_HISTORY](docs/archive/FEATURE_PARITY_HISTORY.md)
(search by id; an id cited by source but absent here is closed).

## Built, waiting for the operator's look

- **Glyph Lattice** — since 2026-10-24; Scenes → Geometry → Glyph Lattice with music, then decide whether `words` defaults on; [design](docs/GLYPH_LATTICE.md).
- **Pixel Cats** — since 2026-10-24; Scenes → Pixel cats, play with lyrics; [six stagings](docs/PIXEL_CATS.md). Unbuilt stagings remain below.
- **Scene families and native gallery** — since 2026-10-23; Scenes flyout, compare the six new scenes and their Look controls while playing; [guide](docs/SCENE_GALLERY.md).
- **Fluid Ink** — since 2026-10-23; Scenes → Fluid Ink, play and open Look; [design and reproduction](docs/FLUID_INK.md).
- **SX4 Tideline / Cadence** — since 2026-09-05, the priority then; now waiting with these items. Use the fresh SX4 listening protocols in the Rust runner and the browser's cut-quality question; [design](docs/SCENE_DESIGN.md), [runner instructions](tools/listening-lab/README.md).
- **CX-5 thumbnails** — awaiting review (start date not recorded in the queue; scene-picker integration documented 2026-10-30); open Scenes on a track; [poster service](docs/POSTER_FRAMES.md).
- **D6 Inspector scrolling** — since 2026-09-05; scroll Look to the last setting on a tall-control scene; [reachability and visual boundary](docs/SCENE_DESIGN.md).
- **MV7 Cadence second pass** — landed 2026-10-12, wants the operator's eye before more is added; Scenes → Cadence with lyrics, then Look; [design](docs/SCENE_DESIGN.md).

Full descriptions and verification boundaries: archive, "HY-2 — live queue
consolidation (2026-11-17)". Waiting is not aesthetic acceptance.

Deliberate exclusions: [AGENTS.md — Not built, and not going to be](AGENTS.md#not-built-and-not-going-to-be).
What changes only on purpose: [AGENTS.md — Judge by what a user or a file can observe](AGENTS.md#judge-by-what-a-user-or-a-file-can-observe).

## Open work

**HY — housekeeping from the 2026-11-17 audit** (claimed by the lead session of
that day). The operator brought an earlier session's audit and asked for "the
obvious ones" to be batched to GPT-6.1-Sol workers and polished by the lead.
Each batch runs in its own worktree and is cherry-picked; tick here as each one
lands, and move this item to the archive when all have.

- [ ] HY-1 docs and references: current-state mentions of `mimo_openrouter.py`,
  the C-era `ui_fixture.sh`/`ui_capture.sh` rows in `docs/PHASE0_INVENTORY.md`,
  `.gitignore`'s raylib 5.5 and `UI_REVIEW.md` lines, the project schema's v1
  name and missing `clips`, the finished `LYRICS_*`/`MIMO_*` investigations
  to `docs/archive/`, the stale "Agent G note" comments.
- [x] HY-2 this file (landed 2026-11-17, 679 → 519 lines; record in the archive): one telling of the Gemini decision, the stale MiMo
  standing fact, pointers instead of copies of `AGENTS.md`, LT2 condensed, the
  waiting-for-review items in one place.
- [ ] HY-3 credentials: AX-7 and AX-8 below, and the CLI's repository-`.env`
  fallback replaced by the 0600 credential file `AGENTS.md` names.
- [ ] HY-4 the CX-4 −100 ms tap-offset default; chat drafts persist across
  restarts (the lead's choice: a draft lost to a restart is work lost).
- [x] HY-5 (landed 2026-11-17: dark-theme and UI/UX retired as covered, welcome repaired and in verify; record in the archive) `tools/`: the three gates nothing runs (dark theme, UI/UX, welcome
  theme) run, then fixed or retired; the near-twin helper names.
- [ ] HY-6 the "OpenAI Knot" scene restyled and renamed. Operator, 2026-11-17,
  verbatim: "we have 'Clawd,' and I figured it would be fair to GPT models and
  fans to have openai in there too somehow, but we can perhaps stylize the
  scene and 3d model in it a bit more and name it something more universal."
  Old CLI names and saved projects keep working.
- [x] HY-7 the lead, 2026-11-17. Dropped the Aug 7 `px3-tmp` stash (a WIP of
  PX3, which landed differently). Deleted the 2026-11-10 branch bundle: every
  commit in it is in master by subject except a "DROP AT MERGE" seam commit and
  a regenerable code-map refresh. In `build/` (16 GB → 13 GB): deleted the 258
  entries older than 2026-11-10 that no doc, tool or memory names (run logs,
  probe and review captures), and in `build/door/` the compositor
  intermediates `base_v1–3.mp4` and `door_no_room_v2.mp4` (superseded by v3
  and by the native render). Kept, with the reason: `build/wave-lead/` (its
  own note: delete after 2026-11-28), `build/consult/` (the CX consultation
  answers the archive cites), `build/lyrics-recording-2026-09-08/` (hosted
  answers, LT2 evidence), `build/lyrics-research-v2/` (the research venv, until
  LT2 closes), `build/protocols/` (CX-4's blind protocols), everything a doc or
  gate names, and everything newer than 2026-11-10. The repository `.env`
  stays: kubuntu's project `.env` files move to the fleet key store as one
  step of that plan (move 9, see the `fleet-secrets` skill), not one by one;
  after HY-3 nothing in this repository reads it.
- Not in this round, by the lead's choice: splitting `main.rs`'s `run()`,
  regrouping `tools/` into folders, and the rules-layer items (the trap list's
  length, "cite the C file and line", the one-file-per-agent rules), which go
  to the operator as proposals. The public GitHub mirror is published through
  the wsiener.dev mirror procedure, not from here.

**Standing decisions from the 2026-11-14 follow-up.** The listening lane's
choice and open remainder are [below](#the-listening-lane-and-the-benchmarks-remainder).
The lead's `--max-spend N` rule bounds only a treatment run's new sends;
prior possible charges are reported separately (2026-11-15, his to overrule;
`docs/VIDEO_GENERATION.md`, "The ceiling, redefined"). Balanced stays as it is;
High is the answer for gradients and grain (his to revisit;
[measurement](docs/archive/BALANCED_ENCODER_2026-11.md)). Operator wording and
decisions are in the archive's "The 2026-11-14 follow-up" and
"The listening lane moves to Gemini 3.8 Flash" sections.

## UI toolkit follow-through

The right-column workspace replaces the duplicate modal; shared egui styling,
Direct and the welcome picker remain. See [UI_TOOLKIT](docs/UI_TOOLKIT.md).
Remaining: persist unfinished toolkit chat drafts across restarts (HY-4;
the lead's 2026-11-17 choice, recorded in HY above).

**WX-6 appearance coverage — still open (2026-10-30).** The EX4-c CPU
failure is diagnosed and fixed: it predates WX-5d (5/5
quiet failures on both `3ec989e` and `be507e1`), and saturation occurs before
the first drain, not during the injected stall. Five quiet poster gates passed;
no poster change was needed. See `docs/HEADLESS_GPU.md` for evidence and the
CPU counter contract. Real-song integration captures also report 13–14 lifetime
analyzer-tap rejections with zero output underruns; their epoch is not localized.
Investigate separately before claiming lossless playback of that fixture.

The paired-look/open-column prototype exceeded the operator's +1/3 GPU runtime
cap: **553.38 s baseline, 849.83 s pilot (+53.57%)**, both quiet. Its pixel
changes were reverted and remain unmerged; `headless_check.sh` and
`lyric_lane_capture.sh` still need the appearance conversion. Replay only
pixel-bearing captures, resolve the classifier/targeting failures in the
headless report, then run both-renderer negative controls. The default 49 px
lyric lane cannot grow past the fixed band's slack. Integration reproduced a
working shrink to 30 px (the actual minimum request is 22). Growing the band
through this grip remains a layout decision, separate from appearance coverage.

**WX-5 follow-ups.** The remaining layout decision is whether the lyric lane's
resize grip should grow the whole band. WX-6 appearance coverage is tracked
above; the rest is in [the workstation guide](docs/WORKSTATION.md#open-work).

**LT2 — lyrics Assist repair.** Goal: audit the complete production lyrics
path on the ten-track corpus in [the investigation](docs/LYRICS_ASSIST_REPAIR_2026-09-06.md),
counting missing/extra performed phrases and boundaries beyond ±0.5 s as errors,
with a ≤3% per-track target (lead and echo count separately; a continuous chop
run counts once). Latest recorded status (2026-09-08): the local
crop repair renders 27 captions instead of 13 on the recorded failure, but
missing choruses and backing-vocal boundaries remain. Ten corpus runs and the
v6 comparison are complete; reference disagreements are not adjudicated error
rates. Detection/timing and whole-track acceptance remain open.
Next: resume the normal local Whisper/MMS investigation from
[the recording](docs/archive/LYRICS_RECORDING_2026-09-08.md), adjudicate wording/
grouping conflicts against independent audio evidence, and verify staged/apply/
render behavior. Try section-bounded CTC with a repetition cross-check, a
single-track lead rather than an accepted result. Normal Assist stays local;
the later approved Antigravity audio route is opt-in. Preserve SX4 work.
[Grouping handoff](docs/archive/LYRICS_GROUPING_REVIEW.md); full status,
operator decisions and measurement: archive, "HY-2", the two LT2 entries.

**Pixel Cats remainder.** Unbuilt stagings: progress bar, pause menu, handheld,
music box ([design](docs/PIXEL_CATS.md)); built scenes await the look above.

**SX4 remainder.** The September 5 priority is now a waiting item above.
Candidate audition cuts remain unverified phrase boundaries; revisit cue-to-cue
type sizing or remaining motion defects if the listening pass finds them.
The Tideline motion correction has only nonvisual verification by request;
earlier renders establish neither its acceptance nor its performance.
[Design and protocols](docs/SCENE_DESIGN.md).

Ordered as a session should pick it up. Trust outranks delight — that ordering is
a CX ruling, not a preference.

| # | Work | Ids | Size |
| --- | --- | --- | --- |
| 1 | **Review SX4, then revisit CX-4.** Track A's original session was rejected; track B is unreviewed. The new design review has four questions per track and a separate cut-quality question in the browser. Original CX-4 files/answers remain intact; do not unblind until both original sessions are complete. Tap-offset and 80/120/200 ms flash comparisons also remain unvalidated | SX4, CX-4 | operator listening |
| 2 | **Listening-lane plan quality and benchmark remainder** — [details below](#the-listening-lane-and-the-benchmarks-remainder) | — | operator listening, then one session |
| 3 | **The remaining CX rulings, in their stated order.** Review the implemented track-specific CX-5 scene thumbnails, then CX-1 marker rails and the proposal review queue (PXF-8), then CX-2 export variants (PXF-4 first, schema v2 — PXF-9) | CX-1, CX-2, CX-5 | large; four or five agents |
| 4 | **Workflow friction and product opportunities** left from the user-perspective review | UX0-B06, B07, B10–B19; UX0-C05, C08, C09, C17; PXF-3, PXF-12, PXF-13 | medium |
| 5 | **Durable-edit remainder, then the missing product surfaces, then honesty and gates** | B3; C2, C3, C5; D6, D8; E2, E3, E4; F1/F2/F3 remainder; G1, G2; GX-3, GX-4, GX-5 | large |
| 6 | **Dev-ex and infrastructure** (DX8 closed 2026-08-29 — E2/E4 unblocked) | DX1–DX7, DX9 | small each |
| 7 | **Assist provider remainder** | AP5-b/c/d, AP6-e | small each; AP5-c is a feature |
| 8 | **Scene follow-ups**, each of which wants a listening pass rather than more code | SX2, SX3 open notes | small |
| 9 | **What directing a video from the command line ran into** (MV1 landed 2026-10-11, MV2 2026-11-13; guide in `docs/MUSIC_VIDEO_CLI.md`) | MV2 remainder, MV3–MV8 | small each |
| 10 | **The look cards' remainder** — MV9-l and MV3-a landed 2026-11-14 as Look's look cards; what they left is listed under MV9 | MV9-l remainder | small each, UI |
| 11 | **The window as a directing workstation** — a dock with a real terminal and native Claude Code / Codex / Antigravity sessions, the window following a treatment live, and the restyle the mockup proposes. Operator request 2026-10-21; WX-0 to WX-5d landed by 2026-10-30, the treatment picker 2026-11-13; WX-6 appearance coverage and the lyric-lane grip decision remain open in [`docs/WORKSTATION.md`](docs/WORKSTATION.md) | WX-6 | gate runtime cap |
| 12 | **Generated video as material** — clips as material (F1) and generating a clip through OpenRouter (F2, H4) landed 2026-11-14 to 2026-11-15; record in [`docs/VIDEO_GENERATION.md`](docs/VIDEO_GENERATION.md). What both halves left, and what was never tried for real, is under VG below | VG remainder | small each |

### The items, in one line each

Full context for every id is in the archive under the section named by its
prefix.

**CX — consultation rulings.**

- **CX-1** — replace the event markers' authorship encoding: **top rail vs bottom
  rail**, not disc vs ring. A legend explains an encoding the eye still cannot
  resolve and spends band height doing it. Carries the proposal review queue.
- **CX-2** — the clip window belongs in `.musi`, as one of a **list** of named
  export variants. Schema v2. Prerequisite: PXF-4.
- **CX-4 (HY-4: tap offset)** — the operator listening session above. The rulings it already made
  (visual-only tap feedback with no stamp click, revised Surprise constants) are
  landed. Track A was rejected on 2026-09-05; track B and tap/flash validation
  remain open. SX4 is the response to that rejection, not a passing CX-4 result.
  The **-100 ms tap-offset default is NOT landed** (found 2026-08-29: `LyricTap`
  derives `Default`, 0.0, and core tests pin it). Since PXF-2 the persistence
  path defers to `LyricTap::default()`, so the ruling is one constant plus
  `tools/headless_check.sh`'s `last stamp` expectation and two core tests.
- **CX-5** — the thumbnail review is in the waiting section above. The stable
  representative moment does not require resume-position persistence.

**PXF — PX-wave debrief follow-ups.** PXF-6, PXF-8, PXF-9 and PXF-11 are not
separate work: they are the *questions* CX-5, CX-1, CX-2 and CX-4 answered, and
the answers are the rows above. The rest are still open as written.

- **PXF-3** — a tap should flash its block in the lane. The only feedback today is
  an 11 px counter the player is not looking at.
- **PXF-4** — the clip is invisible on the timeline. In/Out are set against a
  waveform that shows nothing; drag handles on the strip belong to the timeline
  gesture owner, not the export panel.
- **PXF-13** — crash recovery does not resume the playhead. PXF-7 (landed
  2026-11-13) reopens a saved project where it was; a restored recovery
  snapshot still starts at 0:00.
- **PXF-12** — the route editor's 104-band stepper still has no typed value or
  wheel nudge. Keyboard nudge conflicts with the transport's arrow keys and needs
  a decision rather than a silent resolution.

**UX0-B — workflow friction.** B06 large-document lyric navigation; B07 visible
lyric draft state; B10 reviewable Assist reasoning; B11 actionable timing
uncertainty; B12 selective/reversible Assist apply; B13 meaningful Assist
progress; B14 actionable Assist failure; B15 remote-mode prerequisite checks;
B16 recoverable missing-support state; B17 consistent Assist visual semantics;
B18 a discoverable keymap (it now has the number row's ten scenes to list,
D7); B19 useful tall-window layout.

**UX0-C — product opportunities.** C05 scene thumbnails (= CX-5); C08 a
project-level palette/look; C09 a cover-art/logo layer; C17 room for the caption
tune editor.

**B/C/D/E/F/G — the lettered tranches' remainder.** Several of these read as
whole tranches in the archive and are down to one clause; the clause is what is
here.

- **B3** — preview/export tests at every cue start/end boundary, including seek
  backward, fast-forward and a windowed export beginning after cue zero, asserting
  the selected scene id and a cue-specific setting rather than the enabled flag.
- **C2** — lyric and route draft context guards. Autosave's half is done
  (`Shell::editor_draft_blocks_autosave`); the blocking/resolution workflow on
  track change, scene change, project open, export start and Assist apply is not.
- **C3** — **the Undo half only.** Reset → Confirm landed as UX0-A08 (`b73383d`),
  keyed per `(track, scene)`; "Undo reset" and its preserved pre-reset snapshot
  did not.
- **C5** — adopt a project's embedded scene presets into the shared library, with
  the C's dedup rule, and never let the in-memory and on-disk libraries silently
  disagree after a failed write.
- **D6** — `SettingKind::Toggle` drawn as a labelled binary control rather than
  a numeric slider. Inspector scrolling awaits the visual check listed above (nonvisual checks only, 2026-09-05).
- **D8** — an actionable notice tray: call-site-provided specifications, paths and
  tooltips, Dismiss, Copy path, Assist Retry, a `+N more` indicator. UX0-A11
  landed the opaque card, wrapping, `Severity::dwell` and the close box; start
  there.
- **E2/E3/E4** — prove the copied support bundle actually runs: Assist end to end
  from a real installed layout with re-probed helper discovery, Google Fonts
  import (transactional, once-per-run consent, offline fixture), and a
  distribution/doctor path tested from **outside** the repository root with no
  Cargo invocation. DX8, which blocked these, closed 2026-08-29.
- **F1/F2** — remove the remaining false "not implemented" status text (including
  Cadence's empty preview-only frame, which reads as a broken renderer) and the
  stale agent-era handoff comments and `allow(dead_code)`. Overlaps DX7; close
  them together.
- **F3** — the FFmpeg test-helper `ETXTBSY` race. It is a shared
  generated-executable race, not one flaky assertion: fix it or formally close it,
  and keep a stress test either way.
- **G1** — the integration gates that cover the holes these waves exposed: one
  seeded project proving lyrics, semantics, manual events, scene plans and cue
  settings all reach preview at once; full-versus-windowed export frame hashes;
  autosave across multiple tracks; packaged-helper discovery. Every new gate gets
  a negative control and what the perturbation broke gets recorded.
- **G2** — a final capability audit against `docs/PHASE0_INVENTORY.md`, and an
  interactive session on the operator's real desktop (dialogs, drag/drop,
  fullscreen, launcher startup) that Xvfb cannot stand in for. **Its old
  "open a Rust `.musi` in the C" step is retired** by the 2026-08-03 legacy
  decision; the compatibility contract runs against our own releases.
  **Never tried for real, from the 2026-11-13 wave, to go with it:** the file
  dialogs behind the Direct view's treatment picker and Import clip…; the look
  cards with a real mouse and keyboard; a real agent session beside the
  picker; a 4K export with a slow preset (EX4); a GPU that refuses the 16-bit
  export target; a real phone video, and a clip longer than 20 s or above
  1080p. From the 2026-11-14 follow-up: the generation and listening-lane items
  listed under VG and the listening-lane remainder below.

**EX3/EX4 — the export defects' remainder.** Both defects landed in the
2026-11-13 wave (the 16-bit export target with one dithered quantise; the
writer thread that keeps the progress screen painting). Left:

- **The Render view does not say what Balanced costs**: when the film finish
  is on and the quality is Balanced, a line should say that Balanced flattens
  grain and gradients and High keeps them. Balanced itself stays as it is
  (measured 2026-11-14, `docs/archive/BALANCED_ENCODER_2026-11.md`).
- **The finish's bloom and blur buffers are still RGBA8**
  (`runtime::offscreen::Canvas`).
- Recorded, not fixed: a **cached** mid-export caption atlas failure would make
  the second half of one MP4 blurrier than the first.

**PX2 — the lyric undo history is per-session and per-current-track**, cleared by
`enter_track`. Cue ids restart at 1 in every document, so one track's snapshot
would restore another track's cues wholesale if the clearing were removed. A
per-slot map is the follow-up.

**DX — dev-ex.** DX1 one enabled/disabled widget API; DX2 one wrapping/ellipsis
policy; DX3 shared scrolling policy (before D6); DX4 text-field input split from
rendering; DX5 centralized XDG resolution; DX6 collision-safe test scratch
directories; DX7 purge agent-era ownership language and enable strict rustdoc (17
broken links); DX9 verification speed,
whose largest term was answered by putting the gate on the GPU (`AGENTS.md`), and
whose remainder is cached C harness executables and prebuilt Rust differential
examples.

**AP — assist providers.** AP5-b a structural no-network-hang
regression test; AP5-c a diagnostics/crash bundle collector (a feature, and it
must strip credential name-markers with a canary test); AP5-d the clipboard copy
path; AP6-e deferred discovery for `ffmpeg`, `whisper-cli` and the alignment venv
interpreter, each of which needs its own decision. AP3 and AP4 also close with
"Not built here" lists — `ask` mid-job, a `TC-VERIFY` nothing composes, the ZDR
endpoint list, `prefer_gpu`/`stem_separation`, the staged-snapshot field move.
They are deferrals with reasons, in the archive; promote one here before building
it.

**AX — the 2026-08-29 assist audit.** AX-1 through AX-6 all landed 2026-08-31
across seven commits; the wave and its negative controls are in
`docs/archive/FEATURE_PARITY_HISTORY.md`, the findings themselves in
`docs/archive/ASSIST_AUDIT_2026-08-29.md`. What the wave left open:

- **AX-7 (HY-3) — A7's two `strip_credential_variables` copies, and the login shell
  that gets neither.** `runtime/assist/discover.rs:338` and
  `app/ui/assist_settings.rs:3176` are the same function in two crates, and
  `discover::login_shell_lookup` (`discover.rs:270-295`) spawns `$SHELL -lc`
  with an unfiltered environment — the one spawn in the discovery path that
  runs a user's rc files. AX-1 made this *hotter*, not cooler: `plan::resolve`
  now reaches the `thorough()` rung on the UI thread for every graph carrying a
  Codex route (documented at `plan.rs:438-452`), so the login shell runs at
  Start, not only from the dialog. One copy in `runtime`, called by both, plus
  the strip applied to the shell child. `env.rs:43-45`'s doc comment still
  forbids the blanket strip both copies perform, so the fix has to reconcile
  the comment with the callers rather than pick one.
- **AX-8 (HY-3) — B12's remaining two duplications.** `cache_dir` was unified
  (`runtime::assist::files::cache_dir`, matching Python's XDG rule), but
  `assist_settings::find_tool` (`:3197`) still has no `MUSIALIZER_*_HELPER`
  override rung while its own doc comment says it resolves "the way
  `find_assist_helper` resolves the analysis helper" — and that function
  (`process/font_import.rs:603-626`) honours the override and fails hard on a
  set-but-missing one. So the doctor and both catalog helpers cannot be
  redirected the way the analysis helper can, and the comment is wrong about it.
- **A12** — skipped by AX-1 on purpose, and the reason is worth keeping:
  `CredentialState::Session` is only ever set from the env import today, so the
  display fix is cosmetic. The real question is whether the app should persist a
  `mode: env-import` record for a key it deliberately does not store, which is a
  schema decision, not a defect fix. Promote it only with that decision made.
- **A8** — still deliberately not queued: the E1 strip at six more spawn sites
  (`ffmpeg`, `kdialog`/`zenity`, `xdg-open`, the font-import Python, `curl`) is
  a contract decision for the operator, not an agent's call.
- **Noted, not queued** (from the audit, and AX-1 raised its stakes): a
  thorough-probe `NotFound` is cached by `discover::resolve_cached` for the
  process lifetime, so installing codex mid-session needs a restart — and since
  AX-1 that stale `NotFound` blocks the whole job rather than only the badge.
  Adjacent to AP6-e.

**HX-4** — the `claude -p` generate/digest buttons for protocol authoring. The MVP
loop works without them; the agent writing a protocol reads the JSONL directly.

**GX-1 — probes should name their target, not its pixel.** Either accept a widget
name in `--ui-probe click=`/`hover=` and resolve it through the id table, or add a
report line carrying the control rectangles so the gate computes the point it
presses. Until then every coordinate in `tools/headless_check.sh` is a latent
version of the defect that let three export SIZE buttons sit dead for months.

**GX-3 — the poster gate's frame-budget comparison measures the machine,**
as GX-2's stall assertion did. On 2026-11-14 it failed the full verify
("posters stalled more frames (3) than no posters (1)"), and again in the
full verify on `7abda56` (2026-11-14); run alone three times at load 6 to 7
it gave on/off stalls of 1/2, 0/5 and 2/1. It needs a
comparison that survives a busy machine (repeated runs and medians, or the
poster service's own step timings) before it can decide anything. Do not
loosen it without that.

**GX-4 — tests and gates that fail under load**, seen once each (2026-11-14
to 2026-11-15) and passing alone; find each timing assumption:

- `clip::tests::a_budget_of_two_decoders_runs_two_whatever_the_clips` and
  `process::ffmpeg::tests::dropping_an_encoder_without_finishing_it_still_reaps_and_cleans_up`.
  The clip one failed again in the full verify of 2026-11-16 (load average 5,
  another session's GPU job running; `decoders: 1` where 2 were expected) and
  passed six times alone straight after.
- `tools/treatment_check.sh` run standalone on a fixed display can die on a
  race in `tools/headless_run.sh`: the previous launch's X lock vanishes
  between the check and the read (a builder's report, not reproduced).
- `tools/lyric_focus_check.py` still focuses the last root child, not the
  mapped window (`docs/TRAPS.md`, the XTest driver entry).

**GX-5 — the Feelings charge ledger's two queued findings** (the fourth
review, GPT-6.1-Sol, 2026-11-16; neither loses or hides money one person in
one window can meet):

- A status answer over 1 MiB (thousands of pending charges) is refused whole,
  and the card then reads as an ordinary paid Start with no word that earlier
  requests could not be shown. Say so on the card when the status query
  fails; aggregate or page the list.
- With two helpers on one folder, one that saved its answer and was killed
  before removing its sent record, while the other's answer replaced the
  cache, is swept as "never answered" though it was. It over-counts, and
  Start puts it behind. Write the attempt's `answered` line before its
  evidence can be replaced.
- Also never pressed for real: Apply and Put back on a real answer (the probe
  has one click per run), Cancel while a request is in flight, a settings
  change through the AI dialog while the card is open, two windows on one
  track.

**SX2/SX3 — scene follow-ups.** No HX protocol has been run on Clawd's expression
thresholds, its petal-show arm thresholds (measured against one track), or any of
the three depth passes; the blinded A/B machinery exists but none of the three is
a setting, so a variant switch has to be built first. `TrackDynamics` rides every
`SceneFrame` but only Clawd reads it — any scene with an absolute loudness gate
has the same latent defect.

**MV — directing from the command line.** MV1 (`--treatment`, `--vocabulary`,
`--contact-sheet`, `--still`, `tools/headless_run.sh`) is in the archive. Making
one real video with it found these, each from a contact sheet:

- **MV2's remainder** (named caption styles and the title lane landed
  2026-11-13): a cue's style is not editable in the Lyrics view beyond the
  chip that steps it; nothing has exported a 9:16 video with a title card.
- **MV3 — transitions**, landed 2026-10-12 (edited in the window since
  2026-11-14, MV3-a). Still open:
  - **MV3-b — the outgoing picture is a held frame.** Invisible under half a
    second, visible in a long dissolve out of a busy scene. Two live scenes
    need a second settings table, a second route evaluation and scene state
    advanced past its own cut.
  - **MV3-c — beat-aware lengths.** A transition is seconds; "one bar" and
    "land on the next downbeat" would be worth having once the tempo grid is
    trusted.
- **MV4 — Phosphor's `titles` clips a long line.** The built-in 5x7 face fits
  about twenty characters; a full lyric runs off both edges. Wrap, scale or
  shorten.
- **MV5 — Clawd's petal show cannot be lit *at* a cut.** It needs about three
  seconds of kicks after the scene starts. The workaround (start the section two
  bars early) is documented; a section that begins mid-drop could instead be
  seeded from the track's energy at that instant.
- **MV6 — a treatment cannot name an imported caption face** or the caption
  effects' drive-tuning blocks. Both are refused by name today.

- **MV7 — Cadence's second pass** landed 2026-10-12 (`docs/SCENE_DESIGN.md`):
  clean edges, a halo, colour and reveal controls, a reading-pace cap. Still
  open from the same notes: one typeface only, no per-word colour, no gradient
  across the letterforms. Wants the operator's eye before more is added.

- **MV8 — Stage**, landed 2026-10-12: a scene that draws the lyric line as one
  of six stagings, picked per section. It is the first scene here about what a
  line *says*. Open, in the order a director runs into them:
  - **MV8-a — staging is a section's, not a line's.** One section per line
    works and is verbose; a `stage` hint on the lyric cue itself would let a
    plan stay coarse. A `.musi` extension to the cue.
  - **MV8-b — the chat side follows the line id's parity.** Two lines meant as
    one speaker can land on opposite sides. Wants a control.
  - **MV8-c — no images or props beyond the cat.** A section cannot place a
    picture, a logo, or a second character.
  - **MV8-d — six stagings.** Obvious next ones: a calendar page, a road sign,
    a receipt with totals, a split-flap board, a handwritten note.
  - **MV8-e — no Tune-panel affordance for `layout`.** It is a 0..5 slider;
    it wants named buttons.

- **MV9 — the "Door, No Room" compositor's work, moved into the renderer.**
  Landed 2026-10-21 in full (a–k); MV9-l, editing it in the window, landed
  2026-11-14 as Look's look cards (`docs/MUSIC_VIDEO_CLI.md`, "Editing the
  look in the window"). What the look cards left:
  - No undo for plan edits (the lane has none either).
  - A window edit is not written back into a followed treatment; the cards
    are disabled while one is followed.
  - A caption style's other members (face, box, margins, effects), and
    clearing a voice's or a style's value back to its own, are a treatment's.
  - Giving a line a voice from the Lyrics view (Look's Captions card can).
  - The Pixel group's typed value is keyed by slot and scene only and may
    need the cards' owner rule (a draft belongs to one project).

- **VG remainder — clips and generating them** (row 12):
  - A clip restored from a crash-recovery snapshot is still opened by name,
    and autosave still checks an existing bundled file by name; the bundled
    path is walked beneath its folder everywhere else.
  - The `punch` setting reads as subtle even at 0.6.
  - No `generated` transition (a generated section with `"first":
    "previous"`, `"last": "next"` is the bridge today); the prompt field is
    one scrolling line, not multi-line.
  - A save writes only the clips a section plays, so an unplaced generated
    clip lives only in the store.
  - Accepted from the last review, to fix only if it ever matters: the
    ceiling comparison's tolerance of one billionth of a dollar.
  - Never tried for real (all four real calls went through a treatment run):
    the window's card against the real service, a real Cancel, a failed or
    refused real job, a second model family, a lost answer, two callers at
    once, a cut-short download, an expired job.

- **WX — the window as a directing workstation** (2026-10-21). Parts, order,
  decisions and status live in [`docs/WORKSTATION.md`](docs/WORKSTATION.md);
  that file is updated as each part lands, and its "Open work" is the rest.

### The listening lane and the benchmark's remainder

Gemini 3.8 Flash through OpenRouter is default and recommended on the operator's
word (2026-11-15), without a benchmark for that choice. MiMo was set aside;
its overlay row remains experimental and selectable with **Show experimental**.
The operator's quotes and the route decision are in the archive, "The listening
lane moves to Gemini 3.8 Flash"; request/evidence details in
[ASSIST_PIPELINE](docs/ASSIST_PIPELINE.md) and
[DIVERGENCES](docs/DIVERGENCES.md), "The listening lane's model".

- Not known: whether the suggested plan from Gemini's sections is good enough
  to act on unchecked. Three real answers (0.0475 USD) showed one excerpt cut
  well and energy following loudness; another misjudged a section 2–4 dB quieter
  and put a return 8 s early. Identical audio heard twice in one call was cut
  differently. These are observations, not a benchmark.
- Pointing the harness at Gemini needs a price row and removal of its fixed
  temperature; a new paid run needs the operator's go-ahead. The first MiMo run
  was 60 calls on 2026-11-14 ([results](docs/MIMO_BENCHMARK_RESULTS.md));
  repeats 2 and 3 of most cells remain unrun and are not worth running for a
  model set aside. The truth session and converter in `tools/mimo_bench/` stay
  for a later benchmark; answers are no longer waited for (the app uses none
  of key, metre or instruments). A future key question must not force a choice
  between relative keys.
- Accepted from reviews, to fix only if it matters: the "latest attempt failed"
  notice can be wrong with simultaneous helpers on one cache or a wall-clock
  rollback; an older build does not reuse a cache a newer build wrote.
- Never tried for real: a cut-off or refused answer from either model, a track
  near the request size limit, a route without zero data retention, Start
  pressed in the application itself. See GX-5 for the newer Feelings review
  and remaining real-interaction checks.

## Credential location (HY-3)

The operator's OpenRouter key lives in `~/.config/musializer/credentials.json`.
The desktop refuses repository `.env` credentials; HY-3 owns replacing the
legacy CLI fallback with that credential file.

## How this file stays authoritative

- Start work by claiming one id here and recording its dependencies.
- **When a task lands, take it out of this file** and put its evidence — commit,
  captures, negative control, and what the check caught — in
  `docs/archive/FEATURE_PARITY_HISTORY.md` under its wave. This file grew to 4,385
  lines by doing the opposite, and a queue nobody can read is not a queue.
- Never open a second completion plan. Newly discovered work becomes an item here
  before it becomes code.
- An item leaves only as `completed`, `duplicate`, `not observable`, or an
  operator-approved exclusion, with the reason recorded in the archive.
- Do not restate what the tree already answers. If the question is "does this
  work", the answer is a test, a probe or a report line — not a paragraph here.
