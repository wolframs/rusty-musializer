//! The spectrum analyzer.
//!
//! Port of `../musializer/src/audio_analyzer.c` and `.h`. Every numeric choice
//! here is the oracle's, including the ones that look odd:
//!
//! - the "amplitude" is `ln(re² + im²)` (`audio_analyzer.c:62-65`), not a
//!   magnitude in dB. It is a legacy formula and reproducing it is the point.
//! - `maximum` starts at `1.0`, not at zero, so a quiet frame normalizes against
//!   1 rather than blowing up (`audio_analyzer.c:182`).
//! - `dt` is clamped to 0.1 s because the smoothing recurrence is only stable
//!   while `8*dt <= 1`; a preview stall is a discontinuity, not permission to
//!   overshoot normalized bands (`audio_analyzer.c:173-175`).
//!
//! Bands are logarithmically spaced by a factor of 1.06 per band over the lower
//! half of the FFT, and each band takes the *maximum* amplitude of its bins.

use std::f32::consts::PI;

/// FFT window length. `1 << 13` (`audio_analyzer.h:8`).
pub const FFT_SIZE: usize = 1 << 13;
/// Hard cap on logarithmic bands (`audio_analyzer.h:9`).
pub const MAX_BANDS: usize = 256;

/// Per-band spacing factor (`audio_analyzer.c:180`).
const BAND_STEP: f32 = 1.06;
/// Smoothing rate for the displayed bands (`audio_analyzer.c:205`).
const SMOOTH_RATE: f32 = 8.0;
/// Smoothing rate for the trailing smear (`audio_analyzer.c:206`).
const SMEAR_RATE: f32 = 3.0;
/// Largest `dt` the recurrence stays stable for (`audio_analyzer.c:175`).
const MAX_DT_SECONDS: f32 = 0.1;

/// How a multi-channel frame is reduced to one analysis sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelMode {
    /// Average every channel (`audio_analyzer.c:143-144`).
    Mix,
    /// Take one channel; the index must be less than `channel_count`.
    Select(u32),
}

/// Analyzer configuration (`audio_analyzer.h:16-21`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioAnalyzerConfig {
    pub sample_rate: u32,
    pub channel_count: u32,
    pub channel_mode: ChannelMode,
}

/// The analyzer configuration the oracle uses when no track is loaded
/// (`../musializer/src/plug.c:8368,8399`).
pub const IDLE_SAMPLE_RATE: u32 = 48_000;

impl AudioAnalyzerConfig {
    /// Exactly what the preview path configures
    /// (`analyzer_configure`, `../musializer/src/plug.c:438-449`).
    ///
    /// Two details are easy to get wrong and both are the oracle's:
    ///
    /// - **Channel mode is `Select(0)`, not `Mix`.** The analyzer reads the left
    ///   channel only. A mix looks more principled and is a visible parity
    ///   break, because any content panned off-centre reads at a different
    ///   level.
    /// - **`sample_rate` is the source *file's* rate**, passed as
    ///   `track->music.stream.sampleRate` (`plug.c:660`) — even though raylib
    ///   invokes stream processors *after* resampling to the audio device's rate
    ///   (`plug.c:531-536`). When those differ (a 44.1 kHz file on a 48 kHz
    ///   device, which is the common case) every frequency label the analyzer
    ///   derives is skewed by the ratio. That is the oracle's behaviour, so it
    ///   is reproduced here; see the NOTE ENTRIES in REWRITE_PLAN.md.
    ///
    /// `channel_count` is always 2 because raylib has already mixed to its
    /// stereo format by this point, whatever the file contained.
    #[must_use]
    pub fn preview(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channel_count: 2,
            channel_mode: ChannelMode::Select(0),
        }
    }

    /// The no-track configuration (`plug.c:8368,8399`).
    #[must_use]
    pub fn idle() -> Self {
        Self::preview(IDLE_SAMPLE_RATE)
    }

    /// A stereo configuration that averages both channels.
    ///
    /// Not what the preview path does — see [`AudioAnalyzerConfig::preview`].
    /// Kept because averaging is the right default for anything that is not
    /// reproducing the oracle.
    #[must_use]
    pub fn stereo(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channel_count: 2,
            channel_mode: ChannelMode::Mix,
        }
    }

    fn is_valid(self) -> bool {
        if self.sample_rate == 0 || self.channel_count == 0 {
            return false;
        }
        match self.channel_mode {
            ChannelMode::Mix => true,
            ChannelMode::Select(channel) => channel < self.channel_count,
        }
    }
}

/// A rejected configuration (`audio_analyzer.c:81-98`).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnalyzerError {
    #[error("sample rate must be non-zero")]
    ZeroSampleRate,
    #[error("channel count must be non-zero")]
    ZeroChannelCount,
    #[error("selected channel {selected} is out of range for {channel_count} channels")]
    ChannelOutOfRange { selected: u32, channel_count: u32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Complex {
    real: f32,
    imaginary: f32,
}

impl Complex {
    fn add(self, other: Self) -> Self {
        Self {
            real: self.real + other.real,
            imaginary: self.imaginary + other.imaginary,
        }
    }

    fn sub(self, other: Self) -> Self {
        Self {
            real: self.real - other.real,
            imaginary: self.imaginary - other.imaginary,
        }
    }

    fn mul(self, other: Self) -> Self {
        Self {
            real: self.real * other.real - self.imaginary * other.imaginary,
            imaginary: self.real * other.imaginary + self.imaginary * other.real,
        }
    }
}

/// The analyzer and all of its working memory.
///
/// ~200 KiB of fixed arrays. C keeps them inline so the struct can live in
/// hot-reloaded state without an allocator; hot reload is a non-goal here, but
/// the layout is kept because it makes the port checkable line by line. Prefer
/// [`AudioAnalyzer::boxed`] over moving one of these around by value.
#[derive(Clone)]
pub struct AudioAnalyzer {
    config: AudioAnalyzerConfig,
    input: [f32; FFT_SIZE],
    windowed: [f32; FFT_SIZE],
    fft: [Complex; FFT_SIZE],
    logarithmic: [f32; MAX_BANDS],
    smooth: [f32; MAX_BANDS],
    smear: [f32; MAX_BANDS],
    band_first_bin: [u16; MAX_BANDS],
    band_end_bin: [u16; MAX_BANDS],
    write_cursor: usize,
    sample_count: usize,
    band_count: usize,
}

/// Summarizes rather than dumping: a derived `Debug` would print three 8192-element
/// arrays and be useless in a test failure.
impl std::fmt::Debug for AudioAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioAnalyzer")
            .field("config", &self.config)
            .field("sample_count", &self.sample_count)
            .field("write_cursor", &self.write_cursor)
            .field("band_count", &self.band_count)
            .finish_non_exhaustive()
    }
}

/// A borrowed view of the current spectrum (`audio_analyzer.h:48-56`).
#[derive(Clone, Copy, Debug)]
pub struct SpectrumView<'a> {
    /// Normalized per-band amplitude for this frame.
    pub logarithmic: &'a [f32],
    /// The smoothed bands the scenes draw as bar heights.
    pub smooth: &'a [f32],
    /// The slower-decaying trail behind `smooth`.
    pub smear: &'a [f32],
    pub band_first_bin: &'a [u16],
    pub band_end_bin: &'a [u16],
    pub sample_rate: u32,
}

impl<'a> SpectrumView<'a> {
    #[must_use]
    pub fn band_count(&self) -> usize {
        self.smooth.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.smooth.is_empty()
    }
}

impl AudioAnalyzer {
    /// # Errors
    /// Rejects exactly what C's `audio_analyzer_init` rejects.
    pub fn new(config: AudioAnalyzerConfig) -> Result<Self, AnalyzerError> {
        if config.sample_rate == 0 {
            return Err(AnalyzerError::ZeroSampleRate);
        }
        if config.channel_count == 0 {
            return Err(AnalyzerError::ZeroChannelCount);
        }
        if let ChannelMode::Select(selected) = config.channel_mode {
            if selected >= config.channel_count {
                return Err(AnalyzerError::ChannelOutOfRange {
                    selected,
                    channel_count: config.channel_count,
                });
            }
        }
        Ok(Self {
            config,
            input: [0.0; FFT_SIZE],
            windowed: [0.0; FFT_SIZE],
            fft: [Complex::default(); FFT_SIZE],
            logarithmic: [0.0; MAX_BANDS],
            smooth: [0.0; MAX_BANDS],
            smear: [0.0; MAX_BANDS],
            band_first_bin: [0; MAX_BANDS],
            band_end_bin: [0; MAX_BANDS],
            write_cursor: 0,
            sample_count: 0,
            band_count: 0,
        })
    }

    /// Heap-allocated constructor, to keep 200 KiB off the stack.
    pub fn boxed(config: AudioAnalyzerConfig) -> Result<Box<Self>, AnalyzerError> {
        Ok(Box::new(Self::new(config)?))
    }

    /// Clears the sample history and smoothing state, keeping the configuration
    /// (`audio_analyzer.c:100-106`).
    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.windowed.fill(0.0);
        self.fft.fill(Complex::default());
        self.logarithmic.fill(0.0);
        self.smooth.fill(0.0);
        self.smear.fill(0.0);
        self.band_first_bin.fill(0);
        self.band_end_bin.fill(0);
        self.write_cursor = 0;
        self.sample_count = 0;
        self.band_count = 0;
    }

    #[must_use]
    pub fn config(&self) -> AudioAnalyzerConfig {
        self.config
    }

    #[must_use]
    pub fn band_count(&self) -> usize {
        self.band_count
    }

    fn push_one(&mut self, sample: f32) {
        self.input[self.write_cursor] = sample;
        self.write_cursor = (self.write_cursor + 1) % FFT_SIZE;
        if self.sample_count < FFT_SIZE {
            self.sample_count += 1;
        }
    }

    /// Pushes already-mixed mono samples, returning how many were consumed
    /// (`audio_analyzer.c:117-124`).
    pub fn push_mono(&mut self, samples: &[f32]) -> usize {
        if !self.config.is_valid() {
            return 0;
        }
        for &sample in samples {
            self.push_one(sample);
        }
        samples.len()
    }

    /// Pushes interleaved frames, returning how many *frames* were consumed
    /// (`audio_analyzer.c:126-149`).
    ///
    /// A trailing partial frame is not consumed: C reads `channel_count`
    /// samples per frame unconditionally, so the caller is responsible for
    /// passing whole frames. This checks instead of reading out of bounds.
    pub fn push_interleaved(&mut self, samples: &[f32]) -> usize {
        if !self.config.is_valid() {
            return 0;
        }
        let channels = self.config.channel_count as usize;
        let frame_count = samples.len() / channels;
        for frame in 0..frame_count {
            let source = &samples[frame * channels..frame * channels + channels];
            let mono = match self.config.channel_mode {
                ChannelMode::Select(channel) => source[channel as usize],
                ChannelMode::Mix => {
                    let sum: f32 = source.iter().sum();
                    sum / channels as f32
                }
            };
            self.push_one(mono);
        }
        frame_count
    }

    /// Hann-windows the newest `sample_count` samples, right-aligned in the FFT
    /// buffer with leading zeros (`audio_analyzer.c:151-165`).
    fn prepare_window(&mut self) {
        let zero_count = FFT_SIZE - self.sample_count;
        self.windowed[..zero_count].fill(0.0);

        let oldest = if self.sample_count == FFT_SIZE {
            self.write_cursor
        } else {
            0
        };
        for i in 0..self.sample_count {
            let source = (oldest + i) % FFT_SIZE;
            let destination = zero_count + i;
            let t = destination as f32 / (FFT_SIZE - 1) as f32;
            let hann = 0.5 - 0.5 * (2.0 * PI * t).cos();
            self.windowed[destination] = self.input[source] * hann;
        }
    }

    /// In-place iterative radix-2 FFT (`audio_analyzer.c:29-60`).
    ///
    /// Note the twiddle sign: C uses `+2π/length` with `sin` unnegated, which is
    /// the inverse-transform convention. It does not matter downstream because
    /// only `re² + im²` is read, and reproducing it keeps the port literal.
    fn run_fft(&mut self) {
        let count = FFT_SIZE;
        for i in 0..count {
            self.fft[i] = Complex {
                real: self.windowed[i],
                imaginary: 0.0,
            };
        }

        let mut j = 0usize;
        for i in 1..count {
            let mut bit = count >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                self.fft.swap(i, j);
            }
        }

        let mut length = 2usize;
        while length <= count {
            let angle = 2.0 * PI / length as f32;
            let step = Complex {
                real: angle.cos(),
                imaginary: angle.sin(),
            };
            let mut i = 0usize;
            while i < count {
                let mut weight = Complex {
                    real: 1.0,
                    imaginary: 0.0,
                };
                for j in 0..length / 2 {
                    let even = self.fft[i + j];
                    let odd = self.fft[i + j + length / 2].mul(weight);
                    self.fft[i + j] = even.add(odd);
                    self.fft[i + j + length / 2] = even.sub(odd);
                    weight = weight.mul(step);
                }
                i += length;
            }
            length <<= 1;
        }
    }

    /// The legacy amplitude: `ln(re² + im²)` (`audio_analyzer.c:62-65`).
    ///
    /// For a silent bin this is `ln(0) == -inf`. That is load-bearing: the band
    /// loop keeps a running maximum seeded at `0.0`, so `-inf` never wins and a
    /// silent band reports `0.0` rather than a negative amplitude.
    fn legacy_amplitude(value: Complex) -> f32 {
        (value.real * value.real + value.imaginary * value.imaginary).ln()
    }

    /// Runs the transform and updates the band/smoothing state.
    ///
    /// `dt_seconds` is the deterministic scene/sample-clock delta, not wall
    /// clock — that is what makes preview and export produce the same bands.
    /// Returns `false` for a non-finite or negative `dt`, or if the band loop
    /// would exceed [`MAX_BANDS`] (`audio_analyzer.c:167-210`).
    pub fn analyze(&mut self, dt_seconds: f32) -> bool {
        if !self.config.is_valid() || !dt_seconds.is_finite() || dt_seconds < 0.0 {
            return false;
        }
        let dt_seconds = dt_seconds.min(MAX_DT_SECONDS);

        self.prepare_window();
        self.run_fft();

        let half = FFT_SIZE / 2;
        let mut band_count = 0usize;
        let mut maximum = 1.0f32;

        let mut frequency_bin = 1.0f32;
        while (frequency_bin as usize) < half {
            if band_count >= MAX_BANDS {
                return false;
            }
            let next = (frequency_bin * BAND_STEP).ceil();
            let first = frequency_bin as usize;
            let end = (next as usize).min(half);

            let mut amplitude = 0.0f32;
            for bin in first..end {
                let candidate = Self::legacy_amplitude(self.fft[bin]);
                if candidate > amplitude {
                    amplitude = candidate;
                }
            }
            if amplitude > maximum {
                maximum = amplitude;
            }
            self.logarithmic[band_count] = amplitude;
            self.band_first_bin[band_count] = first as u16;
            self.band_end_bin[band_count] = end as u16;
            band_count += 1;

            frequency_bin = next;
        }

        for i in 0..band_count {
            self.logarithmic[i] /= maximum;
            self.smooth[i] += (self.logarithmic[i] - self.smooth[i]) * SMOOTH_RATE * dt_seconds;
            self.smear[i] += (self.smooth[i] - self.smear[i]) * SMEAR_RATE * dt_seconds;
        }
        self.band_count = band_count;
        true
    }

    /// A borrowed view of the current spectrum (`audio_analyzer.c:212-224`).
    #[must_use]
    pub fn spectrum(&self) -> SpectrumView<'_> {
        let n = self.band_count;
        SpectrumView {
            logarithmic: &self.logarithmic[..n],
            smooth: &self.smooth[..n],
            smear: &self.smear[..n],
            band_first_bin: &self.band_first_bin[..n],
            band_end_bin: &self.band_end_bin[..n],
            sample_rate: self.config.sample_rate,
        }
    }

    /// True when every smoothed band has decayed below `epsilon`
    /// (`audio_analyzer.c:226-235`).
    #[must_use]
    pub fn settled(&self, epsilon: f32) -> bool {
        if !epsilon.is_finite() || epsilon < 0.0 {
            return false;
        }
        (0..self.band_count)
            .all(|i| self.smooth[i].abs() <= epsilon && self.smear[i].abs() <= epsilon)
    }

    /// Centre frequency of an FFT bin, in Hz (`audio_analyzer.c:237-244`).
    #[must_use]
    pub fn bin_frequency(&self, bin: usize) -> f32 {
        if bin > FFT_SIZE / 2 {
            return 0.0;
        }
        bin as f32 * self.config.sample_rate as f32 / FFT_SIZE as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_analyzer() -> Box<AudioAnalyzer> {
        AudioAnalyzer::boxed(AudioAnalyzerConfig::stereo(44_100)).unwrap()
    }

    #[test]
    fn rejects_configurations_c_rejects() {
        let bad_rate = AudioAnalyzerConfig {
            sample_rate: 0,
            ..AudioAnalyzerConfig::stereo(1)
        };
        assert_eq!(
            AudioAnalyzer::new(bad_rate).unwrap_err(),
            AnalyzerError::ZeroSampleRate
        );

        let bad_channels = AudioAnalyzerConfig {
            channel_count: 0,
            ..AudioAnalyzerConfig::stereo(44_100)
        };
        assert_eq!(
            AudioAnalyzer::new(bad_channels).unwrap_err(),
            AnalyzerError::ZeroChannelCount
        );

        let bad_select = AudioAnalyzerConfig {
            channel_mode: ChannelMode::Select(2),
            ..AudioAnalyzerConfig::stereo(44_100)
        };
        assert_eq!(
            AudioAnalyzer::new(bad_select).unwrap_err(),
            AnalyzerError::ChannelOutOfRange {
                selected: 2,
                channel_count: 2
            }
        );
    }

    #[test]
    fn rejects_non_finite_and_negative_dt() {
        let mut analyzer = stereo_analyzer();
        assert!(!analyzer.analyze(f32::NAN));
        assert!(!analyzer.analyze(f32::INFINITY));
        assert!(!analyzer.analyze(-0.001));
        assert!(analyzer.analyze(0.0));
    }

    #[test]
    fn a_silent_analyzer_produces_bands_but_no_energy() {
        let mut analyzer = stereo_analyzer();
        assert!(analyzer.analyze(1.0 / 60.0));
        // The band layout does not depend on the audio, only on FFT_SIZE.
        assert!(analyzer.band_count() > 0);
        assert!(analyzer.band_count() <= MAX_BANDS);
        assert!(analyzer.settled(1.0e-6));
        assert!(analyzer
            .spectrum()
            .smooth
            .iter()
            .all(|band| band.abs() < 1.0e-6));
    }

    /// The band layout is a pure function of FFT_SIZE and the 1.06 step, so it
    /// is a stable contract the scene agents can rely on.
    #[test]
    fn band_layout_is_logarithmic_and_covers_the_lower_half() {
        let mut analyzer = stereo_analyzer();
        assert!(analyzer.analyze(0.016));
        let spectrum = analyzer.spectrum();
        assert_eq!(spectrum.band_first_bin[0], 1, "bands start at bin 1, not 0");
        assert_eq!(
            *spectrum.band_end_bin.last().unwrap() as usize,
            FFT_SIZE / 2,
            "the last band is clamped to the Nyquist bin"
        );
        // Half-open and contiguous: each band's end is the next band's start.
        for i in 1..spectrum.band_count() {
            assert_eq!(spectrum.band_first_bin[i], spectrum.band_end_bin[i - 1]);
        }
    }

    #[test]
    fn a_tone_lands_in_the_band_containing_its_frequency() {
        let mut analyzer = stereo_analyzer();
        let sample_rate = 44_100.0f32;
        let tone_hz = 1_000.0f32;
        // Fill the whole window so there is no zero padding.
        let mut interleaved = Vec::with_capacity(FFT_SIZE * 2);
        for n in 0..FFT_SIZE {
            let sample = (2.0 * PI * tone_hz * n as f32 / sample_rate).sin();
            interleaved.push(sample);
            interleaved.push(sample);
        }
        assert_eq!(analyzer.push_interleaved(&interleaved), FFT_SIZE);
        // Drive the smoothing to convergence at the clamped stable dt.
        for _ in 0..400 {
            assert!(analyzer.analyze(MAX_DT_SECONDS));
        }

        let spectrum = analyzer.spectrum();
        let loudest = (0..spectrum.band_count())
            .max_by(|&a, &b| spectrum.smooth[a].total_cmp(&spectrum.smooth[b]))
            .unwrap();
        let first_hz = analyzer.bin_frequency(spectrum.band_first_bin[loudest] as usize);
        let end_hz = analyzer.bin_frequency(spectrum.band_end_bin[loudest] as usize);
        assert!(
            first_hz <= tone_hz && tone_hz <= end_hz,
            "loudest band {loudest} spans {first_hz}..{end_hz} Hz, expected it to contain {tone_hz} Hz"
        );
        assert!(
            !analyzer.settled(1.0e-3),
            "a 1 kHz tone should not read as silence"
        );
    }

    #[test]
    fn dt_is_clamped_so_bands_cannot_overshoot() {
        let mut analyzer = stereo_analyzer();
        let mut interleaved = Vec::with_capacity(FFT_SIZE * 2);
        for n in 0..FFT_SIZE {
            let sample = (2.0 * PI * 440.0 * n as f32 / 44_100.0).sin();
            interleaved.push(sample);
            interleaved.push(sample);
        }
        analyzer.push_interleaved(&interleaved);
        // A 2-second stall would make the raw recurrence gain 16x and diverge.
        for _ in 0..200 {
            assert!(analyzer.analyze(2.0));
        }
        let spectrum = analyzer.spectrum();
        assert!(
            spectrum
                .smooth
                .iter()
                .all(|band| band.is_finite() && (-0.05..=1.05).contains(band)),
            "a long stall must not push normalized bands out of range"
        );
    }

    /// Two properties at once, and the second one surprised the port:
    ///
    /// 1. `Mix` and `Select` really do reduce channels differently, so the band
    ///    *profile* differs.
    /// 2. The **peak band is always 1.0** regardless of how loud the input is,
    ///    because `analyze` divides every band by the frame's own maximum. A
    ///    scene therefore cannot read absolute loudness out of `bands` — that is
    ///    what `rms` and `peak` on the frame are for. Faithful to C
    ///    (`audio_analyzer.c:204`), and worth pinning so nobody "fixes" it.
    #[test]
    fn mix_and_select_reduce_channels_differently_but_both_normalize_to_one() {
        let config = AudioAnalyzerConfig {
            channel_mode: ChannelMode::Select(1),
            ..AudioAnalyzerConfig::stereo(44_100)
        };
        let mut select = AudioAnalyzer::boxed(config).unwrap();
        let mut mix = stereo_analyzer();
        // Left is silent, right carries the tone. Mix halves it; Select(1) does not.
        let mut interleaved = Vec::with_capacity(FFT_SIZE * 2);
        for n in 0..FFT_SIZE {
            interleaved.push(0.0);
            interleaved.push((2.0 * PI * 440.0 * n as f32 / 44_100.0).sin());
        }
        select.push_interleaved(&interleaved);
        mix.push_interleaved(&interleaved);
        for _ in 0..400 {
            select.analyze(MAX_DT_SECONDS);
            mix.analyze(MAX_DT_SECONDS);
        }

        let select_bands = select.spectrum().smooth.to_vec();
        let mix_bands = mix.spectrum().smooth.to_vec();
        assert_eq!(select_bands.len(), mix_bands.len());
        assert_ne!(
            select_bands, mix_bands,
            "halving one channel must change the profile"
        );

        for (label, bands) in [("select", &select_bands), ("mix", &mix_bands)] {
            let peak = bands.iter().copied().fold(0.0f32, f32::max);
            assert!(
                (peak - 1.0).abs() < 1.0e-3,
                "{label} peak band is {peak}, expected normalization to pin it at 1.0"
            );
        }
    }

    /// Pins the two details of `analyzer_configure` that are easy to "improve"
    /// into a parity break.
    #[test]
    fn the_preview_configuration_matches_the_oracle() {
        let config = AudioAnalyzerConfig::preview(44_100);
        assert_eq!(
            config.channel_mode,
            ChannelMode::Select(0),
            "the oracle reads the left channel only, it does not mix"
        );
        assert_eq!(
            config.channel_count, 2,
            "raylib has already mixed to stereo"
        );
        assert_eq!(
            config.sample_rate, 44_100,
            "the file's rate, not the device's"
        );
        assert_eq!(AudioAnalyzerConfig::idle().sample_rate, IDLE_SAMPLE_RATE);
        assert_eq!(
            AudioAnalyzerConfig::idle().channel_mode,
            ChannelMode::Select(0)
        );
    }

    #[test]
    fn reset_clears_history_but_keeps_configuration() {
        let mut analyzer = stereo_analyzer();
        analyzer.push_mono(&[1.0; 512]);
        analyzer.analyze(0.016);
        let config = analyzer.config();
        analyzer.reset();
        assert_eq!(analyzer.config(), config);
        assert_eq!(analyzer.band_count(), 0);
        assert!(analyzer.spectrum().is_empty());
    }

    #[test]
    fn a_partial_trailing_frame_is_not_consumed() {
        let mut analyzer = stereo_analyzer();
        // Five samples is two whole stereo frames plus a stray.
        assert_eq!(analyzer.push_interleaved(&[0.1, 0.2, 0.3, 0.4, 0.5]), 2);
    }
}

/// The cases from `../musializer/tests/test_audio_analyzer.c` that the port above
/// did not already cover.
///
/// **Appended by Agent A as verification, not as a rewrite.** The implementation
/// was already landed by the integration owner for the Phase 1 gate; this module
/// exists because a port is only verified against the suite that constrained the
/// original, and eight of the C suite's assertions had no Rust counterpart. All of
/// them pass unmodified, so no divergence was found.
///
/// Two C assertions are deliberately absent because Rust cannot express them:
/// `audio_analyzer_init` with `(AudioAnalyzerChannelMode)99`
/// (`test_audio_analyzer.c:33`) and the `NULL`-pointer push arms
/// (`test_audio_analyzer.c:36-37`), whose nearest analogue — an empty slice — is
/// covered below.
#[cfg(test)]
mod c_suite_parity {
    use super::*;

    const TEST_SAMPLE_RATE: u32 = 44_100;

    fn mono_analyzer() -> Box<AudioAnalyzer> {
        AudioAnalyzer::boxed(AudioAnalyzerConfig {
            sample_rate: TEST_SAMPLE_RATE,
            channel_count: 1,
            channel_mode: ChannelMode::Mix,
        })
        .unwrap()
    }

    /// Mirrors `make_sine` (`test_audio_analyzer.c:9-15`), including its `tau`
    /// literal and f32 arithmetic.
    fn make_sine(count: usize, frequency: f32, amplitude: f32) -> Vec<f32> {
        use std::f32::consts::TAU;
        (0..count)
            .map(|i| amplitude * (TAU * frequency * i as f32 / TEST_SAMPLE_RATE as f32).sin())
            .collect()
    }

    fn strongest_band(spectrum: &SpectrumView<'_>) -> usize {
        (1..spectrum.band_count()).fold(0, |strongest, i| {
            if spectrum.logarithmic[i] > spectrum.logarithmic[strongest] {
                i
            } else {
                strongest
            }
        })
    }

    /// `test_audio_analyzer.c:36-40`. An empty push is the reachable analogue of
    /// C's `NULL` push, and `bin_frequency` past Nyquist reads zero rather than
    /// extrapolating a frequency the transform does not contain.
    #[test]
    fn empty_pushes_consume_nothing_and_bins_past_nyquist_read_zero() {
        let mut analyzer =
            AudioAnalyzer::boxed(AudioAnalyzerConfig::stereo(TEST_SAMPLE_RATE)).unwrap();
        assert_eq!(analyzer.push_mono(&[]), 0);
        assert_eq!(analyzer.push_interleaved(&[]), 0);
        assert_eq!(analyzer.bin_frequency(FFT_SIZE), 0.0);
        assert_eq!(analyzer.bin_frequency(FFT_SIZE / 2 + 1), 0.0);
        // The Nyquist bin itself is valid and is the largest one that is.
        assert!(analyzer.bin_frequency(FFT_SIZE / 2) > 0.0);
        assert_eq!(analyzer.bin_frequency(0), 0.0, "bin 0 is DC, which is 0 Hz");
    }

    /// `test_audio_analyzer.c:43-60`. One sample in an 8192-point window is
    /// 8191 zeros of padding, and the band layout must still resolve.
    #[test]
    fn a_single_sample_still_produces_a_stable_zeroed_spectrum() {
        let mut analyzer = mono_analyzer();
        assert_eq!(analyzer.push_mono(&[0.0]), 1);
        assert!(analyzer.analyze(1.0 / 60.0));

        let spectrum = analyzer.spectrum();
        assert!(spectrum.band_count() > 0);
        assert!(spectrum.band_count() <= MAX_BANDS);
        for i in 0..spectrum.band_count() {
            assert_eq!(spectrum.logarithmic[i], 0.0, "band {i}");
            assert_eq!(spectrum.smooth[i], 0.0, "band {i}");
            assert_eq!(spectrum.smear[i], 0.0, "band {i}");
        }
        assert!(analyzer.settled(1.0e-3));
    }

    /// `test_audio_analyzer.c:62-79`. The existing suite drives the smoothing to
    /// convergence over 400 frames; this pins the *single-frame* normalization,
    /// which is the property the Song Atlas offline pass depends on because it
    /// reads `logarithmic` after exactly one `analyze`.
    #[test]
    fn a_tone_normalizes_to_one_after_a_single_analyze() {
        let mut analyzer = mono_analyzer();
        let samples = make_sine(FFT_SIZE, 440.0, 0.8);
        assert_eq!(analyzer.push_mono(&samples), FFT_SIZE);
        assert!(analyzer.analyze(1.0 / 60.0));

        let spectrum = analyzer.spectrum();
        let band = strongest_band(&spectrum);
        let lower = analyzer.bin_frequency(spectrum.band_first_bin[band] as usize);
        let upper = analyzer.bin_frequency(spectrum.band_end_bin[band] as usize);
        assert!(lower <= 440.0, "band starts at {lower} Hz");
        assert!(upper >= 440.0, "band ends at {upper} Hz");
        assert!(
            spectrum.logarithmic[band] > 0.99,
            "per-frame normalization pins the peak band at 1.0, got {}",
            spectrum.logarithmic[band]
        );
    }

    /// `test_audio_analyzer.c:81-104`, and the sharpest test in the C suite.
    ///
    /// With `left == -right`, `Mix` cancels to *exact* silence while `Select(0)`
    /// sees the full tone. The existing Rust test uses a silent left channel,
    /// which distinguishes the two modes but would also pass for several wrong
    /// reductions; antiphase cancellation only passes for a true average.
    #[test]
    fn mixing_antiphase_channels_cancels_while_selecting_one_does_not() {
        let stereo_config = |mode| AudioAnalyzerConfig {
            sample_rate: TEST_SAMPLE_RATE,
            channel_count: 2,
            channel_mode: mode,
        };
        let mut mixed = AudioAnalyzer::boxed(stereo_config(ChannelMode::Mix)).unwrap();
        let mut selected = AudioAnalyzer::boxed(stereo_config(ChannelMode::Select(0))).unwrap();

        use std::f32::consts::TAU;
        let mut interleaved = Vec::with_capacity(FFT_SIZE * 2);
        for i in 0..FFT_SIZE {
            let value = (TAU * 880.0 * i as f32 / TEST_SAMPLE_RATE as f32).sin();
            interleaved.push(value);
            interleaved.push(-value);
        }

        assert_eq!(mixed.push_interleaved(&interleaved), FFT_SIZE);
        assert_eq!(selected.push_interleaved(&interleaved), FFT_SIZE);
        assert!(mixed.analyze(1.0 / 60.0));
        assert!(selected.analyze(1.0 / 60.0));

        let mixed_spectrum = mixed.spectrum();
        let selected_spectrum = selected.spectrum();
        assert_eq!(
            mixed_spectrum.logarithmic[strongest_band(&mixed_spectrum)],
            0.0,
            "an antiphase mix must cancel to exactly nothing"
        );
        assert!(
            selected_spectrum.logarithmic[strongest_band(&selected_spectrum)] > 0.99,
            "Select(0) sees the tone at full level"
        );
    }

    /// `test_audio_analyzer.c:106-124`. Pins the smoothing coefficients
    /// themselves — `8*dt` for `smooth` and `3*dt` for `smear`
    /// (`audio_analyzer.c:205-206`) — rather than only their consequences, so a
    /// "nicer" rate cannot be substituted silently.
    #[test]
    fn the_smoothing_recurrence_uses_exactly_eight_and_three_times_dt() {
        let mut analyzer = mono_analyzer();
        let mut impulse = vec![0.0f32; FFT_SIZE];
        impulse[FFT_SIZE / 2] = 1.0;
        analyzer.push_mono(&impulse);
        assert!(analyzer.analyze(0.01));

        let (band, logarithmic, smooth, smear) = {
            let spectrum = analyzer.spectrum();
            let band = strongest_band(&spectrum);
            (
                band,
                spectrum.logarithmic[band],
                spectrum.smooth[band],
                spectrum.smear[band],
            )
        };

        // No new samples, so the transform — and therefore `logarithmic` — is
        // identical on the second pass. Only the smoothing state moves.
        assert!(analyzer.analyze(0.01));
        let spectrum = analyzer.spectrum();
        assert_eq!(
            spectrum.logarithmic[band], logarithmic,
            "the same window must produce the same spectrum"
        );
        let expected_smooth = smooth + (logarithmic - smooth) * 0.08;
        assert!(
            (spectrum.smooth[band] - expected_smooth).abs() < 1.0e-6,
            "smooth: {} vs 8*dt recurrence {expected_smooth}",
            spectrum.smooth[band]
        );
        let expected_smear = smear + (spectrum.smooth[band] - smear) * 0.03;
        assert!(
            (spectrum.smear[band] - expected_smear).abs() < 1.0e-6,
            "smear: {} vs 3*dt recurrence {expected_smear}",
            spectrum.smear[band]
        );
    }

    /// `test_audio_analyzer.c:126-141`. The existing suite checks a *repeated*
    /// stall stays bounded; this checks a single 0.5 s step, which is the one that
    /// would overshoot to 4x without the clamp.
    #[test]
    fn one_long_dt_cannot_push_the_first_frame_out_of_the_unit_range() {
        let mut analyzer = mono_analyzer();
        analyzer.push_mono(&make_sine(FFT_SIZE, 440.0, 0.9));
        assert!(analyzer.analyze(0.5));
        let spectrum = analyzer.spectrum();
        for i in 0..spectrum.band_count() {
            assert!(spectrum.smooth[i].is_finite(), "band {i}");
            assert!(spectrum.smear[i].is_finite(), "band {i}");
            assert!(
                (0.0..=1.0).contains(&spectrum.smooth[i]),
                "band {i} smooth {} escaped [0, 1]",
                spectrum.smooth[i]
            );
            assert!(
                (0.0..=1.0).contains(&spectrum.smear[i]),
                "band {i} smear {} escaped [0, 1]",
                spectrum.smear[i]
            );
        }
    }

    /// `test_audio_analyzer.c:143-164`, and the only test that exercises the ring
    /// buffer's wraparound in `prepare_window`.
    ///
    /// Pushing a full window of silence and *then* the tone must produce a
    /// bit-identical spectrum to pushing the tone alone: in the first case the
    /// window is read starting from a wrapped `write_cursor`, in the second from
    /// index 0. Equality is exact, not approximate — the same samples in the same
    /// order are the same floats.
    #[test]
    fn the_circular_window_keeps_only_the_latest_samples() {
        let silence = vec![0.0f32; FFT_SIZE];
        let sine = make_sine(FFT_SIZE, 1_000.0, 0.7);

        let mut direct = mono_analyzer();
        let mut wrapped = mono_analyzer();
        direct.push_mono(&sine);
        wrapped.push_mono(&silence);
        wrapped.push_mono(&sine);
        assert!(direct.analyze(1.0 / 30.0));
        assert!(wrapped.analyze(1.0 / 30.0));

        let a = direct.spectrum();
        let b = wrapped.spectrum();
        assert_eq!(a.band_count(), b.band_count());
        assert_eq!(a.logarithmic, b.logarithmic);
        assert_eq!(a.smooth, b.smooth);
        assert_eq!(a.smear, b.smear);
    }

    /// `test_audio_analyzer.c:166-186`. The export path relies on this: an offline
    /// render feeds the analyzer a silence tail and needs the bands to actually
    /// reach zero within a bounded number of frames, not merely approach it.
    #[test]
    fn an_offline_silence_tail_reaches_settlement_in_bounded_time() {
        let mut analyzer = mono_analyzer();
        let block = (TEST_SAMPLE_RATE / 30) as usize;
        analyzer.push_mono(&make_sine(FFT_SIZE, 220.0, 0.9));
        assert!(analyzer.analyze(1.0 / 30.0));
        assert!(!analyzer.settled(1.0e-3), "a 220 Hz tone is not silence");

        let silence = vec![0.0f32; block];
        let mut tail_frames = 0usize;
        while !analyzer.settled(1.0e-3) && tail_frames < 300 {
            analyzer.push_mono(&silence);
            assert!(analyzer.analyze(1.0 / 30.0));
            tail_frames += 1;
        }
        assert!(analyzer.settled(1.0e-3), "never settled");
        assert!(tail_frames > 0);
        assert!(tail_frames < 300, "took {tail_frames} frames to settle");
    }

    /// Not in the C suite. `settled` rejects a non-finite or negative epsilon
    /// (`audio_analyzer.c:228`) rather than treating it as "any tolerance", which
    /// matters because a caller computing epsilon from a setting could hand it a
    /// `NaN`.
    #[test]
    fn settled_rejects_a_nonsensical_tolerance() {
        let mut analyzer = mono_analyzer();
        assert!(analyzer.analyze(0.016));
        assert!(
            analyzer.settled(0.0),
            "silence is settled at zero tolerance"
        );
        assert!(!analyzer.settled(f32::NAN));
        assert!(!analyzer.settled(-0.1));
        assert!(!analyzer.settled(f32::NEG_INFINITY));
        assert!(
            !analyzer.settled(f32::INFINITY),
            "an infinite tolerance is rejected, not treated as 'always settled'"
        );
    }
}
