//! Replays a decoded track through the export's exact per-frame cadence and
//! prints what Clawd's beat-driven choreography saw — one CSV row per frame.
//!
//! Diagnostic instrument for "the cats' jumps don't want to land" (operator,
//! 2026-08-25): the contact sheets show *zero* hops over ten seconds of a real
//! track while the state reports 19 beat wraps, and a picture cannot say which
//! link in the chain — flux, onset, tracker phase, wrap detection, hop
//! impulse — is the one lying. This prints all of them.
//!
//! Input is raw interleaved f32 PCM (`ffmpeg -f f32le`), so nothing here needs
//! raylib, a window, or an audio device:
//!
//! ```sh
//! ffmpeg -i track.mp3 -f f32le -acodec pcm_f32le track.raw
//! cargo run -p musializer-core --example cat_probe -- track.raw 44100 30 10
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
use musializer_core::scenes::clawd::ClawdState;
use musializer_core::timing::render_export::{
    frame_delta_seconds, frame_time_seconds, sample_cursor,
};

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 5 {
        eprintln!("usage: cat_probe RAW_F32LE_PATH SAMPLE_RATE FPS SECONDS [CHANNELS=2]");
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

    // The preview/export analyzer configuration: stereo interleaved input,
    // left channel selected, the file's own rate (`AudioAnalyzerConfig::preview`).
    let mut analyzer =
        AudioAnalyzer::new(AudioAnalyzerConfig::preview(sample_rate)).expect("analyzer");
    let mut tracker = BeatTracker::new();
    let settings = SceneSettings::new();
    let mut state = ClawdState::new(0);

    // The same profile the application builds at track load, from the same
    // samples this replay will hear — so `energy` and the show phases below
    // are exactly what a preview or export of this track computes.
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
    println!(
        "frame,t,rms,flux,onset,phase,wrap,beats,bounce,hop0,hop1,hop2,amp,bassflux,energy,kickrate,show,showlvl,expr"
    );
    let mut previous_phase = 0.0f32;
    for index in 0..total_frames {
        // `RenderJob::take_samples`: frame zero hears nothing, every later
        // frame hears exactly the samples between the two cursors.
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
        // The candidate kick signal: flux scoped to the lowest quarter of the
        // bands — the same positive-excursion shape as the global flux, over
        // the same region `bass_from_trails` calls bass.
        let low = (spectrum.smooth.len() / 4)
            .max(1)
            .min(spectrum.smooth.len());
        let bass_flux = if low == 0 {
            0.0
        } else {
            spectrum.smooth[..low]
                .iter()
                .zip(&spectrum.smear[..low])
                .map(|(band, trail)| (band - trail).max(0.0))
                .sum::<f32>()
                / low as f32
        };
        let mut audio = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        let time_seconds = frame_time_seconds(index, fps);
        audio.track_beat(&mut tracker, time_seconds);

        let wrap = audio.beat_phase.is_finite() && audio.beat_phase < previous_phase - 0.5;
        previous_phase = if audio.beat_phase.is_finite() {
            audio.beat_phase
        } else {
            0.0
        };

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
        state.update(&frame);

        println!(
            "{},{:.3},{:.4},{:.4},{},{:.4},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.3},{:.2},{},{:.2},{}",
            index,
            time_seconds,
            frame.audio.rms,
            frame.audio.spectral_flux,
            u8::from(frame.audio.onset),
            frame.audio.beat_phase,
            u8::from(wrap),
            state.beat_count(),
            state.bounce(),
            state.cat(0).map_or(-1.0, |c| c.hop),
            state.cat(1).map_or(-1.0, |c| c.hop),
            state.cat(2).map_or(-1.0, |c| c.hop),
            state.amplitude(),
            bass_flux,
            state.energy(),
            state.kick_rate(),
            state.show_phase().name(),
            state.show_level(),
            state.expression().name(),
        );
    }
    eprintln!(
        "beats={} learned_intervals={}",
        state.beat_count(),
        tracker.learned_intervals()
    );
}
