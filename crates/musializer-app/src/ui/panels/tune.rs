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

use musializer_core::scene::settings;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::shell_layout::WorkspaceFrame;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};

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

            // An expanded row replaces the whole slider zone rather than sitting
            // beside it (`plug.c:5517-5528`), so nothing below runs for it.
            if expanded > 0.0 {
                self.route_editor_row(d, input, row, input.scene, index, commands);
                continue;
            }

            let value = input.settings.get(input.scene, index);
            let effective = input
                .routed
                .map_or(value, |routed| routed.get(input.scene, index));
            let routed = (effective - value).abs() > f32::EPSILON;

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
                row.x + row.width - readout_width,
                row.y,
                metric::UI_FONT_VALUE,
                if routed {
                    color::accent()
                } else {
                    color::ui_ink()
                },
            );

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

    /// The expanded route editor, drawn in place of one setting's slider zone.
    ///
    /// **Agent G fills this.** Returning the height it consumed is the contract:
    /// the caller advances by that instead of by the slider row\'s height, which
    /// is how the C makes one row taller without a second layout pass.
    ///
    /// Returning `0.0` means "not open for this row", and the caller draws the
    /// ordinary slider.
    /// How tall the route editor row for this setting is, or `0.0` when it is
    /// not open for it.
    ///
    /// **Agent G fills this.** The oracle's answer is
    /// `24+26+40+40+70+26+32+4` px, plus 24 more when the source is `band`
    /// (`plug.c:5517-5523`), and it is asked *before* the row is measured so a
    /// row that will not fit is never drawn — the layout rule this repository
    /// has already paid for.
    #[allow(unused_variables)]
    pub(crate) fn route_editor_height(
        &self,
        scene: musializer_core::scene::SceneId,
        index: usize,
    ) -> f32 {
        0.0
    }

    // `ptr_arg` fires only because the stub never pushes; Agent G's body will.
    #[allow(clippy::too_many_arguments, clippy::ptr_arg, unused_variables)]
    pub(crate) fn route_editor_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        scene: musializer_core::scene::SceneId,
        index: usize,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        0.0
    }
}
