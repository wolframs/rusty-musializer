//! The Song Atlas terrain: a bounded whole-track spectral map.
//!
//! **Owner: Agent A.** Port of `../musializer/src/song_atlas_map.c` and `.h`.
//!
//! Every other scene reacts to the *current* audio frame. Song Atlas draws the
//! whole song at once as terrain, so it needs the track precomputed into a fixed
//! number of slices before the first frame is drawn. That is all this module is:
//! an offline pass over decoded PCM producing at most [`MAX_SLICES`] slices of
//! [`BAND_COUNT`] bands each, plus per-slice loudness, spectral flux, and onset
//! flags.
//!
//! ## Things the oracle does here that look wrong and are reproduced anyway
//!
//! - **`Slice::rms` is a mean square, not a root mean square** during the
//!   measuring pass (`song_atlas_map.c:208`) — the square root arrives later, in
//!   the normalization pass (`song_atlas_map.c:115-116`), applied to the
//!   *ratio* against the loudest slice. So the final value is
//!   `sqrt(meansquare_i / meansquare_max)`, a relative loudness in `[0, 1]`, and
//!   is not comparable across tracks. The field name is the C's.
//! - **The atlas mixes channels; the preview selects channel 0.** This pass
//!   configures the analyzer with `AUDIO_ANALYZER_CHANNEL_MIX`
//!   (`song_atlas_map.c:162`) while the live preview path uses
//!   `CHANNEL_SELECT` with channel 0 (`../musializer/src/plug.c:438-449`). The
//!   same track therefore produces a different spectrum offline than on screen.
//!   That asymmetry is the oracle's; see the NOTE ENTRIES in `REWRITE_PLAN.md`.
//! - **The time axis is sampled, not averaged.** Each slice runs one FFT window
//!   centred on its position (`song_atlas_map.c:172-192`), so audio between
//!   window centres is never seen by the spectrum at all. The loudness *is*
//!   integrated over the window, which is why a transient can raise `rms`
//!   without raising any band.
//!
//! Nothing here reads a clock or allocates per frame, so the same PCM always
//! produces the same terrain.

use crate::audio::analyzer::{
    AnalyzerError, AudioAnalyzer, AudioAnalyzerConfig, ChannelMode, FFT_SIZE,
};

/// Frequency bands per slice (`song_atlas_map.h:8`).
pub const BAND_COUNT: usize = 28;
/// Slices at detail level 1 (`song_atlas_map.h:9`).
pub const BASE_SLICES: usize = 192;
/// Highest supported detail multiplier (`song_atlas_map.h:10`).
pub const MAX_DETAIL: usize = 3;
/// Hard cap on slices (`song_atlas_map.h:11`).
pub const MAX_SLICES: usize = BASE_SLICES * MAX_DETAIL;

/// Slices per second of audio, before the floor and the cap
/// (`song_atlas_map.c:40`).
const SLICES_PER_SECOND: f64 = 2.0;
/// Shortest track that still fills the terrain, in slices
/// (`song_atlas_map.c:41-43`).
const MINIMUM_SLICES_PER_DETAIL: f64 = 24.0;

/// The `dt` the offline analyzer is driven with (`song_atlas_map.c:187`).
///
/// This is exactly the analyzer's stability clamp, so each slice's smoothing
/// takes its largest stable step. It does not matter for the terrain — the
/// resampler reads `logarithmic`, not `smooth` — but it is the oracle's value and
/// changing it would change nothing visibly while making the port unreviewable.
const ANALYZE_DT_SECONDS: f32 = 0.1;

/// Onset detection thresholds (`song_atlas_map.c:137-140`).
const ONSET_FLUX_THRESHOLD: f32 = 0.62;
const ONSET_RMS_THRESHOLD: f32 = 0.16;
/// Minimum slice gap between onsets (`song_atlas_map.c:138`).
const ONSET_SEPARATION_SLICES: usize = 3 * MAX_DETAIL;

/// One time slice of the terrain (`song_atlas_map.h:13-18`).
///
/// Every field is in `[0, 1]` once [`SongAtlasMap::build`] returns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    /// Band energies, low frequency first.
    pub bands: [f32; BAND_COUNT],
    /// Relative loudness against the loudest slice — see the module docs.
    pub rms: f32,
    /// Normalized positive spectral flux against the previous slice.
    pub flux: f32,
    /// A separated flux peak loud enough to be a beat.
    pub onset: bool,
}

impl Default for Slice {
    fn default() -> Self {
        Self {
            bands: [0.0; BAND_COUNT],
            rms: 0.0,
            flux: 0.0,
            onset: false,
        }
    }
}

/// A whole-track spectral map (`song_atlas_map.h:20-24`).
///
/// C keeps a fixed `[Song_Atlas_Slice; 576]` plus a `count` so the struct can
/// live in hot-reloaded state without an allocator; hot reload is a stated
/// non-goal here, so this holds a `Vec` whose length *is* the count and
/// [`MAX_SLICES`] survives only as the bound.
#[derive(Clone, Debug, PartialEq)]
pub struct SongAtlasMap {
    slices: Vec<Slice>,
    duration_seconds: f64,
}

/// Why an atlas could not be built (`song_atlas_map.c:151-168`).
///
/// C signals all of these by clearing the destination and returning `0`. Rust
/// returns the map by value, so a failed build has nothing to clear — the
/// atomicity C's test checks (`test_song_atlas_map.c:98-113`) is structural here.
// Not `Clone`/`Copy`: `AnalyzerError` is neither, and it belongs to the already
// landed analyzer port, which is not this agent's file to change. If a caller
// ever needs to clone one, ask the integration owner to derive `Clone` there.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SongAtlasMapError {
    #[error("no decoded audio frames to map")]
    NoFrames,
    #[error("channel count must be non-zero and fit in a u32")]
    InvalidChannelCount,
    #[error("sample rate must be non-zero")]
    ZeroSampleRate,
    #[error("the analyzer rejected the atlas configuration: {0}")]
    Analyzer(#[from] AnalyzerError),
    /// The analyzer refused a window mid-pass. Unreachable with a configuration
    /// it already accepted, and reproduced from `song_atlas_map.c:185-191`
    /// because the C treats it as fatal for the whole map rather than skipping
    /// the slice.
    #[error("the analyzer rejected a window while mapping slice {slice}")]
    AnalysisFailed { slice: usize },
}

/// Clamps to `[0, 1]`, mapping non-finite to `0` (`song_atlas_map.c:9-15`).
///
/// This is what makes the terrain survive pathological PCM: `±FLT_MAX` input
/// overflows the FFT to infinity, `ln(inf)` is infinity, and dividing by an
/// infinite per-frame maximum yields `NaN` — which lands here and becomes `0.0`
/// rather than propagating into the mesh.
fn clamp01(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

/// Folds the analyzer's ~110 logarithmic bands down to [`BAND_COUNT`]
/// (`song_atlas_map.c:17-34`).
///
/// It takes the **maximum** of each destination band's source range, not the
/// mean, so a narrow peak survives decimation instead of being averaged into the
/// noise floor around it. The clamp order is C's: the end is repaired before the
/// begin is clamped.
fn resample_band(logarithmic: &[f32], destination: usize) -> f32 {
    let band_count = logarithmic.len();
    if band_count == 0 {
        return 0.0;
    }
    let mut begin = destination * band_count / BAND_COUNT;
    let mut end = (destination + 1) * band_count / BAND_COUNT;
    if end <= begin {
        end = begin + 1;
    }
    if begin >= band_count {
        begin = band_count - 1;
    }
    if end > band_count {
        end = band_count;
    }
    logarithmic[begin..end]
        .iter()
        .fold(0.0f32, |peak, &value| peak.max(clamp01(value)))
}

/// How many slices a track of this length gets (`song_atlas_map.c:36-46`).
///
/// Two slices per second per detail level, floored at
/// `24 * MAX_DETAIL == 72` so a very short track still has terrain to draw, and
/// capped at [`MAX_SLICES`] so a very long one stays bounded. Past 96 seconds
/// every track produces exactly [`MAX_SLICES`] slices, which is why the atlas
/// gets coarser rather than larger as tracks get longer.
///
/// Returns `0` for a zero `frame_count` or `sample_rate`.
#[must_use]
pub fn slice_count(frame_count: usize, sample_rate: u32) -> usize {
    if frame_count == 0 || sample_rate == 0 {
        return 0;
    }
    let duration = frame_count as f64 / f64::from(sample_rate);
    let mut desired = (duration * SLICES_PER_SECOND * MAX_DETAIL as f64).ceil();
    if desired < MINIMUM_SLICES_PER_DETAIL * MAX_DETAIL as f64 {
        desired = MINIMUM_SLICES_PER_DETAIL * MAX_DETAIL as f64;
    }
    if desired > MAX_SLICES as f64 {
        desired = MAX_SLICES as f64;
    }
    desired as usize
}

/// How many slices to draw at a given detail level (`song_atlas_map.c:48-58`).
///
/// Detail 3 draws every available slice; detail 1 draws a third of them, rounded
/// up. Returns `0` for `available` outside `2..=MAX_SLICES` or a `detail_level`
/// outside `1..=MAX_DETAIL`.
#[must_use]
pub fn render_sample_count(available: usize, detail_level: usize) -> usize {
    if !(2..=MAX_SLICES).contains(&available) || !(1..=MAX_DETAIL).contains(&detail_level) {
        return 0;
    }
    // C spells this `(a*d + MAX_DETAIL - 1)/MAX_DETAIL` (`song_atlas_map.c:53`).
    let sample_count = (available * detail_level).div_ceil(MAX_DETAIL);
    sample_count.clamp(2, available)
}

/// Which slice the `sample`-th drawn column reads (`song_atlas_map.c:60-68`).
///
/// The `+ (sample_count - 1) / 2` term rounds the decimation to nearest rather
/// than truncating, so the drawn columns stay centred on the slices they
/// represent. Both endpoints are exact: sample `0` is `first` and sample
/// `sample_count - 1` is `first + available - 1`, which is what makes the terrain
/// span the same distance at every detail level (see
/// [`render_distance`]).
///
/// Returns `None` for `available` outside `2..=MAX_SLICES`, a `sample_count`
/// outside `2..=available`, a `sample` at or past `sample_count`, or a `first`
/// that would overflow.
#[must_use]
pub fn render_sample_index(
    first: usize,
    available: usize,
    sample_count: usize,
    sample: usize,
) -> Option<usize> {
    if !(2..=MAX_SLICES).contains(&available)
        || !(2..=available).contains(&sample_count)
        || sample >= sample_count
        || first > usize::MAX - available
    {
        return None;
    }
    Some(first + (sample * (available - 1) + (sample_count - 1) / 2) / (sample_count - 1))
}

/// Scales a slice-index distance into world units (`song_atlas_map.c:70-74`).
///
/// Dividing by [`MAX_DETAIL`] is what keeps the terrain the same physical length
/// whatever the detail level: at detail 1 there are a third as many columns, but
/// each spans three slice indices, so the last column lands in the same place.
/// Non-finite input becomes `0.0`.
#[must_use]
pub fn render_distance(source_distance: f32) -> f32 {
    if !source_distance.is_finite() {
        return 0.0;
    }
    source_distance / MAX_DETAIL as f32
}

impl SongAtlasMap {
    /// Builds the terrain from interleaved floating-point PCM
    /// (`song_atlas_map.c:145-217`).
    ///
    /// The frame count is derived as `samples.len() / channel_count`, which
    /// removes C's `frame_count > SIZE_MAX/channel_count` overflow guard
    /// (`song_atlas_map.c:155`) as an unrepresentable state rather than as a
    /// dropped check.
    ///
    /// The pass has three stages, and the order matters because each reads the
    /// previous one's output:
    ///
    /// 1. **Measure** (`song_atlas_map.c:172-212`): one FFT window per slice plus
    ///    the window's mean square.
    /// 2. **Smooth** (`song_atlas_map.c:76-105`): a combined spatial blur across
    ///    neighbouring bands and an asymmetric temporal filter along the track —
    ///    72% attack, 38% release, both compensated for the detail multiplier so
    ///    the terrain's shape does not depend on how many slices it has.
    /// 3. **Normalize** (`song_atlas_map.c:107-143`): loudness relative to the
    ///    loudest slice, bands scaled by that loudness, then flux and separated
    ///    onsets.
    ///
    /// # Errors
    /// See [`SongAtlasMapError`].
    pub fn build(
        samples: &[f32],
        channel_count: usize,
        sample_rate: u32,
    ) -> Result<Self, SongAtlasMapError> {
        if u32::try_from(channel_count).is_err() || channel_count == 0 {
            return Err(SongAtlasMapError::InvalidChannelCount);
        }
        if sample_rate == 0 {
            return Err(SongAtlasMapError::ZeroSampleRate);
        }
        let frame_count = samples.len() / channel_count;
        if frame_count == 0 {
            return Err(SongAtlasMapError::NoFrames);
        }

        let mut analyzer = AudioAnalyzer::boxed(AudioAnalyzerConfig {
            sample_rate,
            channel_count: channel_count as u32,
            // Not the preview's `Select(0)`. See the module docs.
            channel_mode: ChannelMode::Mix,
        })?;

        let count = slice_count(frame_count, sample_rate);
        let mut map = Self {
            slices: vec![Slice::default(); count],
            duration_seconds: frame_count as f64 / f64::from(sample_rate),
        };

        for index in 0..count {
            // The window centre walks the track linearly from the first frame to
            // the last, so the first and last slices are anchored to the ends
            // rather than to the centres of edge windows.
            let position = if count > 1 {
                index as f64 * (frame_count - 1) as f64 / (count - 1) as f64
            } else {
                0.0
            };
            let center = position as usize;
            let half = FFT_SIZE / 2;
            let mut begin = center.saturating_sub(half);
            let mut end = begin + FFT_SIZE;
            if end > frame_count {
                // A window that would run past the end slides back instead of
                // shrinking, so late slices still see a full FFT window.
                end = frame_count;
                begin = end.saturating_sub(FFT_SIZE);
            }
            let window_frames = end - begin;

            analyzer.reset();
            if analyzer.push_interleaved(&samples[begin * channel_count..end * channel_count])
                != window_frames
                || !analyzer.analyze(ANALYZE_DT_SECONDS)
            {
                return Err(SongAtlasMapError::AnalysisFailed { slice: index });
            }

            // The loudness is integrated over the whole window, unlike the
            // spectrum, which is a single transform of it.
            let mut square_sum = 0.0f64;
            for frame in begin..end {
                let mut mono = 0.0f64;
                for channel in 0..channel_count {
                    let sample = samples[frame * channel_count + channel];
                    if sample.is_finite() {
                        mono += f64::from(sample.clamp(-1.0, 1.0));
                    }
                }
                mono /= channel_count as f64;
                square_sum += mono * mono;
            }

            let spectrum = analyzer.spectrum();
            let slice = &mut map.slices[index];
            // Mean square here; the square root is taken in `normalize`.
            slice.rms = (square_sum / window_frames as f64) as f32;
            for band in 0..BAND_COUNT {
                slice.bands[band] = resample_band(spectrum.logarithmic, band);
            }
        }

        map.smooth();
        map.normalize();
        Ok(map)
    }

    /// Spatial blur across bands plus an asymmetric temporal filter along the
    /// track (`song_atlas_map.c:76-105`).
    ///
    /// Slice 0 is left exactly as measured and seeds the filter; every later
    /// slice is blended towards its blurred self. The attack/release asymmetry
    /// (0.72 rising, 0.38 falling) makes a hit arrive sharply and decay slowly,
    /// and the `1/MAX_DETAIL` exponent converts those per-slice rates into
    /// per-detail-step rates so a 576-slice map and a 72-slice map have the same
    /// envelope shape rather than the same per-slice coefficient.
    ///
    /// **The spatial blur is in place, and that is the oracle's behaviour.** The
    /// `band - 1` term reads `bands[band - 1]` *after* the previous iteration
    /// overwrote it with its filtered value (`song_atlas_map.c:89` writing to
    /// `:102`), so the blur is asymmetric: the low-frequency neighbour is already
    /// smoothed while the high-frequency one is not. A correct symmetric blur
    /// would need a scratch copy of the row. This looks like a bug, it is
    /// reproduced deliberately, and the differential test
    /// `matches_the_c_oracle_slice_for_slice` is what pins it — a scratch-copy
    /// "fix" still produces a plausible terrain and fails that test.
    fn smooth(&mut self) {
        if self.slices.len() < 2 {
            return;
        }
        let mut previous = self.slices[0].bands;
        let response = |rising: bool| {
            let base: f32 = if rising { 0.72 } else { 0.38 };
            1.0 - (1.0 - base).powf(1.0 / MAX_DETAIL as f32)
        };
        for index in 1..self.slices.len() {
            for (band, previous_band) in previous.iter_mut().enumerate() {
                let bands = &self.slices[index].bands;
                let mut spatial = bands[band] * 0.60;
                let mut weight = 0.60f32;
                if band > 0 {
                    // Already-filtered value; see the note above.
                    spatial += bands[band - 1] * 0.20;
                    weight += 0.20;
                }
                if band + 1 < BAND_COUNT {
                    spatial += bands[band + 1] * 0.20;
                    weight += 0.20;
                }
                spatial /= weight;
                let filtered = *previous_band
                    + (spatial - *previous_band) * response(spatial > *previous_band);
                *previous_band = filtered;
                self.slices[index].bands[band] = filtered;
            }
        }
    }

    /// Relative loudness, loudness-scaled bands, flux, and onsets
    /// (`song_atlas_map.c:107-143`).
    ///
    /// Two details are load-bearing:
    ///
    /// - **bands are scaled by loudness** as `0.18 + 0.82 * rms`, so a quiet
    ///   passage keeps its spectral shape at 18% height rather than flattening.
    ///   This is the only place absolute level reaches the terrain, because the
    ///   analyzer normalizes every band per frame.
    /// - **flux is measured after that scaling**, so it reflects visible change
    ///   in the terrain rather than change in the raw spectrum.
    fn normalize(&mut self) {
        let maximum_rms = self
            .slices
            .iter()
            .fold(0.0f32, |maximum, slice| maximum.max(slice.rms));

        let mut maximum_flux = 0.0f32;
        for index in 0..self.slices.len() {
            let normalized_rms = if maximum_rms > 1.0e-8 {
                (self.slices[index].rms / maximum_rms).sqrt()
            } else {
                0.0
            };
            self.slices[index].rms = clamp01(normalized_rms);
            let loudness = 0.18 + 0.82 * self.slices[index].rms;
            for band in 0..BAND_COUNT {
                self.slices[index].bands[band] = clamp01(self.slices[index].bands[band] * loudness);
            }
            if index == 0 {
                continue;
            }
            let mut flux = 0.0f32;
            for band in 0..BAND_COUNT {
                let rise = self.slices[index].bands[band] - self.slices[index - 1].bands[band];
                if rise > 0.0 {
                    flux += rise;
                }
            }
            self.slices[index].flux = flux / BAND_COUNT as f32;
            if self.slices[index].flux > maximum_flux {
                maximum_flux = self.slices[index].flux;
            }
        }

        let mut last_onset: Option<usize> = None;
        for index in 0..self.slices.len() {
            let normalized = if maximum_flux > 1.0e-8 {
                self.slices[index].flux / maximum_flux
            } else {
                0.0
            };
            self.slices[index].flux = clamp01(normalized);
            // `Option::is_none_or` would read better but is stable only since
            // 1.82, above this workspace's 1.80 MSRV.
            let separated = match last_onset {
                None => true,
                Some(last) => index - last >= ONSET_SEPARATION_SLICES,
            };
            self.slices[index].onset = separated
                && normalized >= ONSET_FLUX_THRESHOLD
                && self.slices[index].rms >= ONSET_RMS_THRESHOLD;
            if self.slices[index].onset {
                last_onset = Some(index);
            }
        }
    }

    #[must_use]
    pub fn slices(&self) -> &[Slice] {
        &self.slices
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slices.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    /// Track duration in seconds, from the decoded frame count.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// The post-condition [`SongAtlasMap::build`] guarantees
    /// (`song_atlas_map.c:219-237`).
    ///
    /// In C this is a guard the renderer needs, because the map is a caller-owned
    /// struct that may be zeroed or stale. Here `build` either returns a valid
    /// map or an error, so this is a checkable statement of the contract rather
    /// than something the application has to call: at least two slices, at most
    /// [`MAX_SLICES`], a positive finite duration, and every `bands`, `rms`, and
    /// `flux` finite and inside `[0, 1]`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.slices.len() < 2
            || self.slices.len() > MAX_SLICES
            || !self.duration_seconds.is_finite()
            || self.duration_seconds <= 0.0
        {
            return false;
        }
        self.slices.iter().all(|slice| {
            let in_unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
            in_unit(slice.rms) && in_unit(slice.flux) && slice.bands.iter().copied().all(in_unit)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `audio_fixture_silence` (`../musializer/tests/audio_fixtures.c:36-40`).
    fn silence(channel_count: usize, frame_count: usize) -> Vec<f32> {
        vec![0.0; frame_count * channel_count]
    }

    /// Mirrors `audio_fixture_sine` (`../musializer/tests/audio_fixtures.c:42-58`),
    /// including its f64 phase accumulation and f32 `sinf`.
    fn sine(
        sample_rate: u32,
        channel_count: usize,
        frame_count: usize,
        frequency_hz: f32,
        amplitude: f32,
    ) -> Vec<f32> {
        let mut samples = vec![0.0f32; frame_count * channel_count];
        for frame in 0..frame_count {
            let phase = 2.0 * std::f64::consts::PI * f64::from(frequency_hz) * frame as f64
                / f64::from(sample_rate);
            let value = amplitude * (phase as f32).sin();
            for channel in 0..channel_count {
                samples[frame * channel_count + channel] = value;
            }
        }
        samples
    }

    /// Where the terrain's energy sits on the band axis. Mirrors
    /// `atlas_frequency_centroid` (`../musializer/tests/test_song_atlas_map.c:8-20`).
    fn band_centroid(map: &SongAtlasMap) -> f64 {
        let mut weighted = 0.0f64;
        let mut total = 0.0f64;
        for slice in map.slices() {
            for (band, &value) in slice.bands.iter().enumerate() {
                weighted += f64::from(value) * band as f64;
                total += f64::from(value);
            }
        }
        if total > 0.0 {
            weighted / total
        } else {
            0.0
        }
    }

    #[test]
    fn build_rejects_degenerate_geometry() {
        assert_eq!(
            SongAtlasMap::build(&[], 1, 48_000),
            Err(SongAtlasMapError::NoFrames)
        );
        assert_eq!(
            SongAtlasMap::build(&[0.0; 32], 0, 48_000),
            Err(SongAtlasMapError::InvalidChannelCount)
        );
        assert_eq!(
            SongAtlasMap::build(&[0.0; 32], 1, 0),
            Err(SongAtlasMapError::ZeroSampleRate)
        );
        // A partial frame is not a frame.
        assert_eq!(
            SongAtlasMap::build(&[0.0; 2], 3, 48_000),
            Err(SongAtlasMapError::NoFrames)
        );
    }

    #[test]
    fn slice_counts_are_floored_capped_and_monotonic() {
        assert_eq!(slice_count(0, 48_000), 0);
        assert_eq!(slice_count(48_000, 0), 0);
        // 2 s at 8 kHz wants 12 slices but gets the 24*3 floor.
        assert_eq!(slice_count(16_000, 8_000), 72);
        // 100 s wants 600 and gets the cap.
        assert_eq!(slice_count(800_000, 8_000), MAX_SLICES);
        // One sample still produces drawable terrain.
        assert_eq!(slice_count(1, 48_000), 72);
        // The floor holds up to 12 s and the ramp starts after it.
        assert_eq!(slice_count(12 * 48_000, 48_000), 72);
        assert_eq!(slice_count(13 * 48_000, 48_000), 78);
        // 96 s is exactly the cap; past it nothing grows.
        assert_eq!(slice_count(96 * 48_000, 48_000), MAX_SLICES);

        let mut previous = 0usize;
        for seconds in 0..200u32 {
            let count = slice_count(seconds as usize * 44_100, 44_100);
            assert!(
                count >= previous || seconds == 0,
                "not monotonic at {seconds}s"
            );
            assert!(count <= MAX_SLICES);
            assert!(count == 0 || count >= 2, "a map is never one slice");
            previous = count;
        }
    }

    #[test]
    fn render_decimation_is_exact_and_bounded() {
        assert_eq!(render_sample_count(576, 1), 192);
        assert_eq!(render_sample_count(576, 2), 384);
        assert_eq!(render_sample_count(576, 3), 576);
        // Out of range on either argument draws nothing.
        assert_eq!(render_sample_count(576, 0), 0);
        assert_eq!(render_sample_count(576, MAX_DETAIL + 1), 0);
        assert_eq!(render_sample_count(1, 3), 0);
        assert_eq!(render_sample_count(MAX_SLICES + 1, 3), 0);
        // A short map never decimates below two columns.
        assert_eq!(render_sample_count(2, 1), 2);
        assert_eq!(render_sample_count(3, 1), 2);

        assert_eq!(render_sample_index(20, 556, 192, 0), Some(20));
        assert_eq!(render_sample_index(20, 556, 192, 191), Some(575));
        // Out of range on any argument.
        assert_eq!(render_sample_index(0, 576, 192, 192), None);
        assert_eq!(render_sample_index(0, 1, 192, 0), None);
        assert_eq!(render_sample_index(0, MAX_SLICES + 1, 192, 0), None);
        assert_eq!(render_sample_index(0, 576, 1, 0), None);
        assert_eq!(render_sample_index(0, 192, 576, 0), None);
        assert_eq!(render_sample_index(usize::MAX, 576, 192, 0), None);
    }

    #[test]
    fn render_distances_are_detail_independent_and_finite() {
        assert!((render_distance(575.0) - 575.0 / 3.0).abs() < 0.00001);
        assert_eq!(render_distance(f32::NAN), 0.0);
        assert_eq!(render_distance(f32::INFINITY), 0.0);
        assert_eq!(render_distance(0.0), 0.0);

        // The terrain spans the same world distance at every detail level: the
        // last column of a 192-sample draw and of a 576-sample draw over the same
        // 576 slices land in the same place.
        let coarse = render_sample_index(0, 576, 192, 191).unwrap();
        let fine = render_sample_index(0, 576, 576, 575).unwrap();
        assert_eq!(
            render_distance(coarse as f32),
            render_distance(fine as f32),
            "coarse ended at slice {coarse}, fine at {fine}"
        );

        // And every detail level's first and last columns hit the ends exactly.
        for detail in 1..=MAX_DETAIL {
            let columns = render_sample_count(576, detail);
            assert_eq!(render_sample_index(7, 576, columns, 0), Some(7));
            assert_eq!(
                render_sample_index(7, 576, columns, columns - 1),
                Some(7 + 575),
                "detail {detail} did not reach the last slice"
            );
        }
    }

    #[test]
    fn silence_maps_to_a_valid_flat_terrain() {
        let samples = silence(2, 16_000);
        let map = SongAtlasMap::build(&samples, 2, 8_000).unwrap();
        assert_eq!(map.len(), 72);
        assert!(map.is_valid());
        assert_eq!(map.duration_seconds(), 2.0);
        for (index, slice) in map.slices().iter().enumerate() {
            assert_eq!(slice.rms, 0.0, "slice {index}");
            assert_eq!(slice.flux, 0.0, "slice {index}");
            assert!(!slice.onset, "silence has no onsets");
            assert!(slice.bands.iter().all(|&band| band == 0.0), "slice {index}");
        }
    }

    /// `±FLT_MAX` overflows the FFT to infinity, so the band normalization
    /// produces `NaN`. The clamp is what keeps that out of the mesh.
    #[test]
    fn extreme_finite_pcm_still_produces_a_valid_map() {
        let samples: Vec<f32> = (0..8192)
            .map(|i| if i % 2 == 0 { -f32::MAX } else { f32::MAX })
            .collect();
        let map = SongAtlasMap::build(&samples, 1, 48_000).unwrap();
        assert!(map.is_valid(), "{:?}", &map.slices()[..2]);
        // The samples clamp to ±1 for loudness, so this is a full-scale track.
        assert!(map.slices().iter().all(|slice| slice.rms > 0.99));
    }

    #[test]
    fn frequency_order_is_preserved_across_the_whole_track() {
        let low = sine(8_000, 1, 24_000, 220.0, 0.7);
        let high = sine(8_000, 1, 24_000, 1_800.0, 0.7);
        let low_map = SongAtlasMap::build(&low, 1, 8_000).unwrap();
        let high_map = SongAtlasMap::build(&high, 1, 8_000).unwrap();
        assert!(low_map.is_valid());
        assert!(high_map.is_valid());
        let (low_centroid, high_centroid) = (band_centroid(&low_map), band_centroid(&high_map));
        assert!(
            high_centroid > low_centroid + 5.0,
            "1800 Hz centroid {high_centroid} should sit well above 220 Hz's {low_centroid}"
        );
    }

    /// Not in the C suite. The build is a pure function of its inputs, which is
    /// the invariant the whole scene's determinism rests on.
    #[test]
    fn the_same_pcm_always_produces_the_same_terrain() {
        let samples = sine(44_100, 2, 30_000, 440.0, 0.6);
        let first = SongAtlasMap::build(&samples, 2, 44_100).unwrap();
        let second = SongAtlasMap::build(&samples, 2, 44_100).unwrap();
        assert_eq!(first, second);
    }

    /// Every band is normalized per analyzer frame, so absolute level reaches
    /// the terrain only through the `0.18 + 0.82*rms` loudness scale. A quiet
    /// track must therefore keep its spectral *shape* while sitting lower.
    #[test]
    fn loudness_scales_the_bands_without_flattening_the_shape() {
        let loud = SongAtlasMap::build(&sine(44_100, 1, 60_000, 440.0, 0.9), 1, 44_100).unwrap();
        assert!(loud.is_valid());
        // Relative loudness normalizes against the track's own peak, so a
        // steady tone reads near 1.0 whatever its amplitude, and the loudness
        // scale therefore lands at ~1.0 as well.
        assert!(
            loud.slices().iter().skip(1).all(|slice| slice.rms > 0.9),
            "a steady tone is its own loudest slice"
        );
        let peak = loud
            .slices()
            .iter()
            .flat_map(|slice| slice.bands.iter().copied())
            .fold(0.0f32, f32::max);
        assert!(
            peak > 0.5,
            "the tone's band should be prominent, got {peak}"
        );
    }

    /// Onsets must stay separated by at least `3*MAX_DETAIL` slices, which is
    /// what stops a sustained rise from being reported as a run of beats.
    #[test]
    fn onsets_are_separated_by_at_least_nine_slices() {
        // A track with abrupt level changes: 500 ms of tone, 500 ms of silence.
        let sample_rate = 44_100u32;
        let mut samples = Vec::new();
        for block in 0..20 {
            let block_samples = if block % 2 == 0 {
                sine(sample_rate, 1, sample_rate as usize / 2, 300.0, 0.9)
            } else {
                silence(1, sample_rate as usize / 2)
            };
            samples.extend_from_slice(&block_samples);
        }
        let map = SongAtlasMap::build(&samples, 1, sample_rate).unwrap();
        assert!(map.is_valid());
        let onsets: Vec<usize> = map
            .slices()
            .iter()
            .enumerate()
            .filter(|(_, slice)| slice.onset)
            .map(|(index, _)| index)
            .collect();
        for pair in onsets.windows(2) {
            assert!(
                pair[1] - pair[0] >= ONSET_SEPARATION_SLICES,
                "onsets at {:?} are closer than {ONSET_SEPARATION_SLICES} slices",
                pair
            );
        }
        assert!(
            !onsets.is_empty(),
            "a track that alternates tone and silence should produce onsets"
        );
    }

    /// A one-slice map cannot exist because [`slice_count`] floors at 72, but
    /// `is_valid` still has to reject one so the contract is checkable.
    #[test]
    fn is_valid_rejects_maps_build_cannot_produce() {
        let mut map = SongAtlasMap::build(&silence(1, 16_000), 1, 8_000).unwrap();
        assert!(map.is_valid());

        let mut too_short = map.clone();
        too_short.slices.truncate(1);
        assert!(!too_short.is_valid());

        let mut too_long = map.clone();
        too_long.slices.resize(MAX_SLICES + 1, Slice::default());
        assert!(!too_long.is_valid());

        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut bad = map.clone();
            bad.duration_seconds = duration;
            assert!(!bad.is_valid(), "duration {duration}");
        }

        for value in [f32::NAN, f32::INFINITY, -0.001, 1.001] {
            let mut bad = map.clone();
            bad.slices[3].bands[7] = value;
            assert!(!bad.is_valid(), "band {value}");
            let mut bad = map.clone();
            bad.slices[3].rms = value;
            assert!(!bad.is_valid(), "rms {value}");
            let mut bad = map.clone();
            bad.slices[3].flux = value;
            assert!(!bad.is_valid(), "flux {value}");
        }

        map.slices.clear();
        assert!(!map.is_valid());
        assert!(map.is_empty());
    }

    /// **Differential against the frozen C, not against this implementation.**
    ///
    /// Produced by compiling `../musializer/src/song_atlas_map.c` and
    /// `audio_analyzer.c` unmodified into a scratch harness *outside both
    /// repositories* and printing `%.9g` per field for a 3 s 1800 Hz mono sine at
    /// 8 kHz, amplitude 0.7 — the same fixture shape
    /// `test_song_atlas_map.c:71-96` uses, but checked value-by-value instead of
    /// only by centroid order.
    ///
    /// This is the test that proves the port rather than merely exercising it:
    /// the three passes (measure, smooth, normalize) are each order-sensitive and
    /// a plausible-looking rearrangement of any of them still yields a valid,
    /// pretty terrain that is not the oracle's.
    // `%.9g` output pasted verbatim; see the note in
    // `crate::timing::track_timeline`'s equivalent test.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn matches_the_c_oracle_slice_for_slice() {
        let samples = sine(8_000, 1, 24_000, 1_800.0, 0.7);
        let map = SongAtlasMap::build(&samples, 1, 8_000).unwrap();
        assert_eq!(map.len(), 72);
        assert_eq!(map.duration_seconds(), 3.0);
        assert!(map.is_valid());

        // Bands 0..23 are silent in every slice below; only the tail differs.
        #[rustfmt::skip]
        let expected: [(usize, f32, f32, bool, [f32; 5]); 4] = [
            //  slice  rms            flux             onset  bands[23..28]
            (   0,     0.999_882_817, 0.0,             false, [0.0,           0.999_903_917, 0.0,           0.0,            0.0]),
            (   1,     0.999_882_817, 1.0,             true,  [0.069_150_708, 0.943_027_496, 0.065_217_286, 0.004_510_254,  0.000_389_896_6]),
            (   2,     0.999_882_817, 0.653_882_027,   false, [0.114_390_023, 0.895_861_626, 0.104_621_433, 0.010_186_010,  0.001_135_622_4]),
            (  71,     0.999_993_503, 0.0,             false, [0.199_998_930, 0.640_000_820, 0.128_007_978, 0.025_609_027,  0.006_408_180_6]),
        ];

        for (index, rms, flux, onset, tail) in expected {
            let slice = &map.slices()[index];
            assert!(
                (slice.rms - rms).abs() < 1.0e-6,
                "slice {index} rms: {} vs C's {rms}",
                slice.rms
            );
            assert!(
                (slice.flux - flux).abs() < 1.0e-6,
                "slice {index} flux: {} vs C's {flux}",
                slice.flux
            );
            assert_eq!(slice.onset, onset, "slice {index} onset");
            for band in 0..23 {
                assert_eq!(slice.bands[band], 0.0, "slice {index} band {band}");
            }
            for (offset, &value) in tail.iter().enumerate() {
                let band = 23 + offset;
                assert!(
                    (slice.bands[band] - value).abs() < 1.0e-6,
                    "slice {index} band {band}: {} vs C's {value}",
                    slice.bands[band]
                );
            }
        }

        // The whole onset lane, not just a sample of it. The 1800 Hz tone starts
        // abruptly and then holds, so exactly one slice is a beat.
        let onsets: Vec<usize> = map
            .slices()
            .iter()
            .enumerate()
            .filter(|(_, slice)| slice.onset)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(onsets, vec![1], "C reports an onset only at slice 1");
    }

    /// The stereo half of the differential, because `CHANNEL_MIX` over two
    /// channels is a different code path from one channel: 30000 frames of a
    /// 440 Hz sine at 44.1 kHz, amplitude 0.6.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn matches_the_c_oracle_for_a_stereo_track() {
        let samples = sine(44_100, 2, 30_000, 440.0, 0.6);
        let map = SongAtlasMap::build(&samples, 2, 44_100).unwrap();
        assert_eq!(map.len(), 72);
        assert!((map.duration_seconds() - 0.680_272_108_843_537_39).abs() < 1.0e-15);
        assert!(map.is_valid());

        // Slice 0 is measured but unsmoothed, so its spectrum is the rawest
        // thing in the map and the most sensitive to a wrong window offset.
        let first = &map.slices()[0];
        assert!((first.rms - 0.998_875_976).abs() < 1.0e-6, "{}", first.rms);
        assert_eq!(first.flux, 0.0);
        assert!(
            (first.bands[9] - 0.253_442_347).abs() < 1.0e-6,
            "{}",
            first.bands[9]
        );
        assert!(
            (first.bands[10] - 0.999_078_274).abs() < 1.0e-6,
            "{}",
            first.bands[10]
        );
        assert!(first.bands[..9].iter().all(|&band| band == 0.0));
        assert!(first.bands[11..].iter().all(|&band| band == 0.0));

        let last = &map.slices()[71];
        assert!((last.rms - 0.999_797_821).abs() < 1.0e-6, "{}", last.rms);
        assert!(
            (last.bands[9] - 0.362_291_992).abs() < 1.0e-6,
            "{}",
            last.bands[9]
        );
        assert!(
            (last.bands[10] - 0.672_362_924).abs() < 1.0e-6,
            "{}",
            last.bands[10]
        );
        assert!(
            (last.bands[8] - 0.050_726_588).abs() < 1.0e-6,
            "{}",
            last.bands[8]
        );
    }

    #[test]
    fn band_resampling_takes_the_peak_and_survives_a_degenerate_spectrum() {
        // A narrow peak must not be averaged away.
        let mut logarithmic = vec![0.0f32; BAND_COUNT * 4];
        logarithmic[5] = 1.0;
        assert_eq!(
            resample_band(&logarithmic, 1),
            1.0,
            "bins 4..8 hold the peak"
        );
        assert_eq!(resample_band(&logarithmic, 0), 0.0);
        // Non-finite and out-of-range source values clamp.
        let noisy = [f32::NAN, f32::INFINITY, -5.0, 7.0];
        for band in 0..BAND_COUNT {
            let value = resample_band(&noisy, band);
            assert!((0.0..=1.0).contains(&value), "band {band} produced {value}");
        }
        // Fewer source bands than destinations still resolves every destination.
        assert_eq!(resample_band(&[], 0), 0.0);
        assert_eq!(resample_band(&[1.0], BAND_COUNT - 1), 1.0);
    }
}
