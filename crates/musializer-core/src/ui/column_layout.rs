//! The right column's geometry (WX-5d).
//!
//! **Owner: the Frame builder.** Raylib-free, so the width rule — one width for
//! every view, `clamp(round(0.28 × window), 320, 460)` and never less than a
//! 640 px workspace, a user drag clamped to 300–560 — is tested here and
//! pinned post-legacy by `layout_dump`, not photographed.
//!
//! One width for every view is the point of the rule: switching from Lyrics
//! to Assist changes what the column says and nothing else, so the preview,
//! the transport and the band keep every pixel (§0).

/// The share of the window an undragged column takes.
pub const COLUMN_SHARE: f32 = 0.28;
/// An undragged column's floor and ceiling.
pub const COLUMN_AUTO_MIN: f32 = 320.0;
pub const COLUMN_AUTO_MAX: f32 = 460.0;
/// A dragged column's floor and ceiling: a little wider either way than the
/// automatic rule, so a drag can still do something at both ends.
pub const COLUMN_DRAG_MIN: f32 = 300.0;
pub const COLUMN_DRAG_MAX: f32 = 560.0;
/// What the workspace — rail, preview, transport, band — always keeps. At the
/// 960 px minimum window this is what makes the column 320.
pub const COLUMN_WORKSPACE_MIN: f32 = 640.0;

/// The column's width in a window `window_width` wide, from the user's
/// dragged width when there is one.
///
/// A non-finite drag is no drag. The workspace floor wins over every other
/// bound, so in a window narrower than 960 the column gives way rather than
/// the preview; it never goes negative.
#[must_use]
pub fn column_width(window_width: f32, dragged: Option<f32>) -> f32 {
    if !window_width.is_finite() {
        return 0.0;
    }
    let width = match dragged.filter(|value| value.is_finite()) {
        Some(value) => value.clamp(COLUMN_DRAG_MIN, COLUMN_DRAG_MAX),
        None => (COLUMN_SHARE * window_width)
            .round()
            .clamp(COLUMN_AUTO_MIN, COLUMN_AUTO_MAX),
    };
    width.min(window_width - COLUMN_WORKSPACE_MIN).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's table (§1): the 1920, 1280 and 960 rows were measured
    /// from captures, 1707 is 2560x1440 at 150 %.
    #[test]
    fn the_automatic_width_is_the_designs_table() {
        for (window, width) in [
            (960.0, 320.0),
            (1280.0, 358.0),
            (1600.0, 448.0),
            (1707.0, 460.0),
            (1920.0, 460.0),
            (2560.0, 460.0),
        ] {
            assert_eq!(column_width(window, None), width, "{window}");
        }
    }

    #[test]
    fn a_drag_is_clamped_and_still_leaves_the_workspace_its_floor() {
        assert_eq!(column_width(1280.0, Some(500.0)), 500.0);
        assert_eq!(column_width(1920.0, Some(9000.0)), COLUMN_DRAG_MAX);
        assert_eq!(column_width(1920.0, Some(10.0)), COLUMN_DRAG_MIN);
        // 960 - 640: the drag cannot take the preview's floor.
        assert_eq!(column_width(960.0, Some(560.0)), 320.0);
        assert_eq!(column_width(1280.0, Some(f32::NAN)), 358.0);
    }

    #[test]
    fn a_window_under_the_minimum_gives_the_column_up_first() {
        assert_eq!(column_width(800.0, None), 160.0);
        assert_eq!(column_width(600.0, None), 0.0);
        assert_eq!(column_width(f32::NAN, None), 0.0);
    }
}
