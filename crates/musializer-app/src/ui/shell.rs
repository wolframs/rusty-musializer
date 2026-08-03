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

use musializer_core::project::event_timeline::ManualEventAction;
use musializer_core::project::preset_store::{PresetAction, SharedPresetsView};
use musializer_core::scene::routes::{ParameterMapping, RouteSources};
use musializer_core::scene::{SceneId, SceneSettings};
use musializer_core::ui::notice::{NoticeQueue, NoticeSpec, Severity};
use musializer_core::ui::row_typography;
use musializer_core::ui::scroll_list::{BarHit, ListMetrics, ScrollState};
use musializer_core::ui::timeline_layout::TimelineBand;
use musializer_core::ui::timeline_view::{self, TimelineView};
use musializer_core::ui::transport_bar;
use musializer_core::ui::workspace_layout::{TracksPanelMode, UiRect};
use musializer_runtime::font::{Faces, UiFonts};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::icons;
use super::preferences::UiPreferences;
use super::scale::{UiScale, UiScalePreference};
use super::shell_layout::{LayoutOverrides, WelcomeFrame, WorkspaceFrame, DEFAULT_TIMELINE_HEIGHT};
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle, Widgets};
use crate::cli::UiPanel;
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
    /// Enable or disable playback of the current track's retained automatic
    /// scene plan. The composition root owns restoring the base scene when the
    /// plan is disabled; the Assist panel only emits this durable intent.
    SetAutoScenes(bool),
    /// A file the user dropped on the window.
    LoadTrack(PathBuf),
    /// Commit the route editor's draft onto the current track
    /// (`plug.c:5852`). Adding and replacing are the same command: the table
    /// keys by parameter, so a second route for one parameter is a replacement
    /// rather than a duplicate.
    ApplyRoute {
        scene: SceneId,
        route: ParameterMapping,
    },
    /// Drop the committed route for one parameter (`plug.c:5862`).
    RemoveRoute {
        scene: SceneId,
        parameter: String,
    },
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
    /// The export panel's preset rows write through the current track
    /// (`plug.c:2569-2572`).
    SetRenderConfig(musializer_core::timing::render_export::RenderExportConfig),
    /// Ask for a destination and start an export (`plug.c:7120-7138`).
    ///
    /// There is deliberately no matching Cancel: cancellation is read on the
    /// progress screen, which the session draws and answers in the same tick, so
    /// it never has to travel through `main.rs`.
    StartRender,
    /// The manual event row's outcome (`plug.c:2834-2971`).
    ManualEvent(ManualEventAction),
    /// One durable edit from the always-visible scene-plan lane.
    ScenePlan(super::panels::scene_timeline::ScenePlanEdit),
    /// The shared preset block's outcome (`plug.c:5979-6100`).
    Preset(PresetAction),
    /// Output volume in `[0, 1]`, from the transport row's slider.
    ///
    /// Not in the oracle, which has only `--mute` at startup
    /// (`musializer.c:399-405`). A command rather than shell state because the
    /// volume lives on raylib's audio device, and the shell may not hold a handle
    /// to it — the same reason `OpenAudio` is a command.
    SetVolume(f32),
    /// Flip the output between muted and the stored volume.
    ///
    /// Separate from `SetVolume(0.0)` so that unmuting can restore the level the
    /// user had set rather than guessing one.
    ToggleMute,
    /// Take the *window* in or out of fullscreen.
    ///
    /// The shell has already switched its own layout by the time this is emitted;
    /// this is only the part that needs `&mut RaylibHandle`, which does not exist
    /// inside a drawing pair. Split that way so the headless probe can take the
    /// expanded layout without making a window call that Xvfb cannot serve.
    SetFullscreen(bool),
    /// Persist workstation UI state outside the current `.musi` project.
    SaveUiPreferences(UiPreferences),
}

/// What the shell needs to know to draw one frame.
///
/// Borrowed, never owned. The lifetime is the contract: the shell may not retain
/// anything it is handed.
pub struct ShellInput<'a> {
    pub window: (f32, f32),
    pub ui_scale: UiScale,
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
    /// Every analysis source at its current value, so a panel reads the same
    /// numbers the routes were evaluated from rather than a subset. `rms` above
    /// predates this and stays because the toolbar's readout is the only caller
    /// that wants one scalar without knowing what a source is.
    pub route_sources: RouteSources<'a>,
    /// The shared per-user preset library, its selection, and whether the store
    /// file is writable (`p->shared_presets`, `plug.c:265-270`).
    pub presets: SharedPresetsView<'a>,
    pub band_count: usize,
    pub peak_band: usize,
    pub rms: f32,
    /// The stored output volume in `[0, 1]`, which is what the slider shows even
    /// while muted — mute is a toggle the user expects to undo, and a slider that
    /// zeroed itself would lose the level they set.
    pub volume: f32,
    pub muted: bool,
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
    /// Whether the diagnostic readout is drawn over the preview.
    ///
    /// **Off by default**, unlike every earlier build of this rewrite. The line is
    /// a developer HUD — frame counter, band index, consumed sample count — and
    /// leaving it over a music visualiser by default is the interface explaining
    /// itself to the wrong audience. The probe harness turns it back on, because a
    /// capture that carries its own evidence is the reason it was written.
    pub hud_visible: bool,
    pub notices: NoticeQueue,
    pub timeline: TimelineView,
    /// One owner for every timeline drag. A scene boundary and the waveform
    /// scrubber must never both interpret the same press.
    pub(crate) timeline_gesture: Option<TimelineGesture>,
    /// The most recent drag position. Seeking is deliberately deferred until
    /// release: repeatedly flushing and refilling a decoder while the pointer
    /// moves is both audible and needlessly expensive.
    scrub_target_seconds: Option<f64>,
    /// Whether playback should resume after the release-time seek.
    scrub_restore_playing: bool,
    /// The lyrics editor's draft, selection, panes and pending edits.
    ///
    /// Agent I had this in a `thread_local` while this field did not exist and
    /// documented its own removal; it is gone. The same thing happened with the
    /// route editor, which is the shape of the fan-out working: an agent that
    /// cannot touch a shared file names what it needs instead of quietly
    /// reaching for a global and leaving it there.
    pub lyrics: super::panels::lyrics::LyricEditor,
    /// Selection and in-flight boundary preview for the scene-plan lane.
    pub scene_lane: super::panels::scene_timeline::SceneLaneEditor,
    /// The font browser's catalogue, query, selection and consent, plus the
    /// importer it drives.
    ///
    /// On `Shell` because it outlives a frame and because the pane is drawn from
    /// inside another agent's panel — threading it through that call would make
    /// the lyrics editor carry state it has no business knowing about.
    pub font_browser: super::panels::fonts::FontBrowser,
    /// The route editor's draft, and the track slot it is keyed against.
    ///
    /// Lives here rather than in `panels::tune` because a draft outlives a frame
    /// and `Shell` is where per-frame-surviving state belongs. Agent G had it in
    /// a `thread_local` while this field did not exist and flagged it as
    /// something that should not survive the merge; it did not.
    pub route_editor: super::panels::tune::EditorHost,
    /// The tracks list's scroll position and momentum. The C keeps this in
    /// function statics, which is why its list code can only ever serve one panel;
    /// per-list state here is what lets the same policy serve the browsers too.
    track_scroll: ScrollState,
    pub ui_preferences: UiPreferences,
    ui_scale_override: Option<UiScalePreference>,
    split_drag: Option<SplitKind>,
    last_split_press: Option<(SplitKind, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitKind {
    Sidebar,
    Inspector,
    Timeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimelineGesture {
    Scrub,
    SceneBoundary,
}

/// Every surface in this interface that can take a keystroke as *text*.
///
/// One list, in one place, because the shell reads the keyboard before any panel
/// is drawn: a panel with a focused field has already lost the keypress by the
/// time it runs, so the guard has to be asked here and it has to know about all
/// of them. Scattering the flags over `||`s in the caller is what produced
/// UX0-A06 (review 1.6) — the predicate knew about the lyrics cue field and not
/// about the font browser's filter, and typing "Space Mono" into the filter
/// toggled playback, fullscreen, mute, the readout and the inspector, cycled the
/// scene and seeked the track.
///
/// **The rule for adding one: a new text surface adds a variant here, and the
/// compiler then requires an arm in [`Shell::text_entry_focused`] and in the
/// test that sweeps `ALL`.** Nothing else in the shell asks about focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextEntrySurface {
    /// The lyrics editor's cue field.
    LyricCue,
    /// The font browser's family filter.
    FontQuery,
}

impl TextEntrySurface {
    pub(crate) const ALL: [Self; 2] = [Self::LyricCue, Self::FontQuery];
}

/// The keys one frame of the shell can act on, read out of raylib in one place.
///
/// Separated from [`Shell::keyboard_actions`] because a `RaylibDrawHandle` only
/// exists inside a live window, and the shortcut-suppression rule this carries is
/// exactly the kind of policy a capture cannot photograph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyboardFrame {
    pub control: bool,
    pub shift: bool,
    pub scale_up: bool,
    pub scale_down: bool,
    pub scale_auto: bool,
    pub toggle_play: bool,
    pub toggle_fullscreen: bool,
    pub escape: bool,
    pub toggle_mute: bool,
    pub toggle_hud: bool,
    pub seek_start: bool,
    pub seek_end: bool,
    /// Left and right arrows, which repeat: holding one to scan through a track
    /// is the reason a 0.1 s step is useful at all.
    pub nudge_back: bool,
    pub nudge_forward: bool,
    pub cycle_scene: bool,
    pub toggle_inspector: bool,
}

impl KeyboardFrame {
    fn read(d: &RaylibDrawHandle<'_>) -> Self {
        use raylib::consts::KeyboardKey as Key;

        let held = |key| d.is_key_pressed(key) || d.is_key_pressed_repeat(key);
        Self {
            control: d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL),
            shift: d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT),
            scale_up: d.is_key_pressed(Key::KEY_EQUAL) || d.is_key_pressed(Key::KEY_KP_ADD),
            scale_down: d.is_key_pressed(Key::KEY_MINUS) || d.is_key_pressed(Key::KEY_KP_SUBTRACT),
            scale_auto: d.is_key_pressed(Key::KEY_ZERO) || d.is_key_pressed(Key::KEY_KP_0),
            toggle_play: d.is_key_pressed(Key::KEY_SPACE),
            toggle_fullscreen: d.is_key_pressed(Key::KEY_F),
            escape: d.is_key_pressed(Key::KEY_ESCAPE),
            toggle_mute: d.is_key_pressed(Key::KEY_M),
            toggle_hud: d.is_key_pressed(Key::KEY_H),
            seek_start: d.is_key_pressed(Key::KEY_HOME),
            seek_end: d.is_key_pressed(Key::KEY_END),
            nudge_back: held(Key::KEY_LEFT),
            nudge_forward: held(Key::KEY_RIGHT),
            cycle_scene: d.is_key_pressed(Key::KEY_TAB),
            toggle_inspector: d.is_key_pressed(Key::KEY_T),
        }
    }
}

/// The frame facts the keyboard path reads out of [`ShellInput`].
///
/// Four scalars rather than the input itself, because `ShellInput` borrows the
/// font bank, the workspace and the preset store and so cannot be built without
/// a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeyboardContext {
    pub ui_scale: UiScale,
    pub time_seconds: f64,
    pub duration_seconds: f64,
    pub scene_index: usize,
}

impl KeyboardContext {
    fn of(input: &ShellInput<'_>) -> Self {
        Self {
            ui_scale: input.ui_scale,
            time_seconds: input.time_seconds,
            duration_seconds: input.duration_seconds,
            scene_index: input.scene.index(),
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    #[must_use]
    pub fn new() -> Self {
        Self::with_preferences(UiPreferences::default())
    }

    #[must_use]
    pub fn with_preferences(ui_preferences: UiPreferences) -> Self {
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
            hud_visible: false,
            timeline: TimelineView::new(0.0),
            timeline_gesture: None,
            scrub_target_seconds: None,
            scrub_restore_playing: false,
            track_scroll: ScrollState::new(),
            route_editor: super::panels::tune::EditorHost::default(),
            font_browser: super::panels::fonts::FontBrowser::new(),
            lyrics: super::panels::lyrics::LyricEditor::new(),
            scene_lane: super::panels::scene_timeline::SceneLaneEditor::default(),
            ui_preferences,
            ui_scale_override: None,
            split_drag: None,
            last_split_press: None,
        }
    }

    pub fn set_ui_scale_override(&mut self, preference: Option<UiScalePreference>) {
        self.ui_scale_override = preference;
    }

    #[must_use]
    pub fn ui_scale_preference(&self) -> UiScalePreference {
        self.ui_scale_override.unwrap_or(self.ui_preferences.scale)
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
    pub fn timeline_height(&self, window: (f32, f32), workspace: &Workspace) -> f32 {
        let window_height = window.1;
        // The manual event row is reserved before it is measured, which is why
        // its height is a constant as well as a return value.
        //
        // It is **not** added when the lyrics editor is open, and that divergence
        // is arithmetic rather than taste. In the oracle those buttons share the
        // band's controls row with Lyrics/Assist/Export (`plug.c:2867-2870`, six
        // controls in one row); this rewrite seats those three in the toolbar
        // instead, so a separate event row is chrome the oracle never spends.
        // `LYRIC_EDITOR_TIMELINE_CHROME` is budgeted against the oracle's band,
        // and at 720p there is no room for both without pushing the sidebar under
        // its floor — which drops the tracks panel entirely, as a capture showed.
        // The event lane keeps its own affordance in the no-panel band.
        let events = super::panels::events::EVENT_ROW_HEIGHT;
        let panel_height = match self.panel {
            UiPanel::None | UiPanel::Tune => events + DEFAULT_TIMELINE_HEIGHT,
            UiPanel::Export => events + WorkspaceFrame::export_timeline_height(window_height),
            // Both full-band editors take the band to themselves, for the same
            // arithmetic reason: their chrome budgets are the oracle's, and the
            // oracle spends that budget on a controls row this rewrite has
            // already moved into the toolbar.
            UiPanel::Lyrics => self.lyrics.timeline_height(window_height, 0.0),
            UiPanel::Assist => self.assist_timeline_height(window, &workspace.assist),
        };
        panel_height + super::panels::scene_timeline::SCENE_SECTION_HEIGHT
    }

    fn resolved_timeline_height(&self, window: (f32, f32), workspace: &Workspace) -> f32 {
        let automatic = self.timeline_height(window, workspace);
        let Some(requested) = self.ui_preferences.timeline_height else {
            return automatic;
        };
        let minimum = super::panels::scene_timeline::SCENE_SECTION_HEIGHT
            + match self.panel {
                UiPanel::None | UiPanel::Tune => super::panels::events::EVENT_ROW_HEIGHT + 150.0,
                UiPanel::Export => 260.0,
                // 121 chrome + 10 bottom + 22 lane + 5 gap + the form's 223 px.
                UiPanel::Lyrics => 381.0,
                // Assist has no scrolling body, so its measured content remains the
                // floor; resizing can give it room, never clip an action.
                UiPanel::Assist => automatic - super::panels::scene_timeline::SCENE_SECTION_HEIGHT,
            };
        let maximum = (window.1 - metric::HUD_BUTTON_SIZE - 150.0).max(minimum);
        requested.clamp(minimum, maximum)
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
            let timeline = self.resolved_timeline_height(input.window, input.workspace);
            let overrides = LayoutOverrides {
                inspector_width: self.ui_preferences.inspector_width,
                tracks_width: self.ui_preferences.sidebar_width,
            };
            if overrides == LayoutOverrides::default() {
                WorkspaceFrame::layout(
                    input.window.0,
                    input.window.1,
                    self.inspector_open,
                    input.workspace.len(),
                    timeline,
                )
            } else {
                WorkspaceFrame::layout_with_overrides(
                    input.window.0,
                    input.window.1,
                    self.inspector_open,
                    input.workspace.len(),
                    timeline,
                    overrides,
                )
            }
        }
    }

    /// Everything that has to happen before a frame's first widget or keypress.
    ///
    /// One call rather than each subsystem being poked from each screen: both
    /// [`Shell::draw`] and [`Shell::draw_welcome`] need it, and per-frame state
    /// that only one of them resets is how a stale flag survives (UX0-A02,
    /// UX0-A06).
    fn begin_frame(&mut self, ui_scale: UiScale) {
        self.widgets.begin_frame(ui_scale);
        self.font_browser.begin_frame();
    }

    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
    ) -> Vec<ShellCommand> {
        let mut commands = Vec::new();
        self.begin_frame(input.ui_scale);
        if self.fullscreen {
            d.set_mouse_cursor(raylib::consts::MouseCursor::MOUSE_CURSOR_DEFAULT);
        }

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
            self.splitters(d, frame, input, &mut commands);
        }
        self.notice_tray(d, input.fonts.ui(), frame.preview);

        // Last, and deliberately so: a tooltip belongs above everything, and the
        // toolbar that owns most of them is the *first* thing drawn. Requested
        // where the control is and drawn here is the only ordering that works.
        if let Some(tooltip) = self.widgets.tooltip().cloned() {
            widgets::draw_tooltip(d, input.fonts.ui(), &tooltip, input.window);
        }

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
        self.begin_frame(input.ui_scale);
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
        self.keyboard_actions(KeyboardFrame::read(d), KeyboardContext::of(input), commands);
    }

    /// [`Shell::keyboard`] without raylib.
    ///
    /// Split from the reading half so the one rule that decides whether a global
    /// shortcut fires at all — [`Shell::text_entry_has_focus`] — is assertable.
    /// It has to be: a shortcut that fires while the user is typing (UX0-A06,
    /// review 1.6) leaves no trace in a capture, and the defect it caused was
    /// that typing "Space Mono" into the font filter toggled playback, mute,
    /// fullscreen, the readout and the inspector and seeked the track.
    fn keyboard_actions(
        &mut self,
        keys: KeyboardFrame,
        context: KeyboardContext,
        commands: &mut Vec<ShellCommand>,
    ) {
        if keys.control {
            let plus = keys.scale_up;
            let minus = keys.scale_down;
            if keys.scale_auto {
                self.ui_scale_override = None;
                self.ui_preferences.scale = UiScalePreference::Auto;
                self.notify(
                    Severity::Success,
                    "UI scale: Auto",
                    "The desktop scale and window size now choose the shell scale.",
                );
                commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
            } else if plus != minus {
                let scale = if plus {
                    context.ui_scale.next()
                } else {
                    context.ui_scale.previous()
                };
                self.ui_scale_override = None;
                self.ui_preferences.scale = UiScalePreference::Fixed(scale);
                self.notify(
                    Severity::Success,
                    &format!("UI scale: {}%", scale.percent()),
                    "The scene and exported video resolution are unchanged.",
                );
                commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
            }
        }

        // A text field takes every key, including Space and the arrows. Without
        // this, typing a lyric line would scrub the track and toggle playback
        // under the cursor.
        if self.text_entry_has_focus() {
            return;
        }

        // The C's bindings (`ui_theme.h:60-64`), plus Tab for scene cycling,
        // which the C spells with its own scene shortcuts.
        if keys.toggle_play {
            commands.push(ShellCommand::TogglePlay);
        }
        if keys.toggle_fullscreen {
            self.set_fullscreen(!self.fullscreen, commands);
        }
        // Escape leaves fullscreen but does not enter it — the convention every
        // media player follows, and the one that makes Escape safe to press.
        if keys.escape && self.fullscreen {
            self.set_fullscreen(false, commands);
        }
        if keys.toggle_mute {
            commands.push(ShellCommand::ToggleMute);
        }
        if keys.toggle_hud {
            self.hud_visible = !self.hud_visible;
        }

        // Fine positioning. These are the bindings the seek buttons' tooltips
        // name, evaluated through the same `transport_bar` helpers the buttons
        // use, so a click and a keypress cannot disagree about what Ctrl means.
        if context.duration_seconds > 0.0 {
            if keys.seek_start {
                commands.push(ShellCommand::Seek(0.0));
            }
            if keys.seek_end {
                commands.push(ShellCommand::Seek(context.duration_seconds));
            }
            let back = keys.nudge_back;
            let forward = keys.nudge_forward;
            if back != forward {
                let sign = if back { -1.0 } else { 1.0 };
                let step = transport_bar::seek_step_seconds(keys.control, keys.shift) * sign;
                commands.push(ShellCommand::Seek(transport_bar::nudged(
                    context.time_seconds,
                    step,
                    context.duration_seconds,
                )));
            }
        }
        if keys.cycle_scene {
            let step = if keys.shift {
                SceneId::ALL.len() - 1
            } else {
                1
            };
            let next = (context.scene_index + step) % SceneId::ALL.len();
            if let Some(id) = SceneId::from_index(next) {
                commands.push(ShellCommand::SelectScene(id));
            }
        }
        if keys.toggle_inspector {
            self.set_inspector_open(!self.inspector_open);
        }
    }

    /// The transport row (`toolbar`, `plug.c:7366-7420`), extended.
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
    ///
    /// # What is not the oracle's
    ///
    /// The C's row is six text buttons and a timecode. This one adds a fine-seek
    /// group and a right-hand utility cluster — volume, the diagnostic readout
    /// toggle, fullscreen — and draws every control as an icon. All of that is
    /// invention, and it is only affordable because icons are square: eleven
    /// controls occupy less of the row than the oracle's six labels did.
    ///
    /// The cost of icons is discoverability, paid for in two places rather than
    /// waved at. Every control carries a tooltip naming it and its shortcut
    /// ([`super::icons`]), and every control has a text fallback for the build
    /// where the icon atlas did not load. The band still owns the middle; the
    /// cluster arithmetic is [`transport_bar`], which is raylib-free so that the
    /// widths where controls are shed can be swept in a test instead of found by
    /// resizing a window.
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
        let font = input.fonts.ui();
        let control_size = transport_bar::CONTROL_SIZE;
        let row_y = bar.y + (bar.height - control_size) * 0.5;

        // The right-hand cluster is placed first because it is measured from the
        // window's right edge, and everything else is laid out against what it
        // leaves. Volume is only offered when there is a stream to set it on.
        let utilities = transport_bar::utilities(bar, row_y, has_track);
        let middle_right = utilities.map_or(bar.x + bar.width, |cluster| cluster.left_edge);
        let middle_width = (middle_right - bar.x).max(0.0);

        let timecode = format!(
            "{} / {}",
            widgets::format_timestamp(input.time_seconds),
            widgets::format_timestamp(input.duration_seconds)
        );
        let timecode_width = widgets::measure(font, &timecode, metric::UI_FONT_VALUE);

        // The middle group, richest first. The seek trio is shed before the panel
        // buttons and the panel buttons before the transport button, which is the
        // order `transport_bar` documents: every seek action has a keyboard
        // binding, so dropping the group costs no capability, and the transport
        // button is the one control the row cannot do without.
        let transport = if input.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        };
        let panels = [icons::TUNE, icons::EXPORT, icons::LYRICS, icons::ASSIST];
        let seek = [icons::SEEK_START, icons::SEEK_BACK, icons::SEEK_FORWARD];

        // Natural widths. With the icon face loaded every control is a square, so
        // the row is uniform; on the text fallback each button asks for what its
        // word needs and the band scales them together, which is the same
        // `ui_row_typography.h:9-13` rule the oracle's row follows.
        let natural = |control: &icons::Control| -> f32 {
            if input.fonts.icons_available() {
                control_size
            } else {
                widgets::measure(font, control.text, metric::UI_FONT_LABEL)
                    + row_typography::UI_ROW_LABEL_PADDING
                    + 8.0
            }
        };

        let mut group: Vec<icons::Control> = Vec::with_capacity(8);
        let mut band = None;
        // Three candidate compositions, tried richest first. `TimelineBand::fits`
        // is what rejects one — it is the same "even the minimum scale overflows"
        // answer the oracle's row consults, so the shedding here and the scaling
        // there cannot disagree about what a row can hold.
        for candidate in [
            [&[transport][..], &seek[..], &panels[..]].concat(),
            [&[transport][..], &panels[..]].concat(),
            vec![transport],
        ] {
            let widths: Vec<f32> = candidate.iter().map(natural).collect();
            let Some(laid) = TimelineBand::layout(
                bar.x,
                row_y,
                middle_width,
                control_size,
                metric::UI_CONTROL_GAP,
                &widths,
                // No trailing "Clear manual" button in the transport row, so the
                // band's clear slot is zero-width here.
                0.0,
                timecode_width,
            ) else {
                continue;
            };
            if laid.fits || candidate.len() == 1 {
                group = candidate;
                band = Some(laid);
                break;
            }
        }
        let Some(band) = band else {
            return ToolbarResult::default();
        };

        let widths: Vec<f32> = group.iter().map(natural).collect();
        let scaled: Vec<f32> = widths.iter().map(|width| width * band.scale).collect();
        let fallback_labels: Vec<&str> = group.iter().map(|control| control.text).collect();
        // One size for the whole row, whichever face it is drawn in.
        let font_size = if input.fonts.icons_available() {
            control_size * 0.5 * band.scale
        } else {
            widgets::row_font_size(font, &fallback_labels, &scaled, control_size)
        };

        let mut cursor = bar.x + metric::UI_CONTROL_GAP;
        for (index, control) in group.iter().enumerate() {
            let boundary = UiRect::new(cursor, row_y, scaled[index], control_size);
            cursor += scaled[index] + metric::UI_CONTROL_GAP * band.scale;

            // The seek group and the panel buttons are numbered by *what they are*
            // rather than by their position in the row, because the row's
            // composition changes with the window width. An id that moved with the
            // layout would let a press claimed before a resize be released by
            // whichever control inherited the index.
            let (namespace, slot, selected) = match control {
                c if *c == icons::SEEK_START => (widgets::id::SEEK, 0, false),
                c if *c == icons::SEEK_BACK => (widgets::id::SEEK, 1, false),
                c if *c == icons::SEEK_FORWARD => (widgets::id::SEEK, 2, false),
                c if *c == icons::TUNE => (widgets::id::TOOLBAR, 1, self.inspector_open),
                c if *c == icons::EXPORT => {
                    (widgets::id::TOOLBAR, 2, self.panel == UiPanel::Export)
                }
                c if *c == icons::LYRICS => {
                    (widgets::id::TOOLBAR, 3, self.panel == UiPanel::Lyrics)
                }
                c if *c == icons::ASSIST => {
                    (widgets::id::TOOLBAR, 4, self.panel == UiPanel::Assist)
                }
                _ => (widgets::id::TOOLBAR, 0, false),
            };

            // Every control here needs a track. Drawn disabled rather than hidden,
            // so the control names the feature even when it cannot run — and the
            // tooltip still answers, which is the one thing a disabled button can
            // usefully do.
            let glyph = icons::glyph(input.fonts, control);
            if !has_track {
                match &glyph {
                    icons::Glyph::Icon(face, text) => self
                        .widgets
                        .disabled_icon_button(d, face, boundary, text, font_size),
                    icons::Glyph::Text(font, text) => {
                        self.widgets
                            .disabled_button(d, font, boundary, text, Some(font_size))
                    }
                }
                continue;
            }
            let id = widgets::widget_id(namespace, slot);
            let state = match &glyph {
                icons::Glyph::Icon(face, text) => self.widgets.icon_button(
                    d,
                    face,
                    id,
                    boundary,
                    text,
                    selected,
                    ButtonStyle::Neutral,
                    font_size,
                ),
                icons::Glyph::Text(font, text) => self.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary,
                    text,
                    selected,
                    ButtonStyle::Neutral,
                    Some(font_size),
                ),
            };
            self.widgets.hint(d, state, id, boundary, control.tip);
            if !state.clicked {
                continue;
            }
            match control {
                c if *c == icons::SEEK_START => commands.push(ShellCommand::Seek(0.0)),
                c if *c == icons::SEEK_BACK || *c == icons::SEEK_FORWARD => {
                    let sign = if *control == icons::SEEK_BACK {
                        -1.0
                    } else {
                        1.0
                    };
                    commands.push(ShellCommand::Seek(self.nudge_target(d, input, sign)));
                }
                c if *c == icons::TUNE => self.set_inspector_open(!self.inspector_open),
                c if *c == icons::EXPORT => {
                    self.toggle_panel(UiPanel::Export);
                }
                c if *c == icons::LYRICS => {
                    self.toggle_panel(UiPanel::Lyrics);
                }
                c if *c == icons::ASSIST => {
                    self.toggle_panel(UiPanel::Assist);
                }
                _ => commands.push(ShellCommand::TogglePlay),
            }
        }

        if let Some(cluster) = utilities {
            self.utility_cluster(d, input, cluster, font_size, has_track, commands);
        }

        // The timecode goes where the band put it, and only if the band said it
        // fits there. Drawing it at `bar.x + bar.width - width` regardless is the
        // exact mistake `timeline_layout.h` was written to stop.
        let timecode_inline = band.timecode_inline && !band.timecode.is_empty();
        if timecode_inline {
            widgets::draw_text(
                d,
                font,
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
            middle_right - metric::UI_CONTROL_GAP
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
                    font,
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

    /// The right-hand cluster: readout toggle, mute, volume, fullscreen.
    ///
    /// Split out because the middle group and this one shed controls for different
    /// reasons — the middle against the band's scale floor, this one against the
    /// window's right edge — and interleaving the two made the one function
    /// impossible to follow.
    fn utility_cluster(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        cluster: transport_bar::UtilityCluster,
        font_size: f32,
        has_track: bool,
        commands: &mut Vec<ShellCommand>,
    ) {
        let icon_button = |shell: &mut Self,
                           d: &mut RaylibDrawHandle<'_>,
                           slot: u32,
                           boundary: UiRect,
                           control: &icons::Control,
                           selected: bool,
                           enabled: bool|
         -> bool {
            let glyph = icons::glyph(input.fonts, control);
            if !enabled {
                match &glyph {
                    icons::Glyph::Icon(face, text) => shell
                        .widgets
                        .disabled_icon_button(d, face, boundary, text, font_size),
                    icons::Glyph::Text(font, text) => {
                        shell
                            .widgets
                            .disabled_button(d, font, boundary, text, Some(font_size))
                    }
                }
                return false;
            }
            let id = widgets::widget_id(widgets::id::UTILITY, slot);
            let state = match &glyph {
                icons::Glyph::Icon(face, text) => shell.widgets.icon_button(
                    d,
                    face,
                    id,
                    boundary,
                    text,
                    selected,
                    ButtonStyle::Neutral,
                    font_size,
                ),
                icons::Glyph::Text(font, text) => shell.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary,
                    text,
                    selected,
                    ButtonStyle::Neutral,
                    Some(font_size),
                ),
            };
            shell.widgets.hint(d, state, id, boundary, control.tip);
            state.clicked
        };

        if let Some(boundary) = cluster.readout {
            // Always enabled: the readout is the one control that is *more* useful
            // with no track open, because "no track" is one of the things it says.
            if icon_button(
                self,
                d,
                0,
                boundary,
                &icons::READOUT,
                self.hud_visible,
                true,
            ) {
                self.hud_visible = !self.hud_visible;
            }
        }

        if let Some(boundary) = cluster.mute {
            let control = icons::Control {
                icon: icons::volume_icon(input.volume, input.muted),
                ..if input.muted {
                    icons::UNMUTE
                } else {
                    icons::MUTE
                }
            };
            if icon_button(self, d, 1, boundary, &control, input.muted, has_track) {
                commands.push(ShellCommand::ToggleMute);
            }
        }

        if let Some(boundary) = cluster.volume {
            // Inset by the knob's radius at each end. `slider` centres the knob on
            // the value's position, so at 0 and 1 half of it hangs outside the rect
            // it was given — and a capture at 960 px with the inspector open showed
            // exactly that: a full-volume knob touching the fullscreen button
            // beside it. The layout module reserves the box; this is the drawing's
            // own business, so it is corrected here rather than by widening the
            // reservation.
            let inset = widgets::SLIDER_KNOB_RADIUS;
            let boundary = UiRect::new(
                boundary.x + inset,
                boundary.y,
                (boundary.width - inset * 2.0).max(1.0),
                boundary.height,
            );
            // The slider shows the *stored* volume even while muted, rather than
            // dropping to zero: mute is a toggle the user expects to undo, and a
            // slider that zeroed itself would lose the level they had set.
            let id = widgets::widget_id(widgets::id::UTILITY, 2);
            if let Some(value) = self.widgets.slider(d, id, boundary, input.volume) {
                commands.push(ShellCommand::SetVolume(value));
            }
            // No `hint` here: a slider explains itself by moving, and a tooltip
            // over one being dragged covers the thing it describes.
        }

        let control = if self.fullscreen {
            icons::WINDOWED
        } else {
            icons::FULLSCREEN
        };
        if icon_button(
            self,
            d,
            3,
            cluster.fullscreen,
            &control,
            self.fullscreen,
            true,
        ) {
            self.set_fullscreen(!self.fullscreen, commands);
        }
    }

    /// Whether any text field is taking keystrokes.
    ///
    /// The shell reads the keyboard before any panel is drawn, so a panel with a
    /// focused field cannot defend itself — it has already lost the keypress by the
    /// time it runs. This is the guard, and it has to be asked *here*.
    ///
    /// It asks every surface in [`TextEntrySurface`] and nothing else; that enum
    /// is where a new one is declared (review 1.6, UX0-A06).
    fn text_entry_has_focus(&self) -> bool {
        TextEntrySurface::ALL
            .iter()
            .any(|surface| self.text_entry_focused(*surface))
    }

    /// Whether one named surface is holding the keyboard.
    ///
    /// Both arms pair the field's own focus flag with the pane being on screen,
    /// and that pairing is the point rather than belt-and-braces: neither flag is
    /// cleared when its panel closes, so a focused field left behind by a closed
    /// panel would silence every global shortcut for the rest of the session —
    /// the same stranded-state defect as UX0-A02, one layer up.
    fn text_entry_focused(&self, surface: TextEntrySurface) -> bool {
        match surface {
            // The cue field is drawn by, and only by, the lyrics panel.
            TextEntrySurface::LyricCue => self.panel == UiPanel::Lyrics && self.lyrics.is_typing(),
            // The filter is a hit-tested region rather than a
            // [`super::text_input::TextField`], and it is drawn from inside
            // another panel's body, so the browser reports its own visibility.
            TextEntrySurface::FontQuery => self.font_browser.query_has_focus(),
        }
    }

    /// The position a nudge button asks for, honouring the modifier keys.
    ///
    /// Shared with the keyboard path so a click and an arrow key cannot disagree
    /// about what Ctrl means — the tooltip states the ladder, and a tooltip that
    /// lies is worse than none.
    fn nudge_target(&self, d: &RaylibDrawHandle<'_>, input: &ShellInput<'_>, sign: f64) -> f64 {
        use raylib::consts::KeyboardKey as Key;
        let fine = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
        let coarse = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
        let step = transport_bar::seek_step_seconds(fine, coarse) * sign;
        transport_bar::nudged(input.time_seconds, step, input.duration_seconds)
    }

    /// Enters or leaves fullscreen, keeping the layout and the window in step.
    ///
    /// Two things happen and they are not the same thing: the shell's own
    /// `fullscreen` flag switches the workspace to the expanded layout, and the
    /// command asks `main.rs` to toggle the *window*. The second has to be a
    /// command because it needs `&mut RaylibHandle`, which does not exist inside a
    /// drawing pair — and because the headless probe must be able to take the
    /// layout without the window call, which would fail or hang under Xvfb.
    fn set_fullscreen(&mut self, on: bool, commands: &mut Vec<ShellCommand>) {
        if self.fullscreen == on {
            return;
        }
        self.fullscreen = on;
        self.abandon_workspace_drags(commands);
        commands.push(ShellCommand::SetFullscreen(on));
    }

    /// Ends any drag whose surface has just stopped being drawn (UX0-A02).
    ///
    /// Fullscreen hides every panel, and the gesture code that would finish a drag
    /// only runs while its panel is drawn. A splitter drag left in flight does not
    /// simply pause: on the way back out it sees a mouse button that is no longer
    /// down and writes the preferences file from a keypress the user made minutes
    /// ago — or worse, sees one that *is* down again and snaps the panel to
    /// wherever the pointer happens to be.
    ///
    /// The two drags are ended differently on purpose. A splitter is dropped
    /// silently: its width was applied to `ui_preferences` live, so nothing is
    /// lost on screen, and a save command emitted from an unrelated keypress is
    /// exactly the spurious write this fixes. A scrub is *completed*, because it
    /// paused playback when it started — dropping it would leave the track paused
    /// at a position the playhead never moved to, with no event the user could
    /// connect it to.
    fn abandon_workspace_drags(&mut self, commands: &mut Vec<ShellCommand>) {
        self.split_drag = None;
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.timeline_gesture = None;
            if let Some(target) = self.scrub_target_seconds.take() {
                commands.push(ShellCommand::Seek(target));
            }
            if std::mem::take(&mut self.scrub_restore_playing) {
                commands.push(ShellCommand::TogglePlay);
            }
        }
        // A scene-boundary drag belongs to the scene lane, which cancels its own
        // preview when it finds the gesture gone (`scene_timeline.rs:530-533`).
        // Cancelling rather than committing is right here: the boundary is only
        // retimed on release, and the release never happened.
        if self.timeline_gesture == Some(TimelineGesture::SceneBoundary) {
            self.timeline_gesture = None;
        }
    }

    /// Opens or closes the inspector, ending a drag on the boundary it owns.
    ///
    /// The inspector's splitter exists only while the inspector does, so closing
    /// it mid-drag strands the drag the same way fullscreen does — and an
    /// invisible splitter that still resizes a closed panel is the harder half to
    /// diagnose.
    fn set_inspector_open(&mut self, open: bool) {
        self.inspector_open = open;
        if !open && self.split_drag == Some(SplitKind::Inspector) {
            self.split_drag = None;
        }
    }

    fn toggle_panel(&mut self, panel: UiPanel) {
        if self.panel == panel {
            self.panel = UiPanel::None;
            return;
        }
        self.panel = panel;
    }

    /// Draggable workspace boundaries. Geometry remains a pure layout concern;
    /// this is only the immediate-mode gesture that updates its optional inputs.
    fn splitters(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        use raylib::consts::{MouseButton, MouseCursor};

        if input.workspace.current().is_none() {
            return;
        }
        const HIT: f32 = 8.0;
        let sidebar = UiRect::new(
            frame.preview.x - HIT * 0.5,
            0.0,
            HIT,
            frame.timeline.y.max(0.0),
        );
        let inspector = self
            .inspector_open
            .then(|| UiRect::new(frame.inspector.x - HIT * 0.5, 0.0, HIT, input.window.1));
        let timeline = UiRect::new(
            0.0,
            frame.timeline.y - HIT * 0.5,
            frame.widths.workspace_width,
            HIT,
        );
        let mouse = input.ui_scale.mouse(d);
        let hovered = inspector
            .filter(|rect| rect.contains_point(mouse.x, mouse.y))
            .map(|_| SplitKind::Inspector)
            .or_else(|| {
                sidebar
                    .contains_point(mouse.x, mouse.y)
                    .then_some(SplitKind::Sidebar)
            })
            .or_else(|| {
                timeline
                    .contains_point(mouse.x, mouse.y)
                    .then_some(SplitKind::Timeline)
            });

        if d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            if let Some(kind) = hovered {
                let now = d.get_time();
                let double = self
                    .last_split_press
                    .is_some_and(|(last, time)| last == kind && now - time <= 0.35);
                self.last_split_press = Some((kind, now));
                if double {
                    match kind {
                        SplitKind::Sidebar => self.ui_preferences.sidebar_width = None,
                        SplitKind::Inspector => self.ui_preferences.inspector_width = None,
                        SplitKind::Timeline => self.ui_preferences.timeline_height = None,
                    }
                    self.split_drag = None;
                    self.notify(
                        Severity::Success,
                        "Panel size: Auto",
                        "The content-aware workspace split has been restored.",
                    );
                    commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
                } else {
                    self.split_drag = Some(kind);
                }
            }
        }

        if let Some(kind) = self.split_drag {
            if d.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                match kind {
                    SplitKind::Sidebar => {
                        self.ui_preferences.sidebar_width = Some(mouse.x.clamp(168.0, 520.0))
                    }
                    SplitKind::Inspector => {
                        self.ui_preferences.inspector_width =
                            Some((input.window.0 - mouse.x).clamp(240.0, 520.0))
                    }
                    SplitKind::Timeline => {
                        self.ui_preferences.timeline_height =
                            Some((input.window.1 - mouse.y).clamp(80.0, 4096.0))
                    }
                }
            } else {
                self.split_drag = None;
                commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
            }
        }

        let active = self.split_drag.or(hovered);
        d.set_mouse_cursor(match active {
            Some(SplitKind::Timeline) => MouseCursor::MOUSE_CURSOR_RESIZE_NS,
            Some(SplitKind::Sidebar | SplitKind::Inspector) => MouseCursor::MOUSE_CURSOR_RESIZE_EW,
            None => MouseCursor::MOUSE_CURSOR_DEFAULT,
        });

        for (kind, rect) in [
            (SplitKind::Sidebar, sidebar),
            (SplitKind::Timeline, timeline),
        ] {
            draw_splitter(d, rect, kind, active);
        }
        if let Some(rect) = inspector {
            draw_splitter(d, rect, SplitKind::Inspector, active);
        }
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

        let mouse = input.ui_scale.mouse(d);
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
        let mut clip = widgets::begin_scissor(d, area, input.ui_scale);
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
            if state.clicked && id != input.scene {
                commands.push(ShellCommand::SelectScene(id));
            }
        }
    }

    /// The track's amplitude envelope behind the timeline
    /// (`draw_timeline_waveform`, `plug.c:2696-2751`).
    ///
    /// One vertical line per pixel column, each spanning the min and max of every
    /// envelope bin the column covers — so a zoomed-out view of a five-minute track
    /// shows peaks rather than whatever a single sampled bin happened to hold. The
    /// bin range per column comes from [`TimelineView::seconds_at`] at the column's
    /// two edges, which is what keeps the envelope aligned with the ticks and the
    /// playhead under zoom instead of merely near them.
    ///
    /// `end = first + 1` when the ranges collapse (`plug.c:2729`): zoomed in far
    /// enough, several columns fall inside one bin, and without that floor they
    /// would each draw an empty span and the envelope would vanish exactly where it
    /// is being inspected most closely.
    fn waveform_lane(
        &self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        strip: UiRect,
        duration: f64,
    ) {
        let centre = strip.y + strip.height * 0.5;
        d.draw_line_ex(
            Vector2::new(strip.x, centre),
            Vector2::new(strip.x + strip.width, centre),
            1.0,
            widgets::alpha(color::ui_muted(), 0.28),
        );

        let bins = input
            .workspace
            .current()
            .and_then(|track| track.timeline_waveform.as_ref())
            .map_or(&[][..], |waveform| waveform.bins());
        if bins.is_empty()
            || strip.width < 1.0
            || strip.height < 4.0
            || !duration.is_finite()
            || duration <= 0.0
        {
            // Said, not left blank. A flat lane and an undecodable file look
            // identical, and one of them means the track will export silence.
            let message = "Waveform unavailable";
            let font = input.fonts.ui();
            let width = widgets::measure(font, message, metric::UI_FONT_CAPTION);
            widgets::draw_text(
                d,
                font,
                message,
                strip.x + (strip.width - width) * 0.5,
                centre - metric::UI_FONT_CAPTION * 0.5,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            return;
        }

        let columns = (strip.width.floor() as usize).clamp(1, 4096);
        let amplitude = (strip.height * 0.43).max(1.0);
        let bins_per_second = bins.len() as f64 / duration;
        for column in 0..columns {
            let start = self.timeline.seconds_at(
                f64::from(strip.x) + column as f64,
                f64::from(strip.x),
                f64::from(strip.width),
                duration,
            );
            let end = self.timeline.seconds_at(
                f64::from(strip.x) + column as f64 + 1.0,
                f64::from(strip.x),
                f64::from(strip.width),
                duration,
            );
            let mut first = (start * bins_per_second) as usize;
            if first >= bins.len() {
                first = bins.len() - 1;
            }
            let mut last = (end * bins_per_second) as usize;
            if last <= first {
                last = first + 1;
            }
            let last = last.min(bins.len());

            // Seeded at zero, not at the first bin, which is the C's own choice
            // (`plug.c:2733-2734`): every column's span therefore includes the
            // centre line, so a quiet passage draws a thin line rather than a
            // detached sliver floating above or below it.
            let mut minimum = 0.0f32;
            let mut maximum = 0.0f32;
            for bin in &bins[first..last] {
                minimum = minimum.min(bin.minimum);
                maximum = maximum.max(bin.maximum);
            }
            let x = strip.x + (column as f32 + 0.5) * strip.width / columns as f32;
            let peak = minimum.abs().max(maximum.abs()).min(1.0);
            let colour = widgets::alpha(
                widgets::brightness(color::accent(), -0.18 + peak * 0.24),
                0.38 + peak * 0.48,
            );
            d.draw_line_ex(
                Vector2::new(x, centre - maximum * amplitude),
                Vector2::new(x, centre - minimum * amplitude),
                1.0,
                colour,
            );
        }
    }

    /// The timeline strip: waveform lane, ticks, playhead, scrubber.
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
        // The manual event row, above the waveform lane (`plug.c:2861-2971`).
        // It reports what it took; 0.0 means it could not seat its controls and
        // the strip gets the space back.
        let mut events = Vec::new();
        let row = self.event_row(
            d,
            input,
            UiRect::new(
                content.x + padding,
                content.y + padding,
                (content.width - padding * 2.0).max(0.0),
                (content.height - padding * 2.0).max(0.0),
            ),
            &mut events,
        );
        commands.extend(events.into_iter().map(ShellCommand::ManualEvent));
        let scene_row = self.scene_plan_section(
            d,
            input,
            UiRect::new(
                content.x + padding,
                content.y + padding + row,
                (content.width - padding * 2.0).max(0.0),
                (content.height - padding * 2.0 - row).max(0.0),
            ),
            commands,
        );
        let strip = UiRect::new(
            content.x + padding,
            content.y + padding + row + scene_row,
            (content.width - padding * 2.0).max(0.0),
            56.0f32.min((content.height - padding * 2.0 - row - scene_row).max(0.0)),
        );
        if strip.is_empty() {
            return;
        }
        d.draw_rectangle_rec(widgets::rectangle(strip), color::ui_raised());
        d.draw_rectangle_lines_ex(widgets::rectangle(strip), 1.0, color::ui_rule());
        self.waveform_lane(d, input, strip, duration);

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
        let mouse = input.ui_scale.mouse(d);
        let over_strip = mouse.x >= strip.x
            && mouse.x <= strip.x + strip.width
            && mouse.y >= strip.y
            && mouse.y <= strip.y + strip.height;
        let wheel = d.get_mouse_wheel_move();
        // A boundary drag stores a time under the pointer. Zooming the view out
        // from underneath it would move that time while the hand stayed still.
        if over_strip
            && wheel != 0.0
            && self.timeline_gesture != Some(TimelineGesture::SceneBoundary)
        {
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
                    // Top of the lane, and never at zero (`plug.c:3066-3069`).
                    // Both of those were guesses until the waveform landed
                    // underneath and made them checkable: the label at zero sits on
                    // the lane's left edge and is clipped to half a timestamp, and
                    // at the bottom the labels compete with the loudest part of the
                    // envelope instead of with its quiet centre. The height gate is
                    // the oracle's too — a short lane drops labels rather than
                    // printing them across the waveform.
                    //
                    // Not smaller than UI_FONT_CAPTION: the 11 px labels in an
                    // earlier capture rendered the colon and the point as boxes
                    // in raylib's 10 px bitmap font. A tick label nobody can read
                    // is a tick label that is not there.
                    if tick > 0.0 && strip.height >= 48.0 {
                        widgets::draw_text(
                            d,
                            input.fonts.ui(),
                            &widgets::format_timestamp(tick),
                            x + 4.0,
                            strip.y + 4.0,
                            metric::UI_FONT_CAPTION,
                            color::ui_muted(),
                        );
                    }
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
            // The grab handle at the top (`plug.c:3109-3112`). A bare line was
            // enough to read while the lane was empty; over an envelope in the same
            // accent colour it is not, which is presumably why the oracle has one.
            d.draw_triangle(
                Vector2::new(playhead - 5.0, strip.y),
                Vector2::new(playhead, strip.y + 7.0),
                Vector2::new(playhead + 5.0, strip.y),
                color::accent(),
            );
        }
        // Follow playback with the least scroll that keeps the playhead inside,
        // which is safe to call every frame (`timeline_view.h:52-55`).
        // Keep the view stationary while a boundary owns the pointer. Playback
        // may continue, but follow-scrolling here would make a stationary hand
        // retime the cue as the playhead approached the edge.
        if input.playing && self.timeline_gesture != Some(TimelineGesture::SceneBoundary) {
            self.timeline.reveal(duration, input.time_seconds);
        }

        // Scrub. The drag is tracked here rather than through the button claim
        // because a scrub that leaves the strip must keep scrubbing.
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;
        if over_strip
            && self.timeline_gesture.is_none()
            && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
        {
            self.timeline_gesture = Some(TimelineGesture::Scrub);
            self.scrub_restore_playing = input.playing;
            if input.playing {
                commands.push(ShellCommand::TogglePlay);
            }
        }
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            let seconds = self.timeline.seconds_at(
                f64::from(mouse.x),
                f64::from(strip.x),
                f64::from(strip.width),
                duration,
            );
            self.scrub_target_seconds = Some(seconds);
            if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
                self.timeline_gesture = None;
                if let Some(target) = self.scrub_target_seconds.take() {
                    commands.push(ShellCommand::Seek(target));
                }
                if std::mem::take(&mut self.scrub_restore_playing) {
                    commands.push(ShellCommand::TogglePlay);
                }
            }
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

        // The fine-positioning ladder, written where the positioning happens.
        //
        // It is here rather than only in the seek buttons' tooltips because the
        // seek group is the *first* thing the transport row sheds: below about
        // 700 px of toolbar those three buttons are gone, and with them the only
        // place their modifiers were named. The keys still work, so a line that
        // disappears with the buttons would hide a working feature — the exact
        // shape of failure this repository has a rule about.
        //
        // Drawn only when there is room beside the zoom readout, since printing
        // through it would be worse than not saying so.
        let zoom_width = widgets::measure(input.fonts.ui(), &zoom_label, metric::UI_FONT_CAPTION);
        let hint =
            "Arrows: 1 s  \u{00b7}  Ctrl: 0.1 s  \u{00b7}  Shift: 10 s  \u{00b7}  Home/End: ends";
        let hint_width = widgets::measure(input.fonts.ui(), hint, metric::UI_FONT_CAPTION);
        let hint_x = strip.x + zoom_width + 24.0;
        if hint_x + hint_width <= strip.x + strip.width - 92.0 {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                hint,
                hint_x,
                strip.y + strip.height + 4.0,
                metric::UI_FONT_CAPTION,
                color::ui_disabled(),
            );
        }

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
    fn notice_tray(&mut self, d: &mut RaylibDrawHandle<'_>, font: &UiFonts, preview: UiRect) {
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

fn draw_splitter(
    d: &mut RaylibDrawHandle<'_>,
    hit: UiRect,
    kind: SplitKind,
    active: Option<SplitKind>,
) {
    let selected = active == Some(kind);
    let tint = if selected {
        color::accent()
    } else {
        color::ui_rule()
    };
    let thickness = if selected { 2.0 } else { 1.0 };
    match kind {
        SplitKind::Sidebar | SplitKind::Inspector => {
            let x = hit.x + hit.width * 0.5;
            d.draw_line_ex(
                Vector2::new(x, hit.y),
                Vector2::new(x, hit.y + hit.height),
                thickness,
                tint,
            );
        }
        SplitKind::Timeline => {
            let y = hit.y + hit.height * 0.5;
            d.draw_line_ex(
                Vector2::new(hit.x, y),
                Vector2::new(hit.x + hit.width, y),
                thickness,
                tint,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stub_panel_does_not_reserve_height_it_never_draws() {
        // The rule, as a test: height is reserved by panels that draw rows, and
        // by nothing else. The list of which panels those are has changed twice
        // as the fan-out landed — Lyrics and Export were on the stub side of it
        // until their agents finished — and each time this test is what said so.
        let mut shell = Shell::new();
        let workspace = crate::workspace::Workspace::new();
        let baseline = shell.timeline_height((1280.0, 720.0), &workspace);
        // Tune is the inspector, not a bottom panel: it draws no rows down here.
        shell.panel = UiPanel::Tune;
        assert_eq!(
            shell.timeline_height((1280.0, 720.0), &workspace),
            baseline,
            "Tune reserved height for rows it never draws"
        );
        // Every panel that draws rows asks for the room. This list has grown
        // once per agent as the fan-out landed, and each time this test is what
        // said the old one had expired.
        for panel in [UiPanel::Export, UiPanel::Lyrics, UiPanel::Assist] {
            shell.panel = panel;
            assert!(
                shell.timeline_height((1280.0, 1080.0), &workspace) > baseline,
                "{panel:?} draws rows but did not ask for their height"
            );
        }
    }

    #[test]
    fn toggling_a_real_panel_twice_returns_to_the_timeline_without_a_stub_notice() {
        let mut shell = Shell::new();
        shell.toggle_panel(UiPanel::Export);
        assert_eq!(shell.panel, UiPanel::Export);
        shell.toggle_panel(UiPanel::Export);
        assert_eq!(shell.panel, UiPanel::None);
    }

    /// The toolbar's own band, computed the way [`Shell::toolbar`] computes it.
    ///
    /// `controls` is how many icon buttons the row is trying to seat: 8 for the
    /// full composition (transport, the seek trio, four panels), 5 with the seek
    /// trio shed, 1 for the transport button alone.
    ///
    /// With the icon face loaded every control is a square, so unlike the oracle's
    /// text row there is nothing to measure — which is why this helper no longer
    /// needs the stubbed measurer the old one carried.
    fn toolbar_band(bar_width: f32, controls: usize) -> Option<TimelineBand> {
        use musializer_core::ui::transport_bar;

        let bar = UiRect::new(0.0, 0.0, bar_width, metric::HUD_BUTTON_SIZE);
        let utilities = transport_bar::utilities(bar, 0.0, true);
        let middle = utilities.map_or(bar_width, |cluster| cluster.left_edge - bar.x);
        let widths = vec![transport_bar::CONTROL_SIZE; controls];
        // The default face's rough average advance at the value size; the timecode
        // is the one thing in this row still measured as text.
        let timecode_width =
            "00:00.000 / 00:00.000".chars().count() as f32 * metric::UI_FONT_VALUE * 0.5;
        TimelineBand::layout(
            bar.x,
            0.0,
            middle.max(0.0),
            transport_bar::CONTROL_SIZE,
            metric::UI_CONTROL_GAP,
            &widths,
            // No trailing "Clear manual" button in the transport row.
            0.0,
            timecode_width,
        )
    }

    #[test]
    fn the_toolbar_never_squeezes_its_controls_below_the_legibility_floor() {
        // The band's contract: it will shrink to TIMELINE_BAND_MIN_SCALE and no
        // further, and it says so through `fits`. A capture at 960x640 with the
        // inspector open — a 440 px toolbar — is what caught the old arithmetic
        // reading "Pau?  Tune  Exp?  Lyr?".
        use musializer_core::ui::timeline_layout::TIMELINE_BAND_MIN_SCALE;

        for width in [440.0f32, 640.0, 960.0, 1280.0] {
            let band = toolbar_band(width, 5).expect("the band accepts these inputs");
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
        // which is why `ToolbarResult` travels. A capture at 960x640 with the
        // inspector open shows exactly that handover.
        let narrow = toolbar_band(440.0, 8).expect("valid");
        let wide = toolbar_band(1280.0, 8).expect("valid");
        assert!(
            !narrow.timecode_inline || !narrow.fits,
            "a 440 px band claimed room for both the row and the timecode"
        );
        assert!(wide.timecode_inline, "a 1280 px band should seat both");
        assert!(wide.fits);
        assert!(!wide.controls.overlaps(wide.timecode));
    }

    #[test]
    fn the_toolbar_sheds_whole_groups_rather_than_overflowing() {
        // `fits == false` does not mean the band returned something that fits. It
        // means the band has already scaled to its floor and the row still
        // overflows, so **the caller has to drop controls**
        // (`timeline_layout.h:45-47`). `Shell::toolbar` responds by trying three
        // compositions richest-first, and the invariant is that the last of them —
        // the lone transport button — always fits.
        //
        // Swept rather than spot-checked because the interesting widths are the two
        // boundaries, and neither is where anyone would guess.
        for width in 200..=1920 {
            let bar = width as f32;
            let chosen = [8usize, 5, 1].into_iter().find_map(|count| {
                let band = toolbar_band(bar, count)?;
                (band.fits || count == 1).then_some((count, band))
            });
            let Some((count, band)) = chosen else {
                // Too narrow even for the utility cluster's fullscreen button; the
                // row draws nothing, which is honest rather than degenerate.
                continue;
            };
            if band.fits {
                assert!(
                    band.controls_width <= bar + 0.01,
                    "{width}px: the band said a {}px row of {count} fits, and it does not",
                    band.controls_width
                );
            } else {
                assert_eq!(
                    count, 1,
                    "{width}px settled on {count} controls that do not fit"
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

    /// The utility cluster and the middle group must never overlap, at any width.
    ///
    /// They are laid out by two different modules against opposite edges of the
    /// same bar, which is precisely the arrangement `timeline_layout.h:12-21`
    /// records as having printed one group through the other in the C.
    #[test]
    fn the_middle_group_never_reaches_into_the_utility_cluster() {
        use musializer_core::ui::transport_bar;

        for width in 200..=1920 {
            let bar = UiRect::new(0.0, 0.0, width as f32, metric::HUD_BUTTON_SIZE);
            let Some(cluster) = transport_bar::utilities(bar, 0.0, true) else {
                continue;
            };
            for count in [8usize, 5, 1] {
                let Some(band) = toolbar_band(width as f32, count) else {
                    continue;
                };
                if !band.fits && count != 1 {
                    continue;
                }
                let leftmost = [cluster.readout, cluster.mute, cluster.volume]
                    .into_iter()
                    .flatten()
                    .chain(std::iter::once(cluster.fullscreen))
                    .map(|rect| rect.x)
                    .fold(f32::MAX, f32::min);
                assert!(
                    band.controls.x + band.controls.width <= leftmost + 0.01,
                    "{width}px with {count} controls: the row reaches {} into a cluster starting at {leftmost}",
                    band.controls.x + band.controls.width
                );
                break;
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

    fn context() -> KeyboardContext {
        KeyboardContext {
            ui_scale: UiScale::default(),
            time_seconds: 30.0,
            duration_seconds: 120.0,
            scene_index: 0,
        }
    }

    /// Every global shortcut on one frame. Chorded and repeat-only keys are left
    /// out: this is the set a user types into a text field by accident.
    fn every_shortcut() -> KeyboardFrame {
        KeyboardFrame {
            toggle_play: true,
            toggle_fullscreen: true,
            toggle_mute: true,
            toggle_hud: true,
            seek_start: true,
            seek_end: true,
            nudge_back: true,
            cycle_scene: true,
            toggle_inspector: true,
            ..KeyboardFrame::default()
        }
    }

    /// Puts one named surface in the state the user reaches by clicking into it
    /// and typing.
    ///
    /// A `match` rather than a list of setters, so that adding a
    /// [`TextEntrySurface`] variant fails to compile here until somebody says how
    /// it takes focus — which is the half of UX0-A06 that was missed.
    fn focus(shell: &mut Shell, surface: TextEntrySurface) {
        match surface {
            TextEntrySurface::LyricCue => {
                shell.panel = UiPanel::Lyrics;
                let document = musializer_core::project::lyrics::LyricsDocument::new(120.0)
                    .expect("a 120 s document is valid");
                shell.lyrics.begin_new(&document, 0.0);
            }
            TextEntrySurface::FontQuery => shell.font_browser.focus_query_for_test(),
        }
    }

    /// UX0-A06 (review 1.6). Typing "Space Mono" into the font filter used to
    /// toggle playback, fullscreen, mute and the readout, cycle the scene, open
    /// the inspector and seek the track — one shortcut per letter.
    #[test]
    fn no_global_shortcut_fires_while_any_text_surface_has_focus() {
        for surface in TextEntrySurface::ALL {
            let mut shell = Shell::new();
            focus(&mut shell, surface);
            assert!(
                shell.text_entry_has_focus(),
                "{surface:?} does not report focus, so the guard cannot see it"
            );

            let before = (shell.hud_visible, shell.inspector_open, shell.fullscreen);
            let mut commands = Vec::new();
            shell.keyboard_actions(every_shortcut(), context(), &mut commands);

            assert_eq!(commands, Vec::new(), "{surface:?} let a command through");
            assert_eq!(
                (shell.hud_visible, shell.inspector_open, shell.fullscreen),
                before,
                "{surface:?} let a shortcut change the shell"
            );
        }
    }

    /// The other half: a guard that suppressed everything unconditionally would
    /// pass the test above and break the application.
    #[test]
    fn every_global_shortcut_still_fires_when_nothing_is_being_typed_into() {
        let mut shell = Shell::new();
        assert!(!shell.text_entry_has_focus());

        let mut commands = Vec::new();
        shell.keyboard_actions(every_shortcut(), context(), &mut commands);

        assert!(commands.contains(&ShellCommand::TogglePlay));
        assert!(commands.contains(&ShellCommand::ToggleMute));
        assert!(commands.contains(&ShellCommand::SetFullscreen(true)));
        assert!(commands.contains(&ShellCommand::Seek(0.0)));
        assert!(commands.contains(&ShellCommand::Seek(120.0)));
        assert!(commands
            .iter()
            .any(|command| matches!(command, ShellCommand::SelectScene(_))));
        assert!(shell.hud_visible);
        assert!(shell.inspector_open);
    }

    /// A stale focus flag is the same defect as a stale widget claim: the panel
    /// goes, the flag stays, and the keyboard is dead for the rest of the session.
    #[test]
    fn a_text_surface_that_stopped_being_drawn_stops_taking_the_keyboard() {
        for surface in TextEntrySurface::ALL {
            let mut shell = Shell::new();
            focus(&mut shell, surface);
            assert!(shell.text_entry_has_focus());

            // Close the pane the way its own control does, without touching the
            // field: the lyrics panel is dismissed, the font browser stops
            // drawing its filter.
            shell.panel = UiPanel::None;
            shell.begin_frame(UiScale::default());

            assert!(
                !shell.text_entry_has_focus(),
                "{surface:?} kept the keyboard after its pane closed"
            );
        }
    }

    /// UX0-A02, the splitter half. `F` mid-drag left `split_drag` set, and the
    /// save it owed fired on the way back out of fullscreen — minutes later, from
    /// an unrelated keypress.
    #[test]
    fn fullscreen_taken_mid_splitter_drag_saves_nothing_and_leaves_no_drag() {
        let mut shell = Shell::new();
        shell.split_drag = Some(SplitKind::Timeline);

        let mut commands = Vec::new();
        shell.keyboard_actions(
            KeyboardFrame {
                toggle_fullscreen: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );

        assert!(shell.fullscreen);
        assert_eq!(shell.split_drag, None);
        assert_eq!(commands, vec![ShellCommand::SetFullscreen(true)]);
    }

    #[test]
    fn closing_the_inspector_mid_drag_leaves_no_drag_on_a_splitter_that_is_gone() {
        let mut shell = Shell::new();
        shell.set_inspector_open(true);
        shell.split_drag = Some(SplitKind::Inspector);

        shell.set_inspector_open(false);
        assert_eq!(shell.split_drag, None);

        // A drag on one of the other two boundaries is not the inspector's to end.
        shell.set_inspector_open(true);
        shell.split_drag = Some(SplitKind::Sidebar);
        shell.set_inspector_open(false);
        assert_eq!(shell.split_drag, Some(SplitKind::Sidebar));
    }

    /// The same stranding, one gesture over: a scrub pauses playback when it
    /// starts, so abandoning it silently would leave the track paused at a
    /// position the playhead never reached.
    #[test]
    fn fullscreen_taken_mid_scrub_completes_the_seek_rather_than_stranding_it() {
        let mut shell = Shell::new();
        shell.timeline_gesture = Some(TimelineGesture::Scrub);
        shell.scrub_target_seconds = Some(42.0);
        shell.scrub_restore_playing = true;

        let mut commands = Vec::new();
        shell.keyboard_actions(
            KeyboardFrame {
                toggle_fullscreen: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );

        assert_eq!(shell.timeline_gesture, None);
        assert_eq!(shell.scrub_target_seconds, None);
        assert!(commands.contains(&ShellCommand::Seek(42.0)));
        assert!(commands.contains(&ShellCommand::TogglePlay));
        assert!(!shell.scrub_restore_playing);
    }
}
