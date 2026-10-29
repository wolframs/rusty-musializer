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

use std::path::{Path, PathBuf};

use musializer_core::project::event_timeline::ManualEventAction;
use musializer_core::project::preset_store::{PresetAction, SharedPresetsView};
use musializer_core::scene::routes::{ParameterMapping, RouteSources};
use musializer_core::scene::{SceneId, SceneSettings};
use musializer_core::ui::notice::{self, NoticeQueue, NoticeSpec, Severity};
use musializer_core::ui::scene_browser::{PickerState, SceneFamily};
use musializer_core::ui::scroll_list::{BarHit, ListMetrics, ScrollState};
use musializer_core::ui::timeline_view::{self, TimelineView};
use musializer_core::ui::transport_bar::{self, TransportRow};
use musializer_core::ui::workspace_layout::{RailIcon, UiRect};
use musializer_runtime::font::{Faces, UiFonts};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::icons;
use super::panels::{lyrics, scene_timeline};
use super::preferences::UiPreferences;
use super::scale::{UiScale, UiScalePreference};
use super::shell_layout::{LayoutOverrides, WelcomeFrame, WorkspaceFrame, DEFAULT_TIMELINE_HEIGHT};
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle, Widgets};
use crate::cli::UiPanel;
use crate::workspace::{SaveState, Workspace};
use musializer_core::ui::workspace_layout::TOP_BAR_HEIGHT;

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
    /// Show the scene owning an existing route draft without rewriting a scene
    /// plan or changing the track's saved base scene.
    ResumeRouteDraft {
        track_slot: usize,
        scene: SceneId,
    },
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
    /// A file the user dropped on the window, or picked, that is being *tried*
    /// as audio.
    ///
    /// "Tried as" rather than "is": [`classify_drop`] sends anything it does not
    /// recognise here, which is the oracle's own else branch (`plug.c:7559-7562`).
    /// The failure notice therefore has to name the type that was attempted, or a
    /// dropped `.pdf` reports as a corrupt song.
    LoadTrack(PathBuf),
    /// A dropped `.musi` (D1, `plug.c:7542-7547`).
    ///
    /// Distinct from [`Self::OpenProject`], which asks for a path; this one
    /// already has one and must not raise a picker.
    OpenDroppedProject(PathBuf),
    /// A dropped or picked PNG/JPEG/BMP, to become ASCII Field's glyph grid
    /// (D1/D2, `plug.c:7548-7559`).
    ImportAsciiImage(PathBuf),
    /// Ask for an image through a native picker, then import it (D2,
    /// `plug.c:6358-6394`). A command for the same reason [`Self::OpenAudio`] is.
    ImportAsciiImageDialog,
    /// Drop the current track's image-backed grid (D2, `plug.c:6386-6393`).
    ClearAsciiImage,
    /// Reopen a project named by the welcome screen's recent list (UX0-C06).
    OpenRecentProject(PathBuf),
    /// Take an entry out of the recent list and persist the shorter list.
    ///
    /// The only way a user can act on an entry whose file has moved, which is why
    /// it exists at all: the alternative is a row that errors every time it is
    /// clicked and can never be got rid of.
    ForgetRecentProject(PathBuf),
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
    /// Ask for a destination and publish the frame at the playhead as a PNG
    /// (UX0-C10).
    ///
    /// A separate command rather than a flag on [`Self::StartRender`] because
    /// the two produce different files through different backends — a still
    /// needs no encoder — and a single command with a mode would make "did the
    /// press start an export?" un-answerable from the seam's own type.
    ExportStill,
    /// Ask for a destination and write a contact sheet of the track's
    /// sections (`--sheet-at segments`) through the batch path the CLI uses
    /// (WX-5c §4, the top bar's Contact sheet). Separate from
    /// [`Self::ExportStill`] for the reason that one is separate from
    /// [`Self::StartRender`].
    ExportContactSheet,
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
    /// Restore the app-owned session snapshot advertised on the welcome screen.
    RecoverSession,
    /// Explicitly discard both recovery generations.
    DiscardRecovery,
    /// Persist workstation UI state outside the current `.musi` project.
    SaveUiPreferences(UiPreferences),
    /// Write the current track's cue document to a `.lyrics.tsv` through a
    /// native save dialog (`export_lyrics_document`, `lyrics_editor_ui.c:1084-1140`).
    ///
    /// A command for the same reason [`Self::OpenAudio`] is: the picker is modal
    /// and blocks until answered, and doing that inside a begin/end drawing pair
    /// would hold the frame open across it. The two lyrics variants were the
    /// only thing missing — `LyricsDocument::bridge_export`/`bridge_import` have
    /// been ported and tested since Agent B's band and had no caller (D3).
    ExportLyrics,
    /// Replace the current track's cue document from a `.lyrics.tsv`.
    ImportLyrics,
    /// A dropped `*.protocol.json` (HX-2). Additive over D1: the double
    /// extension can never be audio or an image, and a bare `.json` still
    /// takes the oracle's else branch into the audio decoder.
    OpenDroppedProtocol(PathBuf),
    /// Apply a `*.treatment.json` now and again whenever it changes (WX-3).
    FollowTreatment(PathBuf),
    StopFollowingTreatment,
    /// One keystroke of a running feedback protocol (HX-2). A command because
    /// every consequence — applying a snapshot, seeking the stream, appending
    /// the answer line — needs owners the shell may not hold.
    Protocol(ProtocolAction),
}

/// What a protocol keystroke asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolAction {
    /// Keys `1`-`4`: answer the current item.
    Answer(u8),
    /// `R`: audition the current item's window again.
    Replay,
    /// `B`: the other look, re-auditioned. Never named `a` or `b` on screen.
    Flip,
    /// `N`: move to the next item without answering this one.
    Next,
}

impl ShellCommand {
    /// Whether this command, when it succeeds, changes data that a `.musi` file
    /// records — and therefore whether its handler owes the track a
    /// `Track::mark_dirty` (C1).
    ///
    /// # Why this exists rather than a comment
    ///
    /// C1 asked for an *audit*, and an audit is a fact about one afternoon. The
    /// next command added to this enum gets no audit, and the way that failure
    /// presents is the worst one available: the edit works, the interface looks
    /// right, and the work is silently never written. This match is exhaustive,
    /// so adding a variant does not compile until somebody answers the question.
    ///
    /// It classifies the *command*, not the handler, so it cannot by itself prove
    /// a handler calls `mark_dirty` — `mutates_project` is the checklist, and the
    /// per-path tests are the check. What it does guarantee is that the checklist
    /// can never silently fall out of date with the enum.
    ///
    /// The audit this encodes (2026-08-07) found the marking already correct for
    /// every `true` arm below; the two C1 defects were on paths that do not go
    /// through a `ShellCommand` at all — the ASCII import and the lyric-edit
    /// drain — which is itself the argument for classifying those separately
    /// rather than trusting a walk of this enum.
    #[must_use]
    #[allow(
        dead_code,
        reason = "the C1 checklist itself. Only the classification test reads it, and its value is that the exhaustive match refuses to compile when a variant is added — which is a property of it existing, not of it being called"
    )]
    pub(crate) fn mutates_project(&self) -> bool {
        match self {
            // Scene binding, tuning, routes, events, plan and output settings are
            // all fields of the serialized project.
            ShellCommand::SelectScene(_)
            | ShellCommand::SetSetting { .. }
            | ShellCommand::ResetScene(_)
            | ShellCommand::SetAutoScenes(_)
            | ShellCommand::ApplyRoute { .. }
            | ShellCommand::RemoveRoute { .. }
            | ShellCommand::SetRenderConfig(_)
            | ShellCommand::ManualEvent(_)
            | ShellCommand::ScenePlan(_) => true,
            // `Preset::Apply` writes the scene settings and so is durable; the
            // other arms write the *shared* preset store, which is its own file
            // and not `.musi` state. The command cannot be split finer here, so
            // it counts as durable and `handle_preset` marks only on Apply.
            ShellCommand::Preset(_) => true,
            // Transport, selection, window and device state. None of it is
            // written, and marking any of it dirty would make a project autosave
            // itself for being *listened to* — which is how a "modified" flag
            // stops meaning anything.
            ShellCommand::TogglePlay
            | ShellCommand::Seek(_)
            | ShellCommand::SelectTrack(_)
            | ShellCommand::ResumeRouteDraft { .. }
            | ShellCommand::SetVolume(_)
            | ShellCommand::ToggleMute
            | ShellCommand::SetFullscreen(_)
            | ShellCommand::RecoverSession
            | ShellCommand::DiscardRecovery
            | ShellCommand::StartRender
            // A still is a read of the project, written to a PNG outside it
            // (UX0-C10) — same footing as StartRender.
            | ShellCommand::ExportStill
            // A sheet is many stills: a read of the project, like ExportStill.
            | ShellCommand::ExportContactSheet => false,
            // The ASCII image is four fields of the serialized project (D2):
            // import and clear both change what a `.musi` records, and their
            // handlers mark dirty on success. The dialog variant is classified
            // below with `OpenAudio`: it only asks which file, and the durable
            // command it produces is the one that owes the mark.
            ShellCommand::ImportAsciiImage(_) | ShellCommand::ClearAsciiImage => true,
            // These write files, or replace the track wholesale. A freshly opened
            // or saved track is clean by definition, and `SaveUiPreferences`
            // writes the per-user config rather than the project.
            ShellCommand::LoadTrack(_)
            | ShellCommand::OpenAudio
            | ShellCommand::OpenProject
            | ShellCommand::OpenDroppedProject(_)
            | ShellCommand::OpenRecentProject(_)
            | ShellCommand::ImportAsciiImageDialog
            | ShellCommand::SaveProject
            | ShellCommand::SaveProjectAs
            | ShellCommand::SaveUiPreferences(_) => false,
            // The recent list is per-user config (`recent.json`), not `.musi`
            // state (UX0-C06).
            ShellCommand::ForgetRecentProject(_) => false,
            // A TSV import replaces the timed-lyrics document transactionally
            // and its handler marks dirty; an export only reads it, and D3's
            // contract is explicit that exporting must not dirty the project.
            ShellCommand::ImportLyrics => true,
            ShellCommand::ExportLyrics => false,
            // Following is a session choice; each time the file applies, the
            // handler marks the track dirty itself (WX-3), because the apply
            // happens on a later frame than the command.
            ShellCommand::FollowTreatment(_) => true,
            ShellCommand::StopFollowingTreatment => false,
            // A protocol session is a session artifact (HX-1's whole argument
            // for the sidecar): answers append to `*.answers.jsonl`, and the
            // snapshots it applies are auditions, not authored work.
            ShellCommand::OpenDroppedProtocol(_) | ShellCommand::Protocol(_) => false,
        }
    }
}

/// What a dropped file is being taken for (D1).
///
/// A named answer rather than a `bool` pair, because the whole requirement is
/// that a failure is *reported by its attempted type* — and a two-branch `if`
/// leaves the third arm's wording to whoever writes the error, which is how
/// "Audio could not be loaded" ended up on a dropped `.musi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    /// `.musi` (`plug.c:7542`).
    Project,
    /// PNG, JPEG or BMP (`plug.c:7548-7551`).
    Image,
    /// Everything else, which is the oracle's else branch rather than a
    /// whitelist (`plug.c:7559`). A `.txt` is *attempted* as audio and refused by
    /// the decoder, which is a better answer than a fourth "unsupported" arm: the
    /// list of formats raylib can open is raylib's to know, not this function's,
    /// and a whitelist here would start silently refusing files that work.
    Audio,
    /// `*.protocol.json` (HX-2). Matched on the double extension, not on
    /// `.json`: a bare `.json` is not a protocol and keeps taking the audio
    /// branch, exactly as the oracle's else arm would.
    Protocol,
    /// `*.treatment.json` (WX-3): applied to the open track and then
    /// followed, so an agent's edits show up as it saves them.
    Treatment,
}

impl DropKind {
    /// The noun a failure notice uses. Kept beside the classification so the two
    /// cannot drift.
    #[must_use]
    pub fn attempted_noun(self) -> &'static str {
        match self {
            DropKind::Project => "project",
            DropKind::Image => "image",
            DropKind::Audio => "audio file",
            DropKind::Protocol => "protocol",
            DropKind::Treatment => "treatment",
        }
    }
}

/// Which of the three branches a dropped path takes (D1, `plug.c:7542-7562`).
///
/// Case-insensitive, which the oracle's `IsFileExtension` is not. A deliberate
/// divergence: a `.PNG` off a camera or a Windows share is unambiguously an
/// image, and the C's behaviour there — hand it to the audio decoder and report
/// a corrupt song — is a defect rather than a contract. Nothing in a `.musi`, an
/// MP4 or a documented command line can observe the difference.
#[must_use]
pub fn classify_drop(path: &Path) -> DropKind {
    // The protocol arm keys on the *double* extension (HX-2), checked before
    // the single-extension match because `Path::extension` only ever sees
    // `json`. Case-insensitive like everything else here.
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.ends_with(".protocol.json") {
        return DropKind::Protocol;
    }
    if name.ends_with(".treatment.json") {
        return DropKind::Treatment;
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "musi" => DropKind::Project,
        "png" | "jpg" | "jpeg" | "bmp" => DropKind::Image,
        _ => DropKind::Audio,
    }
}

/// [`classify_drop`] as the command it produces.
#[must_use]
fn drop_command(path: &Path) -> ShellCommand {
    let path = path.to_path_buf();
    match classify_drop(&path) {
        DropKind::Project => ShellCommand::OpenDroppedProject(path),
        DropKind::Image => ShellCommand::ImportAsciiImage(path),
        DropKind::Audio => ShellCommand::LoadTrack(path),
        DropKind::Protocol => ShellCommand::OpenDroppedProtocol(path),
        DropKind::Treatment => ShellCommand::FollowTreatment(path),
    }
}

/// What the shell needs to know to draw one frame.
///
/// Borrowed, never owned. The lifetime is the contract: the shell may not retain
/// anything it is handed.
#[derive(Clone, Copy)]
pub struct ShellInput<'a> {
    pub window: (f32, f32),
    pub ui_scale: UiScale,
    /// The faces to draw and measure with. Borrowed rather than owned for the same
    /// reason as everything else here, and travelling in the input rather than as a
    /// separate parameter so that no panel can measure a string with one face and
    /// draw it with another.
    pub fonts: &'a Faces,
    /// The poster service's read side (WX-5b): what the plan lane draws for
    /// each run. A shared borrow for the whole frame; requests go back through
    /// [`Shell::take_poster_requests`], never through this.
    pub posters: &'a crate::posters::PosterBoard,
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
    /// The caption effect drives' figures for the same frame — RMS, the
    /// trail-derived bass, beat phase, flux and the clock — so the effects
    /// pane's live meters read exactly what the overlay's resolver read
    /// (UX0-C14). Not derivable from `route_sources`: bass comes from the
    /// *trails*, which no route source carries.
    pub effect_inputs: musializer_core::project::caption_effects::EffectInputs,
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
    /// Egui editor dialog. Its modal input boundary protects legacy text fields.
    pub editor_open: bool,
    /// The right-hand dock: terminals and agents (WX-1). Not modal; the
    /// workspace lays itself out beside it.
    pub dock_open: bool,
    /// Whether a dock surface holds the keyboard this frame. Written by the
    /// composition root, read by [`Shell::text_entry_has_focus`].
    pub dock_keyboard: bool,
    /// The runs the plan lane drew this frame and wants posters for (WX-5b),
    /// drained by the composition root after the frame.
    pub(crate) poster_requests: Vec<crate::posters::PosterRequest>,
    /// `--ui-probe run-open=N`, delivered by the lane once a track is open.
    pub probe_run_open: Option<usize>,
    /// `--ui-probe key=escape`: one Escape for the next keyboard read, set by
    /// `main.rs` on the frame the probe presses and taken by that read.
    pub probe_escape: bool,
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
    /// Which encoder an export uses (2026-08-08, `--encoder`).
    ///
    /// Session state rather than project state: it describes the machine, not
    /// the piece. A `.musi` carried to a box with no NVIDIA card must not insist
    /// on `h264_nvenc`.
    pub video_encoder: musializer_runtime::process::ffmpeg::VideoEncoder,
    pub notices: NoticeQueue,
    pub timeline: TimelineView,
    /// One owner for every timeline drag. A scene boundary and the waveform
    /// scrubber must never both interpret the same press.
    pub(crate) timeline_gesture: Option<TimelineGesture>,
    /// The most recent drag position. Seeking is deliberately deferred until
    /// release: repeatedly flushing and refilling a decoder while the pointer
    /// moves is both audible and needlessly expensive.
    scrub_target_seconds: Option<f64>,
    /// Release-frame preview retained until the composition root applies the
    /// emitted seek after drawing. Prevents a one-frame snap back to decoder time.
    scrub_release_preview_seconds: Option<f64>,
    /// Whether playback should resume after the release-time seek.
    scrub_restore_playing: bool,
    /// Origin of a middle-button timeline pan. The view is recomputed from this
    /// fixed pair each frame, so pointer sampling jitter cannot accumulate.
    timeline_pan: Option<TimelinePan>,
    /// A manual pan deliberately detaches the view from playback-follow until
    /// the user asks to Follow again.
    pub(crate) timeline_manual_view: bool,
    /// The wheel gesture captured over any timed lane by
    /// [`Shell::request_timeline_zoom`] and applied at the start of the next
    /// frame, before any aligned lane draws.
    ///
    /// At most one claim per frame, so two lanes cannot compound one notch.
    timeline_zoom_pending: Option<TimelineWheel>,
    /// The merged manual/semantic event view the waveform lane draws markers
    /// from, cached exactly as the C caches it (`combined_scene_events`,
    /// `plug.c:1085-1113`): rebuilt only when the current track or either lane's
    /// revision changes, because the merge validates and sorts both lanes and a
    /// track may carry 2,048 events.
    timeline_events: musializer_core::scene::events::SceneEventMerge,
    /// What the markers drew last frame, for the `timeline:` report line.
    pub(crate) timeline_event_markers: EventMarkerReport,
    /// The cache key: `(current track index, manual revision, semantic revision)`.
    ///
    /// The track index is in the key for the reason the C puts
    /// `p->scene_events_track` in its own: two tracks can hold lanes at the same
    /// revisions, and without it a track swap would draw the previous track's
    /// markers over this track's waveform.
    timeline_events_key: Option<(usize, u64, u64)>,
    /// `--ui-probe wheel=NOTCHES`, waiting for the first frame to draw (LX2).
    pub probe_wheel: Option<f32>,
    /// `--ui-probe drop=PATH`, a synthesized file drop waiting for the first
    /// frame to draw (D1).
    ///
    /// **Invented for the reason `hover=` and `wheel=` were.** Xvfb has no
    /// drag-and-drop, so `dropped_files` — the whole of D1 — was a branch no
    /// capture could enter, and the three arms are the kind of dispatch that
    /// reads correctly in the source while sending everything to one of them.
    /// Consumed once, whatever the frame count.
    pub probe_drop: Option<PathBuf>,
    /// What [`Shell::dropped_files`] made of that path, for the report line.
    ///
    /// The dispatch is recorded rather than recomputed by the reporter, because a
    /// reporter that called [`classify_drop`] itself would print the same answer
    /// whether or not the drop ever reached `dropped_files` — which is precisely
    /// the "the gate is green and the control is dead" failure EX1 cost.
    pub probe_drop_dispatch: Option<(PathBuf, DropKind)>,
    /// The welcome screen's recent-project list (UX0-C06).
    ///
    /// Shell state rather than a [`ShellInput`] field, because it changes only
    /// when a project is opened or forgotten — threading a borrow through every
    /// frame to carry something that is almost always unchanged would make five
    /// other panels' call sites pay for one screen's list.
    pub recent: super::preferences::recent::RecentProjects,
    /// Wall clock in Unix seconds, for the recent list's "3 days ago".
    ///
    /// Supplied by the composition root rather than read here, so the shell keeps
    /// its no-side-effects property and a test can pin the age it renders.
    pub recent_now_unix: Option<i64>,
    /// The store was refused at startup, so the column says so instead of drawing
    /// an empty list.
    ///
    /// An unreadable history and no history at all are different facts, and a
    /// blank region is indistinguishable from a broken one.
    pub recent_unavailable: bool,
    /// Whether the app-owned state directory contains a recovery generation.
    pub recovery_available: bool,
    /// One frame of `--ui-probe middle-drag=`, set by the composition root.
    ///
    /// `None` on every other frame, so the real device drives the pan whenever
    /// the probe is not staging one — which is what keeps the injection from
    /// being a second code path the ordinary build never runs.
    pub probe_middle_drag_frame: Option<ProbeMiddleDrag>,
    /// `--ui-probe wheel-shift=1`: hold Shift for the probe's wheel notch (D4).
    ///
    /// A flag rather than a second notch key, because Shift is a *modifier* on
    /// the notch `wheel=` already delivers — two keys that both carried a count
    /// would let a spec ask for a zoom and a pan on one frame, which no hand can
    /// do.
    pub probe_wheel_shift: bool,
    /// `--ui-probe wheel-alt=1`: Tune's explicit value-edit modifier.
    pub probe_wheel_alt: bool,
    /// Whether Shift was held when this frame's wheel was read, so a notch over
    /// any timed lane pans rather than zooms.
    ///
    /// Latched in [`Self::begin_frame`] rather than read at each call site: the
    /// waveform strip, the scene-plan lane and the lyric cue lane all consult
    /// the same seam, and the third is drawn by a file this tranche does not own.
    wheel_pan_modifier: bool,
    /// The same value, held for exactly the frame that consumed it, so every
    /// lane in that frame is offered the notch and the frame after is not.
    probe_wheel_frame: Option<f32>,
    /// A drag in flight on the transport row's position bar (review 1.9,
    /// UX0-A09).
    ///
    /// Separate from [`Self::timeline_gesture`] because the two surfaces are
    /// disjoint and each has its own release path, but it obeys the same
    /// transactional rule: the seek is deferred to release, and
    /// [`Self::abandon_workspace_drags`] completes it if the toolbar stops being
    /// drawn mid-drag — which fullscreen does.
    transport_scrub: Option<TransportScrub>,
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
    pub(crate) scene_picker: PickerState,
    pub(crate) scene_picker_report: String,
    /// The export panel's clip window: which part of the track the next export
    /// covers (UX0-C01).
    ///
    /// **Session-only, and deliberately not in the `.musi` file.** A clip is a
    /// thing you are doing right now — "post the chorus" — not a property of the
    /// project, and persisting it would mean a reopened project silently
    /// rendering 30 seconds of a four-minute track. Whether it *should* become
    /// a durable per-track field is a schema question, recorded in
    /// `FEATURE_PARITY_PLAN.md` under UX0-C01 rather than decided here.
    ///
    /// On `Shell` for the same reason as every other field around it: it
    /// outlives a frame. `--render-window` seeds it at startup so the panel and
    /// the command line are one state.
    pub export_clip: musializer_core::timing::render_export::ClipSelection,
    /// Optional playhead time whose deterministic render replaces encoded
    /// frame zero of the next video export.
    ///
    /// Session-only for the same reason as [`Self::export_clip`]: this is a
    /// publishing choice for the render being made now, not authored project
    /// content. `None` preserves the ordinary export path byte-for-byte.
    pub export_share_frame_seconds: Option<f64>,
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
    /// The **AI settings** modal (tranche AP3). Application-modal inside the
    /// window, so it lives on `Shell` for the same reason every other
    /// frame-surviving surface does.
    pub assist_settings: super::assist_settings::AssistSettingsDialog,
    /// `sha256(OPENROUTER_API_KEY)[0..8]` when one was imported from the
    /// environment at startup, and nothing else — the key itself stays in
    /// `main.rs`'s `SessionCredentials`. Set once, so the dialog can say
    /// "session only" without holding a second copy of a credential.
    pub session_credential_fingerprint: Option<String>,
    /// The running feedback protocol, if `--protocol` or a drop loaded one
    /// (HX-2).
    ///
    /// On `Shell` because the keyboard, the marker rail and the question card
    /// all consult it every frame; every side effect its transitions ask for
    /// travels back to `main.rs` as a [`ShellCommand::Protocol`].
    pub protocol: Option<super::protocol::ProtocolSession>,
    /// A mouse cursor a panel asked for this frame, via [`Shell::request_cursor`].
    ///
    /// It exists because [`Shell::splitters`] sets the cursor unconditionally and
    /// runs *after* every panel, so a panel calling `set_mouse_cursor` itself had
    /// its choice overwritten one call later — silently, and only on the frames
    /// where the pointer was over a panel affordance, which is every frame that
    /// matters. Routing the request through one field makes the splitters the one
    /// owner of the cursor rather than the accidental last writer.
    ///
    /// Reset to `None` in [`Shell::begin_frame`]. A cursor is a statement about
    /// where the pointer is *now*, and the pointer moves between frames, so
    /// anything sticky here would keep a resize arrow on screen after the hand
    /// had left the handle.
    pointer_cursor: Option<raylib::consts::MouseCursor>,
    /// First frame on which fullscreen gained an attention state. The expanded
    /// label lives for 2.5 seconds, then the same state collapses to its dot.
    fullscreen_attention_since: Option<f64>,
    fullscreen_attention_token: &'static str,
    /// What the top bar drew in its last frame (WX-5c). Its type is
    /// `ui::top_bar`'s, so the top bar can grow its own state without editing
    /// this file.
    pub(crate) top_bar_state: super::top_bar::TopBarState,
    /// The rail's open flyout (WX-5c §1). `--ui-probe flyout=` sets it; until
    /// the rail is drawn nothing reads it but the `rail:` line.
    pub open_flyout: Option<Flyout>,
    /// The rail of the last workspace frame drawn, for the `rail:` line.
    /// `None` on the welcome screen, which has no rail, and in the fullscreen
    /// and editor frames, which hide it.
    pub(crate) last_rail: Option<musializer_core::ui::workspace_layout::WorkspaceRail>,
    /// Where the open flyout drew in the last frame, for the same line.
    pub(crate) last_flyout: Option<UiRect>,
    /// What the notice tray drew in the last frame, for the `notices:` line.
    notices_drawn: NoticeTrayReport,
    /// The developer readout for this frame, long and short forms, handed in
    /// by `main.rs` before [`Shell::draw`]. The shell draws it because only
    /// the shell knows what floats over the preview: drawn after the shell,
    /// it used to print across an open flyout and over the tooltip.
    pub hud_readout: Option<(String, String)>,
    /// What the transport row drew in the last frame, for the `transport:`
    /// line.
    transport_report: String,
    /// `--ui-probe scene-family=`, held against the plan's scene changes
    /// until a family row is pressed (see [`Self::browse_scene_family`]).
    pub(crate) probe_family: Option<SceneFamily>,
    /// `MUSIALIZER_COLUMN_LAYOUT=1` (WX-5d seam): the secondary panels draw
    /// in the right column. Set once by the composition root at startup from
    /// [`super::column::column_layout_from_env`]; see
    /// [`Self::placement`](super::column).
    pub column_layout: bool,
    /// What Look drew in its last frame, for the `tune layout:` line (WX-5d).
    /// Its type is `panels::tune`'s, so the Look & Assist builder grows it
    /// without editing this file.
    pub(crate) look_column: super::panels::tune::LookColumn,
    /// What Assist drew in its last frame, for the `assist column:` line
    /// (WX-5d). `panels::assist`'s type, for the same reason.
    pub(crate) assist_column: super::panels::assist::AssistColumn,
}

/// The colour a save state's word draws in, wherever it is drawn: the tracks
/// header today, the top bar from WX-5c on. One function so the two can never
/// disagree about which states ask for attention.
#[must_use]
pub(crate) fn save_state_tint(state: SaveState) -> Color {
    match state {
        SaveState::Failed => color::ui_danger(),
        SaveState::UnfiledWork | SaveState::Unsaved => color::ui_warning(),
        // Saved and "No file" are both calm states, and neither should pull
        // the eye off the work. The word carries the difference.
        SaveState::Saved | SaveState::NoProjectFile => color::ui_muted(),
    }
}

/// The rail's two flyouts (WX-5c §1), named as `--ui-probe flyout=` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Tracks,
    Scenes,
}

impl Flyout {
    /// `flyout=tracks|scenes|none`: the outer `None` is an unknown name, the
    /// inner one an explicit "closed".
    #[must_use]
    pub fn from_probe_name(name: &str) -> Option<Option<Self>> {
        match name {
            "tracks" => Some(Some(Flyout::Tracks)),
            "scenes" => Some(Some(Flyout::Scenes)),
            "none" => Some(None),
            _ => None,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Flyout::Tracks => "tracks",
            Flyout::Scenes => "scenes",
        }
    }
}

/// The top bar's mode tabs (WX-5c §4): **views onto existing state, never a
/// second copy of it.** A tab is selected when the state it names holds, and
/// pressing one goes through the same guards the transport's panel buttons
/// do. Declared here rather than in `ui::top_bar` because the state they read
/// is this file's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeTab {
    /// No bottom panel and the inspector closed.
    Direct,
    /// `UiPanel::Lyrics`.
    Lyrics,
    /// The Tune inspector open. It coexists with a bottom panel, so `Look`
    /// can be selected together with Lyrics, Export or Assist.
    Look,
    /// `UiPanel::Export`.
    Export,
    /// `UiPanel::Assist`.
    Assist,
}

impl ModeTab {
    pub const ALL: [ModeTab; 5] = [
        ModeTab::Direct,
        ModeTab::Lyrics,
        ModeTab::Look,
        ModeTab::Export,
        ModeTab::Assist,
    ];

    /// The token the `top bar:` line prints.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ModeTab::Direct => "direct",
            ModeTab::Lyrics => "lyrics",
            ModeTab::Look => "look",
            ModeTab::Export => "export",
            ModeTab::Assist => "assist",
        }
    }

    /// The word on the tab.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ModeTab::Direct => "Direct",
            ModeTab::Lyrics => "Lyrics",
            ModeTab::Look => "Look",
            ModeTab::Export => "Export",
            ModeTab::Assist => "Assist",
        }
    }
}

impl ModeTab {
    /// The column view this tab names (WX-5d): tabs are views onto one
    /// [`ColumnView`]. `Export` is `Render` until the switch flips and Export
    /// leaves the tabs for Render… (§3.5).
    #[must_use]
    #[allow(
        dead_code,
        reason = "WX-5d seam: the Frame builder's tab press routes through it"
    )]
    pub fn column_view(self) -> ColumnView {
        match self {
            ModeTab::Direct => ColumnView::Direct,
            ModeTab::Lyrics => ColumnView::Lyrics,
            ModeTab::Look => ColumnView::Look,
            ModeTab::Export => ColumnView::Render,
            ModeTab::Assist => ColumnView::Assist,
        }
    }
}

/// What the right column shows (WX-5d §2): one value, which the tabs, the
/// rail's column toggle and Render… are all views onto.
///
/// **In the seam it is derived, never stored**: [`Shell::column_view`] reads
/// it off today's state (`panel`, `inspector_open`, `dock_open`) and
/// [`Shell::set_column_view`] writes that state, so there is no second copy to
/// disagree with. The Frame builder folds `UiPanel` and the inspector flag
/// into it later. Persisted in `ui.json` as [`ColumnView::name`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnView {
    /// The column is closed; the preview has its width.
    Closed,
    /// The dock: chats and terminals.
    Direct,
    Lyrics,
    /// The Tune inspector.
    Look,
    Assist,
    /// Render settings and the commit that renders (was the Export panel).
    Render,
}

impl ColumnView {
    pub const ALL: [ColumnView; 6] = [
        ColumnView::Closed,
        ColumnView::Direct,
        ColumnView::Lyrics,
        ColumnView::Look,
        ColumnView::Assist,
        ColumnView::Render,
    ];

    /// The token `column=` takes, the `column:` line prints and `ui.json`
    /// stores. Stable: a rename breaks every file that stored the old one.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ColumnView::Closed => "closed",
            ColumnView::Direct => "direct",
            ColumnView::Lyrics => "lyrics",
            ColumnView::Look => "look",
            ColumnView::Assist => "assist",
            ColumnView::Render => "render",
        }
    }

    /// [`Self::name`]'s inverse; `None` for anything else.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitKind {
    Inspector,
    Timeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimelineGesture {
    Scrub,
    Pan,
    SceneBoundary,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TimelinePan {
    origin_x: f32,
    origin_start_seconds: f64,
}

/// What one wheel notch over a timed lane asked the shared view to do.
///
/// Two variants rather than a factor and a delta both being `Option`, because a
/// notch is exactly one gesture: it zooms, or — with Shift held — it pans, and
/// never both. The old shape was a bare `Option<(f64, f64)>` whose two `f64`s
/// were "factor" and "anchor"; adding pan to it would have meant a third number
/// whose meaning depended on the other two, which is the `Option`-as-disguise
/// shape this repository has already paid for once in `beat_tracker`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TimelineWheel {
    /// Multiply the span by `factor`, holding `anchor_seconds` under the pointer.
    Zoom { factor: f64, anchor_seconds: f64 },
    /// Slide the window by `delta_seconds` without changing the span.
    Pan { delta_seconds: f64 },
}

/// One staged frame of a probe middle-drag: where the pointer is and what the
/// middle button is doing.
///
/// Carries the position as well as the button because a pan is measured from
/// the press point to the current pointer, so a probe that moved only the button
/// would drive a pan of zero and look exactly like a broken gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeMiddleDrag {
    pub x: f32,
    pub y: f32,
    pub pressed: bool,
    pub down: bool,
}

/// The C's marker head radius (`DrawCircleV(..., 5.0f, ...)`, `plug.c:3099`).
const MARKER_HEAD_RADIUS: f32 = 5.0;

/// How thick the semantic marker's ring is.
///
/// 2 px of a 5 px radius: thin enough that the two heads are obviously the same
/// size and different shapes, thick enough to survive the 150 % scale where a
/// 1 px ring would alias into a smudge.
const MARKER_RING_THICKNESS: f32 = 2.0;

/// What the event markers drew, for the probe report.
///
/// A count per lane rather than a total, because a total cannot tell the case
/// this tranche exists to make visible — a track with only semantic markers and
/// a track with only manual ones produce the same total and the same picture at
/// a glance.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EventMarkerReport {
    pub manual: usize,
    pub semantic: usize,
    /// Events whose timestamp lies outside `[0, duration]`.
    pub off_track: usize,
    /// Events inside the track but outside the visible window.
    pub off_screen: usize,
    /// The tooltip text of the marker under the pointer, if any.
    pub hovered: Option<String>,
}

/// Whether either Shift key is down, which turns a wheel notch over a timed lane
/// into a pan (D4).
///
/// Both keys, because a modifier bound to only one of them is a control that
/// works for right-handed mice and not left-handed ones.
fn shift_held(d: &RaylibDrawHandle<'_>) -> bool {
    use raylib::consts::KeyboardKey::{KEY_LEFT_SHIFT, KEY_RIGHT_SHIFT};
    d.is_key_down(KEY_LEFT_SHIFT) || d.is_key_down(KEY_RIGHT_SHIFT)
}

/// How far one Shift-wheel notch slides the window, as a fraction of the visible
/// span.
///
/// A fraction rather than a number of seconds, so the gesture keeps the same
/// *feel* at every zoom: at whole-track it is a coarse jump and at 40x it is a
/// fine nudge, which is what a user zoomed in to inspect one beat actually wants.
/// A fixed second count would be unusably coarse at high zoom and unusably slow
/// at low zoom — the same argument the 1.2x-per-notch zoom factor makes
/// multiplicatively.
///
/// 0.15 leaves 85 % of the previous window on screen, so the eye can carry a
/// feature across the step rather than re-finding it.
pub(crate) const TIMELINE_WHEEL_PAN_FRACTION: f64 = 0.15;

/// The one coupling `Shell::timeline_group_chrome` cannot check at runtime.
///
/// The cue lane is drawn by `panels::lyrics`, which spends its own `LANE_GAP`
/// between the waveform and the lane and cannot spend more — its
/// `LYRIC_EDITOR_TIMELINE_CHROME` assertion forbids the band growing. The seam
/// this shell paints into that gap is sized from [`metric::LANE_GAP`]. If the
/// two ever disagree the seam is drawn over the cue lane or leaves a stripe of
/// bare panel, and neither shows up as a failure anywhere — so it fails here,
/// at compile time, instead.
const _: () = assert!(lyrics::LANE_GAP == metric::LANE_GAP);

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayheadGeometry {
    line_x: f32,
    handle_x: Option<f32>,
}

fn playhead_geometry(
    lane: UiRect,
    raw_x: f32,
    line_width: f32,
    handle: bool,
) -> Option<PlayheadGeometry> {
    if lane.is_empty() || !raw_x.is_finite() || raw_x < lane.x || raw_x > lane.x + lane.width {
        return None;
    }
    let half_line = line_width.max(1.0) * 0.5;
    let line_low = lane.x + half_line;
    let line_high = lane.x + lane.width - half_line;
    let snapped_low = line_low.ceil();
    let snapped_high = line_high.floor();
    if snapped_low > snapped_high {
        return None;
    }
    let handle_x = if handle && lane.width >= 10.0 {
        Some(raw_x.clamp(lane.x + 5.0, lane.x + lane.width - 5.0).round())
    } else {
        None
    };
    Some(PlayheadGeometry {
        // Integer logical pixels keep the high-contrast marker stable while the
        // waveform scrolls fractionally beneath it.
        line_x: raw_x.round().clamp(snapped_low, snapped_high),
        handle_x,
    })
}

/// Draw one bounded, pixel-snapped playhead on any timed lane.
///
/// The line keeps the exact edge position as closely as its stroke permits. The
/// optional triangular handle moves inward near either edge, so the end-of-track
/// marker never paints outside the PCM element.
pub(crate) fn draw_timeline_playhead<D: RaylibDraw>(
    d: &mut D,
    view: TimelineView,
    lane: UiRect,
    seconds: f64,
    line_width: f32,
    handle: bool,
) {
    let raw_x = view.x_at(seconds, f64::from(lane.x), f64::from(lane.width)) as f32;
    let Some(geometry) = playhead_geometry(lane, raw_x, line_width, handle) else {
        return;
    };
    d.draw_line_ex(
        Vector2::new(geometry.line_x, lane.y),
        Vector2::new(geometry.line_x, lane.y + lane.height),
        line_width,
        color::accent(),
    );
    if let Some(handle_x) = geometry.handle_x {
        d.draw_triangle(
            Vector2::new(handle_x - 5.0, lane.y),
            Vector2::new(handle_x, lane.y + 7.0),
            Vector2::new(handle_x + 5.0, lane.y),
            color::accent(),
        );
    }
}

/// A drag in flight on the transport row's position bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransportScrub {
    /// Where the hand is, in track seconds. Emitted on release, not per frame.
    target_seconds: f64,
    /// Whether playback was running when the drag began.
    restore_playing: bool,
}

/// One frame of the position bar's gesture, without raylib.
///
/// A struct rather than eight arguments because the whole reason it exists is to
/// be *called from a test*: a headless run has no pointer, so a click at a known
/// fraction of a known track is the only way to assert that the bar seeks at all
/// — and asserting that it seeks to the right second is not something a capture
/// could do even with a pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransportScrubInput {
    /// The seek groove, in window coordinates.
    pub track: UiRect,
    pub pointer_x: f32,
    /// Whether the pointer is inside the bar's press area this frame.
    pub hovered: bool,
    /// The press edge.
    pub pressed: bool,
    /// Whether the button is still held.
    pub down: bool,
    pub playing: bool,
    pub duration_seconds: f64,
    /// False for a stream the decoder cannot seek in. The bar then draws as a
    /// position readout and claims nothing.
    pub seekable: bool,
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
    /// A Tune inspector value chip being typed into (UX0-B09).
    TuneValue,
    /// The dock's terminal (WX-2). A shell is useless if Space pauses the
    /// song, so while it holds the keyboard every global stands down.
    Dock,
}

impl TextEntrySurface {
    pub(crate) const ALL: [Self; 4] =
        [Self::LyricCue, Self::FontQuery, Self::TuneValue, Self::Dock];
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
    /// `S`, which only does anything chorded with Control.
    ///
    /// Not the oracle's — the frozen C has no save shortcut at all. It is here
    /// because review 1.12 (UX0-A12) found a reachable state with *no* route to
    /// saving: with the tracks panel collapsed, the four action buttons were the
    /// only ones, and nothing on screen said so. The collapsed strip is the real
    /// fix; this is the second route, and the strip's tooltip names it.
    pub save: bool,
    /// `1`-`4`, the protocol answer row (HX-2). Consulted only while a
    /// protocol session is live, so the digits stay free everywhere else.
    pub answer_digit: Option<u8>,
    /// `R`, `B`, `N` — replay, other look, next item. Same rule as the digits.
    pub protocol_replay: bool,
    pub protocol_flip: bool,
    pub protocol_next: bool,
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
            save: d.is_key_pressed(Key::KEY_S),
            answer_digit: [
                (Key::KEY_ONE, Key::KEY_KP_1, 1u8),
                (Key::KEY_TWO, Key::KEY_KP_2, 2),
                (Key::KEY_THREE, Key::KEY_KP_3, 3),
                (Key::KEY_FOUR, Key::KEY_KP_4, 4),
            ]
            .into_iter()
            .find(|(key, pad, _)| d.is_key_pressed(*key) || d.is_key_pressed(*pad))
            .map(|(_, _, digit)| digit),
            protocol_replay: d.is_key_pressed(Key::KEY_R),
            protocol_flip: d.is_key_pressed(Key::KEY_B),
            protocol_next: d.is_key_pressed(Key::KEY_N),
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
            editor_open: false,
            dock_open: ui_preferences.dock_open,
            dock_keyboard: false,
            poster_requests: Vec::new(),
            probe_run_open: None,
            probe_escape: false,
            widgets: Widgets::new(),
            notices: NoticeQueue::default(),
            inspector_open: false,
            panel: UiPanel::None,
            fullscreen: false,
            video_encoder: musializer_runtime::process::ffmpeg::VideoEncoder::default(),
            hud_visible: false,
            timeline: TimelineView::new(0.0),
            timeline_gesture: None,
            scrub_target_seconds: None,
            scrub_release_preview_seconds: None,
            scrub_restore_playing: false,
            timeline_pan: None,
            timeline_manual_view: false,
            timeline_zoom_pending: None,
            timeline_events: musializer_core::scene::events::SceneEventMerge::new(),
            timeline_events_key: None,
            timeline_event_markers: EventMarkerReport::default(),
            probe_wheel: None,
            probe_wheel_shift: false,
            probe_wheel_alt: false,
            probe_middle_drag_frame: None,
            wheel_pan_modifier: false,
            probe_wheel_frame: None,
            probe_drop: None,
            probe_drop_dispatch: None,
            recent: super::preferences::recent::RecentProjects::default(),
            recent_now_unix: None,
            recent_unavailable: false,
            recovery_available: false,
            transport_scrub: None,
            track_scroll: ScrollState::new(),
            route_editor: super::panels::tune::EditorHost::default(),
            font_browser: super::panels::fonts::FontBrowser::new(),
            lyrics: super::panels::lyrics::LyricEditor::new(),
            scene_lane: super::panels::scene_timeline::SceneLaneEditor::default(),
            scene_picker: PickerState::default(),
            scene_picker_report: "not drawn".into(),
            export_clip: musializer_core::timing::render_export::ClipSelection::full_track(),
            export_share_frame_seconds: None,
            ui_preferences,
            ui_scale_override: None,
            split_drag: None,
            last_split_press: None,
            assist_settings: super::assist_settings::AssistSettingsDialog::new(),
            session_credential_fingerprint: None,
            protocol: None,
            pointer_cursor: None,
            fullscreen_attention_since: None,
            fullscreen_attention_token: "none",
            top_bar_state: super::top_bar::TopBarState::default(),
            open_flyout: None,
            last_rail: None,
            last_flyout: None,
            notices_drawn: NoticeTrayReport::default(),
            hud_readout: None,
            transport_report: "none".to_owned(),
            probe_family: None,
            column_layout: false,
            look_column: super::panels::tune::LookColumn::default(),
            assist_column: super::panels::assist::AssistColumn::default(),
        }
    }

    pub fn appearance(&self) -> super::theme::UiTheme {
        self.ui_preferences.theme
    }
    pub fn set_appearance(&mut self, theme: super::theme::UiTheme) -> ShellCommand {
        self.ui_preferences.theme = theme;
        super::theme::set_current(theme);
        ShellCommand::SaveUiPreferences(self.ui_preferences)
    }

    pub fn set_ui_scale_override(&mut self, preference: Option<UiScalePreference>) {
        self.ui_scale_override = preference;
    }

    /// Starts a fresh whole-track view and clears every transient gesture tied
    /// to the previous track. Used by the composition root instead of reaching
    /// into [`Self::timeline`] so a manual-pan follow suspension cannot leak
    /// across track changes.
    pub fn reset_timeline(&mut self, duration_seconds: f64) {
        self.timeline.reset(duration_seconds);
        self.timeline_gesture = None;
        self.scrub_target_seconds = None;
        self.scrub_release_preview_seconds = None;
        self.scrub_restore_playing = false;
        self.timeline_pan = None;
        self.timeline_manual_view = false;
        self.timeline_zoom_pending = None;
    }

    /// The one playhead every timed lane draws. During a transactional scrub the
    /// decoder has not moved yet, so the preview target is the honest position.
    pub(crate) fn timeline_playhead_seconds(&self, actual_seconds: f64) -> f64 {
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.scrub_target_seconds
                .or(self.scrub_release_preview_seconds)
                .unwrap_or(actual_seconds)
        } else {
            self.scrub_release_preview_seconds.unwrap_or(actual_seconds)
        }
    }

    #[must_use]
    pub fn ui_scale_preference(&self) -> UiScalePreference {
        self.ui_scale_override.unwrap_or(self.ui_preferences.scale)
    }

    /// Pushes a notice, dropping the result: the overflow policy in
    /// [`NoticeQueue`] is the right answer and there is nothing better for a
    /// caller to do with a refusal.
    ///
    /// **The lifetime comes from the severity** ([`Severity::dwell`]), not from
    /// this function and not from the call site. Every one of the ~51 callers
    /// used to get `persistent: false, 6.0` — including the ones reporting an
    /// export that had failed — while [`NoticeQueue`] supported persistence
    /// correctly the whole time (review 1.11, UX0-A11). Fixing it here rather
    /// than at the call sites is the point: a policy spread over fifty-one
    /// literals is a policy that drifts back.
    pub fn notify(&mut self, severity: Severity, title: &str, detail: &str) {
        let dwell = severity.dwell();
        let _ = self.notices.push(&NoticeSpec {
            severity,
            persistent: dwell.persistent,
            duration_seconds: dwell.duration_seconds,
            title: Some(title),
            detail,
            path: "",
        });
    }

    /// Whether the track in `slot` owns an uncommitted editor draft, and so must
    /// not be autosaved yet (C4).
    ///
    /// **Per track rather than global, and that distinction is the requirement.**
    /// `autosave_is_due` has always taken an `editor_dirty` flag, but `main.rs`
    /// passed a hard-coded `false`, so a half-typed cue never suppressed anything.
    /// Wiring the *global* "is any editor dirty" query in instead would have
    /// over-corrected the other way: typing in track A's lyric form would freeze
    /// autosave for every other open track, which is the same class of bug —
    /// silently not writing work the user believes is being written.
    ///
    /// Both halves ask the same question of the same slot: a dirty lyric draft
    /// whose owner is this track, or a route edit that is open on this track and
    /// dirty.
    #[must_use]
    pub(crate) fn editor_draft_blocks_autosave(&self, workspace: &Workspace, slot: usize) -> bool {
        let lyric = self.lyrics.draft_owner() == Some(slot)
            && workspace
                .get(slot)
                .is_some_and(|owner| self.lyrics.has_unsaved_draft(&owner.lyrics));
        let route =
            self.route_editor_open_for_active_track(Some(slot)) && self.route_edit_is_dirty();
        lyric || route
    }

    /// Which route to saving this frame's chrome puts on screen (review 1.12,
    /// UX0-A12).
    ///
    /// Never absent outside fullscreen: that is the whole point, and
    /// `a_save_route_survives_every_panel_and_window_configuration` sweeps it.
    #[must_use]
    pub(crate) fn save_affordance(&self, frame: &WorkspaceFrame) -> SaveAffordance {
        if self.fullscreen {
            return SaveAffordance::Fullscreen;
        }
        // The rail's Tracks icon is one press from Save, and the top bar
        // carries the state word (WX-5c). Both are drawn in every workspace
        // frame the rail is seated in; the rail's own layout drops Tracks
        // last, so a frame with a rail and no Tracks icon is a rail too short
        // for one 28 px box, which no permitted window produces.
        if !frame.top_bar.is_empty()
            && frame
                .rail_layout
                .is_some_and(|rail| rail.icon(RailIcon::Tracks).is_some())
        {
            return SaveAffordance::TopBar;
        }
        // The editor frame, which hides the workspace's chrome while its own
        // modal is up; Ctrl+S still saves.
        SaveAffordance::Fullscreen
    }

    /// One line naming what this frame's chrome actually carries.
    ///
    /// Written for the same reason `Faces::describe` is: a capture proves a
    /// surface drew *something*, and the failures this repository keeps finding
    /// are surfaces that drew a plausible substitute. "save CollapsedStrip" in a
    /// probe report is what makes the collapsed state reviewable at all — the
    /// full panel and its stand-in photograph as two different sidebars, and
    /// neither picture says which one was meant.
    #[must_use]
    pub fn describe(&self, frame: &WorkspaceFrame) -> String {
        format!(
            "panel {:?}  rail {}  save {:?}  readout {}",
            self.panel,
            if frame.rail.is_empty() {
                "none"
            } else {
                self.open_flyout.map_or("closed", Flyout::name)
            },
            self.save_affordance(frame),
            if self.hud_visible { "on" } else { "off" }
        )
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
        //
        // Since WX-5c no state spends it: the row's controls are chips on the
        // lane head, which took the scene section's controls row
        // (`docs/WX5C_DESIGN.md` §3), so the band is 66 px shorter with no
        // panel or with Export open, and the preview has those pixels. The
        // band is bottom-anchored, so everything below the old row keeps its
        // window y — every band probe in the gate keeps its point.
        let panel_height = match self.panel {
            UiPanel::None | UiPanel::Tune => DEFAULT_TIMELINE_HEIGHT,
            UiPanel::Export => WorkspaceFrame::export_timeline_height(window_height),
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
                UiPanel::None | UiPanel::Tune => 150.0,
                // Derived from the export panel's own layout constants: the old
                // bare 260.0 undershot what the panel consumes above its
                // boundary, and a persisted shorter split blanked it across
                // restarts (review 1.4, UX0-A04).
                UiPanel::Export => {
                    super::panels::export::EXPORT_MIN_BAND_HEIGHT
                        - super::panels::scene_timeline::SCENE_SECTION_HEIGHT
                }
                // Asked, not asserted (LX1-d follow-up). This was the literal
                // `381.0` with a comment adding up five constants, and all five
                // moved when the lane grew and the zoom row moved below it. The
                // panel owns those numbers, so the panel is what gets asked.
                UiPanel::Lyrics => self.lyrics.minimum_band_height(window.1),
                // Assist has no scrolling body, so its measured content remains the
                // floor; resizing can give it room, never clip an action.
                UiPanel::Assist => automatic - super::panels::scene_timeline::SCENE_SECTION_HEIGHT,
            };
        let maximum = (window.1 - metric::HUD_BUTTON_SIZE - TOP_BAR_HEIGHT - 150.0).max(minimum);
        requested.clamp(minimum, maximum)
    }

    /// Draws one frame of chrome and returns what the user asked for.
    ///
    /// The preview rectangle is returned alongside so the caller can draw the
    /// scene into it. Chrome is drawn *after* the scene, so the caller's order is
    /// `layout` → draw scene → `draw`.
    #[must_use]
    pub fn layout(&self, input: &ShellInput<'_>) -> WorkspaceFrame {
        self.frame_for(input.window, input.workspace)
    }

    /// [`Self::layout`] for a caller that has the window and the workspace but no
    /// [`ShellInput`] — which is every caller that cannot open a window, since
    /// `ShellInput` borrows the font bank.
    ///
    /// One owner for the frame, deliberately. The alternative was for the report
    /// and the tests to rebuild it from the same five arguments, and a frame
    /// rebuilt without the user's split overrides would disagree with the drawn
    /// one about `tracks_mode` — which is exactly the state review 1.12 is about.
    #[must_use]
    pub fn frame_for(&self, window: (f32, f32), workspace: &Workspace) -> WorkspaceFrame {
        if self.editor_open && !self.assist_settings.is_open() {
            return WorkspaceFrame::editor(window);
        }
        if self.fullscreen {
            return WorkspaceFrame::fullscreen(window.0, window.1, true);
        }
        let timeline = self.resolved_timeline_height(window, workspace);
        // `ui_preferences.sidebar_width` is still read from `ui.json`, so a
        // file written before WX-5c loads, and nothing lays out by it: the
        // rail has one width.
        let overrides = LayoutOverrides {
            inspector_width: self.ui_preferences.inspector_width,
        };
        WorkspaceFrame::layout_with_overrides(
            window.0,
            window.1,
            self.inspector_open,
            timeline,
            overrides,
        )
    }

    /// [`Self::describe`] for the slice report, which has the window and the
    /// workspace rather than a laid-out frame.
    #[must_use]
    pub fn describe_workspace(&self, window: (f32, f32), workspace: &Workspace) -> String {
        self.describe(&self.frame_for(window, workspace))
    }

    /// Everything that has to happen before a frame's first widget or keypress.
    ///
    /// One call rather than each subsystem being poked from each screen: both
    /// [`Shell::draw`] and [`Shell::draw_welcome`] need it, and per-frame state
    /// that only one of them resets is how a stale flag survives (UX0-A02,
    /// UX0-A06).
    fn begin_frame(&mut self, ui_scale: UiScale, wheel_pan_modifier: bool) {
        self.widgets.begin_frame(ui_scale);
        self.font_browser.begin_frame();
        // Read once per frame, for the same reason `probe_wheel_frame` is taken
        // once per frame: three lanes ask for the wheel and all three must agree
        // about what the notch meant. Reading the key at each call site would
        // also make it unreachable from the lyric cue lane, whose file this
        // tranche does not own.
        self.wheel_pan_modifier = wheel_pan_modifier || self.probe_wheel_shift;
        self.scrub_release_preview_seconds = None;
        // A cursor request is only ever true of the frame that made it.
        self.pointer_cursor = None;
        // `--ui-probe wheel=` is delivered on exactly one frame, and taking it
        // here rather than at the read is what makes that true: the strip and
        // the lyric lane both ask for the wheel, the strip asks first, and a
        // `take` at the read would let it swallow a notch aimed at the lane.
        // Every caller in this frame sees the same value, which is what the
        // device does, and the frame after this one sees `None`.
        self.probe_wheel_frame = self.probe_wheel.take();
    }

    /// The wheel this frame, honouring `--ui-probe wheel=`.
    ///
    /// A headless run has no wheel, exactly as it has no pointer, so "the wheel
    /// zooms from the lane you are aiming in" was a binding no capture could
    /// reach — and the three timed lanes are drawn by three modules against
    /// three rectangles, which is where a region test is wrong in a way that
    /// reads correctly in the source.
    /// The span every timed lane is drawing, for the probe report (LX2).
    ///
    /// The zoom readout says this on screen, but only as typeset text a capture
    /// cannot read back. The wheel's whole observable effect is this view, and
    /// the three lanes now claiming it are drawn by three modules against three
    /// rectangles — the case where a region test is wrong about which lane it
    /// names looks exactly like the case where it is right, in a picture.
    #[must_use]
    /// This frame's poster requests, handed to the service after the frame.
    pub fn take_poster_requests(&mut self) -> Vec<crate::posters::PosterRequest> {
        std::mem::take(&mut self.poster_requests)
    }

    /// The `scene runs:` report line (WX-5b): what the plan lane drew in its
    /// last frame, as runs, and the view it drew them in. Read from that frame
    /// rather than from current state, because the report is printed at
    /// shutdown, after the view has been let go.
    #[must_use]
    pub fn describe_runs(&self, workspace: &Workspace) -> String {
        if workspace.current().is_none() {
            return "none".to_owned();
        }
        self.scene_lane.describe()
    }

    pub fn describe_timeline(&self, duration_seconds: f64) -> String {
        let zoom = if self.timeline.span_seconds > 0.0 {
            duration_seconds / self.timeline.span_seconds
        } else {
            0.0
        };
        let markers = &self.timeline_event_markers;
        format!(
            "{zoom:.3}x  {:.3}..{:.3}  of {duration_seconds:.3}{}  \
             gesture={}  \
             markers=manual:{} semantic:{} off-screen:{} off-track:{}{}",
            self.timeline.start_seconds,
            self.timeline.start_seconds + self.timeline.span_seconds,
            if self.timeline_manual_view {
                "  free-view"
            } else {
                ""
            },
            // A stranded pointer claim is the failure mode a drag probe exists to
            // catch, and it is *invisible* in a picture: the view stays exactly
            // where the hand left it either way, and the next click is what
            // behaves strangely. So the release has to be reported rather than
            // photographed.
            match self.timeline_gesture {
                None => "none",
                Some(TimelineGesture::Scrub) => "scrub",
                Some(TimelineGesture::Pan) => "pan",
                Some(TimelineGesture::SceneBoundary) => "scene-boundary",
            },
            // Per lane, not a total. A total cannot distinguish a track whose
            // markers are all model-derived from one whose markers are all
            // hand-placed, and telling those apart is the whole point of drawing
            // them differently — so a gate asserting a total would pass on a
            // build that had silently collapsed the two lanes into one.
            markers.manual,
            markers.semantic,
            // Counted rather than merely skipped, because "the marker is not on
            // screen" and "the marker was never built" produce the same empty
            // lane. This is the off-screen boundary case in a form a capture can
            // read back.
            markers.off_screen,
            markers.off_track,
            markers
                .hovered
                .as_ref()
                .map_or(String::new(), |text| format!("  hover=[{text}]"))
        )
    }

    pub(crate) fn wheel_delta(&self, d: &RaylibDrawHandle<'_>) -> f32 {
        self.probe_wheel_frame
            .unwrap_or_else(|| d.get_mouse_wheel_move())
    }

    /// Asks for a mouse cursor over whatever the caller is drawing, honoured by
    /// [`Self::splitters`] once every panel has had its turn.
    ///
    /// **Last request of the frame wins**, and the reasoning is worth writing
    /// down because the obvious argument for the opposite rule does not hold
    /// here. Panels are *not* drawn back-to-front: `draw` walks them in layout
    /// order over disjoint regions of the workspace, so "first to claim the
    /// pointer position" says nothing about which one owns the pixel — under a
    /// pointer inside the sidebar, the timeline drawing later is not the timeline
    /// drawing on top. Two requests in one frame therefore already mean two
    /// surfaces overlap, and where surfaces overlap this shell is a plain
    /// painter's algorithm: the last one drawn is the one the user is pointing
    /// at. So the last request is the honest answer.
    ///
    /// Note this is the opposite of the widget bank's press rule, where the
    /// *first* widget to see a press claims it (which is how the modal blocker
    /// works — it is drawn first on purpose). The asymmetry is not an oversight:
    /// a press is consumed and must go to exactly one claimant, while a cursor is
    /// a property of the topmost pixel and has no notion of being used up.
    ///
    /// An active or hovered workspace splitter still outranks every request. Its
    /// hit strip is drawn over everything, and once a drag is in flight the
    /// cursor must not change just because the pointer crossed a panel.
    pub(crate) fn request_cursor(&mut self, cursor: raylib::consts::MouseCursor) {
        self.pointer_cursor = Some(cursor);
    }

    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
    ) -> Vec<ShellCommand> {
        let mut commands = Vec::new();
        self.begin_frame(input.ui_scale, shift_held(d));

        // The AI settings modal is application-modal *inside* the window
        // (tranche AP3). Blocking falls out of the oracle's own claim rule
        // rather than a new mechanism: a press is claimed by the first widget to
        // see it, so a full-window blocker drawn into this bank before any panel
        // means nothing underneath can be pressed. The dialog draws its controls
        // into a bank of its own, which this claim cannot reach. See
        // `ui/assist_settings.rs`'s module comment.
        let modal = self.assist_settings.is_open() || self.editor_open;
        if modal {
            self.widgets.button(
                d,
                widgets::widget_id(super::assist_settings::MODAL_BLOCK_NAMESPACE, 0),
                UiRect::new(0.0, 0.0, input.window.0, input.window.1),
            );
        }

        if !modal {
            self.dropped_files(d, &mut commands);
            self.keyboard(d, input, &mut commands);
        }

        // WX-5c: an open flyout floats over the preview, the transport and the
        // band, and is modal for presses. A press outside it closes it; one
        // inside must not reach anything underneath. Everything drawn under
        // it gets a copy of this frame's input whose pointer is not there
        // (`UiScale::occluded`), and the widget bank reads the same scale, so
        // a press on a tile cannot also start a scrub on the lane below it.
        let flyout = if modal {
            None
        } else {
            self.flyout_begin(d, frame)
        };
        // The picker reports what it drew this frame, and it draws only in
        // the scenes flyout; a stale line would read as a picker on screen.
        self.scene_picker_report = "not drawn".to_owned();
        let occluded = ShellInput {
            ui_scale: input.ui_scale.occluded(flyout),
            ..*input
        };
        let under = &occluded;
        let pointer_scale = self.widgets.set_ui_scale(under.ui_scale);

        // The toolbar runs first because its band decides whether the timecode
        // fits beside the transport buttons, and the timeline is where it goes
        // when it does not. The regions are disjoint, so drawing it first costs
        // nothing — and one owner of that decision is the whole point.
        let toolbar = self.toolbar(d, frame, under, &mut commands);
        self.last_rail = frame.rail_layout;
        self.last_flyout = None;

        if !self.fullscreen {
            // WX-5c: the top bar and the rail. Both rects are empty in the
            // fullscreen and editor frames.
            self.top_bar(d, under, frame.top_bar, &mut commands);
            self.rail(d, frame, under, &mut commands);
            // Not drawn under the modal, and not for tidiness: the lyrics cue
            // field reads raylib's keyboard directly rather than through the
            // widget bank (`ui/text_input.rs`), so a key typed into the modal's
            // masked field would also land in a cue — a credential in a `.musi`
            // file.
            if !modal {
                self.timeline_strip(d, frame, under, toolbar, &mut commands);
            }
            if self.inspector_open && !self.editor_open {
                let placement = self.placement();
                self.inspector(d, frame, under, &mut commands, placement);
            }
            if !self.editor_open {
                self.splitters(d, frame, under, &mut commands);
            }
        }
        self.widgets.set_ui_scale(pointer_scale);
        // The protocol question card sits over the preview's bottom edge, and
        // the notice tray stacks *above* it rather than over it — the first
        // smoke capture had the session-start notice covering the question
        // text. Drawn in fullscreen too: a listening session is exactly when
        // the panels are hidden.
        let mut notice_region = frame.preview;
        // An open flyout covers the preview's left edge, which is where the
        // tray stacks; the tray moves right of it rather than under it.
        if let Some(rect) = flyout {
            let right = (rect.x + rect.width).min(notice_region.x + notice_region.width);
            notice_region.width = (notice_region.x + notice_region.width - right).max(0.0);
            notice_region.x = right;
        }
        // The developer readout, under the notices, the flyout and the
        // tooltip, and moved right of an open flyout so it stays readable
        // rather than being half covered.
        self.hud_readout_line(d, input, frame.preview, notice_region);
        if let Some(session) = &self.protocol {
            if !modal {
                if let Some(card_top) = super::protocol::draw_card(
                    d,
                    input.fonts.ui(),
                    session,
                    frame.preview,
                    input.playing,
                ) {
                    notice_region.height = (card_top - notice_region.y).max(0.0);
                }
            }
        }
        // The tray also starts past the flyout's shadow: a card laid on that
        // gradient reads as dimmed on its left edge, half under the flyout.
        if let Some(rect) = flyout {
            let shadow_end = (rect.x + rect.width + super::rail::FLYOUT_SHADOW)
                .min(notice_region.x + notice_region.width);
            if shadow_end > notice_region.x {
                notice_region.width = (notice_region.x + notice_region.width - shadow_end).max(0.0);
                notice_region.x = shadow_end;
            }
        }
        self.notice_tray(d, input.fonts.ui(), input.ui_scale.value(), notice_region);
        self.fullscreen_attention(d, input);
        // Over everything above, under the AI settings modal and the tooltip.
        if flyout.is_some() {
            self.flyout(d, frame, input, &mut commands);
        }

        if self.assist_settings.is_open() {
            // AP3-R S11: the one fact the dialog cannot read off disk. It states
            // that routing changes apply to the next job (§5 invariant 3), and
            // without this it could never say whether there is a current one.
            self.assist_settings
                .set_job_running(input.workspace.assist.is_active());
            self.assist_settings
                .draw(d, input.fonts, input.window, input.ui_scale);
        }

        // Last, and deliberately so: a tooltip belongs above everything, and the
        // toolbar that owns most of them is the *first* thing drawn. Requested
        // where the control is and drawn here is the only ordering that works.
        // Suppressed under the modal, where a tip from a control the user cannot
        // reach would be drawn over the dialog explaining it.
        if let Some(tooltip) = self.widgets.tooltip().cloned() {
            if !modal {
                widgets::draw_tooltip(d, input.fonts.ui(), &tooltip, input.window);
            }
        }

        // Fullscreen sheds the splitters along with the panels, so the one place
        // that answers a cursor request is not running. This is the substitute,
        // and it is at the *end* of the frame rather than the start on purpose:
        // the toolbar is still drawn in fullscreen, so a request from it would be
        // made after a cursor set up here and lost. Nothing in the fullscreen
        // composition asks for a cursor today, which makes this a no-op that
        // resolves to `MOUSE_CURSOR_DEFAULT` — the same thing it did before — but
        // one that will not need finding again when something does.
        if self.fullscreen {
            d.set_mouse_cursor(
                self.pointer_cursor
                    .unwrap_or(raylib::consts::MouseCursor::MOUSE_CURSOR_DEFAULT),
            );
        }

        self.notices.tick(f64::from(d.get_frame_time()));
        commands
    }

    /// Draws [`Shell::hud_readout`] at the top left of `region` (the preview
    /// less an open flyout), clipped to it, in the long form when it fits.
    ///
    /// With no preview (the editor frame) it goes to the window's corner, as
    /// before. Off unless [`Shell::hud_visible`]; the long/short choice reads
    /// the width it actually gets, so a flyout shortens it rather than
    /// letting it run under the inspector.
    fn hud_readout_line(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        preview: UiRect,
        region: UiRect,
    ) {
        let Some((long, short)) = self.hud_readout.take() else {
            return;
        };
        if !self.hud_visible {
            return;
        }
        let font = input.fonts.ui();
        if preview.is_empty() {
            widgets::draw_text(d, font, &short, 12.0, 12.0, 18.0, Color::RAYWHITE);
            return;
        }
        if region.is_empty() {
            return;
        }
        let text = if region.width >= 700.0 { &long } else { &short };
        let mut scissor = widgets::begin_scissor(d, region, input.ui_scale);
        widgets::draw_text(
            &mut scissor,
            font,
            text,
            region.x + 12.0,
            region.y + 12.0,
            18.0,
            Color::RAYWHITE,
        );
    }

    /// Fullscreen's deliberately tiny save-state surface (CX-3/PXF-1).
    /// Nothing is drawn for a saved or genuinely fresh session; warning and
    /// failure are the only states allowed to spend pixels over the work.
    fn fullscreen_attention(&mut self, d: &mut RaylibDrawHandle<'_>, input: &ShellInput<'_>) {
        let state = input
            .workspace
            .current()
            .and_then(TrackAttention::from_track);
        let Some(attention) = state.filter(|_| self.fullscreen) else {
            self.fullscreen_attention_since = None;
            self.fullscreen_attention_token = "none";
            return;
        };
        let now = d.get_time();
        let since = *self.fullscreen_attention_since.get_or_insert(now);
        let expanded = fullscreen_attention_expanded(since, now);
        self.fullscreen_attention_token = if expanded {
            attention.expanded_token()
        } else {
            attention.dot_token()
        };
        let tint = attention.color();
        let right = input.window.0 - 10.0;
        let centre_y = 10.0;
        if expanded {
            // A floating pill in the save state's own words (WX-5 follow-up):
            // the dot and then the label, as the top bar says "● Unsaved",
            // on the opaque floating surface rather than a translucent slab
            // whose contrast depended on the frame under it.
            let label = attention.label();
            let text = widgets::measure(input.fonts.ui(), label, metric::UI_FONT_CAPTION);
            let width =
                ATTENTION_PILL_PAD + ATTENTION_DOT + ATTENTION_DOT_GAP + text + ATTENTION_PILL_PAD;
            let pill = UiRect::new(right - width, 4.0, width, ATTENTION_PILL_HEIGHT);
            widgets::floating_surface(d, pill);
            let centre = pill.y + pill.height * 0.5;
            d.draw_circle_v(
                Vector2::new(pill.x + ATTENTION_PILL_PAD + ATTENTION_DOT * 0.5, centre),
                ATTENTION_DOT * 0.5,
                tint,
            );
            widgets::draw_text(
                d,
                input.fonts.ui(),
                label,
                pill.x + ATTENTION_PILL_PAD + ATTENTION_DOT + ATTENTION_DOT_GAP,
                centre - metric::UI_FONT_CAPTION * 0.5 - 1.0,
                metric::UI_FONT_CAPTION,
                tint,
            );
        } else {
            d.draw_circle(right as i32, centre_y as i32, 3.0, tint);
        }
    }

    #[must_use]
    pub(crate) fn fullscreen_attention_token(&self) -> &'static str {
        self.fullscreen_attention_token
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
        appearance_input_blocked: bool,
    ) -> Vec<ShellCommand> {
        let mut commands = Vec::new();
        // The welcome screen has no timed lane, so the modifier can never be
        // consulted; passing the real key state anyway keeps one rule.
        self.begin_frame(input.ui_scale, shift_held(d));
        // No track, no rail and no top bar (WX-5c §1).
        self.last_rail = None;
        self.top_bar_state = super::top_bar::TopBarState::default();
        if self.editor_open || appearance_input_blocked {
            self.widgets.button(
                d,
                widgets::widget_id(super::assist_settings::MODAL_BLOCK_NAMESPACE, 0),
                UiRect::new(0.0, 0.0, input.window.0, input.window.1),
            );
        } else {
            self.dropped_files(d, &mut commands);
        }

        let (w, h) = input.window;
        let frame = WelcomeFrame::layout(w, h, self.recovery_available);
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
            // A large faint numeral, not the accent: on this screen the one
            // thing that is "now" is the action it exists for (WX-5a).
            color::ui_disabled(),
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
            "Open an audio file, choose a scene, refine timing, then export a video.",
            frame.body.x,
            frame.body.y + 112.0,
            17.0,
            color::ui_muted(),
        );

        // `Open audio` is the primary action — the accent fill — which is how
        // the C marks the one action the screen exists for (`plug.c:7790`); since
        // WX-5a that is `ButtonStyle::Primary` rather than a selected state. A
        // recovery generation changes that priority: opening another session
        // before deciding its fate could eventually replace the only recovery
        // copy, so both entry points stay visibly unavailable until Recover or
        // Dismiss is explicit.
        if self.recovery_available {
            self.widgets
                .disabled_button(d, font, frame.open_audio, "Open audio", None);
            self.widgets
                .disabled_button(d, font, frame.open_project, "Open project", None);
        } else {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(widgets::id::WELCOME, 0),
                    frame.open_audio,
                    "Open audio",
                    false,
                    ButtonStyle::Primary,
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
        }
        if let Some(recovery) = frame.recovery {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(widgets::id::WELCOME, 2),
                    recovery.recover,
                    "Recover session",
                    false,
                    ButtonStyle::Primary,
                    None,
                )
                .clicked
            {
                commands.push(ShellCommand::RecoverSession);
            }
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(widgets::id::WELCOME, 3),
                    recovery.dismiss,
                    "Dismiss",
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                commands.push(ShellCommand::DiscardRecovery);
            }
            widgets::draw_text(
                d,
                font,
                "Recover or dismiss the saved session to open another track.",
                recovery.hint.x,
                recovery.hint.y,
                15.0,
                color::ui_warning(),
            );
        } else {
            widgets::draw_text(
                d,
                font,
                "or drop audio anywhere in this window",
                frame.drop_hint.x,
                frame.drop_hint.y,
                15.0,
                color::ui_muted(),
            );
        }

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
                    color::ui_muted(),
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

        self.recent_column(d, input, &frame, &mut commands);

        // The tray covers the whole window here, not a preview rectangle: there is
        // no preview, and a load failure is exactly the message this screen has to
        // be able to show (`plug.c:7830`).
        self.notice_tray(d, font, input.ui_scale.value(), UiRect::new(0.0, 0.0, w, h));
        self.notices.tick(f64::from(d.get_frame_time()));

        // Last, as in [`Shell::draw`], and for the same reason: `hint` only
        // *queues* a tip, and whoever owns the frame has to paint it.
        //
        // This screen had no such call until the recent list gave it its first
        // hinted controls, so the full path of a row — the only place it is
        // written down — was being queued and dropped every frame. Exactly the
        // failure this repository keeps paying for: the tip was requested
        // correctly, `hint` was called correctly, and nothing but looking for the
        // paint would have found it.
        if let Some(tooltip) = self.widgets.tooltip().cloned() {
            widgets::draw_tooltip(d, font, &tooltip, input.window);
        }
        commands
    }

    /// The welcome screen's recent-project list (UX0-C06).
    ///
    /// Invented; the oracle has no history at all. It lives in the region right of
    /// the 72 % column rule because that is the only part of this screen the C
    /// leaves empty, and because a returning user's first question — "where was I"
    /// — deserves an answer beside the one for a first-time user rather than
    /// instead of it.
    ///
    /// Three states are drawn rather than two. An empty list says so, and a list
    /// that could not be *read* says that instead: those are different facts, and
    /// a blank column would be indistinguishable from a broken one.
    fn recent_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        frame: &WelcomeFrame,
        commands: &mut Vec<ShellCommand>,
    ) {
        if frame.recent_visible == 0 {
            return;
        }
        let font = input.fonts.ui();
        let header = frame.recent_header;
        widgets::draw_text_tracked(
            d,
            font,
            "RECENT",
            header.x,
            header.y,
            metric::UI_FONT_CAPTION,
            2.0,
            color::ui_muted(),
        );
        let rule_y = header.y + header.height + 8.0;
        d.draw_line_ex(
            Vector2::new(header.x, rule_y),
            Vector2::new(header.x + header.width, rule_y),
            1.0,
            color::ui_rule(),
        );

        if self.recent_unavailable {
            // Named, not blank. This is the one state the store's whole
            // failure-tolerance design exists for, and it must be visible on
            // screen rather than only in a notice that scrolls away.
            widgets::draw_text_faded(
                d,
                font,
                "History unavailable.",
                header.x,
                frame.recent_rows[0].y,
                header.width,
                24.0,
                metric::UI_FONT_LABEL,
                color::ui_warning(),
            );
            widgets::draw_text_faded(
                d,
                font,
                "recent.json could not be read, and",
                header.x,
                frame.recent_rows[0].y + 22.0,
                header.width,
                24.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            widgets::draw_text_faded(
                d,
                font,
                "will not be overwritten.",
                header.x,
                frame.recent_rows[0].y + 38.0,
                header.width,
                24.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            return;
        }

        if self.recent.is_empty() {
            widgets::draw_text_faded(
                d,
                font,
                "Projects you save will",
                header.x,
                frame.recent_rows[0].y,
                header.width,
                24.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            widgets::draw_text_faded(
                d,
                font,
                "appear here.",
                header.x,
                frame.recent_rows[0].y + 18.0,
                header.width,
                24.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            return;
        }

        // Collected first, so the borrow of `self.recent` is over before the
        // widgets below take `&mut self`.
        let rows: Vec<(PathBuf, String, bool, Option<String>)> = self
            .recent
            .entries()
            .iter()
            .take(frame.recent_visible)
            .map(|entry| {
                (
                    entry.path.clone(),
                    entry.name.clone(),
                    entry.missing,
                    super::preferences::recent::describe_age(
                        entry.opened_unix,
                        self.recent_now_unix,
                    ),
                )
            })
            .collect();

        const FORGET_SIZE: f32 = 18.0;
        for (index, (path, name, missing, age)) in rows.into_iter().enumerate() {
            let row = frame.recent_rows[index];
            let open_id = widgets::widget_id(widgets::id::WELCOME_RECENT, index as u32 * 2);
            let forget_id = widgets::widget_id(widgets::id::WELCOME_RECENT, index as u32 * 2 + 1);
            let forget = UiRect::new(
                row.x + row.width - FORGET_SIZE - 2.0,
                row.y + (row.height - FORGET_SIZE) * 0.5,
                FORGET_SIZE,
                FORGET_SIZE,
            );
            let text_width = (forget.x - row.x - 12.0).max(0.0);
            // The row *minus* the forget box, and the two must not overlap.
            //
            // They did, and the click probe caught it on its first run: the open
            // area was the whole row, it is drawn first, and so it claimed every
            // press aimed at the cross — which then opened the project instead of
            // forgetting it. That is EX1 exactly, one namespace later: the hover
            // highlight was correct, the cross was drawn in the right place, and
            // nothing but an injected press could tell that it was dead.
            let open_area =
                UiRect::new(row.x, row.y, (forget.x - row.x - 4.0).max(0.0), row.height);

            // The whole row *left of the cross* is the target, not just the name.
            // A 44 px row is the affordance the same way the 50 px cue lane is
            // (LX1-d): a click target you have to aim at is one a returning user
            // misses.
            let state = self.widgets.button(d, open_id, open_area);
            if state.hovered && !missing {
                widgets::fill(d, row, color::ui_hover());
                widgets::fill(
                    d,
                    UiRect::new(row.x - 6.0, row.y, 2.0, row.height),
                    color::ui_selected_edge(),
                );
            }
            // A missing file's row is *not* clickable, and that is the difference
            // between offering removal and erroring: pressing it would raise a
            // notice the user can do nothing about, every time, forever.
            if state.clicked && !missing {
                commands.push(ShellCommand::OpenRecentProject(path.clone()));
            }
            self.widgets.hint(
                d,
                state,
                open_id,
                open_area,
                &if missing {
                    format!("Not found: {}", path.display())
                } else {
                    format!("Open {}", path.display())
                },
            );

            widgets::draw_text_faded(
                d,
                font,
                &name,
                row.x,
                row.y + 5.0,
                text_width,
                20.0,
                metric::UI_FONT_LABEL,
                if missing {
                    color::ui_warning()
                } else {
                    color::ui_ink()
                },
            );
            // Age and folder on one line: the folder is what disambiguates two
            // projects with the same title, and the age is what a returning user
            // actually scans for.
            let folder = path
                .parent()
                .map(|parent| parent.display().to_string())
                .unwrap_or_default();
            let meta = match (missing, age) {
                (true, _) => format!("File is missing - {folder}"),
                (false, Some(age)) => format!("{age} - {folder}"),
                (false, None) => folder,
            };
            widgets::draw_text_faded(
                d,
                font,
                &meta,
                row.x,
                row.y + 24.0,
                text_width,
                20.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );

            // Drawn on every row, never only on hover. An affordance you have to
            // be standing on to find cannot be found by a pointer that never
            // lands there — LX2-a, learned on the cue lane's own handles.
            let forget_state = self.widgets.button(d, forget_id, forget);
            let tint = if forget_state.hovered {
                color::ui_ink()
            } else if missing {
                color::ui_warning()
            } else {
                color::ui_muted()
            };
            let inset = 5.0;
            d.draw_line_ex(
                Vector2::new(forget.x + inset, forget.y + inset),
                Vector2::new(
                    forget.x + forget.width - inset,
                    forget.y + forget.height - inset,
                ),
                1.5,
                tint,
            );
            d.draw_line_ex(
                Vector2::new(forget.x + forget.width - inset, forget.y + inset),
                Vector2::new(forget.x + inset, forget.y + forget.height - inset),
                1.5,
                tint,
            );
            self.widgets
                .hint(d, forget_state, forget_id, forget, "Forget this project");
            if forget_state.clicked {
                commands.push(ShellCommand::ForgetRecentProject(path));
            }
        }
    }

    /// Files dropped on the window, in either screen.
    ///
    /// The C handles this once for the whole application rather than per screen,
    /// and the welcome screen's own copy printing "or drop audio anywhere in this
    /// window" is a promise that has to hold on both.
    /// Typed dispatch (D1, `plug.c:7536-7565`).
    ///
    /// Every path is classified before anything is loaded, so a dropped project
    /// opens as a project and a dropped picture becomes glyphs instead of both
    /// being handed to the audio decoder and reported as a corrupt song.
    fn dropped_files(&mut self, d: &RaylibDrawHandle<'_>, commands: &mut Vec<ShellCommand>) {
        // `--ui-probe drop=PATH` first, and *instead of* the device: Xvfb has no
        // drag-and-drop any more than it has a wheel, so this branch is the only
        // way the three dispatch arms can be photographed. Taken rather than
        // read, so a 30-frame probe drops once.
        if let Some(path) = self.probe_drop.take() {
            self.probe_drop_dispatch = Some((path.clone(), classify_drop(&path)));
            commands.push(drop_command(&path));
        }
        if !d.is_file_dropped() {
            return;
        }
        for path in d.load_dropped_files().paths() {
            commands.push(drop_command(Path::new(path)));
        }
    }

    fn keyboard(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if d.is_key_pressed(raylib::consts::KeyboardKey::KEY_F8)
            && !d.is_mouse_button_down(raylib::consts::MouseButton::MOUSE_BUTTON_LEFT)
            && !self.text_entry_has_focus()
            && self.lyric_draft_allows_context_change(input.workspace)
        {
            self.editor_open = true;
            return;
        }
        let mut keys = KeyboardFrame::read(d);
        keys.escape |= std::mem::take(&mut self.probe_escape);
        self.keyboard_actions(keys, KeyboardContext::of(input), commands);
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

        // The protocol answer row (HX-2), before the global bindings so a
        // session owns its keys — and only while one is running, so `1`-`4`,
        // `R`, `B` and `N` mean nothing anywhere else. Ctrl-chorded digits
        // stay with the scale ladder above.
        if self.protocol.is_some() && !keys.control {
            if let Some(digit) = keys.answer_digit {
                commands.push(ShellCommand::Protocol(ProtocolAction::Answer(digit)));
            }
            if keys.protocol_replay {
                commands.push(ShellCommand::Protocol(ProtocolAction::Replay));
            }
            if keys.protocol_flip {
                commands.push(ShellCommand::Protocol(ProtocolAction::Flip));
            }
            if keys.protocol_next {
                commands.push(ShellCommand::Protocol(ProtocolAction::Next));
            }
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
        //
        // An open flyout takes Escape first (WX-5c): it is the thing on top,
        // and fullscreen cannot be on while one is open.
        if keys.escape && self.open_flyout.is_some() {
            self.open_flyout = None;
        } else if keys.escape && self.fullscreen {
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
        // Ctrl+S / Ctrl+Shift+S (review 1.12, UX0-A12). Deliberately *not*
        // guarded on the lyric draft, for the reason the Save button is not:
        // saving changes no context, and refusing it would be telling the user to
        // discard work in order to save work.
        if keys.control && keys.save {
            commands.push(if keys.shift {
                ShellCommand::SaveProjectAs
            } else {
                ShellCommand::SaveProject
            });
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
        self.transport_report = "none".to_owned();
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
        let shown_time = self.timeline_playhead_seconds(input.time_seconds);
        let timecode = format!(
            "{} / {}",
            widgets::format_timestamp(shown_time),
            widgets::format_timestamp(input.duration_seconds)
        );
        let timecode_size = metric::UI_FONT_VALUE + 2.0;
        let timecode_width = widgets::measure_tabular(font, &timecode, timecode_size).ceil();
        // The row's whole composition — what it carries and where — is
        // `transport_bar::TransportRow`'s, raylib-free and swept for
        // monotonicity: the seek trio goes first, then the inline timecode
        // (to the band's header), then the scrub bar. Tune, Export, Lyrics and
        // Assist are the top bar's tabs and Editor and Dock the rail's icons
        // since WX-5c, so none of them is here to shed.
        let Some(row) = TransportRow::layout(bar, timecode_width, has_track) else {
            return ToolbarResult::default();
        };
        let mut report = vec![format!("step={}", row.step)];

        // The round play button: an ink disc with the glyph cut out of it in
        // the surface colour, the mockup's one solid shape on the row.
        let play_id = widgets::widget_id(widgets::id::TOOLBAR, 0);
        let control = if input.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        };
        let state = if has_track {
            self.widgets.button(d, play_id, row.play)
        } else {
            widgets::ButtonState::default()
        };
        let disc = if !has_track {
            color::ui_disabled()
        } else if state.pressed {
            widgets::brightness(color::ui_ink(), -0.12)
        } else if state.hovered {
            widgets::brightness(color::ui_ink(), 0.08)
        } else {
            color::ui_ink()
        };
        d.draw_circle_v(
            Vector2::new(
                row.play.x + row.play.width * 0.5,
                row.play.y + row.play.height * 0.5,
            ),
            row.play.width * 0.5,
            disc,
        );
        // The play glyph's own optical centre sits left of its box, so it is
        // nudged a pixel right, as every round play button does.
        let nudge = if input.playing { 0.0 } else { 1.5 };
        let glyph_box = UiRect::new(
            row.play.x + nudge,
            row.play.y,
            row.play.width,
            row.play.height,
        );
        match icons::glyph(input.fonts, &control) {
            icons::Glyph::Icon(face, text) => {
                widgets::icon_label(d, face, glyph_box, &text, 16.0, color::ui_surface());
            }
            icons::Glyph::Text(font, text) => {
                widgets::text_label(
                    d,
                    font,
                    glyph_box,
                    &text,
                    metric::UI_FONT_MICRO,
                    color::ui_surface(),
                );
            }
        }
        self.widgets.hint(d, state, play_id, row.play, control.tip);
        if state.clicked {
            commands.push(ShellCommand::TogglePlay);
        }
        report.push(format!("play={}", super::rail::rect_token(row.play)));

        // The position in ink and the length in muted grey (WX-5a, the
        // mockup's `3:12.00 / 5:22.52`): the number that moves is the one to
        // read. Split at the separator; tabular widths are a per-glyph sum, so
        // the two halves land exactly where the whole string did.
        if let Some(rect) = row.timecode {
            let y = rect.y + (rect.height - timecode_size) * 0.5;
            let split = timecode.find(" / ").unwrap_or(timecode.len());
            let (now, total) = timecode.split_at(split);
            widgets::draw_text_tabular(d, font, now, rect.x, y, timecode_size, color::ui_ink());
            widgets::draw_text_tabular(
                d,
                font,
                total,
                rect.x + widgets::measure_tabular(font, now, timecode_size),
                y,
                timecode_size,
                color::ui_muted(),
            );
            report.push(format!("timecode={}", super::rail::rect_token(rect)));
        }

        // The fine-seek trio, as ghost icons: secondary to the play button,
        // and each with its key ladder in the tooltip. Numbered by what they
        // are, so a press claimed before a resize is released by the same one.
        if let Some(seek) = row.seek {
            for (slot, control, rect) in [
                (0, icons::SEEK_START, seek.start),
                (1, icons::SEEK_BACK, seek.back),
                (2, icons::SEEK_FORWARD, seek.forward),
            ] {
                let size = 14.0;
                let glyph = icons::glyph(input.fonts, &control);
                if !has_track {
                    match &glyph {
                        icons::Glyph::Icon(face, text) => {
                            widgets::icon_label(d, face, rect, text, size, color::ui_disabled());
                        }
                        icons::Glyph::Text(font, text) => {
                            widgets::text_label(
                                d,
                                font,
                                rect,
                                text,
                                metric::UI_FONT_MICRO,
                                color::ui_disabled(),
                            );
                        }
                    }
                    continue;
                }
                let id = widgets::widget_id(widgets::id::SEEK, slot);
                let state = match &glyph {
                    icons::Glyph::Icon(face, text) => self.widgets.icon_button(
                        d,
                        face,
                        id,
                        rect,
                        text,
                        false,
                        ButtonStyle::Ghost,
                        size,
                    ),
                    icons::Glyph::Text(font, text) => self.widgets.text_button(
                        d,
                        font,
                        id,
                        rect,
                        text,
                        false,
                        ButtonStyle::Ghost,
                        Some(metric::UI_FONT_MICRO),
                    ),
                };
                self.widgets.hint(d, state, id, rect, control.tip);
                if state.clicked {
                    commands.push(ShellCommand::Seek(if slot == 0 {
                        0.0
                    } else {
                        self.nudge_target(d, input, if slot == 1 { -1.0 } else { 1.0 })
                    }));
                }
            }
            report.push(format!(
                "seek=start@{};back@{};forward@{}",
                super::rail::rect_token(seek.start),
                super::rail::rect_token(seek.back),
                super::rail::rect_token(seek.forward)
            ));
        }

        // The position bar goes in whatever is left between the trio and the
        // utilities. See [`Self::position_bar`] for what used to be here.
        if let Some(scrub) = row.scrub {
            self.position_bar(d, input, scrub, has_track, commands);
            report.push(format!("scrub={}", super::rail::rect_token(scrub.hit)));
        }

        if let Some(cluster) = row.utilities {
            self.utility_cluster(d, input, cluster, has_track, commands);
            for (name, rect) in [
                ("readout", cluster.readout),
                ("mute", cluster.mute),
                ("volume", cluster.volume),
                ("fullscreen", Some(cluster.fullscreen)),
            ] {
                if let Some(rect) = rect {
                    report.push(format!("{name}={}", super::rail::rect_token(rect)));
                }
            }
        }
        self.transport_report = report.join(" ");
        ToolbarResult {
            timecode_inline: row.timecode.is_some(),
        }
    }

    /// The `transport:` report line (WX-5c §2): the row's ladder step and the
    /// rect of every control it drew, as `name=x,y,w,h` (the seek trio as
    /// `name@x,y,w,h` entries joined by `;`). The tooltip and click probes on
    /// the row read their points here rather than from `preview frame:` plus
    /// a constant.
    #[must_use]
    pub fn describe_transport(&self) -> &str {
        &self.transport_report
    }

    /// The transport row's position bar: seek control, with the analyzer's level
    /// embedded (review 1.9, UX0-A09).
    ///
    /// # What was here
    ///
    /// An RMS level meter. A green bar filling from the left, immediately beside
    /// the timecode, in the position every media player on the machine uses for
    /// progress — and with no widget id at all, so a click on it did nothing. At
    /// 0.15 s into a track it read about 22% full. Two failures at once: the one
    /// control a user would reach for first was inert, and it was actively
    /// misreporting the position while looking authoritative.
    ///
    /// # What it is now
    ///
    /// A real seek bar. The fill is playback position, the drag scrubs, and the
    /// transaction is the timeline scrubber's — pause on press, follow the hand,
    /// seek once on release — because repeatedly flushing a decoder while the
    /// pointer moves is both audible and expensive. The level survives as
    /// [`transport_bar::ScrubBar::level`]: a 4 px strip under the groove, in the
    /// success green it always was, which is legible as a level and cannot be
    /// mistaken for progress. It is the readout that makes a stuck analyzer
    /// visible in a screenshot, so it is worth keeping — it reads `rms` and not a
    /// band, because bands are normalized per frame by the frame's own maximum
    /// (`audio_analyzer.c:204`) and a meter driven from `bands` would be flat.
    ///
    /// # When the stream cannot be seeked
    ///
    /// `transport_seekable` is false for the formats the decoder cannot position
    /// in. The bar then draws as a plain position readout — grey fill, no
    /// playhead, no claim on the pointer — and its tooltip says why. Drawing an
    /// accent-blue scrubber that silently ignored every drag is the thing this
    /// whole finding is about.
    fn position_bar(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        scrub: transport_bar::ScrubBar,
        has_track: bool,
        commands: &mut Vec<ShellCommand>,
    ) {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        if !has_track {
            return;
        }
        let seekable = input
            .workspace
            .current()
            .is_some_and(|track| track.transport_seekable)
            && input.duration_seconds.is_finite()
            && input.duration_seconds > 0.0;

        widgets::fill(d, scrub.track, color::ui_control_edge());

        // The id lives in the fine-seek namespace rather than a new one: this bar
        // *is* a seek control, and slots 0-2 are the three buttons beside it.
        let id = widgets::widget_id(widgets::id::SEEK, 3);
        // Registered even when the stream cannot be seeked, so the box still
        // swallows the press instead of letting whatever is behind it answer, and
        // so the tooltip explaining the refusal can be asked for.
        let state = self.widgets.button(d, id, scrub.hit);
        let mouse = input.ui_scale.mouse(d);
        let dragging = if self.editor_open || self.assist_settings.is_open() {
            None
        } else {
            self.transport_scrub(
                TransportScrubInput {
                    track: scrub.track,
                    pointer_x: mouse.x,
                    hovered: state.hovered,
                    pressed: d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT),
                    down: d.is_mouse_button_down(MOUSE_BUTTON_LEFT),
                    playing: input.playing,
                    duration_seconds: input.duration_seconds,
                    seekable,
                },
                commands,
            )
        };

        // While dragging, the bar shows the hand rather than the decoder: the
        // seek has not been sent yet, so the playhead has genuinely not moved.
        let shown = dragging.unwrap_or(input.time_seconds);
        let fraction = transport_bar::scrub_fraction(shown, input.duration_seconds);
        widgets::fill(
            d,
            UiRect::new(
                scrub.track.x,
                scrub.track.y,
                scrub.track.width * fraction,
                scrub.track.height,
            ),
            // The played part is a filled amount, so it is the level tone; the
            // handle below is the playhead, so it is the accent (WX-5a).
            if seekable {
                color::ui_level()
            } else {
                color::ui_disabled()
            },
        );
        if seekable {
            // A playhead that overhangs the groove, so the bar reads as a
            // scrubber with a handle rather than as a progress bar with a fill.
            let x = scrub.track.x + scrub.track.width * fraction;
            let overhang = 3.0;
            widgets::fill(
                d,
                UiRect::new(
                    (x - 1.5).clamp(scrub.track.x, scrub.track.x + scrub.track.width - 3.0),
                    scrub.track.y - overhang,
                    3.0,
                    scrub.track.height + overhang * 2.0,
                ),
                color::accent(),
            );
        }

        widgets::fill(d, scrub.level, color::ui_rule());
        if input.band_count > 0 {
            let level = input.rms.clamp(0.0, 1.0);
            widgets::fill(
                d,
                UiRect::new(
                    scrub.level.x,
                    scrub.level.y,
                    scrub.level.width * level,
                    scrub.level.height,
                ),
                color::ui_success(),
            );
        }

        self.widgets.hint(
            d,
            state,
            id,
            scrub.hit,
            if seekable {
                "Seek [Home/End, arrows]"
            } else {
                "Position — this stream cannot be seeked"
            },
        );

        if let Some(caption) = telemetry_caption(
            self.hud_visible,
            scrub.hit.width,
            input.band_count,
            input.peak_band,
            input.rms,
        ) {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                &caption,
                scrub.hit.x,
                scrub.hit.y - 16.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }
    }

    /// Claim the shared timed lanes for one transactional seek.
    ///
    /// Scene bodies and the PCM strip call the same method; lyric bodies do not,
    /// because their left-button drag belongs to cue editing. The decoder is
    /// paused once here and seeked once by [`Self::complete_timeline_scrub`].
    pub(crate) fn begin_timeline_scrub(
        &mut self,
        playing: bool,
        commands: &mut Vec<ShellCommand>,
    ) -> bool {
        if self.timeline_gesture.is_some() {
            return false;
        }
        self.timeline_gesture = Some(TimelineGesture::Scrub);
        self.scrub_restore_playing = playing;
        self.timeline_manual_view = false;
        if playing {
            commands.push(ShellCommand::TogglePlay);
        }
        true
    }

    /// Update the preview target for the shared lane scrub and complete it when
    /// the physical button is released. `lane` may be the scene or PCM lane:
    /// their X geometry is deliberately identical through `TimelineView`.
    pub(crate) fn update_timeline_scrub(
        &mut self,
        pointer_x: f32,
        lane: UiRect,
        duration_seconds: f64,
        button_down: bool,
        commands: &mut Vec<ShellCommand>,
    ) -> Option<f64> {
        if self.timeline_gesture != Some(TimelineGesture::Scrub) {
            return None;
        }
        let seconds = self.timeline.seconds_at(
            f64::from(pointer_x),
            f64::from(lane.x),
            f64::from(lane.width),
            duration_seconds,
        );
        self.scrub_target_seconds = Some(seconds);
        if !button_down {
            self.complete_timeline_scrub(commands);
        }
        Some(seconds)
    }

    fn complete_timeline_scrub(&mut self, commands: &mut Vec<ShellCommand>) {
        if self.timeline_gesture != Some(TimelineGesture::Scrub) {
            return;
        }
        self.timeline_gesture = None;
        if let Some(target) = self.scrub_target_seconds.take() {
            self.scrub_release_preview_seconds = Some(target);
            commands.push(ShellCommand::Seek(target));
        }
        if std::mem::take(&mut self.scrub_restore_playing) {
            commands.push(ShellCommand::TogglePlay);
        }
    }

    /// Middle-button pan shared by the scene, PCM and lyric timed lanes.
    ///
    /// The active gesture keeps updating after the pointer leaves its source
    /// lane. Releasing anywhere clears it, while [`Self::abandon_workspace_drags`]
    /// clears it if the surface disappears first.
    pub(crate) fn timeline_pan_gesture(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        lane: UiRect,
    ) {
        use raylib::consts::MouseButton::MOUSE_BUTTON_MIDDLE;

        let device = input.ui_scale.mouse(d);
        // `--ui-probe middle-drag=` substitutes the whole pointer for this
        // gesture, not just the button: a pan is measured from where the press
        // landed to where the pointer is now, so injecting a button while
        // `GetMousePosition` stayed at one place would drive a pan of exactly
        // zero seconds and photograph as the gesture being broken.
        let (pointer_x, pressed, down) = match self.probe_middle_drag_frame {
            Some(phase) => (phase.x, phase.pressed, phase.down),
            None => (
                device.x,
                d.is_mouse_button_pressed(MOUSE_BUTTON_MIDDLE),
                d.is_mouse_button_down(MOUSE_BUTTON_MIDDLE),
            ),
        };
        let pointer_y = self
            .probe_middle_drag_frame
            .map_or(device.y, |phase| phase.y);

        if self.timeline_gesture.is_none()
            && !self.timeline.is_whole(input.duration_seconds)
            && lane.contains_point(pointer_x, pointer_y)
            && pressed
        {
            self.begin_timeline_pan(pointer_x);
        }
        self.update_timeline_pan(pointer_x, lane, input.duration_seconds, down);
    }

    fn begin_timeline_pan(&mut self, origin_x: f32) {
        self.timeline_gesture = Some(TimelineGesture::Pan);
        self.timeline_pan = Some(TimelinePan {
            origin_x,
            origin_start_seconds: self.timeline.start_seconds,
        });
        self.timeline_manual_view = true;
    }

    fn update_timeline_pan(
        &mut self,
        pointer_x: f32,
        lane: UiRect,
        duration_seconds: f64,
        button_down: bool,
    ) {
        if self.timeline_gesture != Some(TimelineGesture::Pan) {
            return;
        }
        if !button_down {
            self.timeline_gesture = None;
            self.timeline_pan = None;
            return;
        }
        let Some(pan) = self.timeline_pan else {
            self.timeline_gesture = None;
            return;
        };
        let delta_seconds = f64::from(pan.origin_x - pointer_x)
            * self.timeline.seconds_per_pixel(f64::from(lane.width));
        self.timeline.start_seconds = pan.origin_start_seconds;
        self.timeline.pan(duration_seconds, delta_seconds);
    }

    /// Captures one wheel notch over a timed lane as a zoom of the shared view.
    ///
    /// Every lane drawn against [`Self::timeline`] may call this — the waveform
    /// strip, the scene-plan lane and the lyric cue lane — because all three are
    /// views of the same time axis, and it was arbitrary that the wheel only
    /// worked over one of them (operator request, 2026-08-06).
    ///
    /// The caller must have proven the pointer is inside its **own lane rect**
    /// before calling, and must not widen that to the section or panel around it.
    /// The capture is deliberately narrow: the lyrics panel is a scrolling list
    /// sharing the band with its cue lane, and a wheel region that spilled past
    /// the lane would take the list's scroll away from it.
    ///
    /// **First claim of the frame wins**, which is the opposite of
    /// [`Self::request_cursor`]'s rule and for a reason that does not apply
    /// there: a wheel notch is a single physical event, and `get_mouse_wheel_move`
    /// reports the same value to every caller in the frame. Two lanes accepting
    /// it would multiply the zoom factor by itself, so one notch over an overlap
    /// would zoom twice as far as one notch anywhere else. Whose claim it is
    /// barely matters — the lanes are disjoint, so an overlap means a hit test is
    /// already wrong — but that it is exactly one claim does.
    ///
    /// `wheel` is `d.get_mouse_wheel_move()`; `pointer_x` and the lane bounds are
    /// in the same logical space as [`TimelineView::seconds_at`].
    ///
    /// **With Shift held the same notch pans instead of zooming** (D4). The name
    /// is kept because the lyric cue lane calls this too and that file has a
    /// different owner; the seam is "one wheel notch over a timed lane", and
    /// routing the modifier here rather than at each call site is what makes all
    /// three lanes gain the gesture at once — the same argument LX2-c made for
    /// zoom. Shift is read from the shell rather than passed in for the same
    /// reason.
    pub(crate) fn request_timeline_zoom(
        &mut self,
        wheel: f32,
        pointer_x: f32,
        lane_x: f32,
        lane_width: f32,
        duration_seconds: f64,
    ) {
        // A gesture in flight owns the view: zooming under a scrub or a pan moves
        // the axis the drag is measured against, and the content slides out from
        // under the hand.
        if wheel == 0.0
            || duration_seconds <= 0.0
            || self.timeline_gesture.is_some()
            || self.timeline_zoom_pending.is_some()
        {
            return;
        }
        if self.wheel_pan_modifier {
            // Panning a whole-track view is a no-op that would still consume the
            // notch and set `timeline_manual_view`, leaving the Follow button lit
            // over a view that never moved — a control that says it did something
            // it did not.
            if self.timeline.is_whole(duration_seconds) {
                return;
            }
            // Wheel up is *earlier*, matching a vertical scroll: rolling away
            // from the hand moves the window back through the track, the same
            // direction the content moves under a list.
            let delta_seconds =
                -f64::from(wheel) * self.timeline.span_seconds * TIMELINE_WHEEL_PAN_FRACTION;
            self.timeline_zoom_pending = Some(TimelineWheel::Pan { delta_seconds });
            // A wheel pan detaches follow exactly as a middle-drag pan does. It
            // is the same gesture by a different input, and having one of them
            // fight playback-follow while the other did not would be the sort of
            // difference that reads as a bug.
            self.timeline_manual_view = true;
            return;
        }
        let anchor = self.timeline.seconds_at(
            f64::from(pointer_x),
            f64::from(lane_x),
            f64::from(lane_width),
            duration_seconds,
        );
        // 1.2 per notch, applied at the top of the next frame rather than here —
        // see the comment on the take in `timeline_strip`, which is why the delay
        // is deliberate and must stay.
        self.timeline_zoom_pending = Some(TimelineWheel::Zoom {
            factor: 1.2f64.powf(f64::from(wheel)),
            anchor_seconds: anchor,
        });
    }

    /// One frame of the position bar's drag, raylib-free.
    ///
    /// Returns the in-flight target while a drag is running, so the caller draws
    /// where the hand is. The seek itself is emitted on release by
    /// [`Self::complete_transport_scrub`].
    pub(crate) fn transport_scrub(
        &mut self,
        input: TransportScrubInput,
        commands: &mut Vec<ShellCommand>,
    ) -> Option<f64> {
        if !input.seekable {
            // A stream can stop being seekable between frames only by the track
            // changing under the drag, which is exactly when a stranded gesture
            // would fire a seek at the *new* track.
            self.complete_transport_scrub(commands);
            return None;
        }
        if self.transport_scrub.is_none() && input.hovered && input.pressed {
            self.transport_scrub = Some(TransportScrub {
                target_seconds: 0.0,
                restore_playing: input.playing,
            });
            // Paused for the drag, and resumed by `complete`. Without this the
            // playhead races the hand and the fill fights the drag.
            if input.playing {
                commands.push(ShellCommand::TogglePlay);
            }
        }
        let scrub = self.transport_scrub.as_mut()?;
        scrub.target_seconds =
            transport_bar::scrub_seconds(input.pointer_x, input.track, input.duration_seconds);
        let target = scrub.target_seconds;
        if !input.down {
            self.complete_transport_scrub(commands);
        }
        Some(target)
    }

    /// Ends a position-bar drag, sending the seek it owes.
    ///
    /// Completed rather than dropped, for the reason
    /// [`Self::abandon_workspace_drags`] gives about the timeline scrubber: the
    /// drag paused playback when it began, so abandoning it silently would leave
    /// the track paused at a position the playhead never reached.
    fn complete_transport_scrub(&mut self, commands: &mut Vec<ShellCommand>) {
        let Some(scrub) = self.transport_scrub.take() else {
            return;
        };
        commands.push(ShellCommand::Seek(scrub.target_seconds));
        if scrub.restore_playing {
            commands.push(ShellCommand::TogglePlay);
        }
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
        has_track: bool,
        commands: &mut Vec<ShellCommand>,
    ) {
        // Ghost icons (WX-5c): a glyph until hovered, and the selection look
        // while the state it names holds — the readout on, the output muted,
        // fullscreen on. The volume beside mute is a chip.
        let font_size = 16.0;
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
                    icons::Glyph::Icon(face, text) => {
                        widgets::icon_label(
                            d,
                            face,
                            boundary,
                            text,
                            font_size,
                            color::ui_disabled(),
                        );
                    }
                    icons::Glyph::Text(font, text) => widgets::text_label(
                        d,
                        font,
                        boundary,
                        text,
                        metric::UI_FONT_MICRO,
                        color::ui_disabled(),
                    ),
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
                    ButtonStyle::Ghost,
                    font_size,
                ),
                icons::Glyph::Text(font, text) => shell.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary,
                    text,
                    selected,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_MICRO),
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
            // The chip shows the *stored* volume even while muted, rather than
            // dropping to zero: mute is a toggle the user expects to undo, and a
            // level that zeroed itself would lose the one they had set.
            let id = widgets::widget_id(widgets::id::UTILITY, 2);
            if has_track {
                let (state, value) = self.widgets.volume_chip(
                    d,
                    input.fonts.ui(),
                    id,
                    boundary,
                    input.volume,
                    input.muted,
                );
                if let Some(value) = value {
                    commands.push(ShellCommand::SetVolume(value));
                }
                // The wheel over the chip steps it by 5 %, which is the
                // gesture a chip this small invites.
                let wheel = if state.hovered {
                    self.wheel_delta(d)
                } else {
                    0.0
                };
                if wheel != 0.0 && wheel.is_finite() {
                    commands.push(ShellCommand::SetVolume(
                        (input.volume + wheel.signum() * 0.05).clamp(0.0, 1.0),
                    ));
                }
                self.widgets
                    .hint(d, state, id, boundary, "Volume: drag or scroll");
            }
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

    /// Opens or hides the dock and remembers the choice. Hiding keeps its
    /// terminals running; only closing a tab ends one.
    pub fn toggle_dock(&mut self) -> ShellCommand {
        self.dock_open = !self.dock_open;
        if !self.dock_open {
            self.dock_keyboard = false;
        }
        self.ui_preferences.dock_open = self.dock_open;
        ShellCommand::SaveUiPreferences(self.ui_preferences)
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
            // Paired with the inspector being on screen for the same reason the
            // other two are paired with their panes: a field left focused by a
            // pane that has since closed must not silence every shortcut for the
            // rest of the session. `T` closes the inspector without asking the
            // panel first, which is exactly that case.
            TextEntrySurface::TuneValue => self.inspector_open && self.tune_value_typing(),
            // Set by the composition root from the dock's own focus, every
            // frame; paired with the dock being open for the reason above.
            TextEntrySurface::Dock => self.dock_open && self.dock_keyboard,
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
        // The position bar is drawn by the toolbar, and fullscreen replaces the
        // toolbar — so a drag left in flight there is stranded exactly the way a
        // timeline scrub is, and is completed for the same reason.
        self.complete_transport_scrub(commands);
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.complete_timeline_scrub(commands);
        }
        // Panning has no deferred model command, so abandoning it is just a
        // release. The already-visible view remains where the hand left it.
        if self.timeline_gesture == Some(TimelineGesture::Pan) {
            self.timeline_gesture = None;
            self.timeline_pan = None;
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

    /// The column's view, read off today's state (WX-5d seam).
    ///
    /// A bottom panel wins over the inspector: before the fold, Look could be
    /// open beside one, and the panel is the bigger thing on screen. Then
    /// the inspector, then the dock, else closed.
    #[must_use]
    #[allow(
        dead_code,
        reason = "WX-5d seam: the `column:` line and the tabs read it once the Frame builder builds them"
    )]
    pub fn column_view(&self) -> ColumnView {
        match self.panel {
            UiPanel::Lyrics => ColumnView::Lyrics,
            UiPanel::Assist => ColumnView::Assist,
            UiPanel::Export => ColumnView::Render,
            UiPanel::None | UiPanel::Tune if self.inspector_open => ColumnView::Look,
            UiPanel::None | UiPanel::Tune if self.dock_open => ColumnView::Direct,
            UiPanel::None | UiPanel::Tune => ColumnView::Closed,
        }
    }

    /// Shows `view`, by writing the state [`Self::column_view`] reads, so the
    /// two round-trip for every view. One view at a time: Look no longer
    /// coexists with a panel (§2). The dock is only opened (Direct) or closed
    /// (Closed); behind another view it stays open, because hiding a chat
    /// never ends it. No lyric-draft guard here: the tab press keeps its own,
    /// and the probe sets state directly as `panel=` does.
    pub fn set_column_view(&mut self, view: ColumnView) {
        let (panel, inspector) = match view {
            ColumnView::Closed | ColumnView::Direct => (UiPanel::None, false),
            ColumnView::Lyrics => (UiPanel::Lyrics, false),
            ColumnView::Look => (UiPanel::None, true),
            ColumnView::Assist => (UiPanel::Assist, false),
            ColumnView::Render => (UiPanel::Export, false),
        };
        self.panel = panel;
        self.set_inspector_open(inspector);
        match view {
            ColumnView::Direct => self.dock_open = true,
            ColumnView::Closed => {
                self.dock_open = false;
                self.dock_keyboard = false;
            }
            _ => {}
        }
        self.ui_preferences.dock_open = self.dock_open;
    }

    /// Whether the state a top-bar tab names holds (WX-5c §4). `Look` can be
    /// selected together with one of the bottom panels; `Direct` only when
    /// neither is open. `UiPanel::Tune` is no bottom panel (see
    /// [`Self::timeline_height`]), so it counts as Direct.
    #[must_use]
    pub(crate) fn mode_tab_selected(&self, tab: ModeTab) -> bool {
        match tab {
            ModeTab::Direct => {
                matches!(self.panel, UiPanel::None | UiPanel::Tune) && !self.inspector_open
            }
            ModeTab::Lyrics => self.panel == UiPanel::Lyrics,
            ModeTab::Look => self.inspector_open,
            ModeTab::Export => self.panel == UiPanel::Export,
            ModeTab::Assist => self.panel == UiPanel::Assist,
        }
    }

    /// A press on a top-bar tab, through the transport's own paths: `Look`
    /// toggles the inspector as the Tune button does, and the three panels
    /// toggle behind the lyric-draft guard that every context change takes.
    /// `Direct` closes both, behind the same guard, since leaving the lyrics
    /// editor with a half-typed cue is a context change too.
    pub(crate) fn activate_mode_tab(&mut self, tab: ModeTab, workspace: &Workspace) {
        let panel = match tab {
            ModeTab::Look => {
                self.set_inspector_open(!self.inspector_open);
                return;
            }
            ModeTab::Direct => {
                if self.lyric_draft_allows_context_change(workspace) {
                    self.panel = UiPanel::None;
                    self.set_inspector_open(false);
                }
                return;
            }
            ModeTab::Lyrics => UiPanel::Lyrics,
            ModeTab::Export => UiPanel::Export,
            ModeTab::Assist => UiPanel::Assist,
        };
        if self.lyric_draft_allows_context_change(workspace) {
            self.toggle_panel(panel);
        }
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
            // No track means no boundaries to drag, but this is still the only
            // call that answers a cursor request, so it has to answer before
            // leaving. Previously nothing set the cursor at all on these frames,
            // which left whatever the last frame chose on screen.
            d.set_mouse_cursor(
                self.pointer_cursor
                    .unwrap_or(MouseCursor::MOUSE_CURSOR_DEFAULT),
            );
            return;
        }
        const HIT: f32 = 8.0;
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

        // The splitters own the cursor for the frame, and they run last, which is
        // why a panel cannot simply set it for itself (see [`Shell::request_cursor`]).
        // An active or hovered splitter outranks any request: its hit strip is
        // drawn over the panels, and a drag in flight must keep its own arrow even
        // as the pointer travels across a panel that would like a different one.
        let active = self.split_drag.or(hovered);
        d.set_mouse_cursor(match active {
            Some(SplitKind::Timeline) => MouseCursor::MOUSE_CURSOR_RESIZE_NS,
            Some(SplitKind::Inspector) => MouseCursor::MOUSE_CURSOR_RESIZE_EW,
            None => self
                .pointer_cursor
                .unwrap_or(MouseCursor::MOUSE_CURSOR_DEFAULT),
        });

        draw_splitter(d, timeline, SplitKind::Timeline, active);
        if let Some(rect) = inspector {
            draw_splitter(d, rect, SplitKind::Inspector, active);
        }
    }

    /// The tracks flyout's body (`tracks_panel`, `plug.c`): the four actions,
    /// then the track list, in `panel` (WX-5c §1).
    ///
    /// The widget ids are the sidebar panel's, so a press claimed by a probe
    /// or a hand names the same control it always did. The action row is the
    /// old panel's stacked form (two rows of two), which the flyout's 336 px
    /// always has room for, so the mode ladder that hid the panel at short
    /// windows has nothing left to decide.
    pub(crate) fn tracks_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        panel: UiRect,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let content = widgets::panel(d, input.fonts.ui(), panel, "TRACKS");
        // The save state, right-aligned in the panel's own title row (UX0-B01,
        // F1). The top bar carries it too from WX-5c on; here it sits beside
        // the Save it describes.
        self.tracks_save_state(d, panel, input);
        let row = UiRect::new(
            panel.x + metric::UI_PANEL_PADDING,
            panel.y + TRACKS_ACTION_TOP,
            panel.width - metric::UI_PANEL_PADDING * 2.0,
            TRACKS_ACTION_ROW * 2.0 + TRACKS_ACTION_GAP,
        );
        if !panel.contains(row) {
            return;
        }

        // The oracle's four, in its order (`action_labels`, `plug.c:5165-5166`).
        // There is no "Close": the frozen C cannot close a single track, and a
        // button for it would be an invented feature rather than parity.
        //
        // Save carries a `*` while the current track has work to write (UX0-B01).
        let current = input.workspace.current();
        let save_marked = current.is_some_and(|track| track.save_state().needs_attention());
        let unnamed = current.is_some_and(|track| track.project_path.is_none());
        let save_label = match (unnamed, save_marked) {
            (true, true) => "Keep this cut… *",
            (true, false) => "Keep this cut…",
            (false, true) => "Save *",
            (false, false) => "Save",
        };
        let labels: [&str; 4] = ["Open project", "Add audio", save_label, "Save As"];
        let columns = 2;
        let cell_width = (row.width - TRACKS_ACTION_GAP) * 0.5;
        let cell_height = TRACKS_ACTION_ROW;
        let widths = [cell_width; 4];
        let font = input.fonts.ui();
        let font_size = widgets::row_font_size(font, &labels, &widths, cell_height);

        for (index, label) in labels.iter().enumerate() {
            let column = index % columns;
            let line = index / columns;
            let boundary = UiRect::new(
                row.x + column as f32 * (cell_width + TRACKS_ACTION_GAP),
                row.y + line as f32 * (cell_height + TRACKS_ACTION_GAP),
                cell_width,
                cell_height,
            );
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
            if index == 2 {
                self.widgets
                    .hint(d, state, id, boundary, "Save the project [Ctrl+S]");
            }
            if state.clicked {
                match index {
                    // Both Opens make a *different* track current, so they are the
                    // same context change as clicking a track row (review 1.3).
                    0 | 1 if !self.lyric_draft_allows_context_change(input.workspace) => {}
                    0 => commands.push(ShellCommand::OpenProject),
                    1 => commands.push(ShellCommand::OpenAudio),
                    // Saving changes no context: it writes the track the draft
                    // already belongs to, and blocking it would be telling the
                    // user to discard work in order to save work.
                    2 => commands.push(ShellCommand::SaveProject),
                    _ => commands.push(ShellCommand::SaveProjectAs),
                }
                // An action that opens a dialog or writes a file is done with
                // the flyout; the list stays open for picking a track.
                self.open_flyout = None;
            }
        }

        // The track list, below the action row.
        let list_top = row.y + row.height + metric::UI_CONTROL_GAP;
        if !input.workspace.is_empty() {
            self.track_list(d, panel, input, commands, content, list_top);
        } else {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "no track open",
                panel.x + metric::UI_PANEL_PADDING,
                list_top,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }
    }

    /// The current track's save state, drawn right-aligned in the TRACKS header
    /// (UX0-B01, F1).
    ///
    /// # Why this exists at all
    ///
    /// `project_dirty` was maintained correctly for the whole life of this
    /// application and read by exactly two things: the quit modal and the probe
    /// report. So the answer to "is my work safe?" was available only *after* the
    /// user had decided to quit — and a user who never quits, or who loses the
    /// session to a crash, never got it. This is the continuously visible answer.
    ///
    /// # Why the reason is drawn and not just the word
    ///
    /// "Save failed" on its own is a silent amber dot with extra steps. The
    /// failure is latched — autosave will not retry until the next edit — so a
    /// user who cannot see *why* has no way to know whether to free disk space,
    /// fix a permission, or pick a different destination. When there is room the
    /// reason goes under the header; when there is not, the word still appears
    /// and the notice tray carries the sentence.
    fn tracks_save_state(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        panel: UiRect,
        input: &ShellInput<'_>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        let state = track.save_state();
        let font = input.fonts.ui();
        let label = state.label();
        let width = widgets::measure(font, label, metric::UI_FONT_CAPTION);
        let x = panel.x + panel.width - metric::UI_PANEL_PADDING - width;
        // Never over the "TRACKS" title: at a narrow sidebar the two would
        // overlap into an unreadable smear, and the title is the one that says
        // which panel this is.
        let title_end = panel.x
            + metric::UI_PANEL_PADDING
            + widgets::measure(font, "TRACKS", metric::UI_FONT_CAPTION)
            + 8.0;
        if x < title_end {
            return;
        }
        let tint = save_state_tint(state);
        widgets::draw_text(
            d,
            font,
            label,
            x,
            panel.y + 8.0,
            metric::UI_FONT_CAPTION,
            tint,
        );

        // The reason, when there is one and the panel is tall enough to hold a
        // line under the header without eating the action row.
        let Some(reason) = &track.project_save_error else {
            return;
        };
        let action_top = TRACKS_ACTION_TOP;
        let reason_y = panel.y + widgets::PANEL_HEADER_HEIGHT + 2.0;
        if reason_y + metric::UI_FONT_CAPTION > panel.y + action_top {
            return;
        }
        let available = panel.width - metric::UI_PANEL_PADDING * 2.0;
        for line in notice::wrap_detail(&save_error_summary(reason), available, 1, |text| {
            widgets::measure(font, text, metric::UI_FONT_CAPTION)
        }) {
            widgets::draw_text(
                d,
                font,
                &line,
                panel.x + metric::UI_PANEL_PADDING,
                reason_y,
                metric::UI_FONT_CAPTION,
                color::ui_danger(),
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
        panel: UiRect,
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
            panel.x,
            list_top,
            panel.width,
            (content.y + content.height - list_top).max(0.0),
        );
        if area.height <= 0.0 {
            return;
        }
        let metrics = ListMetrics::measure(panel.width, area.height, 0.0, count);

        let mouse = input.ui_scale.mouse(d);
        let over_panel = panel.contains_point(mouse.x, mouse.y);
        let input_allowed = !self.editor_open && !self.assist_settings.is_open();
        if over_panel && input_allowed {
            self.track_scroll.wheel(d.get_mouse_wheel_move(), &metrics);
        }

        // The thumb is measured before `advance` so that a drag reads the same
        // rectangle the user pressed on, and released before the rows are drawn.
        let bar_x = panel.x + panel.width - metrics.bar_width;
        if input_allowed {
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
        } else {
            self.track_scroll.end_drag();
        }

        let current = input.workspace.current_index();
        let row_width = metrics.row_width(panel.width);
        // Scissor mode is GL state, so drawing through the parent handle inside
        // the pair is still clipped; the handle type only enforces the begin/end
        // pairing. Opened once around the whole list rather than per row.
        let mut clip = widgets::begin_scissor(d, area, input.ui_scale);
        for (index, name) in input.workspace.display_names().enumerate() {
            let (row_y, row_height) = metrics.row_offset(index, self.track_scroll.offset());
            let top = area.y + row_y;
            // Outside, or too little of it left to read: no draw, and — the part
            // that matters — no widget id registered, so nothing off-screen can
            // claim the press.
            if !track_row_is_legible(top, row_height, area) {
                continue;
            }
            let boundary = UiRect::new(panel.x + metrics.padding, top, row_width, row_height);
            let selected = current == Some(index);
            // Offset past the action buttons above, so a track row and an action
            // never hash to the same id.
            let id = widgets::widget_id(widgets::id::TRACKS, 16 + index as u32);
            // A track name is the user's own words: the authored face carries
            // the glyphs the Latin-only chrome bank would draw as `?`
            // (review 1.5). When the chrome bank covers the name it draws it,
            // in the label size, as the top bar does with the same name — one
            // name in two faces a few pixels apart read as two designs.
            let chrome = musializer_runtime::font::AuthoredText::from_ui(input.fonts.ui());
            let (face, size) = if chrome.missing(name) == 0 {
                (chrome, metric::UI_FONT_LABEL)
            } else {
                (input.fonts.authored(), metric::UI_FONT_AUTHORED_ROW)
            };
            let state = self.widgets.text_button_in_authored(
                &mut clip,
                face,
                id,
                boundary,
                // The press is tested against the visible part only.
                boundary.intersect(area),
                name,
                selected,
                ButtonStyle::Neutral,
                Some(size),
            );
            // A state dot at the row's right edge (UX0-B01). The header names the
            // *current* track's state in words; this is the only thing that says
            // anything about the others, and all-track autosave (C4) is exactly a
            // claim about tracks the user is not looking at — so "track 3 still
            // has unsaved work" has to be visible without selecting track 3.
            //
            // Drawn only for the two states that need attention. A dot on every
            // row would be a column of decoration the eye stops reading, which is
            // how the amber-dot-for-everything design fails.
            if let Some(track) = input.workspace.get(index) {
                let save = track.save_state();
                if save.needs_attention() {
                    let radius = 3.0;
                    let centre_x = boundary.x + boundary.width - radius - 4.0;
                    let centre_y = boundary.y + boundary.height * 0.5;
                    // Inside the visible part only, for the same reason the press
                    // is: a dot painted where the row is clipped away is a mark
                    // floating in the panel's padding.
                    if area.contains_point(centre_x, centre_y) {
                        clip.draw_circle_v(
                            Vector2::new(centre_x, centre_y),
                            radius,
                            if save == SaveState::Failed {
                                color::ui_danger()
                            } else {
                                color::ui_warning()
                            },
                        );
                    }
                }
            }
            // review 1.3 (UX0-A03). The click used to push the command with no
            // guard at all, so a half-typed cue on this track became an edit
            // against the next one. The oracle guards the same click
            // (`plug.c:5263`), and the refusal is the panel's own words.
            if state.clicked && !selected && self.lyric_draft_allows_context_change(input.workspace)
            {
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

    /// `--ui-probe scene-family=`: browse a family, and open the scenes
    /// flyout so a capture shows what it browsed (WX-5c §1).
    ///
    /// The family holds until a family row is pressed. The probe stage runs
    /// before the first frame, so a plan that sets the scene on that frame
    /// would otherwise move the picker to the new scene's family — which is
    /// right for a hand and makes a capture of "browsing Geometry" show
    /// whatever the plan happened to play.
    pub fn browse_scene_family(&mut self, family: SceneFamily, active: SceneId) {
        self.scene_picker.observe_scene(active);
        self.scene_picker.browse(family);
        self.probe_family = Some(family);
        self.open_flyout = Some(Flyout::Scenes);
    }

    pub fn describe_scene_picker(&self) -> &str {
        &self.scene_picker_report
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
        // Where the playhead line is drawn, which follows a scrub before the
        // decoder does, so the warm part ends exactly under it.
        let playhead = self.timeline_playhead_seconds(input.time_seconds);
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
            // Grey ahead of the playhead, warm behind it (WX-5a, the mockup's
            // waveform). It used to be the accent, which made the largest
            // coloured area on screen the one thing that is never "now".
            let played = end <= playhead;
            let colour = widgets::alpha(
                if played {
                    color::ui_level()
                } else {
                    color::ui_waveform()
                },
                0.62 + peak * 0.38,
            );
            d.draw_line_ex(
                Vector2::new(x, centre - maximum * amplitude),
                Vector2::new(x, centre - minimum * amplitude),
                1.0,
                colour,
            );
        }
    }

    /// Rebuilds the merged event view only when it can have changed.
    ///
    /// Port of `combined_scene_events` (`plug.c:1085-1113`), including *why* it
    /// is cached: [`SceneEventMerge::build`] validates both lanes in full, copies
    /// them, qualifies every semantic id against the partial result and sorts —
    /// and a track may carry 2,048 events. Doing that inside the draw pass every
    /// frame is work the revision counters exist to avoid.
    ///
    /// The key carries the track index as well as the two revisions, which is the
    /// C's `p->scene_events_track`: two tracks can sit at the same revisions, and
    /// without it swapping tracks would draw the previous track's markers.
    fn refresh_timeline_events(&mut self, input: &ShellInput<'_>) {
        let key = input
            .workspace
            .current_index()
            .zip(input.workspace.current())
            .map(|(index, track)| {
                (
                    index,
                    track.manual_events.revision(),
                    track.semantic_events.revision(),
                )
            });
        if key == self.timeline_events_key {
            return;
        }
        self.timeline_events_key = key;
        let Some(track) = input.workspace.current() else {
            self.timeline_events.clear();
            return;
        };
        if self
            .timeline_events
            .build(track.manual_events.events(), track.semantic_events.events())
            .is_err()
        {
            // The C logs and empties (`plug.c:1104-1108`). Drawing a stale merge
            // would be worse than drawing none: the markers would claim times
            // that are no longer in the project.
            self.timeline_events.clear();
        }
    }

    /// The merged manual/semantic event markers, over the waveform lane (D4).
    ///
    /// Port of `plug.c:3086-3100`, drawn in the same place in the same order —
    /// after the waveform and the ticks, before the playhead — so a marker is
    /// never hidden by a gridline and never hides the playhead.
    ///
    /// **Two axes, and the oracle only draws one.** The C colours a marker by
    /// event *type* (`event_type_color`, `plug.c:1521-1530`) and says nothing
    /// about which lane it came from. That was survivable there and is not here,
    /// because the two axes genuinely cross: the manual event row's `+ Feel`
    /// button records [`EventType::Semantic`] into the **manual** lane
    /// (`plug.c:2897`), so an amber marker may be either. And after an Assist run
    /// the question a user actually has is *"did I put that there, or did the
    /// model?"* — the same question `CueOrigin` answers in the cue lane (LX1).
    ///
    /// So type keeps the colour, exactly as the C has it, and the lane is carried
    /// by the head: **a manual marker has a filled disc, a semantic one a hollow
    /// ring**, with the semantic line at lower alpha. Shape rather than a second
    /// colour, because a second colour would have to fight the four type colours
    /// for the same channel and would make a lyric marker and a cue marker
    /// indistinguishable — which is the information the C chose to show.
    /// [`SceneEventMerge::lanes`] is where the lane comes from; it cannot be
    /// recovered from a merged record, which is why that exists.
    ///
    /// Every marker carries a tooltip naming its lane, its type and its time,
    /// because a shape distinction that is never written down anywhere is a
    /// legend the user has to guess.
    fn event_markers(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        strip: UiRect,
        duration: f64,
    ) -> EventMarkerReport {
        use musializer_core::scene::events::{EventLane, UNKNOWN_EVENT_RGBA};
        use raylib::prelude::Color;

        let mut report = EventMarkerReport::default();
        let mouse = input.ui_scale.mouse(d);
        // Collected before drawing so the tooltip for the marker under the
        // pointer can be raised *after* every marker is painted, and outside the
        // scissor below — a tip clipped to the lane it explains would be cut in
        // half. Drawing a tip inside the loop would also let a later marker's
        // line paint over it.
        let mut hovered: Option<(UiRect, String, usize)> = None;

        // The clip the oracle opens around this whole block (`plug.c:3050`), and
        // it is load-bearing rather than tidy: the cull below admits a marker up
        // to 8 px outside the lane, deliberately, so a marker whose *line* is
        // just off-screen still shows the part of its head that belongs on
        // screen. Without the scissor that head — 5 px in every direction —
        // paints onto the panel background outside the lane instead. It is not
        // hypothetical: the 4x capture in the gate has an event exactly on the
        // right edge whose head measured 54 px against the usual 39, because it
        // was spilling over the border.
        let mut clip = widgets::begin_scissor(d, strip, input.ui_scale);

        for (index, (event, lane)) in self.timeline_events.iter_with_lane().enumerate() {
            // The C's own bounds: outside the track is not drawn at all, and a
            // marker more than 8 px past either end of the lane is skipped rather
            // than clamped to the edge (`plug.c:3089-3093`). Clamping would pile
            // every off-screen event onto the two edge columns and read as a
            // dense cluster that is not there — which is the off-screen boundary
            // case the scene lane already had to learn.
            if event.timestamp_seconds < 0.0 || event.timestamp_seconds > duration {
                report.off_track += 1;
                continue;
            }
            let x = self.timeline.x_at(
                event.timestamp_seconds,
                f64::from(strip.x),
                f64::from(strip.width),
            ) as f32;
            if x < strip.x - 8.0 || x > strip.x + strip.width + 8.0 {
                report.off_screen += 1;
                continue;
            }

            let rgba = event.kind().map_or(UNKNOWN_EVENT_RGBA, |kind| kind.rgba());
            let colour = Color::get_color(rgba);
            let line_alpha = match lane {
                // The C's 0.75 (`plug.c:3096`), kept for the manual lane so a
                // hand-placed marker looks exactly as it did.
                EventLane::Manual => 0.75,
                // Lower, because a derived marker should not out-shout one the
                // user placed. It is still well above the waveform it sits on.
                EventLane::Semantic => 0.45,
            };
            clip.draw_line_ex(
                Vector2::new(x, strip.y),
                Vector2::new(x, strip.y + strip.height),
                3.0,
                widgets::alpha(colour, line_alpha),
            );
            match lane {
                EventLane::Manual => {
                    clip.draw_circle_v(Vector2::new(x, strip.y), MARKER_HEAD_RADIUS, colour);
                    report.manual += 1;
                }
                EventLane::Semantic => {
                    // A ring, drawn as a filled disc in the lane's own surface
                    // punched out of a filled disc in the type colour. Two
                    // circles rather than `draw_circle_lines`, whose 1 px stroke
                    // vanishes against a busy envelope at the exact size where
                    // the distinction has to be readable.
                    clip.draw_circle_v(Vector2::new(x, strip.y), MARKER_HEAD_RADIUS, colour);
                    clip.draw_circle_v(
                        Vector2::new(x, strip.y),
                        MARKER_HEAD_RADIUS - MARKER_RING_THICKNESS,
                        color::ui_raised(),
                    );
                    report.semantic += 1;
                }
            }

            // The hit box is the head and a couple of pixels either side of the
            // line, not the whole lane column: the lane is also the scrub target
            // and the pan surface, and a tooltip that appeared over a third of
            // the strip would be in the way of both.
            let hit = UiRect::new(
                x - MARKER_HEAD_RADIUS,
                strip.y,
                MARKER_HEAD_RADIUS * 2.0,
                strip.height,
            );
            if hovered.is_none() && hit.contains_point(mouse.x, mouse.y) {
                let kind = event.kind().map_or("unknown", |kind| match kind {
                    musializer_core::scene::events::EventType::Lyric => "lyric",
                    musializer_core::scene::events::EventType::Semantic => "feel",
                    musializer_core::scene::events::EventType::Cue => "cue",
                    musializer_core::scene::events::EventType::Custom => "custom",
                });
                hovered = Some((
                    hit,
                    format!(
                        "{} {kind}  \u{00b7}  {}",
                        lane.label(),
                        widgets::format_timestamp(event.timestamp_seconds)
                    ),
                    index,
                ));
            }
        }

        // Closed before the tooltip: a tip clipped to the lane it explains would
        // be cut off at the lane's own edge, which is worst for exactly the
        // markers nearest the edges.
        drop(clip);

        if let Some((anchor, text, index)) = hovered {
            report.hovered = Some(text.clone());
            self.widgets.hint(
                d,
                widgets::ButtonState {
                    hovered: true,
                    clicked: false,
                    pressed: false,
                },
                widgets::widget_id(widgets::id::TIMELINE_EVENTS, index as u32),
                anchor,
                &text,
            );
        }
        report
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
        // Reset before any early return, so a frame that drew no strip reports no
        // markers rather than last frame's. A stale count is exactly the kind of
        // evidence that reads as working while the surface is blank.
        self.timeline_event_markers = EventMarkerReport::default();
        if band.is_empty() {
            return;
        }
        self.refresh_timeline_events(input);
        let content = widgets::panel(d, input.fonts.ui(), band, "TIMELINE");
        // The band's gestures, said where they happen (the mockup's band head,
        // WX-5c). Only while the timecode is inline: when it is not, this
        // corner is the timecode's.
        if toolbar.timecode_inline {
            let hint =
                "WHEEL ZOOM \u{00b7} SHIFT PAN \u{00b7} DOUBLE-CLICK A RUN TO OPEN ITS SECTIONS";
            let font = input.fonts.ui();
            let width = widgets::measure_tracked(font, hint, metric::UI_FONT_MICRO, 1.0);
            let x = band.x + band.width - metric::UI_PANEL_PADDING - width;
            let title_end = band.x + metric::UI_PANEL_PADDING + 80.0;
            if x > title_end {
                widgets::draw_text_tracked(
                    d,
                    font,
                    hint,
                    x,
                    band.y + 9.0,
                    metric::UI_FONT_MICRO,
                    1.0,
                    color::ui_disabled(),
                );
            }
        }
        // The timecode's fallback home, when the toolbar's band could not seat it
        // beside the transport buttons (`timeline_layout.h:42-44`). Right-aligned
        // in this panel's header, where nothing else is drawn.
        if !toolbar.timecode_inline {
            let shown_time = self.timeline_playhead_seconds(input.time_seconds);
            let timecode = format!(
                "{} / {}",
                widgets::format_timestamp(shown_time),
                widgets::format_timestamp(input.duration_seconds)
            );
            let font = input.fonts.ui();
            let width = widgets::measure_tabular(font, &timecode, metric::UI_FONT_VALUE);
            widgets::draw_text_tabular(
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
        let mouse = input.ui_scale.mouse(d);

        // A wheel event is captured only inside a timed lane's own rect — the
        // scene-plan lane and the PCM lane below, the cue lane in the lyrics
        // panel — so it cannot steal scrolling from the lyric list that shares
        // the band with them. Applying the captured mutation here, one frame
        // later, means scene, PCM and lyric lanes all consume the new view
        // together instead of tearing for one frame.
        match self.timeline_zoom_pending.take() {
            Some(TimelineWheel::Zoom {
                factor,
                anchor_seconds,
            }) => self.timeline.zoom(duration, factor, anchor_seconds),
            Some(TimelineWheel::Pan { delta_seconds }) => {
                self.timeline.pan(duration, delta_seconds);
            }
            None => {}
        }

        // Follow before *any* timed lane draws. The old order mutated the view
        // after the scene lane, waveform, ticks and playhead had consumed it,
        // then let the lyric lane below consume the new state in the same frame.
        // Apart from cross-lane disagreement, the marker saw a variable one-frame
        // overshoot and visibly flickered while the content scrolled beneath it.
        if input.playing && self.timeline_gesture.is_none() && !self.timeline_manual_view {
            self.timeline.reveal(duration, input.time_seconds);
        }

        // The manual event row was here (`plug.c:2861-2971`). Since WX-5c its
        // markers are chips on the lane head, the scene section's first row,
        // so the section starts at the top of the content.
        let scene_row = self.scene_plan_section(
            d,
            input,
            UiRect::new(
                content.x + padding,
                content.y + padding,
                (content.width - padding * 2.0).max(0.0),
                (content.height - padding * 2.0).max(0.0),
            ),
            commands,
        );
        let strip = UiRect::new(
            content.x + padding,
            content.y + padding + scene_row,
            (content.width - padding * 2.0).max(0.0),
            56.0f32.min((content.height - padding * 2.0 - scene_row).max(0.0)),
        );
        if strip.is_empty() {
            return;
        }
        // Where the scene-plan lane actually ended, when it drew at all (LX1-e).
        // `scene_plan_section` answers 0.0 when it could not seat its controls,
        // and then the waveform *is* the first lane and there is no seam above
        // it. Both edges are taken from the section's own constants rather than
        // assumed to be `strip.y - LANE_GAP`: a seam derived from the gap it
        // *should* be would keep painting a tidy 5 px band over the lane's
        // bottom border if the section's budget ever drifted, which is exactly
        // the failure the negative control for this tranche produced.
        let scene_lane_bottom = (scene_row > 0.0).then_some(
            content.y
                + padding
                + scene_timeline::SCENE_LANE_OFFSET
                + scene_timeline::SCENE_LANE_HEIGHT,
        );
        let lanes_top =
            scene_lane_bottom.map_or(strip.y, |bottom| bottom - scene_timeline::SCENE_LANE_HEIGHT);
        d.draw_rectangle_rec(widgets::rectangle(strip), color::ui_raised());
        d.draw_rectangle_lines_ex(
            widgets::rectangle(strip),
            metric::LANE_BORDER,
            color::ui_rule(),
        );
        self.timeline_pan_gesture(d, input, strip);

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
            let zoom_row_y = self.open_panel(d, input, content, strip, commands);
            self.timeline_group_chrome(
                d,
                lanes_top,
                scene_lane_bottom,
                strip,
                zoom_row_y,
                &[],
                input.time_seconds,
            );
            return;
        }

        // The common view was already zoomed before any lane drew. This region
        // remains the left-button seek target.
        let over_strip = mouse.x >= strip.x
            && mouse.x <= strip.x + strip.width
            && mouse.y >= strip.y
            && mouse.y <= strip.y + strip.height;
        let wheel = self.wheel_delta(d);
        if over_strip {
            self.request_timeline_zoom(wheel, mouse.x, strip.x, strip.width, duration);
        }
        // The scene-plan lane zooms on the same notch (operator request,
        // 2026-08-06): it is a view of this axis too, and the wheel working over
        // the waveform but not over the blocks above it was an accident of which
        // lane was built first.
        //
        // The region is the lane's own rect and nothing more. Its x range is
        // `strip`'s because `scene_plan_section` is handed the same
        // `content.x + padding` and `content.width - padding * 2` the strip is
        // built from — that shared pair is why the two lanes align at all — and
        // its vertical bounds are the section's own constants, already resolved
        // into `lanes_top` and `scene_lane_bottom` above. Staying strictly inside
        // the lane keeps the wheel away from the controls row above it, which has
        // buttons, and from the lyric list below, whose scroll it would otherwise
        // steal.
        if let Some(scene_bottom) = scene_lane_bottom {
            let over_scene_lane = mouse.x >= strip.x
                && mouse.x <= strip.x + strip.width
                && mouse.y >= lanes_top
                && mouse.y <= scene_bottom;
            if over_scene_lane {
                self.request_timeline_zoom(wheel, mouse.x, strip.x, strip.width, duration);
            }
        }
        self.waveform_lane(d, input, strip, duration);

        // Left-drag seek is shared with scene-body dragging. Update before the
        // marker draws so the preview follows the hand in this frame, while the
        // decoder still receives exactly one seek on release.
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;
        if over_strip
            && self.timeline_gesture.is_none()
            && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
        {
            self.begin_timeline_scrub(input.playing, commands);
        }
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.update_timeline_scrub(
                mouse.x,
                strip,
                duration,
                d.is_mouse_button_down(MOUSE_BUTTON_LEFT),
                commands,
            );
        }

        // Ticks from the ladder, chosen from the visible span rather than the
        // track length — picking it from the length left a zoomed-in window with
        // no label in it at all (`timeline_view.h:76-78`).
        //
        // Collected as well as drawn since LX1-e: the same columns are carried
        // through the seams between lanes by `timeline_group_chrome`, and a
        // second loop there could drift from this one.
        let mut ticks: Vec<f32> = Vec::new();
        let step = timeline_view::tick_step(self.timeline.span_seconds);
        if step > 0.0 {
            let first = (self.timeline.start_seconds / step).floor() * step;
            let mut tick = first;
            while tick <= self.timeline.start_seconds + self.timeline.span_seconds {
                let x = self
                    .timeline
                    .x_at(tick, f64::from(strip.x), f64::from(strip.width))
                    as f32;
                // Ticks live strictly *inside* the lane's border ring, and both
                // bounds are a fix rather than a tidy-up (LX1-e). A lane whose
                // box spans columns `[x, x + width)` carries its rules on
                // `x` and `x + width - 1`, and the old bounds were the closed
                // interval `[x, x + width]`:
                //
                // - the tick at `x + width` painted a rule one column *outside*
                //   the lane and dragged a clipped timestamp label off the edge
                //   with it;
                // - the tick at `x` — always present when the view starts on a
                //   tick, which is every unzoomed track — sat under the left
                //   border, invisible at 100 % and one column outside it at
                //   150 %, where a 1 px logical line straddles two columns.
                //
                // Both were found by measurement rather than by eye:
                // `tools/timeline_lane_alignment.py` reads the outermost rule
                // column of every lane, and the waveform's disagreed with the
                // other two by exactly one column at each end.
                if x >= strip.x + metric::LANE_BORDER
                    && x <= strip.x + strip.width - metric::LANE_BORDER
                {
                    ticks.push(x);
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
                        let label = widgets::format_timestamp(tick);
                        let font = input.fonts.ui();
                        let label_width = widgets::measure(font, &label, metric::UI_FONT_CAPTION);
                        let label_x = x + 4.0;
                        let label_y = strip.y + 4.0;
                        // An opaque plate under the label, which is the oracle's
                        // own fix and its own geometry (`plug.c:3065-3080`:
                        // `-3, -2, +6, +4` around the text box, in
                        // `COLOR_UI_RAISED`). The port drew the label and dropped
                        // the plate.
                        //
                        // The C's comment is the argument and it is a measurement
                        // rather than a preference: the waveform behind these
                        // labels is not a constant background, it runs from the
                        // raised surface in a silent passage to dense accent blue
                        // at full amplitude, where muted ink measures about
                        // 1.16:1. A plate makes the pairing fixed, so the label is
                        // legible over the loudest bar in the track instead of
                        // only over the quiet ones — and *which* bars are loud is
                        // a property of the audio, so without it the defect
                        // appears and disappears as the user scrolls.
                        let plate = UiRect::new(
                            label_x - 3.0,
                            label_y - 2.0,
                            label_width + 6.0,
                            metric::UI_FONT_CAPTION + 4.0,
                        );
                        // Skipped rather than clipped when it would cross the
                        // lane's inner edge. The C lets a late label run off the
                        // strip; here the plate would also paint over the lane's
                        // right border column, and
                        // `tools/timeline_lane_alignment.py` reads exactly that
                        // column to prove the three lanes share one axis — so an
                        // unclipped plate would break the check that guards the
                        // rest of this band. Dropping the label is the same
                        // decision the tick bounds above already make.
                        if plate.x + plate.width <= strip.x + strip.width - metric::LANE_BORDER {
                            d.draw_rectangle_rec(widgets::rectangle(plate), color::ui_raised());
                            widgets::draw_text(
                                d,
                                font,
                                &label,
                                label_x,
                                label_y,
                                metric::UI_FONT_CAPTION,
                                // Full ink, not muted: the plate is what the C
                                // pairs `COLOR_UI_INK` against (`plug.c:3081`),
                                // and a fixed opaque backing is precisely the
                                // condition under which the stronger ink is the
                                // right choice rather than a shouty one.
                                color::ui_ink(),
                            );
                        }
                    }
                }
                tick += step;
            }
        }

        // Markers after the ticks and before the playhead, which is the oracle's
        // order (`plug.c:3086-3111`): a gridline must not cross a marker head,
        // and the playhead must stay the topmost thing in the lane because it is
        // the only one of the three the user is moving.
        self.timeline_event_markers = self.event_markers(d, input, strip, duration);

        // Protocol markers ride the same strip, bottom-anchored where the
        // event lollipops are top-anchored — position is the channel that
        // survives at 5 px, which is CX-1's ruling applied here (HX-2).
        if let Some(session) = &self.protocol {
            super::protocol::draw_markers(
                d,
                session,
                &self.timeline,
                strip,
                duration,
                input.ui_scale,
            );
        }

        // The open panel draws **before** the zoom row now, because it is the
        // panel that says where that row goes (LX1). The readout, the nudge-key
        // hint and the Zoom out button then land under every timed lane rather
        // than between the waveform and the cue lane.
        //
        // Draw order is safe in the one direction that matters: the row lands in
        // a gap the panel deliberately left empty, so the panel's widgets claim
        // their presses first and none of them overlaps the button below.
        let zoom_row_y = self.open_panel(d, input, content, strip, commands);

        // Everything that makes the lanes one instrument rather than three, and
        // it has to run here because `zoom_row_y` is the first moment the bottom
        // of the last lane is known (LX1-e).
        self.timeline_group_chrome(
            d,
            lanes_top,
            scene_lane_bottom,
            strip,
            zoom_row_y,
            &ticks,
            input.time_seconds,
        );

        // The zoom readout, so "why is the strip not the whole track" has an
        // answer on screen.
        let zoom_label = if self.timeline.is_whole(duration) {
            "whole track".to_string()
        } else {
            let mode = if self.timeline_manual_view {
                "  ·  free view"
            } else {
                ""
            };
            format!(
                "{:.1}x{mode}  ({} - {})",
                duration / self.timeline.span_seconds,
                widgets::format_timestamp(self.timeline.start_seconds),
                widgets::format_timestamp(self.timeline.start_seconds + self.timeline.span_seconds)
            )
        };
        widgets::draw_text_tabular(
            d,
            input.fonts.ui(),
            &zoom_label,
            strip.x,
            zoom_row_y + 4.0,
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
        let zoom_width =
            widgets::measure_tabular(input.fonts.ui(), &zoom_label, metric::UI_FONT_CAPTION);
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
                zoom_row_y + 4.0,
                metric::UI_FONT_CAPTION,
                color::ui_disabled(),
            );
        }

        let reset = UiRect::new(strip.x + strip.width - 84.0, zoom_row_y + 2.0, 84.0, 22.0);
        if content.contains(reset) {
            let reset_label = if self.timeline_manual_view {
                "Follow"
            } else {
                "Zoom out"
            };
            let id = widgets::widget_id(widgets::id::TIMELINE, 1);
            if self
                .widgets
                .text_button(
                    d,
                    input.fonts.ui(),
                    id,
                    reset,
                    reset_label,
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                if self.timeline_manual_view {
                    self.timeline_manual_view = false;
                    self.timeline.reveal(duration, input.time_seconds);
                } else {
                    self.reset_timeline(duration);
                }
            }
        }
    }

    /// The chrome that turns the stacked timed lanes into one instrument
    /// (LX1-e).
    ///
    /// The operator's complaint was that the band read as "three separately
    /// designed elements glued together", and it was three separate designs:
    /// the scene lane sat 4 px under its controls row inside a 1 px box, the
    /// waveform strip's box began on the *very next row* below it — so two 1 px
    /// rules drew as one 2 px line — and the lyric cue lane sat 5 px lower with
    /// a top rule, no sides and no bottom. Three playheads crossed them at two
    /// different widths with a break in every gap.
    ///
    /// The system is three numbers ([`metric::LANE_BORDER`],
    /// [`metric::LANE_GAP`], [`metric::LANE_PLAYHEAD_WIDTH`]) plus this
    /// function, which draws what no single lane can:
    ///
    /// - **the seams.** Every gap between two adjacent lanes is filled with
    ///   [`color::ui_lane_trough`] and carries the tick columns through, so the
    ///   band reads as rows of one table rather than as boxes on a background.
    /// - **the frame.** One outline around all of them. It supplies the cue
    ///   lane's missing left, right and bottom edges without `panels::lyrics`
    ///   having to draw a box of its own, and it is what makes the shared inset
    ///   visible: the same two columns bound every lane.
    /// - **the playhead.** One marker, one width, one handle, crossing every
    ///   lane and every seam without a break.
    ///
    /// It runs last because `zoom_row_y` — the bottom of the last lane — is not
    /// known until the open panel has drawn. Everything here is chrome over
    /// content that is already on screen; nothing in it claims a press.
    #[allow(
        clippy::too_many_arguments,
        reason = "every argument is one edge of the group, and bundling them into a struct \
                  would only move the same list to its construction site"
    )]
    fn timeline_group_chrome(
        &self,
        d: &mut RaylibDrawHandle<'_>,
        lanes_top: f32,
        scene_lane_bottom: Option<f32>,
        strip: UiRect,
        lanes_bottom: f32,
        ticks: &[f32],
        time_seconds: f64,
    ) {
        let group = UiRect::new(
            strip.x,
            lanes_top,
            strip.width,
            (lanes_bottom - lanes_top).max(0.0),
        );
        if group.is_empty() {
            return;
        }

        // A seam above the strip only when a lane actually drew above it, and
        // one below only when a panel reported a lane of its own down there.
        // The upper seam is bounded by the lane that ended, not by
        // [`metric::LANE_GAP`], so a wrong gap shows up as a wrong seam instead
        // of being painted over. The lower one has to be derived — the cue lane
        // belongs to `panels::lyrics` — which is what the compile-time
        // assertion beside [`PlayheadGeometry`] exists to keep honest.
        let below_strip = strip.y + strip.height;
        let seams = [
            scene_lane_bottom.map(|bottom| (bottom, strip.y)),
            (lanes_bottom > below_strip).then_some((below_strip, below_strip + metric::LANE_GAP)),
        ];
        for (top, bottom) in seams.into_iter().flatten() {
            if bottom <= top || top < lanes_top || bottom > lanes_bottom {
                continue;
            }
            let seam = UiRect::new(group.x, top, group.width, bottom - top);
            widgets::fill(d, seam, color::ui_lane_trough());
            for &x in ticks {
                if x >= group.x && x <= group.x + group.width {
                    d.draw_line_ex(
                        Vector2::new(x, top),
                        Vector2::new(x, bottom),
                        metric::LANE_BORDER,
                        color::ui_rule(),
                    );
                }
            }
        }

        draw_timeline_playhead(
            d,
            self.timeline,
            group,
            self.timeline_playhead_seconds(time_seconds),
            metric::LANE_PLAYHEAD_WIDTH,
            true,
        );
        d.draw_rectangle_lines_ex(
            widgets::rectangle(group),
            metric::LANE_BORDER,
            color::ui_rule(),
        );
    }

    /// Dispatches to whichever bottom panel is open, and reports where the zoom
    /// row goes.
    ///
    /// One `match` in one place, so an agent fills a function in their own file
    /// and never edits this one. [`UiPanel::Tune`] is absent because the tuning
    /// controls are the right-hand inspector, not a bottom panel.
    ///
    /// The return value is the y the zoom readout, the nudge-key hint and the
    /// Zoom out button draw at (LX1). Every panel but Lyrics answers "flush
    /// against the strip", which is where that row has always been; the lyrics
    /// editor answers with the bottom of its cue lane, so the readout lands
    /// *under* all three timed lanes instead of cutting between two of them.
    /// Returned rather than assumed because only the panel knows how tall its
    /// lane ended up — and since LX1 the cue lane is resizable, so the answer is
    /// not even constant within one panel.
    fn open_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let below_strip = strip.y + strip.height;
        let placement = self.placement();
        match self.panel {
            // The cue lane outlives the editor (D4). Only in these two states:
            // Export and Assist take the whole band for their own budgets, and
            // adding a lane to those would push their bodies down rather than
            // into slack. It draws in room the band already reserves, so no
            // panel height moves — see `closed_lyric_lane`.
            UiPanel::None | UiPanel::Tune => {
                self.closed_lyric_lane(d, input, content, strip, commands)
            }
            UiPanel::Export => {
                self.export_panel(d, input, content, strip, commands, placement);
                below_strip
            }
            UiPanel::Lyrics => self.lyrics_panel(d, input, content, strip, commands, placement),
            UiPanel::Assist => {
                self.assist_panel(d, input, content, strip, commands, placement);
                below_strip
            }
        }
    }

    /// The notice tray, over the preview's bottom-left corner
    /// (`notice_tray`, `plug.c`).
    ///
    /// A card is a floating surface ([`widgets::floating_surface`]): the
    /// appearance's raised surface, a hairline, the 4 px corner and a soft
    /// shadow, the same object as a flyout or a tooltip (WX-5 follow-up). Its
    /// head is a dot and a micro, tracked label, the way the top bar's save
    /// state speaks; Info is neutral, and nothing on it wears the accent. Its
    /// colours come from [`super::theme::Palette::notice_severity`] and
    /// [`super::theme::Palette::float_surface`], which the palette sweep walks in both
    /// appearances. Why the card follows the appearance rather than staying a
    /// fixed dark card: `docs/DIVERGENCES.md`, "Notice card".
    ///
    /// Three things here are review 1.11 (UX0-A11) rather than the oracle, and
    /// they stay:
    ///
    /// - **The card is opaque.** At alpha 232 the colour the user read was the
    ///   card composited over whatever the scene drew that frame, so no ratio
    ///   measured against it was true.
    /// - **Detail text wraps, and the card grows to it.** It was one unwrapped,
    ///   unclipped line on a 380 px card, while real detail strings run 90-150+
    ///   characters — so a failure message ran across the preview and off the
    ///   window. [`notice::wrap_detail`] caps it and ellipsizes what is left, and
    ///   the card is sized from the lines that came back.
    /// - **Every card carries a close box**, keyed by the notice's id, because
    ///   [`Severity::Error`] notices are persistent and a notice that never
    ///   expires needs a way out that is not "restart the application".
    fn notice_tray(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        scale: f32,
        preview: UiRect,
    ) {
        self.notices_drawn = NoticeTrayReport {
            scale,
            region: preview,
            queued: self.notices.len(),
            cards: Vec::new(),
        };
        if preview.is_empty() || self.notices.is_empty() {
            return;
        }
        let width = NOTICE_CARD_WIDTH.min(preview.width - NOTICE_MARGIN * 2.0);
        if width <= 0.0 {
            return;
        }
        let text_width = width - NOTICE_PADDING * 2.0;
        if text_width <= NOTICE_CLOSE_SIZE {
            return;
        }
        let measure = |text: &str, size: f32| widgets::measure(font, text, size);

        let mut bottom = preview.y + preview.height - NOTICE_MARGIN;
        let mut dismissed = None;
        // Newest last in the queue, so draw from the end upward: the most recent
        // notice sits closest to the bottom edge where the eye already is.
        for notice in self.notices.notices().iter().rev() {
            let detail =
                notice::wrap_detail(&notice.detail, text_width, NOTICE_DETAIL_MAX_LINES, {
                    |text| measure(text, metric::UI_FONT_CAPTION)
                });
            let height = notice_card_height(detail.len());
            let y = bottom - height;
            // The tray never climbs past the top of the preview: a card drawn over
            // the toolbar would sit on controls it does not own.
            if y < preview.y {
                break;
            }
            let boundary = UiRect::new(preview.x + NOTICE_MARGIN, y, width, height);
            widgets::floating_surface(d, boundary);

            // The head: a dot and the severity, small and tracked.
            let tone = color::notice_severity(notice.severity);
            let dot = Vector2::new(
                boundary.x + NOTICE_PADDING + NOTICE_DOT_SIZE * 0.5,
                boundary.y + NOTICE_HEAD_TOP + metric::UI_FONT_MICRO * 0.5,
            );
            d.draw_circle_v(dot, NOTICE_DOT_SIZE * 0.5, tone);
            widgets::draw_text_tracked(
                d,
                font,
                notice.severity.label(),
                boundary.x + NOTICE_PADDING + NOTICE_DOT_SIZE + NOTICE_DOT_GAP,
                boundary.y + NOTICE_HEAD_TOP,
                metric::UI_FONT_MICRO,
                NOTICE_LABEL_TRACKING,
                tone,
            );
            // The title is bounded too. It is capacity-limited to 79 bytes, which
            // is still wider than the card at 14 px.
            for line in notice::wrap_detail(&notice.title, text_width, 1, {
                |text| measure(text, metric::UI_FONT_LABEL)
            }) {
                widgets::draw_text(
                    d,
                    font,
                    &line,
                    boundary.x + NOTICE_PADDING,
                    boundary.y + NOTICE_TITLE_TOP,
                    metric::UI_FONT_LABEL,
                    color::ui_ink(),
                );
            }
            for (index, line) in detail.iter().enumerate() {
                widgets::draw_text(
                    d,
                    font,
                    line,
                    boundary.x + NOTICE_PADDING,
                    boundary.y + NOTICE_DETAIL_TOP + index as f32 * NOTICE_DETAIL_LINE,
                    metric::UI_FONT_CAPTION,
                    color::ui_muted(),
                );
            }

            // The close box, a ghost control: its glyph in muted ink until the
            // pointer is on it, then the hover fill under ink. Keyed by the
            // notice's own id rather than by its position in the list, so a
            // press claimed on one card cannot be released by whichever card
            // inherited its index after an eviction.
            let close = UiRect::new(
                boundary.x + boundary.width - NOTICE_CLOSE_INSET - NOTICE_CLOSE_SIZE,
                boundary.y + NOTICE_CLOSE_INSET,
                NOTICE_CLOSE_SIZE,
                NOTICE_CLOSE_SIZE,
            );
            let id = widgets::widget_id(widgets::id::NOTICE, notice.id as u32);
            let state = self.widgets.button(d, id, close);
            let look = widgets::button_look(ButtonStyle::Ghost, false, state.hovered);
            if state.hovered {
                widgets::control_box(d, close, look.fill, look.edge, 1.0);
            }
            let inset = (NOTICE_CLOSE_SIZE - NOTICE_CLOSE_GLYPH) * 0.5;
            d.draw_line_ex(
                Vector2::new(close.x + inset, close.y + inset),
                Vector2::new(
                    close.x + close.width - inset,
                    close.y + close.height - inset,
                ),
                1.5,
                look.ink,
            );
            d.draw_line_ex(
                Vector2::new(close.x + close.width - inset, close.y + inset),
                Vector2::new(close.x + inset, close.y + close.height - inset),
                1.5,
                look.ink,
            );
            self.widgets.hint(d, state, id, close, "Dismiss");
            if state.clicked {
                dismissed = Some(notice.id);
            }
            self.notices_drawn.cards.push(DrawnNotice {
                id: notice.id,
                severity: notice.severity,
                card: boundary,
                dot,
                close,
            });

            bottom = y - NOTICE_GAP;
        }
        // Applied after the loop: the list is borrowed while it is being drawn.
        if let Some(id) = dismissed {
            self.notices.dismiss(id);
        }
    }

    /// The `notices:` report line: what the tray drew in the last frame.
    ///
    /// `none` when the queue is empty. Otherwise the tray's region, how many
    /// notices are queued and how many fitted, then each drawn card newest
    /// first (bottom to top): its id, severity, card rect, dot centre and
    /// close box, in logical pixels at `scale=`. A capture check addresses a
    /// card by these rather than by a pixel measured once.
    #[must_use]
    pub fn describe_notices(&self) -> String {
        let report = &self.notices_drawn;
        if report.queued == 0 {
            return "none".to_owned();
        }
        let at = |area: UiRect| {
            format!(
                "{:.0},{:.0},{:.0},{:.0}",
                area.x, area.y, area.width, area.height
            )
        };
        let cards: Vec<String> = report
            .cards
            .iter()
            .map(|card| {
                format!(
                    "id={} {} card={} dot={:.1},{:.1} close={}",
                    card.id,
                    card.severity.label().to_lowercase(),
                    at(card.card),
                    card.dot.x,
                    card.dot.y,
                    at(card.close),
                )
            })
            .collect();
        format!(
            "queued={} drawn={} scale={:.2} region={} cards=[{}]",
            report.queued,
            report.cards.len(),
            report.scale,
            at(report.region),
            cards.join("; "),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackAttention {
    Warning { unfiled: bool },
    Failure,
}

const FULLSCREEN_ATTENTION_EXPAND_SECONDS: f64 = 2.5;
/// The expanded fullscreen pill: 4/8 rhythm, the save dot's size.
const ATTENTION_PILL_HEIGHT: f32 = 24.0;
const ATTENTION_PILL_PAD: f32 = 10.0;
const ATTENTION_DOT: f32 = 7.0;
const ATTENTION_DOT_GAP: f32 = 7.0;

fn fullscreen_attention_expanded(since: f64, now: f64) -> bool {
    now - since < FULLSCREEN_ATTENTION_EXPAND_SECONDS
}

impl TrackAttention {
    fn from_track(track: &crate::workspace::Track) -> Option<Self> {
        match track.save_state() {
            SaveState::UnfiledWork => Some(Self::Warning { unfiled: true }),
            SaveState::Unsaved => Some(Self::Warning { unfiled: false }),
            SaveState::Failed => Some(Self::Failure),
            SaveState::NoProjectFile | SaveState::Saved => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Warning { unfiled: true } => "Unfiled work · Ctrl+S",
            Self::Warning { unfiled: false } => "Working changes · Ctrl+S",
            Self::Failure => "Save failed · Ctrl+S",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Warning { .. } => color::ui_warning(),
            Self::Failure => color::ui_danger(),
        }
    }

    fn expanded_token(self) -> &'static str {
        match self {
            Self::Warning { unfiled: true } => "unfiled-expanded",
            Self::Warning { unfiled: false } => "changes-expanded",
            Self::Failure => "failure-expanded",
        }
    }

    fn dot_token(self) -> &'static str {
        match self {
            Self::Warning { unfiled: true } => "unfiled-dot",
            Self::Warning { unfiled: false } => "changes-dot",
            Self::Failure => "failure-dot",
        }
    }
}

/// Widest a notice card is drawn.
const NOTICE_CARD_WIDTH: f32 = 380.0;
/// The tray's inset from the preview's left and bottom edges, and the room
/// between cards. On the 4/8 rhythm the top bar and the flyouts keep.
const NOTICE_MARGIN: f32 = 16.0;
const NOTICE_GAP: f32 = 8.0;
const NOTICE_PADDING: f32 = 12.0;
/// The head row: the dot and the micro severity label, at this top.
const NOTICE_HEAD_TOP: f32 = 10.0;
const NOTICE_DOT_SIZE: f32 = 6.0;
const NOTICE_DOT_GAP: f32 = 6.0;
/// The micro heads' tracking, as the panel heads draw it.
const NOTICE_LABEL_TRACKING: f32 = 1.0;
/// The title's top inside the card, under the head row.
const NOTICE_TITLE_TOP: f32 = 27.0;
const NOTICE_DETAIL_TOP: f32 = 47.0;
const NOTICE_DETAIL_LINE: f32 = 16.0;
/// The card grows to its wrapped detail, but only this far.
///
/// Three lines is about 160 characters at this width, which covers the detail
/// strings the application actually produces; past that the tray would start
/// taking the preview it floats over, and the ellipsis says so honestly.
const NOTICE_DETAIL_MAX_LINES: usize = 3;
const NOTICE_BOTTOM_PADDING: f32 = 9.0;
/// The close box sits in the head row's corner; the title and the detail run
/// under it, so the box costs the text no width.
const NOTICE_CLOSE_SIZE: f32 = 20.0;
const NOTICE_CLOSE_INSET: f32 = 6.0;
const NOTICE_CLOSE_GLYPH: f32 = 8.0;

/// A notice card's height for `detail_lines` lines of wrapped detail.
fn notice_card_height(detail_lines: usize) -> f32 {
    if detail_lines == 0 {
        NOTICE_TITLE_TOP + metric::UI_FONT_LABEL + NOTICE_BOTTOM_PADDING + 2.0
    } else {
        NOTICE_DETAIL_TOP + detail_lines as f32 * NOTICE_DETAIL_LINE + NOTICE_BOTTOM_PADDING - 3.0
    }
}

/// What the notice tray drew in one frame, for [`Shell::describe_notices`].
#[derive(Clone, Debug, Default)]
struct NoticeTrayReport {
    scale: f32,
    region: UiRect,
    queued: usize,
    cards: Vec<DrawnNotice>,
}

#[derive(Clone, Copy, Debug)]
struct DrawnNotice {
    id: u64,
    severity: Severity,
    card: UiRect,
    dot: Vector2,
    close: UiRect,
}

/// The least of a track row that may still be drawn, as a fraction of its height
/// (review 1.12, UX0-A12).
///
/// Rows are **clipped, not skipped** — that policy is right and it stays: a
/// skipped row leaves a gap the list cannot explain, and a row half in view is
/// how a scrolling list says "there is more". What the policy did not cover is
/// the sliver. At 1280x720 with the sheet expanded the *selected* track's row
/// came out cut through the middle of its glyphs, which reads as a rendering
/// fault rather than as a list that continues, and it was the one row whose
/// identity the user most needed.
///
/// 0.6 rather than 0.5, because a row is a text line centred vertically in its
/// box: at exactly half the cut lands on the glyphs. Three fifths keeps a whole
/// text line inside the visible part at every row height the list uses.
const TRACK_ROW_MINIMUM_VISIBLE: f32 = 0.6;

/// Whether enough of a row is inside `area` to be worth drawing.
///
/// The threshold yields to the area itself: a list region shorter than one row is
/// a legitimate state, and refusing to draw anything there would replace a sliver
/// with an empty panel.
/// The part of a save failure worth the one line the TRACKS header can spare.
///
/// `ProjectError` composes its messages outermost-first — "The previous project
/// file was preserved: could not create a transaction file: Permission denied
/// (os error 13)." — which is the right order for a notice and the wrong one for
/// a line that will be cut at about sixty characters. Truncating the head keeps
/// the reassurance and throws away the cause, so the user is told a save failed,
/// told their old file is safe, and never told *why*.
///
/// So the innermost clause wins here. Directly under a red "Save failed" it reads
/// as the sentence it is, and the persistent notice still carries the whole
/// thing — this is a summary, not the only copy.
fn save_error_summary(reason: &str) -> String {
    let trimmed = reason.trim().trim_end_matches('.');
    // `rsplit` rather than `split`: the innermost cause is the last clause, and
    // the errno text itself contains no ": " to be confused by.
    trimmed
        .rsplit(": ")
        .next()
        .filter(|clause| !clause.trim().is_empty())
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn track_row_is_legible(top: f32, row_height: f32, area: UiRect) -> bool {
    if row_height <= 0.0 || area.is_empty() || !top.is_finite() {
        return false;
    }
    let visible = (top + row_height).min(area.y + area.height) - top.max(area.y);
    visible > 0.0 && visible >= (row_height * TRACK_ROW_MINIMUM_VISIBLE).min(area.height)
}

/// The tracks flyout's action rows: two rows of two buttons, 46 px under the
/// panel's top (the old sidebar's stacked form, `workspace_layout.h:41-47`).
const TRACKS_ACTION_TOP: f32 = 46.0;
const TRACKS_ACTION_ROW: f32 = 32.0;
const TRACKS_ACTION_GAP: f32 = 4.0;

/// Where this frame's route to saving is, if there is one on screen.
///
/// Reported rather than assumed. The failure review 1.12 found is an affordance
/// being *absent*, and a capture of a missing control is indistinguishable from a
/// capture of a control the reviewer did not look for — so the answer is a value
/// a test can sweep and a report line can print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveAffordance {
    /// The top bar's state word, and Save one rail press away in the tracks
    /// flyout (WX-5c). The flyout replaced both the sidebar's tracks panel
    /// and the strip that stood in for it when the panel did not fit.
    TopBar,
    /// Fullscreen draws no chrome at all. Escape is the way back and every
    /// keyboard route still works, so this is a deliberate absence rather than
    /// the defect.
    Fullscreen,
}

/// The analyzer telemetry line above the position bar, or `None` when it must
/// not be drawn.
///
/// **Gated on the readout flag, which is the second half of review 1.9
/// (UX0-A09).** `"104 bands  peak 18  rms 0.299"` rendered unconditionally —
/// including under `--hud=0`, and including with `H` explicitly toggled off. The
/// HUD flag is the one place this interface decided telemetry should not appear,
/// and this line ignored it. Probe runs default the flag *on*, so a capture still
/// carries its own evidence.
///
/// A free function so the gate is assertable without a window: a capture can show
/// that a string is present, but nothing photographic can show that a string is
/// absent *because* of a flag rather than because the row was too narrow.
fn telemetry_caption(
    hud_visible: bool,
    width: f32,
    band_count: usize,
    peak_band: usize,
    rms: f32,
) -> Option<String> {
    if !hud_visible || band_count == 0 || width < transport_bar::SCRUB_CAPTION_MIN_WIDTH {
        return None;
    }
    Some(format!(
        "{band_count} bands  peak {peak_band}  rms {rms:.3}"
    ))
}

fn draw_splitter(
    d: &mut RaylibDrawHandle<'_>,
    hit: UiRect,
    kind: SplitKind,
    active: Option<SplitKind>,
) {
    let selected = active == Some(kind);
    let tint = if selected {
        color::ui_selected_edge()
    } else {
        color::ui_rule()
    };
    let thickness = if selected { 2.0 } else { 1.0 };
    match kind {
        SplitKind::Inspector => {
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

    /// The top bar's tabs    use super::*;

    /// The top bar's tabs are views onto `UiPanel` and the inspector (WX-5c
    /// §4): selection reads that state, a press writes it through the
    /// transport's paths, and nothing else holds a copy.
    #[test]
    fn mode_tabs_read_and_write_the_panel_and_the_inspector_and_nothing_else() {
        let workspace = crate::workspace::Workspace::new();
        let mut shell = Shell::new();
        let selected = |shell: &Shell| -> Vec<&str> {
            ModeTab::ALL
                .into_iter()
                .filter(|tab| shell.mode_tab_selected(*tab))
                .map(ModeTab::name)
                .collect()
        };
        assert_eq!(selected(&shell), ["direct"]);
        // `UiPanel::Tune` is no bottom panel, so it is still Direct.
        shell.panel = UiPanel::Tune;
        assert_eq!(selected(&shell), ["direct"]);

        shell.activate_mode_tab(ModeTab::Export, &workspace);
        assert_eq!(shell.panel, UiPanel::Export);
        assert_eq!(selected(&shell), ["export"]);
        shell.activate_mode_tab(ModeTab::Look, &workspace);
        assert!(shell.inspector_open);
        assert_eq!(selected(&shell), ["look", "export"]);
        // A second press on a panel's tab closes it, as its transport button did.
        shell.activate_mode_tab(ModeTab::Export, &workspace);
        assert_eq!(shell.panel, UiPanel::None);
        assert_eq!(selected(&shell), ["look"]);

        shell.activate_mode_tab(ModeTab::Lyrics, &workspace);
        shell.activate_mode_tab(ModeTab::Direct, &workspace);
        assert_eq!((shell.panel, shell.inspector_open), (UiPanel::None, false));
        assert_eq!(selected(&shell), ["direct"]);
        shell.activate_mode_tab(ModeTab::Assist, &workspace);
        assert_eq!(selected(&shell), ["assist"]);
    }

    #[test]
    fn fullscreen_attention_is_absent_fresh_and_names_each_risk_state() {
        let mut track = crate::workspace::Track::new(
            PathBuf::from("/tmp/attention.wav"),
            10.0,
            SceneId::Spectrum,
            7,
        )
        .unwrap();
        assert_eq!(TrackAttention::from_track(&track), None);

        track.mark_dirty_significant(1.0);
        track.finish_durable_edit_frame(1.0);
        let unfiled = TrackAttention::from_track(&track).unwrap();
        assert_eq!(unfiled.label(), "Unfiled work · Ctrl+S");
        assert_eq!(unfiled.dot_token(), "unfiled-dot");

        track.project_path = Some(PathBuf::from("/tmp/attention.musi"));
        let changes = TrackAttention::from_track(&track).unwrap();
        assert_eq!(changes.label(), "Working changes · Ctrl+S");

        track.mark_save_failed("disk full");
        let failure = TrackAttention::from_track(&track).unwrap();
        assert_eq!(failure.label(), "Save failed · Ctrl+S");

        assert!(fullscreen_attention_expanded(10.0, 12.499));
        assert!(!fullscreen_attention_expanded(10.0, 12.5));
    }

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

    // The toolbar's composition — what it sheds, in what order, and that the
    // timecode leaves for the band's header rather than printing through the
    // controls — is `transport_bar::TransportRow`'s since WX-5c, and is swept
    // there for monotonicity and overlap at every width from 120 to 1920.

    // ---- UX0-A09 (review 1.9): the position bar ----------------------------

    /// The groove the tests below press on: 200 px wide, starting at x=100, over
    /// a two-minute track. A press at 150 is exactly a quarter in.
    fn scrub_track() -> UiRect {
        UiRect::new(100.0, 20.0, 200.0, transport_bar::SCRUB_TRACK_HEIGHT)
    }

    fn scrub_frame(x: f32, pressed: bool, down: bool, playing: bool) -> TransportScrubInput {
        TransportScrubInput {
            track: scrub_track(),
            pointer_x: x,
            hovered: true,
            pressed,
            down,
            playing,
            duration_seconds: 120.0,
            seekable: true,
        }
    }

    /// The defect, stated as the thing that used to be impossible: the rect where
    /// every media player puts progress had no widget id at all, so clicking it
    /// did nothing while it showed 22% at 0.15 s.
    #[test]
    fn a_press_on_the_position_bar_seeks_to_the_fraction_it_was_released_at() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();

        let target = shell.transport_scrub(scrub_frame(150.0, true, true, false), &mut commands);
        assert_eq!(target, Some(30.0), "a quarter of a 120 s track is 30 s");
        assert!(
            commands.is_empty(),
            "the seek is deferred to release, as the timeline scrubber's is: {commands:?}"
        );

        // Dragged to three quarters, then released there.
        shell.transport_scrub(scrub_frame(250.0, false, true, false), &mut commands);
        assert!(commands.is_empty());
        shell.transport_scrub(scrub_frame(250.0, false, false, false), &mut commands);
        assert_eq!(commands, vec![ShellCommand::Seek(90.0)]);
        assert_eq!(shell.transport_scrub, None);
    }

    /// A drag pauses playback so the fill does not fight the hand, and resumes it
    /// afterwards — the timeline scrubber's transaction, deliberately identical.
    #[test]
    fn a_drag_that_began_during_playback_pauses_and_resumes_around_the_seek() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();

        shell.transport_scrub(scrub_frame(150.0, true, true, true), &mut commands);
        assert_eq!(commands, vec![ShellCommand::TogglePlay]);

        commands.clear();
        shell.transport_scrub(scrub_frame(300.0, false, false, false), &mut commands);
        assert_eq!(
            commands,
            vec![ShellCommand::Seek(120.0), ShellCommand::TogglePlay],
            "the end of the track, and playback back on"
        );
    }

    /// `transport_seekable` is false for streams the decoder cannot position in.
    /// A bar that accepted the drag and silently ignored it would be the same
    /// defect this finding is about, one layer over.
    #[test]
    fn a_stream_that_cannot_be_seeked_emits_nothing_and_starts_no_drag() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();
        let mut input = scrub_frame(150.0, true, true, true);
        input.seekable = false;

        assert_eq!(shell.transport_scrub(input, &mut commands), None);
        assert_eq!(shell.transport_scrub, None);
        input.pressed = false;
        input.down = false;
        assert_eq!(shell.transport_scrub(input, &mut commands), None);
        assert!(
            commands.is_empty(),
            "an unseekable stream seeked: {commands:?}"
        );
    }

    /// A press that lands outside the bar is not a scrub, even while the button
    /// is held: without the hover test a drag begun anywhere would capture it.
    #[test]
    fn a_press_that_did_not_land_on_the_bar_starts_nothing() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();
        let mut input = scrub_frame(150.0, true, true, false);
        input.hovered = false;

        assert_eq!(shell.transport_scrub(input, &mut commands), None);
        assert_eq!(shell.transport_scrub, None);
        assert!(commands.is_empty());
    }

    /// UX0-A02's rule, applied to the new gesture: fullscreen replaces the
    /// toolbar, so a drag in flight there has nowhere to be released.
    #[test]
    fn fullscreen_taken_mid_position_drag_completes_the_seek_rather_than_stranding_it() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();
        shell.transport_scrub(scrub_frame(150.0, true, true, true), &mut commands);
        commands.clear();

        shell.keyboard_actions(
            KeyboardFrame {
                toggle_fullscreen: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );

        assert_eq!(shell.transport_scrub, None);
        assert!(commands.contains(&ShellCommand::Seek(30.0)));
        assert!(commands.contains(&ShellCommand::TogglePlay));
    }

    /// The second half of review 1.9: the telemetry line ignored the readout flag
    /// and rendered even under `--hud=0`.
    #[test]
    fn the_telemetry_caption_is_drawn_only_when_the_readout_is_on() {
        assert_eq!(telemetry_caption(false, 400.0, 104, 18, 0.299), None);
        assert_eq!(
            telemetry_caption(true, 400.0, 104, 18, 0.299).as_deref(),
            Some("104 bands  peak 18  rms 0.299")
        );
        // Probe runs default the flag on, which is what keeps a capture carrying
        // its own evidence — the reason the line exists at all.
        assert!(telemetry_caption(true, 400.0, 104, 18, 0.299).is_some());
        // The width and analyzer floors still apply on top of the flag.
        assert_eq!(telemetry_caption(true, 100.0, 104, 18, 0.299), None);
        assert_eq!(telemetry_caption(true, 400.0, 0, 0, 0.0), None);
    }

    // ---- UX0-A11 (review 1.11): how long a notice lives --------------------

    /// The seam, from the shell's side: fifty-one call sites all passed
    /// `persistent: false, 6.0`, so a render that failed while the user was away
    /// was gone before they got back.
    #[test]
    fn a_failure_outlives_the_confirmation_that_a_save_worked() {
        let mut shell = Shell::new();
        shell.notify(Severity::Success, "Saved", "project.musi");
        shell.notify(Severity::Warning, "Analysis is stale", "Re-run Assist");
        shell.notify(
            Severity::Error,
            "Export failed",
            "ffmpeg exited with status 1",
        );

        // Twenty minutes away from the machine.
        shell.notices.tick(20.0 * 60.0);
        assert_eq!(shell.notices.len(), 1, "only the failure should remain");
        let kept = &shell.notices.notices()[0];
        assert_eq!(kept.severity, Severity::Error);
        assert!(kept.persistent);

        // And it can be got rid of, which is what the card's close box does.
        assert_eq!(
            shell.notices.dismiss(kept.id),
            musializer_core::ui::notice::NoticeResult::Ok
        );
        assert!(shell.notices.is_empty());
    }

    // ---- UX0-A12 (review 1.12): the vanished tracks panel -------------------

    /// The configurations review 1.12 names, plus the corners around them.
    fn save_route_configurations() -> Vec<((f32, f32), UiPanel, bool)> {
        let mut configurations = Vec::new();
        for window in [(960.0f32, 640.0f32), (1280.0, 720.0), (1920.0, 1080.0)] {
            for panel in [
                UiPanel::None,
                UiPanel::Tune,
                UiPanel::Export,
                UiPanel::Lyrics,
                UiPanel::Assist,
            ] {
                for inspector in [false, true] {
                    configurations.push((window, panel, inspector));
                }
            }
        }
        configurations
    }

    /// The defect: with the tracks panel hidden there was **no route to saving at
    /// all**, because the four action buttons were the only one and nothing on
    /// screen said they had gone. Since WX-5c the rail's Tracks icon and the
    /// top bar are in every workspace frame, including the 960x640 Assist
    /// state that used to collapse the panel to a strip.
    #[test]
    fn a_save_route_survives_every_panel_and_window_configuration() {
        let workspace = two_track_workspace();
        for (window, panel, inspector) in save_route_configurations() {
            let mut shell = Shell::new();
            shell.panel = panel;
            shell.inspector_open = inspector;
            let frame = shell.frame_for(window, &workspace);
            assert_eq!(
                shell.save_affordance(&frame),
                SaveAffordance::TopBar,
                "{window:?} with {panel:?} and inspector={inspector} has no way to save"
            );
            let described = shell.describe(&frame);
            assert!(
                described.contains("rail closed") && described.contains("save TopBar"),
                "the report line hides the state it exists to report: {described}"
            );
        }
        // Fullscreen is the one deliberate absence: Escape is the way back.
        let mut shell = Shell::new();
        shell.fullscreen = true;
        let frame = shell.frame_for((1280.0, 720.0), &workspace);
        assert_eq!(shell.save_affordance(&frame), SaveAffordance::Fullscreen);
    }

    /// The clip policy stands; the sliver does not.
    #[test]
    fn a_track_row_is_clipped_but_never_drawn_as_a_sliver() {
        let area = UiRect::new(0.0, 100.0, 200.0, 300.0);
        let row = 40.0;

        // Wholly inside.
        assert!(track_row_is_legible(150.0, row, area));
        // Clipped at the bottom, exactly three fifths visible: still drawn, which
        // is the policy the review says is right.
        assert!(track_row_is_legible(376.0, row, area));
        // One pixel less: the sliver, which is what cut the selected row through
        // the middle of its glyphs.
        assert!(!track_row_is_legible(377.0, row, area));
        // The same at the top edge, where a row scrolls *into* view: at 84 the
        // bottom 24 px are showing, at 83 only 23.
        assert!(track_row_is_legible(84.0, row, area));
        assert!(!track_row_is_legible(83.0, row, area));
        // Wholly outside, either way: no draw and no widget id.
        assert!(!track_row_is_legible(500.0, row, area));
        assert!(!track_row_is_legible(-100.0, row, area));

        // A list region shorter than one row is legitimate, and refusing to draw
        // there would replace a sliver with an empty panel.
        let cramped = UiRect::new(0.0, 0.0, 200.0, 12.0);
        assert!(track_row_is_legible(0.0, row, cramped));
        // Degenerate input stays out of the hit-testing entirely.
        assert!(!track_row_is_legible(0.0, 0.0, area));
        assert!(!track_row_is_legible(f32::NAN, row, area));
        assert!(!track_row_is_legible(0.0, row, UiRect::default()));
    }

    /// Ctrl+S, the second route. Not the oracle's — it has no save shortcut, and
    /// its absence is half of why the hidden panel was unrecoverable.
    #[test]
    fn control_s_saves_and_a_bare_s_does_not() {
        let mut shell = Shell::new();
        let mut commands = Vec::new();

        shell.keyboard_actions(
            KeyboardFrame {
                save: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );
        assert!(commands.is_empty(), "a bare S saved: {commands:?}");

        shell.keyboard_actions(
            KeyboardFrame {
                control: true,
                save: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );
        assert_eq!(commands, vec![ShellCommand::SaveProject]);

        commands.clear();
        shell.keyboard_actions(
            KeyboardFrame {
                control: true,
                shift: true,
                save: true,
                ..KeyboardFrame::default()
            },
            context(),
            &mut commands,
        );
        assert_eq!(commands, vec![ShellCommand::SaveProjectAs]);
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

    /// D1's whole contract, as a table.
    ///
    /// Written as the *command* rather than as [`DropKind`], because the kind is
    /// only half the claim: the bug being prevented is a classifier that answers
    /// correctly while `drop_command` sends two arms to the same place, and a
    /// test that stopped at the enum could not see it.
    #[test]
    fn a_dropped_file_is_dispatched_by_what_it_is() {
        use std::path::PathBuf;
        let cases: [(&str, DropKind); 22] = [
            // The protocol branch (HX-2): the double extension, case blind,
            // and never a bare `.json` — which stays on the oracle's audio
            // else-branch below.
            ("/tmp/cx4.protocol.json", DropKind::Protocol),
            ("/tmp/CX4.PROTOCOL.JSON", DropKind::Protocol),
            ("session.protocol.json", DropKind::Protocol),
            // WX-3, the same double-extension rule.
            ("/tmp/door.treatment.json", DropKind::Treatment),
            ("/tmp/DOOR.TREATMENT.JSON", DropKind::Treatment),
            ("/tmp/settings.json", DropKind::Audio),
            // The project branch (`plug.c:7542`).
            ("/tmp/song.musi", DropKind::Project),
            ("/tmp/UPPER.MUSI", DropKind::Project),
            ("relative.musi", DropKind::Project),
            ("/tmp/dots.in.name.musi", DropKind::Project),
            // The image branch (`plug.c:7548-7551`), all four extensions.
            ("/tmp/cover.png", DropKind::Image),
            ("/tmp/cover.jpg", DropKind::Image),
            ("/tmp/cover.jpeg", DropKind::Image),
            ("/tmp/cover.bmp", DropKind::Image),
            ("/tmp/CAMERA.JPG", DropKind::Image),
            // The else branch is audio, which is the oracle's (`plug.c:7559`) —
            // including for the seven formats it can actually open, and for a
            // file it cannot, which is *attempted* as audio and refused by the
            // decoder with a message that names audio.
            ("/tmp/song.wav", DropKind::Audio),
            ("/tmp/song.flac", DropKind::Audio),
            ("/tmp/song.MP3", DropKind::Audio),
            ("/tmp/notes.txt", DropKind::Audio),
            ("/tmp/archive.tar.gz", DropKind::Audio),
            ("/tmp/no-extension", DropKind::Audio),
            // A directory-looking path with a dot in a parent, so the extension
            // is read off the last component rather than the string.
            ("/tmp/v1.2/track", DropKind::Audio),
        ];
        for (path, expected) in cases {
            let path = PathBuf::from(path);
            assert_eq!(classify_drop(&path), expected, "{}", path.display());
            let command = drop_command(&path);
            let expected_command = match expected {
                DropKind::Project => ShellCommand::OpenDroppedProject(path.clone()),
                DropKind::Image => ShellCommand::ImportAsciiImage(path.clone()),
                DropKind::Audio => ShellCommand::LoadTrack(path.clone()),
                DropKind::Protocol => ShellCommand::OpenDroppedProtocol(path.clone()),
                DropKind::Treatment => ShellCommand::FollowTreatment(path.clone()),
            };
            assert_eq!(command, expected_command, "{}", path.display());
        }
    }

    /// The three arms must land on three *different* commands.
    ///
    /// The negative control for the table above: a `drop_command` whose match
    /// collapsed two arms would still satisfy every row if the expectation were
    /// derived from the same match, which is why this asserts the commands are
    /// pairwise distinct instead.
    #[test]
    fn the_five_drop_arms_are_five_different_commands() {
        use std::path::Path;
        let project = drop_command(Path::new("/tmp/a.musi"));
        let image = drop_command(Path::new("/tmp/a.png"));
        let audio = drop_command(Path::new("/tmp/a.wav"));
        let protocol = drop_command(Path::new("/tmp/a.protocol.json"));
        let treatment = drop_command(Path::new("/tmp/a.treatment.json"));
        let commands = [project, image, audio, protocol, treatment];
        for (index, left) in commands.iter().enumerate() {
            for right in commands.iter().skip(index + 1) {
                assert_ne!(left, right);
            }
        }
        // And each failure names a different type, which is the reporting half
        // of D1: "unsupported/corrupt input is reported by its attempted type".
        let nouns = [
            DropKind::Project.attempted_noun(),
            DropKind::Image.attempted_noun(),
            DropKind::Audio.attempted_noun(),
            DropKind::Protocol.attempted_noun(),
            DropKind::Treatment.attempted_noun(),
        ];
        let distinct: std::collections::HashSet<&str> = nouns.into_iter().collect();
        assert_eq!(distinct.len(), 5, "two branches report the same noun");
    }

    /// The image picker's filter and the drop classifier must agree (D2).
    ///
    /// A picker offering a fifth format would hand back a file the drop path
    /// classifies as audio, so the same image would import from the button and
    /// fail from a drop — a difference nothing else in this tree would catch.
    #[test]
    fn the_image_picker_offers_exactly_what_the_drop_path_imports() {
        use std::path::PathBuf;
        for pattern in musializer_runtime::process::dialogs::filters::ASCII_IMAGE.patterns {
            let path = PathBuf::from(pattern.replace('*', "/tmp/sample"));
            assert_eq!(
                classify_drop(&path),
                DropKind::Image,
                "the picker offers {pattern}, which the drop path does not import"
            );
        }
        assert_eq!(
            musializer_runtime::process::dialogs::filters::ASCII_IMAGE
                .patterns
                .len(),
            4
        );
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

    /// Escape closes an open flyout before it does anything else (WX-5c §1),
    /// and with no flyout open it still leaves fullscreen.
    #[test]
    fn escape_closes_the_flyout_first() {
        let escape = KeyboardFrame {
            escape: true,
            ..KeyboardFrame::default()
        };
        let mut shell = Shell::new();
        shell.open_flyout = Some(Flyout::Scenes);
        let mut commands = Vec::new();
        shell.keyboard_actions(escape, context(), &mut commands);
        assert_eq!(shell.open_flyout, None);
        assert!(commands.is_empty(), "{commands:?}");

        shell.fullscreen = true;
        shell.keyboard_actions(escape, context(), &mut commands);
        assert!(!shell.fullscreen);
        assert_eq!(commands, vec![ShellCommand::SetFullscreen(false)]);
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
            TextEntrySurface::TuneValue => shell.focus_tune_value_for_test(),
            TextEntrySurface::Dock => {
                shell.dock_open = true;
                shell.dock_keyboard = true;
            }
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

    /// The protocol keys exist only while a session is running (HX-2): `2`
    /// pressed in an ordinary session must mean nothing, and `2` pressed in a
    /// listening session must be an answer.
    #[test]
    fn protocol_keys_belong_to_a_running_session_and_nobody_else() {
        use musializer_core::feedback::{AnswerKind, Protocol, ProtocolItem, Window};

        let mut keys = KeyboardFrame {
            answer_digit: Some(2),
            protocol_replay: true,
            protocol_flip: true,
            protocol_next: true,
            ..KeyboardFrame::default()
        };

        let mut shell = Shell::new();
        let mut commands = Vec::new();
        shell.keyboard_actions(keys, context(), &mut commands);
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, ShellCommand::Protocol(_))),
            "protocol keys fired with no session"
        );

        let protocol = Protocol {
            title: "t".to_string(),
            audio_path: "x.wav".to_string(),
            audio_sha256: musializer_core::project::sha256::digest_hex(b"x"),
            items: vec![ProtocolItem {
                id: "one".to_string(),
                at_seconds: 1.0,
                window: Window {
                    pre: 1.0,
                    post: 2.0,
                },
                question: "?".to_string(),
                kind: AnswerKind::Choice,
                options: vec!["yes".to_string(), "no".to_string()],
                apply: None,
            }],
        };
        shell.protocol = Some(super::super::protocol::ProtocolSession::new(
            protocol,
            "t.protocol.json".to_string(),
            std::path::PathBuf::from("t.answers.jsonl"),
            Vec::new(),
        ));
        let mut commands = Vec::new();
        shell.keyboard_actions(keys, context(), &mut commands);
        for expected in [
            ShellCommand::Protocol(ProtocolAction::Answer(2)),
            ShellCommand::Protocol(ProtocolAction::Replay),
            ShellCommand::Protocol(ProtocolAction::Flip),
            ShellCommand::Protocol(ProtocolAction::Next),
        ] {
            assert!(commands.contains(&expected), "{expected:?} did not fire");
        }

        // Ctrl chords the digits to the scale ladder, never to an answer.
        keys.control = true;
        let mut commands = Vec::new();
        shell.keyboard_actions(keys, context(), &mut commands);
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, ShellCommand::Protocol(_))),
            "a Ctrl-chorded digit answered a protocol item"
        );
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
            // drawing its filter, and `T` closes the Tune inspector. The
            // inspector is a separate flag from `panel`, so closing it needs its
            // own line here — a guard that only paired with `panel` would leave
            // a Tune value chip holding the keyboard forever (PX6).
            shell.panel = UiPanel::None;
            shell.inspector_open = false;
            // The dock hides without the composition root clearing its flag
            // first, which is the same stranded state one surface over (WX-1).
            shell.dock_open = false;
            shell.begin_frame(UiScale::default(), false);

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
        shell.split_drag = Some(SplitKind::Timeline);
        shell.set_inspector_open(false);
        assert_eq!(shell.split_drag, Some(SplitKind::Timeline));
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

    #[test]
    fn scene_and_pcm_surfaces_share_one_transactional_scrub_preview() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);
        let lane = UiRect::new(10.0, 20.0, 100.0, 24.0);
        let mut commands = Vec::new();

        assert!(shell.begin_timeline_scrub(true, &mut commands));
        assert_eq!(commands, vec![ShellCommand::TogglePlay]);
        assert_eq!(
            shell.update_timeline_scrub(85.0, lane, 120.0, true, &mut commands),
            Some(90.0)
        );
        assert_eq!(shell.timeline_playhead_seconds(12.0), 90.0);
        assert_eq!(commands, vec![ShellCommand::TogglePlay]);

        shell.update_timeline_scrub(85.0, lane, 120.0, false, &mut commands);
        assert_eq!(shell.timeline_gesture, None);
        assert_eq!(
            commands,
            vec![
                ShellCommand::TogglePlay,
                ShellCommand::Seek(90.0),
                ShellCommand::TogglePlay
            ]
        );
    }

    #[test]
    fn a_wheel_notch_anchors_the_zoom_under_the_pointer_in_whichever_lane_asked() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);
        // Two lanes at different x offsets, both views of the same 120 s axis.
        // The pointer sits a quarter of the way into each, so both must resolve
        // the same 30 s anchor — that is what makes the wheel mean the same
        // thing over the scene lane as over the waveform.
        shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 120.0);
        let waveform = shell.timeline_zoom_pending.take().expect("waveform claim");
        shell.request_timeline_zoom(1.0, 300.0, 200.0, 400.0, 120.0);
        let scene = shell.timeline_zoom_pending.take().expect("scene claim");

        for (name, claim) in [("waveform", waveform), ("scene", scene)] {
            let TimelineWheel::Zoom {
                factor,
                anchor_seconds,
            } = claim
            else {
                panic!("{name} claimed a pan without Shift held");
            };
            assert!((anchor_seconds - 30.0).abs() < 1e-9, "{name} anchor");
            assert!((factor - 1.2).abs() < 1e-9, "{name} factor");
        }
    }

    /// Shift turns the notch into a pan (D4), and the property that matters is
    /// that it moves the window **without changing the span**. A "pan" that also
    /// zoomed would be indistinguishable from a zoom in any capture, because both
    /// change what is on screen.
    #[test]
    fn a_shift_wheel_notch_pans_the_window_and_never_changes_the_span() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);
        // Zoom in first: panning a whole-track view is refused, and rightly.
        shell.timeline.zoom(120.0, 4.0, 60.0);
        let span_before = shell.timeline.span_seconds;
        let start_before = shell.timeline.start_seconds;

        shell.wheel_pan_modifier = true;
        shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 120.0);
        let TimelineWheel::Pan { delta_seconds } =
            shell.timeline_zoom_pending.take().expect("a pan claim")
        else {
            panic!("Shift held and it still zoomed");
        };
        // Wheel up is earlier, matching a vertical scroll.
        assert!(delta_seconds < 0.0, "wheel up must move the window back");
        assert!(
            (delta_seconds.abs() - span_before * TIMELINE_WHEEL_PAN_FRACTION).abs() < 1e-9,
            "one notch is a fixed fraction of the visible span"
        );

        shell.timeline.pan(120.0, delta_seconds);
        assert!(
            (shell.timeline.span_seconds - span_before).abs() < 1e-9,
            "a pan changed the zoom"
        );
        assert!(shell.timeline.start_seconds < start_before);
        // And it detaches follow, exactly as a middle-drag pan does — the same
        // gesture by a different input.
        assert!(shell.timeline_manual_view);
    }

    /// A whole-track view has nowhere to pan to, and accepting the notch anyway
    /// would light the Follow button over a view that never moved.
    #[test]
    fn a_shift_wheel_notch_over_a_whole_track_view_is_refused_outright() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);
        assert!(shell.timeline.is_whole(120.0));

        shell.wheel_pan_modifier = true;
        shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 120.0);
        assert_eq!(shell.timeline_zoom_pending, None);
        assert!(
            !shell.timeline_manual_view,
            "a refused notch still suspended playback-follow"
        );
    }

    #[test]
    fn one_wheel_notch_is_claimed_once_however_many_lanes_see_it() {
        // `get_mouse_wheel_move` reports the same notch to every lane in the
        // frame. If two of them accepted it the factors would multiply, so one
        // notch over an overlap would zoom 1.44x where it zooms 1.2x anywhere
        // else. The claim is therefore first-come and single.
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);

        shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 120.0);
        shell.request_timeline_zoom(3.0, 210.0, 10.0, 400.0, 120.0);
        let TimelineWheel::Zoom {
            factor,
            anchor_seconds,
        } = shell.timeline_zoom_pending.expect("first claim survives")
        else {
            panic!("a bare notch must zoom");
        };
        assert!((factor - 1.2).abs() < 1e-9, "the second lane compounded it");
        assert!(
            (anchor_seconds - 30.0).abs() < 1e-9,
            "the second lane re-anchored it"
        );
    }

    #[test]
    fn a_wheel_notch_is_refused_by_every_guard_that_should_refuse_it() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);

        // No notch at all.
        shell.request_timeline_zoom(0.0, 110.0, 10.0, 400.0, 120.0);
        assert_eq!(shell.timeline_zoom_pending, None);

        // No track: `seconds_at` has no axis to resolve against, and the pending
        // pair would be applied against the next track's duration.
        shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 0.0);
        assert_eq!(shell.timeline_zoom_pending, None);

        // A gesture in flight owns the view. Zooming under a scrub moves the axis
        // the drag is measured against, and the content slides out from under the
        // hand — including a pan, whose origin is a fixed pixel.
        for gesture in [
            TimelineGesture::Scrub,
            TimelineGesture::Pan,
            TimelineGesture::SceneBoundary,
        ] {
            shell.timeline_gesture = Some(gesture);
            shell.request_timeline_zoom(1.0, 110.0, 10.0, 400.0, 120.0);
            assert_eq!(shell.timeline_zoom_pending, None, "{gesture:?} was ignored");
        }
        shell.timeline_gesture = None;
    }

    #[test]
    fn a_cursor_request_is_last_wins_and_never_survives_the_frame() {
        use raylib::consts::MouseCursor;

        let mut shell = Shell::new();
        assert_eq!(shell.pointer_cursor, None);

        shell.request_cursor(MouseCursor::MOUSE_CURSOR_RESIZE_NS);
        shell.request_cursor(MouseCursor::MOUSE_CURSOR_RESIZE_EW);
        assert_eq!(
            shell.pointer_cursor,
            Some(MouseCursor::MOUSE_CURSOR_RESIZE_EW),
            "the topmost surface's request lost to one drawn under it"
        );

        // The pointer moves between frames, so a request is only ever true of the
        // frame that made it. Without this reset a resize arrow stays on screen
        // after the hand has left the handle.
        shell.begin_frame(UiScale::default(), false);
        assert_eq!(shell.pointer_cursor, None);
    }

    #[test]
    fn middle_pan_uses_a_fixed_origin_and_releases_without_a_command() {
        let mut shell = Shell::new();
        shell.reset_timeline(120.0);
        shell.timeline.zoom(120.0, 4.0, 60.0);
        let lane = UiRect::new(10.0, 20.0, 100.0, 24.0);
        let origin = shell.timeline.start_seconds;

        shell.begin_timeline_pan(60.0);
        shell.update_timeline_pan(50.0, lane, 120.0, true);
        assert_eq!(shell.timeline.start_seconds, origin + 3.0);
        // Recomputing from the fixed origin means the same pointer cannot drift.
        shell.update_timeline_pan(50.0, lane, 120.0, true);
        assert_eq!(shell.timeline.start_seconds, origin + 3.0);
        assert!(shell.timeline_manual_view);

        shell.update_timeline_pan(50.0, lane, 120.0, false);
        assert_eq!(shell.timeline_gesture, None);
        assert_eq!(shell.timeline_pan, None);
    }

    #[test]
    fn playhead_handle_stays_inside_both_pcm_edges() {
        let lane = UiRect::new(100.0, 20.0, 400.0, 56.0);
        let left = playhead_geometry(lane, 100.0, 2.0, true).expect("left edge");
        let right = playhead_geometry(lane, 500.0, 2.0, true).expect("right edge");

        assert!(left.line_x >= lane.x && left.line_x <= lane.x + lane.width);
        assert!(right.line_x >= lane.x && right.line_x <= lane.x + lane.width);
        assert_eq!(left.handle_x, Some(105.0));
        assert_eq!(right.handle_x, Some(495.0));
        assert!(left.handle_x.unwrap() - 5.0 >= lane.x);
        assert!(right.handle_x.unwrap() + 5.0 <= lane.x + lane.width);
    }

    // ---- UX0-A03 (review 1.3): the lyric draft's owning track ---------------

    /// The cue the tests below bind to, in both documents. Row 2, so there is a
    /// cue on either side of it.
    const DRAFT_ROW: usize = 2;
    /// What row 2 says on track A before the edit, and what it says on track B
    /// already.
    const A_TEXT: &str = "the same sheet";
    const B_TEXT: &str = "the same sheet!";

    /// Two tracks carrying the *same* lyric sheet, which is the case the bug
    /// needs and the one a user reaches by duplicating a project or working on a
    /// remix. Cue ids restart at 1 in every document, so both row 2s have the
    /// same id — and track B's row 2 already reads what the user is about to type
    /// into track A's.
    ///
    /// That coincidence is the point. With the dirtiness question put to the
    /// *current* track rather than to the draft's owner, this draft reports clean
    /// against B, walks through the guard, and the deferred `Update` lands on B.
    fn two_track_workspace() -> crate::workspace::Workspace {
        use std::path::PathBuf;

        use musializer_core::project::lyrics::{LyricCue, LyricsDocument};
        use musializer_core::scene::SceneId;

        let mut workspace = crate::workspace::Workspace::new();
        for (name, row_text) in [("/tmp/a.wav", A_TEXT), ("/tmp/b.wav", B_TEXT)] {
            let mut track =
                crate::workspace::Track::new(PathBuf::from(name), 120.0, SceneId::Spectrum, 7)
                    .expect("120s is a valid duration");
            let mut document = LyricsDocument::new(120.0).expect("a positive duration");
            for index in 0..4usize {
                let start = 10.0 + index as f64 * 5.0;
                document
                    .insert(LyricCue {
                        id: 0,
                        start_seconds: start,
                        end_seconds: start + 2.0,
                        text: if index == DRAFT_ROW {
                            row_text.to_string()
                        } else {
                            format!("shared line {index}")
                        },
                        origin: Default::default(),
                    })
                    .expect("the fixture cues are valid");
            }
            track.lyrics = document;
            workspace.push(track);
        }
        workspace
    }

    #[test]
    fn a_save_failure_is_summarised_by_its_cause_not_its_reassurance() {
        // The real message, in the order `ProjectError` composes it.
        assert_eq!(
            save_error_summary(
                "The previous project file was preserved: could not create a transaction file: Permission denied (os error 13)."
            ),
            "Permission denied (os error 13)"
        );
        // A message with no nesting is already its own cause.
        assert_eq!(save_error_summary("disk full"), "disk full");
        // A trailing colon must not yield an empty line; the whole text is
        // better than nothing at all.
        assert_eq!(save_error_summary("could not write: "), "could not write:");
        assert_eq!(save_error_summary(""), "");
    }

    /// The C1 checklist, pinned (see [`ShellCommand::mutates_project`]).
    ///
    /// The exhaustive match is what forces a *new* variant to be classified; this
    /// is what stops an *existing* one being reclassified by accident. Both
    /// directions are mistakes with the same shape — a durable command marked
    /// transient loses work silently, and a transient one marked durable makes
    /// pressing Play dirty the project — so each list is named in full rather
    /// than counted.
    #[test]
    fn every_command_is_classified_durable_or_transient() {
        use musializer_core::scene::SceneId;
        use musializer_core::timing::render_export::RenderExportConfig;

        let durable: Vec<ShellCommand> = vec![
            ShellCommand::SelectScene(SceneId::Loom),
            ShellCommand::SetSetting {
                scene: SceneId::Loom,
                index: 0,
                value: 1.0,
            },
            ShellCommand::ResetScene(SceneId::Loom),
            ShellCommand::SetAutoScenes(true),
            ShellCommand::SetRenderConfig(RenderExportConfig::default()),
        ];
        for command in durable {
            assert!(
                command.mutates_project(),
                "{command:?} writes into a .musi and must mark the track dirty"
            );
        }

        let transient: Vec<ShellCommand> = vec![
            ShellCommand::TogglePlay,
            ShellCommand::Seek(12.0),
            ShellCommand::SelectTrack(0),
            ShellCommand::SetVolume(0.5),
            ShellCommand::ToggleMute,
            ShellCommand::SetFullscreen(true),
            ShellCommand::StartRender,
            ShellCommand::OpenAudio,
            ShellCommand::OpenProject,
            ShellCommand::SaveProject,
            ShellCommand::SaveProjectAs,
        ];
        for command in transient {
            assert!(
                !command.mutates_project(),
                "{command:?} must not dirty the project: listening to a track is not editing it"
            );
        }
    }

    /// The bug, at the layer that used to have no guard at all. The Tracks row
    /// pushed `SelectTrack` unconditionally, the switch happened, and the
    /// deferred `Update` then wrote track A's text over track B's cue #3.
    #[test]
    fn a_dirty_lyric_draft_blocks_a_track_switch_and_says_so() {
        let mut shell = Shell::new();
        let workspace = two_track_workspace();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();
        let id = document.cues()[DRAFT_ROW].id;

        shell.lyrics.open_dirty_draft_for_test(0, &document, id);
        assert_eq!(shell.lyrics.draft_owner(), Some(0));

        let before = shell.notices.len();
        assert!(
            !shell.lyric_draft_allows_context_change(&workspace),
            "the draft is dirty and the switch went through anyway"
        );
        assert!(
            shell.notices.len() > before,
            "the refusal has to be visible; a silent no-op reads as a broken button"
        );
    }

    /// The old bug route, end to end and at the layer `main.rs` drives: edit a cue
    /// on track A, reach track B without the guard, and let the deferred edit
    /// flush. It must be refused, B must be byte-identical, and the user must be
    /// told — a dropped edit they believe they applied is the same loss as the
    /// overwrite, one track over.
    #[test]
    fn a_deferred_edit_that_outlived_its_track_is_refused_and_reported() {
        let mut shell = Shell::new();
        let mut workspace = two_track_workspace();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();
        let id = document.cues()[DRAFT_ROW].id;
        let untouched = workspace.get(1).expect("slot 1").lyrics.clone();

        shell.lyrics.open_dirty_draft_for_test(0, &document, id);
        let track = workspace.get(0).expect("slot 0").clone();
        shell.lyrics.apply_draft_for_test(&track);
        assert!(shell.lyrics.has_pending());

        // The unguarded path: the selection moved and the editor was never told.
        assert!(workspace.select(1));

        let edits = shell.drain_lyric_edits(workspace.current_index());
        assert!(edits.is_empty(), "an edit owned by slot 0 reached slot 1");
        assert_eq!(
            shell.notices.len(),
            1,
            "the drop has to be said out loud; the user thinks they applied it"
        );

        // And what `main.rs` would then write changes nothing about B.
        let current = workspace.get_mut(1).expect("slot 1");
        for edit in edits {
            let _ = edit.apply(current);
        }
        assert_eq!(current.lyrics, untouched);
    }

    /// The dirtiness is asked of the **owning** track, not the current one. With
    /// the question put to whichever track is current, a draft carried onto a
    /// document whose cue happens to match would report clean and walk straight
    /// through the guard.
    #[test]
    fn the_guard_asks_the_track_that_owns_the_draft() {
        let mut shell = Shell::new();
        let mut workspace = two_track_workspace();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();
        let id = document.cues()[DRAFT_ROW].id;
        shell.lyrics.open_dirty_draft_for_test(0, &document, id);

        // Slot 1 is current now, and it has its own cue with this exact id.
        assert!(workspace.select(1));
        assert!(
            !shell.lyric_draft_allows_context_change(&workspace),
            "the guard read slot 1's cue #3 and called slot 0's draft clean"
        );
    }

    /// The other half: a clean editor must not make the interface feel stuck.
    #[test]
    fn a_clean_lyric_draft_leaves_every_context_change_unguarded() {
        let mut shell = Shell::new();
        let workspace = two_track_workspace();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();

        // Nothing opened at all.
        assert!(shell.lyric_draft_allows_context_change(&workspace));

        // Bound to a cue, untouched.
        shell.lyrics.enter_track(Some(0));
        shell.lyrics.select_single(&document, document.cues()[1].id);
        assert!(shell.lyric_draft_allows_context_change(&workspace));
        assert_eq!(shell.notices.len(), 0);
    }

    /// Requirement 3 of UX0-A03, end to end: Apply, the write `main.rs` makes at
    /// the end of the frame, and then a switch that must not be argued with.
    #[test]
    fn applying_the_draft_then_switching_tracks_is_not_guarded() {
        let mut shell = Shell::new();
        let mut workspace = two_track_workspace();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();
        let id = document.cues()[DRAFT_ROW].id;
        let untouched = workspace.get(1).expect("slot 1").lyrics.clone();

        shell.lyrics.open_dirty_draft_for_test(0, &document, id);
        let track = workspace.get(0).expect("slot 0").clone();
        shell.lyrics.apply_draft_for_test(&track);

        let drain = shell.lyrics.take_pending(workspace.current_index());
        assert_eq!(drain.refused, 0);
        let owner = workspace.get_mut(0).expect("slot 0");
        for edit in drain.edits {
            edit.apply(owner).expect("the model accepts its own draft");
        }
        assert_eq!(
            owner.lyrics.find(id).expect("the cue survives").text,
            B_TEXT
        );

        assert!(shell.lyric_draft_allows_context_change(&workspace));
        assert_eq!(shell.notices.len(), 0);
        // The whole document, not one cue: an edit that reached the wrong track
        // could have moved a boundary rather than a word.
        assert_eq!(workspace.get(1).expect("slot 1").lyrics, untouched);
    }

    #[test]
    fn every_column_view_round_trips_through_todays_state() {
        let mut shell = Shell::with_preferences(UiPreferences::default());
        // Forwards and backwards through every pair, so no view leaves state
        // behind that the next one reads.
        for from in ColumnView::ALL {
            for to in ColumnView::ALL {
                shell.set_column_view(from);
                assert_eq!(shell.column_view(), from, "{from:?}");
                shell.set_column_view(to);
                assert_eq!(shell.column_view(), to, "{from:?} -> {to:?}");
                assert_eq!(shell.ui_preferences.dock_open, shell.dock_open);
            }
        }
    }

    #[test]
    fn look_no_longer_coexists_with_a_panel_once_set_as_a_view() {
        let mut shell = Shell::with_preferences(UiPreferences::default());
        shell.panel = UiPanel::Lyrics;
        shell.inspector_open = true;
        // Before the fold, the panel wins the derived view…
        assert_eq!(shell.column_view(), ColumnView::Lyrics);
        // …and setting Look ends the pairing.
        shell.set_column_view(ColumnView::Look);
        assert_eq!((shell.panel, shell.inspector_open), (UiPanel::None, true));
        shell.set_column_view(ColumnView::Assist);
        assert_eq!(
            (shell.panel, shell.inspector_open),
            (UiPanel::Assist, false)
        );
    }

    #[test]
    fn a_hidden_dock_stays_open_behind_another_view_and_closing_releases_the_keyboard() {
        let mut shell = Shell::with_preferences(UiPreferences::default());
        shell.set_column_view(ColumnView::Direct);
        shell.dock_keyboard = true;
        shell.set_column_view(ColumnView::Lyrics);
        assert!(shell.dock_open, "hiding a chat never ends it");
        shell.set_column_view(ColumnView::Closed);
        assert!(!shell.dock_open);
        assert!(!shell.dock_keyboard);
    }

    #[test]
    fn column_view_names_are_stable_and_unique() {
        let names: Vec<&str> = ColumnView::ALL.iter().map(|view| view.name()).collect();
        assert_eq!(
            names,
            ["closed", "direct", "lyrics", "look", "assist", "render"]
        );
        for view in ColumnView::ALL {
            assert_eq!(ColumnView::from_name(view.name()), Some(view));
        }
        assert_eq!(ColumnView::from_name("export"), None);
        assert_eq!(
            ModeTab::ALL.map(ModeTab::column_view),
            [
                ColumnView::Direct,
                ColumnView::Lyrics,
                ColumnView::Look,
                ColumnView::Render,
                ColumnView::Assist,
            ]
        );
    }
}
