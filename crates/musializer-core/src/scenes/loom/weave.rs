//! Loom's woven record of the song, raylib-free.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_loom_weave.c` and `.h`.
//! One of the three modules the C project extracted so it could be tested without
//! a window; keep it pure. Its C tests are ported at the bottom of this file from
//! `../musializer/tests/test_scene_loom_weave.c`.
//!
//! The weave keeps two kinds of measured-audio memory: short envelopes that make
//! the working edge feel alive, and a per-slot record of the spectrum frozen as
//! the fell passes it. The second is what makes the finished cloth a stable
//! tapestry of the whole song rather than a live spectrogram — **a woven slot is
//! stamped once and never rewritten**, so replaying or seeking backward cannot
//! change cloth that is already behind the playhead.

/// `scene_loom_weave.h:12-15`
pub const SLOTS: usize = 144;
pub const BINS: usize = 24;

/// One frame of input (`scene_loom_weave.h:17-26`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WeaveInput<'a> {
    pub time_seconds: f64,
    pub duration_seconds: f64,
    pub delta_seconds: f32,
    /// Analyzer smooth bands, lowest frequency first.
    pub bands: &'a [f32],
    pub rms: f32,
    pub spectral_flux: f32,
    pub onset: bool,
}

/// One frozen slot of cloth (`scene_loom_weave.h:28-33`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaveColumn {
    pub woven: bool,
    pub energy: f32,
    pub tension: f32,
    pub profile: [f32; BINS],
}

impl Default for WeaveColumn {
    fn default() -> Self {
        Self {
            woven: false,
            energy: 0.0,
            tension: 0.0,
            profile: [0.0; BINS],
        }
    }
}

/// `scene_loom_weave.h:35-45`
#[derive(Clone, Debug, PartialEq)]
pub struct Weave {
    pub initialized: bool,
    pub onset_active: bool,
    pub last_time_seconds: f64,
    /// Smoothed loudness, 0..1.
    pub energy: f32,
    /// Smoothed spectral flux, 0..1.
    pub tension: f32,
    /// 1.0 on an onset edge, exponential decay.
    pub onset_pulse: f32,
    /// Rising-edge count; anchors per-burst glint placement so a burst keeps its
    /// positions for the life of one envelope instead of teleporting per frame.
    pub onset_serial: u32,
    /// Live coarse spectrum, low bin first.
    pub profile: [f32; BINS],
    pub columns: [WeaveColumn; SLOTS],
}

impl Default for Weave {
    fn default() -> Self {
        Self {
            initialized: false,
            onset_active: false,
            last_time_seconds: 0.0,
            energy: 0.0,
            tension: 0.0,
            onset_pulse: 0.0,
            onset_serial: 0,
            profile: [0.0; BINS],
            columns: [WeaveColumn::default(); SLOTS],
        }
    }
}

/// `loom_weave_clamp01` (`scene_loom_weave.c:6-11`).
fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `loom_weave_band_target` (`:13-25`) — one coarse bin as the mean of the
/// analyzer bands it covers, lifted by 1.5 and clamped.
fn band_target(input: &WeaveInput<'_>, bin: usize) -> f32 {
    let count = input.bands.len();
    if count == 0 {
        return 0.0;
    }
    let begin = bin * count / BINS;
    let mut end = (bin + 1) * count / BINS;
    if end <= begin {
        end = begin + 1;
    }
    if end > count {
        end = count;
    }
    let total: f32 = input.bands[begin..end].iter().copied().map(clamp01).sum();
    clamp01(total / (end - begin) as f32 * 1.5)
}

impl Weave {
    /// `loom_weave_init` (`:70-74`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `loom_weave_register_onset` (`:27-34`).
    ///
    /// Only a *rising* edge starts a new burst: the analyzer keeps the onset flag
    /// high across a sustained hit, and re-rolling glint positions every frame of
    /// it would make them flicker.
    fn register_onset(&mut self, onset: bool) {
        if onset && !self.onset_active {
            self.onset_pulse = 1.0;
            self.onset_serial += 1;
        }
        self.onset_active = onset;
    }

    /// `loom_weave_rebase` (`:36-48`) — snap the live envelopes to the current
    /// measurement after a transport discontinuity.
    #[allow(clippy::needless_range_loop)] // `bin` is the frequency ordinate.
    fn rebase(&mut self, input: &WeaveInput<'_>, time: f64) {
        self.energy = clamp01(input.rms * 1.9);
        self.tension = clamp01(input.spectral_flux * 4.5);
        self.onset_pulse = 0.0;
        self.register_onset(input.onset);
        for bin in 0..BINS {
            self.profile[bin] = band_target(input, bin);
        }
        self.last_time_seconds = time;
        self.initialized = true;
    }

    /// `loom_weave_stamp_columns` (`:53-68`).
    ///
    /// Slots whose whole span lies behind the playhead are stamped once with the
    /// current smoothed measurement and then left alone. Seeking far forward
    /// stamps every skipped slot so the cloth never shows holes; they take the
    /// rebased measurement as their best available evidence.
    fn stamp_columns(&mut self, time: f64, duration: f64) {
        if !duration.is_finite() || duration <= 0.0 || time <= 0.0 {
            return;
        }
        let passed = time / duration * SLOTS as f64;
        let complete = if passed >= SLOTS as f64 {
            SLOTS
        } else {
            passed as usize
        };
        for slot in 0..complete {
            let column = &mut self.columns[slot];
            if column.woven {
                continue;
            }
            column.woven = true;
            column.energy = self.energy;
            column.tension = self.tension;
            column.profile = self.profile;
        }
    }

    /// `loom_weave_update` (`:76-116`).
    ///
    /// The asymmetric blends are deliberate: envelopes rise quickly and relax
    /// slowly so hits land and decays read. Rebase targets use the same scaling,
    /// so a seek does not shift the level the cloth records.
    // Same reasoning as `Motion::update`: each discontinuity clause names a
    // distinct transport situation, and the per-bin loops index a fixed-size array
    // whose index is also the frequency ordinate.
    #[allow(clippy::manual_range_contains, clippy::needless_range_loop)]
    pub fn update(&mut self, input: &WeaveInput<'_>) {
        let time = if input.time_seconds.is_finite() && input.time_seconds > 0.0 {
            input.time_seconds
        } else {
            0.0
        };
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
            self.rebase(input, time);
            self.stamp_columns(time, input.duration_seconds);
            return;
        }

        if delta > 0.1 {
            delta = 0.1;
        }
        self.last_time_seconds = time;
        if delta > 0.0 {
            let energy = clamp01(input.rms * 1.9);
            let tension = clamp01(input.spectral_flux * 4.5);
            let energy_blend =
                1.0 - ((if energy > self.energy { -10.0 } else { -3.0 }) * delta).exp();
            let tension_blend =
                1.0 - ((if tension > self.tension { -14.0 } else { -4.0 }) * delta).exp();
            self.energy += (energy - self.energy) * energy_blend;
            self.tension += (tension - self.tension) * tension_blend;
            self.onset_pulse *= (-4.0 * delta).exp();
            for bin in 0..BINS {
                let target = band_target(input, bin);
                let blend = 1.0
                    - ((if target > self.profile[bin] {
                        -12.0
                    } else {
                        -4.5
                    }) * delta)
                        .exp();
                self.profile[bin] += (target - self.profile[bin]) * blend;
            }
        }
        self.register_onset(input.onset);
        self.stamp_columns(time, input.duration_seconds);
    }
}

/// The slot whose song-time span contains `time_seconds`, clamped to a valid index
/// (`loom_weave_slot`, `:118-127`).
#[must_use]
pub fn slot(time_seconds: f64, duration_seconds: f64) -> usize {
    if !time_seconds.is_finite()
        || !duration_seconds.is_finite()
        || duration_seconds <= 0.0
        || time_seconds <= 0.0
    {
        return 0;
    }
    let position = time_seconds / duration_seconds * SLOTS as f64;
    if position >= (SLOTS - 1) as f64 {
        return SLOTS - 1;
    }
    position as usize
}

/// Linear sample of a coarse profile at `band_t` in `[0, 1]`; 0 is the lowest band
/// (`loom_weave_profile_sample`, `:129-143`).
#[must_use]
pub fn profile_sample(profile: &[f32; BINS], band_t: f32) -> f32 {
    if !band_t.is_finite() {
        return 0.0;
    }
    if band_t <= 0.0 {
        return clamp01(profile[0]);
    }
    if band_t >= 1.0 {
        return clamp01(profile[BINS - 1]);
    }
    let position = band_t * (BINS - 1) as f32;
    let low = position as usize;
    let high = if low + 1 < BINS { low + 1 } else { BINS - 1 };
    let fraction = position - low as f32;
    clamp01(profile[low] + (profile[high] - profile[low]) * fraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BANDS: usize = 64;

    fn bands(low: f32, high: f32) -> Vec<f32> {
        (0..TEST_BANDS)
            .map(|i| if i < TEST_BANDS / 2 { low } else { high })
            .collect()
    }

    fn input<'a>(
        time_seconds: f64,
        delta_seconds: f32,
        rms: f32,
        flux: f32,
        onset: bool,
        bands: &'a [f32],
        duration_seconds: f64,
    ) -> WeaveInput<'a> {
        WeaveInput {
            time_seconds,
            duration_seconds,
            delta_seconds,
            bands,
            rms,
            spectral_flux: flux,
            onset,
        }
    }

    /// Port of `loom_weave_records_columns_once_as_the_fell_passes`.
    #[test]
    fn columns_are_recorded_once_as_the_fell_passes() {
        let loud = bands(0.8, 0.05);
        let quiet = bands(0.02, 0.01);
        let duration = 14.4; // 0.1 seconds per slot

        let mut weave = Weave::new();
        for frame in 0..60u32 {
            weave.update(&input(
                f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                0.4,
                0.1,
                false,
                &loud,
                duration,
            ));
        }
        // 59/60 of a second played: slots 0..8 are complete, slot 9 is not.
        assert!(weave.columns[8].woven);
        assert!(!weave.columns[9].woven);
        let low_sample = profile_sample(&weave.columns[8].profile, 0.1);
        let high_sample = profile_sample(&weave.columns[8].profile, 0.9);
        assert!(low_sample > 0.4);
        assert!(low_sample > high_sample);

        let snapshot = weave.columns[8];
        for frame in 60..120u32 {
            weave.update(&input(
                f64::from(frame) / 60.0,
                1.0 / 60.0,
                0.01,
                0.0,
                false,
                &quiet,
                duration,
            ));
        }
        // Already-woven cloth is a stable record: later audio never restamps it.
        assert_eq!(snapshot, weave.columns[8]);
        assert!(weave.columns[18].woven);
        assert!(profile_sample(&weave.columns[18].profile, 0.1) < low_sample);
    }

    /// Port of `loom_weave_seeks_keep_the_woven_record`.
    #[test]
    fn seeking_keeps_the_woven_record() {
        let values = bands(0.6, 0.3);
        let duration = 14.4;

        let mut weave = Weave::new();
        for frame in 0..=120u32 {
            weave.update(&input(
                f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                0.3,
                0.05,
                false,
                &values,
                duration,
            ));
        }
        assert!(weave.columns[19].woven);
        let snapshot = weave.columns[10];

        // Seeking backward is a transport discontinuity: envelopes rebase, but the
        // tapestry behind the old fell survives untouched.
        weave.update(&input(0.5, 0.0, 0.9, 0.2, true, &values, duration));
        assert!(weave.columns[19].woven);
        assert_eq!(snapshot, weave.columns[10]);
        assert!((weave.energy - (0.9f32 * 1.9).min(1.0)).abs() < 0.0001);

        // Seeking far forward stamps the skipped slots so the cloth never shows
        // holes; they take the rebased measurement as their best evidence.
        weave.update(&input(10.0, 0.0, 0.2, 0.02, false, &values, duration));
        for slot in 0..100 {
            assert!(weave.columns[slot].woven, "slot {slot} left unwoven");
        }
        assert!(!weave.columns[100].woven);
    }

    /// Port of `loom_weave_onset_pulse_tracks_rising_edges`.
    #[test]
    fn the_onset_pulse_tracks_rising_edges_only() {
        let mut weave = Weave::new();
        let mut serial_before = 0u32;
        for frame in 0..30u32 {
            // The analyzer keeps the onset flag high across a sustained hit; only
            // the rising edge may start a new burst.
            let onset = (10..15).contains(&frame);
            weave.update(&input(
                f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                0.2,
                0.05,
                onset,
                &[],
                60.0,
            ));
            if frame == 9 {
                serial_before = weave.onset_serial;
            }
            if frame == 10 {
                assert_eq!(weave.onset_serial, serial_before + 1);
                assert!((weave.onset_pulse - 1.0).abs() < 0.0001);
            }
        }
        assert_eq!(weave.onset_serial, serial_before + 1);
        assert!(weave.onset_pulse < 1.0 && weave.onset_pulse > 0.0);

        weave.update(&input(0.5, 1.0 / 60.0, 0.2, 0.05, true, &[], 60.0));
        assert_eq!(weave.onset_serial, serial_before + 2);
    }

    /// Port of `loom_weave_is_deterministic_and_bounded_under_hostile_input`.
    #[test]
    fn hostile_input_stays_bounded_and_deterministic() {
        let mut first = Weave::new();
        let mut second = Weave::new();
        for frame in 0..240u32 {
            let values = bands(
                if frame % 2 != 0 { 3.0 } else { -1.0 },
                if frame % 3 != 0 { f32::NAN } else { 0.4 },
            );
            let sample = input(
                f64::from(frame) / 60.0,
                if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                if frame % 2 != 0 { 1.5 } else { -0.5 },
                if frame % 3 != 0 { 0.4 } else { f32::NAN },
                frame % 37 == 0,
                &values,
                4.0,
            );
            first.update(&sample);
            second.update(&sample);
            assert!((0.0..=1.0).contains(&first.energy));
            assert!((0.0..=1.0).contains(&first.tension));
            assert!((0.0..=1.0).contains(&first.onset_pulse));
            for bin in 0..BINS {
                assert!((0.0..=1.0).contains(&first.profile[bin]));
            }
        }
        assert_eq!(first, second);
        for column in &first.columns {
            if column.woven {
                assert!((0.0..=1.0).contains(&column.energy));
            }
        }
    }

    /// Port of `loom_weave_slot_and_profile_sample_clamp_edges`.
    #[test]
    fn slot_and_profile_sample_clamp_their_edges() {
        assert_eq!(slot(-1.0, 10.0), 0);
        assert_eq!(slot(5.0, 0.0), 0);
        assert_eq!(slot(5.0, -3.0), 0);
        assert_eq!(slot(f64::NAN, 10.0), 0);
        assert_eq!(slot(20.0, 10.0), SLOTS - 1);
        assert_eq!(slot(10.0, 10.0), SLOTS - 1);
        assert_eq!(slot(0.0, 10.0), 0);

        let mut profile = [0.0f32; BINS];
        for (bin, value) in profile.iter_mut().enumerate() {
            *value = bin as f32 / (BINS - 1) as f32;
        }
        assert!((profile_sample(&profile, 0.0) - 0.0).abs() < 0.0001);
        assert!((profile_sample(&profile, 1.0) - 1.0).abs() < 0.0001);
        assert!((profile_sample(&profile, 0.5) - 0.5).abs() < 0.03);
        assert!((profile_sample(&profile, -2.0) - 0.0).abs() < 0.0001);
        assert!(profile_sample(&profile, f32::NAN).abs() < 0.0001);
    }

    #[test]
    fn an_empty_band_slice_produces_a_flat_profile() {
        // A frame before the analyzer has produced anything must not panic on the
        // bin-range arithmetic.
        let mut weave = Weave::new();
        weave.update(&input(0.5, 1.0 / 60.0, 0.3, 0.1, false, &[], 60.0));
        assert_eq!(weave.profile, [0.0; BINS]);
    }
}
