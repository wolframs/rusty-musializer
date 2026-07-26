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

    /// Every palette entry, so a contrast sweep can iterate rather than list.
    ///
    /// Read only by the tests below. That is the point of it: the C header warns
    /// that "a constant defined only there is invisible to the contrast suite",
    /// and a list the suite walks is how a colour added without a contrast check
    /// gets caught.
    #[allow(
        dead_code,
        reason = "the palette audit in this module's tests is its only reader"
    )]
    pub const ALL: [(&str, u32); 13] = [
        ("accent", ACCENT),
        ("background", BACKGROUND),
        ("ui_surface", UI_SURFACE),
        ("ui_raised", UI_RAISED),
        ("ui_ink", UI_INK),
        ("ui_muted", UI_MUTED),
        ("ui_disabled", UI_DISABLED),
        ("ui_rule", UI_RULE),
        ("ui_danger", UI_DANGER),
        ("ui_warning", UI_WARNING),
        ("ui_success", UI_SUCCESS),
        ("track_button_hoverover", TRACK_BUTTON_HOVEROVER),
        ("white", WHITE),
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
        for (name, value) in rgba::ALL {
            assert_eq!(value & 0xFF, 0xFF, "{name} is not opaque");
        }
    }
}
