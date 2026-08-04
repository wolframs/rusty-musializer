//! Per-frame caption effect resolution: glow pulse, hue drive, soft shadow.
//!
//! First post-legacy feature (operator decision, 2026-08-03) — there is no C
//! counterpart to cite. Everything here is a pure function of the frame's
//! audio figures and the authored [`CaptionEffects`], which is what makes an
//! export reproduce the preview exactly: both call [`resolve`] with the same
//! [`EffectInputs`] built from the same [`crate::scene::SceneAudioFrame`].
//!
//! The renderer stays dumb on purpose. It receives resolved pixels-and-colours
//! ([`ResolvedCaptionFx`]) plus a fixed tap table ([`GLOW_TAPS`]), so the whole
//! of "what does the music do to the glow this frame" is testable here without
//! a window.

use crate::project::model::{CaptionEffects, EffectDrive};
use std::f32::consts::FRAC_1_SQRT_2;

/// The audio figures an effect drive may read, plus the deterministic clock.
///
/// Built once per frame from the scene frame's audio. `bass` is derived by
/// [`EffectInputs::from_audio`] rather than passed raw so every caller agrees
/// on what "bass" means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectInputs {
    pub time_seconds: f64,
    pub rms: f32,
    pub bass: f32,
    pub beat_phase: f32,
    pub flux: f32,
}

impl EffectInputs {
    /// Derives the drive inputs from a frame's audio figures.
    ///
    /// `bass` is the mean of the lowest quarter of `trails` — the *smoothed*
    /// band memory, not the instantaneous bands, so a bass-driven glow breathes
    /// instead of flickering.
    #[must_use]
    pub fn from_audio(
        time_seconds: f64,
        rms: f32,
        trails: &[f32],
        beat_phase: f32,
        flux: f32,
    ) -> Self {
        let low = (trails.len() / 4).max(1).min(trails.len().max(1));
        let bass = if trails.is_empty() {
            0.0
        } else {
            trails[..low].iter().sum::<f32>() / low as f32
        };
        Self {
            time_seconds,
            rms,
            bass,
            beat_phase,
            flux,
        }
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// One drive's 0..=1 value for this frame.
///
/// [`EffectDrive::None`] is 0 here; the *meaning* of "no drive" differs between
/// pulse (steady full intensity) and hue (no shift), so each use site owns it.
///
/// - `Rms`/`Bass` are square-rooted: band amplitudes are already normalized but
///   sit low in the range, and the root makes the audible loudness ramp read as
///   a visible ramp.
/// - `Beat` is `(1 - phase)^3`: full at the beat, decayed sharply across the
///   interval — a pulse, not a sine.
/// - `Flux` carries a x4 gain before the clamp; the per-band positive excursion
///   is small even in busy material.
/// - `Time` is a triangle wave over eight seconds, so a bounded hue range
///   sweeps back and forth continuously instead of snapping at the wrap.
#[must_use]
pub fn drive_value(drive: EffectDrive, inputs: &EffectInputs) -> f32 {
    match drive {
        EffectDrive::None => 0.0,
        EffectDrive::Rms => clamp01(inputs.rms).sqrt(),
        EffectDrive::Bass => clamp01(inputs.bass).sqrt(),
        EffectDrive::Beat => {
            let remaining = 1.0 - clamp01(inputs.beat_phase);
            remaining * remaining * remaining
        }
        EffectDrive::Flux => clamp01(inputs.flux * 4.0),
        EffectDrive::Time => {
            let cycle = (inputs.time_seconds / 8.0).fract() as f32;
            1.0 - (2.0 * cycle - 1.0).abs()
        }
    }
}

/// The glow the renderer should draw this frame, already modulated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedGlow {
    /// Hue-shifted colour whose alpha **is** the final intensity. The renderer
    /// splits it across [`GLOW_TAPS`] and draws additively; it must not scale
    /// it again.
    pub rgba: u32,
    /// Halo radius in pixels at the resolved font size.
    pub radius_px: f32,
}

/// Everything the caption renderer needs beyond the legacy style fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedCaptionFx {
    /// `None` when the glow is off or modulated to invisibility this frame.
    pub glow: Option<ResolvedGlow>,
    /// 0 keeps the legacy hard-offset shadow.
    pub shadow_blur_px: f32,
    /// Multiplies the shadow colour's alpha. 1.0 is legacy.
    pub shadow_alpha_scale: f32,
    /// `DrawRectangleRounded` roundness for the plate and its outline.
    pub plate_roundness: f32,
}

/// Resolves the authored effects against one frame's inputs.
#[must_use]
pub fn resolve(
    effects: &CaptionEffects,
    inputs: &EffectInputs,
    font_size_px: f32,
) -> ResolvedCaptionFx {
    let glow = resolve_glow(effects, inputs, font_size_px);
    ResolvedCaptionFx {
        glow,
        shadow_blur_px: (effects.shadow_blur as f32) * font_size_px,
        shadow_alpha_scale: effects.shadow_opacity as f32,
        plate_roundness: effects.plate_roundness as f32,
    }
}

fn resolve_glow(
    effects: &CaptionEffects,
    inputs: &EffectInputs,
    font_size_px: f32,
) -> Option<ResolvedGlow> {
    let strength = effects.glow_strength as f32;
    if strength <= 0.0 || font_size_px <= 0.0 {
        return None;
    }
    let intensity = if effects.glow_pulse == EffectDrive::None {
        strength
    } else {
        let depth = clamp01(effects.glow_pulse_depth as f32);
        strength * (1.0 - depth + depth * drive_value(effects.glow_pulse, inputs))
    };
    let base_alpha = (effects.glow_rgba & 0xFF) as f32 / 255.0;
    let alpha = clamp01(intensity * base_alpha);
    if alpha < 1.0 / 255.0 {
        return None;
    }
    let shift = if effects.glow_hue_drive == EffectDrive::None {
        0.0
    } else {
        (effects.glow_hue_range as f32) * drive_value(effects.glow_hue_drive, inputs)
    };
    let shifted = hue_shift_rgba(effects.glow_rgba, shift);
    let rgba = (shifted & 0xFFFF_FF00) | (alpha * 255.0).round() as u32;
    Some(ResolvedGlow {
        rgba,
        radius_px: (effects.glow_radius as f32) * font_size_px,
    })
}

/// One additive glow tap: a unit offset (scaled by the radius) and its share of
/// the resolved alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowTap {
    pub dx: f32,
    pub dy: f32,
    pub weight: f32,
}

/// Two rings of eight plus a centre tap.
///
/// The outer ring sits at the full radius, the inner at 0.55 of it and rotated
/// 22.5° so the two never line up into visible spokes. Weights sum to ~1.5
/// rather than 1.0: additive blending saturates against bright pixels, and the
/// headroom is what makes the halo's core read as hot instead of washed flat.
/// The renderer multiplies each weight by the resolved alpha per tap.
#[rustfmt::skip]
pub const GLOW_TAPS: [GlowTap; 17] = [
    GlowTap { dx:  1.0,        dy:  0.0,        weight: 0.075 },
    GlowTap { dx:  FRAC_1_SQRT_2, dy:  FRAC_1_SQRT_2, weight: 0.075 },
    GlowTap { dx:  0.0,        dy:  1.0,        weight: 0.075 },
    GlowTap { dx: -FRAC_1_SQRT_2, dy:  FRAC_1_SQRT_2, weight: 0.075 },
    GlowTap { dx: -1.0,        dy:  0.0,        weight: 0.075 },
    GlowTap { dx: -FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2, weight: 0.075 },
    GlowTap { dx:  0.0,        dy: -1.0,        weight: 0.075 },
    GlowTap { dx:  FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2, weight: 0.075 },
    GlowTap { dx:  0.5081329, dy:  0.21048355, weight: 0.1   },
    GlowTap { dx:  0.21048355, dy:  0.5081329, weight: 0.1   },
    GlowTap { dx: -0.21048355, dy:  0.5081329, weight: 0.1   },
    GlowTap { dx: -0.5081329, dy:  0.21048355, weight: 0.1   },
    GlowTap { dx: -0.5081329, dy: -0.21048355, weight: 0.1   },
    GlowTap { dx: -0.21048355, dy: -0.5081329, weight: 0.1   },
    GlowTap { dx:  0.21048355, dy: -0.5081329, weight: 0.1   },
    GlowTap { dx:  0.5081329, dy: -0.21048355, weight: 0.1   },
    GlowTap { dx:  0.0,        dy:  0.0,        weight: 0.1   },
];

/// Nine taps for the soft shadow: the same outer ring at reduced weight plus a
/// dominant centre, drawn with normal (not additive) blending, so a blurred
/// shadow stays a shadow rather than a dark glow.
#[rustfmt::skip]
pub const SHADOW_TAPS: [GlowTap; 9] = [
    GlowTap { dx:  1.0,        dy:  0.0,        weight: 0.09 },
    GlowTap { dx:  FRAC_1_SQRT_2, dy:  FRAC_1_SQRT_2, weight: 0.09 },
    GlowTap { dx:  0.0,        dy:  1.0,        weight: 0.09 },
    GlowTap { dx: -FRAC_1_SQRT_2, dy:  FRAC_1_SQRT_2, weight: 0.09 },
    GlowTap { dx: -1.0,        dy:  0.0,        weight: 0.09 },
    GlowTap { dx: -FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2, weight: 0.09 },
    GlowTap { dx:  0.0,        dy: -1.0,        weight: 0.09 },
    GlowTap { dx:  FRAC_1_SQRT_2, dy: -FRAC_1_SQRT_2, weight: 0.09 },
    GlowTap { dx:  0.0,        dy:  0.0,        weight: 0.28 },
];

/// Rotates an `0xRRGGBBAA` colour's hue by `degrees`, preserving saturation,
/// value and alpha.
///
/// Hand-rolled rather than raylib's `ColorFromHSV` because this crate is
/// raylib-free and because the export path must not depend on GPU-side colour
/// math. Standard hexcone conversion; the round trip is pinned by tests.
#[must_use]
pub fn hue_shift_rgba(rgba: u32, degrees: f32) -> u32 {
    if degrees == 0.0 {
        return rgba;
    }
    let r = ((rgba >> 24) & 0xFF) as f32 / 255.0;
    let g = ((rgba >> 16) & 0xFF) as f32 / 255.0;
    let b = ((rgba >> 8) & 0xFF) as f32 / 255.0;
    let alpha = rgba & 0xFF;

    let maximum = r.max(g).max(b);
    let minimum = r.min(g).min(b);
    let chroma = maximum - minimum;
    let hue = if chroma == 0.0 {
        // Achromatic: hue rotation is the identity, and inventing a hue here
        // would tint greys the author picked as greys.
        return rgba;
    } else if maximum == r {
        60.0 * (((g - b) / chroma).rem_euclid(6.0))
    } else if maximum == g {
        60.0 * ((b - r) / chroma + 2.0)
    } else {
        60.0 * ((r - g) / chroma + 4.0)
    };
    let saturation = chroma / maximum;
    let value = maximum;

    let hue = (hue + degrees).rem_euclid(360.0);
    let c = value * saturation;
    let x = c * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match hue {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let channel = |v: f32| (((v + m) * 255.0).round() as u32).min(255);
    (channel(r) << 24) | (channel(g) << 16) | (channel(b) << 8) | alpha
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::model::caption_fx;

    fn quiet() -> EffectInputs {
        EffectInputs {
            time_seconds: 0.0,
            rms: 0.0,
            bass: 0.0,
            beat_phase: 0.5,
            flux: 0.0,
        }
    }

    #[test]
    fn default_effects_resolve_to_the_legacy_composition() {
        let resolved = resolve(&CaptionEffects::default(), &quiet(), 64.0);
        assert_eq!(resolved.glow, None);
        assert_eq!(resolved.shadow_blur_px, 0.0);
        assert_eq!(resolved.shadow_alpha_scale, 1.0);
        assert!((resolved.plate_roundness - 0.12).abs() < 1e-6);
    }

    #[test]
    fn steady_glow_ignores_pulse_depth_when_no_drive_is_chosen() {
        let effects = CaptionEffects {
            glow_strength: 1.0,
            glow_pulse_depth: 1.0,
            ..CaptionEffects::default()
        };
        let glow = resolve(&effects, &quiet(), 100.0).glow.expect("glow on");
        assert_eq!(glow.rgba & 0xFF, 0xFF, "full strength keeps full alpha");
        assert!((glow.radius_px - 18.0).abs() < 1e-4, "0.18 of 100 px");
    }

    #[test]
    fn an_rms_pulse_at_full_depth_goes_dark_in_silence_and_full_at_peak() {
        let effects = CaptionEffects {
            glow_strength: 1.0,
            glow_pulse: EffectDrive::Rms,
            glow_pulse_depth: 1.0,
            ..CaptionEffects::default()
        };
        assert_eq!(resolve(&effects, &quiet(), 64.0).glow, None, "silence");
        let loud = EffectInputs {
            rms: 1.0,
            ..quiet()
        };
        let glow = resolve(&effects, &loud, 64.0).glow.expect("loud frame");
        assert_eq!(glow.rgba & 0xFF, 0xFF);
    }

    #[test]
    fn the_beat_drive_peaks_on_the_beat_and_decays() {
        let on_beat = EffectInputs {
            beat_phase: 0.0,
            ..quiet()
        };
        let late = EffectInputs {
            beat_phase: 0.9,
            ..quiet()
        };
        assert!((drive_value(EffectDrive::Beat, &on_beat) - 1.0).abs() < 1e-6);
        assert!(drive_value(EffectDrive::Beat, &late) < 0.01);
    }

    #[test]
    fn the_time_drive_is_a_continuous_triangle() {
        let at = |t: f64| {
            drive_value(
                EffectDrive::Time,
                &EffectInputs {
                    time_seconds: t,
                    ..quiet()
                },
            )
        };
        assert!((at(0.0) - 0.0).abs() < 1e-6);
        assert!((at(4.0) - 1.0).abs() < 1e-6);
        assert!((at(8.0) - 0.0).abs() < 1e-6);
        // Continuity across the wrap: the two sides of t=8 agree.
        assert!((at(7.999) - at(8.001)).abs() < 2e-3);
    }

    #[test]
    fn hue_shift_rotates_red_to_green_to_blue_and_back() {
        let red = 0xFF00_00FF;
        assert_eq!(hue_shift_rgba(red, 120.0), 0x00FF_00FF);
        assert_eq!(hue_shift_rgba(red, 240.0), 0x0000_FFFF);
        assert_eq!(hue_shift_rgba(red, 360.0), red);
        assert_eq!(hue_shift_rgba(red, 0.0), red);
    }

    #[test]
    fn hue_shift_leaves_greys_and_alpha_alone() {
        assert_eq!(hue_shift_rgba(0x8080_80C0, 90.0), 0x8080_80C0);
        assert_eq!(hue_shift_rgba(0xFFC8_64B7, 45.0) & 0xFF, 0xB7);
    }

    #[test]
    fn resolved_alpha_is_strength_times_the_authored_alpha() {
        let effects = CaptionEffects {
            glow_strength: 0.5,
            glow_rgba: 0xFFFF_FF80,
            ..CaptionEffects::default()
        };
        let glow = resolve(&effects, &quiet(), 64.0).glow.expect("glow on");
        // 0.5 * (0x80/255) = 0.2510 -> 64
        assert_eq!(glow.rgba & 0xFF, 64);
    }

    #[test]
    fn bass_comes_from_the_low_quarter_of_the_trails() {
        let mut trails = vec![0.0f32; 16];
        trails[..4].copy_from_slice(&[0.8, 0.8, 0.8, 0.8]);
        let inputs = EffectInputs::from_audio(0.0, 0.0, &trails, 0.0, 0.0);
        assert!((inputs.bass - 0.8).abs() < 1e-6);
        let empty = EffectInputs::from_audio(0.0, 0.0, &[], 0.0, 0.0);
        assert_eq!(empty.bass, 0.0);
    }

    #[test]
    fn every_effect_bound_admits_its_own_default() {
        assert!(CaptionEffects::default().validate());
        let full = CaptionEffects {
            glow_strength: caption_fx::GLOW_STRENGTH_MAXIMUM,
            glow_radius: caption_fx::GLOW_RADIUS_MAXIMUM,
            glow_pulse: EffectDrive::Flux,
            glow_pulse_depth: caption_fx::PULSE_DEPTH_MAXIMUM,
            glow_hue_drive: EffectDrive::Time,
            glow_hue_range: caption_fx::HUE_RANGE_MAXIMUM,
            shadow_blur: caption_fx::SHADOW_BLUR_MAXIMUM,
            shadow_opacity: caption_fx::SHADOW_OPACITY_MAXIMUM,
            plate_roundness: caption_fx::PLATE_ROUNDNESS_MAXIMUM,
            ..CaptionEffects::default()
        };
        assert!(full.validate());
        assert!(!CaptionEffects {
            glow_strength: 1.1,
            ..CaptionEffects::default()
        }
        .validate());
        assert!(!CaptionEffects {
            shadow_blur: f64::NAN,
            ..CaptionEffects::default()
        }
        .validate());
    }

    #[test]
    fn glow_tap_weights_carry_the_documented_headroom() {
        let total: f32 = GLOW_TAPS.iter().map(|tap| tap.weight).sum();
        assert!(
            (total - 1.5).abs() < 1e-4,
            "additive headroom is 1.5, got {total}"
        );
        let shadow: f32 = SHADOW_TAPS.iter().map(|tap| tap.weight).sum();
        assert!(
            (shadow - 1.0).abs() < 1e-4,
            "shadow taps sum to 1, got {shadow}"
        );
    }
}
