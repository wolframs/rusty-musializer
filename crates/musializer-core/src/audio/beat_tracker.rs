//! A continuous beat phase derived from onsets.
//!
//! **Owner: Agent A.** Port of `../musializer/src/beat_tracker.c` and `.h`.
//!
//! The tracker's job is to hand scenes a phase in `[0, 1)` that is *always*
//! usable, even before it has heard anything credible. Its design follows from
//! that: before two credible onsets it runs a neutral 120 BPM clock anchored to
//! track time (`beat_tracker.h:19-23`), so a scene never has to special-case
//! "no tempo yet"; and a time reversal or a large gap starts a new local clock
//! rather than producing a phase jump, because a seek or a preview stall is a
//! discontinuity in the *observer*, not evidence about the music.
//!
//! It is driven by track time, never by a wall clock, which is what keeps preview
//! and export identical.

/// The neutral clock used before any tempo is learned (`beat_tracker.c:6-8`).
pub const DEFAULT_BPM: u32 = 120;

/// Fastest credible beat: 240 BPM (`beat_tracker.c:10`).
const MINIMUM_INTERVAL: f64 = 0.25;
/// Slowest credible beat: 40 BPM (`beat_tracker.c:11`).
const MAXIMUM_INTERVAL: f64 = 1.50;
/// A gap larger than this is a seek or a stall, not a long beat
/// (`beat_tracker.c:12`).
const DISCONTINUITY_SECONDS: f64 = 0.75;
/// Onsets weaker than this are noise (`beat_tracker.c:53`).
const ONSET_STRENGTH_FLOOR: f32 = 0.04;
/// Observations before the estimate stops moving quickly (`beat_tracker.c:62`).
const FAST_LEARNING_OBSERVATIONS: usize = 4;
/// Blend weight while still learning, then afterwards (`beat_tracker.c:62`).
const FAST_WEIGHT: f64 = 0.42;
const SLOW_WEIGHT: f64 = 0.20;

/// Beat phase state (`beat_tracker.h:7-15`).
///
/// [`Default`] is C's post-`beat_tracker_reset` state, so
/// `BeatTracker::default()` is the correct way to start one. The C code
/// deliberately calls `reset` on a stack value that was never initialized, which
/// is why `reset` is a `memset` plus one field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatTracker {
    previous_time: f64,
    anchor_time: f64,
    last_onset_time: f64,
    interval_seconds: f64,
    learned_intervals: usize,
    initialized: bool,
    has_onset: bool,
}

impl Default for BeatTracker {
    fn default() -> Self {
        Self {
            previous_time: 0.0,
            anchor_time: 0.0,
            last_onset_time: 0.0,
            interval_seconds: 60.0 / f64::from(DEFAULT_BPM),
            learned_intervals: 0,
            initialized: false,
            has_onset: false,
        }
    }
}

/// Folds an observed interval towards the current estimate by octaves
/// (`beat_tracker.c:21-30`).
///
/// A detector that fires on every eighth note and one that fires on every
/// quarter note describe the same tempo, so an observation that is roughly half
/// or double the running estimate is doubled or halved into agreement with it
/// rather than fighting it. Both loops refuse to leave the credible interval
/// range, so folding can decline to converge instead of producing an absurd
/// tempo.
fn tempo_fold(mut interval: f64, reference: f64) -> f64 {
    while interval < reference * 0.67 && interval * 2.0 <= MAXIMUM_INTERVAL {
        interval *= 2.0;
    }
    while interval > reference * 1.50 && interval * 0.5 >= MINIMUM_INTERVAL {
        interval *= 0.5;
    }
    interval
}

impl BeatTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns to the neutral 120 BPM clock (`beat_tracker.c:14-19`).
    ///
    /// Note that this discards the learned interval, unlike the internal restart
    /// [`BeatTracker::update`] performs on a discontinuity, which preserves a
    /// credible interval across the gap.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The current beat interval in seconds; `0.5` until a tempo is learned.
    #[must_use]
    pub fn interval_seconds(&self) -> f64 {
        self.interval_seconds
    }

    /// Beats per minute implied by [`BeatTracker::interval_seconds`].
    #[must_use]
    pub fn bpm(&self) -> f64 {
        60.0 / self.interval_seconds
    }

    /// How many credible intervals have been folded into the estimate.
    ///
    /// Zero means the phase is still the neutral clock's.
    #[must_use]
    pub fn learned_intervals(&self) -> usize {
        self.learned_intervals
    }

    /// Whether a first credible onset has been seen since the last restart.
    #[must_use]
    pub fn has_onset(&self) -> bool {
        self.has_onset
    }

    /// Advances the tracker to `time_seconds` and returns the beat phase.
    ///
    /// Returns `None` for input C rejects (`beat_tracker.c:35-38`): a non-finite
    /// or negative `time_seconds`, or a non-finite or negative `onset_strength`.
    /// State is untouched in that case.
    ///
    /// The behaviour, in the order the C performs it:
    ///
    /// 1. **Restart on discontinuity** (`beat_tracker.c:40-51`). A first call, a
    ///    backwards jump, or a forward jump beyond
    ///    `DISCONTINUITY_SECONDS` re-anchors the clock to `time_seconds` and
    ///    clears `has_onset`. The learned *interval* survives the restart when it
    ///    is credible, so a seek within one song does not throw away its tempo,
    ///    but the phase reference does not.
    /// 2. **Learn from onsets** (`beat_tracker.c:53-70`). The first credible
    ///    onset only anchors; from the second on, an inter-onset gap inside
    ///    `[0.25, 1.5]` s is octave-folded and blended into the estimate. An
    ///    out-of-range gap is ignored *entirely* — it does not even move the
    ///    anchor — so a doubled kick or a dropout cannot drag the phase.
    /// 3. **Report phase** as the fractional part of
    ///    `(time - anchor) / interval` (`beat_tracker.c:72-75`).
    ///
    /// Because the anchor moves to every accepted onset, the phase is continuous
    /// at the moment of an onset (it reads `0.0` there) rather than jumping.
    pub fn update(&mut self, time_seconds: f64, onset: bool, onset_strength: f32) -> Option<f32> {
        if !time_seconds.is_finite()
            || time_seconds < 0.0
            || !onset_strength.is_finite()
            || onset_strength < 0.0
        {
            return None;
        }

        if !self.initialized
            || time_seconds < self.previous_time
            || time_seconds - self.previous_time > DISCONTINUITY_SECONDS
        {
            let interval = self.interval_seconds;
            self.reset();
            if interval.is_finite() && (MINIMUM_INTERVAL..=MAXIMUM_INTERVAL).contains(&interval) {
                self.interval_seconds = interval;
            }
            self.initialized = true;
            self.anchor_time = time_seconds;
            self.previous_time = time_seconds;
        }

        if onset && onset_strength >= ONSET_STRENGTH_FLOOR {
            if self.has_onset {
                let observed = time_seconds - self.last_onset_time;
                if (MINIMUM_INTERVAL..=MAXIMUM_INTERVAL).contains(&observed) {
                    let observed = tempo_fold(observed, self.interval_seconds);
                    let weight = if self.learned_intervals < FAST_LEARNING_OBSERVATIONS {
                        FAST_WEIGHT
                    } else {
                        SLOW_WEIGHT
                    };
                    self.interval_seconds += (observed - self.interval_seconds) * weight;
                    self.learned_intervals += 1;
                    self.last_onset_time = time_seconds;
                    self.anchor_time = time_seconds;
                }
            } else {
                self.has_onset = true;
                self.last_onset_time = time_seconds;
                self.anchor_time = time_seconds;
            }
        }

        let mut position = (time_seconds - self.anchor_time) / self.interval_seconds;
        position -= position.floor();
        if position < 0.0 {
            position += 1.0;
        }
        let phase = position as f32;
        self.previous_time = time_seconds;

        // C writes `*phase` before this check and still returns false
        // (`beat_tracker.c:77`), so a C caller that ignores the return value sees
        // the same value Rust withholds here. It is defensive either way: the
        // interval is kept inside [0.25, 1.5] by construction, so the division
        // cannot produce a non-finite position.
        if phase.is_finite() && (0.0..1.0).contains(&phase) {
            Some(phase)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_negative_time_and_negative_strength() {
        let mut tracker = BeatTracker::new();
        assert_eq!(tracker.update(-1.0, false, 0.0), None);
        assert_eq!(tracker.update(f64::NAN, false, 0.0), None);
        assert_eq!(tracker.update(f64::INFINITY, false, 0.0), None);
        assert_eq!(tracker.update(0.0, false, f32::NAN), None);
        assert_eq!(tracker.update(0.0, false, f32::INFINITY), None);
        assert_eq!(tracker.update(0.0, false, -0.1), None);
        // Rejection is atomic: nothing above was initialized or anchored.
        assert_eq!(tracker, BeatTracker::default());
        assert_eq!(tracker.update(0.0, false, 0.0), Some(0.0));
    }

    /// The phase is in `[0, 1)` for every accepted input, which is the contract
    /// scenes rely on when they use it as a table index or a cosine argument.
    #[test]
    fn phase_stays_in_the_half_open_unit_range() {
        let mut tracker = BeatTracker::new();
        let mut time = 0.0f64;
        for step in 0..2_000u32 {
            // A 5 ms tick with an onset every 23rd sample: a deliberately
            // uncooperative 6.9 BPM-ish pattern the folder cannot latch.
            let onset = step % 23 == 0;
            let phase = tracker
                .update(time, onset, 0.9)
                .expect("finite non-negative time is always accepted");
            assert!(
                (0.0..1.0).contains(&phase),
                "phase {phase} escaped [0, 1) at t={time}"
            );
            time += 0.005;
        }
    }

    #[test]
    fn the_neutral_clock_runs_at_120_bpm_before_learning() {
        let mut tracker = BeatTracker::new();
        assert_eq!(tracker.interval_seconds(), 0.5);
        assert_eq!(tracker.bpm(), f64::from(DEFAULT_BPM));
        assert_eq!(tracker.update(3.0, false, 0.0), Some(0.0));
        assert_eq!(tracker.update(3.125, false, 0.0), Some(0.25));
        let phase = tracker.update(3.499, false, 0.0).unwrap();
        assert!(phase > 0.99 && phase < 1.0, "phase was {phase}");
        assert_eq!(tracker.learned_intervals(), 0);
    }

    #[test]
    fn credible_onset_intervals_are_learned_and_anchor_the_phase() {
        let mut tracker = BeatTracker::new();
        assert_eq!(tracker.update(0.0, true, 0.2), Some(0.0));
        assert_eq!(tracker.update(0.6, true, 0.2), Some(0.0));
        assert_eq!(tracker.update(1.2, true, 0.2), Some(0.0));
        assert_eq!(tracker.learned_intervals(), 2);
        // 0.5 -> 0.542 -> 0.56636, converging on the observed 0.6 s.
        assert!(tracker.interval_seconds() > 0.55);
        assert!(tracker.interval_seconds() < 0.60);
        // An accepted onset re-anchors, so the phase reads 0 exactly on it.
        let phase = tracker.update(1.35, false, 0.0).unwrap();
        assert!(phase > 0.25 && phase < 0.28, "phase was {phase}");
    }

    #[test]
    fn implausible_gaps_and_weak_onsets_are_ignored_entirely() {
        let mut tracker = BeatTracker::new();
        assert_eq!(tracker.update(0.0, true, 0.2), Some(0.0));
        // 0.1 s apart is 600 BPM: below MINIMUM_INTERVAL, so not an observation.
        assert_eq!(tracker.update(0.1, true, 0.2), Some(0.2));
        assert_eq!(tracker.learned_intervals(), 0);
        // Still anchored at the first onset, not at 0.1.
        assert_eq!(tracker.interval_seconds(), 0.5);

        // A sub-threshold strength is not an onset at all.
        let mut quiet = BeatTracker::new();
        assert_eq!(quiet.update(0.0, true, 0.039), Some(0.0));
        assert!(!quiet.has_onset(), "0.039 is below the 0.04 noise floor");
        assert_eq!(quiet.update(0.5, true, 0.04), Some(0.0));
        assert!(quiet.has_onset(), "0.04 is exactly at the floor and counts");
    }

    #[test]
    fn a_discontinuity_starts_a_new_clock_but_keeps_a_credible_tempo() {
        let mut tracker = BeatTracker::new();
        for beat in 0..6 {
            tracker.update(f64::from(beat) * 0.6, true, 0.2);
        }
        let learned = tracker.interval_seconds();
        assert!(learned > 0.55 && learned < 0.61, "learned {learned}");

        // A forward jump past 0.75 s re-anchors: the phase reads 0 at the seek
        // target, and the tempo estimate survives.
        let phase = tracker.update(30.0, false, 0.0).unwrap();
        assert_eq!(phase, 0.0);
        assert!(!tracker.has_onset(), "the phase reference is gone");
        assert_eq!(tracker.learned_intervals(), 0);
        assert_eq!(
            tracker.interval_seconds(),
            learned,
            "a credible interval survives a seek"
        );

        // A backwards jump does the same.
        assert_eq!(tracker.update(0.5, false, 0.0), Some(0.0));
    }

    /// `reset` is the public "forget everything" and is *not* the same as the
    /// internal restart on a discontinuity: it drops the tempo too.
    #[test]
    fn reset_discards_the_learned_tempo_unlike_a_seek() {
        let mut tracker = BeatTracker::new();
        for beat in 0..6 {
            tracker.update(f64::from(beat) * 0.6, true, 0.2);
        }
        assert!(tracker.interval_seconds() > 0.55);
        tracker.reset();
        assert_eq!(tracker, BeatTracker::default());
        assert_eq!(tracker.interval_seconds(), 0.5);
    }

    /// The blend weight drops from 0.42 to 0.20 after four observations, so a
    /// late outlier moves the estimate less than an early one. Pinned because a
    /// single constant looks arbitrary until something depends on it.
    #[test]
    fn learning_slows_down_after_four_observations() {
        fn interval_after(observations: usize) -> f64 {
            let mut tracker = BeatTracker::new();
            let mut time = 0.0;
            // Prime with exact 0.5 s beats so the estimate sits still at 0.5.
            for _ in 0..=observations {
                tracker.update(time, true, 0.2);
                time += 0.5;
            }
            assert_eq!(tracker.learned_intervals(), observations);
            assert!((tracker.interval_seconds() - 0.5).abs() < 1.0e-12);
            // Then one 0.7 s outlier and see how far it drags the estimate.
            tracker.update(time - 0.5 + 0.7, true, 0.2);
            tracker.interval_seconds()
        }

        let early = interval_after(2);
        let late = interval_after(5);
        assert!(
            (early - (0.5 + 0.2 * 0.42)).abs() < 1.0e-12,
            "early {early}"
        );
        assert!((late - (0.5 + 0.2 * 0.20)).abs() < 1.0e-12, "late {late}");
        assert!(early > late);
    }

    /// Octave folding is what stops a detector that fires twice per beat from
    /// halving the reported tempo.
    #[test]
    fn tempo_folding_agrees_with_the_running_estimate_by_octaves() {
        // Half the reference folds up to meet it.
        assert!((tempo_fold(0.3, 0.6) - 0.6).abs() < 1.0e-12);
        // Double the reference folds down.
        assert!((tempo_fold(1.2, 0.6) - 0.6).abs() < 1.0e-12);
        // Already in agreement: untouched.
        assert!((tempo_fold(0.6, 0.6) - 0.6).abs() < 1.0e-12);
        // Folding refuses to leave the credible range rather than doubling a
        // slow interval into an absurd one.
        assert!((tempo_fold(1.0, 4.0) - 1.0).abs() < 1.0e-12);
        assert!((tempo_fold(0.3, 0.05) - 0.3).abs() < 1.0e-12);
        // A fold result always stays inside [MINIMUM, MAXIMUM] when the input is.
        for reference in [0.25, 0.4, 0.5, 0.75, 1.0, 1.5] {
            for observed in [0.25, 0.3, 0.5, 0.6, 0.9, 1.2, 1.5] {
                let folded = tempo_fold(observed, reference);
                assert!(
                    (MINIMUM_INTERVAL..=MAXIMUM_INTERVAL).contains(&folded),
                    "fold({observed}, {reference}) = {folded}"
                );
            }
        }
    }
}
