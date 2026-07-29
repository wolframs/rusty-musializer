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
use musializer_core::project::event_timeline::ManualEventAction;
use musializer_core::project::preset_store::{
    self, PresetAction, PresetLibrary, SharedPresetsView,
};
use musializer_core::scene::routes::{RouteSources, RouteTable};
use musializer_core::scene::settings;
use musializer_core::scene::{SceneAudioFrame, SceneFrame, SceneId, SceneInstance, SceneSettings};
use musializer_core::scenes::ascii_field::ascii_art;
use musializer_core::timing::render_export::{Quality as RenderQuality, RenderExportConfig};
use musializer_core::ui::notice::Severity;
use musializer_runtime::audio_bridge;
use musializer_runtime::font::Faces;
use musializer_runtime::preset_files;
use musializer_runtime::process::dialogs::{self, FileDialog};
use musializer_runtime::project_files;
use raylib::prelude::*;

mod cli;
mod project;
mod scene_host;
mod scenes;
mod ui;
mod workspace;

use cli::{Action, Cli, Outcome};
use ui::panels::assist::{AssistController, AssistEffect};
use ui::panels::export::ExportSession;
use ui::shell::{Shell, ShellCommand, ShellInput};
use workspace::{Track, Workspace};

/// Seed for a scene with no track to derive one from.
///
/// This is the C's own initial seed, `UINT64_C(0x4D555349414C495A)` — ASCII
/// `MUSIALIZ` — from `scene_instance_init` in `plug_init`
/// (`../musializer/src/plug.c:8401`). It matters because scene state is seeded
/// deterministically: a different constant here would give every freshly opened
/// track a different star field than the oracle's for the same audio, and
/// export determinism between the two implementations is worth having for free.
///
/// Per-track seeds come from [`Workspace::inherited_scene`]; this is only the
/// value the very first track inherits.
const DEFAULT_SCENE_SEED: u64 = 0x4D55_5349_414C_495A;

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
        workspace: Workspace::new(),
        pending_settings: SceneSettings::default(),
        pending_routes: RouteTable::new(),
        pending_ascii: None,
        shared_presets: PresetLibrary::new(),
        preset_store_path: None,
        presets_editable: false,
        preset_selection: 0,
        preset_delete_armed: false,
        shell: Shell::new(),
    };

    // The Assist supervisor. `find_assist_helper` probes relative to the
    // executable's directory, which is raylib's `GetApplicationDirectory()`.
    let application_directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut assist = AssistController::new(&application_directory);
    app.workspace.assist.helper_available = assist.helper_available();

    // The shared preset store, read once (`plug.c:8397-8410`). A store that
    // cannot be read is not fatal: the library stays empty and writes are
    // refused, so a recoverable file is never overwritten by an empty one.
    app.preset_store_path = preset_files::default_path();
    match app.preset_store_path.clone() {
        None => app.shell.notify(
            Severity::Error,
            "Shared presets are unavailable",
            "No location for the preset store could be derived from the environment.",
        ),
        Some(path) => match preset_files::load(&path) {
            Ok(library) => {
                app.shared_presets = library.unwrap_or_default();
                app.presets_editable = true;
            }
            Err(error) => app.shell.notify(
                Severity::Error,
                "Shared presets could not be read",
                &format!(
                    "{}: {error}. Saving is disabled so the file is not overwritten.",
                    path.display()
                ),
            ),
        },
    }

    // Step 3: the argv actions, left to right.
    let mut input: Option<Input> = None;
    for action in std::mem::take(&mut options.actions) {
        match action {
            Action::Mute => audio.set_master_volume(0.0),
            Action::SelectScene(id) => app.select_scene(id),
            // The scene is selected whether or not the import succeeds, which is
            // the oracle's order (`musializer.c:413-422` selects unconditionally
            // after reporting). A user who mistyped the filename still lands on the
            // scene they asked for, drawing its procedural mode.
            Action::AsciiImage(path) => {
                match import_ascii_image(&mut app, &path) {
                    Ok((columns, rows)) => println!(
                        "ascii: imported {} as {columns}x{rows} glyphs",
                        path.display()
                    ),
                    // A refused import is a failed exit status, not a warning
                    // buried in a log: `--ascii-image` is a scripted flag, and a
                    // script that gets 0 back has been told the image is on screen.
                    Err(detail) => {
                        eprintln!("warning: could not load command-line ASCII image: {detail}");
                        app.shell.notify(
                            Severity::Error,
                            "ASCII image could not be imported",
                            &detail,
                        );
                        options.error = true;
                    }
                }
                app.select_scene(SceneId::AsciiField);
            }
            // `plug_record_event` (`plug.c:1055-1069`). No track is open yet —
            // the actions run before an input is resolved — so this lands in the
            // workspace's pending lane and is handed to the first track that
            // opens (`plug.c:844-851`). The C reports a rejected record with the
            // same message as a parse failure (`musializer.c:423-431`).
            Action::RecordEvent(event) => {
                if app.workspace.record_event(event).is_err() {
                    eprintln!(
                        "warning: Invalid command-line event; expected type:seconds:id:value"
                    );
                    options.error = true;
                }
            }
            // The last input wins, exactly as in the C, and a project and an
            // audio file compete for the same slot — which is why they share one
            // variable rather than each having their own.
            Action::OpenProject(path) => input = Some(Input::Project(path)),
            Action::LoadTrack(path) => input = Some(Input::Audio(path)),
        }
    }

    // Step 4: routes, deferred until every input is resolved.
    for route in std::mem::take(&mut options.routes) {
        let Some((scene, _index, _descriptor)) = settings::descriptor_by_key(&route.parameter)
        else {
            continue;
        };
        if let Err(error) = app.routes_mut().add(scene, route) {
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
    if options.reload_once {
        unimplemented_action(
            &mut options,
            &mut app,
            "--reload-once",
            "hot reload is an explicit first-pass non-goal",
        );
    }
    let mut assist_probe_misplaced = false;
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
        // `assist=confirm` arms the review step (`plug.c:3807-3810`). It needs
        // `panel=assist`, because arming a step in a panel nobody can see would
        // photograph the wrong state.
        assist_probe_misplaced = probe.assist_confirmation && probe.panel != cli::UiPanel::Assist;
        if probe.assist_confirmation && !assist_probe_misplaced {
            app.workspace.assist.set_confirmation_pending(true);
        }
    }

    // Interleaved stereo scratch, drained from the ring each frame. Sized for a
    // long frame at 44.1 kHz so a hitch does not silently discard audio. Declared
    // before the first track load because `close_audio` drains through it.
    let mut scratch = vec![0.0f32; 4096 * audio_bridge::MIXED_CHANNELS];

    // The Music must be dropped — and the processor detached — while the audio
    // device and window are still alive. Getting that order wrong is one of the
    // plan's named traps, which is why every transition goes through
    // `bind_current_audio`/`close_audio` rather than being written out at each site.
    let mut music: Option<Music<'_>> = None;
    // `--ui-probe play=0` photographs a paused track. `is_some_and` rather than
    // `is_none_or`, which needs a newer Rust than this workspace's MSRV.
    let play = !options
        .ui_probe
        .as_ref()
        .is_some_and(|probe| !probe.playing);
    match input.as_ref() {
        None => {}
        Some(Input::Audio(path)) => {
            if let Err(error) = open_track(
                &audio,
                path,
                &mut analyzer,
                &mut music,
                &mut app,
                &mut scratch,
                play,
            ) {
                // The C's `Could not load command-line track` (`:548`).
                eprintln!("warning: could not load {}: {error}", path.display());
                options.error = true;
            }
        }
        Some(Input::Project(path)) => {
            if let Err(error) = open_project(
                &audio,
                path,
                &mut analyzer,
                &mut music,
                &mut app,
                &mut scratch,
            ) {
                // The C's `Could not load command-line project` (`musializer.c`).
                eprintln!("warning: could not open {}: {error}", path.display());
                options.error = true;
            } else if !play {
                if let Some(open) = music.as_ref() {
                    open.pause_stream();
                }
            }
        }
    }

    if assist_probe_misplaced {
        unimplemented_action(
            &mut options,
            &mut app,
            "--ui-probe",
            "assist=confirm needs panel=assist",
        );
    }

    // `--analysis-bridge`, after every input is resolved, exactly where the C
    // applies it (`musializer.c:579-586`). It applies rather than staging: a
    // batch entry point with no review step must not leave the result unapplied.
    if let Some(path) = options.analysis_bridge.clone() {
        match assist.import_bridge(&path, &mut app.workspace, 0.0) {
            Ok(notices) => {
                for notice in notices {
                    app.shell
                        .notify(notice.severity, &notice.title, &notice.detail);
                }
            }
            Err(error) => {
                eprintln!(
                    "warning: could not load command-line analysis bridge {}: {error}",
                    path.display()
                );
                options.error = true;
            }
        }
    }

    // `--ui-probe lyrics-file=` selects the sheet the next lyrics run will use
    // (`plug.c:3812-3823`).
    if let Some(path) = options
        .ui_probe
        .as_ref()
        .and_then(|probe| probe.lyrics_reference_path.clone())
    {
        if app.workspace.current().is_none() || !path.is_file() {
            eprintln!(
                "warning: could not apply --ui-probe lyrics-file={}; it needs a loaded track and an existing file",
                path.display()
            );
            options.error = true;
        } else {
            for notice in assist.set_lyric_sheet(&mut app.workspace, &path) {
                app.shell
                    .notify(notice.severity, &notice.title, &notice.detail);
            }
        }
    }

    // `--save-project`, after every input is resolved so it saves what the rest
    // of the command line actually produced (`musializer.c:571-577`).
    if let Some(destination) = options.save_project.clone() {
        match save_project_to(&mut app, music.as_ref(), &destination, false) {
            Ok(()) => println!("saved {}", destination.display()),
            Err(error) => {
                eprintln!("warning: could not save {}: {error}", destination.display());
                options.error = true;
            }
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
        // The lyrics editor's probe keys, which need the track they edit.
        if let Some(track) = app.workspace.current_mut() {
            let mut lyrics = std::mem::take(&mut app.shell.lyrics);
            let honoured = lyrics.apply_probe(probe, track);
            app.shell.lyrics = lyrics;
            if !honoured {
                eprintln!("warning: --ui-probe lyrics keys could not be applied");
                options.error = true;
            }
        }
        // Applied here with the rest of the probe, before the first frame, so the
        // evidence line is printed once rather than being guarded by a flag on the
        // draft. It needs the committed route, which needs the track.
        if let Some(key) = probe.route_editor.clone() {
            let slot = app.workspace.current_index().unwrap_or(0);
            let scene = app.scene.id();
            let committed = app
                .routes()
                .scene(scene)
                .items()
                .iter()
                .find(|mapping| mapping.parameter == key)
                .cloned();
            let line = app
                .shell
                .open_route_editor_probe(&key, scene, slot, committed.as_ref());
            println!("{line}");
        }
    }

    let mut report = Report::default();

    // `--save-project` without `--render` skips the main loop entirely
    // (`musializer.c:617`, `:637`). The save itself is Agent B's and already
    // reported above; honouring the skip keeps the exit path identical.
    // Render configuration onto the current track (`plug_configure_render`,
    // `plug.c:7145-7169`). Width and height move together, a zero means "leave
    // it", and an invalid result is refused whole rather than half-applied.
    if !options.error
        && (options.resolution.is_some() || options.fps.is_some() || quality.is_some())
    {
        let mut config = app
            .workspace
            .current()
            .map_or_else(RenderExportConfig::default, |track| track.render_config);
        if let Some((width, height)) = options.resolution {
            if width != 0 && height != 0 {
                config.width = width;
                config.height = height;
            }
        }
        if let Some(fps) = options.fps {
            if fps != 0 {
                config.fps = fps;
            }
        }
        if let Some(named) = quality {
            config.set_quality(match named {
                cli::Quality::Balanced => RenderQuality::Balanced,
                cli::Quality::High => RenderQuality::High,
                cli::Quality::Master => RenderQuality::Master,
            });
        }
        // The name the *command line* used, so a rejection names the flag value
        // the user typed rather than the enum it resolved to.
        let requested = quality.map_or("unchanged", cli::Quality::name);
        match config.validate() {
            Ok(()) => {
                if let Some(track) = app.workspace.current_mut() {
                    track.render_config = config;
                }
            }
            Err(error) => {
                eprintln!(
                    "warning: invalid render configuration ({}x{} at {} fps, quality {requested}): {error}",
                    config.width, config.height, config.fps
                );
                options.error = true;
            }
        }
    }

    let mut export: Option<ExportSession> = None;
    // `--render` runs the export and exits (`musializer.c:650`), where an export
    // started from the panel returns to the workspace.
    let exit_after_render = options.render.is_some();
    let mut running = !options.exit_after_save();
    if let Some(destination) = options.render.clone() {
        if options.error {
            // An earlier stage already failed; rendering on top of it would
            // produce a file the command line did not describe.
            running = false;
        } else {
            export = ExportSession::begin(
                &mut rl,
                &thread,
                &audio,
                music.as_ref(),
                &mut app,
                &mut analyzer,
                &destination,
                options.render_window,
            );
            if export.is_none() {
                running = false;
                options.error = true;
            }
        }
    }
    // Set once the close guard has warned with no dialog available; see
    // `confirm_close`.
    let mut close_warned = false;
    while running {
        // Exactly once per frame: raylib's `WindowShouldClose` clears the GLFW
        // flag as it reads it (`rcore_desktop_glfw.c`), which is what lets the
        // C's `WindowShouldClose() && plug_confirm_close()` refuse a quit without
        // re-asking every frame afterwards (`musializer.c:638`).
        if rl.window_should_close() && confirm_close(&mut app, export.is_some(), &mut close_warned)
        {
            break;
        }
        // An export replaces the frame loop while it runs: it owns the window,
        // the analyzer and the scene for the duration, and draws its own progress
        // screen (`musializer.c:641-651`).
        if export.is_some() {
            let finished = export.as_mut().expect("just checked").tick(
                &mut rl,
                &thread,
                music.as_ref(),
                &mut app,
                &mut analyzer,
                &mut renderer,
                &fonts,
            );
            if finished {
                export = None;
                if exit_after_render {
                    running = false;
                }
            }
            continue;
        }
        if let Some(music) = music.as_ref() {
            music.update_stream();
        }

        // The Assist supervisor, polled before anything is drawn so the panel
        // shows this frame's job state rather than last frame's.
        for notice in assist.poll(&mut app.workspace) {
            app.shell
                .notify(notice.severity, &notice.title, &notice.detail);
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
        // The track's decoded duration, not the stream's, because that is what
        // the C puts in the frame (`plug.c:1169`) and what every timeline and
        // export length is measured against. They agree, but only one of them
        // survives a track that is added and not yet playing.
        let duration_seconds = app
            .workspace
            .current()
            .map_or(0.0, |track| track.duration_seconds);
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
        // Copied out of `app` rather than borrowed from it. `settings()` and
        // `routes()` borrow the whole `App` (they choose between the current
        // track's tables and the pending ones), so holding either across
        // `app.scene.update` or `app.shell.draw` would deny those the `&mut` they
        // need. A `SceneSettings` is 480 bytes of `f32`, and `apply` already
        // produces a whole copy whenever a route fires.
        let base = *app.settings();
        let routed = app.routes().apply(app.scene.id(), &sources, &base);
        let effective = routed.as_ref().unwrap_or(&base);

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

        // Before the draw, and only for the scene that needs it
        // (`scene_render`, `plug.c:1313-1315`). Placed here rather than inside the
        // begin/end drawing pair because it pauses and resumes the audio stream,
        // which is not something to do mid-frame.
        if app.scene.id() == SceneId::SongAtlas {
            let _ = ensure_song_atlas_map(&audio, &mut app, music.as_ref());
        }

        // Borrowed from the current track for exactly this frame, which is what
        // stops one track's terrain or glyph grid from being drawn under another.
        let assets =
            app.workspace
                .current()
                .map_or_else(scene_host::TrackAssets::default, |track| {
                    scene_host::TrackAssets {
                        atlas_map: track.atlas_map(),
                        ascii_grid: track.ascii_grid(),
                    }
                });

        let shell_input = ShellInput {
            window: (rl.get_screen_width() as f32, rl.get_screen_height() as f32),
            fonts: &fonts,
            scene: app.scene.id(),
            settings: &base,
            routed: routed.as_ref(),
            time_seconds,
            duration_seconds,
            playing,
            workspace: &app.workspace,
            presets: SharedPresetsView {
                library: &app.shared_presets,
                selected: app.preset_selection,
                editable: app.presets_editable,
                delete_armed: app.preset_delete_armed,
            },
            route_sources: sources,
            band_count: spectrum.band_count(),
            peak_band: report.peak_band_last,
            rms: frame.audio.rms,
        };
        // With no track open the C draws the welcome screen instead of the
        // workspace (`preview_screen`, `plug.c:7769`), so the workspace frame is
        // not even computed on that path — there is no preview to lay out around.
        let commands;
        if app.workspace.current().is_none() {
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
                renderer.draw(
                    &mut scissor,
                    &fonts,
                    &app.scene,
                    &frame,
                    assets,
                    preview,
                    1.0,
                );
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

        // The lyrics editor writes through the track rather than through a
        // `ShellCommand` per keystroke: an edit is a whole cue operation and the
        // editor already validated it against the model's own rules.
        let edits = app.shell.lyrics.take_pending();
        if !edits.is_empty() {
            let now = rl.get_time();
            if let Some(track) = app.workspace.current_mut() {
                let mut failed = None;
                for edit in edits {
                    if let Err(error) = edit.apply(track) {
                        failed = Some(error);
                        break;
                    }
                }
                track.mark_dirty(now);
                if let Some(error) = failed {
                    app.shell.notify(
                        Severity::Warning,
                        "Lyric edit was refused",
                        &format!("{error:?}"),
                    );
                }
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
                    app.settings_mut().set(scene, index, value);
                }
                ShellCommand::ResetScene(scene) => app.settings_mut().reset_scene(scene),
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
                ShellCommand::SetRenderConfig(config) => {
                    if let Some(track) = app.workspace.current_mut() {
                        track.render_config = config;
                        track.mark_dirty(rl.get_time());
                    }
                }
                ShellCommand::StartRender => {
                    if let Some(destination) = ui::panels::export::ask_for_destination(&mut app) {
                        export = ExportSession::begin(
                            &mut rl,
                            &thread,
                            &audio,
                            music.as_ref(),
                            &mut app,
                            &mut analyzer,
                            &destination,
                            None,
                        );
                    }
                }
                ShellCommand::ManualEvent(action) => {
                    // The playhead this frame, not the timeline widget's view: a cue is
                    // recorded where the transport is (`plug.c:1979-2030`).
                    handle_manual_event(&mut app, action, time_seconds, rl.get_time());
                }
                ShellCommand::Preset(action) => {
                    handle_preset(&mut app, action, rl.get_time());
                }
                ShellCommand::ApplyRoute { scene, route } => {
                    // Add and replace are one command: the table keys by
                    // parameter, so committing over an existing route replaces it
                    // (`plug.c:5852`). A refusal is the editor's own validation
                    // having been bypassed, so it is reported rather than ignored.
                    let parameter = route.parameter.clone();
                    remove_route(&mut app, scene, &parameter);
                    if let Err(error) = app.routes_mut().add(scene, route) {
                        app.shell.notify(
                            Severity::Error,
                            "Route could not be applied",
                            &format!("{parameter}: {error}"),
                        );
                    } else {
                        mark_current_track_dirty(&mut app, rl.get_time());
                    }
                }
                ShellCommand::RemoveRoute { scene, parameter } => {
                    if remove_route(&mut app, scene, &parameter) {
                        mark_current_track_dirty(&mut app, rl.get_time());
                    }
                }
                ShellCommand::SelectTrack(index) => {
                    if let Err(error) = select_track(
                        &audio,
                        index,
                        &mut analyzer,
                        &mut music,
                        &mut app,
                        &mut scratch,
                    ) {
                        // The C's wording when a track switch cannot be prepared
                        // (`plug.c:5269-5271`): the previous track keeps playing,
                        // and saying so is the point of the notice.
                        app.shell.notify(
                            Severity::Error,
                            "Track could not be prepared",
                            &format!("The current track remains active: {error}"),
                        );
                    }
                }
                ShellCommand::OpenAudio => {
                    open_audio_dialog(&audio, &mut analyzer, &mut music, &mut app, &mut scratch)
                }
                ShellCommand::OpenProject => {
                    let dialog = FileDialog::new("Open Musializer project")
                        .with_filter(dialogs::filters::MUSIALIZER_PROJECT);
                    match dialog.pick_file() {
                        // Cancellation is deliberately silent.
                        Ok(None) => {}
                        Ok(Some(path)) => {
                            if let Err(error) = open_project(
                                &audio,
                                &path,
                                &mut analyzer,
                                &mut music,
                                &mut app,
                                &mut scratch,
                            ) {
                                app.shell.notify(
                                    Severity::Error,
                                    "Project could not be opened",
                                    &error,
                                );
                            }
                        }
                        Err(error) => app.shell.notify(
                            Severity::Warning,
                            "No file picker is available",
                            &format!("{error}. Pass --project on the command line instead."),
                        ),
                    }
                }
                ShellCommand::SaveProject => {
                    save_project_command(&mut app, music.as_ref(), true);
                }
                ShellCommand::SaveProjectAs => {
                    if let Some(destination) = ask_for_project_path(&mut app) {
                        match save_project_to(&mut app, music.as_ref(), &destination, false) {
                            Ok(()) => app.shell.notify(
                                Severity::Info,
                                "Project saved",
                                "Audio, ASCII imagery, lyrics, scenes, events, and output settings are durable.",
                            ),
                            Err(error) => app.shell.notify(
                                Severity::Error,
                                "Project could not be saved",
                                &error,
                            ),
                        }
                    }
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

        // The Assist panel's request, drained once the drawing pair has closed:
        // every one of these spawns, signals, reads a file or edits a track.
        if let Some(request) = app.workspace.assist.take_request() {
            let now = rl.get_time();
            let outcome = assist.handle(request, &mut app.workspace, now);
            for notice in outcome.notices {
                app.shell
                    .notify(notice.severity, &notice.title, &notice.detail);
            }
            match outcome.effect {
                None => {}
                Some(AssistEffect::Clipboard(path)) => rl.set_clipboard_text(&path).unwrap_or(()),
                Some(AssistEffect::ChooseLyricSheet) => {
                    let dialog = FileDialog::new("Choose authored lyrics")
                        .with_filter(dialogs::filters::LYRIC_TEXT);
                    match dialog.pick_file() {
                        // Cancellation is deliberately silent.
                        Ok(None) => {}
                        Ok(Some(path)) => {
                            for notice in assist.set_lyric_sheet(&mut app.workspace, &path) {
                                app.shell
                                    .notify(notice.severity, &notice.title, &notice.detail);
                            }
                        }
                        Err(error) => app.shell.notify(
                            Severity::Warning,
                            "No file picker is available",
                            &format!("{error}. Pass --ui-probe lyrics-file= instead."),
                        ),
                    }
                }
            }
        }

        report.frames += 1;

        // Autosave, polled after the frame like the C's (`plug.c:7580-7583`).
        // Every track, not only the current one, because a background track can
        // be dirty from a project open. `editor_dirty` is `false` until Agents G
        // and I have drafts to report.
        let now = rl.get_time();
        let due: Vec<usize> = (0..app.workspace.len())
            .filter(|&index| {
                app.workspace
                    .get(index)
                    .is_some_and(|track| project::autosave_is_due(track, now, false))
            })
            .collect();
        for index in due {
            // Only the current track has a bound stream to read the sample rate
            // from, which is the same reason `save_project_to` needs one.
            if app.workspace.current_index() != Some(index) {
                continue;
            }
            if let Some(path) = app
                .workspace
                .get(index)
                .and_then(|track| track.project_path.clone())
            {
                // A failure sets `project_autosave_failed`, which stops the retry
                // until the next edit clears it — so this cannot become a loop
                // that writes every frame.
                let _ = save_project_to(&mut app, music.as_ref(), &path, true);
            }
        }

        // `--probe-reopen`: swap tracks halfway through the run, so a headless
        // check exercises detach/drop/drain/rebind/reattach rather than only the
        // fresh-load path. Taken rather than borrowed, so it happens exactly once.
        if let Some(limit) = options.probe_frames {
            if report.frames == u64::from(limit) / 2 {
                if let Some(path) = options.probe_reopen.take() {
                    // Add *and* select, because adding alone no longer rebinds:
                    // the oracle only auto-plays the first track (`plug.c:843`).
                    // Selecting is what exercises detach/drop/drain/rebind, which
                    // is the whole point of this probe.
                    let swapped = open_track(
                        &audio,
                        &path,
                        &mut analyzer,
                        &mut music,
                        &mut app,
                        &mut scratch,
                        true,
                    )
                    .and_then(|index| {
                        select_track(
                            &audio,
                            index,
                            &mut analyzer,
                            &mut music,
                            &mut app,
                            &mut scratch,
                        )
                    });
                    report.reopened = Some(match swapped {
                        Ok(()) => Reopen::Ok {
                            frame: report.frames,
                            consumed_before: report.consumed_frames,
                        },
                        Err(error) => Reopen::Failed(error),
                    });
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
    // Before the window goes, so a helper tree is never orphaned by an exit.
    if !assist.shutdown() {
        eprintln!("warning: the Assist helper could not be reaped promptly");
    }
    close_audio(&mut music, &mut app, &mut scratch);

    report.print(
        raylib_version,
        &app,
        analyzer.band_count(),
        requested_scene,
        &fonts,
        &assist,
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
/// Deliberately small and flat: `RenderController` and `AssistController` land
/// here later, and the shape that survives is the one where each is a field with
/// a narrow `&mut` rather than a shared cell.
///
/// The settings/routes pair here is the C's *pending* pair, not a second copy of
/// the track's. `p->scene_settings` and `p->pending_scene_routes` are what the
/// oracle edits and evaluates when no track is open, and the pending routes are
/// handed to the first track that loads (`plug.c:852-853`). Everything else lives
/// on the [`Track`], because every close guard, dirty flag and editor draft in
/// the C is per-track.
struct App {
    scene: SceneInstance,
    workspace: Workspace,
    /// `p->scene_settings` (`plug.c:1026`): the editable values with no track.
    pending_settings: SceneSettings,
    /// `p->pending_scene_routes` (`plug.c:282`).
    pending_routes: RouteTable,
    /// `p->ascii_cells` and its three companions (`plug.c:274-278`): an image
    /// imported before any track existed, waiting for one to belong to.
    ///
    /// Unlike [`Self::pending_routes`], this is claimed by **whichever** track opens
    /// next, not only by the first one (`plug.c:825-839` runs for every new track,
    /// where the route handoff at `:852` is inside a `current_track() == NULL`
    /// guard). Both are cleared as they are handed over, so nothing is inherited
    /// twice.
    pending_ascii: Option<workspace::AsciiImage>,
    /// The shared per-user preset library (`p->shared_presets`, `plug.c:265`).
    ///
    /// Distinct from a track's own presets, which are project data written into
    /// the `.musi`. This one is a per-user file, read once at startup.
    shared_presets: PresetLibrary,
    /// Where that file lives, or `None` when no location could be derived.
    preset_store_path: Option<PathBuf>,
    /// False when the store was rejected at startup. Mutations are then refused
    /// rather than overwriting a file that might still be recoverable
    /// (`shared_presets_editable`, `plug.c:4200-4209`).
    presets_editable: bool,
    /// The selected preset within the active scene, and whether `Delete` is
    /// armed for it. Disarmed whenever either moves, which is `plug.c:5983-5987`
    /// — an armed Delete that survived paging would remove something the user
    /// never armed.
    preset_selection: usize,
    preset_delete_armed: bool,
    shell: Shell,
}

impl App {
    /// `track_effective_scene_settings` (`plug.c:1024-1028`).
    ///
    /// Note that this is the *edit* target as well as the read target: the C
    /// returns a non-const pointer and the tuning inspector writes through it. A
    /// track playing a scene-switch cue edits its playback copy, so the cue can
    /// drive a parameter without the change surviving the cue.
    fn settings(&self) -> &SceneSettings {
        match self.workspace.current() {
            None => &self.pending_settings,
            Some(track) if track.cue_settings_active => &track.playback_scene_settings,
            Some(track) => &track.scene_settings,
        }
    }

    fn settings_mut(&mut self) -> &mut SceneSettings {
        match self.workspace.current_mut() {
            None => &mut self.pending_settings,
            Some(track) if track.cue_settings_active => &mut track.playback_scene_settings,
            Some(track) => &mut track.scene_settings,
        }
    }

    /// `plug_add_scene_route`'s table choice (`plug.c:1077-1080`).
    fn routes(&self) -> &RouteTable {
        match self.workspace.current() {
            Some(track) => &track.scene_routes,
            None => &self.pending_routes,
        }
    }

    fn routes_mut(&mut self) -> &mut RouteTable {
        match self.workspace.current_mut() {
            Some(track) => &mut track.scene_routes,
            None => &mut self.pending_routes,
        }
    }

    /// Binds a scene, seeded from the current track (`scene_seed_for_track`,
    /// `plug.c:611-614`; used at `:987` and `:1354`).
    fn select_scene(&mut self, id: SceneId) {
        if self.scene.id() == id {
            return;
        }
        let seed = self
            .workspace
            .current()
            .map_or(DEFAULT_SCENE_SEED, |track| track.scene_seed);
        self.scene = SceneInstance::new(scene_host::descriptor(id), seed);
        // The track remembers what it is showing, so reselecting it later
        // restores this scene rather than the one the previous track left behind
        // (`plug.c:5265-5268`).
        if let Some(track) = self.workspace.current_mut() {
            track.previous_base_scene = track.base_scene;
            track.base_scene = id;
        }
    }
}

/// Builds the timeline's waveform envelope for a track (`load_timeline_waveform`,
/// `plug.c:688-709`).
///
/// **Eager, at track load**, because the oracle is: the strip has to draw an
/// envelope from the first frame the track is visible, and there is no later
/// moment that is not mid-frame. This is the whole-track decode the oracle already
/// pays for at load, and Song Atlas's is deliberately *not* folded into it — see
/// [`ensure_song_atlas_map`].
///
/// A file that will not decode leaves the envelope `None` and warns, which is what
/// the C does. The strip then says "Waveform unavailable" rather than drawing a
/// flat line that would read as silence.
fn load_timeline_waveform(audio: &RaylibAudio, track: &mut Track) {
    track.timeline_waveform = None;
    let Some(decoded) = musializer_runtime::decode::whole_track(audio, &track.file_path) else {
        eprintln!(
            "warning: waveform preview could not decode {}",
            track.file_path.display()
        );
        return;
    };
    let waveform = musializer_core::timing::track_timeline::Waveform::build(
        &decoded.samples,
        decoded.channels,
        musializer_core::timing::track_timeline::MAX_BINS,
    );
    if !waveform.is_empty() {
        track.timeline_waveform = Some(waveform);
    }
}

/// Builds the current track's whole-song terrain if it is needed and not yet built
/// (`ensure_song_atlas_map`, `plug.c:712-737`).
///
/// # Why this is lazy when the waveform above is eager
///
/// It looks like an inconsistency and it is the oracle's own arithmetic. Both need
/// the same whole-track decode, so the tempting simplification is to decode once at
/// load and build both — and that is wrong twice over. A five-minute stereo track
/// is ~26M frames, so keeping the samples to build the atlas from later costs
/// ~105 MB per open track; and building the atlas at load spends a second full
/// decode plus an offline analysis pass on **every** track, when most sessions
/// never select Song Atlas at all. The C pays the load-time decode once for the
/// envelope, which every track needs, and defers the atlas to the first frame that
/// would draw it.
///
/// The preview stream is paused across the decode and resumed after
/// (`plug.c:719-731`). That is not politeness: this runs inside the frame loop and
/// takes long enough that the stream's buffer would underrun, so without it the
/// first Song Atlas frame is paid for with an audible gap.
///
/// `song_atlas_map_attempted` is set *before* the attempt, so a track whose decode
/// fails is not retried on every subsequent frame.
fn ensure_song_atlas_map(audio: &RaylibAudio, app: &mut App, music: Option<&Music<'_>>) -> bool {
    let Some(track) = app.workspace.current() else {
        return false;
    };
    if track.atlas_map().is_some() {
        return true;
    }
    if track.song_atlas_map_attempted {
        return false;
    }
    let path = track.file_path.clone();

    let resume = music.is_some_and(|m| m.is_stream_playing());
    if resume {
        if let Some(m) = music {
            m.pause_stream();
        }
    }
    let built = build_song_atlas_map(audio, &path);
    if resume {
        if let Some(m) = music {
            // `UpdateMusicStream` before the resume, exactly as the C orders it
            // (`plug.c:729-730`): the buffer drained while the decode ran, and
            // resuming a starved stream before refilling it is the click this
            // pause was supposed to prevent.
            m.update_stream();
            m.resume_stream();
        }
    }

    let Some(track) = app.workspace.current_mut() else {
        return false;
    };
    track.song_atlas_map_attempted = true;
    match built {
        Some(map) => {
            let valid = map.is_valid();
            track.song_atlas_map = Some(map);
            valid
        }
        None => {
            eprintln!(
                "warning: whole-song map could not be prepared for {}",
                path.display()
            );
            false
        }
    }
}

/// The decode-and-build half, with no borrow of `App` held across it.
fn build_song_atlas_map(
    audio: &RaylibAudio,
    path: &Path,
) -> Option<musializer_core::audio::song_atlas_map::SongAtlasMap> {
    let decoded = musializer_runtime::decode::whole_track(audio, path)?;
    musializer_core::audio::song_atlas_map::SongAtlasMap::build(
        &decoded.samples,
        decoded.channels,
        decoded.sample_rate,
    )
    .ok()
}

/// Imports an image as ASCII Field's glyph grid (`plug_load_ascii_image`,
/// `plug.c:894-930`).
///
/// The order is the oracle's and it matters: canonicalize, hash, *then* decode. A
/// file that cannot be identified is refused before any state moves, so a failed
/// import leaves the previous grid intact rather than clearing it — which is what
/// makes re-running `--ascii-image` with a typo harmless.
///
/// With no track open the grid lands in [`App::pending_ascii`] and is handed to the
/// next track that opens, exactly as `p->ascii_cells` is (`plug.c:825-839`).
fn import_ascii_image(app: &mut App, path: &Path) -> Result<(usize, usize), String> {
    let canonical = project_files::canonicalize_existing_file(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let sha256 = project_files::sha256_file_hex(&canonical)
        .map_err(|error| format!("{}: {error}", canonical.display()))?;
    let decoded = musializer_runtime::decode::image_rgba8(&canonical)
        .map_err(|error| format!("{}: {error}", canonical.display()))?;
    let grid = ascii_art::Grid::from_rgba8(
        &decoded.pixels,
        decoded.width,
        decoded.height,
        ascii_art::GRID_MAX_COLUMNS,
        ascii_art::GRID_MAX_ROWS,
    )
    .ok_or_else(|| {
        format!(
            "{}: a {}x{} image could not be fitted to a glyph grid",
            canonical.display(),
            decoded.width,
            decoded.height
        )
    })?;

    let dimensions = (grid.columns(), grid.rows());
    let image = workspace::AsciiImage {
        grid: Some(grid),
        columns: dimensions.0,
        rows: dimensions.1,
        path: canonical,
        sha256,
    };
    match app.workspace.current_mut() {
        Some(track) => {
            track.ascii = Some(image);
            // An imported image is project content, so it dirties the project the
            // same way an edit does (`mark_project_dirty`, `plug.c:920`).
            track.project_dirty = true;
        }
        None => app.pending_ascii = Some(image),
    }
    Ok(dimensions)
}

/// Fills in a restored image's glyph cells by decoding the file again.
///
/// A `.musi` records the image's *identity* — path, hash and dimensions — not its
/// converted cells, so opening a project has to re-run the conversion. The
/// dimensions are then a cross-check rather than an input: if the file on disk fits
/// to a different grid than the project recorded, the hash already matched, so the
/// disagreement means the conversion changed and the recorded dimensions are the
/// stale half.
fn decode_ascii_grid(image: &mut workspace::AsciiImage) {
    let Ok(decoded) = musializer_runtime::decode::image_rgba8(&image.path) else {
        eprintln!(
            "warning: bundled ASCII image could not be decoded: {}",
            image.path.display()
        );
        return;
    };
    let Some(grid) = ascii_art::Grid::from_rgba8(
        &decoded.pixels,
        decoded.width,
        decoded.height,
        ascii_art::GRID_MAX_COLUMNS,
        ascii_art::GRID_MAX_ROWS,
    ) else {
        eprintln!(
            "warning: bundled ASCII image could not be fitted to a glyph grid: {}",
            image.path.display()
        );
        return;
    };
    image.columns = grid.columns();
    image.rows = grid.rows();
    image.grid = Some(grid);
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

/// Adds a track to the workspace, and binds audio to it only if it became the
/// current one (`plug_load_track`, `plug.c:751-861`).
///
/// The "only if" is the oracle's, not a shortcut: loading a second file while one
/// is playing appends it to the list and leaves playback alone (`plug.c:843`).
///
/// Returns the new track's index.
fn open_track<'audio>(
    audio: &'audio RaylibAudio,
    path: &Path,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<usize, String> {
    let path_str = path.to_str().ok_or("audio path is not valid UTF-8")?;
    // Opened before anything is mutated, so an unreadable file leaves the session
    // exactly as it was rather than half-adding a track. This one is only a
    // metadata probe — the stream that plays is opened by `bind_current_audio` —
    // which is the same split as C's `metadata_probe` (`plug.c:4901-4913`).
    let probe = audio
        .new_music(path_str)
        .map_err(|error| error.to_string())?;
    let duration = f64::from(probe.get_time_length());
    drop(probe);

    let (base_scene, seed) = app
        .workspace
        .inherited_scene(app.scene.id(), app.scene.seed());
    let mut track = Track::new(path.to_path_buf(), duration, base_scene, seed)
        .map_err(|error| format!("could not prepare the track: {error}"))?;
    track.transport_seekable =
        musializer_core::timing::track_timeline::path_is_seekable(Some(path_str));
    // At load, before the track is in the workspace, which is where the C does it
    // too (`plug.c:820`, inside `add_track` and before the count is bumped).
    load_timeline_waveform(audio, &mut track);

    // Any new track claims a pending import, first or not (`plug.c:825-839`).
    if let Some(image) = app.pending_ascii.take() {
        track.ascii = Some(image);
    }

    let was_empty = app.workspace.current().is_none();
    if was_empty {
        // Routes accepted before any track existed belong to the first one
        // (`plug.c:852-853`). The pending table is emptied, not copied, so a
        // later track does not silently inherit them too.
        track.scene_routes = std::mem::take(&mut app.pending_routes);
    }
    let index = app.workspace.push(track);

    if was_empty {
        bind_current_audio(audio, analyzer, music, app, scratch, play)?;
    }
    Ok(index)
}

/// Makes the workspace's current track the one the audio device is playing.
///
/// The order here is the whole reason this is one function rather than code at
/// each call site. Detach before the old `Music` drops, or raylib's per-stream
/// processor list holds a callback for a freed stream; drain the ring after the
/// detach, or the first frames of the new track are analysed together with the
/// tail of the old one; and rebind the analyzer from the *file's* sample rate,
/// because that is what `start_preview_track` does (`plug.c:658-660`) and reading
/// the device's rate instead shifts every band.
fn bind_current_audio<'audio>(
    audio: &'audio RaylibAudio,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<(), String> {
    let Some(track) = app.workspace.current() else {
        close_audio(music, app, scratch);
        return Ok(());
    };
    // Opened before the teardown, so a file deleted since it was added leaves the
    // previous track playing rather than leaving silence.
    let opened = open_music(audio, &track.file_path)?;
    bind_audio(opened, analyzer, music, app, scratch, play)
}

/// Opens a stream without binding it to anything.
///
/// Split out so a caller that must not mutate before it knows the file is
/// readable — [`select_track`] — can prove that first.
fn open_music<'audio>(audio: &'audio RaylibAudio, path: &Path) -> Result<Music<'audio>, String> {
    let path_str = path.to_str().ok_or("audio path is not valid UTF-8")?;
    audio.new_music(path_str).map_err(|error| error.to_string())
}

/// Swaps an already-opened stream in as the one the analyzer hears.
fn bind_audio<'audio>(
    opened: Music<'audio>,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<(), String> {
    close_audio(music, app, scratch);

    let file_sample_rate = opened.stream.sampleRate;
    *analyzer = *AudioAnalyzer::boxed(AudioAnalyzerConfig::preview(file_sample_rate))
        .map_err(|error| format!("could not configure the analyzer: {error}"))?;
    if play {
        opened.play_stream();
    }
    // SAFETY: the bridge is installed in `run` before this can be called, the
    // audio device is initialized (it is what produced `opened`), and the stream
    // outlives the attachment because `close_audio` — reached from the next
    // `bind_current_audio` and from `run`'s shutdown — detaches before dropping
    // it.
    unsafe { audio_bridge::attach(opened.stream) }
        .map_err(|error| format!("could not attach the audio bridge: {error}"))?;

    app.shell
        .timeline
        .reset(f64::from(opened.get_time_length()));
    *music = Some(opened);
    Ok(())
}

/// Detaches and drops the playing stream, leaving no stale samples behind.
///
/// Draining the ring is not tidiness. The ring is lock-free SPSC and nothing
/// produces into it once the processor is detached, so this is the only moment a
/// consumer may safely empty it — and the samples in it belong to a track that is
/// about to stop being heard.
///
/// This touches audio only. The track stays in the workspace, because the frozen
/// C has no way to close one.
fn close_audio(music: &mut Option<Music<'_>>, app: &mut App, scratch: &mut [f32]) {
    let Some(open) = music.take() else {
        return;
    };
    // SAFETY: the same stream `bind_current_audio` passed to `attach`, still
    // alive here because it is dropped at the end of this function and not
    // before.
    unsafe { audio_bridge::detach(open.stream) };
    drop(open);
    while audio_bridge::drain_interleaved(scratch) > 0 {}
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

/// The manual event row's outcome (`plug.c:2861-2971`).
///
/// Every arm ends in `mark_dirty`, because each one changes something a `.musi`
/// records. Arming and undoing do not, which is the C's distinction too: a
/// confirmation that has not been answered has changed nothing yet.
fn handle_manual_event(app: &mut App, action: ManualEventAction, time: f64, now: f64) {
    use ManualEventAction as Action;
    let scene = app.scene.id();
    let Some(track) = app.workspace.current_mut() else {
        return;
    };
    match action {
        Action::Record(event) => {
            if track.record_manual_event(event).is_ok() {
                track.mark_dirty(now);
            }
        }
        Action::RecordSceneCue => match track.record_scene_cue(scene, time) {
            Ok(()) => track.mark_dirty(now),
            Err(error) => {
                let detail = error.to_string();
                app.shell
                    .notify(Severity::Warning, "Scene cue was not recorded", &detail);
            }
        },
        Action::ArmClear => track.manual_clear.arm(),
        Action::Clear => {
            // Disjoint field borrows: the clear owns the undo slot, the timeline
            // and the id allocator are the track's.
            track
                .manual_clear
                .clear(&mut track.manual_events, &mut track.next_manual_event_id);
            track.mark_dirty(now);
        }
        Action::UndoClear => {
            track
                .manual_clear
                .undo(&mut track.manual_events, &mut track.next_manual_event_id);
            track.mark_dirty(now);
        }
    }
}

/// The shared preset block's outcome (`plug.c:5979-6100`).
///
/// Every mutation writes the store immediately rather than at shutdown: the C
/// does the same, and a preset library that only survives a clean exit is one
/// that loses work to the crash it was meant to protect against.
fn handle_preset(app: &mut App, action: PresetAction, now: f64) {
    let scene = app.scene.id();
    // Any movement disarms Delete, so a second click cannot land on a preset the
    // user never armed (`plug.c:5983-5987`).
    let mut mutated = false;
    match action {
        PresetAction::Select(index) => {
            app.preset_selection = index;
            app.preset_delete_armed = false;
        }
        PresetAction::Apply(index) => {
            app.preset_delete_armed = false;
            if let Some(preset) = app.shared_presets.presets(scene).get(index) {
                let snapshot = preset.snapshot;
                if app.settings_mut().apply_snapshot(scene, &snapshot) {
                    mark_current_track_dirty(app, now);
                }
            }
        }
        PresetAction::SaveNew => {
            app.preset_delete_armed = false;
            let Some(snapshot) = app.settings().capture(scene) else {
                return;
            };
            let name = preset_store::generated_name(&app.shared_presets);
            if let Some(index) = app.shared_presets.push(scene, &name, &snapshot) {
                app.preset_selection = index;
                mutated = true;
            }
        }
        PresetAction::Replace(index) => {
            app.preset_delete_armed = false;
            if let Some(snapshot) = app.settings().capture(scene) {
                mutated = app.shared_presets.replace_snapshot(scene, index, &snapshot);
            }
        }
        PresetAction::ArmDelete(index) => {
            app.preset_selection = index;
            app.preset_delete_armed = true;
        }
        PresetAction::Delete(index) => {
            app.preset_delete_armed = false;
            if app.shared_presets.remove(scene, index) {
                app.preset_selection = preset_store::selection_after_remove(
                    app.shared_presets.presets(scene).len(),
                    index,
                );
                mutated = true;
            }
        }
    }
    if mutated {
        save_shared_presets(app);
    }
}

/// Writes the shared library back, reporting a failure rather than losing it
/// silently.
fn save_shared_presets(app: &mut App) {
    let Some(path) = app.preset_store_path.clone() else {
        return;
    };
    if let Err(error) = preset_files::save(&path, &app.shared_presets) {
        // The edit stays in memory: refusing to keep it as well would lose the
        // work twice over.
        app.shell.notify(
            Severity::Error,
            "Presets could not be saved",
            &format!(
                "{}: {error}. The change is still in this session.",
                path.display()
            ),
        );
    }
}

/// Drops the committed route for one parameter, if there is one.
///
/// `RouteTable::remove` takes a position, and every caller here knows a
/// parameter key instead — the table keys by parameter, so the lookup is the
/// same one `add` does to reject a duplicate.
fn remove_route(app: &mut App, scene: SceneId, parameter: &str) -> bool {
    let Some(index) = app
        .routes()
        .scene(scene)
        .items()
        .iter()
        .position(|mapping| mapping.parameter == parameter)
    else {
        return false;
    };
    app.routes_mut().remove(scene, index)
}

/// `mark_project_dirty` (`plug.c:616-622`), for the current track.
///
/// Every edit that a `.musi` would record goes through here, which is what makes
/// autosave's 1.5-second settle measure from the last edit rather than from the
/// first.
fn mark_current_track_dirty(app: &mut App, now_seconds: f64) {
    if let Some(track) = app.workspace.current_mut() {
        track.mark_dirty(now_seconds);
    }
}

/// Whether the window may close (`plug_confirm_close`, `plug.c:7200-7250`).
///
/// Returns `true` to quit. With nothing unresolved it never asks, which is the
/// C's first check and the reason a normal session closes instantly.
///
/// # The divergence, and why
///
/// The C asks through `tinyfd_messageBox`, which always answers something
/// because tinyfd falls back to a terminal prompt. Here the dialog is `kdialog`
/// or `zenity` as a child process, and neither may be installed. Refusing to
/// quit forever would trap the user in the application; quitting silently would
/// discard the work the guard exists to protect. So an unavailable dialog
/// refuses the **first** request and says why, in the tray and on stderr, and
/// honours the second. That is finite, it never loses work without having said
/// so, and it is the smallest invention that satisfies both halves.
fn confirm_close(app: &mut App, exporting: bool, already_warned: &mut bool) -> bool {
    let dirty = app
        .workspace
        .tracks()
        .iter()
        .filter(|track| track.has_unsaved_work())
        .count();
    let route_edit = app.shell.route_edit_is_dirty();
    // The other five conditions the C weighs — an open lyric draft, an open route
    // edit, staged Assist suggestions, a running analysis and a running export —
    // belong to Agents I, G, J and H. Each adds a line to this list.
    if dirty == 0 && !route_edit && !exporting && !app.workspace.assist.blocks_close() {
        return true;
    }
    // The C builds this list line by line from six conditions (`plug.c:7222-7247`).
    // Three of the six — staged Assist suggestions, a running analysis and a
    // running export — belong to Agents H and J and each adds a line here.
    let mut items = String::new();
    if route_edit {
        items.push_str("\n- Apply or discard the open audio-route edit.");
    }
    // The C's order (`plug.c:7222-7241`).
    if app.workspace.assist.candidate.is_some() {
        items.push_str("\n- Apply or discard the validated Assist result.");
    }
    if exporting {
        items.push_str("\n- Cancel the running video export.");
    }
    if app.workspace.assist.is_active() {
        items.push_str("\n- Cancel the running analysis job.");
    }
    if dirty > 0 {
        items.push_str(&format!(
            "\n- Save {dirty} unnamed or unresolved track project{}.",
            if dirty == 1 { "" } else { "s" }
        ));
    }
    let message = format!(
        "Resolve these items before quitting, or discard them now:\n{items}\n\nQuit anyway and discard or cancel the items above?"
    );
    match dialogs::confirm_warning("Unresolved Musializer work", &message) {
        Ok(quit) => quit,
        Err(error) => {
            if *already_warned {
                eprintln!(
                    "warning: quitting with {dirty} unsaved project(s) after a second request"
                );
                return true;
            }
            *already_warned = true;
            eprintln!("warning: {dirty} unsaved project(s), and no dialog is available to confirm: {error}");
            app.shell.notify(
                Severity::Warning,
                "Unresolved work",
                &format!(
                    "{dirty} project(s) have unsaved changes and no confirmation dialog is available. Save them, or ask to close again to discard."
                ),
            );
            false
        }
    }
}

/// What the command line asked to open. One slot, because a project and an audio
/// file compete for it and the last one wins (`musializer.c:500-506`).
enum Input {
    Audio(PathBuf),
    Project(PathBuf),
}

/// Opens a `.musi` and makes its track current (`open_project_path`,
/// `plug.c:4665-5044`).
///
/// The whole track is built, with every asset digest already verified, before
/// anything in the session is replaced. A project that fails to open leaves the
/// workspace exactly as it was, which is why this cannot be written as "clear,
/// then load".
fn open_project<'audio>(
    audio: &'audio RaylibAudio,
    path: &Path,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
) -> Result<(), String> {
    let opened = project::open_path(path, |audio_path| {
        // A metadata probe, exactly as C's `LoadMusicStream` at `plug.c:4901` is:
        // the stream that plays is opened by `bind_current_audio`.
        let probe = open_music(audio, audio_path)?;
        Ok(f64::from(probe.get_time_length()))
    })
    .map_err(|error| error.to_string())?;

    if let Some(project::OpenWarning::LegacyAudioPath) = opened.warning {
        app.shell.notify(
            Severity::Warning,
            "Project used a legacy asset path",
            "Audio was found in the launch directory because it was not beside the project. Move it beside the project or use an absolute path for portability.",
        );
    }

    let mut track = opened.track;
    // The same load-time preprocessing a plain audio file gets. A project's track
    // reaches the workspace through a different door, and the envelope is derived
    // from the audio rather than stored in the `.musi`, so it has to be built on
    // both paths or the strip is blank for exactly the tracks that carry authored
    // work.
    load_timeline_waveform(audio, &mut track);
    if let Some(image) = track.ascii.as_mut() {
        decode_ascii_grid(image);
    }
    let index = app.workspace.push(track);
    // `push` only selects when the workspace was empty, but opening a project
    // always makes its track current (`plug.c:5031`). The audio bind is
    // `select_track`'s either way, so this is one call rather than two paths.
    if app.workspace.current_index() == Some(index) {
        let (scene, seed) = {
            let track = &app.workspace.tracks()[index];
            (track.base_scene, track.scene_seed)
        };
        app.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
        bind_current_audio(audio, analyzer, music, app, scratch, true)?;
    } else {
        select_track(audio, index, analyzer, music, app, scratch)?;
    }
    app.shell.notify(
        Severity::Info,
        "Project opened",
        "Lyrics, embedded semantic cues, authored lanes, scene plan, and output settings were restored.",
    );
    Ok(())
}

/// Saves the current track's project to `destination`.
///
/// The sample rate and channel count come off the live stream because that is
/// where the C reads them (`plug.c:4304-4306`) and the track model deliberately
/// holds no audio handle. A track with no stream bound cannot be saved, which is
/// the same restriction the C has by construction.
fn save_project_to(
    app: &mut App,
    music: Option<&Music<'_>>,
    destination: &Path,
    reuse_published: bool,
) -> Result<(), String> {
    let stream = music.ok_or("there is no track to save")?.stream;
    let track = app
        .workspace
        .current_mut()
        .ok_or("there is no track to save")?;
    project::save_to_path(
        track,
        destination,
        stream.sampleRate,
        stream.channels as u16,
        reuse_published,
    )
    .map_err(|error| error.to_string())
}

/// The Save button (`save_project`, `plug.c:4641-4646`): saves in place when the
/// track has a project path, and otherwise falls through to Save As.
fn save_project_command(app: &mut App, music: Option<&Music<'_>>, ask_if_unnamed: bool) {
    let existing = app
        .workspace
        .current()
        .and_then(|track| track.project_path.clone());
    let destination = match existing {
        Some(path) => Some(path),
        None if ask_if_unnamed => match ask_for_project_path(app) {
            Some(path) => Some(path),
            None => return,
        },
        None => return,
    };
    let Some(destination) = destination else {
        return;
    };
    match save_project_to(app, music, &destination, false) {
        Ok(()) => app.shell.notify(
            Severity::Info,
            "Project saved",
            "Audio, ASCII imagery, lyrics, scenes, events, and output settings are durable.",
        ),
        Err(error) => app
            .shell
            .notify(Severity::Error, "Project could not be saved", &error),
    }
}

/// Asks where to save, seeded with the suggestion `save_project_as` computes.
///
/// Called from outside the drawing pair, like every other modal here.
fn ask_for_project_path(app: &mut App) -> Option<PathBuf> {
    let suggestion = app.workspace.current().map(project::suggested_save_path);
    let mut dialog = FileDialog::new("Save Musializer project")
        .with_filter(dialogs::filters::MUSIALIZER_PROJECT);
    if let Some(path) = suggestion {
        dialog = dialog.with_default_path(path);
    }
    match dialog.save_file() {
        Ok(path) => path,
        Err(error) => {
            app.shell.notify(
                Severity::Warning,
                "No file picker is available",
                &format!("{error}. Pass --save-project on the command line instead."),
            );
            None
        }
    }
}

/// Makes another track current, rebinding the scene and the audio device
/// (the tracks-panel click, `plug.c:5261-5283`).
///
/// The order is the oracle's and it is defensive: the scene is bound *first*, so
/// a scene that cannot be prepared leaves the current track playing rather than
/// leaving the session with no audio and no picture. Selecting the same track is
/// a no-op rather than a restart.
///
/// The C also runs `lyric_editor_allow_context_change` and
/// `route_editor_allow_active_context_change` before any of this. Those guards
/// belong to the panels that own the drafts (Agents G and I); the call sites here
/// are where they hook in.
fn select_track<'audio>(
    audio: &'audio RaylibAudio,
    index: usize,
    analyzer: &mut AudioAnalyzer,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
) -> Result<(), String> {
    if app.workspace.current_index() == Some(index) {
        return Ok(());
    }
    let track = app
        .workspace
        .get(index)
        .ok_or_else(|| format!("there is no track {index}"))?;
    let (scene, seed) = (track.base_scene, track.scene_seed);
    // Everything that can fail happens before anything is mutated, so a refused
    // switch leaves the session exactly as it was — which is what the notice at
    // the call site promises.
    let opened = open_music(audio, &track.file_path)?;

    app.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
    app.workspace.select(index);
    bind_audio(opened, analyzer, music, app, scratch, true)
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
        assist: &AssistController,
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
        println!("routes:          {}", app.routes().scene(scene).len());
        // The workspace, as evidence rather than as existence. `--probe-reopen`
        // adds a second track and selects it, and only this line distinguishes
        // "swapped the stream" from "swapped the stream *and* kept both tracks in
        // the list with the right one current" — which is the whole difference
        // between the oracle's track model and the single slot it replaced.
        match app.workspace.current() {
            None => println!("tracks:          0 open"),
            Some(track) => println!(
                "tracks:          {} open, current {} \"{}\"",
                app.workspace.len(),
                app.workspace.current_index().unwrap_or(0),
                track.display_name()
            ),
        }
        // The project half, as evidence: a `.musi` that was opened but left no
        // path behind, or one that is dirty the moment it was written, both exit
        // 0 and look identical without this line.
        match app.workspace.current() {
            Some(track) => match &track.project_path {
                Some(path) => println!(
                    "project:         {} ({})",
                    path.display(),
                    if track.project_dirty {
                        "dirty"
                    } else {
                        "clean"
                    }
                ),
                None => println!("project:         none (audio only)"),
            },
            None => println!("project:         no track"),
        }
        // The three whole-track derivations, each one distinguishing "the scene
        // drew" from "the scene had something to draw". Every one of them is a
        // surface that looks plausible when it is empty: ASCII Field falls back to a
        // procedural spectrogram, Song Atlas to a live idle terrain, and the
        // timeline strip to a flat lane. Without these lines a capture of an
        // unwired import is indistinguishable from a capture of a working one —
        // which is exactly how all three sat unwired for two bands.
        match app.workspace.current() {
            None => println!("waveform:        no track"),
            Some(track) => match &track.timeline_waveform {
                Some(waveform) => println!("waveform:        {} bins", waveform.len()),
                None => println!("waveform:        unavailable (decode failed)"),
            },
        }
        match app.workspace.current() {
            None => println!("atlas:           no track"),
            Some(track) => match (track.atlas_map(), track.song_atlas_map_attempted) {
                (Some(map), _) => println!(
                    "atlas:           {} slices, {} onsets",
                    map.slices().len(),
                    map.slices().iter().filter(|slice| slice.onset).count()
                ),
                (None, true) => println!("atlas:           attempted, unavailable"),
                // Not a failure. The map is built at the first frame that would
                // draw it, so on any other scene this is the correct state and the
                // line says which of the two it is.
                (None, false) => println!("atlas:           not needed by this scene"),
            },
        }
        match app.workspace.current() {
            None => println!("ascii:           no track"),
            Some(track) => match (track.ascii_grid(), &track.ascii) {
                (Some(grid), _) => println!(
                    "ascii:           {}x{} glyphs from {}",
                    grid.columns(),
                    grid.rows(),
                    track
                        .ascii
                        .as_ref()
                        .map_or_else(|| "?".into(), |image| image.path.display().to_string())
                ),
                (None, Some(image)) => println!(
                    "ascii:           {}x{} recorded, not decoded",
                    image.columns, image.rows
                ),
                (None, None) => println!("ascii:           none (procedural mode)"),
            },
        }
        println!("panel:           {}", app.shell.panel.label());
        // Evidence, not existence: the Assist panel draws the same box whether
        // the helper was found or not, and whether the confirmation step is
        // armed or not. This line is what a capture asserts on.
        {
            let session = &app.workspace.assist;
            println!(
                "assist:          helper={} state={:?} body={:?} mode={} confirm={} staged={} sheet={:?}",
                match assist.helper() {
                    Some(path) => path.display().to_string(),
                    None => "not found".to_string(),
                },
                session.job_state,
                session.panel_content(),
                session.mode().argument(),
                session.confirmation_pending(),
                session.candidate.is_some(),
                ui::panels::assist::resolve_lyric_reference(app.workspace.current()).0,
            );
        }
        // `headless_check.sh` greps this: how many cues the editor is showing and
        // which pane is open, which a screenshot of a scrolled list cannot say.
        println!(
            "lyrics:          {}",
            app.shell.lyrics.describe(app.workspace.current())
        );
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
