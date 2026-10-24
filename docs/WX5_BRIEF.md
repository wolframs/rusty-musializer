# WX-5 redesign — lead brief (2026-10-24)

*Committed as written for the run; paths under `build/` were that run's scratch
and are not in the repository. The fixture is a copy of the operator's Door, No
Room project — make a fresh copy, never open `~/Music` directly.*

Written by the lead (main session) for the designer and the critic. The operator
answered four questions this morning; those answers are the frame.

## Operator answers, 2026-10-24

- **Scope: WX-5a → 5b → 5c in this run.** One commit set per step, gate green
  between steps.
- **Secondary panels (Export, Tune, Lyrics, Assist, AI settings, protocol card,
  Editor, dock): re-skin only.** They take the new tokens and type scale; their
  layouts are restructured in a later wave. Don't restructure them now.
- **Theme: the dark mockup look becomes the default; the light appearance is
  kept**, running on the same tokens with the same structure.
- **Latitude: the mockup is the spine, details are free.** Fixed: one accent that
  means *now*; the plan lane as runs; the rail with flyouts; the preview as the
  largest thing on screen. Spacing, controls, flyouts, motion, the transport's
  exact shape are the designer's to invent.

Background the operator said earlier (WORKSTATION.md, memory): "MAN IS THIS
INTERFACE UGLY", "not married to the current UI", "barely use it by hand
anymore" — the window is now where you **watch** a video being directed and
**talk** to whoever directs it. Mockup reaction: "That mockup is sick looking :O".
Read AGENTS.md "Who this is for": a result should look like something worth
posting, an idle affordance should make you want to try it.

## Where things are

- Worktree: `/home/wolfram/Projects/rusty-musializer-wx5`, branch `wx5-redesign`,
  based on master `fe78a00`. Both agents work there, **taking turns** (the lead
  sequences them). Commit on that branch; do not push; do not touch master or the
  main checkout's source.
- Mockup: `docs/images/wx-mockup.jpg`; its generator `tools/wx_mockup.py`
  (and `build/wx/wx_mockup.html` in the main checkout is the HTML it came from).
  Design record: `docs/WORKSTATION.md` (WX-5 section and "Next sessions" 1–3).
- As-is capture of Door, No Room at 1920x1080 (light skin, the current default):
  `/home/wolfram/Projects/rusty-musializer/build/redesign/asis/main-1920.png`.
- Inventories from three Haiku explorers (file:line maps — trust but verify):
  `/home/wolfram/Projects/rusty-musializer/build/redesign/explore-shell.md`,
  `explore-timeline.md`, `explore-panels-gates.md`.
- Realistic fixture: a **copy** of the operator's Door, No Room project at
  `/home/wolfram/Projects/rusty-musializer/build/redesign/fixture/Door, No Room.musi`
  (+ `.assets`). Never open anything under `~/Music` — autosave and treatment
  following can write a project.

## Lead decisions (take these as settled unless you find a concrete reason; then tell the lead)

Tokens (5a)
- One source of truth for colour and type: `ui/theme.rs`. egui's `apply_theme`
  derives from it (today it hard-codes Color32s that already drift).
- Accent = *now*: playhead, current section/run, the primary action (Render).
  Nothing else wears it. Selection = outline + tint, never a solid accent slab.
- Default appearance flips to the dark mockup look. Existing `ui.json` values
  (`light-blue`, `black-amber`) must still load and mean something sensible;
  `--ui-theme` keeps accepting its existing names. Renaming labels is fine.
- One type scale for raylib and egui (today 19/16/13/15 vs 20/15/15/12).
- Hard-coded `Color::` in UI files move to tokens where they are chrome. Content
  colours (scene identity colours, cue-origin colours, event-type colours) stay
  semantic tables but must be checked for contrast on the new surfaces.

Plan lane (5b)
- A run breaks on a **scene change only**. Inside a run: a hairline per internal
  cut, a light edge where a transition arrives, and the grade bar under the run is
  segmented per section (so a grade change inside a run is visible).
- Every existing lane gesture stays reachable and probe-able (select a section,
  drag a boundary, split, capture tuning, delete, Auto). Double-click a run opens
  its sections (mockup), or zoom past a threshold shows sections — designer's call,
  but no editing capability is lost.
- Thumbnails are **track-specific poster frames** (CX-5's ruling), one per run:
  a frame-request service (see explore-timeline.md §3 and the archived design it
  cites), rendered off the playhead through the export's offline path with the
  still's warm-up rather than a replay from zero, built incrementally under a
  per-frame budget, cached, placeholder = the scene's colour until ready. If that
  proves too costly, say so with a measurement before falling back.
- Waveform grey; the played part warms. Lyric lane keeps its words.
- `scene marks:` is currently asserted nowhere: add an assertion, and a report
  line for the runs (runs, sections, thumbnails ready/total).

Rail and preview (5c)
- Tracks sidebar and scene browser become a narrow icon rail with flyouts; the
  preview takes the space. Top bar per mockup: title, artist, duration, format,
  saved state; mode tabs mapping onto the existing panels; Still / Contact sheet /
  Render actions (Render is the accent). A UI action with no backing yet is drawn
  disabled with a reason (AGENTS.md "Missing beats pretending"), not hidden.
- The event row (+Feel/+Scene/+Custom/Clear) and the segment controls row move
  into context (selection, flyout, a compact toolbar) — still reachable.
- `core::ui::workspace_layout` changes on purpose: update
  `tools/differential_layout.sh`/its dump deliberately and record the divergence
  and its reason in AGENTS.md's table.
- The picker in a flyout owns the Pixel Cats decisions WORKSTATION.md records
  (six stagings because six fit unscrolled; two rows of family tabs). Fix the
  picker shape; building the four unbuilt stagings is out of scope.

## Rules that bind both agents

- AGENTS.md is the repository guide; read it. Especially: audio isolation
  (`tools/headless_run.sh` for every hand launch, `--mute`), the WAYLAND trap,
  widget ids only via `widgets::id::ALL`, `--ui-probe` for anything a capture must
  prove, report lines for states two pictures can't tell apart, "The headless gate
  runs on the GPU" (exact fills vs blends/hairlines; a new pixel classifier gets
  a negative control on both renderers).
- Gate assertions that move are **updated on purpose, never loosened**. Each
  updated pixel/colour check keeps a negative control.
- The headless gate uses Xvfb `:77` by default. In the worktree run it with
  `MUSIALIZER_CAPTURE_DISPLAY=:78` so the main checkout can gate concurrently.
  Run long commands with the Bash tool's `run_in_background`, never `( … ) &`;
  `pgrep -af Xvfb` before starting a gate. Stop every background process you
  started before you report.
- `cargo fmt`, clippy clean, `cargo test` green; `tools/verify.sh` green at the
  end of each step; `cargo build --release` at the end of each step.
- Docs: WORKSTATION.md status per part, AGENTS.md divergence rows where the
  window now does something on purpose that a later session might "fix" back,
  UI_TOOLKIT.md where the egui/theme boundary changes.
- Commit messages: conventional style as in `git log` (`feat(wx5a): …`,
  `polish(wx5a): …`), ending with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never `git add -A` blindly (OUTSIDE-DROPS and song analysis must not be
  committed); add paths.
- The operator explicitly allowed both of you to spawn **Haiku** subagents
  (`model: "haiku"`) for small sub-threads: lookups, small related edits,
  quick A/B iterations of a detail, tracing "what happens if". Use
  `subagent_type: "Explore"` for read-only work and `general-purpose` with
  `model: "haiku"` when it must edit. Give each a narrow brief with paths. They
  share your worktree — don't let two edit the same file at once.
- Judge your work by looking: capture Door, No Room (fixture copy) at 1920x1080,
  1280x720 and 960x640, in both appearances, at 100 % and 150 % UI scale, and
  read the captures. The "does it feel like something worth working in" test is
  part of done.
