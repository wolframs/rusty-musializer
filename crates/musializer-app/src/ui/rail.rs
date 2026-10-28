//! The rail and its flyouts (WX-5c, `docs/WX5C_DESIGN.md` §1).
//!
//! The rail replaces the 320 px sidebar: four icons beside the preview and the
//! transport — Tracks and Scenes at the top, Editor and Dock at the bottom —
//! and the two that held the sidebar's panels open a **flyout** over the
//! preview's left edge instead of taking width from it for good. A flyout is
//! transient: it closes on Escape, on a press outside it, or on its icon, and
//! while it is open no press inside it reaches what it covers.
//!
//! The geometry is `musializer_core::ui::workspace_layout::WorkspaceRail`,
//! pure and pinned by `tools/differential_layout.sh`. This file is the
//! drawing and the open/close rules.

use musializer_core::ui::workspace_layout::{RailIcon, UiRect};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::icons;
use super::shell::{Flyout, Shell, ShellCommand, ShellInput};
use super::shell_layout::WorkspaceFrame;
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle};

/// How far the flyout's shadow falls on the preview to its right.
const FLYOUT_SHADOW: f32 = 14.0;

impl Shell {
    /// Decides, before anything is drawn, whether the open flyout survives
    /// this frame, and returns its rect if so.
    ///
    /// Run first because a press outside the flyout must close it on the same
    /// frame — otherwise the frame would draw the press's target occluded and
    /// the press would land on nothing. A press on a rail icon is not
    /// "outside": the icon itself toggles on release, and closing here would
    /// let that release reopen what the press just closed.
    pub(crate) fn flyout_begin(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
    ) -> Option<UiRect> {
        let Some(rail) = frame.rail_layout else {
            // Fullscreen and the editor hide the rail, and its flyouts with
            // it; coming back shows the plain workspace.
            self.open_flyout = None;
            return None;
        };
        self.open_flyout?;
        let rect = rail.flyout;
        let pointer = self.widgets.pointer(d);
        let on_icon = rail
            .icons
            .iter()
            .flatten()
            .any(|icon| icon.contains_point(pointer.x, pointer.y));
        if pointer.pressed && !on_icon && !rect.contains_point(pointer.x, pointer.y) {
            self.open_flyout = None;
            return None;
        }
        Some(rect)
    }

    /// The rail: its surface, a rule on its right edge, and four icons.
    ///
    /// Ghost buttons ([`ButtonStyle::Ghost`]): a glyph in muted ink until it
    /// is hovered, and the selection look — outline and tint, never the accent
    /// — while the state it names holds: its flyout open, the editor open, the
    /// dock open.
    pub(crate) fn rail(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(rail) = frame.rail_layout else {
            return;
        };
        let rect = rail.rail;
        if rect.is_empty() {
            return;
        }
        widgets::fill(d, rect, color::ui_surface());
        d.draw_line_ex(
            Vector2::new(rect.x + rect.width - 0.5, rect.y),
            Vector2::new(rect.x + rect.width - 0.5, rect.y + rect.height),
            1.0,
            color::ui_rule(),
        );

        for (slot, icon) in RailIcon::ALL.into_iter().enumerate() {
            let Some(boundary) = rail.icons[slot] else {
                continue;
            };
            let (control, selected) = match icon {
                RailIcon::Tracks => (icons::RAIL_TRACKS, self.open_flyout == Some(Flyout::Tracks)),
                RailIcon::Scenes => (icons::RAIL_SCENES, self.open_flyout == Some(Flyout::Scenes)),
                RailIcon::Editor => (icons::EDITOR, self.editor_open),
                RailIcon::Dock => (icons::DOCK, self.dock_open),
            };
            let id = widgets::widget_id(widgets::id::RAIL, slot as u32);
            let font_size = (boundary.height * 0.44).round();
            let state = match icons::glyph(input.fonts, &control) {
                icons::Glyph::Icon(face, text) => self.widgets.icon_button(
                    d,
                    face,
                    id,
                    boundary,
                    &text,
                    selected,
                    ButtonStyle::Ghost,
                    font_size,
                ),
                icons::Glyph::Text(font, text) => self.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary,
                    &text,
                    selected,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_MICRO),
                ),
            };
            self.widgets.hint(d, state, id, boundary, control.tip);
            if !state.clicked {
                continue;
            }
            match icon {
                RailIcon::Tracks => self.toggle_flyout(Flyout::Tracks),
                RailIcon::Scenes => self.toggle_flyout(Flyout::Scenes),
                RailIcon::Editor => {
                    if self.lyric_draft_allows_context_change(input.workspace) {
                        self.open_flyout = None;
                        self.editor_open = true;
                    }
                }
                RailIcon::Dock => commands.push(self.toggle_dock()),
            }
        }
    }

    /// Opens `flyout`, or closes it if it is the one already open. Opening
    /// the other one replaces it: there is one flyout seat.
    pub(crate) fn toggle_flyout(&mut self, flyout: Flyout) {
        self.open_flyout = if self.open_flyout == Some(flyout) {
            None
        } else {
            Some(flyout)
        };
    }

    /// The open flyout, over everything but the AI settings modal and the
    /// tooltip. Drawn with this frame's plain input: it is the one surface the
    /// pointer is not occluded from.
    pub(crate) fn flyout(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let (Some(kind), Some(rail)) = (self.open_flyout, frame.rail_layout) else {
            return;
        };
        let rect = rail.flyout;
        if rect.is_empty() {
            return;
        }
        // A soft edge on the preview side, so the flyout reads as lifted off
        // the work rather than as a panel that pushed it aside.
        d.draw_rectangle_gradient_h(
            (rect.x + rect.width) as i32,
            rect.y as i32,
            FLYOUT_SHADOW as i32,
            rect.height as i32,
            Color::new(0, 0, 0, 70),
            Color::new(0, 0, 0, 0),
        );
        match kind {
            Flyout::Tracks => self.tracks_panel(d, rect, input, commands),
            Flyout::Scenes => self.scene_browser(d, rect, input, commands),
        }
        d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, color::ui_control_edge());
        self.last_flyout = Some(rect);
    }

    /// The `rail:` report line (WX-5c §1): the rail of the last workspace
    /// frame, its icons, the open flyout and where it drew.
    ///
    /// `icons=` entries are `name@x,y,w,h` separated by `;`, like the scene
    /// picker's `tiles=`, because the rects carry commas of their own. A probe
    /// that presses an icon reads its point from here.
    #[must_use]
    pub fn describe_rail(&self) -> String {
        let Some(rail) = self.last_rail else {
            return "none".to_owned();
        };
        let icons: Vec<String> = RailIcon::ALL
            .into_iter()
            .zip(rail.icons)
            .filter_map(|(icon, rect)| {
                rect.map(|rect| format!("{}@{}", icon.name(), rect_token(rect)))
            })
            .collect();
        format!(
            "rect={} icons={} open={} flyout={}",
            rect_token(rail.rail),
            if icons.is_empty() {
                "none".to_owned()
            } else {
                icons.join(";")
            },
            self.open_flyout.map_or("none", Flyout::name),
            self.last_flyout.map_or("none".to_owned(), rect_token),
        )
    }
}

/// `x,y,w,h`, rounded to whole logical pixels, as every report line prints a
/// rect.
pub(crate) fn rect_token(rect: UiRect) -> String {
    format!(
        "{:.0},{:.0},{:.0},{:.0}",
        rect.x, rect.y, rect.width, rect.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::ui::workspace_layout::WorkspaceRail;

    #[test]
    fn a_rail_icon_toggles_its_flyout_and_the_other_one_replaces_it() {
        let mut shell = Shell::new();
        shell.toggle_flyout(Flyout::Tracks);
        assert_eq!(shell.open_flyout, Some(Flyout::Tracks));
        shell.toggle_flyout(Flyout::Scenes);
        assert_eq!(shell.open_flyout, Some(Flyout::Scenes));
        shell.toggle_flyout(Flyout::Scenes);
        assert_eq!(shell.open_flyout, None);
    }

    #[test]
    fn the_rail_line_names_every_icon_and_the_flyout_it_drew() {
        let mut shell = Shell::new();
        assert_eq!(shell.describe_rail(), "none");
        shell.last_rail = WorkspaceRail::layout(1280.0, 720.0, 46.0, 460.0);
        shell.open_flyout = Some(Flyout::Scenes);
        shell.last_flyout = Some(UiRect::new(52.0, 46.0, 336.0, 674.0));
        assert_eq!(
            shell.describe_rail(),
            "rect=0,46,52,414 icons=tracks@8,56,36,36;scenes@8,98,36,36;\
             editor@8,372,36,36;dock@8,414,36,36 open=scenes flyout=52,46,336,674"
        );
    }
}
