//! Replays a decoded track through the export's per-frame cadence into Fluid
//! Ink's solver, timing every update and printing what the music did to it.
//!
//! Two questions a contact sheet cannot answer: how long the solver takes per
//! frame (the preview has to keep up with playback), and whether the musical
//! envelopes — kicks, bass pressure, transients, the quiet breath — fire where
//! the music does. Input is raw interleaved f32 PCM, as for `cat_probe`:
//!
//! ```sh
//! ffmpeg -i track.mp3 -f f32le -acodec pcm_f32le -ar 44100 -ac 2 /tmp/track.raw
//! cargo run --release -p musializer-core --example fluid_probe -- /tmp/track.raw 44100 60 30 [START]
//! ```
//!
//! With `START`, frames before it are replayed silently (not timed or printed),
//! exactly as a still's replay would. CSV goes to stdout, timing to stderr.

use std::env;
use std::fs;
use std::time::Instant;

use musializer_core::audio::analyzer::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::audio::beat_tracker::BeatTracker;
use musializer_core::audio::track_dynamics::TrackDynamics;
use musializer_core::scene::{
    events::EventTimelineView, SceneAudioFrame, SceneFrame, SceneSettings, SceneState,
    SemanticFrame,
};
use musializer_core::scenes::fluid_ink::FluidInkState;
use musializer_core::timing::render_export::{
    frame_delta_seconds, frame_time_seconds, sample_cursor,
};

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 5 {
        eprintln!("usage: fluid_probe RAW_F32LE_PATH SAMPLE_RATE FPS SECONDS [START]");
        std::process::exit(2);
    }
    let raw = fs::read(&arguments[1]).expect("readable raw PCM");
    let sample_rate: u32 = arguments[2].parse().expect("sample rate");
    let fps: u32 = arguments[3].parse().expect("fps");
    let seconds: f64 = arguments[4].parse().expect("seconds");
    let start: f64 = arguments.get(5).map_or(0.0, |s| s.parse().expect("start"));
    let channels = 2usize;
    let samples: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let frame_count = (samples.len() / channels) as u64;
    let duration_seconds = frame_count as f64 / f64::from(sample_rate);
    let mut analyzer =
        AudioAnalyzer::new(AudioAnalyzerConfig::preview(sample_rate)).expect("analyzer");
    let mut tracker = BeatTracker::new();
    let settings = SceneSettings::new();
    let mut state = FluidInkState::new(0);
    let dynamics = TrackDynamics::profile(&samples, channels, sample_rate);

    let first = (start * f64::from(fps)).floor() as u64;
    let total_frames = first + (seconds * f64::from(fps)).ceil() as u64;
    let mut cursor = 0u64;
    let mut times = Vec::new();
    println!("frame,t,rms,level,kick,kicks,pressure,sprays,spray,breath,ink");
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
        let mut audio = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        let time_seconds = frame_time_seconds(index, fps);
        audio.track_beat(&mut tracker, time_seconds);
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
        let began = Instant::now();
        state.update(&frame);
        if index < first {
            continue;
        }
        times.push(began.elapsed().as_secs_f64() * 1000.0);
        let m = state.music();
        println!(
            "{},{:.3},{:.4},{:.3},{:.3},{},{:.3},{},{:.3},{:.3},{:.3}",
            index,
            time_seconds,
            frame.audio.rms,
            m.level,
            m.kick,
            m.kicks,
            m.pressure,
            m.sprays,
            m.spray,
            m.breath,
            state.ink_coverage(),
        );
    }
    times.sort_by(f64::total_cmp);
    let n = times.len().max(1);
    let mean = times.iter().sum::<f64>() / n as f64;
    eprintln!(
        "update ms: mean {:.2}, p50 {:.2}, p95 {:.2}, worst {:.2} over {} frames ({} steps)",
        mean,
        times[n / 2],
        times[(n * 95 / 100).min(n - 1)],
        times[n - 1],
        times.len(),
        state.steps(),
    );
}
