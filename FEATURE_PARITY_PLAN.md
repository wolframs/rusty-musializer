# Rusty Musializer — the live queue

**This file carries only what is open.** It kept the name it was born with, but
it stopped being a parity plan a long time ago: the C was declared legacy on
2026-08-03 and the application passed its ceiling in the waves that followed.
Nothing here needs a parity justification, and "the C does it this way" is not an
argument for or against any item below.

If another document, source comment, old agent handoff or ignored worktree names
unfinished work, reconcile it here before acting on it. There is one queue.

**Where the closed work went (2026-08-29).** Every finished wave — LX, EX, PX,
CX, HX, AP, LT1, DX's landed half, P0–P4's landed half, UX0-A, UX0-D, and the
three scene waves SX1–SX3 — is in
[`docs/archive/FEATURE_PARITY_HISTORY.md`](docs/archive/FEATURE_PARITY_HISTORY.md),
verbatim, with its evidence, its operator quotes and its negative controls
intact. Read it when you need to know *why* something is the way it is. The item
ids below are the ids it uses, so `grep` finds the context.

Roughly a dozen source doc comments cite `FEATURE_PARITY_PLAN.md` plus an id
(`UX0-C01`, `LT1-R R9`, `F1`, `CX-4`). **An id a source comment names and this
file does not list is a closed item** — its record is in the archive under the
same id. Nothing was renamed.

Other standing references: `AGENTS.md` for repository rules, the `unsafe`
inventory, the divergence table and the deliberate exclusions; `REWRITE_PLAN.md`
and `docs/archive/` for history; `docs/PHASE0_INVENTORY.md` for formats, CLI
grammar and schemas.

## Deliberate exclusions

These are not open work and do not block anything. They are restated from
`AGENTS.md`, which is authoritative:

- Microphone capture (`MUSIALIZER_MICROPHONE`)
- Hot reload, including a functional `--reload-once`
- Non-Linux platforms, including Windows, macOS and OpenBSD

Exact pixel identity with the frozen C, its source organization and its known
defects are also not goals. A new deliberate exclusion needs operator approval
and lands in both files.

## What is not negotiable while any of this is built

Restated because it is the one thing an open queue can quietly break. An
unplanned change in any of these is a bug, not a design choice — a *planned* one
needs a schema bump or an updated harness and a recorded reason:

- The `.musi` format and every schema in `docs/PHASE0_INVENTORY.md`. A project
  saved by any earlier build of *this* application must still open.
- Analysis numbers: analyzer, beat tracker, band layout.
- Settings semantics: descriptor keys, bounds, defaults, precision, clamping.
- Export determinism. The same project produces the same frames.
- The CLI grammar, including flag-order effects and exit status.
- Anything with a `tools/differential_*.sh` harness. The harness is the contract.

## Open work

Ordered as a session should pick it up. Trust outranks delight — that ordering is
a CX ruling, not a preference.

| # | Work | Ids | Size |
| --- | --- | --- | --- |
| 0 | **The fifteen-minute listening session.** The only item in the whole queue a human has to do: validate the tap offset (4 min), compare 80/120/200 ms tap flashes (2 min), blind-compare five current against five revised Surprise seeds on two tracks (9 min), against CX-4's stated pass condition. Blind protocol files are written and waiting — `build/protocols/cx4-surprise-{a,b}.protocol.json`, driven by `tools/cx4_surprise_session.sh`, with the browser counterpart in `tools/listening-lab/protocols/`. Unblind through the matching `.key.json` **only after** answering | CX-4 | 15 min, operator |
| 1 | **MiMo v2.5 capability benchmark** — operator's stated priority. Design and harness exist (`docs/MIMO_BENCHMARK_PLAN.md`, `tools/mimo_bench/`). **Needs an explicit operator go-ahead**: it sends audio to OpenRouter and spends credits | — | one session |
| 2 | **The remaining CX rulings, in their stated order.** CX-5 scene thumbnails behind a poster-frame service (PXF-7 first), then CX-1 marker rails and the proposal review queue (PXF-8), then CX-2 export variants (PXF-4 first, schema v2 — PXF-9) | CX-1, CX-2, CX-5 | large; four or five agents |
| 3 | **Workflow friction and product opportunities** left from the user-perspective review | UX0-B06, B07, B10–B19; UX0-C05, C08, C09, C17; PXF-3, PXF-5, PXF-12 | medium |
| 4 | **Durable-edit remainder, then the missing product surfaces, then honesty and gates** | B3; C2, C3, C5; D6, D7, D8; E2, E3, E4; F1/F2/F3 remainder; G1, G2 | large |
| 5 | **Dev-ex and infrastructure** (DX8 closed 2026-08-29 — E2/E4 unblocked) | DX1–DX7, DX9 | small each |
| 6 | **Assist provider remainder** | AP5-b/c/d, AP6-e | small each; AP5-c is a feature |
| 7 | **Scene follow-ups**, each of which wants a listening pass rather than more code | SX2, SX3 open notes | small |

### The items, in one line each

Full context for every id is in the archive under the section named by its
prefix.

**CX — consultation rulings.**

- **CX-1** — replace the event markers' authorship encoding: **top rail vs bottom
  rail**, not disc vs ring. A legend explains an encoding the eye still cannot
  resolve and spends band height doing it. Carries the proposal review queue.
- **CX-2** — the clip window belongs in `.musi`, as one of a **list** of named
  export variants. Schema v2. Prerequisite: PXF-4.
- **CX-4** — the operator listening session above. The rulings it already made
  (visual-only tap feedback with no stamp click, revised Surprise constants) are
  landed; only the nine-minute blind seed comparison is outstanding — **and the
  -100 ms tap-offset default is NOT landed** (found 2026-08-29: `LyricTap`
  derives `Default`, 0.0, and core tests pin it). Since PXF-2 the persistence
  path defers to `LyricTap::default()`, so the ruling is one constant plus
  `tools/headless_check.sh`'s `last stamp` expectation and two core tests.
- **CX-5** — scene thumbnails, but only the **track-specific** kind. Prerequisite:
  PXF-7.

**PXF — PX-wave debrief follow-ups.** PXF-6, PXF-8, PXF-9 and PXF-11 are not
separate work: they are the *questions* CX-5, CX-1, CX-2 and CX-4 answered, and
the answers are the rows above. The rest are still open as written.

- **PXF-3** — a tap should flash its block in the lane. The only feedback today is
  an 11 px counter the player is not looking at.
- **PXF-4** — the clip is invisible on the timeline. In/Out are set against a
  waveform that shows nothing; drag handles on the strip belong to the timeline
  gesture owner, not the export panel.
- **PXF-5** — the still blocks the frame loop behind a static "Rendering still
  frame". The export's own progress screen is the model.
- **PXF-7** — resume position: `Metadata` has no playhead field, so reopening a
  recent project starts at 0:00.
- **PXF-12** — the route editor's 104-band stepper still has no typed value or
  wheel nudge. Keyboard nudge conflicts with the transport's arrow keys and needs
  a decision rather than a silent resolution.

**UX0-B — workflow friction.** B06 large-document lyric navigation; B07 visible
lyric draft state; B10 reviewable Assist reasoning; B11 actionable timing
uncertainty; B12 selective/reversible Assist apply; B13 meaningful Assist
progress; B14 actionable Assist failure; B15 remote-mode prerequisite checks;
B16 recoverable missing-support state; B17 consistent Assist visual semantics;
B18 a discoverable keymap; B19 useful tall-window layout.

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
- **D6** — Tune reachability: a scrollable inspector, and `SettingKind::Toggle`
  drawn as a labelled binary control rather than a numeric slider.
- **D7** — **direct 1–0 scene selection and its tile tooltips.** Ctrl+S /
  Ctrl+Shift+S are bound (UX0-A) and the "text entry suppresses every global
  shortcut" test exists (UX0-A06); the number row does not.
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

**EX3/EX4 — two export defects named and deliberately not fixed.**

- **Gradient banding in exported frames is real and is not the codec.** A 0→40
  ramp round-trips through `yuv444p` at 0.41 RMS, so the contours are in the frame
  handed to FFmpeg: `LoadRenderTexture` is RGBA8, the halo's ping-pong buffers are
  RGBA8, and every blend is gamma-space with no `GL_FRAMEBUFFER_SRGB` anywhere in
  `rlgl.h`. The fix is an RGBA16F export target through
  `rlLoadFramebuffer`/`rlFramebufferAttach` — a new `unsafe` island that must check
  `rlFramebufferComplete` before trusting the driver.
- **The export progress screen does its blocking frame write inside its own
  `begin_drawing`/`EndDrawing` pair**, so while the encoder is backed up neither
  Escape nor Cancel is polled and the window does not repaint. At 4K `-preset
  slow` that is a visibly frozen application that reads as "the export hung".
  Moving the write outside the pair is the fix.
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

**HX-4** — the `claude -p` generate/digest buttons for protocol authoring. The MVP
loop works without them; the agent writing a protocol reads the JSONL directly.

**GX-1 — probes should name their target, not its pixel.** Either accept a widget
name in `--ui-probe click=`/`hover=` and resolve it through the id table, or add a
report line carrying the control rectangles so the gate computes the point it
presses. Until then every coordinate in `tools/headless_check.sh` is a latent
version of the defect that let three export SIZE buttons sit dead for months.

**SX2/SX3 — scene follow-ups.** No HX protocol has been run on Clawd's expression
thresholds, its petal-show arm thresholds (measured against one track), or any of
the three depth passes; the blinded A/B machinery exists but none of the three is
a setting, so a variant switch has to be built first. `TrackDynamics` rides every
`SceneFrame` but only Clawd reads it — any scene with an absolute loudness gate
has the same latent defect.

## Two standing facts a new session needs

- The operator's OpenRouter key lives in `~/.config/musializer/credentials.json`.
  The repository `.env` is legacy CLI-only and the desktop path refuses it.
- `xiaomi/mimo-v2.5` is `experimental` in the suitability overlay, so **Show
  experimental** must be on for the MiMo lane to be routable until a benchmark
  earns it a `recommended` row.

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
