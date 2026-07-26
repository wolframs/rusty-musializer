//! Constellation's audio envelopes, raylib-free.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_constellation_motion.c`
//! and `.h`. This module is one of the three the C project extracted precisely so
//! it could be tested without a window; keep it pure. Its C tests are ported at
//! the bottom of this file from
//! `../musializer/tests/test_scene_constellation_motion.c`.
//!
//! Everything here is a smoothing filter over the measured-audio lane. The one
//! subtle part is the discontinuity test: a seek, a pause, or a reload must
//! *rebase* the envelopes rather than integrate across the gap, or a scrub leaves
//! a visible slide behind the playhead.

/// One frame of input (`scene_constellation_motion.h:6-12`).
#[derive(Clone, Copy, Debug, Default)]
pub struct MotionInput {
    pub time_seconds: f64,
    pub delta_seconds: f32,
    pub rms: f32,
    pub spectral_flux: f32,
    pub onset: bool,
}

/// The smoothed state (`scene_constellation_motion.h:14-20`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub initialized: bool,
    pub last_time_seconds: f64,
    /// Smoothed loudness, 0..1.
    pub energy: f32,
    /// Smoothed spectral flux, 0..1.
    pub flux: f32,
    /// 1.0 while an onset is held, decaying afterwards.
    pub onset_pulse: f32,
}

/// `constellation_motion_clamp01` (`scene_constellation_motion.c:6-11`).
fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// Normalizes a song time the way the C does: anything non-finite or
/// non-positive reads as 0.
fn normalize_time(time_seconds: f64) -> f64 {
    if time_seconds.is_finite() && time_seconds > 0.0 {
        time_seconds
    } else {
        0.0
    }
}

impl Motion {
    /// `constellation_motion_init` (`:26-30`) — an all-zero, uninitialized filter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `constellation_motion_rebase` (`:13-24`).
    ///
    /// Snaps the envelopes straight to the current measurement. Blending across a
    /// discontinuity is what this exists to avoid.
    fn rebase(&mut self, input: &MotionInput) {
        self.energy = clamp01(input.rms * 1.8);
        self.flux = clamp01(input.spectral_flux * 5.0);
        self.onset_pulse = if input.onset { 1.0 } else { 0.0 };
        self.last_time_seconds = normalize_time(input.time_seconds);
        self.initialized = true;
    }

    /// `constellation_motion_update` (`:32-60`).
    ///
    /// The four-way discontinuity test is copied exactly, including the last
    /// clause: a nonzero time jump reported with a **zero** delta is how the C
    /// application signals a seek while paused, and treating it as an ordinary
    /// frame would smear the old envelopes across the jump.
    // The four discontinuity clauses are kept as separate comparisons, in the C's
    // order, because each one names a different transport situation. Folding two of
    // them into a range check would make the list harder to compare with the oracle.
    #[allow(clippy::manual_range_contains)]
    pub fn update(&mut self, input: &MotionInput) {
        let time = normalize_time(input.time_seconds);
        let elapsed = time - self.last_time_seconds;
        let mut delta = if input.delta_seconds.is_finite() && input.delta_seconds > 0.0 {
            input.delta_seconds
        } else {
            0.0
        };
        let discontinuity = !self.initialized
            || elapsed < -0.001
            || elapsed > 0.25
            || delta > 0.20
            || (elapsed.abs() > 0.02 && delta == 0.0);
        if discontinuity {
            self.rebase(input);
            return;
        }

        if delta > 0.1 {
            delta = 0.1;
        }
        self.last_time_seconds = time;
        if delta <= 0.0 {
            return;
        }

        let blend = 1.0 - (-5.0 * delta).exp();
        let energy = clamp01(input.rms * 1.8);
        let flux = clamp01(input.spectral_flux * 5.0);
        self.energy += (energy - self.energy) * blend;
        self.flux += (flux - self.flux) * blend;
        self.onset_pulse *= (-7.0 * delta).exp();
        if input.onset {
            self.onset_pulse = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        time_seconds: f64,
        delta_seconds: f32,
        rms: f32,
        flux: f32,
        onset: bool,
    ) -> MotionInput {
        MotionInput {
            time_seconds,
            delta_seconds,
            rms,
            spectral_flux: flux,
            onset,
        }
    }

    /// Port of `constellation_motion_rebases_transport_discontinuities`.
    #[test]
    fn rebasing_makes_a_seek_indistinguishable_from_a_fresh_filter() {
        let mut played = Motion::new();
        for frame in 0..180u32 {
            played.update(&input(
                f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                0.15 + (frame % 9) as f32 * 0.04,
                0.02 + (frame % 7) as f32 * 0.01,
                frame % 45 == 0,
            ));
        }

        let mut fresh = Motion::new();
        let seek = input(0.75, 0.0, 0.62, 0.11, true);
        played.update(&seek);
        fresh.update(&seek);
        assert_eq!(played, fresh);
    }

    /// Port of `constellation_motion_is_deterministic_and_bounded`.
    #[test]
    fn hostile_input_stays_bounded_and_deterministic() {
        let mut first = Motion::new();
        let mut second = Motion::new();
        for frame in 0..240u32 {
            let sample = input(
                12.0 + f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                if frame % 2 != 0 { 1.5 } else { -0.5 },
                if frame % 3 != 0 { 0.4 } else { -1.0 },
                frame % 37 == 0,
            );
            first.update(&sample);
            second.update(&sample);
            assert!((0.0..=1.0).contains(&first.energy));
            assert!((0.0..=1.0).contains(&first.flux));
            assert!((0.0..=1.0).contains(&first.onset_pulse));
        }
        assert_eq!(first, second);
    }

    #[test]
    fn a_time_jump_with_a_zero_delta_counts_as_a_seek() {
        // This is the clause a reader is most likely to think is redundant. It is
        // not: the paused-and-scrubbed case reports movement with no frame delta.
        let mut motion = Motion::new();
        // Establish the filter at silence, then feed it one ordinary frame of
        // signal so the energy is visibly mid-blend rather than already on target.
        motion.update(&input(1.0, 1.0 / 60.0, 0.0, 0.0, false));
        assert_eq!(motion.energy, 0.0);
        motion.update(&input(1.0 + 1.0 / 60.0, 1.0 / 60.0, 0.5, 0.1, false));
        let smoothed = motion.energy;
        assert!(
            smoothed > 0.0 && smoothed < clamp01(0.5 * 1.8),
            "energy {smoothed} should be part-way to the target"
        );

        motion.update(&input(1.10, 0.0, 0.5, 0.1, false));
        assert_eq!(
            motion.energy,
            clamp01(0.5 * 1.8),
            "a scrub rebases instead of blending"
        );
    }

    #[test]
    fn a_negative_or_non_finite_song_time_reads_as_zero() {
        let mut motion = Motion::new();
        motion.update(&input(f64::NAN, 1.0 / 60.0, 0.3, 0.05, false));
        assert_eq!(motion.last_time_seconds, 0.0);
        motion.update(&input(-5.0, 1.0 / 60.0, 0.3, 0.05, false));
        assert_eq!(motion.last_time_seconds, 0.0);
        assert!(motion.initialized);
    }
}
