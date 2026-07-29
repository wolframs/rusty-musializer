//! Dumps the Rust Song Atlas terrain for a set of deterministic synthetic tracks,
//! in the same format as `tests/differential/song_atlas_map_oracle.c`.
//!
//! Run through `tools/differential_song_atlas_map.sh`, which builds
//! `song_atlas_map.c` and `audio_analyzer.c` from the frozen C source, runs both,
//! and compares numerically. The module is pure — interleaved PCM in, a vector of
//! slices out — and its three passes are each order-sensitive in ways that still
//! produce a plausible terrain when rearranged, which is precisely the class of
//! error a review cannot catch.
//!
//! An `examples/` target rather than a `[[bin]]` on purpose: examples need no
//! manifest entry, so this does not touch a file another agent might be editing.
//!
//! The fixture generators are duplicated between here and the C harness
//! deliberately. Sharing them would let a bug hide in the shared half.

use std::f64::consts::PI;

use musializer_core::audio::song_atlas_map::{
    render_distance, render_sample_count, render_sample_index, slice_count, SongAtlasMap,
    BAND_COUNT,
};

/// Ten significant digits in exponential form, matching the C harness. f32
/// round-trips in nine, so printing is not the limiting factor and any delta the
/// comparison reports is arithmetic. Exponential form also keeps a float token
/// from ever looking like an integer to the comparison script, which is how it
/// knows which columns to compare exactly.
fn f32_token(value: f32) -> String {
    format!("{:.9e}", value)
}

fn f64_token(value: f64) -> String {
    format!("{:.16e}", value)
}

/// A sine sampled at an absolute frame index, with the phase accumulated in `f64`
/// and narrowed once before `sin`. Duplicated in `song_atlas_map_oracle.c`; the
/// evaluation order below is load-bearing for bit-identical PCM.
fn fixture_sine(frame: usize, sample_rate: u32, frequency: f32, amplitude: f32) -> f32 {
    let phase = 2.0 * PI * f64::from(frequency) * frame as f64 / f64::from(sample_rate);
    amplitude * (phase as f32).sin()
}

/// Fixture 1 and 3: a single mono tone.
fn mono_sine(frame_count: usize, sample_rate: u32, frequency: f32, amplitude: f32) -> Vec<f32> {
    (0..frame_count)
        .map(|frame| fixture_sine(frame, sample_rate, frequency, amplitude))
        .collect()
}

/// Fixture 2: stereo where the two channels carry *different* partials, so
/// `ChannelMode::Mix` is actually exercised. A stereo fixture whose channels are
/// equal cannot tell mixing apart from selecting channel 0, and the atlas mixes
/// while the live preview selects.
fn stereo_partials(frame_count: usize, sample_rate: u32) -> Vec<f32> {
    let mut samples = Vec::with_capacity(frame_count * 2);
    for frame in 0..frame_count {
        let left = 0.50 * fixture_sine(frame, sample_rate, 220.0, 1.0)
            + 0.30 * fixture_sine(frame, sample_rate, 1760.0, 1.0);
        let right = 0.40 * fixture_sine(frame, sample_rate, 330.0, 1.0)
            + 0.20 * fixture_sine(frame, sample_rate, 3520.0, 1.0);
        samples.push(left);
        samples.push(right);
    }
    samples
}

/// Fixture 4: half a second of tone, half a second of silence, repeating. The hard
/// cut lands mid-cycle, so every block boundary is a broadband transient — which
/// is what the onset detector and its minimum-separation rule need in order to be
/// checked at all.
fn transient_mono(frame_count: usize, sample_rate: u32) -> Vec<f32> {
    let block_frames = sample_rate as usize / 2;
    (0..frame_count)
        .map(|frame| {
            if (frame / block_frames) % 2 == 0 {
                fixture_sine(frame, sample_rate, 300.0, 0.9)
            } else {
                0.0
            }
        })
        .collect()
}

/// Fixture 5: three partials under a slow envelope, plus a short click twice a
/// second, long enough to hit the slice cap. The envelope keeps the loudness
/// normalization from being a no-op and the clicks keep the flux lane populated
/// across all 576 slices.
fn long_stereo(frame_count: usize, sample_rate: u32) -> Vec<f32> {
    let click_period = sample_rate as usize / 2;
    let mut samples = Vec::with_capacity(frame_count * 2);
    for frame in 0..frame_count {
        let envelope = 0.55 + 0.45 * fixture_sine(frame, sample_rate, 3.0, 1.0);
        let a = fixture_sine(frame, sample_rate, 180.0, 1.0);
        let b = 0.60 * fixture_sine(frame, sample_rate, 900.0, 1.0);
        let c = 0.35 * fixture_sine(frame, sample_rate, 2600.0, 1.0);
        let mut base = (a + b + c) * envelope * 0.4;
        if frame % click_period < 48 {
            base += 0.5;
        }
        samples.push(base);
        samples.push(base * 0.75);
    }
    samples
}

fn dump_fixture(name: &str, samples: &[f32], channel_count: usize, sample_rate: u32) {
    // C signals every rejection by clearing the destination and returning 0; Rust
    // returns an error and has nothing to clear. Both print the same thing here,
    // so a divergence in *what is rejected* is visible too.
    let map = SongAtlasMap::build(samples, channel_count, sample_rate);
    println!("fixture {name}");
    match map {
        Ok(map) => {
            println!("count {}", map.len());
            println!("duration {}", f64_token(map.duration_seconds()));
            println!("valid {}", u8::from(map.is_valid()));
            for (index, slice) in map.slices().iter().enumerate() {
                let mut line = format!(
                    "slice {index} {} {} {}",
                    f32_token(slice.rms),
                    f32_token(slice.flux),
                    u8::from(slice.onset),
                );
                for band in 0..BAND_COUNT {
                    line.push(' ');
                    line.push_str(&f32_token(slice.bands[band]));
                }
                println!("{line}");
            }
        }
        Err(error) => {
            println!("count 0");
            println!("duration {}", f64_token(0.0));
            println!("valid 0");
            eprintln!("build failed for fixture {name}: {error}");
        }
    }
}

/// The four pure helpers alongside the build, because they are the same module and
/// they are integers: any difference is an indexing bug, not a rounding one.
fn dump_helpers() {
    #[rustfmt::skip]
    const COUNTS: &[(usize, u32)] = &[
        (0, 48_000), (48_000, 0), (1, 48_000), (16_000, 8_000),
        (12 * 48_000, 48_000), (13 * 48_000, 48_000), (96 * 48_000, 48_000),
        (800_000, 8_000), (441_000, 22_050), (24_000, 8_000), (30_000, 44_100),
        (3_000, 8_000), (123_457, 44_100),
    ];
    for &(frame_count, sample_rate) in COUNTS {
        println!(
            "helper_slice_count {frame_count} {sample_rate} {}",
            slice_count(frame_count, sample_rate)
        );
    }

    #[rustfmt::skip]
    const SAMPLE_COUNTS: &[(usize, usize)] = &[
        (0, 1), (1, 1), (2, 0), (2, 1), (2, 3), (3, 1), (72, 1), (72, 2),
        (72, 3), (120, 1), (576, 1), (576, 2), (576, 3), (576, 4), (577, 3),
    ];
    for &(available, detail) in SAMPLE_COUNTS {
        println!(
            "helper_render_sample_count {available} {detail} {}",
            render_sample_count(available, detail)
        );
    }

    #[rustfmt::skip]
    const INDICES: &[(usize, usize, usize, usize)] = &[
        (0, 576, 192, 0), (0, 576, 192, 1), (0, 576, 192, 95),
        (0, 576, 192, 191), (0, 576, 192, 192), (20, 556, 192, 0),
        (20, 556, 192, 191), (7, 576, 576, 575), (0, 1, 192, 0),
        (0, 577, 192, 0), (0, 576, 1, 0), (0, 192, 576, 0),
        (usize::MAX, 576, 192, 0), (5, 72, 24, 13),
    ];
    for &(first, available, sample_count, sample) in INDICES {
        // C returns SIZE_MAX where this returns None. Printing `none` for both
        // keeps the two rejection sets comparable without pretending SIZE_MAX is
        // a slice index.
        let result = render_sample_index(first, available, sample_count, sample)
            .map_or_else(|| "none".to_string(), |index| index.to_string());
        println!("helper_render_sample_index {first} {available} {sample_count} {sample} {result}");
    }

    // The labels avoid anything Python's `float()` would accept — no bare `nan`,
    // `inf`, or digits — so the comparison script cannot mistake a label for a
    // number and compare it with a tolerance. A NaN compared that way passes
    // unconditionally, which is a silent hole in a harness.
    #[rustfmt::skip]
    const DISTANCES: &[(&str, f32)] = &[
        ("zero", 0.0), ("one", 1.0), ("d575", 575.0), ("negative", -12.5),
        ("tiny", 1.0e-3), ("not_a_number", f32::NAN),
        ("positive_infinity", f32::INFINITY),
        ("negative_infinity", f32::NEG_INFINITY),
    ];
    for &(label, input) in DISTANCES {
        println!(
            "helper_render_distance {label} {}",
            f32_token(render_distance(input))
        );
    }
}

fn main() {
    dump_helpers();

    // 3 s mono tone at 8 kHz: the floored 72-slice case, and the shape
    // ../musializer/tests/test_song_atlas_map.c checks by centroid only.
    dump_fixture(
        "mono_sine_3s",
        &mono_sine(24_000, 8_000, 1_800.0, 0.7),
        1,
        8_000,
    );
    // Stereo with differing channels, so ChannelMode::Mix is exercised.
    dump_fixture(
        "stereo_partials",
        &stereo_partials(30_000, 44_100),
        2,
        44_100,
    );
    // Shorter than one FFT window: every slice reads the same clamped window,
    // which is where the slice arithmetic is most likely to be off by one without
    // anything looking wrong.
    dump_fixture(
        "short_mono",
        &mono_sine(3_000, 8_000, 440.0, 0.35),
        1,
        8_000,
    );
    // 20 s of alternating tone and silence: onsets and their separation.
    dump_fixture(
        "transient_mono",
        &transient_mono(441_000, 22_050),
        1,
        22_050,
    );
    // 100 s: past the cap, so exactly MAX_SLICES slices.
    dump_fixture("long_stereo_cap", &long_stereo(800_000, 8_000), 2, 8_000);
}
