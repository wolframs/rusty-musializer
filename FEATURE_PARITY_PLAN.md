# Rusty Musializer Feature-Parity Plan

This is the **single authoritative task list** for reaching feature parity with
the frozen C Musializer. If another document, source comment, old agent handoff or
ignored worktree names unfinished work, reconcile it here before acting on it.

`REWRITE_PLAN.md` is the historical design and session record. Its NOTE ENTRIES
remain valuable evidence, but its phase plans and agent handoffs are no longer a
live queue. `docs/PHASE0_INVENTORY.md` is a behavioral inventory of the C oracle,
not a task list. `AGENTS.md` contains repository rules and deliberate exclusions.

`UX_PERSPECTIVE_REVIEW.md` is the evidence document for a committed user-facing
review, not a second task list. Its complete set of confirmed defects, workflow
findings, opportunities and verification blind spots is normalized as **UX0**, the
next task below. The review's numbered sections retain the detailed evidence;
UX0 is authoritative about whether each item is still open.

## Target and meaning of parity

- **Oracle:** `../musializer`, branch `master`, frozen commit
  `9300af942bd00d8c85fc4e3c8c02cf2b6356764f` (`9300af9`). It is read-only.
- **Audited Rust baseline:** `a664b7d`, 2026-08-01.
- **Baseline evidence:** `tools/verify.sh` is 19 passed / 0 failed: thirteen
  differential harnesses plus the headless capture gate.
- **Feature parity means observable capability parity.** A C workflow must be
  available in Rust and its persistent or rendered result must retain the same
  meaning. Mechanisms, ownership, UI composition and implementation structure may
  improve when the result is equivalent or better.
- **Code-detail equality is not a goal.** Do not reproduce global state, hot
  reload architecture, tinyfiledialogs, unsafe pointer shapes or C source layout
  merely because the oracle uses them.
- **A difference is a parity gap** when it changes a `.musi` file, rendered video,
  analysis number, documented command, user workflow or durable edit.
- **The C is legacy (operator decision, 2026-08-03).** See `AGENTS.md` — the
  rewrite supersedes the C; formats may evolve past it with a `schema_version`
  bump, harnesses pin kept behaviour rather than forbid change, and this plan
  remains the queue only for capability parity a user would otherwise lose.

### Deliberate exclusions

These do not block parity unless the operator changes the target:

- Microphone capture (`MUSIALIZER_MICROPHONE`)
- Hot reload, including functional `--reload-once`
- Non-Linux platforms, including Windows, macOS and OpenBSD; this is a
  Linux-first rewrite

Exact pixel identity, C source organization and reproducing known C defects are
also not goals. Any new deliberate product-level exclusion must be approved and
added both here and to `AGENTS.md`.

## Current fit

The following are already delivered and must stay green:

- All ten scene drawing implementations and deterministic scene state
- Audio playback, analyzer and beat tracker
- Per-scene settings, routes, route persistence and preset model/UI
- Multi-track workspace and `.musi` open/save with asset verification
- Bidirectional C/Rust `.musi` differential round trips and frame-lane boundary
  selection: 2,550 values, delta 0
- FFmpeg export, deterministic full/windowed rendering and publication
- Lyrics editor, caption-style editor and local/imported font runtime support
- Assist UI/state/controller and analysis-bridge import
- Manual event capture, clear/undo and scene-cue model
- Whole-track waveform, Song Atlas map and image-backed ASCII derivations
- All bottom-panel layouts and the headless scene/panel capture sweep
- Native-size Space Grotesk shell typography with compiler-enforced draw and
  measurement boundaries

The remaining work is concentrated at integration boundaries and missing product
entry points. A green pure-module harness does not prove those boundaries.

## Operator-requested UI scale and workspace resizing

Added 2026-08-01. This is a product extension rather than a parity gap: scale
1.0 and every automatic split must retain the frozen layout, while larger UI
scales and user-sized splits are Rust-side capabilities.

- [x] Introduce an explicit logical UI coordinate space, stepped UI scale and
      deterministic command-line/probe override. Keep scene/export pixels
      independent from shell scale.
- [x] Rasterize Space Grotesk at the physical sizes each scale needs while
      measuring and laying out in logical units; do not magnify the 1x atlases.
- [x] Map pointer and scissor coordinates through the same scale boundary as
      drawing, and report the effective scale in headless evidence.
- [x] Add draggable sidebar, Tune-inspector and bottom-panel splitters. Preserve
      the existing content-aware layout as Auto, clamp every user split against
      the preview and panel floors, and reset a split by double-clicking it.
- [x] Persist scale and split preferences per user, outside `.musi`; a corrupt
      preference file must survive unread and must not be overwritten.
- [x] Keep all existing 1x gates green and add silent headless captures at 125%,
      150% and a 2560x1440 window, including font, hit-target, clipping and
      splitter evidence.

Completed 2026-08-01. `tools/verify.sh` is 20 passed / 0 failed. The private-Xvfb
gate covers 100%, 125%, explicit 150%, 1440p Auto selecting 150%, custom logical
splits, and a scaled tooltip hit target; the three new layout frames were also
inspected at original resolution. Every UI font report used a scale-matched
native atlas with zero non-native requests. The scene and export paths remain in
framebuffer pixels outside the shell camera.

## LX1 — lyric cue provenance and a workable cue lane (operator request, 2026-08-06)

Opened from four defects the operator hit on `You Can't Get Me.mp3`, all of them
one problem seen from different sides: **after an assist run the lane cannot tell
you what it knows.** Every block is the same amber whether a human placed it or
an aligner guessed at it, lines the localizer failed to place do not appear at
all, overlapping cues collapse into one rectangle, and the only surface that
named the failures showed three rows and pointed at a file with no way to open
it.

| id | Work | Where | State |
| --- | --- | --- | --- |
| LX1-a | `CueOrigin` on `LyricCue` — user applied / AI certain / AI ambiguous / potential — filtered out of `at_time` and `cue_shadow`, persisted as an optional `origin` field, promoted to *user applied* by every editing operation | `core::project::lyrics`, `core::project::io`, `core::project::analysis_bridge` | **done** |
| LX1-b | Overlap row assignment and clipboard arithmetic as raylib-free policy | `core::ui::lyric_lane_stack`, `core::ui::lyric_clipboard` | **done** |
| LX1-c | The zoom/`Zoom out` row moves below the cue lane; `open_panel` reports where it goes | `ui/shell.rs`, `ui/panels/lyrics.rs` | **done** |
| LX1-d | Lane rendering: per-origin colours, immediate non-sticky tooltips, overlap fan-out, a 2.25x lane resizable to 3x with a drawn grip, Ctrl+C/X/V | `ui/panels/lyrics.rs`, `ui/preferences.rs`, `ui/widgets.rs` | **done** |
| LX1-e | One visual system across the scene lane, waveform and cue lane — matching borders, insets and gaps | `ui/shell.rs`, `ui/panels/scene_timeline.rs`, `ui/theme.rs` | **done** |
| LX1-f | Unresolved and abstained lines become `Potential` cues at their coarse proposal, so the gap at 0:41–1:18 is editable; review list scrolls and can reveal its artifact | `ui/panels/assist.rs`, `core::project::analysis_candidate`, `runtime::process::reveal` | **done** |

**The affordance nothing photographed.** The lane's resize grip was drawn inside
`lane_resize_gesture`, which runs *before* `widgets::fill(d, lane, ..)` — so it
was painted over in every frame the application has ever rendered, hover or not.
The gesture worked, the clamp was right and `cargo test` was green; the operator
reported the resize as simply not working. `tools/lyric_lane_capture.sh` now
locates the lane by its seams and asserts two centred grip lines above its bottom
edge, and asserts their *absence* on a window with no range to drag. Negative
control: forcing `resizable` to `false` fails it. This is the "a surface nothing
photographs does not get reviewed" rule catching a second instance.

**The literal that drifted.** `Shell::resolved_timeline_height` clamped a
persisted split against a hard-coded `381.0`, commented as "121 chrome + 10
bottom + 22 lane + 5 gap + the form's 223 px". Every one of those five numbers
moved in LX1, so an operator who had once dragged the band short kept a band 33
px too small across restarts and the editing form drew "Enlarge the window to
edit a cue." on a 1378 px-tall window. It asks `LyricEditor::minimum_band_height`
now, which is built from the same `lane_chrome` the band request and the resize
ceiling use.

**A limit worth recording.** The lane's drag ceiling is derived from what the
editing form (223 px) and the sidebar floor (301 px) still need, so 66 px is only
reachable from a 1080p window up; at 720p the ceiling computes to the 33 px base.
That is the correct trade — the panel yields before the sidebar does — and it was
checked rather than assumed: rebuilding with the pre-LX1 22 px lane gives the
*identical* `tracks Hidden` at 1280x720, so the taller lane costs the tracks panel
nothing.

**A gate that was wrong about correct drawing.** `tools/timeline_lane_alignment.py`
first reported "0 seams" on every capture. The lanes were right; the detector
demanded one unbroken run of trough colour, and the seam deliberately carries the
tick columns and the playhead through it (`Shell::timeline_group_chrome`), so a
1240 px seam with eight ticks in it had no run longer than about 155 px. It counts
pixels now, which is immune to being interrupted. Negative control: shifting the
cue lane 2 px right yields `lane3 playhead columns (11, 13) != lane1 (11,)`;
reverted byte-for-byte, and all twelve captures agree again on `x=10..1269` at
720p, `x=10..949` at the minimum window and `x=15..2034` at 1440p/150 %.

**The decision worth recording.** `schema_version` does **not** move for LX1.
`model.rs` accepts exactly one version, so bumping it would make this build
refuse to open every project it has already written — which is the compatibility
contract we actually have. `origin` is therefore an optional field with a
documented default, the mechanism `io.rs`'s module comment reserves for exactly
this, and the same one `caption_style.effects` used. A cue nobody has marked
serializes byte-for-byte as it did before, which is why
`differential_project_io.sh` is still 2550 values with a delta of 0.

**LX1-f's own decisions**, recorded because a later session will otherwise
relitigate them:

- **Proposals are parked at staging, not at Apply.** `stage_lyrics_review` is the
  one seam both a finished job and `--ui-probe assist=candidate` go through, so
  the lane the panel counts is the lane Apply publishes. Two insertion points
  would be two chances for "Lyrics: 0 → 52" to stop describing what Apply does.
- **A proposal with a start and no end gets 3.0 s**
  (`assist::PROPOSAL_DEFAULT_SECONDS`). `LYRIC_MIN_CUE_SECONDS` is 0.02 s, which
  the model accepts and no user can hit with a mouse; the point of parking a
  proposal is to give them something to grab.
- **A proposal with no time at all, or one starting inside the decoder's 0.25 s
  padding tail, is not parked** and stays a review row. Clamping the second into
  the track would claim a position it never made *and* fail `normalize_duration`
  at Apply, breaking the placements around it.
- **Parked proposals persist to `.musi`.** They are project content of a new kind
  — an unanswered question the aligner asked — and the alternative (a session-only
  overlay) would silently discard the work list on quit. `origin` already
  round-trips, `at_time` and `cue_shadow` already skip them, and any edit promotes
  one to `UserApplied`, so the persistence costs nothing observable in a frame or
  an export.

**LX1-e's system**, likewise. The operator's phrase was "three separately
designed elements glued together", and the measurements confirmed it: at
1280x720 the scene lane sat 4 px under its controls row inside a 1 px box, the
waveform strip's box started on the *very next row* below it (rows 375 and 376,
so two rules drew as one 2 px line), and the cue lane sat 5 px lower with a top
rule, no sides and no bottom. Three playheads crossed them at two weights with a
break in every gap.

- **Three numbers, in `theme::metric`.** `LANE_BORDER` 1.0, `LANE_GAP` 5.0,
  `LANE_PLAYHEAD_WIDTH` 2.0, plus `rgba::UI_LANE_TROUGH` for the seam. The gap
  is 5 rather than 6 because `panels::lyrics` already spends exactly 5 and its
  `LYRIC_EDITOR_TIMELINE_CHROME` assertion forbids that band growing; a
  `const _: () = assert!` in `shell.rs` pins the two together, because a
  disagreement there paints a seam over the cue lane and fails nowhere.
- **The trailing gap is spent inside `SCENE_SECTION_HEIGHT` (54 → 60).**
  `timeline_height` adds that constant to whatever the open panel asked for, so
  a gap inside the section grows the band with it; a gap added below the section
  would come out of the lyrics editor instead.
- **`Shell::timeline_group_chrome` draws what no single lane can** — the seams,
  one frame around all of them, and one playhead. The frame is what gives the
  cue lane its missing left, right and bottom edges, so `panels::lyrics` needed
  no edit at all.
- **The upper seam is bounded by the lane that ended, not by `LANE_GAP`.** The
  first version used the constant, and the negative control for a 3 px gap then
  *passed* — the seam had simply painted over the lane's bottom border. Deriving
  it from `SCENE_LANE_OFFSET + SCENE_LANE_HEIGHT` makes a wrong gap show as a
  wrong seam.
- **`tools/timeline_lane_alignment.py` is the contract**, run over ten captures
  by `headless_check.sh`. It reads each lane's outermost frame column and its
  playhead columns and requires them equal, because a lane inset by a different
  amount maps the same second onto a different column and *nothing else here can
  see it* — everything inside a lane moves together, so the frame still looks
  self-coherent. It found two real defects on its first run: the tick at the
  view's last visible second painted a rule one column outside the lane's right
  border, and the tick at its first did the same on the left at 150 % scale.
  Both bounds are now inset by `LANE_BORDER`.
- **Its negative control was a 2 px inset on the waveform strip.** That moved
  the lane's left edge 10 → 12, moved the group frame with it, and split the cue
  lane's playhead into two columns (149 against 150–151) because
  `panels::lyrics` still drew its own marker at the true position. All **1301**
  unit tests stayed green throughout, which is the usual measurement: a
  property assertion cannot pin a pixel.

## LX2 — the lane says what it can do (operator feedback, 2026-08-06)

Three findings from using LX1 on a real track. The first arrived as a
correction: *"I have a user defined cue here now, and apparently since I moved
it and it became user defined, I cannot pull on its start/ending anymore"*,
retracted a moment later — *"It's zoom level tied. When I zoom in a little, I
could see the drag hover begin and ending"*. The retraction is the finding. The
handles worked the whole time; they were unfindable, and unfindable is the same
bug the resize grip had, in a different place.

| id | Work | Where | State |
| --- | --- | --- | --- |
| LX2-a | Both grab bands on any hovered block, at the hit test's own width, the pointed-at one solid, plus a `RESIZE_EW` cursor and `RESIZE_NS` over the lane's own edge | `ui/panels/lyrics.rs`, `ui/shell.rs` | **done** |
| LX2-b | The cue's own words inside its block, through the authored face, fading out before the edge | `ui/panels/lyrics.rs`, `ui/widgets.rs` | **done** |
| LX2-c | The wheel zooms from the scene-plan and lyric lanes, not only the PCM strip | `ui/shell.rs`, `ui/panels/lyrics.rs` | **done** |
| LX2-d | Capture evidence for all three: `--ui-probe wheel=`, a `timeline:` report line, and four new measurements in `tools/lyric_lane_capture.sh` | `cli.rs`, `main.rs`, `tools/lyric_lane_capture.sh` | **done** |

**A cursor was unavailable to every panel, and the comment saying so was the
fix.** `Shell::splitters` runs after every panel and called `set_mouse_cursor`
unconditionally, `None => MOUSE_CURSOR_DEFAULT`, so a shape asked for anywhere
else was overwritten in the same frame. `Shell::request_cursor` now records one
and `splitters` honours it when no splitter is active. The request is
**last-wins**, which is deliberately the opposite of the widget bank's
first-wins press rule: a press is consumed and must go to one claimant, while a
cursor is a property of the topmost pixel and is not used up.

**The wheel claim is first-wins, for the opposite reason.** One notch is
reported to *every* caller in the frame, so two lanes accepting it would
multiply the factor — one notch over an overlap would zoom 1.44x where it zooms
1.2x everywhere else.

**How faint is too faint was measured, not judged.** The first version drew the
offered (not-yet-pointed-at) band at 14 % ink, which the capture reported as 16
luminance steps under the block's own fill — under the 22 the check demands, and
visibly nothing. It is 32 % now, which reads as 35. The point is that this was
found by a measurement rather than by looking at a screenshot and feeling
satisfied, which is how it got shipped at 2 px in the first place.

**The rows are nested, not tiled, and the labels found it.** `row_geometry`
gives every row a band that runs to the *bottom* of the lane and starts partway
down the one above, so three centred labels land within a few pixels of each
other and overlap — which the first version did, visibly. A label belongs in the
strip nothing is drawn over, which is `next_row_offset - this_row_offset`. Doing
that needed a datum `LyricStack` did not expose: `rows()` is the deepest pile in
the *document*, so an isolated cue in a document that also holds a four-deep pile
was being squeezed into a strip that covers nothing. `LyricStack::cluster_rows`
answers locally, and the label only draws when its own strip is at least 16 px.
The result is a property worth keeping: **drag the lane taller and stacked cues
gain their labels** — at 50 px a three-deep cluster is colour and tooltip only,
at 66 px all three read.

**Four new measurements, and each one has a demonstrated negative control:**

| check | perturbation | result |
| --- | --- | --- |
| the grab bands exist and darken under the pointer | offered band back to 14 % ink | `drew no grab band (2px / 2px)` |
| block text is present and fades | `fade_width` forced to 0 | tail ink 62.7 against a head of 63.4, and the text reached the block edge |
| the lane's wheel reaches the shared view | the lane's region test forced false | `two notches over the cue lane gave 1.000x, not 1.44x` |
| the tooltip never covers the lane | (existing) — its "any near-black pixel" proxy now had to become a *run* of 40, because LX2-a legitimately puts near-black bands inside a hovered block | — |

**`tools/timeline_lane_alignment.py` was reading the whole window row.** It
searches for the playhead by accent colour across the full width, while its
border search is correctly restricted to the lane. With the Tune inspector open,
that inspector's accent sliders and live meters sit at the same heights as the
scene lane and were counted as playhead columns — so the check failed on a frame
whose lanes were perfectly aligned, and, because a meter moves with the audio,
it failed only sometimes. Now restricted to the lane's own span; the control
(rolling one lane 3 px sideways in a passing PNG) still reports all three
disagreements.

**Still uncovered.** `--ui-probe`'s invented keys — `hover`, `sidebar`,
`inspector`, `timeline-height`, `audio-stall`, `route`, `picker`, `tune`,
`assist`, and now `wheel` — are documented only in `cli.rs`'s own doc comments.
`docs/PHASE0_INVENTORY.md` §3.6 tabulates the *oracle's* grammar and has never
listed ours. That is a documentation gap for whoever writes a capture script
next, not a behaviour gap.

## LX3 — the scene plan owns the scene while it is on (operator bugs, 2026-08-06)

Three reports from splitting scenes on a real track. Two of them are **one
cause**:

> *"Applying scene tuning settings to a scene selected after a split applies the
> tuning settings to ALL SCENE SPLITS OF THAT SCENE KIND … (also I'm not sure
> which one gets the tuning applied to)"*
>
> *"I put a scene split at the very start of the track, and had other scene
> splits already in, but every time I selected a new scene, it still told me
> '{scene} was selected as base scene'"*

`Track::select_base_scene` sets `scene_switches.enabled = false`
(`workspace.rs`, from `track_select_base_scene`, `plug.c:963-977`). So every
scene click switched the running plan off. The preview then showed one scene for
the whole track, and — with no cue driving — `App::settings_mut` fell through to
the track-wide `scene_settings`, which is per *scene kind*. That is exactly what
"applies to all splits of that kind" looks like from the outside, and the
base-scene notice was the same bug announcing itself.

| id | Work | Where | State |
| --- | --- | --- | --- |
| LX3-a | An interactive scene selection retargets one segment and leaves the plan driving; `--scene` still sets the base scene | `main.rs`, `ui/shell.rs` | **done** |
| LX3-b | The Tune header names the blast radius: a driving segment, a paused plan, or a segment with no captured tuning | `ui/panels/tune.rs` | **done** |
| LX3-c | `--ui-probe scene-pick=`, a `scene segments:` report line, and the `retarget` capture | `cli.rs`, `main.rs`, `tools/headless_check.sh` | **done** |
| LX3-d | `EndTextureMode` leaves rlgl's framebuffer size at the halo blur buffer's, so `SceneViewport` narrows to a tiny rect | `runtime/halo.rs`, `runtime/draw.rs` | **done** |

**Which segment a click lands on, and why not the selected one for tuning.** The
retarget prefers the segment selected in the lane and falls back to the one under
the playhead, because a lane selection is the more recent statement of intent and
is the only way to edit a segment you are not listening to. Tuning deliberately
does *not* follow the lane selection: a slider you cannot see the effect of is
worse than one bound to the wrong segment, so tuning stays on the segment that is
rendering and the header says which that is.

**A retarget to the scene a segment already uses is refused, not performed.**
`retarget_scene_cue` recaptures the snapshot from the track-wide table, so
"changing" a Pentagram segment to Pentagram would silently throw away tuning the
user had captured into it.

**The scene browser's `id != input.scene` guard had to go, conditionally.** It is
a correct no-op guard for a base-scene choice and wrong for a retarget: the
segment selected in the lane is very often not the one playing, and giving it the
live scene is a legal edit the guard swallowed in silence.

**LX3-d is a raylib asymmetry, not our arithmetic.** `BeginTextureMode` sets
`rlSetFramebufferWidth/Height` (`rcore.c:1079-1107`); `EndTextureMode` restores
the viewport and `CORE.Window.currentFbo` through `SetupViewport`
(`rcore.c:1109-1131`, `:3537`) and **never** restores the rlgl pair. So after any
preview-path caption glow, `rlGetFramebufferWidth()` reports the blur buffer's
size for the rest of the session. The only readers here are `HaloBlur` and
`draw::SceneViewport`, and the latter takes it as the full framebuffer: it then
either produces a degenerate rect and the scene draws nothing — "the scene panel
only shows a subtle glow" — or narrows, and on drop pins
`rlViewport(0, 0, small, small)` at GL's bottom-left origin, which is the
operator's "UI renders extremely small in the bottom left". One cause, both
symptoms, and it needs a caption glow plus one of the four scenes that use a
viewport (Orbital Lattice, Song Atlas, Spectral Terrarium, Constellation). It was
measured at **188x50 against a 1280x720 window** — a scale factor of 0.15.

**Every one of the four gates here fails on a frame that is a plausible picture
of the passing one, which is why all four read report lines rather than pixels:**

| check | perturbation | what the control printed |
| --- | --- | --- |
| a scene click leaves the plan driving | `SelectScene` routed back to `select_scene` | `auto scenes: disabled (2 cues)`, `scene segments: spectrum, loom` — and `scene: pentagram (Pentagram Orbits)` either way, identically |
| the click lands on one segment | (the same control) | the plan never changed |
| the Tune header names the scope | (the same control) | `Editing: base scene - 2 segments are paused`, which is the exact state the operator was tuning in |
| the GL viewport survives a caption glow | the `halo.rs` restore reverted | `gl=188x50 render=1280x720 mismatched-frames=12 of 12`, exit status 0 throughout |

The last row carries a warning worth keeping: with `SceneViewport` hardened, the
*picture* is right even while `halo.rs` is broken, so a pixel-comparison gate
would have gone permanently green over a live defect. The report line is the only
assertion that still fires.

**Not covered.** The `gl framebuffer:` audit samples in the main loop, so an
export run reports `never sampled` — the export path renders zero frames through
it. `HaloBlur::render`'s render-target branch was already correct (its
reconstructed `BeginTextureMode` sets the pair), which is why exports never
showed this.

## PX6 — Tune becomes a place to play (2026-08-07)

UX0-B08, UX0-B09, UX0-C04 and UX0-C07 landed together because they are one
thought: **Tune is where a user explores, and exploring is only safe if you can
get back.** The C's inspector writes every slider straight into
`scene_settings_set` with no history, so the way back from "let me see what that
preset does" is to remember eight numbers. Every item below exists to remove that.

| id | Work | Where | State |
| --- | --- | --- | --- |
| PX6-a | `core::ui::tune_explore`: snapshot/audition, bounded randomize, typed and stepped values — pure, with randomness injected | `crates/musializer-core/src/ui/tune_explore.rs` | **done** |
| PX6-b | The route affordance is a word, its tip is dynamic, and a disabled action says why | `ui/panels/tune.rs` | **done** |
| PX6-c | Typed value chips, wheel fine step, per-setting reset on the label | `ui/panels/tune.rs` | **done** |
| PX6-d | Audition bar: Nudge / Surprise, then A/B / Revert / Keep | `ui/panels/tune.rs` | **done** |
| PX6-e | `--ui-probe tune-seed=`, `tune-explore=`, `tune-type=`, plus `tune values:` and `tune entry:` | `cli.rs`, `main.rs`, `tools/headless_check.sh` | **done** |

**Post-legacy extensions, recorded as such.** Audition, A/B, Surprise and Nudge
have no counterpart in the frozen C and are not parity work. They change nothing
a `.musi` file can express — an audition is session state and is never
serialized — so no schema version moves.

**The one change a file can see, and it is deliberate.** A slider drag now writes
a value snapped to the descriptor's own `precision`, where the C only did that for
`precision == 0`. So a two-place slider stores 1.23 instead of 1.2345, and a
`.musi` written after a drag can differ in the low decimals from one the C would
have written. It is deliberate because the readout *prints* two places: before
this, the number on screen was a rounded picture of the number in the file, a
typed 1.23 and a dragged "1.23" were different values, and an A/B claiming
bit-exactness over values the user could not see the ends of would have been
theatre. `differential_settings.sh` covers the descriptor table, not slider
output, and stays green untouched.

**Why an explicit A/B rather than hold-to-audition.** The plan text offered
either. A held button cannot be compared against *while it is held* — you get one
look, then it is over — and what a user actually wants is to flip back and forth
with the track playing. So the snapshot is taken automatically by the first
exploratory gesture (nobody has to remember to press "save first"), and the flip
is a button that can be pressed as many times as it takes.

**How Surprise is biased, and why each bias is there.** Uniform-over-the-range
produces results nobody keeps, so four rules narrow it, each measured by a test
rather than asserted in prose:

| bias | value | why |
| --- | --- | --- |
| snap to the descriptor's precision | — | the readout must not lie about what was rolled |
| keep clear of both ends | 5 % of the span | hard endpoints *are* the degenerate looks: zero density, zero glow, zero link weight, everything at maximum |
| move only some sliders | 75 % | re-rolling all twelve of Song Atlas's controls gives a scene the user cannot recognise as the one they were tuning |
| flip a toggle rarely | 25 % | `atlas.wireframe` and `atlas.hue_motion` are whole-scene character switches, not variations |

Sliders draw **triangular about the descriptor default**, so the scene's designed
character is the centre of mass and the tails are rare. The exception is the five
`-180..180` angle controls (`pulse.hue`, `orbital.hue`, `atlas.color`,
`atlas.orbit`, `pentagram.hue`), which draw **uniform**: hue is circular, there is
no "designed centre" to pull toward, and pulling toward the default would make the
one control a user most wants shuffled the one that barely moves. The carve-out is
read off the bounds (`minimum < 0 < maximum`), not off a list of keys, so a new
angle descriptor gets it without being enumerated — and it reads the descriptor
contract rather than changing it. A test measures the effect: hue's mean magnitude
is >50 degrees, where a triangular draw about 0 would give ~27.

**Does Surprise produce keepable results?** Often, on the evidence of the seeded
sweeps — it stays inside every bound over 200 seeds x 10 scenes, keeps its hands
off a quarter of the controls so the scene stays itself, and never lands on an
endpoint. What has *not* been done is a human sitting with a real track judging
whether the pictures are good, and no automated check can stand in for that. It is
tuned to be worth pressing; whether it is worth pressing twice is an operator
call.

**What the two probe families each prove, and why both.** `--ui-probe click=`
presses one control per run and is the only thing that proves a control is wired
at all — EX1's lesson, where three export SIZE buttons were dead with every other
gate green. It cannot state a claim about a *sequence*, and every UX0-C04 claim is
one: explore, compare, come back. `--ui-probe tune-explore=` runs the sequence.
The gate cross-checks them: the same seed through the Surprise **button** and
through the sequence probe must produce byte-identical `tune values:`, so neither
family can drift into exercising code the other does not reach.

**`tune values:` prints shortest-round-trip floats, not the readout's two
places.** "Revert restored it exactly" and "Revert restored it to two decimals"
are different claims, and only the first is worth making. Two different `f32` bit
patterns cannot print the same string in that form, so string equality in the gate
*is* bit equality. A unit test pins the distinction by constructing two values one
ulp apart that `{:.2}` cannot tell apart.

**A pre-existing layout defect, fixed on the way past.** The row loop measured
each setting row against the panel's content rectangle alone, while "Reset scene"
is pinned to the panel floor — so on a twelve-control scene at the 960x640
minimum the last row and the "+N more (enlarge the window)" notice drew
*underneath* the button. That was already true; adding 48 px of audition bar is
what made it reachable at 720p as well, so the list now stops above the button.

**Not covered, and left deliberately.**

- **B09's "efficient navigation across all 104 bands"** is the route editor's
  band stepper, not the descriptor list, and it wants a typed band number and a
  jump-to-peak. Left out: it is a different control with a different failure mode,
  and folding it in would have made this change span two features.
- **Keyboard nudging** is the wheel only. Arrow keys over a hovered row would need
  a focus notion the inspector does not have, and the arrows are already the
  transport's fine seek — a binding conflict that deserves its own decision rather
  than being resolved silently here.
- **D6's scroll/toggle items** were not taken; nothing here blocked on them. The
  list still truncates with a notice rather than scrolling.
- **A capture of the *typed* keystrokes.** Xvfb has no keyboard, so `tune-type=`
  exercises the parse/clamp/write path a keystroke would reach, not the keystrokes
  themselves. The suppression of global shortcuts while typing is covered by
  `shell.rs`'s `TextEntrySurface::ALL` sweep instead, which is the test that
  already exists for exactly this class of bug (UX0-A06).

## EX1 — the export panel's SIZE row could not be clicked (operator bug, 2026-08-06)

> *"there's a bug with the export UI buttons: I can't pick different sizes"*

**A widget-id collision, and the id table was the thing that was supposed to make
it impossible.** `panels/events.rs` declared `EVENT_ROW_NAMESPACE: u32 = 7` and
`PRESET_NAMESPACE: u32 = 8` as bare literals, with a comment saying a leaf agent
does not edit `widgets.rs` and that they would be folded in at merge. They were
not. `widgets::id::EXPORT` is `7` and `widgets::id::SEEK` is `8`.

The manual event row draws before the export panel, so on the release frame
`+ Feel`, `+ Scene` and `+ Custom` — ids 0, 1 and 2 — matched
`active_button_id`, took the release, set `clicked = hovered` (false, the pointer
was down in the export panel), and zeroed the claim. By the time the SIZE row was
drawn there was nothing left to cash. **2160p is index 3, has no counterpart in
that row, and worked**, which is why the symptom reached the operator as "I can't
pick different sizes" rather than as a dead panel. The preset picker's `8`
aliased the transport row's fine-seek group and scrub bar the same way.

| id | Work | Where | State |
| --- | --- | --- | --- |
| EX1-a | `--ui-probe click=XxY`, an `export config:` line and a `click probe:` line naming what claimed the press | `cli.rs`, `main.rs`, `ui/widgets.rs` | **done** |
| EX1-b | One allocation table (`widgets::id::ALL`); every panel-private namespace becomes an alias of an entry in it | `ui/widgets.rs`, `ui/panels/{events,lyrics,assist}.rs`, `ui/assist_settings.rs` | **done** |
| EX1-c | The gate presses five export controls and one gap between two of them | `tools/headless_check.sh` | **done** |

**Hover could not have caught this, and had been checked.** `--ui-probe hover=`
lit the 720p button correctly at 100 % and at 150 %, because the highlight comes
from the same `contains_point` that was never wrong. Only a press separates "this
control is under the pointer" from "this control receives the press". That is the
whole reason `click=` exists, and it is the same argument that produced `hover=`
and `wheel=` before it.

**Three collision tests were green throughout.** `widgets.rs`, `events.rs` and
`lyrics.rs` each had one, and each enumerated a hand-written list of namespaces —
different lists, all stale, none containing the pair that actually collided.
`widgets.rs`'s had six entries and predated `EXPORT` and `SEEK` existing. So the
table is now `widgets::id::ALL`, beside the constants rather than inside a test,
`events.rs` asserts *membership* in it rather than re-deriving disjointness, and
a second test names every constant individually so that adding one to the module
without adding it to `ALL` fails with the name in the message.

**The negative control is a click into the 8 px gap between two buttons.**
Without it every assertion in the new gate section is satisfied by a probe that
silently pressed nothing, since the default configuration is what a no-op leaves
behind. It asserts `claimed=nothing` and an unchanged `export config:`.

**What `click=` cannot reach, deliberately.** The press is injected at `Widgets`'
own pointer seam, not at the device — raylib exposes no way to synthesize a
button and Xvfb has none. So it drives everything that goes through
`Widgets::button`/`::slider`, which is every button in the shell, and does not
drive the gestures that read raylib directly: the timeline scrub, the cue drags,
the middle-button pan. Those own their own state machines and need their own
probe. It also holds the press for three frames after three settling ones,
because the claim rule takes a press on the press edge and cashes it on the
release edge — a probe that did both in one frame would report every working
control as broken.

## EX3 — Master rendered thin bright detail *worse* than Balanced (operator report, 2026-08-06)

> *"when I run exports on Master quality, I can't claim that the export quality
> is particularly high … the black levels + detail isn't always 'Master' quality"*

**The supersample downscale was averaging in gamma space.** High and Master render
into a 2x target and then resolve. That resolve was `Image::resize`, whose 8-bit
fast path calls **`stbir_resize_uint8_linear`** (`rtextures.c:1770-1773`). In stb's
naming `_linear` means *the input is already linear light*; the variant that
decodes sRGB first is `stbir_resize_uint8_srgb` (`stb_image_resize2.h:8009`). Our
frames are sRGB-encoded. Averaging encoded code values as though they were light
darkens every high-contrast edge, because the transfer function is convex.

Measured on one white output pixel against the app's own `0x151515` clear at 2x,
integrating linear light across the profile:

| resolve | profile | integrated light |
| --- | --- | --- |
| none — what Balanced does at 1x | `255` | 0.9925 |
| gamma-space Mitchell — what shipped | `22 51 205 51 22` | **0.6618** |
| linear-light box — `core::render::resolve` | — | 0.9925 |

So the tier that supersamples was losing **a third of the light in every thin
bright feature** relative to the tier that does not. On a real Spectrum frame at
1080p Master the glow edges move by up to **+77 code values** (an edge pixel goes
`[80,103,93]` → `[124,175,157]`), 59 k pixels get brighter and 53 k get darker —
the darker ones being Mitchell's ringing overshoot, which a box average does not
produce.

| id | Work | Where | State |
| --- | --- | --- | --- |
| EX3-a | `core::render::resolve`: an integer-factor box average in linear light, with sRGB decode/encode tables | `core/render/resolve.rs` | **done** |
| EX3-b | The export step resolves through it, in one pass instead of four | `ui/panels/export.rs` | **done** |
| EX3-c | `decode::image_pixels_rgba8` — borrow the readback instead of `get_image_data`, which is the third unchecked `LoadImageColors` wrapper | `runtime/decode.rs` | **done** |
| EX3-d | An `export pipeline:` report line and a tray warning when supersampling silently falls back | `ui/panels/export.rs` | **done** |

**It is also faster.** The old path was four full-frame passes — `load_image`,
`Image::resize`, `get_image_data`, and a per-pixel rebuild of the byte vector.
The new one is a readback and a single resolve. 60 Master frames at 1080p:
**19.8 s against 30.0 s**, same debug build, same machine.

**A factor of 1 is a byte-for-byte copy, deliberately.** Balanced never
supersamples and must be unchanged by this module existing; a round trip through
the two tables would move a handful of codes for nothing. Pinned by a test.

**Measured and rejected.** Each of these was a plausible fix that the numbers
killed, and they are recorded so nobody re-derives them:

| candidate | result |
| --- | --- |
| `-color_range pc` (the "crushed blacks" theory) | The current output is **already correct**: `ffprobe` reports `color_range=tv` with the full bt709 description, and full range measured *worse* (37.50 dB against 37.54) and bigger. It also flips `pix_fmt` to the deprecated `yuvj420p`. Left alone |
| `yuv420p10le` | Worse, not better — 36.65 dB against 37.54. The source render target is RGBA8 (`rtextures.c:4250`), so 10 bits carry nothing. The gradient banding is created in the framebuffer, before FFmpeg sees it |
| `yuv422p` | Buys nothing here: it subsamples horizontally only, and this application's thin features are predominantly vertical |
| `-aq-mode 3` (dark-biased) | Sounded right for a near-black scene, measured **worse**: 37.39 against 37.54 |
| `-crf 8` | +0.23 dB for **+41 % bytes**. A placebo |
| `-tune animation` / `stillimage`, `-sws_dither ed` | +0.10 dB at best; `-sws_dither ed` is bit-identical |
| `yuv444p` + `high444` | Genuinely 8 dB better and 3 % *smaller* — a 1 px cyan line decodes as `(97,227,225)` today. **Operator declined, 2026-08-06:** High 4:4:4 Predictive has no hardware decoder on any phone, TV or browser, and every tier stays universally playable |

**Still open, and deliberately not done.** The gradient banding is real and is
*not* the codec: a 0→40 ramp round-trips through `yuv444p` at 0.41 RMS, so the
contours are in the frame handed to FFmpeg. `LoadRenderTexture` is RGBA8, the
halo's ping-pong buffers are RGBA8, and every blend is gamma-space with no
`GL_FRAMEBUFFER_SRGB` anywhere in `rlgl.h`. Fixing it means an RGBA16F export
target through `rlLoadFramebuffer`/`rlFramebufferAttach`, which is a new `unsafe`
island and needs the driver checked for `rlFramebufferComplete` first. Worth
doing after EX2, and worthless before EX3-a, which is why it waited.

**Gap named rather than closed:** `tools/headless_check.sh` exercises the export
*panel* and never a single export *frame*'s pixels. Every finding above lives in
a path no gate photographs. The `export pipeline:` line is the first thing that
reports from inside it.

## EX4 — "rendering sometimes introduces little hiccups" (operator report, 2026-08-06)

**The exported file cannot be corrupted by a timing hitch**, and that was worth
establishing rather than assuming: `scene_delta`, `scene_time`, the sample cursor
and `draws_this_frame` all derive from `frame_index` (`render_job.rs:472-521`),
the frame write is a blocking `write_all` so a full pipe backpressures the
application rather than dropping a frame, and a transport failure sets
`transport_ok = false` which refuses the publishing rename. The stderr-deadlock
classic is also impossible here — `ffmpeg.rs:425` inherits stderr rather than
piping it, so it cannot fill.

**What the report could not see was a preview stall between 85 ms and 170 ms.**
The scratch buffer is 4096 frames (~85 ms) and the ring is 8192 (~170 ms), and
`main.rs` raises raylib's output stream buffer to 8192 as well. A stall inside
that band desynchronizes the picture from the music while `output underruns:` and
the ring's `dropped` counter **both stay at zero**. Nothing printed anything.

| id | Work | Where | State |
| --- | --- | --- | --- |
| EX4-a | `frame budget:` — the worst frame in the run, which frame it was, and how many stalled | `main.rs` | **done** |
| EX4-b | `peak ring fill` on the `audio frames:` line | `main.rs` | **done** |
| EX4-c | A gate section that forces a 120 ms stall and asserts the two new figures fire *while* the old ones stay clean | `tools/headless_check.sh` | **done** |

**The threshold is 25 ms, not 16.7 ms.** `set_target_fps(60)` lands a healthy
frame within a float hair of one period, so a `> 1/60` test reported **118 of 120
frames over budget** and said nothing — which is what the first version did. One
and a half periods is the point past which the *next* frame cannot be presented
on time, so it is a dropped frame rather than jitter.

Measured against `--ui-probe audio-stall=`, which is the existing bounded hook:

| forced stall | `frame budget:` | `dropped` | `underruns` | `peak ring fill` |
| --- | --- | --- | --- | --- |
| none | worst 16.7 ms, 0 of 120 stalled | 0 | 0 | 18 % |
| 60 ms | worst 69.3 ms, 1 stalled | 0 | 0 | 41 % |
| 120 ms | worst 129.6 ms, 1 stalled | **0** | **0** | **82 %** |
| 200 ms | worst 209.3 ms, 1 stalled | 5 | 0 | 100 % |

The 120 ms row is the gate's, and it asserts `0 dropped` and `underruns=0` on
purpose: if that run ever stops being indistinguishable from a healthy one on the
old lines, the new ones have stopped being the only evidence.

**Suspects the lines are now instrumented to convict, in order.** None is
confirmed — they need the operator's own numbers from a real track:

1. **Autosave on the main thread.** `save_project_to` runs synchronously in the
   frame loop 1.5 s after any edit, and its path hashes the **entire bundled
   source audio** with SHA-256 (`project_files.rs:280`) and then does two `fsync`s
   (`publish.rs:284`, `:303`). A 50 MB WAV cold plus a contended filesystem is
   comfortably past 170 ms. Fires once after an edit, then goes quiet — which
   matches "sometimes… rare" better than anything else found.
2. **Song Atlas's whole-track decode**, which *deliberately* pauses the stream on
   the first frame that scene draws (`main.rs:2059-2107`). Not a hiccup, a
   designed multi-second gap — but the interface says nothing while it happens.
3. **The at-size caption atlas rebuild.** One codepoint not yet seen bumps
   `Coverage::generation`, which invalidates every cached atlas (`font.rs:478-499`,
   `:942-944`) and forces a 10–30 ms rasterize. Plain-ASCII lyrics never trigger
   it; a typographic apostrophe does, once, in the frame it first appears.

**Two resource paths silently change the exported pixels**, both found here and
one of them fixed as EX3-d: the supersample fallback (now a tray warning and a
report line) and a *cached* mid-export caption atlas failure (`font.rs:979-985`),
which would make the second half of a single MP4 blurrier than the first. The
second is recorded, not fixed.

**Not done.** The export progress screen runs the blocking frame write inside its
own `begin_drawing`/`EndDrawing` pair (`export.rs:926-957`), so while the encoder
is backed up neither Escape nor Cancel is polled and the screen does not repaint.
At 4K `-preset slow` that is a visibly frozen window. It reads as "the export
hung" and is a fair candidate for the operator's complaint; moving the write
outside the pair is the fix.

## EX2 — vertical and square exports (operator request, 2026-08-06)

> *"I think we once talked about different export formats (e.g. for mobile, 1:1,
> besides the standard 16:9)"*

**The pipeline was already aspect-agnostic and nobody could tell.**
`RenderExportConfig::validate` accepts any even geometry from 16x16 to
7680x4320, and `--resolution 1080x1920` rendered a correct vertical MP4 before a
line of this was written. What was missing was a way to *ask* for it — and, once
asked, two scenes that composed for a landscape frame and did not say so.

Thirty renders, ten scenes at 1920x1080 / 1080x1920 / 1080x1080, every frame
looked at. Nothing letterboxed, nothing stretched, **no circle became an
ellipse** — every radial primitive is either min-axis-derived or drawn square.
Two surfaces were genuinely broken and the rest merely composed differently.

| id | Work | Where | State |
| --- | --- | --- | --- |
| EX2-a | `Aspect` — 16:9, 9:16, 1:1, 4:5 — and an ASPECT row beside QUALITY | `core/timing/render_export.rs`, `ui/panels/export.rs` | **done** |
| EX2-b | The caption sizes and margins from the frame's **short** edge, and its line cap preserves text area rather than line count | `scenes/caption.rs`, `core/project/caption_layout.rs` | **done** |
| EX2-c | ASCII Field's live grid is fitted to the frame instead of a fixed 80x42 | `core/scenes/ascii_field/ascii_art.rs`, `scenes/ascii_field.rs` | **done** |
| EX2-d | The preview is framed to the export's aspect, with a visible edge, and says so in `preview frame:` | `main.rs`, `core/ui/workspace_layout.rs`, `ui/theme.rs` | **done** |

**The rung names the short edge.** That is the one reading that makes "1080p"
mean the same amount of picture in every shape: at 16:9 the short edge is the
height and the answer is the C's own 1920x1080, so every existing preset is
byte-identical; at 9:16 it is the width and the answer is 1080x1920, which is
what every vertical platform calls 1080p. A rung and an aspect are independent —
pressing 2160p on a vertical export stays vertical — and both are asserted.

**The caption was the real bug, and it lost content rather than looking wrong.**
Font size came from `boundary.height` and the wrap width from `boundary.width`
(`plug.c:1219-1307` does the same, correctly, because its four presets are all
16:9). At 9:16 that is 78 % larger type inside a 3.1x narrower measure, against
a hard three-line cap: a seeded 158-character cue rendered in full at 16:9 and
stopped mid-word at 9:16, losing roughly 60 %. Three changes, and each preserves
landscape exactly:

- every fraction is taken of `min(width, height)`, which *is* the height on any
  landscape frame, so 16:9 is unchanged as an equality rather than as a claim;
- the line ceiling preserves **text area** instead of line count — three at 16:9,
  six at 9:16/1:1/4:5, floored at the C's three so no shape can ever fit less
  than the oracle did. The epsilon on its `ceil` is load-bearing: `3 * (16/9) *
  1080 / 1920` is exactly three in real arithmetic and lands either side of it in
  `f32`, and a hair above would give every existing project a fourth line;
- the plate is clamped inside the frame. That last one is **not** an aspect bug —
  `box_width` was capped against the boundary but never against the margin, so an
  authored `margin_scale` near its 0.400 ceiling put an edge-anchored plate partly
  off-canvas at 16:9 too. 9:16 only made it easy to hit.

**ASCII Field drew a 1004x527 band in a 1080x1920 frame — 28 % of the height.**
Its live grid was a fixed 80x42 of square cells, min-fitted and centred. It now
derives the cell edge from the short axis and takes as many as fit:
`cell = min(w, h) * (16/9) / 80`, `columns = round(w / cell)`,
`rows = round(h * (42/45) / cell)`. The `42/45` is the fill fraction the C's own
grid produces at 16:9 — written as a fraction rather than as `0.93333`, which
lands on `41.99985` and rounds to 42 only by luck. **The 1920x1080 MP4 is
byte-identical before and after**, same md5, not "looks the same".

Both axes clamp at 96 rather than rows at 54. The 54 ceiling looked mandatory —
`spectrum_history` is a fixed `[[f32; 96]; 54]` — and is not: `history_row = row *
MAX_ROWS / draw_rows` is a *rescale*, always below 54 for `row < draw_rows`, and
`density()` is `get`/`get` returning 0.0 out of range. Checked at every indexing
site. Clamping rows at 54 fills 63 % of a 9:16 frame against 87.5 % uncapped,
which defeats the fill fraction the formula exists to preserve.

**21:9 letterboxes on purpose.** Past 96 columns the cells can no longer be
`cell` wide and fill the frame; widening them instead would put every glyph over
its horizontal neighbour, and the scene's whole read is a monospaced terminal. A
symmetric letterbox at an aspect nobody exports to reads as framing; overlapping
glyphs read as a fault.

**The preview is now framed to the export, which is a visible change at every
aspect including 16:9.** With a bottom panel open the preview band is routinely
3:1, and a 16:9 export inside it used to be composed for a 3:1 rect — a picture
the export would never produce. It is now pillarboxed to 16:9, which costs
preview area and buys the truth. The surround alone was not enough and a capture
proved it: Pentagram Orbits at 9:16 is near-black scene against a near-black
surround with an invisible seam, so a one-pixel rule draws the frame edge.

**Measured and left alone.** The other eight scenes compose differently at 9:16
without breaking: Pulse Field, Pentagram and Cadence's ambient state size from
the min axis and leave dead bands; Loom's weave cells stretch; Spectrum's bands
become needles; and the four 3D scenes crop horizontally because raylib's
`BeginMode3D` preserves the vertical FOV (`rcore.c:1032`) — Orbital Lattice
actually fills a tall frame better than a wide one. Only Song Atlas loses its
subject, its terrain slab running off both sides. All shippable, none fixed.

## DX — dev-ex audit follow-ups (codex agent, 2026-08-07, `dc694e3`)

A separate operator-directed codex agent landed `dc694e3`: the public-facing
README rebuild, the self-repairing `docs/CODE_MAP.md` + `tools/code_map.py`
generator, and a parallelized `tools/verify.sh` (bounded four-job pool, 3.17x
warm speedup, Xvfb/audio gate kept serial). Its three audits named these
follow-ups; recorded here so they queue rather than evaporate. None are
user-observable features; most are shared-infrastructure work that PX-wave
surfaces would otherwise reinvent separately.

- [ ] **DX1 — one enabled/disabled widget API** that always retains ids,
      tooltips, focus and press-consumption semantics.
- [ ] **DX2 — consolidate wrapping/ellipsis policy** behind a
      measurement-closure API.
- [ ] **DX3 — shared row/pixel scrolling policies** before Tune overflow (D6)
      and large lyric navigation (UX0-B06) each grow their own.
- [ ] **DX4 — separate text-field input from rendering** so ordinary, modal,
      path and eventually secret fields share keyboard behaviour safely.
- [ ] **DX5 — centralize XDG/user-directory resolution** and bounded
      optional-file reads.
- [ ] **DX6 — collision-safe RAII test scratch directories** before allowing
      simultaneous whole-repository verification runs. **And displays:** the
      PX wave proved the gap the hard way, 2026-08-07 — four concurrent
      headless gates produced 11 `rlglInit` SIGSEGV coredumps (null GL pointer
      at `InitWindow` when two runs collide on one Xvfb display number and one
      tears the server down mid-init), each popping a DrKonqi notification on
      the operator's desktop. `headless_check.sh` should allocate a free
      display atomically (flock over a display-number pool, or derive from
      PID and verify the socket) instead of trusting a fixed or hand-picked
      number. The EX4 frame-stall gate is also load-sensitive under sibling
      verify runs (0 stalls quiet vs 41 at load 14.6) — a wave's final
      verify belongs on an idle machine until DX6 lands.
- [ ] **DX7 — purge stale agent-era ownership/handoff language** (88 Rust files)
      and enable strict rustdoc (currently 17 broken-link warnings). Overlaps
      F2; close them together.
- [ ] **DX8 — the "authoritative" support manifest is wrong:** it omits current
      runtime dependencies including `lyric_anchor_block.py`,
      `anchor_block_align.py`, `runtime_inventory.py`, `provider_catalog.py`,
      `codex_model_discovery.py` and `atomic_cache.py`. This is an E1 defect
      and an E2/E4 blocker — an extracted distribution built from that manifest
      does not run Assist. Fix the manifest and add a check that fails when a
      helper imports a file the manifest misses.
- [ ] **DX9 — next verification speed tranche:** content-addressed cached C
      harness executables, prebuilt Rust differential examples, and Python unit
      tests split from support integration smoke.

## Start here next session (updated 2026-08-07)

The assist workstream (`d132a3e`..`ed7cb86`) is closed, and so are the operator
rounds of 2026-08-06/07: LX1–LX3 (cue provenance, the lane's affordances, the
plan surviving a scene pick) and EX1–EX4 (the click probe and the id-collision
fix, aspect presets, the linear-light Master resolve, the stall instrumentation)
— commits `8a5175c`..`9db6684`. UX0-A (all 14 confirmed defects) and UX0-D (all
four blind spots) are fully closed. `tools/verify.sh` is 21 passed / 0 failed
and the tree is clean. What is open, in the order a session should pick it up:

| # | Work | Where | Size |
| --- | --- | --- | --- |
| 1 | **MiMo v2.5 capability benchmark** — operator's stated priority. Design and harness in `docs/MIMO_BENCHMARK_PLAN.md` + `tools/mimo_bench/`. Needs an explicit operator go-ahead: it sends audio to OpenRouter and spends credits | new | one session |
| 2 | **UX0-B (19 items) and UX0-C (9 remaining)** plus the C1–C5 durable-edit tranche — the product backlog; suited to a disjoint-ownership agent fan-out | this document | the bulk |
| 3 | D1–D8 missing entry points and timeline affordances, then E2–E4 (prove the copied bundle runs), then F/G honesty and gates | this document | large |
| 4 | AP5-a/b/c/d, AP6-e — modality-loss invalidation, no-network-hang test, diagnostics bundle collector, clipboard canary, deferred discovery decisions | AP5, AP6 tranches | small each; c is a feature |

Two standing facts a new session needs: the operator's OpenRouter key lives in
`~/.config/musializer/credentials.json` (the repo `.env` is legacy CLI-only and
the desktop path refuses it), and `xiaomi/mimo-v2.5` is `experimental` in the
suitability overlay, so **Show experimental** must be on for the MiMo lane to
be routable until a benchmark earns it a `recommended` row.

## Ordered completion map

**UX0 is the next task** once the items above are dispatched. Its work may be divided across agents where ownership
does not overlap, but the complete review is one closure tranche and no named
item may be silently dropped. When UX0 closes, resume the remaining A-G work in
dependency order. P0 establishes the shared render path on which later assertions
depend. P1 makes authored project behavior real. P2 closes missing C workflows.
P3 packages the external product. P4 removes false status and proves the result.

```text
NEXT user-perspective closure
  UX0-A confirmed defects -> UX0-B workflow friction -> UX0-C opportunities
                                                   -> UX0-D verification gates

P0 project-aware frame path
  A1 frame lanes -> A2 lyric overlay -> A3 integration evidence
                  -> B1 automatic switching -> B2 cue semantics -> B3 evidence
                                               -> B4 interactive control

P1 durable editing
  C1 dirty marking -> C2 draft/context guards -> C3 reset workflow
                   -> C4 all-track autosave -> C5 project-preset adoption

P2 missing entry points
  D1 drop dispatch -> D2 ASCII image UI
  D3 lyrics TSV import/export
  D4 timeline information, pan and transactional seek
  D5 CLI execution semantics
  D6 Tune reachability/toggles -> D7 keyboard workflows -> D8 actionable notices

P3 complete external bundle
  E1 support-file manifest -> E2 Assist -> E3 Google Fonts -> E4 dist/doctor

P4 honesty and gate
  F0 robust shell typography -> F1 stale UI/status removal -> F2 stale handoff cleanup
  G1 integration gates -> G2 final feature audit
```

## NEXT — UX0: close the complete user-perspective review

**Source and scope:** `UX_PERSPECTIVE_REVIEW.md`, reviewed at `d22f9de` while
the D4 scene-timeline work was deliberately excluded. Revalidate every code
pointer against current `HEAD` before editing. This task includes the **entire**
review: all 14 confirmed defects, every named workflow recommendation, all ten
product opportunities and all four verification blind spots.

UX0 deliberately includes operator-requested product work beyond strict C parity.
Keeping it here is intentional: this repository has one live queue, and the
review must not become a competing roadmap merely because some recommendations
extend rather than reproduce the oracle.

Confirmed defects, workflow failures and verification blind spots require an
implemented fix plus proportionate evidence. Product opportunities are accepted
into this tranche as real work, not a parking-lot list. An item can leave UX0
only as implemented, merged into a more specific task below with its UX0
acceptance criteria preserved, or explicitly deferred/excluded by the operator
with the reason recorded here. Existing C2/D6/D7/D8/E2/E4/G1 tags are dependency
links rather than duplicate queues; closing the older task does not implicitly
close the UX0 item.

### UX0-A — confirmed defects

- [x] **UX0-A01 — sub-millisecond lyric cue panic:** make timing-row arithmetic
      total for every loadable cue, align the model/editor minimum-gap policy and
      add a regression fixture that previously panicked (`review` 1.1).
      Done in `e650c76`: total `clamp_form_start`/`clamp_form_end` in core,
      form and lane unified on `LYRIC_MIN_CUE_SECONDS = 0.02`, loadability
      unchanged, panic shape pinned by regression tests.
- [x] **UX0-A02 — stranded pointer ownership:** release a widget claim when the
      physical button is up even if its original panel disappeared; clear or
      reconcile splitter drags across fullscreen and test both paths (1.2).
      Done in `a1aafa5`: `release_stranded_claim` in `begin_frame` (previous-
      frame pointer only, so no click can be eaten), splitter drags abandoned
      and scrubs completed on fullscreen/inspector, 4 negative controls.
- [x] **UX0-A03 — cross-track lyric draft corruption [C2]:** bind drafts to their
      owning track and guard every context switch so a stale cue id cannot update
      another document; cover Apply, Discard and Cancel (1.3).
      Done in `2cb8385`: edits stamped with `owner_slot` at push and refused at
      drain on mismatch; SelectTrack/panel/open/add-audio guarded, scene change
      deliberately not (same document), quit folded into confirm_close. C2
      should build on `lyric_draft_allows_context_change`.
- [x] **UX0-A04 — blank Export panel:** derive its minimum height from panel
      geometry, draw an explanation when content cannot fit and require visible
      ink in the headless capture rather than trusting a report line (1.4).
      Done in `7e980a9` + the wave-3 commit: `EXPORT_MIN_BAND_HEIGHT` derived
      from the panel's own rows (including the 27 px TIMELINE header the first
      derivation missed — caught by the ink gate, not by the replay test, which
      now models every rect on the real path), both the automatic budget and
      the persisted-split floor consume it, a too-small box draws a named
      notice, and the gate measures ink (darkest pixel 20 vs blank ~247). The
      full control rows are in a default-size capture for the first time.
- [x] **UX0-A05 — non-Latin authored text:** render lyric rows, editable lyric
      text and track names through an atlas that contains the project's glyphs;
      ellipsize deliberately and assert the seeded editor does not substitute
      question marks (1.5).
      Done in the wave-3 commit: `Faces::authored()` serves the caption atlas
      (fallback ladder caption → caption-alt → ui → default, reported), cue
      rows/field/track names all draw through it, rows ellipsize with U+2026,
      and the gate asserts `face=caption missing=0` on the seeded fixture.
      `missing=0` means the bundled repertoire, not every script — Hebrew,
      Arabic and CJK need the imported-face path, pinned by a test.
- [x] **UX0-A06 — font-search shortcut leakage [D7]:** make every text-entry
      surface participate in one focus policy and prove that typing suppresses
      every global shortcut (1.6).
      Done in `a1aafa5`: `TextEntrySurface::ALL` swept by
      `text_entry_has_focus`, focus gated on the surface being drawn, and the
      suppression test enumerates the surfaces so a new one fails until wired.
      D7's keymap work should reuse the enum.
- [x] **UX0-A07 — hidden Tune edit scope:** state whether Tune is editing base
      scene settings or a particular cue snapshot, including cue/time identity
      where applicable (1.7).
      Done in `b73383d`: header badge from `Track::active_cue`, pure
      `tune_scope_label` under test, `tune scope:` probe-report line as
      capture evidence.
- [x] **UX0-A08 — misleading scene reset [C3]:** make routed values explicit in
      reset behavior/notices and give Reset the Confirm/Undo workflow already
      required by C3 (1.8).
      Done in `b73383d`: arm/confirm keyed per (track, scene), routed-count
      notice, routes untouched. Undo itself remains C3's obligation.
- [x] **UX0-A09 — RMS/progress ambiguity:** either make the transport bar a real
      seek control with secondary level indication or restyle it unmistakably as
      a meter; place analyzer telemetry under the HUD flag (1.9).
      Done in the wave-3 commit: the bar is a real scrub control (pause on
      press, one Seek on release, resume; transactional like the timeline
      scrubber; inert grey when `transport_seekable` is false) with RMS as a
      4 px secondary strip, and the bands/peak/rms caption is HUD-gated with
      probe runs keeping it on.
- [x] **UX0-A10 — raw error variants:** replace both user-facing `Debug` formats
      with the existing `Display` messages and test the visible text (1.10).
      Done in `787010a`: both sites use `Display`, `LyricsValidation` gained a
      cue-naming `Display`, and `errors_display_as_sentences_not_variant_names`
      pins the visible strings.
- [x] **UX0-A11 — ineffective notice tray [D8]:** provide dark-surface contrast,
      wrapping/clipping and severity-derived persistence so consequential errors
      remain legible and present (1.11).
      Done in the wave-3 commit: opaque dark card, all severities ≥6.5:1 (the
      old failing pairs kept as a negative control), detail wraps to ≤3 lines
      with honest ellipsis, `Severity::dwell` at the notify seam (Error
      persists until dismissed, Warning 14 s, Info 6 s), per-notice close box.
      D8's rebuild starts from this.
- [x] **UX0-A12 — Tracks/Save disappearance:** retain current-track identity and
      a save route when a bottom panel consumes the sidebar; never draw an
      illegible fractional row (1.12).
      Done in the wave-3 commit: a 26 px collapsed strip (track name through
      the authored face + Save with an explanatory tooltip) where the panel
      vacated, Ctrl+S/Ctrl+Shift+S bound, rows below 60% visibility not drawn,
      a 30-configuration sweep asserts a save route always exists, and the
      `chrome:` report line is the capture evidence. `workspace_layout` is
      untouched — the layout harness still binds.
- [x] **UX0-A13 — obscured Assist refusal [C2]:** keep the Apply-blocking reason
      visible beside artifact actions and add Candidate, Running and Failed probe
      states so consequential Assist bodies are reviewable (1.13).
      Done in `22292e0`: reserved reason slot (beside/above), buttons never
      move, non-overlap swept over 1361 widths, core untouched so the assist-ui
      harness still binds. Probe states landed with it; captures follow in the
      UX0-D02 gate run.
- [x] **UX0-A14 — silently shadowed lyric overlaps:** show overlap/shadow state in
      the lane and form, warn which cue loses display time and clamp a new cue's
      default end to the next cue where possible (1.14).
      Done in `e650c76`: `LyricsDocument::cue_shadow`/`shadowed_cues` in core
      (cross-checked against `at_time`), lane hatching, form warning naming the
      shadowing cue, `shadowed N` in `describe()`, Add-cue default clamped.
      `at_time` unchanged.

### UX0-B — workflow friction and trust

- [ ] **UX0-B01 — continuous save state [C1/D8]:** show durable dirty state on
      the track and Save affordance; distinguish saved, unsaved and save-failed
      state before quit is attempted (`review` 2, Saving and trust).
- [ ] **UX0-B02 — lyric seek/stamp loop:** make cue selection seek-capable, add
      playhead stamping for start/end, auto-advance after Apply and preserve a
      usable play/tap loop while lyric text has focus (2, lyric timing loop).
- [ ] **UX0-B03 — lyric edit recovery:** add undo/inverse edits and safe deletion
      for single and multi-cue timing changes, including accidental lane drags.
- [ ] **UX0-B04 — precise lyric timing controls:** support an established
      fine/normal/coarse nudge ladder, hold-repeat or equivalent efficient input,
      typed times and one consistent minimum cue gap in form and lane.
- [ ] **UX0-B05 — expose lyric split/merge:** connect the already-ported model
      operations to the editor with playhead-aware behavior and tests.
- [ ] **UX0-B06 — large lyric-document navigation:** add visible/interactive
      scrolling, keyboard navigation, jump-to-playhead and readable ellipsis for
      long rows at 100+ cues.
- [ ] **UX0-B07 — visible lyric draft state [C2/D8]:** mark an edited draft and
      use consistent Apply/Cancel/Discard language in controls and refusals.
- [x] **UX0-B08 — understandable route affordance [D6]:** replace or explain the
      ambiguous `~` control, restore useful static/dynamic tooltips and make the
      reason for a disabled route action visible.
      *Done (PX6): the tilde is a word — `Route` / `Routed` / `Editing` — so the
      row is readable with the pointer parked elsewhere, which is what a tooltip
      could never fix. The routed tip is dynamic and names the route it would
      open (`Driven by Band 2 - Smooth - 0.40 -> 2.20 - click to edit`) instead
      of repeating the static sentence. Apply and Remove, when disabled, now
      carry a hit target of their own purely so `hint` can say **why** —
      `disabled_button` returns no `ButtonState`, so a reason had nowhere to
      hang. Capture `tune-base` / `panel-tune-*`.*
- [x] **UX0-B09 — precision Tune controls [D6]:** support typed values,
      wheel/keyboard nudging, per-setting reset and efficient navigation across
      all 104 bands.
      *Done (PX6) for the descriptor list; the 104-band navigation is the route
      editor's band stepper and is **not** included — see the note below.
      Typed values: every readout is a chip, a click opens a `TextField` bound to
      the current number, Enter commits through
      `tune_explore::parse_typed` and Escape reverts. Clamped and snapped to the
      descriptor's own precision, because `SceneSettings::set` **rejects** rather
      than clamps (`scene_settings.c:143-149`) and a raw typed 99 would otherwise
      vanish with no message. Fine step: the wheel over a row moves one precision
      unit, Shift ten — a ratio, so every control crosses the same fraction of
      its range in the same number of notches. Per-setting reset: the label is
      the button, and a leading `*` marks a setting moved from its default, which
      also answers "what have I changed here" at a glance. Global shortcuts are
      suppressed while typing through a new `TextEntrySurface::TuneValue`, swept
      by `shell.rs`'s existing `ALL` test. Gate: `tune-chip-click` (plus a
      claimed-nothing gap control), `tune-typed-{high,low,grid,junk}`,
      `tune-wheel-{up,down,miss}`.*
- [ ] **UX0-B10 — reviewable Assist reasoning:** carry section reasons, semantic
      summaries and semantic notes through the candidate boundary and render them
      before Apply (2, Assist).
- [ ] **UX0-B11 — actionable timing uncertainty:** preserve cue confidence and
      uncertainty identity and visually identify the exact cues needing review.
- [ ] **UX0-B12 — selective/reversible Assist Apply [C2]:** allow per-lane
      inclusion and retain a pre-apply snapshot with a visible Undo action.
- [ ] **UX0-B13 — meaningful Assist progress [E2]:** surface phase/stage progress
      from helpers so long-running work is distinguishable from a stalled job.
- [ ] **UX0-B14 — actionable Assist failure [D8]:** show the useful tail of the
      failure log and provide Retry/Open actions rather than only a clipboard
      path.
- [ ] **UX0-B15 — remote-mode prerequisites [E2]:** check credentials before
      starting, disclose relevant cost/privacy prerequisites and explain the MiMo
      provider/model route in visible language.
- [ ] **UX0-B16 — recoverable missing-support state [E4]:** name missing files,
      provide the support/doctor remedy and make the state actionable.
- [ ] **UX0-B17 — consistent Assist visual semantics:** reserve amber for actual
      staged/warning/missing states and use the normal selected-state language
      elsewhere.
- [ ] **UX0-B18 — discoverable keymap [D7]:** expose Tab/Shift+Tab, UI scale,
      End and splitter reset (plus the full supported set) in a visible keymap or
      equivalent help surface.
- [ ] **UX0-B19 — useful tall-window layout:** use surplus vertical space for
      content such as larger rows or preset slots rather than simultaneous dead
      regions, without regressing minimum-window behavior.

### UX0-C — product opportunities

**Landed 2026-08-04 (operator-requested lyrics/typography batch, first
post-legacy work):** free INK/PLATE colour picking (hand-built SV/hue/alpha
picker beside the swatch rows), the always-visible "Import a face..." route
(`CaptionFormLayout`, form minimum 152 → 160), at-size caption atlases and an
SDF-typeset Cadence replacing the magnified 64 px atlas, the rounded plate
outline, and `caption_style.effects` — audio-drivable glow (RMS/bass/beat/
flux/time pulse and hue drives), soft shadow and authorable plate roundness,
resolved per frame in `core::project::caption_effects` and serialized only when
authored so pre-effects `.musi` files stay byte-identical. Divergence rows in
`AGENTS.md`; captures and luma/chroma gates in `tools/headless_check.sh`
(`lyrics-effects-*`, `lyrics-picker-*`, `fx-glow`, `fx-soft-shadow`).

**Review round, 2026-08-04 (operator played with the landed batch):** all six
landed the same day. The glow is a real two-pass Gaussian over an offscreen
buffer (`runtime::halo`); the backing tuners live beside BACKING on the Style
pane; a hue drive saturates achromatic bases with the drive value; PULSE/HUE
carry a full mapping editor (quiet/loud in→out, curve, clamp, live meter,
transfer graph) built from `ui/mapping_editor.rs` — the same componentry the
Tune route editor now draws with, over the same `evaluate_mapping` semantics —
persisted as optional `pulse_tuning`/`hue_tuning` blocks that leave earlier
files byte-identical; scene routes gained the `time` source; and tooltips
cover the caption panes and the Tune editor. Probe `tune=pulse|hue`, capture
`lyrics-tune-pulse-960x640`; probe runs without `hover=` can no longer
photograph a stray tooltip (the dwell is infinite unless a tip was asked for).

**Items:**

- [x] **UX0-C11 — real glow:** the 17-tap additive glow reads as discrete copies
      ("gravitational lensing") once RADIUS grows, and 100 % GLOW is thin.
      Replace with an offscreen render-texture blur so radius widens a single
      halo; re-measure the fx gate deltas and re-run the zero-strength control.
      *Done 2026-08-04: two-pass separable Gaussian in `runtime::halo` +
      `halo_blur.fs`, composited additively twice; gate glow delta 123 (taps
      measured 125), zero-strength control exactly 0, radius sweep 0.08/0.3/0.6
      → 169/135/95 with one widening halo and no copies
      (`build/glow-evidence/`); glow export deterministic across runs and
      intact inside the supersampled target. Follow-up, same day: the soft
      shadow rides the same blur through `halo_mask.fs` (luminance-as-alpha,
      normal blending); the seeder's soft-shadow fixture is now bright enough
      to measure (soft-vs-hard delta 152, was 3-4), the zero-blur variant
      degenerates to the legacy composition at exactly delta 0 (a standing
      in-gate control). `GLOW_TAPS`, `SHADOW_TAPS` and `GlowTap` are removed
      from `core::project::caption_effects` — both effects now draw from one
      blur, with no tap table left. Evidence under `build/shadow-evidence/`.*
- [x] **UX0-C12 — effects pane grouping:** BLUR/SHADE/ROUND are backing tuners,
      not effects; move them to the Style pane beside BACKING (contextual to
      Shadow/Plate) so the Effects pane is glow-only.
- [x] **UX0-C13 — achromatic hue drives:** a hue drive on a white/grey/black glow
      colour does nothing; blend saturation in with the drive so an achromatic
      base still sweeps colour.
- [x] **UX0-C14 — drive tuning layer:** give PULSE/HUE the Tune popover's
      quiet/loud in→out ranges, curve and live readout, by extracting the
      mapping editor from `panels/tune.rs` into shared componentry.
- [x] **UX0-C15 — Time route source:** add the caption pane's Time drive (8 s
      triangle) as an `AnalysisSource` for scene routes; keep the route
      differential harnesses green (additive token).
- [x] **UX0-C16 — styling tooltips:** tooltips across the caption Style/Effects
      panes and the Tune mapping editor, using the existing widget tooltip path.
- [ ] **UX0-C17 — give the caption tune editor room:** the mapping editor fits
      the caption pane's 160 px only by cutting Swap, exiling the transfer
      graph to the other column and packing the curve row 6 px from the floor.
      Growing the pane when a disclosure opens means changing
      `lyrics_editor_layout`'s harness-pinned panel heights — a deliberate
      layout change, so it queues rather than sneaks in.

- [ ] **UX0-C01 — clip export:** expose render in/out or start/duration controls
      and drive the existing windowed `RenderPlan` path (`review` 3.1).
- [x] **UX0-C02 — vertical and square output:** add explicit width/height output
      formats without changing the C-ordered persisted resolution enum, then
      capture-audit every scene at tall and square aspect ratios (3.2).
      *Done 2026-08-07 as EX2 (`9db6684`), see its section: ASPECT row
      (16:9/9:16/1:1/4:5, rung names the short edge, 16:9 byte-identical),
      thirty renders — ten scenes at three aspects — audited frame by frame,
      caption and ASCII Field fixed, preview framed to the export aspect.*
- [ ] **UX0-C03 — lyric tap timing:** deliver the play-and-tap stamping workflow,
      row seek and advancement described by B02 as a polished primary flow (3.3).
- [x] **UX0-C04 — preset audition/A-B:** make preset exploration reversible with
      hold-to-audition or an explicit settings snapshot comparison (3.4).
      *Done (PX6) as an explicit snapshot A/B rather than hold-to-audition: a
      held button cannot be compared against while it is held, and the thing a
      user wants is to flip back and forth listening. `core::ui::tune_explore`'s
      `ExploreState` captures a `SettingsSnapshot` before the first exploratory
      action — a preset Apply, Surprise, Nudge or a per-setting reset — and puts
      A/B, Revert and Keep on screen. Revert is **bit-for-bit**: a snapshot is 12
      `f32`s copied back, the same operation loading a cue performs, so it is
      exact rather than close. Exploring five times still reverts all the way to
      where the user was, not to the previous experiment. The session is keyed to
      `(track_slot, scene, cue)` and ends when the target moves, so a base-scene
      snapshot can never be written into a segment it was not captured from
      (LX3). Keep is refused while A is on screen and says why. Gate:
      `tune-revert`, `tune-revert-deep`, `tune-ab-a`, `tune-ab-b`, `tune-keep`.*
- [ ] **UX0-C05 — scene thumbnails:** generate/cache deterministic preview stills
      so scene choice is visual rather than ten text-only names (3.5).
- [ ] **UX0-C06 — recent projects:** use the welcome screen's spare region for a
      durable, failure-tolerant recent-file list and direct reopen flow (3.6).
- [x] **UX0-C07 — Tune randomize/mutate:** add bounded Surprise/Nudge operations
      with the same reversible audition semantics as C04 (3.7).
      *Done (PX6). Randomness is **injected**, not ambient: `tune_explore` takes a
      `RandomSource` and the panel seeds a `SplitMix64` from a press counter, so a
      capture can pin it with `--ui-probe tune-seed=` and assert the values rather
      than "something moved". Four biases, all measured by tests rather than
      argued for: results are snapped to the descriptor's precision so the readout
      cannot lie; Surprise keeps 5 % clear of each end, where the degenerate looks
      live (zero density, zero glow, everything at maximum); it moves 75 % of
      sliders so the scene stays recognisable as itself; and it flips a toggle
      only 25 % of the time, since `atlas.wireframe` is a whole-scene character
      switch. Sliders draw triangular about the descriptor **default**, except the
      five `-180..180` angle controls, which draw uniform — hue is circular and has
      no designed centre to pull toward. Nudge is triangular about the current
      value with a 12 % half-width and never flips a toggle. Every draw goes
      through the audition, so Revert undoes it exactly. Bounds are swept over 200
      seeds x 10 scenes x both strengths — all 81 descriptors, every value
      `accepts`-valid and writable through `set`. Gate: `tune-surprise`,
      `tune-surprise-alt`, `tune-surprise-click`, `tune-nudge`.*
- [ ] **UX0-C08 — project-level palette/look:** define a coherent track-level
      color treatment mapped across scene descriptors and automatic changes
      without breaking routes or saved settings (3.8).
- [ ] **UX0-C09 — cover-art/logo layer:** generalize project image assets into a
      track-level visual layer and define its scene-host/render/route semantics
      (3.9).
- [ ] **UX0-C10 — still-frame export:** publish a user-selected supersampled
      frame through the offline render path and reuse it where appropriate for
      thumbnails/cover output (3.10).

### UX0-D — verification blind spots

- [x] **UX0-D01 — pixel-backed panel gates:** measure expected ink/content inside
      every panel rect, beginning with Export; a textual state report alone is
      insufficient (`review` 4.1, feeds G1).
      Export done (`7e980a9` + wave-3): ffprobe ink assertion in the panel
      rect, and it immediately earned its keep — it caught the 27 px header
      the derivation and its unit test both missed. Extending the technique
      to the remaining panels stays a G1 item.
- [x] **UX0-D02 — Assist state captures:** make Candidate, Running and Failed
      deterministic probe states and visually gate their consequential content
      (4.2, feeds G1).
      Done in `22292e0` + `93972ca`: `--ui-probe assist=` synthesizes all
      three states in-process, seven captures per sweep (including the narrow
      reason-row branch), report-line assertions pin state/staged. The
      blocked-Apply reason is confirmed visible in the frame.
- [x] **UX0-D03 — non-Latin editor gate:** assert that the seeded authored text
      remains distinguishable and is not replaced by U+003F in editor surfaces
      (4.3, feeds G1).
      Done in the wave-3 commit: `missing=` in the `lyrics:` report line is a
      direct glyph-coverage count for the strings on screen (inference-free,
      unlike a `?`-pixel heuristic); the gate asserts `missing=0` on every
      lyric capture plus positive `face=caption` pins on the cue-pane frames,
      including a new capture that binds the Greek/Cyrillic line into the
      editable field.
- [x] **UX0-D04 — dark-overlay contrast coverage:** extend the palette/contrast
      sweep to severity text and every other semantic color used on dark overlay
      surfaces (4.4, feeds G1).
      Done in the wave-3 commit: every `theme::rgba` entry declares its
      `Surface`, the sweep measures each against its declared backgrounds
      (20 entries, 16 pairs, all AA), and the old failing chrome-on-dark pairs
      are pinned as a permanent negative control.

UX0 completes only when every checkbox above has implementation evidence and the
review document has a final disposition annotation or companion table for all
items. Record commits, captures, negative controls and any operator-approved
deferrals here as work lands; do not create a separate UX completion plan.

## P0 — make saved project content reach the renderer

### A1 — one project-aware `SceneFrame` path

- [x] Replace the preview and export uses of `..SceneFrame::idle(effective)` with
      one shared frame-building path.
- [x] At `time_seconds`, sample the current track's semantic lane with
      `project::semantic_lane::sample`.
- [x] Select the active lyric with `LyricsDocument::at_time`.
- [x] Merge manual and semantic events through `SceneEventMerge`; preserve the
      C ordering and semantic-id qualification already pinned by the differential
      harness.
- [x] Feed the same semantic, lyric and merged-event view to preview and export.
- [x] Preserve the no-track behavior: default semantics, no lyric, empty events.
- [x] Define failure behavior visibly. An invalid merge must be reported and use
      an empty frame view; it must not retain a prior track's events.

Observable closure:

- Semantic valence/tension/confidence modulates every scene that reads it.
- Cadence receives active lyrics instead of always drawing its ambient fallback.
- Loom receives its semantic lane instead of always relying on fallback data.
- Constellation receives both manual and model-derived event markers.
- Preview and export see identical project data at the same scene time.

### A2 — shared lyric captions

- [x] Port the application-side `draw_scene_lyric_overlay` composition using the
      existing `caption_layout` and `runtime::font` machinery.
- [x] Draw the shared overlay after every scene except Cadence, which owns its
      lyric composition just as the C app does.
- [x] Honor the project's caption anchor, width, size, margin, text/box colors,
      box mode and imported caption face in both preview and export.
- [x] Keep the overlay resolution-independent through the export pixel scale.
- [x] Add evidence for long UTF-8 text, the three-line/ellipsis contract, all box
      modes, a non-default anchor and an imported face.

### A3 — frame-boundary evidence

- [x] Generate a synthetic `.musi` containing lyrics, semantic events and manual
      events; do not commit user media.
- [x] Add a headless report line naming the active lyric id, semantic availability
      and merged event count.
- [x] Capture at least one shared-caption scene, Cadence, Loom and Constellation.
- [x] Export selected frames and assert that seeded project lanes change the
      output. Include a negative control that drops one lane and demonstrably
      changes the evidence.
- [x] Confirm the same fixture written by C and Rust produces equivalent lane
      selection at boundary timestamps.

Completion evidence (2026-08-01, `84ad8bb`): `ProjectFrameLanes` is the shared
preview/export constructor and owns a
fresh merge so an invalid lane logs its error and exposes zero events. The pure
suite pins no-track state, exact lyric/semantic boundaries, canonical ids/order,
UTF-8 three-line ellipsis, all box modes, a non-default anchor and 2x export
scaling. `tools/headless_check.sh` generates the `.musi` and bundled imported face,
captures Spectrum, Cadence, Loom and Constellation, reports
`lyric=1 semantic=available source=11 merged-events=4` in both preview and export
at `t=1.000`, and repeats both full outputs deterministically. Removing lyrics,
semantics or manual events changes both preview and export hashes and changes only
the named report fields. The expanded bidirectional project harness compares 510
values per path / 2,550 total at delta 0; its 1 ms-early lyric negative control
produced four boundary failures in each direct comparison (12 discrepancies).

## P0 — automatic scene plans and cue semantics

### B1 — drive automatic switching in preview and export

- [x] Apply `SceneSwitchTimeline::update` before frame construction on both paths.
- [x] Make `--auto-scenes` actually enable the imported/saved plan; reject an
      empty plan with the oracle's observable behavior.
- [x] Replay accepted Assist section plans and enabled `.musi` scene plans.
- [x] Apply a cue's captured settings snapshot before audio routes, matching the
      C precedence: base/cue settings first, per-frame routing second.
- [x] Reset the switch cursor and cue-settings state on seek, track change,
      project open, export start/end and plan enable/disable.
- [x] Keep preview and export scene selection identical for the same time.

### B2 — base-scene selection and manual `+ Scene` capture

- [x] When a user selects a different base scene, set the pending-selection state,
      remember the previous base scene and mark the project dirty.
- [x] Selecting a base scene while Auto scenes is enabled must disable playback of
      the plan, retain its cues and explain that result in the notice tray.
- [x] The first manual scene cue recorded after time zero must backfill the former
      base scene from zero before inserting the new cue.
- [x] Recording a cue must capture the selected scene's effective tuning snapshot,
      clear the pending flag and apply the cue immediately at the playhead.
- [x] Tune edits made while a cue snapshot is active must update that cue rather
      than silently changing only the base scene settings.

### B3 — scene-plan evidence

- [ ] Add preview and export tests at every cue start/end boundary, including seek
      backward, fast-forward and a windowed export beginning after cue zero.
- [ ] Assert selected scene id and a cue-specific setting, not merely the enabled
      flag or the words "automatic scene plan".
- [x] Prove a manual first cue at `t > 0` retains the prior base scene on
      `[0, t)`.
- [x] Add a negative control that disables the call to `update` and is caught.

Completion evidence (2026-08-01, `a664b7d`, operator-reported lyric deletion and
PCM scene cue repair): the lyrics form now assigns disjoint widget ids to Apply, Discard,
Delete and both timing rows. Reusing Delete's id for START -0.1 reproduced the
failure and the new collision test failed 8/9 before restoration; the isolated
Xvfb click now leaves 5 cues from the 6-cue fixture and clears the selection.
`Track::advance_scene_plan` is the shared preview/export driver and installs a
cue snapshot before routes. A two-cue 8-second project reports Loom after its
4.0-second boundary in preview and in a 0.2-second windowed MP4 beginning at
4.0; the same disabled project reaches Loom through `--auto-scenes`. Unit tests
pin base-scene pending/backfill state, exact boundary transitions, seek-style
reset and active-cue tuning. Returning before `SceneSwitchTimeline::update` made
the boundary test fail (`None` versus `Some(Spectrum)`) and was reverted.

### B4 — interactive automatic-scene control

- [x] Add the C workflow's visible Auto-scenes On/Off control for a non-empty
      saved or Assist-created plan; `--auto-scenes` must not be the only way to
      enable it.
- [x] Reset the switch cursor and cue-settings state on either transition, and
      restore the base scene when disabling the plan.
- [x] Retain the cues, mark the project dirty and report the cue count/state.
- [x] Capture enabled and disabled states and prove that the next preview/export
      frame follows the same plan state.

Audit evidence: the C control and transition are at `plug.c:2207-2226`; the Rust
Assist composition at `ui/panels/assist.rs:1089-1119` has no equivalent. The only
production enable path is startup `--auto-scenes`.

Completion evidence (2026-08-01, working tree): the Assist header now shows the
C workflow's `Current auto scenes: On/Off (N)` button whenever the current track
has a plan and the panel is at least 560 px wide. Its `ShellCommand` runs through
one track transition that refuses an empty plan, retains every cue, rewinds the
cursor, clears cue-specific settings, marks dirty and restores the base
`SceneInstance` on disable. A unit test pins empty-plan refusal, cue retention and
both-direction rewind. The silent Xvfb gate opens a generated two-cue project at
5.0 seconds and captures both states: disabled reports `disabled (2 cues)` and
Constellation, while `--auto-scenes` reports `enabled (2 cues)` and Loom; their
frames differ. The gate first failed by refusing a copied project whose bundled
audio directory was absent, proving the fixture still enforces asset identity;
copying the content-addressed bundle produced the green run.

## P1 — durable edits, guards and autosave

### C1 — complete dirty marking

- [ ] Audit every command that mutates data serialized into `.musi` and route it
      through one dirty-marking policy.
- [ ] At minimum, mark dirty after setting changes, scene reset, base-scene
      selection, cue-snapshot tuning, caption-style edits, lyric import, ASCII
      clear/import and scene-plan enable/disable.
- [ ] Keep purely transient playback, selection, hover and panel state clean.
- [ ] Test that each durable mutation starts/restarts the 1.5-second autosave
      settle and participates in the quit guard.

### C2 — lyric and route draft context guards

- [ ] Expose one truthful `lyric_editor_has_unsaved_draft` query.
- [ ] Block or resolve dirty lyric/route drafts before track change, scene change,
      panel change, project open, save, export start, Assist apply and quit where
      the C workflow protects them.
- [ ] Include the active lyric draft in `confirm_close` and in autosave's
      `editor_dirty` argument.
- [ ] Never let Assist replace lyrics while a conflicting lyric draft is open.
- [ ] Give every refusal an actionable notice; do not silently discard or switch.
- [ ] Cover Apply, Discard and Cancel paths, not just the blocked path.

### C3 — reversible scene reset

- [ ] Replace immediate "Reset scene" with the C workflow: Reset, Confirm, then
      Undo reset.
- [ ] Preserve the pre-reset settings until another reset/context invalidates the
      undo snapshot.
- [ ] Mark reset and undo as durable edits and commit to an active cue snapshot
      when applicable.
- [ ] Capture all three button states.

### C4 — autosave every track

- [ ] Cache decoded sample rate and channel count on each `Track` so project
      serialization does not depend on the currently bound music stream.
- [ ] Autosave every due track, not only the current one.
- [ ] Refuse autosave while that track owns a dirty editor draft.
- [ ] Test two dirty tracks, a background track loaded from a project, failure of
      one save without suppressing another, and recovery after a failed save.

### C5 — adopt project-carried presets

- [ ] When a project opens, merge its embedded scene presets into the shared
      preset library by scene and values, matching the C's deduplication rule.
- [ ] Persist newly adopted presets and make them immediately visible in Tune.
- [ ] Notify only when the shared library actually changes; a persistence failure
      must not make the in-memory and on-disk libraries silently disagree.
- [ ] Test reopening a project whose preset is absent, already present and equal
      in name but different in values.

Audit evidence: C calls `shared_presets_adopt` after hydration at
`plug.c:4992-5005`; Rust hydrates `track.scene_presets` but `open_project` never
merges them into `app.shared_presets`, which is the library Tune reads.

## P2 — missing C workflows and timeline affordances

### D1 — typed file-drop dispatch

- [ ] Dispatch dropped `.musi` files through project open.
- [ ] Dispatch PNG/JPEG/BMP through ASCII import and select ASCII Field.
- [ ] Continue dispatching supported audio formats through track load.
- [ ] Preserve the C behavior that an image dropped before audio is staged for the
      next track.
- [ ] Report unsupported/corrupt input by its attempted type.
- [ ] Add non-interactive probes or direct command tests for all three branches.

### D2 — ASCII image import and clear UI

- [ ] Add "Import image -> ASCII" to the scene browser with PNG/JPEG/BMP filters.
- [ ] Show "Clear image" only when the active track owns an image-backed grid.
- [ ] Import transactionally, select ASCII Field on success and mark the project
      dirty; a failed decode must preserve the previous grid.
- [ ] Clear path, digest, cells and dimensions together and mark dirty.
- [ ] Capture empty, populated, cleared and staged-with-no-track states.

### D3 — timed-lyrics TSV import/export

- [ ] Replace the disabled Lyrics Export/Import buttons with native save/open
      dialog commands.
- [ ] Use `LyricsDocument::bridge_export` and `bridge_import`; do not invent a
      second codec.
- [ ] Import transactionally, validate against track duration, mark dirty and
      preserve the old document on failure.
- [ ] Export must not dirty the project.
- [ ] Test cancel, overwrite confirmation/backend failure, invalid TSV, valid
      round trip and UTF-8 text.

### D4 — timeline content and navigation

- [ ] Draw the merged manual/semantic event markers over the waveform lane.
- [ ] Draw the lyric cue lane even when the Lyrics editor is closed.
- [ ] Add Shift-wheel pan and middle-drag pan with robust pointer-claim release.
- [x] Make waveform scrubbing transactional: pause on press, track the target
      while dragging, seek once on release and restore the prior play state.
- [x] On every transport discontinuity, clear queued pre-seek PCM and analyzer
      history as well as beat, scene-clock, scene-plan and cue-settings state.
- [ ] Keep wheel zoom anchored at the pointer and every lane aligned through the
      shared `TimelineView` conversion.
- [ ] Give tick labels an opaque backing so waveform amplitude cannot erase their
      contrast.
- [ ] Capture event colors, lyric spans, zoom, pan and an off-screen boundary.

Operator follow-up (2026-08-04), scoped to the existing D4 ownership:

- [x] Keep proportional Space Grotesk for time readouts, but lay out its digits in
      tabular cells so `00:11.111` and `00:55.555` reserve exactly the same width.
      The toolbar timecode, progress/level bars, zoom/view readout and adjacent
      arrow-key hint must not breathe while playback advances.
- [x] Apply playback-follow before any timeline lane draws and pixel-snap the
      shared playhead, so the scene, waveform and lyric lanes consume one view
      state per frame without marker flicker during a zoomed follow-scroll.
- [x] Bound the waveform playhead handle at both strip edges; at the track end no
      part of the marker may draw outside the PCM element.
- [x] Left-drag on the waveform and scene-plan body must preview one playhead and
      commit one transactional seek on release. Preserve lyric-cue left-drag for
      cue editing and preserve scene-boundary drag for retiming.
- [x] Middle-drag any timed lane to pan the shared view. A manual pan suspends
      playback-follow until the visible Follow affordance is used, and release or
      a disappearing surface must never strand the pointer claim.
- [x] Retain a higher-detail whole-track waveform envelope than the C oracle's
      2,048-bin hot-reload array; document the memory/detail tradeoff and pin the
      Rust-side cap with a test.
- [x] Scope lyric-cue hover to both axes of the actual cue lane. A pointer over the
      scene preview, controls or waveform must not highlight a cue that merely
      shares its X coordinate.

Completion evidence (2026-08-04): live time strings draw through proportional
Space Grotesk with tabular digit cells; rendered `00:01.111`/`00:05.555`
snapshots keep both transport bars and the lower hint at identical X bounds.
Playback-follow and deferred wheel zoom now mutate the shared view before any
lane draws, and one bounded pixel-snapped renderer serves the scene, PCM and lyric
playheads. Its edge test and a `00:07.999 / 00:08.000` capture keep the right-hand
line and triangle inside the PCM box. One shell transaction now owns scene-body
and PCM scrubs (pause, preview, one release-time Seek, conditional resume), while
scene boundary and lyric-cue drags retain their domain behavior. Middle-pan uses
a fixed origin, releases on physical button-up, persists as `free view`, and the
visible Follow action restores automatic tracking. The waveform cap is 32,768
min/max pairs (16x the C hot-reload array, 256 KiB per track); the full headless
report observes all 32,768. Lyric hover has a two-axis lane gate and a negative
control for the same X over preview/PCM. App/core tests are 231/694 passed, clippy
is clean, `tools/verify.sh --quick` is 19/0, and the private-Xvfb sweep passes with
0 ordinary output underruns plus its forced-starvation negative control.

Completion evidence (2026-08-03): C's press/drag/release contract at
`plug.c:3174-3199` now maps to one pause command on press, an in-memory target
through the drag and one transactional seek plus conditional resume on release.
`seek_preview` mirrors the full reset at `plug.c:2661-2678`: resume a paused
raylib buffer so Stop can reset it, stop, clamp/seek, refill both halves, clear
the callback ring plus analyzer/beat history, reset the scene clock/plan/cue
state, then play and restore the prior pause state. The headless gate observes
zero output underruns on the ordinary and seek/reopen paths; its main-thread
stall negative control observes 45-46 output-starved mixer reads, proving the new
counter measures the silence-producing queue rather than analyzer-ring drops.

The same audit found the Rust composition root had omitted the C build's 8,192
frame preview half-buffer and started a stream before its first refill. Rust now
sets that size before opening any `Music`, attaches the analyzer, fills both
halves and only then plays; it services the decoder before and after frame-boundary
maintenance and pauses/refills around whole-track decode, font rasterization and
probe PNG encoding. The ordinary Xvfb run changed from 17 analyzer-ring drops / 30
output underruns to 0 / 0. Spectrum's onset-only full-width white floor gradient
was also removed: the frame-eight floor crop remains at luma 21, while restoring
the old pass raises it to 55 and fails the gate.

Operator acceptance (2026-08-03, after `7099741`): real-device playback and the
Spectrum fix were tested interactively and reported to work well. Treat the audio
consistency and white-floor-flash defects as closed; retain the zero-underrun and
onset-floor negative controls as regression gates.

The next timeline gap is separate: `+ Scene` can capture a scene-plan cue at the
playhead, and `SceneSwitchTimeline` already supports split/insert, remove, retime
and retarget operations, but no scene-plan lane is drawn and there is no
comfortable editor for those operations.

- [x] Add an editable scene-plan lane to the timeline; creation, selection,
      retargeting, boundary movement and deletion need a discoverable workflow.
- [x] Extract the shared timed-lane mechanics from the lyric editor:
      `TimelineView` geometry, clipping/minimum visible width and drag threshold.
      Keep stable-id selection, hit zones and preview state in the lane policies,
      while one shell-level owner arbitrates pointer gestures. Reuse those
      primitives for lyrics, scenes and later lanes instead of cloning
      `lyric_lane_edit` or coupling the scene editor to the Lyrics panel.
- [x] Keep lane policy and durable commands domain-specific. Lyrics may overlap,
      multi-select, move as a group and resize either edge; scene segments form a
      contiguous whole-track partition, select by stable cue id and move only a
      shared internal boundary. Apply scene split, retime, retarget, tuning
      capture, remove and enable/disable commands outside the draw pass.
- [x] Make scene boundaries independently hit-testable from scene segments and
      keep transition presentation separate from segment identity. A later
      transition can then carry its own kind, duration and curve and render as a
      boundary overlay without changing segment coverage, selection semantics or
      the cut-only `.musi` representation in this task.
- [x] Move timeline-lane gesture arbitration into the shell so a lane drag cannot
      also scrub, pan or seek. Preserve preview == committed-result tests, add
      short/off-screen boundary cases, and capture the lane both enabled and
      disabled at whole-track and zoomed views.

Completion evidence (2026-08-03): the always-visible lane draws contiguous,
scene-colored blocks above the waveform and exposes Auto, Split here, previous/
next scene, Capture tuning and Delete controls. Splitting selects the resulting
segment on the next frame without predicting a model id; a boundary drag previews
the exact value `SceneSwitchTimeline::retime` accepts. Stable-id workspace methods
apply every durable edit and reset live cue state before the composition root
refreshes the current automatic scene.

`ui::timed_lane` now owns the geometry the lyric and scene painters share, while
`ui::scene_lane_edit` keeps the contiguous-partition policy separate. Tests pin
boundary priority over both adjacent bodies, a boundary exactly on the zoom edge,
an off-screen boundary that must not be recreated at the lane edge, retime
preview == commit, whole-track coverage after retarget/remove, and AA text
contrast for all ten scene colors in enabled and disabled states. The silent
headless gate captures the same two-cue plan disabled, enabled and at 4x zoom;
its lane-only crop measures saturation 55.1 enabled and 20.0 disabled, where a
missing/empty lane is zero.

Review follow-up: while a scene boundary owns the pointer, wheel zoom and
playback follow-scrolling are suspended so the time under a stationary hand
cannot move with the view. Playback itself is not paused, and ordinary follow
resumes after release.

### D5 — CLI execution semantics

- [x] Execute positional audio, `--project`, `--scene`, `--event` and
      `--ascii-image` actions left-to-right instead of reducing them to one input.
- [x] Preserve every successfully loaded audio track; apply deferred routes only
      after all immediate input actions so they target the resulting project.
- [x] Select ASCII Field only after a successful image import.
- [x] Once any CLI error occurs, keep parsing for diagnostics but suppress the
      later bridge, auto-scenes, save-project, UI-probe and render side effects
      that the C suppresses.
- [x] Add side-effect assertions for multiple inputs, interleaved actions, routes
      accompanying a project and an early error followed by `--save-project`.

Audit evidence: the C applies input actions immediately and defers only routes
(`musializer.c:500-561`). Rust collects them into one `Option<Input>` and applies
routes before opening it (`main.rs:307-382`, `:464-499`); later side effects also
run despite `options.error` (`:510-632`).

Completion evidence (2026-08-01, working tree): argv replay now owns the stream
and opens every input at its position; only routes remain deferred. Render config
precedes save, all later stages honor the shared error, startup mutations are
marked clean across every loaded track, and unknown flags again follow the C's
positional-audio arm. The headless negative control first reproduced all three
principal failures — `1 open` with only the last input, failed ASCII selecting
ASCII Field, and an error-returning command still publishing its project — then
reported `2 open, current 0`, retained Loom, and left the destination absent.
Additional gates pin a route applied after project hydration and a saved
854x480/24 fps/master render config. Workspace/app tests are 138 passed and
Clippy is warning-free.

### D6 — Tune reachability and descriptor controls

- [ ] Make the Tune inspector vertically scrollable with clipping, robust pointer
      release and a visible position indicator so every descriptor remains
      reachable at every supported window size and route-editor expansion.
- [ ] Render `SettingKind::Toggle` as a labelled binary control rather than a
      numeric slider; preserve the Song Atlas manual-count detail readout.
- [ ] Capture the last descriptor, both toggle states and an expanded route row at
      the minimum supported window size.

Audit evidence: C scrolls and clips the inspector and branches for toggles
(`plug.c:6116-6255`). Rust stops drawing when the next row does not fit and treats
every unrouted descriptor as a slider (`ui/panels/tune.rs:281-415`).

### D7 — keyboard workflows and discoverability

- [ ] Restore Ctrl+S, Ctrl+Shift+S, R for Export and direct 1-0 scene selection,
      using the same draft/context guards as the corresponding buttons.
- [ ] Expose 1-0 in scene-tile tooltips; retain Tab cycling as a Rust addition.
- [ ] Add input tests proving text entry suppresses every global shortcut.

Audit evidence: C handles these at `plug.c:1382-1396` and `:7585-7610`. Rust's
global keyboard path (`ui/shell.rs:571-637`) has transport, Tab and Tune bindings
but none of those C workflows.

### D8 — actionable notice tray

- [ ] Allow application call sites to provide the full notice specification
      instead of forcing every notice to be transient and pathless.
- [ ] Restore wrapping, persistence, file paths/tooltips, Dismiss, Copy path,
      Assist Retry and a `+N more` indicator for hidden notices.
- [ ] Test clipboard/backend failure, overflow, long UTF-8 text and the retry
      action; capture transient, persistent and path-bearing cards.

Audit evidence: C's tray implements these actions at `plug.c:6663-6738`.
`Shell::notify` discards persistence/path data and the Rust tray is text-only
(`ui/shell.rs:257-268`, `:1842-1902`).

## P3 — package the complete external product

### E1 — define and copy the support bundle

Completed 2026-08-01 (`a664b7d`): source-bundle restoration depends only on
the frozen oracle's reviewed first-party support files. E2/E3 consume the same
manifest, while E4 will reuse it for archive staging.

- [x] Treat the C distribution's support-file list as the starting manifest, not
      `external_analysis.py` alone.
- [x] Bring over and review the required Python modules, prompt, schemas and docs:
      `analysis_io.py`, `analyze_audio.py`, `external_analysis.py`,
      `google_fonts.py`, `import_whisper.py`, `lyric_align.py`,
      `mimo_openrouter.py`, `musializer_doctor.py`, the lyrics cleanup prompt and
      every schema those tools read or write.
- [x] Preserve the helpers as independent tools; first-party application code
      remains Rust.
- [x] Remove C-build assumptions and resolve resources relative to the installed
      bundle without requiring the C checkout.
- [x] Add one authoritative Rust distribution-support manifest so install and
      archive paths cannot drift.

Evidence: all copied helper/assets matched the frozen files by SHA-256 before
the doctor and user documentation were adapted to Cargo/Rust paths.
`runtime::support::DISTRIBUTION_SUPPORT_FILES` names the bundle and tests every
entry. `tools/support_bundle_check.sh` compiles all helpers, runs a real local
Sections analysis over synthetic PCM, parses its bridge through Rust, dry-runs
the model-authorized modes without credentials, and proves a headerless bridge
fails as its negative control.

### E2 — make Assist runnable

Lyrics-timing evidence added 2026-08-01: the local `lyrics` path now follows
Whisper/reference review with CUDA MMS forced alignment and publishes only the
refined lane. Two embedded-metadata tracks and audio-identical tag-stripped
copies complete end to end, including native bridge parsing, independent
Demucs-vocal controls, remote phrase-count audits, and targeted negative
controls. The full methodology and metrics are in
`docs/LYRICS_TIMING_INVESTIGATION.md`. This closes the timing investigation but
does not close E2's remaining discovery, lifecycle, staging, or UI work.

- [ ] Verify all local/offline Assist modes without network credentials.
- [ ] Verify explicit Codex/OpenRouter paths only when opted in; never copy
      credentials into fixtures, logs or the repository.
- [ ] Exercise real helper discovery from source, installed launcher and extracted
      distribution layouts.
- [ ] Test cancellation, timeout, process-group cleanup, invalid bridge output,
      staging, Apply and Discard.
- [ ] Re-probe helper availability at the user decision boundary so installing or
      removing support files updates the controls/status without restarting;
      preserve hard failure for an explicitly set but missing helper override.
- [ ] Keep Assist progress/review state visible in fullscreen with a non-exported
      badge for running, cleanup, candidate-ready and failure/artifact states.
- [x] Keep a fake helper for deterministic lifecycle coverage, but add at least one
      real support-bundle smoke test.

### E3 — make Google Fonts import runnable

- [ ] Package `google_fonts.py`, `analysis_io.py` and its schema dependencies.
- [ ] Keep network consent once-per-run and never persist it.
- [ ] Verify catalogue caching, host allow-list/redirect refusal, digest checks,
      licence publication, import into a project and project reopen.
- [ ] Poll the importer once per application frame, including while the browser
      pane is closed, and consume each verified import exactly once.
- [ ] Apply an import transactionally: rasterize it, set the imported caption
      face plus asset/runtime paths, mark dirty and notify success; rasterization
      failure must preserve the prior face and project fields.
- [ ] Re-probe helper availability when Browse/Fetch is requested rather than
      caching startup state for the entire session.
- [ ] Keep a fully offline fake/helper fixture for the default verification gate.

### E4 — installation, doctor and distribution

- [ ] Provide the C app's equivalent self-contained distribution/archive workflow,
      including executable, launcher, MIME data, resources and support bundle.
- [ ] Make `musializer_doctor.py` or a Rust equivalent report FFmpeg, helper,
      Whisper/model and writable-data-path readiness.
- [ ] Test the per-user launcher from a directory other than the repository root.
- [ ] Test an extracted distribution on a clean path with no Cargo invocation and
      no dependency on `../musializer`.
- [ ] Document optional external dependencies and which features degrade without
      them.

## AP — assistance provider configuration (accepted 2026-08-04)

Design authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` (P0, done), which carries
the operator's storage decisions from `AGENTS.md` — no OS/vendor wallet, a 0600
credentials file, user-chosen models directory. Later phases (dialog, execution
wiring) get their tranches only after these land. The lyrics-timing benchmark
(`docs/LYRICS_TIMING_RESEARCH_PLAN.md`, trimmed per
`docs/LYRICS_TIMING_WEB_EVIDENCE.md`) runs in parallel and is not gated on this.

### LT1 — anchor→block lyric localization in production

Benchmark verdict 2026-08-04 (`docs/LYRICS_TIMING_BENCHMARK_RESULTS.md`):
anchor→block MMS reached 100% authored-line coverage on all four tracks and
recovered the canary's Whisper-looped outro lines; whole-song Qwen failed its
decision gate. Both gates cleared 2026-08-04: AP1 landed (`d132a3e`) and the
operator adjudicated all spot-check lines (anchor lane correct 14/21, its only
failures the predicted repeated-phrase and weak-exclamation classes; verdicts
in `build/lyrics-research-v2/ground_truth_adjudication.json`). Key design
consequence: the aligner's own score does not separate right from wrong —
review flags come from cross-lane disagreement and unresolved lines
(Invariant 4), never from raw model score.

- [x] (2026-08-04) Replace the per-cue Whisper-window MMS path with anchor→block
      alignment. Policy in `tools/lyric_anchor_block.py` (pure, no model),
      acoustics in `tools/anchor_block_align.py` (alignment venv), orchestration
      in `external_analysis.run_assist`. `lyrics.sync.json` is demoted to the
      coarse proposal; `lyrics.aligned.json` is the anchor/block result. The
      no-reference lane keeps `force_align_lyrics.py` unchanged. Invariant 1 is
      enforced, not assumed: `validate_full_coverage` refuses a lane where cues
      plus `unresolved` do not account for every alignable authored line.
- [x] (2026-08-04) Whisper evidence pass: `--max-context 0` on every run.
      whisper-cli 1.8.6 has **no** `--no-context` (`params.no_context` is never
      wired to an argument), but zero max context skips history conditioning at
      `whisper.cpp:7097`. Measured: the canary's 90 s loop is gone and its first
      outro line transcribes at 90.6 s; duplicate segments across the four
      tracks fall 31→13, 27→11, 22→0, and the choir track gains evidence 32→55.
      **VAD is measured harmful and stays off**: Silero v6.2.0 keeps 0.4 s of
      the canary at threshold 0.50 and 3 segments at 0.10. It is opt-in through
      `MUSIALIZER_WHISPER_VAD_MODEL`, and an absent model degrades to no VAD.
      Both are recorded in the lane's `request_settings`.
- [x] (2026-08-04) Surface unresolved lines and review flags in the Assist review UI by
      name and time range, so the user is pointed at what to fix instead of
      hunting. A flag is cross-view disagreement (coarse proposal vs
      anchor-block placement, >3 s) or an unresolved/abstained line — not the
      aligner's own score, which the adjudication proved uninformative. The
      additive JSON fields are in place (`unresolved`, `review_flags`; see
      `docs/PHASE0_INVENTORY.md` 6.6) and the current Rust ignores them.
- [x] (2026-08-04) Repeated-phrase abstention. A repeated authored line abstains
      when the coarse view puts it nearer a *sibling* occurrence's block
      placement than its own by more than the 3 s review tolerance, or when two
      identical lines collapse onto one acoustic phrase. **The pinned example no
      longer reproduces**: with the repetition loop contained, both views place
      `shut-up-cat` line 26 on its own occurrence (110.0 s / 108.3 s), so it
      keeps a cue. The criterion is pinned instead against the recorded pre-LT1
      numbers in `tests/test_lyric_anchor_block.py`, where it fires on line 26
      and on nothing else in that six-occurrence group.
- [x] (2026-08-04) Localization policy versioned in cache provenance:
      `localization_policy` / `localization_policy_version` on the document and
      in `timing_refinement`, checked by `_anchor_block_cache_accepts` alongside
      the whisper/coarse/reference digests. The coarse lane records
      `role: coarse_proposal`, and the Whisper lane's `text_conditioning` and
      `vad_model_sha256` invalidate any lane decoded under the old flags.
- [x] (2026-08-04) Evidence: all four benchmark tracks rerun through the
      *production* path reach 100% coverage with 0 omitted and 0 unresolved
      (15/15, 33/33, 51/51, 42/42); the canary's outro lines are real cues at
      90.72 s and 93.94 s against the adjudicated 90.7/93.9; every
      adjudicated-correct line matches the anchor-lane time to ≤0.21 s; the
      choir track's order violation is **fixed** (0, not pinned) — it came from
      the pre-`--max-context 0` evidence. `tools/support_bundle_check.sh`,
      `cargo test` and `tools/verify.sh --quick` are green, and the four real
      bridges parse through `analysis_bridge_check` unchanged. Known regression:
      `shipped-the-disposition` line 41 (`rawr.`, the weak one-word-exclamation
      class the adjudication predicted) moved from 150.6 s to 134.6 s against a
      truth of 156.2 s, and is **not** flagged, because both views agree and
      both are wrong — cross-view disagreement cannot catch a shared-evidence
      error.

### LT1-R — fresh-eyes review fixes for the lyrics-review surface

Independent user-perspective review of `7881776` (2026-08-04): ten confirmed
defects plus one verification blind spot; fixtures and captures under
`build/review-lt1/`. Fixed 2026-08-05 with two demonstrated negative controls;
R9's navigation followed on 2026-08-05.

- [x] R1 stale review: a per-audio cache dir plus a manifest that names every
      artifact makes a Sections run display the previous Full-assist run's
      lyric flags. Tie the review to this run's lyric lane on both sides
      (helper emits lyric counts only when the run had a lyrics lane; Rust
      requires it and draws the block only under the lyrics-lane guard).
- [x] R2 the "+N more" tail is the first thing the 960x640 scissor eats, and
      rows cut mid-glyph; the honesty mechanism must be the last row standing.
- [x] R3 a flags-only document silently drops named unresolved lines (union
      `unresolved[]` into the entries), and the "document could not be read"
      sentence prints when the document *was* read — make it truthful.
- [x] R4 parse-dropped entries vanish without a tail when flagged <= 4; any
      drawn<total shows the tail.
- [x] R5 the `assist review:` report line describes the parse, not the panel —
      add rows-drawn so the gate can assert against clipping, plus a 960x640
      gate capture.
- [x] R6 `reason` is parsed, truncated, tested and never drawn; surface it
      (disagreement delta on CHECK rows, why-ambiguous on AMBIGUOUS).
- [x] R7 "line 1 0:12.0" reads as "line 10:12.0" — separator.
- [x] R8 UNPLACED rows show the coarse *proposal* in the same grammar as real
      placements; label it as proposed.
- [x] R9 (2026-08-05) no route from a flagged line to where it gets fixed. The
      row is now the route: pressing one opens the Lyrics panel, binds the cue
      that carries **that line's text** — never a neighbour by time — and emits
      `ShellCommand::Seek`. An unresolved line has no cue to bind, so it lands at
      the coarse proposal and a notice names the line and its window; the
      heading's hint became "Click a line to open it in Lyrics" and every row
      carries a `widgets::hint` tooltip naming which of the three outcomes it
      has. Guarded by `lyric_draft_allows_context_change`, like every other route
      into that panel. Evidence: eight tests through the command seam (which cue
      was bound is not something a capture can assert), a hover capture
      (`assist-review-hover`, tip 20/237 against a bare 247, row fill 234 against
      247 for the same row unpointed and 247 for its neighbour) and two press
      captures over the seeded project (`assist-nav-cue`, `assist-nav-unplaced`).
      Negative control: falling back to nearest-by-time made the unplaced row
      bind cue #5, which both press assertions catch. **Limitation:** rows are
      pointer-only. `ui/widgets.rs` has no focus or traversal model at all — no
      surface in this interface is keyboard-reachable except through the global
      `KeyboardFrame` shortcuts — so a focus ring here would be half a model in
      one panel. Recorded rather than invented.
- [x] R10 manifest/document count disagreement is resolved silently; make it
      visible or report-asserted.
- [x] R11 `probe_candidate` hardcodes all lanes, so a lyrics-only candidate is
      unphotographable by anyone, gate included — add a probe seam for lane
      selection.

### AP6 — operator-reported dialog defects (2026-08-05)

Reported from the running app, so each fix carries a capture.

- [x] AP6-a the section rail sat flush against the divider; both insets are now
      `DIALOG_PADDING` and `section_gaps()` is asserted in rail and tabs layouts
      (pixel-scanned from the capture, not just reported).
- [x] AP6-b `Show experimental: off` still offered the experimental model. The
      selected-id escape hatch past the overlay filter was the defect. Offers
      are now strictly what may be offered; the configured model is still shown
      and marked `not offered: experimental`; a contract left with nothing
      offerable gets a full-width sentence naming the toggle.
- [x] AP6-c `codex not on PATH` was false — a desktop-entry process inherits a
      minimal PATH. `runtime::assist::discover` resolves override → PATH →
      eleven well-known install prefixes → login-shell PATH, with executability
      checks, a 2.5 s child timeout, per-process memoization, and the resolved
      path plus the method shown in the Codex section (searched list when
      absent). `Codex default` remains the only model fallback.
- [x] AP6-d found while wiring AP6-c: `tools/codex_model_discovery.py` had no
      `__main__`, so "Refresh Codex models" imported the module, exited 0 and
      reported success. It now has `refresh`/`show` subcommands with tests.
- [ ] AP6-e deferred discovery: ffmpeg (lives in `runtime::process`), whisper-cli
      (paired to its install tree by `external_analysis.py` — a PATH-first
      answer could select a binary that does not match its model) and the
      alignment venv interpreter (not a PATH name at all). Each needs its own
      decision; none is a small change.

### AP5 — acceptance evidence sweep (2026-08-05)

The research plan's eleven provider-settings negative controls were audited
control-by-control against live assertions: nine COVERED (one — keyboard
operability at narrow scale — was PARTIAL and fixed during the sweep with the
`ai-focus-narrow` gate capture), two PARTIAL with the missing piece named.
Every AI-settings capture carries a report-line or pixel assertion. Canary
scan coverage vs contracts §4: E1–E6, E8 (structural), E10 (structural) and
E11 scanned; E7 and E9 are real gaps, below. Totals at sweep close:
cargo 1269/0, python 110/0, full verify 21/0.

- [ ] AP5-a modality-loss invalidation (control 2): `preflight()` checks
      catalog membership by id only — a selected model that loses its required
      modality in a refreshed catalog still reads Ready. Add a modality-fit
      fact to `PreflightFacts` + tests (`core::assist::execution`).
- [ ] AP5-b no-network-hang regression test (control 4): structurally
      non-blocking (`thread::spawn` + `try_recv`), but nothing drives a stuck
      child and times the poll; needs `poll_background`'s receiver handling
      extracted raylib-free first.
- [ ] AP5-c diagnostics/crash bundle collector (contracts §4 E7): named by the
      design doc, never built. When built: name-marker env strip + canary
      route + scan coverage. Related to the external support-bundle work in
      the E-series, not part of it today.
- [ ] AP5-d clipboard copy path (E9): the "Copy diagnostics" payload is proven
      canary-free via `describe()`'s test, but no test exercises the copy call
      site itself; needs a probe seam or a raylib-free extraction.

### AP1 — persistence foundation

- [x] AP1-a (2026-08-04) `musializer.assist-settings/v1` in `musializer-core` (pure, no
      filesystem) plus a loader modelled on `ui/preferences.rs`: size cap,
      `deny_unknown_fields`, corrupt file is an error not a reset, atomic
      temp+rename write. Evidence: round-trip, unknown-field, oversize and
      corrupt-file tests; two writes of a default profile are byte-identical.
- [x] AP1-b (2026-08-04) credentials store `musializer.assist-credentials/v1` at
      `$XDG_CONFIG_HOME/musializer/credentials.json`, dir `0700`, file `0600`
      set before secret bytes exist, temp file created `0600`; loose-permission
      files are refused, not repaired. Evidence: `mode & 0o077 == 0` asserted on
      file and dir; a `0644` fixture loads as a permission error without being
      chmodded; Forget on a two-provider file leaves the other entry
      byte-identical.
- [x] AP1-c (2026-08-04) `Secret` newtype: no `Clone`, no derived `Debug`, best-effort zeroize
      on drop, hand-written `Debug` printing `<redacted>`, no new crate.
      Evidence: `format!("{:?}")` contains no key characters; non-`Clone` pinned.
- [x] AP1-d (2026-08-04) env import and self-strip: read `OPENROUTER_API_KEY` once at startup
      into the session store, then remove it from the process environment before
      threads start (`SAFETY:` comment + `AGENTS.md` unsafe-inventory row).
      Evidence: a child spawned from a build with a planted key shows no
      credential-named variable in `/proc/self/environ`; the helper's
      `_safe_local_env` test stays green.
- [x] AP1-e (2026-08-04) `.env` migration: the desktop path passes an explicit flag disabling
      `external_analysis.py`'s repository-`.env` fallback; the CLI path keeps
      it. Evidence: `_openrouter_env` with the flag and a populated `.env`
      fixture yields no `OPENROUTER_API_KEY`; `tools/support_bundle_check.sh`
      still asserts `"credentials": "environment only; omitted"`.
- [x] AP1-f (2026-08-04) canary leak scan `tools/secret_canary_check.sh`: plant a sentinel
      key through file, session and env-import routes, run a dry-run assist job,
      scan config dir, cache dir, `build/analysis/`, a saved `.musi`, the job
      log, the support bundle and `/proc/<pid>/cmdline` for zero occurrences;
      wire into `tools/verify.sh`. Negative control: leak the sentinel into the
      dry-run JSON, watch the scan fail, revert byte-for-byte.

### AP2 — discovery

- [x] AP2-a (2026-08-04) extend `musializer_doctor.py` with per-runtime identity for Whisper,
      the MMS/CTC aligner and any stem separator: path, version, model path,
      `sha256` where practical, language support, GPU readiness. Evidence: a
      schema test on doctor output; a missing binary yields a per-runtime
      `unavailable` state with a remediation string, not a global failure.
- [x] AP2-b (2026-08-05; 9 pure tests in `core::assist::models_dir`, 8 probe
      tests in `runtime::assist::models`, 12 doctor tests) models-directory
      resolution per the operator rule: default
      `<install dir>/models/`, fallback to a home-directory musializer folder
      when unwritable, always overridable and always displayed. Evidence: pure
      unit tests over an injected writability probe (writable default,
      unwritable default, explicit override); the resolved path appears in
      doctor output.
      The resolution is pure (`ModelsDirRequest` in, `ModelsDirResolution` out,
      probe injected); the real probe creates and removes one file in the
      nearest existing ancestor, because permission arithmetic gets read-only
      mounts and full filesystems wrong. `ModelsDirResolution` carries every
      losing candidate, and the doctor prints all of them plus "an override
      wins over the default above", so the rule's "never a location the user
      was not shown" is visible rather than implied. Negative controls: the
      home fallback promoted above a writable install default fails 1 core
      test; a writable default beating the user's override fails 3 doctor
      tests; both reverted byte-for-byte (`sha256sum -c`).
- [x] AP2-c (2026-08-04; live `codex app-server` JSON-RPC probe returned 7
      models, old-Codex stub yields `Codex default`) Codex `model/list` discovery where the installed Codex supports it;
      cache non-secret catalog metadata only. Evidence: a stubbed old-Codex
      response yields exactly `Codex default` and no guessed id; a test asserts
      no Codex auth file is read.
- [x] AP2-d (2026-08-04; 34 tests in `tests/test_provider_discovery.py`, live
      fetch cached 25 audio+text models) OpenRouter catalog cache: bounded fetch of `GET /api/v1/models`
      with the stored filters, normalized to an allowlist of fields, written
      under `$XDG_CACHE_HOME/musializer` with cache schema version, source URL,
      timestamps, filters, atomic replacement. Evidence: malformed, oversized,
      duplicated-id and truncated inputs are each refused while the prior valid
      cache stays readable.
- [x] AP2-e (2026-08-05; 8 tests in `core::assist::suitability`, 3 seeded rows)
      suitability overlay: a versioned in-repo table keyed by (model id,
      contract id) with `recommended`/`experimental`/`unsupported`, evidence
      date, prompt/schema version, scope, languages, limitations. Evidence:
      every `recommended` row names an evidence date and a benchmarked
      prompt/schema version; an absent model resolves to `experimental`, never
      `recommended`.
      A `const` table (`OVERLAY`, revision
      `musializer.assist-suitability/2026-08-04`) rather than an asset, because
      core opens no files and a mistyped contract id should not compile. The
      three rows are this repository's real evidence and nothing else:
      `mms-ctc`/`TC-ALIGN` recommended (2026-08-04, four-track benchmark plus
      operator adjudication, `anchor-block-mms/1`,
      `musializer.lyric-timing/v1`); `qwen3-fa`/`TC-ALIGN` unsupported (failed
      its decision gate the same day); `xiaomi/mimo-v2.5`/`TC-SEMANTIC`
      experimental (in use, never benchmarked). Negative control: defaulting an
      absent pair to `recommended` fails the lookup test; reverted
      byte-for-byte.
- [x] AP2-f (2026-08-05, with AP3) offline/stale UX: the OpenRouter section
      shows the last valid cache, its age and an explicit badge
      (`never fetched` / `unreadable` / `current` / `stale`), and `Refresh` is
      disabled with its reason while `catalog.network_allowed` is false. **An
      absent cache is "never fetched", not an empty catalog** — the gate asserts
      both spellings. Nothing hangs: the fetch is a child process on a
      background thread and the frame never waits on it.

### AP3-R — fresh-eyes review fixes for the AI settings dialog

Independent review of `d46d956` (2026-08-05): twelve confirmed findings, one
suspected; fixtures under `build/review/`. No item silently dropped. **Complete
2026-08-05.** Every item is evidenced against the reviewer's own fixture, and the
gate carries the checks so none of them can regress quietly.

- [x] S1 a failed settings load is invisible on Routing (identical pixels to a
      clean load) — persistent load-error banner on every section, matrix in an
      explicit disabled state. Evidence: the reviewer measured max luma
      difference **0** between a clean routing capture and both a corrupt-file
      and an unknown-`active_profile` one; it is **218** now, and the ring drops
      from 16 tabstops to 6 (Close plus the five section tabs). The gate's
      corrupt fixture opens on `routing`.
- [x] S2 Save force-switched `active_profile` to `custom`, orphaning the user's
      own profile forever — `set_override` writes to the active profile whatever
      its id, and only the read-only built-in `recommended` copies on write,
      reusing an existing user profile rather than inventing a second `custom`.
      The picker lists every profile in the file plus `recommended`. A
      before/after test asserts the profile list is unchanged and the active one
      still active.
- [x] S3 READY said Ready on a route with no model chosen and no eligible
      catalog entry — readiness ANDs credential, model choice and an eligible
      endpoint (§5 invariant 4) and names the **first** missing piece. An absent
      catalog is `Unknown("Catalog never fetched")`, not a confident block.
      Evidence: the reviewer's `falseready` fixture went `[ready]` → `[blocked]`,
      and the gate pins the reason as `No model chosen`.
- [x] S4 no keyboard scrolling; Tab walked focus below the fold invisibly —
      scroll-follows-focus (the ring's own draw records where it went), plus
      PageUp/PageDown/Home/End. Off-screen focus is now unstateable rather than
      merely fixed, and `focus-rect=`/`focus-visible=` are in the report.
- [x] S5 ~15 dead tabstops on a fresh install; disabled controls carried no
      tooltip (`picker()` returned before `hint()`) — one shared predicate per
      control, used by both `focus_order` and the drawing code, asserted in both
      directions by a test. `hint()` moved before the early return and every
      disabled control names its reason.
- [x] S6 TC-VERIFY's fallback picker was drawn enabled and silently did nothing
      until a route existed — disabled, hinted "Choose a route first".
- [x] S7 catalog strings unsanitised for display — one `sanitize_display` for
      every catalog-, Codex- and doctor-derived string: control **and bidi**
      characters stripped, whitespace collapsed, hard-capped with an ellipsis.
      `field_line` ellipsizes the label column independently of the value column.
      Checked against the reviewer's pathological fixture (1322-character id,
      embedded newline and tab, right-to-left override).
- [x] S8 the built-in recommended profile's own models were labelled
      experimental and their pickers permanently disabled — `whisper.cpp` /
      TC-COARSE is a `recommended` suitability row on the four-track benchmark's
      own Whisper evidence (`docs/LYRICS_TIMING_BENCHMARK_RESULTS.md`,
      2026-08-04, `anchor-block-mms/1`, `musializer.lyric-timing/v1`).
      `xiaomi/mimo-v2.5` stays experimental. A picker whose only option is the
      current selection is disabled **with** a hint naming the Show experimental
      toggle when that filter is what emptied it.
- [x] S9 comprehension — `ContractId::human_label()` drawn as the primary text
      with the `TC-*` token dimmed beneath, a boundary legend under the matrix,
      readiness badges made hoverable so their tooltip carries the whole
      remediation, the "No key" hint pointing at the OpenRouter section, and the
      credentials path shown in the OpenRouter Connection block. The CONTRACT
      column widened 104 → 150 px, which moved `ROUTING_MATRIX_DIALOG_WIDTH`
      882 → 928 and `ROUTING_MATRIX_BODY_WIDTH` 676 → 722 with it.
- [x] S10 `MUSIALIZER_ASSIST_SETTINGS_SCROLL` was dead (clamped against an
      unmeasured height) — applied on the first drawn frame instead. The gate
      captures the bottom of `local` (scroll 261 of 261) and of `openrouter`
      (102 of 102, against a catalog long enough to overflow), and the report
      carries `overflow=` and `at-bottom=` so a section that fits cannot pass as
      a section that scrolled.
- [x] S11 the dialog could not tell a running job from an idle one — one boolean
      from the shell (`workspace.assist.is_active()`), a banner on every section,
      and `job-running=` in the report. Gate: `--ui-probe assist=running` reports
      `true` with a 208 luma difference at the top of the body.
- [x] S12 a schema-invalid credentials *entry* reported "not valid JSON" with no
      path, entry or fix — `CredentialFault::{Io, NotJson, Schema, TooLarge}`,
      each with its own remediation, and the schema one names the entry. The
      second pass reads key names and the JSON *type* of `secret` only; no entry
      is ever deserialized, so no secret is materialized by the diagnosis.
- [x] S13 (confirmed) `commit_key`/`forget_key` wrote the credentials file
      immediately while the settings metadata went only into the dirty draft, so
      Forget then Escape-discard left `saved` claiming `mode: file` with a dead
      fingerprint. Both stores move together now, and the settings file is
      rewritten too when there were no other unsaved edits. Evidence on the
      reviewer's two-provider fixture: after Forget, `assist.json`'s credential
      block is gone and the other provider's entry is byte-identical.
- [x] S14 the `MUSIALIZER_ASSIST_SETTINGS_*` env vars are documented in
      `docs/PHASE0_INVENTORY.md` §9.4, with the two path overrides, and
      including the new `_HOVER` seam.

One seam was added rather than only fixed: `MUSIALIZER_ASSIST_SETTINGS_HOVER`.
The dialog draws into a `Widgets` bank of its own, so `--ui-probe hover=`'s
zeroing of the shell's tooltip dwell never reached it — the badge and
disabled-control tooltips this tranche added were unphotographable, and a slow
run could have popped an unrelated one into a capture. With `_OPEN` set the
dialog's dwell is infinite unless `_HOVER=1` asks for a tip, which is the same
rule `main.rs` applies to the shell's bank. `MUSIALIZER_ASSIST_SETTINGS_ESCAPE`
also defers by one frame when `_ACTIVATE` is set, because an Enter is only
consumed while a control is being drawn — without it the reviewer's
"Forget then Escape" sequence silently tested the Escape alone.

Negative controls, demonstrated and reverted byte-for-byte (`sha256sum -c`):

| perturbation | the gate | the unit tests |
| --- | --- | --- |
| `if !has_model` → `if false && !has_model` in `readiness` | fails: the reason reads `No eligible endpoint` where `No model chosen` is due | **1 failed**, 304 passed |
| the scroll-follows-focus correction skipped | fails: `focus-rect=688,858` against a body ending at 668, `focus-visible=false` | **305 passed** |
| `settings_editable()` forced true | fails: 17 tabstops where 6 are due, and no `load-error=true` | **1 failed**, 304 passed |

The first one is the instructive pair. Its first version **passed**: the gate
asserted only `[blocked]`, and the fixture's catalog also leaves no eligible
endpoint, so removing the model check swapped one blocked reason for another and
the check never noticed. The routing report line carries the readiness *label*
rather than its token because of it — `blocked` is one word for four different
missing pieces, and a check that can only see the word is a check that can only
see a quarter of the invariant. The second is `layout`'s lesson again: a property
the unit suite cannot express at all, caught by a capture.

### AP3 — the AI settings entry point and dialog

Design intent: `docs/LYRICS_TIMING_RESEARCH_PLAN.md`, "Visible settings entry
point and dialog". Authority: `docs/ASSIST_PROVIDER_CONTRACTS.md`. Landed
2026-08-05 in `crates/musializer-app/src/ui/assist_settings.rs` (one module),
plus the heading-row button in `ui/panels/assist.rs` and five wiring lines in
`ui/shell.rs`/`main.rs`. **A settings surface only**: opening it starts no
analysis and cannot touch a job in flight.

- [x] AP3-a entry point: a persistent, text-labelled `AI settings` button with a
      sliders icon at the right of the `ASSISTED ANALYSIS` heading row, drawn
      before the body branches so it is present in every panel state. Evidence:
      six captures, each carrying an `assist settings button:` line naming the
      body it was drawn under, the panel width and whether the icon face loaded
      (`Ready`, `Confirmation`, `Running`, `Candidate`, `Empty`/`Failed`, and the
      960x640 minimum). **No narrow-width second header row**, and that is a
      measurement: the narrowest assist panel any supported window produces is
      668 px, so a second row would be a branch nothing could reach or
      photograph. The auto-scenes toggle yields instead, which is the priority
      the design document sets.
- [x] AP3-b the modal: application-modal inside the window, scrollable, drawn
      over everything, input-blocking. Blocking reuses the oracle's own claim
      rule rather than a new mechanism — a full-window blocker in the shell's
      widget bank, drawn before any panel, and the dialog draws into a bank of
      its own. The timeline strip is **not drawn at all** while the modal is up,
      because `ui/text_input.rs` reads raylib's keyboard directly and a key typed
      into the masked field would otherwise also land in a lyric cue — a
      credential in a `.musi` file.
- [x] AP3-c five sections: Routing (one row per `TC-*`, boundary/route/model/
      fallback/readiness, `TC-MEASURED` shown and locked, pickers filtered by
      contract eligibility *and* the suitability overlay, `unsupported` never
      offered, experimental behind a warning), Local models (models-directory
      ladder with every losing candidate, doctor-style runtime identity),
      Codex (executable readiness, discovered models or exactly `Codex default`,
      reasoning effort per eligible task), OpenRouter (connection state, masked
      key lifecycle, catalog with age and badge, Refresh), Privacy (remote-audio
      policy, ZDR per audio contract, provenance, dry-run route summary).
- [x] AP3-d keyboard: a dialog-scoped traversal ring, which is what makes it
      complete rather than half a model — every focusable control is drawn by
      the one module, so `focus_order` can be built from the live model. Tab /
      Shift-Tab wrap, Enter activates exactly one control, Escape closes with one
      explicit confirm step when there are unsaved edits. Evidence: a capture
      after eight simulated Tab steps (`focus=8/23`, focus-ring chroma 132.1
      against 128.1 for the same crop unfocused), an Escape assertion, and a
      dirty-Escape assertion (`dirty=true confirm-close=true`).
- [x] AP3-e honesty rules, each with its own gate assertion: absent cache is
      "never fetched"; a corrupt settings file is an error and is **not
      overwritten** (the gate re-reads the file); a loose-permission credentials
      file shows the refusal, the mode and the fix, and its mode on disk is
      unchanged; catalog strings are display data only.
- [x] AP3-f the masked key field. The drawn string is a function of the
      credential's **length alone**, which is what makes it assertable from a
      capture rather than from a comment: the gate runs two probes with two
      different 27-character secrets and compares the cropped field, which must
      be byte-identical (and not blank — the darkest pixel is checked at 20). No
      report line, log line or capture may contain the canary, and every dialog
      run greps for it.
- [x] AP3-g no live network in any capture. `Test` is the only control that can
      open a socket and it is stubbed by `MUSIALIZER_ASSIST_SETTINGS_KEY_TEST` in
      every gate run; `Refresh` is gated by `catalog.network_allowed`, which is
      false by default. The real `Test` goes through `curl --config -` on stdin,
      so no flag ever takes a key (E2) and nothing is written to disk.
- [x] AP3-h negative controls, demonstrated and reverted byte-for-byte
      (`sha256sum -c`): removing the modal guard around `timeline_strip` made the
      Assist panel draw underneath the dialog and its `assist settings button:`
      line appear in a dialog run, which the gate catches; drawing the pending
      credential instead of the mask made the two field crops differ
      (`34d17dbf…` against `1404c6bf…` where both are `c4735bf8…` when correct)
      and put `sk-or-v1-MUSICANARY…` on screen.
- [x] AP3-i one thing the gate caught that no review would have: the suitability
      marker was drawn at 10 px, which is outside the native interface bank, and
      the run reported `non-native-requests=40` — the exact scaled-atlas blur F0
      rebuilt the shell's typography to delete. It is 11 px now and every section
      reports zero. Two other defects came from captures rather than tests: the
      marker overlapped the row beneath it at the plain row gap, and the routing
      matrix's compact threshold was first written in *body* widths, which made
      the table collapse at 900 px, come back at 810 and collapse again at 730 —
      the vanish-and-reappear defect `core::ui::transport_bar` was rebuilt to make
      unstateable. It is a dialog width now, with a monotonicity sweep pinning it.

Probe seams, all environment variables in the style of
`MUSIALIZER_ASSIST_PROBE_DIR` because `cli.rs` is not this tranche's file:
`MUSIALIZER_ASSIST_SETTINGS_OPEN`, `_TAB`, `_SCROLL`, `_KEY`, `_NOW`,
`_KEY_TEST`, `_REFRESH`, `_DOCTOR`, `_ESCAPE`, `_DIRTY`, `_ACTIVATE`, and
`_HOVER` (added by AP3-R). The `_ACTIVATE` one is what makes the write path
reachable at all: nothing in a headless run can press a key, so without it
`Save` — and "Enter activates" — would only ever be a unit test. The gate uses it
to commit an override and then re-reads the file. All of them are documented in
`docs/PHASE0_INVENTORY.md` §9.4.

Not built here, and named rather than left implied:

- The `Cancelling` panel body has no probe seam, so the entry point is
  photographed in six of the seven bodies. Adding one needs `cli.rs`.
- Provider `order`/`only`/`ignore` and price bounds are in the schema and in the
  dry-run summary, but have no editor yet; only `zdr_required` is editable.
- Execution ignored these settings entirely. **Tranche AP4 below closed that.**

### AP4 — execution wiring

Landed 2026-08-05. Authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` §5 and §6.
The dialog edited settings that execution ignored; jobs now run on the resolved
routes, and the graph they ran on is what provenance records.

- [x] AP4-a resolve at Start, snapshot immutably. `core::assist::execution`
      owns the resolver — **moved down from `ui/assist_settings.rs`**, which now
      re-exports it, so the dry-run summary and the execution snapshot are the
      same function called twice rather than two tables that can drift.
      `runtime::assist::plan` gathers the impure facts and writes
      `<job folder>/assist-execution.json` before the helper exists;
      `AssistSpec` passes the path, the helper embeds the record in
      `assist-manifest.json` and stamps each artifact's `provenance` with the
      per-contract identity. Schema in `docs/PHASE0_INVENTORY.md` §6.7.
- [x] AP4-b the actual flags. `mimo_openrouter.py`'s hard-coded
      `xiaomi/mimo-v2.5` became `--model`, and it gained `--only`, `--ignore`
      and the three `--max-price-*` bounds; `--provider` already carried
      `order`. `external_analysis.py assist` gained `--execution-snapshot` and
      `--codex-reasoning-effort`, and the three local-runtime flags it already
      had are now passed from `assist.json`'s `local_runtimes`. **`model_id` in
      the manifest is observed, not requested**: the OpenRouter response's own
      `model` field, `codex exec --model`, or the model file the local lane ran.
- [x] AP4-c the credential, deliberately. AP1 stripped `OPENROUTER_API_KEY` from
      the application's environment and the desktop path passes `--no-dotenv`,
      which between them left a remote desktop job with **no key path at all**;
      this is the item that closed it. `AuthorizedCredential` is constructible
      only from a snapshot with a route that opens a socket and this job's own
      confirmation, so a local-only job cannot be handed one however the call
      site is written. It goes into that one child's environment: never argv,
      never a log, never the manifest.
- [x] AP4-d the confirmation shows the resolved graph: one row per composed
      task with its model and boundary, a heading that says whether audio
      leaves, and §5 invariant 1 stated as a measurement rather than a promise
      (`any_fallback_can_raise_boundary` walks the applied policies and the
      whole ladder). Extra height on the app side, like the LT1 review list, so
      `assist_ui_state::ui_layout` and its differential harness are untouched.
- [x] AP4-e boundary invariants in execution. No cross-boundary fallback exists
      anywhere in the path. **`ask` is resolved to `none` at Start** and named
      in the confirmation, because this build cannot pause a running job for an
      answer and a half-built pause that silently continued is the one thing §5
      invariant 1 forbids; that is the implemented semantics, recorded in §6.7.
      A missing key, an unrouted contract and a provider selection that is
      unsatisfiable by construction all refuse **before** anything spawns, each
      naming its own repair rather than the word "blocked".
- [x] AP4-f cache acceptance by route identity (§5 rule 7), the LT1 pattern
      keyed by contract. Evidence from a real local lyrics run on a fixture:
      a second run reports every lane `reused`; editing `TC-ALIGN`'s model in
      the snapshot makes that one lane `generated` and leaves `TC-COARSE`
      `reused`.
- [x] AP4-g negative controls, demonstrated and reverted (`sha256sum -c` clean):
      authorizing every graph put `OPENROUTER_API_KEY=sk-or-v1-MUSICANARY…` in a
      **local-only** job's child environment dump, which the canary scan finds;
      disabling the missing-key refusal turned the gate's
      `blocks=TC-SEMANTIC:No key` into `blocks=none` and failed one unit test.
      A settings edit after Start left the running job's snapshot byte-identical
      (`24bc2044…` both sides) and the manifest still recorded `mms-ctc` and
      `recommended` while the file on disk said `qwen3-fa` and `studio`.

Not built here, and named rather than left implied:

- **`ask` mid-job.** Pausing a running job to offer a substitute route needs a
  job state and a panel body nothing here has. Resolved to `none` at Start with
  a visible note; recorded as the implemented semantics, not deferred silently.
- **`TC-VERIFY` is composed by nothing.** Independent timing verification has no
  lane in the helper, and offering a route for a stage that cannot run is the
  invented capability the honesty rule forbids.
- **A ZDR endpoint list.** `tools/provider_catalog.py` normalizes modalities,
  context and price and carries no endpoints, so "no ZDR endpoint for this
  model" is not decidable offline and is not claimed. What *is* refused
  pre-spawn is a provider selection emptied by construction — `only` minus
  `ignore`, a disjoint `order` with fallbacks off, a zero price bound — with the
  message naming ZDR when it is also set. Beyond that `provider.zdr` is sent and
  OpenRouter refuses rather than substituting.
- **`prefer_gpu` and `stem_separation`** are in `assist.json` and reach no flag
  yet; the helper has no stem lane to point them at.
- **The staged snapshot lives on `AssistController`, not on
  `AnalysisCandidate`.** That type is `musializer-core/src/project/`, outside
  this tranche's files. The two are staged and cleared together, so the pair is
  as inert as the candidate is; moving the field is a one-line change for
  whoever next opens that file.

## P4 — honest status, stale handoffs and final gate

### F0 — robust native-size shell typography

Completed 2026-08-01 (`a664b7d`, operator-requested robustness sweep):

- [x] Replace the single 64 px UI atlas with native Space Grotesk atlases for
      every fitted 11--22 px size and the fixed 24, 28, 34, 38 and 84 px sizes.
- [x] Route all ordinary shell drawing, measurement, buttons, text input and
      panels through a distinct `UiFonts` type that cannot be used as a raw
      raylib face. Keep caption faces and Font Awesome icons as explicit raw-face
      exceptions.
- [x] Quantize fitted rows before measurement, draw UI glyphs 1:1 without
      mipmaps, and snap text origins and tracking to integer pixels.
- [x] Report the native sizes used and fail the headless gate when any shell
      label requests a scaled/non-native size.
- [x] Exercise welcome, every scene and panel, preview/export and both supported
      window sizes through the silent headless path.

Evidence: changing `Faces::ui()` from `&Face` to `&UiFonts` first produced 61
compile errors across raw draw, measure, fitted-row and helper boundaries, which
made the retrofit inventory compiler-complete instead of search-pattern based.
The full headless gate then collected 81 application font reports; every one
loaded all 17 atlases and reported `non-native-requests=0`, while preview/export
determinism and the existing scene contracts stayed green. As the runtime
negative control, changing one welcome label from 15.0 to 15.5 px still rendered
but reported three non-native requests, and the new gate rejected it. The source
perturbation was reverted before the green run.

### F1 — remove false unfinished status

- [x] Stop `toggle_panel` from emitting `ShellCommand::NotImplemented` for the real
      Export, Lyrics and Assist panels; delete the obsolete command if no genuine
      use remains.
- [x] Remove the scene-browser footer `* not ported yet` while every registered
      scene reports a ported drawing path.
- [ ] Replace stale "largest remaining gap" claims in status text with a pointer
      to this plan.
- [ ] Ensure intentionally unavailable actions name their real prerequisite
      (`helper missing`, `FFmpeg missing`, `no track`) rather than saying "stub".
- [ ] Show truthful Saved/Unsaved/Save failed/No project file state in the Tracks
      header, including lyric and route drafts.
- [ ] Show Cadence's preview-only no-timed-lyrics guidance instead of leaving an
      empty scene that resembles a broken renderer.

### F2 — retire completed agent seams without changing behavior

This is centralized dangling-work cleanup, not feature parity by itself:

- [ ] Remove or rewrite obsolete "no caller yet", "Agent X must wire this" and
      "nothing dispatches this" comments whose call sites now exist.
- [ ] Remove stale module/function `allow(dead_code)` attributes that only existed
      for the completed fan-out. Keep intentional future-scene placeholder code
      explicitly documented if it remains.
- [ ] Reconcile the old duplicate `transport_duration_text` boundary note with the
      landed core timing module; deduplicate only with numeric evidence.
- [ ] Review the old requests to hoist shared scene draw helpers. Consolidate only
      where it reduces live duplication without perturbing verified formulas.
- [ ] Keep `REWRITE_PLAN.md` as append-only historical evidence; do not rewrite its
      old session statements to pretend they were never true.

### F3 — optional structural cleanup, explicitly not a parity blocker

- [ ] Consider moving `Workspace::assist` to `Shell` and deleting its four `Cell`
      fields. Do this only with the existing Assist differential and capture suite
      green before and after.
- [ ] Treat further ownership, module and math-helper consolidation as normal
      refactoring. It does not belong on the parity critical path unless it fixes
      an observable defect.
- [ ] Fix or formally close the FFmpeg test-helper `ETXTBSY` race. The historical
      report named
      `process::ffmpeg::tests::a_completed_encode_is_published_by_rename`; on
      2026-07-31 `tools/verify.sh --quick` instead failed
      `an_encoder_that_ignores_the_pipe_closing_is_killed_after_the_grace_period`
      at helper spawn with `ExecutableFileBusy`, after which that exact test
      passed in isolation. Treat this as a shared generated-executable race, not
      as one flaky assertion, and retain a regression stress test when fixed.

### G1 — integration gates that cover the discovered holes

- [ ] Add a seeded-project headless scenario that simultaneously proves lyrics,
      semantics, manual events, scene plans and cue settings reach preview.
- [ ] Run the same project through full and windowed export and compare selected
      frame hashes/state reports.
- [ ] Add a drop-dispatch test, lyric TSV test, autosave-multiple-tracks test and
      packaged-helper discovery test.
- [ ] Add CLI ordering/side-effect, interactive scene-plan, project-preset
      adoption, Tune overflow/toggle, transactional-seek and font-application
      gates from D4-D8, B4, C5 and E3.
- [ ] Every new gate gets a negative control; record what the perturbation broke.
- [ ] Choose further differential harnesses by evidence weakness, not by counting
      modules. A property-only unit suite is the strongest candidate signal.

### G2 — final feature-parity audit

- [ ] Walk the C application surface from `docs/PHASE0_INVENTORY.md`: CLI, file
      types, panels, timeline gestures, projects, scenes, captions, Assist, fonts,
      presets, events, export, launcher and distribution.
- [ ] For every C capability, record `fit`, `improved mechanism`, `deliberate
      exclusion` or an open task id in the matrix below. No uncategorized row.
- [ ] Run `tools/verify.sh`; require 0 failures and a clean oracle at `9300af9`.
- [ ] Open a C-written `.musi` in Rust and a Rust-written `.musi` in C, including
      embedded ASCII image/font assets, lyrics, semantic/manual lanes, routes,
      presets and enabled scene cues.
- [ ] Verify an interactive session on the operator's real desktop separately from
      Xvfb: dialogs, drag/drop, fullscreen, audio controls and launcher startup.
      Audio playback and the Spectrum onset-floor fix were accepted on 2026-08-03;
      the other interaction families remain open.
- [ ] At completion, the deliberate exclusions above are the complete observable
      difference list. Update `README.md` and `AGENTS.md` from this file.

## Feature-surface ledger

| C feature family | Current Rust state | Closure |
| --- | --- | --- |
| Ten scenes and audio reaction | Fit, including saved semantic/lyric/event inputs | none; retain G1 evidence |
| Shared captions and Cadence lyrics | Render integration fit; Cadence empty-state guidance missing | F1 |
| Automatic scene plans | Fit in preview/export, CLI and the interactive Assist header | none; retain G1 evidence |
| Manual scene cues | Capture/base/pending semantics and editable timeline lane fit; boundary render evidence incomplete | B3 |
| Settings/routes/presets | Core fit; dirty/reset, project-preset adoption and Tune control gaps remain | C1, C3, C5, D6 |
| `.musi` open/save | Differential round trip fit | C1-C5 guard/autosave/adoption completion |
| Multi-track workspace | Fit interactively | C2, C4 |
| Lyrics editing | Editor fit; document import/export and broad draft guards missing | C2, D3 |
| Manual/semantic events | Scene consumption fit; timeline markers missing | D4 |
| Timeline | Waveform/zoom, transactional seek and scene-plan editing fit; lyric/event lanes and pan missing | D4 |
| ASCII image mode | CLI/project rendering fit; GUI and typed drop missing | D1-D2 |
| Assist | UI/controller/bridge and source bundle fit; live/fullscreen/helper-state validation remains | C2, E2-E4 |
| Google Fonts | Job/manifest verification exists; background polling and project application are unwired | E3 |
| FFmpeg export | Project lanes and scene plans fit; remaining boundary evidence is B3 | B3 |
| CLI | Documented flags and execution order fit; reload excluded | none |
| Tune UI | Settings/routes/presets exist; scrolling and toggle presentation differ | D6 |
| Keyboard workflows | Transport/fine-seek plus Rust additions fit; C save/export/direct-scene keys missing | D7 |
| Notices/save status | Basic transient notices fit; actionable tray and save-state visibility missing | D8, F1 |
| Linux launcher/MIME | Source install fit | E4 |
| Self-contained distribution/doctor | Missing | E4 |
| User-perspective review | Complete evidence review exists; every named defect, workflow, opportunity and blind spot is open as the next tranche | UX0 |
| Microphone/hot reload/non-Linux | Deliberately excluded | none |

## Consolidated dangling-note ledger

This table resolves the old planning fragments so they are not rediscovered as
parallel task lists.

| Inherited note | Disposition |
| --- | --- |
| `REWRITE_PLAN.md` TODO: route editor UI and persistence | Completed and differentially covered; stale historical TODO, not active work |
| Completion-plan W1/W2 and G through O | Landed; retain NOTE ENTRIES as history. Remaining integration gaps have new A-G ids above |
| Agent I: Lyrics panel call sites | Landed; only TSV import/export and broad draft guards remain, D3/C2 |
| Agent J: Assist panel seams | Landed; real helper bundle and lyric-draft conflict remain, E2/C2 |
| Agent K: font-browser seams | UI/job controller landed; background polling and transactional project application remain E3; stale comments go to F2 |
| Agent L: event/preset call sites | Event frame consumption landed; timeline markers remain D4 and project-preset adoption is C5 |
| W1 note: autosave only current track | Active as C4 |
| `Workspace::assist` ownership deferral | Optional structural cleanup F3, not a parity requirement |
| Item O: add every possible harness | Reframed as risk-based evidence work in G1 |
| Recorded intermittent FFmpeg `ETXTBSY` test-helper race | Reproduced in a second test and retained as test-reliability cleanup in F3 |
| `UX_PERSPECTIVE_REVIEW.md` | Evidence companion, fully normalized into next-task umbrella UX0; it is not a parallel queue |
| Source comments naming agents/integration-owner handoffs | Audit and remove as F2 after verifying each call site |
| Ignored `.claude/worktrees/**/REWRITE_PLAN.md` files | Historical worktree snapshots, not tracked plans and not sources of current tasks |

## How this file stays authoritative

- Start work by claiming one task id here and recording dependencies.
- When a task lands, check its boxes and append the commit plus evidence under the
  task. Do not add a second completion plan elsewhere.
- New C-only behavior discovered during implementation becomes a task here before
  it becomes code.
- A task may be removed from the critical path only as `completed`, `duplicate`,
  `not observable`, or an operator-approved deliberate exclusion, with the reason
  recorded here.
- Keep historical investigation and negative-control details in
  `REWRITE_PLAN.md` NOTE ENTRIES or focused code/test comments; keep the live queue
  here concise and current.
