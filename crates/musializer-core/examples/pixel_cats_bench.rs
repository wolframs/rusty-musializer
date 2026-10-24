//! How long a Pixel Cats frame takes on the CPU, per staging.
//!
//!     cargo run --release -p musializer-core --example pixel_cats_bench
//!
//! Updates and renders each staging for 1200 frames (20 s at 60 fps) over a
//! synthetic groove with a lyric on screen, at the 320x180 canvas a 1080p or
//! 720p output draws, and at 400x225 (a canvas set to fewer pixel rows draws
//! less, not more). Prints mean, 99th percentile and worst milliseconds per
//! frame for update + render + the compositing pass. A measurement, not a
//! gate: the numbers belong to this machine.

use std::time::Instant;

use musializer_core::scene::{LyricCue, SceneAudioFrame, SceneFrame, SceneSettings, SceneState};
use musializer_core::scenes::pixel_cats::{raster::Raster, render, PixelCatsState, Staging};

fn main() {
    let settings = SceneSettings::new();
    let cue = LyricCue {
        id: 1,
        start_seconds: 0.5,
        end_seconds: 19.0,
        text: "I found a terminal behind the drums".into(),
        voice: None,
    };
    for (width, height) in [(320, 180), (400, 225)] {
        println!("canvas {width}x{height}");
        for staging in Staging::ALL {
            let mut state = PixelCatsState::new(staging, 7);
            let mut raster = Raster::new(width, height);
            let mut rgba = Vec::new();
            let mut bands = [0.02f32; 104];
            let mut trails = [0.02f32; 104];
            let mut times = Vec::with_capacity(1200);
            for step in 0..1200u64 {
                let time = step as f64 / 60.0;
                let beat = (time * 3.0).fract() as f32;
                for (i, band) in bands.iter_mut().enumerate() {
                    *band = if i < 26 {
                        0.25 + 0.7 * (-beat * 9.0).exp()
                    } else {
                        0.5 + 0.2 * ((time as f32) * 3.0 + i as f32).sin()
                    };
                }
                for (trail, band) in trails.iter_mut().zip(bands.iter()) {
                    *trail += (band - *trail) * 0.18;
                }
                let mut frame = SceneFrame::idle(&settings);
                frame.time_seconds = time;
                frame.duration_seconds = 60.0;
                frame.delta_seconds = 1.0 / 60.0;
                frame.audio = SceneAudioFrame::from_spectrum(&bands, &trails);
                frame.audio.beat_phase = beat;
                frame.lyric = Some(&cue).filter(|cue| time >= cue.start_seconds);
                let started = Instant::now();
                state.update(&frame);
                let report = render(&state, &frame, width, height, &mut raster, &mut rgba);
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                std::hint::black_box(report);
            }
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            println!(
                "  {:8} mean {:.3} ms  p99 {:.3} ms  worst {:.3} ms",
                staging.name(),
                mean,
                times[times.len() * 99 / 100],
                times[times.len() - 1]
            );
        }
    }
}
