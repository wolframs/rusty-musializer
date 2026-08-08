//! Per-frame caption effect resolution: glow pulse, hue drive, soft shadow.
//!
//! First post-legacy feature (operator decision, 2026-08-03) — there is no C
//! counterpart to cite. Everything here is a pure function of the frame's
//! audio figures and the authored [`CaptionEffects`], which is what makes an
//! export reproduce the preview exactly: both call [`resolve`] with the same
//! [`EffectInputs`] built from the same [`crate::scene::SceneAudioFrame`].
//!
//! The renderer stays dumb on purpose. It receives resolved pixels-and-colours
//! ([`ResolvedCaptionFx`]), so the whole of "what does the music do to the
//! glow this frame" is testable here without a window. Both the glow and the
//! soft shadow are drawn from the same Gaussian blur of the glyph coverage
//! (`runtime::halo`, UX0-C11) — additively for the halo, as a
//! luminance-masked tint for the penumbra.

use crate::project::model::{CaptionEffects, EffectDrive};

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

/// The application's one definition of "bass": the mean of the lowest quarter of
/// the *smoothed* band memory.
///
/// Public and named because a second scene now reads it. Phosphor Dream's audio
/// coupling used to take the lowest eighth of the instantaneous `bands` instead,
/// which read `0.00` on every frame of the synthetic fixture while the caption
/// glow on the same frames was driven fine — two numbers called "bass" in one
/// application, disagreeing. The trails are already smoothed, which is why a
/// bass-driven glow breathes instead of flickering, and that is as true for a
/// camera zoom as for a halo.
#[must_use]
pub fn bass_from_trails(trails: &[f32]) -> f32 {
    if trails.is_empty() {
        return 0.0;
    }
    let low = (trails.len() / 4).max(1).min(trails.len());
    trails[..low].iter().sum::<f32>() / low as f32
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
        Self {
            time_seconds,
            rms,
            bass: bass_from_trails(trails),
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
        // One triangle definition for the whole application: scene routes'
        // `AnalysisSource::Time` reads the same clock, so "Time" peaks on the
        // same frame everywhere.
        EffectDrive::Time => crate::scene::routes::time_triangle(inputs.time_seconds) as f32,
    }
}

/// The glow the renderer should draw this frame, already modulated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedGlow {
    /// Hue-shifted colour whose alpha **is** the final intensity. The renderer
    /// tints the blurred halo with it exactly once; it must not scale it
    /// again.
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
        let tuned = effects
            .pulse_tuning
            .apply(f64::from(drive_value(effects.glow_pulse, inputs))) as f32;
        strength * (1.0 - depth + depth * tuned)
    };
    let base_alpha = (effects.glow_rgba & 0xFF) as f32 / 255.0;
    let alpha = clamp01(intensity * base_alpha);
    if alpha < 1.0 / 255.0 {
        return None;
    }
    let shifted = if effects.glow_hue_drive == EffectDrive::None {
        effects.glow_rgba
    } else {
        let tuned = effects
            .hue_tuning
            .apply(f64::from(drive_value(effects.glow_hue_drive, inputs)))
            as f32;
        let shift = (effects.glow_hue_range as f32) * tuned;
        hue_drive_rgba(effects.glow_rgba, shift, tuned)
    };
    let rgba = (shifted & 0xFFFF_FF00) | (alpha * 255.0).round() as u32;
    Some(ResolvedGlow {
        rgba,
        radius_px: (effects.glow_radius as f32) * font_size_px,
    })
}

/// Rotates an `0xRRGGBBAA` colour's hue by `degrees`, preserving saturation,
/// value and alpha.
///
/// Hand-rolled rather than raylib's `ColorFromHSV` because this crate is
/// raylib-free and because the export path must not depend on GPU-side colour
/// math. Standard hexcone conversion; the round trip is pinned by tests.
#[must_use]
pub fn hue_shift_rgba(rgba: u32, degrees: f32) -> u32 {
    hue_drive_rgba(rgba, degrees, 0.0)
}

/// [`hue_shift_rgba`] with a drive: the hue rotates by `degrees` **and** the
/// drive blends saturation toward full — and, in proportion to how achromatic
/// the base is, value too.
///
/// This is the UX0-C13 fix. A pure rotation is the identity on white, grey and
/// black, so an authored achromatic glow silently ignored its hue drive — the
/// author saw a working control do nothing. With the drive in the colour math,
/// an achromatic base *sweeps into* colour as the drive rises and returns to
/// the authored colour in silence: white pulses to a full hue, grey to a
/// bright one, black blooms from nothing. A fully saturated base is unchanged
/// (the blend is the identity at saturation 1), so authored vivid colours keep
/// their landed behaviour exactly.
///
/// Both blends are continuous and monotonic in the base saturation, so
/// dragging the picker through "almost grey" cannot pop.
#[must_use]
pub fn hue_drive_rgba(rgba: u32, degrees: f32, drive: f32) -> u32 {
    let drive = drive.clamp(0.0, 1.0);
    if degrees == 0.0 && drive == 0.0 {
        return rgba;
    }
    let r = ((rgba >> 24) & 0xFF) as f32 / 255.0;
    let g = ((rgba >> 16) & 0xFF) as f32 / 255.0;
    let b = ((rgba >> 8) & 0xFF) as f32 / 255.0;
    let alpha = rgba & 0xFF;

    let maximum = r.max(g).max(b);
    let minimum = r.min(g).min(b);
    let chroma = maximum - minimum;
    // Achromatic bases take hue 0 (red) as the sweep's origin; without a drive
    // the saturation blend below is the identity there, so greys the author
    // picked as greys stay greys — the pre-C13 behaviour.
    let hue = if chroma == 0.0 {
        0.0
    } else if maximum == r {
        60.0 * (((g - b) / chroma).rem_euclid(6.0))
    } else if maximum == g {
        60.0 * ((b - r) / chroma + 2.0)
    } else {
        60.0 * ((r - g) / chroma + 4.0)
    };
    let saturation = if maximum > 0.0 { chroma / maximum } else { 0.0 };
    let value = maximum;

    let saturation = saturation + (1.0 - saturation) * drive;
    // Value lifts only as far as the base lacks saturation: a dark *saturated*
    // red keeps its darkness, while black — where a saturation blend alone
    // still yields black — blooms toward full brightness with the drive.
    let value = value + (1.0 - value) * drive * (1.0 - chroma);

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
    fn a_driven_hue_saturates_achromatic_bases_instead_of_ignoring_them() {
        // White at full drive sweeps to the full hue at 120°: pure green.
        assert_eq!(hue_drive_rgba(0xFFFF_FFFF, 120.0, 1.0), 0x00FF_00FF);
        // Black blooms toward the bright hue rather than staying invisible.
        assert_eq!(hue_drive_rgba(0x0000_00FF, 240.0, 1.0), 0x0000_FFFF);
        // No drive, no change: the authored grey stays the authored grey.
        assert_eq!(hue_drive_rgba(0x8080_80C0, 0.0, 0.0), 0x8080_80C0);
        // A fully saturated base is untouched by the blend — landed behaviour.
        assert_eq!(hue_drive_rgba(0xFF00_00FF, 0.0, 1.0), 0xFF00_00FF);
        // Half drive on white is halfway into the sweep, not a pop.
        let half = hue_drive_rgba(0xFFFF_FFFF, 0.0, 0.5);
        assert_eq!(half, 0xFF80_80FF, "half-saturated red keeps full value");
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
    fn drive_tuning_shapes_the_pulse_with_route_mapping_semantics() {
        use crate::project::model::DriveTuning;
        use crate::scene::routes::Interpolation;
        // A window of 0.5..1.0 mapped to 0..1: an RMS of 0.25 (drive 0.5 after
        // the square root) sits at the window's floor and the pulse at full
        // depth goes dark; the identity default would have kept it half lit.
        let effects = CaptionEffects {
            glow_strength: 1.0,
            glow_pulse: EffectDrive::Rms,
            glow_pulse_depth: 1.0,
            pulse_tuning: DriveTuning {
                input_min: 0.5,
                input_max: 1.0,
                output_min: 0.0,
                output_max: 1.0,
                curve: Interpolation::Linear,
                clamp: true,
            },
            ..CaptionEffects::default()
        };
        let quarter = EffectInputs {
            rms: 0.25,
            ..quiet()
        };
        assert_eq!(resolve(&effects, &quarter, 64.0).glow, None, "tuned dark");
        let identity = CaptionEffects {
            pulse_tuning: DriveTuning::default(),
            ..effects.clone()
        };
        assert!(resolve(&identity, &quarter, 64.0).glow.is_some());

        // A swapped hue tuning (1 -> 0) inverts the sweep: silence is the far
        // end of the range, full drive is the authored colour.
        let swapped = CaptionEffects {
            glow_strength: 1.0,
            glow_hue_drive: EffectDrive::Rms,
            glow_hue_range: 120.0,
            glow_rgba: 0xFF00_00FF,
            hue_tuning: DriveTuning {
                output_min: 1.0,
                output_max: 0.0,
                ..DriveTuning::default()
            },
            ..CaptionEffects::default()
        };
        let loud = EffectInputs {
            rms: 1.0,
            ..quiet()
        };
        let glow = resolve(&swapped, &loud, 64.0).glow.expect("glow on");
        assert_eq!(glow.rgba, 0xFF00_00FF, "full drive maps to amount 0");
        let glow = resolve(&swapped, &quiet(), 64.0).glow.expect("glow on");
        assert_eq!(glow.rgba, 0x00FF_00FF, "silence maps to the full sweep");
    }
}
