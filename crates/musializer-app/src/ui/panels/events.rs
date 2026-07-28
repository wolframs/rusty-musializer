//! The manual event row, and the preset controls.
//!
//! **Owner: Agent L.** C sources: `plug.c:2834-2880`, `event_timeline.c`,
//! `semantic_lane.c`, `preset_store.c` — all the policy is ported, including
//! `core::scene::events`, the module whose header comment got the merge wrong in
//! seven ways. Trust the `.c`.
//!
//! Missing: the `+ Feel` / `+ Scene` / `+ Custom` row with `Clear manual`'s
//! confirm and undo states, and the preset UI.
//!
//! **Keep the oracle's honesty.** A manual marker carries one value, so the
//! semantic lane skips it and **only Constellation reacts**. The C's tooltip
//! says so; say it too, rather than implying every scene responds.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::RaylibDrawHandle;

use super::super::shell::{Shell, ShellCommand, ShellInput};

impl Shell {
    /// The manual event row, above the timeline.
    ///
    /// **Agent L fills this.** Returns the height it consumed so the timeline
    /// strip can lay itself out around it — the layout rule this repository has
    /// already paid for is that a row only some modes have is a parameter, never
    /// an assumption.
    // No call site yet: its caller is another agent's file, and wiring a call
    // into a stub that agent is about to replace would be churn. The table in
    // `super`'s module comment says who calls it.
    #[allow(dead_code)]
    pub(crate) fn event_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let _ = (d, input, area, commands);
        0.0
    }
}
