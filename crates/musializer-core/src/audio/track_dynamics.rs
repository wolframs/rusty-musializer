//! Track-relative loudness bounds, so "quiet" and "loud" mean *this track's*
//! quiet and loud.
//!
//! **No oracle.** Post-legacy (2026-08-26), built for Clawd's expression gates
//! and petal-show mode, and measured before it was designed: on the operator's
//! *Parameter People* the amplitude envelope every scene reads
//! (`clamp01(rms * 2)` over the analyzer's self-normalizing bands) has a median
//! of 0.942 and a 90th percentile of 1.0, so any absolute "quiet" constant
//! below ~0.8 is unreachable — Clawd's pleading face fired on 8 of 5736 frames
//! across the whole track. A gate written against a synthetic fixture's range
//! is a gate that never fires on music.
//!
//! The profile is two numbers: robust floor and ceiling percentiles of the
//! frame-rms the scenes actually see, derived once from the whole decoded
//! track. Percentiles rather than extrema because one freak transient must not
//! flatten the rest of the track's range, and a minimum span guard because a
//! heavily compressed track would otherwise amplify noise into a full swing.
//!
//! The profiled quantity is deliberately [`SceneAudioFrame::from_spectrum`]'s
//! own `rms` — band-space, not PCM — computed by running the real analyzer
//! over the track at a fixed internal cadence. Profiling PCM loudness in dB
//! looks more principled and is wrong here: the analyzer's `ln`-amplitude
//! bands are already log-domain and normalized against a running maximum, so
//! the per-frame value a scene compares against lives in that space, not in
//! dBFS. Robust percentiles do not care that the preview runs at a different
//! frame rate than [`PROFILE_STEPS_PER_SECOND`].

use crate::audio::analyzer::{AudioAnalyzer, AudioAnalyzerConfig, ChannelMode};
use crate::scene::SceneAudioFrame;
use crate::timing::render_export::sample_cursor;

/// The profiler's internal analysis cadence. 30 steps per second gives a
/// three-minute track ~5400 samples — far more than percentiles need — while
/// keeping the whole-track FFT pass cheap enough to run at track load.
pub const PROFILE_STEPS_PER_SECOND: u32 = 30;

/// Sorted-rank positions of the two bounds. 10th/95th rather than min/max:
/// the floor should sit inside the track's quiet material, not at its single
/// stillest frame, and the ceiling below its single loudest transient.
const FLOOR_PERCENTILE: f64 = 0.10;
const CEILING_PERCENTILE: f64 = 0.95;

/// Smallest usable floor..ceiling span, in band-rms units. A track compressed
/// flatter than this gets the span widened symmetrically rather than having
/// its residual noise stretched into a full 0..1 swing.
const MIN_SPAN: f32 = 0.08;

/// A profiling step counts as "sound" above this band-rms. Below it the step
/// is leader, tail, or gap — still profiled, but a track needs some actual
/// sound before two percentiles of it mean anything.
const MIN_SOUND_RMS: f32 = 0.02;

/// How many sound-bearing steps the profile requires — two seconds' worth at
/// the profiling cadence. Fewer and the track is silence or nearly so, and a
/// profile of noise would hand every consumer a confidently wrong range.
const MIN_SOUND_STEPS: usize = (2 * PROFILE_STEPS_PER_SECOND) as usize;

/// Robust per-track loudness bounds. Two `f32`s, `Copy`, cheap to hand to
/// every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackDynamics {
    floor: f32,
    ceiling: f32,
}

impl TrackDynamics {
    /// Profiles a decoded track: interleaved f32 PCM, the file's own channel
    /// count and sample rate — the same triple `decode::whole_track` returns.
    ///
    /// `None` when the input cannot carry a meaningful profile (empty, zero
    /// rate, or under [`MIN_SOUND_STEPS`] of audible material). A consumer
    /// falls back to its absolute constants then, and must *say so* — a
    /// profile silently absent is the unwired-feature trap this repository
    /// documents.
    #[must_use]
    pub fn profile(samples: &[f32], channel_count: usize, sample_rate: u32) -> Option<Self> {
        if samples.is_empty() || channel_count == 0 || sample_rate == 0 {
            return None;
        }
        let mode = if channel_count == 1 {
            ChannelMode::Select(0)
        } else {
            // Averaging rather than the preview's historical `Select(0)`:
            // a loudness profile should hear the whole mix, and there is no
            // C behaviour to reproduce here.
            ChannelMode::Mix
        };
        let mut analyzer = AudioAnalyzer::new(AudioAnalyzerConfig {
            sample_rate,
            channel_count: channel_count as u32,
            channel_mode: mode,
        })
        .ok()?;

        let frame_count = (samples.len() / channel_count) as u64;
        let delta = 1.0 / PROFILE_STEPS_PER_SECOND as f32;
        let mut values: Vec<f32> = Vec::new();
        let mut cursor = 0u64;
        let mut step = 1u64;
        while cursor < frame_count {
            let next =
                sample_cursor(step, sample_rate, PROFILE_STEPS_PER_SECOND, frame_count).ok()?;
            let from = cursor as usize * channel_count;
            let to = (next as usize * channel_count).min(samples.len());
            cursor = next;
            step += 1;
            if to > from {
                analyzer.push_interleaved(&samples[from..to]);
            }
            analyzer.analyze(delta);
            let spectrum = analyzer.spectrum();
            let rms = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear).rms;
            if rms.is_finite() {
                values.push(rms);
            }
        }

        if values.iter().filter(|&&v| v > MIN_SOUND_RMS).count() < MIN_SOUND_STEPS {
            return None;
        }
        values.sort_by(f32::total_cmp);
        let rank = |p: f64| values[(((values.len() - 1) as f64) * p).round() as usize];
        Self::from_bounds(rank(FLOOR_PERCENTILE), rank(CEILING_PERCENTILE))
    }

    /// Bounds directly, for tests and probes. Applies the same
    /// [`MIN_SPAN`] widening the profiler does; refuses non-finite or
    /// reversed input.
    #[must_use]
    pub fn from_bounds(floor: f32, ceiling: f32) -> Option<Self> {
        if !floor.is_finite() || !ceiling.is_finite() || ceiling < floor {
            return None;
        }
        let span = ceiling - floor;
        if span >= MIN_SPAN {
            return Some(Self { floor, ceiling });
        }
        // Widen symmetrically about the midpoint, sliding up off zero rather
        // than clamping the span short: a near-silent compressed track ends up
        // mapped mostly below its own midpoint, which is the honest reading.
        let mid = (floor + ceiling) * 0.5;
        let floor = (mid - MIN_SPAN * 0.5).max(0.0);
        Some(Self {
            floor,
            ceiling: floor + MIN_SPAN,
        })
    }

    #[must_use]
    pub fn floor(&self) -> f32 {
        self.floor
    }

    #[must_use]
    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// Maps a frame's band-rms into this track's own `0..1`: clamped, then
    /// smoothstepped so both ends ease rather than kink. `0` is the track's
    /// own quiet, `1` its own loud, by construction of the bounds.
    #[must_use]
    pub fn level(&self, rms: f32) -> f32 {
        if !rms.is_finite() {
            return 0.0;
        }
        let x = ((rms - self.floor) / (self.ceiling - self.floor)).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_track(seconds: f32, amplitude: f32, sample_rate: u32) -> Vec<f32> {
        let frames = (seconds * sample_rate as f32) as usize;
        (0..frames)
            .flat_map(|i| {
                let s = amplitude
                    * (std::f32::consts::TAU * 220.0 * i as f32 / sample_rate as f32).sin();
                [s, s]
            })
            .collect()
    }

    #[test]
    fn degenerate_input_is_refused() {
        assert!(TrackDynamics::profile(&[], 2, 48_000).is_none());
        assert!(TrackDynamics::profile(&[0.1; 96_000], 0, 48_000).is_none());
        assert!(TrackDynamics::profile(&[0.1; 96_000], 2, 0).is_none());
        // Pure silence has no dynamics to profile.
        let silence = vec![0.0f32; 48_000 * 8 * 2];
        assert!(TrackDynamics::profile(&silence, 2, 48_000).is_none());
    }

    #[test]
    fn bounds_are_ordered_and_finite_on_real_shaped_input() {
        // Quiet leader, loud middle, quiet tail — the shape of a song.
        let mut samples = sine_track(3.0, 0.05, 48_000);
        samples.extend(sine_track(6.0, 0.8, 48_000));
        samples.extend(sine_track(3.0, 0.05, 48_000));
        let profile = TrackDynamics::profile(&samples, 2, 48_000).expect("profile");
        assert!(profile.floor().is_finite() && profile.ceiling().is_finite());
        assert!(profile.ceiling() > profile.floor());
        // The loud section must read near the top of the track's own range and
        // the quiet leader near the bottom — that is the entire point.
        assert!(profile.ceiling() - profile.floor() >= MIN_SPAN);
    }

    #[test]
    fn level_maps_the_tracks_own_range_onto_zero_one() {
        let profile = TrackDynamics::from_bounds(0.2, 0.6).expect("bounds");
        assert_eq!(profile.level(0.2), 0.0);
        assert_eq!(profile.level(0.6), 1.0);
        assert_eq!(profile.level(0.0), 0.0);
        assert_eq!(profile.level(1.0), 1.0);
        let mid = profile.level(0.4);
        assert!((mid - 0.5).abs() < 1e-6, "smoothstep midpoint: {mid}");
        assert_eq!(profile.level(f32::NAN), 0.0);
        // Monotone across the span.
        let mut previous = -1.0f32;
        for i in 0..=40 {
            let level = profile.level(0.2 + 0.4 * i as f32 / 40.0);
            assert!(level >= previous);
            previous = level;
        }
    }

    #[test]
    fn a_compressed_track_gets_the_minimum_span() {
        let profile = TrackDynamics::from_bounds(0.50, 0.51).expect("bounds");
        assert!((profile.ceiling() - profile.floor() - MIN_SPAN).abs() < 1e-6);
        // Widened about the midpoint, not off one end.
        let mid = (profile.floor() + profile.ceiling()) * 0.5;
        assert!((mid - 0.505).abs() < 1e-3, "midpoint drifted to {mid}");
        // Near zero the widening slides up instead of clamping short.
        let low = TrackDynamics::from_bounds(0.0, 0.01).expect("bounds");
        assert_eq!(low.floor(), 0.0);
        assert!((low.ceiling() - MIN_SPAN).abs() < 1e-6);
    }

    #[test]
    fn reversed_or_non_finite_bounds_are_refused() {
        assert!(TrackDynamics::from_bounds(0.6, 0.2).is_none());
        assert!(TrackDynamics::from_bounds(f32::NAN, 0.5).is_none());
        assert!(TrackDynamics::from_bounds(0.1, f32::INFINITY).is_none());
    }

    #[test]
    fn the_same_pcm_always_profiles_the_same_bounds() {
        let mut samples = sine_track(2.0, 0.1, 44_100);
        samples.extend(sine_track(4.0, 0.7, 44_100));
        let a = TrackDynamics::profile(&samples, 2, 44_100).expect("profile");
        let b = TrackDynamics::profile(&samples, 2, 44_100).expect("profile");
        assert_eq!(a, b);
    }
}
