//! Timeline top-band placement: controls, the trailing clear button, timecode.
//!
//! **Owner: Agent F.** Port of `timeline_layout.c/.h` from the frozen C oracle at
//! `../musializer` (commit `9300af9`, read-only).
//!
//! The panel/event control row, the "Clear manual" button that trails it, and the
//! right-aligned timecode used to be positioned independently against the same
//! band. The control row declared a width it never used and its children ran 36 px
//! past it, while the timecode was right-aligned into the same 38 px strip, so
//! below roughly 785 px of workspace the timecode printed straight through
//! "+ Custom" and "Clear manual". That is reachable in a *supported* window: at
//! the 960 px minimum with the inspector open the band is only 680 px wide
//! (`timeline_layout.h:9-21`) — the minimum window is the yardstick, not a guess.
//!
//! The rule here is that the parent extent is computed **from the children**
//! rather than declared beside them, so a label or button width change cannot
//! silently reintroduce the overlap.

use super::workspace_layout::UiRect;

/// Most controls a band can seat before the trailing clear button
/// (`timeline_layout.h:23`).
pub const TIMELINE_BAND_CONTROL_CAPACITY: usize = 8;

/// Below this the shared label font (see `ui_row_typography.h`) has already
/// bottomed out, so squeezing further trades collision for illegibility
/// (`timeline_layout.h:25-27`).
pub const TIMELINE_BAND_MIN_SCALE: f32 = 0.72;

/// Clear space kept between the control row and the timecode so they read as
/// separate groups instead of touching (`timeline_layout.h:29-31`).
pub const TIMELINE_BAND_GAP: f32 = 12.0;

/// Port of `Timeline_Band` (`timeline_layout.h:33-48`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineBand {
    /// Multiplier applied to every control width and to the clear button.
    pub scale: f32,
    /// True extent of the scaled row, measured from its children.
    pub controls_width: f32,
    pub controls: UiRect,
    pub clear: UiRect,
    pub timecode: UiRect,
    /// `false` when the band cannot seat the timecode beside the controls; the
    /// caller must place it elsewhere (the transport row) rather than draw it over
    /// the buttons.
    pub timecode_inline: bool,
    /// `false` when even [`TIMELINE_BAND_MIN_SCALE`] overflows: the caller is out
    /// of room and something has to be dropped. Reported rather than hidden.
    pub fits: bool,
}

impl TimelineBand {
    /// Places the band's three groups. Port of `timeline_band_layout`
    /// (`timeline_layout.c:10-86`).
    ///
    /// `control_widths` describes the buttons before the trailing clear button.
    /// Returns `None` only on unusable input — C's "returns false and leaves `*out`
    /// untouched" (`timeline_layout.h:51-56`), made structural by `Option`. C's two
    /// NULL-pointer rejections have no Rust analogue; the empty-slice case covers
    /// `control_count == 0`.
    // Faithful to the C parameter list; a params struct would only move the same
    // eight values one indirection away.
    #[allow(clippy::too_many_arguments)]
    pub fn layout(
        band_x: f32,
        band_y: f32,
        band_width: f32,
        band_height: f32,
        margin: f32,
        control_widths: &[f32],
        clear_width: f32,
        timecode_width: f32,
    ) -> Option<Self> {
        if control_widths.is_empty() || control_widths.len() > TIMELINE_BAND_CONTROL_CAPACITY {
            return None;
        }
        if !band_x.is_finite()
            || !band_y.is_finite()
            || !band_width.is_finite()
            || !band_height.is_finite()
            || !margin.is_finite()
            || !clear_width.is_finite()
            || !timecode_width.is_finite()
        {
            return None;
        }
        if band_width <= 0.0
            || band_height <= 0.0
            || margin < 0.0
            || clear_width < 0.0
            || timecode_width < 0.0
        {
            return None;
        }

        let mut natural = clear_width;
        for &width in control_widths {
            if !width.is_finite() || width < 0.0 {
                return None;
            }
            // Each control is followed by a margin, including the last one, because
            // the clear button trails the row rather than abutting it.
            natural += width + margin;
        }
        if natural <= 0.0 {
            return None;
        }

        let content_x = band_x + margin;
        let content_width = band_width - margin * 2.0;

        // First choice: controls and timecode share the band.
        let inline_room = content_width - timecode_width - TIMELINE_BAND_GAP;
        let mut scale = 1.0f32;
        let mut timecode_inline = true;
        if inline_room < natural {
            scale = if inline_room > 0.0 {
                inline_room / natural
            } else {
                0.0
            };
            if scale < TIMELINE_BAND_MIN_SCALE {
                // Shrinking the row far enough to seat the timecode would make the
                // labels unreadable, so the timecode moves instead of the buttons
                // becoming illegible.
                timecode_inline = false;
                scale = if content_width < natural {
                    content_width / natural
                } else {
                    1.0
                };
            }
        }
        let fits = scale >= TIMELINE_BAND_MIN_SCALE;
        if !fits {
            scale = TIMELINE_BAND_MIN_SCALE;
        }
        if scale > 1.0 {
            scale = 1.0;
        }

        let controls_width = natural * scale;
        let mut cursor = content_x;
        for &width in control_widths {
            cursor += width * scale + margin * scale;
        }

        let timecode = if timecode_inline {
            UiRect::new(
                band_x + band_width - margin - timecode_width,
                band_y,
                timecode_width,
                band_height,
            )
        } else {
            UiRect::new(0.0, 0.0, 0.0, 0.0)
        };

        Some(Self {
            scale,
            controls_width,
            controls: UiRect::new(content_x, band_y, controls_width, band_height),
            clear: UiRect::new(cursor, band_y, clear_width * scale, band_height),
            timecode,
            timecode_inline,
            fits,
        })
    }
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

    /// The shipped control row: six panel/event buttons then "Clear manual".
    const SHIPPED_CONTROLS: [f32; 6] = [74.0, 74.0, 90.0, 74.0, 82.0, 86.0];
    const SHIPPED_CLEAR: f32 = 112.0;
    const SHIPPED_MARGIN: f32 = 6.0;
    const SHIPPED_HEIGHT: f32 = 38.0;
    /// "0:00 / 0:40" at 22 px. A long track ("1:23:45 / 1:23:45") measures wider,
    /// which is why the collision was first noticed on long fixtures.
    const SHIPPED_TIMECODE: f32 = 145.0;

    fn band(width: f32, timecode_width: f32) -> Option<TimelineBand> {
        TimelineBand::layout(
            0.0,
            0.0,
            width,
            SHIPPED_HEIGHT,
            SHIPPED_MARGIN,
            &SHIPPED_CONTROLS,
            SHIPPED_CLEAR,
            timecode_width,
        )
    }

    #[test]
    fn timeline_band_keeps_the_natural_row_when_the_window_is_wide() {
        let layout = band(1920.0, SHIPPED_TIMECODE).expect("1920 px band");
        near(layout.scale, 1.0, 0.0001);
        assert!(layout.timecode_inline);
        assert!(layout.fits);
        // 480 px of buttons + 6 margins + the 112 px clear button.
        near(layout.controls_width, 628.0, 0.01);
        // The declared parent extent is the real one: the clear button ends exactly
        // where the row says it does. This is the property whose absence let the
        // children run 36 px past controls.width.
        near(
            layout.clear.x + layout.clear.width,
            layout.controls.x + layout.controls_width,
            0.01,
        );
    }

    #[test]
    fn timeline_band_never_lets_the_timecode_touch_the_controls() {
        // The reported defect, at the narrowest supported band: a 960 px window
        // with the inspector open leaves 680 px here.
        let layout = band(680.0, SHIPPED_TIMECODE).expect("680 px band");
        if layout.timecode_inline {
            assert!(
                layout.controls.x + layout.controls_width + TIMELINE_BAND_GAP
                    <= layout.timecode.x + 0.01
            );
        }
        // Whatever it chose, the clear button no longer reaches the timecode.
        assert!(layout.clear.x + layout.clear.width <= 680.0 - SHIPPED_MARGIN + 0.01);
    }

    #[test]
    fn timeline_band_moves_the_timecode_before_the_labels_go_unreadable() {
        // The narrowest supported band with a short timecode: the row gives up a
        // little and both still share the strip. 668 px of content less the 145 px
        // timecode and the 12 px gap leaves 511 for a 628 px row.
        let layout = band(680.0, SHIPPED_TIMECODE).expect("680 px band");
        assert!(layout.timecode_inline);
        near(layout.scale, 511.0 / 628.0, 0.001);
        assert!(layout.scale >= TIMELINE_BAND_MIN_SCALE);

        // Same band, but an hour-long track widens the timecode to 232 px. Now
        // seating both would need scale 0.675, past the readability floor, so the
        // timecode relocates rather than the buttons becoming illegible -- and the
        // row goes back to full size because it no longer shares the strip.
        let layout = band(680.0, 232.0).expect("680 px band, long timecode");
        assert!(!layout.timecode_inline);
        near(layout.scale, 1.0, 0.0001);
        assert!(layout.fits);
        assert_eq!(layout.timecode.width as usize, 0);
    }

    #[test]
    fn timeline_band_reports_running_out_of_room_instead_of_hiding_it() {
        // 628 px of natural row cannot fit 400 px even at the minimum scale.
        let layout = band(400.0, SHIPPED_TIMECODE).expect("400 px band");
        assert!(!layout.fits);
        near(layout.scale, TIMELINE_BAND_MIN_SCALE, 0.0001);
        // Clamped rather than driven to zero, so the caller still draws something
        // recognisable while `fits` tells it the truth.
        assert!(layout.controls_width > 0.0);
    }

    #[test]
    fn timeline_band_holds_its_invariants_across_every_supported_width() {
        // 680 is the narrowest band a supported window can produce; sweep well past
        // 4K so a future wide-monitor default cannot reintroduce the overlap.
        let mut width = 680.0f32;
        while width <= 4000.0 {
            // Short and long timecodes; the long form is what made the defect
            // visible in the first place.
            for timecode_width in [145.0f32, 232.0] {
                let layout = band(width, timecode_width).expect("supported band");

                // Children stay inside the parent extent.
                assert!(
                    layout.clear.x + layout.clear.width
                        <= layout.controls.x + layout.controls_width + 0.01
                );
                // The row stays inside the band.
                assert!(layout.controls.x + layout.controls_width <= width - SHIPPED_MARGIN + 0.01);
                // The scale never leaves its declared range.
                assert!(layout.scale >= TIMELINE_BAND_MIN_SCALE);
                assert!(layout.scale <= 1.0);

                if layout.timecode_inline {
                    // The whole point: a gap always survives between the two groups.
                    assert!(
                        layout.controls.x + layout.controls_width + TIMELINE_BAND_GAP
                            <= layout.timecode.x + 0.01
                    );
                    // And the timecode itself stays on the band.
                    assert!(
                        layout.timecode.x + layout.timecode.width <= width - SHIPPED_MARGIN + 0.01
                    );
                    assert!(layout.timecode.x >= layout.controls.x);
                }
            }
            width += 1.0;
        }
    }

    #[test]
    fn timeline_band_rejects_unusable_input_without_writing_a_result() {
        // C's two NULL cases (`out` and `control_widths`) cannot be expressed here.
        // An over-capacity count needs a real over-length slice rather than C's
        // lie about the array length.
        let over_capacity = [74.0f32; TIMELINE_BAND_CONTROL_CAPACITY + 1];
        let negative = [74.0f32, -1.0, 90.0, 74.0, 82.0, 86.0];
        let rejected = [
            TimelineBand::layout(0.0, 0.0, 800.0, 38.0, 6.0, &[], 112.0, 145.0),
            TimelineBand::layout(0.0, 0.0, 800.0, 38.0, 6.0, &over_capacity, 112.0, 145.0),
            TimelineBand::layout(0.0, 0.0, 0.0, 38.0, 6.0, &SHIPPED_CONTROLS, 112.0, 145.0),
            TimelineBand::layout(
                0.0,
                0.0,
                f32::NAN,
                38.0,
                6.0,
                &SHIPPED_CONTROLS,
                112.0,
                145.0,
            ),
            TimelineBand::layout(
                0.0,
                0.0,
                800.0,
                38.0,
                6.0,
                &SHIPPED_CONTROLS,
                112.0,
                f32::INFINITY,
            ),
            TimelineBand::layout(0.0, 0.0, 800.0, 38.0, -1.0, &SHIPPED_CONTROLS, 112.0, 145.0),
            TimelineBand::layout(0.0, 0.0, 800.0, 38.0, 6.0, &negative, 112.0, 145.0),
        ];
        // A rejected call must not have written a partial layout -- structural here.
        for got in rejected {
            assert!(got.is_none());
        }
    }

    #[test]
    fn timeline_band_honours_a_band_that_does_not_start_at_the_origin() {
        let layout = TimelineBand::layout(
            320.0,
            40.0,
            1200.0,
            SHIPPED_HEIGHT,
            SHIPPED_MARGIN,
            &SHIPPED_CONTROLS,
            SHIPPED_CLEAR,
            SHIPPED_TIMECODE,
        )
        .expect("offset band");
        near(layout.controls.x, 326.0, 0.01);
        near(layout.controls.y, 40.0, 0.01);
        assert!(layout.timecode_inline);
        near(
            layout.timecode.x + layout.timecode.width,
            320.0 + 1200.0 - 6.0,
            0.01,
        );
        assert!(
            layout.controls.x + layout.controls_width + TIMELINE_BAND_GAP
                <= layout.timecode.x + 0.01
        );
    }
}
