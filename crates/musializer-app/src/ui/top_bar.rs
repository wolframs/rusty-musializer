//! The top bar (WX-5c, `docs/WX5C_DESIGN.md` §4): the mark, the project
//! title and its status, the mode tabs, and Still / Contact sheet / Render.
//!
//! **Owner: the Top bar agent.** This file is the seam the 5c split was cut
//! along (§8): the rail's agent owns the frame and `ui/shell.rs`, this file
//! owns what is drawn inside [`WorkspaceFrame::top_bar`]. Everything here
//! reads the shell through `pub(crate)` state and methods; what it needs that
//! does not exist yet is asked of the rail's agent rather than added there.
//!
//! What the seam provides:
//!
//! - the rect, from [`WorkspaceFrame::top_bar`]: the workspace's width at
//!   y = 0, **zero height until the rail lands** (then 46 logical px), and
//!   empty in the fullscreen and editor frames;
//! - the widget namespace `widgets::id::TOP_BAR`;
//! - [`ModeTab`] with [`Shell::mode_tab_selected`] and
//!   [`Shell::activate_mode_tab`], the tabs as views onto `UiPanel` and the
//!   inspector;
//! - `shell::save_state_tint`, the save word's colour as the tracks header
//!   draws it;
//! - `ShellCommand::ExportStill`, `ExportContactSheet` and `StartRender`.
//!
//! [`WorkspaceFrame::top_bar`]: super::shell_layout::WorkspaceFrame::top_bar
//! [`ModeTab`]: super::shell::ModeTab

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::shell::{Shell, ShellCommand, ShellInput};

/// What the top bar drew in its last frame, kept for [`Shell::describe_top_bar`].
///
/// Lives on [`Shell`] as `top_bar_state`, and is this file's to grow: a field
/// added here needs no edit to `ui/shell.rs`. Reset to the default on the
/// welcome screen, which has no top bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TopBarState {
    /// The rect the bar was drawn into, `None` when it had no room.
    pub drawn: Option<UiRect>,
}

impl Shell {
    /// Draws the top bar into `rect`. Called by [`Shell::draw`] after the
    /// preview chrome, outside fullscreen.
    ///
    /// The seam draws nothing: the rect is zero-height until the rail lands.
    pub(crate) fn top_bar(
        &mut self,
        _d: &mut RaylibDrawHandle<'_>,
        _input: &ShellInput<'_>,
        rect: UiRect,
        _commands: &mut Vec<ShellCommand>,
    ) {
        self.top_bar_state.drawn = (!rect.is_empty()).then_some(rect);
    }

    /// The `top bar:` report line (§4): title, info, save state, tabs,
    /// actions and their rects. `none` while the bar has had no room.
    #[must_use]
    pub fn describe_top_bar(&self) -> String {
        match self.top_bar_state.drawn {
            None => "none".to_owned(),
            Some(rect) => format!(
                "rect={:.0},{:.0},{:.0},{:.0}",
                rect.x, rect.y, rect.width, rect.height
            ),
        }
    }
}
