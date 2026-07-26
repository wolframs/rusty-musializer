//! How tall the lyric editor's panel asks to be, and what fits inside it.
//!
//! **Owner: Agent F.** Port of `lyrics_editor_layout.c/.h` from the frozen C
//! oracle at `../musializer` (commit `9300af9`, read-only).
//!
//! The lyric editor used to be given a fixed 172 px panel at every window size,
//! carved out of a timeline whose height was the constant 330. Its editing form
//! needs 223 px, so Apply, Discard and Delete were drawn 20 px below the panel and
//! 14 px below the bottom of the framebuffer — at 960x640, 1280x720 and 1920x1080
//! alike. The list showed three cues out of a 1024-cue capacity on a full-HD
//! display while half the window sat empty (`lyrics_editor_layout.h:10-18`).
//!
//! The panel now asks for the height its content needs, bounded so the sidebar
//! keeps a usable scene browser and tracks panel. Both layout rules this
//! repository already paid for show up here: the reserved height is derived from
//! what is actually drawn, and the floor it is measured against is what the
//! minimum supported window (960x640) really produces — at 640 the form wins and
//! the sidebar yields, because
//! [`WorkspaceSidebar::layout`](super::workspace_layout::WorkspaceSidebar::layout)
//! collapses the tracks panel cleanly rather than letting it overflow.

use super::workspace_layout::{WORKSPACE_SCENES_MINIMUM, WORKSPACE_TRACKS_SINGLE_MINIMUM};

/// One cue row (`lyrics_editor_layout.h:20-22`).
pub const LYRIC_EDITOR_ROW_HEIGHT: f32 = 36.0;
/// The chrome around the list: padding above and below, the "LYRIC CUES" header,
/// and the slack the row loop already assumes (`lyrics_editor_layout.h:20-23`).
pub const LYRIC_EDITOR_LIST_CHROME: f32 = 52.0;

/// The editing form's own extent: 10 px padding, then the header, the two time
/// rows, the text field, the action row at +146 with its 36 px height, and the
/// Ctrl+Enter hint below it, then 10 px padding
/// (`lyrics_editor_layout.h:25-28`). This is the number the shipped 172 px panel
/// was 51 px short of.
pub const LYRIC_EDITOR_FORM_MINIMUM: f32 = 223.0;

/// Rows worth growing for. Below four the form dominates anyway; above twelve the
/// panel would eat the preview for a list that scrolls perfectly well
/// (`lyrics_editor_layout.h:30-33`).
pub const LYRIC_EDITOR_MIN_ROWS: usize = 4;
pub const LYRIC_EDITOR_MAX_ROWS: usize = 12;

/// Controls, transport, waveform lane and their margins, above the panel. This is
/// the same constant `assist_timeline_height` documents, so the two cannot drift
/// (`lyrics_editor_layout.h:35-37`).
pub const LYRIC_EDITOR_TIMELINE_CHROME: f32 = 158.0;

/// Sidebar the panel refuses to squeeze below: the scene browser's content floor
/// plus a tracks panel that can still host its action row
/// (`lyrics_editor_layout.h:39-42`).
///
/// Kept as the sum of the two workspace constants rather than the number 301,
/// exactly as the C macro does: the floor is only meaningful as "what those two
/// panels need", and inlining it would let the three drift apart.
pub const LYRIC_EDITOR_SIDEBAR_MINIMUM: f32 =
    WORKSPACE_SCENES_MINIMUM + WORKSPACE_TRACKS_SINGLE_MINIMUM;

/// Panel height that would show every cue, clamped to the row bounds
/// (`lyrics_editor_layout.c:5-12`). Never below the form minimum, because a panel
/// too short for the form is the original defect.
// The two-step clamp is the oracle's (`lyrics_editor_layout.c:7-9`); `clamp`
// panics when max < min instead of reproducing it, so keep the ifs.
#[allow(clippy::manual_clamp)]
pub fn required_height(cue_count: usize) -> f32 {
    let mut rows = cue_count;
    if rows < LYRIC_EDITOR_MIN_ROWS {
        rows = LYRIC_EDITOR_MIN_ROWS;
    }
    if rows > LYRIC_EDITOR_MAX_ROWS {
        rows = LYRIC_EDITOR_MAX_ROWS;
    }
    let wanted = LYRIC_EDITOR_LIST_CHROME + rows as f32 * LYRIC_EDITOR_ROW_HEIGHT;
    if wanted < LYRIC_EDITOR_FORM_MINIMUM {
        LYRIC_EDITOR_FORM_MINIMUM
    } else {
        wanted
    }
}

/// Height to request for this window: what the content wants, reduced to protect
/// the sidebar, but never below the form minimum. Port of
/// `lyric_editor_panel_height` (`lyrics_editor_layout.c:14-28`).
///
/// The form is the point of the panel, so on the shortest windows the sidebar
/// yields rather than the form. A non-finite or non-positive window falls back to
/// the form minimum instead of returning an error: the panel still has to be drawn
/// somewhere, and the form is what must survive.
pub fn panel_height(screen_height: f32, cue_count: usize) -> f32 {
    if !screen_height.is_finite() || screen_height <= 0.0 {
        return LYRIC_EDITOR_FORM_MINIMUM;
    }
    let mut wanted = required_height(cue_count);
    let ceiling = screen_height - LYRIC_EDITOR_SIDEBAR_MINIMUM - LYRIC_EDITOR_TIMELINE_CHROME;
    if wanted > ceiling {
        wanted = ceiling;
    }
    // A window too short to give both is a window where the editing controls
    // matter more than the track list; WorkspaceSidebar::layout collapses the
    // tracks panel cleanly rather than letting it overflow.
    if wanted < LYRIC_EDITOR_FORM_MINIMUM {
        wanted = LYRIC_EDITOR_FORM_MINIMUM;
    }
    wanted
}

/// Cue rows a panel of this height can show (`lyrics_editor_layout.c:30-40`).
///
/// The drawing code derives the list from the panel the same way: the list loses
/// the panel's 20 px of vertical padding, then the 32 px header above the first
/// row. Both magic numbers are copied from the oracle deliberately — they are the
/// drawing code's, and changing one here silently desynchronises the two.
pub fn visible_rows(panel_height: f32) -> usize {
    if !panel_height.is_finite() {
        return 0;
    }
    let list_height = panel_height - 20.0;
    if list_height <= LYRIC_EDITOR_ROW_HEIGHT {
        return 0;
    }
    let rows = (list_height - 32.0) / LYRIC_EDITOR_ROW_HEIGHT;
    if rows <= 0.0 {
        return 0;
    }
    // C's `(size_t)rows`: truncation toward zero, and the guards above have already
    // excluded the non-finite and non-positive cases that would make the cast UB.
    rows as usize
}

/// Whether the editing form fits inside a panel of this height
/// (`lyrics_editor_layout.c:42-45`).
pub fn form_fits(panel_height: f32) -> bool {
    panel_height.is_finite() && panel_height >= LYRIC_EDITOR_FORM_MINIMUM
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C's `EXPECT_NEAR`, which also fails on a non-finite actual
    /// (`tests/test_support.h:63-70`).
    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= tolerance,
            "actual={actual} expected={expected} tolerance={tolerance}"
        );
    }

    /// The panel is drawn inside a timeline of `LYRIC_EDITOR_TIMELINE_CHROME` plus
    /// the panel, anchored to the bottom of the window. This mirrors the
    /// application's own arithmetic so the assertions below are about real screen
    /// positions (`tests/test_lyrics_editor_layout.c:6-15`).
    fn panel_bottom_overflow(screen_height: f32, cue_count: usize) -> f32 {
        let panel = panel_height(screen_height, cue_count);
        // Where the form's last control ends, measured from the panel top.
        let form_extent = LYRIC_EDITOR_FORM_MINIMUM;
        form_extent - panel
    }

    #[test]
    fn lyric_editor_form_never_extends_past_its_panel() {
        // This is the assertion that fails against the shipped 172 px panel at every
        // supported window size: Apply, Discard and Delete were drawn below it.
        let heights = [640.0f32, 720.0, 800.0, 900.0, 1080.0, 1440.0, 2160.0];
        let counts = [0usize, 1, 3, 4, 8, 12, 64, 1024];
        for height in heights {
            for count in counts {
                let panel = panel_height(height, count);
                assert!(form_fits(panel), "{height}x{count} panel {panel}");
                assert!(panel_bottom_overflow(height, count) <= 0.0);
            }
        }
    }

    #[test]
    fn lyric_editor_panel_grows_with_the_window_instead_of_staying_fixed() {
        // Eight cues, the fixture's count. The shipped panel showed three rows at
        // every size; taller windows must now show more.
        let at_640 = visible_rows(panel_height(640.0, 8));
        let at_720 = visible_rows(panel_height(720.0, 8));
        let at_1080 = visible_rows(panel_height(1080.0, 8));

        assert!(at_640 >= 4);
        assert!(at_720 > at_640);
        assert!(at_1080 > at_720);
        // A full-HD window shows the whole fixture rather than a third of it.
        assert_eq!(at_1080, 8);
    }

    #[test]
    fn lyric_editor_panel_protects_the_sidebar_where_it_can() {
        // At 720p and above the sidebar keeps its scene browser floor and a tracks
        // panel that can host its action row.
        let mut height = 720.0f32;
        while height <= 2160.0 {
            let panel = panel_height(height, 8);
            let sidebar = height - (panel + LYRIC_EDITOR_TIMELINE_CHROME);
            assert!(
                sidebar >= LYRIC_EDITOR_SIDEBAR_MINIMUM - 0.01,
                "at {height} the sidebar got {sidebar}"
            );
            height += 1.0;
        }
        // At the 640 minimum the form wins and the sidebar yields; the tracks panel
        // collapses cleanly instead of overflowing. See WorkspaceSidebar::layout.
        near(panel_height(640.0, 8), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
    }

    #[test]
    fn lyric_editor_required_height_clamps_the_row_count() {
        // Fewer than four cues still needs a panel that can host the form.
        near(required_height(0), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
        near(required_height(1), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
        near(required_height(4), LYRIC_EDITOR_FORM_MINIMUM, 0.001);

        let eight = required_height(8);
        near(
            eight,
            LYRIC_EDITOR_LIST_CHROME + 8.0 * LYRIC_EDITOR_ROW_HEIGHT,
            0.001,
        );

        // A thousand cues must not ask for a thousand rows of panel.
        near(
            required_height(1024),
            required_height(LYRIC_EDITOR_MAX_ROWS),
            0.001,
        );
    }

    #[test]
    fn lyric_editor_visible_rows_agrees_with_the_panel_it_is_given() {
        // Round trip: a panel sized for N rows must actually show N.
        for rows in LYRIC_EDITOR_MIN_ROWS..=LYRIC_EDITOR_MAX_ROWS {
            let panel = LYRIC_EDITOR_LIST_CHROME + rows as f32 * LYRIC_EDITOR_ROW_HEIGHT;
            assert_eq!(visible_rows(panel), rows);
        }
        assert_eq!(visible_rows(0.0), 0);
        assert_eq!(visible_rows(-100.0), 0);
        assert_eq!(visible_rows(f32::NAN), 0);
    }

    #[test]
    fn lyric_editor_layout_rejects_degenerate_windows() {
        near(panel_height(f32::NAN, 8), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
        near(panel_height(0.0, 8), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
        near(panel_height(-720.0, 8), LYRIC_EDITOR_FORM_MINIMUM, 0.001);
        assert!(!form_fits(f32::NAN));
        assert!(!form_fits(172.0)); // The shipped height.
    }
}
