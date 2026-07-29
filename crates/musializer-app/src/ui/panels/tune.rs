//! The tuning inspector, and the route editor row inside it.
//!
//! **Owner: Agent G** for the route editor row. The slider list itself is
//! already ported and is only here because the row it expands lives inside it.
//!
//! The route editor is **not a new panel** — that was a misreading this
//! repository has already made once. It replaces the setting's own 30 px slider
//! zone with a taller block: `24+26+40+40+70+26+32+4` px, plus 24 more when the
//! source is `band` (`route_editor_state.h:10-16`, `plug.c:5517-5528`). Draft
//! edits, Apply commits, Close discards, and dirty participates in the close
//! guards — all of which [`musializer_core::ui::route_editor_state`] already
//! implements and tests.
//!
//! # Three seams this file cannot reach, and what it does instead
//!
//! Agent G's fan-out brief forbids editing `shell.rs`, `widgets.rs`, `theme.rs`
//! and `main.rs`, which is where three things the oracle's editor needs live.
//! Each is worked around here in a way that is one line to undo, and each is
//! marked `SEAM:` at its use site so a later session can find all of them with a
//! grep rather than by reading:
//!
//! 1. **The draft has nowhere to live.** The C keeps `Route_Editor_State` in the
//!    global `Plug` (`plug.c:5646`); the natural home here is a `Shell` field.
//!    The draft lives on [`Shell`] as `route_editor`, reached through
//!    [`Shell::with_editor`]/[`Shell::peek_editor`]. It was a `thread_local`
//!    while that field did not exist — Agent G flagged it as something that
//!    should not survive the merge, and it did not.
//! 2. **Apply and Remove need `&mut RouteTable`.** The shell only ever sees
//!    `&Workspace`, and the two [`ShellCommand`]s that would carry the commit do
//!    not exist yet. Both buttons therefore draw with the oracle's enable rules
//!    and say so by name through [`ShellCommand::NotImplemented`] — "missing
//!    beats pretending" — rather than being hidden or silently doing nothing.
//! 3. **The live source values are not in [`ShellInput`].** `main.rs` builds a
//!    `RouteSources` for the frame loop but hands the shell only `rms`. So RMS
//!    reads live and the other four sources honestly report `no signal`, which is
//!    the same wording the C uses when a source is unavailable
//!    (`plug.c:5673-5674`).

use musializer_core::scene::routes::{AnalysisSource, Interpolation, ParameterMapping, RouteTable};
use musializer_core::scene::settings::{self, SettingDescriptor};
use musializer_core::scene::SceneId;
use musializer_core::ui::notice::Severity;
use musializer_core::ui::route_editor_state::{self, RouteEditorState};
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle, Vector2};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::shell_layout::WorkspaceFrame;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};

// -- geometry, from `plug.c:5517-5523` ---------------------------------------

/// The seven stacked zones of the editor area, in the oracle's order
/// (`scene_route_editor_area_height`, `plug.c:5517-5523`).
///
/// Written as the sum the C writes rather than as `262.0`, because the numbers
/// are what a later reader checks against the drawing code below: each one is the
/// height of the block that consumes it, and `route_editor_row` advances its
/// cursor by exactly these.
#[rustfmt::skip]
const ROUTE_EDITOR_AREA_HEIGHT: f32 = 24.0  // live caption + meter
                                    + 26.0  // the five source buttons
                                    + 40.0  // the low anchor
                                    + 40.0  // the high anchor
                                    + 70.0  // the transfer graph
                                    + 26.0  // the curve stepper
                                    + 32.0  // the action row
                                    + 4.0; // trailing

/// The band stepper only exists for [`AnalysisSource::Band`] (`plug.c:5522`).
const ROUTE_EDITOR_BAND_ROW_HEIGHT: f32 = 24.0;

/// The label/readout/`~` line above the editor area.
///
/// **Divergence, deliberate.** The C's setting row is 76 px with a 29 px header
/// and 12 px of trailing rule space, so its expanded row is `area + 41`
/// (`plug.c:6113`, `:6191`). This inspector's row is 46 px with a 22 px header
/// and a 4 px gap, so its expanded row is `area + 26`. The *area* is reproduced
/// exactly — it is the part with controls in it — and only the row furniture
/// around it follows this inspector's own rhythm. Matching the C's 41 here would
/// put 15 px of nothing under a block that already ends in its own 4 px of
/// trailing space.
const ROUTE_EDITOR_ROW_HEADER: f32 = 22.0;
const ROUTE_EDITOR_ROW_GAP: f32 = 4.0;

/// `gap` in `scene_route_editor_panel` (`plug.c:5648`).
const GAP: f32 = 5.0;
/// The column between an anchor's two sliders, holding the arrow
/// (`plug.c:5723`).
const ARROW_WIDTH: f32 = 18.0;

// -- widget ids ---------------------------------------------------------------
//
// All inside `widgets::id::INSPECTOR`, below the 900 the reset button uses. The
// per-row `~` buttons are indexed by setting so two rows can never claim one
// another's press; the editor's own controls are singletons because at most one
// editor is open.
mod slot {
    /// `~` for setting `index`.
    pub const ROUTE_TOGGLE: u32 = 100;
    /// The collapsed routed summary, which is also a hit target (`plug.c:6210`).
    pub const ROUTED_SUMMARY: u32 = 150;
    pub const SOURCE: u32 = 200;
    pub const BAND_PREVIOUS: u32 = 210;
    pub const BAND_NEXT: u32 = 211;
    pub const INPUT_LOW: u32 = 220;
    pub const INPUT_HIGH: u32 = 221;
    pub const OUTPUT_LOW: u32 = 222;
    pub const OUTPUT_HIGH: u32 = 223;
    pub const CURVE_PREVIOUS: u32 = 230;
    pub const CURVE_NEXT: u32 = 231;
    pub const ACTION: u32 = 240;
}

// -- the draft's temporary home ----------------------------------------------

/// SEAM 1: the route editor draft, and the track slot it is keyed against.
///
/// See this module's header. `track_slot` is cached here because
/// [`Shell::route_editor_height`] is asked before the row is measured and is not
/// handed the workspace, while [`RouteEditorState::targets`] needs the slot to
/// keep a hidden draft for another track from expanding a row on this one
/// (`route_editor_state.c:101-114`).
pub(crate) struct EditorHost {
    state: RouteEditorState,
    track_slot: usize,
}

impl Default for EditorHost {
    fn default() -> Self {
        Self {
            state: RouteEditorState::new(),
            track_slot: 0,
        }
    }
}

impl Shell {
    fn with_editor<T>(&mut self, f: impl FnOnce(&mut EditorHost) -> T) -> T {
        f(&mut self.route_editor)
    }

    fn peek_editor<T>(&self, f: impl FnOnce(&EditorHost) -> T) -> T {
        f(&self.route_editor)
    }
}

/// Whether an open, dirty route draft should stop the application quitting.
///
/// This is `route_editor_dirty` in the C's close guard (`plug.c:7248`), which
/// `main.rs`'s `confirm_close` still weighs only dirty projects. Exposed here so
/// adding it there is one call rather than a second copy of the draft.
#[allow(
    dead_code,
    reason = "the close guard in main.rs is the integration owner's line to add; see the Agent G note in REWRITE_PLAN.md"
)]
impl Shell {
    pub(crate) fn route_edit_is_dirty(&self) -> bool {
        self.peek_editor(|host| host.state.is_dirty())
    }
}

/// `--ui-probe route=KEY`: opens the editor on a named setting.
///
/// Applied by `main.rs` with the rest of the probe, before the first frame. It
/// exists at all because the expanded editor is 260 px of drawing whose only
/// other entrance is a mouse click, and this repository has already paid twice
/// for a surface no capture could reach.
///
/// Returns the line the run prints as evidence. A screenshot cannot say whether
/// the block under the cursor is the editor or a very tall slider, so the line
/// names which setting opened, how tall the row it asked for is, and whether it
/// opened onto a committed route or a fresh full-range draft.
/// `headless_check.sh` asserts on it.
impl Shell {
    pub(crate) fn open_route_editor_probe(
        &mut self,
        key: &str,
        drawn_scene: SceneId,
        track_slot: usize,
        committed: Option<&ParameterMapping>,
    ) -> String {
        let Some((scene, index, _)) = settings::descriptor_by_key(key) else {
            return format!("route editor: {key} UNKNOWN");
        };
        // A key from another scene is not an error: a capture may probe a scene it
        // then switches away from. It simply does not expand a row here.
        if scene != drawn_scene {
            return format!("route editor: {key} not on the drawn scene");
        }
        self.route_editor.track_slot = track_slot;
        if !self.with_editor(|host| host.state.open(track_slot, scene, index, committed)) {
            return format!("route editor: {key} REFUSED");
        }
        let (height, source) = self.peek_editor(|host| {
            let source = host
                .state
                .draft()
                .map_or("?", |draft| route_editor_state::source_label(draft.source));
            (
                ROUTE_EDITOR_ROW_HEADER
                    + ROUTE_EDITOR_AREA_HEIGHT
                    + if host
                        .state
                        .draft()
                        .is_some_and(|draft| draft.source == AnalysisSource::Band)
                    {
                        ROUTE_EDITOR_BAND_ROW_HEIGHT
                    } else {
                        0.0
                    }
                    + ROUTE_EDITOR_ROW_GAP,
                source,
            )
        });
        format!(
            "route editor: {key} open row={height}px source={source} committed={}",
            u8::from(committed.is_some())
        )
    }
}

impl Shell {
    /// The tuning inspector: one slider per setting of the active scene.
    ///
    /// Bounds, defaults and precision all come from the descriptor table in
    /// [`settings`], which was checked column-by-column against the C. The
    /// inspector never invents a range.
    pub(crate) fn inspector(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if frame.inspector.is_empty() {
            return;
        }
        // The slot the draft is keyed against. Cached before any row is measured,
        // because `route_editor_height` is asked without the workspace.
        let track_slot = input.workspace.current_index().unwrap_or(0);
        self.with_editor(|host| host.track_slot = track_slot);

        let content = widgets::panel(d, input.fonts.ui(), frame.inspector, "TUNE");
        let font = input.fonts.ui();
        let padding = metric::UI_PANEL_PADDING;
        let mut y = content.y + padding;

        widgets::draw_text(
            d,
            input.fonts.ui(),
            input.scene.display_name(),
            content.x + padding,
            y,
            metric::UI_FONT_HEADER,
            color::ui_ink(),
        );
        y += metric::UI_FONT_HEADER + metric::UI_CONTROL_GAP;

        // The shared preset block, between the header and the first slider
        // (`plug.c:5979-6100`): 42 px collapsed, 98 px populated, 0 if it does
        // not fit.
        let mut presets = Vec::new();
        y += self.preset_block(
            d,
            input,
            UiRect::new(content.x, y, content.width, content.y + content.height - y),
            input.presets,
            &mut presets,
        );
        commands.extend(presets.into_iter().map(ShellCommand::Preset));

        let descriptors = settings::descriptors(input.scene);
        let row_height = 46.0f32;
        for (index, descriptor) in descriptors.iter().enumerate() {
            // The expanded route editor asks for the row before it is measured,
            // because a row it owns is taller than a slider row and the strip
            // must be checked against the height it will actually use.
            let expanded = self.route_editor_height(input.scene, index);
            let row = UiRect::new(
                content.x + padding,
                y,
                content.width - padding * 2.0,
                if expanded > 0.0 { expanded } else { row_height },
            );
            if !content.contains(row) {
                // Out of room. Say so rather than silently dropping the tail: a
                // truncated list that does not admit it is a feature nobody can
                // find.
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    &format!("+{} more (enlarge the window)", descriptors.len() - index),
                    content.x + padding,
                    y,
                    metric::UI_FONT_CAPTION,
                    color::ui_warning(),
                );
                break;
            }
            y += row.height;

            let committed = committed_route(input, input.scene, index).cloned();
            let routed = committed.is_some();

            // The `~` affordance, right-aligned on the label line
            // (`plug.c:6169-6186`). Selected while the setting is routed or its
            // editor is open, so the row says which of the two it is.
            let toggle = UiRect::new(row.x + row.width - 26.0, row.y - 3.0, 26.0, 20.0);
            let toggle_id =
                widgets::widget_id(widgets::id::INSPECTOR, slot::ROUTE_TOGGLE + index as u32);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    toggle_id,
                    toggle,
                    "~",
                    routed || expanded > 0.0,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
            {
                self.toggle_route_editor(input, index, committed.as_ref(), expanded > 0.0);
            }

            // The label and the readout are drawn for *every* row, expanded
            // included (`plug.c:6156-6190`): the C's editor replaces the slider
            // zone, not the line that names the setting. Losing the readout there
            // would leave the one row the user is working on as the only one that
            // does not say what value it currently produces.
            let value = input.settings.get(input.scene, index);
            let effective = input
                .routed
                .map_or(value, |routed| routed.get(input.scene, index));

            widgets::draw_text(
                d,
                input.fonts.ui(),
                descriptor.label,
                row.x,
                row.y,
                metric::UI_FONT_CAPTION,
                if routed {
                    color::accent()
                } else {
                    color::ui_muted()
                },
            );
            let readout = format!(
                "{:.*}{}",
                descriptor.precision as usize,
                effective,
                if routed { "  routed" } else { "" }
            );
            let readout_width = widgets::measure(font, &readout, metric::UI_FONT_VALUE);
            widgets::draw_text(
                d,
                input.fonts.ui(),
                &readout,
                row.x + row.width - readout_width - toggle.width - 8.0,
                row.y,
                metric::UI_FONT_VALUE,
                if routed {
                    color::accent()
                } else {
                    color::ui_ink()
                },
            );

            // An expanded row replaces the whole slider zone rather than sitting
            // beside it (`plug.c:5517-5528`), so nothing below runs for it.
            if expanded > 0.0 {
                self.route_editor_row(d, input, row, input.scene, index, commands);
                continue;
            }

            // A routed setting shows the route, not a slider that appears to move
            // on its own: the summary, the live meter, and a hit target that opens
            // the editor (`plug.c:6197-6215`).
            if let Some(route) = committed.as_ref() {
                self.routed_row(d, input, row, index, descriptor, route);
                continue;
            }

            let span = descriptor.maximum - descriptor.minimum;
            let normalized = if span > 0.0 {
                (effective - descriptor.minimum) / span
            } else {
                0.0
            };
            let track = UiRect::new(row.x, row.y + 22.0, row.width, 20.0);
            let id = widgets::widget_id(widgets::id::INSPECTOR, index as u32);
            if let Some(fraction) = self.widgets.slider(d, id, track, normalized) {
                let mut proposed = descriptor.minimum + fraction * span;
                // Precision 0 settings are integers in the C's readout, so the
                // slider must produce integers too or the readout lies.
                if descriptor.precision == 0 {
                    proposed = proposed.round();
                }
                commands.push(ShellCommand::SetSetting {
                    scene: input.scene,
                    index,
                    value: proposed,
                });
            }
        }

        let reset = UiRect::new(
            content.x + padding,
            content.y + content.height - metric::UI_BUTTON_HEIGHT - padding,
            content.width - padding * 2.0,
            metric::UI_BUTTON_HEIGHT,
        );
        if content.contains(reset) {
            let id = widgets::widget_id(widgets::id::INSPECTOR, 900);
            if self
                .widgets
                .text_button(
                    d,
                    input.fonts.ui(),
                    id,
                    reset,
                    "Reset scene",
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                commands.push(ShellCommand::ResetScene(input.scene));
            }
        }
    }

    /// Open, close, or refuse — the `~` button's three answers
    /// (`plug.c:6174-6186`).
    ///
    /// A *dirty* draft refuses rather than being replaced, and says why. That is
    /// the whole reason the draft exists: a half-authored route must not be lost
    /// to a click on a different row.
    fn toggle_route_editor(
        &mut self,
        input: &ShellInput<'_>,
        index: usize,
        committed: Option<&ParameterMapping>,
        editing_this_row: bool,
    ) {
        let track_slot = input.workspace.current_index().unwrap_or(0);
        let dirty = self.peek_editor(|host| host.state.is_dirty());
        if dirty {
            self.notify(
                Severity::Warning,
                "Route edit in progress",
                "Apply or discard the open route draft first.",
            );
            return;
        }
        if editing_this_row {
            self.with_editor(|host| host.state.close());
            return;
        }
        let opened =
            self.with_editor(|host| host.state.open(track_slot, input.scene, index, committed));
        if !opened {
            // `open` only refuses for a setting that has no descriptor or a
            // committed route that does not belong to it — both of which mean the
            // table and the descriptor tables disagree. Say so rather than leaving
            // a button that did nothing.
            self.notify(
                Severity::Error,
                "Route editor could not open",
                "This setting's committed route does not match its descriptor.",
            );
        }
    }

    /// The collapsed routed row: summary, live meter, and a click target that
    /// opens the editor (`plug.c:6197-6215`).
    fn routed_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        index: usize,
        descriptor: &SettingDescriptor,
        route: &ParameterMapping,
    ) {
        let font = input.fonts.ui();
        let summary = ascii_fallback(
            font.is_loaded(),
            &route_editor_state::summary(route, descriptor.precision),
        );
        widgets::draw_text(
            d,
            font,
            &summary,
            row.x,
            row.y + 20.0,
            metric::UI_FONT_CAPTION,
            color::ui_ink(),
        );
        let meter = UiRect::new(row.x, row.y + 37.0, row.width, 7.0);
        self.route_meter(d, input, meter, route);

        // The whole zone is the hit target, not just the `~` button
        // (`plug.c:6208-6215`).
        let hit = UiRect::new(row.x, row.y + 18.0, row.width, 28.0);
        let id = widgets::widget_id(widgets::id::INSPECTOR, slot::ROUTED_SUMMARY + index as u32);
        if self.widgets.button(d, id, hit).clicked {
            self.toggle_route_editor(input, index, Some(route), false);
        }
    }

    /// How tall the route editor row for this setting is, or `0.0` when it is
    /// not open for it.
    ///
    /// The oracle's editor *area* is `24+26+40+40+70+26+32+4` px, plus 24 more
    /// when the source is `band` (`plug.c:5517-5523`), and it is asked *before*
    /// the row is measured so a row that will not fit is never drawn — the layout
    /// rule this repository has already paid for. See
    /// [`ROUTE_EDITOR_ROW_HEADER`] for why the row is that plus 26 rather than
    /// the C's plus 41.
    pub(crate) fn route_editor_height(&self, scene: SceneId, index: usize) -> f32 {
        self.peek_editor(|host| {
            if !host.state.targets(host.track_slot, scene, index) {
                return 0.0;
            }
            let band = host
                .state
                .draft()
                .is_some_and(|draft| draft.source == AnalysisSource::Band);
            ROUTE_EDITOR_ROW_HEADER
                + ROUTE_EDITOR_AREA_HEIGHT
                + if band {
                    ROUTE_EDITOR_BAND_ROW_HEIGHT
                } else {
                    0.0
                }
                + ROUTE_EDITOR_ROW_GAP
        })
    }

    /// The expanded route editor, drawn in place of one setting's slider zone
    /// (`scene_route_editor_panel`, `plug.c:5643-5872`).
    ///
    /// Returns the height it consumed, which is the contract the caller advances
    /// by instead of by the slider row's height — how the C makes one row taller
    /// without a second layout pass.
    #[allow(
        clippy::too_many_arguments,
        reason = "the stub's signature, which the inspector's row loop calls"
    )]
    pub(crate) fn route_editor_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        scene: SceneId,
        index: usize,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let Some(descriptor) = settings::descriptor(scene, index) else {
            return 0.0;
        };
        let Some(draft) = self.peek_editor(|host| host.state.draft().cloned()) else {
            return 0.0;
        };
        let font = input.fonts.ui();

        // The label, the readout and the `~` button on the line above were all
        // drawn by the caller, for every row alike. This function owns the zone
        // the slider would have had, and nothing else.
        let area = UiRect::new(
            row.x,
            row.y + ROUTE_EDITOR_ROW_HEADER,
            row.width,
            row.height - ROUTE_EDITOR_ROW_HEADER - ROUTE_EDITOR_ROW_GAP,
        );
        // The oracle draws the block straight onto the panel; a faint raised
        // ground is what tells a reader that 260 px of controls belong to one row
        // rather than to the list.
        widgets::fill(d, area, color::ui_surface());

        let mut cursor = area.y;
        let live = self.live_value(input, draft.source, draft.band_index);

        // 1. The live caption and the input-window meter (`plug.c:5654-5681`).
        let mapped = live.and_then(|value| draft.output_value(descriptor, value));
        let caption = match (live, mapped) {
            (Some(live), Some(mapped)) => format!(
                "LIVE {} {:.3} {} {:.*}",
                route_editor_state::source_label(draft.source),
                live,
                arrow(font.is_loaded()),
                descriptor.precision as usize,
                mapped
            ),
            (Some(live), None) => format!(
                "LIVE {}  {:.3}",
                route_editor_state::source_label(draft.source),
                live
            ),
            _ => format!(
                "LIVE {}  no signal",
                route_editor_state::source_label(draft.source)
            ),
        };
        widgets::draw_text(d, font, &caption, area.x, cursor, 12.0, color::ui_muted());
        self.route_meter(
            d,
            input,
            UiRect::new(area.x, cursor + 14.0, area.width, 7.0),
            &draft,
        );
        cursor += 24.0;

        // 2. The five sources (`plug.c:5683-5697`).
        let source_width = (area.width - GAP * 4.0) / 5.0;
        for (slot_index, source) in AnalysisSource::ALL.into_iter().enumerate() {
            let button = UiRect::new(
                area.x + slot_index as f32 * (source_width + GAP),
                cursor,
                source_width,
                22.0,
            );
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::SOURCE + slot_index as u32);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    button,
                    route_editor_state::source_label(source),
                    draft.source == source,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
            {
                self.with_editor(|host| host.state.set_source(source));
            }
        }
        cursor += 26.0;

        // 3. The band stepper, only for the band source (`plug.c:5699-5721`).
        if draft.source == AnalysisSource::Band {
            // The live analyzer's band count, capped by the analyzer's own
            // maximum inside `step_band`. Zero means no analysis has arrived yet,
            // and the C falls back to the cap rather than refusing to step.
            let band_limit = if input.band_count > 0 {
                input.band_count
            } else {
                musializer_core::audio::analyzer::MAX_BANDS
            };
            let previous = UiRect::new(area.x, cursor, 34.0, 20.0);
            let next = UiRect::new(area.x + area.width - 34.0, cursor, 34.0, 20.0);
            for (id_slot, delta, label, boundary) in [
                (slot::BAND_PREVIOUS, -1, "<", previous),
                (slot::BAND_NEXT, 1, ">", next),
            ] {
                let id = widgets::widget_id(widgets::id::INSPECTOR, id_slot);
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        id,
                        boundary,
                        label,
                        false,
                        ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    )
                    .clicked
                {
                    self.with_editor(|host| host.state.step_band(delta, band_limit));
                }
            }
            let caption = format!("Band {} of {} (low to high)", draft.band_index, band_limit);
            let width = widgets::measure(font, &caption, 13.0);
            widgets::draw_text(
                d,
                font,
                &caption,
                area.x + (area.width - width) * 0.5,
                cursor + (20.0 - 13.0) * 0.5,
                13.0,
                color::ui_ink(),
            );
            cursor += ROUTE_EDITOR_BAND_ROW_HEIGHT;
        }

        // 4. The two anchors (`plug.c:5723-5776`).
        //
        // Each anchor pairs one source level with one output value, and the
        // physical grouping mirrors that pairing. The C notes why: the layout that
        // grouped the four sliders by axis hid the diagonal input→output
        // relationship the route actually is.
        let span = descriptor.maximum - descriptor.minimum;
        let span = if span <= 0.0 { 1.0 } else { span };
        let pair_width = (area.width - ARROW_WIDTH - GAP * 2.0) * 0.5;
        for anchor in 0..2u32 {
            let high = anchor == 1;
            let name = route_editor_state::anchor_label(draft.source, high);
            let anchor_input = if high {
                draft.input_max
            } else {
                draft.input_min
            };
            let anchor_output = if high {
                draft.output_max
            } else {
                draft.output_min
            };
            widgets::draw_text(
                d,
                font,
                &format!(
                    "{name}  {anchor_input:.2} {} {:.*}",
                    arrow(font.is_loaded()),
                    descriptor.precision as usize,
                    anchor_output
                ),
                area.x,
                cursor,
                12.0,
                color::ui_muted(),
            );
            let input_slider = UiRect::new(area.x, cursor + 15.0, pair_width, 20.0);
            let output_slider = UiRect::new(
                area.x + pair_width + ARROW_WIDTH + GAP * 2.0,
                cursor + 15.0,
                pair_width,
                20.0,
            );
            let arrow_glyph = arrow(font.is_loaded());
            let arrow_size = widgets::measure(font, arrow_glyph, 14.0);
            widgets::draw_text(
                d,
                font,
                arrow_glyph,
                area.x + pair_width + GAP + (ARROW_WIDTH - arrow_size) * 0.5,
                cursor + 15.0 + (20.0 - 14.0) * 0.5,
                14.0,
                color::ui_muted(),
            );

            let id = widgets::widget_id(
                widgets::id::INSPECTOR,
                if high {
                    slot::INPUT_HIGH
                } else {
                    slot::INPUT_LOW
                },
            );
            if let Some(fraction) = self
                .widgets
                .slider(d, id, input_slider, anchor_input as f32)
            {
                self.with_editor(|host| {
                    if high {
                        host.state.set_input_max(f64::from(fraction))
                    } else {
                        host.state.set_input_min(f64::from(fraction))
                    }
                });
            }
            let id = widgets::widget_id(
                widgets::id::INSPECTOR,
                if high {
                    slot::OUTPUT_HIGH
                } else {
                    slot::OUTPUT_LOW
                },
            );
            let normalized = (anchor_output as f32 - descriptor.minimum) / span;
            if let Some(fraction) = self.widgets.slider(d, id, output_slider, normalized) {
                let value = f64::from(descriptor.minimum + fraction * span);
                self.with_editor(|host| {
                    if high {
                        host.state.set_output_high(value)
                    } else {
                        host.state.set_output_low(value)
                    }
                });
            }
            cursor += 40.0;
        }

        // 5. The transfer graph (`plug.c:5778-5782`).
        let plot = UiRect::new(area.x, cursor, area.width, 64.0);
        transfer_graph(d, plot, &draft, descriptor, live);
        cursor += 70.0;

        // 6. The curve stepper (`plug.c:5784-5808`).
        let curve_previous = UiRect::new(area.x, cursor, 34.0, 22.0);
        let curve_next = UiRect::new(area.x + area.width - 34.0, cursor, 34.0, 22.0);
        let curves = Interpolation::ALL.len();
        let current = Interpolation::ALL
            .iter()
            .position(|curve| *curve == draft.interpolation)
            .unwrap_or(0);
        for (id_slot, step, label, boundary) in [
            (slot::CURVE_PREVIOUS, curves - 1, "<", curve_previous),
            (slot::CURVE_NEXT, 1, ">", curve_next),
        ] {
            let id = widgets::widget_id(widgets::id::INSPECTOR, id_slot);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    boundary,
                    label,
                    false,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
            {
                let next = Interpolation::ALL[(current + step) % curves];
                self.with_editor(|host| host.state.set_curve(next));
            }
        }
        let caption = format!(
            "Curve: {}",
            route_editor_state::curve_label(draft.interpolation)
        );
        let width = widgets::measure(font, &caption, 13.0);
        widgets::draw_text(
            d,
            font,
            &caption,
            area.x + (area.width - width) * 0.5,
            cursor + (22.0 - 13.0) * 0.5,
            13.0,
            color::ui_ink(),
        );
        cursor += 26.0;

        // 7. The action row (`plug.c:5810-5871`).
        self.route_actions(d, input, area, cursor, &draft, commands);
        row.height
    }

    /// Clamp / Swap / Apply / Remove / Close (`plug.c:5810-5871`).
    ///
    /// The enable rules are the oracle's: Apply is live only for a draft that is
    /// valid *and* either dirty or never committed, Remove only for a draft with a
    /// committed route behind it, and Close reads `Discard` while the draft is
    /// dirty so the destructive answer names itself.
    fn route_actions(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        cursor: f32,
        draft: &ParameterMapping,
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        let (dirty, can_apply, has_committed) = self.peek_editor(|host| {
            (
                host.state.is_dirty(),
                host.state.can_apply(),
                host.state
                    .session()
                    .is_some_and(|session| session.committed.is_some()),
            )
        });
        let apply_label = if has_committed { "Applied" } else { "Apply" };
        let close_label = if dirty { "Discard" } else { "Close" };
        let labels = ["Clamp", "Swap", apply_label, "Remove", close_label];

        let action_width = (area.width - GAP * 4.0) / 5.0;
        let widths = [action_width; 5];
        let font_size = widgets::row_font_size(font, &labels, &widths, 28.0);
        let boundary = |slot_index: usize| {
            UiRect::new(
                area.x + slot_index as f32 * (action_width + GAP),
                cursor,
                action_width,
                28.0,
            )
        };
        let id =
            |slot_index: u32| widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION + slot_index);

        if self
            .widgets
            .text_button(
                d,
                font,
                id(0),
                boundary(0),
                labels[0],
                draft.clamp,
                ButtonStyle::Neutral,
                Some(font_size),
            )
            .clicked
        {
            let next = !draft.clamp;
            self.with_editor(|host| host.state.set_clamp(next));
        }
        if self
            .widgets
            .text_button(
                d,
                font,
                id(1),
                boundary(1),
                labels[1],
                false,
                ButtonStyle::Neutral,
                Some(font_size),
            )
            .clicked
        {
            self.with_editor(|host| host.state.swap_output());
        }

        // SEAM 2: committing needs `&mut RouteTable`, which the shell never sees.
        // The enable rules are the oracle's so the row still teaches what Apply
        // costs; the click says the commit is not built rather than doing nothing.
        if can_apply && (dirty || !has_committed) {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id(2),
                    boundary(2),
                    "Apply",
                    true,
                    ButtonStyle::Neutral,
                    Some(font_size),
                )
                .clicked
            {
                commands.push(ShellCommand::ApplyRoute {
                    scene: input.scene,
                    route: draft.clone(),
                });
                self.with_editor(|host| host.state.close());
            }
        } else {
            self.widgets
                .disabled_button(d, font, boundary(2), apply_label, Some(font_size));
        }

        if has_committed {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id(3),
                    boundary(3),
                    labels[3],
                    false,
                    ButtonStyle::Danger,
                    Some(font_size),
                )
                .clicked
            {
                commands.push(ShellCommand::RemoveRoute {
                    scene: input.scene,
                    parameter: draft.parameter.clone(),
                });
                self.with_editor(|host| host.state.close());
            }
        } else {
            self.widgets
                .disabled_button(d, font, boundary(3), labels[3], Some(font_size));
        }

        if self
            .widgets
            .text_button(
                d,
                font,
                id(4),
                boundary(4),
                close_label,
                false,
                if dirty {
                    ButtonStyle::Danger
                } else {
                    ButtonStyle::Neutral
                },
                Some(font_size),
            )
            .clicked
        {
            self.with_editor(|host| host.state.close());
        }
    }

    /// The live-source meter: where this frame's source value sits inside the
    /// route's input window (`scene_route_meter`, `plug.c:5574-5590`).
    ///
    /// Draws empty when no signal is available, which is the state the C
    /// describes and — SEAM 3 — the state four of the five sources are in here.
    fn route_meter(
        &self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        bar: UiRect,
        route: &ParameterMapping,
    ) {
        let fill = self
            .live_value(input, route.source, route.band_index)
            .map_or(0.0, |live| route_editor_state::meter_position(route, live));
        widgets::fill(d, bar, color::ui_rule());
        widgets::fill(
            d,
            UiRect::new(bar.x, bar.y, bar.width * fill, bar.height),
            color::accent(),
        );
    }

    /// SEAM 3: this frame's value for one analysis source.
    ///
    /// `main.rs` already builds a [`musializer_core::scene::routes::RouteSources`]
    /// for the frame loop, but [`ShellInput`] carries only `rms`. RMS therefore
    /// reads live and the rest report no signal — which the meter and the caption
    /// both render as the C renders an unavailable source, rather than as a
    /// confident zero.
    fn live_value(
        &self,
        input: &ShellInput<'_>,
        source: AnalysisSource,
        band_index: u16,
    ) -> Option<f64> {
        // Every source now, not just RMS: `ShellInput` carries the same
        // `RouteSources` the frame loop evaluated the routes from, so the meter
        // and the route agree by construction rather than by coincidence.
        input.route_sources.value(source, band_index)
    }
}

/// The committed route driving one setting of the current track, if any
/// (`route_editor_find_route`, `route_editor_state.c:296-309`).
///
/// Reads the *track's* table rather than the effective settings, because "this
/// value differs from the slider" and "this setting is routed" are different
/// questions and only the second one may open the editor.
fn committed_route<'a>(
    input: &'a ShellInput<'_>,
    scene: SceneId,
    index: usize,
) -> Option<&'a ParameterMapping> {
    let table: &RouteTable = &input.workspace.current()?.scene_routes;
    route_editor_state::find_route(table, scene, index)
}

/// The transfer curve: source level across, setting value up
/// (`scene_route_transfer_graph`, `plug.c:5592-5641`).
///
/// Every sample goes through [`ParameterMapping::output_value`] — the same
/// function the frame loop uses — so curve shape, swapped outputs, clamping and
/// toggle quantization are drawn exactly as they will play. The shaded band is
/// the span between the two anchors; the dot rides the curve at the live value.
fn transfer_graph(
    d: &mut RaylibDrawHandle<'_>,
    plot: UiRect,
    route: &ParameterMapping,
    descriptor: &SettingDescriptor,
    live: Option<f64>,
) {
    if plot.is_empty() {
        return;
    }
    widgets::fill(d, plot, color::ui_raised());
    let window_left = route.input_min.clamp(0.0, 1.0) as f32;
    let window_right = route.input_max.clamp(0.0, 1.0) as f32;
    if window_right > window_left {
        let mut shade = color::accent();
        shade.a = (0.12 * 255.0) as u8;
        widgets::fill(
            d,
            UiRect::new(
                plot.x + plot.width * window_left,
                plot.y,
                plot.width * (window_right - window_left),
                plot.height,
            ),
            shade,
        );
    }
    d.draw_rectangle_lines_ex(widgets::rectangle(plot), 1.0, color::ui_rule());

    let mut span = f64::from(descriptor.maximum) - f64::from(descriptor.minimum);
    if span <= 0.0 {
        span = 1.0;
    }
    const SAMPLES: i32 = 64;
    let mut previous: Option<Vector2> = None;
    for step in 0..=SAMPLES {
        let source = f64::from(step) / f64::from(SAMPLES);
        let Some(value) = route.output_value(descriptor, source) else {
            previous = None;
            continue;
        };
        let point = Vector2::new(
            plot.x + plot.width * source as f32,
            plot.y + plot.height * (1.0 - ((value - f64::from(descriptor.minimum)) / span) as f32),
        );
        if let Some(from) = previous {
            d.draw_line_ex(from, point, 2.0, color::accent());
        }
        previous = Some(point);
    }

    if let Some(live) = live {
        if let Some(mapped) = route.output_value(descriptor, live) {
            let dot = Vector2::new(
                plot.x + plot.width * live.clamp(0.0, 1.0) as f32,
                plot.y
                    + plot.height
                        * (1.0 - ((mapped - f64::from(descriptor.minimum)) / span) as f32),
            );
            d.draw_circle_v(dot, 4.0, color::ui_ink());
            d.draw_circle_v(dot, 2.5, color::accent());
        }
    }
}

/// `→`, or `->` on the fallback face.
///
/// raylib's default face stops at ASCII 126 and draws U+2192 as a missing-glyph
/// box. The interface face has it (`font::ui_codepoint`'s `0x2190..=0x2199`
/// range), so the substitution is reachable only when that face failed to load —
/// exactly the guard `widgets`'s ellipsis already carries, for the same reason.
/// Takes `loaded` rather than the face itself so both substitutions are testable
/// without a window — a `Face::Default` can only be built by calling raylib.
fn arrow(loaded: bool) -> &'static str {
    if loaded {
        "\u{2192}"
    } else {
        "->"
    }
}

/// The same guard for [`route_editor_state::summary`], which carries U+00B7 and
/// U+2192 from the C's string literal.
fn ascii_fallback(loaded: bool, text: &str) -> String {
    if loaded {
        return text.to_string();
    }
    text.replace('\u{2192}', "->").replace('\u{00B7}', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::scene::settings::index;

    // The draft is a field of `Shell`, so each test gets a fresh one from
    // `Shell::new()` and there is nothing to leak between them. It used to be a
    // `thread_local`, which is why these tests were once wrapped in a
    // close-before-and-after helper; that helper is gone with the global.

    #[test]
    fn a_closed_editor_leaves_every_row_a_slider() {
        let shell = Shell::new();
        for index in 0..settings::descriptors(SceneId::Loom).len() {
            assert_eq!(shell.route_editor_height(SceneId::Loom, index), 0.0);
        }
    }

    #[test]
    fn the_expanded_row_is_the_oracles_area_plus_this_inspectors_furniture() {
        // The oracle's `scene_route_editor_area_height` (`plug.c:5517-5523`),
        // written out so a change to the constant has to change this number too.
        const ORACLE_AREA: f32 = 24.0 + 26.0 + 40.0 + 40.0 + 70.0 + 26.0 + 32.0 + 4.0;
        assert_eq!(ROUTE_EDITOR_AREA_HEIGHT, ORACLE_AREA);

        let mut shell = Shell::new();
        let weight = index::loom::WEIGHT;
        assert!(shell
            .route_editor
            .state
            .open(0, SceneId::Loom, weight, None));
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, weight),
            ORACLE_AREA + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP
        );
        // Only the row that hosts the draft grows.
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, index::loom::DENSITY),
            0.0
        );
        // And only for the scene it was opened on.
        assert_eq!(shell.route_editor_height(SceneId::SongAtlas, weight), 0.0);

        // The band source adds its stepper row, and the height is asked before
        // the row is measured — so the change has to be visible here.
        assert!(shell.route_editor.state.set_source(AnalysisSource::Band));
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, weight),
            ORACLE_AREA
                + ROUTE_EDITOR_BAND_ROW_HEIGHT
                + ROUTE_EDITOR_ROW_HEADER
                + ROUTE_EDITOR_ROW_GAP
        );
    }

    #[test]
    fn a_draft_for_another_track_does_not_expand_this_tracks_row() {
        // `route_editor_targets` is track-scoped for exactly this reason
        // (`route_editor_state.c:101-114`): a hidden draft must not reshape the
        // list of a track it does not belong to.
        let mut shell = Shell::new();
        let weight = index::loom::WEIGHT;
        assert!(shell
            .route_editor
            .state
            .open(1, SceneId::Loom, weight, None));
        shell.route_editor.track_slot = 0;
        assert_eq!(shell.route_editor_height(SceneId::Loom, weight), 0.0);
        shell.route_editor.track_slot = 1;
        assert!(shell.route_editor_height(SceneId::Loom, weight) > 0.0);
    }

    #[test]
    fn the_expanded_row_fits_the_inspector_at_the_minimum_window() {
        // The layout rule as a number. `WorkspaceFrame::layout` at the 960x640
        // minimum with the inspector open has to leave a content rectangle the
        // expanded row can sit in, or the row is never drawn and the feature is
        // unreachable at a size the application permits.
        let frame = WorkspaceFrame::layout(960.0, 640.0, true, 1, 150.0);
        let padding = metric::UI_PANEL_PADDING;
        // The panel header rule, then the scene name, then the first row.
        let available = frame.inspector.height
            - 27.0
            - padding
            - metric::UI_FONT_HEADER
            - metric::UI_CONTROL_GAP;
        assert!(
            available >= ROUTE_EDITOR_AREA_HEIGHT + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP,
            "the expanded row needs {} px and the 960x640 inspector offers {available}",
            ROUTE_EDITOR_AREA_HEIGHT + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP
        );
    }

    #[test]
    fn the_fallback_face_never_prints_a_missing_glyph_box() {
        // Both substitutions are guarded on `is_loaded`, and the guard is what a
        // capture caught the last time it was missing. Here only the ASCII side is
        // reachable without a window, which is the side that has to be correct.
        assert_eq!(
            ascii_fallback(false, "Band 2 \u{00B7} Smooth \u{00B7} 0.40 \u{2192} 2.20"),
            "Band 2 - Smooth - 0.40 -> 2.20"
        );
        assert_eq!(arrow(false), "->");
        assert_eq!(arrow(true), "\u{2192}");
    }
}
