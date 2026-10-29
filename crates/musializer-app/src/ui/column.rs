//! The right column: one place every secondary view draws (WX-5d,
//! `build/redesign/wx5d-design.md`).
//!
//! **Owner: the Frame builder.** Direct (the dock), Lyrics, Look, Assist and
//! Render share one column of one width beside the preview, instead of each
//! panel taking the band under the timeline or a column of its own. Which view
//! it shows is [`super::shell::ColumnView`]; where its rects go is
//! [`super::shell_layout::WorkspaceFrame::column`].
//!
//! # The seam
//!
//! This file was created by the WX-5d seam commit so the three builders never
//! edit a `mod.rs`. Until the merge flips it, the column layout sits behind
//! [`COLUMN_LAYOUT_VARIABLE`]: off, every panel draws exactly as before; on,
//! each panel entry is handed [`Placement::Column`] and draws its column
//! branch, which falls back to the band until its builder has built it.

use musializer_core::ui::column_layout;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::shell::{ColumnView, Shell, ShellCommand, ShellInput};
use super::shell_layout::{WorkspaceFrame, DEFAULT_TIMELINE_HEIGHT};
use super::theme::{color, metric};
use super::widgets;
use crate::workspace::Workspace;

/// `MUSIALIZER_COLUMN_LAYOUT=1` turns the column layout on (WX-5d seam).
///
/// Read once at startup, like the `MUSIALIZER_ASSIST_*` seams, so a frame can
/// never see half a layout. Any other value, or none, is off: the switch exists
/// so the gate stays green on the band while the column is built, and it goes
/// away when the merge flips it.
pub const COLUMN_LAYOUT_VARIABLE: &str = "MUSIALIZER_COLUMN_LAYOUT";

/// Whether a value of [`COLUMN_LAYOUT_VARIABLE`] turns the column on. Exactly
/// `1`: a stray `0` or `false` must not switch a layout on.
#[must_use]
pub fn column_layout_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// [`column_layout_enabled`] for this process's environment.
#[must_use]
pub fn column_layout_from_env() -> bool {
    column_layout_enabled(std::env::var(COLUMN_LAYOUT_VARIABLE).ok().as_deref())
}

/// Where a panel entry draws this frame (WX-5d §4, seam item 3).
///
/// Passed to `lyrics_panel`, `assist_panel`, `export_panel` and `inspector`,
/// so each builder builds its `Column` branch in its own file. `Band` is
/// today's behaviour; the merge deletes it.
///
/// # The `Column` contract
///
/// - The view draws in [`Shell::column_rect`]: the whole column, head
///   included. The shell has already filled it with the surface and drawn its
///   left edge; the view draws its own head with [`widgets::column_head`]
///   (its title, then its switch in `trailing_right`) and its body below.
/// - `lyrics_panel` is called from the band as before, because the Lyrics
///   view owns the band's cue lane too; `content` and `strip` are the band's.
/// - `assist_panel` and `export_panel` are called once per frame while they
///   are the view, after the band. Their `content` is the column rect and
///   their `strip` a zero-height rect 28 px above it, which is only so a
///   `Band` fallback lands in the column instead of over the band.
/// - `inspector` is called while Look is the view, with `frame.inspector`
///   and `frame.column` both the column rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The panel's pre-WX-5d place: the band under the timeline, or for the
    /// Tune inspector its own right-hand column.
    Band,
    /// The right column (see the contract above).
    Column,
}

/// The column's resize grip: 6 px centred on its left edge (§2, "Resize").
#[must_use]
pub fn column_grip(column: UiRect) -> UiRect {
    UiRect::new(column.x - 2.5, column.y, 6.0, column.height)
}

/// What the column drew in its last frame, for the `column:` line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColumnReport {
    /// `None` when no workspace frame with the switch on has been drawn.
    pub drawn: Option<(ColumnView, UiRect)>,
    /// The band's rect in that frame: the same in every view (§1).
    pub band: UiRect,
    /// The Direct tab's dot while another view shows (§2): what the dock is
    /// doing that the user should come back for.
    pub badge: DirectBadge,
}

/// The Direct tab's dot (§2, "Running chats"). Never the accent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectBadge {
    #[default]
    None,
    /// An agent is working: success.
    Working,
    /// An agent waits on Allow/Deny: warning.
    Waiting,
    /// A reply arrived that nobody has looked at: ink.
    Reply,
}

impl DirectBadge {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            DirectBadge::None => "none",
            DirectBadge::Working => "working",
            DirectBadge::Waiting => "waiting",
            DirectBadge::Reply => "reply",
        }
    }

    /// The dot's tone, or nothing to draw.
    #[must_use]
    pub fn tone(self) -> Option<widgets::StatusTone> {
        match self {
            DirectBadge::None => None,
            DirectBadge::Working => Some(widgets::StatusTone::Success),
            DirectBadge::Waiting => Some(widgets::StatusTone::Warning),
            DirectBadge::Reply => Some(widgets::StatusTone::Ink),
        }
    }
}

/// The band's height with the column layout: the Direct band, 260, or the
/// user's dragged height, the same in every view (§1, "The band is
/// constant"). One floor, the band's own, and one ceiling, which leaves the
/// preview 150 px under the top bar and the transport.
#[must_use]
pub fn column_band_height(window_height: f32, requested: Option<f32>) -> f32 {
    let scene = super::panels::scene_timeline::SCENE_SECTION_HEIGHT;
    let automatic = DEFAULT_TIMELINE_HEIGHT + scene;
    let floor = scene + 150.0;
    let ceiling = (window_height
        - musializer_core::ui::workspace_layout::TOP_BAR_HEIGHT
        - metric::HUD_BUTTON_SIZE
        - 150.0)
        .max(floor);
    requested
        .filter(|value| value.is_finite())
        .map_or(automatic, |value| value.clamp(floor, ceiling))
}

impl Shell {
    /// Where the panels draw this frame: the column when the switch is on.
    #[must_use]
    pub(crate) fn placement(&self) -> Placement {
        if self.column_layout {
            Placement::Column
        } else {
            Placement::Band
        }
    }

    /// The column's rect in the frame being drawn, head included: what a
    /// view draws into under [`Placement::Column`]. Empty when the switch is
    /// off, the column is closed, or the frame (fullscreen, the editor)
    /// hides it.
    #[must_use]
    pub(crate) fn column_rect(&self) -> UiRect {
        self.column_frame
    }

    /// The column's width this frame, or `None` while it is closed.
    #[must_use]
    pub(crate) fn column_width(&self, window_width: f32) -> Option<f32> {
        (self.column_view() != ColumnView::Closed)
            .then(|| column_layout::column_width(window_width, self.ui_preferences.column_width))
    }

    /// Where the F8 Editor's panel goes (WX-5d §3.6): with the column layout
    /// the column's own place — its x, the full height, 16 px from the top,
    /// the bottom and the window's right edge — so opening it moves nothing
    /// on the right. Without it, the Editor's own rule.
    #[must_use]
    pub fn editor_panel(&self, window: (f32, f32)) -> UiRect {
        if !self.column_layout {
            return super::shell_layout::editor_rect(window);
        }
        let width = column_layout::column_width(window.0, self.ui_preferences.column_width);
        UiRect::new(
            window.0 - width,
            16.0,
            (width - 16.0).max(0.0),
            (window.1 - 32.0).max(0.0),
        )
    }

    /// The frame with the column (§1); `Shell::frame_for` calls it while the
    /// switch is on and the frame is neither fullscreen nor the editor.
    #[must_use]
    pub(crate) fn column_frame_for(&self, window: (f32, f32)) -> WorkspaceFrame {
        let mut frame = WorkspaceFrame::with_column(
            window.0,
            window.1,
            self.column_width(window.0),
            column_band_height(window.1, self.ui_preferences.timeline_height),
        );
        if self.column_view() == ColumnView::Look {
            frame.inspector = frame.column;
        }
        frame
    }

    /// Seeds the column from `ui.json` at startup (§2, "Persistence"): an
    /// absent `column_open` is open on Direct, an older file's `dock_width`
    /// seeds `column_width` once, and the view the column was last left on
    /// comes back.
    pub fn seed_column_from_preferences(&mut self) {
        if self.ui_preferences.column_width.is_none() {
            self.ui_preferences.column_width = self.ui_preferences.dock_width.map(|width| {
                width.clamp(
                    column_layout::COLUMN_DRAG_MIN,
                    column_layout::COLUMN_DRAG_MAX,
                )
            });
        }
        let view = match self.ui_preferences.column_open {
            None => ColumnView::Direct,
            Some(false) => ColumnView::Closed,
            Some(true) => self
                .ui_preferences
                .column_view
                .unwrap_or(ColumnView::Direct),
        };
        self.set_column_view(view);
    }

    /// Shows `view` if the column shows something else, or closes the column
    /// if it already shows `view`: a tab, T on Look, Render…, Ctrl+` on
    /// Direct (§2). Leaving Lyrics goes through the draft guard every context
    /// change takes. Returns whether the view changed.
    pub(crate) fn toggle_column_view(
        &mut self,
        view: ColumnView,
        workspace: &Workspace,
        commands: &mut Vec<ShellCommand>,
    ) -> bool {
        let target = if self.column_view() == view {
            ColumnView::Closed
        } else {
            view
        };
        self.show_column_view(target, workspace, commands)
    }

    /// The rail's column toggle: closes an open column, or reopens it on the
    /// view it was last left on (Direct the first time).
    pub(crate) fn toggle_column(
        &mut self,
        workspace: &Workspace,
        commands: &mut Vec<ShellCommand>,
    ) {
        let target = if self.column_view() == ColumnView::Closed {
            self.ui_preferences
                .column_view
                .filter(|view| *view != ColumnView::Closed)
                .unwrap_or(ColumnView::Direct)
        } else {
            ColumnView::Closed
        };
        self.show_column_view(target, workspace, commands);
    }

    /// [`Shell::set_column_view`] for a user's press: behind the lyric-draft
    /// guard, remembered in `ui.json`.
    pub(crate) fn show_column_view(
        &mut self,
        view: ColumnView,
        workspace: &Workspace,
        commands: &mut Vec<ShellCommand>,
    ) -> bool {
        let current = self.column_view();
        if current == view {
            return false;
        }
        if current == ColumnView::Lyrics && !self.lyric_draft_allows_context_change(workspace) {
            return false;
        }
        self.set_column_view(view);
        self.ui_preferences.column_open = Some(view != ColumnView::Closed);
        if view != ColumnView::Closed {
            self.ui_preferences.column_view = Some(view);
        }
        commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
        true
    }

    /// The column's surface and left edge, drawn before any view so each
    /// view draws on the same ground.
    pub(crate) fn column_chrome(&mut self, d: &mut RaylibDrawHandle<'_>, _input: &ShellInput<'_>) {
        let rect = self.column_frame;
        if !self.column_layout {
            return;
        }
        if rect.is_empty() {
            self.column_report.drawn = Some((self.column_view(), rect));
            return;
        }
        widgets::fill(d, rect, color::ui_surface());
        widgets::fill(
            d,
            UiRect::new(rect.x, rect.y, 1.0, rect.height),
            color::ui_rule(),
        );
        self.column_report.drawn = Some((self.column_view(), rect));
    }

    /// The views the column draws itself, rather than from the band: Assist
    /// and Render. Look is the inspector, Lyrics comes from the band, and
    /// Direct is the dock, which the composition root draws with egui.
    pub(crate) fn column_views(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        _frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let rect = self.column_rect();
        if rect.is_empty() {
            return;
        }
        // See `Placement`'s contract for why `strip` sits 28 px above.
        let strip = UiRect::new(rect.x, rect.y - 28.0, rect.width, 0.0);
        match self.column_view() {
            ColumnView::Assist => {
                self.assist_panel(d, input, rect, strip, commands, Placement::Column);
            }
            ColumnView::Render => {
                self.export_panel(d, input, rect, strip, commands, Placement::Column);
            }
            ColumnView::Closed | ColumnView::Direct | ColumnView::Lyrics | ColumnView::Look => {}
        }
    }

    /// The `column:` report line (§5): view, open state and the head, body
    /// and grip rects, and the Direct tab's dot. Empty while the switch is
    /// off, and an empty line is not printed, so no report changes then.
    #[must_use]
    pub(crate) fn describe_column(&self, _workspace: &Workspace) -> String {
        if !self.column_layout {
            return String::new();
        }
        let Some((view, rect)) = self.column_report.drawn else {
            return "none".to_owned();
        };
        let at = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        let badge = format!("direct:{}", self.column_report.badge.name());
        if rect.is_empty() {
            // Closed, or hidden by a frame that has no column (fullscreen,
            // the Editor): the view is still the one it will come back on.
            return format!(
                "view={} open=0 band={} badge={badge}",
                view.name(),
                at(self.column_report.band)
            );
        }
        let head = widgets::column_head_layout(rect, 0.0);
        format!(
            "view={} open=1 rect={} head={} body={} grip={} band={} badge={badge}",
            view.name(),
            at(rect),
            at(head.head),
            at(head.body),
            at(column_grip(rect)),
            at(self.column_report.band),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_exact_one_turns_the_column_on() {
        assert!(column_layout_enabled(Some("1")));
        for off in [
            None,
            Some(""),
            Some("0"),
            Some("true"),
            Some(" 1"),
            Some("yes"),
        ] {
            assert!(!column_layout_enabled(off), "{off:?} must leave the band");
        }
        assert_eq!(COLUMN_LAYOUT_VARIABLE, "MUSIALIZER_COLUMN_LAYOUT");
    }

    #[test]
    fn the_band_is_260_or_the_users_in_every_view() {
        assert_eq!(column_band_height(720.0, None), 260.0);
        assert_eq!(column_band_height(1080.0, Some(400.0)), 400.0);
        // The ceiling leaves the preview 150 under the top bar and transport.
        assert_eq!(
            column_band_height(720.0, Some(9000.0)),
            720.0 - 46.0 - 50.0 - 150.0
        );
        // The band's own floor.
        assert_eq!(column_band_height(720.0, Some(10.0)), 80.0 + 150.0);
        assert_eq!(column_band_height(720.0, Some(f32::NAN)), 260.0);
    }

    #[test]
    fn the_grip_is_centred_on_the_columns_edge() {
        let grip = column_grip(UiRect::new(922.0, 46.0, 358.0, 674.0));
        assert_eq!(grip, UiRect::new(919.5, 46.0, 6.0, 674.0));
        assert_eq!(grip.x + grip.width * 0.5, 922.5);
    }

    #[test]
    fn the_column_reopens_where_it_was_left_and_a_press_on_the_view_closes_it() {
        let workspace = Workspace::new();
        let mut commands = Vec::new();
        let mut shell =
            Shell::with_preferences(super::super::preferences::UiPreferences::default());
        shell.column_layout = true;
        shell.seed_column_from_preferences();
        assert_eq!(
            shell.column_view(),
            ColumnView::Direct,
            "absent column_open is Direct"
        );
        assert!(shell.toggle_column_view(ColumnView::Assist, &workspace, &mut commands));
        assert_eq!(shell.column_view(), ColumnView::Assist);
        assert!(shell.toggle_column_view(ColumnView::Assist, &workspace, &mut commands));
        assert_eq!(shell.column_view(), ColumnView::Closed);
        assert_eq!(shell.ui_preferences.column_open, Some(false));
        shell.toggle_column(&workspace, &mut commands);
        assert_eq!(
            shell.column_view(),
            ColumnView::Assist,
            "back where it was left"
        );
        assert_eq!(commands.len(), 3);
    }

    #[test]
    fn an_older_files_dock_width_seeds_the_column_once() {
        let mut preferences = super::super::preferences::UiPreferences::default();
        preferences.dock_open = true;
        preferences.dock_width = Some(480.0);
        let mut shell = Shell::with_preferences(preferences);
        shell.column_layout = true;
        shell.seed_column_from_preferences();
        assert_eq!(shell.ui_preferences.column_width, Some(480.0));
        assert_eq!(shell.column_width(1920.0), Some(480.0));
        preferences.column_width = Some(400.0);
        preferences.column_open = Some(false);
        let mut shell = Shell::with_preferences(preferences);
        shell.column_layout = true;
        shell.seed_column_from_preferences();
        assert_eq!(shell.ui_preferences.column_width, Some(400.0));
        assert_eq!(shell.column_view(), ColumnView::Closed);
        assert_eq!(shell.column_width(1920.0), None);
    }

    #[test]
    fn the_switch_alone_decides_the_placement() {
        let mut shell =
            Shell::with_preferences(super::super::preferences::UiPreferences::default());
        assert_eq!(shell.placement(), Placement::Band);
        shell.column_layout = true;
        assert_eq!(shell.placement(), Placement::Column);
    }
}
