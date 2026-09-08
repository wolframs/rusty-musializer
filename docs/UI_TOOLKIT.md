# Toolkit editor and appearance

Choose **Theme** in the top-right corner of the startup/project starter screen,
before opening audio or a project. The choice applies immediately and is saved
for future launches. Editor uses the same picker and selection.

Open **Editor** in the playback toolbar or press **F8**. The right-hand editor
has Tune and Lyrics tabs, Play/Pause, five-second seek buttons, Save project,
and a Light / blue or Black / amber appearance selector. Appearance saves in workstation UI preferences, independently
of music projects and exported scenes. The amber palette also covers the existing
workspace controls.

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
- `ui/theme.rs` owns the appearance enum and a UI-thread palette compatibility
  bridge for panels that have not moved yet. The pure core has no toolkit state.
- Draw egui last on the default framebuffer, after raylib mode guards finish.
  Its backend must drop before the window/context.

`--editor-ui` opens the editor at startup. `--ui-theme light-blue|black-amber`
sets a session override; changing the selector explicitly saves a preference.
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

The dark palette separates panel surfaces, raised rows, enabled controls, and
hover fills. Enabled controls use stronger outlines than structural separators;
disabled controls retain the panel fill and quiet rules. Shared dark tokens live
in `ui/theme.rs` and are also used by egui. Tooltips use explicit opaque surface,
text, and border colors, including the authored-text tooltip in the lyric lane.
`tools/dark_theme_check.sh` captures both tooltip skins and the dark Editor under
muted private Xvfb; evidence is written to `build/dark-theme-review/`. Palette
tests cover text contrast and the dark control boundaries, with the original
white-on-pale tooltip pairing retained as a failing contrast control.
