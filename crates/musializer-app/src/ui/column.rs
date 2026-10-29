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

use super::shell::Shell;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The panel's pre-WX-5d place: the band under the timeline, or for the
    /// Tune inspector its own right-hand column.
    Band,
    /// The right column's body.
    Column,
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

    /// The `column:` report line (§5): view, open state and the head, body
    /// and grip rects. Empty until the column is built, and an empty line is
    /// not printed.
    #[must_use]
    pub(crate) fn describe_column(&self, _workspace: &Workspace) -> String {
        String::new()
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
    fn the_switch_alone_decides_the_placement() {
        let mut shell =
            Shell::with_preferences(super::super::preferences::UiPreferences::default());
        assert_eq!(shell.placement(), Placement::Band);
        shell.column_layout = true;
        assert_eq!(shell.placement(), Placement::Column);
    }
}
