//! Workspace panel tiling and sidebar budgets.
//!
//! **Owner: Agent F.** Port of `workspace_layout.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! The sidebar splits one vertical budget between the tracks panel and the scene
//! browser. In C that split was three lines of inline arithmetic in `plug.c`, and
//! it could hand the tracks panel zero height while the panel drew and registered
//! its action buttons at a fixed offset anyway. The tracks panel is drawn first,
//! and `ui_widgets_button_with_id` awards a press to whichever widget claimed it
//! first, so those invisible buttons stole clicks aimed at the scene tiles painted
//! over them: choosing a scene ran Save, or opened a file dialog
//! (`workspace_layout.h:7-19`).
//!
//! The rule this module enforces is that **a panel which cannot contain its own
//! controls is not drawn at all** — and the action row it would draw is a
//! parameter ([`TracksPanelMode::action_row`]) rather than an assumption baked
//! into the drawing code, because only some modes have one. Staying raylib-free
//! is what lets the budget and the containment promise be asserted headlessly.

/// A raylib-free rectangle. Port of `Ui_Rect` (`workspace_layout.h:21-23`).
///
/// Kept in C's `f32` width deliberately: every expectation in the ported tests
/// was computed in C `float`, so widening to `f64` would change results.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl UiRect {
    /// The C tests' `rect()` helper, and the way the sibling layout modules build
    /// their rects.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// `ui_rect_is_finite` (`workspace_layout.c:6-10`).
    pub fn is_finite(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
    }

    /// `ui_rect_is_empty` (`workspace_layout.c:12-15`). Non-finite counts as
    /// empty, so a NaN rect can never be treated as a hit box.
    pub fn is_empty(&self) -> bool {
        !self.is_finite() || self.width <= 0.0 || self.height <= 0.0
    }

    /// Inclusive containment: an inner rect flush with an outer edge is contained
    /// (`workspace_layout.c:17-26`).
    ///
    /// An empty inner rect has no pixels to fall outside, but it also has no
    /// business being registered as a hit box, so it reports as *not* contained.
    /// That is the check the click hijack needed: a 36 px action row at +50 inside
    /// a zero-height panel is not contained, so the panel must not draw it.
    pub fn contains(&self, inner: UiRect) -> bool {
        if !self.is_finite() || !inner.is_finite() {
            return false;
        }
        if inner.is_empty() || self.is_empty() {
            return false;
        }
        inner.x >= self.x
            && inner.y >= self.y
            && inner.x + inner.width <= self.x + self.width
            && inner.y + inner.height <= self.y + self.height
    }

    /// Touching edges do not overlap; only a positive-area intersection does
    /// (`workspace_layout.c:28-33`). Stacked panels share an edge by design, so a
    /// shared edge must not read as a collision.
    pub fn overlaps(&self, other: UiRect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    /// Inclusive point containment — `CheckCollisionPointRec`'s rule, which is
    /// what every hit test in `plug.c` goes through.
    ///
    /// Separate from [`UiRect::contains`] because that one deliberately refuses
    /// an empty inner rect, so a point expressed as a zero-size rect always
    /// reports as outside. Two callers have already reached for it; it belongs
    /// here rather than being written a third time.
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        if self.is_empty() || !x.is_finite() || !y.is_finite() {
            return false;
        }
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    /// `ui_rect_intersect` (`workspace_layout.c:35-45`). Returns the all-zero rect
    /// when the two do not overlap, which [`UiRect::is_empty`] reports as empty.
    pub fn intersect(&self, other: UiRect) -> UiRect {
        if !self.overlaps(other) {
            return UiRect::new(0.0, 0.0, 0.0, 0.0);
        }
        let x = if self.x > other.x { self.x } else { other.x };
        let y = if self.y > other.y { self.y } else { other.y };
        let right = if self.x + self.width < other.x + other.width {
            self.x + self.width
        } else {
            other.x + other.width
        };
        let bottom = if self.y + self.height < other.y + other.height {
            self.y + self.height
        } else {
            other.y + other.height
        };
        UiRect::new(x, y, right - x, bottom - y)
    }

    /// The largest rect of the given width-over-height ratio that fits inside
    /// this one, centred (EX2).
    ///
    /// **Not the oracle's**, because the oracle has no aspect to fit to: its four
    /// export presets are all 16:9 and the preview panel simply *was* the frame.
    /// Once an export can be 9:16 the preview stops being a picture of the
    /// output, and a user authoring a vertical video is placing captions against
    /// a shape their file will not have. This is the arithmetic behind showing
    /// them the real one.
    ///
    /// Returns `self` unchanged for a non-finite or non-positive ratio, so a
    /// degenerate configuration draws the panel it always drew rather than
    /// collapsing the preview to nothing.
    #[must_use]
    pub fn fit_aspect(&self, ratio: f32) -> UiRect {
        if self.is_empty() || !ratio.is_finite() || ratio <= 0.0 {
            return *self;
        }
        let by_width = self.width / ratio;
        let (width, height) = if by_width <= self.height {
            (self.width, by_width)
        } else {
            (self.height * ratio, self.height)
        };
        UiRect::new(
            self.x + (self.width - width) * 0.5,
            self.y + (self.height - height) * 0.5,
            width,
            height,
        )
    }
}

/// Action row geometry, shared with the drawing code so the two cannot drift
/// (`workspace_layout.h:41-47`).
pub const WORKSPACE_TRACKS_SINGLE_ACTION_TOP: f32 = 50.0;
pub const WORKSPACE_TRACKS_SINGLE_ACTION_HEIGHT: f32 = 36.0;
pub const WORKSPACE_TRACKS_STACKED_ACTION_TOP: f32 = 46.0;
pub const WORKSPACE_TRACKS_STACKED_ACTION_HEIGHT: f32 = 32.0;
pub const WORKSPACE_TRACKS_ACTION_GAP: f32 = 4.0;
pub const WORKSPACE_TRACKS_SINGLE_HEADER: f32 = 96.0;
pub const WORKSPACE_TRACKS_STACKED_HEADER: f32 = 124.0;

/// A panel qualifies for a mode only when it contains that mode's action row, so
/// the single-row floor is derived from that row rather than guessed
/// (`workspace_layout.h:49-51`).
pub const WORKSPACE_TRACKS_SINGLE_MINIMUM: f32 =
    WORKSPACE_TRACKS_SINGLE_ACTION_TOP + WORKSPACE_TRACKS_SINGLE_ACTION_HEIGHT;
/// Two rows are only worth their vertical cost once a track row still fits below
/// (`workspace_layout.h:52-53`).
pub const WORKSPACE_TRACKS_STACKED_MINIMUM: f32 = 168.0;

/// `scene_browser` draws a 27 px header, a 36 px footer, 16 px of padding, four
/// 4 px gaps, and five rows that clamp to a 24 px floor: 27+36+16+16+120
/// (`workspace_layout.h:55-57`). Measured against the panel the minimum supported
/// window (960x640) actually produces, not against a guessed threshold.
pub const WORKSPACE_SCENES_MINIMUM: f32 = 215.0;
/// Raised from 292 together with the scene-tile cap in `plug.c`'s `scene_browser`.
/// 292 was exactly enough for 38 px tiles, so raising either one alone changes
/// nothing: the browser either cannot use the extra height or is not given it.
/// 355 = 27 header + 36 footer + 16 padding + 5 rows of 52 + 4 gaps of 4
/// (`workspace_layout.h:58-62`).
pub const WORKSPACE_SCENES_MAXIMUM: f32 = 355.0;

/// A track row is `sidebar_width*0.2` tall (see `plug.c`'s `tracks_panel`), so the
/// panel's content height depends on the width it was given
/// (`workspace_layout.h:64-66`).
pub const WORKSPACE_TRACKS_ITEM_RATIO: f32 = 0.2;
/// Enough for a long track list to stay usable without starving the browser; the
/// scene floor is what actually protects the browser, this just stops an enormous
/// list asking for the whole sidebar (`workspace_layout.h:67-70`).
pub const WORKSPACE_TRACKS_MAXIMUM: f32 = 520.0;

/// How much of the tracks panel the budget can actually host
/// (`workspace_layout.h:33-38`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TracksPanelMode {
    /// Not drawn: no room for even one action row.
    #[default]
    Hidden,
    /// One row of four actions, then the track list.
    Single,
    /// Two rows of two actions, then the track list.
    Stacked,
}

impl TracksPanelMode {
    /// Offset and height of the action row this mode draws, measured from the
    /// panel top: `(top, height)`. Port of `workspace_tracks_action_row`
    /// (`workspace_layout.c:47-65`).
    ///
    /// [`TracksPanelMode::Hidden`] yields `None` because it draws no row at all —
    /// C's `return false` with `*top`/`*height` left untouched. A row only some
    /// modes have must be a parameter, never an assumption: reserving height for a
    /// row that is never drawn is exactly how the panel stole the browser's space.
    pub fn action_row(self) -> Option<(f32, f32)> {
        match self {
            TracksPanelMode::Single => Some((
                WORKSPACE_TRACKS_SINGLE_ACTION_TOP,
                WORKSPACE_TRACKS_SINGLE_ACTION_HEIGHT,
            )),
            // Two rows and the gap between them.
            TracksPanelMode::Stacked => Some((
                WORKSPACE_TRACKS_STACKED_ACTION_TOP,
                WORKSPACE_TRACKS_STACKED_ACTION_HEIGHT * 2.0 + WORKSPACE_TRACKS_ACTION_GAP,
            )),
            TracksPanelMode::Hidden => None,
        }
    }
}

/// The sidebar split (`workspace_layout.h:72-76`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorkspaceSidebar {
    /// Zero height when `tracks_mode` is [`TracksPanelMode::Hidden`].
    pub tracks: UiRect,
    pub scenes: UiRect,
    pub tracks_mode: TracksPanelMode,
}

impl WorkspaceSidebar {
    /// Splits the sidebar top to bottom. Port of `workspace_sidebar_layout`
    /// (`workspace_layout.c:67-120`).
    ///
    /// The two rects always tile the budget exactly and never overlap. The scene
    /// browser is served first because it has a hard content floor and no
    /// collapsed form; the tracks panel takes the remainder and degrades through
    /// the mode ladder, disappearing rather than overflowing.
    ///
    /// Returns `None` for non-finite or non-positive input — C's "returns false
    /// and leaves `*out` untouched", made structural by `Option`.
    ///
    /// `track_count` sizes the tracks panel to its content rather than letting it
    /// absorb every spare pixel. The empty list used to be the only elastic region
    /// in the sidebar, so a taller window grew whitespace under the tracks while
    /// the scene grid stayed pinned at its cap. Surplus now raises the browser
    /// instead (`workspace_layout.h:78-89`).
    // Cap-then-floor, in that order, is the oracle's (`workspace_layout.c:89-95`).
    // `f32::clamp` panics when max < min and propagates NaN differently, so the
    // sequential ifs are the faithful form.
    #[allow(clippy::manual_clamp)]
    pub fn layout(sidebar_width: f32, sidebar_height: f32, track_count: usize) -> Option<Self> {
        if !sidebar_width.is_finite() || !sidebar_height.is_finite() {
            return None;
        }
        if sidebar_width <= 0.0 || sidebar_height <= 0.0 {
            return None;
        }

        // What the tracks panel actually needs for its chrome and its rows. Asking
        // for its content instead of taking the remainder is the whole point: an
        // empty or short list no longer turns a taller window into whitespace.
        let mut tracks_wanted = WORKSPACE_TRACKS_STACKED_HEADER
            + track_count as f32 * sidebar_width * WORKSPACE_TRACKS_ITEM_RATIO;
        // The oracle sends an overflowed (infinite) ask to the *minimum*, not the
        // maximum, so a track count near 1e37 would yield a smaller panel than 4096
        // tracks does. Kept as-is: unreachable in practice, and it is the frozen
        // behaviour.
        if !tracks_wanted.is_finite() || tracks_wanted < WORKSPACE_TRACKS_STACKED_MINIMUM {
            tracks_wanted = WORKSPACE_TRACKS_STACKED_MINIMUM;
        }
        if tracks_wanted > WORKSPACE_TRACKS_MAXIMUM {
            tracks_wanted = WORKSPACE_TRACKS_MAXIMUM;
        }

        // The scene browser then takes what is left, still bounded by its own floor
        // and cap, so surplus height reaches the scene grid rather than the gap
        // under the track list.
        let mut scenes_height = sidebar_height - tracks_wanted;
        if scenes_height > WORKSPACE_SCENES_MAXIMUM {
            scenes_height = WORKSPACE_SCENES_MAXIMUM;
        }
        if scenes_height < WORKSPACE_SCENES_MINIMUM {
            scenes_height = WORKSPACE_SCENES_MINIMUM;
        }
        // A sidebar smaller than the scene floor gives the browser everything it can.
        if scenes_height > sidebar_height {
            scenes_height = sidebar_height;
        }

        let mut tracks_height = sidebar_height - scenes_height;
        if tracks_height < 0.0 {
            tracks_height = 0.0;
        }

        let mode = if tracks_height >= WORKSPACE_TRACKS_STACKED_MINIMUM {
            TracksPanelMode::Stacked
        } else if tracks_height >= WORKSPACE_TRACKS_SINGLE_MINIMUM {
            TracksPanelMode::Single
        } else {
            TracksPanelMode::Hidden
        };
        // Below one action row the panel would draw its buttons past its own bottom
        // edge, over the scene browser, and win their clicks. Yield the space instead.
        if mode == TracksPanelMode::Hidden {
            tracks_height = 0.0;
            scenes_height = sidebar_height;
        }

        Some(Self {
            tracks: UiRect::new(0.0, 0.0, sidebar_width, tracks_height),
            scenes: UiRect::new(0.0, tracks_height, sidebar_width, scenes_height),
            tracks_mode: mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preview letterbox (EX2), pinned in both directions and at the
    /// degenerate edges.
    ///
    /// The pillarbox case is the one that matters: a 9:16 export inside a
    /// landscape preview panel is what a user authoring a vertical video sees,
    /// and it is the case the panel used to get silently wrong by showing the
    /// scene at the panel's own shape.
    #[test]
    fn a_preview_fits_the_export_aspect_inside_itself_and_stays_centred() {
        let panel = UiRect::new(100.0, 50.0, 800.0, 400.0);

        // Taller than the panel: pillarboxed, height fills, centred in x.
        let vertical = panel.fit_aspect(9.0 / 16.0);
        assert_eq!(vertical.height, 400.0);
        near(vertical.width, 225.0, 0.01);
        near(vertical.x, 100.0 + (800.0 - 225.0) * 0.5, 0.01);
        assert_eq!(vertical.y, 50.0);

        // 16:9 is *narrower* than this 2:1 panel, so it still pillarboxes: the
        // height fills and the width comes down to 711. The panel being wider
        // than 16:9 is the ordinary case in this workspace, not an exotic one —
        // with a bottom panel open the preview band is routinely 3:1 — so this
        // is the assertion that would catch a `fit_aspect` that only ever
        // letterboxed.
        let wide = panel.fit_aspect(16.0 / 9.0);
        assert_eq!(wide.height, 400.0);
        near(wide.width, 711.11, 0.01);
        near(wide.x, 100.0 + (800.0 - 711.11) * 0.5, 0.01);

        // Wider than the panel: letterboxed, width fills, centred in y.
        let letterboxed = panel.fit_aspect(4.0);
        assert_eq!(letterboxed.width, 800.0);
        near(letterboxed.height, 200.0, 0.01);
        near(letterboxed.y, 50.0 + (400.0 - 200.0) * 0.5, 0.01);
        assert_eq!(letterboxed.x, 100.0);

        // The exact-match case must be the identity, or a 16:9 export inside a
        // 16:9 panel would gain a one-pixel border from rounding.
        let exact = UiRect::new(0.0, 0.0, 1920.0, 1080.0).fit_aspect(1920.0 / 1080.0);
        assert_eq!((exact.x, exact.y), (0.0, 0.0));
        near(exact.width, 1920.0, 0.01);
        near(exact.height, 1080.0, 0.01);

        // A ratio that cannot be honoured leaves the panel alone rather than
        // collapsing the preview to nothing.
        for ratio in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let unchanged = panel.fit_aspect(ratio);
            assert_eq!(
                (unchanged.x, unchanged.y, unchanged.width, unchanged.height),
                (panel.x, panel.y, panel.width, panel.height),
                "ratio {ratio} should be refused"
            );
        }
        assert!(UiRect::new(0.0, 0.0, 0.0, 0.0).fit_aspect(1.0).is_empty());
    }

    /// C's `EXPECT_NEAR`, which also fails on a non-finite actual
    /// (`tests/test_support.h:63-70`).
    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= tolerance,
            "actual={actual} expected={expected} tolerance={tolerance}"
        );
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> UiRect {
        UiRect::new(x, y, width, height)
    }

    #[test]
    fn ui_rect_containment_is_inclusive_and_rejects_degenerate_input() {
        let outer = rect(0.0, 0.0, 100.0, 100.0);
        assert!(outer.contains(rect(10.0, 10.0, 50.0, 50.0)));
        // Flush with every edge still counts.
        assert!(outer.contains(rect(0.0, 0.0, 100.0, 100.0)));
        assert!(!outer.contains(rect(-1.0, 0.0, 10.0, 10.0)));
        assert!(!outer.contains(rect(0.0, 95.0, 10.0, 10.0)));
        // The defect this module exists for: a control below a zero-height panel.
        assert!(!rect(0.0, 0.0, 240.0, 0.0).contains(rect(10.0, 50.0, 52.0, 36.0)));
        assert!(!outer.contains(rect(10.0, 10.0, 0.0, 10.0)));
        assert!(!outer.contains(rect(f32::NAN, 10.0, 10.0, 10.0)));
        assert!(!rect(0.0, 0.0, f32::INFINITY, 10.0).contains(rect(1.0, 1.0, 2.0, 2.0)));
    }

    #[test]
    fn ui_rect_overlap_excludes_touching_edges() {
        assert!(rect(0.0, 0.0, 10.0, 10.0).overlaps(rect(5.0, 5.0, 10.0, 10.0)));
        // Stacked panels share an edge and must not count as overlapping.
        assert!(!rect(0.0, 0.0, 10.0, 10.0).overlaps(rect(0.0, 10.0, 10.0, 10.0)));
        assert!(!rect(0.0, 0.0, 10.0, 0.0).overlaps(rect(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn ui_rect_intersect_clamps_to_the_shared_area() {
        let got = rect(0.0, 0.0, 10.0, 10.0).intersect(rect(5.0, 5.0, 10.0, 10.0));
        near(got.x, 5.0, 0.001);
        near(got.y, 5.0, 0.001);
        near(got.width, 5.0, 0.001);
        near(got.height, 5.0, 0.001);

        let none = rect(0.0, 0.0, 10.0, 10.0).intersect(rect(50.0, 50.0, 10.0, 10.0));
        assert!(none.is_empty());
    }

    #[test]
    fn workspace_sidebar_layout_rejects_degenerate_input() {
        // C passed a sentinel `*out` to prove rejection left it untouched; here the
        // `Option` makes that structural, so the sentinel is the variable itself.
        let sentinel = rect(1.0, 2.0, 3.0, 4.0);
        let mut layout = WorkspaceSidebar {
            tracks: sentinel,
            scenes: sentinel,
            tracks_mode: TracksPanelMode::Stacked,
        };
        for got in [
            WorkspaceSidebar::layout(240.0, f32::NAN, 1),
            WorkspaceSidebar::layout(240.0, f32::INFINITY, 1),
            WorkspaceSidebar::layout(240.0, 0.0, 1),
            WorkspaceSidebar::layout(240.0, -10.0, 1),
            WorkspaceSidebar::layout(0.0, 600.0, 1),
        ] {
            assert!(got.is_none());
            if let Some(got) = got {
                layout = got;
            }
        }
        // Rejection must leave the caller's layout untouched.
        near(layout.tracks.width, 3.0, 0.001);
    }

    // The states measured from the real application at the 960x640 minimum window.
    // content_y is 146 and assist_timeline_height caps at screen - toolbar - 150.
    #[test]
    fn workspace_sidebar_layout_hides_the_tracks_panel_when_it_cannot_host_its_row() {
        // Assist with a staged candidate: the sidebar budget is exactly zero after
        // the scene browser is served. This is the state that stole scene clicks.
        let layout = WorkspaceSidebar::layout(240.0, 208.0, 1).expect("208 px sidebar");
        assert_eq!(layout.tracks_mode, TracksPanelMode::Hidden);
        near(layout.tracks.height, 0.0, 0.001);
        near(layout.scenes.height, 208.0, 0.001);

        // Assist confirmation, reachable with two clicks and no model at all.
        let layout = WorkspaceSidebar::layout(240.0, 242.0, 1).expect("242 px sidebar");
        assert_eq!(layout.tracks_mode, TracksPanelMode::Hidden);
        near(layout.tracks.height, 0.0, 0.001);

        // Assist ready: 89 px of tracks panel, which does contain its 86 px action
        // row. This one works today and must keep working.
        let layout = WorkspaceSidebar::layout(240.0, 304.0, 1).expect("304 px sidebar");
        assert_eq!(layout.tracks_mode, TracksPanelMode::Single);
        assert!(layout.tracks.height >= WORKSPACE_TRACKS_SINGLE_MINIMUM);

        // Lyrics or export panel open at 640: unchanged single-row behaviour.
        let layout = WorkspaceSidebar::layout(240.0, 310.0, 1).expect("310 px sidebar");
        assert_eq!(layout.tracks_mode, TracksPanelMode::Single);

        // No bottom panel at 720p: the roomy case keeps two stacked rows. The
        // tracks panel takes what one 64 px row plus its chrome needs (188) rather
        // than the old fixed 168, and the browser takes the rest instead of being
        // pinned at its cap with the surplus left as whitespace under the list.
        let layout = WorkspaceSidebar::layout(320.0, 460.0, 1).expect("460 px sidebar");
        assert_eq!(layout.tracks_mode, TracksPanelMode::Stacked);
        near(layout.tracks.height, 188.0, 0.001);
        near(layout.scenes.height, 272.0, 0.001);
    }

    #[test]
    fn workspace_sidebar_layout_spends_surplus_on_the_scene_browser() {
        // The inversion this module was changed for. With a short track list, extra
        // sidebar height used to land in the gap below the tracks because the
        // browser was already at its cap; it now reaches the scene grid until the
        // browser is full, and only then goes to the tracks panel.
        let small = WorkspaceSidebar::layout(240.0, 460.0, 1).expect("460 px sidebar");
        // 500 is below the point where the browser hits its cap (172 + 355 = 527);
        // past that the surplus correctly goes to the tracks panel instead.
        let large = WorkspaceSidebar::layout(240.0, 500.0, 1).expect("500 px sidebar");
        assert!(large.scenes.height > small.scenes.height);
        near(large.tracks.height, small.tracks.height, 0.001);

        // Past the browser's cap the tracks panel gets the remainder, so nothing is
        // lost -- the priority simply reversed.
        let huge = WorkspaceSidebar::layout(240.0, 900.0, 1).expect("900 px sidebar");
        near(huge.scenes.height, WORKSPACE_SCENES_MAXIMUM, 0.001);
        assert!(huge.tracks.height > large.tracks.height);

        // A longer list asks for more and gets it, at the browser's expense down to
        // its floor but never below.
        let many = WorkspaceSidebar::layout(240.0, 500.0, 8).expect("500 px, 8 tracks");
        assert!(many.tracks.height > large.tracks.height);
        assert!(many.scenes.height >= WORKSPACE_SCENES_MINIMUM);

        // And an absurd list cannot starve the browser below its floor.
        let absurd = WorkspaceSidebar::layout(240.0, 500.0, 4096).expect("500 px, 4096 tracks");
        assert!(absurd.scenes.height >= WORKSPACE_SCENES_MINIMUM);
        near(absurd.tracks.height + absurd.scenes.height, 500.0, 0.01);
    }

    #[test]
    fn workspace_sidebar_layout_tiles_the_budget_and_contains_every_action_row() {
        let mut height = 1.0f32;
        while height <= 1400.0 {
            // Track counts spanning empty, typical, and far past what fits.
            for count in [0usize, 1, 3, 12, 512] {
                let layout =
                    WorkspaceSidebar::layout(240.0, height, count).expect("supported sidebar");

                // The two panels tile the sidebar exactly: no gap, no overlap.
                near(layout.tracks.height + layout.scenes.height, height, 0.01);
                near(layout.scenes.y, layout.tracks.height, 0.001);
                assert!(!layout.tracks.overlaps(layout.scenes));
                assert!(layout.tracks.height >= 0.0);
                assert!(layout.scenes.height >= 0.0);

                if layout.tracks_mode == TracksPanelMode::Hidden {
                    near(layout.tracks.height, 0.0, 0.001);
                    continue;
                }

                // The invariant the click hijack violated: the panel contains the row
                // it draws, so no action button can register a hit box over the scene
                // grid.
                let (top, row_height) = layout
                    .tracks_mode
                    .action_row()
                    .expect("a visible mode has a row");
                let row = UiRect::new(
                    layout.tracks.x + 10.0,
                    layout.tracks.y + top,
                    layout.tracks.width - 20.0,
                    row_height,
                );
                assert!(layout.tracks.contains(row));
                assert!(!row.overlaps(layout.scenes));

                if layout.tracks_mode == TracksPanelMode::Stacked {
                    assert!(layout.tracks.height >= WORKSPACE_TRACKS_STACKED_MINIMUM);
                } else {
                    assert!(layout.tracks.height >= WORKSPACE_TRACKS_SINGLE_MINIMUM);
                }
            }
            height += 1.0;
        }
    }

    #[test]
    fn workspace_sidebar_layout_serves_the_scene_browser_floor_when_it_can() {
        let mut height = 215.0f32;
        while height <= 1400.0 {
            let layout = WorkspaceSidebar::layout(240.0, height, 1).expect("supported sidebar");
            assert!(layout.scenes.height >= WORKSPACE_SCENES_MINIMUM);
            assert!(
                layout.scenes.height <= WORKSPACE_SCENES_MAXIMUM
                    || layout.tracks_mode == TracksPanelMode::Hidden
            );
            height += 1.0;
        }
        // Below the floor the browser simply takes everything there is.
        let tiny = WorkspaceSidebar::layout(240.0, 120.0, 1).expect("120 px sidebar");
        assert_eq!(tiny.tracks_mode, TracksPanelMode::Hidden);
        near(tiny.scenes.height, 120.0, 0.001);
    }

    #[test]
    fn workspace_tracks_action_row_has_no_row_when_hidden() {
        assert!(TracksPanelMode::Hidden.action_row().is_none());
        let (top, height) = TracksPanelMode::Single
            .action_row()
            .expect("single mode draws a row");
        near(top + height, WORKSPACE_TRACKS_SINGLE_MINIMUM, 0.001);
        // C also asserted the NULL-out-pointer rejection; `Option` removes that
        // failure mode entirely, so there is nothing left to test.
    }
}
