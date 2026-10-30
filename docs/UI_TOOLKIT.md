# Workspace controls and appearance

The right column holds Direct, Lyrics, Look, Assist and Render. The F8 modal
Editor was retired on 2026-10-30 by the operator: "we'll retire it and ensure
that all of its functionality gets correctly taken over into the new UI".
Look owns scene settings, Explore, presets and mappings. Lyrics owns cue text,
time fields, playhead stamps, Apply/Discard, navigation and undo. The transport,
top bar and time axis remain available while editing.

**Pixel look** is a group below Look's scene parameters: a filter on any scene,
scoped to Whole video or This section under the playhead. Choose a palette,
console colours or Pixels only, set the short-edge pixel count (slider or typed
value), and choose Answer the kick. A section can inherit The video's look or
explicitly turn it Off. Listed treatment colours remain visible and untouched
until another palette is chosen. Commands are the same `SetVideoPixel` and
`SetPixel`; values, defaults, schema and `.musi` bytes are unchanged.

Dark / Light is a quiet segmented switch in Look's fixed footer. The welcome
appearance picker remains available before opening audio. Explicit choices
persist in workstation UI preferences, independently of projects and exports.
`--ui-theme dark|light` and `black-amber|light-blue` remain session overrides.
`--editor-ui` was deliberately removed; use the column tabs, or `column=look`
in a UI probe. There is no application F8 binding or pencil rail slot.

The column has one visible lyric draft with Apply/Discard and context-change
guards. The independent modal's per-cue hidden draft cache was retired with it;
there are no competing lyric buffers or selection bridge. Column text fields
retain keyboard ownership: use Tracks → Save while a field is focused, or
Ctrl+S after leaving the field. The normal draft
still participates in quit/recovery guards. This policy difference is listed
in the retirement inventory for lead review.

## Integration boundary

- raylib column widgets emit `ShellCommand` / `LyricsEdit`; `main.rs` owns
  persistence, dirty marking and undo.
- `ui/egui_theme.rs` owns shared egui styling and the welcome picker, with its
  pointer-ownership and contrast tests. All tokens come from `ui/theme.rs`.
- `ui/egui_backend/` remains the input/texture/mesh bridge for Direct, terminal
  sessions, popups and welcome. egui 0.35 remains a required dependency.
- egui draws last on the default framebuffer, after raylib mode guards end;
  its backend drops before the window/context.
- Pure `core::ui::pixel_look` and `tune_groups` own geometry and choice rules.
  Numeric tests pin exact layouts and compare serialized projects against the
  toolkit's explicit values.

## Checks

`tools/pixel_look_check.sh` presses report-addressed controls with `click=`,
including gap controls; checks actual model values, native typed entry and
Escape, both appearance choices and restart persistence; captures both scopes
in dark/light at 960x640, 1280x720 and 1920x1080. It replaces the removed
`egui_editor_check.sh`. Existing Lyrics/Look/column gates retain settings,
cue edits, Save/undo, routing, dock and renderer coverage. Captures and negative
controls live under `build/wx5-editor/`; handoff notes and inventory under
`build/redesign/`. All app launches use the isolated, muted launcher.

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
  is `ui_level`, a warm grey. The column uses `widgets::slider_row` for scene values and pixel size.
- **Controls are rounded** (`metric::UI_RADIUS`, `widgets::control_box`); lanes
  and list rows stay square because they are data on an axis.
- **Things that float are one object** (2026-10-29): a notice card, the
  protocol card, the fullscreen pill and anything else drawn over the work
  rather than in the layout use `widgets::floating_surface` — the raised fill
  (`Palette::float_surface`), a hairline, the control radius and a soft shadow
  composited premultiplied. Tooltips take the same shape on their own dark
  tokens (`widgets::tooltip_plate`); egui popups get `popup_shadow`. A status
  speaks as a dot and a micro, tracked label (`Palette::notice_severity`), the
  way the top bar's save state does, never as a stripe.
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

WX-5 polish adds `protocol_marker`: an opaque violet graphic on each
appearance's lane fill (7.186638077640:1 dark, 6.230594944630:1 light), separate
from the blue light accent. Answered protocol pennants shorten instead of
fading below the contrast bar. READY reasons use tracked micro text and wrap in
a 130 px seat, keeping their full sentence visible.

About opens from the top-left logo (tooltip: “About Musializer”) or the quiet welcome control; its theme-role floating surface, scrollable credits and embedded notices close with ×, Escape or the scrim and own input while open.
The existing shell preserves a 960×640 logical minimum by clamping scale; About's 150% captures therefore use 1440×960 and 1920×1080 physical windows. Its pure geometry test also pins the smaller logical space of physical 960×640 and 1280×720 at 150%, without changing the shell's scale policy.

The legal reader keeps the interface face for its covered text and uses the built-in authored face for supplementary contributor alphabets (including Cyrillic); the About report names the serving face and missing-glyph count. Notice wrapping is cached by row, width, scale and available face.
