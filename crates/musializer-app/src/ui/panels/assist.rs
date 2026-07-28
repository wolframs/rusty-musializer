//! The Assist confirmation panel.
//!
//! **Owner: Agent J.** C sources: `analysis_bridge.c`, `analysis_candidate.c`,
//! `assist_ui_state.c`, `plug.c:2143`, `:2176-2337` — all ported, including
//! `runtime::process::assist` with the `setsid`/`EPERM` trap already handled.
//!
//! Missing: the confirmation panel naming the lyric sheet a run will use, with
//! Choose/Replace/Clear; the importer; and `--ui-probe assist=`.
//!
//! **Do not touch `process::assist`'s process-group handling.** There is a test
//! that fails loudly with the reason if anyone simplifies it: `os.setsid()` in
//! the helper fails with `EPERM` if the caller is already a group leader, so
//! giving the child its own group from the parent kills it at startup.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};

impl Shell {
    /// The Assist panel, in the timeline strip's place.
    ///
    /// **Agent J fills this.**
    pub(crate) fn assist_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let _ = commands;
        super::stub(
            d,
            input.fonts.ui(),
            content,
            strip,
            "ASSIST",
            "Confirmation step names the lyric sheet a run will use, with Choose/Replace/Clear.",
        );
    }
}
