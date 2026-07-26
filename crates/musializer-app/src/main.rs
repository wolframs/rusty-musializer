//! The Musializer binary: composition root.
//!
//! This grew around the Phase 1 vertical slice rather than replacing it. The
//! slice's path — window, audio device, callback bridge, analyzer, Spectrum,
//! clean shutdown, and the `--probe-frames`/`--probe-shot`/`--size` diagnostics
//! with their report — is still the only thing proving P1, and
//! `tools/headless_check.sh` depends on that report's format. What is new is a
//! real CLI ([`cli`]), all ten scenes selectable ([`scene_host`]), and an
//! operable workspace around the preview ([`ui::shell`]).
//!
//! ## Order of operations
//!
//! Reproduced from `../musializer/src/musializer.c:315-662`, because the order is
//! semantics, not sequence:
//!
//! 1. `-h`/`--help`/`--version` pre-pass over all of `argv` — exits 0 before a
//!    window opens, even when the rest of the line is invalid.
//! 2. Window, minimum size, audio device.
//! 3. The `argv` actions, left to right, so a later input overwrites an earlier
//!    one.
//! 4. **Then** the deferred routes, so a project hydration cannot overwrite a
//!    route that appeared earlier in `argv` (`musializer.c:553-561`).
//! 5. Render config, save, probe — each short-circuited by the shared error flag.
//!
//! ## Where the `App` state is
//!
//! Deliberately not a `Plug *p` equivalent. The frame loop owns the audio and the
//! analyzer; [`ui::shell::Shell`] owns editor state and returns
//! [`ShellCommand`](ui::shell::ShellCommand)s rather than mutating anything. That
//! is why there is no `Rc<RefCell<_>>` anywhere in this file.

use std::path::{Path, PathBuf};

use musializer_core::audio::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::scene::routes::{RouteSources, RouteTable};
use musializer_core::scene::settings;
use musializer_core::scene::{SceneAudioFrame, SceneFrame, SceneId, SceneInstance, SceneSettings};
use musializer_core::ui::notice::Severity;
use musializer_runtime::audio_bridge;
use musializer_runtime::font::Faces;
use musializer_runtime::process::dialogs::{self, FileDialog};
use raylib::prelude::*;

mod cli;
mod scene_host;
mod scenes;
mod ui;

use cli::{Action, Cli, Outcome};
use ui::shell::{Shell, ShellCommand, ShellInput};

/// Seed for a scene with no track to derive one from. The C seeds per track
/// (`scene_seed_for_track`); until the track model lands (Agent B), one constant
/// keeps scene state deterministic, which is the property that actually matters.
const DEFAULT_SCENE_SEED: u64 = 0x5eed_0000_0000_0001;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(status) => status,
        Err(message) => {
            eprintln!("musializer: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<std::process::ExitCode, String> {
    // Keeps the raylib-5-5-link crate in the link graph; see its build.rs.
    let raylib_version = musializer_runtime::ensure_raylib_linked();

    // The pre-pass wins from any position and opens no window.
    let mut options = match cli::parse(std::env::args().skip(1)) {
        Outcome::Help => {
            let program = std::env::args()
                .next()
                .unwrap_or_else(|| "musializer".into());
            print!("{}", cli::help_text(&program));
            return Ok(std::process::ExitCode::SUCCESS);
        }
        Outcome::Version => {
            // Three spellings of the version exist in the C, from three separate
            // literals. This build claims none of them; see REWRITE_PLAN.md's
            // open question.
            println!(
                "musializer-rs {} (raylib {raylib_version})",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(std::process::ExitCode::SUCCESS);
        }
        Outcome::Parsed(cli) => cli,
    };

    for warning in &options.warnings {
        eprintln!("warning: {warning}");
    }
    // Captured before the actions are drained, so the report can check the scene
    // that got bound against the one the command line asked for. A `--scene` flag
    // that parsed but did not take effect is exactly the failure a clean exit
    // would otherwise hide.
    let requested_scene = options.requested_scene();

    let (width, height) = options
        .ui_probe
        .as_ref()
        .and_then(|probe| probe.size)
        .map_or(options.window, |(w, h)| (w as i32, h as i32));

    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Musializer (Rust)")
        .msaa_4x()
        .resizable()
        .build();
    // GLFW clamps a smaller request to this, which is why a deliberately tiny
    // `--ui-probe size=` photographs the smallest layout the app permits
    // (`musializer.c:354`, `:599-601`).
    rl.set_window_min_size(cli::MIN_WINDOW.0, cli::MIN_WINDOW.1);
    rl.set_target_fps(60);
    if options.ui_probe.is_some() {
        // Park the window at the origin so a capture of a display sized to the
        // window needs no guesswork about compositor placement
        // (`musializer.c:605-607`).
        rl.set_window_position(0, 0);
    }

    let audio = RaylibAudio::init_audio_device()
        .map_err(|error| format!("could not initialize the audio device: {error}"))?;

    // After the window, because the atlas is a GPU upload. Never fails: a face
    // that will not rasterize falls back to raylib's default and says so.
    let fonts = Faces::load(&mut rl, &thread);

    let mut renderer = scene_host::SceneRenderer::load(&mut rl, &thread)?;

    audio_bridge::install(audio_bridge::DEFAULT_CAPACITY)
        .map_err(|error| format!("could not install the audio bridge: {error}"))?;

    // Starts in the oracle's no-track configuration and is reconfigured from the
    // file's sample rate once a track loads, mirroring `analyzer_configure`.
    // 200 KiB of arrays, so it is boxed.
    let mut analyzer = AudioAnalyzer::boxed(AudioAnalyzerConfig::idle())
        .map_err(|error| format!("could not create the analyzer: {error}"))?;

    let mut app = App {
        scene: SceneInstance::new(
            scene_host::descriptor(SceneId::Spectrum),
            DEFAULT_SCENE_SEED,
        ),
        settings: SceneSettings::default(),
        routes: RouteTable::new(),
        shell: Shell::new(),
        track: None,
    };

    // Step 3: the argv actions, left to right.
    let mut audio_path: Option<PathBuf> = None;
    for action in std::mem::take(&mut options.actions) {
        match action {
            Action::Mute => audio.set_master_volume(0.0),
            Action::SelectScene(id) => app.select_scene(id),
            Action::AsciiImage(path) => {
                app.select_scene(SceneId::AsciiField);
                unimplemented_action(
                    &mut options,
                    &mut app,
                    "--ascii-image",
                    &format!(
                        "{} was not imported: ASCII glyph import is Agent C's",
                        path.display()
                    ),
                );
            }
            Action::RecordEvent(_) => unimplemented_action(
                &mut options,
                &mut app,
                "--event",
                "the manual event lane needs Agent B's event timeline",
            ),
            Action::OpenProject(path) => unimplemented_action(
                &mut options,
                &mut app,
                "--project",
                &format!(
                    "{} was not opened: the .musi model is Agent B's",
                    path.display()
                ),
            ),
            // The last input wins, exactly as in the C.
            Action::LoadTrack(path) => audio_path = Some(path),
        }
    }

    // Step 4: routes, deferred until every input is resolved.
    for route in std::mem::take(&mut options.routes) {
        let Some((scene, _index, _descriptor)) = settings::descriptor_by_key(&route.parameter)
        else {
            continue;
        };
        if let Err(error) = app.routes.add(scene, route) {
            eprintln!("warning: could not add command-line route: {error:?}");
            options.error = true;
        }
    }

    // Render configuration, validated where the C validates it: the flag stores
    // the quality name and `plug_configure_render` is what rejects it
    // (`musializer.c:563-569`, `plug.c:7157-7160`). The C also only validates at
    // all when one of width/fps/quality was given, and only when no earlier error
    // has already poisoned the stage.
    let mut quality = None;
    if !options.error
        && (options.resolution.is_some() || options.fps.is_some() || options.quality.is_some())
    {
        match options.quality.as_deref().map(cli::Quality::from_name) {
            Some(None) => {
                eprintln!(
                    "warning: invalid render configuration; quality is balanced, high, or master"
                );
                options.error = true;
            }
            Some(Some(named)) => quality = Some(named),
            None => {}
        }
    }

    // Step 5: the stages the rewrite has not built.
    if options.render.is_some() {
        unimplemented_action(
            &mut options,
            &mut app,
            "--render",
            "FFmpeg export supervision is Agent E's",
        );
    }
    if options.save_project.is_some() {
        unimplemented_action(
            &mut options,
            &mut app,
            "--save-project",
            "transactional .musi saving is Agent B's",
        );
    }
    if options.analysis_bridge.is_some() {
        unimplemented_action(
            &mut options,
            &mut app,
            "--analysis-bridge",
            "the analysis bridge importer is Agent B's",
        );
    }
    if options.reload_once {
        unimplemented_action(
            &mut options,
            &mut app,
            "--reload-once",
            "hot reload is an explicit first-pass non-goal",
        );
    }
    if let Some(quality) = quality {
        // Parsed and reported, so a script can see its value was understood even
        // though the encoder that would honour it is Agent E's.
        eprintln!("note: render quality {} parsed but unused", quality.name());
    }
    if let Some(probe) = options.ui_probe.as_ref() {
        // Only the parts of the probe that have a surface to open are honoured;
        // the rest is reported rather than silently ignored, because a capture
        // script that photographs the wrong state is the failure this flag exists
        // to prevent (`musializer.c:128-130`).
        app.shell.panel = probe.panel;
        app.shell.fullscreen = probe.fullscreen;
        if probe.panel == cli::UiPanel::Tune {
            app.shell.inspector_open = true;
        }
        if probe.assist_confirmation
            || probe.lyric_selection.is_some()
            || probe.caption_style_pane
            || probe.font_browser.is_some()
            || probe.lyrics_reference_path.is_some()
        {
            unimplemented_action(
                &mut options,
                &mut app,
                "--ui-probe",
                "assist=, lyric=, style=, fonts= and lyrics-file= need panels that are still stubs",
            );
        }
    }

    // Interleaved stereo scratch, drained from the ring each frame. Sized for a
    // long frame at 44.1 kHz so a hitch does not silently discard audio. Declared
    // before the first track load because `open_track` drains through it.
    let mut scratch = vec![0.0f32; 4096 * audio_bridge::MIXED_CHANNELS];

    // The Music must be dropped — and the processor detached — while the audio
    // device and window are still alive. Getting that order wrong is one of the
    // plan's named traps, which is why every transition goes through
    // `open_track`/`close_track` rather than being written out at each site.
    let mut music: Option<Music<'_>> = None;
    if let Some(path) = audio_path.as_ref() {
        if let Err(error) = open_track(
            &audio,
            path,
            &mut analyzer,
            &mut music,
            &mut app,
            &mut scratch,
            // `--ui-probe play=0` photographs a paused track. `is_some_and` rather
            // than `is_none_or`, which needs a newer Rust than this workspace's MSRV.
            !options
                .ui_probe
                .as_ref()
                .is_some_and(|probe| !probe.playing),
        ) {
            // The C's `Could not load command-line track` (`:548`).
            eprintln!("warning: could not load {}: {error}", path.display());
            options.error = true;
        }
    }

    if let Some(probe) = options.ui_probe.as_ref() {
        // "a panel or seek probe needs a loaded, seekable track"
        // (`musializer.c:609-611`).
        if music.is_none() && (probe.panel != cli::UiPanel::None || probe.seek_seconds.is_some()) {
            eprintln!(
                "warning: could not apply --ui-probe state; a panel or seek probe needs a loaded, seekable track"
            );
            options.error = true;
        }
        if let (Some(music), Some(seconds)) = (music.as_ref(), probe.seek_seconds) {
            music.seek_stream(seconds as f32);
        }
        if let (Some(music), Some(zoom)) = (music.as_ref(), probe.timeline_zoom) {
            let duration = f64::from(music.get_time_length());
            app.shell.timeline.reset(duration);
            app.shell
                .timeline
                .zoom(duration, zoom, f64::from(music.get_time_played()));
        }
    }

    let mut report = Report::default();

    // `--save-project` without `--render` skips the main loop entirely
    // (`musializer.c:617`, `:637`). The save itself is Agent B's and already
    // reported above; honouring the skip keeps the exit path identical.
    let mut running = !options.exit_after_save();
    while running && !rl.window_should_close() {
        if let Some(music) = music.as_ref() {
            music.update_stream();
        }

        let drained = audio_bridge::drain_interleaved(&mut scratch);
        if drained > 0 {
            let consumed =
                analyzer.push_interleaved(&scratch[..drained * audio_bridge::MIXED_CHANNELS]);
            report.consumed_frames += consumed as u64;
        }

        // The scene clock is the frame delta, which is what preview and export
        // must agree on.
        let delta = rl.get_frame_time();
        if analyzer.analyze(delta) {
            report.analyzed_frames += 1;
        }

        let time_seconds = music
            .as_ref()
            .map_or(0.0, |m| f64::from(m.get_time_played()));
        let duration_seconds = music
            .as_ref()
            .map_or(0.0, |m| f64::from(m.get_time_length()));
        let playing = music.as_ref().is_some_and(|m| m.is_stream_playing());

        let spectrum = analyzer.spectrum();
        let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        report.peak_seen = report.peak_seen.max(audio_frame.peak);
        audio_frame.beat_phase = 0.0; // Agent A's beat tracker lands here.

        // Which band is loudest. Reported so a headless check can assert that the
        // spectrum *moved* rather than that it merely drew something: with a swept
        // fixture, a peak band that never changes means the analyzer is stuck.
        if let Some(band) = (0..spectrum.band_count())
            .max_by(|&a, &b| spectrum.smooth[a].total_cmp(&spectrum.smooth[b]))
        {
            report.observe_peak_band(band);
        }

        // Routes are evaluated into a staged copy, and the frame reads the staged
        // copy. Preview and export come through the same path, which is what keeps
        // routed parameters identical between them (`plug.c:1147-1166`).
        let sources = RouteSources::from_audio(&audio_frame);
        let routed = app.routes.apply(app.scene.id(), &sources, &app.settings);
        let effective = routed.as_ref().unwrap_or(&app.settings);

        let frame = SceneFrame {
            time_seconds,
            duration_seconds,
            delta_seconds: delta,
            frame_index: report.frames,
            audio: audio_frame,
            settings: effective,
            ..SceneFrame::idle(effective)
        };
        app.scene.update(&frame);

        let band_centre_hz = spectrum
            .band_first_bin
            .get(report.peak_band_last)
            .map_or(0.0, |&bin| analyzer.bin_frequency(bin as usize));

        let shell_input = ShellInput {
            window: (rl.get_screen_width() as f32, rl.get_screen_height() as f32),
            fonts: &fonts,
            scene: app.scene.id(),
            settings: &app.settings,
            routed: routed.as_ref(),
            time_seconds,
            duration_seconds,
            playing,
            track_name: app.track.as_deref(),
            band_count: spectrum.band_count(),
            peak_band: report.peak_band_last,
            rms: frame.audio.rms,
        };
        // With no track open the C draws the welcome screen instead of the
        // workspace (`preview_screen`, `plug.c:7769`), so the workspace frame is
        // not even computed on that path — there is no preview to lay out around.
        let commands;
        if app.track.is_none() {
            let mut d = rl.begin_drawing(&thread);
            d.clear_background(ui::theme::color::ui_surface());
            commands = app.shell.draw_welcome(&mut d, &shell_input);
        } else {
            let layout = app.shell.layout(&shell_input);

            // Scoped so the draw handle — and with it the frame — is dropped
            // before the commands run: a command may pause the stream or rebind
            // the scene, and neither should happen inside a begin/end drawing
            // pair.
            let mut d = rl.begin_drawing(&thread);
            // COLOR_BACKGROUND, from the palette rather than a literal, so the
            // contrast checks see the same number the window is cleared with.
            d.clear_background(ui::theme::color::background());

            // Scene first, chrome over it, and the scene clipped to its own
            // rectangle so a scene that draws past its boundary cannot paint over
            // a panel (`plug.c:7712-7716`).
            if !layout.preview.is_empty() {
                let preview = ui::widgets::rectangle(layout.preview);
                let mut scissor = d.begin_scissor_mode(
                    preview.x as i32,
                    preview.y as i32,
                    preview.width as i32,
                    preview.height as i32,
                );
                renderer.draw(&mut scissor, &fonts, &app.scene, &frame, preview, 1.0);
            }

            commands = app.shell.draw(&mut d, &layout, &shell_input);

            // A one-line readout, so a headless capture carries its own evidence
            // rather than needing a separate log to be trusted.
            //
            // Two things about it are corrections a capture forced, and no test
            // would have: it used to be drawn at the window origin, where the
            // tracks panel covered its left half, and it used to run past the
            // preview's right edge into the tuning inspector. It now starts inside
            // the preview and is clipped to it, and it drops to a short form when
            // the long form will not fit — a readout that overwrites a panel is
            // worse than a shorter readout.
            let readout = if layout.preview.width >= 700.0 {
                format!(
                    "frame {}  scene={}  t={time_seconds:.2}s  bands={}  peak band={} ({band_centre_hz:.0} Hz)  rms={:.3}  audio frames={}",
                    report.frames,
                    app.scene.id().stable_name(),
                    spectrum.band_count(),
                    report.peak_band_last,
                    frame.audio.rms,
                    report.consumed_frames,
                )
            } else {
                format!(
                    "{}  t={time_seconds:.2}s  peak {} ({band_centre_hz:.0} Hz)",
                    app.scene.id().stable_name(),
                    report.peak_band_last,
                )
            };
            // Drawn with the interface face like everything else, rather than
            // raylib's default: this line is the evidence a capture is read for,
            // and it was the last thing on screen still rendering in the bitmap
            // face.
            if layout.preview.is_empty() {
                ui::widgets::draw_text(
                    &mut d,
                    fonts.ui(),
                    &readout,
                    12.0,
                    12.0,
                    18.0,
                    Color::RAYWHITE,
                );
            } else {
                let preview = ui::widgets::rectangle(layout.preview);
                let mut scissor = d.begin_scissor_mode(
                    preview.x as i32,
                    preview.y as i32,
                    preview.width as i32,
                    preview.height as i32,
                );
                ui::widgets::draw_text(
                    &mut scissor,
                    fonts.ui(),
                    &readout,
                    preview.x + 12.0,
                    preview.y + 12.0,
                    18.0,
                    Color::RAYWHITE,
                );
            }
        }

        for command in commands {
            match command {
                ShellCommand::TogglePlay => {
                    if let Some(music) = music.as_ref() {
                        if music.is_stream_playing() {
                            music.pause_stream();
                        } else {
                            music.resume_stream();
                        }
                    }
                }
                ShellCommand::Seek(seconds) => {
                    if let Some(music) = music.as_ref() {
                        music.seek_stream(seconds as f32);
                    }
                }
                ShellCommand::SelectScene(id) => app.select_scene(id),
                ShellCommand::SetSetting {
                    scene,
                    index,
                    value,
                } => {
                    // `set` refuses a value the descriptor rejects, so a bad
                    // slider cannot smuggle one past the bounds.
                    app.settings.set(scene, index, value);
                }
                ShellCommand::ResetScene(scene) => app.settings.reset_scene(scene),
                ShellCommand::LoadTrack(path) => {
                    // Drop-to-open, which the welcome screen promises in so many
                    // words. It used to answer "restart with…", because the loop
                    // held the Music by shared reference and could not replace it;
                    // `open_track` owns that transition now.
                    if let Err(error) = open_track(
                        &audio,
                        &path,
                        &mut analyzer,
                        &mut music,
                        &mut app,
                        &mut scratch,
                        true,
                    ) {
                        app.shell.notify(
                            Severity::Error,
                            "Audio could not be loaded",
                            &format!(
                                "{}: {error}",
                                path.file_name().map_or_else(
                                    || path.display().to_string(),
                                    |name| name.to_string_lossy().into_owned()
                                )
                            ),
                        );
                    }
                }
                ShellCommand::OpenAudio => {
                    open_audio_dialog(&audio, &mut analyzer, &mut music, &mut app, &mut scratch)
                }
                ShellCommand::OpenProject => {
                    // The picker would work; what is behind it does not. Saying so
                    // beats opening a dialog whose result is then discarded.
                    app.shell.notify(
                        Severity::Info,
                        "Not built yet",
                        "Opening a .musi project needs the project model, which is Agent B's.",
                    );
                }
                ShellCommand::NotImplemented(what) => {
                    app.shell.notify(
                        Severity::Info,
                        "Not built yet",
                        &format!("{what} is still a stub in the Rust rewrite."),
                    );
                }
            }
        }

        report.frames += 1;

        // `--probe-reopen`: swap tracks halfway through the run, so a headless
        // check exercises detach/drop/drain/rebind/reattach rather than only the
        // fresh-load path. Taken rather than borrowed, so it happens exactly once.
        if let Some(limit) = options.probe_frames {
            if report.frames == u64::from(limit) / 2 {
                if let Some(path) = options.probe_reopen.take() {
                    report.reopened = Some(
                        match open_track(
                            &audio,
                            &path,
                            &mut analyzer,
                            &mut music,
                            &mut app,
                            &mut scratch,
                            true,
                        ) {
                            Ok(()) => Reopen::Ok {
                                frame: report.frames,
                                consumed_before: report.consumed_frames,
                            },
                            Err(error) => Reopen::Failed(error),
                        },
                    );
                }
            }
        }
        if let Some(limit) = options.probe_frames {
            if report.frames >= u64::from(limit) {
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
                    if !Path::new(path_str).is_file() {
                        return Err(format!("could not write {path_str}"));
                    }
                }
                running = false;
            }
        }
    }

    // Detach before the Music drops, so raylib's per-stream processor list never
    // holds a callback for a freed stream. The same function every track switch
    // goes through, so there is one place where that order is written down.
    close_track(&mut music, &mut app, &mut scratch);

    report.print(
        raylib_version,
        &app,
        analyzer.band_count(),
        requested_scene,
        &fonts,
    );

    // `exit_status = command_line_error ? 1 : 0` (`musializer.c:618`).
    Ok(if options.exit_status() == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    })
}

/// The application's own state, such as it is.
///
/// Deliberately small and flat: `RenderController`, `AssistController` and the
/// track list all land here later, and the shape that survives is the one where
/// each is a field with a narrow `&mut` rather than a shared cell.
struct App {
    scene: SceneInstance,
    /// The editable values. Routes are applied into a staged copy per frame, never
    /// written back, so a routed parameter does not become an edit.
    settings: SceneSettings,
    routes: RouteTable,
    shell: Shell,
    track: Option<String>,
}

impl App {
    fn select_scene(&mut self, id: SceneId) {
        if self.scene.id() == id {
            return;
        }
        self.scene = SceneInstance::new(scene_host::descriptor(id), DEFAULT_SCENE_SEED);
    }
}

/// Reports a stage the rewrite has not built, on stderr and in the tray, and
/// fails the exit status.
///
/// Failing is the point. Silently ignoring `--render` would let a script believe
/// it produced a video, which is worse than an error.
fn unimplemented_action(options: &mut Cli, app: &mut App, flag: &str, detail: &str) {
    eprintln!("warning: {flag} is not implemented yet: {detail}");
    app.shell.notify(
        Severity::Warning,
        &format!("{flag} is not implemented"),
        detail,
    );
    options.error = true;
}

/// Binds a track, replacing whatever was open.
///
/// The order here is the whole reason this is one function rather than code at
/// each call site. Detach before the old `Music` drops, or raylib's per-stream
/// processor list holds a callback for a freed stream; drain the ring after the
/// detach, or the first frames of the new track are analysed together with the
/// tail of the old one; and rebind the analyzer from the *file's* sample rate,
/// because that is what `start_preview_track` does (`plug.c:658-660`) and reading
/// the device's rate instead shifts every band.
fn open_track<'audio>(
    audio: &'audio RaylibAudio,
    path: &Path,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<(), String> {
    let path_str = path.to_str().ok_or("audio path is not valid UTF-8")?;
    // Loaded before the old track is torn down, so a failure leaves the session
    // exactly as it was rather than closing what was playing.
    let opened = audio
        .new_music(path_str)
        .map_err(|error| error.to_string())?;

    close_track(music, app, scratch);

    let file_sample_rate = opened.stream.sampleRate;
    *analyzer = *AudioAnalyzer::boxed(AudioAnalyzerConfig::preview(file_sample_rate))
        .map_err(|error| format!("could not configure the analyzer: {error}"))?;
    if play {
        opened.play_stream();
    }
    // SAFETY: the bridge is installed in `run` before this can be called, the
    // audio device is initialized (it is what produced `opened`), and the stream
    // outlives the attachment because `close_track` — reached from the next
    // `open_track` and from `run`'s shutdown — detaches before dropping it.
    unsafe { audio_bridge::attach(opened.stream) }
        .map_err(|error| format!("could not attach the audio bridge: {error}"))?;

    app.track = Some(track_name(path));
    app.shell
        .timeline
        .reset(f64::from(opened.get_time_length()));
    *music = Some(opened);
    Ok(())
}

/// Detaches and drops the current track, leaving no stale samples behind.
///
/// Draining the ring is not tidiness. The ring is lock-free SPSC and nothing
/// produces into it once the processor is detached, so this is the only moment a
/// consumer may safely empty it — and the samples in it belong to a track that is
/// about to stop existing.
fn close_track(music: &mut Option<Music<'_>>, app: &mut App, scratch: &mut [f32]) {
    let Some(open) = music.take() else {
        return;
    };
    // SAFETY: the same stream `open_track` passed to `attach`, still alive here
    // because it is dropped at the end of this function and not before.
    unsafe { audio_bridge::detach(open.stream) };
    drop(open);
    while audio_bridge::drain_interleaved(scratch) > 0 {}
    app.track = None;
    app.shell.timeline.reset(0.0);
}

/// Asks for an audio file and opens it, reporting either failure in the tray.
///
/// Called from outside the drawing pair: the picker is modal and blocks until the
/// user answers.
fn open_audio_dialog<'audio>(
    audio: &'audio RaylibAudio,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
) {
    let dialog = FileDialog::new("Open audio").with_filter(dialogs::filters::AUDIO);
    match dialog.pick_file() {
        // Cancellation. Deliberately silent: the user changed their mind, and a
        // notice saying so would be noise.
        Ok(None) => {}
        Ok(Some(path)) => {
            if let Err(error) = open_track(audio, &path, analyzer, music, app, scratch, true) {
                app.shell.notify(
                    Severity::Error,
                    "Audio could not be loaded",
                    // The C's wording (`plug.c:7797-7798`), plus the reason.
                    &format!("The file is unsupported, corrupt, or unreadable: {error}"),
                );
            }
        }
        Err(error) => app.shell.notify(
            Severity::Warning,
            "No file picker is available",
            &format!("{error}. Drop a file on the window, or pass it on the command line."),
        ),
    }
}

fn track_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The slice report. `tools/headless_check.sh` reads this, and its shape is the
/// reason the check can distinguish "drew something" from "tracked the input".
#[derive(Debug)]
struct Report {
    frames: u64,
    consumed_frames: u64,
    analyzed_frames: u64,
    peak_seen: f32,
    peak_band_last: usize,
    peak_band_low: usize,
    peak_band_high: usize,
    reopened: Option<Reopen>,
}

/// What `--probe-reopen` did, so the report can say whether the swap worked.
///
/// The interesting number is `consumed_before`: subtracting it from the final
/// count says how many audio frames arrived through the *second* attachment. A
/// swap that silently left the bridge detached would still exit 0 and still draw,
/// and only this difference would show it.
#[derive(Debug)]
enum Reopen {
    Ok { frame: u64, consumed_before: u64 },
    Failed(String),
}

impl Default for Report {
    fn default() -> Self {
        Self {
            frames: 0,
            consumed_frames: 0,
            analyzed_frames: 0,
            peak_seen: 0.0,
            peak_band_last: 0,
            peak_band_low: usize::MAX,
            peak_band_high: 0,
            reopened: None,
        }
    }
}

impl Report {
    fn observe_peak_band(&mut self, band: usize) {
        self.peak_band_last = band;
        self.peak_band_low = self.peak_band_low.min(band);
        self.peak_band_high = self.peak_band_high.max(band);
    }

    fn print(
        &self,
        raylib_version: &str,
        app: &App,
        band_count: usize,
        requested_scene: Option<SceneId>,
        fonts: &Faces,
    ) {
        let dropped = audio_bridge::ring().map_or(0, |ring| ring.dropped());
        println!("--- slice report ---");
        println!("verified:        window opened, audio device initialized, clean shutdown");
        println!("raylib:          {raylib_version}");
        // Evidence, not assertion. A silent fall back to raylib's 10 px bitmap
        // face is the kind of regression that gets noticed by eye weeks later, and
        // `tools/headless_check.sh` greps this line.
        println!("fonts:           {}", fonts.describe());
        println!("frames rendered: {}", self.frames);
        println!("analyzer runs:   {}", self.analyzed_frames);
        println!(
            "audio frames:    {} consumed, {dropped} dropped",
            self.consumed_frames
        );
        println!("bands:           {band_count}");
        println!("peak seen:       {:.4}", self.peak_seen);
        let moved = self.peak_band_low != usize::MAX && self.peak_band_high > self.peak_band_low;
        if self.peak_band_low == usize::MAX {
            println!("peak band:       never established");
        } else {
            println!(
                "peak band:       {}..{} (last {})",
                self.peak_band_low, self.peak_band_high, self.peak_band_last
            );
        }
        println!("dropped frames:  {dropped}");
        // The scene lines are what a `--scene` check reads: the name that was
        // actually bound, and whether its drawing half is real or a placeholder.
        let scene = app.scene.id();
        println!(
            "scene:           {} ({})",
            scene.stable_name(),
            scene.display_name()
        );
        println!(
            "scene drawing:   {}",
            if scene_host::drawing_is_ported(scene) {
                "ported"
            } else {
                "placeholder"
            }
        );
        println!("routes:          {}", app.routes.scene(scene).len());
        println!("panel:           {}", app.shell.panel.label());
        match &self.reopened {
            None => println!("reopen:          not requested"),
            Some(Reopen::Failed(error)) => println!("reopen:          FAILED: {error}"),
            Some(Reopen::Ok {
                frame,
                consumed_before,
            }) => println!(
                "reopen:          ok at frame {frame}; {} audio frames through the new stream",
                self.consumed_frames.saturating_sub(*consumed_before)
            ),
        }
        // Evidence, not assertion: `--scene loom` that parses and then leaves
        // Spectrum bound would otherwise exit 0 and look fine.
        match requested_scene {
            Some(requested) if requested == scene => {
                println!("scene request:   honoured ({})", requested.stable_name());
            }
            Some(requested) => println!(
                "scene request:   MISMATCH: asked for {}, bound {}",
                requested.stable_name(),
                scene.stable_name()
            ),
            None => println!("scene request:   none (default)"),
        }
        // Three separate claims, because "it compiled" and "it drew something" are
        // both weaker than the gate. A swept fixture that leaves the peak band
        // stationary means the analyzer is stuck even though the picture looks
        // fine.
        println!(
            "verdict:         {}",
            match (self.consumed_frames > 0, self.peak_seen > 0.0, moved) {
                (true, true, true) =>
                    "audio advanced, the spectrum responded, and the peak band tracked the sweep",
                (true, true, false) =>
                    "audio advanced and bands were non-zero, but the peak band never moved",
                (true, false, _) => "audio advanced but the spectrum stayed flat",
                (false, _, _) => "no audio was consumed",
            }
        );
    }
}
