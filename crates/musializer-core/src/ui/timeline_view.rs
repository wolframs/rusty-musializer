//! Timeline view state over the layout.
//!
//! **Owner: Agent F.** Port of `timeline_view.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! A zoomable, pannable window onto a track. The timeline strip used to map the
//! whole track across the waveform width, which makes a two-second lyric cue in
//! a four-minute song about 15 px wide — too small to grab, let alone to place a
//! boundary in (`timeline_view.h:6-11`). Every seconds↔pixel conversion in the
//! strip goes through this model so the waveform, the tick labels, the event
//! markers, the lyric blocks, the playhead and the scrubber cannot disagree
//! about where a moment is.
//!
//! The model holds no pixels: callers pass the destination rectangle's left edge
//! and width per call (`timeline_view.h:13-15`). That keeps it testable
//! headlessly and keeps a resized window from invalidating stored state.
//!
//! Everything here is `f64`, matching the C's `double`. Narrowing to `f32` would
//! lose the sub-millisecond resolution the span floor is chosen to preserve.
//!
//! Every `isfinite` guard in the C is reproduced literally. They are not
//! defensive noise: a NaN that reaches a pixel conversion poisons every
//! subsequent frame, and the C tests assert the recovery behaviour.

/// Smallest window the view will zoom to (`timeline_view.h:17-21`).
///
/// A 0.25 s span across a 900 px strip is about 0.28 ms per pixel, already finer
/// than the millisecond the lyric bridge format round-trips, so further zoom
/// would buy precision the format cannot store. The Rust waveform now retains a
/// larger envelope than the oracle so this floor remains useful rather than
/// stretching a 2,048-bin whole-track preview across the lane.
pub const TIMELINE_VIEW_MIN_SPAN_SECONDS: f64 = 0.25;

/// Fraction of the visible span kept between the playhead and the edge when the
/// view follows playback (`timeline_view.h:23-25`). Zero would re-centre on
/// every frame at the boundary.
pub const TIMELINE_VIEW_REVEAL_MARGIN: f64 = 0.12;

/// A zoomable, pannable window onto a track (`timeline_view.h:27-30`).
///
/// Every mutator re-establishes the same invariant before returning
/// (`timeline_view.h:32-35`):
///
/// - `start_seconds >= 0`
/// - `span_seconds` in `[min(TIMELINE_VIEW_MIN_SPAN_SECONDS, duration), duration]`
/// - `start_seconds + span_seconds <= duration`
///
/// A non-finite or non-positive duration collapses the view to zero, which is
/// the state callers already treat as "there is no timeline to draw".
///
/// The C's `view == NULL` guards are gone with the pointer; every other guard is
/// kept.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimelineView {
    pub start_seconds: f64,
    pub span_seconds: f64,
}

/// `timeline_view.c:6-9`. A duration that cannot bound a window means there is
/// no timeline.
fn usable_duration(duration_seconds: f64) -> bool {
    duration_seconds.is_finite() && duration_seconds > 0.0
}

/// `timeline_view.c:11-17`. A track shorter than the floor is shown whole; the
/// floor must never exceed the material or the window would extend past the end.
fn minimum_span(duration_seconds: f64) -> f64 {
    if duration_seconds < TIMELINE_VIEW_MIN_SPAN_SECONDS {
        duration_seconds
    } else {
        TIMELINE_VIEW_MIN_SPAN_SECONDS
    }
}

impl TimelineView {
    /// A view showing the whole track, i.e. `reset` from scratch. This is the C
    /// test suite's `whole()` helper (`tests/test_timeline_view.c:12-17`), which
    /// is also how every caller starts.
    #[must_use]
    pub fn new(duration_seconds: f64) -> Self {
        let mut view = Self::default();
        view.reset(duration_seconds);
        view
    }

    /// `timeline_view.c:19-24`.
    pub fn reset(&mut self, duration_seconds: f64) {
        self.start_seconds = 0.0;
        self.span_seconds = if usable_duration(duration_seconds) {
            duration_seconds
        } else {
            0.0
        };
    }

    /// `timeline_view.c:26-49`. Re-establishes the struct invariant.
    pub fn clamp(&mut self, duration_seconds: f64) {
        if !usable_duration(duration_seconds) {
            self.start_seconds = 0.0;
            self.span_seconds = 0.0;
            return;
        }
        // Non-finite state can only arrive through hot reload of an older layout
        // or a caller that did its own arithmetic; recover to the whole track
        // rather than propagating NaN into every pixel conversion below.
        if !self.span_seconds.is_finite()
            || self.span_seconds <= 0.0
            || !self.start_seconds.is_finite()
        {
            self.reset(duration_seconds);
            return;
        }
        let floor_span = minimum_span(duration_seconds);
        if self.span_seconds < floor_span {
            self.span_seconds = floor_span;
        }
        if self.span_seconds > duration_seconds {
            self.span_seconds = duration_seconds;
        }
        if self.start_seconds < 0.0 {
            self.start_seconds = 0.0;
        }
        let mut last_start = duration_seconds - self.span_seconds;
        if last_start < 0.0 {
            last_start = 0.0;
        }
        if self.start_seconds > last_start {
            self.start_seconds = last_start;
        }
    }

    /// True when the view shows the whole track, i.e. zooming out further is
    /// inert (`timeline_view.c:51-56`).
    ///
    /// Note the direction of the degenerate cases: no usable duration and a
    /// non-finite span both answer *true*, because the caller uses this to grey
    /// out a zoom-out control and there is nothing to zoom out to.
    #[must_use]
    pub fn is_whole(&self, duration_seconds: f64) -> bool {
        if !usable_duration(duration_seconds) {
            return true;
        }
        if !self.span_seconds.is_finite() {
            return true;
        }
        self.span_seconds >= duration_seconds
    }

    /// `timeline_view.c:58-83`. `factor > 1` zooms in, `factor < 1` zooms out.
    ///
    /// `anchor_seconds` keeps its pixel position wherever it can: the moment
    /// under the cursor does not slide away, which is what makes wheel zoom feel
    /// attached to the pointer (`timeline_view.h:42-45`). The anchor is only
    /// violated when honouring it would push the window off either end.
    ///
    /// A non-finite or non-positive `factor` is a caller bug and is ignored. A
    /// non-finite *anchor* is not the same kind of mistake: it means "zoom on
    /// whatever is in the middle", which is the useful behaviour for a keyboard
    /// zoom with no pointer.
    pub fn zoom(&mut self, duration_seconds: f64, factor: f64, anchor_seconds: f64) {
        self.clamp(duration_seconds);
        if !usable_duration(duration_seconds) {
            return;
        }
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }

        let old_span = self.span_seconds;
        let mut span = old_span / factor;
        let floor_span = minimum_span(duration_seconds);
        if span < floor_span {
            span = floor_span;
        }
        if span > duration_seconds {
            span = duration_seconds;
        }

        // Hold the anchor's fractional position in the window fixed. An anchor
        // outside the current window (a keyboard zoom against a playhead that
        // has scrolled away) still works: the fraction falls outside [0,1] and
        // the clamp below pulls the result back to a legal window.
        let anchor_seconds = if anchor_seconds.is_finite() {
            anchor_seconds
        } else {
            self.start_seconds + old_span * 0.5
        };
        let mut fraction = if old_span > 0.0 {
            (anchor_seconds - self.start_seconds) / old_span
        } else {
            0.5
        };
        if !fraction.is_finite() {
            fraction = 0.5;
        }
        self.start_seconds = anchor_seconds - fraction * span;
        self.span_seconds = span;
        self.clamp(duration_seconds);
    }

    /// `timeline_view.c:85-94`. Panning never changes the span: a clamp written
    /// as "shrink the span to fit" would keep the invariant and silently zoom
    /// (`tests/test_timeline_view.c:121-123`).
    pub fn pan(&mut self, duration_seconds: f64, delta_seconds: f64) {
        self.clamp(duration_seconds);
        if !usable_duration(duration_seconds) {
            return;
        }
        if !delta_seconds.is_finite() {
            return;
        }
        self.start_seconds += delta_seconds;
        self.clamp(duration_seconds);
    }

    /// Scrolls by the least amount that brings `seconds` inside the window with
    /// [`TIMELINE_VIEW_REVEAL_MARGIN`] of the span to spare (`timeline_view.c:96-112`).
    ///
    /// A moment already comfortably inside the window leaves the view untouched,
    /// so this is safe to call per frame while following playback.
    pub fn reveal(&mut self, duration_seconds: f64, seconds: f64) {
        self.clamp(duration_seconds);
        if !usable_duration(duration_seconds) {
            return;
        }
        if !seconds.is_finite() {
            return;
        }
        let margin = self.span_seconds * TIMELINE_VIEW_REVEAL_MARGIN;
        let low = self.start_seconds + margin;
        let high = self.start_seconds + self.span_seconds - margin;
        if seconds < low {
            self.start_seconds = seconds - margin;
        } else if seconds > high {
            self.start_seconds = seconds - self.span_seconds + margin;
        }
        self.clamp(duration_seconds);
    }

    /// Maps a moment onto the strip (`timeline_view.c:114-121`).
    ///
    /// The result is intentionally allowed to fall outside `[left, left + width]`
    /// (`timeline_view.h:58-61`): a caller drawing a cue block needs the real
    /// off-screen edge to clip against. Clamping here would draw every earlier
    /// cue as though it started exactly at the left edge, which is how a scrolled
    /// lane starts lying about cue boundaries.
    ///
    /// Degenerate inputs return `left`, so a caller that draws before it checks
    /// gets the strip origin rather than a NaN.
    #[must_use]
    pub fn x_at(&self, seconds: f64, left: f64, width: f64) -> f64 {
        if !seconds.is_finite() || !left.is_finite() || !width.is_finite() || width <= 0.0 {
            return left;
        }
        if !self.span_seconds.is_finite() || self.span_seconds <= 0.0 {
            return left;
        }
        left + (seconds - self.start_seconds) / self.span_seconds * width
    }

    /// Inverse of [`Self::x_at`], clamped to `[0, duration]` because the result
    /// is always used as a transport position or a cue boundary
    /// (`timeline_view.c:123-138`).
    ///
    /// Dragging a cue boundary off the left of the strip must yield 0, not a
    /// negative start that the lyrics model would reject with no visible reason.
    #[must_use]
    pub fn seconds_at(&self, x: f64, left: f64, width: f64, duration_seconds: f64) -> f64 {
        if !usable_duration(duration_seconds) {
            return 0.0;
        }
        if !x.is_finite() || !left.is_finite() || !width.is_finite() || width <= 0.0 {
            return self.start_seconds;
        }
        if !self.span_seconds.is_finite() || self.span_seconds <= 0.0 {
            return self.start_seconds;
        }
        let seconds = self.start_seconds + (x - left) / width * self.span_seconds;
        if !seconds.is_finite() || seconds < 0.0 {
            return 0.0;
        }
        if seconds > duration_seconds {
            return duration_seconds;
        }
        seconds
    }

    /// The finest interval a one-pixel pointer movement can express
    /// (`timeline_view.c:140-145`). Callers use it to decide whether a drag is
    /// worth committing and to size an edge grab region.
    #[must_use]
    pub fn seconds_per_pixel(&self, width: f64) -> f64 {
        if !width.is_finite() || width <= 0.0 {
            return 0.0;
        }
        if !self.span_seconds.is_finite() || self.span_seconds <= 0.0 {
            return 0.0;
        }
        self.span_seconds / width
    }
}

/// Spacing between labelled gridlines for a given visible span, chosen from a
/// fixed ladder of readable intervals (`timeline_view.c:147-164`).
///
/// The defect this replaces: the strip picked the step from the *track length*,
/// so zooming into a four-minute song kept the 30 s spacing and left a window
/// with no labelled gridline in it at all (`timeline_view.h:76-78`).
#[must_use]
pub fn tick_step(span_seconds: f64) -> f64 {
    // Intervals a listener reads without arithmetic: fractions of a second, then
    // seconds, then the bar-ish 15/30/60 groupings, then minutes.
    #[rustfmt::skip]
    const LADDER: [f64; 14] = [
        0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0,
        15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
    ];
    if !span_seconds.is_finite() || span_seconds <= 0.0 {
        return LADDER[0];
    }
    // Around eight divisions: enough to locate a moment, few enough that the
    // labels do not collide at the narrowest supported strip.
    let target = span_seconds / 8.0;
    for step in LADDER {
        if step >= target {
            return step;
        }
    }
    LADDER[LADDER.len() - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from `../musializer/tests/test_timeline_view.c`. The demo fixture is
    // about 40 s; a real song is nearer 240 s. Both shapes are exercised because
    // the interesting clamps are relative to duration.
    const TRACK: f64 = 240.0;
    const STRIP_LEFT: f64 = 12.0;
    const STRIP_WIDTH: f64 = 1200.0;

    /// The C's `EXPECT_NEAR`, which also fails on a non-finite actual value
    /// (`tests/test_support.h:63-69`).
    #[track_caller]
    fn expect_near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= tolerance,
            "actual={actual} expected={expected} tolerance={tolerance}"
        );
    }

    fn whole() -> TimelineView {
        TimelineView::new(TRACK)
    }

    /// The invariant every mutator promises (`tests/test_timeline_view.c:22-28`).
    /// Asserting it after each operation is what makes the clamping testable at
    /// all: the individual numbers below are examples, this is the contract.
    #[track_caller]
    fn expect_legal(view: &TimelineView, duration: f64) {
        assert!(view.start_seconds.is_finite() && view.start_seconds >= 0.0);
        assert!(view.span_seconds.is_finite() && view.span_seconds > 0.0);
        assert!(view.span_seconds <= duration + 1e-9);
        assert!(view.start_seconds + view.span_seconds <= duration + 1e-9);
    }

    #[test]
    fn timeline_view_starts_showing_the_whole_track() {
        let view = whole();
        expect_near(view.start_seconds, 0.0, 1e-9);
        expect_near(view.span_seconds, TRACK, 1e-9);
        assert!(view.is_whole(TRACK));
        // The unzoomed mapping must be exactly the arithmetic it replaced, or
        // every capture in tools/ui_states.txt shifts by a pixel.
        expect_near(view.x_at(0.0, STRIP_LEFT, STRIP_WIDTH), STRIP_LEFT, 1e-9);
        expect_near(
            view.x_at(TRACK, STRIP_LEFT, STRIP_WIDTH),
            STRIP_LEFT + STRIP_WIDTH,
            1e-9,
        );
        expect_near(
            view.x_at(60.0, STRIP_LEFT, STRIP_WIDTH),
            STRIP_LEFT + STRIP_WIDTH * 0.25,
            1e-9,
        );
    }

    #[test]
    fn timeline_view_zoom_keeps_the_anchor_under_the_pointer() {
        let mut view = whole();
        let anchor = 90.0;
        let before = view.x_at(anchor, STRIP_LEFT, STRIP_WIDTH);
        view.zoom(TRACK, 4.0, anchor);
        expect_legal(&view, TRACK);
        expect_near(view.span_seconds, 60.0, 1e-9);
        // The whole point of anchored zoom: the moment the cursor was over does
        // not move. A centred zoom would put it back at the middle of the strip.
        expect_near(view.x_at(anchor, STRIP_LEFT, STRIP_WIDTH), before, 1e-6);
        // ... and it is genuinely not the centre, which is what makes the check
        // above non-vacuous.
        assert!((before - (STRIP_LEFT + STRIP_WIDTH * 0.5)).abs() > 100.0);
    }

    #[test]
    fn timeline_view_zoom_at_the_ends_sacrifices_the_anchor_not_the_window() {
        let mut view = whole();
        // Anchoring on the very first moment would want start = 0 - 0*span,
        // which is legal; anchoring near the tail wants a window running past
        // the end.
        view.zoom(TRACK, 8.0, TRACK);
        expect_legal(&view, TRACK);
        expect_near(view.span_seconds, 30.0, 1e-9);
        expect_near(view.start_seconds, TRACK - 30.0, 1e-9);

        let mut head = whole();
        head.zoom(TRACK, 8.0, 0.0);
        expect_legal(&head, TRACK);
        expect_near(head.start_seconds, 0.0, 1e-9);
    }

    #[test]
    fn timeline_view_will_not_zoom_past_the_span_floor() {
        let mut view = whole();
        for _ in 0..40 {
            view.zoom(TRACK, 2.0, 120.0);
            expect_legal(&view, TRACK);
        }
        expect_near(view.span_seconds, TIMELINE_VIEW_MIN_SPAN_SECONDS, 1e-9);
        // At the floor a pixel is still coarser than nothing: the drag
        // resolution the UI advertises has to be a real number, not zero.
        let resolution = view.seconds_per_pixel(STRIP_WIDTH);
        assert!(resolution > 0.0 && resolution < 0.001);
    }

    #[test]
    fn timeline_view_zooming_out_always_returns_to_the_whole_track() {
        let mut view = whole();
        view.zoom(TRACK, 64.0, 200.0);
        assert!(!view.is_whole(TRACK));
        for _ in 0..40 {
            view.zoom(TRACK, 0.5, 200.0);
            expect_legal(&view, TRACK);
        }
        assert!(view.is_whole(TRACK));
        expect_near(view.start_seconds, 0.0, 1e-9);
        expect_near(view.span_seconds, TRACK, 1e-9);
    }

    #[test]
    fn timeline_view_pan_stops_at_both_ends() {
        let mut view = whole();
        view.zoom(TRACK, 4.0, 120.0);
        let span = view.span_seconds;

        view.pan(TRACK, -1000.0);
        expect_legal(&view, TRACK);
        expect_near(view.start_seconds, 0.0, 1e-9);
        expect_near(view.span_seconds, span, 1e-9);

        view.pan(TRACK, 1000.0);
        expect_legal(&view, TRACK);
        expect_near(view.start_seconds, TRACK - span, 1e-9);
        // Panning must never change how much you can see. A clamp written as
        // "shrink the span to fit" would pass the legality check and silently
        // zoom.
        expect_near(view.span_seconds, span, 1e-9);
    }

    #[test]
    fn timeline_view_reveal_only_moves_when_the_moment_is_outside() {
        let mut view = whole();
        view.zoom(TRACK, 4.0, 120.0);
        let before = view;

        // Dead centre: already visible with margin to spare, so nothing happens.
        view.reveal(TRACK, view.start_seconds + view.span_seconds * 0.5);
        expect_near(view.start_seconds, before.start_seconds, 1e-9);

        // Past the right edge: scrolls just enough to bring it in with a margin.
        let target = before.start_seconds + before.span_seconds + 5.0;
        view.reveal(TRACK, target);
        expect_legal(&view, TRACK);
        assert!(target >= view.start_seconds && target <= view.start_seconds + view.span_seconds);
        expect_near(
            view.start_seconds,
            target - view.span_seconds + view.span_seconds * TIMELINE_VIEW_REVEAL_MARGIN,
            1e-6,
        );

        // Behind the left edge, symmetric.
        view.reveal(TRACK, 3.0);
        expect_legal(&view, TRACK);
        assert!(3.0 >= view.start_seconds && 3.0 <= view.start_seconds + view.span_seconds);
    }

    #[test]
    fn timeline_view_round_trips_pixels_and_seconds() {
        let mut view = whole();
        view.zoom(TRACK, 12.0, 77.0);
        let mut x = STRIP_LEFT;
        while x <= STRIP_LEFT + STRIP_WIDTH {
            let seconds = view.seconds_at(x, STRIP_LEFT, STRIP_WIDTH, TRACK);
            let back = view.x_at(seconds, STRIP_LEFT, STRIP_WIDTH);
            expect_near(back, x, 1e-6);
            x += 37.0;
        }
    }

    #[test]
    fn timeline_view_seconds_at_stays_inside_the_track() {
        let mut view = whole();
        view.zoom(TRACK, 6.0, 20.0);
        // Dragging a cue boundary off the left of the strip must yield 0, not a
        // negative start that lyrics_update would reject with no visible reason.
        expect_near(
            view.seconds_at(STRIP_LEFT - 4000.0, STRIP_LEFT, STRIP_WIDTH, TRACK),
            0.0,
            1e-9,
        );
        expect_near(
            view.seconds_at(STRIP_LEFT + 9000.0, STRIP_LEFT, STRIP_WIDTH, TRACK),
            TRACK,
            1e-9,
        );
    }

    #[test]
    fn timeline_view_x_at_reports_offscreen_edges_honestly() {
        let mut view = whole();
        view.zoom(TRACK, 8.0, 120.0);
        // A cue that begins before the window must map to a negative offset so
        // the caller can clip it. Clamping here would draw every earlier cue as
        // though it started exactly at the left edge, which is how a scrolled
        // lane starts lying about cue boundaries.
        let x = view.x_at(0.0, STRIP_LEFT, STRIP_WIDTH);
        assert!(x < STRIP_LEFT - 1.0);
    }

    #[test]
    fn timeline_view_survives_a_track_shorter_than_the_span_floor() {
        let mut view = TimelineView::new(0.1);
        expect_legal(&view, 0.1);
        view.zoom(0.1, 100.0, 0.05);
        expect_legal(&view, 0.1);
        // The floor must yield to the material, otherwise the window extends
        // past the end of a very short track.
        expect_near(view.span_seconds, 0.1, 1e-9);
        assert!(view.is_whole(0.1));
    }

    #[test]
    fn timeline_view_rejects_degenerate_input_without_producing_nan() {
        let mut view = whole();
        view.zoom(TRACK, 4.0, 120.0);
        let before = view;

        view.zoom(TRACK, f64::NAN, 120.0);
        view.zoom(TRACK, 0.0, 120.0);
        view.zoom(TRACK, -2.0, 120.0);
        view.pan(TRACK, f64::NAN);
        view.reveal(TRACK, f64::INFINITY);
        expect_near(view.start_seconds, before.start_seconds, 1e-9);
        expect_near(view.span_seconds, before.span_seconds, 1e-9);

        // A NaN anchor is not the caller's fault the same way: it means "zoom on
        // whatever is in the middle", which is the useful behaviour for a
        // keyboard zoom with no pointer.
        view.zoom(TRACK, 2.0, f64::NAN);
        expect_legal(&view, TRACK);
        expect_near(view.span_seconds, before.span_seconds * 0.5, 1e-9);

        // A corrupt view recovers to the whole track instead of poisoning pixels.
        let mut broken = TimelineView {
            start_seconds: f64::NAN,
            span_seconds: f64::NAN,
        };
        broken.clamp(TRACK);
        expect_legal(&broken, TRACK);
        assert!(broken.is_whole(TRACK));
    }

    #[test]
    fn timeline_view_tick_step_keeps_labels_inside_every_window() {
        // The defect this replaces: the step was chosen from the track length,
        // so a 240 s song kept 30 s spacing at every zoom and a 4 s window
        // contained no labelled gridline at all.
        const SPANS: [f64; 10] = [0.25, 0.5, 1.0, 4.0, 12.0, 40.0, 90.0, 240.0, 600.0, 3600.0];
        for span in SPANS {
            let step = tick_step(span);
            assert!(step > 0.0 && step.is_finite());
            // At least two divisions, so there is always a labelled reference in
            // view, and no more than about forty, so they do not collide.
            assert!(span / step >= 2.0);
            assert!(span / step <= 40.0);
        }
        // Monotone: zooming in never widens the spacing.
        let mut previous = tick_step(0.25);
        let mut span = 0.25;
        while span < 4000.0 {
            let step = tick_step(span);
            assert!(step >= previous);
            previous = step;
            span *= 1.3;
        }
        expect_near(tick_step(-1.0), 0.05, 1e-9);
        expect_near(tick_step(f64::NAN), 0.05, 1e-9);
    }

    #[test]
    fn timeline_view_treats_a_missing_track_as_no_timeline() {
        let view = TimelineView::new(0.0);
        expect_near(view.span_seconds, 0.0, 1e-9);
        assert!(view.is_whole(0.0));
        expect_near(view.seconds_per_pixel(STRIP_WIDTH), 0.0, 1e-9);
        // Callers draw before they check; these must not divide by zero.
        expect_near(view.x_at(5.0, STRIP_LEFT, STRIP_WIDTH), STRIP_LEFT, 1e-9);
        expect_near(
            view.seconds_at(900.0, STRIP_LEFT, STRIP_WIDTH, 0.0),
            0.0,
            1e-9,
        );
    }
}
