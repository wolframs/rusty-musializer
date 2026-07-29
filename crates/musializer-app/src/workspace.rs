//! The track list, and everything a track owns.
//!
//! Port of `../musializer/src/track.h`'s `Track` and `Tracks`, plus the parts of
//! `plug.c` that create, select and describe them: `plug_load_track`
//! (`plug.c:751-861`), `current_track` (`:548-556`), `track_display_name`
//! (`:559-566`), `scene_seed_for_track` (`:611-614`) and `mark_project_dirty`
//! (`:616-622`).
//!
//! # Why this is raylib-free, and where the `Music` went
//!
//! The C `Track` owns a `Music` and every loaded track keeps its stream attached
//! to the analyzer callback. That cannot come across as-is: raylib-rs's `Music`
//! borrows the `RaylibAudio` device, so a `Track` holding one would make this
//! whole type `Workspace<'audio>` and infect `App`, the shell input, and every
//! panel that only ever wanted a name and a duration.
//!
//! It also does not need to. Selecting a different track in the C **stops** the
//! outgoing stream (`plug.c:5273`) and `start_preview_track` (`:658-669`) plays
//! the incoming one from zero, so per-track playback position is not preserved
//! across a switch — there is no state in that `Music` worth keeping alive. So
//! `main.rs` holds one `Music` for the current track and reopens on switch, and
//! this module stays a plain data model that a headless test can build.
//!
//! The observable difference is that a switch costs a `LoadMusicStream`. That is
//! a streaming open, not a decode, and the C already accepts a whole-file decode
//! plus SHA-256 plus waveform reduction at *load* time.
//!
//! # Three places `Option` replaces a C sentinel pair
//!
//! Each of these is one fact in C's two fields, and the pair is a state that can
//! disagree with itself:
//!
//! - `project_metadata` + `project_metadata_initialized` → `Option<Metadata>`
//! - `ascii_cells`/`ascii_columns`/`ascii_rows`/path/hash → `Option<AsciiImage>`
//! - every `char path[PLUG_RELOAD_PATH_CAPACITY]` that may be empty → `Option<PathBuf>`
//!
//! `song_atlas_map_attempted` is deliberately **not** folded into its `Option`,
//! because "no map" and "tried and failed to build a map" are genuinely
//! different and the C distinguishes them to avoid retrying every frame.

// Most of this model has no reader yet, and that is deliberate. W2 exists so the
// six Band 1 panels do not each invent their own shim (REWRITE_PLAN.md, the
// completion plan's dependency order), which means the fields land before the
// panels that read them. Remove this allow at the parity gate — by then every
// field should have a reader, and one that does not is a missing panel.
#![allow(dead_code)]

use std::path::PathBuf;

use musializer_core::audio::song_atlas_map::SongAtlasMap;
use musializer_core::project::event_timeline::{self, EventTimeline, ManualClear};
use musializer_core::project::lyrics::{LyricsDocument, LyricsError};
use musializer_core::project::model::{AnalysisLaneReference, CaptionStyle, Metadata};
use musializer_core::project::preset_store::PresetLibrary;
use musializer_core::project::scene_switch::{SceneSwitchError, SceneSwitchTimeline};
use musializer_core::scene::events::{EventRecord, EventTimelineError};
use musializer_core::scene::routes::RouteTable;
use musializer_core::scene::{SceneId, SceneSettings};
use musializer_core::scenes::ascii_field::ascii_art::Grid as AsciiGrid;
use musializer_core::timing::render_export::RenderExportConfig;
use musializer_core::timing::track_identity;
use musializer_core::timing::track_timeline::Waveform;
use musializer_core::ui::assist_ui_state::AssistSession;

/// An ASCII source image bound to a track (`track.h`'s `ascii_cells`,
/// `ascii_columns`, `ascii_rows`, `ascii_image_path`, `ascii_image_sha256`).
#[derive(Clone, Debug)]
pub struct AsciiImage {
    /// The converted cells, or `None` when only the image's identity is known.
    ///
    /// The C has no such state: its cells and its dimensions are one buffer, so
    /// a project always restores both. Here opening a project restores the
    /// identity immediately and decodes the image separately, and a blank grid
    /// of the right size would claim cells had been restored when they had not.
    pub grid: Option<AsciiGrid>,
    /// The dimensions the project recorded, which survive whether or not the
    /// image has been decoded yet.
    pub columns: usize,
    pub rows: usize,
    pub path: PathBuf,
    /// Hex SHA-256 of the source file. Empty when hashing was deferred, exactly
    /// as C leaves the buffer empty rather than failing the load.
    pub sha256: String,
}

/// Why a `+ Scene` cue was refused (`record_scene_cue`'s three notices,
/// `plug.c:1984-2022`).
///
/// The messages are the oracle's, kept next to the failure rather than at the
/// call site so the tray and a test read the same words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SceneCueError {
    #[error("Move the playhead before the end of the track.")]
    Playhead,
    #[error("The selected scene settings are invalid.")]
    Settings,
    #[error("The scene cue could not be added to the switch plan.")]
    Plan(SceneSwitchError),
}

impl From<SceneSwitchError> for SceneCueError {
    fn from(error: SceneSwitchError) -> Self {
        Self::Plan(error)
    }
}

/// One open track (`track.h:31-89`).
///
/// Every field a panel edits lives here rather than in `App`, because the C's
/// close guards, dirty flags and editor drafts are all per-track: the route
/// editor keys its draft by track slot (`route_editor_state.h:10-16`) and
/// `lyric_editor_allow_context_change` asks about *this* track's draft.
#[derive(Clone, Debug)]
pub struct Track {
    // ---- identity -------------------------------------------------------
    /// The canonical absolute path the track was opened from.
    pub file_path: PathBuf,
    pub duration_seconds: f64,
    pub transport_seekable: bool,
    /// Hex SHA-256 of the audio file, or empty when the calculation was
    /// deferred. C treats a failure here as non-fatal and lets saving retry it
    /// (`plug.c:806-812`), and so does this.
    pub audio_sha256: String,

    // ---- authored content ----------------------------------------------
    pub lyrics: LyricsDocument,
    pub scene_switches: SceneSwitchTimeline,
    /// Model-derived interpretation. Kept apart from `manual_events` because
    /// they are two evidence lanes, and merging them is the one thing the
    /// rewrite must not do.
    pub semantic_events: EventTimeline,
    /// User-authored markers only.
    pub manual_events: EventTimeline,
    pub next_manual_event_id: u64,
    /// The `Clear manual` button's armed flag and undo buffer, whose whole state
    /// machine is [`ManualClear`].
    ///
    /// **Per track, where the C keeps one global pair** (`p->event_undo`,
    /// `p->clear_events_confirmation`). That is a deliberate divergence and it
    /// fixes a bug rather than reproducing one: in the frozen C you can clear
    /// track A's lane, select track B, and click `Undo clear` — which runs
    /// `event_timeline_replace(&track->manual_events, &p->event_undo)` against
    /// *B* and moves A's markers onto it (`plug.c:2944`). Nothing in a `.musi`
    /// file distinguishes the two designs, so by `AGENTS.md`'s parity test this
    /// is a mechanism choice, and the safe mechanism wins.
    pub manual_clear: ManualClear,

    // ---- scene binding --------------------------------------------------
    pub base_scene: SceneId,
    pub previous_base_scene: SceneId,
    pub scene_selection_pending: bool,
    pub scene_seed: u64,
    pub scene_instance_id: u64,

    // ---- tuning ---------------------------------------------------------
    /// The editable values.
    pub scene_settings: SceneSettings,
    /// The values playback restores to when a cue's span ends. C keeps this
    /// separate so a cue can drive a parameter without becoming an edit.
    pub playback_scene_settings: SceneSettings,
    pub scene_routes: RouteTable,
    pub cue_settings_active: bool,
    /// Track-local presets are project data: read and written byte-stable for
    /// old `.musi` files, copied into the shared library on open, and no longer
    /// surfaced directly in the UI (`track.h:52-55`).
    pub scene_presets: PresetLibrary,

    // ---- output ---------------------------------------------------------
    pub render_config: RenderExportConfig,

    // ---- derived from the audio ----------------------------------------
    pub timeline_waveform: Option<Waveform>,
    pub song_atlas_map: Option<SongAtlasMap>,
    /// Distinct from `song_atlas_map.is_none()`: a failed build must not be
    /// retried every frame.
    pub song_atlas_map_attempted: bool,
    pub ascii: Option<AsciiImage>,

    // ---- captions -------------------------------------------------------
    /// Preview and export read this same struct, so what the workspace shows is
    /// what renders (`track.h:63-65`).
    pub caption_style: CaptionStyle,
    /// Where the imported face and its licence live *now*, on this machine. The
    /// style itself carries the project-relative path that was last written, so
    /// these hold the verified source a Save As would re-bundle from. Both are
    /// `None` unless `caption_style.font` is present (`track.h:66-71`).
    pub caption_font_path: Option<PathBuf>,
    pub caption_licence_path: Option<PathBuf>,

    // ---- session state, deliberately not persisted ----------------------
    /// An authored lyric sheet the next Assist run should synchronize against.
    /// Not written to the `.musi`: it is an input to analysis rather than
    /// project content, the words end up in the project as cues anyway, and
    /// every other asset the format records is content-addressed and bundled
    /// (`track.h:72-79`).
    pub lyrics_reference_path: Option<PathBuf>,

    // ---- project identity ------------------------------------------------
    pub project_path: Option<PathBuf>,
    pub project_metadata: Option<Metadata>,
    pub project_dirty: bool,
    pub project_autosave_failed: bool,
    pub project_dirty_since: f64,
    /// Provenance, not dependencies: the referenced files are not needed to
    /// reopen evaluated project data.
    pub analysis_lanes: Vec<AnalysisLaneReference>,
}

impl Track {
    /// Builds a track the way `plug_load_track` does (`plug.c:766-830`), given
    /// the facts only the runtime can supply.
    ///
    /// `base_scene` and `scene_seed` are parameters rather than defaults because
    /// C inherits both from whatever track is already active
    /// (`plug.c:794-795`); [`Workspace::inherited_scene`] computes the pair.
    ///
    /// The caption style is initialized rather than zeroed, and that is not
    /// pedantry: a zeroed style reads face and box as their first enumerator
    /// with every scale at zero, so captions vanish on any track opened outside
    /// a project (`plug.c:775-778`).
    pub fn new(
        file_path: PathBuf,
        duration_seconds: f64,
        base_scene: SceneId,
        scene_seed: u64,
    ) -> Result<Self, LyricsError> {
        let scene_settings = SceneSettings::new();
        Ok(Self {
            file_path,
            duration_seconds,
            transport_seekable: false,
            audio_sha256: String::new(),
            lyrics: LyricsDocument::new(duration_seconds)?,
            scene_switches: SceneSwitchTimeline::new(),
            semantic_events: EventTimeline::new(),
            manual_events: EventTimeline::new(),
            next_manual_event_id: 1,
            manual_clear: ManualClear::new(),
            base_scene,
            previous_base_scene: base_scene,
            scene_selection_pending: false,
            scene_seed,
            scene_instance_id: 1,
            playback_scene_settings: scene_settings,
            scene_settings,
            scene_routes: RouteTable::new(),
            cue_settings_active: false,
            scene_presets: PresetLibrary::new(),
            render_config: RenderExportConfig::default(),
            timeline_waveform: None,
            song_atlas_map: None,
            song_atlas_map_attempted: false,
            ascii: None,
            caption_style: CaptionStyle::default(),
            caption_font_path: None,
            caption_licence_path: None,
            lyrics_reference_path: None,
            project_path: None,
            project_metadata: None,
            project_dirty: false,
            project_autosave_failed: false,
            project_dirty_since: 0.0,
            analysis_lanes: Vec::new(),
        })
    }

    /// What the tracks panel, the render banner and the Assist prompts call this
    /// track (`track_display_name`, `plug.c:559-566`).
    ///
    /// The project title wins when there is one; otherwise it is the file name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        track_identity::display_name(
            self.project_metadata
                .as_ref()
                .map(|meta| meta.title.as_str()),
            self.file_path.to_str(),
        )
    }

    /// `mark_project_dirty` (`plug.c:616-622`).
    ///
    /// Clearing `project_autosave_failed` is the part worth not losing: a fresh
    /// edit earns a fresh autosave attempt, so one failure does not silence the
    /// mechanism for the rest of the session.
    pub fn mark_dirty(&mut self, now_seconds: f64) {
        self.project_dirty = true;
        self.project_autosave_failed = false;
        self.project_dirty_since = now_seconds;
    }

    /// Recomputes `next_manual_event_id` from the timeline it must not collide
    /// with (`timeline_next_id`, `plug.c:624-634`).
    ///
    /// `u64::MAX` is skipped rather than incremented past, because `id + 1`
    /// would wrap to zero and zero is not a valid id.
    pub fn refresh_next_manual_event_id(&mut self) {
        // Delegated rather than reimplemented: `Track::record_manual_event` and
        // `ManualClear::undo` both have to agree with this rule, and two copies
        // of it is how they would stop agreeing.
        self.next_manual_event_id = event_timeline::next_id_for(&self.manual_events);
    }

    /// The settings the tuning inspector and a scene cue actually read
    /// (`track_effective_scene_settings`, `plug.c:1024-1028`).
    ///
    /// A track playing a scene-switch cue reads and writes its *playback* copy,
    /// so a cue can drive a parameter without the change surviving the cue.
    #[must_use]
    pub fn effective_settings(&self) -> &SceneSettings {
        if self.cue_settings_active {
            &self.playback_scene_settings
        } else {
            &self.scene_settings
        }
    }

    /// The per-track half of `plug_record_event` (`plug.c:1055-1069`).
    ///
    /// A thin delegation on purpose: the allocator rule and the undo retirement
    /// are [`event_timeline`]'s, tested there against the `.c`, so this cannot
    /// drift from them.
    pub fn record_manual_event(&mut self, event: EventRecord) -> Result<(), EventTimelineError> {
        event_timeline::record_into(
            &mut self.manual_events,
            &mut self.next_manual_event_id,
            event,
        )?;
        self.manual_clear.forget();
        Ok(())
    }

    /// `record_scene_cue` (`plug.c:1979-2030`): cue `scene` and its current
    /// tuning at the playhead.
    ///
    /// The first cue of a track that has switched scene since it loaded implies a
    /// second one at `0.0` for the *previous* scene, or the plan would retroject
    /// the new scene over the part of the track the user already watched under
    /// the old one (`plug.c:1997-2013`).
    pub fn record_scene_cue(
        &mut self,
        scene: SceneId,
        time_seconds: f64,
    ) -> Result<(), SceneCueError> {
        if !time_seconds.is_finite() || time_seconds < 0.0 || time_seconds >= self.duration_seconds
        {
            return Err(SceneCueError::Playhead);
        }
        let snapshot = self
            .effective_settings()
            .capture(scene)
            .ok_or(SceneCueError::Settings)?;

        let mut staged = self.scene_switches.clone();
        let scene_count = SceneId::ALL.len() as u32;
        // No `previous != scene` test here, deliberately: the C's guard is only
        // that the previous scene is a legal one (`plug.c:2000`), which `SceneId`
        // makes true by construction. Adding the comparison would drop a cue the
        // oracle stages.
        if staged.is_empty() && time_seconds > 0.001 && self.scene_selection_pending {
            let previous = self
                .scene_settings
                .capture(self.previous_base_scene)
                .ok_or(SceneCueError::Settings)?;
            staged.cue_at(
                0.0,
                self.duration_seconds,
                self.previous_base_scene.index() as u32,
                scene_count,
                1.0,
                &previous,
            )?;
        }
        staged.cue_at(
            time_seconds,
            self.duration_seconds,
            scene.index() as u32,
            scene_count,
            1.0,
            &snapshot,
        )?;

        self.scene_switches = staged;
        self.scene_selection_pending = false;
        self.cue_settings_active = false;
        Ok(())
    }

    /// True when this track has unsaved project work.
    ///
    /// A track that was never part of a project is never dirty, which is what
    /// keeps the quit guard from asking about a plain audio file.
    #[must_use]
    pub fn has_unsaved_work(&self) -> bool {
        self.project_dirty
    }
}

/// The open tracks and which one is current (`track.h:91-95`, plus `plug.c`'s
/// `current_track` index).
///
/// C stores the selection as an `int` with `-1` for "none"; here it is an
/// `Option<usize>` that cannot be out of range, because `select` is the only way
/// to move it and it validates.
///
/// There is deliberately no `remove`: the frozen C has no way to close a single
/// track, only `plug_reset` (`plug.c:8403`), and inventing one would be a
/// feature rather than parity.
#[derive(Clone, Debug)]
pub struct Workspace {
    tracks: Vec<Track>,
    current: Option<usize>,
    /// Markers recorded before any track existed (`p->event_timeline`,
    /// `plug.c:1058`), which is the only lane `--event` has to write into: the
    /// command line runs before an input is resolved.
    pending_events: EventTimeline,
    /// `p->next_event_id` (`plug.c:1062`).
    pending_next_event_id: u64,
    /// The one Assist session (Agent J). The C's sixteen `p->assist_*` fields
    /// minus the process handle, which stays in `ui::panels::assist`.
    ///
    /// It rides with the track list because it names its target by index
    /// (`p->assist_track_index`, `p->assist_candidate_track_index`), and both are
    /// indices into `tracks`.
    ///
    /// Agent J preferred moving this to `Shell` — as the route editor, the
    /// lyrics editor and the font browser all were — which would let the four
    /// `Cell`s inside it go. Not taken, and the reason is risk rather than
    /// disagreement: it is a sixty-call-site refactor of a panel that is
    /// delivered, tested and captured, done at the end of a merge. The `Cell`s
    /// guard a payload-free one-frame intent channel and nothing that owns a
    /// process, a file or a track, so what they buy is small and what they cost
    /// is smaller. Worth doing; not worth doing untested.
    pub assist: AssistSession,
}

impl Default for Workspace {
    /// `plug_init`'s event fields (`plug.c:8386`): the pending allocator starts
    /// at 1, because zero is the "no event" sentinel and never a legal id.
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            current: None,
            pending_events: EventTimeline::new(),
            pending_next_event_id: 1,
            assist: AssistSession::default(),
        }
    }
}

impl Workspace {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `plug_record_event` (`plug.c:1055-1069`).
    ///
    /// Writes into the current track's lane, or into the pending one when no
    /// track is open — which is the case every `--event` on the command line hits,
    /// because the actions run before an input is resolved.
    pub fn record_event(&mut self, event: EventRecord) -> Result<(), EventTimelineError> {
        match self.current {
            Some(index) => self.tracks[index].record_manual_event(event),
            None => event_timeline::record_into(
                &mut self.pending_events,
                &mut self.pending_next_event_id,
                event,
            ),
        }
    }

    /// The markers waiting for the first track to open.
    #[must_use]
    pub fn pending_events(&self) -> &EventTimeline {
        &self.pending_events
    }

    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    #[must_use]
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// `current_track` (`plug.c:548-556`).
    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        self.current.map(|index| &self.tracks[index])
    }

    pub fn current_mut(&mut self) -> Option<&mut Track> {
        self.current.map(|index| &mut self.tracks[index])
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Track> {
        self.tracks.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Track> {
        self.tracks.get_mut(index)
    }

    /// The scene and seed a newly loaded track inherits (`plug.c:794-795`).
    ///
    /// A new track adopts the active track's scene so that opening a second file
    /// does not silently change what is on screen. With nothing open, the
    /// caller's live scene instance supplies both.
    #[must_use]
    pub fn inherited_scene(&self, fallback_scene: SceneId, fallback_seed: u64) -> (SceneId, u64) {
        match self.current() {
            Some(track) => (track.base_scene, track.scene_seed),
            None => (fallback_scene, fallback_seed),
        }
    }

    /// Appends a track and returns its index.
    ///
    /// **It becomes current only if nothing was** (`plug.c:843-856`). Loading a
    /// second file while one is playing adds it to the list and leaves playback
    /// alone; that is the C's behaviour and it is the reason `push` returns the
    /// index instead of assuming the caller now needs to bind audio.
    pub fn push(&mut self, mut track: Track) -> usize {
        let becomes_current = self.current.is_none();
        // Markers recorded before any track existed belong to the first one
        // (`plug.c:844-851`), and the pending lane is *emptied*, not copied, so a
        // second track does not silently inherit them too. `replace` is what
        // validates them, and a lane it refuses is left where it is rather than
        // half-adopted.
        if becomes_current
            && !self.pending_events.is_empty()
            && track.manual_events.replace(&self.pending_events).is_ok()
        {
            track.refresh_next_manual_event_id();
            self.pending_events.clear();
            self.pending_next_event_id = 1;
        }
        let index = self.tracks.len();
        self.tracks.push(track);
        if becomes_current {
            self.current = Some(index);
        }
        index
    }

    /// Moves the selection. Returns `false` for an out-of-range index, leaving
    /// the selection untouched.
    ///
    /// This performs no audio work and clears no editor state: the guards
    /// (`lyric_editor_allow_context_change`, `route_editor_allow_active_context_change`)
    /// run in the shell *before* this is reached, because a refused guard must
    /// leave the selection exactly as it was (`plug.c:5261-5277`).
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        self.current = Some(index);
        true
    }

    /// Empties the workspace (`plug_reset`, `plug.c:8403`).
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
    }

    /// Whether any track holds unsaved project work — the question the quit
    /// guard asks (`plug.c:7248`).
    #[must_use]
    pub fn any_unsaved_work(&self) -> bool {
        self.tracks.iter().any(Track::has_unsaved_work)
    }

    /// The display names, in list order. What the tracks panel draws.
    pub fn display_names(&self) -> impl Iterator<Item = &str> {
        self.tracks.iter().map(Track::display_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str) -> Track {
        Track::new(PathBuf::from(name), 12.0, SceneId::Spectrum, 7)
            .expect("12s is a valid duration")
    }

    #[test]
    fn a_new_track_starts_on_the_inherited_scene_and_seed() {
        let track = track("/tmp/a.wav");
        assert_eq!(track.base_scene, SceneId::Spectrum);
        assert_eq!(track.previous_base_scene, SceneId::Spectrum);
        assert_eq!(track.scene_seed, 7);
        assert_eq!(track.scene_instance_id, 1);
        assert_eq!(track.next_manual_event_id, 1);
        // A zeroed caption style would make captions vanish; C initializes it
        // explicitly and so does this.
        assert!(track.caption_style.is_default());
    }

    #[test]
    fn the_first_track_becomes_current_and_the_second_does_not() {
        // plug.c:843 — `if (current_track() == NULL)`. Loading a second file
        // while one plays must not interrupt playback.
        let mut workspace = Workspace::new();
        assert_eq!(workspace.current_index(), None);
        assert_eq!(workspace.push(track("/tmp/a.wav")), 0);
        assert_eq!(workspace.current_index(), Some(0));
        assert_eq!(workspace.push(track("/tmp/b.wav")), 1);
        assert_eq!(
            workspace.current_index(),
            Some(0),
            "the second track must not steal the selection"
        );
    }

    #[test]
    fn selection_refuses_an_out_of_range_index_without_moving() {
        let mut workspace = Workspace::new();
        workspace.push(track("/tmp/a.wav"));
        workspace.push(track("/tmp/b.wav"));
        assert!(workspace.select(1));
        assert_eq!(workspace.current_index(), Some(1));
        assert!(!workspace.select(2));
        assert_eq!(workspace.current_index(), Some(1));
    }

    #[test]
    fn a_new_track_inherits_from_the_current_one_not_from_the_fallback() {
        let mut workspace = Workspace::new();
        let mut first = track("/tmp/a.wav");
        first.base_scene = SceneId::Loom;
        first.scene_seed = 99;
        workspace.push(first);
        assert_eq!(
            workspace.inherited_scene(SceneId::Cadence, 1),
            (SceneId::Loom, 99)
        );

        let empty = Workspace::new();
        assert_eq!(
            empty.inherited_scene(SceneId::Cadence, 1),
            (SceneId::Cadence, 1)
        );
    }

    #[test]
    fn the_display_name_prefers_the_project_title() {
        let mut track = track("/tmp/song.wav");
        assert_eq!(track.display_name(), "song.wav");
        track.project_metadata = Some(Metadata {
            title: "Nocturne".to_string(),
            ..Metadata::default()
        });
        assert_eq!(track.display_name(), "Nocturne");
    }

    #[test]
    fn marking_dirty_clears_the_autosave_failure() {
        // A fresh edit earns a fresh attempt: one failure must not silence
        // autosave for the rest of the session (plug.c:616-622).
        let mut track = track("/tmp/a.wav");
        track.project_autosave_failed = true;
        track.mark_dirty(41.5);
        assert!(track.project_dirty);
        assert!(!track.project_autosave_failed);
        assert_eq!(track.project_dirty_since, 41.5);
    }

    #[test]
    fn the_quit_guard_sees_a_dirty_track_anywhere_in_the_list() {
        let mut workspace = Workspace::new();
        workspace.push(track("/tmp/a.wav"));
        workspace.push(track("/tmp/b.wav"));
        assert!(!workspace.any_unsaved_work());
        workspace.get_mut(1).expect("two tracks").mark_dirty(0.0);
        assert!(workspace.any_unsaved_work());
    }

    #[test]
    fn the_next_manual_event_id_skips_u64_max() {
        // `id + 1` would wrap to zero, and zero is not a valid event id
        // (plug.c:624-634).
        use musializer_core::scene::events::EventRecord;
        let mut track = track("/tmp/a.wav");
        let record = EventRecord {
            timestamp_seconds: 1.0,
            id: u64::MAX,
            event_type: 1,
            value_count: 1,
            ..EventRecord::default()
        };
        track.manual_events.record(record).expect("a valid record");
        track.refresh_next_manual_event_id();
        assert_eq!(track.next_manual_event_id, 1);
    }

    fn marker(seconds: f64, id: u64) -> EventRecord {
        use musializer_core::project::event_timeline::manual_marker;
        use musializer_core::scene::events::EventType;
        manual_marker(seconds, id, EventType::Custom).expect("a usable id")
    }

    #[test]
    fn a_track_owns_its_own_clear_confirmation_and_undo_buffer() {
        // The state machine itself is tested in `event_timeline`; what this
        // pins is the divergence — the buffer is per track, so undoing on B
        // cannot restore A's markers (plug.c:2944).
        use musializer_core::project::event_timeline::ClearButton;
        let mut workspace = Workspace::new();
        workspace.push(track("/tmp/a.wav"));
        workspace.push(track("/tmp/b.wav"));

        let first = workspace.get_mut(0).expect("two tracks");
        first.record_manual_event(marker(1.0, 1)).unwrap();
        first.manual_clear.arm();
        first
            .manual_clear
            .clear(&mut first.manual_events, &mut first.next_manual_event_id);

        assert_eq!(
            workspace.get(0).expect("two tracks").manual_clear.button(),
            ClearButton::Undo
        );
        assert_eq!(
            workspace.get(1).expect("two tracks").manual_clear.button(),
            ClearButton::Clear,
            "the other track must not be offered someone else's markers"
        );
    }

    #[test]
    fn events_recorded_before_any_track_are_handed_to_the_first_one() {
        // plug.c:844-851 — which is the only lane `--event` can reach, because
        // the command-line actions run before an input is resolved.
        let mut workspace = Workspace::new();
        workspace.record_event(marker(1.0, 1)).unwrap();
        workspace.record_event(marker(2.0, 2)).unwrap();
        assert_eq!(workspace.pending_events().len(), 2);

        workspace.push(track("/tmp/a.wav"));
        let first = workspace.current().expect("the first track is current");
        assert_eq!(first.manual_events.len(), 2);
        assert_eq!(first.next_manual_event_id, 3);
        assert!(workspace.pending_events().is_empty());

        // A second track inherits nothing, and a recorded event now lands on the
        // current track instead of the pending lane.
        workspace.push(track("/tmp/b.wav"));
        assert!(workspace
            .get(1)
            .expect("two tracks")
            .manual_events
            .is_empty());
        workspace.record_event(marker(3.0, 7)).unwrap();
        assert_eq!(workspace.current().expect("current").manual_events.len(), 3);
    }

    #[test]
    fn a_scene_cue_refuses_a_playhead_at_or_past_the_end() {
        let mut track = track("/tmp/a.wav");
        assert_eq!(
            track.record_scene_cue(SceneId::Spectrum, 12.0),
            Err(SceneCueError::Playhead)
        );
        assert_eq!(
            track.record_scene_cue(SceneId::Spectrum, f64::NAN),
            Err(SceneCueError::Playhead)
        );
        assert!(track.scene_switches.is_empty());
        track.record_scene_cue(SceneId::Spectrum, 4.0).unwrap();
        assert_eq!(track.scene_switches.len(), 1);
    }

    #[test]
    fn the_first_cue_of_a_switched_track_also_covers_what_came_before_it() {
        // plug.c:1997-2013: without the implied cue at 0.0 the plan would
        // retroject the new scene over the part already watched under the old.
        let mut track = track("/tmp/a.wav");
        track.previous_base_scene = SceneId::Loom;
        track.base_scene = SceneId::Spectrum;
        track.scene_selection_pending = true;
        track.record_scene_cue(SceneId::Spectrum, 4.0).unwrap();
        assert_eq!(track.scene_switches.len(), 2);
        assert_eq!(track.scene_switches.cues()[0].start_seconds, 0.0);
        assert_eq!(
            track.scene_switches.cues()[0].scene_index,
            SceneId::Loom.index() as u32
        );
        assert!(!track.scene_selection_pending);
    }
}
