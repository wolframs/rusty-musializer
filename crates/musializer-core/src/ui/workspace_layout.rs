//! Workspace rectangles and the rail.
//!
//! **Owner: Agent F.** [`UiRect`] is a port of `workspace_layout.c/.h` from the
//! frozen C oracle at `../musializer` (commit `9300af9`, read-only), and is
//! still compared with it by `tools/differential_layout.sh`.
//!
//! **The sidebar split is retired (WX-5c).** `WorkspaceSidebar::layout` — the
//! oracle's `workspace_sidebar_layout`, which split one vertical budget between
//! the tracks panel and the scene browser and hid the tracks panel when it
//! could not contain its own action row (`workspace_layout.h:7-19`) — is gone
//! with the sidebar it laid out, along with `TracksPanelMode` and its
//! constants. The window draws a 52 px rail and two flyouts instead
//! ([`WorkspaceRail`]), and a harness that pinned code the window no longer
//! calls would prove nothing. The two constants the lyrics band budget still
//! reads moved into `lyrics_editor_layout` at their values. The rule the
//! sidebar paid for — a panel that cannot contain its controls is not drawn —
//! is kept by the rail, which drops icons rather than overflowing.

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

/// The top bar's height in logical units (WX-5c, `docs/WX5C_DESIGN.md` §4).
///
/// **Post-legacy, no oracle**: the C has no top bar. Here rather than in the
/// app because the rail starts where the bar ends, and [`WorkspaceRail`] is
/// pinned by `tools/differential_layout.sh`'s post-legacy section.
pub const TOP_BAR_HEIGHT: f32 = 46.0;
/// The rail's width (§1). The old sidebar's 320 px minus this is what the
/// preview gained.
pub const RAIL_WIDTH: f32 = 52.0;
/// A flyout's width (§1). It floats over the preview's left edge.
pub const FLYOUT_WIDTH: f32 = 336.0;
/// A rail icon's box at full size, and the smallest it may shrink to before
/// the bottom pair is dropped.
pub const RAIL_ICON_SIZE: f32 = 36.0;
pub const RAIL_ICON_MINIMUM: f32 = 28.0;
/// Between two icons of one group.
pub const RAIL_ICON_GAP: f32 = 6.0;
/// Above the first icon and below the last.
pub const RAIL_ICON_MARGIN: f32 = 10.0;
/// The least space between the top group and the bottom group before the
/// bottom group stops sitting at the bottom and stacks under the top one.
pub const RAIL_GROUP_GAP: f32 = 12.0;

/// The four rail icons, top to bottom (§1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailIcon {
    /// The tracks flyout: open, add, save, the track list.
    Tracks,
    /// The scenes flyout: the picker and the ASCII image footer.
    Scenes,
    /// The toolkit editor (F8), moved here from the transport.
    Editor,
    /// The dock (Ctrl+`), moved here from the transport.
    Dock,
}

impl RailIcon {
    pub const ALL: [RailIcon; 4] = [
        RailIcon::Tracks,
        RailIcon::Scenes,
        RailIcon::Editor,
        RailIcon::Dock,
    ];

    /// The token the `rail:` report line prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            RailIcon::Tracks => "tracks",
            RailIcon::Scenes => "scenes",
            RailIcon::Editor => "editor",
            RailIcon::Dock => "dock",
        }
    }
}

/// The rail beside the preview and the transport, its icons, and where a
/// flyout opens (WX-5c §1, §5).
///
/// **Post-legacy, no oracle.** It replaces `WorkspaceSidebar`, which the
/// window no longer draws. Pure, so `tools/differential_layout.sh` can pin it
/// over the same sweep it runs the oracle's layouts through.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorkspaceRail {
    /// From the top bar's bottom to the band's top, [`RAIL_WIDTH`] wide.
    pub rail: UiRect,
    /// One box per [`RailIcon`], in [`RailIcon::ALL`]'s order. `None` when the
    /// rail is too short to seat it: Editor and Dock go first (both have a key),
    /// then Scenes; Tracks is the last to go.
    pub icons: [Option<UiRect>; 4],
    /// Where a flyout opens: at the rail's right edge, from the top bar's
    /// bottom to the window's. It may cover the transport and the band, because
    /// it is transient.
    pub flyout: UiRect,
}

impl WorkspaceRail {
    /// Lays the rail out for a window (logical units), a top bar of
    /// `top_bar_height`, and a band whose top edge is `band_top`.
    ///
    /// The icons sit in two groups: Tracks and Scenes from the top, Editor and
    /// Dock from the bottom. A rail too short for the gap between them stacks
    /// the bottom pair under the top pair; one too short for that shrinks every
    /// icon toward [`RAIL_ICON_MINIMUM`]; below that the bottom pair is
    /// dropped, then Scenes. Each step strictly needs less height than the one
    /// before, so a rail that grows never loses an icon.
    ///
    /// `None` for non-finite input or a window narrower than the rail.
    #[must_use]
    pub fn layout(
        window_width: f32,
        window_height: f32,
        top_bar_height: f32,
        band_top: f32,
    ) -> Option<Self> {
        if !window_width.is_finite()
            || !window_height.is_finite()
            || !top_bar_height.is_finite()
            || !band_top.is_finite()
            || window_width < RAIL_WIDTH
        {
            return None;
        }
        let top = top_bar_height.max(0.0);
        let rail = UiRect::new(0.0, top, RAIL_WIDTH, (band_top - top).max(0.0));
        let flyout = UiRect::new(
            RAIL_WIDTH,
            top,
            FLYOUT_WIDTH.min(window_width - RAIL_WIDTH),
            (window_height - top).max(0.0),
        );

        let height = rail.height;
        // Height the icons alone ask for, with `n` icons of side `s` stacked.
        let stacked = |n: f32, s: f32| RAIL_ICON_MARGIN * 2.0 + n * s + (n - 1.0) * RAIL_ICON_GAP;
        let split = stacked(4.0, RAIL_ICON_SIZE) + RAIL_GROUP_GAP - RAIL_ICON_GAP;
        let x = |s: f32| (RAIL_WIDTH - s) * 0.5;
        let at = |index: usize, s: f32| {
            UiRect::new(
                x(s),
                rail.y + RAIL_ICON_MARGIN + index as f32 * (s + RAIL_ICON_GAP),
                s,
                s,
            )
        };
        let mut icons = [None; 4];
        if height >= split {
            // Two groups: the top pair from the top, the bottom pair from the
            // bottom, the slack between them.
            let s = RAIL_ICON_SIZE;
            icons[0] = Some(at(0, s));
            icons[1] = Some(at(1, s));
            let bottom = rail.y + rail.height - RAIL_ICON_MARGIN;
            icons[3] = Some(UiRect::new(x(s), bottom - s, s, s));
            icons[2] = Some(UiRect::new(x(s), bottom - s * 2.0 - RAIL_ICON_GAP, s, s));
        } else if height >= stacked(4.0, RAIL_ICON_MINIMUM) {
            // One column of four, as large as fits.
            let s = ((height - RAIL_ICON_MARGIN * 2.0 - 3.0 * RAIL_ICON_GAP) / 4.0)
                .clamp(RAIL_ICON_MINIMUM, RAIL_ICON_SIZE)
                .floor();
            for (index, icon) in icons.iter_mut().enumerate() {
                *icon = Some(at(index, s));
            }
        } else if height >= stacked(2.0, RAIL_ICON_MINIMUM) {
            let s = ((height - RAIL_ICON_MARGIN * 2.0 - RAIL_ICON_GAP) / 2.0)
                .clamp(RAIL_ICON_MINIMUM, RAIL_ICON_SIZE)
                .floor();
            icons[0] = Some(at(0, s));
            icons[1] = Some(at(1, s));
        } else if height >= stacked(1.0, RAIL_ICON_MINIMUM) {
            icons[0] = Some(at(0, RAIL_ICON_MINIMUM));
        }
        Some(Self {
            rail,
            icons,
            flyout,
        })
    }

    /// The box for one icon, if it was seated.
    #[must_use]
    pub fn icon(&self, icon: RailIcon) -> Option<UiRect> {
        let index = RailIcon::ALL
            .iter()
            .position(|candidate| *candidate == icon)?;
        self.icons[index]
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

    /// The rail at the windows the gate photographs, pinned as numbers. A
    /// property ("the icon is inside the rail") is satisfied by a wrong
    /// formula; these are what the arithmetic produces.
    #[test]
    fn the_rail_seats_two_groups_of_icons_at_the_photographed_windows() {
        // 1280x720 with no panel: the band is 260 tall, so it starts at 460.
        let rail = WorkspaceRail::layout(1280.0, 720.0, TOP_BAR_HEIGHT, 460.0).unwrap();
        assert_eq!(rail.rail, UiRect::new(0.0, 46.0, 52.0, 414.0));
        assert_eq!(rail.flyout, UiRect::new(52.0, 46.0, 336.0, 674.0));
        assert_eq!(rail.icons[0], Some(UiRect::new(8.0, 56.0, 36.0, 36.0)));
        assert_eq!(rail.icons[1], Some(UiRect::new(8.0, 98.0, 36.0, 36.0)));
        assert_eq!(rail.icons[2], Some(UiRect::new(8.0, 372.0, 36.0, 36.0)));
        assert_eq!(rail.icons[3], Some(UiRect::new(8.0, 414.0, 36.0, 36.0)));
        assert_eq!(rail.icon(RailIcon::Dock), rail.icons[3]);

        // 960x640, the minimum: the band starts at 380.
        let rail = WorkspaceRail::layout(960.0, 640.0, TOP_BAR_HEIGHT, 380.0).unwrap();
        assert_eq!(rail.rail, UiRect::new(0.0, 46.0, 52.0, 334.0));
        assert_eq!(rail.flyout, UiRect::new(52.0, 46.0, 336.0, 594.0));
        assert_eq!(rail.icons[2], Some(UiRect::new(8.0, 292.0, 36.0, 36.0)));
        assert_eq!(rail.icons[3], Some(UiRect::new(8.0, 334.0, 36.0, 36.0)));
    }

    #[test]
    fn a_short_rail_stacks_then_shrinks_then_drops_its_bottom_pair() {
        // 182 is exactly four full icons stacked plus the margins; 188 the two
        // groups with their 12 px between.
        let split = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 188.0).unwrap();
        assert_eq!(split.icons[1], Some(UiRect::new(8.0, 98.0, 36.0, 36.0)));
        assert_eq!(split.icons[2], Some(UiRect::new(8.0, 146.0, 36.0, 36.0)));
        assert_eq!(split.icons[3], Some(UiRect::new(8.0, 188.0, 36.0, 36.0)));
        let stacked = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 187.0).unwrap();
        assert_eq!(stacked.icons[2], Some(UiRect::new(8.0, 140.0, 36.0, 36.0)));
        assert_eq!(stacked.icons[3], Some(UiRect::new(8.0, 182.0, 36.0, 36.0)));
        // 160 px: four icons of 30.
        let shrunk = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 160.0).unwrap();
        assert_eq!(shrunk.icons[0], Some(UiRect::new(11.0, 56.0, 30.0, 30.0)));
        assert_eq!(shrunk.icons[3], Some(UiRect::new(11.0, 164.0, 30.0, 30.0)));
        // 149 px: four of 28 need 150, so the bottom pair goes.
        let pair = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 149.0).unwrap();
        assert_eq!(pair.icons[2], None);
        assert_eq!(pair.icons[3], None);
        assert_eq!(pair.icons[1], Some(UiRect::new(8.0, 98.0, 36.0, 36.0)));
        let one = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 50.0).unwrap();
        assert_eq!(
            one.icons,
            [Some(UiRect::new(12.0, 56.0, 28.0, 28.0)), None, None, None]
        );
        let none = WorkspaceRail::layout(1280.0, 720.0, 46.0, 46.0 + 47.0).unwrap();
        assert_eq!(none.icons, [None; 4]);
    }

    #[test]
    fn a_rail_that_grows_never_loses_an_icon_and_never_leaves_itself() {
        let mut seated = 0usize;
        for height in 0..=900 {
            let rail = WorkspaceRail::layout(1280.0, 1000.0, 46.0, 46.0 + height as f32).unwrap();
            let count = rail.icons.iter().flatten().count();
            assert!(count >= seated, "{height}: {count} icons after {seated}");
            seated = count;
            let placed: Vec<UiRect> = rail.icons.iter().flatten().copied().collect();
            for (index, icon) in placed.iter().enumerate() {
                assert!(
                    rail.rail.contains(*icon),
                    "{height}: icon {index} leaves the rail"
                );
                for other in &placed[index + 1..] {
                    assert!(!icon.overlaps(*other), "{height}: two icons overlap");
                }
            }
        }
        assert_eq!(seated, 4);
        assert!(WorkspaceRail::layout(f32::NAN, 720.0, 46.0, 400.0).is_none());
        assert!(WorkspaceRail::layout(40.0, 720.0, 46.0, 400.0).is_none());
        // A window narrower than rail plus flyout gives the flyout what is left.
        let narrow = WorkspaceRail::layout(300.0, 720.0, 46.0, 400.0).unwrap();
        assert_eq!(narrow.flyout.width, 248.0);
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
}
