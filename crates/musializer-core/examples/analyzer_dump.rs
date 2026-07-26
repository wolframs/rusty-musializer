//! Dumps the Rust analyzer's output for a deterministic synthetic signal, in the
//! same format as `tests/differential/analyzer_oracle.c`.
//!
//! Run through `tools/differential_analyzer.sh`, which builds the oracle's
//! analyzer from the frozen C source, runs both, and compares numerically. That
//! is the difference between believing the port is faithful and knowing it.
//!
//! An `examples/` target rather than a `[[bin]]` on purpose: examples need no
//! manifest entry, so this does not touch a file another agent might be editing.
//!
//! The signal generator is duplicated between here and the C harness
//! deliberately. Sharing it would let a bug hide in the shared half.

use std::f32::consts::PI;

use musializer_core::audio::analyzer::{AudioAnalyzer, AudioAnalyzerConfig};

const SAMPLE_RATE: u32 = 44_100;
const PUSH_FRAMES: u32 = 8192;
const ANALYZE_STEPS: usize = 120;
const DT_SECONDS: f32 = 0.016_666_667;

/// Three fixed partials plus a slow amplitude envelope, so energy lands in
/// several well-separated logarithmic bands.
fn synthetic_sample(n: u32) -> f32 {
    let t = n as f32 / SAMPLE_RATE as f32;
    let a = (2.0 * PI * 220.0 * t).sin();
    let b = 0.6 * (2.0 * PI * 1320.0 * t).sin();
    let c = 0.35 * (2.0 * PI * 5280.0 * t).sin();
    let envelope = 0.55 + 0.45 * (2.0 * PI * 3.0 * t).sin();
    (a + b + c) * envelope * 0.4
}

fn main() {
    let mut analyzer = AudioAnalyzer::boxed(AudioAnalyzerConfig::preview(SAMPLE_RATE))
        .expect("preview config is valid");

    let mut interleaved = Vec::with_capacity(PUSH_FRAMES as usize * 2);
    for frame in 0..PUSH_FRAMES {
        let sample = synthetic_sample(frame);
        interleaved.push(sample);
        // Right is a scaled copy, so a channel-mode divergence shows up rather
        // than cancelling.
        interleaved.push(sample * 0.75);
    }
    let consumed = analyzer.push_interleaved(&interleaved);
    assert_eq!(
        consumed, PUSH_FRAMES as usize,
        "push_interleaved consumed the wrong count"
    );

    for step in 0..ANALYZE_STEPS {
        assert!(
            analyzer.analyze(DT_SECONDS),
            "analyze failed at step {step}"
        );
    }

    let spectrum = analyzer.spectrum();
    println!("band_count {}", spectrum.band_count());
    for i in 0..spectrum.band_count() {
        println!(
            "{i} {} {} {:.9} {:.9} {:.9}",
            spectrum.band_first_bin[i],
            spectrum.band_end_bin[i],
            spectrum.logarithmic[i],
            spectrum.smooth[i],
            spectrum.smear[i],
        );
    }
}
