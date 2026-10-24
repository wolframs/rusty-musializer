//! Replays a decoded track through the export's per-frame cadence and prints
//! what the two Atmosphere scenes (Aurora Veil, Neon Tunnel) derived from it —
//! one CSV row per frame.
//!
//! The instrument for tuning their musical response against real material
//! rather than against a synthetic fixture: a follower that sits pinned at the
//! top of its range, or a kick detector that never fires, photographs as a
//! perfectly plausible frame.
//!
//! ```sh
//! ffmpeg -i track.mp3 -f f32le -acodec pcm_f32le track.raw
//! cargo run -p musializer-core --example atmosphere_probe -- track.raw 44100 30 60
//! ```

use std::env;
use std::fs;

use musializer_core::audio::analyzer::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::audio::beat_tracker::BeatTracker;
use musializer_core::audio::track_dynamics::TrackDynamics;
use musializer_core::scene::{
    events::EventTimelineView, SceneAudioFrame, SceneFrame, SceneSettings, SceneState,
    SemanticFrame,
};
use musializer_core::scenes::aurora_veil::{AuroraVeilState, Features};
use musializer_core::scenes::neon_tunnel::NeonTunnelState;
use musializer_core::timing::render_export::{
    frame_delta_seconds, frame_time_seconds, sample_cursor,
};

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 5 {
        eprintln!("usage: atmosphere_probe RAW_F32LE_PATH SAMPLE_RATE FPS SECONDS [CHANNELS=2]");
        std::process::exit(2);
    }
    let raw = fs::read(&arguments[1]).expect("readable raw PCM");
    let sample_rate: u32 = arguments[2].parse().expect("sample rate");
    let fps: u32 = arguments[3].parse().expect("fps");
    let seconds: f64 = arguments[4].parse().expect("seconds");
    let channels: usize = arguments
        .get(5)
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
    let mut tracker = BeatTracker::new();
    let settings = SceneSettings::new();
    let mut aurora = AuroraVeilState::new(17);
    let mut tunnel = NeonTunnelState::new(17);
    let dynamics = TrackDynamics::profile(&samples, channels, sample_rate);
    match dynamics {
        Some(profile) => eprintln!(
            "dynamics: floor={:.4} ceiling={:.4}",
            profile.floor(),
            profile.ceiling()
        ),
        None => eprintln!("dynamics: none (fallback gates)"),
    }

    let total_frames = (seconds * f64::from(fps)).ceil() as u64;
    let mut cursor = 0u64;
    println!("frame,t,rms,flux,bands,aurora,tunnel,level,bass,kick");
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
        aurora.update(&frame);
        tunnel.update(&frame);
        let features = Features::of(&frame.audio, dynamics);
        println!(
            "{},{:.3},{:.4},{:.4},{},{},{},{:.3},{:.3},{:.4}",
            index,
            time_seconds,
            frame.audio.rms,
            frame.audio.spectral_flux,
            frame.audio.bands.len(),
            aurora.probe_line(),
            tunnel.probe_line(),
            features.level,
            features.bass,
            features.kick,
        );
    }
}
