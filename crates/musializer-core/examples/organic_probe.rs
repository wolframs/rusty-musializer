//! Replays a decoded track through the export's per-frame cadence and prints,
//! per second, what Metaball Garden and Voronoi Bloom heard: kicks, glints,
//! bloom waves, shatters, and the slow section energy both of them breathe on.
//!
//! The instrument behind their transient thresholds (`HAT_THRESHOLD`,
//! `SHATTER_KICK`, ...): a gate written against a synthetic fixture is a gate
//! that never fires on music, so the numbers were read off a real mix.
//!
//! ```sh
//! ffmpeg -i track.mp3 -f f32le -acodec pcm_f32le track.raw
//! cargo run -p musializer-core --example organic_probe -- track.raw 44100 30 [CHANNELS=2]
//! ```

use std::env;
use std::fs;

use musializer_core::audio::analyzer::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::audio::track_dynamics::TrackDynamics;
use musializer_core::scene::{
    events::EventTimelineView, SceneAudioFrame, SceneFrame, SceneSettings, SceneState,
    SemanticFrame,
};
use musializer_core::scenes::metaball_garden::MetaballGardenState;
use musializer_core::scenes::voronoi_bloom::VoronoiBloomState;
use musializer_core::timing::render_export::{
    frame_delta_seconds, frame_time_seconds, sample_cursor,
};

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 4 {
        eprintln!("usage: organic_probe RAW_F32LE_PATH SAMPLE_RATE FPS [CHANNELS=2]");
        std::process::exit(2);
    }
    let raw = fs::read(&arguments[1]).expect("readable raw PCM");
    let sample_rate: u32 = arguments[2].parse().expect("sample rate");
    let fps: u32 = arguments[3].parse().expect("fps");
    let channels: usize = arguments
        .get(4)
        .map(|text| text.parse().expect("channels"))
        .unwrap_or(2);
    let samples: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let frame_count = (samples.len() / channels) as u64;
    let duration_seconds = frame_count as f64 / f64::from(sample_rate);
    let mut analyzer =
        AudioAnalyzer::new(AudioAnalyzerConfig::preview(sample_rate)).expect("analyzer");
    let settings = SceneSettings::new();
    let mut garden = MetaballGardenState::new(42);
    let mut bloom = VoronoiBloomState::new(42);
    let dynamics = TrackDynamics::profile(&samples, channels, sample_rate);
    let total_frames = (duration_seconds * f64::from(fps)).floor() as u64;
    let mut cursor = 0u64;
    println!(
        "second,energy_mb,kicks,glints,squash_max,burst_max,energy_vb,waves,shatters,bass_vb,hat_max,kick_max"
    );
    let (mut last_kicks, mut last_glints, mut last_waves, mut last_shatters) = (0, 0, 0, 0);
    let (mut squash_max, mut burst_max, mut hat_max, mut kick_max) =
        (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for index in 0..total_frames {
        let slice = if index == 0 {
            &[][..]
        } else {
            let next = sample_cursor(index, sample_rate, fps, frame_count).expect("cursor");
            let from = cursor as usize * channels;
            let to = (next as usize * channels).min(samples.len());
            cursor = next;
            samples.get(from..to).unwrap_or(&[])
        };
        if !slice.is_empty() {
            analyzer.push_interleaved(slice);
        }
        let delta = frame_delta_seconds(index, fps);
        analyzer.analyze(delta);
        let spectrum = analyzer.spectrum();
        let audio = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        let excursion = |from: f32, to: f32| {
            let n = audio.bands.len().min(audio.trails.len());
            let (a, b) = ((n as f32 * from) as usize, (n as f32 * to) as usize);
            audio.bands[a..b]
                .iter()
                .zip(&audio.trails[a..b])
                .map(|(x, y)| (x - y).max(0.0))
                .sum::<f32>()
                / (b - a).max(1) as f32
        };
        hat_max = hat_max.max(excursion(0.66, 1.0));
        kick_max = kick_max.max(excursion(0.0, 0.25));
        let time_seconds = frame_time_seconds(index, fps);
        let frame = SceneFrame {
            time_seconds,
            duration_seconds,
            delta_seconds: delta,
            frame_index: index,
            audio,
            semantic: SemanticFrame::default(),
            lyric: None,
            events: EventTimelineView::EMPTY,
            settings: &settings,
            dynamics,
        };
        garden.update(&frame);
        bloom.update(&frame);
        squash_max = squash_max.max(garden.squash());
        burst_max = burst_max.max(garden.burst());
        if (index + 1) % u64::from(fps) == 0 {
            println!(
                "{},{:.2},{},{},{:.2},{:.2},{:.2},{},{},{:.2},{:.3},{:.3}",
                (index + 1) / u64::from(fps),
                garden.energy(),
                garden.kick_count() - last_kicks,
                garden.sparkle_count() - last_glints,
                squash_max,
                burst_max,
                bloom.energy(),
                bloom.wave_count() - last_waves,
                bloom.shatter_count() - last_shatters,
                bloom.bass(),
                hat_max,
                kick_max,
            );
            last_kicks = garden.kick_count();
            last_glints = garden.sparkle_count();
            last_waves = bloom.wave_count();
            last_shatters = bloom.shatter_count();
            squash_max = 0.0;
            burst_max = 0.0;
            hat_max = 0.0;
            kick_max = 0.0;
        }
    }
    eprintln!(
        "totals: kicks={} glints={} waves={} shatters={}",
        garden.kick_count(),
        garden.sparkle_count(),
        bloom.wave_count(),
        bloom.shatter_count()
    );
}
