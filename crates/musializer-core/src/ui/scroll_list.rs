//! Vertical scrolling-list policy: geometry, momentum, and the scrollbar.
//!
//! Port of the tracks panel's list half (`../musializer/src/plug.c:5213-5382`).
//! Nothing here draws; it answers where each row is, where the thumb is, and what
//! a wheel notch or a drag does to the offset.
//!
//! **This is shared, not the tracks panel's.** The C repeats this shape for the
//! tuning inspector (`p->scene_settings_scroll`), the lyrics list and the font
//! browser, and the rewrite's fan-out agreement is that duplication is a merge
//! cost paid up front rather than three times over. A panel supplies its own
//! `item_size`; everything else follows.
//!
//! # Two things about the oracle's momentum that look like bugs and are not
//!
//! - **Damping is per frame, not per second** (`plug.c:5219`): `velocity *= 0.9`
//!   runs once per `plug_update`, so a 60 fps session decays twice as fast as a
//!   30 fps one. Reproduced rather than corrected, because the alternative is a
//!   list that scrolls a visibly different distance per notch than the oracle's
//!   for the same gesture, and this is a feel constant rather than a measurement.
//! - **The offset is clamped after integration, never the velocity**
//!   (`plug.c:5233-5237`). Momentum keeps running against a clamped edge and
//!   decays there, so a fling into the end stops dead instead of bouncing.

/// `panel_boundary.width*0.2` — one row's full stride, padding included
/// (`plug.c:5215`).
#[must_use]
pub fn item_size(panel_width: f32) -> f32 {
    panel_width * 0.2
}

/// `item_size*0.1` (`plug.c:5239`). Inset on all four sides of a row, which is
/// why a row is `item_size - padding*2` tall and the stride is still `item_size`.
#[must_use]
pub fn item_padding(item_size: f32) -> f32 {
    item_size * 0.1
}

/// `panel_boundary.width*0.03` (`plug.c:5214`).
///
/// Reserved from every row's width whether or not the bar is drawn, so rows do
/// not change width when the list grows past the visible area.
#[must_use]
pub fn bar_width(panel_width: f32) -> f32 {
    panel_width * 0.03
}

/// A list's measured geometry for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListMetrics {
    pub item_size: f32,
    pub padding: f32,
    pub bar_width: f32,
    /// `fmaxf(1.0f, panel_height - header_height)` (`plug.c:5216`). The floor of
    /// 1 is the oracle's, and it is what keeps `visible/scrollable` finite when a
    /// panel is squeezed to nothing.
    pub visible: f32,
    /// `item_size*count` (`plug.c:5217`).
    pub scrollable: f32,
    /// Clamped at zero, so a list shorter than its panel cannot scroll
    /// (`plug.c:5235-5236`).
    pub max_offset: f32,
}

impl ListMetrics {
    #[must_use]
    pub fn measure(panel_width: f32, panel_height: f32, header_height: f32, count: usize) -> Self {
        let item_size = item_size(panel_width);
        let visible = (panel_height - header_height).max(1.0);
        let scrollable = item_size * count as f32;
        Self {
            item_size,
            padding: item_padding(item_size),
            bar_width: bar_width(panel_width),
            visible,
            scrollable,
            max_offset: (scrollable - visible).max(0.0),
        }
    }

    /// Whether the bar is drawn at all (`plug.c:5342`).
    #[must_use]
    pub fn needs_bar(&self) -> bool {
        self.scrollable > self.visible
    }

    /// The row rectangle for `index`, as `(y, height)` offsets from the top of
    /// the list area (`plug.c:5244-5249`).
    ///
    /// The `y` may be negative or past `visible`: a row scrolled out of view still
    /// has a position, and the caller clips. Returning `None` for an off-screen
    /// row would make the caller's loop depend on the clip, which is how a row
    /// that is invisible but still claims clicks gets written.
    #[must_use]
    pub fn row_offset(&self, index: usize, scroll: f32) -> (f32, f32) {
        (
            index as f32 * self.item_size + self.padding - scroll,
            self.item_size - self.padding * 2.0,
        )
    }

    /// A row's width inside a panel of `panel_width` (`plug.c:5247`).
    #[must_use]
    pub fn row_width(&self, panel_width: f32) -> f32 {
        panel_width - self.padding * 2.0 - self.bar_width
    }

    /// The thumb as `(y, height)` offsets from the top of the list area
    /// (`plug.c:5343-5359`).
    ///
    /// `None` when the list fits. The height is *not* floored at a minimum, which
    /// the oracle also does not do: a very long list gets a very small thumb.
    #[must_use]
    pub fn thumb(&self, scroll: f32) -> Option<(f32, f32)> {
        if !self.needs_bar() {
            return None;
        }
        let proportion = self.visible / self.scrollable;
        let position = scroll / self.scrollable;
        Some((self.visible * position, self.visible * proportion))
    }
}

/// Where a press on the scrollbar landed, relative to the thumb
/// (`plug.c:5366-5380`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHit {
    /// On the thumb: begin dragging.
    Thumb,
    /// In the gutter above the thumb: page towards the start.
    Above,
    /// In the gutter below the thumb: page towards the end.
    Below,
}

/// One list's scroll state across frames.
///
/// The C keeps `panel_scroll`, `panel_velocity`, `scrolling` and
/// `scrolling_mouse_offset` as **function statics**, which is why the oracle can
/// only ever have one scrolling list: a second panel using that code would share
/// the first one's position. Here every list owns its own, which is the divergence
/// that lets the same policy serve the inspector and the browsers too.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    scroll: f32,
    velocity: f32,
    /// Distance from the thumb's top to the grab point, held for the duration of
    /// a drag (`scrolling_mouse_offset`, `plug.c:5370`).
    drag_grab: Option<f32>,
}

impl ScrollState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn offset(&self) -> f32 {
        self.scroll
    }

    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag_grab.is_some()
    }

    /// A wheel notch over the panel (`plug.c:5221-5223`).
    ///
    /// The `8` is the oracle's, and it is a multiplier on `item_size` rather than
    /// on a pixel constant, so the gesture moves the same number of *rows* in a
    /// narrow sidebar as in a wide one.
    pub fn wheel(&mut self, notches: f32, metrics: &ListMetrics) {
        self.velocity += notches * metrics.item_size * 8.0;
    }

    /// A click in the gutter (`plug.c:5372-5379`). The `16` is the oracle's, so a
    /// gutter click moves twice as far as a wheel notch.
    pub fn page(&mut self, hit: BarHit, metrics: &ListMetrics) {
        match hit {
            BarHit::Above => self.velocity += metrics.item_size * 16.0,
            BarHit::Below => self.velocity -= metrics.item_size * 16.0,
            BarHit::Thumb => {}
        }
    }

    /// Begins a thumb drag, remembering where on the thumb the press landed.
    pub fn begin_drag(&mut self, grab_offset: f32) {
        self.drag_grab = Some(grab_offset);
    }

    pub fn end_drag(&mut self) {
        self.drag_grab = None;
    }

    /// Advances one frame (`plug.c:5219-5237`).
    ///
    /// `pointer_offset` is the mouse's y relative to the top of the list area;
    /// it is only read while dragging. The order is the oracle's and it matters:
    /// damping, then integration, then — if dragging — the drag *overwrites* the
    /// integrated value rather than adding to it, and only then is the result
    /// clamped. A drag therefore beats momentum instead of fighting it.
    pub fn advance(&mut self, delta_seconds: f32, pointer_offset: f32, metrics: &ListMetrics) {
        self.velocity *= 0.9;
        self.scroll -= self.velocity * delta_seconds;
        if let Some(grab) = self.drag_grab {
            self.scroll = (pointer_offset - grab) / metrics.visible * metrics.scrollable;
        }
        self.scroll = self.scroll.clamp(0.0, metrics.max_offset);
    }

    /// Resets to the top, discarding momentum. What opening a panel does
    /// (`set_scene_settings_open`, `plug.c:5390-5393`).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(count: usize) -> ListMetrics {
        // A 300 px sidebar with a 26 px header and 200 px of list area.
        ListMetrics::measure(300.0, 226.0, 26.0, count)
    }

    #[test]
    fn the_geometry_constants_are_the_oracles_fractions() {
        let m = metrics(4);
        assert_eq!(m.item_size, 60.0); // 300 * 0.2
        assert_eq!(m.padding, 6.0); // 60 * 0.1
        assert_eq!(m.bar_width, 9.0); // 300 * 0.03
        assert_eq!(m.visible, 200.0);
        assert_eq!(m.scrollable, 240.0);
        assert_eq!(m.max_offset, 40.0);
        // The stride is item_size; the row is inset by the padding on each side.
        assert_eq!(m.row_offset(0, 0.0), (6.0, 48.0));
        assert_eq!(m.row_offset(1, 0.0), (66.0, 48.0));
        assert_eq!(m.row_width(300.0), 300.0 - 12.0 - 9.0);
    }

    #[test]
    fn a_list_that_fits_cannot_scroll_and_has_no_bar() {
        let m = metrics(2);
        assert_eq!(m.scrollable, 120.0);
        assert_eq!(m.max_offset, 0.0);
        assert!(!m.needs_bar());
        assert_eq!(m.thumb(0.0), None);

        let mut state = ScrollState::new();
        state.wheel(-5.0, &m);
        state.advance(1.0 / 60.0, 0.0, &m);
        assert_eq!(state.offset(), 0.0, "a short list must not drift");
    }

    #[test]
    fn a_squeezed_panel_keeps_a_finite_visible_height() {
        // `fmaxf(1.0, ...)`: without it, `visible/scrollable` divides by zero and
        // the thumb becomes NaN (plug.c:5216).
        let m = ListMetrics::measure(300.0, 26.0, 26.0, 10);
        assert_eq!(m.visible, 1.0);
        let (y, height) = m.thumb(0.0).expect("a long list has a thumb");
        assert!(y.is_finite() && height.is_finite());
    }

    #[test]
    fn the_thumb_shrinks_with_the_list_and_travels_with_the_offset() {
        let m = metrics(10);
        assert_eq!(m.scrollable, 600.0);
        let (top_y, height) = m.thumb(0.0).expect("scrollable");
        assert_eq!(top_y, 0.0);
        // visible/scrollable = 200/600, times the 200 px track.
        assert!((height - 200.0 / 3.0).abs() < 1e-4);
        let (bottom_y, _) = m.thumb(m.max_offset).expect("scrollable");
        assert!(bottom_y > top_y);
        // At the end stop the thumb's bottom edge lands exactly on the track's,
        // because `max_offset + visible == scrollable` makes the two fractions
        // sum to one. Worth pinning: it is what tells the user there is nothing
        // below, and an off-by-one in either fraction would leave a gap there.
        assert!((bottom_y + height - m.visible).abs() < 1e-4);
    }

    #[test]
    fn momentum_decays_and_stops_dead_at_the_end() {
        let m = metrics(10);
        // One notch is 8*item_size of velocity, and per-frame damping of 0.9
        // integrates that to `8*item_size*0.9/(1-0.9)/60` px — 72 here. A single
        // notch deliberately does not reach the end of a ten-row list.
        let mut state = ScrollState::new();
        state.wheel(-1.0, &m); // wheel down: negative velocity scrolls forwards
        for _ in 0..600 {
            state.advance(1.0 / 60.0, 0.0, &m);
        }
        assert!(
            (state.offset() - 72.0).abs() < 0.5,
            "got {}",
            state.offset()
        );

        // A fling large enough to overshoot rests against the end rather than
        // bouncing: the offset is clamped, the velocity is not, so the remaining
        // momentum decays harmlessly against the stop.
        let mut flung = ScrollState::new();
        flung.wheel(-20.0, &m);
        for _ in 0..600 {
            flung.advance(1.0 / 60.0, 0.0, &m);
            assert!(flung.offset() >= 0.0);
            assert!(flung.offset() <= m.max_offset);
        }
        assert_eq!(flung.offset(), m.max_offset);
    }

    #[test]
    fn a_drag_overrides_momentum_rather_than_adding_to_it() {
        let m = metrics(10);
        let mut state = ScrollState::new();
        state.wheel(-4.0, &m);
        state.begin_drag(0.0);
        assert!(state.is_dragging());
        // Grabbed at the very top of the thumb and dropped at the midpoint of the
        // track: the offset is the proportional position, not that plus a fling.
        state.advance(1.0 / 60.0, m.visible * 0.5, &m);
        let expected = (m.scrollable * 0.5).min(m.max_offset);
        assert!((state.offset() - expected).abs() < 1e-3);
        state.end_drag();
        assert!(!state.is_dragging());
    }

    #[test]
    fn a_gutter_click_pages_twice_as_far_as_a_wheel_notch() {
        let m = metrics(10);
        let mut wheeled = ScrollState::new();
        wheeled.wheel(-1.0, &m);
        let mut paged = ScrollState::new();
        paged.page(BarHit::Below, &m);
        wheeled.advance(1.0 / 60.0, 0.0, &m);
        paged.advance(1.0 / 60.0, 0.0, &m);
        assert!((paged.offset() - wheeled.offset() * 2.0).abs() < 1e-4);
    }

    #[test]
    fn reset_discards_position_and_momentum() {
        let m = metrics(10);
        let mut state = ScrollState::new();
        state.wheel(-3.0, &m);
        state.advance(1.0 / 60.0, 0.0, &m);
        assert!(state.offset() > 0.0);
        state.reset();
        assert_eq!(state, ScrollState::new());
    }
}
