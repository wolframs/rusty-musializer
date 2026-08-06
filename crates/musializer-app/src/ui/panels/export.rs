//! The export panel, the progress screen, and the session that drives them.
//!
//! C sources: `draw_export_panel` (`plug.c:2543-2659`),
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
//! loop needs all three only at the session boundary. Keeping the transport here
//! makes `main.rs`'s share one call while preview and export still use the same
//! project-frame builder.
use std::path::{Path, PathBuf};

use musializer_core::audio::AudioAnalyzerConfig;
use musializer_core::project::frame_lanes::SceneFrameTiming;
use musializer_core::render::resolve::LinearResolver;
use musializer_core::scene::routes::RouteSources;
use musializer_core::scene::{SceneAudioFrame, SceneInstance};
use musializer_core::timing::render_export::{FrameRate, Quality, RenderExportConfig, Resolution};
use musializer_core::ui::notice::Severity;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::Faces;
use musializer_runtime::process::ffmpeg::{ffmpeg_available, Finished};
use musializer_runtime::process::render_job::{RenderJob, RenderRequest};
use raylib::prelude::{
    Color, Music, RaylibAudio, RaylibDraw, RaylibDrawHandle, RaylibHandle, RaylibMode2DExt,
    RaylibTexture2D, RaylibTextureModeExt, RaylibThread, RenderTexture2D,
};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::cli::UiPanel;
use crate::scene_host;

/// Widget id namespace for this panel.
///
/// Kept beside the panel because only its controls use this namespace; the value
/// aliases the central allocation so collisions remain testable in one place.
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

/// Named y-offsets for [`Shell::export_panel`]'s body, in the order they are
/// drawn (review 1.4). These used to be bare literals scattered through the
/// function; naming them is what lets [`EXPORT_CONTENT_MIN_HEIGHT`] be a sum
/// of the same numbers the drawing code uses; rather than a second, independently
/// hand-picked guess that could drift out of agreement with the layout it is
/// supposed to describe.
mod body_layout {
    /// Y offset (from the boundary's top) of the SIZE/FPS control row.
    pub(super) const FIRST_ROW_Y: f32 = 62.0;
    /// Vertical distance from the SIZE/FPS row down to the QUALITY row.
    pub(super) const ROW_ADVANCE: f32 = 46.0;
    /// Y offset (from the QUALITY row's own y) of the quality-detail line —
    /// the last thing this panel draws before the footer.
    pub(super) const DETAIL_OFFSET: f32 = 73.0;
    /// Point size of the quality-detail line.
    pub(super) const DETAIL_FONT_SIZE: f32 = 14.0;
    /// How far above the boundary's bottom edge the footer row sits.
    pub(super) const FOOTER_BOTTOM_MARGIN: f32 = 44.0;

    /// Y offset of the QUALITY row.
    pub(super) const SECOND_ROW_Y: f32 = FIRST_ROW_Y + ROW_ADVANCE;
    /// Bottom of the quality-detail line — the lowest body text the footer
    /// must clear.
    pub(super) const DETAIL_BOTTOM: f32 = SECOND_ROW_Y + DETAIL_OFFSET + DETAIL_FONT_SIZE;
}

/// The least content-box (`boundary`) height at which [`Shell::export_panel`]
/// draws its body instead of the one-line "too small" notice.
///
/// Derived from [`body_layout`]'s offsets plus [`metric::UI_CONTROL_GAP`] as
/// the buffer between the quality-detail line and the footer row — the same
/// spacing unit this panel already uses between its own controls, rather than
/// a new invented margin. Review 1.4 found the panel's old gate (a bare
/// `80.0`) was never checked against this arithmetic and was well short of it.
pub(crate) const EXPORT_CONTENT_MIN_HEIGHT: f32 =
    body_layout::DETAIL_BOTTOM + metric::UI_CONTROL_GAP + body_layout::FOOTER_BOTTOM_MARGIN;

/// This panel's own gap between the timeline strip above it and its boundary
/// (`top = strip.y + strip.height + 28.0`, in [`Shell::export_panel`] below).
/// Named so [`EXPORT_MIN_BAND_HEIGHT`] can reference the same number the
/// function actually adds, instead of repeating the literal.
const STRIP_GAP: f32 = 28.0;

/// The timeline strip's height cap, at generous room.
///
/// This is `shell.rs::timeline_strip`'s `56.0f32.min(...)` literal, duplicated
/// rather than imported: that literal lives in a file this panel does not own
/// (see the file-ownership rule in `AGENTS.md`), and there is no constant
/// there yet to reference. If that literal changes, this one has to move with
/// it — recorded here so the next reader looking for why [`EXPORT_MIN_BAND_HEIGHT`]
/// is what it is does not have to rediscover the dependency.
const ASSUMED_STRIP_HEIGHT: f32 = 56.0;

/// How much of the *band* — the timeline height `Shell::resolved_timeline_height`
/// hands to the whole strip — is spent before this panel's own `boundary` ever
/// begins: the TIMELINE panel header (`widgets::panel` consumes it before
/// returning the content rect — a real 27 px this constant once missed, caught
/// only by measuring a capture), two lots of panel padding (one before the
/// manual event row, one at the bottom of this panel's own boundary
/// calculation), the manual event row, the scene-plan lane, the timeline strip
/// itself, and this panel's own gap under the strip. Traced in
/// `shell.rs::timeline_strip` and [`Shell::export_panel`]'s `boundary`
/// construction.
///
/// [`EVENT_ROW_HEIGHT`](super::events::EVENT_ROW_HEIGHT) and
/// [`SCENE_SECTION_HEIGHT`](super::scene_timeline::SCENE_SECTION_HEIGHT) are
/// imported rather than duplicated, so a change to either row's budget cannot
/// silently reopen this gap the way the `260.0` floor did.
const BAND_TO_BOUNDARY_OFFSET: f32 = widgets::PANEL_HEADER_HEIGHT
    + metric::UI_PANEL_PADDING * 2.0
    + super::events::EVENT_ROW_HEIGHT
    + super::scene_timeline::SCENE_SECTION_HEIGHT
    + ASSUMED_STRIP_HEIGHT
    + STRIP_GAP;

/// The minimum band height `Shell::resolved_timeline_height`'s Export floor
/// must supply so this panel's `boundary` clears [`EXPORT_CONTENT_MIN_HEIGHT`]
/// and draws its controls, rather than the one-line notice.
///
/// Single source of truth for review 1.4: the floor used to be a bare `260.0`
/// that nothing checked against what this function actually consumes above
/// its own boundary, and a user who persisted a shorter timeline height (legal
/// in every other panel) got a lit Export button over a blank white band with
/// no path to an MP4, across restarts. See the session report for the
/// `shell.rs` edit this constant is meant to replace `260.0` with.
pub(crate) const EXPORT_MIN_BAND_HEIGHT: f32 = EXPORT_CONTENT_MIN_HEIGHT + BAND_TO_BOUNDARY_OFFSET;

impl Shell {
    /// The export panel, in the timeline strip's place
    /// (`draw_export_panel`, `plug.c:2543-2659`).
    ///
    /// The oracle draws its body only when its box clears 80 px (`plug.c:3141`):
    /// a panel that cannot host its own controls must not register clicks for
    /// invisible ones. This port keeps that refusal — at
    /// [`EXPORT_CONTENT_MIN_HEIGHT`], not the oracle's bare `80.0` — but does
    /// not go silent the way `plug.c` does: review 1.4 found a box that missed
    /// the threshold by a few pixels rendering as an unexplained blank band
    /// under a lit-up Export button, so below that height this draws a
    /// one-line "too small" notice instead of nothing.
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
        if boundary.width <= 0.0 {
            // No box at all — not even a rectangle to put a notice in.
            return;
        }

        let font = input.fonts.ui();
        widgets::fill(d, boundary, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, color::ui_rule());

        // Strictly less: `EXPORT_CONTENT_MIN_HEIGHT` is by its own derivation
        // the *least sufficient* height (the footer exactly clears the detail
        // row there), and the derived band floor lands the boundary on exactly
        // that value — `<=` here drew the notice at the very size the floor
        // guarantees.
        if boundary.height < EXPORT_CONTENT_MIN_HEIGHT {
            // Named and explained, not blank (review 1.4): a lit-up Export
            // button that opens onto a blank white band, with no path to an
            // MP4, is worse than a panel that says it needs more room.
            // `Shell::resolved_timeline_height`'s Export floor is meant to
            // keep this from being reachable in practice (it now derives
            // from `EXPORT_MIN_BAND_HEIGHT`, the band-height counterpart of
            // this constant); this is the fallback for whatever gets past it
            // regardless — a hostile `ui.json`, or a window too short for
            // even the floor to fit.
            widgets::draw_text(
                d,
                font,
                "EXPORT",
                boundary.x + padding,
                boundary.y + 6.0,
                metric::UI_FONT_CAPTION,
                color::accent(),
            );
            widgets::draw_text(
                d,
                font,
                "needs a taller timeline than this window has room for",
                boundary.x + padding,
                boundary.y + 24.0,
                metric::UI_FONT_CAPTION,
                color::ui_warning(),
            );
            return;
        }

        // Everything inside is clipped to the box, so a narrow window cuts a
        // label off rather than printing it across the tracks rail.
        let mut clip = widgets::begin_scissor(d, boundary, input.ui_scale);

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

        let mut y = boundary.y + body_layout::FIRST_ROW_Y;
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

        y += body_layout::ROW_ADVANCE;
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
            y + body_layout::DETAIL_OFFSET,
            body_layout::DETAIL_FONT_SIZE,
            color::ui_muted(),
        );
        // The footer. `ffmpeg_available` is called once and used twice, as the
        // C calls it twice per frame (`plug.c:2639`, `:2644`) — a cached answer
        // would keep saying "FFmpeg required" after the user installed it.
        let encoder_present = ffmpeg_available();
        let render = UiRect::new(
            boundary.x + boundary.width - padding - 212.0,
            boundary.y + boundary.height - body_layout::FOOTER_BOTTOM_MARGIN,
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
        ui_scale: super::super::scale::UiScale,
        job: &RenderJob,
    ) -> bool {
        let (w, h) =
            ui_scale.logical_size((d.get_screen_width() as f32, d.get_screen_height() as f32));
        let font = fonts.ui();
        self.widgets.begin_frame(ui_scale);
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
            let mut clip = widgets::begin_scissor(
                d,
                UiRect::new(bar.x, bar.y + 27.0, bar.width, 24.0),
                ui_scale,
            );
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
    /// The supersample resolve's sRGB tables, built once for the whole export
    /// rather than per frame (EX3).
    resolver: LinearResolver,
    /// Where the transport was when the export started, so the preview comes
    /// back to it (`plug.c:6918-6919`, restored at `:7276-7279`).
    restore_position: f32,
    restore_playing: bool,
    /// The first encoded frame names the project data it saw. A rendered picture
    /// cannot prove which lane produced it, and the same line lets the headless
    /// gate compare export with a parked preview at the identical scene time.
    reported_frame_lanes: bool,
}

/// Builds any whole-track data the export's frames will need, before the first one
/// (`plug.c:6945-6955`).
///
/// The preview builds Song Atlas's terrain lazily, at the first frame that would
/// draw it, and pauses the audio stream while it decodes. An export has neither
/// affordance: there is no stream to pause, and a mid-timeline decode would stall
/// the encoder's pipe for as long as the decode takes. So the C hoists the build to
/// export start, and gates it on [`Track::uses_song_atlas`] rather than on the live
/// scene — a windowed render can pass through a Song Atlas *cue* the preview never
/// showed.
///
/// A failure here is deliberately not fatal. The C warns and renders anyway
/// (`plug.c:6950-6953`), and the scene falls back to its live idle terrain, which is
/// a worse video but a video.
fn prepare_track_assets(audio: &RaylibAudio, app: &mut crate::App) {
    let needed = app
        .workspace
        .current()
        .is_some_and(|track| track.uses_song_atlas() && track.atlas_map().is_none());
    if !needed {
        return;
    }
    if let Some(track) = app.workspace.current_mut() {
        // The one place the "do not retry" flag is deliberately cleared. The C
        // checks only `song_atlas_map_valid` here, not `attempted`
        // (`plug.c:6944-6946`), so an export retries a build the preview already
        // failed. That is not an oversight: the flag exists to keep a failing decode
        // out of the *frame loop*, and an export is a one-shot setup step where the
        // cost is paid once against a whole encode.
        track.song_atlas_map_attempted = false;
    }
    crate::ensure_song_atlas_map(audio, app, None);
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
        analysis: &mut crate::Analysis,
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
            // raylib only resets a paused buffer from its playing state.
            if !restore_playing {
                music.resume_stream();
            }
            music.stop_stream();
        }

        let (target, achieved_factor) = match offline_render_target(rl, thread, &config) {
            Some(pair) => pair,
            None => {
                app.shell.notify(
                    Severity::Error,
                    "Export surface could not be created",
                    "Try a lower resolution or Balanced quality.",
                );
                restore_preview(music, app, analysis, restore_position, restore_playing);
                return None;
            }
        };
        // What the pipeline is *actually* doing, printed once at the start
        // (EX3). Not one line of it was reported before: a supersample that
        // silently fell back produced a softer video with no message a user
        // would see, and the resolve's colour space — the thing that made High
        // and Master render thin bright detail worse than Balanced — was a
        // property of a raylib call nobody had read.
        println!(
            "export pipeline: {}x{} out, {}x{} target ({}x asked, {achieved_factor}x got), \
             {} resolve, quality {}",
            config.width,
            config.height,
            target.width(),
            target.height(),
            config.supersample_factor,
            if achieved_factor > 1 {
                "linear-light box"
            } else {
                "none (copy)"
            },
            config.quality.name(),
        );
        if achieved_factor < config.supersample_factor {
            app.shell.notify(
                Severity::Warning,
                "Supersampling was not available",
                &format!(
                    "{} asks for {}x, but a {}x{} target could not be allocated. \
                     This export will render at the output resolution and will be softer.",
                    config.quality.name(),
                    config.supersample_factor,
                    config.width * config.supersample_factor,
                    config.height * config.supersample_factor,
                ),
            );
        }

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
                restore_preview(music, app, analysis, restore_position, restore_playing);
                return None;
            }
        };

        // The analyzer follows the decoded wave, not the preview's stereo mix
        // (`plug.c:7094`), and the scene restarts from the track's seed so that
        // the export does not inherit whatever state the preview had wandered
        // into (`plug.c:7019`).
        match analysis.reconfigure(job.analyzer_config()) {
            Ok(()) => {}
            Err(error) => {
                app.shell.notify(
                    Severity::Error,
                    "Export was not started",
                    &format!("The analyzer could not be configured for export: {error}"),
                );
                let _ = job.finish(true);
                restore_preview(music, app, analysis, restore_position, restore_playing);
                return None;
            }
        }
        app.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
        if let Some(track) = app.workspace.current_mut() {
            track.scene_switches.reset();
            track.cue_settings_active = false;
        }
        prepare_track_assets(audio, app);
        app.shell.panel = UiPanel::None;

        let pixels = vec![0u8; config.width as usize * config.height as usize * 4];
        Some(ExportSession {
            job: Some(job),
            target,
            pixels,
            resolver: LinearResolver::new(),
            restore_position,
            restore_playing,
            reported_frame_lanes: false,
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
        analysis: &mut crate::Analysis,
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
            return self.conclude(true, music, app, analysis);
        }
        if self.job().is_complete() && self.job_mut().begin_finishing() {
            return self.conclude(false, music, app, analysis);
        }

        let mut failure: Option<(&'static str, String)> = None;
        {
            let mut d = rl.begin_drawing(thread);
            let ui_scale =
                super::super::scale::UiScale::new(fonts.ui().scale()).unwrap_or_default();
            let cancelled = {
                let mut ui_draw = d.begin_mode2D(ui_scale.camera());
                app.shell
                    .export_progress(&mut ui_draw, fonts, ui_scale, self.job())
            };
            if cancelled {
                self.job_mut().request_cancel();
            }
            let active = !self.job().is_finishing() && !self.job().cancel_requested();
            if active {
                let mut steps = self.job().batch_size();
                while steps > 0 && !self.job().is_complete() {
                    steps -= 1;
                    match self.step(&mut d, thread, app, analysis, renderer, fonts) {
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
            return self.conclude(true, music, app, analysis);
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
        analysis: &mut crate::Analysis,
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
            analysis.analyzer.push_interleaved(samples);
        }
        let delta = self.job().scene_delta();
        analysis.analyzer.analyze(delta);

        let spectrum = analysis.analyzer.spectrum();
        let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        // The export's own clock, not the stream's — there is no stream. Routed
        // parameters staying preview/export identical is a stated invariant, so a
        // `beat_phase` route has to advance here exactly as it does in the preview,
        // and both go through the one `track_beat`.
        audio_frame.track_beat(&mut analysis.beat, self.job().scene_time());
        let time_seconds = self.job().scene_time();
        app.apply_auto_scene_switch(time_seconds);
        let sources = RouteSources::from_audio(&audio_frame, time_seconds);
        let base = *app.settings();
        let routed = app.routes().apply(app.scene.id(), &sources, &base);
        let effective = routed.as_ref().unwrap_or(&base);
        let duration_seconds = app
            .workspace
            .current()
            .map_or(0.0, |track| track.duration_seconds);
        let frame_lanes = crate::project_frame_lanes(app.workspace.current(), time_seconds);
        let lane_status = frame_lanes.status();
        let frame = frame_lanes.scene_frame(
            SceneFrameTiming {
                time_seconds,
                duration_seconds,
                delta_seconds: delta,
                frame_index: self.job().frame_index(),
            },
            audio_frame,
            effective,
        );

        if self.job().draws_this_frame() && !self.reported_frame_lanes {
            println!(
                "export frame lanes: t={time_seconds:.3} lyric={} semantic={} source={} merged-events={}",
                lane_status
                    .lyric_id
                    .map_or_else(|| "none".to_owned(), |id| id.to_string()),
                if lane_status.semantic_available {
                    "available"
                } else {
                    "unavailable"
                },
                lane_status.semantic_source_id,
                lane_status.merged_event_count,
            );
            self.reported_frame_lanes = true;
        }

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
            // The same per-track assets the preview draws. Song Atlas's terrain is
            // built once when the export starts rather than on demand here, because
            // an export cannot pause to decode a whole track mid-timeline — see
            // `prepare_track_assets`.
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
            renderer.draw(
                &mut texture,
                fonts,
                &app.scene,
                &frame,
                assets,
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
        // Read before the pixel borrow: `image_pixels_rgba8` takes `&mut`, and
        // the returned slice keeps that borrow alive for the resolve.
        let (source_width, source_height) =
            (image.width.max(0) as usize, image.height.max(0) as usize);
        // The readback, the supersample resolve and the flatten to RGBA bytes,
        // in one pass instead of four (EX3).
        //
        // This used to be `Image::resize` followed by `get_image_data` followed
        // by a per-pixel rebuild of the byte vector. `Image::resize` is
        // `ImageResize`, whose 8-bit fast path calls `stbir_resize_uint8_linear`
        // (`rtextures.c:1770-1773`) — the variant that treats its input as
        // *already linear*. Averaging sRGB code values as though they were light
        // darkens every high-contrast edge, so High and Master were resolving
        // thin bright detail worse than Balanced, which never resamples at all.
        // `core::render::resolve` does the same average in linear light, and
        // being pure arithmetic it is pinned by a number rather than by a
        // capture. `get_image_data` also goes, being the third raylib-rs wrapper
        // that forms a slice from `LoadImageColors` without a null check.
        let source =
            musializer_runtime::decode::image_pixels_rgba8(&mut image).map_err(|error| {
                (
                    "Export failed while reading a frame",
                    format!("{error}. The previous destination was preserved."),
                )
            })?;
        self.resolver
            .resolve(
                source,
                source_width,
                source_height,
                config.width as usize,
                config.height as usize,
                &mut self.pixels,
            )
            .map_err(|error| {
                (
                    "Export failed while resolving a frame",
                    format!(
                        "{error}: a {source_width}x{source_height} target for a \
                         {}x{} output. The previous destination was preserved.",
                        config.width, config.height
                    ),
                )
            })?;
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
        analysis: &mut crate::Analysis,
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
            analysis,
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
///
/// Returns the target **and the factor it actually got**, because the two can
/// differ and the difference is invisible: the panel says "Master uses 2x
/// spatial supersampling", the fallback said so only on stderr, and the
/// resulting MP4 is a perfectly plausible softer video. That is the same shape
/// as the three `None` fallbacks this repository shipped unreviewed for two
/// bands — a picture that looks like content while the feature is off.
fn offline_render_target(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    config: &RenderExportConfig,
) -> Option<(RenderTexture2D, u32)> {
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
            return Some((target, factor));
        }
        eprintln!(
            "RENDER: could not create a supersampled target; falling back to the output resolution"
        );
    }
    rl.load_render_texture(thread, config.width, config.height)
        .ok()
        .map(|target| (target, 1))
}

/// Puts the preview back the way the export found it (`start_preview_track` plus
/// the restore at `plug.c:7275-7279`).
fn restore_preview(
    music: Option<&Music<'_>>,
    app: &mut crate::App,
    analysis: &mut crate::Analysis,
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
    // Back to the preview configuration, and the beat tracker resets with it: the
    // export just drove the analyzer over the whole track, so the tempo it learned
    // belongs to an offline pass rather than to the stream about to resume.
    let _ = analysis.reconfigure(AudioAnalyzerConfig::preview(music.stream.sampleRate));
    if position > 0.0 {
        music.seek_stream(position);
    }
    // Export stopped the stream, so no callback can race this reset. Any live
    // preview PCM in the ring predates the offline pass and must not be analyzed
    // after it. Prime the decoder at the restored position before playback.
    if let Some(ring) = musializer_runtime::audio_bridge::ring() {
        ring.reset();
    }
    music.update_stream();
    music.play_stream();
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
        // A click must never be swallowed. This replaces the temporary test that
        // asserted the opposite while the panel had no `ShellCommand`s to reach
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

    /// Review 1.4's core claim, pinned as arithmetic: at exactly
    /// [`EXPORT_CONTENT_MIN_HEIGHT`], the footer row must not overlap the
    /// quality-detail line above it, and one pixel less must. This is the
    /// same comparison `export_panel` makes (`boundary.height <=
    /// EXPORT_CONTENT_MIN_HEIGHT`) and the same offsets it draws with
    /// ([`body_layout`]), so a future edit to any offset in that module
    /// breaks this test instead of silently reopening the overlap.
    #[test]
    fn export_content_min_height_is_exactly_where_the_footer_stops_overlapping() {
        let detail_bottom = body_layout::DETAIL_BOTTOM;
        // The height at which the footer sits flush against the detail line,
        // with no slack at all.
        let tight_min = detail_bottom + body_layout::FOOTER_BOTTOM_MARGIN;

        // EXPORT_CONTENT_MIN_HEIGHT is the tight minimum plus one control gap
        // of breathing room — the same spacing unit this panel already uses
        // between its own controls — not the tight minimum itself.
        assert_eq!(
            EXPORT_CONTENT_MIN_HEIGHT,
            tight_min + metric::UI_CONTROL_GAP
        );

        let footer_top = EXPORT_CONTENT_MIN_HEIGHT - body_layout::FOOTER_BOTTOM_MARGIN;
        assert!(
            footer_top >= detail_bottom,
            "footer starts at {footer_top}, before the detail line ends at {detail_bottom}"
        );

        // One pixel inside the safety gap — below the *tight* minimum — is
        // where the footer actually starts overlapping the line above it.
        let one_pixel_into_the_gap = tight_min - 1.0;
        let footer_top_short = one_pixel_into_the_gap - body_layout::FOOTER_BOTTOM_MARGIN;
        assert!(
            footer_top_short < detail_bottom,
            "the tight minimum is not pinned: {one_pixel_into_the_gap} still leaves room"
        );
    }

    /// The band-height counterpart of the test above: replays the exact chain
    /// `shell.rs::timeline_strip` and `Shell::export_panel` use to turn a band
    /// height into `boundary.height` — panel padding, the manual event row,
    /// the scene-plan lane, the timeline strip, and this panel's own gap
    /// underneath it — and asserts that handing it [`EXPORT_MIN_BAND_HEIGHT`]
    /// produces *exactly* [`EXPORT_CONTENT_MIN_HEIGHT`] of boundary, not
    /// merely "enough". `EVENT_ROW_HEIGHT` and `SCENE_SECTION_HEIGHT` are the
    /// real constants those modules export, not copies, so a change to either
    /// row's budget moves this test rather than being missed by it.
    #[test]
    fn export_min_band_height_reproduces_the_panels_own_boundary_formula() {
        let padding = metric::UI_PANEL_PADDING;
        let row = super::super::events::EVENT_ROW_HEIGHT;
        let scene_row = super::super::scene_timeline::SCENE_SECTION_HEIGHT;

        // The band, exactly as `Shell::timeline_strip` receives it — and then
        // the TIMELINE header `widgets::panel` consumes before handing back the
        // content rect. The first version of this test skipped that step and
        // passed while a real capture showed the too-small notice: the replay
        // must include every rect transformation on the real path.
        let band = UiRect::new(0.0, 0.0, 1280.0, EXPORT_MIN_BAND_HEIGHT);
        let content = UiRect::new(
            band.x,
            band.y + widgets::PANEL_HEADER_HEIGHT,
            band.width,
            band.height - widgets::PANEL_HEADER_HEIGHT,
        );

        // `shell.rs::timeline_strip`'s construction of `strip`.
        let strip_y = content.y + padding + row + scene_row;
        let strip_height =
            ASSUMED_STRIP_HEIGHT.min((content.height - padding * 2.0 - row - scene_row).max(0.0));

        // `Shell::export_panel`'s construction of `boundary`.
        let top = strip_y + strip_height + STRIP_GAP;
        let boundary_height = (content.y + content.height - top - padding).max(0.0);

        assert!(
            (boundary_height - EXPORT_CONTENT_MIN_HEIGHT).abs() < 0.01,
            "EXPORT_MIN_BAND_HEIGHT ({}) should reproduce EXPORT_CONTENT_MIN_HEIGHT \
             ({}) through the panel's own formula, got {boundary_height}",
            EXPORT_MIN_BAND_HEIGHT,
            EXPORT_CONTENT_MIN_HEIGHT,
        );
    }
}
