# Toolkit editor and appearance

Choose **Theme** in the top-right corner of the startup/project starter screen,
before opening audio or a project. The choice applies immediately and is saved
for future launches. Editor uses the same picker and selection.

Open **Editor** in the playback toolbar or press **F8**. The right-hand editor
has Tune and Lyrics tabs, Play/Pause, five-second seek buttons, Save project,
and a Dark or Light appearance selector. Appearance saves in workstation UI
preferences, independently of music projects and exported scenes. Dark is the
default since WX-5a (2026-10-24); both appearances cover the whole workspace.

This is the first representative migration to **egui 0.35**, retaining the raylib
scene renderer, audio engine and export path. Tune uses the real scene setting
descriptors; Lyrics edits existing cues with text, start and end fields. Apply
uses the existing lyric validation and undo history. Save project and Ctrl+S
work inside Editor; unapplied lyric drafts must be applied before their text or
timing becomes project data. Close Editor before using the workspace undo shortcut. Unapplied drafts survive closing/reopening
within the current application session; they are not saved to disk. Unapplied
drafts participate in the existing quit warning, including when Editor is closed.
Opening follows the timeline's selected cue; closing selects the cue last edited
back in the timeline. Previous/Next navigate cues without losing their drafts.
Escape or F8 closes Editor. Long titles truncate with a full-title hover hint,
and the save status remains visible beside Save project.

The rest of the workspace remains on its existing controls. Future panel moves
should reuse the command boundary and theme rather than duplicate business logic.
The editor is modal for workspace input while playback and preview continue.
While open, inactive workspace panels are hidden and the complete scene is fitted
to the space on its left. This prevents the controls from covering the artwork.
Closing restores the previous workspace layout.

## Integration boundary

- `ui/egui_editor.rs` builds widgets and emits `ShellCommand` / `LyricsEdit`.
- `main.rs` applies those through the normal persistence, dirty and undo paths.
- `ui/egui_backend/` translates raylib input, textures and tessellated meshes.
  It adapts MIT-licensed egui-raylib code (license alongside the module), with
  deferred texture frees, atlas upload flushing, clipping and focus handling.
- `ui/theme.rs` owns the appearance enum, the `Palette` both appearances fill
  in, and the type scale (`metric::UI_FONT_*`). raylib reads them through
  `theme::color`, egui through `egui_editor::apply_theme`, which derives every
  `Visuals` colour, text style and the interface face from them (WX-5a). Nothing
  in egui carries a colour or size of its own. The pure core has no toolkit
  state.
- Draw egui last on the default framebuffer, after raylib mode guards finish.
  Its backend must drop before the window/context.

`--editor-ui` opens the editor at startup. `--ui-theme dark|light` (or the
persisted names `black-amber|light-blue`, which mean the same) sets a session
override; changing the selector explicitly saves a preference.
These flags combine with normal track/project paths and `--ui-scale`.

## Checks

`tools/egui_editor_check.sh` drives real pointer, wheel and keyboard events in
private Xvfb with process-muted playback and an unreachable audio sink. It checks
setting persistence, cue text/timing persistence, undo of the entire cue array,
and theme persistence/reload, with screenshots and JSON evidence under `build/`.
Focused editor tests cover real egui input events, draft retention and contrast.
The normal headless gate covers the scene renderer and exports after the
[raylib 6 upgrade](RAYLIB_6_UPGRADE.md).

The 2026-09-08 real-input run is recorded at
`build/egui-editor-modal-final/result.json`, including a regression check that a
click over the covered transport leaves the paused playhead at 0.800 seconds.
`build/egui-verify-final.log` records the quick gate (8 passed), including
1,728 passing Rust tests and one intentionally ignored runtime test.

The complete headless run (`build/egui-headless-final.log`) passed scene,
framebuffer, caption, export and protocol assertions. Its sole remaining failure
was an outdated recovery button coordinate in the existing layout test. The
corrected recovery section was rerun in full and passed snapshot integrity,
guarded reopen and restart checks (`build/raylib-upgrade/recovery-qa/result.log`).
The full headless script was not repeated after that test-only correction.
The release build log is `build/egui-release-final.log`.

## UX review, 2026-09-08

The focused editor view fits the complete export-shaped preview beside the
controls. Inactive tracks, scene browser and timeline no longer sit beneath it;
closing restores them. A pencil icon distinguishes Editor from the old Tune
inspector. Save and Ctrl+S work inside the editor, transport labels show the
current action, and five-second seeking avoids having to leave to audition a
different passage. Cue navigation preserves drafts and returns selection to the
workspace timeline. Long titles no longer compete with save controls; scrolling
uses the remaining panel height instead of a fixed header estimate.

The broader audit covered Welcome, the workspace, Tune, Lyrics, Assist and Export
at 960x640 and 1280x900, across the two themes. Assist status lines now ellipsize
inside the panel and reveal the full diagnostic on hover. Normal playing-to-paused
behavior was checked with Fence Around: the preview remains visible. A launch
parked before any audio analysis can still have a blank spectrum; that is distinct
from normal pause.

All captures use private Xvfb, --mute, unreachable Pulse/Wayland endpoints and
isolated preferences. No Plasma login session is needed for these checks.
The quick verification log is `build/ux-verify-final.log`, and the release build
log is `build/ux-release.log`. Visual evidence is under `build/ux-review/`.

The final real-input run is `build/ux-review/egui-final4/result.json`: settings,
scrolling, cue navigation, draft retention, Apply, in-editor Save, workspace undo,
Escape without quitting, F8 reopen and theme reload passed. At minimum size,
`build/ux-review/editor-tune-reset-960.png` confirms Reset is reachable. Escape is
owned by application/editor handlers; raylib's implicit Escape-to-quit is disabled.

Startup theme coverage: `tools/welcome_theme_check.sh` checks both theme choices,
persistence across restart, and opening audio through the normal picker followed
by Editor without reverting the skin. Its evidence is under
`build/welcome-theme-check/`. Pointer ownership is restricted to the picker and
its popup, so the startup actions stay interactive.

## Tokens and type (WX-5a, 2026-10-24)

`ui/theme.rs` holds one `Palette` per appearance: the mockup's dark workstation
(`docs/images/wx-mockup.jpg`) and the light appearance on the same roles. The
rules it encodes:

- **The accent means now.** The playhead, the scene section under it, the
  current cue row, the live reading on a mapping plot, the render progress bar,
  and the primary action (`ButtonStyle::Primary`: Render, Open audio, Recover
  session). Nothing else wears it, and `the_accent_is_worn_by_nothing_else`
  measures every other role, the notice severities and the event-type colours
  against it.
- **Selection is an outline and a tint** (`ui_selected` + `ui_selected_edge`,
  `widgets::button_look`, `widgets::selected_fill`), focus rings use the same
  outline, and a filled amount — slider, meter, scrub bar, the played waveform —
  is `ui_level`, a warm grey. egui sliders go through
  `egui_editor::level_slider`, which scopes the rail to `ui_control_edge` and
  its trailing fill to `ui_level`: egui paints a rail in the button rest fill,
  which with the palette applied was the panel's own colour.
- **Controls are rounded** (`metric::UI_RADIUS`, `widgets::control_box`); lanes
  and list rows stay square because they are data on an axis.
- **One type scale** for both toolkits, in logical pixels (egui points):
  header 16, label 14, value 14, caption 13, micro 11, and fitted button rows
  capped at 16. egui draws in Space Grotesk, the face the raylib chrome uses;
  its monospace stays Hack because the terminal grid depends on it.

The contrast sweep walks every role in both palettes against every surface it
can sit on, including the hover and selected tints, so a label cannot pass on a
panel and fail on its own selected row. Its negative control is a grey that
passes on the panel and fails on the selected tint, which the pre-WX-5a
two-surface sweep let through.

The terminal keeps its own dark palette in both appearances; its status bar
draws with `palette::DARK`'s roles.

WX-5b added three roles: `ui_cue_block` and `ui_cue_now` (the cue lane's
neutral block and the warm tint of the cue under the playhead — not the
accent; the playhead crossing it is), and `ui_run_cut`, the opaque hairline at
a cut inside a plan-lane run, which the gate counts by exact colour. The light
appearance's waveform ahead of the playhead lost half its blue cast
(`#BAC2D6` → `#BDC1CB`); `tools/timeline_tick_plate.py` classifies the envelope
by its channel order, so the two moved together.

WX-5c added no roles and one style. `ButtonStyle::Ghost` is a control that
draws only its glyph, in `ui_muted`, until it is hovered (`ui_hover`) or its
state holds (the selection outline and tint): the rail's four icons, the
transport's seek trio, readout, mute and fullscreen. The round play button is
an `ui_ink` disc with the glyph in `ui_surface`, and the volume chip is a
Neutral control with the level as a 3 px `ui_level` fill along its bottom and
the percentage over it. A flyout is `widgets::panel`'s `ui_surface` with a
`ui_control_edge` border and a soft black edge falling on the preview; none
of it wears the accent, which the gate's census checks with a flyout open in
the dark appearance and with an accent slab painted onto the rail as its
negative control.

The rail and its flyouts are raylib, not egui: they share the shell's widget
bank, so the claim rule, `--ui-probe click=` and `last_claimed_id` reach them
like any other control. A flyout's modality for presses is
`UiScale::occluded`: the shell draws everything under an open flyout with a
copy of the frame's input whose scale reports the pointer far outside the
window while it is over the flyout, and swaps the widget bank to that scale
for the same span. egui's dock is beside the workspace and never under a
flyout, so it is untouched.
