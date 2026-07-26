//! The Musializer binary.
//!
//! Right now this is the Phase 1 vertical slice: open a window, initialize audio,
//! play a track, feed samples through the allocation-free callback bridge,
//! analyze them in Rust, draw Spectrum, and shut down cleanly. Agent F grows a
//! workspace around this; the frame loop's shape is the integration owner's.
//!
//! ## The probe flags
//!
//! `--probe-frames N` and `--probe-shot PATH` exist so this can check its own
//! work without a human looking at a screen, following
//! `../musializer/tools/UI_REVIEW.md`. They are diagnostics, deliberately named
//! so they cannot be confused with the product CLI Agent F will implement.

use std::path::PathBuf;

use musializer_core::audio::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::scene::{SceneAudioFrame, SceneFrame, SceneSettings};
use musializer_runtime::audio_bridge;
use raylib::prelude::*;

mod scenes;

/// The minimum window the C project supports. A panel's minimum size must be
/// measured against what this actually produces, not against a guessed threshold
/// (REWRITE_PLAN.md, Agent F's layout rules).
const MIN_WINDOW: (i32, i32) = (960, 640);
const DEFAULT_WINDOW: (i32, i32) = (1280, 720);

struct Options {
    audio_path: Option<PathBuf>,
    probe_frames: Option<u32>,
    probe_shot: Option<PathBuf>,
    window: (i32, i32),
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        audio_path: None,
        probe_frames: None,
        probe_shot: None,
        window: DEFAULT_WINDOW,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--probe-frames" => {
                let value = args.next().ok_or("--probe-frames needs a count")?;
                options.probe_frames = Some(
                    value
                        .parse()
                        .map_err(|_| format!("bad frame count: {value}"))?,
                );
            }
            "--probe-shot" => {
                options.probe_shot = Some(PathBuf::from(
                    args.next().ok_or("--probe-shot needs a path")?,
                ));
            }
            "--size" => {
                let value = args.next().ok_or("--size needs WIDTHxHEIGHT")?;
                let (w, h) = value.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                options.window = (
                    w.parse().map_err(|_| format!("bad width: {w}"))?,
                    h.parse().map_err(|_| format!("bad height: {h}"))?,
                );
            }
            "--version" => {
                println!(
                    "musializer-rs {} (raylib {})",
                    env!("CARGO_PKG_VERSION"),
                    musializer_runtime::RAYLIB_VERSION
                );
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            // Unlike the C parser, an unknown flag is an error rather than a
            // file path. The oracle falls through to its positional arm and
            // tries to load `--typo` as audio (`../musializer/src/musializer.c:546-550`),
            // which is a diagnostic gap worth not reproducing.
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other}"));
            }
            positional => {
                if options.audio_path.is_some() {
                    return Err("only one audio path is supported so far".into());
                }
                options.audio_path = Some(PathBuf::from(positional));
            }
        }
    }
    Ok(options)
}

fn print_help() {
    println!(
        "usage: musializer [AUDIO] [--size WxH]\n\
         \n\
         Phase 1 vertical slice: window, audio, analyzer, Spectrum.\n\
         \n\
         diagnostics:\n\
         \x20 --probe-frames N   render N frames then exit\n\
         \x20 --probe-shot PATH  write a PNG of the last rendered frame"
    );
}

fn main() -> Result<(), String> {
    // Keeps the raylib-5-5-link crate in the link graph; see its build.rs.
    let raylib_version = musializer_runtime::ensure_raylib_linked();

    let options = parse_options()?;

    let (width, height) = options.window;
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Musializer (Rust)")
        .msaa_4x()
        .resizable()
        .build();
    rl.set_window_min_size(MIN_WINDOW.0, MIN_WINDOW.1);
    rl.set_target_fps(60);

    let audio = RaylibAudio::init_audio_device()
        .map_err(|error| format!("could not initialize the audio device: {error}"))?;

    let mut shader = scenes::spectrum::CircleShader::load(&mut rl, &thread)?;

    audio_bridge::install(audio_bridge::DEFAULT_CAPACITY)
        .map_err(|error| format!("could not install the audio bridge: {error}"))?;

    // Starts in the oracle's no-track configuration and is reconfigured from the
    // file's sample rate once a track loads, mirroring `analyzer_configure`.
    // 200 KiB of arrays, so it is boxed.
    let mut analyzer = AudioAnalyzer::boxed(AudioAnalyzerConfig::idle())
        .map_err(|error| format!("could not create the analyzer: {error}"))?;

    let settings = SceneSettings::default();

    // Scoped so the Music is dropped — and the processor detached — while the
    // audio device and window are still alive. Getting that order wrong is one
    // of the plan's named traps.
    let music = match options.audio_path.as_ref() {
        Some(path) => {
            let path_str = path.to_str().ok_or("audio path is not valid UTF-8")?;
            let music = audio
                .new_music(path_str)
                .map_err(|error| format!("could not load {}: {error}", path.display()))?;
            // `start_preview_track` reconfigures the analyzer from the file's
            // sample rate (`../musializer/src/plug.c:658-660`).
            let file_sample_rate = music.stream.sampleRate;
            *analyzer = *AudioAnalyzer::boxed(AudioAnalyzerConfig::preview(file_sample_rate))
                .map_err(|error| format!("could not configure the analyzer: {error}"))?;
            music.play_stream();
            // SAFETY: the bridge is installed above, the audio device is
            // initialized, and `music` outlives the detach below because both
            // live in this function's scope and the detach happens before the
            // end of it.
            unsafe { audio_bridge::attach(music.stream) }
                .map_err(|error| format!("could not attach the audio bridge: {error}"))?;
            Some(music)
        }
        None => {
            eprintln!("note: no audio path given — the window opens but Spectrum will idle");
            None
        }
    };

    // Interleaved stereo scratch, drained from the ring each frame. Sized for a
    // long frame at 44.1 kHz so a hitch does not silently discard audio.
    let mut scratch = vec![0.0f32; 4096 * audio_bridge::MIXED_CHANNELS];

    let mut frame_index: u64 = 0;
    let mut consumed_frames: u64 = 0;
    let mut analyzed_frames: u64 = 0;
    let mut peak_seen: f32 = 0.0;
    let mut peak_band_last: usize = 0;
    let mut peak_band_low: usize = usize::MAX;
    let mut peak_band_high: usize = 0;

    while !rl.window_should_close() {
        if let Some(music) = music.as_ref() {
            music.update_stream();
        }

        let drained = audio_bridge::drain_interleaved(&mut scratch);
        if drained > 0 {
            let consumed =
                analyzer.push_interleaved(&scratch[..drained * audio_bridge::MIXED_CHANNELS]);
            consumed_frames += consumed as u64;
        }

        // The scene clock is the frame delta, which is what preview and export
        // must agree on.
        let delta = rl.get_frame_time();
        if analyzer.analyze(delta) {
            analyzed_frames += 1;
        }

        let time_seconds = music
            .as_ref()
            .map_or(0.0, |m| f64::from(m.get_time_played()));
        let duration_seconds = music
            .as_ref()
            .map_or(0.0, |m| f64::from(m.get_time_length()));

        let spectrum = analyzer.spectrum();
        let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        peak_seen = peak_seen.max(audio_frame.peak);
        audio_frame.beat_phase = 0.0; // Agent A's beat tracker lands here.

        // Which band is loudest, and at what frequency. Reported so a headless
        // check can assert that the spectrum *moved* rather than that it merely
        // drew something: with a swept fixture, a peak band that never changes
        // means the analyzer is stuck.
        let peak_band = (0..spectrum.band_count())
            .max_by(|&a, &b| spectrum.smooth[a].total_cmp(&spectrum.smooth[b]));
        if let Some(band) = peak_band {
            peak_band_last = band;
            peak_band_low = peak_band_low.min(band);
            peak_band_high = peak_band_high.max(band);
        }

        let frame = SceneFrame {
            time_seconds,
            duration_seconds,
            delta_seconds: delta,
            frame_index,
            audio: audio_frame,
            settings: &settings,
            ..SceneFrame::idle(&settings)
        };

        let band_centre_hz = spectrum
            .band_first_bin
            .get(peak_band_last)
            .map_or(0.0, |&bin| analyzer.bin_frequency(bin as usize));

        let screen = Rectangle::new(
            0.0,
            0.0,
            rl.get_screen_width() as f32,
            rl.get_screen_height() as f32,
        );

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::get_color(0x181818ff));
        scenes::spectrum::draw(&mut d, &frame, &mut shader, screen, 1.0);
        // A one-line readout, so a headless capture carries its own evidence
        // rather than needing a separate log to be trusted.
        d.draw_text(
            &format!(
                "frame {frame_index}  t={time_seconds:.2}s  bands={}  peak band={peak_band_last} ({:.0} Hz)  rms={:.3}  audio frames={consumed_frames}",
                spectrum.band_count(),
                band_centre_hz,
                frame.audio.rms,
            ),
            12,
            12,
            18,
            Color::RAYWHITE,
        );
        drop(d);

        frame_index += 1;
        if let Some(limit) = options.probe_frames {
            if frame_index >= u64::from(limit) {
                if let Some(path) = options.probe_shot.as_ref() {
                    let path_str = path
                        .to_str()
                        .ok_or("--probe-shot path is not valid UTF-8")?;
                    // Not `take_screenshot`: raylib's `TakeScreenshot` runs the
                    // path through `GetFileName` and writes to the working
                    // directory, so a `build/x/y.png` argument silently lands in
                    // the repository root. Grabbing the image and exporting it
                    // honours the path given.
                    let image = rl.load_image_from_screen(&thread);
                    image.export_image(path_str);
                    // `export_image` reports nothing, so confirm by looking.
                    if !std::path::Path::new(path_str).is_file() {
                        return Err(format!("could not write {path_str}"));
                    }
                }
                break;
            }
        }
    }

    // Detach before the Music drops, so raylib's per-stream processor list never
    // holds a callback for a freed stream.
    if let Some(music) = music.as_ref() {
        // SAFETY: the same stream passed to `attach`, still alive here.
        unsafe { audio_bridge::detach(music.stream) };
    }
    drop(music);

    let dropped = audio_bridge::ring().map_or(0, |ring| ring.dropped());
    println!("--- slice report ---");
    println!("verified:        window opened, audio device initialized, clean shutdown");
    println!("raylib:          {raylib_version}");
    println!("frames rendered: {frame_index}");
    println!("analyzer runs:   {analyzed_frames}");
    println!("audio frames:    {consumed_frames} consumed, {dropped} dropped");
    println!("bands:           {}", analyzer.band_count());
    println!("peak seen:       {peak_seen:.4}");
    let peak_band_moved = peak_band_low != usize::MAX && peak_band_high > peak_band_low;
    if peak_band_low == usize::MAX {
        println!("peak band:       never established");
    } else {
        println!("peak band:       {peak_band_low}..{peak_band_high} (last {peak_band_last})");
    }
    println!("dropped frames:  {dropped}");
    // Three separate claims, because "it compiled" and "it drew something" are
    // both weaker than the gate. A swept fixture that leaves the peak band
    // stationary means the analyzer is stuck even though the picture looks fine.
    println!(
        "verdict:         {}",
        match (consumed_frames > 0, peak_seen > 0.0, peak_band_moved) {
            (true, true, true) =>
                "audio advanced, the spectrum responded, and the peak band tracked the sweep",
            (true, true, false) =>
                "audio advanced and bands were non-zero, but the peak band never moved",
            (true, false, _) => "audio advanced but the spectrum stayed flat",
            (false, _, _) => "no audio was consumed",
        }
    );

    Ok(())
}
