//! WCAG 2.1 relative luminance and contrast ratio over packed `0xRRGGBBAA`
//! colours.
//!
//! **Owner: Agent F.** Port of `ui_contrast.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! Deliberately raylib-free (`ui_contrast.h:7-9`), and that is the entire point:
//! it lets the palette be checked in a headless test rather than by eye. The
//! amber severity label shipped at 3.96:1 until this suite caught it.
//!
//! Alpha is ignored by [`relative_luminance`] and [`ratio`]
//! (`ui_contrast.h:14-15`): they describe *fully composited* colours, so a
//! caller drawing something translucent must composite it with [`blend`] first
//! and pass the result, never the source value.
//!
//! Everything is `f64`, matching C's `double`. The thresholds are decided at two
//! decimal places — `0x777777` on white lands at 4.48:1, just under the 4.5 bar
//! — so narrowing the arithmetic would move real pass/fail verdicts.

/// WCAG 1.4.3 AA minimum for normal-size body text (`ui_contrast.h:12`).
///
/// Named so call sites and tests say what the number means instead of repeating
/// `4.5`.
pub const AA_TEXT: f64 = 4.5;

/// WCAG 1.4.11 minimum for large text and for non-text UI components such as
/// rules and borders (`ui_contrast.h:13`).
pub const AA_COMPONENT: f64 = 3.0;

/// One sRGB channel, gamma-expanded (`ui_contrast.c:5-11`).
///
/// The `0.03928` knee is WCAG 2.1's own number. Later editions of the formula
/// print `0.04045`; do not "correct" it, the threshold values recorded in the
/// palette tests were measured against this one.
fn channel(rgba: u32, shift: u32) -> f64 {
    let value = f64::from((rgba >> shift) & 0xFF) / 255.0;
    if value <= 0.03928 {
        return value / 12.92;
    }
    ((value + 0.055) / 1.055).powf(2.4)
}

/// Relative luminance: 0.0 for the darkest colour, 1.0 for the brightest
/// (`ui_contrast.c:13-18`). The alpha byte is ignored.
pub fn relative_luminance(rgba: u32) -> f64 {
    0.2126 * channel(rgba, 24) + 0.7152 * channel(rgba, 16) + 0.0722 * channel(rgba, 8)
}

/// Contrast ratio between two composited colours (`ui_contrast.c:20-27`).
///
/// Symmetric in its arguments, and ranges from 1.0 to 21.0. Alpha is ignored;
/// see the module docs.
pub fn ratio(foreground: u32, background: u32) -> f64 {
    let first = relative_luminance(foreground);
    let second = relative_luminance(background);
    let lighter = if first > second { first } else { second };
    let darker = if first > second { second } else { first };
    (lighter + 0.05) / (darker + 0.05)
}

/// Composites `foreground` over `background` at `alpha` (0..1) and returns the
/// packed result, so a translucent overlay can be measured at the opacity it is
/// actually drawn with (`ui_contrast.c:29-43`).
///
/// The returned alpha byte is always `0xFF`: the result is opaque once
/// composited.
// The channel clamp stays as C's two `if`s rather than `clamp`, because the whole
// point of this function's clamping is that it does not propagate NaN.
#[allow(clippy::manual_clamp)]
pub fn blend(foreground: u32, background: u32, alpha: f64) -> u32 {
    // The C writes `if (!(alpha >= 0.0)) alpha = 0.0;` — the negated comparison
    // is deliberate, because it also catches NaN. `alpha.clamp(0.0, 1.0)` would
    // propagate NaN into the mix and silently change behaviour, so the negated
    // test is reproduced literally.
    let mut alpha = if alpha >= 0.0 { alpha } else { 0.0 };
    if alpha > 1.0 {
        alpha = 1.0;
    }
    // Starts at the alpha byte: the result is opaque once composited.
    let mut result = 0xFFu32;
    // C iterates `for (unsigned shift = 24; shift >= 8; shift -= 8)`, which
    // terminates only because the next value underflows an unsigned — the loop
    // visits 24, 16, 8 and then wraps out of its own condition. Spelled out as a
    // literal array here so nothing depends on wraparound.
    for shift in [24u32, 16, 8] {
        let front = f64::from((foreground >> shift) & 0xFF);
        let back = f64::from((background >> shift) & 0xFF);
        let mut mixed = front * alpha + back * (1.0 - alpha);
        if mixed < 0.0 {
            mixed = 0.0;
        }
        if mixed > 255.0 {
            mixed = 255.0;
        }
        result |= ((mixed + 0.5) as u32 & 0xFF) << shift;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors of `ui_palette.h:13-25`, kept local until a `ui::palette` module is
    // ported. The C header's rule applies here too: a colour the application
    // draws with but that is absent from this list is invisible to the contrast
    // suite.
    #[rustfmt::skip]
    mod palette {
        pub const ACCENT:                 u32 = 0x002F_A7FF;
        pub const UI_SURFACE:             u32 = 0xF7F7_F8FF;
        pub const UI_RAISED:              u32 = 0xFFFF_FFFF;
        pub const UI_INK:                 u32 = 0x1414_14FF;
        pub const UI_MUTED:               u32 = 0x6666_6BFF;
        pub const UI_DISABLED:            u32 = 0x8C8C_92FF;
        pub const UI_RULE:                u32 = 0xD2D2_D6FF;
        pub const UI_DANGER:              u32 = 0xC628_28FF;
        pub const UI_WARNING:             u32 = 0x9E5D_00FF;
        pub const UI_SUCCESS:             u32 = 0x1879_4EFF;
        pub const TRACK_BUTTON_HOVEROVER: u32 = 0xE7EA_F2FF;
        pub const WHITE:                  u32 = 0xFFFF_FFFF;
    }
    use palette::*;

    fn expect_near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// Port of the C `EXPECT_TEXT_CONTRAST` macro: names the colour pair in words
    /// so a failure reads as a design report rather than an opaque expression.
    #[track_caller]
    fn expect_text_contrast(what: &str, foreground: u32, background: u32) {
        let contrast = ratio(foreground, background);
        assert!(
            contrast >= AA_TEXT,
            "{what} reads at {contrast:.2}:1, below the {AA_TEXT:.1}:1 body-text minimum"
        );
    }

    #[test]
    fn ui_contrast_matches_the_wcag_reference_values() {
        // Anchors from the specification, so a bug in the gamma expansion cannot
        // hide behind palette values that happen to pass anyway.
        expect_near(ratio(0x0000_00FF, 0xFFFF_FFFF), 21.0, 0.001);
        expect_near(ratio(0xFFFF_FFFF, 0xFFFF_FFFF), 1.0, 0.001);
        expect_near(relative_luminance(0xFFFF_FFFF), 1.0, 0.0001);
        expect_near(relative_luminance(0x0000_00FF), 0.0, 0.0001);
        // Mid grey: the classic 0x777777 on white is just under AA.
        expect_near(ratio(0x7777_77FF, 0xFFFF_FFFF), 4.48, 0.01);
        // Symmetric in its arguments.
        expect_near(
            ratio(0x002F_A7FF, 0xF7F7_F8FF),
            ratio(0xF7F7_F8FF, 0x002F_A7FF),
            0.0001,
        );
    }

    #[test]
    fn ui_contrast_blend_composites_before_measuring() {
        // A translucent colour must be measured at the opacity it is drawn with,
        // not as its source value.
        assert_eq!(blend(0x0000_00FF, 0xFFFF_FFFF, 0.0), 0xFFFF_FFFF);
        assert_eq!(blend(0x0000_00FF, 0xFFFF_FFFF, 1.0), 0x0000_00FF);
        let half = blend(0x0000_00FF, 0xFFFF_FFFF, 0.5);
        assert_eq!((half >> 24) & 0xFF, 128);
        // Out-of-range and NaN alphas clamp rather than producing nonsense.
        assert_eq!(blend(0x0000_00FF, 0xFFFF_FFFF, -1.0), 0xFFFF_FFFF);
        assert_eq!(blend(0x0000_00FF, 0xFFFF_FFFF, 2.0), 0x0000_00FF);
        assert_eq!(blend(0x0000_00FF, 0xFFFF_FFFF, f64::NAN), 0xFFFF_FFFF);
    }

    #[test]
    fn ui_palette_body_text_clears_wcag_aa() {
        // Panel and button surfaces are the two backgrounds text lands on.
        expect_text_contrast("ink on the panel surface", UI_INK, UI_SURFACE);
        expect_text_contrast("ink on a raised control", UI_INK, UI_RAISED);
        expect_text_contrast("ink on a hovered control", UI_INK, TRACK_BUTTON_HOVEROVER);
        expect_text_contrast("muted labels on the panel surface", UI_MUTED, UI_SURFACE);
        expect_text_contrast("muted labels on a raised control", UI_MUTED, UI_RAISED);
        expect_text_contrast("accent values on the panel surface", ACCENT, UI_SURFACE);
        expect_text_contrast("accent values on a raised control", ACCENT, UI_RAISED);
        expect_text_contrast("a selected control's white label", WHITE, ACCENT);
        expect_text_contrast("tooltip text", WHITE, UI_INK);

        // Notice severity labels are drawn in these colours on the notice card,
        // which is the panel surface. The amber failed here at 3.96:1 until it was
        // darkened from 0xB26A00 to 0x9E5D00.
        expect_text_contrast("the warning severity label", UI_WARNING, UI_SURFACE);
        expect_text_contrast("the success severity label", UI_SUCCESS, UI_SURFACE);
        expect_text_contrast("danger labels on a raised control", UI_DANGER, UI_RAISED);
    }

    #[test]
    fn ui_palette_disabled_text_stays_readable_but_clearly_recessed() {
        // WCAG exempts disabled controls, so this is a house rule rather than a
        // conformance requirement: a disabled label should still be legible enough
        // to read the option you cannot pick.
        assert!(ratio(UI_DISABLED, UI_SURFACE) >= AA_COMPONENT);
        assert!(ratio(UI_DISABLED, UI_RAISED) >= AA_COMPONENT);

        // The design intent that actually matters: disabled must read as weaker
        // than muted, or "unavailable" and "secondary" become the same signal.
        assert!(ratio(UI_DISABLED, UI_SURFACE) < ratio(UI_MUTED, UI_SURFACE));
    }

    #[test]
    fn ui_palette_control_borders_are_a_recorded_deviation() {
        // PINS A KNOWN DEVIATION, not a passing standard. An enabled button is
        // UI_RAISED (white) on UI_SURFACE (near-white), which is about 1.02:1, so
        // its 1 px UI_RULE border is very nearly the only thing that identifies
        // the control. WCAG 1.4.11 asks for 3:1 there.
        //
        // Raising UI_RULE to clear it would darken every divider, rail, and panel
        // separator in the workspace, which is a visual-design decision rather
        // than a bug fix -- recorded as gate D7 in the C repo's EXTENSION_PLAN.md.
        //
        // This test exists so the deviation is measured rather than forgotten. If
        // it fails because the rule got darker, that is the gate being answered:
        // update the numbers and delete this comment.
        let on_surface = ratio(UI_RULE, UI_SURFACE);
        let on_raised = ratio(UI_RULE, UI_RAISED);
        expect_near(on_surface, 1.41, 0.02);
        expect_near(on_raised, 1.51, 0.02);
        assert!(on_surface < AA_COMPONENT);

        // A selected control does not rely on the rule: it fills with the accent.
        assert!(ratio(ACCENT, UI_SURFACE) >= AA_COMPONENT);
    }
}
