//! Track waveform envelopes and exact transport arithmetic.
//!
//! **Owner: Agent A.** Port of `../musializer/src/track_timeline.c` and `.h`.
//!
//! Three unrelated jobs live here because they are the three things the timeline
//! strip needs before it can draw or seek:
//!
//! - [`build_waveform`] reduces whole-track PCM to a bounded, peak-normalized
//!   min/max envelope;
//! - [`seek_relative`] is the one clamping rule shared by transport buttons,
//!   keyboard navigation, and pointer seeking, so they cannot disagree;
//! - [`path_is_seekable`] keeps transport affordances truthful for the stream
//!   formats raylib cannot seek.
//!
//! Nothing here reads a clock or allocates during playback, which is what lets
//! preview and export agree.

/// Hard cap on envelope bins (`track_timeline.h:7`).
///
/// The C `Track_Timeline_Waveform` is a fixed 2048-element array so it can live
/// in hot-reloaded state without an allocator. Hot reload is a stated non-goal
/// here, so [`Waveform`] owns a `Vec` instead and this constant survives as the
/// bound rather than as an array length.
pub const MAX_BINS: usize = 2048;

/// One column of the envelope: the extremes of every sample in its span
/// (`track_timeline.h:9-12`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bin {
    pub minimum: f32,
    pub maximum: f32,
}

/// A whole-track display envelope (`track_timeline.h:14-17`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waveform {
    bins: Vec<Bin>,
}

impl Waveform {
    /// Builds an envelope with at most [`MAX_BINS`] columns.
    ///
    /// See [`build_waveform`] for the arithmetic; this is the convenience
    /// wrapper that owns its storage. `bin_capacity` is clamped to [`MAX_BINS`]
    /// so a caller cannot ask for more columns than the C type can hold.
    #[must_use]
    pub fn build(samples: &[f32], channel_count: usize, bin_capacity: usize) -> Self {
        let capacity = bin_capacity.min(MAX_BINS);
        let mut bins = vec![Bin::default(); capacity];
        let count = build_waveform(samples, channel_count, &mut bins);
        bins.truncate(count);
        Self { bins }
    }

    #[must_use]
    pub fn bins(&self) -> &[Bin] {
        &self.bins
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bins.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bins.is_empty()
    }
}

/// Builds a peak-normalized display envelope from interleaved PCM, returning how
/// many bins were written (`track_timeline.c:39-90`).
///
/// The Rust signature derives the frame count from `samples.len() /
/// channel_count` instead of taking it separately. That removes C's
/// `frame_count > SIZE_MAX/channel_count` overflow guard
/// (`track_timeline.c:47`) as an unrepresentable state rather than as a dropped
/// check; a caller that wants only a prefix of a decoded buffer slices it.
///
/// Behaviour worth knowing, all of it the oracle's:
///
/// - **Bin boundaries are exact, not approximate.** The span of bin *i* is
///   `i*step + (i*remainder)/count` (`track_timeline.c:54-58`), which
///   distributes the `frame_count % count` leftover frames across the whole
///   strip rather than piling them into the last column. Every frame lands in
///   exactly one bin.
/// - **`minimum` and `maximum` are seeded at `0.0`, not at the first sample**
///   (`track_timeline.c:61-62`), so an all-positive span still reports
///   `minimum == 0.0` and the envelope is always drawn around the centre line.
/// - **Non-finite samples are skipped, not zeroed** (`track_timeline.c:67`), and
///   finite samples are clamped to `[-1, 1]` before they can influence the peak.
/// - **Silence stays a zero-height line.** Normalization is skipped entirely
///   when the global peak is below `1e-6` (`track_timeline.c:82`), so a silent
///   track does not get amplified into noise by a division by almost nothing.
///
/// Returns `0` — writing nothing — for empty `samples`, a zero `channel_count`,
/// or an empty `bins`.
pub fn build_waveform(samples: &[f32], channel_count: usize, bins: &mut [Bin]) -> usize {
    if samples.is_empty() || channel_count == 0 || bins.is_empty() {
        return 0;
    }
    let frame_count = samples.len() / channel_count;
    if frame_count == 0 {
        return 0;
    }

    let count = frame_count.min(bins.len());
    let frame_step = frame_count / count;
    let frame_remainder = frame_count % count;
    let mut global_peak = 0.0f32;

    for (bin_index, bin) in bins.iter_mut().enumerate().take(count) {
        let first_frame = bin_index * frame_step + (bin_index * frame_remainder) / count;
        let next_bin = bin_index + 1;
        let mut end_frame = next_bin * frame_step + (next_bin * frame_remainder) / count;
        if end_frame <= first_frame {
            end_frame = first_frame + 1;
        }

        let mut minimum = 0.0f32;
        let mut maximum = 0.0f32;
        for frame in first_frame..end_frame {
            let base = frame * channel_count;
            for channel in 0..channel_count {
                let mut value = samples[base + channel];
                if !value.is_finite() {
                    continue;
                }
                value = value.clamp(-1.0, 1.0);
                if value < minimum {
                    minimum = value;
                }
                if value > maximum {
                    maximum = value;
                }
            }
        }
        *bin = Bin { minimum, maximum };
        let magnitude = minimum.abs().max(maximum.abs());
        if magnitude > global_peak {
            global_peak = magnitude;
        }
    }

    if global_peak > 0.000_001 {
        let scale = 1.0 / global_peak;
        for bin in bins[..count].iter_mut() {
            bin.minimum *= scale;
            bin.maximum *= scale;
        }
    }
    count
}

/// Clamps a playhead position into `[0, duration]` (`track_timeline.c:30-37`).
///
/// A non-finite or non-positive duration collapses to `0.0`, because there is no
/// timeline to be positioned on.
#[must_use]
fn clamp_position(seconds: f64, duration_seconds: f64) -> f64 {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0.0;
    }
    if !seconds.is_finite() {
        return 0.0;
    }
    if seconds <= 0.0 {
        return 0.0;
    }
    if seconds >= duration_seconds {
        return duration_seconds;
    }
    seconds
}

/// Applies a relative seek exactly and clamped (`track_timeline.c:92-103`).
///
/// The comparisons are deliberately written against the *delta* rather than
/// against `current + delta`: `delta >= duration - current` returns exactly
/// `duration` and `delta <= -current` returns exactly `0.0`, so hitting either
/// end of the track lands on the boundary with no floating-point residue. Only
/// an interior seek performs the addition.
///
/// Invalid geometry — any non-finite argument, or a non-positive duration —
/// leaves the position unchanged, returning the clamped current position
/// (`track_timeline.h:28`).
#[must_use]
pub fn seek_relative(current_seconds: f64, delta_seconds: f64, duration_seconds: f64) -> f64 {
    let current = clamp_position(current_seconds, duration_seconds);
    if !current_seconds.is_finite()
        || !delta_seconds.is_finite()
        || !duration_seconds.is_finite()
        || duration_seconds <= 0.0
    {
        return current;
    }
    if delta_seconds >= duration_seconds - current {
        return duration_seconds;
    }
    if delta_seconds <= -current {
        return 0.0;
    }
    current + delta_seconds
}

/// ASCII-case-insensitive suffix test (`track_timeline.c:7-21`).
fn suffix_equal(value: &str, suffix: &str) -> bool {
    let value = value.as_bytes();
    let suffix = suffix.as_bytes();
    value.len() >= suffix.len() && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Whether the transport may offer seeking for this path
/// (`track_timeline.c:23-28`).
///
/// raylib cannot seek tracker-module streams, so `.xm` and `.mod` report `false`
/// and the UI can grey out the affordance instead of offering a control that
/// silently does nothing. Every other extension — including formats raylib may
/// not be able to open at all — reports `true`: this is a decoder-capability
/// question, not a format whitelist.
///
/// The comparison is case-insensitive over ASCII only, which is C's `tolower`
/// arithmetic and is all a file extension needs.
#[must_use]
pub fn path_is_seekable(path: Option<&str>) -> bool {
    let path = match path {
        Some(path) if !path.is_empty() => path,
        _ => return false,
    };
    !suffix_equal(path, ".xm") && !suffix_equal(path, ".mod")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_rejects_degenerate_geometry_without_writing() {
        let samples = [0.1f32, 0.2, 0.3, 0.4];
        let mut bins = [Bin {
            minimum: 99.0,
            maximum: 99.0,
        }; 4];
        assert_eq!(build_waveform(&[], 1, &mut bins), 0);
        assert_eq!(build_waveform(&samples, 0, &mut bins), 0);
        assert_eq!(build_waveform(&samples, 1, &mut []), 0);
        // A partial frame is not a frame: 2 samples over 3 channels is nothing.
        assert_eq!(build_waveform(&samples[..2], 3, &mut bins), 0);
        assert!(bins.iter().all(|bin| bin.minimum == 99.0));
    }

    #[test]
    fn waveform_treats_silence_and_non_finite_samples_as_a_flat_line() {
        let samples = [0.0f32, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        let mut bins = [Bin {
            minimum: 99.0,
            maximum: 99.0,
        }; 4];
        assert_eq!(build_waveform(&samples, 1, &mut bins), 4);
        for bin in bins {
            assert_eq!(bin, Bin::default(), "non-finite input must not draw");
        }
    }

    #[test]
    fn waveform_preserves_the_envelope_and_normalizes_the_peak() {
        #[rustfmt::skip]
        let samples = [
            -0.25f32, 0.10,
             0.50,    0.20,
            -0.75,    0.40,
             0.25,    0.10,
        ];
        let mut bins = [Bin::default(); 2];
        assert_eq!(build_waveform(&samples, 2, &mut bins), 2);
        // The global peak is |-0.75|, so everything scales by 1/0.75.
        assert!((bins[0].minimum - -1.0 / 3.0).abs() < 0.0001);
        assert!((bins[0].maximum - 2.0 / 3.0).abs() < 0.0001);
        assert!((bins[1].minimum - -1.0).abs() < 0.0001);
        assert!((bins[1].maximum - 1.0 / 1.875).abs() < 0.0001);
    }

    /// Not in the C suite, but the exact-boundary formula is the whole reason
    /// `build_waveform` is not a naive `frame_count / count` stride: with 10
    /// frames in 4 bins the leftover 2 frames must be spread, not appended.
    #[test]
    fn bin_boundaries_partition_every_frame_exactly_once() {
        for frame_count in 1..64usize {
            for capacity in 1..16usize {
                let count = frame_count.min(capacity);
                let step = frame_count / count;
                let remainder = frame_count % count;
                let mut covered = 0usize;
                let mut previous_end = 0usize;
                for bin in 0..count {
                    let first = bin * step + (bin * remainder) / count;
                    let end = (bin + 1) * step + ((bin + 1) * remainder) / count;
                    assert_eq!(first, previous_end, "bins must be contiguous");
                    assert!(end > first, "every bin covers at least one frame");
                    covered += end - first;
                    previous_end = end;
                }
                assert_eq!(previous_end, frame_count, "the last bin ends at the end");
                assert_eq!(covered, frame_count);
            }
        }
    }

    #[test]
    fn waveform_is_bounded_by_max_bins_however_much_is_asked_for() {
        let samples = vec![0.5f32; 100_000];
        let waveform = Waveform::build(&samples, 1, usize::MAX);
        assert_eq!(waveform.len(), MAX_BINS);
        assert!(!waveform.is_empty());
        // 100000 frames over 2048 bins: each bin sees ~48 identical samples.
        assert!(waveform.bins().iter().all(|bin| bin.maximum == 1.0));
    }

    #[test]
    fn waveform_clamps_out_of_range_samples_before_taking_the_peak() {
        // Without the clamp the 4.0 would set the peak and scale 1.0 down to
        // 0.25; with it, both read full scale.
        let samples = [1.0f32, 4.0];
        let mut bins = [Bin::default(); 2];
        assert_eq!(build_waveform(&samples, 1, &mut bins), 2);
        assert_eq!(bins[0].maximum, 1.0);
        assert_eq!(bins[1].maximum, 1.0);
    }

    /// **Differential against the frozen C, not against this implementation.**
    ///
    /// Produced by compiling `../musializer/src/track_timeline.c` unmodified into
    /// a scratch harness *outside both repositories* and printing `%.9g` per
    /// field: 1000 frames of a 50 Hz stereo sine at 8 kHz, amplitude 0.8, into 16
    /// bins. 1000 does not divide by 16, so this exercises the remainder
    /// distribution as well as the envelope and the normalization.
    // The literals below are the C's `%.9g` output pasted verbatim, which is more
    // digits than `f32` can hold. Truncating them to fit would replace the
    // oracle's value with a value derived from this implementation's rounding,
    // which is exactly what a differential test must not do.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn matches_the_c_oracle_bin_for_bin() {
        let (frames, channels) = (1000usize, 2usize);
        let mut samples = vec![0.0f32; frames * channels];
        for frame in 0..frames {
            let phase = 2.0 * std::f64::consts::PI * 50.0 * frame as f64 / 8000.0;
            let value = 0.8 * (phase as f32).sin();
            samples[frame * channels] = value;
            samples[frame * channels + 1] = value;
        }

        #[rustfmt::skip]
        let expected: [(f32, f32); 16] = [
            ( 0.0,            1.0),
            (-1.0,            0.649_448_037),
            (-0.980_785_251,  0.852_640_212),
            (-0.346_116_781,  1.0),
            (-1.0,            0.0),
            (-0.309_016_794,  1.0),
            (-0.987_688_243,  0.831_469_774),
            (-1.0,            0.678_800_642),
            (-0.039_259_731,  1.0),
            (-1.0,            0.0),
            (-0.555_569_470,  1.0),
            (-0.908_142_865,  0.962_455_511),
            (-1.0,            0.418_658_972),
            ( 0.0,            1.0),
            (-1.0,            0.195_092_037),
            (-0.785_317_957,  0.999_228_954),
        ];

        let mut bins = [Bin::default(); 16];
        assert_eq!(build_waveform(&samples, channels, &mut bins), 16);
        for (index, (minimum, maximum)) in expected.into_iter().enumerate() {
            assert!(
                (bins[index].minimum - minimum).abs() < 1.0e-6,
                "bin {index} minimum: {} vs C's {minimum}",
                bins[index].minimum
            );
            assert!(
                (bins[index].maximum - maximum).abs() < 1.0e-6,
                "bin {index} maximum: {} vs C's {maximum}",
                bins[index].maximum
            );
        }
    }

    #[test]
    fn relative_seek_is_exact_and_clamped() {
        assert!((seek_relative(12.5, 0.1, 60.0) - 12.6).abs() < 1.0e-7);
        assert_eq!(seek_relative(12.5, -1.0, 60.0), 11.5);
        assert_eq!(seek_relative(2.0, -10.0, 60.0), 0.0);
        assert_eq!(seek_relative(58.0, 10.0, 60.0), 60.0);
        assert_eq!(seek_relative(f64::NAN, 1.0, 60.0), 0.0);
    }

    /// Landing on a boundary must be *exactly* the boundary, not a value one ULP
    /// past it that a later `>= duration` check would misread as finished.
    #[test]
    fn seeking_to_either_end_lands_on_the_boundary_exactly() {
        assert_eq!(seek_relative(0.1, -0.1, 60.0), 0.0);
        assert_eq!(seek_relative(59.9, 0.1, 60.0), 60.0);
        assert_eq!(seek_relative(0.0, 60.0, 60.0), 60.0);
    }

    #[test]
    fn invalid_transport_geometry_leaves_the_position_alone() {
        // A non-finite delta or duration keeps the clamped current position.
        assert_eq!(seek_relative(12.5, f64::NAN, 60.0), 12.5);
        assert_eq!(seek_relative(12.5, f64::INFINITY, 60.0), 12.5);
        assert_eq!(seek_relative(12.5, 1.0, f64::NAN), 0.0);
        // No timeline means no position at all.
        assert_eq!(seek_relative(12.5, 1.0, 0.0), 0.0);
        assert_eq!(seek_relative(12.5, 1.0, -5.0), 0.0);
        // A current position past the end clamps in before the delta applies.
        assert_eq!(seek_relative(120.0, -1.0, 60.0), 59.0);
    }

    #[test]
    fn seek_capability_matches_the_decoder_contract() {
        assert!(path_is_seekable(Some("song.wav")));
        assert!(path_is_seekable(Some("album/live.FLAC")));
        assert!(!path_is_seekable(Some("tracker.XM")));
        assert!(!path_is_seekable(Some("tracker.mod")));
        assert!(!path_is_seekable(None));
        assert!(!path_is_seekable(Some("")));
    }

    /// The suffix test is a suffix test, not an extension parser: a file called
    /// exactly `.mod` is unseekable, and one called `demo.model` is not, because
    /// `.mod` is not its tail. Both follow from C's byte comparison.
    #[test]
    fn seek_capability_compares_only_the_tail() {
        assert!(!path_is_seekable(Some(".mod")));
        assert!(!path_is_seekable(Some(".xm")));
        assert!(path_is_seekable(Some("demo.model")));
        assert!(path_is_seekable(Some("xm")));
        assert!(path_is_seekable(Some("mod")));
        assert!(!path_is_seekable(Some("/a/b/Song.MOD")));
    }
}
