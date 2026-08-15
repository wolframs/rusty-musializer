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

## PX3 — a clip and a still: the two files a track becomes (UX0-C01, UX0-C10)

> *"post the chorus as a teaser"* and *"I need a cover"* — the two exports this
> application could not do, in a wave whose brief is the moment work becomes
> shareable.

**Both are post-legacy product extensions. The frozen C has neither.** It renders
whole tracks to MP4 and nothing else. `render_export_window_frames` exists in the
C transport and `plug.c` calls it only from `plug_configure_render_window`, a
command-line entry point — so a clip was expressible on a command line and
nowhere a user could see or edit it. A still frame is not expressible at all: to
get a cover out of the frozen binary you export a video and pull a frame back
through h264 4:2:0, which is measurably lossy on exactly the thin saturated
features these scenes are made of.

| id | Work | Where | State |
| --- | --- | --- | --- |
| PX3-a | `ClipSelection` — the editable window, its clamping, and the frame count a readout can print | `core/timing/render_export.rs` | **done** |
| PX3-b | The CLIP row: Full track, In <- playhead, Out <- playhead, and a readout naming the window | `ui/panels/export.rs` | **done** |
| PX3-c | The clip reaches `RenderRequest::window`, and `--render-window` seeds the row | `main.rs` | **done** |
| PX3-d | `Save still`: the frame at the playhead as a PNG, through the export renderer | `ui/panels/export.rs` | **done** |
| PX3-e | `--ui-probe save-to=`, `export clip:`, `export still:`, and a gate section that presses all of it | `cli.rs`, `main.rs`, `tools/headless_check.sh` | **done** |
| PX3-f | `SHARE FRAME`: use the playhead's deterministic picture as encoded frame zero without shifting the export | `ui/panels/export.rs`, `main.rs` | **done** |

**One press is a renderable clip.** `set_start` selects from the playhead *to the
end of the track* and `set_end` from the start *to here*, so the common gesture —
"from the drop onward" — costs one click, and the second click only closes it.
Setting either end past the other re-reads as an open-ended selection rather than
being refused: a refusal leaves the control looking broken at the moment the user
is moving fastest, and both readings are single gestures somebody makes on
purpose. `every_edited_selection_is_a_window_the_transport_accepts` sweeps the
reachable states and proves each one is a window `window_frames` accepts, which
is what keeps a clip export from failing at start with a notice nobody can act
on.

**The row is drawn first, above SIZE, and that is arithmetic rather than taste.**
The timeline band grows upward from the window's bottom edge while the footer
stays pinned to it, so a row added at the *top* leaves every control below it at
the same screen coordinate — which is what kept EX1's and EX2's hand-aimed
click-probe coordinates valid. A press aimed one row off does not fail; it
presses a different control and asserts against its result.
`adding_a_row_above_size_leaves_every_control_below_it_in_place` pins the
distance from the panel's bottom edge (185 px to SIZE, 139 to QUALITY) so a later
edit breaks a test instead of silently re-aiming the gate.

**The third row cost 20 px of band, not 46, and a failing test is why.** Three
control rows under the old full-width description line ask 424 px of timeline
band; a 640 px window can only give 410, and `the_export_panel_yields_before_the_preview_does`
failed with exactly that pair. That is not a test being fussy — it is review
1.4's defect returning at the smallest supported size, a lit Export button over a
"needs a taller timeline" notice. The description moved up beside the EXPORT
title, where it costs no vertical space and shortens itself when the panel is
narrow.

**The still is the video frame, and proving that took a cross-check rather than a
hash.** `with_export_frame` and `draw_offline_frame` are now shared by
`ExportSession::step` and the still, so the analyzer feed, the beat phase, the
automatic scene switch, the routed settings, the project lanes, the supersampled
target and EX3's linear-light resolve are one implementation. The frame index is
`still_frame_index`, the same floor `window_frames` uses for a window's start, and
every frame from zero is prepared before it — for the reason `render_job.rs` gives
for never seeking.

**It shipped upside down, and two md5-equal runs called it correct.**
`LoadImageFromTexture`/`rlReadScreenPixels` return an OpenGL framebuffer whose
first row is the *bottom* of the picture, which is why
`Encoder::send_frame_flipped` exists (`ffmpeg.rs:448-455`). The PNG writer copied
the same buffer top-down. A mirrored Spectrum — bars hanging from the top instead
of standing on a baseline — is a completely plausible picture, it rendered
identically every run, and the determinism check hashed two of them and passed.
Only comparing the PNG against a frame decoded out of an MP4 of the same moment
caught it:

| comparison | before the fix | after |
| --- | --- | --- |
| still at 4.0 s vs the MP4's frame at 4.0 s | 22.68 dB | **45.55 dB** |
| the same still vs the MP4's frame at 4.5 s (control) | 21.48 dB | 21.53 dB |

The control is the half of it that matters: without it, 45 dB proves nothing,
because two adjacent frames of a slow scene would also score well. The ceiling is
h264 4:2:0 on a saturated cyan feature, not our error — EX3 measured the same
codec turning a 1 px cyan line into `(97,227,225)`.

**The share frame changes the embed, not the timeline.** `SHARE FRAME -> Use
playhead` prepares the selected time through the still's deterministic replay,
then resets the analyzer, beat tracker, automatic plan and `SceneInstance` before
the real export begins. The ordinary first timeline frame is still updated and
drawn — its pixels alone are discarded — and the selected pixels are handed to
the encoder for encoded frame zero. No frame is inserted, duplicated or moved;
audio, duration and every later scene update keep their normal timestamp. The
choice is opt-in and session-only, so `Normal` preserves the previous export path
and remains the visible default.

**Evidence.**

| claim | how |
| --- | --- |
| Window arithmetic and validation | 8 new tests in `core::timing::render_export`, including a sweep over every reachable selection and a re-clamp against a shorter track |
| The CLIP row takes a press | Gate: `click=` on In and Out with `time=` parking the playhead, each asserting the whole `export clip:` line **and** an unchanged `export config:` |
| Nothing pressed nothing | Gate: a click in the 8 px gap between Full track and In — `claimed=nothing`, clip unchanged |
| The clip reaches the file | Gate: the render button pressed with `save-to=`, then ffprobe — **90 frames, 3.000 s**, and `export frame lanes: t=2.000` proving it is a window rather than a short render from zero |
| The still is deterministic | Gate: two runs, identical md5 |
| The still is *that* frame | Gate: PSNR against the MP4's own frame, with the frame 0.5 s later as the negative control |
| The share choice takes a press at the 960x640 minimum | Gate: `click=` on `Use playhead`, then `export share: playhead at 4.000 s` and a non-empty widget claim |
| Only encoded frame zero is substituted | Gate: a 2.0–3.0 s clip with a 4.0 s share frame remains **30 frames / 1.000 s video / 1.000 s audio**; frame zero matches the ordinary 4.0 s export at >40 dB, rejects the ordinary 2.0 s opener at <30 dB, and frame one still matches the ordinary clip at >40 dB |
| 16:9 full-track export is unchanged | `85d6dcc7b6fe71ba8ce4e010f1f781e4` from a build of `9db6684` and from this branch — same md5, not "looks the same" |
| Panel ids do not collide | `the_panels_own_widget_indices_never_collide` claims all 23 indices this panel mints; `widgets::id::ALL` only protects namespaces, and EX1 was an index collision |

**`--ui-probe save-to=PATH` is new, and it is `click=`'s missing half.** EX1's
probe proves a control takes a press; it cannot prove the press produced a file,
and the two controls this panel exists for both open a modal picker Xvfb does not
have. `save-to=` substitutes for the *dialog*, not for the decision — every
refusal after it still applies. It is what makes the clip section the first gate
in this repository to produce an MP4 by pressing the button a user presses.

**Divergences recorded.**

| The oracle | Here | Why |
| --- | --- | --- |
| A render window exists in the transport and only `--render-window` can ask for one | A CLIP row on the export panel, with both ends set from the playhead and a readout of the window and its frame count | UX0-C01. The teaser is the shareable artifact of an AI-music workflow and the C could not make one without a command line |
| `--render-window` is a command-line-only state | It seeds the panel's CLIP row, and the panel is what the render button reads | A panel that said "whole track" while the flag was in force would be lying about the file it is about to write |
| A clip and a full render propose the same file name | `suggest_clip_path` adds `-clip-MMmSSsMMM-MMmSSsMMM` | Otherwise the teaser silently replaces the full render of the same track and scene |
| No still export at all | `Save still`, through the export renderer, needing no encoder | UX0-C10. It is the one control in this panel that works without FFmpeg installed |
| Every MP4 begins with its timeline's first frame | `SHARE FRAME` can replace encoded frame zero with the playhead's deterministic render | Social embeds commonly choose the first decoded frame; the user can choose an inviting preview without changing the song or video timing |

**Not done, deliberately.**

- **The clip is session-only and does not persist into `.musi`.** A clip is
  something you are doing now, not a property of the project, and a reopened file
  quietly rendering 30 s of a four-minute track is worse than re-selecting.
  Whether it should become a durable per-track field is a **schema question for
  the integration owner**; the state lives on `Shell` so promoting it later is a
  field move, not a redesign.
- **No in/out handles on the timeline.** The playhead buttons are the whole
  gesture today. Dragging the window on the timeline strip is the affordance a
  user would reach for next, and it belongs to the timeline's own gesture owner
  (`shell.rs`), not to this panel.
- **The still is not reused for scene thumbnails (UX0-C05) or cover output
  (UX0-C09).** The share-frame feature reuses its one-frame renderer, but it is
  intentionally not CX-5's cached, revision-keyed, cancellable poster service.
  Scene/recent-project thumbnails still need that service rather than calling
  this synchronous path ten times.
- **A long track's still blocks the frame loop.** Decoding and fast-forwarding to
  the playhead is a second or two on an eight-second fixture in a debug build and
  will be longer on a real track; the window draws "Rendering still frame" and
  then holds. The export's own progress screen is the model if it becomes
  annoying.

## PX2 — the lyric timing loop gets its loop (2026-08-07)

The review's verdict was that the editor's foundation was good and the *loop*
was missing: "timing 60 lines is ~360 precise clicks across two panes". Six
items closed together — UX0-B02, B03, B04, B05, UX0-C03, D3 and D4's cue-lane
item — because they are one workflow and closing any one of them alone leaves it
still unusable. The per-item evidence is in each checkbox above; what follows is
the reasoning that spans them.

| Item | Landed as |
| --- | --- |
| UX0-B02 | `select_and_seek`, one `Seek` per frame, drained in `lyrics_panel` |
| UX0-B03 | `LyricHistory`, 64 snapshots, one per drained batch |
| UX0-B04 | `cue_nudge_step_seconds`, `HoldRepeat`, `parse_cue_timestamp` |
| UX0-B05 | `LyricsEdit::Split`/`Merge`, Ctrl+B / Ctrl+J, `split_text_at_fraction` |
| UX0-C03 | `LyricTap` — arm, then one press per line |
| D3 | `ExportLyrics`/`ImportLyrics`, `import_bridge_document` |
| D4 (lane) | `closed_lyric_lane`, in slack the band already reserved |

**Enter is the tap key, and the choice was made by elimination rather than
taste.** `Shell::keyboard` reads the keyboard *before any panel draws*, and its
globals are unconditional once no text field has focus: `T` opens the inspector,
`M` mutes, `S` saves, `F` goes fullscreen, `H` toggles the readout, Space plays,
Tab cycles the scene, the arrows seek. Neither side consumes a press, so a panel
key shadowing one of those fires **both**. Enter was the only unbound key that is
large, central and reachable without looking — which for a control pressed in
time with music is not a nicety. Ctrl+Enter arms, and cannot collide with the
form's own Ctrl+Enter because that one requires the cue field focused and an
armed run requires it blurred.

**That exclusion is also the fix for the defect the review named.** `begin_new`
focuses the text field, a focused field stands down every transport key including
Space, and so the natural add → type → play → tap loop broke exactly at the tap.
Rather than carve an exception into the focus rule, tap mode and text entry are
**mutually exclusive states**: arming blurs the field, clicking a field disarms
the run. There is no key that means two things, so there is nothing to get wrong.
The same rule made `is_typing` answer for the new typed-time fields too — a second
text surface that did not answer it would have re-created UX0-A06 one field over,
with `1:23` seeking the track and toggling the readout.

**Snapshots, not inverse edits, and the note is in the code because it will look
like an easy simplification later.** An inverse has to reproduce everything the
forward edit touched, and these operations touch more than they name: `split`
allocates an id from `next_id`, `merge` destroys one, and `update`/`retime`
promote `CueOrigin` to `UserApplied` — provenance that is load-bearing rather than
decorative, since `at_time` refuses to hand a `Potential` cue to a frame. An
inverse log that gets any one wrong produces a document that is *valid* and
quietly different, which is the failure no test written from the forward
direction catches. A snapshot cannot be wrong about any of them, and the
round-trip test is an equality rather than a list of properties someone thought
of. `revision` deliberately does not round-trip: an undo *is* a change.

**The tap cursor is a snapshot of ids taken at arming time, and this is the one
piece of the design that is not obvious.** A stamp *moves* a cue and the document
sorts by start time, so stamping line 1 at 0:50 while lines 2-6 still sit at their
imported 1-3 s re-sorts it behind them. A cursor walking canonical order then
hands out line 2 twice and never reaches line 1. Capturing the order at arming
makes that unstateable rather than guarded against, and
`a_run_survives_the_re_sort_its_own_stamps_cause` is the pin.

**What the captures found that the tests did not.** Two defects surfaced from the
first headless run, both invisible to unit tests and to a screenshot:

- A tap bound the form from the document while its own retime was still pending,
  so the draft was compared against the *pre-stamp* span and came out dirty. Every
  tap therefore armed the "Finish the lyric edit first" guard and the next press
  was refused — a tap run that stops after one tap. The report line said `draft
  dirty` after a clean run of four; nothing else could have.
- The undo probe checked `can_undo()` before the frame loop, when the taps it was
  meant to reverse had not drained yet, so `lyric-tap=N,lyric-undo=1` refused
  itself. The gate now asserts the `history u/r` pair instead of the exit status.

**The control ladder's monotonicity sweep also earned its keep on first run**, by
failing: it swept narrow-to-wide, which is not the direction the claim is about.
The property is that as the panel *loses* room controls may only leave —
`transport_bar`'s lesson, where greedy placement made mute vanish and then
reappear as the window narrowed.

**Divergences from the frozen C**, all additive; none changes a `.musi` file, a
rendered frame, a settings bound or the CLI grammar:

| The oracle | Here | Why |
| --- | --- | --- |
| A row click selects and never seeks (`lyrics_editor_ui.c:1255`) | It selects **and seeks to the cue's start** | Checking a line meant reading its timecode off the row and scrubbing to it by hand — 60 lines is 120 of those. The start rather than the middle, so pressing play immediately answers "is this on the beat" |
| No stamping at all | `LyricTap`: arm, then one Enter per line, each press closing the line before it | The review's "~360 precise clicks across two panes". The pairing is review 1.14's rule applied as a gesture rather than as a default, which is what makes a run produce contiguous captions |
| — | A tap offset, `[` / `]`, ±250 ms, session-only | A player who lands consistently early or late needs to say so. **Defaults to 0**: a silent −200 ms would be the tool lying about the data it recorded |
| No undo anywhere | 64 document snapshots, one per user action | Delete was one unconfirmed click and a 3 px accidental lane drag committed a 64-cue `ShiftMany` with no way back. Binding Delete to a key is only defensible because this exists |
| A fixed 0.1 s nudge, no repeat (`:219-250`) | The transport's Ctrl/Shift ladder at a tenth of its scale, hold-to-repeat, and labels that state the current step | A cue boundary is placed against a syllable and a playhead against a section, so the shape is shared and the numbers are not. A fixed "-0.1" label lies whenever Ctrl is down |
| Times shown to the millisecond, typeable in tenths | The readout is a field; `parse_cue_timestamp` refuses rather than guessing | It has shown milliseconds since the C and never accepted them. `-1:30`, `1e3`, `+30` and `1:60` are all refused, because a silently-clamped typed time is a control that lies |
| `lyrics_split`/`lyrics_merge` ported, tested, with no call site | Ctrl+B / Ctrl+J and two buttons; split divides the text at the space nearest the seam | Splitting a long imported line is the commonest subtitle edit there is; the only route was retyping both halves |
| Export/Import offered unconditionally | Export is disabled for a document `bridge_export` would refuse | A dialog that ends in an error is worse than a button that says it cannot |
| The cue lane exists only while the editor is open | It draws in every state except Export and Assist | Timing is judged against the waveform and the scene plan, which stay. It costs no band height — see D4 above |
| — | `--ui-probe lyric-tap=N`, `lyric-undo=1`, and `tap …, last stamp …, history u/r, typing …` on the `lyrics:` line | Xvfb has no keyboard. An armed run and a disarmed one differ by one 11 px line of hint text, and a stamp that landed leaves exactly the frame a refused one does |

**Left undone deliberately**, so the next session does not have to rediscover it:

- **UX0-B06 is untouched** (large-document navigation). Up/Down now walk the list
  and seek, which is a piece of it, but the scrollbar still claims no widget id
  and there is no jump-to-playhead.
- **The undo history is per-session and per-current-track**, cleared by
  `enter_track`. Cue ids restart at 1 in every document, so one track's snapshot
  would restore another track's cues wholesale. A per-slot map is the obvious
  follow-up and was not built.
- **The tap offset is session-only.** Persisting it means a field in
  `UiPreferences`, which is another tranche's file this session.
- **Cancel and overwrite confirmation for the TSV dialogs are not gate-covered.**
  They are the dialog layer's own behaviour and the gate cannot reach them
  without opening a modal Xvfb has no way to answer.

## PX — wave debrief follow-ups (2026-08-07)

Six agents (PX1–PX6) closed C1, C4, D1–D4's remainder, D3, UX0-B01–B05,
UX0-C01, C03, C04, C06, C07 and C10 in one wave; the six merges are
`03548b9`..`e63fee2` and each agent's honest "what is missing" survives here.

**Five of these were filed as "operator" taste calls and that was wrong.** The
operator's response, 2026-08-07: *"these are hardly personal-taste questions so
much as questions whose answers should be given from an 'am I producing good
software with good UX and good value?' perspective"*. They were put to
`gpt-5.6-sol` through five headless `codex exec` consultations
(`build/consult/`, gitignored) and every one came back decided. The rulings are
in **CX** below; the items here now carry them. What actually needed a human was
much smaller than the label claimed — see CX-4.

- [x] **PXF-1 — "No file" should escalate with accumulated edits (PX1).** A
      user who tuned for an hour with no project file sees calm grey,
      indistinguishable at a glance from Saved. `needs_attention()` returning
      false is right on frame one and wrong after real edits; escalate
      `NoProjectFile` once durable edits accumulate. Also: fullscreen has no
      save-state surface at all. Done by CX-3, 2026-08-11: bounded transaction,
      time and significance escalation; two-generation recovery including dirty
      drafts; welcome-screen restore; and the fullscreen attention dot are all
      application-boundary gated.
- [ ] **PXF-2 — persist the tap offset (PX2).** `[`/`]` calibration resets
      every launch, which for a calibration control is exactly wrong. Needs a
      `UiPreferences` field.
- [ ] **PXF-3 — a tap should flash its block in the lane (PX2).** The only
      feedback is an 11 px counter the player is not looking at during
      playback.
- [ ] **PXF-4 — the clip is invisible on the timeline (PX3).** In/Out are set
      against a waveform that shows nothing; drag handles on the strip belong
      to the timeline gesture owner, not the export panel.
- [ ] **PXF-5 — the still blocks the frame loop (PX3)** with a static
      "Rendering still frame"; the export's own progress screen is the model.
- [ ] **PXF-6 — scene thumbnails are the unlock three agents pointed at
      (UX0-C05).** PX4's recent rows are text, not a shelf of work; PX3's
      still-frame path is the natural renderer for them and for cover output.
- [ ] **PXF-7 — resume position:** `Metadata` has no playhead field, so
      reopening a recent project starts at 0:00 (PX4).
- [ ] **PXF-8 — the event markers do NOT want a legend (PX5).** Decided by
      CX-1: a legend explains an encoding the eye still cannot resolve, and
      spends band height to do it. Replace the encoding instead — authorship
      becomes **top rail vs bottom rail**, not disc vs ring.
- [ ] **PXF-9 — the clip window belongs in `.musi`, as one of a list (PX3).**
      Decided by CX-2, with two prerequisites and a defect found on the way. See
      CX-2; this is no longer a field move.
- [ ] **PXF-10 — the app segfaults rather than exiting with a message when the
      display has no GL (PX5, and 11 coredumps during the wave).** raylib's
      `InitWindow` dereferences null; guard before init and say what happened.
- [ ] **PXF-11 — most of this was derivable and has now been derived.** The
      claim that a listening session was needed held for **one** of the three
      questions. CX-4 rules: the tap offset default is **-100 ms** with a
      calibration gesture that replaces it (research-grounded, not taste); the
      tap feedback treatment is **visual, with no stamp click** (a click masks
      the onset the user is judging and is known to perturb synchronisation);
      Surprise's constants are **three-of-five wrong** on argument alone. What
      genuinely needs ears is narrow: whether the *revised* Surprise bias
      produces keepable results. CX-4 defines that as a **nine-minute blind
      seed comparison** with a pass condition, not an open-ended session.
- [ ] **PXF-12 — B09 remainder (PX6):** the route editor's 104-band stepper is
      a different control with a different failure mode, deliberately not
      taken; keyboard nudge conflicts with the transport's arrow keys and
      needs a decision rather than a silent resolution.

## GX — the gate was not hermetic, and the PX wave's green was not reproducible (2026-08-07)

Found while re-running `tools/verify.sh` after the consultation, and it is the
most important thing on this page. **Fixed**; recorded because the failure mode
is subtle and will otherwise be reintroduced.

`crates/musializer-app/src/ui/preferences.rs` resolves `ui.json` through
`XDG_CONFIG_HOME` with a `$HOME/.config` fallback, and `tools/headless_check.sh`
set neither. So every capture was laid out against **the operator's real
splitter positions** — `sidebar_width`, `inspector_width`, `timeline_height`,
`lyric_lane_height`. Three PX sections then hard-coded pointer coordinates read
off that layout:

| section | probe | was | is |
| --- | --- | --- | --- |
| D4 timeline (PX5) | `hover=`, `middle-drag=` | y=470 | y=610 |
| first-run entry points (PX4) | `click=` Import / Clear | y=246 / y=271 | y=375 / y=400 |
| Tune wheel (PX6) | `hover=` amplitude row | x=900 | x=1050 |

Those coordinates **never** worked against a default configuration — verified by
building PX5's own merge commit `683395a` in a worktree and probing it with an
isolated config home, where the marker row is at 610 exactly as it is on master.
The wave's `22 passed / 0 failed` was true only for the preference state on this
machine at that hour. When the operator opened the application after the wave
landed and moved a splitter, **twenty assertions began failing at once**, and
every one of them read as a feature regression in a shipped feature.

The fix is three exports at the top of `headless_check.sh` — `XDG_CONFIG_HOME`,
`XDG_STATE_HOME`, `XDG_DATA_HOME` all pointed at scratch directories under
`$OUT_DIR`, wiped every run — plus the re-calibration above, each value
re-derived by probing and confirmed to reproduce the assertion's *existing*
expected output. Not one expected value changed: the logic was right and only
the coordinates were wrong, which is exactly why this was invisible.

`tools/timeline_tick_plate.py` needed a real fix too. It locates the waveform
lane by finding the **longest** run of blue-dominant rows, which is only the
timeline while the timeline is taller than the preview — Spectrum's own bars are
blue-dominant, and at the default 1280x720 layout they occupy 51 rows against the
lane's 46, so the tool measured the preview and reported its vacuity guard. It
now takes the **lowest** substantial run, because the band's position is
structural where its height is not.

Three lessons, in the order they cost time:

- **A gate whose result depends on a file outside the repository is not
  evidence.** It cannot fail on someone else's machine and it cannot fail twice
  the same way on this one. Isolate every per-user root, not just the one a
  section happened to need — `headless_check.sh` already isolated
  `XDG_CONFIG_HOME` for the route-persistence section and `HOME` for the codex
  discovery section, so the *idea* was present and applied locally instead of
  globally.
- **A hand-derived pixel coordinate is a hard-coded expectation about a layout
  nobody pinned.** The repository's own rule — "a property assertion cannot pin a
  value" — has a mirror image: a *value* assertion about geometry needs the
  geometry pinned, or it pins the machine. The durable fix is to make the probe
  name its target (`click=import-ascii`) or to report lane rectangles so the gate
  can compute the point. Filed as **GX-1**.
- **Locating a rectangle by "the biggest thing that looks like it" is a
  heuristic, and heuristics need a negative control too.** The tick-plate tool's
  comment correctly argues against hard-coded band heights and then chose a rule
  that silently selects a different rectangle when the window changes shape.

- [ ] **GX-1 — probes should name their target, not its pixel.** Either accept a
      widget name in `--ui-probe click=`/`hover=` and resolve it through the id
      table, or add a report line carrying the lane and control rectangles so the
      gate computes the point it presses. Until then every coordinate in
      `headless_check.sh` is a latent version of the defect above.
- [x] **GX-2 — the 25 ms frame-stall assertion measures the machine, not the
      build** (fixed 2026-08-08, third session burned). The evidence: failed
      twice at load average 15 — worst 25.3 ms, then 33.2 ms — and passed on
      the same commit minutes later at load 2 with worst 16.7 ms, 0 of 240
      stalled; headroom is 8.3 ms and contention consumes all of it. The fix is
      this entry's own prescription: the 1-minute load average is sampled on
      both sides of the ordinary run and printed with the verdict, and the
      failure fires only when the machine was **quiet** (both samples under
      half the cores). Above that it is a loud ADVISORY carrying the load —
      a stall under load is contention, not a verdict, and two agent sessions
      sharing the box must not make the gate un-runnable. A regression cannot
      hide behind a quiet machine, and the deliberately-stalled control
      (129 ms against the 25 ms budget) keeps its unconditional hard exit, so
      the check's discriminating power is untouched.

## CX — the consultation rulings (2026-08-07)

Five design questions the PX wave left open were put to `gpt-5.6-sol` in
parallel, read-only, each with the repository's own files named as required
reading. The prompts are in `build/consult/q*.md` and the answers in
`build/consult/final-a*.md`; both are gitignored, so the rulings that matter are
transcribed here. **Every code claim in them was verified against the tree
before being recorded** — one was a live defect nobody had noticed, and it is
fixed below.

### CX-0 — the defect the consultation found on the way past

`suggest_path`/`suggest_clip_path`/`suggest_still_path` printed the export's
`height` as the `p` rung. EX2 defines the rung as the **short** edge, so:

| geometry | proposed | should be |
| --- | --- | --- |
| 1920x1080 (16:9) | `1080p30` | `1080p30` — unchanged |
| 1080x1920 (9:16) | **`1920p30`** | `1080p30-9x16` |
| 1080x1080 (1:1) | **`1080p30`** — collides with 16:9 | `1080p30-1x1` |
| 1234x568 (no preset) | `568p30` | `1234x568-30fps` |

The 1:1 row is the one that costs work: a square render and a wide render of the
same track and scene proposed **the same file name**, so the second silently
replaced the first — the exact collision `suggest_clip_path` was written to
prevent, one axis over. Fixed with `Aspect::file_token` plus
`video_token`/`still_token`; 16:9 stays byte-identical because "1080p" already
means 1920x1080 to everyone, and the marked case carries the mark.

### CX-1 — event markers: no legend, change the encoding (PXF-8)

- **A legend is the wrong instrument.** It would teach a distinction the eye
  still cannot make: the head is a 5 px disc with a 2 px ring, over a waveform.
  LX1 is not a precedent — lyric provenance rides 50–66 px blocks and decides
  whether a cue renders at all.
- **Move authorship from shape to position: two edge-anchored rails.** Manual
  events keep the top-anchored lollipop (filled disc, solid stem); proposals
  become **bottom-anchored flags** (upward tab, dashed stem). Colour keeps
  meaning type in both. Top-versus-bottom is perceivable *before* either shape
  resolves, costs no band height, and settles the `+ Feel` ambiguity that forced
  the second channel: an amber top marker is mine, an amber bottom marker is
  proposed.
- **Provisional means patterned, not faded.** The 0.45-alpha full-height stem is
  what makes a set of proposals read as fog. Use a near-opaque flag with a 1 px
  outline and a crisp dashed stem (~3 px on, 3 px off), short by default and
  extended to a full-height high-contrast guide only on hover/selection. No
  hollow centre — punched-out detail is the first thing aliasing destroys at
  this size.
- **Cluster by projected pixel separation, not by zoom factor.** At whole-track
  density, collapse near proposals into one type-coloured capsule with a count.
- **The invitation (D of the question): make proposals a review queue.** An
  actionable summary in the existing event-row footprint — `AI proposals · 14 ·
  Review` — entering a focused pass that dims manual markers, zooms to the first
  cluster, auditions from just before each proposal and offers
  Accept/Move/Dismiss/Next, with `5 of 14 reviewed` and full undo. Accept
  promotes the event into the manual lane keeping its type. That turns the
  marker from metadata into the entrance from analysis to authorship.
- **Acceptance test:** with no hover and no help, a user names manual vs
  proposed at a glance for randomly indicated marks, and 14–30 proposals do not
  obscure the envelope. Calibrate the cluster threshold from captures at 720p,
  100 % and 150 % UI scale, over one sparse and one dense waveform.

### CX-2 — the clip window persists, as named export variants (PXF-9, PXF-4)

- **Persist it.** An exact musical boundary is authored work. The transient act
  is choosing a destination and pressing Export; "the teaser begins on this
  beat" is as much part of the project as captions, scene timing and aspect.
- **One window is the wrong shape — go straight to a bounded list (max 8) of
  named *variants*, not ranges.** "Vertical teaser" means 01:12–01:42 **and**
  9:16/1080p. If variants share one global output config, picking the square
  loop after the vertical teaser quietly renders it vertically. Each variant
  owns id, name, full-track-or-window, and width/height/fps/quality/format.
  Output *paths* and render history stay out — those describe a publication
  attempt, not the work.
- **PXF-4 is a prerequisite, not a sibling.** A persisted invisible window is a
  trap: reopen a project and silently render 20 seconds. Excluded regions dim,
  In/Out get draggable handles owned by the timeline gesture layer (D4's owner,
  not the export panel), and the active variant is named on the strip.
- **A field move alone would ship defects.** Verified against the tree: (i)
  `Shell::export_clip` (`shell.rs:542`) is app-global and survives a track
  switch, so track A's clip already reaches track B; (ii) `main.rs:983` seeds it
  unconditionally from `--render-window`, which would erase restored state, so
  the flag must override one render rather than write the saved variant; (iii)
  the model already carries `output.start_seconds`/`end_seconds`, so v2 must
  *replace* that authority rather than add a second one; (iv) filename
  suggestions needed CX-0 before variants could be told apart at all; (v)
  replacing the audio must **report** a variant that no longer fits, never
  silently clamp it.
- **Shape:** `musializer.project/v2`, `project-v1.schema.json` left immutable,
  `export_variants { next_id, active_id, items[0..8] }` with `window: null`
  meaning full track. A v1 file migrates **in memory** — full-duration range
  becomes one "Full track" variant, a genuine partial range becomes "Imported
  clip" — and only the next save writes v2.

### CX-3 — save state: escalate on accumulated risk, and back it with recovery (PXF-1)

**Status (2026-08-11): delivered and headless-gated.** A continuous mutation
run closes as one durable transaction; three transactions, five minutes, or a
significance event escalates to `Unfiled work`. Recovery writes the ordinary
project payload plus app-owned risk and dirty lyric/route drafts to at most
`current.json` and `previous.json`; the 1.5-second poll uses the audio digest
cached at explicit open and never reads or copies the audio. The welcome screen
can recover the session, verifies every referenced asset before replacing the
workspace, and returns the track unnamed with the warning `Save As to keep it`.
Named save or explicit discard retires both generations. Fullscreen reports and
draws the amber/red attention-only treatment, including the initial
`Unfiled work · Ctrl+S` expansion. Evidence is pinned in the 437 app tests and
the `Unfiled work: recovery restart round trip` section of
`tools/headless_check.sh`, which crosses three isolated application processes,
guards a competing open, and inspects the snapshot's data boundary.

- **The escalation rule.** `No file` stays calm until the **first** of: three
  durable edit *transactions* (one user gesture — a whole slider drag, an
  editor Apply, an import — not every `ShellCommand` a drag emits); **five
  minutes** of active use after the first durable edit; or immediately on a
  **significance event** (lyrics created or imported, first scene-plan segment,
  Assist applied, imagery imported, an export attempted after an edit). Then the
  label becomes **`Unfiled work`** in amber — new words, not just a new colour,
  because the state is not "no file", it is "work with nowhere to go".
- **Why the hybrid:** "any edit" turns the first curious knob turn into
  paperwork; a raw count overcounts sliders and undercounts a bulk import; time
  alone leaves a whole imported lyric document calm for five minutes;
  significance alone misses an hour assembled from small tuning decisions.
- **Go past colour, but not to a fake default project.** The product to be is
  the one that **recovers your work**, not the one that warned you. Smallest
  honest version: after the first durable edit, write an app-owned recovery
  snapshot on the existing 1.5 s settle into `$XDG_STATE_HOME/musializer/
  recovery/`, keeping drafts *as drafts*, referencing audio by path + digest
  (never re-copying or re-hashing a large file on the frame thread), two
  generations, cleared only by a named save or explicit discard. On restart:
  **Recover session** → **Save As to keep it**. Call it *recovery*, never
  "autosaved project" — it has no user-chosen home. Do **not** silently create
  permanent projects in a default folder; that trades loss for invisible files
  and a recents shelf full of experiments.
- **Fullscreen gets a 6 px attention-only dot**, top right: amber for unsaved or
  escalated unfiled work, red for failure, **nothing** when saved or when
  genuinely fresh. On first appearance it expands for ~2.5 s to
  `Unfiled work · Ctrl+S`, then collapses. Transient-on-entry is not enough — a
  user can enter fullscreen clean and then change scenes from the keyboard.
- **The reframing:** `Untitled session — recoverable` → **Keep this cut…** →
  `Saved` → `Working changes`. A first save then creates a recent-project card
  with a still-frame thumbnail, which is where CX-5 picks it up.

### CX-4 — tap feel and Surprise: derivable, mostly (PXF-2, PXF-3, PXF-11)

- **Tap offset: default `-100 ms`** (place the cue 100 ms *before* the
  keypress), **then calibrate.** The two literatures are not in conflict, they
  describe different behaviours: tapping to a *predictable* beat leads it by
  ~30–50 ms (negative mean asynchrony), while *reacting* to an unpredicted
  auditory event measures ~150 ms. A Musializer user has the lyrics in front of
  them and usually knows the song, so they sit nearer prediction — but vocal
  entries are less regular than a metronome, and buffering, key sampling and the
  occasional genuinely reactive tap all bias late. -100 ms is the least-bad
  population fallback: early enough to kill the "caption chases the singer"
  feel, not as early as pure reaction would justify.
- **The real design is the calibration gesture, not the constant.** Play a
  detected or generated beat through the normal audio path, take **eight** Enter
  presses, discard the first two, persist
  `offset = median(beat_time - keypress_time)`. Four taps are too noisy. This
  captures user, keyboard *and* output latency together and may legitimately
  come out either sign. Keep `[`/`]` as live 10 ms trim; persist per PXF-2.
  Comparable tools (Amara, Aegisub, Subtitle Edit) all assume a rough pass plus
  a correction pass rather than pretending live stamps are final.
- **Tap feedback: visual, and no click.** On an accepted tap, flash the **whole
  corrected cue block** — not the keypress position — bright fill plus 3 px
  outline for 120 ms, then a 180 ms glow decay; draw a short impact mark at the
  stamped time; and **ghost the next lyric line** near the transport, which aids
  prediction as much as it acknowledges the press. Refusals get a distinct
  150 ms red pulse at the same place — nobody reads a toast while keeping time.
  **Do not click on every stamp:** it masks the musical onset being judged and
  auditory feedback is known to alter synchronisation, so it is not neutral
  acknowledgement. Clicks belong only to the calibration/count-in mode. All of
  the visual half is headless-testable at fixed probe frames.
- **Surprise: keep two of the five constants, replace three.**

  | constant | ruling |
  | --- | --- |
  | precision snapping | **keep** — a displayed value must be the applied value |
  | Nudge's 12 % reach, triangular about current | **keep** — a sound local mutation |
  | `SURPRISE_INSET` 5 % of every range | **remove** — it excludes meaningful endpoints. Verified: `settings.pulse.petals` is `0..12` where **0 means auto**, and a 5 % inset makes 0 undrawable; on `-180..180` it carves a gratuitous 36° forbidden arc. Replace with per-descriptor endpoint-risk metadata |
  | `SURPRISE_MOVE_CHANCE` 0.75 | **0.45** — nine changed controls out of twelve is not "the scene stays recognisable"; it only looks recognisable because it is still the same renderer |
  | `SURPRISE_TOGGLE_CHANCE` 0.25 | **0.15** — with two toggles, 0.25 gives a 44 % chance that at least one whole-scene mode flips *every press*, which is not "occasionally" |

  Also: `is_angle` infers "circular" from `minimum < 0 && maximum > 0`. That is
  currently true of all five (verified: four hue controls plus
  `settings.atlas.orbit`), but it is unsafe inference for any future symmetric
  range that is not circular, and `atlas.orbit` is a camera composition control
  rather than colour. Mark cyclic descriptors explicitly. And the right
  user-facing knob is **one Adventure control** (default 50/100) that moves the
  count and the distance together, over per-descriptor sensitivity underneath —
  density, camera, glow and hue do not tolerate equal perturbation.
- **What still needs ears — and it is fifteen minutes, not an afternoon.** The
  offset design and the feedback treatment are derivable now and were derived.
  Only Surprise's *keepability* needs a human: one sparse and one energetic
  track, Song Atlas and Cadence, five current seeds and five proposed seeds per
  scene compared blind, recording only keep / interesting-but-fixable / reject.
  **Pass condition: ≥2 keeps and ≤1 reject per scene, and consecutive presses
  visibly distinct.** Four more minutes validates the tap offset (calibrate,
  stamp eight familiar lines, inspect the median residual); two more compares
  80/120/200 ms flashes.

**Status (2026-08-08): the Surprise ruling is landed; the listening session is
a protocol file.** `SURPRISE_MOVE_CHANCE` 0.75 → 0.45, `SURPRISE_TOGGLE_CHANCE`
0.25 → 0.15, the blanket 5 % inset replaced by per-end metadata
(`DRAWABLE_LOW_KEYS`: `pulse.petals`, `phosphor.field`, `phosphor.ramp` — all
"0 = auto", now drawable, with a test proving each low end is actually drawn),
and cyclic controls are the explicit `CYCLIC_KEYS` list (the four C-era hues
plus `phosphor.hue`) instead of `is_angle`'s bounds inference — so
`atlas.orbit` now draws triangular about its default like the camera control it
is, pinned by a measured orbit-vs-colour spread test. The gate's seed-4242 pin
was re-pinned **deliberately**; the old pin fails against the new draw, which
is the negative control, and the comment at the pin says so. The keepability
session itself ships as `build/protocols/cx4-surprise-*.protocol.json` (HX),
with the current-vs-revised mapping in a `.key.json` the operator should not
open until after answering. **Not built:** the single Adventure knob over
per-descriptor sensitivity (a UI design of its own), the -100 ms tap offset,
calibration gesture and tap flash — those are PXF-2/PXF-3 work this wave did
not claim.

### CX-5 — thumbnails are the unlock, but only the track-specific kind (PXF-6)

- **Yes, with a condition.** Ten 24–52 px *text* tiles make choosing a visual
  language a ten-step select/watch/backtrack loop; pictures make it perceptual
  comparison. But the case against is real — tiny thumbnails can become coloured
  noise, a silent or transitional playhead misrepresents a scene, stateful
  scenes make correct generation much harder than screenshotting, and a **stale
  preview is worse than text because it confidently lies**. So:
  **track-specific thumbnails are an unlock; canned pictures are decoration.**
- **Priority, explicitly: trust outranks delight.** Validate tap feel (cheap
  uncertainty reduction) → PXF-1/CX-3 (closed) → thumbnails → tap flash, markers,
  clip persistence.
- **Kind: a deterministic still of the current track at the current playhead**,
  through the settings, routes, lyrics, semantic data, imagery, seed and aspect
  that clicking that tile would actually produce, at ~256x144 with the name kept
  overlaid. Rejected: looping animations (ten competing loops are noise), canned
  stills (conceal reactivity, tuning, lyrics, imported imagery), live miniatures
  (ten scene updates per display frame, forever, competing with the real
  preview).
- **The non-obvious cost:** several scenes accumulate deterministic history, so
  an honest preview is *not* ten final draws. Prepare audio and analyzer state
  once, replay ten candidate `SceneInstance`s through that shared frame
  sequence, then draw each final state once.
- **Generation policy.** Trigger on track open, first reveal of the browser,
  pause, a seek stationary ~250 ms, or a relevant project edit — **never chase
  the playhead during playback**; keep the last coherent set. Cancel obsolete
  work by generation number. Key on track fingerprint + project revision + frame
  index + scene id + seed + effective settings/routes/cue tuning + lane/asset
  revision + aspect + renderer-cache version. Draw one or two tiles per app
  frame under a time budget, hold results privately until **all ten** succeed,
  then swap one atlas atomically. **No per-tile pop-in** — show ten uniform
  placeholders and one panel-level `Preparing previews at 01:23`. Persist only
  the latest complete atlas per track in a bounded XDG cache, never in `.musi`.
- **Recent projects: use the playhead at last successful save**, which is both a
  visual memory and where the user expects to resume — so **PXF-7 comes first**.
  Loudest-moment picks spectacle over recognition; a fixed timestamp ignores
  structure. Update the poster only after a successful save, so the shelf shows
  durable work.
- **Build a poster-frame *service*, not a thumbnail widget.** A deterministic
  frame request (project revision, track, frame index, optional forced scene,
  aspect, size, provenance) also yields chosen project posters, shareable
  stills, cover-art aspect variants, a per-cue contact sheet or storyboard, and
  preset comparison. Keep the recipe and provenance; the cached bitmap is not
  the product. Closing PXF-5 (the still blocking the frame loop) falls out of
  the same cancellable prepared-frame batch.

## HX — human-feedback protocols (operator proposal, 2026-08-08)

The operator, reading CX-4's listening-session recipe: *"what if you can place
concrete markers with concrete questions at concrete points for a concrete audio
track?"* — and, on top, wire `claude -p` into the assist features to help the
feedback loop.

The observation this answers is structural. Today the loop is: an agent writes a
prose protocol into this file, the operator reads it, holds it in their head,
runs the app, and reports back in chat. Three lossy hops — and one thing prose
can **never** do: blind the operator. CX-4's Surprise comparison wants five
current seeds against five proposed seeds judged blind, and a plan section that
names which is which has already unblinded it. The application is the only party
that can apply variant A or B without saying which. That makes this a
correctness feature for feedback, not a convenience.

### The design

**HX-1 — the protocol file.** A sidecar, deliberately **not** `.musi`: an
agent's questions are session artifacts, not authored work, and a project file
accumulating a bot's questionnaires is CX-3's "recents shelf full of
experiments" problem in a new coat. `*.protocol.json`, schema-versioned,
referring to its audio by path **and digest** (the ASCII-import pattern) so the
wrong track is refused rather than mis-asked. Items carry: `id`, `at_seconds`,
an audition window (pre/post seconds), the question text, an answer `kind`
(`choice` / `scale` / `text`), options, and an optional `apply` block — scene,
seed, and up to two `SettingsSnapshot`s (the PX6 type, 12 f32s) labelled only
`a`/`b`. The `apply` block is what makes a blind A/B a *file* an agent can
write.

**HX-2 — the in-app runner.** Loaded via `--protocol PATH` (additive flag; the
CLI grammar is a contract) and via drop — `classify_drop`
(`crates/musializer-app/src/ui/shell.rs:307`) gains a `.protocol.json` arm,
which is additive over D1 since the extension can never be audio or image.
Markers draw on the timeline as their own rail — this is CX-1's review-queue
interaction wearing a different hat: **Next** jumps to the marker's window,
auditions from `pre` seconds before it, the question card draws through the
notice machinery (`crates/musializer-core/src/ui/notice.rs`), and the operator
answers with `1`–`4`/typed text without leaving playback. A `choice` item with
an `apply` block gets an A/B toggle that re-auditions the window under the
other snapshot — the CX-4 session becomes: press play, listen, press a number,
next. Progress reads `3 of 10 answered`; quitting mid-run loses nothing
(answers are already on disk, HX-3).

**HX-3 — the answers file.** Append-only JSONL beside the protocol
(`*.answers.jsonl`): one line per answer with `item_id`, the answer, the
variant order the app *actually* played (recorded at answer time, so the
unblinding survives for the agent while never reaching the screen), wall-clock,
playhead, and how many times the window was re-auditioned — that last number is
itself feedback (a question auditioned five times was a hard question).
Append-only rather than atomic-replace because a crash mid-session must lose at
most one line, and an agent reading it needs no lock.

**HX-4 — the loop closes without any new AI plumbing.** The agent that wrote
the protocol reads the JSONL and edits the plan/constants. That is the whole
MVP: `claude -p` is *already* the thing reading this repository. The in-app
wiring is a second step, and it has a precedented seam — the assist stack
already discovers and supervises `codex` child processes
(`crates/musializer-runtime/src/assist/discover.rs`, `process_group`), so a
`claude -p` runner is the same shape. Two uses earn it: **generate** (a button
that turns a plan section like CX-4 into a protocol file) and **digest** (turn
a finished answers file into a proposed plan edit the operator reviews).
`claude` carries its own auth under `~/.claude`, so nothing touches the E1
credential contract — but it inherits the child-env hygiene that exists for
`codex` anyway. Rate/cost honesty: a run is operator-initiated, never ambient.

**HX-5 — the evidence.** The invented-probe pattern applies unchanged: a
`--ui-probe protocol-answer=ID:CHOICE` key and a `protocol:` report line
(loaded file, item count, answered count, current item, which variant is live),
because Xvfb can neither hear the track nor press `2`. The gate asserts a
seeded protocol round-trips: load, answer two items headlessly, read the JSONL
back, confirm the variant order recorded matches what the report line claimed
was played.

### Why this is worth a wave

- CX-4's nine-minute session stops being prose and becomes a file this session
  can emit tonight; blinding becomes real; and every future "operator judgment"
  item (PXF-8's density calibration, CX-1's acceptance test, thumbnail
  legibility) is a protocol rather than a paragraph.
- It is almost entirely assembled from existing parts: PX6's
  `SettingsSnapshot`, CX-1's review-queue interaction, D1's drop dispatch, the
  notice card, the digest check, JSONL sidecars, the probe pattern.
- The same runner is a **user** feature later ("A or B?" is how non-developers
  tune anything), but nothing in HX-1..5 commits to that.

Order: HX-1+HX-3 (pure core: schema, parse, refuse, append) → HX-2 (runner +
rail) → HX-5 (gate) → HX-4's generate/digest buttons last, since the MVP loop
works without them.

### Status (2026-08-08, HX wave)

- [x] **HX-1 + HX-3** — `core::feedback`: `musializer.protocol/v1` parse/emit
      and the `musializer.protocol-answer/v1` JSONL line. Strict in the `.musi`
      codec's sense; every refusal in the design has a named test (wrong
      digest, junk JSON, unknown kind/scene, snapshot invalid for its own
      item's scene, a third snapshot label, out-of-charset and duplicate ids).
      The answers reader tolerates exactly one torn line, only at the end.
- [x] **HX-2** — `--protocol PATH` (additive), the `classify_drop`
      `.protocol.json` arm (matched on the *double* extension so a bare
      `.json` keeps the oracle's audio branch), bottom-anchored pennants on
      the waveform strip, the keyboard-first question card, keys `1`-`4` /
      `R` / `B` / `N` that exist only while a session runs, the self-pausing
      audition window, and append-then-mark answer flow. Two deviations from
      the sketch, both deliberate: the card is its own drawing (the notice
      machinery has no options affordance — the tray now stacks *above* the
      card instead), and markers overlay the strip's bottom edge rather than
      taking a new band, so no `workspace_layout` guarantee moves. `text`
      items parse and round-trip but the runner cannot take typed answers yet
      — the card says so by name and `N` skips (missing beats pretending).
- [x] **HX-5** — `--ui-probe protocol-flip=ID` / `protocol-answer=ID:CHOICE
      [+ID:CHOICE]` (by id, never pixels — GX-1), the `protocol:` report line,
      and the gate section: flip + answer through the probe, read the JSONL
      back, assert the recorded variant order equals the claimed one, prove a
      relaunch resumes from disk, and the negative control — a wrong digest
      refuses with a nonzero exit and no answers file.
- [ ] **HX-4** — the `claude -p` generate/digest buttons. The MVP loop works
      without them; the agent writing the protocol reads the JSONL directly.
- [x] **HX-W — browser listening lab (2026-08-15).**
      `tools/listening-lab/` is the audio-candidate counterpart to the in-app
      runner: agent-authored local paths, server-side blind labels, WaveSurfer
      PCM/timeline/region views, millisecond seek and ±10/100 ms steps,
      same-playhead A/B switching, and agent-composable single/multi/scale/
      timestamp fields with conditional reveal, required-field completion and a
      collapsed prose escape hatch, all appended under
      `build/listening-lab/answers/`. External-companion mode lets the Rust
      runner retain audio, scene state and blind snapshot order while this lab
      records the structured judgment; generated feedback sheets for both
      current CX-4 sessions ask directly for keepability, music fit, repair
      targets and pairwise visual distinctness. Its headless Playwright gate
      starts Chromium with `--mute-audio` and covers transport precision, A/B
      continuity, structured partial/completed saves, conditional controls,
      external handoff, answer/reload recovery, hidden source identity, and
      byte-range serving. It deliberately does **not** claim HX-4: there is no
      in-app remote-model generation/digest button. The lab's README also
      recovers the earlier 23-item lyric-timing adjudication for future authors.

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
four blind spots) are fully closed. The six-agent PX wave then closed C1, C4,
D1, D2, D3, D4's remainder, UX0-B01–B05 and UX0-C01/C03/C04/C06/C07/C10 in one
day (merges `03548b9`..`e63fee2`; debrief in the PX section above). What is
open, in the order a session should pick it up:

| # | Work | Where | Size |
| --- | --- | --- | --- |
| 0 | **The fifteen-minute listening session** — the only thing in the whole PXF debrief that a human genuinely has to do, now that CX has scoped it: validate the tap offset (4 min), compare 80/120/200 ms flashes (2 min), and blind-compare five current against five proposed Surprise seeds on two tracks (9 min), against CX-4's stated pass condition | CX-4 | 15 minutes, operator |
| 1 | **MiMo v2.5 capability benchmark** — operator's stated priority. Design and harness in `docs/MIMO_BENCHMARK_PLAN.md` + `tools/mimo_bench/`. Needs an explicit operator go-ahead: it sends audio to OpenRouter and spends credits | new | one session |
| 2 | **The remaining CX rulings, in their stated order:** CX-3 save trust is closed; next is CX-5 thumbnails behind a poster-frame service (PXF-7 first), then CX-1 marker rails and the proposal review queue, then CX-2 export variants (PXF-4 first, schema v2). CX-4's revised Surprise constants are already landed; its listening protocol remains operator work. Trust outranks delight — that ordering is a ruling, not a preference | CX + PX sections | large; four or five agents |
| 2b | The remaining UX0-B (B06, B10–B19) and UX0-C (C08, C09, C17); PXF-3, PXF-5, PXF-10, PXF-12 | UX0 sections | medium |
| 3 | C2/C3/C5 durable-edit remainder, D5–D8, then E2–E4 (prove the copied bundle runs), then F/G honesty and gates; DX1–DX9 dev-ex items, DX8 first | this document | large |
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

- [x] **UX0-B01 — continuous save state [C1/D8]:** show durable dirty state on
      the track and Save affordance; distinguish saved, unsaved and save-failed
      state before quit is attempted (`review` 2, Saving and trust).
      Done by PX1, 2026-08-07, and extended by CX-3 on 2026-08-11.
      `Track::save_state()` returns one of five `SaveState`s and the TRACKS
      header draws its word, right-aligned,
      colour-coded — muted for Saved and No file, warning for Unsaved, danger for
      Save failed; accumulated unnamed work becomes amber `Unfiled work`. A
      failure also draws its reason under the header and raises a
      persistent notice naming the track and the recovery. Both Save buttons (the
      panel's and the collapsed strip's) read `Save *` while there is work to
      write, and the strip's tooltip names the state in words. Rows other than
      the current one carry a state dot, because all-track autosave is a claim
      about tracks the user is not looking at. Reported as `save state:` and
      gated in `tools/headless_check.sh:4175-4338`, including an ink check that
      the word reaches the header rather than only the report.
- [x] **UX0-B02 — lyric seek/stamp loop:** make cue selection seek-capable, add
      playhead stamping for start/end, auto-advance after Apply and preserve a
      usable play/tap loop while lyric text has focus (2, lyric timing loop).
      *Done (PX2). A row click and Up/Down call `select_and_seek`, which parks
      a `seek_request` the panel wrapper drains into one `ShellCommand::Seek`
      per frame — drained in `lyrics_panel` rather than `draw_lyrics` because
      the latter has five early returns and a key-requested seek was stranded
      by four of them. Stamping is `LyricTap` (see C03). The focus half is
      resolved by making tap mode and text entry **mutually exclusive states**
      rather than two things fighting over the keyboard: arming blurs the
      field, so no key means two things. Evidence:
      `selecting_a_cue_asks_the_transport_to_go_there`,
      `arming_a_tap_run_blurs_the_cue_field_so_no_key_means_two_things`, and
      the gate's `timing-tap` capture.*
- [x] **UX0-B03 — lyric edit recovery:** add undo/inverse edits and safe deletion
      for single and multi-cue timing changes, including accidental lane drags.
      *Done (PX2). `core::project::lyrics::LyricHistory`, a 64-deep snapshot
      stack, recorded once per drained batch in `main.rs` — so a 64-cue
      `ShiftMany` from an accidental lane drag, or a five-cue Ctrl+X, is one
      Ctrl+Z. **Snapshots rather than inverse edits, deliberately**: `split`
      allocates an id, `merge` destroys one, and `update`/`retime` overwrite
      the `CueOrigin` that `at_time` reads, so an inverse log that misses any
      one of those yields a valid document quietly different from the user's.
      Delete is now bound to the Delete key as well as the button, which is
      only defensible because Ctrl+Z exists. Evidence: seven history tests
      including a per-state round trip forwards and back over one of every
      edit kind; two negative controls, both reverted byte-for-byte —
      dropping `next_id` from `replace()` failed both round trips with
      `next_id differs`, flattening snapshot origins failed both with `cues
      differ`. Gate: `timing-undo` reports `history 0/1` with cue 4 back at
      its seeded `00:01.000`.*
- [x] **UX0-B04 — precise lyric timing controls:** support an established
      fine/normal/coarse nudge ladder, hold-repeat or equivalent efficient input,
      typed times and one consistent minimum cue gap in form and lane.
      *Done (PX2). `cue_nudge_step_seconds` mirrors the transport's ladder in
      **shape** — Ctrl fine, Shift coarse, fine wins when both are held — at a
      tenth of its scale (0.01/0.1/1.0), because a cue boundary is placed
      against a syllable and a playhead against a section; sharing the shape
      and not the numbers is what stops a user learning Ctrl twice. The button
      labels state the step the modifiers currently mean rather than a fixed
      "-0.1" that lies whenever Ctrl is down. Hold-repeat is `HoldRepeat`,
      counted in **frames** so a headless probe can reproduce it. Typed times
      go through `parse_cue_timestamp`, which refuses `-1:30`, `1e3`, `1:60`
      and `+30` rather than guessing. The minimum-gap half was already closed
      by review 1.1 (`clamp_form_start`/`clamp_form_end` both use
      `LYRIC_MIN_CUE_SECONDS`); there is now a round-trip test pinning the
      parser against `widgets::format_timestamp`, which is the only thing
      making the cross-crate split safe.*
- [x] **UX0-B05 — expose lyric split/merge:** connect the already-ported model
      operations to the editor with playhead-aware behavior and tests.
      *Done (PX2). `LyricsEdit::Split`/`Merge`, reached by Ctrl+B and Ctrl+J
      and by two buttons. Split refuses a playhead outside the cue (or within
      20 ms of either end) rather than producing a sliver, and divides the
      text at the space nearest the seam's position through the line —
      `split_text_at_fraction`, which never returns an empty half because
      `validate_text` refuses empty text and a refusal after the user has
      committed is worse than a rough guess.*
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

- [x] **UX0-C01 — clip export:** expose render in/out or start/duration controls
      and drive the existing windowed `RenderPlan` path (`review` 3.1).
      *Done (PX3, `40a8f27`). A CLIP row above SIZE — `Full track`,
      `In <- playhead`, `Out <- playhead` — over `ClipSelection` in
      `core::timing::render_export`, plus an `export clip:` report line. See the
      PX3 section below for the evidence, the divergence and the schema question
      the integration owner has to answer.*
- [x] **UX0-C02 — vertical and square output:** add explicit width/height output
      formats without changing the C-ordered persisted resolution enum, then
      capture-audit every scene at tall and square aspect ratios (3.2).
      *Done 2026-08-07 as EX2 (`9db6684`), see its section: ASPECT row
      (16:9/9:16/1:1/4:5, rung names the short edge, 16:9 byte-identical),
      thirty renders — ten scenes at three aspects — audited frame by frame,
      caption and ASCII Field fixed, preview framed to the export aspect.*
- [x] **UX0-C03 — lyric tap timing:** deliver the play-and-tap stamping workflow,
      row seek and advancement described by B02 as a polished primary flow (3.3).
      *Done (PX2). `core::ui::lyric_lane_edit::LyricTap`: arm a run from the
      playhead, then one press per line. Each press **closes the line before it
      and opens the next at the playhead**, which is review 1.14's rule applied
      as a gesture rather than as a default, and is what makes a run of taps
      produce contiguous captions instead of cues with whatever durations they
      arrived with. The tap key is **Enter**, chosen by elimination: `T`, `M`,
      `S`, `F`, `H`, Space, Tab and the arrows are all shell globals fired
      unconditionally once no field has focus, and the shell reads the keyboard
      before any panel draws — a panel key shadowing one would fire both.
      Ctrl+Enter arms.
      **The one non-obvious invariant:** the cue id order is snapshotted at
      arming time and the cursor indexes that, because a stamp *moves* a cue and
      the document sorts by start — stamping line 1 at 0:50 while lines 2-6 sit
      at their imported 1-3 s re-sorts it behind them, and a cursor walking
      canonical order hands out line 2 twice and never reaches line 1. Pinned by
      `a_run_survives_the_re_sort_its_own_stamps_cause`.
      A tap offset (`[` / `]`, ±250 ms, session-only) compensates a player who
      lands consistently early or late; it defaults to 0 because a silent
      −200 ms would be the tool lying about the data it recorded.
      Evidence: ten `LyricTap` tests including a double-keypress refusal, a cue
      deleted mid-run, and a sweep asserting every stamp is one the model would
      accept; `--ui-probe lyric-tap=N`; gate captures `timing-idle`/`timing-tap`.
      **Not done, and named rather than implied:** tapping never invents text —
      it places the times of lines that already exist, and says so when the
      document is empty.*
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
- [x] **UX0-C06 — recent projects:** use the welcome screen's spare region for a
      durable, failure-tolerant recent-file list and direct reopen flow (3.6).
- [x] **UX0-C07 — Tune randomize/mutate:** add bounded Surprise/Nudge operations
      Done (PX4). The column lives right of the welcome screen's 72 % rule — the
      only region the C leaves empty — and draws name, relative age and parent
      folder per row, with the full path as a tooltip.

      **A second per-user file, `recent.json`, not a field in `ui.json`,** and
      that is the failure-tolerance requirement rather than tidiness: both stores
      are refused wholesale when they do not parse, so folding them together
      would make one truncated write cost the operator their splits *and* their
      history. The write cadences are also unrelated, and `UiPreferences` is
      `Copy` and travels by value inside `ShellCommand::SaveUiPreferences`.
      Same schema-string/size-bound/atomic-replace policy as its neighbour.

      **Three states, not two.** No history, an *unreadable* history, and a
      history whose files have moved are different facts; a blank column would be
      indistinguishable from a broken one. A missing entry draws amber, says
      "File is missing", refuses to open, and keeps its forget cross — offering
      removal rather than erroring on every click forever.

      **`--project` on the command line records an entry only in a session run**
      (`is_session_run`: no `--probe-frames`, `--render` or `--save-project`).
      Without that guard `tools/verify.sh` would append a dozen scratch fixtures
      to the operator's real `~/.config/musializer/recent.json` as a side effect
      of being run. Interactive recording is deliberately *not* guarded, because
      that is what the gate has to be able to prove.

      Evidence: 11 model tests in `ui::preferences::recent::tests` (ordering,
      move-to-front, cap, missing-probe, round trip, five corrupt-file cases each
      asserted byte-identical afterwards, over-capacity, refused writes, and the
      age wording); 2 layout tests pinning the seats (7 rows at 960x640, 8 at
      720/1080, no overlap with the step number or the format strip); gate block
      `tools/headless_check.sh:4175-4551` with captures of the empty, populated,
      corrupt and missing states plus five `--ui-probe click=` assertions and a
      gutter negative control.

      **Two defects found by the evidence rather than by review**, both of the
      class this repository keeps paying for:

      - The row's open target was the full row width and is drawn first, so it
        claimed every press aimed at the forget cross and *opened the project*
        instead of forgetting it — EX1's defect one namespace later. Hover
        highlighting was correct and the cross was drawn in the right place; only
        an injected press could see it. Fixed by making the two rects disjoint,
        and pinned by the `claimed=0xc00000002` assertion.
      - `draw_welcome` never painted the queued tooltip. `Widgets::hint` only
        queues one and `Shell::draw` drains it, but the welcome screen had no
        hinted control until this list, so every tip it requested was queued and
        dropped — with `hint` called correctly and the request well-formed. A
        row's full path is written down nowhere else. Found by asking where the
        paint happens, not by a capture, and now pinned by a YMIN crop with the
        unhovered frame as its negative control (20 against 113).
- [ ] **UX0-C07 — Tune randomize/mutate:** add bounded Surprise/Nudge operations
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
- [x] **UX0-C10 — still-frame export:** publish a user-selected supersampled
      frame through the offline render path and reuse it where appropriate for
      thumbnails/cover output (3.10).
      *Done (PX3, `40a8f27` + `6c84387`). `Save still` in the export footer
      writes the frame at the playhead as a PNG through the same
      `with_export_frame`/`draw_offline_frame`/`LinearResolver` path an encoded
      frame takes, with an `export still:` report line naming the frame index.
      Reuse for scene thumbnails (UX0-C05) and cover output (UX0-C09) is **not**
      done — see the PX3 section.*

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

- [x] Audit every command that mutates data serialized into `.musi` and route it
      through one dirty-marking policy.
- [x] At minimum, mark dirty after setting changes, scene reset, base-scene
      selection, cue-snapshot tuning, caption-style edits, lyric import, ASCII
      clear/import and scene-plan enable/disable.
- [x] Keep purely transient playback, selection, hover and panel state clean.
- [x] Test that each durable mutation starts/restarts the 1.5-second autosave
      settle and participates in the quit guard.

Done by PX1, 2026-08-07. The audit was run against `build_project`'s own field
list (`project.rs:120-272`) rather than against the command enum, because that is
what actually defines "serialized into `.musi`". Result: **the marking was
already complete except for two defects**, both of which were assignments that
looked like `mark_dirty` and were not.

| Mutation path | Before | Now |
| --- | --- | --- |
| `SetSetting`, `ResetScene`, `SetRenderConfig`, `ApplyRoute`, `RemoveRoute`, `SelectScene`, `SetAutoScenes`, `ManualEvent`, `ScenePlan`, `Preset::Apply`, Assist apply, caption style | `mark_dirty` | unchanged |
| ASCII image import (`main.rs:2270`) | `project_dirty = true` | `mark_dirty(now)` |
| lyric-edit drain (`main.rs:1342`) | marked dirty even when every edit was refused | marks only when at least one applied |
| `TogglePlay`, `Seek`, `SelectTrack`, `SetVolume`, `ToggleMute`, `SetFullscreen`, `SaveUiPreferences`, `ManualEvent::ArmClear`, `Preset::Select`/`SaveNew`/`Delete` | clean | unchanged (correct — transient, or the shared preset store rather than `.musi`) |

The ASCII one is the instructive defect. `project_dirty = true` never moved
`project_dirty_since`, so the settle was measured from a stale instant — usually
`0.0`, making the write due on the *next frame* rather than after the import
settled — and never cleared `project_autosave_failed`, so an import after any
failed save was never autosaved at all, silently. That is the whole reason
`mark_dirty` is a method: the three fields are one invariant.

Evidence: `workspace.rs` tests `marking_dirty_clears_the_autosave_failure`,
`the_next_edit_clears_the_failure_and_its_now_stale_reason` (settle restart
pinned at `project_dirty_since == 9.0`), and `project.rs`
`the_settle_window_holds_a_track_back_until_it_has_elapsed` (11.4 s not due,
11.5 s due). End to end: `tools/headless_check.sh` `save-state-background`
observes a `scene-pick` edit reaching disk as a changed `.musi` digest.

### C2 — lyric and route draft context guards

- [ ] Expose one truthful `lyric_editor_has_unsaved_draft` query.
- [ ] Block or resolve dirty lyric/route drafts before track change, scene change,
      panel change, project open, save, export start, Assist apply and quit where
      the C workflow protects them.
- [ ] Include the active lyric draft in `confirm_close` and in autosave's
      `editor_dirty` argument.
      **Autosave half done by PX1, 2026-08-07** (as part of C4):
      `Shell::editor_draft_blocks_autosave` answers per track and is what
      `project::autosave_due_tracks` consults. Until then `main.rs` passed a
      hard-coded `false` for `editor_dirty`, so the parameter had never once
      suppressed a save. `confirm_close` already consults `lyric_draft_is_dirty`;
      the rest of C2 (the blocking/resolution workflow) is untouched.
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

- [x] Cache decoded sample rate and channel count on each `Track` so project
      serialization does not depend on the currently bound music stream.
- [x] Autosave every due track, not only the current one.
- [x] Refuse autosave while that track owns a dirty editor draft.
- [x] Test two dirty tracks, a background track loaded from a project, failure of
      one save without suppressing another, and recovery after a failed save.

Done by PX1, 2026-08-07.

**This is parity restoration, not a new feature and not a divergence.** The
frozen C loops the whole workspace — `for (i = 0; i < p->tracks.count; ++i)
poll_project_autosave(&p->tracks.items[i]);` (`plug.c:7581-7583`) — and
`poll_project_autosave` (`plug.c:5065-5075`) takes a `Track *`, not the current
one. The port had regressed against the oracle here because it read the sample
rate off the bound stream, which only the current track has. Worth stating
plainly: this was a place where the C was right and the rewrite was wrong.

One deliberate refinement of the C's guard. The oracle checks
`(track == current_track() && lyric_editor_has_unsaved_draft(track))`, so a lyric
draft only ever suppresses the current track; here the question is asked of the
draft's own owner slot (`Shell::editor_draft_blocks_autosave`). The effect is the
same — the editor binds to one track at a time — but it does not depend on that
track also being current, which is the assumption review 1.3 already found to be
unsafe elsewhere in this file.

`Track::audio_sample_rate`/`audio_channels` are filled by
`load_timeline_waveform` (the whole-file decode every track already pays for at
load) and again by `bind_audio` from the opened stream, and by `open_path` from
the project's own `AudioAsset`. `project::save_to_path` no longer takes them as
parameters and `save_project_to` no longer takes a `Music` at all — it takes a
workspace **slot**. A track whose format is still `0` is refused with a named
error rather than written as a `.musi` claiming 0 Hz.

The selection moved out of the frame loop into `project::autosave_due_tracks`,
so the all-track claim is a unit test rather than only a capture. The per-track
draft guard is `Shell::editor_draft_blocks_autosave`; `autosave_is_due`'s
`editor_dirty` parameter had existed since it was written and `main.rs` passed a
hard-coded `false` into it, so no draft had ever suppressed anything.

Autosave failures are no longer discarded (`let _ =`). They raise a persistent
`Severity::Error` notice naming the track, the reason and the recovery, and they
latch the reason onto the track for the Tracks panel to draw.

Evidence: six tests in `project.rs` — two dirty tracks both due, the settle
boundary at 11.4/11.5 s, a draft holding back *only* its own track, no project
path never due, one track's latched failure not suppressing its sibling and
clearing on the next edit, and the 0 Hz refusal.

**End to end, with a recorded negative control.**
`tools/headless_check.sh`'s `save-state-background` row edits a project with
`--ui-probe scene-pick=cadence`, then uses `--probe-reopen` to make a *different*
track current before the 1.5 s settle expires, and requires the background
project's digest to change. Restoring the `continue` this task deleted makes that
row report `unsaved, *no-file` with the `.musi` byte-identical, at 60, 70, 80 and
100 frames; removing it gives `saved, *no-file` with the digest changed at all
four. The frame count matters and is the reason the control was needed — at 150
frames the write lands while the track is still current and the row passes either
way, which is how the first version of this case proved nothing.

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

- [x] Dispatch dropped `.musi` files through project open.
- [x] Dispatch PNG/JPEG/BMP through ASCII import and select ASCII Field.
- [x] Continue dispatching supported audio formats through track load.
- [x] Preserve the C behavior that an image dropped before audio is staged for the
      next track.
- [x] Report unsupported/corrupt input by its attempted type.
- [x] Add non-interactive probes or direct command tests for all three branches.

Done (PX4, `e53de93`). `ui::shell::classify_drop` is the one decision point and
`drop_command` turns it into one of three commands; `dropped_files` runs every
path — dropped or probed — through both.

**The else branch is audio, which is the oracle's (`plug.c:7559`), not a
whitelist.** A `.txt` is *attempted* as audio and refused by the decoder with a
notice that names audio, which is what "reported by its attempted type" asks for.
A fourth "unsupported" arm would mean this application, rather than raylib,
deciding what raylib can open — and it would start silently refusing files that
work.

**Divergence from the oracle: the match is case-insensitive.** `IsFileExtension`
is not, so in the C a `.PNG` off a camera or a Windows share goes to the audio
decoder and reports as a corrupt song. That is a defect rather than a contract,
and nothing in a `.musi`, an MP4 or a documented command line can observe the
difference.

Invented for the checks: **`--ui-probe drop=PATH`**, one synthesized drop on the
first frame that draws, through the same classifier the device path uses. Xvfb has
no drag-and-drop, so the whole of D1 was a three-arm branch nothing in this
repository could enter — the exact shape of EX1's SIZE row. Paired with a
`drop probe:` report line that records what the shell *dispatched*, not what a
reporter recomputed, so it cannot read green while the branch is dead.

The `ascii:` report line now names a **staged** grid instead of printing "no
track", because "no track" cannot be told from the drop having been discarded —
the same defect class as a lane that never reaches a frame.

Evidence: 3 unit tests in `ui::shell::tests` (a 16-row dispatch table asserted as
*commands* rather than as the enum, a pairwise-distinctness negative control, and
a check that the picker's filter and the classifier agree on exactly four
formats); gate block `tools/headless_check.sh:4175-4551`, five drop captures
asserting both the branch chosen and what that branch then did — project opens
and lands in the recent list, image with no track stages 54x54, image with a
track becomes the grid *and* selects ASCII Field, audio loads, `.txt` is
attempted as audio and refused with 0 tracks open.

### D2 — ASCII image import and clear UI

- [x] Add "Import image -> ASCII" to the scene browser with PNG/JPEG/BMP filters.
- [x] Show "Clear image" only when the active track owns an image-backed grid.
- [x] Import transactionally, select ASCII Field on success and mark the project
      dirty; a failed decode must preserve the previous grid.
- [x] Clear path, digest, cells and dimensions together and mark dirty.
- [x] Capture empty, populated, cleared and staged-with-no-track states.

Done (PX4, `e53de93`). `Shell::ascii_image_footer` draws below the scene tiles;
`dialogs::filters::ASCII_IMAGE` is the picker filter and a unit test pins it to
the same four extensions `classify_drop` imports, so an image cannot import from
the button and fail from a drop.

**The footer reserves both button heights whether or not Clear is drawn.** Sizing
the tile grid from the reservation means importing an image cannot make ten scene
tiles jump a row and clearing it cannot make them jump back — a footer that
reserved only what it drew would resize its neighbour as a side effect of an
unrelated action.

Transactionality is inherited rather than re-implemented: `import_ascii_image`
already canonicalizes, hashes, then decodes, so a failed decode never moves
state. Scene selection is on the success path only — the oracle's `&&`
(`plug.c:7552`) — because switching to an empty ASCII Field is a bad reward for a
typo. Clearing drops one `Option`, so path, digest, cells and dimensions cannot
part ways; "together" is structural rather than a discipline four assignments
have to keep.

Evidence: four captures in the gate block, and each state proved by a press
rather than by a picture. Empty asserts that Clear's *seat* claims nothing — a
capture cannot distinguish "absent" from "drawn in the background colour".
Cleared asserts `claimed=0xd00000002` and `ascii: none (procedural mode)`. Import
asserts `claimed=0xd00000001` under a `PATH` carrying neither `kdialog` nor
`zenity`: Xvfb *is* a reachable display, so an unguarded picker would draw a modal
on the capture display and block forever — a hang, not a test.

### D3 — timed-lyrics TSV import/export

**Done (PX2).** `ShellCommand::ExportLyrics`/`ImportLyrics` carry both out of the
drawing pair the way `OpenProject` already does, because a modal picker blocks
until answered.

- [x] Replace the disabled Lyrics Export/Import buttons with native save/open
      dialog commands.
- [x] Use `LyricsDocument::bridge_export` and `bridge_import`; do not invent a
      second codec.
- [x] Import transactionally, validate against track duration, mark dirty and
      preserve the old document on failure.
      *`core::project::lyrics::import_bridge_document` owns the whole
      transaction and is pure, which is the point: the interesting half of an
      import is what it refuses, and every refusal used to be buried behind a
      dialog no test can open. Two layers — `bridge_import` stages into a fresh
      document so malformed bytes touch nothing, then `normalize_duration`
      re-bases the file's **own** duration onto this track's. Adopting the
      file's duration is the defect that guards against: it would silently
      re-length the destination and put every cue past the real end out of
      reach of the timeline. A cue crossing the tail clamps; a cue beginning
      past the end refuses the whole import, because clamping that produces a
      zero-length cue rather than a shorter one.*
- [x] Export must not dirty the project.
      *Writing a copy of what is already in the `.musi` changes nothing about
      the `.musi`, and a Save prompt after an export would teach the user that
      exporting costs them something. Export is also **offered only for a
      document the codec can write** — `bridge_export` validates first and
      refuses a zero duration, so a cue-less track gets a disabled button
      rather than a dialog ending in an error. The C offers it
      unconditionally.*
- [x] Test cancel, overwrite confirmation/backend failure, invalid TSV, valid
      round trip and UTF-8 text.
      *Five unit tests in `bridge_import_tests`: a valid round trip that is
      byte-identical the second time and carries Greek and Cyrillic through
      base64; **eight** malformed files each refused as a format error (empty,
      plain text, truncated header, wrong version, no trailing newline, empty
      text field, zero cue id, non-base64 text, embedded NUL); a file from
      another track re-based / clamped / refused; a destination with no length;
      and the empty document, which reads as a failure and is not.
      In the gate: `lyrics-click-export` and `lyrics-click-import` press both
      buttons with an **empty PATH**, which is what makes it both safe and
      useful — `choose_backend` then finds neither kdialog nor zenity and
      refuses before spawning anything, so the probe cannot hang on a modal
      Xvfb has no way to answer, and the refusal **is** the backend-failure
      branch. Both presses are asserted to claim distinct widget ids, a click
      into the 7 px gap between them is asserted to claim nothing, and the
      "No file picker is available" card is measured on screen.
      **Not covered, and named rather than implied:** cancel and overwrite
      confirmation are the dialog layer's own behaviour (`kdialog --getsavefilename`
      returns empty on cancel and confirms overwrite itself); the gate cannot
      reach them without opening a modal it cannot answer, and `dialogs.rs`
      keeps its own tests for the backend choice.*

### D4 — timeline content and navigation

- [x] Draw the merged manual/semantic event markers over the waveform lane.
      Colour by event type (the C's own axis), lane by head shape — filled disc
      for manual, ring for semantic — with a tooltip naming both. Evidence:
      `tools/timeline_event_markers.py` reads all six markers' colours *and*
      lanes back off `d4-markers-whole.png`.
- [x] Draw the lyric cue lane even when the Lyrics editor is closed.
      *Done (PX2). `Shell::closed_lyric_lane`, wired into `open_panel`'s
      `None`/`Tune` arms. Timing is judged against the waveform and the scene
      plan, which stay on screen when the editor does not — so closing the
      editor to see more preview used to take the cue blocks with it, and the
      commonest question in this workflow ("does that line land on that
      transient?") could only be asked in the layout that hides most of the
      picture.
      **It costs no band height**, which is the part worth defending. The
      `None`/`Tune` band is `EVENT_ROW_HEIGHT + DEFAULT_TIMELINE_HEIGHT +
      SCENE_SECTION_HEIGHT`, and the 180 spends 56 on the waveform strip and 28
      on the zoom row; the lane draws in slack already reserved and already
      empty, clamped to whatever is left, and draws nothing if that slack is
      ever spent. Growing the band instead would move `workspace_layout`'s
      sidebar guarantee at 720p — the arithmetic that already forced the manual
      event row out of the lyrics band — and UX0-C17 records that class of
      change as one that queues.
      Export and Assist are excluded on purpose: they take the whole band for
      their own budgets, so a lane there would push their bodies down rather
      than into slack.
      Fully interactive through the same `Shell::lyric_lane` the editor draws,
      so there is no second implementation to diverge, and every gesture is
      reversible now that Ctrl+Z exists.
      Evidence: the gate's lane-alignment sweep for `panel-none-*`,
      `panel-tune-*` and both 1440p captures moved 2 → 3, which is a
      **strengthening** — the closed lane is drawn from a different call site
      than the open one, and this is what proves it lands on the same time axis
      as the waveform above it. Plus a pixel measurement of user-origin cue
      blocks in the lane band, scoped to the bottom of the frame because the
      event row's "+ Scene" button is outlined in the same green and an
      unscoped count reports 148 of them on a frame with no cues at all.*
- [x] Add Shift-wheel pan and middle-drag pan with robust pointer-claim release.
      Middle-drag existed; Shift-wheel is new, and both now have a headless
      probe. `gesture=none` on the `timeline:` line is the release evidence.
- [x] Make waveform scrubbing transactional: pause on press, track the target
      while dragging, seek once on release and restore the prior play state.
- [x] On every transport discontinuity, clear queued pre-seek PCM and analyzer
      history as well as beat, scene-clock, scene-plan and cue-settings state.
- [x] Keep wheel zoom anchored at the pointer and every lane aligned through the
      shared `TimelineView` conversion. Landed with LX2-c; this tranche added the
      capture that proves it (`d4-wheel-zoom`, 5.760x centred on the pointer) and
      the Shift branch that must *not* re-anchor.
- [x] Give tick labels an opaque backing so waveform amplitude cannot erase their
      contrast.
- [x] Capture event colors, zoom, pan and an off-screen boundary.
      Lyric spans stay with PX2, who owns the cue lane.

Completion evidence (2026-08-07, PX5 — markers, pan and the tick plate):

**The lane's provenance was not recoverable, and that is why the markers needed a
core change rather than only a draw call.** The obvious way to colour a marker by
lane is to test `SEMANTIC_ID_LANE_BIT` on the merged id. It is wrong twice over,
and both halves are now pinned as tests. `qualify_semantic_id` adds
`COLLISION_PROBE_STEP` when the XORed id is already taken, which lands anywhere in
the 64-bit space and **clears the bit as often as not** — and that is not a corner
case, it is one XOR collision away: a manual id of `BIT|9` and a semantic id of
`9` collide on the first try. Meanwhile a manual id is carried through untouched,
so a manual event authored with the high bit set is indistinguishable from a
qualified semantic one. Type does not answer it either: **the manual event row's
`+ Feel` button records `EventType::Semantic` into the manual lane**
(`plug.c:2897`), so an amber marker may be either. `SceneEventMerge` now carries a
parallel `lanes` list, permuted with the records by the same sort rather than
sorted separately.

That change is **additive by construction**: no id and no ordering moves, so
`differential_event_merge.sh` is still 12 cases / 15 merged events / exact, run
against the frozen C with **no harness edit at all**. Same for
`differential_timeline_view.sh` and `differential_layout.sh` — this tranche
changed no geometry and no conversion, so all three are anchors that stayed put.
There is no deliberate divergence in this tranche and therefore no harness to
update; the one divergence recorded is a **presentation** choice the C has no
opinion on (head shape), and it is in the AGENTS.md table.

**Two negative controls, each decisive, and each aimed at a different check:**

| control | the gate | the unit tests |
| --- | --- | --- |
| the tick plate's one `draw_rectangle_rec` deleted | bleed **0 → 1444** waveform pixels; all seven labels go 0 → ~200, raised fraction halves 0.45 → 0.21 | **1368 green** |
| the semantic head's punch-out circle deleted | two markers read back as `manual` where the fixture has `semantic` | **1368 green**, *and the `timeline:` line was byte-identical* |

The second is the instructive one. `timeline:` counts markers per lane from the
same list the heads are drawn from, so collapsing the lanes in the *draw* left
`markers=manual:4 semantic:2` completely unchanged. Only reading the head shape
back off the framebuffer catches it, which is why `tools/timeline_event_markers.py`
exists rather than one more assertion on the report line.

**The plate is white on white, which is exactly why nothing had caught it.** The
lane's own background is `ui_raised`, so a missing plate is invisible wherever the
audio is quiet and the label is unreadable wherever it is loud — the defect *comes
and goes as the user scrolls*. `tools/timeline_tick_plate.py` therefore asserts an
exact zero (no envelope pixel inside a label's halo) rather than a contrast
threshold, and refuses to pass **vacuously**: at least one label must have the
envelope within 12 rows, or a fixture of pure silence would satisfy it. Two things
in that tool were wrong before they were right, both found by running it: an
ink-colour match within 24 found *no labels at all*, because a 12 px glyph is
mostly antialiased grey; and a box reconstructed from `plug.c`'s own `-3, -2, +6,
+4` offsets hung three rows below the real plate, because those offsets are around
the text *origin* and a digit's ink starts lower.

**The oracle's clip turned out to be load-bearing, and reading the measurement is
what found it.** `plug.c:3050` opens a `BeginScissorMode` around the whole
waveform block, which is easy to read as housekeeping. It is not: the marker cull
below it admits a marker up to **8 px outside** the lane on purpose, so a marker
whose line is just off-screen still shows the part of its head that belongs on
screen — and a head is 5 px in every direction. Without the scissor that head
paints onto the panel background outside the lane. The first version here had no
scissor, and the evidence was already sitting in the capture: the 4x frame's
right-edge marker measured **54 px of head against the usual 39**, because it was
spilling over the border. The tooltip is deliberately raised *after* the scissor
closes, since a tip clipped to the lane it explains would be cut off worst for the
markers nearest the edges.

**A `.musi` cannot carry an off-track event.** `validate_event_lanes` refuses any
lane holding a timestamp past `audio.duration_seconds`, so the oracle's
`plug.c:3089` bound is reachable only through live recording — the gate uses
`--event cue:38:99:1` on an 8 s fixture, and asserts `off-track:1` with nothing
drawn. That is recorded because the first fixture tried to seed one and the
project silently failed to load.

**Pan and zoom are told apart by the span, not by the picture.** Both move the
view, so a capture cannot distinguish them. From one 4x view at one pointer
position: a bare notch gives `5.760x 2.556..3.944` (span 2.000 → 1.388), a Shift
notch gives `4.000x 1.650..3.650` (span held at exactly 2.000), and the reverse
notch gives `2.850..4.850` — symmetric to the millisecond, and exactly
`2 x 2.000 x 0.15`. Middle-drag is the same: `900→400` and `400→900` move the
window ±0.794 s about the same origin. Both pans set `free-view`; both refusals
over a whole-track view leave it off, because a Follow button lit over a view that
never moved is a control claiming it did something.

**`--ui-probe middle-drag=FROMxTO` and `wheel-shift=0|1` are new, and
`--ui-probe wheel=` had no gate section at all before this.** The survey found
that `timeline:` — added by LX2 specifically as the wheel's evidence — was
asserted nowhere in `headless_check.sh`. `middle-drag=` exists because `click=`
cannot reach the pan: the pan reads `MOUSE_BUTTON_MIDDLE` from raylib directly
rather than through `Widgets`' pointer seam, since it claims nothing from the
bank. It stages press / two moved frames / release after the same three settling
frames `click=` waits for, and moves the *pointer* as well as the button — a
probe that moved only the button would drive a pan of zero seconds and photograph
as a broken gesture.

**A stranded claim is invisible in a picture**, so the `timeline:` line now
reports `gesture=`. The view sits where the hand left it whether or not the claim
released; the symptom is the *next* interaction behaving strangely, which arrives
as a bug report rather than a gate failure. The gate asserts `gesture=none` after
every drag, including one released at x=-400 — **outside the window** — which is
the overshoot case the plan names and the common way to end a fast drag.

New gate section: `tools/headless_check.sh:4174-4390`, one contiguous block,
17 assertions over 11 captures. All 17 pass, and the whole `headless_check.sh`
exits 0 with the section in it.

**The `frame budget` assertion is load-sensitive, and it is worth writing down
because it will mislead the next parallel wave.** It demands `0 of 240` frames
past a 25.0 ms threshold and bails the *entire* sweep with `exit 1`
(`headless_check.sh:242`), so every later section — including this one — is
skipped when it trips. On one unchanged commit (`4d7f974`) it measured **18.9 ms
/ 0 stalled** on an idle machine, **27.8 ms / 1** with two sibling agents running
gates, and **44.2 ms / 73** under heavier load. None of that is a code change.
A sibling independently measured the *base* commit failing it the same way. Two
practical consequences: give every concurrent gate its own
`MUSIALIZER_CAPTURE_DISPLAY` (a collision segfaults the app at `InitWindow` — this
run lost a whole sweep to `exit=139` on the transport captures), and read a stall
failure as a scheduling result until an idle re-run says otherwise.

**Left for the operator to judge: whether the markers want a legend.** LX1 gave
the cue lane "a legend and tooltips", and the symmetry argument says do the same
here. The reason it was not built: the four *type* colours already have a legend,
and it is the manual event row sitting directly above the strip — `+ Feel`,
`+ Cue` and `+ Custom` each carry the swatch of the colour they create, so the
key is next to the thing it keys. What no legend states is the **head shape**,
and that is in every marker's tooltip. A fifth strip of chrome in a band that is
already three lanes plus a zoom row is a real cost, so this is a taste call rather
than a gap — but it is a taste call, and it belongs to the operator.

Not covered: the gate asserts the tooltip's *text and hit test* (`hover=[manual
lyric  ·  00:00.750]` on the report line) but not that the tip **rendered**. The
tooltip render path is shared with every other tooltip and is gated by the
existing peak-luma check elsewhere in the sweep, so this is a deliberate boundary
rather than an oversight — recorded because "the shell knew the text" and "the
user saw it" are different claims.

Out of scope and left alone: `ui/panels/lyrics.rs` and the lyric cue lane (PX2's),
every other panel, all `mod.rs` files and the root manifests.

Noticed and **not** fixed, because it is outside this tranche: the application
**segfaults** rather than exiting with a message when GLFW cannot open the display
(`WARNING: GLFW: Failed to initialize GLFW` then SIGSEGV). Every headless tool in
`tools/` starts its own Xvfb first, so nothing here trips it, but a user running
the binary over a broken `DISPLAY` gets a core dump instead of a sentence.

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
- [x] Show truthful Saved/Working changes/Save failed/No file/Unfiled work state in the Tracks
      header, including lyric and route drafts.
      Done by PX1, 2026-08-07 — see UX0-B01 above for the surface and C4 for the
      draft guard. Drafts participate through `editor_draft_blocks_autosave`,
      which holds a track at Unsaved for as long as it owns an uncommitted lyric
      or route edit rather than writing a document the user has not committed to.
      Note the deliberate limit: the header names the *save* state, and a draft's
      own dirty marker is UX0-B07's surface, not this one.
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

## SX1 — Phosphor Dream, the first scene with no oracle (operator request, 2026-08-08)

The operator was handed the source of a third party's generative ASCII
screensaver — a Python offline renderer, `dreamscape.py` plus its `README.md`,
dropped into `OUTSIDE-DROPS/` — and asked for it as a scene. Scene name was left
to the agent.

**Shipped as `Phosphor Dream` / `phosphor`, scene id 10.** Not `dreamscape`: the
piece it grew out of is named after a copyrighted track, and a `stable_name` is
forever.

### What it is

Ten procedural scalar fields — domain-warped noise, four-sine plasma, checkered
tunnel, six-fold kaleidoscope, seven metaballs, falling glyph columns,
hyperspace, log spiral, moiré, interfering ripples — drawn as a grid of glyphs,
cycling on a `dwell` clock with a dithered crossfade between seven character
alphabets, under a slow rotate/zoom/ripple of the whole coordinate space. Then a
CRT: offscreen Gaussian bloom, chromatic split on the glow only, scanlines and a
rolling refresh band.

### What it cost, and what it moved

`SCENE_COUNT` went from 10 to 11 — the first time it has moved since the fork.
Appended at id 10, never inserted, so every C-era id keeps its value and a
`.musi` written before today resolves its scenes unchanged. `ORACLE_SCENE_COUNT`
is the new name for the ten the frozen C has, and it is what the harnesses read.

Three differential harnesses failed on the eleventh scene for reasons unrelated
to what they test. All three were **split rather than relaxed** — see the
`AGENTS.md` divergence table and the two `tests/differential/*_post_legacy.txt`
files. Both negative controls were run.

### What the captures found that the tests did not

Three defects, none of which any unit test could have caught, all found by
looking at a frame or at the report line:

1. **The filmic rolloff was in the wrong place.** The source applies it to the
   *bloomed* signal; running the same curve on the bare cell value is a 40 %
   dimmer, and it rendered Plasma as two green blobs on black. Not reproduced now.
2. **Flat rectangles for `░▒▓█` destroyed the ASCII.** Correct coverage, and a
   contiguous bright region merged into a featureless plate. Drawn as 4x4 dither
   patterns now, which is what the real shade blocks are.
3. **The scene had its own definition of "bass".** It averaged the lowest eighth
   of the instantaneous bands and read `0.00` on every frame while the caption
   glow's drive on those same frames worked. Both now go through
   `caption_effects::bass_from_trails`.

The `phosphor dream:` report line — field, alphabet, grid, `amp`/`bass`, mean and
peak cell, bloom outcome — exists because of (1) and (3). This scene draws a
plausible frame in at least four wrong states and a capture cannot separate them.

### Attribution (resolved 2026-08-08)

The source is by **Digi** — GitHub <https://github.com/digi-the-robot>,
X <https://x.com/digi_dot_exe>, canonical <https://linktr.ee/digi_the_robot> —
who handed it over and asked to be named. The credit is in the module docs of
both halves and here.

### Open

- **`OUTSIDE-DROPS/` holds Digi's original source** and was committed in
  `4ff32ca` and pushed. With attribution settled that is now a credited
  inclusion rather than an accident, but it remains their code: any edit to
  those files is out of bounds, and removing the directory is the operator's
  call, not an agent tidy-up.
- The scene is faithful to the source's authored brightness, which on a synthetic
  fixture reads dark. It reads well on real broadband audio (`amp` 0.7, `bass`
  0.4). If the operator wants it brighter at defaults, the lever is the base
  `pulse` term in `evaluate_grid`, not `reactivity` — the ±10 % audio modulation
  is deliberately gentle and the source's author asked that it stay that way for
  photosensitivity reasons.
