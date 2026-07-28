//! The font browser pane, and the faces it adds.
//!
//! **Owner: Agent K.** C sources: `font_catalogue.c`, `font_import_state.c`,
//! `plug.c:373-426`, `:1547+`. `core::ui::font_catalogue`-side state,
//! `runtime::process::font_import` and `runtime::font` already exist.
//!
//! This is a **pane inside the lyrics editor**, not a panel of its own — the
//! oracle drives it with `p->lyric_editor.font_pane` (`plug.c:3786`), which is
//! why there is no `UiPanel` variant for it and why it is called from
//! [`super::lyrics`].
//!
//! Missing: the browser, the once-per-run network consent — **deliberately not
//! persisted**, because consent to contact a third party is asked once per run
//! rather than remembered until someone withdraws it — and then `runtime::font`
//! grows from two faces to four: Space Grotesk at the full caption set, and the
//! project's imported face keyed by path. `Faces::caption()` is the seam.
//!
//! One open question to settle here: Agent E's deliberate
//! `font_catalogue_parse` atomicity divergence.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};

impl Shell {
    /// The font browser, inside the lyrics editor.
    ///
    /// **Agent K fills this.** Called by [`super::lyrics`] when its font pane is
    /// showing, so the two never fight over the same rectangle.
    // No call site yet: its caller is another agent's file, and wiring a call
    // into a stub that agent is about to replace would be churn. The table in
    // `super`'s module comment says who calls it.
    #[allow(dead_code)]
    pub(crate) fn font_browser_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let _ = commands;
        // A pane, not a bottom panel, so it has no strip to sit under: it fills
        // the rectangle the lyrics editor hands it.
        super::stub(
            d,
            input.fonts.ui(),
            area,
            UiRect::new(area.x, area.y - 28.0, area.width, 0.0),
            "FONTS",
            "Browse and import a caption face, with its licence, once per run.",
        );
    }
}
