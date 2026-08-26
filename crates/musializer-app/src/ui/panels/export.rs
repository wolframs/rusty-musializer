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
use musializer_core::project::frame_lanes::{FrameLaneStatus, SceneFrameTiming};
use musializer_core::render::resolve::LinearResolver;
use musializer_core::scene::routes::RouteSources;
use musializer_core::scene::{SceneAudioFrame, SceneInstance};
use musializer_core::timing::render_export::{
    self as render_export, Aspect, ClipSelection, FrameRate, Quality, RenderExportConfig,
    Resolution,
};
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

/// Indices inside [`EXPORT_WIDGETS`], for the controls added after EX2.
///
/// The older rows still index inline — SIZE at 0, FPS at 8, QUALITY at 16,
/// the footer at 24, ASPECT at 32 — and those spacings are what made room here.
/// Named rather than inline because the CLIP row's three ids are read twice
/// each (to draw and to answer the press) and a mistyped second copy would be a
/// button that highlights and never fires.
mod clip_ids {
    pub(super) const FULL_TRACK: u32 = 40;
    pub(super) const SET_IN: u32 = 41;
    pub(super) const SET_OUT: u32 = 42;
    /// The footer's still-frame button (UX0-C10).
    pub(super) const STILL: u32 = 43;
    /// Restore the ordinary first frame of the output.
    pub(super) const SHARE_NORMAL: u32 = 44;
    /// Replace encoded frame zero with the deterministic playhead frame.
    pub(super) const SHARE_PLAYHEAD: u32 = 45;
}

/// The CLIP row's readout, in the two lengths the panel can afford.
struct ClipReadout {
    long: String,
    short: String,
}

/// What the CLIP row says about the current selection.
///
/// Separated from the drawing so the wording is testable: this line is the only
/// place the interface states what an export will actually cover, and "in
/// 01:12.500 -> out 01:42.000" being wrong by a frame is invisible in a capture.
fn clip_readout(clip: ClipSelection, duration_seconds: f64, fps: u32) -> ClipReadout {
    let frames = clip
        .frames(duration_seconds, fps)
        .map_or(0, |frames| frames.end - frames.start);
    match clip.window(duration_seconds, fps) {
        None => ClipReadout {
            long: format!(
                "whole track  |  {}  |  {frames} frames",
                widgets::format_timestamp(duration_seconds)
            ),
            short: format!("whole track  |  {frames} frames"),
        },
        Some((start, length)) => {
            let long = format!(
                "in {}  ->  out {}  |  {length:.1} s  |  {frames} frames",
                widgets::format_timestamp(start),
                widgets::format_timestamp(start + length),
            );
            ClipReadout {
                long,
                short: format!(
                    "{} -> {}",
                    widgets::format_timestamp(start),
                    widgets::format_timestamp(start + length)
                ),
            }
        }
    }
}

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
    /// Publish the frame at the playhead as a PNG (UX0-C10).
    ///
    /// A command for the same reason [`Self::Start`] is one: it opens a modal
    /// destination picker and then draws into a render target, neither of which
    /// can happen inside the drawing pair this panel is called from.
    Still,
}

/// The single seam between this panel and `main.rs`.
fn dispatch(request: &ExportRequest, commands: &mut Vec<ShellCommand>) {
    match request {
        ExportRequest::Configure(config) => commands.push(ShellCommand::SetRenderConfig(*config)),
        ExportRequest::Start => commands.push(ShellCommand::StartRender),
        ExportRequest::Still => commands.push(ShellCommand::ExportStill),
    }
}

/// Which resolution preset a configuration matches, if any
/// (`set_active_render_config`, `plug.c:635-656`).
///
/// `None` is a real answer, not a fallback: `--resolution 1234x568` is a valid
/// configuration that no preset button represents, and highlighting the nearest
/// one would tell the user their export is 1080p when it is not.
fn selected_resolution(config: &RenderExportConfig) -> Option<Resolution> {
    // Matched by the *short* edge and cross-checked against an aspect preset, so
    // a vertical 1080x1920 highlights 1080p rather than nothing (EX2). The
    // cross-check is what keeps `None` a real answer: `--resolution 1234x568`
    // has a short edge of 568, which is on no rung, and even a geometry whose
    // short edge does land on one is not a preset unless its shape is.
    Aspect::of(config.width, config.height)?;
    Resolution::of_short_edge(config.width.min(config.height))
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
    /// Y offset (from the boundary's top) of the CLIP row (UX0-C01).
    ///
    /// **First, above SIZE, and that ordering is load-bearing twice.** It reads
    /// in the order the questions are asked — *which part of the track*, then
    /// what shape and how good — and, because the timeline band grows upward
    /// from the window's bottom edge while the footer stays pinned to it, a row
    /// added at the top leaves every control below it at the same screen
    /// position it had before. That is what kept EX1's and EX2's click-probe
    /// coordinates valid rather than silently aiming a press one row off.
    ///
    /// The 62 it replaced was the SIZE row's, under a full-width description
    /// line. Three control rows plus that line ask 424 px of timeline band,
    /// and a 640 px window can only give 410 — which would have put the export
    /// panel back behind its own "needs a taller timeline" notice at the
    /// smallest supported size, the exact defect review 1.4 exists about. The
    /// description moved up beside the EXPORT title, which costs no vertical
    /// space at all, and the row starts where it used to end.
    pub(super) const FIRST_ROW_Y: f32 = 36.0;
    /// Vertical distance from the CLIP row down to the SIZE/FPS row.
    pub(super) const ROW_ADVANCE: f32 = 46.0;
    /// Y offset (from the QUALITY row's own y) of the quality-detail line —
    /// the last thing this panel draws before the footer.
    pub(super) const DETAIL_OFFSET: f32 = 73.0;
    /// Point size of the quality-detail line.
    pub(super) const DETAIL_FONT_SIZE: f32 = 14.0;
    /// How far above the boundary's bottom edge the footer row sits.
    pub(super) const FOOTER_BOTTOM_MARGIN: f32 = 44.0;

    /// Y offset of the SIZE/FPS row.
    pub(super) const SECOND_ROW_Y: f32 = FIRST_ROW_Y + ROW_ADVANCE;
    /// Y offset of the QUALITY/ASPECT row.
    pub(super) const THIRD_ROW_Y: f32 = SECOND_ROW_Y + ROW_ADVANCE;
    /// Bottom of the quality-detail line — the lowest body text the footer
    /// must clear.
    pub(super) const DETAIL_BOTTOM: f32 = THIRD_ROW_Y + DETAIL_OFFSET + DETAIL_FONT_SIZE;
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
        // Beside the title rather than under it (UX0-C01). The CLIP row needs a
        // row's worth of height and the band cannot afford one at 640 px, and
        // this line is the cheapest 26 px in the panel: it is a standing
        // statement about how exports behave, not something that changes.
        // Shortened rather than clipped when the panel is narrow, for the same
        // reason the clip readout is — a sentence cut off mid-word reads as a
        // rendering fault.
        {
            let title_width = widgets::measure(font, "EXPORT", metric::UI_FONT_HEADER);
            let note_x = boundary.x + padding + title_width + 16.0;
            let available = (boundary.x + boundary.width - padding - note_x).max(0.0);
            let long = "One deterministic scene path. The destination is replaced only after the encoder succeeds.";
            let short = "The destination is replaced only after the encoder succeeds.";
            let note = if widgets::measure(font, long, metric::UI_FONT_VALUE) <= available {
                long
            } else {
                short
            };
            if widgets::measure(font, note, metric::UI_FONT_VALUE) <= available {
                widgets::draw_text(
                    &mut clip,
                    font,
                    note,
                    note_x,
                    boundary.y + padding + 4.0,
                    metric::UI_FONT_VALUE,
                    color::ui_muted(),
                );
            }
        }

        // The panel reads the *track's* configuration, never a copy of its own:
        // `--resolution` on the command line and a click here have to be the
        // same state or the buttons would disagree with the render.
        let config = input
            .workspace
            .current()
            .map_or_else(RenderExportConfig::default, |track| track.render_config);

        // CLIP, above everything else, because "which part of the track" is the
        // first question a teaser export asks (UX0-C01). The row is drawn even
        // with no track: the buttons refuse the press and the readout says why,
        // rather than the row silently not existing.
        let clip_row = UiRect::new(
            boundary.x,
            boundary.y + body_layout::FIRST_ROW_Y,
            boundary.width,
            metric::UI_BUTTON_HEIGHT,
        );
        self.clip_row(
            &mut clip,
            input,
            boundary,
            clip_row,
            config,
            input.duration_seconds,
        );

        let mut y = boundary.y + body_layout::SECOND_ROW_Y;
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

        // ASPECT, to the right of QUALITY, mirroring FPS beside SIZE (EX2).
        //
        // On an existing row rather than a fourth one on purpose: the panel's
        // height floor is *derived* from `body_layout`'s offsets all the way out
        // to `EXPORT_MIN_BAND_HEIGHT` and the timeline band's own budget, so a
        // new row would push the minimum window height up for every panel, not
        // just this one. There is room here — the quality row ends well short of
        // the box — and the SIZE|FPS pair above already establishes the reading.
        widgets::draw_text(
            &mut clip,
            font,
            "ASPECT",
            x + 8.0,
            y + 10.0,
            13.0,
            color::ui_muted(),
        );
        x += 66.0;
        let selected_aspect = Aspect::of(config.width, config.height);
        for (index, aspect) in Aspect::ALL.into_iter().enumerate() {
            let button = UiRect::new(x, y, 62.0, metric::UI_BUTTON_HEIGHT);
            if boundary.contains(button)
                && self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(EXPORT_WIDGETS, 32 + index as u32),
                        button,
                        aspect.name(),
                        selected_aspect == Some(aspect),
                        ButtonStyle::Neutral,
                        None,
                    )
                    .clicked
            {
                let mut next = config;
                next.set_aspect(aspect);
                dispatch(&ExportRequest::Configure(next), commands);
            }
            x += 62.0 + gap;
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
        // The frame estimate is the *clip's*, not the track's (UX0-C01): a
        // summary that kept counting the whole track while a 30 s window was
        // selected would be the panel's one sentence about what it is about to
        // produce, and wrong.
        let approximate_frames = self
            .export_clip
            .frames(input.duration_seconds, config.fps)
            .map_or_else(
                || {
                    (input.duration_seconds * f64::from(config.fps))
                        .ceil()
                        .max(0.0) as u64
                },
                |frames| frames.end - frames.start,
            );
        let extent = if self.export_clip.is_enabled() {
            "clip"
        } else {
            "whole track"
        };
        let summary = format!(
            "{}  |  {}x{} at {} fps  |  {}  |  {extent}, est. {approximate_frames} frames  |  {scene_label}",
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
        // The still, left of Close (UX0-C10). To the *left* of the two controls
        // that were already there, so neither of them moves: EX1's click-probe
        // coordinates aim at the footer's right edge and a shifted Close would
        // have been a silently mis-aimed press rather than a failure.
        let still = UiRect::new(close.x - 132.0 - gap, render.y, 132.0, render.height);
        // The share-preview choice, left of Save still. It uses the same
        // playhead gesture as CLIP and the still: listen/scrub to an inviting
        // image, then press once. Two explicit choices keep the default visible
        // and make undo discoverable; a toggle labelled only with the selected
        // time would not say how to get the ordinary opening frame back.
        let share_at = UiRect::new(still.x - 142.0 - gap, render.y, 142.0, render.height);
        let share_normal = UiRect::new(share_at.x - 72.0 - gap, render.y, 72.0, render.height);
        let share_label_x = share_normal.x - 104.0;
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
        // A still needs no encoder — it is a PNG, written by raylib — so it is
        // live in exactly the case the render button is not, which is worth
        // saying: a user without FFmpeg can still get a cover out of this
        // panel.
        if boundary.contains(still) {
            if input.workspace.current().is_some() {
                let still_id = widgets::widget_id(EXPORT_WIDGETS, clip_ids::STILL);
                let state = self.widgets.text_button(
                    &mut clip,
                    font,
                    still_id,
                    still,
                    "Save still",
                    false,
                    ButtonStyle::Neutral,
                    Some(footer_font),
                );
                self.widgets.hint(
                    &clip,
                    state,
                    still_id,
                    still,
                    "Write the frame at the playhead as a PNG, through the export renderer",
                );
                if state.clicked {
                    dispatch(&ExportRequest::Still, commands);
                }
            } else {
                self.widgets.disabled_button(
                    &mut clip,
                    font,
                    still,
                    "Save still",
                    Some(footer_font),
                );
            }
        }
        if boundary.contains(share_normal) && share_label_x >= boundary.x + padding {
            widgets::draw_text(
                &mut clip,
                font,
                "SHARE FRAME",
                share_label_x,
                render.y + 10.0,
                13.0,
                color::ui_muted(),
            );
            let has_track = input.workspace.current().is_some();
            if has_track {
                let normal_id = widgets::widget_id(EXPORT_WIDGETS, clip_ids::SHARE_NORMAL);
                let normal_state = self.widgets.text_button(
                    &mut clip,
                    font,
                    normal_id,
                    share_normal,
                    "Normal",
                    self.export_share_frame_seconds.is_none(),
                    ButtonStyle::Neutral,
                    Some(footer_font),
                );
                self.widgets.hint(
                    &clip,
                    normal_state,
                    normal_id,
                    share_normal,
                    "Let the output begin with its normal first timeline frame",
                );
                if normal_state.clicked {
                    self.export_share_frame_seconds = None;
                }

                let at_id = widgets::widget_id(EXPORT_WIDGETS, clip_ids::SHARE_PLAYHEAD);
                let at_label = self.export_share_frame_seconds.map_or_else(
                    || "Use playhead".to_owned(),
                    |seconds| format!("At {}", widgets::format_timestamp(seconds)),
                );
                let at_state = self.widgets.text_button(
                    &mut clip,
                    font,
                    at_id,
                    share_at,
                    &at_label,
                    self.export_share_frame_seconds.is_some(),
                    ButtonStyle::Neutral,
                    Some(footer_font),
                );
                self.widgets.hint(
                    &clip,
                    at_state,
                    at_id,
                    share_at,
                    "Use the frame at the playhead as encoded frame 1; audio, duration, and every later frame stay in place",
                );
                if at_state.clicked {
                    self.export_share_frame_seconds = Some(input.time_seconds);
                }
            } else {
                self.widgets.disabled_button(
                    &mut clip,
                    font,
                    share_normal,
                    "Normal",
                    Some(footer_font),
                );
                self.widgets.disabled_button(
                    &mut clip,
                    font,
                    share_at,
                    "Use playhead",
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

    /// The CLIP row: full track, the two playhead marks, and what they add up to
    /// (UX0-C01).
    ///
    /// **Nothing in the frozen C draws this.** `render_export_window_frames`
    /// exists there and `plug.c` calls it only from
    /// `plug_configure_render_window` — a command-line entry point — so a clip
    /// was expressible on a command line and nowhere a user could see it. The
    /// row is the missing half.
    ///
    /// The two marks read from the playhead rather than from a text field on
    /// purpose: the gesture a person actually performs is "play until it sounds
    /// right, then press", and a field would ask them to read a number off the
    /// transport and type it back in. It is also why one press is enough to get
    /// a renderable clip — [`ClipSelection::set_start`] selects to the end of
    /// the track — so "post from the drop onward" costs one click and the
    /// destination dialog.
    fn clip_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        row: UiRect,
        config: RenderExportConfig,
        duration_seconds: f64,
    ) {
        let font = input.fonts.ui();
        let gap = metric::UI_CONTROL_GAP;
        let padding = metric::UI_PANEL_PADDING;
        widgets::draw_text(
            d,
            font,
            "CLIP",
            boundary.x + padding,
            row.y + 10.0,
            13.0,
            color::ui_muted(),
        );

        let has_track = duration_seconds > 0.0;
        let mut x = boundary.x + 68.0;
        let buttons: [(u32, f32, &str, &str); 3] = [
            (
                clip_ids::FULL_TRACK,
                92.0,
                "Full track",
                "Export the whole track",
            ),
            (
                clip_ids::SET_IN,
                116.0,
                "In <- playhead",
                "Start the export at the playhead (to the end, until you set an out point)",
            ),
            (
                clip_ids::SET_OUT,
                116.0,
                "Out <- playhead",
                "End the export at the playhead",
            ),
        ];
        for (index, width, label, hint) in buttons {
            let button = UiRect::new(x, row.y, width, row.height);
            x += width + gap;
            if !boundary.contains(button) {
                continue;
            }
            let selected = match index {
                clip_ids::FULL_TRACK => !self.export_clip.is_enabled(),
                _ => false,
            };
            if !has_track {
                // Named and refused rather than absent: a row of live-looking
                // buttons over a workspace with no track is the "a control that
                // silently does nothing" failure this repository already paid
                // for once.
                self.widgets.disabled_button(d, font, button, label, None);
                continue;
            }
            let state = self.widgets.text_button(
                d,
                font,
                widgets::widget_id(EXPORT_WIDGETS, index),
                button,
                label,
                selected,
                ButtonStyle::Neutral,
                None,
            );
            self.widgets.hint(
                d,
                state,
                widgets::widget_id(EXPORT_WIDGETS, index),
                button,
                hint,
            );
            if state.clicked {
                match index {
                    clip_ids::FULL_TRACK => self.export_clip.clear(),
                    clip_ids::SET_IN => {
                        self.export_clip
                            .set_start(input.time_seconds, duration_seconds, config.fps)
                    }
                    _ => {
                        self.export_clip
                            .set_end(input.time_seconds, duration_seconds, config.fps);
                    }
                }
            }
        }

        // The readout. Long form while it fits, short form otherwise, because
        // the panel narrows with the window and a clipped readout that ends
        // mid-number would be worse than one that never showed the seconds.
        let readout = clip_readout(self.export_clip, duration_seconds, config.fps);
        let available = (boundary.x + boundary.width - padding - x - 12.0).max(0.0);
        let text = if widgets::measure(font, &readout.long, 14.0) <= available {
            readout.long
        } else {
            readout.short
        };
        if widgets::measure(font, &text, 14.0) <= available {
            widgets::draw_text(
                d,
                font,
                &text,
                x + 12.0,
                row.y + 11.0,
                14.0,
                // Accent while a clip is live, muted otherwise. The only other
                // sign the export has stopped covering the whole track is
                // "Full track" losing its highlight, and a capture showed that
                // reading as ink-on-white against muted-on-white — a
                // distinction nobody notices at a glance, on the one line that
                // says how much of their track is about to be rendered.
                if self.export_clip.is_enabled() {
                    color::accent()
                } else {
                    color::ui_muted()
                },
            );
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
pub(crate) fn ask_for_destination(
    app: &mut crate::App,
    probe_destination: Option<&Path>,
) -> Option<PathBuf> {
    use musializer_runtime::process::dialogs::{self, FileDialog};

    let track = app.workspace.current()?;
    // The oracle's suggestion, and its fallback when the source path cannot
    // produce one (`plug.c:7124-7131`).
    let scene_name = if track.scene_switches.enabled {
        "scene-plan"
    } else {
        track.base_scene.stable_name()
    };
    // A clip proposes its own name, so a teaser cannot silently replace the
    // full render of the same track and scene (UX0-C01).
    let clip = app
        .shell
        .export_clip
        .window(track.duration_seconds, track.render_config.fps);
    let suggested = track
        .file_path
        .to_str()
        .and_then(|path| match clip {
            Some((start, length)) => track
                .render_config
                .suggest_clip_path(path, scene_name, start, start + length)
                .ok(),
            None => track.render_config.suggest_path(path, scene_name).ok(),
        })
        .unwrap_or_else(|| "./musializer-render.mp4".to_string());
    // `--ui-probe save-to=` stands in for the picker, and only for it: Xvfb has
    // no file dialog any more than it has a pointer, so without this the one
    // control this panel exists for could be pressed in a capture and never
    // reach a file (EX1's argument, one step further along).
    if let Some(path) = probe_destination {
        return Some(path.to_path_buf());
    }

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

/// Publishes the frame at the playhead as a PNG (UX0-C10).
///
/// **Not the oracle's at all**: the frozen C can render a video and nothing
/// else, so a user who wanted a cover or a thumbnail had to export an MP4 and
/// take a frame out of it — through a lossy codec, at whatever moment the
/// scrubber happened to land on.
///
/// What makes this a *still of the video* rather than a screenshot:
///
/// - the frame index is [`render_export::still_frame_index`], the same floor the
///   clip window's start uses, so "still here" and "clip from here" publish the
///   same frame;
/// - every frame from zero is prepared through [`with_export_frame`] before it,
///   for the reason `render_job.rs` gives for never seeking — the analyzer, the
///   beat tracker, the scene state and the automatic plan are all path
///   dependent, so a still taken by jumping would be a different picture;
/// - it draws through [`draw_offline_frame`] into the same supersampled target
///   an export uses, and resolves with the same [`LinearResolver`] (EX3).
///
/// It deliberately does **not** need FFmpeg: a PNG is written by raylib, so this
/// is the one thing in the panel that works without an encoder installed.
///
/// Returns the published path, or `None` if the user cancelled or it failed —
/// both of which are reported in the tray by name.
#[allow(
    clippy::too_many_arguments,
    reason = "the same borrowed resources one export frame needs; see `ExportSession::begin`"
)]
pub(crate) fn export_still(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    audio: &RaylibAudio,
    music: Option<&Music<'_>>,
    app: &mut crate::App,
    analysis: &mut crate::Analysis,
    renderer: &mut scene_host::SceneRenderer,
    fonts: &Faces,
    time_seconds: f64,
    probe_destination: Option<&Path>,
) -> Option<PathBuf> {
    if app.workspace.current().is_none() {
        app.shell.notify(
            Severity::Error,
            "No still was written",
            "There is no track to render a frame from.",
        );
        return None;
    }

    let destination = ask_for_still_destination(app, time_seconds, probe_destination)?;

    let restore_position = music.map_or(0.0, Music::get_time_played);
    let restore_playing = music.is_some_and(Music::is_stream_playing);
    if let Some(music) = music {
        if !restore_playing {
            music.resume_stream();
        }
        music.stop_stream();
    }

    let prepared = prepare_frame_pixels(
        rl,
        thread,
        audio,
        app,
        analysis,
        renderer,
        fonts,
        time_seconds,
        "Rendering still frame",
    );
    restore_preview(music, app, analysis, restore_position, restore_playing);

    match prepared.and_then(|prepared| {
        write_resolved_png(&prepared.pixels, &prepared.config, &destination)?;
        Ok(prepared)
    }) {
        Ok(prepared) => {
            // The line a capture can assert, and the only evidence that the
            // still is the frame the video would have encoded: the index, not
            // just the time (EX1's argument about `export config:`).
            println!(
                "export still:    t={time_seconds:.3} frame {} of {} at \
                 {}x{} ({}x target), {}",
                prepared.frame_index,
                prepared.total_frames,
                prepared.config.width,
                prepared.config.height,
                prepared.achieved_factor,
                destination.display(),
            );
            app.shell.notify(
                Severity::Success,
                "Still frame saved",
                &format!(
                    "Frame {} at {:.3} s, rendered through the export path: {}",
                    prepared.frame_index,
                    time_seconds,
                    destination.display()
                ),
            );
            Some(destination)
        }
        Err(detail) => {
            app.shell.notify(
                Severity::Error,
                "The still frame could not be written",
                &format!("{detail}. Nothing else was changed."),
            );
            None
        }
    }
}

/// A deterministic frame prepared from track time zero, in the same bottom-up
/// RGBA layout the video encoder consumes.
struct PreparedFrame {
    pixels: Vec<u8>,
    config: RenderExportConfig,
    frame_index: u64,
    total_frames: u64,
    achieved_factor: u32,
}

/// Replays the offline scene path to one frame and reads back resolved pixels.
///
/// This is shared by PNG stills and share-preview substitution. Keeping the
/// replay here is the important part: stateful scenes, analyzer history, beat
/// tracking, routes, captions and automatic scene switches all see every frame
/// they would have seen in an ordinary export. The caller must stop preview
/// playback first and restore or reinitialize the preview/export state after.
#[allow(
    clippy::too_many_arguments,
    reason = "the same borrowed resources one offline export frame needs"
)]
fn prepare_frame_pixels(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    audio: &RaylibAudio,
    app: &mut crate::App,
    analysis: &mut crate::Analysis,
    renderer: &mut scene_host::SceneRenderer,
    fonts: &Faces,
    time_seconds: f64,
    progress_label: &str,
) -> Result<PreparedFrame, String> {
    let track = app
        .workspace
        .current()
        .ok_or_else(|| "there is no track to render a frame from".to_owned())?;
    let config = track.render_config;
    let source = track.file_path.clone();
    let duration_seconds = track.duration_seconds;
    let (scene, seed) = (track.base_scene, track.scene_seed);

    // Decoded here rather than streamed, exactly as `RenderJob::start` does:
    // the analyzer must hear the file's own samples, not raylib's stereo mix.
    let source_text = source
        .to_str()
        .ok_or_else(|| "the source audio path is not valid text".to_owned())?;
    let decoded = audio
        .new_wave(source_text)
        .ok()
        .filter(raylib::core::audio::Wave::is_wave_valid)
        .and_then(|wave| {
            let format = (wave.sample_rate(), wave.channels(), wave.frame_count());
            musializer_runtime::decode::wave_samples(&wave).map(|samples| (samples, format))
        });
    let (samples, (sample_rate, channels, frame_count)) = decoded.ok_or_else(|| {
        "the source audio decoder rejected the file, so the frame could not be prepared".to_owned()
    })?;
    let (total_frames, frame_index) =
        render_export::total_frames(u64::from(frame_count), sample_rate, config.fps)
            .and_then(|total| {
                render_export::still_frame_index(time_seconds, config.fps, total)
                    .map(|index| (total, index))
            })
            .map_err(|_| "the playhead is not on a frame this track can produce".to_owned())?;

    let (mut target, achieved_factor) =
        offline_render_target(rl, thread, &config).ok_or_else(|| {
            "the offline surface could not be created; try a lower resolution or Balanced quality"
                .to_owned()
        })?;
    analysis
        .reconfigure(AudioAnalyzerConfig {
            sample_rate,
            channel_count: channels,
            channel_mode: musializer_core::audio::ChannelMode::Select(0),
        })
        .map_err(|error| format!("the analyzer could not be configured: {error}"))?;
    app.scene = SceneInstance::new(scene_host::descriptor(scene), seed);
    if let Some(track) = app.workspace.current_mut() {
        track.scene_switches.reset();
        track.cue_settings_active = false;
    }
    prepare_track_assets(audio, app);

    // One frame of "this is happening", before a decode-and-fast-forward that
    // can take a second on a long track. Without it the window holds whatever
    // was on screen and the press reads as ignored.
    {
        let (screen_width, screen_height) = (rl.get_screen_width(), rl.get_screen_height());
        let font = fonts.ui();
        // 34 is the export progress screen's own native size. A free 30 px
        // request would quantize to 28 and make the font-bank gate report a
        // bypass, even though this transient screen still belongs to the UI.
        let width = widgets::measure(font, progress_label, 34.0);
        let mut d = rl.begin_drawing(thread);
        d.clear_background(color::background());
        widgets::draw_text(
            &mut d,
            font,
            progress_label,
            (screen_width as f32 - width) / 2.0,
            screen_height as f32 / 2.0 - 20.0,
            34.0,
            color::white(),
        );
    }

    let mut sample_cursor = 0u64;
    {
        let mut d = rl.begin_drawing(thread);
        for index in 0..=frame_index {
            // `RenderJob::take_samples` (`plug.c:7987-8011`): frame zero hears
            // nothing, and every frame after it hears exactly the samples
            // between the two cursors.
            let slice = if index == 0 {
                &[][..]
            } else {
                let next = render_export::sample_cursor(
                    index,
                    sample_rate,
                    config.fps,
                    u64::from(frame_count),
                )
                .map_err(|error| format!("{error}"))?;
                let channel_count = channels as usize;
                let from = sample_cursor as usize * channel_count;
                let to = (next as usize * channel_count).min(samples.len());
                sample_cursor = next;
                samples.get(from..to).unwrap_or(&[])
            };
            let draws = index == frame_index;
            let pixel_scale = config
                .target_scale(target.width() as u32, target.height() as u32)
                .unwrap_or(1.0);
            with_export_frame(
                app,
                analysis,
                index,
                config.fps,
                duration_seconds,
                slice,
                |app, frame, _status| {
                    if draws {
                        draw_offline_frame(
                            &mut d,
                            thread,
                            &mut target,
                            app,
                            renderer,
                            fonts,
                            frame,
                            pixel_scale,
                        );
                    } else {
                        app.scene.update(frame);
                    }
                },
            );
        }
    }

    let mut pixels = vec![0u8; config.width as usize * config.height as usize * 4];
    resolve_offline_frame(
        &mut target,
        &mut LinearResolver::new(),
        &mut pixels,
        &config,
    )?;
    Ok(PreparedFrame {
        pixels,
        config,
        frame_index,
        total_frames,
        achieved_factor,
    })
}

/// Where a still goes, following the video's own convention (UX0-C10).
fn ask_for_still_destination(
    app: &mut crate::App,
    time_seconds: f64,
    probe_destination: Option<&Path>,
) -> Option<PathBuf> {
    use musializer_runtime::process::dialogs::{FileDialog, FileFilter};

    if let Some(path) = probe_destination {
        return Some(path.to_path_buf());
    }
    let track = app.workspace.current()?;
    let scene_name = if track.scene_switches.enabled {
        "scene-plan"
    } else {
        track.base_scene.stable_name()
    };
    let suggested = track
        .file_path
        .to_str()
        .and_then(|path| {
            track
                .render_config
                .suggest_still_path(path, scene_name, time_seconds)
                .ok()
        })
        .unwrap_or_else(|| "./musializer-still.png".to_string());

    // The PNG filter is built here rather than added to `dialogs::filters`:
    // that module is the oracle's call sites, and a still is not one of them.
    let dialog = FileDialog::new("Export still frame")
        .with_filter(FileFilter {
            label: "PNG image",
            patterns: &["*.png"],
        })
        .with_default_path(&suggested);
    match dialog.save_file() {
        Ok(path) => path,
        Err(error) => {
            app.shell.notify(
                Severity::Warning,
                "No file picker is available",
                &format!("{error}. The still was not written."),
            );
            None
        }
    }
}

/// Reads the offline target back, resolves it, and writes the PNG.
///
/// The readback and the resolve are the export's, verbatim (EX3): a still that
/// averaged in gamma space would be exactly the third of the light the video
/// export used to lose, in a file whose whole purpose is to be looked at.
fn resolve_offline_frame(
    target: &mut RenderTexture2D,
    resolver: &mut LinearResolver,
    pixels: &mut Vec<u8>,
    config: &RenderExportConfig,
) -> Result<(), String> {
    let mut image = target
        .load_image()
        .map_err(|error| format!("the frame could not be read back: {error}"))?;
    let (source_width, source_height) = (image.width.max(0) as usize, image.height.max(0) as usize);
    let source = musializer_runtime::decode::image_pixels_rgba8(&mut image)
        .map_err(|error| format!("the frame could not be read back: {error}"))?;
    resolver
        .resolve(
            source,
            source_width,
            source_height,
            config.width as usize,
            config.height as usize,
            pixels,
        )
        .map_err(|error| format!("the frame could not be resolved: {error}"))
}

/// Writes already-resolved bottom-up export pixels as a conventional top-down
/// PNG. The video encoder performs the same flip while streaming raw frames.
fn write_resolved_png(
    pixels: &[u8],
    config: &RenderExportConfig,
    destination: &Path,
) -> Result<(), String> {
    let Some(path) = destination.to_str() else {
        return Err("the destination path is not valid text".to_owned());
    };
    // `Image::gen_image_color` allocates through raylib's own allocator in
    // `UNCOMPRESSED_R8G8B8A8`, which is what `ExportImage`'s PNG writer wants
    // and what lets the resolved bytes be blitted in without an `unsafe` island
    // of our own. `ImageDrawPixel` writes rather than blends, so this is a copy.
    let mut out = raylib::texture::Image::gen_image_color(
        config.width as i32,
        config.height as i32,
        Color::BLANK,
    );
    // **Bottom row first**, exactly as `Encoder::send_frame_flipped` writes to
    // `rawvideo` (`ffmpeg.rs:448-455`, `ffmpeg_posix.c:338-370`):
    // `LoadImageFromTexture`/`rlReadScreenPixels` hand back an OpenGL
    // framebuffer whose first row is the *bottom* of the picture. Without this
    // the still is a vertical mirror of the video frame — and a mirrored
    // Spectrum, with its bars hanging from the top instead of standing on the
    // baseline, is a completely plausible picture that renders identically on
    // every run. Two md5-equal runs would have called it deterministic, and did:
    // it was caught only by comparing the PNG against a frame of the MP4.
    for y in 0..config.height as i32 {
        let source_row = config.height as usize - 1 - y as usize;
        for x in 0..config.width as i32 {
            let offset = (source_row * config.width as usize + x as usize) * 4;
            out.draw_pixel(
                x,
                y,
                Color::new(
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ),
            );
        }
    }
    // `ExportImage` returns nothing at all through raylib-rs, so the file itself
    // is the only evidence it worked — checked rather than assumed, because a
    // success notice over a file that was never written is exactly the "a
    // fallback that looks like content" failure this repository has paid for.
    out.export_image(path);
    if !destination.is_file() {
        return Err(format!(
            "raylib refused to write {}; check the directory exists and is writable",
            destination.display()
        ));
    }
    Ok(())
}

/// One export frame's scene state, built the one way.
///
/// **This is what makes a still the same picture as the video frame beside it**
/// (UX0-C10). The two callers are [`ExportSession::step`] and
/// [`prepare_frame_pixels`], and every decision that shapes a frame — which samples the
/// analyzer has heard, the beat phase, the automatic scene switch, the routed
/// settings, the project lanes — happens here once rather than in two places
/// that could drift. The closure takes the assembled frame because
/// [`SceneFrame`](musializer_core::scene::SceneFrame) borrows the lanes, the
/// spectrum and the settings, none of which can be returned.
///
/// `frame_index` and `fps` are the only clock: delta and time come from
/// [`render_export::frame_delta_seconds`] and
/// [`render_export::frame_time_seconds`], which are `render_job.rs`'s own
/// definitions moved somewhere a still can reach them.
fn with_export_frame<R>(
    app: &mut crate::App,
    analysis: &mut crate::Analysis,
    frame_index: u64,
    fps: u32,
    duration_seconds: f64,
    samples: &[f32],
    draw: impl FnOnce(&mut crate::App, &musializer_core::scene::SceneFrame<'_>, FrameLaneStatus) -> R,
) -> R {
    if !samples.is_empty() {
        analysis.analyzer.push_interleaved(samples);
    }
    let delta = render_export::frame_delta_seconds(frame_index, fps);
    analysis.analyzer.analyze(delta);

    let spectrum = analysis.analyzer.spectrum();
    let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
    // The export's own clock, not the stream's — there is no stream. Routed
    // parameters staying preview/export identical is a stated invariant, so a
    // `beat_phase` route has to advance here exactly as it does in the preview,
    // and both go through the one `track_beat`.
    let time_seconds = render_export::frame_time_seconds(frame_index, fps);
    audio_frame.track_beat(&mut analysis.beat, time_seconds);
    app.apply_auto_scene_switch(time_seconds);
    let sources = RouteSources::from_audio(&audio_frame, time_seconds);
    let base = *app.settings();
    let routed = app.routes().apply(app.scene.id(), &sources, &base);
    let effective = routed.as_ref().unwrap_or(&base);
    let frame_lanes = crate::project_frame_lanes(app.workspace.current(), time_seconds);
    let lane_status = frame_lanes.status();
    let frame = frame_lanes.scene_frame(
        SceneFrameTiming {
            time_seconds,
            duration_seconds,
            delta_seconds: delta,
            frame_index,
        },
        audio_frame,
        effective,
        app.workspace
            .current()
            .and_then(|track| track.track_dynamics),
    );
    draw(app, &frame, lane_status)
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
    /// This frame's decoded samples, copied out of the job so the shared frame
    /// path can borrow the session's target at the same time. Reused between
    /// frames for the same reason [`Self::pixels`] is.
    samples_scratch: Vec<f32>,
    /// Where the transport was when the export started, so the preview comes
    /// back to it (`plug.c:6918-6919`, restored at `:7276-7279`).
    restore_position: f32,
    restore_playing: bool,
    /// The first encoded frame names the project data it saw. A rendered picture
    /// cannot prove which lane produced it, and the same line lets the headless
    /// gate compare export with a parked preview at the identical scene time.
    reported_frame_lanes: bool,
    /// Deterministic playhead pixels used only for the first frame handed to
    /// the encoder. The ordinary frame is still prepared and drawn first, so
    /// consuming this cannot change any subsequent scene state or timing.
    share_frame: Option<PreparedFrame>,
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
        renderer: &mut scene_host::SceneRenderer,
        fonts: &Faces,
        destination: &Path,
        window: Option<(f64, f64)>,
        share_frame_seconds: Option<f64>,
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

        // Prepare the selected share frame in its own replay before the real
        // export starts. `prepare_frame_pixels` resets and advances all
        // stateful lanes from track time zero; the ordinary export reset below
        // then starts from zero again. Thus the chosen image can replace only
        // encoder frame zero without leaking its future scene state into frame
        // one, shifting audio, or adding a frame to the file.
        let share_frame = match share_frame_seconds {
            None => None,
            Some(seconds) => match prepare_frame_pixels(
                rl,
                thread,
                audio,
                app,
                analysis,
                renderer,
                fonts,
                seconds,
                "Preparing share frame",
            ) {
                Ok(prepared) => {
                    println!(
                        "export share frame: playhead t={:.3} frame {} -> encoded frame 0; audio and duration unchanged",
                        render_export::frame_time_seconds(prepared.frame_index, prepared.config.fps),
                        prepared.frame_index,
                    );
                    Some(prepared)
                }
                Err(error) => {
                    app.shell.notify(
                        Severity::Error,
                        "Export was not started",
                        &format!("The selected share frame could not be prepared: {error}."),
                    );
                    restore_preview(music, app, analysis, restore_position, restore_playing);
                    return None;
                }
            },
        };

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
            encoder: app.shell.video_encoder,
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
            samples_scratch: Vec::new(),
            restore_position,
            restore_playing,
            reported_frame_lanes: false,
            share_frame,
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
        // Copied out of the job rather than borrowed across the frame build:
        // `take_samples` borrows the session, and the shared frame path needs
        // the session's own target back inside the closure. One `memcpy` of a
        // frame's worth of audio (a few thousand floats) against a whole
        // rendered frame is not a cost worth a second code path.
        {
            let ExportSession {
                job,
                samples_scratch,
                ..
            } = self;
            let job = job
                .as_mut()
                .expect("a session is dropped on the tick its job is concluded");
            let samples = job.take_samples().map_err(|error| {
                (
                    "Export timeline failed",
                    format!("{error}. The previous destination was preserved."),
                )
            })?;
            samples_scratch.clear();
            samples_scratch.extend_from_slice(samples);
        }
        let frame_index = self.job().frame_index();
        let fps = self.job().config().fps;
        // The job and the shared clock must agree, or a still taken at the same
        // index would be a different frame. Cheap enough to check every frame in
        // a debug build, and this is the one invariant that would otherwise be
        // proven only by a paragraph.
        debug_assert!(
            (render_export::frame_delta_seconds(frame_index, fps) - self.job().scene_delta()).abs()
                < f32::EPSILON
                && (render_export::frame_time_seconds(frame_index, fps) - self.job().scene_time())
                    .abs()
                    < f64::EPSILON,
            "the shared frame clock drifted from the job's"
        );
        let duration_seconds = app
            .workspace
            .current()
            .map_or(0.0, |track| track.duration_seconds);
        let draws = self.job().draws_this_frame();
        let reported = self.reported_frame_lanes;
        let pixel_scale = self
            .job()
            .config()
            .target_scale(self.target.width() as u32, self.target.height() as u32)
            .unwrap_or(1.0);

        let ExportSession {
            target,
            samples_scratch,
            ..
        } = self;
        let drew = with_export_frame(
            app,
            analysis,
            frame_index,
            fps,
            duration_seconds,
            samples_scratch,
            |app, frame, lane_status| -> Result<bool, (&'static str, String)> {
                if draws && !reported {
                    println!(
                        "export frame lanes: t={:.3} lyric={} semantic={} source={} merged-events={}",
                        frame.time_seconds,
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
                }
                if !draws {
                    // Prepared, not drawn: this is what makes a windowed export
                    // bit-identical to the same frames of a full one.
                    app.scene.update(frame);
                    return Ok(false);
                }
                draw_offline_frame(d, thread, target, app, renderer, fonts, frame, pixel_scale);
                Ok(true)
            },
        )?;
        if draws {
            self.reported_frame_lanes = true;
        }
        if !drew {
            self.job_mut().advance();
            return Ok(false);
        }
        let config = self.job().config();

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
        let ExportSession {
            job,
            pixels,
            share_frame,
            ..
        } = self;
        let job = job
            .as_mut()
            .expect("a session is dropped on the tick its job is concluded");
        let uses_share_frame = job.encoded_frames() == 0 && share_frame.is_some();
        let encoded_pixels = if uses_share_frame {
            &share_frame.as_ref().expect("checked above").pixels
        } else {
            pixels
        };
        job.send_frame(
            encoded_pixels,
            config.width as usize,
            config.height as usize,
        )
        .map_err(|error| {
            (
                "Export failed while writing a frame",
                format!("{error}. The previous destination was preserved."),
            )
        })?;
        if uses_share_frame {
            // Release an 8 MiB buffer at 1080p as soon as the only frame that
            // can use it is safely in the encoder pipe.
            *share_frame = None;
        }
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

/// Draws one frame into the offline target, exactly as an encoded frame is
/// drawn.
///
/// The second half of the "a still is the video frame" claim (UX0-C10):
/// [`with_export_frame`] shares the state and this shares the draw, so the only
/// thing the still does differently is where the pixels go afterwards.
#[allow(
    clippy::too_many_arguments,
    reason = "the borrowed resources one export frame needs; a bundle struct would move the same list one line up, and both callers already hold them separately"
)]
fn draw_offline_frame(
    d: &mut RaylibDrawHandle<'_>,
    thread: &RaylibThread,
    target: &mut RenderTexture2D,
    app: &mut crate::App,
    renderer: &mut scene_host::SceneRenderer,
    fonts: &Faces,
    frame: &musializer_core::scene::SceneFrame<'_>,
    pixel_scale: f32,
) {
    let (target_width, target_height) = (target.width(), target.height());
    let mut texture = d.begin_texture_mode(thread, target);
    texture.clear_background(color::background());
    let boundary = widgets::rectangle(UiRect::new(
        0.0,
        0.0,
        target_width as f32,
        target_height as f32,
    ));
    app.scene.update(frame);
    // The same per-track assets the preview draws. Song Atlas's terrain is
    // built once when the export starts rather than on demand here, because
    // an export cannot pause to decode a whole track mid-timeline — see
    // `prepare_track_assets`.
    let assets = app
        .workspace
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
        frame,
        assets,
        boundary,
        pixel_scale,
    );
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

    /// The CLIP row's own ids, and every other index this panel mints, are
    /// distinct (UX0-C01).
    ///
    /// `widgets::id::ALL` protects *namespaces*; nothing protects the indices
    /// inside one, and this panel now allocates five groups by hand — 0, 8, 16,
    /// 24 and 32 — with the clip row wedged in at 40. EX1 is the whole argument
    /// for why an index collision is worth a test rather than a careful read:
    /// two controls with one id draw correctly, highlight correctly, and one of
    /// them silently never takes a press.
    #[test]
    fn the_panels_own_widget_indices_never_collide() {
        let mut used: Vec<(u32, &str)> = Vec::new();
        let mut claim = |index: u32, name: &'static str| {
            if let Some((_, other)) = used.iter().find(|(taken, _)| *taken == index) {
                panic!("export index {index} is claimed by both {other} and {name}");
            }
            used.push((index, name));
        };
        for index in 0..Resolution::ALL.len() as u32 {
            claim(index, "SIZE");
        }
        for index in 0..FrameRate::ALL.len() as u32 {
            claim(8 + index, "FPS");
        }
        for index in 0..Quality::ALL.len() as u32 {
            claim(16 + index, "QUALITY");
        }
        claim(24, "render");
        claim(25, "close");
        claim(26, "cancel");
        for index in 0..Aspect::ALL.len() as u32 {
            claim(32 + index, "ASPECT");
        }
        claim(clip_ids::FULL_TRACK, "clip full track");
        claim(clip_ids::SET_IN, "clip in");
        claim(clip_ids::SET_OUT, "clip out");
        claim(clip_ids::STILL, "save still");
        claim(clip_ids::SHARE_NORMAL, "normal share frame");
        claim(clip_ids::SHARE_PLAYHEAD, "playhead share frame");
        // 4 sizes + 3 rates + 3 qualities + 3 footer + 4 aspects + 6 clip/share.
        assert_eq!(used.len(), 23);
    }

    /// The CLIP row's readout says what the export will cover, in whichever
    /// length the panel can afford.
    #[test]
    fn the_clip_readout_names_the_window_and_its_frames() {
        let (duration, fps) = (8.0, 30u32);
        let full = clip_readout(ClipSelection::full_track(), duration, fps);
        assert_eq!(full.long, "whole track  |  00:08.000  |  240 frames");
        assert_eq!(full.short, "whole track  |  240 frames");

        let mut clip = ClipSelection::full_track();
        clip.set_start(2.0, duration, fps);
        clip.set_end(5.0, duration, fps);
        let readout = clip_readout(clip, duration, fps);
        assert_eq!(
            readout.long,
            "in 00:02.000  ->  out 00:05.000  |  3.0 s  |  90 frames"
        );
        assert_eq!(readout.short, "00:02.000 -> 00:05.000");
        // The short form is what a narrow panel falls back to, so it must
        // really be shorter — a "fallback" that does not fit is not one.
        assert!(readout.short.len() < readout.long.len());

        // No track: a sentence rather than a divide-by-zero or a lie.
        let empty = clip_readout(ClipSelection::full_track(), 0.0, fps);
        assert_eq!(empty.long, "whole track  |  00:00.000  |  0 frames");
    }

    /// **The invariant that let a third row be added without moving anything.**
    ///
    /// The export panel's boundary is pinned to the window's bottom edge and the
    /// timeline band grows upward, so the *distance from the bottom* of the
    /// content box is what fixes a control's screen position. Every gate
    /// coordinate EX1 and EX2 aimed by hand — 542 for the SIZE row, 589 for
    /// QUALITY — depends on this number being unchanged, and a press aimed one
    /// row off does not fail: it presses a different control and asserts against
    /// its result.
    ///
    /// 185 is the SIZE row's distance from the bottom of the minimum content
    /// box before the CLIP row existed (`247 - 62`). Adding a row above it moved
    /// both the row and the box by the same amount, which is why it still holds.
    #[test]
    fn adding_a_row_above_size_leaves_every_control_below_it_in_place() {
        assert_eq!(
            EXPORT_CONTENT_MIN_HEIGHT - body_layout::SECOND_ROW_Y,
            185.0,
            "the SIZE row moved relative to the panel's bottom edge; every \
             click-probe coordinate in tools/headless_check.sh is now aimed \
             one row off"
        );
        assert_eq!(
            EXPORT_CONTENT_MIN_HEIGHT - body_layout::THIRD_ROW_Y,
            139.0,
            "the QUALITY/ASPECT row moved relative to the panel's bottom edge"
        );
        // And the CLIP row is genuinely a row above SIZE, not an overlap.
        // A compile-time comparison, so it is a build failure rather than a
        // test failure — the same shape as the band-chrome assertion LX1-c
        // kept.
        const _: () = assert!(
            body_layout::FIRST_ROW_Y + metric::UI_BUTTON_HEIGHT <= body_layout::SECOND_ROW_Y
        );
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
