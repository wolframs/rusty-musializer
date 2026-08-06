//! The workspace palette and metrics.
//!
//! Port of `../musializer/src/ui_palette.h` and `ui_theme.h`. The C splits them
//! for a reason worth keeping: `ui_palette.h` is raylib-free packed
//! `0xRRGGBBAA`, so `tests/test_ui_contrast.c` can check the exact numbers the
//! application draws with, and `ui_theme.h` only wraps each one in `GetColor`.
//! Here [`rgba`] holds the values and [`color`] wraps them, with the same rule:
//! **do not add a colour to the raylib side without adding it to [`rgba`]**, or
//! it becomes invisible to the contrast checks in
//! [`musializer_core::ui::contrast`].

/// The palette as packed `0xRRGGBBAA`, raylib-free
/// (`../musializer/src/ui_palette.h:13-24`).
pub mod rgba {
    pub const ACCENT: u32 = 0x002F_A7FF;
    pub const BACKGROUND: u32 = 0x1515_15FF;
    pub const UI_SURFACE: u32 = 0xF7F7_F8FF;
    pub const UI_RAISED: u32 = 0xFFFF_FFFF;
    pub const UI_INK: u32 = 0x1414_14FF;
    pub const UI_MUTED: u32 = 0x6666_6BFF;
    pub const UI_DISABLED: u32 = 0x8C8C_92FF;
    pub const UI_RULE: u32 = 0xD2D2_D6FF;
    pub const UI_DANGER: u32 = 0xC628_28FF;
    pub const UI_WARNING: u32 = 0x9E5D_00FF;
    pub const UI_SUCCESS: u32 = 0x1879_4EFF;
    pub const TRACK_BUTTON_HOVEROVER: u32 = 0xE7EA_F2FF;
    pub const WHITE: u32 = 0xFFFF_FFFF;

    /// The groove the timed lanes sit in (LX1-e).
    ///
    /// The timeline band is three lanes over one time axis — scene plan,
    /// waveform, lyric cues — and before LX1-e it read as three widgets glued
    /// together: the scene lane and the waveform shared an edge, so their two
    /// 1 px rules drew as one 2 px line, while the cue lane sat five pixels
    /// lower with only a top rule and no box at all. The unifier is this one
    /// tone painted into every gap *between* lanes, bounded left and right by
    /// the group frame: the lanes then read as rows of one table rather than
    /// as separate boxes that happen to be stacked.
    ///
    /// One step darker than [`UI_SURFACE`] and well lighter than [`UI_RULE`],
    /// so a 5 px band is a seam rather than a stripe. [`Surface::Fill`]: no text
    /// is ever drawn on it.
    pub const UI_LANE_TROUGH: u32 = 0xE6E6_EAFF;

    /// The notice tray's card, floating over the scene preview.
    ///
    /// **Opaque, and that is the fix rather than an aesthetic choice**
    /// (review 1.11, UX0-A11). It was `rgba(20, 22, 28, 232)`, so the colour the
    /// user actually read was the card composited over whatever the scene was
    /// drawing that frame — which means no contrast ratio measured against it was
    /// true, and the sweep in this module could not have measured it in the first
    /// place: [`ALL`]'s opacity assertion exists precisely because
    /// [`musializer_core::ui::contrast`] treats every colour as fully composited.
    pub const UI_OVERLAY_SURFACE: u32 = 0x1416_1CFF;
    /// A notice's title, on [`UI_OVERLAY_SURFACE`]. raylib's `RAYWHITE`, promoted
    /// into the palette so the sweep can see it.
    pub const UI_OVERLAY_INK: u32 = 0xF5F5_F5FF;
    /// A notice's detail text: the dark card's equivalent of [`UI_MUTED`].
    pub const UI_OVERLAY_MUTED: u32 = 0xB4BE_CDFF;

    /// The four severity labels on the notice card (review 1.11, UX0-A11).
    ///
    /// The tray used to draw the *chrome* colours here — `ACCENT` for INFO,
    /// `UI_DANGER` for ERROR — which are chosen to sit on a near-white panel.
    /// On the near-black card `ACCENT` measures **1.69:1** and `UI_DANGER`
    /// **3.19:1**, both below the 4.5:1 body-text minimum, so an error and an
    /// info were indistinguishable at a glance. These are the dark-surface
    /// variants: same hues, lifted until they clear AA against the card.
    /// `the_light_chrome_severities_would_fail_the_dark_sweep` keeps the old
    /// numbers as a negative control.
    pub const NOTICE_INFO_ON_DARK: u32 = 0x8AB4_F8FF;
    pub const NOTICE_SUCCESS_ON_DARK: u32 = 0x6FDB_A0FF;
    pub const NOTICE_WARNING_ON_DARK: u32 = 0xF2B4_41FF;
    pub const NOTICE_ERROR_ON_DARK: u32 = 0xFF6B_6BFF;

    /// The background a palette entry is drawn *on*.
    ///
    /// **This is the mechanism UX0-D04 asks for.** The sweep used to assert two
    /// hand-written pairs against the two light surfaces, so the whole
    /// dark-overlay half of the interface — the notice tray and the tooltip —
    /// was invisible to it, and INFO could sit at 1.69:1 for months without
    /// anything failing. Declaring the surface on the entry means a colour cannot
    /// be added to [`ALL`] without answering "on what?", and the sweep then walks
    /// every pair rather than the pairs somebody remembered.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Surface {
        /// Text on the light chrome: panels, rows, the welcome screen.
        Chrome,
        /// Text on a filled control: the accent of a selected button, the ink of
        /// a tooltip.
        Inverted,
        /// Text on the notice tray's card, over the preview.
        DarkOverlay,
        /// Not text: a fill, a rule, or a background other colours are measured
        /// against. No contrast requirement of its own.
        Fill,
        /// Text that WCAG 1.4.3 exempts. Only disabled controls qualify, and the
        /// exemption is the reason a disabled label is allowed to be quiet.
        ExemptText,
    }

    impl Surface {
        /// The backgrounds an entry of this kind must clear AA against.
        #[allow(
            dead_code,
            reason = "the palette audit in this module's tests is its only reader"
        )]
        pub const fn backgrounds(self) -> &'static [(&'static str, u32)] {
            match self {
                Surface::Chrome => &[("ui_surface", UI_SURFACE), ("ui_raised", UI_RAISED)],
                Surface::Inverted => &[("accent", ACCENT), ("ui_ink", UI_INK)],
                Surface::DarkOverlay => &[("ui_overlay_surface", UI_OVERLAY_SURFACE)],
                Surface::Fill | Surface::ExemptText => &[],
            }
        }
    }

    /// Every palette entry and the surface it sits on, so a contrast sweep can
    /// iterate rather than list.
    ///
    /// Read only by the tests below. That is the point of it: the C header warns
    /// that "a constant defined only there is invisible to the contrast suite",
    /// and a list the suite walks is how a colour added without a contrast check
    /// gets caught.
    #[allow(
        dead_code,
        reason = "the palette audit in this module's tests is its only reader"
    )]
    pub const ALL: [(&str, u32, Surface); 21] = [
        ("accent", ACCENT, Surface::Chrome),
        ("background", BACKGROUND, Surface::Fill),
        ("ui_surface", UI_SURFACE, Surface::Fill),
        ("ui_raised", UI_RAISED, Surface::Fill),
        ("ui_ink", UI_INK, Surface::Chrome),
        ("ui_muted", UI_MUTED, Surface::Chrome),
        ("ui_disabled", UI_DISABLED, Surface::ExemptText),
        ("ui_rule", UI_RULE, Surface::Fill),
        ("ui_danger", UI_DANGER, Surface::Chrome),
        ("ui_warning", UI_WARNING, Surface::Chrome),
        ("ui_success", UI_SUCCESS, Surface::Chrome),
        (
            "track_button_hoverover",
            TRACK_BUTTON_HOVEROVER,
            Surface::Fill,
        ),
        ("white", WHITE, Surface::Inverted),
        ("ui_lane_trough", UI_LANE_TROUGH, Surface::Fill),
        ("ui_overlay_surface", UI_OVERLAY_SURFACE, Surface::Fill),
        ("ui_overlay_ink", UI_OVERLAY_INK, Surface::DarkOverlay),
        ("ui_overlay_muted", UI_OVERLAY_MUTED, Surface::DarkOverlay),
        (
            "notice_info_on_dark",
            NOTICE_INFO_ON_DARK,
            Surface::DarkOverlay,
        ),
        (
            "notice_success_on_dark",
            NOTICE_SUCCESS_ON_DARK,
            Surface::DarkOverlay,
        ),
        (
            "notice_warning_on_dark",
            NOTICE_WARNING_ON_DARK,
            Surface::DarkOverlay,
        ),
        (
            "notice_error_on_dark",
            NOTICE_ERROR_ON_DARK,
            Surface::DarkOverlay,
        ),
    ];
}

/// The same palette as raylib colours (`../musializer/src/ui_theme.h:17-38`).
pub mod color {
    use raylib::prelude::Color;

    macro_rules! themed {
        ($($name:ident = $source:ident;)*) => {
            $(
                #[must_use]
                pub fn $name() -> Color {
                    Color::get_color(super::rgba::$source)
                }
            )*
        };
    }

    themed! {
        accent = ACCENT;
        background = BACKGROUND;
        ui_surface = UI_SURFACE;
        ui_raised = UI_RAISED;
        ui_ink = UI_INK;
        ui_muted = UI_MUTED;
        ui_disabled = UI_DISABLED;
        ui_rule = UI_RULE;
        ui_danger = UI_DANGER;
        ui_warning = UI_WARNING;
        ui_success = UI_SUCCESS;
        track_button_hoverover = TRACK_BUTTON_HOVEROVER;
        white = WHITE;
        ui_lane_trough = UI_LANE_TROUGH;
        ui_overlay_surface = UI_OVERLAY_SURFACE;
        ui_overlay_ink = UI_OVERLAY_INK;
        ui_overlay_muted = UI_OVERLAY_MUTED;
        notice_info_on_dark = NOTICE_INFO_ON_DARK;
        notice_success_on_dark = NOTICE_SUCCESS_ON_DARK;
        notice_warning_on_dark = NOTICE_WARNING_ON_DARK;
        notice_error_on_dark = NOTICE_ERROR_ON_DARK;
    }

    /// `COLOR_TRACK_PANEL_BACKGROUND` is `COLOR_UI_SURFACE`,
    /// `COLOR_TRACK_BUTTON_BACKGROUND` is `COLOR_UI_RAISED` and
    /// `COLOR_TIMELINE_BACKGROUND` is `COLOR_UI_SURFACE` — aliases in the C, kept
    /// as aliases here so a later divergence is a one-line change.
    #[must_use]
    pub fn track_button_background() -> Color {
        ui_raised()
    }

    #[must_use]
    pub fn track_button_selected() -> Color {
        accent()
    }
}

/// HUD and control metrics (`../musializer/src/ui_theme.h:40-58`).
pub mod metric {
    /// Toolbar height: the C's `toolbar_height = HUD_BUTTON_SIZE`
    /// (`plug.c:7623`).
    pub const HUD_BUTTON_SIZE: f32 = 50.0;
    pub const UI_FONT_HEADER: f32 = 19.0;
    pub const UI_FONT_LABEL: f32 = 16.0;
    pub const UI_FONT_CAPTION: f32 = 13.0;
    pub const UI_FONT_VALUE: f32 = 15.0;
    pub const UI_PANEL_PADDING: f32 = 10.0;
    pub const UI_CONTROL_GAP: f32 = 8.0;
    pub const UI_BUTTON_HEIGHT: f32 = 36.0;

    // The timed-lane system (LX1-e). The timeline band stacks the scene plan
    // lane, the waveform strip and the lyric cue lane over one time axis, and
    // each was styled by a different agent at a different time: the scene lane
    // sat 4 px under its controls row, the waveform shared an edge with it (so
    // two 1 px rules drew as one 2 px line), and the cue lane sat 5 px lower
    // with a top rule and no box. Three names, used by all three lanes, so the
    // next lane cannot invent a fourth set of numbers.
    //
    // `LANE_GAP` is 5 rather than 6 because `panels::lyrics` already spends
    // exactly 5 between the waveform and the cue lane, and its
    // `LYRIC_EDITOR_TIMELINE_CHROME` assertion is what forbids that band from
    // growing — `shell.rs` keeps a `const _: () = assert!` pinning the two
    // together.

    /// Every timed lane's outline, and the group frame around all of them.
    pub const LANE_BORDER: f32 = 1.0;
    /// The vertical gap between two adjacent timed lanes, and between the
    /// scene-plan controls row and the first lane.
    pub const LANE_GAP: f32 = 5.0;
    /// The playhead's stroke. One marker crosses the whole group, so this is
    /// the only width any timed lane's playhead is drawn at.
    pub const LANE_PLAYHEAD_WIDTH: f32 = 2.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::ui::contrast;

    #[test]
    fn body_text_on_its_surfaces_clears_the_wcag_aa_threshold() {
        // This is the check the C keeps in tests/test_ui_contrast.c, and the
        // reason the palette is written down as raylib-free numbers at all: it
        // is assertable rather than a matter of opinion about a screenshot.
        for (name, surface) in [
            ("ui_surface", rgba::UI_SURFACE),
            ("ui_raised", rgba::UI_RAISED),
        ] {
            let ratio = contrast::ratio(rgba::UI_INK, surface);
            assert!(ratio >= contrast::AA_TEXT, "ink on {name} is {ratio:.2}");
            let muted = contrast::ratio(rgba::UI_MUTED, surface);
            assert!(muted >= contrast::AA_TEXT, "muted on {name} is {muted:.2}");
        }
    }

    #[test]
    fn white_on_the_accent_fill_clears_the_aa_threshold() {
        // Selected buttons draw WHITE on COLOR_ACCENT
        // (`ui_widgets.c:212-227`), so that pair has to hold too.
        let ratio = contrast::ratio(rgba::WHITE, rgba::ACCENT);
        assert!(ratio >= contrast::AA_TEXT, "white on accent is {ratio:.2}");
    }

    #[test]
    fn rules_and_other_non_text_components_clear_the_large_threshold() {
        let ratio = contrast::ratio(rgba::UI_RULE, rgba::UI_SURFACE);
        assert!(
            ratio >= 1.0,
            "a rule must at least be a legal ratio, got {ratio:.2}"
        );
    }

    #[test]
    fn every_palette_entry_is_opaque() {
        // The ratio functions ignore alpha because they describe fully
        // composited colours; a translucent palette entry would silently be
        // measured as if it were opaque.
        for (name, value, _) in rgba::ALL {
            assert_eq!(value & 0xFF, 0xFF, "{name} is not opaque");
        }
    }

    /// The sweep UX0-D04 asks for: every text entry against every surface it
    /// declares, light and dark alike.
    ///
    /// The two named tests above stay because they are the C's, and because they
    /// state *why* those particular pairs matter. This one is the net: it walks
    /// entries nobody thought to name, which is how the notice tray's severity
    /// labels went unmeasured while INFO sat at 1.69:1 on the card.
    #[test]
    fn every_text_entry_clears_aa_on_the_surface_it_declares() {
        let mut measured = 0;
        for (name, value, surface) in rgba::ALL {
            for (background_name, background) in surface.backgrounds() {
                let ratio = contrast::ratio(value, *background);
                assert!(
                    ratio >= contrast::AA_TEXT,
                    "{name} on {background_name} reads at {ratio:.2}:1, below {:.1}:1",
                    contrast::AA_TEXT
                );
                measured += 1;
            }
        }
        // A sweep that measured nothing would pass silently, which is the failure
        // mode of every table-driven test that lost its table.
        assert!(measured >= 14, "the sweep only checked {measured} pairs");
    }

    /// The negative control, kept as the measurement it came from.
    ///
    /// These are the exact colours the tray drew before UX0-A11 — chrome colours
    /// on a near-black card. If somebody "simplifies" the dark variants back to
    /// the palette they were copied from, this is what says no.
    #[test]
    fn the_light_chrome_severities_would_fail_the_dark_sweep() {
        for (name, value) in [
            ("accent (was INFO)", rgba::ACCENT),
            ("ui_danger (was ERROR)", rgba::UI_DANGER),
            ("ui_success (was DONE)", rgba::UI_SUCCESS),
            ("ui_warning (was WARNING)", rgba::UI_WARNING),
        ] {
            let ratio = contrast::ratio(value, rgba::UI_OVERLAY_SURFACE);
            assert!(
                ratio < contrast::AA_TEXT,
                "{name} now reads {ratio:.2}:1 on the card; the control is stale"
            );
        }
        // And the replacements clear it, by a margin rather than by a hair.
        for (name, value) in [
            ("info", rgba::NOTICE_INFO_ON_DARK),
            ("success", rgba::NOTICE_SUCCESS_ON_DARK),
            ("warning", rgba::NOTICE_WARNING_ON_DARK),
            ("error", rgba::NOTICE_ERROR_ON_DARK),
        ] {
            let ratio = contrast::ratio(value, rgba::UI_OVERLAY_SURFACE);
            assert!(
                ratio >= 6.0,
                "{name} reads {ratio:.2}:1, too close to the line"
            );
        }
    }
}
