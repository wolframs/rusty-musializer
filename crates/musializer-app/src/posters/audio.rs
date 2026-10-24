//! The track's audio as the poster replay hears it.
//!
//! Kept from the load-time decode that builds the timeline waveform, so a
//! track is decoded once rather than once for the strip and again the first
//! time a poster is asked for (`load_timeline_waveform` in `main.rs`).
//!
//! **Channel 0 only, as 16-bit integers.** The export's analyzer hears channel
//! 0 and nothing else (`ChannelMode::Select(0)` in `prepare_frames`), so the
//! other channels are dead weight; and the integers halve what is left. A
//! five-minute track costs ~26 MB instead of the ~105 MB the interleaved float
//! decode would (`ensure_song_atlas_map`'s argument against keeping samples).
//! The quantisation is 96 dB down, under anything the analyzer's smoothed bands
//! resolve, and a poster is "close to the export frame" by contract anyway.

use std::sync::Arc;

use musializer_runtime::decode::DecodedAudio;

/// What a replay needs: one channel, its rate, and the decoder's frame count
/// (which `render_export::sample_cursor` clamps to).
pub struct PosterPcm {
    samples: Vec<i16>,
    pub sample_rate: u32,
}

impl PosterPcm {
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.samples.len() as u64
    }

    /// Frames `from..to` as floats, into `out`. Clamped to the track.
    pub fn read(&self, from: u64, to: u64, out: &mut Vec<f32>) {
        out.clear();
        let end = (to as usize).min(self.samples.len());
        let start = (from as usize).min(end);
        out.extend(
            self.samples[start..end]
                .iter()
                .map(|&sample| f32::from(sample) / 32767.0),
        );
    }
}

/// A cheap handle to [`PosterPcm`], shared by the track and a running job.
#[derive(Clone)]
pub struct PosterAudio(pub Arc<PosterPcm>);

impl PosterAudio {
    /// Channel 0 of a decoded track; `None` for a track with no frames.
    #[must_use]
    pub fn from_decoded(decoded: &DecodedAudio) -> Option<Self> {
        if decoded.channels == 0 || decoded.samples.is_empty() {
            return None;
        }
        let samples = decoded
            .samples
            .iter()
            .step_by(decoded.channels)
            .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .collect();
        Some(Self(Arc::new(PosterPcm {
            samples,
            sample_rate: decoded.sample_rate,
        })))
    }
}

/// A summary, because the default would print every sample of the track into
/// any `{:?}` of a `Track`.
impl std::fmt::Debug for PosterAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PosterAudio({} frames at {} Hz)",
            self.0.frame_count(),
            self.0.sample_rate
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_channel_zero_and_reads_it_back_as_floats() {
        let decoded = DecodedAudio {
            samples: vec![0.5, -1.0, -0.25, 1.0, 1.5, 0.0],
            channels: 2,
            sample_rate: 48_000,
        };
        let audio = PosterAudio::from_decoded(&decoded).expect("frames");
        assert_eq!(audio.0.frame_count(), 3);
        let mut out = Vec::new();
        audio.0.read(0, 10, &mut out);
        assert_eq!(out.len(), 3);
        assert!((out[0] - 0.5).abs() < 1e-4);
        assert!((out[1] + 0.25).abs() < 1e-4);
        // Clipped, never wrapped.
        assert!((out[2] - 1.0).abs() < 1e-6);
        audio.0.read(2, 1, &mut out);
        assert!(out.is_empty());
        assert!(format!("{audio:?}").contains("3 frames"));
    }
}
