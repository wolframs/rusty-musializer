//! The export panel and the render transport.
//!
//! **Owner: Agent H.** C sources: `render_export.c`, `ffmpeg_posix.c`,
//! `plug.c:7132`, `:7157-7160`.
//!
//! What already exists underneath: `runtime::process::ffmpeg` (with its
//! `ETXTBSY` test lock), `core::timing::render_export` for the frame arithmetic,
//! `runtime::process::publish` for the transactional move into place, and
//! `WorkspaceFrame::export_timeline_height` for the taller strip this panel asks
//! for. Missing: the panel, the save dialog, progress and cancel, and the
//! fast-forward window (`render_start_frame`) that keeps a windowed export
//! bit-identical to the same frames of a full render.
//!
//! Definition of done includes encoding a short synthetic fixture in
//! `tools/headless_check.sh` and asserting the frame count **and** that the same
//! export twice is byte-identical.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};

impl Shell {
    /// The export panel, in the timeline strip's place.
    ///
    /// **Agent H fills this.**
    pub(crate) fn export_panel(
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
            "EXPORT",
            "Resolution, frame rate and quality, then FFmpeg supervision and transactional publication.",
        );
    }
}
