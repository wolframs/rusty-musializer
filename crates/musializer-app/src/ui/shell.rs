//! The workspace shell: one frame of chrome around the scene preview.
//!
//! Distributed from `../musializer/src/plug.c`, which is 8,682 lines and is the
//! composition root — a source to distribute from, not a file to port. What
//! worked in the C was moving *state* out of the shell into raylib-free modules,
//! not moving drawing code between files, so everything here that could be a
//! decision instead of a pixel already lives in
//! [`musializer_core::ui`] or [`super::shell_layout`].
//!
//! The shell therefore does three things and no more: read input, draw, and
//! return [`ShellCommand`]s. It owns no audio handle, no analyzer and no track
//! list. That is what keeps `main.rs` the only place resource ownership lives.

use std::path::PathBuf;

use musializer_core::scene::{SceneId, SceneSettings};
use musializer_core::ui::notice::{NoticeQueue, NoticeSpec, Severity};
use musializer_core::ui::row_typography;
use musializer_core::ui::scroll_list::{BarHit, ListMetrics, ScrollState};
use musializer_core::ui::timeline_layout::TimelineBand;
use musializer_core::ui::timeline_view::{self, TimelineView};
use musializer_core::ui::workspace_layout::{TracksPanelMode, UiRect};
use musializer_runtime::font::{Face, Faces};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt, Vector2};

use super::shell_layout::{WelcomeFrame, WorkspaceFrame, DEFAULT_TIMELINE_HEIGHT};
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle, Widgets};
use crate::cli::UiPanel;
use crate::scene_host;
use crate::workspace::Workspace;

/// What the shell asks the application to do.
///
/// A command rather than a mutation: the shell can be driven in a test and its
/// decisions inspected, and `main.rs` stays the only owner of the audio device,
/// the analyzer and the window.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellCommand {
    TogglePlay,
    /// Absolute transport position in seconds.
    Seek(f64),
    SelectScene(SceneId),
    /// One scene setting, already clamped by the descriptor.
    SetSetting {
        scene: SceneId,
        index: usize,
        value: f32,
    },
    ResetScene(SceneId),
    /// A file the user dropped on the window.
    LoadTrack(PathBuf),
    /// Make another open track current (`plug.c:5261-5283`).
    SelectTrack(usize),
    /// Ask for an audio file through a native picker (`plug.c:7790-7800`).
    ///
    /// A command rather than the shell opening the dialog itself, because a modal
    /// picker blocks until the user answers and doing that from inside a
    /// begin/end drawing pair would hold the frame open across it.
    OpenAudio,
    /// Ask for a `.musi` project (`plug.c:7802-7805`).
    OpenProject,
    /// Save the current track's project, asking for a destination only when it
    /// has none (`save_project`, `plug.c:4641-4646`).
    SaveProject,
    /// Always ask for a destination (`save_project_as`, `plug.c:4615-4639`).
    SaveProjectAs,
    /// A panel the rewrite has not built yet. Carried as a command rather than
    /// silently ignored, so the notice tray can say so by name — a stub that
    /// says nothing is indistinguishable from a bug.
    NotImplemented(&'static str),
}

/// What the shell needs to know to draw one frame.
///
/// Borrowed, never owned. The lifetime is the contract: the shell may not retain
/// anything it is handed.
pub struct ShellInput<'a> {
    pub window: (f32, f32),
    /// The faces to draw and measure with. Borrowed rather than owned for the same
    /// reason as everything else here, and travelling in the input rather than as a
    /// separate parameter so that no panel can measure a string with one face and
    /// draw it with another.
    pub fonts: &'a Faces,
    pub scene: SceneId,
    pub settings: &'a SceneSettings,
    /// The effective settings after routes, when they differ from `settings`.
    /// Shown as the routed readout so a routed row does not look like a slider
    /// that moved on its own.
    pub routed: Option<&'a SceneSettings>,
    pub time_seconds: f64,
    pub duration_seconds: f64,
    pub playing: bool,
    /// The open tracks and which one is current.
    ///
    /// The whole workspace rather than a name, because every Band 1 panel reads
    /// per-track state — the route editor keys its draft by track slot, the
    /// export panel reads the track's render config, the lyrics editor its
    /// document. Passing a display name would mean six agents each threading
    /// their own second channel to the same object.
    pub workspace: &'a Workspace,
    pub band_count: usize,
    pub peak_band: usize,
    pub rms: f32,
}

/// What the toolbar managed to place, so the timeline knows what is left to it.
///
/// The one field is the band's `timecode_inline` answer. It travels rather than
/// being recomputed, because two places deciding independently where the timecode
/// goes is precisely the bug `timeline_layout.h:12-21` records — the two of them
/// drew into the same strip and printed through each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolbarResult {
    pub timecode_inline: bool,
}

/// Shell state that survives between frames.
pub struct Shell {
    pub widgets: Widgets,
    /// The right-hand tuning inspector.
    pub inspector_open: bool,
    /// Which bottom panel is open. [`UiPanel::None`] is the plain timeline.
    pub panel: UiPanel,
    pub fullscreen: bool,
    pub notices: NoticeQueue,
    pub timeline: TimelineView,
    /// Which of the timeline's own controls the pointer is dragging, so a scrub
    /// that leaves the strip keeps scrubbing.
    scrubbing: bool,
    /// The tracks list's scroll position and momentum. The C keeps this in
    /// function statics, which is why its list code can only ever serve one panel;
    /// per-list state here is what lets the same policy serve the browsers too.
    track_scroll: ScrollState,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    #[must_use]
    pub fn new() -> Self {
        // No startup notice. There used to be a persistent one saying "Drop an
        // audio file on the window to begin", because an empty workspace that says
        // nothing teaches a new user nothing. `draw_welcome` says all of that
        // properly now, so the notice was both redundant and harmful: being
        // persistent it stayed in the tray after a track loaded, and on the welcome
        // screen it covered the format strip along the bottom edge — which a
        // headless capture is what showed.
        Self {
            widgets: Widgets::new(),
            notices: NoticeQueue::default(),
            inspector_open: false,
            panel: UiPanel::None,
            fullscreen: false,
            timeline: TimelineView::new(0.0),
            scrubbing: false,
            track_scroll: ScrollState::new(),
        }
    }

    /// Pushes a notice, dropping the result: the overflow policy in
    /// [`NoticeQueue`] is the right answer and there is nothing better for a
    /// caller to do with a refusal.
    pub fn notify(&mut self, severity: Severity, title: &str, detail: &str) {
        let _ = self.notices.push(&NoticeSpec {
            severity,
            persistent: false,
            duration_seconds: 6.0,
            title: Some(title),
            detail,
            path: "",
        });
    }

    /// The timeline height this frame's open panel asks for.
    ///
    /// A parameter rather than an assumption, per the layout rule: the panel that
    /// draws the rows is the panel that asks for the height. A stub asks for
    /// nothing extra, which is why opening one does not shrink the preview.
    #[must_use]
    pub fn timeline_height(&self, window_height: f32) -> f32 {
        match self.panel {
            UiPanel::None | UiPanel::Tune => DEFAULT_TIMELINE_HEIGHT,
            UiPanel::Export => WorkspaceFrame::export_timeline_height(window_height),
            // Lyrics and Assist are stubs. When they land they ask through
            // `lyrics_editor_layout::panel_height` and `assist_ui_state`'s
            // `timeline_height`, both of which are already ported in
            // musializer-core; until the panels draw rows, asking for their
            // height would steal it from the preview for nothing.
            UiPanel::Lyrics | UiPanel::Assist => DEFAULT_TIMELINE_HEIGHT,
        }
    }

    /// Draws one frame of chrome and returns what the user asked for.
    ///
    /// The preview rectangle is returned alongside so the caller can draw the
    /// scene into it. Chrome is drawn *after* the scene, so the caller's order is
    /// `layout` → draw scene → `draw`.
    #[must_use]
    pub fn layout(&self, input: &ShellInput<'_>) -> WorkspaceFrame {
        if self.fullscreen {
            WorkspaceFrame::fullscreen(input.window.0, input.window.1, true)
        } else {
            WorkspaceFrame::layout(
                input.window.0,
                input.window.1,
                self.inspector_open,
                input.workspace.len(),
                self.timeline_height(input.window.1),
            )
        }
    }

    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
    ) -> Vec<ShellCommand> {
        let mut commands = Vec::new();
        self.widgets.begin_frame();

        self.dropped_files(d, &mut commands);
        self.keyboard(d, input, &mut commands);

        // The toolbar runs first because its band decides whether the timecode
        // fits beside the transport buttons, and the timeline is where it goes
        // when it does not. The regions are disjoint, so drawing it first costs
        // nothing — and one owner of that decision is the whole point.
        let toolbar = self.toolbar(d, frame, input, &mut commands);

        if !self.fullscreen {
            self.tracks_panel(d, frame, input, &mut commands);
            self.scene_browser(d, frame, input, &mut commands);
            self.timeline_strip(d, frame, input, toolbar, &mut commands);
            if self.inspector_open {
                self.inspector(d, frame, input, &mut commands);
            }
        }
        self.notice_tray(d, input.fonts.ui(), frame.preview);

        self.notices.tick(f64::from(d.get_frame_time()));
        commands
    }

    /// The welcome screen, drawn instead of the workspace while no track is open
    /// (`preview_screen`'s `else` branch, `plug.c:7769-7830`).
    ///
    /// A separate screen rather than the workspace with everything disabled, which
    /// is what this rewrite did before. Both are defensible, but the C's answer is
    /// better and it is the oracle: an empty workspace makes a first-time user read
    /// eleven greyed-out controls to discover the one thing they can do, where this
    /// puts that one thing under the cursor and names the three steps that follow.
    ///
    /// Geometry comes from [`WelcomeFrame`] so it is assertable at the window sizes
    /// the application permits; this method is only pixels and clicks.
    pub fn draw_welcome(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
    ) -> Vec<ShellCommand> {
        let mut commands = Vec::new();
        self.widgets.begin_frame();
        self.dropped_files(d, &mut commands);

        let (w, h) = input.window;
        let frame = WelcomeFrame::layout(w, h);
        let font = input.fonts.ui();

        // A light surface, not the scene background: this screen is chrome, and
        // the C clears it to COLOR_UI_SURFACE for that reason (`plug.c:7770`).
        d.draw_rectangle(0, 0, w as i32, h as i32, color::ui_surface());
        d.draw_line(
            32,
            frame.header_rule_y as i32,
            (w - 32.0) as i32,
            frame.header_rule_y as i32,
            color::ui_rule(),
        );
        d.draw_line(
            frame.column_rule_x as i32,
            32,
            frame.column_rule_x as i32,
            (h - 32.0) as i32,
            color::ui_rule(),
        );

        widgets::draw_text_tracked(
            d,
            font,
            "MUSIALIZER",
            frame.masthead.x,
            frame.masthead.y,
            24.0,
            2.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            d,
            font,
            "01",
            frame.step_number.x,
            frame.step_number.y,
            84.0,
            color::accent(),
        );

        widgets::draw_text(
            d,
            font,
            "Turn one track into a",
            frame.body.x,
            frame.body.y,
            38.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            d,
            font,
            "finished visual score.",
            frame.body.x,
            frame.body.y + 46.0,
            38.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            d,
            font,
            "Open an audio file, choose a scene, refine timing, then export a deterministic MP4.",
            frame.body.x,
            frame.body.y + 112.0,
            17.0,
            color::ui_muted(),
        );

        // `Open audio` is drawn selected — accent fill, white label — which is how
        // the C marks the one action the screen exists for (`plug.c:7790`).
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(widgets::id::WELCOME, 0),
                frame.open_audio,
                "Open audio",
                true,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            commands.push(ShellCommand::OpenAudio);
        }
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(widgets::id::WELCOME, 1),
                frame.open_project,
                "Open project",
                false,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            commands.push(ShellCommand::OpenProject);
        }
        widgets::draw_text(
            d,
            font,
            "or drop audio anywhere in this window",
            frame.drop_hint.x,
            frame.drop_hint.y,
            15.0,
            color::ui_muted(),
        );

        // The steps are the first thing to go when the window is too short for
        // everything, because they are the only part of the screen that is
        // explanation rather than affordance.
        if frame.fits(h) {
            d.draw_line_ex(
                Vector2::new(frame.steps_rule.x, frame.steps_rule.y),
                Vector2::new(
                    frame.steps_rule.x + frame.steps_rule.width,
                    frame.steps_rule.y,
                ),
                1.0,
                color::ui_rule(),
            );
            let steps = [
                "Choose or automate scenes",
                "Edit lyrics and timing",
                "Review settings and export",
            ];
            for (index, caption) in steps.iter().enumerate() {
                let column = frame.steps[index];
                widgets::draw_text(
                    d,
                    font,
                    &format!("{}", index + 1),
                    column.x,
                    column.y,
                    28.0,
                    color::accent(),
                );
                widgets::draw_text(
                    d,
                    font,
                    caption,
                    column.x,
                    column.y + 40.0,
                    15.0,
                    color::ui_ink(),
                );
            }
        }

        widgets::draw_text_tracked(
            d,
            font,
            "WAV  OGG  MP3  QOA  XM  MOD  FLAC",
            frame.formats.x,
            frame.formats.y,
            14.0,
            2.0,
            color::ui_muted(),
        );

        // The tray covers the whole window here, not a preview rectangle: there is
        // no preview, and a load failure is exactly the message this screen has to
        // be able to show (`plug.c:7830`).
        self.notice_tray(d, font, UiRect::new(0.0, 0.0, w, h));
        self.notices.tick(f64::from(d.get_frame_time()));
        commands
    }

    /// Files dropped on the window, in either screen.
    ///
    /// The C handles this once for the whole application rather than per screen,
    /// and the welcome screen's own copy printing "or drop audio anywhere in this
    /// window" is a promise that has to hold on both.
    fn dropped_files(&mut self, d: &RaylibDrawHandle<'_>, commands: &mut Vec<ShellCommand>) {
        if !d.is_file_dropped() {
            return;
        }
        for path in d.load_dropped_files().paths() {
            commands.push(ShellCommand::LoadTrack(PathBuf::from(path)));
        }
    }

    fn keyboard(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        use raylib::consts::KeyboardKey as Key;

        // The C's bindings (`ui_theme.h:60-64`), plus Tab for scene cycling,
        // which the C spells with its own scene shortcuts.
        if d.is_key_pressed(Key::KEY_SPACE) {
            commands.push(ShellCommand::TogglePlay);
        }
        if d.is_key_pressed(Key::KEY_F) {
            self.fullscreen = !self.fullscreen;
        }
        if d.is_key_pressed(Key::KEY_TAB) {
            let shift = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
            let step = if shift { SceneId::ALL.len() - 1 } else { 1 };
            let next = (input.scene.index() + step) % SceneId::ALL.len();
            if let Some(id) = SceneId::from_index(next) {
                commands.push(ShellCommand::SelectScene(id));
            }
        }
        if d.is_key_pressed(Key::KEY_T) {
            self.inspector_open = !self.inspector_open;
        }
    }

    /// The transport row (`toolbar`, `plug.c:7366-7420`).
    ///
    /// Placement goes through [`TimelineBand`], the ported band policy, rather
    /// than through arithmetic beside it. That module exists because the control
    /// row and the timecode used to be positioned independently against the same
    /// band, and below roughly 785 px of workspace the timecode printed straight
    /// through the buttons (`timeline_layout.h:12-21`). The toolbar has exactly
    /// that shape, and the 960 px minimum window with the inspector open is
    /// exactly the reachable case that header names — a capture at that size is
    /// what sent this code through the band in the first place.
    ///
    /// The band decides three things: the scale every button shares, whether the
    /// timecode can sit beside them, and whether even the minimum scale overflows.
    /// The last is *reported* rather than hidden, and this caller is what then
    /// drops something.
    fn toolbar(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) -> ToolbarResult {
        let bar = frame.toolbar;
        if bar.is_empty() {
            return ToolbarResult::default();
        }
        d.draw_rectangle_rec(widgets::rectangle(bar), color::ui_surface());
        d.draw_line_ex(
            Vector2::new(bar.x, bar.y),
            Vector2::new(bar.x + bar.width, bar.y),
            1.0,
            color::ui_rule(),
        );

        let has_track = input.workspace.current().is_some();
        let labels: [&str; 6] = [
            if input.playing { "Pause" } else { "Play" },
            "Tune",
            "Export",
            "Lyrics",
            "Assist",
            if self.fullscreen { "Windowed" } else { "Full" },
        ];
        let font = input.fonts.ui();
        let timecode = format!(
            "{} / {}",
            widgets::format_timestamp(input.time_seconds),
            widgets::format_timestamp(input.duration_seconds)
        );
        let timecode_width = widgets::measure(font, &timecode, metric::UI_FONT_VALUE);

        // Each button's *natural* width: what its own label needs at the label
        // size, plus the row padding. The band scales them together, so neighbours
        // stay proportional instead of each shrinking to fit itself — which is the
        // defect `ui_row_typography.h:9-13` describes.
        let mut natural = [0.0f32; 6];
        for (index, label) in labels.iter().enumerate() {
            natural[index] = widgets::measure(font, label, metric::UI_FONT_LABEL)
                + row_typography::UI_ROW_LABEL_PADDING
                + 8.0;
        }

        let row_y = bar.y + (bar.height - metric::UI_BUTTON_HEIGHT) * 0.5;
        let Some(band) = TimelineBand::layout(
            bar.x,
            row_y,
            bar.width,
            metric::UI_BUTTON_HEIGHT,
            metric::UI_CONTROL_GAP,
            &natural,
            // No trailing "Clear manual" button in the transport row, so the
            // band's clear slot is zero-width here.
            0.0,
            timecode_width,
        ) else {
            return ToolbarResult::default();
        };

        // `fits == false` means even TIMELINE_BAND_MIN_SCALE overflows, so
        // something has to go rather than be squeezed into illegibility. The
        // transport button is the one control the row cannot do without, so
        // everything else goes — and the timecode moves to the timeline panel,
        // which is the fallback home the band's `timecode_inline == false` asks
        // for.
        let full_row = band.fits;
        let count = if full_row { labels.len() } else { 1 };

        let mut cursor = bar.x + metric::UI_CONTROL_GAP;
        let scaled: Vec<f32> = natural[..count]
            .iter()
            .map(|width| width * band.scale)
            .collect();
        let label_slice = &labels[..count];
        let font_size =
            widgets::row_font_size(font, label_slice, &scaled, metric::UI_BUTTON_HEIGHT);

        for (index, label) in label_slice.iter().enumerate() {
            let boundary = UiRect::new(cursor, row_y, scaled[index], metric::UI_BUTTON_HEIGHT);
            cursor += scaled[index] + metric::UI_CONTROL_GAP * band.scale;

            let selected = match index {
                1 => self.inspector_open,
                2 => self.panel == UiPanel::Export,
                3 => self.panel == UiPanel::Lyrics,
                4 => self.panel == UiPanel::Assist,
                5 => self.fullscreen,
                _ => false,
            };
            // Play needs a track; the panels do too. Drawn disabled rather than
            // hidden, so the control names the feature even when it cannot run.
            if index < 5 && !has_track {
                self.widgets
                    .disabled_button(d, font, boundary, label, Some(font_size));
                continue;
            }
            let id = widgets::widget_id(widgets::id::TOOLBAR, index as u32);
            let state = self.widgets.text_button(
                d,
                input.fonts.ui(),
                id,
                boundary,
                label,
                selected,
                ButtonStyle::Neutral,
                Some(font_size),
            );
            if !state.clicked {
                continue;
            }
            match index {
                0 => commands.push(ShellCommand::TogglePlay),
                1 => self.inspector_open = !self.inspector_open,
                2 => self.toggle_panel(UiPanel::Export, "Export", commands),
                3 => self.toggle_panel(UiPanel::Lyrics, "Lyrics", commands),
                4 => self.toggle_panel(UiPanel::Assist, "Assist", commands),
                5 => self.fullscreen = !self.fullscreen,
                _ => {}
            }
        }

        // The timecode goes where the band put it, and only if the band said it
        // fits there. Drawing it at `bar.x + bar.width - width` regardless is the
        // exact mistake `timeline_layout.h` was written to stop.
        let timecode_inline = band.timecode_inline && full_row && !band.timecode.is_empty();
        if timecode_inline {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                &timecode,
                band.timecode.x,
                band.timecode.y + (band.timecode.height - metric::UI_FONT_VALUE) * 0.5,
                metric::UI_FONT_VALUE,
                color::ui_ink(),
            );
        }

        // A level meter in whatever is left between the buttons and the timecode.
        // It reads `rms`, not a band: bands are normalized per frame by the frame's
        // own maximum (`audio_analyzer.c:204`), so the loudest band always reads
        // ~1.0 regardless of level and a meter driven from `bands` would be a flat
        // line. This is the readout that makes a stuck analyzer visible in a
        // screenshot.
        let meter_right = if timecode_inline {
            band.timecode.x - metric::UI_CONTROL_GAP
        } else {
            bar.x + bar.width - metric::UI_CONTROL_GAP
        };
        let meter = UiRect::new(
            cursor,
            bar.y + bar.height * 0.5 - 5.0,
            (meter_right - cursor).max(0.0),
            10.0,
        );
        // A 20 px meter is a decoration, not a readout. Below a width it can
        // actually express a level in, it is not drawn.
        if meter.width >= 60.0 && input.band_count > 0 {
            d.draw_rectangle_rec(widgets::rectangle(meter), color::ui_rule());
            let level = input.rms.clamp(0.0, 1.0);
            d.draw_rectangle_rec(
                widgets::rectangle(UiRect::new(
                    meter.x,
                    meter.y,
                    meter.width * level,
                    meter.height,
                )),
                color::ui_success(),
            );
            if meter.width > 190.0 {
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    &format!(
                        "{} bands  peak {}  rms {:.3}",
                        input.band_count, input.peak_band, input.rms
                    ),
                    meter.x,
                    meter.y - 16.0,
                    metric::UI_FONT_CAPTION,
                    color::ui_muted(),
                );
            }
        }

        ToolbarResult { timecode_inline }
    }

    fn toggle_panel(
        &mut self,
        panel: UiPanel,
        name: &'static str,
        commands: &mut Vec<ShellCommand>,
    ) {
        if self.panel == panel {
            self.panel = UiPanel::None;
            return;
        }
        self.panel = panel;
        commands.push(ShellCommand::NotImplemented(name));
    }

    /// The tracks rail (`tracks_panel`, `plug.c`).
    ///
    /// Drawn only when the layout says the panel can host its own action row. The
    /// alternative — drawing it anyway at a fixed offset — is the defect
    /// `workspace_layout.h:7-19` documents: invisible buttons that claim clicks
    /// aimed at the scene tiles painted over them.
    fn tracks_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if frame.tracks_mode == TracksPanelMode::Hidden || frame.tracks.is_empty() {
            return;
        }
        let content = widgets::panel(d, input.fonts.ui(), frame.tracks, "TRACKS");
        let Some((top, height)) = frame.tracks_mode.action_row() else {
            return;
        };
        let row = UiRect::new(
            frame.tracks.x + metric::UI_PANEL_PADDING,
            frame.tracks.y + top,
            frame.tracks.width - metric::UI_PANEL_PADDING * 2.0,
            height,
        );
        // The layout promised this fits; assert it rather than trust it, because
        // this is the exact promise the C broke.
        if !frame.tracks.contains(row) {
            return;
        }

        let stacked = frame.tracks_mode == TracksPanelMode::Stacked;
        // The oracle's four, in its order (`action_labels`, `plug.c:5165-5166`).
        // There is no "Close": the frozen C cannot close a single track, and a
        // button for it would be an invented feature rather than parity.
        let labels: [&str; 4] = ["Open project", "Add audio", "Save", "Save As"];
        let columns = if stacked { 2 } else { 4 };
        let cell_width = (row.width - (columns - 1) as f32 * 4.0) / columns as f32;
        let cell_height = if stacked {
            (row.height - 4.0) * 0.5
        } else {
            row.height
        };
        let widths = [cell_width; 4];
        let font = input.fonts.ui();
        let font_size = widgets::row_font_size(font, &labels, &widths, cell_height);

        for (index, label) in labels.iter().enumerate() {
            let column = index % columns;
            let line = index / columns;
            let boundary = UiRect::new(
                row.x + column as f32 * (cell_width + 4.0),
                row.y + line as f32 * (cell_height + 4.0),
                cell_width,
                cell_height,
            );
            // Every one of these needs Agent B's project model. Disabled and
            // named beats absent: the affordance is what tells the user the
            // feature exists at all.
            // Opening a project and adding audio work with an empty workspace;
            // saving needs something to save.
            let unavailable = index >= 2 && input.workspace.current().is_none();
            if unavailable {
                self.widgets
                    .disabled_button(d, font, boundary, label, Some(font_size));
                continue;
            }
            let id = widgets::widget_id(widgets::id::TRACKS, index as u32);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                boundary,
                label,
                false,
                ButtonStyle::Neutral,
                Some(font_size),
            );
            if state.clicked {
                match index {
                    0 => commands.push(ShellCommand::OpenProject),
                    1 => commands.push(ShellCommand::OpenAudio),
                    2 => commands.push(ShellCommand::SaveProject),
                    _ => commands.push(ShellCommand::SaveProjectAs),
                }
            }
        }

        // The track list, below the action row.
        let list_top = frame.tracks.y + top + height + metric::UI_CONTROL_GAP;
        if !input.workspace.is_empty() {
            self.track_list(d, frame, input, commands, content, list_top);
        } else {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "no track open",
                frame.tracks.x + metric::UI_PANEL_PADDING,
                list_top,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }
    }

    /// The scrolling track list (`plug.c:5213-5382`).
    ///
    /// Split out of [`Self::tracks_panel`] because it is the one part of that
    /// panel with state that outlives a frame. The geometry, the momentum and the
    /// thumb are [`scroll_list`]'s, so all of that is asserted headlessly; what is
    /// here is the drawing and the raylib input.
    ///
    /// Rows are **clipped, not skipped**. A row that is half out of view is drawn
    /// half, and its hit rectangle is intersected with the visible area so the
    /// hidden half cannot claim a click — the failure `workspace_layout.h:7-19`
    /// records, arrived at from the other direction.
    fn track_list(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
        content: UiRect,
        list_top: f32,
    ) {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let count = input.workspace.len();
        // The list area is what is left of the panel's content below the action
        // row, so `header_height` here is everything above the first row.
        let area = UiRect::new(
            frame.tracks.x,
            list_top,
            frame.tracks.width,
            (content.y + content.height - list_top).max(0.0),
        );
        if area.height <= 0.0 {
            return;
        }
        let metrics = ListMetrics::measure(frame.tracks.width, area.height, 0.0, count);

        let mouse = d.get_mouse_position();
        let over_panel = frame.tracks.contains_point(mouse.x, mouse.y);
        if over_panel {
            self.track_scroll.wheel(d.get_mouse_wheel_move(), &metrics);
        }

        // The thumb is measured before `advance` so that a drag reads the same
        // rectangle the user pressed on, and released before the rows are drawn.
        let bar_x = frame.tracks.x + frame.tracks.width - metrics.bar_width;
        if let Some((thumb_y, thumb_height)) = metrics.thumb(self.track_scroll.offset()) {
            let thumb = UiRect::new(bar_x, area.y + thumb_y, metrics.bar_width, thumb_height);
            if self.track_scroll.is_dragging() {
                if d.is_mouse_button_released(MOUSE_BUTTON_LEFT) {
                    self.track_scroll.end_drag();
                }
            } else if thumb.contains_point(mouse.x, mouse.y) {
                if d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) {
                    self.track_scroll.begin_drag(mouse.y - thumb.y);
                }
            } else if mouse.x >= bar_x
                && mouse.x <= bar_x + metrics.bar_width
                && mouse.y >= area.y
                && mouse.y <= area.y + area.height
                && d.is_mouse_button_released(MOUSE_BUTTON_LEFT)
            {
                let hit = if mouse.y < thumb.y {
                    BarHit::Above
                } else {
                    BarHit::Below
                };
                self.track_scroll.page(hit, &metrics);
            }
        }

        self.track_scroll
            .advance(d.get_frame_time(), mouse.y - area.y, &metrics);

        let current = input.workspace.current_index();
        let row_width = metrics.row_width(frame.tracks.width);
        // Scissor mode is GL state, so drawing through the parent handle inside
        // the pair is still clipped; the handle type only enforces the begin/end
        // pairing. Opened once around the whole list rather than per row.
        let mut clip = d.begin_scissor_mode(
            area.x as i32,
            area.y as i32,
            area.width as i32,
            area.height as i32,
        );
        for (index, name) in input.workspace.display_names().enumerate() {
            let (row_y, row_height) = metrics.row_offset(index, self.track_scroll.offset());
            let top = area.y + row_y;
            // Fully outside: no draw, and — the part that matters — no widget id
            // registered, so nothing off-screen can claim the press.
            if top + row_height <= area.y || top >= area.y + area.height {
                continue;
            }
            let boundary =
                UiRect::new(frame.tracks.x + metrics.padding, top, row_width, row_height);
            let selected = current == Some(index);
            // Offset past the action buttons above, so a track row and an action
            // never hash to the same id.
            let id = widgets::widget_id(widgets::id::TRACKS, 16 + index as u32);
            let state = self.widgets.text_button_in(
                &mut clip,
                input.fonts.ui(),
                id,
                boundary,
                // The press is tested against the visible part only.
                boundary.intersect(area),
                name,
                selected,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_LABEL),
            );
            if state.clicked && !selected {
                commands.push(ShellCommand::SelectTrack(index));
            }
        }
        drop(clip);

        if let Some((thumb_y, thumb_height)) = metrics.thumb(self.track_scroll.offset()) {
            widgets::fill(
                d,
                UiRect::new(bar_x, area.y + thumb_y, metrics.bar_width, thumb_height),
                color::ui_rule(),
            );
        }
    }

    /// The scene browser (`scene_browser`, `plug.c`).
    ///
    /// Its content floor is why the sidebar serves it first: it has no collapsed
    /// form (`workspace_layout.h:78-81`).
    fn scene_browser(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if frame.scenes.is_empty() {
            return;
        }
        let content = widgets::panel(d, input.fonts.ui(), frame.scenes, "SCENES");
        let padding = 8.0f32;
        let columns = 2usize;
        let rows = SceneId::ALL.len().div_ceil(columns);
        let available_height = content.height - padding * 2.0 - 24.0;
        if available_height <= 0.0 {
            return;
        }
        let gap = 4.0f32;
        // Tiles clamp to a 24 px floor and a 52 px cap, the numbers
        // WORKSPACE_SCENES_MINIMUM and _MAXIMUM are derived from
        // (`workspace_layout.h:55-62`). Raising one without the other changes
        // nothing, which is why they are written down together there.
        let tile_height =
            ((available_height - gap * (rows - 1) as f32) / rows as f32).clamp(24.0, 52.0);
        let tile_width = (content.width - padding * 2.0 - gap) / columns as f32;

        let labels: Vec<&str> = SceneId::ALL.iter().map(|id| id.display_name()).collect();
        let widths = vec![tile_width; labels.len()];
        let font = input.fonts.ui();
        let font_size = widgets::row_font_size(font, &labels, &widths, tile_height);

        for (index, id) in SceneId::ALL.into_iter().enumerate() {
            let column = index % columns;
            let row = index / columns;
            let boundary = UiRect::new(
                content.x + padding + column as f32 * (tile_width + gap),
                content.y + padding + row as f32 * (tile_height + gap),
                tile_width,
                tile_height,
            );
            // A tile that does not fit inside the panel is not drawn. The panel
            // is what owns those pixels; drawing past it is how the C stole
            // clicks.
            if !content.contains(boundary) {
                continue;
            }
            let widget = widgets::widget_id(widgets::id::SCENE_BROWSER, index as u32);
            let state = self.widgets.text_button(
                d,
                input.fonts.ui(),
                widget,
                boundary,
                id.display_name(),
                id == input.scene,
                ButtonStyle::Neutral,
                Some(font_size),
            );
            // A scene whose drawing half is still a placeholder is marked, so the
            // browser does not promise ten finished scenes.
            if !scene_host::drawing_is_ported(id) {
                d.draw_circle_v(
                    Vector2::new(boundary.x + boundary.width - 8.0, boundary.y + 8.0),
                    3.0,
                    color::ui_warning(),
                );
                // The badge is a drawn dot rather than a glyph on purpose: the
                // footer legend below has to be ASCII because raylib's default
                // font stops at 126, and "*" is the closest honest stand-in for
                // the C's U+00B7 until the imported face lands.
            }
            if state.clicked && id != input.scene {
                commands.push(ShellCommand::SelectScene(id));
            }
        }

        // The footer names what the badge means, because an unexplained dot is
        // worse than no dot.
        let footer_y = content.y + content.height - 22.0;
        if footer_y > content.y {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "* not ported yet",
                content.x + padding,
                footer_y,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }
    }

    /// The timeline strip: waveform lane placeholder, ticks, playhead, scrubber.
    ///
    /// Every seconds↔pixel conversion goes through [`TimelineView`] so the ticks,
    /// the playhead and the scrubber cannot disagree about where a moment is
    /// (`timeline_view.h:6-15`).
    fn timeline_strip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        toolbar: ToolbarResult,
        commands: &mut Vec<ShellCommand>,
    ) {
        let band = frame.timeline;
        if band.is_empty() {
            return;
        }
        let content = widgets::panel(d, input.fonts.ui(), band, "TIMELINE");
        // The timecode's fallback home, when the toolbar's band could not seat it
        // beside the transport buttons (`timeline_layout.h:42-44`). Right-aligned
        // in this panel's header, where nothing else is drawn.
        if !toolbar.timecode_inline {
            let timecode = format!(
                "{} / {}",
                widgets::format_timestamp(input.time_seconds),
                widgets::format_timestamp(input.duration_seconds)
            );
            let font = input.fonts.ui();
            let width = widgets::measure(font, &timecode, metric::UI_FONT_VALUE);
            widgets::draw_text(
                d,
                input.fonts.ui(),
                &timecode,
                band.x + band.width - width - metric::UI_PANEL_PADDING,
                band.y + 6.0,
                metric::UI_FONT_VALUE,
                color::ui_ink(),
            );
        }
        let duration = input.duration_seconds;
        self.timeline.clamp(duration);

        let padding = metric::UI_PANEL_PADDING;
        let strip = UiRect::new(
            content.x + padding,
            content.y + padding,
            (content.width - padding * 2.0).max(0.0),
            56.0f32.min((content.height - padding * 2.0).max(0.0)),
        );
        if strip.is_empty() {
            return;
        }
        d.draw_rectangle_rec(widgets::rectangle(strip), color::ui_raised());
        d.draw_rectangle_lines_ex(widgets::rectangle(strip), 1.0, color::ui_rule());

        if duration <= 0.0 {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "open a track to see its timeline",
                strip.x + 8.0,
                strip.y + strip.height * 0.5 - 7.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            self.open_panel(d, input, content, strip, commands);
            return;
        }

        // Wheel zoom about the pointer, so the moment under the cursor does not
        // slide away (`timeline_view.h:43-47`).
        let mouse = d.get_mouse_position();
        let over_strip = mouse.x >= strip.x
            && mouse.x <= strip.x + strip.width
            && mouse.y >= strip.y
            && mouse.y <= strip.y + strip.height;
        let wheel = d.get_mouse_wheel_move();
        if over_strip && wheel != 0.0 {
            let anchor = self.timeline.seconds_at(
                f64::from(mouse.x),
                f64::from(strip.x),
                f64::from(strip.width),
                duration,
            );
            self.timeline
                .zoom(duration, 1.2f64.powf(f64::from(wheel)), anchor);
        }

        // Ticks from the ladder, chosen from the visible span rather than the
        // track length — picking it from the length left a zoomed-in window with
        // no label in it at all (`timeline_view.h:76-78`).
        let step = timeline_view::tick_step(self.timeline.span_seconds);
        if step > 0.0 {
            let first = (self.timeline.start_seconds / step).floor() * step;
            let mut tick = first;
            while tick <= self.timeline.start_seconds + self.timeline.span_seconds {
                let x = self
                    .timeline
                    .x_at(tick, f64::from(strip.x), f64::from(strip.width))
                    as f32;
                if x >= strip.x && x <= strip.x + strip.width {
                    d.draw_line_ex(
                        Vector2::new(x, strip.y),
                        Vector2::new(x, strip.y + strip.height),
                        1.0,
                        color::ui_rule(),
                    );
                    // Not smaller than UI_FONT_CAPTION: the 11 px labels in an
                    // earlier capture rendered the colon and the point as boxes
                    // in raylib's 10 px bitmap font. A tick label nobody can read
                    // is a tick label that is not there.
                    widgets::draw_text(
                        d,
                        input.fonts.ui(),
                        &widgets::format_timestamp(tick),
                        x + 3.0,
                        strip.y + strip.height - 16.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
                tick += step;
            }
        }

        // Playhead.
        let playhead = self.timeline.x_at(
            input.time_seconds,
            f64::from(strip.x),
            f64::from(strip.width),
        ) as f32;
        if playhead >= strip.x && playhead <= strip.x + strip.width {
            d.draw_line_ex(
                Vector2::new(playhead, strip.y),
                Vector2::new(playhead, strip.y + strip.height),
                2.0,
                color::accent(),
            );
        }
        // Follow playback with the least scroll that keeps the playhead inside,
        // which is safe to call every frame (`timeline_view.h:52-55`).
        if input.playing {
            self.timeline.reveal(duration, input.time_seconds);
        }

        // Scrub. The drag is tracked here rather than through the button claim
        // because a scrub that leaves the strip must keep scrubbing.
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;
        if over_strip && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) {
            self.scrubbing = true;
        }
        if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
            self.scrubbing = false;
        }
        if self.scrubbing {
            let seconds = self.timeline.seconds_at(
                f64::from(mouse.x),
                f64::from(strip.x),
                f64::from(strip.width),
                duration,
            );
            commands.push(ShellCommand::Seek(seconds));
        }

        // The zoom readout, so "why is the strip not the whole track" has an
        // answer on screen.
        let zoom_label = if self.timeline.is_whole(duration) {
            "whole track".to_string()
        } else {
            format!(
                "{:.1}x  ({} - {})",
                duration / self.timeline.span_seconds,
                widgets::format_timestamp(self.timeline.start_seconds),
                widgets::format_timestamp(self.timeline.start_seconds + self.timeline.span_seconds)
            )
        };
        widgets::draw_text(
            d,
            input.fonts.ui(),
            &zoom_label,
            strip.x,
            strip.y + strip.height + 4.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        let reset = UiRect::new(
            strip.x + strip.width - 84.0,
            strip.y + strip.height + 2.0,
            84.0,
            22.0,
        );
        if content.contains(reset) {
            let id = widgets::widget_id(widgets::id::TIMELINE, 1);
            if self
                .widgets
                .text_button(
                    d,
                    input.fonts.ui(),
                    id,
                    reset,
                    "Zoom out",
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                self.timeline.reset(duration);
            }
        }

        self.open_panel(d, input, content, strip, commands);
    }

    /// Dispatches to whichever bottom panel is open.
    ///
    /// One `match` in one place, so an agent fills a function in their own file
    /// and never edits this one. [`UiPanel::Tune`] is absent because the tuning
    /// controls are the right-hand inspector, not a bottom panel.
    fn open_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        match self.panel {
            UiPanel::None | UiPanel::Tune => {}
            UiPanel::Export => self.export_panel(d, input, content, strip, commands),
            UiPanel::Lyrics => self.lyrics_panel(d, input, content, strip, commands),
            UiPanel::Assist => self.assist_panel(d, input, content, strip, commands),
        }
    }

    /// The notice tray, over the preview's bottom-left corner
    /// (`notice_tray`, `plug.c`).
    fn notice_tray(&mut self, d: &mut RaylibDrawHandle<'_>, font: &Face, preview: UiRect) {
        if preview.is_empty() || self.notices.is_empty() {
            return;
        }
        let width = 380.0f32.min(preview.width - 24.0);
        if width <= 0.0 {
            return;
        }
        let row_height = 56.0f32;
        let mut y = preview.y + preview.height - 12.0 - row_height;
        // Newest last in the queue, so draw from the end upward: the most recent
        // notice sits closest to the bottom edge where the eye already is.
        for notice in self.notices.notices().iter().rev() {
            if y < preview.y {
                break;
            }
            let boundary = UiRect::new(preview.x + 12.0, y, width, row_height);
            let accent = match notice.severity {
                Severity::Info => color::accent(),
                Severity::Success => color::ui_success(),
                Severity::Warning => color::ui_warning(),
                Severity::Error => color::ui_danger(),
            };
            d.draw_rectangle_rec(widgets::rectangle(boundary), Color::new(20, 22, 28, 232));
            d.draw_rectangle_rec(
                widgets::rectangle(UiRect::new(boundary.x, boundary.y, 3.0, boundary.height)),
                accent,
            );
            widgets::draw_text(
                d,
                font,
                notice.severity.label(),
                boundary.x + 10.0,
                boundary.y + 4.0,
                metric::UI_FONT_CAPTION,
                accent,
            );
            widgets::draw_text(
                d,
                font,
                &notice.title,
                boundary.x + 10.0,
                boundary.y + 19.0,
                metric::UI_FONT_LABEL,
                Color::RAYWHITE,
            );
            if !notice.detail.is_empty() {
                widgets::draw_text(
                    d,
                    font,
                    &notice.detail,
                    boundary.x + 10.0,
                    boundary.y + 37.0,
                    metric::UI_FONT_CAPTION,
                    Color::new(180, 190, 205, 255),
                );
            }
            y -= row_height + 4.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stub_panel_does_not_reserve_height_it_never_draws() {
        // The rule, as a test. Opening Lyrics or Assist while their panels are
        // stubs must not shrink the preview, because the height would buy
        // nothing.
        let mut shell = Shell::new();
        let baseline = shell.timeline_height(720.0);
        for panel in [UiPanel::Lyrics, UiPanel::Assist, UiPanel::Tune] {
            shell.panel = panel;
            assert_eq!(
                shell.timeline_height(720.0),
                baseline,
                "{panel:?} reserved height for a panel it does not draw"
            );
        }
        // Export does draw a taller region, and asks for it.
        shell.panel = UiPanel::Export;
        assert!(shell.timeline_height(1080.0) > baseline);
    }

    #[test]
    fn toggling_a_panel_twice_returns_to_the_timeline() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();
        shell.toggle_panel(UiPanel::Export, "Export", &mut commands);
        assert_eq!(shell.panel, UiPanel::Export);
        assert_eq!(commands, vec![ShellCommand::NotImplemented("Export")]);
        shell.toggle_panel(UiPanel::Export, "Export", &mut commands);
        assert_eq!(shell.panel, UiPanel::None);
        // Closing does not re-announce.
        assert_eq!(commands.len(), 1);
    }

    /// The toolbar's own band, computed the way [`Shell::toolbar`] computes it but
    /// with a stubbed text measurer, so the policy is assertable without a window.
    ///
    /// The measurer is the default font's rough average advance at the label size
    /// (raylib's default face is a fixed 10x10 cell, so ~0.5 em per character is
    /// close). Exactness is not the point — the point is that the *policy* is
    /// exercised at the sizes a capture showed to be tight.
    fn toolbar_band(bar_width: f32, playing: bool, fullscreen: bool) -> TimelineBand {
        let labels = [
            if playing { "Pause" } else { "Play" },
            "Tune",
            "Export",
            "Lyrics",
            "Assist",
            if fullscreen { "Windowed" } else { "Full" },
        ];
        let measure = |text: &str, size: f32| text.chars().count() as f32 * size * 0.5;
        let mut natural = [0.0f32; 6];
        for (index, label) in labels.iter().enumerate() {
            natural[index] =
                measure(label, metric::UI_FONT_LABEL) + row_typography::UI_ROW_LABEL_PADDING + 8.0;
        }
        let timecode_width = measure("00:00.000 / 00:00.000", metric::UI_FONT_VALUE);
        TimelineBand::layout(
            0.0,
            0.0,
            bar_width,
            metric::UI_BUTTON_HEIGHT,
            metric::UI_CONTROL_GAP,
            &natural,
            0.0,
            timecode_width,
        )
        .expect("the band accepts these inputs")
    }

    #[test]
    fn the_toolbar_never_squeezes_its_labels_below_the_legibility_floor() {
        // The band's contract: it will shrink to TIMELINE_BAND_MIN_SCALE and no
        // further, and it says so through `fits`. A capture at 960x640 with the
        // inspector open — a 440 px toolbar — is what caught the old arithmetic
        // reading "Pau?  Tune  Exp?  Lyr?".
        use musializer_core::ui::timeline_layout::TIMELINE_BAND_MIN_SCALE;

        for width in [440.0f32, 640.0, 960.0, 1280.0] {
            let band = toolbar_band(width, false, false);
            assert!(
                band.scale >= TIMELINE_BAND_MIN_SCALE,
                "{width}px scaled to {} — below the legibility floor",
                band.scale
            );
            assert!(band.scale <= 1.0, "{width}px scaled up to {}", band.scale);
        }
    }

    #[test]
    fn a_narrow_toolbar_moves_the_timecode_out_rather_than_over_the_buttons() {
        // The whole reason the band exists. At the narrow end the timecode must
        // not be inline, and the timeline panel is then responsible for it —
        // which is why `ToolbarResult` travels.
        let narrow = toolbar_band(440.0, false, false);
        let wide = toolbar_band(1280.0, false, false);
        assert!(
            !narrow.timecode_inline || !narrow.fits,
            "a 440 px band claimed room for both the row and the timecode"
        );
        assert!(wide.timecode_inline, "a 1280 px band should seat both");
        assert!(wide.fits);
        // Inline or not, the timecode rect never overlaps the control row.
        if wide.timecode_inline {
            assert!(!wide.controls.overlaps(wide.timecode));
        }
    }

    #[test]
    fn the_toolbar_row_stays_inside_the_bar_at_every_supported_width() {
        // A sweep, because the interesting failures are at the two boundaries: the
        // one where the band stops seating the timecode inline, and the one where
        // it stops fitting at all.
        //
        // Note what `fits == false` does *not* mean: it does not mean the band
        // returns something that fits. It means the band has already scaled to its
        // floor and the row still overflows, so **the caller has to drop
        // controls** (`timeline_layout.h:45-47`). This test found that the first
        // time round by asserting the wrong thing, which is worth recording: at
        // 300 px the band honestly reports a 322 px row. `Shell::toolbar` responds
        // by drawing the transport button alone, and that is the invariant below.
        for width in 300..=1920 {
            let bar = width as f32;
            let band = toolbar_band(bar, true, true);
            if band.fits {
                assert!(
                    band.controls_width <= bar + 0.01,
                    "{width}px: the band said a {}px row fits, and it does not",
                    band.controls_width
                );
            } else {
                // What the shell actually draws in this case: one scaled button.
                let transport = toolbar_band(bar, true, true).scale
                    * (metric::UI_FONT_LABEL * 0.5 * 5.0
                        + row_typography::UI_ROW_LABEL_PADDING
                        + 8.0);
                assert!(
                    transport + metric::UI_CONTROL_GAP * 2.0 <= bar,
                    "{width}px: even the lone transport button does not fit"
                );
            }
            if band.timecode_inline && band.fits {
                assert!(
                    band.timecode.x + band.timecode.width <= bar + 0.01,
                    "{width}px: the timecode runs past the bar"
                );
                assert!(
                    !band.controls.overlaps(band.timecode),
                    "{width}px: the timecode prints through the controls"
                );
            }
        }
    }

    #[test]
    fn the_tray_starts_empty_so_the_welcome_screen_is_not_covered() {
        // The tray draws over the bottom-left of whatever screen is up, which is
        // where the welcome screen puts its supported-format strip. A persistent
        // startup notice therefore hid it — and stayed in the tray after a track
        // loaded, because persistent notices do not expire.
        let shell = Shell::new();
        assert!(shell.notices.is_empty());
    }
}
