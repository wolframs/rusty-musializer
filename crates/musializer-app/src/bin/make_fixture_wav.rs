//! Writes a synthetic WAV for headless checks.
//!
//! The repository rule is synthetic fixtures only — no user audio ever enters
//! this tree. A generator rather than a committed `.wav` keeps the fixture
//! auditable and the repository small.
//!
//! ```sh
//! cargo run --bin make-fixture-wav -- out.wav [seconds]
//! ```
//!
//! The signal is a slow sweep from 110 Hz to 3.5 kHz with a 2 Hz amplitude
//! pulse. Both properties are deliberate: the sweep walks energy across the
//! analyzer's logarithmic bands so a *moving* peak proves the FFT is live rather
//! than stuck, and the pulse makes the reaction visible frame to frame.

use std::f32::consts::TAU;
use std::io::Write;

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const BITS_PER_SAMPLE: u16 = 16;

const SWEEP_START_HZ: f32 = 110.0;
const SWEEP_END_HZ: f32 = 3_500.0;
const PULSE_HZ: f32 = 2.0;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: make-fixture-wav <out.wav> [seconds]");
        std::process::exit(2);
    });
    let seconds: f32 = args
        .next()
        .map(|value| value.parse().expect("seconds must be a number"))
        .unwrap_or(10.0);

    let frame_count = (SAMPLE_RATE as f32 * seconds) as u32;
    let mut samples: Vec<i16> = Vec::with_capacity(frame_count as usize * CHANNELS as usize);

    // Integrate the swept frequency into a phase so the sweep has no
    // discontinuities; stepping phase by an instantaneous frequency would click.
    let mut phase = 0.0f32;
    for frame in 0..frame_count {
        let t = frame as f32 / SAMPLE_RATE as f32;
        let progress = if seconds > 0.0 { t / seconds } else { 0.0 };
        let frequency = SWEEP_START_HZ + (SWEEP_END_HZ - SWEEP_START_HZ) * progress;
        phase += TAU * frequency / SAMPLE_RATE as f32;
        if phase > TAU {
            phase -= TAU;
        }
        let pulse = 0.55 + 0.45 * (TAU * PULSE_HZ * t).sin();
        let value = phase.sin() * pulse * 0.8;
        let quantized = (value * i16::MAX as f32) as i16;
        // Right channel slightly quieter, so a channel-mix bug is visible.
        samples.push(quantized);
        samples.push((quantized as f32 * 0.75) as i16);
    }

    let data_bytes = samples.len() * 2;
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);

    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * (BITS_PER_SAMPLE / 8) as u32;
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    file.write_all(b"RIFF")?;
    file.write_all(&((36 + data_bytes) as u32).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&CHANNELS.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&(data_bytes as u32).to_le_bytes())?;
    for sample in &samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    file.flush()?;

    println!(
        "wrote {path}: {seconds}s, {SAMPLE_RATE} Hz, {CHANNELS} ch, \
         {SWEEP_START_HZ}->{SWEEP_END_HZ} Hz sweep with a {PULSE_HZ} Hz pulse"
    );
    Ok(())
}
