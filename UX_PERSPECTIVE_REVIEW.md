# UX perspective review

A code and UX/UI review of rusty-musializer **as seen by its user** — a musician
or hobbyist who opens a song, tunes scenes, times lyrics, leans on Assist, and
exports a video. Parity with the frozen C is not the yardstick here; the product
is judged on its own terms.

- **Reviewed state:** committed master `d22f9de`, in a detached worktree
  (`../rusty-musializer-uxreview`), untouched by the concurrent timeline work.
- **Method:** nine parallel review agents (visual design from real captures,
  first-run walkthrough, lyrics-editor deep dive, Assist UX, Tune UX,
  feedback/error sweep, two code bug hunts, product-opportunity scan), every
  bug claim then adversarially verified by an independent agent against the
  code. All bug claims below are **confirmed** — each was traced to the exact
  code path, none survived on plausibility.
- **Evidence:** `file:line` references are into this tree at `d22f9de`. Capture
  names refer to `tools/headless_check.sh` output
  (`../rusty-musializer-uxreview/build/headless/*.png` as of this review).
- **Plan tags:** a `[C2]`-style tag means the finding overlaps an open
  `FEATURE_PARITY_PLAN.md` task; the detail here should feed that task rather
  than spawn a duplicate. The in-flight D4 timeline scene-lane work was
  excluded from review scope entirely.

---

## 1. Confirmed defects, ranked

### 1.1 Nudging a tightly-timed cue crashes the app — `f64::clamp` panics

`time_row`'s final clamp is `value.clamp(0.0, other - 0.001)` for START and
`value.clamp(other + 0.001, duration)` for END
(`crates/musializer-app/src/ui/panels/lyrics.rs:1621-1627`). Rust's `f64::clamp`
asserts `min <= max`. `validate_cue` requires only `end > start` with **no
minimum gap** (`crates/musializer-core/src/project/lyrics.rs:267-278`; TSV
import likewise), so a cue shorter than 1 ms — from a hand-edited `.musi`,
external tooling, or an aligner — is loadable, and selecting it panics the whole
process on the next render of the timing row. The C oracle used sequential `if`s
and merely produced a slightly out-of-range value.

**Fix:** order-independent arithmetic (`value.max(0.0).min(...)` shaped so it
cannot invert), and/or a minimum gap in `validate_cue`/import so the state is
unrepresentable. Add the sub-millisecond cue as a regression fixture.

### 1.2 Closing a panel mid-drag permanently freezes every control

`widgets::button()`/`slider()` claim a press into the shared `active_button_id`
and clear it only when the *same id* is drawn again on a release frame
(`widgets.rs:190-219`, `:483`); `begin_frame` never resets it. `Shell::keyboard`
runs before panels are drawn and `T`/`F` toggle the inspector/fullscreen
unconditionally (`shell.rs:706-708`, `:749-751`). Hold a Tune slider, press `T`
(or `F`): the slider is never drawn again, the claim never releases, and **every
button and slider in the app is dead for the rest of the session**
(`shell.rs:425-431`).

**Fix:** one-line safety net — clear `active_button_id` in `begin_frame`
whenever the physical mouse button is up. The same root cause produces a minor
sibling: a splitter drag suspended by fullscreen fires a spurious
`SaveUiPreferences` on resume (`shell.rs:432`, `:1320-1339`); clear `split_drag`
on entering fullscreen.

### 1.3 Switching tracks mid-edit silently applies one track's lyric draft onto another track's cue `[C2]`

`Shell.lyrics` is a single editor shared across tracks, remembering a cue only
by bare `id: u64`; ids restart at 1 in every document, so track A's cue #3 and
track B's cue #3 are unrelated but collide. The Tracks-row click pushes
`SelectTrack` with no draft guard (`shell.rs:1570-1572`), `select_track`
switches immediately (`main.rs:2665-2689` — the comment there names the guard
hook that was never wired), and the deferred `LyricsEdit::Update` then applies
against whichever track is current at flush time. Apply **overwrites a lyric
the user never opened**; with mismatched track lengths it instead fails with a
raw `OutOfRange`. The existing `allow_context_change` guard only runs for
in-panel navigation.

**Fix (feeds C2):** gate `SelectTrack` (and panel/scene/project/quit context
changes) on `has_unsaved_draft`, and give the draft an owning track identity —
mirroring `RouteEditorState::track_slot` — so a stale binding is
unrepresentable, not merely guarded.

### 1.4 The Export panel can draw literally nothing — and did, in both shipped captures

`export_panel` returns before drawing anything when its box is ≤ 80 px
(`export.rs:143`). The box works out to `band.height - ~197`, so the panel needs
a ~277 px band — but `resolved_timeline_height`'s Export floor is **260**
(`shell.rs:357`), ~17 px short. Any user who has ever dragged the timeline
splitter smaller (legal in other panels, persisted globally to
`~/.config/musializer/ui.json`) gets a lit-up Export button, a blank white band,
and no path to an MP4 — across restarts. Both committed captures
(`panel-export-1280x720.png`, `panel-export-960x640.png`) show exactly this,
and the headless gate passed anyway because it greps the `panel: export` report
line rather than measuring ink.

**Fix:** derive the Export floor from the same constants the panel consumes
(never a second literal); where the box genuinely cannot fit, draw the
Assist-style one-line explanation (`assist.rs:1051-1073` does this right)
instead of returning silently — the repo's own "missing beats pretending" rule.
Make the gate measure ink inside the panel rect (peak-luma, as the tooltip gate
already does).

### 1.5 Non-Latin lyrics render as `?????` in the exact editor meant to edit them

The UI atlas admits only Latin + punctuation ranges
(`crates/musializer-runtime/src/font.rs:745-751`), while the caption face
carries the full curated set (699 vs 1784 glyphs in the run reports). The
seeded fixture proves it on screen: the preview typesets
"Καλημέρα κόσμε — мир продолжает звучать" perfectly while the cue list and the
editable text field one panel below show rows of `?` with only the em dash
surviving (`lyrics-selected-1280x720.png`). A user writing Greek, Cyrillic,
Hebrew or CJK cannot read, proofread, or meaningfully place a caret in their own
lyric — every cue row looks identical. Track names degrade the same way.

**Fix:** draw project-authored strings (cue list, bound text field, track names)
through the already-rasterized caption atlas, keeping the narrow UI face for
chrome; ellipsize instead of hard-clipping rows. Add a headless assertion that a
seeded non-Latin cue produces no U+003F in the editor pane.

### 1.6 Typing in the font search box still fires Space/F/M/H/T/Tab/seek shortcuts `[D7-adjacent]`

`text_entry_has_focus()` checks only the lyrics cue field (`shell.rs:1208-1210`
— its own comment admits the gap). The font browser's `query_active` flag
(`fonts.rs:317`) is unknown to it, so filtering fonts for "Space Mono" toggles
playback, fullscreen, mute, the HUD, the inspector, and seeks the transport.

**Fix:** include the browser's query state in the focus predicate; D7's planned
"text entry suppresses every global shortcut" test should enumerate *all* text
surfaces, not just the lyrics field.

### 1.7 Tune never says whether a slider edits the base scene or one cue's snapshot

`cue_settings_active` silently redirects every slider edit and Reset into a
cue's captured snapshot whenever the playhead sits on a cue
(`workspace.rs:432-462`), and **no UI surface reads it** — the Tune header shows
only the scene name (`tune.rs:257-266`). With automatic scene plans now real,
this is a scope surprise on every edit.

**Fix:** one badge in the Tune header — "Editing: Cue 3 (0:42)" vs
"Editing: base scene".

### 1.8 "Reset scene" silently does nothing to routed rows

The `ResetScene` handler resets settings only (`main.rs:1235-1241`); routed rows
keep displaying the routed effective value (`tune.rs:342-345`), so the click
produces no visible change and no message. Compounding it: Reset is a single
unconfirmed click, while the *less* destructive preset Delete two inches away
has an arm/confirm step.

**Fix:** notice ("3 routed settings were not reset") or clear routes too; give
Reset the same arm/confirm pattern `[C3 will make it reversible anyway]`.

### 1.9 The transport's RMS meter impersonates a seek bar, and telemetry ignores the HUD flag

A green fill bar sits exactly where every media player puts progress, beside the
timecode, and is not clickable (`shell.rs:1011-1055` — draw only, no widget id);
at t=0.15 s it reads ~22 % full. Users will click it and read it as position.
Above it, "104 bands  peak 18  rms 0.299" renders unconditionally even with the
HUD explicitly off (`hud-forced-off.png`) — the one place the project decided
telemetry should not appear.

**Fix:** either make that rect the actual scrub bar (level as a thin overlay) or
restyle it so it cannot read as progress; move the bands/peak/rms caption under
the HUD flag.

### 1.10 Two raw `Debug` leaks in user-facing messages (one-line fixes)

- Lyric-edit refusals print `{error:?}` — the bare variant name (`NotAdjacent`,
  `OutOfRange`) — while `LyricsError` already derives proper thiserror sentences
  that `Display` would render (`main.rs:1152-1156` vs
  `crates/musializer-core/src/project/lyrics.rs:35-55`).
- Project open with invalid lyric data wraps `format!("{error:?}")` into
  `ProjectError::Build` (`project.rs:539-542`) while the sibling lanes two lines
  below correctly use `.to_string()`.

### 1.11 Notice tray: illegible severities, unwrapped text, six-second errors `[D8]`

Three confirmed defects that D8's rebuild must cover, with specifics worth
keeping:

- **Contrast:** severity labels use light-chrome palette colors on the
  near-black card (`shell.rs:2091-2110`): INFO 1.69:1, ERROR 3.22:1 — all below
  AA; an error is indistinguishable from an info at a glance. `theme.rs`'s
  contrast tests never assert against the dark overlay — add dark-surface
  variants to `rgba` so the sweep sees them.
- **Overflow:** detail text draws unwrapped, unclipped, on a ≤380 px card
  (`shell.rs:2074-2130`); several real detail strings run 90–150+ chars.
- **Persistence:** all 51 `notify` call sites hard-code
  `persistent: false, 6.0 s` (`shell.rs:306-315`) — including "Export failed
  while finishing". The queue already supports persistence correctly
  (`notice.rs:342-348`); `persistent: true` appears only in a unit test. A
  20-minute export that fails while the user is away leaves a normal-looking
  screen. Derive persistence from severity.

### 1.12 Opening a bottom panel deletes the Tracks panel — and Save with it

At 960x640 with Assist open (and 1280x720 with the sheet expanded) the sidebar
starts at SCENES: the tracks panel, Save, Save As, Open project and the current
track's name are gone with no marker (`workspace_layout.rs:279-292`,
`TracksPanelMode::Hidden`). Since Ctrl+S is not yet bound `[D7]`, that state has
**no route to saving at all** except closing the panel — which nothing suggests.
The intermediate state clips the selected track row in half mid-glyph
(`panel-lyrics-1280x720.png`), so the one indicator of *which track you are
editing* is illegible exactly where editing happens (rows are clipped-not-
skipped by documented policy, `shell.rs:1472-1476`; the policy is right, the
sliver outcome isn't).

**Fix:** give Hidden a collapsed form (current track name + save affordance) or
move Save into the toolbar; require a minimum legible row fraction before
drawing one.

### 1.13 Assist: the reason Apply is blocked is painted underneath the Copy buttons `[C2]`

The blocking explanation draws at the same x/y where
`assist_artifact_actions` then draws its opaque 36 px buttons
(`assist.rs:1702-1730`, `widgets.rs:393-395`) — so precisely when Apply is
greyed and the user needs the reason, the reason is covered. Related process
gap: `--ui-probe assist=` accepts only `confirm`, so the Candidate/Running/
Failed bodies — the panel's most consequential surfaces — have **never been
photographed** (`cli.rs:1301-1303`, capture set is ready/confirm/missing/sheet
only). This overlap bug is exactly the class that hides there.

### 1.14 Overlapping lyric cues silently swallow a line

Overlaps are legal and `at_time` resolves to "most recently started"
(`lyrics.rs:652-670`); clamps ignore neighbours, and `Add cue` defaults to a
2 s block at the playhead, so accidental overlaps are easy. The shadowed cue
simply never renders — in preview or export — while the lane draws both blocks
identically. A user proofreading sees a line missing with nothing pointing at
why.

**Fix:** distinct lane styling for shadowed spans + a one-line warning in the
form; clamp `Add cue`'s default end to the next cue's start.

---

## 2. Workflow friction — where a real user gives up

### Saving and trust `[C1]`

Nothing anywhere says whether work is saved. `project_dirty` is maintained and
read by exactly two consumers: the quit modal and the probe report
(`main.rs:2451`, `:2883`). No dirty dot on the track row, no Save-button state,
no title text — the answer arrives only *after* the user decided to quit. The
cheapest continuous feedback: a dot on the track row + accent Save button when
`has_unsaved_work()`; both hooks already exist.

### The lyric timing loop is missing its loop

The editor's foundation is genuinely good (real text field, hit-tested lane,
honest draft model) — but timing 60 lines is ~360 precise clicks across two
panes:

- **No seek-to-cue** (row click selects, never seeks), **no key to stamp a
  time**, **no auto-advance** after Apply. Worse, `begin_new` focuses the text
  field and a focused field stands down *all* transport keys including Space —
  the natural add→type→play→tap loop breaks at the tap
  (`lyrics.rs:1226-1228`, `:393-405`; `shell.rs:694-699`).
- **No undo anywhere**, and Delete is one unconfirmed click; a 3 px accidental
  lane drag commits a 64-cue `ShiftMany` with no way back
  (`lyrics.rs:1508-1527`). The arm/undo pattern already exists for manual-event
  clear (`event_timeline.rs:400-468`) — copy it, or keep inverse edits and bind
  Ctrl+Z.
- **Nudge is a fixed 0.1 s click** with no hold-repeat, no Ctrl/Shift ladder —
  while the transport two rows above *teaches* exactly that ladder — and times
  shown to the millisecond cannot be typed
  (`lyrics.rs:1614-1627`, `widgets.rs:202-217`). Also: the form clamps a cue to
  1 ms minimum while the lane enforces 20 ms — the two should agree
  (`lyric_lane_edit.rs:50-55`).
- **Split and merge are fully ported, tested, and unreachable** — no call site
  in the app (`lyrics.rs:508-520`, `:558-566`). Splitting a long imported line
  at the playhead is the most common subtitle-editor operation; today it means
  retyping eight cues.
- **At 100+ cues**: wheel-only scrolling (the drawn scrollbar claims no widget
  id), no keyboard nav, no jump-to-playhead, rows hard-clip at ~48 % width
  (`lyrics.rs:1231-1246`, `:1140-1147`).
- **Draft state is invisible** — no dirty marker on the form; the refusal
  arrives as a toast over the preview telling the user to "discard" when the
  button says "Cancel edit" (`lyrics.rs:1067-1077`, `:1457-1461`) `[D8]`.

### Tune and routing

- The route affordance — the app's signature capability — is a **26x20 button
  labelled `~`**, rendering as a 5 px dash that reads as a decrement button. No
  tooltip: `widgets::hint` exists but is wired only to the toolbar and
  transport (`tune.rs:317-333`, callers of `hint` at `shell.rs:963`, `:1121`).
  The oracle gave this editor six tooltips, including the dynamic "why Apply is
  greyed" explanation (`plug.c:6169-6184`, `:5822-5860`) — none ported. Port
  the strings; give `~` a real glyph.
- No typed values, no wheel/arrow nudge, no per-setting reset anywhere
  (`widgets.rs:438-493` — drag-only slider), and Band selection among 104 is one
  click per band (`tune.rs:657-703`). For a parameter-heavy creative tool these
  are table stakes: click-to-type on the readout, wheel-over-slider, and
  wheel/scrub on the band stepper. `[D6 covers scrolling/toggles — this is the
  rest]`

### Assist

- **The model's reasoning is parsed, bounded, then thrown away.** `SECTION`
  reasons, per-cue semantic summaries, and up to 16 KiB of `SEMANTIC_NOTE` —
  the literal thing "MiMo feelings" promises — survive to
  `AnalysisCandidate::prepare` and are dropped; the review screen can only say
  "Scene changes: 0 → 8", so Apply is a blind decision
  (`analysis_bridge.rs:127-154`, `analysis_candidate.rs:145-243`,
  `assist.rs:1591-1639`). Rendering these in the staged-review body is the
  cheapest change with the biggest trust payoff.
- **"3 timing cues need review" — but never *which* three.** Per-cue
  `uncertain`/confidence flags exist in the bridge and die at the candidate
  boundary; `LyricCue` has no such field, and the docs make this flag the
  centrepiece of the timing policy (`analysis_bridge.rs:112`, `:193-196`;
  `docs/ASSIST_PIPELINE.md:126-130`). Persist it and tint the flagged cues.
- **Apply is all-or-nothing and irreversible**: one Apply replaces lyrics,
  sections, and events wholesale, then drops the candidate — no per-lane
  checkboxes, no undo (`analysis_candidate.rs:290-316`, `assist.rs:512-523`).
  A curious "Full assist" click costs a hand-tuned scene plan. Lane checkboxes
  + a pre-apply snapshot with "Undo apply" in the success notice.
- **A 40-minute job reports a mm:ss clock and nothing else** — the panel
  literally prints "No percentage is reported"; helper stdout goes to
  `/dev/null`, stderr to a log the app never reads
  (`assist.rs:1403-1411`, `process/assist.rs:261-266`). One
  `STAGE\t<name>\t<n>/<total>` line per phase tailed into the status line
  separates "working" from "wedged" `[E2]`.
- **Failure hands the user a clipboard *path*** instead of the cause the log
  already contains; no Retry, no Open (`assist.rs:363-375`, `:1738-1791`).
  Tail the log's last lines into the body `[D8]`.
- **Remote modes arm with no credential check** — a keyless user waits minutes
  for a generic failure; the consent copy covers privacy but not prerequisites
  or cost (`assist.rs:242-264`) `[E2]`. And "MiMo" is never explained as a
  model-via-OpenRouter anywhere in visible strings
  (`assist_ui_state.rs:56-92`).
- **Missing-helper state is inert**: one sentence, no path, no remedy, while
  `missing_support_files()` and `musializer_doctor.py` both exist unwired
  (`assist_ui_state.rs:260`, `support.rs:26-50`) `[E4]`.
- The panel's permanent **amber border reads as a warning state** and its amber
  selection outline contradicts the accent-fill selection language used
  everywhere else (`assist.rs:1047`, `:1239`). Reserve amber for staged/missing
  states.

### Discoverability

- Four bindings have no button and therefore no tooltip home: Tab/Shift+Tab
  scene cycling, Ctrl+=/-/0 UI scale, End, double-click-splitter-to-reset
  (`shell.rs:663-748`, `:1301-1313`). The scale one bites hardest: a 1440p user
  who finds the UI small has no discoverable control at all. A `?` keymap card
  is one static panel `[D7]`.
- At 2560x1440 the app looks *least* finished on the best monitor: three
  simultaneously empty regions (tracks void, ~500 px inspector gap, dead
  timeline column) while the same panels clip at 960x640
  (`ui-scale-150-2560x1440.png`). Let surplus height feed row height or show
  the preset slots.

---

## 3. Opportunities, ranked by leverage over effort

The striking pattern: the four best product wins are **UI seams onto machinery
that is already built and tested**.

1. **Clip export (in/out range).** `RenderPlan::resolve` + `--render-window`
   already do windowed renders end-to-end; the panel hard-codes `None`
   (`main.rs:1283`, `render_job.rs:145-190`, `cli.rs:424-433`). Two time fields
   (or in/out markers) + threading `Some((start, duration))` through
   `StartRender`. Iterating on a chorus currently costs a full-track encode.
2. **Vertical & square output.** `Resolution::ALL` is four 16:9 presets; the
   category's distribution surface is Shorts/Reels/9:16
   (`render_export.rs:36-57`). Store explicit width/height (don't extend the
   C-ordered enum; keeps `.musi` round-trip safe); scenes already take a
   viewport, so the work is a per-scene tall-frame audit — checkable by
   capture, not by eye.
3. **The lyric tap-timing loop** (see §2) — `[`/`]` stamp start/end at the
   playhead and advance; double-click a row to seek. Turns the app's hardest
   manual workflow from ~360 clicks into play-and-tap.
4. **Preset audition / A/B.** `PresetAction::Apply` overwrites with no stash —
   trying a preset is a one-way door (`main.rs:2333-2343`). Hold-to-audition
   and an A/B snapshot toggle are pure `SettingsSnapshot` capture/apply, no
   persistence needed.
5. **Scene thumbnails.** Ten dramatically different scenes are ten words in
   text tiles (`shell.rs:1585-1650`). Scenes are deterministic given
   (scene, seed, frame) and the offline render target exists — a cached
   ~150x84 still per scene makes the app explorable in one glance.
6. **Recent files on the welcome screen.** The right column holds a decorative
   "01" and nothing else (28 % of the width); the per-user preferences store
   already exists (`preferences.rs:15-24` — bump to v2, the reader denies
   unknown fields). Highest-value content on the screen, removes a modal from
   every launch.
7. **Randomize/mutate in Tune.** Every setting has validated bounds in the
   descriptor table; `scene_seed` exists. "Surprise" + "Nudge" buttons over the
   same audition/stash affordance as #4 — the MilkDrop shuffle appeal for free.
8. **Project-level palette ("this track is teal").** Nine scenes have nine
   differently-named hue/saturation controls; automatic scene switches read as
   palette changes. A track-level Look mapped onto existing descriptors as a
   pre-route offset.
9. **Cover-art/logo layer.** The content-addressed asset bundling is
   category-general but only ASCII Field uses an image
   (`model.rs:540`, `assets.rs:33-60`). A track-level backdrop/badge drawn by
   `scene_host` reuses it verbatim and becomes a route target for free.
10. **Still-frame export.** "Save frame…" from the offline supersampled target
    — cover art, thumbnails, and the scene-browser stills of #5 are the same
    operation (`export.rs:1069-1090`).

---

## 4. Verification blind spots this review exposed

Feed these into G1 — each let a shipped defect pass a green gate:

- **The panel gate greps report lines, not pixels.** `panel: export` passed
  while the panel drew nothing (§1.4). Measure ink (peak luma in the header
  rect) as the tooltip gate already does.
- **Assist's consequential states are unphotographable.** `--ui-probe assist=`
  accepts only `confirm`; Candidate/Running/Failed have never been in a frame —
  and §1.13 hid exactly there. Add `assist=candidate|running|failed` probes.
- **Non-Latin text is seeded but never asserted on.** The fixture deliberately
  seeds Greek/Cyrillic; no check notices the editor rendering `?` (§1.5).
  Assert zero U+003F in the cue-list pane for the seeded fixture.
- **Contrast tests only cover light surfaces** (§1.11). Put the dark-overlay
  severity pairs into the palette table the sweep walks.

---

*Review date 2026-08-03. Worktree and captures retained at
`../rusty-musializer-uxreview` (delete with `git worktree remove` when done).*
