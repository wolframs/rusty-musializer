//! ASCII Field: deterministic state, update, and the pure colour pipeline.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_ascii_field.c`'s
//! `init`/`update` half plus its pure helpers (frozen at `9300af9`, read-only).
//! [`ascii_art`] is the separate raylib-free module the scene shares with the
//! image importer. The drawing half is `musializer-app::scenes::ascii_field`.
//!
//! The state is a scrolling spectrogram: 54 rows of 96 columns, sampled at a
//! fixed 18 Hz regardless of frame rate (`scene_ascii_field.c:180`), so the
//! waterfall scrolls at the same speed in a 30 fps export as in a 60 fps preview.
//! Everything else the scene animates is derived from `frame.time_seconds` at
//! draw time, which is what makes a seek land where the transport says rather
//! than where an integrated history had got to.
//!
//! The pure colour and phase helpers live here rather than beside the drawing
//! code because they are arithmetic over the frame and therefore testable
//! headlessly; only the raylib draw calls themselves are in `musializer-app`.

// C's guard clauses are kept as chains of comparisons rather than range
// `contains` calls, so each guard diffs against the line it came from.
#![allow(clippy::manual_range_contains)]

pub mod ascii_art;

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use ascii_art::{Cell, Rgba};

/// `ASCII_FIELD_MAX_COLUMNS` (`scene_ascii_field.c:8`).
pub const MAX_COLUMNS: usize = 96;
/// `ASCII_FIELD_MAX_ROWS` (`scene_ascii_field.c:9`).
pub const MAX_ROWS: usize = 54;
/// `scene_ascii_field.c:448`. Version 4: the history layout has changed three
/// times, so older stored state must be rebound rather than reinterpreted.
pub const STATE_VERSION: u32 = 4;

/// The fixed sampling cadence, `1.0/18.0` seconds (`scene_ascii_field.c:180`).
pub const SAMPLE_INTERVAL: f64 = 1.0 / 18.0;
/// A gap this large is a seek or a stall, and clears the waterfall
/// (`scene_ascii_field.c:176`).
pub const DISCONTINUITY_SECONDS: f64 = 0.75;

/// `ascii_clamp01` (`scene_ascii_field.c:18-23`): non-finite reads as 0.
#[must_use]
pub fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `ascii_lerp` (`scene_ascii_field.c:46-49`). The amount is clamped, not the
/// result.
#[must_use]
pub fn lerp(first: f32, second: f32, amount: f32) -> f32 {
    first + (second - first) * clamp01(amount)
}

/// `ascii_color_byte` (`scene_ascii_field.c:51-56`): saturating float to byte,
/// rounding at the half.
#[must_use]
pub fn color_byte(value: f32) -> u8 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    (value + 0.5) as u8
}

/// `ascii_seed_phase` (`scene_ascii_field.c:36-44`): splitmix64's finalizer, low
/// 16 bits, scaled to a full turn.
#[must_use]
pub fn seed_phase(seed: u64) -> f32 {
    let mut seed = seed;
    seed ^= seed >> 30;
    seed = seed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    seed ^= seed >> 27;
    seed = seed.wrapping_mul(0x94d0_49bb_1331_11eb);
    seed ^= seed >> 31;
    (seed & 0xffff) as f32 / 65535.0 * 2.0 * std::f32::consts::PI
}

/// Maps a grid column onto an analyzer band (`ascii_audio_band`,
/// `scene_ascii_field.c:25-34`).
///
/// The index expression is C's verbatim, integer division included: it spreads
/// `columns` cells across `bands.len()` bands without floating point, so the same
/// column always reads the same band.
#[must_use]
pub fn audio_band(bands: &[f32], column: usize, columns: usize) -> f32 {
    let count = bands.len();
    if count == 0 || columns == 0 {
        return 0.0;
    }
    let mut index = (count / columns) * column + ((count % columns) * column) / columns;
    if index >= count {
        index = count - 1;
    }
    clamp01(bands[index])
}

/// The per-cell colour (`ascii_main_color`, `scene_ascii_field.c:58-106`).
///
/// Pure arithmetic over the cell and the frame's audio, so it lives in this
/// crate; the drawing half only converts the result to a raylib `Color`.
///
/// The unusual step is the `lift`: glyph density already carries the source
/// luminance, so the source hue is boosted toward a target brightness rather than
/// multiplied by it. Multiplying would push dark ink to invisible after video
/// encoding, which is the failure the C comment describes.
#[must_use]
pub fn main_color(cell: &Cell, band: f32, energy: f32, semantic_tension: f32, gain: f32) -> Rgba {
    let luminance = clamp01(cell.luminance);
    let edge = clamp01(cell.edge_strength * 3.0);
    let detail = luminance.max(edge * 0.82);
    let mut source_r = f32::from(cell.foreground.r) / 255.0;
    let mut source_g = f32::from(cell.foreground.g) / 255.0;
    let mut source_b = f32::from(cell.foreground.b) / 255.0;
    let source_max = source_r.max(source_g.max(source_b));
    let target = 0.42 + detail.powf(0.72) * 0.58;
    let mut lift = target / source_max.max(0.05);
    if lift > 8.5 {
        lift = 8.5;
    }
    source_r = clamp01(source_r * lift);
    source_g = clamp01(source_g * lift);
    source_b = clamp01(source_b * lift);
    // A floor per channel, weighted toward green and blue: the CRT this imitates
    // has no deep red phosphor, so the floor is not neutral grey.
    source_r = source_r.max(target * 0.18);
    source_g = source_g.max(target * 0.65);
    source_b = source_b.max(target * 0.78);

    const CYAN_R: f32 = 0.0;
    const CYAN_G: f32 = 240.0 / 255.0;
    const CYAN_B: f32 = 1.0;
    let phosphor = 0.11 + edge * 0.12 + band * 0.08;
    source_r = lerp(source_r, CYAN_R, phosphor);
    source_g = lerp(source_g, CYAN_G, phosphor);
    source_b = lerp(source_b, CYAN_B, phosphor);

    let source_alpha = f32::from(cell.foreground.a) / 255.0;
    let mut alpha = (0.44 + detail.powf(0.68) * 0.56) * (0.86 + energy * 0.08 + band * 0.08);
    alpha *= 0.30 + source_alpha * 0.70;
    alpha *= 0.94 + semantic_tension * 0.06;
    alpha *= clamp01(0.62 + 0.38 * gain);
    // Gain scales the delivered light, applied after the phosphor pipeline so it
    // survives the lift/blend stages instead of being renormalized.
    Rgba {
        r: color_byte(clamp01(source_r * gain) * 255.0),
        g: color_byte(clamp01(source_g * gain) * 255.0),
        b: color_byte(clamp01(source_b * gain) * 255.0),
        a: color_byte(clamp01(alpha) * 255.0),
    }
}

/// `Ascii_Field_State` (`scene_ascii_field.c:12-16`).
///
/// Bounded: one fixed `MAX_ROWS x MAX_COLUMNS` history, about 20 KiB, and nothing
/// that grows.
#[derive(Clone, Debug, PartialEq)]
pub struct AsciiFieldState {
    seed: u64,
    /// `-1.0` means "nothing sampled yet", exactly as in C.
    last_sample_time: f64,
    /// Row 0 is the newest sample; the waterfall shifts down.
    spectrum_history: Box<[[f32; MAX_COLUMNS]; MAX_ROWS]>,
}

impl AsciiFieldState {
    /// `ascii_field_init` (`scene_ascii_field.c:162-168`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            last_sample_time: -1.0,
            spectrum_history: Box::new([[0.0; MAX_COLUMNS]; MAX_ROWS]),
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn last_sample_time(&self) -> f64 {
        self.last_sample_time
    }

    /// The scrolling spectrogram the drawing half samples.
    #[must_use]
    pub fn spectrum_history(&self) -> &[[f32; MAX_COLUMNS]; MAX_ROWS] {
        &self.spectrum_history
    }

    /// One history cell, `0.0` outside the grid.
    #[must_use]
    pub fn density(&self, row: usize, column: usize) -> f32 {
        self.spectrum_history
            .get(row)
            .and_then(|row| row.get(column))
            .copied()
            .unwrap_or(0.0)
    }
}

impl SceneState for AsciiFieldState {
    fn id(&self) -> SceneId {
        SceneId::AsciiField
    }

    /// `ascii_field_update` (`scene_ascii_field.c:170-198`).
    ///
    /// Three behaviours worth keeping: a non-finite time is ignored entirely; a
    /// backward or long-forward jump clears the whole waterfall so unrelated
    /// regions are never joined; and sampling is rate-limited to
    /// [`SAMPLE_INTERVAL`] so the scroll speed is frame-rate independent.
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.time_seconds.is_finite() {
            return;
        }
        let elapsed = if self.last_sample_time < 0.0 {
            0.0
        } else {
            frame.time_seconds - self.last_sample_time
        };
        if self.last_sample_time >= 0.0 && (elapsed < 0.0 || elapsed > DISCONTINUITY_SECONDS) {
            *self.spectrum_history = [[0.0; MAX_COLUMNS]; MAX_ROWS];
            self.last_sample_time = -1.0;
        }
        if self.last_sample_time >= 0.0 && elapsed < SAMPLE_INTERVAL {
            return;
        }

        // C's memmove of `(MAX_ROWS-1)*MAX_COLUMNS` floats: every row shifts one
        // step older, and row 0 is overwritten below.
        self.spectrum_history.copy_within(0..MAX_ROWS - 1, 1);
        let bands = frame.audio.bands;
        let trails = frame.audio.trails;
        for column in 0..MAX_COLUMNS {
            let value = audio_band(bands, column, MAX_COLUMNS);
            let mut trail = 0.0f32;
            if !trails.is_empty() && !bands.is_empty() {
                // Note the asymmetry with `audio_band` above: the trail uses a
                // plain proportional index and is bounded by `bands.len()`, not
                // by the trail array's own length. Reproduced as-is; the two
                // arrays are always the same length in practice.
                let mut index = column * bands.len() / MAX_COLUMNS;
                if index >= bands.len() {
                    index = bands.len() - 1;
                }
                trail = trails.get(index).copied().map_or(0.0, clamp01);
            }
            self.spectrum_history[0][column] = clamp01(value * 0.78 + trail * 0.22);
        }
        self.last_sample_time = frame.time_seconds;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The registry entry (`scene_ascii_field.c:445-453`).
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::AsciiField,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(AsciiFieldState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneAudioFrame, SceneSettings};

    fn frame_at<'a>(
        settings: &'a SceneSettings,
        time: f64,
        smooth: &'a [f32],
        smear: &'a [f32],
    ) -> SceneFrame<'a> {
        SceneFrame {
            time_seconds: time,
            delta_seconds: 1.0 / 60.0,
            audio: SceneAudioFrame::from_spectrum(smooth, smear),
            ..SceneFrame::idle(settings)
        }
    }

    #[test]
    fn the_descriptor_matches_the_c_one() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SceneId::AsciiField);
        assert_eq!(descriptor.state_version, 4);
        assert_eq!(descriptor.name(), "ASCII Field");
    }

    #[test]
    fn sampling_is_rate_limited_to_eighteen_hertz() {
        let settings = SceneSettings::default();
        let loud = [1.0f32; 32];
        let quiet = [0.0f32; 32];
        let mut state = AsciiFieldState::new(1);

        // The first frame always samples, because nothing has been sampled yet.
        state.update(&frame_at(&settings, 0.0, &loud, &loud));
        assert!(state.density(0, 0) > 0.9);
        assert_eq!(state.last_sample_time(), 0.0);

        // Half an interval later: ignored, so row 0 keeps the loud sample.
        state.update(&frame_at(&settings, SAMPLE_INTERVAL * 0.5, &quiet, &quiet));
        assert!(state.density(0, 0) > 0.9);
        assert_eq!(state.last_sample_time(), 0.0);

        // A full interval later: sampled, and the loud row has scrolled to row 1.
        state.update(&frame_at(&settings, SAMPLE_INTERVAL, &quiet, &quiet));
        assert_eq!(state.density(0, 0), 0.0);
        assert!(state.density(1, 0) > 0.9);
    }

    #[test]
    fn a_seek_or_a_long_gap_clears_the_waterfall() {
        let settings = SceneSettings::default();
        let loud = [1.0f32; 16];
        // A tenth of a second is comfortably more than one 18 Hz interval. Adding
        // exactly `SAMPLE_INTERVAL` would be a coin flip: 1/18 is not
        // representable, so `(10.0 + 1.0/18.0) - 10.0` can land just under it and
        // the frame is skipped.
        for gap in [-1.0f64, DISCONTINUITY_SECONDS + 0.01] {
            let mut state = AsciiFieldState::new(1);
            state.update(&frame_at(&settings, 10.0, &loud, &loud));
            state.update(&frame_at(&settings, 10.1, &loud, &loud));
            assert!(state.density(1, 0) > 0.9, "history built up");

            // The clear happens first, then the same frame samples afresh, so
            // only row 0 survives.
            state.update(&frame_at(&settings, 10.1 + gap, &loud, &loud));
            assert!(state.density(0, 0) > 0.9);
            assert_eq!(state.density(1, 0), 0.0, "gap {gap} did not clear history");
        }
    }

    #[test]
    fn a_non_finite_time_is_ignored_entirely() {
        let settings = SceneSettings::default();
        let loud = [1.0f32; 16];
        let mut state = AsciiFieldState::new(1);
        let before = state.clone();
        state.update(&frame_at(&settings, f64::NAN, &loud, &loud));
        assert_eq!(state, before);
    }

    #[test]
    fn the_history_blends_bands_with_trails_in_the_c_proportion() {
        let settings = SceneSettings::default();
        let bands = [1.0f32; MAX_COLUMNS];
        let trails = [0.0f32; MAX_COLUMNS];
        let mut state = AsciiFieldState::new(1);
        state.update(&frame_at(&settings, 0.0, &bands, &trails));
        // 1.0*0.78 + 0.0*0.22
        assert!((state.density(0, 0) - 0.78).abs() < 1.0e-6);

        let trails = [1.0f32; MAX_COLUMNS];
        let mut state = AsciiFieldState::new(1);
        state.update(&frame_at(&settings, 0.0, &bands, &trails));
        assert!((state.density(0, 0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn the_update_is_deterministic_and_bounded() {
        let settings = SceneSettings::default();
        let mut bands = [0.0f32; 64];
        let mut first = AsciiFieldState::new(0xabcd);
        let mut second = AsciiFieldState::new(0xabcd);
        for index in 0..600u32 {
            for (i, band) in bands.iter_mut().enumerate() {
                *band = ((index as usize + i) % 13) as f32 / 12.0;
            }
            let time = f64::from(index) / 60.0;
            first.update(&frame_at(&settings, time, &bands, &bands));
            second.update(&frame_at(&settings, time, &bands, &bands));
        }
        assert_eq!(first, second);
        // Ten seconds of history in a fixed buffer: nothing grew.
        assert_eq!(first.spectrum_history().len(), MAX_ROWS);
        for row in first.spectrum_history().iter() {
            for &value in row.iter() {
                assert!((0.0..=1.0).contains(&value));
            }
        }
    }

    #[test]
    fn a_column_maps_onto_a_band_without_floating_point() {
        let bands = [0.0f32, 0.25, 0.5, 0.75];
        assert_eq!(audio_band(&bands, 0, 4), 0.0);
        assert_eq!(audio_band(&bands, 3, 4), 0.75);
        // More columns than bands: the last column still lands inside.
        assert_eq!(audio_band(&bands, 95, 96), 0.75);
        // No bands at all, and no columns at all.
        assert_eq!(audio_band(&[], 0, 96), 0.0);
        assert_eq!(audio_band(&bands, 0, 0), 0.0);
    }

    #[test]
    fn the_color_pipeline_lifts_dark_ink_instead_of_multiplying_it_away() {
        let dark = Cell {
            glyph: u32::from(b'#'),
            foreground: Rgba {
                r: 8,
                g: 10,
                b: 12,
                a: 255,
            },
            luminance: 0.6,
            ..Cell::default()
        };
        let color = main_color(&dark, 0.0, 0.0, 0.0, 1.0);
        // A near-black source still delivers a visible mark.
        assert!(color.g > 100, "green was {}", color.g);
        assert!(color.a > 100, "alpha was {}", color.a);
        // And the CRT bias is real: blue and green lead red.
        assert!(color.b > color.r);
        assert!(color.g > color.r);
    }

    #[test]
    fn gain_scales_the_delivered_light_monotonically() {
        let cell = Cell {
            glyph: u32::from(b'L'),
            foreground: Rgba {
                r: 120,
                g: 160,
                b: 200,
                a: 255,
            },
            luminance: 0.5,
            ..Cell::default()
        };
        let low = main_color(&cell, 0.0, 0.0, 0.0, 0.5);
        let high = main_color(&cell, 0.0, 0.0, 0.0, 2.5);
        assert!(high.a >= low.a);
        assert!(high.g >= low.g);
    }

    #[test]
    fn color_byte_saturates_at_both_ends() {
        assert_eq!(color_byte(-1.0), 0);
        assert_eq!(color_byte(f32::NAN), 0);
        assert_eq!(color_byte(0.4), 0);
        assert_eq!(color_byte(0.5), 1);
        assert_eq!(color_byte(254.4), 254);
        assert_eq!(color_byte(1000.0), 255);
    }
}
