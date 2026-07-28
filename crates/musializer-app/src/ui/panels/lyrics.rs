//! The three-pane lyrics editor.
//!
//! **Owner: Agent I.** C sources: `lyrics_editor_ui.c`, `lyrics_editor_layout.c`,
//! `lyric_lane_edit.c` — all three already ported, along with `caption_layout`
//! and the caption face `runtime::font` loads.
//!
//! Missing: the editor itself, the cue lane, and **text entry**, which this
//! codebase has none of. That is the one piece of real UI engineering left in
//! the rewrite: there is no caret, no selection and no clipboard anywhere. It is
//! designed once, in `super::super::widgets`, by the integration owner — request
//! it rather than writing a second one here.
//!
//! Two panes nest inside this one. The caption-style pane is Agent I's;
//! [`super::fonts`]'s browser is Agent K's and is called from here.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};

impl Shell {
    /// The lyrics editor, in the timeline strip's place.
    ///
    /// **Agent I fills this.**
    pub(crate) fn lyrics_panel(
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
            "LYRICS",
            "Three-pane editor, cue lane and caption typography. Layout policy is ported; the panel is not.",
        );
    }
}
