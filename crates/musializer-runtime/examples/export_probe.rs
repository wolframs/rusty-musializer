//! Drives one whole export through the real FFmpeg, and prints evidence.
//!
//! **Owner: Agent H.** `tools/headless_check.sh` runs this; it is the check that
//! an export is *deterministic* and that a windowed export is bit-identical to
//! the same frames of a full one. Neither claim can be made by a screenshot, and
//! neither can be made by `cargo test`, which must not depend on an external
//! encoder.
//!
//! An example rather than a binary because examples need no manifest entry, so
//! this cannot collide with a parallel agent — the pattern
//! `tools/differential_*.sh` already uses.
//!
//! # Why the frames are not the real scenes
//!
//! A scene needs a GL context, which needs a window, and the point of this probe
//! is the *transport*: decode, cursor, analyzer, encoder, publication. So the
//! frame source here is a deterministic bar chart drawn from the analyzer's own
//! output in plain Rust. That still exercises everything between the decoder and
//! the encoder, and it makes the determinism claim sharper rather than weaker —
//! any drift the run reports is the transport's, not a driver's.
//!
//! ```text
//! cargo run -p musializer-runtime --example export_probe -- \
//!     --audio build/fixture.wav --out build/render.mp4 [--window S D] \
//!     [--digest-range A B] [--fps N] [--size WxH] [--quality NAME]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use musializer_core::audio::AudioAnalyzer;
use musializer_core::project::sha256::Sha256;
use musializer_core::timing::render_export::{Quality, RenderExportConfig};
use musializer_runtime::process::ffmpeg::Finished;
use musializer_runtime::process::render_job::{RenderJob, RenderRequest};
use raylib::core::audio::RaylibAudio;

/// The probe's frame geometry, chosen small so a check can afford to run the
/// whole thing twice.
const DEFAULT_SIZE: (u32, u32) = (320, 240);

struct Options {
    audio: PathBuf,
    output: PathBuf,
    window: Option<(f64, f64)>,
    digest_range: Option<(u64, u64)>,
    config: RenderExportConfig,
}

fn main() -> ExitCode {
    let options = match parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("export_probe: {message}");
            return ExitCode::FAILURE;
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("export_probe: {message}");
            ExitCode::FAILURE
        }
    }
}

fn parse() -> Result<Options, String> {
    let mut audio = None;
    let mut output = None;
    let mut window = None;
    let mut digest_range = None;
    let mut config = RenderExportConfig {
        width: DEFAULT_SIZE.0,
        height: DEFAULT_SIZE.1,
        fps: 30,
        ..RenderExportConfig::default()
    };
    config.set_quality(Quality::Balanced);

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0usize;
    while index < arguments.len() {
        let value = |offset: usize| -> Result<&String, String> {
            arguments
                .get(index + offset)
                .ok_or_else(|| format!("{} needs {offset} value(s)", arguments[index]))
        };
        match arguments[index].as_str() {
            "--audio" => {
                audio = Some(PathBuf::from(value(1)?));
                index += 1;
            }
            "--out" => {
                output = Some(PathBuf::from(value(1)?));
                index += 1;
            }
            "--window" => {
                let start: f64 = value(1)?.parse().map_err(|_| "bad window start")?;
                let duration: f64 = value(2)?.parse().map_err(|_| "bad window duration")?;
                window = Some((start, duration));
                index += 2;
            }
            "--digest-range" => {
                let from: u64 = value(1)?.parse().map_err(|_| "bad digest range start")?;
                let to: u64 = value(2)?.parse().map_err(|_| "bad digest range end")?;
                digest_range = Some((from, to));
                index += 2;
            }
            "--fps" => {
                config.fps = value(1)?.parse().map_err(|_| "bad fps")?;
                index += 1;
            }
            "--size" => {
                let (width, height) = value(1)?.split_once('x').ok_or("size wants WIDTHxHEIGHT")?;
                config.width = width.parse().map_err(|_| "bad width")?;
                config.height = height.parse().map_err(|_| "bad height")?;
                index += 1;
            }
            "--quality" => {
                config.set_quality(match value(1)?.as_str() {
                    "balanced" => Quality::Balanced,
                    "high" => Quality::High,
                    "master" => Quality::Master,
                    other => return Err(format!("unknown quality {other}")),
                });
                index += 1;
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }

    Ok(Options {
        audio: audio.ok_or("--audio is required")?,
        output: output.ok_or("--out is required")?,
        window,
        digest_range,
        config,
    })
}

fn run(options: &Options) -> Result<(), String> {
    // The decoder lives behind the audio device in raylib-rs even though
    // `LoadWave` needs no device; under the headless check PULSE_SERVER is
    // deliberately unresolvable and miniaudio falls back to a null backend.
    let audio = RaylibAudio::init_audio_device()
        .map_err(|error| format!("could not initialize the audio device: {error}"))?;

    let request = RenderRequest {
        destination: &options.output,
        source_audio: &options.audio,
        config: options.config,
        window: options.window,
        protected: &[],
    };
    let mut job = RenderJob::start(&audio, &request).map_err(|error| error.to_string())?;

    let mut analyzer = AudioAnalyzer::boxed(job.analyzer_config())
        .map_err(|error| format!("could not configure the analyzer: {error}"))?;

    let plan = job.plan().clone();
    println!(
        "export: total={} window={}..{} samples={}..{}",
        plan.total_frames, plan.frames.start, plan.frames.end, plan.samples.start, plan.samples.end
    );

    // Two digests: everything this run encoded, and — for a full run — only the
    // frames a windowed run would have covered. Comparing the second against a
    // windowed run's first is the fast-forward claim, stated as a number.
    let mut encoded = Sha256::new();
    let mut ranged = Sha256::new();
    let mut ranged_frames = 0u64;
    let mut sent = 0u64;
    let mut pixels = vec![0u8; options.config.width as usize * options.config.height as usize * 4];

    while !job.is_complete() {
        let samples = job
            .take_samples()
            .map_err(|error| format!("export timeline failed: {error}"))?;
        if !samples.is_empty() {
            analyzer.push_interleaved(samples);
        }
        analyzer.analyze(job.scene_delta());

        if job.draws_this_frame() {
            let index = job.frame_index();
            draw_bars(
                &mut pixels,
                options.config.width as usize,
                options.config.height as usize,
                &analyzer,
            );
            encoded.update(&pixels);
            if options
                .digest_range
                .is_some_and(|(from, to)| index >= from && index < to)
            {
                ranged.update(&pixels);
                ranged_frames += 1;
            }
            job.send_frame(
                &pixels,
                options.config.width as usize,
                options.config.height as usize,
            )
            .map_err(|error| format!("frame {index} could not be written: {error}"))?;
            sent += 1;
        }
        job.advance();
    }

    println!("export: encoded={sent}");
    println!(
        "export: frames-sha256={}",
        musializer_core::project::sha256::hex(&encoded.finalize())
    );
    if options.digest_range.is_some() {
        println!(
            "export: range-frames={ranged_frames} range-sha256={}",
            musializer_core::project::sha256::hex(&ranged.finalize())
        );
    }

    let completion = job.finish(false);
    if let Some(retained) = &completion.retained_staging {
        println!("export: RETAINED staging {}", retained.display());
    }
    match completion.result {
        Ok(Finished::Published) => {}
        Ok(Finished::Cancelled) => return Err("the encoder reported a cancellation".into()),
        Err(error) => return Err(format!("the encoder failed: {error}")),
    }

    let bytes = std::fs::read(&completion.destination)
        .map_err(|error| format!("could not read the published file: {error}"))?;
    println!("export: bytes={}", bytes.len());
    println!(
        "export: file-sha256={}",
        musializer_core::project::sha256::digest_hex(&bytes)
    );
    Ok(())
}

/// One deterministic frame: a bar per analyzer band, over a flat background.
///
/// Every value comes from the analyzer, so a frame differs from its neighbour
/// only because the audio did. Nothing here reads a clock, a random number or an
/// environment variable, which is the property the determinism check rests on.
fn draw_bars(pixels: &mut [u8], width: usize, height: usize, analyzer: &AudioAnalyzer) {
    pixels.fill(0);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    let spectrum = analyzer.spectrum();
    let bands = spectrum.band_count();
    if bands == 0 || width == 0 || height == 0 {
        return;
    }
    for column in 0..width {
        let band = column * bands / width;
        let level = spectrum.smooth[band].clamp(0.0, 1.0);
        let bar = (level * height as f32) as usize;
        for row in (height - bar.min(height))..height {
            let offset = (row * width + column) * 4;
            pixels[offset] = 0x00;
            pixels[offset + 1] = 0x2F;
            pixels[offset + 2] = 0xA7;
            pixels[offset + 3] = 0xFF;
        }
    }
}
