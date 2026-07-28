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
use musializer_core::project::event_timeline::EventTimeline;
use musializer_core::project::lyrics::{LyricsDocument, LyricsError};
use musializer_core::project::model::{AnalysisLaneReference, CaptionStyle, Metadata};
use musializer_core::project::preset_store::PresetLibrary;
use musializer_core::project::scene_switch::SceneSwitchTimeline;
use musializer_core::scene::routes::RouteTable;
use musializer_core::scene::{SceneId, SceneSettings};
use musializer_core::scenes::ascii_field::ascii_art::Grid as AsciiGrid;
use musializer_core::timing::render_export::RenderExportConfig;
use musializer_core::timing::track_identity;
use musializer_core::timing::track_timeline::Waveform;

/// An ASCII source image bound to a track (`track.h`'s `ascii_cells`,
/// `ascii_columns`, `ascii_rows`, `ascii_image_path`, `ascii_image_sha256`).
#[derive(Clone, Debug)]
pub struct AsciiImage {
    pub grid: AsciiGrid,
    pub path: PathBuf,
    /// Hex SHA-256 of the source file. Empty when hashing was deferred, exactly
    /// as C leaves the buffer empty rather than failing the load.
    pub sha256: String,
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
        let mut next = 1u64;
        for event in self.manual_events.events() {
            if event.id >= next && event.id != u64::MAX {
                next = event.id + 1;
            }
        }
        self.next_manual_event_id = next;
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
#[derive(Clone, Debug, Default)]
pub struct Workspace {
    tracks: Vec<Track>,
    current: Option<usize>,
}

impl Workspace {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
    pub fn push(&mut self, track: Track) -> usize {
        let index = self.tracks.len();
        self.tracks.push(track);
        if self.current.is_none() {
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
}
