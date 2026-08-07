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
//! 3. The `argv` actions, left to right. Every input is opened at its position:
//!    plain audio appends, while a project appends and becomes current.
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
use musializer_core::project::frame_lanes::{FrameLaneStatus, ProjectFrameLanes, SceneFrameTiming};
use musializer_core::project::lyrics;
use musializer_core::project::model::CaptionFace;
use musializer_core::project::preset_store::{
    self, PresetAction, PresetLibrary, SharedPresetsView,
};
use musializer_core::scene::routes::{RouteSources, RouteTable};
use musializer_core::scene::settings;
use musializer_core::scene::{SceneAudioFrame, SceneId, SceneInstance, SceneSettings};
use musializer_core::scenes::ascii_field::ascii_art;
use musializer_core::timing::render_export::{Quality as RenderQuality, RenderExportConfig};
use musializer_core::ui::notice::Severity;
use musializer_runtime::assist::env as assist_env;
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

/// Samples every project-owned scene lane through one preview/export path.
///
/// This is the application-boundary half of the oracle's `make_scene_frame`
/// (`plug.c:1115-1185`). The returned value owns its merged events and copied
/// lyric, so neither caller can accidentally borrow a previous track's view.
pub(crate) fn project_frame_lanes(track: Option<&Track>, time_seconds: f64) -> ProjectFrameLanes {
    let lanes = track.map_or_else(ProjectFrameLanes::empty, |track| {
        ProjectFrameLanes::build(
            time_seconds,
            &track.lyrics,
            &track.semantic_events,
            &track.manual_events,
        )
    });
    if let Some(error) = lanes.merge_error() {
        // The C reports the same failure through TraceLog and clears the view
        // (`plug.c:1102-1110`). A corrupt in-memory lane should be impossible
        // after project validation, but if that invariant is ever broken it must
        // be visible and must not draw stale events.
        eprintln!("EVENTS: could not build merged scene view: {error}");
    }
    lanes
}

/// Keeps the one GPU imported-face slot synchronized with the current track.
///
/// Project open verifies the bytes and records their runtime path before this is
/// called. Rasterization remains a window-owned operation, so it happens here,
/// before either preview or export begins drawing (`plug.c:383-427`, `:4971-4990`).
fn sync_caption_face(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    fonts: &mut Faces,
    app: &mut App,
    last_request: &mut Option<PathBuf>,
    music: Option<&Music<'_>>,
) {
    let request = app.workspace.current().and_then(|track| {
        (track.caption_style.face == CaptionFace::Imported)
            .then(|| track.caption_font_path.clone())
            .flatten()
    });
    if *last_request == request {
        return;
    }
    with_preview_paused(music, || {
        *last_request = request.clone();
        match request {
            None => fonts.clear_imported(),
            Some(path) if fonts.load_imported(rl, thread, &path) => {}
            Some(path) => app.shell.notify(
                Severity::Warning,
                "Project caption font could not be read",
                &format!(
                    "{} was verified as a project asset but could not be rasterized. Captions use Alegreya for this session.",
                    path.display()
                ),
            ),
        }
    });
}

/// Runs synchronous preparation without allowing the preview stream to drain.
///
/// `UpdateMusicStream` before resume is essential: pausing prevents the device
/// from consuming stale halves, and the refill prevents the first resumed block
/// from being silence.
fn with_preview_paused<T>(music: Option<&Music<'_>>, work: impl FnOnce() -> T) -> T {
    let resume = music.is_some_and(Music::is_stream_playing);
    if resume {
        music.expect("resume implies a stream").pause_stream();
    }
    let result = work();
    if resume {
        let music = music.expect("resume implies a stream");
        music.update_stream();
        music.resume_stream();
    }
    result
}

/// The oracle derives scene delta from the transport clock, not render time.
/// A seek or a long stall is a discontinuity and therefore produces a zero
/// update rather than advancing state by a visible jump.
fn scene_clock_delta(previous: &mut Option<f64>, time_seconds: f64) -> f32 {
    let Some(earlier) = previous.replace(time_seconds) else {
        return 0.0;
    };
    let elapsed = time_seconds - earlier;
    if !(0.0..=0.5).contains(&elapsed) {
        0.0
    } else {
        elapsed as f32
    }
}

fn main() -> std::process::ExitCode {
    // First statement of the program, and it has to stay first. `remove_var` is
    // `unsafe` in edition 2024 because the C environment is process-global, and
    // its whole contract is that no other thread exists yet — before the window,
    // the audio device, and before any library starts a thread of its own.
    //
    // What it buys is E1 in `docs/ASSIST_PROVIDER_CONTRACTS.md`: after this line
    // there is no `OPENROUTER_API_KEY` in the environment for `ffmpeg`,
    // `kdialog`, `codex` or a Python helper to inherit by accident. The only
    // copy left is the `Secret`, which has one owner and is zeroized on drop.
    //
    // SAFETY: nothing has run before this point — no thread has been spawned,
    // no raylib call has been made, and `cli::parse` has not been reached — so
    // no other thread can be reading the environment concurrently.
    let session_credentials = unsafe { assist_env::import_session_credentials() };
    // The credential itself travels onward to exactly one owner — the Assist
    // controller, which is the only thing in the application that may hand one
    // to a child (P4, §4 E1). Everything else, the AI settings dialog included,
    // gets the fingerprint and says "session only" from it, without ever holding
    // a second copy of the key (AP3, §3 "one owner").
    match run(session_credentials) {
        Ok(status) => status,
        Err(message) => {
            eprintln!("musializer: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(
    session_credentials: musializer_runtime::assist::env::SessionCredentials,
) -> Result<std::process::ExitCode, String> {
    let session_fingerprint = session_credentials.openrouter_fingerprint();
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
            // Resolved at the parity gate, and the resolution is a split.
            //
            // The C has three spellings of one version, from three separate
            // literals: `Musializer 2026.07` in the help header
            // (`musializer.c:255`), `musializer 2026.07` from `--version`
            // (`:323`), and `musializer-2026.07` in a saved `.musi`
            // (`plug.c:4293`). Only the third is a **file field**, so only the
            // third is not negotiable — `project::APPLICATION_VERSION` is that
            // literal exactly, and a differential round trip holds it there.
            //
            // The other two are prose, and this build deliberately does not claim
            // them. Impersonating the frozen binary's version would make the two
            // indistinguishable to a script at exactly the point where they still
            // differ by a documented list (no microphone capture, no hot reload,
            // Linux only). The parity target is named rather than claimed, so the
            // line is still greppable for it.
            println!(
                "musializer-rs {} (parity target musializer 2026.07, raylib {raylib_version})",
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

    // `--ui-probe size=` is applied with the rest of the probe after every CLI
    // stage succeeds. Starting at that size here would make a failed command line
    // mutate the window even though the C suppresses the probe whole.
    let (width, height) = options.window;

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
    let audio = RaylibAudio::init_audio_device()
        .map_err(|error| format!("could not initialize the audio device: {error}"))?;
    // raylib's default half-buffer is only sampleRate/30 (about 33 ms). The C
    // build deliberately raises this to retain roughly 170-186 ms of refill
    // headroom for synchronous editor work. It must be set before any Music is
    // created because the size is copied into each new stream.
    audio.set_audio_stream_buffer_size_default(8192);

    // Output volume, and mute as a separate flag rather than a volume of zero.
    //
    // Two values because unmuting has to restore the level the user set, and a
    // single `volume` that mute wrote zero into would have lost it. The device only
    // ever sees the product, which `apply_volume` below is the one place that
    // computes — a second caller multiplying these itself is how the mute button
    // and the slider would come to disagree.
    //
    // Not in `App`: it is the state of the audio device, not of the workspace, and
    // nothing that gets saved to a `.musi` should be able to reach it.
    // Match the frozen build's deliberately conservative startup level. This is
    // still user-adjustable and independent of the process-local mute flag.
    let mut volume = 0.5f32;
    let mut muted = false;
    let apply_volume = |audio: &RaylibAudio, volume: f32, muted: bool| {
        audio.set_master_volume(if muted { 0.0 } else { volume.clamp(0.0, 1.0) });
    };

    let ui_preferences_path = ui::preferences::default_path();
    let (ui_preferences, mut ui_preferences_editable, ui_preferences_warning) =
        match ui_preferences_path.as_deref() {
            None => (
                ui::preferences::UiPreferences::default(),
                false,
                Some("No per-user configuration directory could be derived.".to_string()),
            ),
            Some(path) => match ui::preferences::load(path) {
                Ok(preferences) => (preferences.unwrap_or_default(), true, None),
                Err(error) => (
                    ui::preferences::UiPreferences::default(),
                    false,
                    Some(format!(
                        "{}: {error}. UI changes remain session-only so the file is not overwritten.",
                        path.display()
                    )),
                ),
            },
        };
    let physical_window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
    let mut ui_scale = ui::scale::effective_scale(
        options.ui_scale.unwrap_or(ui_preferences.scale),
        physical_window,
        rl.get_window_scale_dpi(),
    );

    // After the window, because the atlas is a GPU upload. Never fails: a face
    // that will not rasterize falls back to raylib's default and says so.
    let mut fonts = Faces::load_with_ui_scale(&mut rl, &thread, ui_scale.value());
    let mut caption_font_request = None;

    let mut renderer = scene_host::SceneRenderer::load(&mut rl, &thread)?;

    audio_bridge::install(audio_bridge::DEFAULT_CAPACITY)
        .map_err(|error| format!("could not install the audio bridge: {error}"))?;

    // Starts in the oracle's no-track configuration and is reconfigured from the
    // file's sample rate once a track loads, mirroring `analyzer_configure`.
    // 200 KiB of arrays, so it is boxed.
    let mut analysis = Analysis::idle()?;

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
        recent_path: None,
        recent_editable: false,
        preset_selection: 0,
        preset_delete_armed: false,
        shell: {
            let mut shell = Shell::with_preferences(ui_preferences);
            shell.session_credential_fingerprint = session_fingerprint;
            shell
        },
    };
    app.shell.set_ui_scale_override(options.ui_scale);
    if let Some(detail) = ui_preferences_warning {
        app.shell.notify(
            Severity::Warning,
            "UI preferences are session-only",
            &detail,
        );
    }

    // The Assist supervisor. `find_assist_helper` probes relative to the
    // executable's directory, which is raylib's `GetApplicationDirectory()`.
    let application_directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut assist = AssistController::new(&application_directory);
    assist.set_session_credentials(session_credentials);
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

    // The welcome screen's recent-project list, read once (UX0-C06). Same policy
    // as the preset store above and `ui.json` before it: unreadable is not fatal,
    // the list stays empty, and writing is refused so the file survives. The
    // *screen* says which of the two it is, because a blank column is
    // indistinguishable from a broken one.
    app.recent_path = ui::preferences::recent::default_path();
    match app.recent_path.clone() {
        None => app.shell.recent_unavailable = true,
        Some(path) => match ui::preferences::recent::load(&path) {
            Ok(list) => {
                app.shell.recent = list.unwrap_or_default();
                app.recent_editable = true;
            }
            Err(error) => {
                app.shell.recent_unavailable = true;
                app.shell.notify(
                    Severity::Warning,
                    "Recent projects could not be read",
                    &format!(
                        "{}: {error}. The list is disabled so the file is not overwritten.",
                        path.display()
                    ),
                );
            }
        },
    }
    // Before the first frame, so a moved project is already marked rather than
    // drawing as openable until something happens to re-probe it.
    app.shell.recent.probe(|path| path.is_file());

    // Interleaved stereo scratch, drained from the ring each frame. It and the
    // stream owner must exist before argv replay because input actions open at
    // their actual positions; delaying these two was what forced the old
    // single-slot `Option<Input>` model.
    let mut scratch = vec![0.0f32; 4096 * audio_bridge::MIXED_CHANNELS];
    let mut music: Option<Music<'_>> = None;
    // A probe that asks for a parked transport gets an initially paused stream.
    // The full probe is still applied later, after every gated CLI stage.
    let play = !options
        .ui_probe
        .as_ref()
        .is_some_and(|probe| !probe.playing);

    // Step 3: the argv actions, left to right.
    for action in std::mem::take(&mut options.actions) {
        match action {
            // Sets the *flag*, where the oracle sets the device volume directly
            // (`musializer.c:399-405`). The observable startup state is identical
            // — silence — but this way the transport row's mute button can undo
            // it, where a device volume of zero would leave the button showing
            // "unmute" over a slider that had already lost the level.
            Action::Mute => {
                muted = true;
                apply_volume(&audio, volume, muted);
            }
            Action::SelectScene(id) => {
                app.select_scene(id, 0.0);
            }
            Action::AsciiImage(path) => {
                // 0.0 like the other command-line replays above: the window clock
                // has not started, and `mark_command_line_state_clean` clears
                // this dirtiness a moment later anyway.
                match import_ascii_image(&mut app, &path, 0.0) {
                    // Not prefixed `ascii:` — that key belongs to the slice
                    // report, and a check grepping for it would otherwise match
                    // this line too and assert on the wrong one.
                    Ok((columns, rows)) => {
                        println!(
                            "ascii import:    {} as {columns}x{rows} glyphs",
                            path.display()
                        );
                        // The C's `else if`: a failed import does not change the
                        // scene (`musializer.c:413-420`).
                        app.select_scene(SceneId::AsciiField, 0.0);
                    }
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
            }
            // `plug_record_event` (`plug.c:1055-1069`). Because inputs are opened
            // in place, an event before the first one enters the pending lane and
            // an event after one edits the then-current track.
            Action::RecordEvent(event) => {
                if app.workspace.record_event(event).is_err() {
                    eprintln!(
                        "warning: Invalid command-line event; expected type:seconds:id:value"
                    );
                    options.error = true;
                }
            }
            Action::OpenProject(path) => {
                if let Err(error) = open_project(
                    &audio,
                    &path,
                    &mut analysis,
                    &mut music,
                    &mut app,
                    &mut scratch,
                ) {
                    eprintln!("warning: could not open {}: {error}", path.display());
                    options.error = true;
                } else {
                    // A project opened from argv is as recent as one opened from
                    // the picker — that is the desktop file association's own
                    // path, and leaving it out would make double-clicking a
                    // `.musi` invisible to the welcome screen.
                    //
                    // But only for a *session*. A run that exits after N frames or
                    // after writing a file is a batch job, and this store lives in
                    // the operator's configuration directory: without this guard
                    // `tools/verify.sh` would write a dozen scratch projects into
                    // the real `~/.config/musializer/recent.json` as a side effect
                    // of being run. Interactive recording — a drop, a picker, a
                    // click on the list — is deliberately *not* guarded, because
                    // that is what the headless gate has to be able to prove.
                    if is_session_run(&options) {
                        remember_recent_project(&mut app, &path);
                    }
                    if !play {
                        if let Some(open) = music.as_ref() {
                            open.pause_stream();
                        }
                    }
                }
            }
            Action::LoadTrack(path) => {
                if let Err(error) = open_track(
                    &audio,
                    &path,
                    &mut analysis,
                    &mut music,
                    &mut app,
                    &mut scratch,
                    play,
                ) {
                    eprintln!("warning: could not load {}: {error}", path.display());
                    options.error = true;
                }
            }
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

    // The readout's default, decided once. A probe run turns it on because a
    // capture that carries its own evidence is why the line exists; an interactive
    // run leaves it off because it is a developer HUD. `--hud=0|1` overrides both,
    // which is how a capture of the clean preview is taken.
    app.shell.hud_visible = options.hud.unwrap_or(options.probe_frames.is_some());

    // Where `--ui-probe hover=X,Y` parks the pointer, reasserted every frame.
    let mut hover_at: Option<(f32, f32)> = None;
    // `--ui-probe click=XxY`: where to press, and how far through the press we
    // are. The delay exists because the first frames of a run are still settling
    // — the window may not have its requested geometry yet, and a press against a
    // layout that is about to move would land somewhere nobody aimed.
    let mut click_at: Option<(f32, f32)> = None;
    let mut click_phase: u32 = 0;
    let mut audio_stall_ms: Option<u64> = None;
    let mut scene_clock_previous: Option<f64> = None;

    // Render configuration is the first gated stage after deferred routes. It
    // must precede `--save-project`, or a one-shot `--resolution`/`--fps` change
    // is absent from the file the same command line asks us to write.
    if !options.error
        && (options.resolution.is_some() || options.fps.is_some() || quality.is_some())
    {
        let mut config = app
            .workspace
            .current()
            .map_or_else(RenderExportConfig::default, |track| track.render_config);
        if let Some((width, height)) = options.resolution {
            config.width = width;
            config.height = height;
        }
        if let Some(fps) = options.fps {
            config.fps = fps;
        }
        if let Some(named) = quality {
            config.set_quality(match named {
                cli::Quality::Balanced => RenderQuality::Balanced,
                cli::Quality::High => RenderQuality::High,
                cli::Quality::Master => RenderQuality::Master,
            });
        }
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

    // `--analysis-bridge`, after every input is resolved, exactly where the C
    // applies it (`musializer.c:579-586`). It applies rather than staging: a
    // batch entry point with no review step must not leave the result unapplied.
    if let Some(path) = options.analysis_bridge.clone() {
        if options.error {
            eprintln!(
                "warning: could not load command-line analysis bridge {}",
                path.display()
            );
        } else {
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
    }

    // `--auto-scenes` is evaluated after the project/bridge input, because it
    // enables the plan those stages supplied (`musializer.c:585-589`). An empty
    // plan is a command-line failure, not a successful no-op.
    if options.auto_scenes {
        if options.error {
            eprintln!("warning: could not enable automatic scene switching");
        } else {
            match app.workspace.current_mut() {
                Some(track) if !track.scene_switches.is_empty() => {
                    debug_assert!(track.set_auto_scenes(true));
                }
                _ => {
                    eprintln!("warning: could not enable automatic scene switching");
                    options.error = true;
                }
            }
        }
    }

    // `--save-project`, after every input is resolved so it saves what the rest
    // of the command line actually produced (`musializer.c:589-593`). Like the
    // C, a prior error reports this stage as failed without touching the path.
    if let Some(destination) = options.save_project.clone() {
        if options.error {
            eprintln!(
                "warning: could not save command-line project: {}",
                destination.display()
            );
        } else if let Some(index) = app.workspace.current_index() {
            match save_project_to(&mut app, index, &destination, false) {
                Ok(()) => println!("saved {}", destination.display()),
                Err(error) => {
                    eprintln!("warning: could not save {}: {error}", destination.display());
                    options.error = true;
                }
            }
        } else {
            // `--save-project` with nothing open. Reported rather than skipped:
            // the exit status is a documented part of the CLI grammar, and this
            // used to come back through `save_project_to`'s own "there is no
            // track to save".
            eprintln!(
                "warning: could not save {}: there is no track to save",
                destination.display()
            );
            options.error = true;
        }
    }

    // Startup flags are configuration, not an operator edit. `--save-project`
    // above is the opt-in persistence path; otherwise autosave must not commit a
    // one-off scene, route or render setting after launch.
    app.workspace.mark_command_line_state_clean();

    // The AI settings modal's own probe seam (AP3). An environment variable
    // rather than a `--ui-probe` key, matching `MUSIALIZER_ASSIST_PROBE_DIR` and
    // `MUSIALIZER_ASSIST_PROBE_LANES`, and read here because opening the dialog
    // reads four files and must happen once rather than per frame. Inert unless
    // set, so no ordinary run is affected.
    if let Some(section) = std::env::var(ui::assist_settings::PROBE_OPEN_VARIABLE)
        .ok()
        .as_deref()
        .and_then(ui::assist_settings::Section::parse)
    {
        let fingerprint = app.shell.session_credential_fingerprint.clone();
        app.shell.assist_settings.open(section, fingerprint);
    }

    // The probe is one late, gated stage in the C. Keep *all* of it here: panel
    // state applied earlier was still a side effect after an unrelated CLI error.
    if !options.error {
        if let Some(probe) = options.ui_probe.clone() {
            if let Some((width, height)) = probe.size {
                rl.set_window_size(width as i32, height as i32);
            }
            rl.set_window_position(0, 0);
            app.shell.panel = probe.panel;
            app.shell.fullscreen = probe.fullscreen;
            if let Some(width) = probe.sidebar_width {
                app.shell.ui_preferences.sidebar_width = Some(width);
            }
            if let Some(width) = probe.inspector_width {
                app.shell.ui_preferences.inspector_width = Some(width);
            }
            if let Some(height) = probe.timeline_height {
                app.shell.ui_preferences.timeline_height = Some(height);
            }
            // `hover=` means "photograph this tooltip now"; its absence must
            // mean *no* tooltip can fire, or the frame depends on wherever the
            // X server happened to leave the pointer — with tips across the
            // caption panes (UX0-C16), a fresh Xvfb's centred pointer lands on
            // a hinted control and pops one into an unrelated capture.
            app.shell.widgets.tooltip_delay = if probe.hover.is_some() {
                0.0
            } else {
                f64::INFINITY
            };
            hover_at = probe.hover;
            // `click=` parks the pointer too, so a run that passed both would
            // fight over `SetMousePosition`; the click wins, being the more
            // specific request. It deliberately leaves `tooltip_delay` alone: a
            // click capture is after what the press *did*, and a tip fired by
            // the parked pointer would be drawn over the control it changed.
            if let Some(point) = probe.click {
                hover_at = Some(point);
                click_at = Some(point);
            }
            // Delivered by the shell on the first frame it draws, wherever
            // `hover=` parked the pointer.
            app.shell.probe_wheel = probe.wheel;
            // The same one-shot contract, and through the same classifier the
            // device path uses (D1).
            app.shell.probe_drop = probe.drop_file.clone();
            audio_stall_ms = probe.audio_stall_ms;
            if probe.panel == cli::UiPanel::Tune {
                app.shell.inspector_open = true;
            }
            // `--ui-probe assist=` puts the panel in one of its four states
            // (review 4.2). Candidate, Running and Failed are synthesized in
            // process -- no helper, no file, no wall clock -- so two runs of the
            // same probe produce the same pixels. The clock handed over is the
            // transport's, because the running body's elapsed counter is drawn
            // from `time_seconds - started_at` and the transport is parked.
            if let Some(state) = probe.assist {
                if probe.panel != cli::UiPanel::Assist {
                    eprintln!(
                        "warning: could not apply --ui-probe state; assist= needs panel=assist"
                    );
                    options.error = true;
                } else if let Err(reason) = ui::panels::assist::apply_probe_state(
                    &mut app.workspace,
                    state,
                    probe.seek_seconds.unwrap_or(0.0),
                ) {
                    eprintln!("warning: could not apply --ui-probe assist= state; {reason}");
                    options.error = true;
                }
            }

            // `--ui-probe lyrics-file=` selects the sheet the next lyrics run
            // will use (`plug.c:3812-3823`).
            if let Some(path) = probe.lyrics_reference_path.as_ref() {
                if app.workspace.current().is_none() || !path.is_file() {
                    eprintln!(
                        "warning: could not apply --ui-probe lyrics-file={}; it needs a loaded track and an existing file",
                        path.display()
                    );
                    options.error = true;
                } else {
                    for notice in assist.set_lyric_sheet(&mut app.workspace, path) {
                        app.shell
                            .notify(notice.severity, &notice.title, &notice.detail);
                    }
                }
            }

            // "a panel or seek probe needs a loaded, seekable track"
            // (`musializer.c:609-611`).
            if music.is_none()
                && (probe.panel != cli::UiPanel::None || probe.seek_seconds.is_some())
            {
                eprintln!(
                    "warning: could not apply --ui-probe state; a panel or seek probe needs a loaded, seekable track"
                );
                options.error = true;
            }
            if let (Some(music), Some(seconds)) = (music.as_ref(), probe.seek_seconds) {
                seek_preview(
                    music,
                    &mut analysis,
                    &mut app,
                    seconds,
                    &mut scene_clock_previous,
                );
            }
            if let (Some(music), Some(zoom)) = (music.as_ref(), probe.timeline_zoom) {
                let duration = f64::from(music.get_time_length());
                app.shell.reset_timeline(duration);
                app.shell
                    .timeline
                    .zoom(duration, zoom, f64::from(music.get_time_played()));
            }
            // The lyrics editor's probe keys, which need the track they edit.
            let probe_slot = app.workspace.current_index();
            if let Some(track) = app.workspace.current_mut() {
                let mut lyrics = std::mem::take(&mut app.shell.lyrics);
                // The probe binds the form to a cue, so the editor has to own the
                // track first or the first drawn frame drops what it just set
                // (review 1.3).
                lyrics.enter_track(probe_slot);
                let honoured = lyrics.apply_probe(&probe, track);
                app.shell.lyrics = lyrics;
                if !honoured {
                    eprintln!("warning: --ui-probe lyrics keys could not be applied");
                    options.error = true;
                }
            }
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
            // PX6. After `time=` — the audition is keyed to the segment the
            // playhead is in, so a seek must have happened first or the session
            // would be opened against a target the first frame then retargets
            // away from.
            if probe.tune_seed.is_some()
                || probe.tune_explore.is_some()
                || probe.tune_type.is_some()
            {
                let slot = app.workspace.current_index().unwrap_or(0);
                let scene = app.scene.id();
                let cue = app
                    .workspace
                    .current()
                    .and_then(workspace::Track::active_cue)
                    .map(|(position, _)| position);
                // `SceneSettings` is `Copy`, so the probe edits a copy and the
                // result goes back through the same field `settings_mut` picks —
                // which is what keeps a probe honest about cue targeting.
                let mut settings = *app.settings();
                let lines = ui::panels::tune::apply_tune_probe(
                    &mut app.shell,
                    &probe,
                    scene,
                    slot,
                    cue,
                    &mut settings,
                );
                *app.settings_mut() = settings;
                if let Some(track) = app.workspace.current_mut() {
                    track.commit_active_cue_settings(scene);
                }
                for line in lines {
                    println!("{line}");
                }
            }
            // Last in the probe stage on purpose (LX3): the playhead it reads
            // is the one `time=` just parked, so the segment a scene click
            // lands on is the segment the capture will photograph.
            if let Some(id) = probe.scene_pick {
                app.select_scene_interactive(id, probe.seek_seconds.unwrap_or(0.0), 0.0);
            }
        }
    }

    // Hot reload is an approved exclusion, but its failure is itself gated at
    // the same late point where the C attempts the handoff.
    if options.reload_once && !options.error {
        unimplemented_action(
            &mut options,
            &mut app,
            "--reload-once",
            "hot reload is an explicit first-pass non-goal",
        );
    }

    let mut report = Report::default();

    // `--save-project` without `--render` skips the main loop entirely
    // (`musializer.c:617`, `:637`). The save itself was already reported above;
    // honouring the skip keeps the exit path identical.

    // CLI export begins before the ordinary frame loop, so synchronize once here
    // as well as at the top of each interactive frame.
    sync_caption_face(
        &mut rl,
        &thread,
        &mut fonts,
        &mut app,
        &mut caption_font_request,
        music.as_ref(),
    );
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
                &mut analysis,
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
        // Service the decoder before any potentially expensive UI work. A
        // second refill remains below, after window/asset maintenance, matching
        // the oracle's two service opportunities per interactive frame.
        if let Some(music) = music.as_ref() {
            music.update_stream();
        }
        let physical_window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        let resolved_scale = ui::scale::effective_scale(
            app.shell.ui_scale_preference(),
            physical_window,
            rl.get_window_scale_dpi(),
        );
        if resolved_scale != ui_scale {
            with_preview_paused(music.as_ref(), || {
                fonts.set_ui_scale(&mut rl, &thread, resolved_scale.value());
            });
            ui_scale = resolved_scale;
        }
        sync_caption_face(
            &mut rl,
            &thread,
            &mut fonts,
            &mut app,
            &mut caption_font_request,
            music.as_ref(),
        );
        // Reasserted every frame rather than set once, so the tooltip a capture is
        // after is in the shot regardless of how many frames the run lasts — and
        // so a window that only gets its geometry after the first frame cannot
        // leave the pointer somewhere else.
        if let Some((x, y)) = hover_at {
            rl.set_mouse_position(Vector2::new(x, y));
        }
        // `--ui-probe click=`: press, hold, release, on three consecutive frames
        // after three settling ones (EX1).
        //
        // The settle matters. A window that is still taking its requested
        // geometry lays the panels out somewhere else on frame 0, and a press
        // against that layout lands on whatever control happened to be under the
        // coordinate before the resize — which photographs as a click that did
        // nothing, indistinguishable from the defect the probe exists to find.
        //
        // Three frames for the press itself, rather than one, because the claim
        // rule takes a press on the press edge and only cashes it on the release
        // edge, and a widget that saw both in one frame is exactly the case it
        // drops (`Widgets::button_at`).
        if click_at.is_some() {
            app.shell.widgets.set_pointer_probe(match click_phase {
                3 => Some(ui::widgets::PointerProbe {
                    down: true,
                    pressed: true,
                    released: false,
                }),
                4 => Some(ui::widgets::PointerProbe {
                    down: true,
                    pressed: false,
                    released: false,
                }),
                5 => Some(ui::widgets::PointerProbe {
                    down: false,
                    pressed: false,
                    released: true,
                }),
                _ => None,
            });
            click_phase = click_phase.saturating_add(1);
        }
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
                &mut analysis,
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
        // Negative control for the output-underrun counter. This sleeps only the
        // refill/UI thread; raylib's audio device thread continues consuming the
        // primed stream, which is the condition the diagnostic must detect.
        if report.frames == 60 {
            if let Some(milliseconds) = audio_stall_ms.take() {
                std::thread::sleep(std::time::Duration::from_millis(milliseconds));
            }
        }

        // The Assist supervisor, polled before anything is drawn so the panel
        // shows this frame's job state rather than last frame's.
        for notice in assist.poll(&mut app.workspace) {
            app.shell
                .notify(notice.severity, &notice.title, &notice.detail);
        }

        // How full the bridge ring was *before* this frame drained it (EX4).
        //
        // `output underruns:` and the ring's `dropped` counter both stay at zero
        // through the whole band between "the analyzer fell behind the audio" and
        // "the ring overflowed" — roughly 85 ms to 170 ms here, since the scratch
        // buffer is 4096 frames and the ring 8192. A stall inside that band
        // desynchronizes the picture from the music and leaves no trace in any
        // number the report prints. Peak occupancy is the one figure that does.
        if let Some(ring) = audio_bridge::ring() {
            report.observe_ring(ring.len(), ring.capacity());
        }
        let drained = audio_bridge::drain_interleaved(&mut scratch);
        if drained > 0 {
            let consumed = analysis
                .analyzer
                .push_interleaved(&scratch[..drained * audio_bridge::MIXED_CHANNELS]);
            report.consumed_frames += consumed as u64;
        }

        let analyzer_delta = rl.get_frame_time();
        // The frame the user actually felt. `frames rendered:` counts them and
        // says nothing about how long any one of them took, so a 400 ms autosave
        // fsync or a caption atlas rebuild mid-playback is invisible in every
        // line this report prints — which is why "rendering sometimes hiccups"
        // could only ever be reported by eye.
        report.observe_frame_time(analyzer_delta);
        if analysis.analyzer.analyze(analyzer_delta) {
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
        let scene_delta = scene_clock_delta(&mut scene_clock_previous, time_seconds);

        // Scene-plan selection and its settings snapshot precede routing. Keep a
        // scene with an inline route editor visible during preview, exactly as
        // the oracle does; export has no editor and always advances its plan.
        let route_editor_open = app
            .shell
            .route_editor_open_for_active_track(app.workspace.current_index());
        if !route_editor_open {
            app.apply_auto_scene_switch(time_seconds);
        }

        let spectrum = analysis.analyzer.spectrum();
        let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        report.peak_seen = report.peak_seen.max(audio_frame.peak);
        audio_frame.track_beat(&mut analysis.beat, time_seconds);
        report.beat_phase_low = report.beat_phase_low.min(audio_frame.beat_phase);
        report.beat_phase_high = report.beat_phase_high.max(audio_frame.beat_phase);
        report.beat_interval_last = analysis.beat.interval_seconds();
        report.beat_intervals_learned = analysis.beat.learned_intervals();
        report.flux_seen = report.flux_seen.max(audio_frame.spectral_flux);
        if audio_frame.onset {
            report.onsets_seen += 1;
        }

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
        let sources = RouteSources::from_audio(&audio_frame, time_seconds);
        // Copied out of `app` rather than borrowed from it. `settings()` and
        // `routes()` borrow the whole `App` (they choose between the current
        // track's tables and the pending ones), so holding either across
        // `app.scene.update` or `app.shell.draw` would deny those the `&mut` they
        // need. A `SceneSettings` is 480 bytes of `f32`, and `apply` already
        // produces a whole copy whenever a route fires.
        let base = *app.settings();
        let routed = app.routes().apply(app.scene.id(), &sources, &base);
        let effective = routed.as_ref().unwrap_or(&base);

        let frame_lanes = project_frame_lanes(app.workspace.current(), time_seconds);
        report.frame_lanes = frame_lanes.status();
        let frame = frame_lanes.scene_frame(
            SceneFrameTiming {
                time_seconds,
                duration_seconds,
                delta_seconds: scene_delta,
                frame_index: report.frames,
            },
            audio_frame,
            effective,
        );
        app.scene.update(&frame);

        let band_centre_hz = spectrum
            .band_first_bin
            .get(report.peak_band_last)
            .map_or(0.0, |&bin| analysis.analyzer.bin_frequency(bin as usize));

        // Before the draw, and only for the scene that needs it
        // (`scene_render`, `plug.c:1313-1315`). Placed here rather than inside the
        // begin/end drawing pair because it pauses and resumes the audio stream,
        // which is not something to do mid-frame.
        if app.scene.id() == SceneId::SongAtlas {
            let _ = ensure_song_atlas_map(&audio, &mut app, music.as_ref());
        }

        // The shape the export will actually be (EX2). Falls back to the default
        // configuration's 16:9 with no track open, which is what the export
        // panel would offer anyway, so the welcome-state preview is unchanged.
        let preview_aspect = app.workspace.current().map_or_else(
            || {
                let default = RenderExportConfig::default();
                default.width as f32 / default.height as f32
            },
            |track| track.render_config.width as f32 / track.render_config.height as f32,
        );

        // Borrowed from the current track for exactly this frame, which is what
        // stops one track's terrain or glyph grid from being drawn under another.
        let assets =
            app.workspace
                .current()
                .map_or_else(scene_host::TrackAssets::default, |track| {
                    scene_host::TrackAssets {
                        atlas_map: track.atlas_map(),
                        ascii_grid: track.ascii_grid(),
                        caption_style: Some(&track.caption_style),
                    }
                });

        report.logical_window = ui_scale.logical_size(physical_window);
        report.timeline = app.shell.describe_timeline(duration_seconds);
        report.click_at = click_at;
        let shell_input = ShellInput {
            window: report.logical_window,
            ui_scale,
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
            effect_inputs: musializer_core::project::caption_effects::EffectInputs::from_audio(
                time_seconds,
                frame.audio.rms,
                frame.audio.trails,
                frame.audio.beat_phase,
                frame.audio.spectral_flux,
            ),
            band_count: spectrum.band_count(),
            peak_band: report.peak_band_last,
            rms: frame.audio.rms,
            volume,
            muted,
        };
        // With no track open the C draws the welcome screen instead of the
        // workspace (`preview_screen`, `plug.c:7769`), so the workspace frame is
        // not even computed on that path — there is no preview to lay out around.
        let commands;
        if app.workspace.current().is_none() {
            // The recent list's "3 days ago" is measured against this, refreshed
            // per frame so a session left on the welcome screen does not keep
            // saying "just now" about a project opened an hour earlier. Read here
            // rather than in the shell, which owns no clock on purpose.
            app.shell.recent_now_unix = ui::preferences::recent::now_unix();
            let mut d = rl.begin_drawing(&thread);
            d.clear_background(ui::theme::color::ui_surface());
            let mut ui_draw = d.begin_mode2D(ui_scale.camera());
            commands = app.shell.draw_welcome(&mut ui_draw, &shell_input);
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

            // Scene first, chrome over it. The scene-clipping scissor lives
            // inside `SceneRenderer::draw` now (`plug.c:7712-7716` still
            // applies): the caption glow's offscreen blur passes must run with
            // no scissor active, so the renderer owns exactly when the clip is
            // on — around the scene and the halo composite, never around the
            // blur build.
            // The preview is framed to the *export's* aspect, not the panel's
            // (EX2).
            //
            // Before aspect presets existed this was a distinction without a
            // difference: every export was 16:9 and the panel was near enough.
            // It stops being true the moment a user picks 9:16, and the failure
            // is silent in the worst way — they place a caption against a wide
            // panel, and the file it lands in is tall. Nothing else in the
            // application could have told them; the export panel prints the
            // geometry as text and the preview showed a different shape.
            //
            // The surround is filled first and in its own colour, so the letter-
            // or pillar-box reads as a deliberate frame rather than as a scene
            // that failed to fill its panel — the ASCII Field grid's old
            // behaviour, which is exactly what an unexplained inset looks like.
            if !layout.preview.is_empty() {
                let framed = layout.preview.fit_aspect(preview_aspect);
                report.preview_panel = (layout.preview.width, layout.preview.height);
                report.preview_framed = (framed.width, framed.height);
                let inset =
                    framed.width < layout.preview.width || framed.height < layout.preview.height;
                if inset {
                    let surround = ui::widgets::rectangle(ui_scale.physical_rect(layout.preview));
                    d.draw_rectangle_rec(surround, ui::theme::color::preview_surround());
                }
                let preview = ui::widgets::rectangle(ui_scale.physical_rect(framed));
                renderer.draw(&mut d, &fonts, &app.scene, &frame, assets, preview, 1.0);
                // The frame edge, drawn *after* the scene and only when there is
                // one. A capture of Pentagram Orbits pillarboxed against the
                // surround is the argument for it: both are near-black, the seam
                // is invisible, and the user is looking at a picture that does
                // not say where their video ends — which is the one thing the
                // framing exists to tell them.
                if inset {
                    d.draw_rectangle_lines_ex(
                        preview,
                        ui_scale.value().max(1.0),
                        ui::theme::color::preview_frame_edge(),
                    );
                }
            }

            let mut ui_draw = d.begin_mode2D(ui_scale.camera());
            commands = app.shell.draw(&mut ui_draw, &layout, &shell_input);

            // A one-line readout, so a headless capture carries its own evidence
            // rather than needing a separate log to be trusted.
            //
            // **Off unless asked for.** It is a developer HUD — a frame counter, a
            // band index, a consumed-sample count — and leaving it over a music
            // visualiser by default is the interface explaining itself to the wrong
            // audience. `H`, the transport row's readout button and `--hud` all turn
            // it on, and a probe run turns it on by itself: a capture that carries
            // its own evidence is the whole reason the line was written, and making
            // twenty capture sites each remember a flag would be how one of them
            // silently stops carrying it.
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
            if !app.shell.hud_visible {
                // Nothing drawn. The `readout` above is still built, because the
                // report at the end of the run prints from the same figures and a
                // probe that produced no numbers would be indistinguishable from
                // one that produced wrong ones.
            } else if layout.preview.is_empty() {
                ui::widgets::draw_text(
                    &mut ui_draw,
                    fonts.ui(),
                    &readout,
                    12.0,
                    12.0,
                    18.0,
                    Color::RAYWHITE,
                );
            } else {
                let preview = ui::widgets::rectangle(layout.preview);
                let mut scissor =
                    ui::widgets::begin_scissor(&mut ui_draw, layout.preview, ui_scale);
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
        // Drained against the current slot, not blindly (review 1.3): an edit
        // authored on another track is dropped and reported rather than written
        // through the cue id it happens to share with this one.
        let edits = app.shell.drain_lyric_edits(app.workspace.current_index());
        if !edits.is_empty() {
            let now = rl.get_time();
            if let Some(track) = app.workspace.current_mut() {
                // One snapshot per drained batch, taken *before* anything is
                // applied (UX0-B03). A batch is one user action — one frame's
                // pushes — so a cut of five cues is one Ctrl+Z, not five.
                app.shell.lyrics.record_history(&edits, &track.lyrics);
                let mut failed = None;
                // C1: counted rather than assumed. This used to call `mark_dirty`
                // unconditionally, so a batch whose *first* edit was refused —
                // the loop breaks immediately — still dirtied the project and
                // still started a 1.5 s autosave window, writing a `.musi` that
                // was byte-identical to the one already on disk. Dirty has to
                // mean "something changed", or the indicator this task adds would
                // report Unsaved for work that never happened.
                let mut applied = 0usize;
                for edit in edits {
                    if let Err(error) = edit.apply(track) {
                        failed = Some(error);
                        break;
                    }
                    applied += 1;
                }
                if applied > 0 {
                    track.mark_dirty(now);
                }
                if let Some(error) = failed {
                    app.shell.notify(
                        Severity::Warning,
                        "Lyric edit was refused",
                        &error.to_string(),
                    );
                }
            }
        }

        // Undo and redo, after the drain so a Ctrl+Z pressed in the same frame
        // as an edit reverses that edit rather than the one before it.
        if let Some(track) = app.workspace.current_mut() {
            if let Some((step, outcome)) = app.shell.lyrics.run_history_step(&mut track.lyrics) {
                let now = rl.get_time();
                let verb = match step {
                    ui::panels::lyrics::HistoryStep::Undo => "Undid",
                    ui::panels::lyrics::HistoryStep::Redo => "Redid",
                };
                match outcome {
                    Ok(label) => {
                        track.mark_dirty(now);
                        app.shell.notify(
                            Severity::Success,
                            &format!("{verb}: {label}"),
                            "Ctrl+Z steps back, Ctrl+Shift+Z steps forward.",
                        );
                    }
                    // Unreachable for a state this document was ever in, which
                    // is exactly why it is reported rather than unwrapped.
                    Err(detail) => {
                        app.shell
                            .notify(Severity::Warning, "That step could not be taken", &detail)
                    }
                }
            }
        }

        for command in commands {
            match command {
                ShellCommand::SetVolume(value) => {
                    volume = value.clamp(0.0, 1.0);
                    // Moving the slider unmutes. A slider that visibly moved while
                    // nothing became audible is the control lying about what it
                    // did, and every media player resolves it this way.
                    muted = false;
                    apply_volume(&audio, volume, muted);
                }
                ShellCommand::ToggleMute => {
                    muted = !muted;
                    apply_volume(&audio, volume, muted);
                }
                ShellCommand::SetFullscreen(on) => {
                    set_window_fullscreen(&mut rl, on, options.probe_frames.is_some());
                }
                ShellCommand::SaveUiPreferences(preferences) => {
                    if ui_preferences_editable {
                        if let Some(path) = ui_preferences_path.as_deref() {
                            if let Err(error) = ui::preferences::save(path, preferences) {
                                ui_preferences_editable = false;
                                app.shell.notify(
                                    Severity::Warning,
                                    "UI preferences could not be saved",
                                    &format!(
                                        "{}: {error}. Further changes remain session-only.",
                                        path.display()
                                    ),
                                );
                            }
                        }
                    }
                }
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
                        seek_preview(
                            music,
                            &mut analysis,
                            &mut app,
                            seconds,
                            &mut scene_clock_previous,
                        );
                    }
                }
                ShellCommand::SelectScene(id) => {
                    app.select_scene_interactive(id, time_seconds, rl.get_time());
                }
                ShellCommand::SetAutoScenes(enabled) => {
                    app.set_auto_scenes(enabled, rl.get_time());
                }
                ShellCommand::SetSetting {
                    scene,
                    index,
                    value,
                } => {
                    // `set` refuses a value the descriptor rejects, so a bad
                    // slider cannot smuggle one past the bounds.
                    if app.settings_mut().set(scene, index, value) {
                        if let Some(track) = app.workspace.current_mut() {
                            track.commit_active_cue_settings(scene);
                            track.mark_dirty(rl.get_time());
                        }
                    }
                }
                ShellCommand::ResetScene(scene) => {
                    app.settings_mut().reset_scene(scene);
                    if let Some(track) = app.workspace.current_mut() {
                        track.commit_active_cue_settings(scene);
                        track.mark_dirty(rl.get_time());
                        // review 1.8 (UX0-A08): Reset only ever touched
                        // settings, never routes, so a routed row kept showing
                        // its route's output with nothing on screen saying why
                        // the click looked like it did nothing. Routes are left
                        // alone here on purpose — clearing them would be the
                        // more destructive action, and undoing that is a later
                        // task — but the user is told which rows did not move.
                        let routed_count = track.scene_routes.scene(scene).len();
                        if let Some(message) = ui::panels::tune::reset_routed_notice(routed_count) {
                            app.shell.notify(Severity::Info, "Scene reset", &message);
                        }
                    }
                }
                ShellCommand::LoadTrack(path) => {
                    // Drop-to-open, which the welcome screen promises in so many
                    // words. It used to answer "restart with…", because the loop
                    // held the Music by shared reference and could not replace it;
                    // `open_track` owns that transition now.
                    if let Err(error) = open_track(
                        &audio,
                        &path,
                        &mut analysis,
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
                // D1's project branch. Distinct from `OpenProject`, which raises a
                // picker first; the path is already known here.
                ShellCommand::OpenDroppedProject(path) | ShellCommand::OpenRecentProject(path) => {
                    match open_project(
                        &audio,
                        &path,
                        &mut analysis,
                        &mut music,
                        &mut app,
                        &mut scratch,
                    ) {
                        Ok(()) => remember_recent_project(&mut app, &path),
                        Err(error) => {
                            app.shell
                                .notify(Severity::Error, "Project could not be opened", &error)
                        }
                    }
                }
                ShellCommand::ImportAsciiImage(path) => {
                    import_ascii_image_command(&mut app, &path, time_seconds, rl.get_time());
                }
                ShellCommand::ImportAsciiImageDialog => {
                    let dialog = FileDialog::new("Import image as ASCII")
                        .with_filter(dialogs::filters::ASCII_IMAGE);
                    match dialog.pick_file() {
                        // Cancellation is deliberately silent, as everywhere else.
                        Ok(None) => {}
                        Ok(Some(path)) => {
                            import_ascii_image_command(&mut app, &path, time_seconds, rl.get_time())
                        }
                        Err(error) => app.shell.notify(
                            Severity::Warning,
                            "No file picker is available",
                            &format!("{error}. Pass --ascii-image on the command line instead."),
                        ),
                    }
                }
                ShellCommand::ClearAsciiImage => {
                    clear_ascii_image(&mut app, rl.get_time());
                }
                ShellCommand::ForgetRecentProject(path) => {
                    if app.shell.recent.remove(&path) {
                        persist_recent_projects(&mut app);
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
                            &mut analysis,
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
                ShellCommand::ScenePlan(edit) => {
                    handle_scene_plan_edit(&mut app, edit, time_seconds, rl.get_time());
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
                        &mut analysis,
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
                    open_audio_dialog(&audio, &mut analysis, &mut music, &mut app, &mut scratch)
                }
                ShellCommand::OpenProject => {
                    let dialog = FileDialog::new("Open Musializer project")
                        .with_filter(dialogs::filters::MUSIALIZER_PROJECT);
                    match dialog.pick_file() {
                        // Cancellation is deliberately silent.
                        Ok(None) => {}
                        Ok(Some(path)) => {
                            match open_project(
                                &audio,
                                &path,
                                &mut analysis,
                                &mut music,
                                &mut app,
                                &mut scratch,
                            ) {
                                Ok(()) => remember_recent_project(&mut app, &path),
                                Err(error) => app.shell.notify(
                                    Severity::Error,
                                    "Project could not be opened",
                                    &error,
                                ),
                            }
                        }
                        Err(error) => app.shell.notify(
                            Severity::Warning,
                            "No file picker is available",
                            &format!("{error}. Pass --project on the command line instead."),
                        ),
                    }
                }
                ShellCommand::ExportLyrics => export_lyrics_command(&mut app),
                ShellCommand::ImportLyrics => {
                    let now = rl.get_time();
                    import_lyrics_command(&mut app, now);
                }
                ShellCommand::SaveProject => {
                    save_project_command(&mut app, true);
                }
                ShellCommand::SaveProjectAs => {
                    let Some(index) = app.workspace.current_index() else {
                        continue;
                    };
                    if let Some(destination) = ask_for_project_path(&mut app) {
                        match save_project_to(&mut app, index, &destination, false) {
                            Ok(()) => {
                                // A project the user just named is the one they
                                // will look for next launch, so Save As earns a
                                // place in the list the same way an open does.
                                remember_recent_project(&mut app, &destination);
                                app.shell.notify(
                                    Severity::Info,
                                    "Project saved",
                                    "Audio, ASCII imagery, lyrics, scenes, events, and output settings are durable.",
                                );
                            }
                            Err(error) => app.shell.notify(
                                Severity::Error,
                                "Project could not be saved",
                                &error,
                            ),
                        }
                    }
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
        // Sampled here because the drawing pair has closed: inside one, a texture
        // mode is allowed to have moved rlgl's framebuffer size, and out here it
        // must have been put back.
        report.framebuffer.observe();

        // Autosave, polled after the frame like the C's (`plug.c:7580-7583`).
        //
        // **Every due track, not only the current one (C4).** The loop always
        // computed the full due list and then threw all but the current entry
        // away, because the sample rate came off the bound stream — so a
        // background track dirtied by a project open, an Assist run or a
        // multi-track session simply never autosaved, with nothing on screen
        // saying so. `Track::audio_sample_rate` removed that coupling.
        let now = rl.get_time();
        // The draft guard is per track, not global: a half-typed cue on track A
        // must not stop track B's autosave, and must stop A's — writing a `.musi`
        // mid-draft would persist a document the user has not committed to.
        let due = project::autosave_due_tracks(&app.workspace, now, |index| {
            app.shell
                .editor_draft_blocks_autosave(&app.workspace, index)
        });
        for index in due {
            let Some(path) = app
                .workspace
                .get(index)
                .and_then(|track| track.project_path.clone())
            else {
                continue;
            };
            // A failure latches `project_autosave_failed`, which stops the retry
            // until the next edit clears it — so this cannot become a loop that
            // writes every frame. One track's failure does not `break`: the
            // others are independent files and there is no reason a full disk
            // under one destination should silence a save to another.
            if let Err(error) = save_project_to(&mut app, index, &path, true) {
                // Autosave used to discard this with `let _ =`. A save the user
                // never asked for is still a save they are relying on, and the
                // whole point of the latch is that it will not try again — so
                // silence here means the work stops being written and nothing
                // ever says why (UX0-B01).
                let name = app.workspace.get(index).map_or_else(
                    || path.display().to_string(),
                    |t| t.display_name().to_string(),
                );
                app.shell.notify(
                    Severity::Error,
                    "Autosave failed",
                    &format!("{name}: {error}. Edit again to retry, or use Save As."),
                );
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
                        &mut analysis,
                        &mut music,
                        &mut app,
                        &mut scratch,
                        true,
                    )
                    .and_then(|index| {
                        select_track(
                            &audio,
                            index,
                            &mut analysis,
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
                    // honours the path given. The GPU readback and PNG encoder
                    // are synchronous, so pause/refill the preview around them.
                    with_preview_paused(music.as_ref(), || {
                        let image = rl.load_image_from_screen(&thread);
                        image.export_image(path_str);
                    });
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
        analysis.analyzer.band_count(),
        requested_scene,
        &fonts,
        &renderer,
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
/// The two pieces of per-stream analysis state, which must be reset together.
///
/// They are one type because separating them is the bug this exists to prevent.
/// The C keeps `analyzer` and `beat_tracker` adjacent on `Plug` and resets both in
/// the same two functions (`fft_reset` `plug.c:443-449`, `fft_clean` `:473-478`).
/// Here they were separate, only the analyzer was threaded through the eight
/// functions that rebind audio, and the beat tracker was left with **no caller at
/// all** — fully ported, unit-tested, and never run, while `beat_phase` stayed
/// hardcoded to `0.0` in the frame loop and the CLI went on advertising
/// `--route parameter:beat_phase:...` as a working source.
///
/// Pairing them makes "reset both" structural instead of remembered: there is no
/// longer a way to reconfigure the analyzer without the tracker following, because
/// [`Self::reconfigure`] is the only way to do it.
struct Analysis {
    /// 200 KiB of arrays, so it is boxed.
    analyzer: Box<AudioAnalyzer>,
    beat: musializer_core::audio::beat_tracker::BeatTracker,
}

impl Analysis {
    /// The pre-track state, at the idle configuration (`analyzer_configure`'s
    /// starting point; the real rate arrives with the first stream).
    fn idle() -> Result<Self, String> {
        Ok(Self {
            analyzer: AudioAnalyzer::boxed(AudioAnalyzerConfig::idle())
                .map_err(|error| format!("could not create the analyzer: {error}"))?,
            beat: musializer_core::audio::beat_tracker::BeatTracker::default(),
        })
    }

    /// Rebinds the analyzer and resets the beat tracker with it (`fft_reset`).
    ///
    /// A tempo learned from the previous track must not carry into this one, and
    /// the anchor is an absolute time that the new stream's clock does not share.
    fn reconfigure(&mut self, config: AudioAnalyzerConfig) -> Result<(), String> {
        *self.analyzer = *AudioAnalyzer::boxed(config)
            .map_err(|error| format!("could not configure the analyzer: {error}"))?;
        self.beat.reset();
        Ok(())
    }
}

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
    /// Where the welcome screen's recent-project list lives, or `None` when no
    /// per-user configuration directory could be derived (UX0-C06).
    recent_path: Option<PathBuf>,
    /// False when that store was refused at startup. Writes are then refused for
    /// the rest of the session, for the same reason [`Self::presets_editable`]
    /// exists: a file the user might still repair is never overwritten by one
    /// this process reconstructed from nothing.
    recent_editable: bool,
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

    /// What a scene click means while an automatic plan is running (LX3-a).
    ///
    /// The frozen C has one answer for a scene selection: it becomes the
    /// track's base scene, and the retained plan is switched off as a side
    /// effect (`track_select_base_scene`, `plug.c:963-977`, reproduced in
    /// [`Track::select_base_scene`]). With a plan authored that is a trap, and
    /// the operator hit both halves of it in one session: clicking a scene to
    /// change the segment they were looking at stopped the plan driving — so
    /// the whole track then previewed one scene, and every later tuning edit
    /// landed in the track-wide table that every segment of that scene kind
    /// falls back to. Two reported bugs, one line of cause.
    ///
    /// So while the plan is enabled and non-empty, a scene selection retargets
    /// **one segment** and leaves the plan running. The segment is the one
    /// selected in the lane when there is one, otherwise the one under the
    /// playhead, and the notice names it with its position and span — "which
    /// one did that apply to" was the other half of the report.
    ///
    /// `--scene` on the command line is deliberately *not* routed here.
    /// The CLI grammar is a documented contract (`docs/PHASE0_INVENTORY.md`)
    /// and `--scene X` means "start on X"; it keeps calling
    /// [`App::select_scene`] directly.
    fn select_scene_interactive(
        &mut self,
        id: SceneId,
        time_seconds: f64,
        now_seconds: f64,
    ) -> bool {
        let Some(track) = self.workspace.current() else {
            return self.select_scene(id, now_seconds);
        };
        if !track.scene_switches.enabled || track.scene_switches.is_empty() {
            return self.select_scene(id, now_seconds);
        }

        let cues = track.scene_switches.cues();
        let total = cues.len();
        // An explicit lane selection beats the playhead: it is the more recent
        // statement of intent, and it is the only way to edit a segment you are
        // not currently listening to.
        let selected = self.shell.scene_lane.selected_id;
        let index = cues
            .iter()
            .position(|cue| cue.id == selected)
            .or_else(|| {
                cues.iter().position(|cue| {
                    cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds
                })
            })
            .unwrap_or(total - 1);
        let cue = cues[index];

        // Retargeting a segment to the scene it already uses would still
        // recapture its snapshot from the track-wide table, silently throwing
        // away tuning the user captured into it. Say nothing happened instead.
        if cue.scene_index == id.index() as u32 {
            self.shell.notify(
                Severity::Info,
                "Segment already uses that scene",
                &format!(
                    "Segment {} of {total} is {} from {} to {}.",
                    index + 1,
                    id.display_name(),
                    ui::widgets::format_timestamp(cue.start_seconds),
                    ui::widgets::format_timestamp(cue.end_seconds)
                ),
            );
            return false;
        }

        let result = self
            .workspace
            .current_mut()
            .expect("the track was present above")
            .retarget_scene_cue(cue.id, id);
        match result {
            Err(error) => {
                self.shell.notify(
                    Severity::Warning,
                    "Segment could not be retargeted",
                    &error.to_string(),
                );
                false
            }
            Ok(()) => {
                if let Some(track) = self.workspace.current_mut() {
                    track.mark_dirty(now_seconds);
                }
                // `retarget_scene_cue` rewinds the plan cursor, so this rebinds
                // the live scene when the retargeted segment is the one playing
                // and leaves it alone when it is not.
                self.apply_auto_scene_switch(time_seconds);
                self.shell.notify(
                    Severity::Success,
                    "Segment scene changed",
                    &format!(
                        "{} now plays segment {} of {total} ({} to {}). Turn Auto off to change the track's base scene instead.",
                        id.display_name(),
                        index + 1,
                        ui::widgets::format_timestamp(cue.start_seconds),
                        ui::widgets::format_timestamp(cue.end_seconds)
                    ),
                );
                true
            }
        }
    }

    /// Binds a scene, seeded from the current track (`scene_seed_for_track`,
    /// `plug.c:611-614`; used at `:987` and `:1354`).
    ///
    /// The base-scene primitive. Interactive selections go through
    /// [`App::select_scene_interactive`], which sends them to a segment instead
    /// while an automatic plan is driving.
    fn select_scene(&mut self, id: SceneId, now_seconds: f64) -> bool {
        if self.scene.id() == id {
            return false;
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
            let disabled_plan = track.scene_switches.enabled && !track.scene_switches.is_empty();
            let kept_cues = track.scene_switches.len();
            track.select_base_scene(id);
            track.mark_dirty(now_seconds);
            let detail = if disabled_plan {
                format!(
                    "{} is now the track's base scene. Auto scenes was turned off; its {kept_cues} cues are kept for when you re-enable it.",
                    id.display_name()
                )
            } else if kept_cues > 0 {
                // The plan was already off, so nothing was taken away — but the
                // lane still shows segments, and a base-scene notice beside a
                // full lane reads as though the two disagree. Naming the count
                // says which one the preview is obeying (LX3-c).
                format!(
                    "{} is now the track's base scene. Automatic scenes is off, so its {kept_cues} segments are not driving the preview.",
                    id.display_name()
                )
            } else {
                format!("{} is now the track's base scene.", id.display_name())
            };
            self.shell
                .notify(Severity::Info, "Base scene changed", &detail);
        }
        true
    }

    /// Applies the Assist header's automatic-scene toggle
    /// (`plug.c:2207-2226`). The track model owns the retained plan and cursor;
    /// this composition root owns the live scene instance and user notice.
    fn set_auto_scenes(&mut self, enabled: bool, now_seconds: f64) -> bool {
        let Some(track) = self.workspace.current_mut() else {
            self.shell.notify(
                Severity::Warning,
                "Automatic scenes unavailable",
                "Open a project with an automatic scene plan first.",
            );
            return false;
        };
        let base_scene = track.base_scene;
        let seed = track.scene_seed;
        let cue_count = track.scene_switches.len();
        if !track.set_auto_scenes(enabled) {
            self.shell.notify(
                Severity::Warning,
                "Automatic scenes unavailable",
                "Add or apply at least one scene cue before enabling the plan.",
            );
            return false;
        }
        track.mark_dirty(now_seconds);

        if !enabled {
            self.scene = SceneInstance::new(scene_host::descriptor(base_scene), seed);
        }
        self.shell.notify(
            Severity::Info,
            if enabled {
                "Automatic scenes enabled"
            } else {
                "Automatic scenes disabled"
            },
            &format!(
                "The retained {cue_count}-cue plan is {}.",
                if enabled {
                    "driving preview and export"
                } else {
                    "off; the base scene is restored"
                }
            ),
        );
        true
    }

    /// Binds the scene selected by the current track's automatic plan without
    /// changing that track's base-scene selection (`apply_auto_scene_switch`,
    /// `plug.c:997-1023`).
    fn apply_auto_scene_switch(&mut self, time_seconds: f64) {
        let Some(track) = self.workspace.current_mut() else {
            return;
        };
        let seed = track.scene_seed;
        let Some(scene) = track.advance_scene_plan(time_seconds) else {
            return;
        };
        if self.scene.id() != scene {
            self.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
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
    // C4: every track gets its audio format here, whether or not it is ever the
    // current one. `bind_audio` also caches it from the opened stream, but only
    // the *bound* track goes through that — a second track added with "Add audio"
    // and left in the background would otherwise carry 0 Hz and refuse to save,
    // which is the exact gap all-track autosave exists to close. This decode
    // already runs for every track at load, so the numbers are free.
    track.audio_sample_rate = decoded.sample_rate;
    track.audio_channels = u16::try_from(decoded.channels).unwrap_or(2);
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
fn import_ascii_image(
    app: &mut App,
    path: &Path,
    now_seconds: f64,
) -> Result<(usize, usize), String> {
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
            //
            // C1: this used to assign `project_dirty = true` directly, which is
            // not the same thing and failed in two ways. It never moved
            // `project_dirty_since`, so the 1.5 s settle was measured from a
            // stale instant — usually 0.0, making the write due on the very next
            // frame instead of after the import settled. And it never cleared
            // `project_autosave_failed`, so an import after any failed save was
            // never autosaved at all, silently.
            track.mark_dirty(now_seconds);
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
    analysis: &mut Analysis,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<usize, String> {
    let path_str = path.to_str().ok_or("audio path is not valid UTF-8")?;
    let (base_scene, seed) = app
        .workspace
        .inherited_scene(app.scene.id(), app.scene.seed());
    let mut track = with_preview_paused(music.as_ref(), || {
        // Opened before anything is mutated, so an unreadable file leaves the
        // session exactly as it was rather than half-adding a track. This one is
        // only a metadata probe — the stream that plays is opened by
        // `bind_current_audio` — which is the same split as C's `metadata_probe`
        // (`plug.c:4901-4913`).
        let probe = audio
            .new_music(path_str)
            .map_err(|error| error.to_string())?;
        let duration = f64::from(probe.get_time_length());
        drop(probe);

        let mut track = Track::new(path.to_path_buf(), duration, base_scene, seed)
            .map_err(|error| format!("could not prepare the track: {error}"))?;
        track.transport_seekable =
            musializer_core::timing::track_timeline::path_is_seekable(Some(path_str));
        // At load, before the track is in the workspace, which is where the C
        // does it too (`plug.c:820`, inside `add_track` and before the count is
        // bumped). The whole-file decode is why playback is paused around this
        // preparation transaction.
        load_timeline_waveform(audio, &mut track);
        Ok::<Track, String>(track)
    })?;

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
        bind_current_audio(audio, analysis, music, app, scratch, play)?;
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
    analysis: &mut Analysis,
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
    bind_audio(opened, analysis, music, app, scratch, play)
}

/// Takes the window in or out of fullscreen, unless this is a probe run.
///
/// The `headless` guard is not caution, it is a rule this repository has already
/// paid for. Probe runs happen on a private Xvfb with no window manager; asking
/// for a fullscreen toggle there means asking a compositor that is not present to
/// restack the window, and the size the capture then photographs is not the size
/// it was asked for. The shell's own layout flag has already switched either way,
/// so `--ui-probe fullscreen=1` still photographs the expanded workspace — which
/// is the thing a capture is for.
///
/// raylib's `ToggleFullscreen` is a real display-mode change rather than a
/// borderless resize, which is why it is a command handled out here with
/// `&mut RaylibHandle` rather than something the shell could do for itself.
fn set_window_fullscreen(rl: &mut RaylibHandle, on: bool, headless: bool) {
    if headless || rl.is_window_fullscreen() == on {
        return;
    }
    rl.toggle_fullscreen();
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
    analysis: &mut Analysis,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
    play: bool,
) -> Result<(), String> {
    close_audio(music, app, scratch);

    let file_sample_rate = opened.stream.sampleRate;
    if let Some(track) = app.workspace.current_mut() {
        track.scene_switches.reset();
        track.cue_settings_active = false;
        // C4: cached the moment the stream is opened, so this track stays
        // saveable after it stops being the current one. Reading it here rather
        // than at save time is what decouples `.musi` writing from the audio
        // device — see `save_project_to`.
        track.audio_sample_rate = file_sample_rate;
        track.audio_channels = opened.stream.channels as u16;
    }

    analysis.reconfigure(AudioAnalyzerConfig::preview(file_sample_rate))?;
    // SAFETY: the bridge is installed in `run` before this can be called, the
    // audio device is initialized (it is what produced `opened`), and the stream
    // outlives the attachment because `close_audio` — reached from the next
    // `bind_current_audio` and from `run`'s shutdown — detaches before dropping
    // it.
    unsafe { audio_bridge::attach(opened.stream) }
        .map_err(|error| format!("could not attach the audio bridge: {error}"))?;
    // Both halves begin marked processed. Fill them before playback starts so
    // the device never races the first rendered frame for actual PCM.
    opened.update_stream();
    opened.play_stream();
    if !play {
        // A parked probe is a paused live stream, not a never-started buffer.
        // That distinction matters because raylib only lets the seek transaction
        // reset a paused buffer after it has been resumed.
        opened.pause_stream();
    }

    app.shell
        .reset_timeline(f64::from(opened.get_time_length()));
    *music = Some(opened);
    Ok(())
}

/// Performs one transport discontinuity as an indivisible stream transaction.
fn seek_preview(
    music: &Music<'_>,
    analysis: &mut Analysis,
    app: &mut App,
    seconds: f64,
    scene_clock_previous: &mut Option<f64>,
) {
    let Some(track) = app.workspace.current() else {
        return;
    };
    if !track.transport_seekable {
        return;
    }
    let target = musializer_core::timing::track_timeline::seek_relative(
        0.0,
        seconds,
        track.duration_seconds,
    );
    let was_playing = music.is_stream_playing();
    // raylib only resets a paused AudioBuffer from its playing state. This
    // seemingly redundant resume is therefore required before StopMusicStream,
    // and mirrors `seek_track_to` in the frozen build.
    if !was_playing {
        music.resume_stream();
    }
    music.stop_stream();
    music.seek_stream(target as f32);
    music.update_stream();

    // The stream is stopped, so the callback cannot race either reset. Queued
    // samples and analyzer smoothing both describe the old playhead and must be
    // discarded together.
    if let Some(ring) = audio_bridge::ring() {
        ring.reset();
    }
    analysis.analyzer.reset();
    analysis.beat.reset();
    *scene_clock_previous = None;
    if let Some(track) = app.workspace.current_mut() {
        track.scene_switches.reset();
        track.cue_settings_active = false;
    }

    music.play_stream();
    if !was_playing {
        music.pause_stream();
    }
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
    app.shell.reset_timeline(0.0);
}

/// Asks for an audio file and opens it, reporting either failure in the tray.
///
/// Called from outside the drawing pair: the picker is modal and blocks until the
/// user answers.
fn open_audio_dialog<'audio>(
    audio: &'audio RaylibAudio,
    analysis: &mut Analysis,
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
            if let Err(error) = open_track(audio, &path, analysis, music, app, scratch, true) {
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
    let mut apply_scene_plan = false;
    match action {
        Action::Record(event) => {
            if track.record_manual_event(event).is_ok() {
                track.mark_dirty(now);
            }
        }
        Action::RecordSceneCue => match track.record_scene_cue(scene, time) {
            Ok(()) => {
                track.mark_dirty(now);
                apply_scene_plan = true;
            }
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
    if apply_scene_plan {
        app.apply_auto_scene_switch(time);
        app.shell.notify(
            Severity::Success,
            "Scene cue captured",
            "The scene and its current tuning will load at this playhead position.",
        );
    }
}

/// Applies one command from the scene-plan lane. The lane selects by stable id;
/// this boundary resolves it against the current track, marks the project dirty
/// exactly once, and refreshes the live auto-scene state after a successful edit.
fn handle_scene_plan_edit(
    app: &mut App,
    edit: ui::panels::scene_timeline::ScenePlanEdit,
    time: f64,
    now: f64,
) {
    use ui::panels::scene_timeline::ScenePlanEdit as Edit;

    if let Edit::SetEnabled(enabled) = edit {
        app.set_auto_scenes(enabled, now);
        if enabled {
            app.apply_auto_scene_switch(time);
        }
        return;
    }

    let live_scene = app.scene.id();
    let result = {
        let Some(track) = app.workspace.current_mut() else {
            return;
        };
        let result = match edit {
            Edit::SplitAt { seconds } => track.record_scene_cue(live_scene, seconds),
            Edit::RetimeBoundary {
                right_cue_id,
                seconds,
            } => track.retime_scene_cue(right_cue_id, seconds),
            Edit::Retarget { cue_id, scene } => track.retarget_scene_cue(cue_id, scene),
            Edit::CaptureTuning { cue_id } => track.capture_scene_cue_settings(cue_id),
            Edit::Remove { cue_id } => track.remove_scene_cue(cue_id),
            Edit::SetEnabled(_) => unreachable!("handled above"),
        };
        if result.is_ok() {
            track.mark_dirty(now);
        }
        result
    };

    match result {
        Err(error) => app.shell.notify(
            Severity::Warning,
            "Scene plan edit was refused",
            &error.to_string(),
        ),
        Ok(()) => {
            let enabled = app
                .workspace
                .current()
                .is_some_and(|track| track.scene_switches.enabled);
            if enabled {
                app.apply_auto_scene_switch(time);
            } else if let Some(track) = app.workspace.current() {
                app.scene =
                    SceneInstance::new(scene_host::descriptor(track.base_scene), track.scene_seed);
            }
            app.shell.notify(
                Severity::Success,
                "Scene plan updated",
                match edit {
                    Edit::SplitAt { .. } => "A segment now begins at the playhead.",
                    Edit::RetimeBoundary { .. } => "The shared scene boundary was moved.",
                    Edit::Retarget { .. } => "The selected segment now uses the chosen scene.",
                    Edit::CaptureTuning { .. } => {
                        "The selected segment captured the current tuning."
                    }
                    Edit::Remove { .. } => "The segment was removed and its span was merged.",
                    Edit::SetEnabled(_) => unreachable!("handled above"),
                },
            );
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
                    if let Some(track) = app.workspace.current_mut() {
                        track.commit_active_cue_settings(scene);
                        track.mark_dirty(now);
                    }
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
    // review 1.3 (UX0-A03). The lyric draft belongs here rather than behind its
    // own modal: quitting is already one confirmation over six conditions
    // (`plug.c:7215-7220`), and a second prompt for the draft would ask the user
    // twice about the same decision. Guarding quit the way the track row is
    // guarded would be worse still — refusing to close an application is not a
    // reasonable answer to a half-typed lyric.
    let lyric_draft = app.shell.lyric_draft_is_dirty(&app.workspace);
    // The remaining conditions the C weighs — staged Assist suggestions, a
    // running analysis and a running export — arrive through `assist` and
    // `exporting`.
    if dirty == 0
        && !lyric_draft
        && !route_edit
        && !exporting
        && !app.workspace.assist.blocks_close()
    {
        return true;
    }
    // The C builds this list line by line from six conditions (`plug.c:7222-7247`).
    // Three of the six — staged Assist suggestions, a running analysis and a
    // running export — belong to Agents H and J and each adds a line here.
    let mut items = String::new();
    // The C's order puts the draft first (`plug.c:7226-7228`), and the words are
    // its words.
    if lyric_draft {
        items.push_str("\n- Apply or discard the active lyric draft.");
    }
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
    analysis: &mut Analysis,
    music: &mut Option<Music<'audio>>,
    app: &mut App,
    scratch: &mut [f32],
) -> Result<(), String> {
    let opened = with_preview_paused(music.as_ref(), || {
        let mut opened = project::open_path(path, |audio_path| {
            // A metadata probe, exactly as C's `LoadMusicStream` at `plug.c:4901`
            // is: the stream that plays is opened by `bind_current_audio`.
            let probe = open_music(audio, audio_path)?;
            Ok(f64::from(probe.get_time_length()))
        })
        .map_err(|error| error.to_string())?;

        // The same load-time preprocessing a plain audio file gets. Keep the
        // currently playing preview paused across the whole-file decode.
        load_timeline_waveform(audio, &mut opened.track);
        if let Some(image) = opened.track.ascii.as_mut() {
            decode_ascii_grid(image);
        }
        Ok::<_, String>(opened)
    })?;

    if let Some(project::OpenWarning::LegacyAudioPath) = opened.warning {
        app.shell.notify(
            Severity::Warning,
            "Project used a legacy asset path",
            "Audio was found in the launch directory because it was not beside the project. Move it beside the project or use an absolute path for portability.",
        );
    }

    let track = opened.track;
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
        bind_current_audio(audio, analysis, music, app, scratch, true)?;
    } else {
        select_track(audio, index, analysis, music, app, scratch)?;
    }
    // `lyric_editor_clear_draft()` at `plug.c:5037`. The opened project's track
    // is current now, so any draft still bound to the previous slot has to go
    // (review 1.3); the click that got here was guarded, so it is not lost work.
    app.shell.lyrics.enter_track(app.workspace.current_index());
    app.shell.notify(
        Severity::Info,
        "Project opened",
        "Lyrics, embedded semantic cues, authored lanes, scene plan, and output settings were restored.",
    );
    Ok(())
}

/// Whether this invocation is a user session rather than a batch job (UX0-C06).
///
/// `--probe-frames` exits after a fixed frame count, `--render` after an export
/// and `--save-project` after a write; none of the three is somebody sitting in
/// front of the application, and none should leave a mark in the operator's
/// per-user configuration. `--save-project` in particular is how this
/// repository's own gate manufactures its `.musi` fixtures.
fn is_session_run(options: &Cli) -> bool {
    options.probe_frames.is_none() && options.render.is_none() && options.save_project.is_none()
}

/// Puts `path` at the top of the welcome screen's recent list and persists it
/// (UX0-C06).
///
/// The name is the *track's* display name, which prefers the project's own title
/// over its filename — so the list reads as the user's names for their work
/// rather than as a directory listing. Called after the open or save has
/// succeeded, never before: a list that remembered projects that failed to open
/// would be a list of dead ends.
fn remember_recent_project(app: &mut App, path: &Path) {
    // The *project's* title, then the project file's own stem — not the track's
    // display name, whose fallback is the audio filename. This list names
    // projects, so a `.musi` called `night-drive` with an untitled bundled
    // `source.wav` has to read as "night-drive", which is the name the user
    // chose. `Track::display_name` answers the other question and answered this
    // one wrongly, which a smoke run showed as an entry called "source".
    let name = app
        .workspace
        .current()
        .and_then(|track| track.project_metadata.as_ref())
        .map(|metadata| metadata.title.clone())
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Untitled project".to_string());
    // Absolute, so the same project opened as `./song.musi` and as a full path is
    // one entry. `absolute` is purely lexical — it does not resolve symlinks or
    // require the file to exist — which is what keeps it usable here.
    let normalized = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    app.shell
        .recent
        .record(normalized, name, ui::preferences::recent::now_unix());
    app.shell.recent.probe(|path| path.is_file());
    persist_recent_projects(app);
}

/// Writes the recent list, once, and drops to session-only on the first failure.
fn persist_recent_projects(app: &mut App) {
    if !app.recent_editable {
        return;
    }
    let Some(path) = app.recent_path.clone() else {
        return;
    };
    if let Err(error) = ui::preferences::recent::save(&path, &app.shell.recent) {
        app.recent_editable = false;
        app.shell.notify(
            Severity::Warning,
            "Recent projects could not be saved",
            &format!(
                "{}: {error}. The list is session-only for the rest of this run.",
                path.display()
            ),
        );
    }
}

/// The D1/D2 image branch: import, select ASCII Field, mark dirty, and say which
/// of the two things happened (`plug.c:7548-7559`).
///
/// The staged case is the one worth having a distinct message for. An image
/// dropped before any audio exists is *kept* — it is handed to whichever track
/// opens next (`plug.c:825-839`) — and with no track there is nothing on screen
/// to show that it worked, so silence would read as the drop having been ignored.
fn import_ascii_image_command(app: &mut App, path: &Path, time_seconds: f64, now: f64) {
    // `import_ascii_image` marks the track dirty itself (C1), so the command
    // only selects the scene and says which of the two things happened.
    match import_ascii_image(app, path, now) {
        Ok((columns, rows)) => {
            // Selected on success only, which is the oracle's `&&`
            // (`plug.c:7552`): switching to an empty ASCII Field after a failed
            // decode would replace the user's scene with a worse one as the
            // reward for a typo.
            app.select_scene_interactive(SceneId::AsciiField, time_seconds, now);
            match app.workspace.current() {
                Some(_) => {
                    app.shell.notify(
                        Severity::Info,
                        "Image imported",
                        &format!("ASCII Field is drawing a {columns}x{rows} glyph grid."),
                    );
                }
                None => app.shell.notify(
                    Severity::Info,
                    "ASCII image staged",
                    &format!(
                        "A {columns}x{rows} glyph grid is waiting. Open an audio track when you are ready to preview it."
                    ),
                ),
            }
        }
        Err(detail) => app.shell.notify(
            Severity::Error,
            "Image could not be imported",
            &format!("{detail}. Use a valid PNG, JPEG, or BMP image."),
        ),
    }
}

/// The D2 clear: path, digest, cells and dimensions go together
/// (`plug.c:6386-6393`).
///
/// "Together" is structural rather than a discipline four assignments have to
/// keep: they are the four fields of one [`workspace::AsciiImage`], so dropping
/// the `Option` is the only way to drop any of them.
fn clear_ascii_image(app: &mut App, now: f64) {
    let cleared = match app.workspace.current_mut() {
        Some(track) if track.ascii.is_some() => {
            track.ascii = None;
            track.mark_dirty(now);
            true
        }
        _ => false,
    };
    if cleared {
        app.shell.notify(
            Severity::Info,
            "Image cleared",
            "ASCII Field is back to its procedural spectrogram.",
        );
    }
}

/// Saves one track's project to `destination`, by workspace slot.
///
/// **By slot, and with no `Music`, is the whole of C4.** The C reads the sample
/// rate and channel count off the live stream (`plug.c:4304-4306`), and so did
/// this — which meant a track had to be the *current* one to be saveable at all,
/// and autosave silently skipped every background track that a project open had
/// left dirty. `Track::audio_sample_rate` caches those two numbers at load, so
/// the audio device is no longer in the save path.
fn save_project_to(
    app: &mut App,
    index: usize,
    destination: &Path,
    reuse_published: bool,
) -> Result<(), String> {
    let track = app
        .workspace
        .get_mut(index)
        .ok_or("there is no track to save")?;
    project::save_to_path(track, destination, reuse_published).map_err(|error| error.to_string())
}

/// Writes the current track's cue document to a `.lyrics.tsv` (D3).
///
/// `LyricsDocument::bridge_export` is the codec — the UI bridge the C uses for
/// exactly this (`lyrics.c:603-648`), canonical and locale-independent by
/// construction. No second codec is invented here, and none should be.
///
/// **Exporting must not dirty the project**, which is the one requirement worth
/// naming: writing a copy of what is already in the `.musi` changes nothing
/// about the `.musi`, and a Save prompt after an export would teach the user
/// that exporting costs them something.
fn export_lyrics_command(app: &mut App) {
    let Some(track) = app.workspace.current() else {
        return;
    };
    let body = match track.lyrics.bridge_export() {
        Ok(body) => body,
        Err(error) => {
            app.shell.notify(
                Severity::Warning,
                "These cues cannot be exported",
                &format!("{error}."),
            );
            return;
        }
    };
    // Seeded from the project's own name, so the two files sit together and the
    // user is not asked to invent a name for a derived artifact.
    let suggested = track.project_path.as_ref().map_or_else(
        || PathBuf::from("lyrics.lyrics.tsv"),
        |path| path.with_extension("lyrics.tsv"),
    );
    let dialog = dialogs::FileDialog::new("Export timed lyrics")
        .with_default_path(suggested)
        .with_filter(dialogs::filters::LYRIC_TEXT);
    match dialog.save_file() {
        // Cancellation is silent, as it is at every other dialog here.
        Ok(None) => {}
        Ok(Some(path)) => match std::fs::write(&path, body.as_bytes()) {
            Ok(()) => app.shell.notify(
                Severity::Success,
                "Timed lyrics exported",
                &format!(
                    "{} cue(s) written. The project is unchanged.",
                    app.workspace
                        .current()
                        .map_or(0, |track| track.lyrics.len())
                ),
            ),
            Err(error) => app.shell.notify(
                Severity::Error,
                "The lyrics file could not be written",
                &format!("{}: {error}", path.display()),
            ),
        },
        Err(error) => app.shell.notify(
            Severity::Warning,
            "No file picker is available",
            &format!("{error}. There is no command-line route to this yet."),
        ),
    }
}

/// Replaces the current track's cue document from a `.lyrics.tsv` (D3).
///
/// Transactional in three layers, because an import that half-lands is worse
/// than one that refuses. `bridge_import` stages into a fresh document and
/// publishes through `replace`, so a malformed file never touches anything;
/// `normalize_duration` then re-bases the imported cues onto *this* track's
/// decoded length and refuses outright if a cue begins past the end, because
/// clamping those would produce zero-length cues rather than shorter ones; and
/// only if both succeed is the result written, with the previous document
/// already on the undo stack.
fn import_lyrics_command(app: &mut App, now: f64) {
    if app.workspace.current().is_none() {
        return;
    }
    let dialog =
        dialogs::FileDialog::new("Import timed lyrics").with_filter(dialogs::filters::LYRIC_TEXT);
    let path = match dialog.pick_file() {
        Ok(None) => return,
        Ok(Some(path)) => path,
        Err(error) => {
            app.shell.notify(
                Severity::Warning,
                "No file picker is available",
                &format!("{error}. There is no command-line route to this yet."),
            );
            return;
        }
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            app.shell.notify(
                Severity::Error,
                "That file could not be read",
                &format!("{}: {error}", path.display()),
            );
            return;
        }
    };
    let Some(track) = app.workspace.current_mut() else {
        return;
    };
    let duration = track.lyrics.duration_seconds();

    // The whole transaction is `import_bridge_document`, which is pure and
    // tested: staged into a scratch document so malformed bytes touch nothing,
    // then re-based off the *file's* duration onto this track's. Nothing is
    // written here until it has all succeeded.
    let normalized = match lyrics::import_bridge_document(&bytes, duration) {
        Ok(document) => document,
        Err(lyrics::BridgeImportRefusal::NoTrackLength(error)) => {
            app.shell.notify(
                Severity::Warning,
                "This track has no length to import against",
                &format!("{error}."),
            );
            return;
        }
        Err(lyrics::BridgeImportRefusal::Format(error)) => {
            app.shell.notify(
                Severity::Warning,
                "That is not a timed-lyrics file",
                &format!(
                    "{error}. It must be a .lyrics.tsv written by Export, starting MUSIALIZER-LYRICS-BRIDGE."
                ),
            );
            return;
        }
        Err(lyrics::BridgeImportRefusal::DoesNotFit {
            failure,
            source_duration_seconds,
        }) => {
            app.shell.notify(
                Severity::Warning,
                "Those cues do not fit this track",
                &format!(
                    "{failure}. The file was timed against a {source_duration_seconds:.1} s track and this one is {duration:.1} s."
                ),
            );
            return;
        }
    };
    let imported = normalized.len();
    // The old document goes on the undo stack before it is replaced, so an
    // import over hand-timed work is one Ctrl+Z away from being back.
    app.shell
        .lyrics
        .record_history_label("Import lyrics", &track.lyrics);
    if let Err(failure) = track.lyrics.replace(&normalized) {
        app.shell.notify(
            Severity::Warning,
            "Those cues were refused",
            &format!("{failure}."),
        );
        return;
    }
    track.mark_dirty(now);
    app.shell.lyrics.enter_document_change();
    app.shell.notify(
        Severity::Success,
        "Timed lyrics imported",
        &format!("{imported} cue(s) replaced the previous document. Ctrl+Z puts it back."),
    );
}

/// The Save button (`save_project`, `plug.c:4641-4646`): saves in place when the
/// track has a project path, and otherwise falls through to Save As.
fn save_project_command(app: &mut App, ask_if_unnamed: bool) {
    let Some(index) = app.workspace.current_index() else {
        return;
    };
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
    match save_project_to(app, index, &destination, false) {
        Ok(()) => {
            remember_recent_project(app, &destination);
            app.shell.notify(
                Severity::Info,
                "Project saved",
                "Audio, ASCII imagery, lyrics, scenes, events, and output settings are durable.",
            );
        }
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
/// The C runs `lyric_editor_allow_context_change` and
/// `route_editor_allow_active_context_change` before any of this
/// (`plug.c:5263-5264`). The lyric half now runs at the click site in
/// `ui::shell`, because a refused guard must leave the selection untouched and
/// the command must never be emitted at all; what happens here is the other half
/// of `plug.c:5274` — rebinding the editor once the switch has succeeded, so the
/// draft can never be left pointing at the document it no longer edits
/// (review 1.3).
fn select_track<'audio>(
    audio: &'audio RaylibAudio,
    index: usize,
    analysis: &mut Analysis,
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
    // `lyric_editor_clear_draft()` at `plug.c:5274`, and the reason a stale
    // binding is unreachable rather than merely guarded against (review 1.3).
    app.shell.lyrics.enter_track(Some(index));
    bind_audio(opened, analysis, music, app, scratch, true)
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
    /// The range of `beat_phase` seen across the run.
    ///
    /// A *range*, not a last value, because that is the difference between "the
    /// beat tracker ran" and "the beat tracker advanced". The bug this line exists
    /// for left `beat_phase` at a constant `0.0`, which any single-sample readout
    /// would have reported as a perfectly plausible phase.
    beat_phase_low: f32,
    beat_phase_high: f32,
    beat_interval_last: f64,
    beat_intervals_learned: usize,
    onsets_seen: u64,
    flux_seen: f32,
    /// Project-owned evidence attached to the most recently drawn preview frame.
    frame_lanes: FrameLaneStatus,
    /// The last frame's logical window, kept so `chrome:` can be printed after
    /// the window has closed. The chrome line is what proves a save affordance
    /// was on screen in every panel configuration (review 1.12).
    logical_window: (f32, f32),
    /// The last drawn frame's shared timeline span, kept for the same reason
    /// and by the same trick as `logical_window` above: `close_audio` resets the
    /// view to nothing on the way out, so reading it off the shell at print time
    /// reports `0.000x` for every run (LX2).
    timeline: String,
    /// Whether each frame ended with rlgl's framebuffer size still equal to the
    /// window's render size. A frame that ends out of sync draws the next scene
    /// panel through the wrong scale, and both symptoms of that — an empty panel
    /// and an interface in the bottom-left corner — photograph as coherent
    /// frames, so only a count can report it.
    framebuffer: musializer_runtime::draw::FramebufferAudit,
    /// Where `--ui-probe click=` aimed, kept so the report can say so after the
    /// window is gone. `None` means no click was requested, which is the normal
    /// case and prints nothing.
    click_at: Option<(f32, f32)>,
    /// The worst single frame, and how many missed the 60 Hz budget (EX4).
    ///
    /// Deliberately a worst rather than a mean: a mean over a whole run hides
    /// exactly the event being looked for. The first frame is excluded because
    /// it carries the window's own startup.
    worst_frame_seconds: f32,
    worst_frame_index: u64,
    frames_over_budget: u64,
    /// Peak occupancy of the audio bridge ring, as a fraction of its capacity.
    peak_ring_fill: f32,
    /// The preview panel and the rect the scene was actually drawn into (EX2).
    ///
    /// Two rects rather than one, because the whole point is the difference: a
    /// scene that fills its panel and a scene framed to a 16:9 export are the
    /// same picture, and a capture cannot tell a deliberate pillarbox from a
    /// scene that failed to fill the panel — which is precisely what ASCII
    /// Field's fixed grid looked like before EX2.
    preview_panel: (f32, f32),
    preview_framed: (f32, f32),
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
            beat_phase_low: f32::MAX,
            beat_phase_high: f32::MIN,
            beat_interval_last: 0.0,
            beat_intervals_learned: 0,
            onsets_seen: 0,
            flux_seen: 0.0,
            frame_lanes: FrameLaneStatus::default(),
            logical_window: (0.0, 0.0),
            timeline: String::new(),
            framebuffer: musializer_runtime::draw::FramebufferAudit::default(),
            click_at: None,
            worst_frame_seconds: 0.0,
            worst_frame_index: 0,
            frames_over_budget: 0,
            peak_ring_fill: 0.0,
            preview_panel: (0.0, 0.0),
            preview_framed: (0.0, 0.0),
            reopened: None,
        }
    }
}

fn format_optional_ui_size(value: Option<f32>) -> String {
    value.map_or_else(|| "auto".to_string(), |value| format!("{value:.0}"))
}

/// How long a frame has to run before it counts as a stall (EX4).
///
/// **Not** the 60 Hz period. `set_target_fps(60)` makes a healthy frame land
/// within a float hair of 16.7 ms, so a `> 1/60` test reports 118 of 120 frames
/// as over budget and says nothing — which is what the first version of this
/// line did. 25 ms is one and a half periods: past it the frame after this one
/// cannot be presented on time, so it is a dropped frame rather than jitter.
const FRAME_STALL_SECONDS: f32 = 1.0 / 40.0;

impl Report {
    /// One frame's wall-clock cost (EX4).
    ///
    /// The first two frames are skipped: frame 0 carries window creation and
    /// frame 1 the first font atlas, and neither is a stutter the user can feel
    /// during playback. Everything after that is fair game, including the ones
    /// this project already knows are expensive — the at-size caption atlas
    /// rebuild, the Song Atlas whole-track decode, an autosave's two `fsync`s.
    fn observe_frame_time(&mut self, seconds: f32) {
        if self.frames < 2 || !seconds.is_finite() {
            return;
        }
        if seconds > FRAME_STALL_SECONDS {
            self.frames_over_budget += 1;
        }
        if seconds > self.worst_frame_seconds {
            self.worst_frame_seconds = seconds;
            self.worst_frame_index = self.frames;
        }
    }

    /// The bridge ring's occupancy this frame, before it is drained.
    fn observe_ring(&mut self, used: usize, capacity: usize) {
        if capacity == 0 {
            return;
        }
        let fill = used as f32 / capacity as f32;
        if fill > self.peak_ring_fill {
            self.peak_ring_fill = fill;
        }
    }

    fn observe_peak_band(&mut self, band: usize) {
        self.peak_band_last = band;
        self.peak_band_low = self.peak_band_low.min(band);
        self.peak_band_high = self.peak_band_high.max(band);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the report reads from every long-lived owner in main: app, analyzer, fonts, renderer, assist"
    )]
    fn print(
        &self,
        raylib_version: &str,
        app: &App,
        band_count: usize,
        requested_scene: Option<SceneId>,
        fonts: &Faces,
        renderer: &scene_host::SceneRenderer,
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
        // The scene-text path, which the `fonts:` line cannot carry: the SDF
        // shader belongs to the renderer, and a compiled shader is not the same
        // claim as a Cadence frame that actually typeset through it.
        println!("scene text:      {}", renderer.describe());
        // The caption glow's mechanism and last outcome (UX0-C11), on its own
        // line because the gate pins `scene text:`'s grammar exactly. A frame
        // with no halo and a frame whose halo silently failed to build are the
        // same picture; `off` versus `unavailable` is a claim only this line
        // can carry.
        println!("caption halo:    {}", renderer.describe_caption_halo());
        println!(
            "ui layout:       scale={} sidebar={} inspector={} timeline={}",
            (fonts.ui().scale() * 100.0).round() as u16,
            format_optional_ui_size(app.shell.ui_preferences.sidebar_width),
            format_optional_ui_size(app.shell.ui_preferences.inspector_width),
            format_optional_ui_size(app.shell.ui_preferences.timeline_height),
        );
        println!("frames rendered: {}", self.frames);
        // How long the *worst* frame took, and how many missed the budget (EX4).
        //
        // `frames rendered:` counts frames and says nothing about their cost, so
        // every known stall in this application — the 10-30 ms at-size caption
        // atlas rebuild when a lyric introduces a codepoint, the Song Atlas
        // whole-track decode, an autosave that hashes the source audio and calls
        // `fsync` twice on the main thread — was reportable only by eye. A worst
        // rather than a mean, because a mean over a run is precisely what hides
        // a single 400 ms event.
        println!(
            "frame budget:    worst {:.1}ms at frame {}, {} of {} stalled past {:.1}ms",
            self.worst_frame_seconds * 1000.0,
            self.worst_frame_index,
            self.frames_over_budget,
            self.frames,
            FRAME_STALL_SECONDS * 1000.0,
        );
        println!("analyzer runs:   {}", self.analyzed_frames);
        println!(
            "audio frames:    {} consumed, {dropped} dropped, peak ring fill {:.0}%",
            self.consumed_frames,
            self.peak_ring_fill * 100.0
        );
        println!("output underruns: {}", audio_bridge::output_underruns());
        println!("bands:           {band_count}");
        println!("peak seen:       {:.4}", self.peak_seen);
        // Evidence, not existence. `beat_phase` is a documented route source and it
        // was hardcoded to 0.0 for two bands with the tracker never called, so this
        // reports the *range* — a phase that never moves is the failure, and a
        // single value cannot show it.
        if self.beat_phase_low > self.beat_phase_high {
            println!("beat phase:      never sampled");
        } else {
            println!(
                // `learned` disambiguates the interval, which is otherwise not
                // evidence at all: the tracker's default interval is 0.500s, and
                // the synthetic fixture's 2 Hz pulse is also 0.500s. Without the
                // count, "interval 0.500s" reads identically whether a tempo was
                // learned from the audio or never learned at all.
                "beat phase:      {:.4}..{:.4} (interval {:.3}s, {} learned)",
                self.beat_phase_low,
                self.beat_phase_high,
                self.beat_interval_last,
                self.beat_intervals_learned
            );
        }
        // The tracker's *input*, which is what separates "onset detection is broken"
        // from "this audio taught the tracker nothing". On the synthetic sweep those
        // are genuinely different and only this line tells them apart: onsets **do**
        // fire (8 of 200 frames, peak flux 0.1013 against the 0.08 threshold), and
        // `0 learned` is still correct, because the pulse's onsets land in adjacent
        // frames and a ~0.017s gap is below the 0.25s minimum plausible beat
        // interval. That is the oracle's own rejection
        // (`beat_tracker.c`'s interval window), so the phase free-runs at the
        // neutral 120 BPM — which is what a capture of this fixture should show.
        println!(
            "onsets:          {} of {} frames, peak flux {:.4} (threshold {:.2})",
            self.onsets_seen,
            self.frames,
            self.flux_seen,
            musializer_core::scene::ONSET_FLUX_THRESHOLD
        );
        {}
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
        match app.workspace.current() {
            None => println!("auto scenes:     no track"),
            Some(track) => println!(
                "auto scenes:     {} ({} cues)",
                if track.scene_switches.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                track.scene_switches.len()
            ),
        }
        // Which scene each segment carries, in plan order (LX3). `auto scenes:`
        // above counts them and `scene:` names the one bound right now, so a
        // retarget of a segment the playhead is *not* inside changed nothing
        // any line reported — and a scene click while a plan runs now retargets
        // exactly that kind of segment.
        if let Some(track) = app.workspace.current() {
            let segments: Vec<&str> = track
                .scene_switches
                .cues()
                .iter()
                .map(|cue| {
                    SceneId::from_index(cue.scene_index as usize).map_or("?", SceneId::stable_name)
                })
                .collect();
            println!(
                "scene segments:  {}",
                if segments.is_empty() {
                    "none".to_owned()
                } else {
                    segments.join(", ")
                }
            );
        }
        println!(
            "frame lanes:     lyric={} semantic={} source={} merged-events={}",
            self.frame_lanes
                .lyric_id
                .map_or_else(|| "none".to_owned(), |id| id.to_string()),
            if self.frame_lanes.semantic_available {
                "available"
            } else {
                "unavailable"
            },
            self.frame_lanes.semantic_source_id,
            self.frame_lanes.merged_event_count,
        );
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
            // The staged case had no line at all, and "no track" is exactly the
            // state D1 has to prove something about: an image dropped before any
            // audio is *kept* for the next track, and a report that says only
            // "no track" cannot tell that from the drop having been discarded.
            None => match &app.pending_ascii {
                Some(image) => println!(
                    "ascii:           staged {}x{} glyphs from {} (no track yet)",
                    image.columns,
                    image.rows,
                    image.path.display()
                ),
                None => println!("ascii:           no track"),
            },
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
        // The welcome screen's recent list (UX0-C06). `store=` distinguishes the
        // two states an empty column can be in — a new user and a refused file —
        // which is the whole reason the column draws different words for them.
        println!(
            "recent:          {} entries, {} missing, store={}",
            app.shell.recent.len(),
            app.shell
                .recent
                .entries()
                .iter()
                .filter(|entry| entry.missing)
                .count(),
            if app.shell.recent_unavailable {
                "unavailable"
            } else if app.recent_editable {
                "writable"
            } else {
                "read-only"
            }
        );
        // What `--ui-probe drop=` actually reached (D1). Recorded by the shell as
        // it dispatched, so this cannot be green while the branch is dead.
        match &app.shell.probe_drop_dispatch {
            None => println!("drop probe:      not requested"),
            Some((path, kind)) => println!(
                "drop probe:      {} as {}",
                path.display(),
                kind.attempted_noun()
            ),
        }
        println!("panel:           {}", app.shell.panel.label());
        // The scene panel, and the rect inside it the scene was drawn into
        // (EX2). Equal means the export is the panel's own shape; unequal names
        // the framing a capture would otherwise show as an unexplained inset.
        {
            let (panel_width, panel_height) = self.preview_panel;
            let (framed_width, framed_height) = self.preview_framed;
            let framing = if framed_width < panel_width - 0.5 {
                "pillarbox"
            } else if framed_height < panel_height - 0.5 {
                "letterbox"
            } else {
                "full"
            };
            println!(
                "preview frame:   panel {panel_width:.0}x{panel_height:.0}, \
                 framed {framed_width:.0}x{framed_height:.0} ({framing})"
            );
        }
        // The export geometry a click on the SIZE row is supposed to move
        // (EX1). Nothing printed it, so a capture of the export panel could not
        // tell a row that took the press from one that ignored it: the summary
        // line inside the panel says the same thing either way, and only the
        // highlight moves — which is one button's fill colour in a screenshot.
        if let Some(track) = app.workspace.current() {
            let config = track.render_config;
            println!(
                "export config:   {}x{} at {} fps, {}, supersample {}x",
                config.width,
                config.height,
                config.fps,
                config.quality.name(),
                config.supersample_factor,
            );
        }
        // Every track's save state, current one starred (UX0-B01, C1, C4).
        //
        // The badge is on screen, so why a line? Because the two states a user
        // most needs to tell apart — Unsaved and Save failed — are a word and a
        // hue in a 60 px box, and a capture cannot assert either. It is also the
        // only way to see a *background* track's state at all: nothing draws the
        // rows that are scrolled out, and all-track autosave is precisely a claim
        // about tracks the user is not looking at.
        println!("save state:      {}", app.workspace.describe_save_state());
        // What `--ui-probe click=` actually reached (EX1). `claimed` is the
        // whole point: a press that never landed and a press some other control
        // swallowed leave the same picture, and this is the only line that can
        // tell them apart.
        if let Some((x, y)) = self.click_at {
            let claimed = app.shell.widgets.last_claimed_id();
            println!(
                "click probe:     at={x:.0}x{y:.0} claimed={}",
                if claimed == 0 {
                    "nothing".to_owned()
                } else {
                    format!("{:#x}", claimed)
                }
            );
        }
        // LX2. The wheel now zooms from any of the three timed lanes, and the
        // only thing it changes is this shared view. Nothing reported it, so a
        // capture could not tell a lane that accepted the notch from one that
        // ignored it — both draw a perfectly plausible timeline.
        println!("timeline:        {}", self.timeline);
        // The size invariant `EndTextureMode` does not restore. A frame that ends
        // with rlgl's pair still pointing at a caption blur buffer or an export
        // target scales the next scene panel by that ratio, and the result is a
        // plausible picture either way — an empty panel, or the whole interface
        // in the bottom-left corner. Only a count can say it happened.
        println!("gl framebuffer:  {}", self.framebuffer);
        // Which save route was on screen, and whether the tracks panel was
        // collapsed — the state review 1.12 found unrecoverable is only provable
        // from a capture through this line.
        println!(
            "chrome:          {}",
            app.shell
                .describe_workspace(self.logical_window, &app.workspace)
        );
        // What the Tune panel's own controls are doing (PX6). Separate from the
        // values line because "the field is open" and "the value changed" fail
        // independently.
        println!(
            "tune entry:      {}",
            ui::panels::tune::tune_state_line(
                &app.shell,
                app.scene.id(),
                app.workspace.current_index().unwrap_or(0),
                app.workspace
                    .current()
                    .and_then(workspace::Track::active_cue)
                    .map(|(position, _)| position),
            )
        );
        // The drawn scene's tuning, exactly (PX6). A capture cannot tell 1.37
        // from 1.3700001, and "Revert restored it exactly" is precisely that
        // distinction — so the numbers are printed in a form that round-trips.
        println!(
            "tune values:     {}",
            ui::panels::tune::tune_values_line(app.scene.id(), app.settings())
        );
        // Whether Tune edits the base scene or a cue snapshot is invisible on a
        // capture without this line (review 1.7): the badge text is the evidence.
        println!(
            "tune scope:      {}",
            ui::panels::tune::tune_scope_label(
                app.workspace
                    .current()
                    .and_then(workspace::Track::active_cue)
                    .map(|(index, cue)| (index, cue.start_seconds)),
                app.workspace
                    .current()
                    .map_or(0, |track| track.scene_switches.len()),
                app.workspace
                    .current()
                    .is_some_and(|track| track.scene_switches.enabled),
            )
        );
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

#[cfg(test)]
mod tests {
    use super::scene_clock_delta;

    #[test]
    fn scene_clock_turns_transport_discontinuities_into_zero_delta() {
        let mut previous = None;
        assert_eq!(scene_clock_delta(&mut previous, 12.0), 0.0);
        assert!((scene_clock_delta(&mut previous, 12.025) - 0.025).abs() < 1.0e-6);
        assert_eq!(scene_clock_delta(&mut previous, 4.0), 0.0);
        assert_eq!(scene_clock_delta(&mut previous, 4.75), 0.0);
        assert!((scene_clock_delta(&mut previous, 4.8) - 0.05).abs() < 1.0e-6);
    }
}
