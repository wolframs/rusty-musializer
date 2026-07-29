//! The export panel, the progress screen, and the session that drives them.
//!
//! **Owner: Agent H.** C sources: `draw_export_panel` (`plug.c:2543-2659`),
//! `start_rendering_track_to` (`:6863-7118`), `start_rendering_track`
//! (`:7120-7138`), `rendering_screen` (`:7873-8058`) and
//! `finish_rendering_track` (`:7252-7280`), over `render_export.c` and
//! `ffmpeg_posix.c`.
//!
//! Three things live here, in the order a user meets them:
//!
//! 1. [`Shell::export_panel`] — the resolution, frame-rate and quality rows, the
//!    summary, and the footer that asks for a destination.
//! 2. [`ExportSession`] — one running export. It owns the offline render target
//!    and [`RenderJob`], which owns the encoder, the decoded audio and the frame
//!    cursor. [`ExportSession::tick`] is one exported frame, or one batch of
//!    fast-forwarded ones.
//! 3. [`Shell::export_progress`] — what the window shows while that runs, which
//!    is the *whole* window: an export is a modal state in the oracle, not a
//!    panel (`plug.c:8643-8654`).
//!
//! # The integration seam
//!
//! [`dispatch`] is the one function that reaches `main.rs`, through
//! `ShellCommand::SetRenderConfig` and `::StartRender`.
//!
//! There is deliberately no *cancel* command: cancellation happens on the
//! progress screen, which [`ExportSession`] draws and reads in the same tick, so
//! it never needs to travel through `main.rs`.
//!
//! # Why the session is not in `main.rs`
//!
//! `main.rs` owns the window, the audio device and the analyzer, and the export
//! loop needs all three — but it needs them for eight lines, and putting the
//! loop there would put the oracle's most order-sensitive code in the file
//! nobody in a fan-out may touch. So the session takes `&mut crate::App` and the
//! handles it needs, and `main.rs`'s share is one call.

// Everything below [`ExportSession`] is reachable only from `main.rs`, and this
// fan-out does not edit `main.rs`.
use std::path::{Path, PathBuf};

use musializer_core::audio::{AudioAnalyzer, AudioAnalyzerConfig};
use musializer_core::scene::routes::RouteSources;
use musializer_core::scene::{SceneAudioFrame, SceneFrame, SceneInstance};
use musializer_core::timing::render_export::{FrameRate, Quality, RenderExportConfig, Resolution};
use musializer_core::ui::notice::Severity;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::Faces;
use musializer_runtime::process::ffmpeg::{ffmpeg_available, Finished};
use musializer_runtime::process::render_job::{RenderJob, RenderRequest};
use raylib::prelude::{
    Color, Music, RaylibAudio, RaylibDraw, RaylibDrawHandle, RaylibHandle, RaylibScissorModeExt,
    RaylibTexture2D, RaylibTextureModeExt, RaylibThread, RenderTexture2D,
};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::cli::UiPanel;
use crate::scene_host;

/// Widget id namespace for this panel.
///
/// A local constant rather than a row in `widgets::id`, because adding one there
/// is an edit to a file six agents share. The value continues that module's
/// sequence and the integration owner is asked to fold it in at merge; the
/// numbers are what must not collide, and 7 is free.
const EXPORT_WIDGETS: u32 = widgets::id::EXPORT;

/// What the export panel asked for this frame.
///
/// Deliberately not a `ShellCommand`: those live in `shell.rs`. See the module
/// comment.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExportRequest {
    /// Write this configuration to the current track and mark it dirty
    /// (`plug.c:2569-2572`).
    Configure(RenderExportConfig),
    /// Ask for a destination and start (`start_rendering_track`,
    /// `plug.c:7120-7138`).
    Start,
}

/// The single seam between this panel and `main.rs`.
fn dispatch(request: &ExportRequest, commands: &mut Vec<ShellCommand>) {
    match request {
        ExportRequest::Configure(config) => commands.push(ShellCommand::SetRenderConfig(*config)),
        ExportRequest::Start => commands.push(ShellCommand::StartRender),
    }
}

/// Which resolution preset a configuration matches, if any
/// (`set_active_render_config`, `plug.c:635-656`).
///
/// `None` is a real answer, not a fallback: `--resolution 1234x568` is a valid
/// configuration that no preset button represents, and highlighting the nearest
/// one would tell the user their export is 1080p when it is not.
fn selected_resolution(config: &RenderExportConfig) -> Option<Resolution> {
    Resolution::ALL
        .into_iter()
        .find(|resolution| resolution.dimensions() == (config.width, config.height))
}

/// Which frame-rate preset a configuration matches, if any (`plug.c:647-656`).
fn selected_frame_rate(config: &RenderExportConfig) -> Option<FrameRate> {
    FrameRate::ALL
        .into_iter()
        .find(|frame_rate| frame_rate.fps() == config.fps)
}

/// The one-line explanation under the summary (`plug.c:2625-2629`).
fn quality_detail(quality: Quality) -> &'static str {
    match quality {
        Quality::Balanced => "Balanced uses native resolution and CRF 20.",
        Quality::High => "High uses 2x spatial supersampling and CRF 16.",
        Quality::Master => "Master uses 2x spatial supersampling and CRF 12.",
    }
}

impl Shell {
    /// The export panel, in the timeline strip's place
    /// (`draw_export_panel`, `plug.c:2543-2659`).
    ///
    /// The oracle draws this only when its box clears 80 px (`plug.c:3141`), and
    /// so does this: a panel that cannot host its own controls is not drawn at
    /// all, which is `workspace_layout.h`'s rule and the reason invisible
    /// controls once stole clicks here.
    pub(crate) fn export_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        // Below the strip *and* below the zoom readout beneath it. Drawing into
        // `content` directly is the mistake a capture caught in the scaffold:
        // the box printed over the timeline's ticks.
        let top = strip.y + strip.height + 28.0;
        let boundary = UiRect::new(
            content.x + padding,
            top,
            (content.width - padding * 2.0).max(0.0),
            (content.y + content.height - top - padding).max(0.0),
        );
        if boundary.height <= 80.0 || boundary.width <= 0.0 {
            return;
        }

        let font = input.fonts.ui();
        widgets::fill(d, boundary, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, color::ui_rule());
        // Everything inside is clipped to the box, so a narrow window cuts a
        // label off rather than printing it across the tracks rail.
        let mut clip = d.begin_scissor_mode(
            boundary.x as i32,
            boundary.y as i32,
            boundary.width as i32,
            boundary.height as i32,
        );

        widgets::draw_text(
            &mut clip,
            font,
            "EXPORT",
            boundary.x + padding,
            boundary.y + padding,
            metric::UI_FONT_HEADER,
            color::accent(),
        );
        widgets::draw_text(
            &mut clip,
            font,
            "One deterministic scene path. The destination is replaced only after the encoder succeeds.",
            boundary.x + padding,
            boundary.y + 36.0,
            metric::UI_FONT_VALUE,
            color::ui_muted(),
        );

        // The panel reads the *track's* configuration, never a copy of its own:
        // `--resolution` on the command line and a click here have to be the
        // same state or the buttons would disagree with the render.
        let config = input
            .workspace
            .current()
            .map_or_else(RenderExportConfig::default, |track| track.render_config);

        let mut y = boundary.y + 62.0;
        widgets::draw_text(
            &mut clip,
            font,
            "SIZE",
            boundary.x + padding,
            y + 10.0,
            13.0,
            color::ui_muted(),
        );
        let mut x = boundary.x + 68.0;
        let size_width = 76.0f32;
        let selected_size = selected_resolution(&config);
        for (index, resolution) in Resolution::ALL.into_iter().enumerate() {
            let button = UiRect::new(x, y, size_width, metric::UI_BUTTON_HEIGHT);
            if boundary.contains(button)
                && self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(EXPORT_WIDGETS, index as u32),
                        button,
                        resolution.name(),
                        selected_size == Some(resolution),
                        ButtonStyle::Neutral,
                        None,
                    )
                    .clicked
            {
                let mut next = config;
                next.set_resolution(resolution);
                dispatch(&ExportRequest::Configure(next), commands);
            }
            x += size_width + gap;
        }

        widgets::draw_text(
            &mut clip,
            font,
            "FPS",
            x + 8.0,
            y + 10.0,
            13.0,
            color::ui_muted(),
        );
        x += 48.0;
        let selected_rate = selected_frame_rate(&config);
        for (index, frame_rate) in FrameRate::ALL.into_iter().enumerate() {
            let button = UiRect::new(x, y, 72.0, metric::UI_BUTTON_HEIGHT);
            if boundary.contains(button)
                && self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(EXPORT_WIDGETS, 8 + index as u32),
                        button,
                        frame_rate.name(),
                        selected_rate == Some(frame_rate),
                        ButtonStyle::Neutral,
                        None,
                    )
                    .clicked
            {
                let mut next = config;
                next.set_frame_rate(frame_rate);
                dispatch(&ExportRequest::Configure(next), commands);
            }
            x += 72.0 + gap;
        }

        y += 46.0;
        widgets::draw_text(
            &mut clip,
            font,
            "QUALITY",
            boundary.x + padding,
            y + 10.0,
            13.0,
            color::ui_muted(),
        );
        x = boundary.x + 86.0;
        for (index, quality) in Quality::ALL.into_iter().enumerate() {
            let button = UiRect::new(x, y, 106.0, metric::UI_BUTTON_HEIGHT);
            if boundary.contains(button)
                && self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(EXPORT_WIDGETS, 16 + index as u32),
                        button,
                        quality.name(),
                        config.quality == quality,
                        ButtonStyle::Neutral,
                        None,
                    )
                    .clicked
            {
                let mut next = config;
                next.set_quality(quality);
                dispatch(&ExportRequest::Configure(next), commands);
            }
            x += 106.0 + gap;
        }

        // The summary is the readout that makes the three rows above mean
        // something: it names the track, the geometry, the frame estimate and
        // which scene path will actually run (`plug.c:2611-2624`).
        let track_name = input
            .workspace
            .current()
            .map_or("no track", |track| track.display_name());
        let scene_label = input.workspace.current().map_or("", |track| {
            if track.scene_switches.enabled {
                "automatic scene plan"
            } else {
                track.base_scene.display_name()
            }
        });
        let approximate_frames = (input.duration_seconds * f64::from(config.fps))
            .ceil()
            .max(0.0);
        let summary = format!(
            "{}  |  {}x{} at {} fps  |  {}  |  est. {approximate_frames:.0} frames  |  {scene_label}",
            track_name,
            config.width,
            config.height,
            config.fps,
            config.quality.name(),
        );
        widgets::draw_text(
            &mut clip,
            font,
            &summary,
            boundary.x + padding,
            y + 50.0,
            15.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            &mut clip,
            font,
            quality_detail(config.quality),
            boundary.x + padding,
            y + 73.0,
            14.0,
            color::ui_muted(),
        );
        // The footer. `ffmpeg_available` is called once and used twice, as the
        // C calls it twice per frame (`plug.c:2639`, `:2644`) — a cached answer
        // would keep saying "FFmpeg required" after the user installed it.
        let encoder_present = ffmpeg_available();
        let render = UiRect::new(
            boundary.x + boundary.width - padding - 212.0,
            boundary.y + boundary.height - 44.0,
            212.0,
            metric::UI_BUTTON_HEIGHT,
        );
        let close = UiRect::new(render.x - 98.0 - gap, render.y, 98.0, render.height);
        let render_label = if encoder_present {
            "Choose output and render"
        } else {
            "FFmpeg required"
        };
        let labels = [render_label, "Close"];
        let widths = [render.width, close.width];
        let footer_font = widgets::row_font_size(font, &labels, &widths, render.height);
        if boundary.contains(render) {
            if encoder_present && input.workspace.current().is_some() {
                if self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(EXPORT_WIDGETS, 24),
                        render,
                        render_label,
                        false,
                        ButtonStyle::Neutral,
                        Some(footer_font),
                    )
                    .clicked
                {
                    dispatch(&ExportRequest::Start, commands);
                }
            } else {
                // Named and disabled rather than absent: "FFmpeg required" is
                // the only thing on this screen that tells a user why the one
                // button they came for will not press.
                self.widgets.disabled_button(
                    &mut clip,
                    font,
                    render,
                    render_label,
                    Some(footer_font),
                );
            }
        }
        if boundary.contains(close)
            && self
                .widgets
                .text_button(
                    &mut clip,
                    font,
                    widgets::widget_id(EXPORT_WIDGETS, 25),
                    close,
                    "Close",
                    false,
                    ButtonStyle::Neutral,
                    Some(footer_font),
                )
                .clicked
        {
            self.panel = UiPanel::None;
        }
    }

    /// The whole window while an export runs (`rendering_screen`'s drawing half,
    /// `plug.c:7919-7976`).
    ///
    /// Returns `true` when the user asked to cancel — either with the button or
    /// with Escape, both of which the oracle honours (`plug.c:7889`).
    ///
    /// This is not a panel. An export replaces the workspace, because a
    /// workspace whose controls edit state the running export has already
    /// snapshotted would be lying about what it is about to produce.
    pub(crate) fn export_progress(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        job: &RenderJob,
    ) -> bool {
        let (w, h) = (d.get_screen_width() as f32, d.get_screen_height() as f32);
        let font = fonts.ui();
        d.clear_background(color::background());

        let window_frames = job.plan().encoded_frames();
        let encoded = job.encoded_frames();
        let progress = job.progress();
        let elapsed = job.elapsed_seconds();
        let remaining = job.remaining_seconds();
        let label = if job.is_finishing() {
            "Finishing encoder"
        } else if job.is_fast_forwarding() {
            "Preparing window state"
        } else {
            "Exporting video"
        };
        let title_width = widgets::measure(font, label, 34.0);
        widgets::draw_text(
            d,
            font,
            label,
            w / 2.0 - title_width / 2.0,
            h / 2.0 - 92.0,
            34.0,
            color::white(),
        );

        let detail = if job.is_finishing() {
            format!("{window_frames} frames encoded  |  finalizing MP4")
        } else if job.is_fast_forwarding() {
            format!(
                "{} / {} deterministic frames prepared",
                job.frame_index(),
                job.plan().frames.start
            )
        } else {
            format!(
                "{encoded} / {window_frames} frames  |  {:02}:{:02} elapsed  |  about {:02}:{:02} remaining",
                (elapsed / 60.0) as u32,
                (elapsed % 60.0) as u32,
                (remaining / 60.0) as u32,
                (remaining % 60.0) as u32,
            )
        };
        let detail_width = widgets::measure(font, &detail, 18.0);
        widgets::draw_text(
            d,
            font,
            &detail,
            w / 2.0 - detail_width / 2.0,
            h / 2.0 - 43.0,
            18.0,
            // `ColorAlpha(WHITE, 0.72)` (`plug.c:7957`), not `ui_muted`: this
            // screen is the only dark surface in the interface, and the palette's
            // muted grey was picked — and contrast-checked — against the light
            // ones. A capture of the progress screen is what showed it reading
            // as barely-there grey on near-black.
            Color::new(255, 255, 255, 184),
        );

        let bar_width = w * 2.0 / 3.0;
        let bar = UiRect::new(w / 2.0 - bar_width / 2.0, h / 2.0, bar_width, 14.0);
        widgets::fill(
            d,
            UiRect::new(bar.x, bar.y, bar.width * progress as f32, bar.height),
            color::accent(),
        );
        d.draw_rectangle_lines_ex(widgets::rectangle(bar), 1.0, color::white());

        // The destination, clipped rather than truncated: a long path is worth
        // showing the start of.
        {
            let mut clip =
                d.begin_scissor_mode(bar.x as i32, bar.y as i32 + 27, bar.width as i32, 24);
            widgets::draw_text(
                &mut clip,
                font,
                &job.destination().display().to_string(),
                bar.x,
                bar.y + 29.0,
                14.0,
                // `ColorAlpha(WHITE, 0.52)` (`plug.c:7968`); see the detail line.
                Color::new(255, 255, 255, 133),
            );
        }

        let mut cancelled = d.is_key_pressed(raylib::consts::KeyboardKey::KEY_ESCAPE);
        let cancel = UiRect::new(w - 146.0, 24.0, 122.0, 38.0);
        if !job.is_finishing()
            && self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(EXPORT_WIDGETS, 26),
                    cancel,
                    "Cancel export",
                    false,
                    ButtonStyle::Danger,
                    None,
                )
                .clicked
        {
            cancelled = true;
        }
        cancelled
    }
}

/// The destination picker (`start_rendering_track`, `plug.c:7120-7138`).
///
/// Called from outside the drawing pair, like every other modal in this
/// codebase: the picker blocks until the user answers, and holding a frame open
/// across it would freeze the window mid-paint.
///
/// Returns `None` for cancellation, which is deliberately silent.
pub(crate) fn ask_for_destination(app: &mut crate::App) -> Option<PathBuf> {
    use musializer_runtime::process::dialogs::{self, FileDialog};

    let track = app.workspace.current()?;
    // The oracle's suggestion, and its fallback when the source path cannot
    // produce one (`plug.c:7124-7131`).
    let scene_name = if track.scene_switches.enabled {
        "scene-plan"
    } else {
        track.base_scene.stable_name()
    };
    let suggested = track
        .file_path
        .to_str()
        .and_then(|path| track.render_config.suggest_path(path, scene_name).ok())
        .unwrap_or_else(|| "./musializer-render.mp4".to_string());

    let dialog = FileDialog::new("Export video")
        .with_filter(dialogs::filters::MP4_VIDEO)
        .with_default_path(&suggested);
    match dialog.save_file() {
        Ok(path) => path,
        Err(error) => {
            app.shell.notify(
                Severity::Warning,
                "No file picker is available",
                &format!("{error}. Pass --render on the command line instead."),
            );
            None
        }
    }
}

/// One running export: the offline target, the job, and the transport state the
/// preview has to get back.
///
/// Owning this means owning an FFmpeg child. [`ExportSession::tick`] is the only
/// way it advances and it always ends by finishing the job, so the child cannot
/// outlive the session.
pub(crate) struct ExportSession {
    /// `None` only between [`ExportSession::conclude`] taking the job and the
    /// caller dropping the session, which is the same tick. An `Option` rather
    /// than a placeholder value because `RenderJob::finish` consumes the job and
    /// a job owns a child process — there is no cheap stand-in to put back, and
    /// inventing one would mean a second code path that can reach the encoder.
    job: Option<RenderJob>,
    target: RenderTexture2D,
    /// Reused between frames: a 1080p frame is 8 MiB and reallocating it per
    /// frame would dominate the export.
    pixels: Vec<u8>,
    /// Where the transport was when the export started, so the preview comes
    /// back to it (`plug.c:6918-6919`, restored at `:7276-7279`).
    restore_position: f32,
    restore_playing: bool,
}

impl ExportSession {
    /// Stops playback, prepares the offline target, and starts the encoder
    /// (`start_rendering_track_to`, `plug.c:6863-7118`).
    ///
    /// Every refusal is reported in the tray by name, because "export failed"
    /// is not something a user can act on and "the path aliases your source
    /// audio" is.
    #[allow(
        clippy::too_many_arguments,
        reason = "the composition root's resources, borrowed rather than owned; a bundle struct here would only move the same list one line up"
    )]
    pub(crate) fn begin<'audio>(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        audio: &'audio RaylibAudio,
        music: Option<&Music<'audio>>,
        app: &mut crate::App,
        analyzer: &mut AudioAnalyzer,
        destination: &Path,
        window: Option<(f64, f64)>,
    ) -> Option<Self> {
        let Some(track) = app.workspace.current() else {
            app.shell.notify(
                Severity::Error,
                "Export was not started",
                "There is no track to render.",
            );
            return None;
        };
        let source = track.file_path.clone();
        let config = track.render_config;
        let (scene, seed) = (track.base_scene, track.scene_seed);
        let protected: Vec<PathBuf> = track.project_path.iter().cloned().collect();

        let restore_position = music.map_or(0.0, |music| music.get_time_played());
        let restore_playing = music.is_some_and(Music::is_stream_playing);
        if let Some(music) = music {
            // The stream is the sample ring's producer; an export reads the
            // decoded file instead, and leaving playback running would put live
            // audio into the analyzer alongside it (`plug.c:6920`).
            music.stop_stream();
        }

        let target = match offline_render_target(rl, thread, &config) {
            Some(target) => target,
            None => {
                app.shell.notify(
                    Severity::Error,
                    "Export surface could not be created",
                    "Try a lower resolution or Balanced quality.",
                );
                restore_preview(music, app, analyzer, restore_position, restore_playing);
                return None;
            }
        };

        let request = RenderRequest {
            destination,
            source_audio: &source,
            config,
            window,
            protected: &protected,
        };
        let job = match RenderJob::start(audio, &request) {
            Ok(job) => job,
            Err(error) => {
                app.shell.notify(
                    Severity::Error,
                    "Export was not started",
                    &format!("{error}. The source and any previous destination were preserved."),
                );
                restore_preview(music, app, analyzer, restore_position, restore_playing);
                return None;
            }
        };

        // The analyzer follows the decoded wave, not the preview's stereo mix
        // (`plug.c:7094`), and the scene restarts from the track's seed so that
        // the export does not inherit whatever state the preview had wandered
        // into (`plug.c:7019`).
        match AudioAnalyzer::boxed(job.analyzer_config()) {
            Ok(configured) => *analyzer = *configured,
            Err(error) => {
                app.shell.notify(
                    Severity::Error,
                    "Export was not started",
                    &format!("The analyzer could not be configured for export: {error}"),
                );
                let _ = job.finish(true);
                restore_preview(music, app, analyzer, restore_position, restore_playing);
                return None;
            }
        }
        app.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
        if let Some(track) = app.workspace.current_mut() {
            track.scene_switches.reset();
            track.cue_settings_active = false;
        }
        app.shell.panel = UiPanel::None;

        let pixels = vec![0u8; config.width as usize * config.height as usize * 4];
        Some(ExportSession {
            job: Some(job),
            target,
            pixels,
            restore_position,
            restore_playing,
        })
    }

    /// The running job.
    ///
    /// Every caller checks `self.job.is_none()` first and returns; the panic
    /// message names the invariant rather than hiding behind an `unwrap`.
    fn job(&self) -> &RenderJob {
        self.job
            .as_ref()
            .expect("a session is dropped on the tick its job is concluded")
    }

    fn job_mut(&mut self) -> &mut RenderJob {
        self.job
            .as_mut()
            .expect("a session is dropped on the tick its job is concluded")
    }

    /// One tick of the export (`rendering_screen`, `plug.c:7873-8058`).
    ///
    /// Returns `true` when the export has ended — published, cancelled or
    /// failed — and the session should be dropped. The caller's whole frame is
    /// this call: an export owns the window.
    #[allow(
        clippy::too_many_arguments,
        reason = "same resource list as `begin`; the alternative is a struct that exists only to be destructured here"
    )]
    pub(crate) fn tick(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        music: Option<&Music<'_>>,
        app: &mut crate::App,
        analyzer: &mut AudioAnalyzer,
        renderer: &mut scene_host::SceneRenderer,
        fonts: &Faces,
    ) -> bool {
        if self.job.is_none() {
            return true;
        }
        // Order is the oracle's: cancel wins over completion, completion takes
        // one extra tick so the screen can say "Finishing encoder", and only
        // then does a frame get drawn.
        if self.job().cancel_requested() {
            return self.conclude(true, music, app, analyzer);
        }
        if self.job().is_complete() && self.job_mut().begin_finishing() {
            return self.conclude(false, music, app, analyzer);
        }

        let mut failure: Option<(&'static str, String)> = None;
        {
            let mut d = rl.begin_drawing(thread);
            let cancelled = app.shell.export_progress(&mut d, fonts, self.job());
            if cancelled {
                self.job_mut().request_cancel();
            }
            let active = !self.job().is_finishing() && !self.job().cancel_requested();
            if active {
                let mut steps = self.job().batch_size();
                while steps > 0 && !self.job().is_complete() {
                    steps -= 1;
                    match self.step(&mut d, thread, app, analyzer, renderer, fonts) {
                        Ok(drew) => {
                            // A drawn frame ends the tick; only skipped frames
                            // batch (`plug.c:8055`).
                            if drew {
                                break;
                            }
                        }
                        Err(problem) => {
                            failure = Some(problem);
                            break;
                        }
                    }
                }
            }
        }

        if let Some((title, detail)) = failure {
            app.shell.notify(Severity::Error, title, &detail);
            // A transport failure must never publish whatever partial stream the
            // encoder accepted, even if the child then exits zero
            // (`plug.c:8042-8044`).
            return self.conclude(true, music, app, analyzer);
        }
        false
    }

    /// One frame: samples in, analysis, scene update, and — inside the window —
    /// a drawn and encoded frame (`plug.c:7987-8056`).
    ///
    /// Returns whether the frame was drawn.
    #[allow(
        clippy::too_many_arguments,
        reason = "the same borrowed resources one frame of the preview loop needs"
    )]
    fn step(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        thread: &RaylibThread,
        app: &mut crate::App,
        analyzer: &mut AudioAnalyzer,
        renderer: &mut scene_host::SceneRenderer,
        fonts: &Faces,
    ) -> Result<bool, (&'static str, String)> {
        let samples = self.job_mut().take_samples().map_err(|error| {
            (
                "Export timeline failed",
                format!("{error}. The previous destination was preserved."),
            )
        })?;
        if !samples.is_empty() {
            analyzer.push_interleaved(samples);
        }
        let delta = self.job().scene_delta();
        analyzer.analyze(delta);

        let spectrum = analyzer.spectrum();
        let audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        let sources = RouteSources::from_audio(&audio_frame);
        let base = *app.settings();
        let routed = app.routes().apply(app.scene.id(), &sources, &base);
        let effective = routed.as_ref().unwrap_or(&base);
        let duration_seconds = app
            .workspace
            .current()
            .map_or(0.0, |track| track.duration_seconds);
        let frame = SceneFrame {
            time_seconds: self.job().scene_time(),
            duration_seconds,
            delta_seconds: delta,
            frame_index: self.job().frame_index(),
            audio: audio_frame,
            settings: effective,
            ..SceneFrame::idle(effective)
        };

        if !self.job().draws_this_frame() {
            // Prepared, not drawn: this is what makes a windowed export
            // bit-identical to the same frames of a full one.
            app.scene.update(&frame);
            self.job_mut().advance();
            return Ok(false);
        }

        let config = self.job().config();
        let (target_width, target_height) = (self.target.width(), self.target.height());
        let pixel_scale = config
            .target_scale(target_width as u32, target_height as u32)
            .unwrap_or(1.0);
        {
            let mut texture = d.begin_texture_mode(thread, &mut self.target);
            texture.clear_background(color::background());
            let boundary = widgets::rectangle(UiRect::new(
                0.0,
                0.0,
                target_width as f32,
                target_height as f32,
            ));
            app.scene.update(&frame);
            renderer.draw(
                &mut texture,
                fonts,
                &app.scene,
                &frame,
                boundary,
                pixel_scale,
            );
        }

        let mut image = self.target.load_image().map_err(|error| {
            (
                "Export failed while reading a frame",
                format!("{error}. The previous destination was preserved."),
            )
        })?;
        if image.width != config.width as i32 || image.height != config.height as i32 {
            image.resize(config.width as i32, config.height as i32);
        }
        // `get_image_data` is `LoadImageColors`, which for an already-RGBA
        // framebuffer read is a straight copy. Flattening it here rather than
        // reading `image.data` through a raw pointer keeps this file free of
        // `unsafe`; the cost is one pass over the frame, against an encode.
        let colors = image.get_image_data();
        self.pixels.clear();
        self.pixels.reserve(colors.len() * 4);
        for pixel in colors.iter() {
            self.pixels
                .extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        // The pixel buffer and the job are separate fields, so this needs the
        // two borrows split rather than `self.job_mut()` while `self.pixels` is
        // read.
        let ExportSession { job, pixels, .. } = self;
        let job = job
            .as_mut()
            .expect("a session is dropped on the tick its job is concluded");
        job.send_frame(pixels, config.width as usize, config.height as usize)
            .map_err(|error| {
                (
                    "Export failed while writing a frame",
                    format!("{error}. The previous destination was preserved."),
                )
            })?;
        job.advance();
        Ok(true)
    }

    /// Finalizes and restores the preview (`finish_rendering_track`,
    /// `plug.c:7252-7280`), reporting the outcome by name.
    fn conclude(
        &mut self,
        cancel: bool,
        music: Option<&Music<'_>>,
        app: &mut crate::App,
        analyzer: &mut AudioAnalyzer,
    ) -> bool {
        // `finish` consumes the job, which is why the field is an `Option`: this
        // is the one place it becomes `None`, and the caller drops the session
        // on the `true` this returns.
        let Some(job) = self.job.take() else {
            return true;
        };
        let completion = job.finish(cancel);
        match completion.result {
            Ok(Finished::Published) => app.shell.notify(
                Severity::Success,
                "Export complete",
                &format!(
                    "The video was encoded and published transactionally: {}",
                    completion.destination.display()
                ),
            ),
            Ok(Finished::Cancelled) => app.shell.notify(
                Severity::Info,
                "Export cancelled",
                "The partial file was removed; any previous output is unchanged.",
            ),
            Err(error) => app.shell.notify(
                Severity::Error,
                if cancel {
                    "Export cancellation failed"
                } else {
                    "Export failed while finishing"
                },
                &format!("{error}"),
            ),
        }
        if let Some(retained) = completion.retained_staging {
            app.shell.notify(
                Severity::Warning,
                "Temporary decoded audio was retained",
                &format!(
                    "Remove {} after confirming no export process still uses it.",
                    retained.display()
                ),
            );
        }
        restore_preview(
            music,
            app,
            analyzer,
            self.restore_position,
            self.restore_playing,
        );
        true
    }
}

/// The offline render target (`load_offline_render_target`, `plug.c:451-471`).
///
/// Supersampling is attempted first and falls back to the output resolution
/// rather than failing, because a 4K target at 2x is 8K and not every GPU will
/// allocate it. `MUSIALIZER_RENDER_SUPERSAMPLE=0` disables it, which is the
/// oracle's own escape hatch.
fn offline_render_target(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    config: &RenderExportConfig,
) -> Option<RenderTexture2D> {
    if config.validate().is_err() {
        return None;
    }
    let disabled = std::env::var_os("MUSIALIZER_RENDER_SUPERSAMPLE")
        .is_some_and(|value| value == std::ffi::OsStr::new("0"));
    let factor = if disabled {
        1
    } else {
        config.supersample_factor
    };
    if factor > 1 {
        if let Ok(target) =
            rl.load_render_texture(thread, config.width * factor, config.height * factor)
        {
            return Some(target);
        }
        eprintln!(
            "RENDER: could not create a supersampled target; falling back to the output resolution"
        );
    }
    rl.load_render_texture(thread, config.width, config.height)
        .ok()
}

/// Puts the preview back the way the export found it (`start_preview_track` plus
/// the restore at `plug.c:7275-7279`).
fn restore_preview(
    music: Option<&Music<'_>>,
    app: &mut crate::App,
    analyzer: &mut AudioAnalyzer,
    position: f32,
    playing: bool,
) {
    if let Some(track) = app.workspace.current_mut() {
        track.scene_switches.reset();
        track.cue_settings_active = false;
    }
    let Some(music) = music else {
        return;
    };
    if let Ok(configured) =
        AudioAnalyzer::boxed(AudioAnalyzerConfig::preview(music.stream.sampleRate))
    {
        *analyzer = *configured;
    }
    music.update_stream();
    music.play_stream();
    if position > 0.0 {
        music.seek_stream(position);
    }
    if !playing {
        music.pause_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_rows_highlight_only_an_exact_match() {
        // `set_active_render_config` (`plug.c:635-656`). A configuration from
        // `--resolution 1234x568` matches no button, and the panel must say so
        // by highlighting none rather than by rounding to the nearest.
        let mut config = RenderExportConfig::default();
        assert_eq!(selected_resolution(&config), Some(Resolution::P1080));
        assert_eq!(selected_frame_rate(&config), Some(FrameRate::Fps30));
        for resolution in Resolution::ALL {
            config.set_resolution(resolution);
            assert_eq!(selected_resolution(&config), Some(resolution));
        }
        for frame_rate in FrameRate::ALL {
            config.set_frame_rate(frame_rate);
            assert_eq!(selected_frame_rate(&config), Some(frame_rate));
        }
        let odd = RenderExportConfig {
            width: 1234,
            height: 568,
            fps: 25,
            ..RenderExportConfig::default()
        };
        assert_eq!(selected_resolution(&odd), None);
        assert_eq!(selected_frame_rate(&odd), None);
    }

    #[test]
    fn quality_details_name_the_encoder_settings_they_describe() {
        // These strings are the C's verbatim (`plug.c:2625-2629`) and they are a
        // promise about CRF and supersampling that `ffmpeg::ExportQuality` has
        // to keep.
        assert!(quality_detail(Quality::Balanced).contains("CRF 20"));
        assert!(quality_detail(Quality::High).contains("CRF 16"));
        assert!(quality_detail(Quality::Master).contains("CRF 12"));
        assert!(quality_detail(Quality::Balanced).contains("native resolution"));
        assert_eq!(Quality::Balanced.supersample_factor(), 1);
        assert!(quality_detail(Quality::High).contains("2x"));
        assert_eq!(Quality::High.supersample_factor(), 2);
    }

    /// The seam's contract: every request produces a command, so no control
    /// silently does nothing.
    #[test]
    fn every_control_reaches_the_application() {
        // A click must never be swallowed. This test replaces one Agent H wrote
        // to assert the opposite while the panel had no `ShellCommand`s to reach
        // — it asserted `!PANEL_WIRED` deliberately, so that flipping the flag
        // would break it and force this rewrite rather than let a stale premise
        // pass quietly.
        let mut commands = Vec::new();
        dispatch(&ExportRequest::Start, &mut commands);
        dispatch(
            &ExportRequest::Configure(RenderExportConfig::default()),
            &mut commands,
        );
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0], ShellCommand::StartRender));
        assert!(matches!(commands[1], ShellCommand::SetRenderConfig(_)));
    }
}
