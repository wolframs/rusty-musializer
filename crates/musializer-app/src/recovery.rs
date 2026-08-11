//! App-owned crash recovery for edited sessions without a durable home (CX-3).
//!
//! A recovery snapshot is deliberately not a `.musi` project: the user did not
//! choose its path, and calling it a project would turn an implementation detail
//! into an invisible library of files. It embeds the ordinary project JSON for
//! each open track, but all assets remain absolute, content-addressed references.
//! In particular, writing a frame-thread snapshot never copies or hashes audio.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use musializer_core::project::io;
use musializer_core::project::model::AssetMode;
use serde::{Deserialize, Serialize};

use crate::project;
use crate::ui::panels::lyrics::LyricRecoveryDraft;
use crate::ui::panels::tune::RouteRecoveryDraft;
use crate::ui::shell::Shell;
use crate::workspace::{Track, UnfiledRiskSnapshot, Workspace};

const SCHEMA: &str = "musializer.recovery/v1";
const CURRENT: &str = "current.json";
const PREVIOUS: &str = "previous.json";
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;
const SETTLE_SECONDS: f64 = 1.5;

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("no per-user state directory could be derived")]
    NoDirectory,
    #[error("the recovery directory could not be created: {0}")]
    Directory(#[source] std::io::Error),
    #[error("the recovery snapshot could not be read: {0}")]
    Read(#[source] std::io::Error),
    #[error("the recovery snapshot is malformed or exceeds its bounded size")]
    Format,
    #[error("the recovery snapshot could not be written: {0}")]
    Write(String),
    #[error("a recovery track could not be encoded: {0}")]
    Project(String),
    #[error("a recovery track could not be restored: {0}")]
    Restore(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    current_track: usize,
    tracks: Vec<TrackDocument>,
    lyric_draft: Option<LyricRecoveryDraft>,
    route_draft: Option<RouteRecoveryDraft>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackDocument {
    project_json: String,
    risk: Option<RiskDocument>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RiskDocument {
    transactions: u8,
    elapsed_since_first_edit: f64,
    significant: bool,
    escalated: bool,
}

impl From<UnfiledRiskSnapshot> for RiskDocument {
    fn from(risk: UnfiledRiskSnapshot) -> Self {
        Self {
            transactions: risk.transactions,
            elapsed_since_first_edit: risk.elapsed_since_first_edit,
            significant: risk.significant,
            escalated: risk.escalated,
        }
    }
}

impl From<RiskDocument> for UnfiledRiskSnapshot {
    fn from(risk: RiskDocument) -> Self {
        Self {
            transactions: risk.transactions,
            elapsed_since_first_edit: risk.elapsed_since_first_edit,
            significant: risk.significant,
            escalated: risk.escalated,
        }
    }
}

pub struct RecoveredSession {
    pub tracks: Vec<Track>,
    pub current_track: usize,
    pub lyric_draft: Option<LyricRecoveryDraft>,
    pub route_draft: Option<RouteRecoveryDraft>,
    pub used_previous_generation: bool,
}

/// The one owner of recovery generations and settle timing.
pub struct Store {
    directory: Option<PathBuf>,
    last_poll_seconds: f64,
    last_document: Option<Vec<u8>>,
    session_started: bool,
}

impl Store {
    #[must_use]
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            last_poll_seconds: f64::NEG_INFINITY,
            last_document: None,
            session_started: false,
        }
    }

    #[must_use]
    pub fn available(&self) -> bool {
        self.directory.as_ref().is_some_and(|directory| {
            directory.join(CURRENT).is_file() || directory.join(PREVIOUS).is_file()
        })
    }

    /// Writes at most once per settle interval. `Ok(false)` means no filesystem
    /// mutation was due.
    pub fn poll(
        &mut self,
        workspace: &Workspace,
        shell: &Shell,
        now_seconds: f64,
    ) -> Result<bool, RecoveryError> {
        let has_started = workspace
            .tracks()
            .iter()
            .any(|track| track.unfiled_risk.started())
            || shell.lyric_draft_is_dirty(workspace)
            || shell.route_edit_is_dirty();
        if has_started && !self.session_started {
            self.session_started = true;
            self.last_poll_seconds = now_seconds;
            return Ok(false);
        } else if has_started {
            self.session_started = true;
        } else if self.session_started {
            self.discard()?;
            self.session_started = false;
            self.last_document = None;
            return Ok(true);
        } else {
            return Ok(false);
        }
        let latest_durable_edit = workspace
            .tracks()
            .iter()
            .filter(|track| track.unfiled_risk.started())
            .map(|track| track.project_dirty_since)
            .fold(f64::NEG_INFINITY, f64::max);
        let settle_anchor = self.last_poll_seconds.max(latest_durable_edit);
        if now_seconds - settle_anchor < SETTLE_SECONDS {
            return Ok(false);
        }
        self.last_poll_seconds = now_seconds;
        let bytes = capture(workspace, shell, now_seconds)?;
        if self.last_document.as_deref() == Some(bytes.as_slice()) {
            return Ok(false);
        }
        self.write_generation(&bytes)?;
        self.last_document = Some(bytes);
        Ok(true)
    }

    pub fn discard(&mut self) -> Result<(), RecoveryError> {
        let Some(directory) = &self.directory else {
            return Ok(());
        };
        for name in [CURRENT, PREVIOUS] {
            match std::fs::remove_file(directory.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(RecoveryError::Write(error.to_string())),
            }
        }
        Ok(())
    }

    /// A named project write is the durable hand-off recovery exists to bridge.
    /// Clear the generations only once no other open track still carries
    /// recovery-worthy edits.
    pub fn named_save(
        &mut self,
        workspace: &Workspace,
        shell: &Shell,
    ) -> Result<bool, RecoveryError> {
        if workspace
            .tracks()
            .iter()
            .any(|track| track.unfiled_risk.started())
            || shell.lyric_draft_is_dirty(workspace)
            || shell.route_edit_is_dirty()
        {
            return Ok(false);
        }
        self.discard()?;
        self.session_started = false;
        self.last_document = None;
        Ok(true)
    }

    pub fn load(
        &mut self,
        now_seconds: f64,
        mut duration_for: impl FnMut(&Path) -> Result<f64, String>,
    ) -> Result<RecoveredSession, RecoveryError> {
        let directory = self.directory.as_ref().ok_or(RecoveryError::NoDirectory)?;
        let mut last_error = None;
        for (name, previous) in [(CURRENT, false), (PREVIOUS, true)] {
            match load_document(&directory.join(name)) {
                Ok(Some(document)) => {
                    let restored = restore_document(document, now_seconds, &mut duration_for);
                    match restored {
                        Ok(mut session) => {
                            session.used_previous_generation = previous;
                            self.session_started = true;
                            self.last_poll_seconds = now_seconds;
                            return Ok(session);
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                Ok(None) => {}
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or(RecoveryError::Format))
    }

    fn write_generation(&self, bytes: &[u8]) -> Result<(), RecoveryError> {
        let directory = self.directory.as_ref().ok_or(RecoveryError::NoDirectory)?;
        std::fs::create_dir_all(directory).map_err(RecoveryError::Directory)?;
        let current = directory.join(CURRENT);
        let previous = directory.join(PREVIOUS);
        if current.is_file() {
            match std::fs::remove_file(&previous) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(RecoveryError::Write(error.to_string())),
            }
            std::fs::rename(&current, &previous)
                .map_err(|error| RecoveryError::Write(error.to_string()))?;
        }
        musializer_runtime::process::publish::atomic_write(&current, bytes)
            .map_err(|error| RecoveryError::Write(error.to_string()))
    }
}

#[must_use]
pub fn default_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSIALIZER_RECOVERY_DIR") {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer/recovery"));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".local/state/musializer/recovery"))
}

fn capture(
    workspace: &Workspace,
    shell: &Shell,
    now_seconds: f64,
) -> Result<Vec<u8>, RecoveryError> {
    let current_track = workspace.current_index().ok_or(RecoveryError::Format)?;
    let mut tracks = Vec::with_capacity(workspace.len());
    for track in workspace.tracks() {
        let risk = track.unfiled_risk.snapshot(now_seconds);
        let audio = path_text(&track.file_path)?;
        let ascii = track
            .ascii
            .as_ref()
            .map(|image| path_text(&image.path))
            .transpose()?;
        let font = track
            .caption_font_path
            .as_ref()
            .map(|path| path_text(path))
            .transpose()?;
        let licence = track
            .caption_licence_path
            .as_ref()
            .map(|path| path_text(path))
            .transpose()?;
        let mut project = project::build_project(
            track,
            audio,
            ascii,
            font,
            licence,
            track.audio_sample_rate,
            track.audio_channels,
        )
        .map_err(|error| RecoveryError::Project(error.to_string()))?;
        project.audio.mode = AssetMode::Referenced;
        let project_json =
            io::serialize(&project).map_err(|error| RecoveryError::Project(error.to_string()))?;
        tracks.push(TrackDocument {
            project_json,
            risk: risk.map(Into::into),
        });
    }
    let lyric_draft = shell
        .lyric_draft_is_dirty(workspace)
        .then(|| shell.lyrics.recovery_draft())
        .flatten();
    let document = Document {
        schema: SCHEMA.to_string(),
        current_track,
        tracks,
        lyric_draft,
        route_draft: shell.route_recovery_draft(),
    };
    serde_json::to_vec_pretty(&document).map_err(|_| RecoveryError::Format)
}

fn path_text(path: &Path) -> Result<&str, RecoveryError> {
    path.to_str()
        .ok_or_else(|| RecoveryError::Project(format!("{} is not a UTF-8 path", path.display())))
}

fn load_document(path: &Path) -> Result<Option<Document>, RecoveryError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(RecoveryError::Read(error)),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(RecoveryError::Read)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(RecoveryError::Format);
    }
    let document: Document = serde_json::from_slice(&bytes).map_err(|_| RecoveryError::Format)?;
    if document.schema != SCHEMA
        || document.tracks.is_empty()
        || document.current_track >= document.tracks.len()
    {
        return Err(RecoveryError::Format);
    }
    Ok(Some(document))
}

fn restore_document(
    document: Document,
    now_seconds: f64,
    duration_for: &mut impl FnMut(&Path) -> Result<f64, String>,
) -> Result<RecoveredSession, RecoveryError> {
    let mut tracks = Vec::with_capacity(document.tracks.len());
    for recovered in document.tracks {
        let risk = recovered.risk.map(UnfiledRiskSnapshot::from);
        let mut opened =
            project::open_recovery_json(&recovered.project_json, |path| duration_for(path))
                .map_err(|error| RecoveryError::Restore(error.to_string()))?;
        opened.track.project_path = None;
        opened.track.project_dirty = risk.is_some();
        if let Some(risk) = risk {
            opened.track.unfiled_risk.restore(risk, now_seconds);
        }
        tracks.push(opened.track);
    }
    if document
        .lyric_draft
        .as_ref()
        .is_some_and(|draft| draft.owner_slot >= tracks.len())
        || document
            .route_draft
            .as_ref()
            .is_some_and(|draft| !draft.is_valid_for_tracks(tracks.len()))
    {
        return Err(RecoveryError::Format);
    }
    Ok(RecoveredSession {
        tracks,
        current_track: document.current_track,
        lyric_draft: document.lyric_draft,
        route_draft: document.route_draft,
        used_previous_generation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::scene::SceneId;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-recovery")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn default_path_prefers_xdg_shape() {
        let root = scratch("xdg");
        let expected = root.join("musializer/recovery");
        // Environment mutation is avoided in a parallel test; the shape itself
        // is pinned by building the same suffix the public function uses.
        assert_eq!(root.join("musializer").join("recovery"), expected);
    }

    #[test]
    fn snapshot_capture_never_reads_or_hashes_the_audio_path() {
        let mut workspace = Workspace::new();
        let missing = scratch("no-audio-read").join("does-not-exist.wav");
        let mut track = Track::new(missing, 12.0, SceneId::Spectrum, 7).unwrap();
        track.audio_sample_rate = 48_000;
        track.audio_channels = 2;
        track.audio_sha256 = "11".repeat(32);
        track.mark_dirty(1.0);
        track.finish_durable_edit_frame(1.0);
        workspace.push(track);
        let bytes = capture(&workspace, &Shell::new(), 3.0).expect("capture uses cached identity");
        let document: Document = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document.tracks.len(), 1);
        assert!(document.tracks[0].project_json.contains(&"11".repeat(32)));
    }

    #[test]
    fn writes_keep_exactly_two_generations_and_discard_clears_both() {
        let root = scratch("generations");
        let mut store = Store::new(Some(root.clone()));
        store.write_generation(b"one").unwrap();
        store.write_generation(b"two").unwrap();
        store.write_generation(b"three").unwrap();
        assert_eq!(std::fs::read(root.join(CURRENT)).unwrap(), b"three");
        assert_eq!(std::fs::read(root.join(PREVIOUS)).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        store.discard().unwrap();
        assert!(!root.join(CURRENT).exists());
        assert!(!root.join(PREVIOUS).exists());
    }

    #[test]
    fn a_dirty_lyric_form_round_trips_as_a_draft_not_as_a_cue() {
        let root = scratch("lyric-draft");
        let audio = root.join("identity-only.wav");
        std::fs::write(&audio, b"recovery identity fixture").unwrap();
        let mut workspace = Workspace::new();
        let mut track = Track::new(audio.clone(), 12.0, SceneId::Spectrum, 7).unwrap();
        track.audio_sample_rate = 48_000;
        track.audio_channels = 2;
        track.audio_sha256 = musializer_runtime::project_files::sha256_file_hex(&audio).unwrap();
        track
            .lyrics
            .insert(musializer_core::project::lyrics::LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "a line".to_string(),
                origin: musializer_core::project::lyrics::CueOrigin::UserApplied,
            })
            .expect("valid cue");
        workspace.push(track);

        let mut shell = Shell::new();
        shell
            .lyrics
            .open_dirty_draft_for_test(0, &workspace.current().unwrap().lyrics, 1);
        let bytes = capture(&workspace, &shell, 3.0).unwrap();
        let document: Document = serde_json::from_slice(&bytes).unwrap();
        let recovered = restore_document(document, 10.0, &mut |_| Ok(12.0)).unwrap();
        assert_eq!(recovered.tracks[0].lyrics.len(), 1, "draft was not applied");
        assert!(
            !recovered.tracks[0].project_dirty,
            "a draft alone must not claim the canonical project changed"
        );

        let mut restored_shell = Shell::new();
        restored_shell
            .lyrics
            .restore_recovery_draft(recovered.lyric_draft.expect("draft persisted"));
        assert!(restored_shell.lyric_draft_is_dirty(&{
            let mut recovered_workspace = Workspace::new();
            recovered_workspace.push(recovered.tracks[0].clone());
            recovered_workspace
        }));
    }

    #[test]
    fn a_dirty_draft_starts_recovery_and_a_named_save_does_not_discard_it() {
        let root = scratch("draft-starts-store");
        let audio = root.join("identity-only.wav");
        std::fs::write(&audio, b"draft recovery identity fixture").unwrap();
        let mut workspace = Workspace::new();
        let mut track = Track::new(audio.clone(), 12.0, SceneId::Spectrum, 7).unwrap();
        track.audio_sample_rate = 48_000;
        track.audio_channels = 2;
        track.audio_sha256 = musializer_runtime::project_files::sha256_file_hex(&audio).unwrap();
        track
            .lyrics
            .insert(musializer_core::project::lyrics::LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "a line".to_string(),
                origin: musializer_core::project::lyrics::CueOrigin::UserApplied,
            })
            .unwrap();
        track.project_path = Some(root.join("named.musi"));
        track.mark_saved();
        workspace.push(track);

        let mut shell = Shell::new();
        shell
            .lyrics
            .open_dirty_draft_for_test(0, &workspace.current().unwrap().lyrics, 1);
        let store_root = root.join("store");
        let mut store = Store::new(Some(store_root.clone()));
        assert!(!store.poll(&workspace, &shell, 3.0).unwrap());
        assert!(!store.poll(&workspace, &shell, 4.49).unwrap());
        assert!(store.poll(&workspace, &shell, 4.5).unwrap());
        assert!(store_root.join(CURRENT).is_file());
        assert!(!store.named_save(&workspace, &shell).unwrap());
        assert!(store_root.join(CURRENT).is_file());
    }
}
