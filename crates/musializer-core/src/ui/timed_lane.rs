//! Geometry shared by editable, time-aligned lanes.
//!
//! This module deliberately stops below editing policy. Lyrics may overlap and
//! resize either edge; scene plans are contiguous and move a shared boundary.
//! Both still need identical seconds-to-pixels conversion, minimum visible
//! widths and clipping, and those are the pieces collected here.

use super::timeline_view::TimelineView;

/// A block narrower than this is widened for drawing and hit testing.
pub const MIN_DRAWN_BLOCK_PIXELS: f64 = 3.0;

/// Pointer travel before a press becomes a drag rather than a selection.
pub const DRAG_THRESHOLD_PIXELS: f64 = 3.0;

/// The true and clipped horizontal extent of one timed span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedLaneBlock {
    /// Unclipped start boundary. It may be outside the lane.
    pub true_left: f64,
    /// Unclipped end boundary after applying the minimum drawn width.
    pub true_right: f64,
    /// Visible start, clipped to the lane.
    pub left: f64,
    /// Visible end, clipped to the lane.
    pub right: f64,
    /// Whether the semantic start boundary is actually visible.
    pub start_visible: bool,
    /// Whether the semantic end boundary is actually visible.
    pub end_visible: bool,
}

impl TimedLaneBlock {
    #[must_use]
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    #[must_use]
    pub fn true_width(self) -> f64 {
        self.true_right - self.true_left
    }

    #[must_use]
    pub fn contains_x(self, x: f64) -> bool {
        x.is_finite() && x >= self.true_left && x <= self.true_right
    }
}

/// Maps and clips one span. `None` means it has no drawable overlap with the
/// lane or one of the supplied values is unusable.
#[must_use]
pub fn block_geometry(
    view: &TimelineView,
    lane_x: f64,
    lane_width: f64,
    start_seconds: f64,
    end_seconds: f64,
) -> Option<TimedLaneBlock> {
    if !lane_x.is_finite()
        || !lane_width.is_finite()
        || lane_width <= 0.0
        || !start_seconds.is_finite()
        || !end_seconds.is_finite()
        || end_seconds <= start_seconds
    {
        return None;
    }

    let true_left = view.x_at(start_seconds, lane_x, lane_width);
    let mut true_right = view.x_at(end_seconds, lane_x, lane_width);
    if !true_left.is_finite() || !true_right.is_finite() {
        return None;
    }
    if true_right - true_left < MIN_DRAWN_BLOCK_PIXELS {
        true_right = true_left + MIN_DRAWN_BLOCK_PIXELS;
    }

    let lane_right = lane_x + lane_width;
    if true_right < lane_x || true_left > lane_right {
        return None;
    }
    let left = true_left.max(lane_x);
    let right = true_right.min(lane_right);
    if right - left < 1.0 {
        return None;
    }

    Some(TimedLaneBlock {
        true_left,
        true_right,
        left,
        right,
        start_visible: true_left >= lane_x,
        end_visible: true_right <= lane_right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_spans_have_the_same_drawn_and_hit_width() {
        let view = TimelineView::new(100.0);
        let block = block_geometry(&view, 10.0, 100.0, 50.0, 50.001).unwrap();
        assert_eq!(block.true_width(), MIN_DRAWN_BLOCK_PIXELS);
        assert!(block.contains_x(block.true_left + 2.5));
    }

    #[test]
    fn clipping_keeps_offscreen_boundaries_non_interactive() {
        let mut view = TimelineView::new(100.0);
        view.start_seconds = 25.0;
        view.span_seconds = 25.0;
        let block = block_geometry(&view, 100.0, 400.0, 20.0, 30.0).unwrap();
        assert_eq!(block.left, 100.0);
        assert!(!block.start_visible);
        assert!(block.end_visible);
    }

    #[test]
    fn fully_offscreen_and_degenerate_spans_are_refused() {
        let view = TimelineView::new(10.0);
        assert!(block_geometry(&view, 0.0, 100.0, 11.0, 12.0).is_none());
        assert!(block_geometry(&view, 0.0, 100.0, 4.0, 4.0).is_none());
        assert!(block_geometry(&view, 0.0, f64::NAN, 1.0, 2.0).is_none());
    }
}
