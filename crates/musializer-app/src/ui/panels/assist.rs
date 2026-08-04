//! The Assist confirmation panel, and the job it supervises.
//!
//! **Owner: Agent J.** C sources: `draw_assist_panel` (`plug.c:2166-2540`),
//! `choose_lyric_reference` (`:2140-2163`), `resolve_lyric_reference`
//! (`:2113-2138`), `draw_assist_artifact_actions` (`:2069-2110`),
//! `load_analysis_candidate_for_track` (`:3408-3500`),
//! `stage_candidate_analysis_lanes` (`:3502-3563`), `apply_candidate_to_track`
//! (`:3565-3623`), `apply_assist_candidate` (`:3625-3667`),
//! `discard_assist_candidate` (`:3669-3686`) and `plug_load_analysis_bridge`
//! (`:3689-3722`).
//!
//! Everything underneath was already ported: `core::project::analysis_bridge`
//! reads the helper's document, `core::project::analysis_candidate` stages it,
//! `core::ui::assist_ui_state` decides what the panel shows, and
//! `runtime::process::assist` supervises the process tree.
//!
//! # The split
//!
//! [`AssistController`] owns the process and the filesystem. The panel owns
//! pixels. Neither owns the decision of *what* the panel shows — that is
//! `assist_ui_state`, raylib-free and headlessly tested, so the layout can be
//! asserted without a window.
//!
//! **A job finishing must never mutate a project.** A completed run becomes an
//! `AnalysisCandidate` on the session, inert until the user presses Apply twice.
//! That invariant is why this module reports and stages rather than applying,
//! and it is enforced a second time inside `AnalysisCandidate::apply`, which
//! refuses a candidate carrying more than its mode authorized.
//!
//! # Do not touch `process::assist`'s process-group handling
//!
//! `os.setsid()` in `tools/external_analysis.py` fails with `EPERM` if the
//! caller is already a group leader, so giving the child its own group from the
//! parent kills the helper at startup. There is a test in
//! `runtime::process::assist` that fails loudly with that reason, and the
//! `ESRCH` fallback from `kill(-pid)` to `kill(pid)` covers the race that
//! leaves. Nothing here changes any of it: this module only calls
//! `AssistJob::start`, `poll`, `request_stop` and `cancel_blocking`.

// `AssistController` and everything it calls has no caller in the binary yet:
// `main.rs` owns one, and `main.rs` is the integration owner's file. The exact
// diff is in Agent J's NOTE ENTRIES section. Every item under this allow is
// exercised by this module's own tests, so the allow is about the *binary*, not
// about the code being unreachable. **Delete it with that diff** — after it, a
// dead item here really is dead.
#![allow(
    dead_code,
    reason = "the controller's caller is main.rs; see Agent J's note for the wiring diff"
)]

use std::path::{Path, PathBuf};

use musializer_core::project::analysis_bridge;
use musializer_core::project::analysis_candidate::{
    self, AnalysisCandidate, Lanes, LyricReviewEntry, LyricReviewKind, LyricsReview,
};
use musializer_core::project::model::{
    AnalysisLaneKind, AnalysisLaneReference, Provenance, MAX_ANALYSIS_LANES,
};
use musializer_core::project::scene_switch::SceneSwitchCue;
use musializer_core::scene::{SceneId, SCENE_COUNT};
use musializer_core::ui::assist_ui_state::{
    self, AssistArtifact, AssistConfirmationButtons, AssistJobState, AssistLyricReference,
    AssistMode, AssistPanelContent, AssistRequest, AssistSession, AssistStartBlock,
    AssistStatusInputs, AssistStatusTone, AssistUiLayout,
};
use musializer_core::ui::notice::Severity;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::UiFonts;
use musializer_runtime::process::assist::{
    AssistJob, AssistMode as JobMode, AssistPoll, AssistSpec, StopReason,
};
use musializer_runtime::process::font_import::find_assist_helper;
use musializer_runtime::project_files::sha256_file_hex;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::shell_layout::WorkspaceFrame;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::workspace::{Track, Workspace};

/// Widget id namespace for this panel.
///
/// `widgets::id` lives in `ui/widgets.rs`, which no leaf agent in this fan-out
/// may edit, so the namespace lives here until the integration owner moves it.
/// The value is ASCII `J` rather than the next free small integer precisely so
/// that two agents each picking "the next one" cannot collide before the merge —
/// a colliding id lets one panel's button release another's press.
const ASSIST_WIDGETS: u32 = b'J' as u32;

/// Bounded before it is read, so an accidental pick — a whole album's worth of
/// text, or a binary — is refused where the user can see it rather than deep
/// inside the helper (`ASSIST_LYRIC_REFERENCE_BYTE_LIMIT`, `plug.c:2138`).
pub const LYRIC_REFERENCE_BYTE_LIMIT: u64 = 1024 * 1024;

/// Analysis workspaces live under this directory (`plug.c:3265`).
pub const ANALYSIS_ROOT: &str = "./build/analysis";

/// `UI_WIDGETS_DJB2_INIT` (`ui_widgets.h:46`), used for the per-track workspace
/// directory name.
const DJB2_INIT: u64 = 5381;

/// One thing to say in the notice tray.
///
/// Returned rather than pushed, because the tray belongs to `Shell` and this
/// module runs from the frame loop where `Shell` is reachable but not borrowed.
/// Owned strings: every one of these interpolates a track name or a helper's own
/// message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistNotice {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

impl AssistNotice {
    fn new(severity: Severity, title: &str, detail: impl Into<String>) -> Self {
        Self {
            severity,
            title: title.to_string(),
            detail: detail.into(),
        }
    }
}

/// Something only the frame loop can do, because it blocks or needs raylib.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistEffect {
    /// Open the lyric-sheet picker, then call [`AssistController::set_lyric_sheet`]
    /// with what it returned. A modal picker blocks until the user answers, and
    /// doing that inside a begin/end drawing pair would hold the frame open
    /// across it.
    ChooseLyricSheet,
    /// Put this on the clipboard (`SetClipboardText`, `plug.c:2098`).
    Clipboard(String),
}

/// What handling a request produced.
#[derive(Clone, Debug, Default)]
pub struct AssistOutcome {
    pub notices: Vec<AssistNotice>,
    pub effect: Option<AssistEffect>,
}

impl AssistOutcome {
    fn of(notices: Vec<AssistNotice>) -> Self {
        Self {
            notices,
            effect: None,
        }
    }
}

/// The Assist job supervisor: the half of the session that owns a process tree.
///
/// Kept out of `AssistSession` — and therefore out of `musializer-core` — so the
/// drawable state stays `Clone`, raylib-free and OS-free. The frame loop owns one
/// of these for the whole run.
#[derive(Debug)]
pub struct AssistController {
    job: Option<AssistJob>,
    /// Carried across jobs so artifact names keep advancing
    /// (`p->assist_job_nonce`).
    nonce: u64,
    /// `tools/external_analysis.py`, resolved once at startup. The C probes with
    /// `FileExists` every frame from inside the drawing pair; once is enough, and
    /// a syscall per frame is not free.
    helper: Option<PathBuf>,
}

impl AssistController {
    /// `application_directory` is raylib's `GetApplicationDirectory()` — the
    /// directory of the running executable, not the working directory.
    #[must_use]
    pub fn new(application_directory: &Path) -> Self {
        Self {
            job: None,
            nonce: 0,
            helper: find_assist_helper(application_directory),
        }
    }

    #[must_use]
    pub fn helper_available(&self) -> bool {
        self.helper.is_some()
    }

    /// The resolved helper path, for the report line.
    #[must_use]
    pub fn helper(&self) -> Option<&Path> {
        self.helper.as_deref()
    }

    /// Whether a helper process is alive right now.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.job.as_ref().is_some_and(|job| !job.is_finished())
    }

    /// Dispatches one panel request. Called by the frame loop **after** the
    /// drawing pair has closed: every arm of this either spawns, signals, reads a
    /// file or rewrites a track.
    pub fn handle(
        &mut self,
        request: AssistRequest,
        workspace: &mut Workspace,
        now: f64,
    ) -> AssistOutcome {
        match request {
            AssistRequest::Start => AssistOutcome::of(self.start(workspace, now)),
            AssistRequest::DismissConfirmation => {
                workspace.assist.set_confirmation_pending(false);
                AssistOutcome::default()
            }
            AssistRequest::CancelJob => AssistOutcome::of(self.request_cancel(workspace)),
            AssistRequest::Apply => AssistOutcome::of(self.apply(workspace, now)),
            AssistRequest::Discard => AssistOutcome::of(self.discard(workspace)),
            AssistRequest::ChooseLyricSheet => AssistOutcome {
                notices: Vec::new(),
                effect: Some(AssistEffect::ChooseLyricSheet),
            },
            AssistRequest::ClearLyricSheet => {
                if let Some(track) = workspace.current_mut() {
                    track.lyrics_reference_path = None;
                }
                AssistOutcome::default()
            }
            AssistRequest::Copy(artifact) => {
                let path = workspace.assist.artifact_path(artifact).to_string();
                AssistOutcome {
                    notices: Vec::new(),
                    effect: (!path.is_empty()).then_some(AssistEffect::Clipboard(path)),
                }
            }
        }
    }

    /// `start_assist_job` (`plug.c:3253-3385`), minus the argv and the artifact
    /// reservation, which are `runtime::process::assist`'s.
    ///
    /// The output directory is per track and **stable across jobs**, keyed by a
    /// djb2 of the audio path and the decoded duration, so the Python layer can
    /// reuse its measured and model caches (`plug.c:3262-3280`).
    pub fn start(&mut self, workspace: &mut Workspace, now: f64) -> Vec<AssistNotice> {
        let block = workspace.assist.start_block();
        if block != AssistStartBlock::Allowed {
            return vec![AssistNotice::new(
                Severity::Warning,
                "Analysis could not start",
                block.reason(),
            )];
        }
        let Some(index) = workspace.current_index() else {
            return vec![AssistNotice::new(
                Severity::Warning,
                "Analysis could not start",
                "There is no track to analyse.",
            )];
        };
        let Some(helper) = self.helper.clone() else {
            return vec![AssistNotice::new(
                Severity::Error,
                "Analysis could not start",
                "The Assist helper script is missing from this installation.",
            )];
        };
        let mode = workspace.assist.mode();
        let (audio, duration, sheet) = {
            let track = &workspace.tracks()[index];
            (
                track.file_path.clone(),
                track.duration_seconds,
                // Only an explicitly chosen sheet, and only for a mode that
                // aligns authored lyrics (`plug.c:3344-3351`). Sibling discovery
                // stays in the helper, which is also where the embedded-tag
                // fallback this side cannot see without ffprobe lives.
                mode.uses_lyric_reference()
                    .then(|| track.lyrics_reference_path.clone())
                    .flatten(),
            )
        };
        let output_dir = PathBuf::from(format!(
            "{ANALYSIS_ROOT}/{:016x}",
            workspace_hash(&audio, duration)
        ));

        let spec = AssistSpec {
            helper: &helper,
            audio: &audio,
            output_dir: &output_dir,
            duration_seconds: duration,
            mode: job_mode(mode),
            lyrics_file: sheet.as_deref(),
        };
        match AssistJob::start(&spec, self.nonce) {
            Ok(job) => {
                self.nonce = job.artifacts().nonce;
                let session = &mut workspace.assist;
                session.bridge_path = job.bridge_path().display().to_string();
                session.log_path = job.log_path().display().to_string();
                session.output_dir = output_dir.display().to_string();
                session.failure_detail.clear();
                session.job_state = AssistJobState::Running;
                session.job_track = Some(index);
                session.started_at = now;
                session.set_confirmation_pending(false);
                self.job = Some(job);
                Vec::new()
            }
            Err(error) => {
                let session = &mut workspace.assist;
                session.job_state = AssistJobState::Failed;
                session.failure_detail = capitalize_sentence(&error.to_string());
                vec![AssistNotice::new(
                    Severity::Error,
                    "Analysis could not start",
                    session.failure_detail.clone(),
                )]
            }
        }
    }

    /// `poll_assist_job` (`plug.c:3933-4086`), minus the process arithmetic.
    ///
    /// Called once per frame. A zero exit is **not** by itself a valid result:
    /// the bridge still has to parse, still has to match this audio's digest, and
    /// still has to produce a lane the mode authorized (`plug.c:4051-4076`).
    pub fn poll(&mut self, workspace: &mut Workspace) -> Vec<AssistNotice> {
        let Some(job) = self.job.as_mut() else {
            return Vec::new();
        };
        let poll = match job.poll() {
            Ok(poll) => poll,
            Err(error) => {
                let detail = format!("The helper could not be waited for: {error}");
                self.job = None;
                workspace.assist.job_state = AssistJobState::Failed;
                workspace.assist.failure_detail = detail.clone();
                return vec![AssistNotice::new(
                    Severity::Error,
                    "Analysis failed",
                    detail,
                )];
            }
        };
        // The supervisor owns the 40:00 deadline, and it moves the *panel* state
        // too: a job past it reads as timing out rather than as running.
        if job.stopping() == Some(StopReason::TimedOut)
            && workspace.assist.job_state == AssistJobState::Running
        {
            workspace.assist.job_state = AssistJobState::TimingOut;
        }

        match poll {
            AssistPoll::Running => Vec::new(),
            AssistPoll::Stopped(reason) => {
                self.job = None;
                workspace.assist.job_state = match reason {
                    StopReason::Cancelled => AssistJobState::Cancelled,
                    StopReason::TimedOut => AssistJobState::TimedOut,
                    StopReason::Failing => AssistJobState::Failed,
                };
                Vec::new()
            }
            AssistPoll::Failed => {
                self.job = None;
                let session = &mut workspace.assist;
                session.job_state = AssistJobState::Failed;
                session.failure_detail =
                    "The helper exited before producing a validated result.".to_string();
                let log = session.log_path.clone();
                vec![AssistNotice::new(
                    Severity::Error,
                    "Analysis failed",
                    format!("The helper exited before producing a validated result. Log: {log}"),
                )]
            }
            AssistPoll::Succeeded => {
                self.job = None;
                self.stage_finished_job(workspace)
            }
        }
    }

    /// Reads, validates and stages the bridge a finished job wrote
    /// (`plug.c:4051-4086`).
    fn stage_finished_job(&mut self, workspace: &mut Workspace) -> Vec<AssistNotice> {
        let index = workspace.assist.job_track;
        let mode = workspace.assist.mode();
        let bridge_path = PathBuf::from(workspace.assist.bridge_path.clone());
        let output_dir = workspace.assist.output_dir.clone();
        let lanes = assist_ui_state::mode_lanes(mode);

        let staged = match index.and_then(|index| workspace.get(index)) {
            None => Err("The track this analysis targeted is no longer available.".to_string()),
            Some(track) => load_candidate(&bridge_path, track, lanes),
        };
        match staged {
            Err(detail) => {
                let session = &mut workspace.assist;
                session.job_state = AssistJobState::Failed;
                session.failure_detail = detail.clone();
                vec![AssistNotice::new(
                    Severity::Error,
                    "Analysis result was rejected",
                    detail,
                )]
            }
            Ok(mut loaded) => {
                // The LT1 review surface, read from the job's own artifacts
                // rather than from the bridge. Staged with the candidate, so it
                // is as inert as the candidate is.
                stage_lyrics_review(&mut loaded.candidate, &output_dir);
                // Written back exactly as C does (`plug.c:3446-3450`), so the
                // next run does not hash the whole file again.
                if let Some(digest) = loaded.audio_sha256 {
                    if let Some(track) = index.and_then(|index| workspace.get_mut(index)) {
                        track.audio_sha256 = digest;
                    }
                }
                let name = index
                    .and_then(|index| workspace.get(index))
                    .map_or("the missing target track", Track::display_name)
                    .to_string();
                let preview = first_lyric(&loaded.candidate);
                let available = loaded.candidate.available();
                let session = &mut workspace.assist;
                session.job_state = AssistJobState::Succeeded;
                session.set_apply_confirmation_pending(false);
                // "Authorized but produced nothing" is a truthful terminal
                // outcome, not a staged no-op (`assist_ui_state.h:105-107`).
                // Nothing is staged, and the panel says what the run left alone.
                if !assist_ui_state::result_has_changes(lanes_bits(lanes), lanes_bits(available)) {
                    session.candidate = None;
                    session.candidate_track = None;
                    session.candidate_first_lyric.clear();
                    return vec![AssistNotice::new(
                        Severity::Info,
                        "No analysis changes found",
                        mode.empty_result(),
                    )];
                }
                session.candidate = Some(loaded.candidate);
                session.candidate_mode = mode;
                session.candidate_track = index;
                session.candidate_first_lyric = preview;
                vec![AssistNotice::new(
                    Severity::Success,
                    "Analysis ready for review",
                    format!("Validated suggestions for {name} are staged in the Assist panel."),
                )]
            }
        }
    }

    /// `request_assist_job_cancel` (`plug.c:4088-4091`).
    pub fn request_cancel(&mut self, workspace: &mut Workspace) -> Vec<AssistNotice> {
        let Some(job) = self.job.as_mut() else {
            return Vec::new();
        };
        workspace.assist.job_state = if job.request_stop(StopReason::Cancelled) {
            AssistJobState::Cancelling
        } else {
            // A stop that could not be delivered moves the job to FAILING and
            // **keeps ownership** rather than forgetting the child
            // (`plug.c:4113`, and the comment at `:4188-4189`).
            AssistJobState::Failing
        };
        Vec::new()
    }

    /// The shutdown path: stop the tree and reap it before the process exits
    /// (`cancel_assist_job_blocking`, `plug.c:4129-4195`).
    ///
    /// Returns `false` when the child still could not be reaped, in which case
    /// this controller **must not be dropped and forgotten**. `AssistJob`'s own
    /// `Drop` is the net underneath.
    pub fn shutdown(&mut self) -> bool {
        match self.job.as_mut() {
            None => true,
            Some(job) => job.cancel_blocking().unwrap_or(false),
        }
    }

    /// `apply_assist_candidate` (`plug.c:3625-3667`): the second press of Apply,
    /// and the only path in this module that changes a project.
    pub fn apply(&mut self, workspace: &mut Workspace, now: f64) -> Vec<AssistNotice> {
        let Some(candidate) = workspace.assist.candidate.clone() else {
            return Vec::new();
        };
        let missing = || {
            vec![AssistNotice::new(
                Severity::Error,
                "Suggestions were not applied",
                "The track targeted by this staged result is no longer available.",
            )]
        };
        let Some(index) = workspace.assist.candidate_track else {
            return missing();
        };
        let bridge_path = PathBuf::from(workspace.assist.bridge_path.clone());
        let Some(track) = workspace.get(index) else {
            return missing();
        };
        // Provenance is verified *before* anything is replaced: a bridge whose
        // artifact cannot be hashed is not evidence, and applied lanes with no
        // provenance are worse than no lanes (`plug.c:3635-3646`).
        let Some(staged) = stage_analysis_lanes(track, &candidate, &bridge_path, false) else {
            return vec![AssistNotice::new(
                Severity::Error,
                "Suggestions were not applied",
                "The evidence artifact or its bounded provenance could not be verified.",
            )];
        };

        let Some(track) = workspace.get_mut(index) else {
            return missing();
        };
        if let Err(detail) = apply_candidate_to_track(&candidate, track, now) {
            return vec![AssistNotice::new(
                Severity::Error,
                "Suggestions were not applied",
                detail,
            )];
        }
        track.audio_sha256 = staged.audio_sha256;
        track.analysis_lanes = staged.lanes;
        let name = track.display_name().to_string();

        workspace.assist.clear_candidate();
        vec![AssistNotice::new(
            Severity::Success,
            "Suggestions applied",
            format!("The selected validated lanes are now in {name}."),
        )]
    }

    /// `discard_assist_candidate` (`plug.c:3669-3686`). Dropping it is the whole
    /// operation, because nothing in the editor was ever touched.
    pub fn discard(&mut self, workspace: &mut Workspace) -> Vec<AssistNotice> {
        if workspace.assist.candidate.is_none() {
            return Vec::new();
        }
        let name = workspace
            .assist
            .candidate_track
            .and_then(|index| workspace.get(index))
            .map_or("the missing target track", Track::display_name)
            .to_string();
        workspace.assist.clear_candidate();
        vec![AssistNotice::new(
            Severity::Info,
            "Suggestions discarded",
            format!("The staged result for {name} was discarded. Editor content is unchanged."),
        )]
    }

    /// `plug_load_analysis_bridge` (`plug.c:3689-3722`), which is
    /// `--analysis-bridge FILE`.
    ///
    /// **This one applies immediately rather than staging**, which is the
    /// oracle's behaviour and is deliberate: the flag is a batch entry point with
    /// no interactive review step, so a staged result would sit unapplied and the
    /// exit status would be a lie. Every lane is authorized because the document
    /// itself decides which lanes it carries.
    ///
    /// `Err` means nothing was applied, which the CLI turns into exit 1.
    pub fn import_bridge(
        &mut self,
        path: &Path,
        workspace: &mut Workspace,
        now: f64,
    ) -> Result<Vec<AssistNotice>, String> {
        let index = workspace
            .current_index()
            .ok_or_else(|| "there is no track to attach analysis to".to_string())?;
        let track = &workspace.tracks()[index];
        let loaded = load_candidate(path, track, Lanes::ALL)?;
        if !assist_ui_state::result_has_changes(
            lanes_bits(Lanes::ALL),
            lanes_bits(loaded.candidate.available()),
        ) {
            return Err(
                "the bridge contains no editor changes; lyrics, scenes and semantic \
                        events were left unchanged"
                    .to_string(),
            );
        }
        let staged =
            stage_analysis_lanes(track, &loaded.candidate, path, true).ok_or_else(|| {
                "the bridge provenance path or identity is not project-safe".to_string()
            })?;

        let track = workspace
            .get_mut(index)
            .ok_or_else(|| "there is no track to attach analysis to".to_string())?;
        apply_candidate_to_track(&loaded.candidate, track, now)?;
        track.audio_sha256 = staged.audio_sha256;
        track.analysis_lanes = staged.lanes;
        let name = track.display_name().to_string();
        Ok(vec![AssistNotice::new(
            Severity::Success,
            "Analysis bridge applied",
            format!("The bridge's validated lanes are now in {name}."),
        )])
    }

    /// `choose_lyric_reference`'s validation half (`plug.c:2140-2163`), once the
    /// picker has answered.
    pub fn set_lyric_sheet(&mut self, workspace: &mut Workspace, path: &Path) -> Vec<AssistNotice> {
        let length = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        if length == 0 || length > LYRIC_REFERENCE_BYTE_LIMIT {
            return vec![AssistNotice::new(
                Severity::Error,
                "That lyrics file was not used",
                format!(
                    "The file is empty or larger than the one megabyte a lyric sheet is \
                     allowed to be: {}",
                    path.display()
                ),
            )];
        }
        let Some(track) = workspace.current_mut() else {
            return Vec::new();
        };
        track.lyrics_reference_path = Some(path.to_path_buf());
        vec![AssistNotice::new(
            Severity::Info,
            "Lyrics reference selected",
            format!(
                "The next timed-lyrics run will synchronize these lines against Whisper \
                 instead of transcribing: {}",
                path.display()
            ),
        )]
    }
}

impl Drop for AssistController {
    /// `shutdown` is the reporting path; this is the net. `AssistJob`'s own
    /// `Drop` is louder and does the actual reaping.
    fn drop(&mut self) {
        if self.job.is_some() {
            let _ = self.shutdown();
        }
    }
}

/// The C's lane bitmask, rebuilt for [`assist_ui_state::result_has_changes`].
///
/// `Lanes` is three named booleans here precisely so an unknown bit is
/// unrepresentable, but the "did this run produce anything I authorized?" test
/// is a mask intersection and reads better as one.
fn lanes_bits(lanes: Lanes) -> u32 {
    u32::from(lanes.lyrics) | (u32::from(lanes.sections) << 1) | (u32::from(lanes.semantics) << 2)
}

/// `djb2(DJB2_INIT, file_path)` then over the eight bytes of the duration
/// (`plug.c:3262-3264`).
///
/// Reproduced exactly rather than replaced with a Rust hasher, because the
/// directory name is what lets the Python helper reuse the measured and model
/// caches a run of the frozen C left behind for the same track.
#[must_use]
fn workspace_hash(audio_path: &Path, duration_seconds: f64) -> u64 {
    let mut hash = DJB2_INIT;
    for byte in audio_path.as_os_str().as_encoded_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(*byte));
    }
    for byte in duration_seconds.to_ne_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(byte));
    }
    hash
}

fn job_mode(mode: AssistMode) -> JobMode {
    match mode {
        AssistMode::Lyrics => JobMode::Lyrics,
        AssistMode::Sections => JobMode::Sections,
        AssistMode::Mimo => JobMode::Mimo,
        AssistMode::All => JobMode::All,
    }
}

/// A staged candidate and, when it had to be computed, the audio digest that
/// verified it.
#[derive(Debug)]
struct LoadedCandidate {
    candidate: AnalysisCandidate,
    audio_sha256: Option<String>,
}

/// `load_analysis_candidate_for_track` (`plug.c:3408-3500`).
///
/// Every refusal is a sentence, because each one becomes the panel's failure
/// detail and "it was rejected" is not something a user can act on.
fn load_candidate(path: &Path, track: &Track, lanes: Lanes) -> Result<LoadedCandidate, String> {
    let length = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if length == 0 || length > analysis_bridge::MAX_INPUT_BYTES as u64 {
        return Err("The bridge is empty or exceeds the 4 MiB input limit.".to_string());
    }
    let input =
        std::fs::read(path).map_err(|_| "The validated bridge file is unavailable.".to_string())?;

    let mut computed = None;
    let expected = if track.audio_sha256.is_empty() {
        let digest = sha256_file_hex(&track.file_path)
            .map_err(|_| "The source audio could not be hashed.".to_string())?;
        computed = Some(digest.clone());
        digest
    } else {
        track.audio_sha256.clone()
    };

    // The digest guard is the whole point of the format: it is what stops cached
    // analysis attaching to the wrong audio. The duration is deliberately *not*
    // passed, because the container and the decoder disagree by an encoder
    // padding tail; that difference is bounded separately below.
    let bridge = analysis_bridge::parse(&input, Some(&expected), None)
        .map_err(|error| capitalize_sentence(&error.to_string()))?;

    let bridge_duration = bridge.duration_ms as f64 / 1000.0;
    if (bridge_duration - track.lyrics.duration_seconds()).abs() > 0.25 {
        return Err("The result duration does not match this track.".to_string());
    }

    let candidate = AnalysisCandidate::prepare(&bridge, lanes, bridge_duration, SCENE_COUNT as u32)
        .map_err(|error| capitalize_sentence(&error.to_string()))?;
    Ok(LoadedCandidate {
        candidate,
        audio_sha256: computed,
    })
}

/// `assist-manifest.json`, in the job's output directory
/// (`external_analysis.py:1501`).
pub const ASSIST_MANIFEST_NAME: &str = "assist-manifest.json";

/// The lyric documents to fall back on when the manifest names none
/// (`external_analysis.py:1494-1497`), best first.
const LYRIC_DOCUMENT_NAMES: [&str; 2] = ["lyrics.aligned.json", "lyrics.sync.json"];

/// Reads the LT1 review artifacts a finished job left in its output directory.
///
/// This is the whole path by which `unresolved[]` and `review_flags[]` reach the
/// interface, and it is deliberately **not** the bridge. The bridge grammar is
/// cues; an unresolved line is by definition not one, and widening the bridge to
/// carry non-cues would put the "cues plus unresolved account for every authored
/// line" invariant on both sides of the boundary instead of in the helper where
/// it is enforced.
///
/// `None` means this job wrote no LT1-aware manifest, which is what makes a
/// pre-LT1 job folder render exactly as it did before this tranche. A run that
/// placed everything returns `Some` with counts of zero, and the panel says so.
fn read_lyrics_review(output_dir: &str) -> Option<LyricsReview> {
    if output_dir.is_empty() {
        return None;
    }
    let directory = Path::new(output_dir);
    let manifest = analysis_candidate::parse_review_manifest(&read_bounded(
        &directory.join(ASSIST_MANIFEST_NAME),
    )?)?;
    // Review LT1-R, R1. The cache folder is keyed by audio rather than by mode,
    // so a Sections run's job folder still holds the `lyrics.aligned.json` a
    // previous Full-assist run wrote. Refusing here — **before** any document is
    // opened — is what stops one run's panel from showing another run's flags.
    if !manifest.lyrics_lane {
        return None;
    }

    let mut review = LyricsReview::from_manifest(&manifest);
    // Names, resolved against the folder the supervisor chose. The manifest
    // records absolute paths, and honouring those would let a moved — or
    // hand-edited — job folder send this read somewhere else entirely.
    let names = manifest
        .documents
        .iter()
        .map(String::as_str)
        .chain(LYRIC_DOCUMENT_NAMES);
    for name in names {
        if let Some(bytes) = read_bounded(&directory.join(name)) {
            if review.read_document(&bytes) {
                break;
            }
        }
    }
    Some(review)
}

/// Reads a file only if it is non-empty and within the review input bound.
/// Metadata first, so an enormous artifact is refused rather than read.
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let length = std::fs::metadata(path).ok()?.len();
    if length == 0 || length > analysis_candidate::REVIEW_INPUT_MAX_BYTES as u64 {
        return None;
    }
    std::fs::read(path).ok()
}

/// What the panel actually put on screen this frame (review LT1-R, R5).
///
/// The report line used to describe the *parse*, which is the one thing that
/// cannot be clipped: it said `listed=4` while the 960x640 scissor was eating
/// the fourth row and the tail that would have admitted it. These two numbers
/// come from the drawing code, so a gate can assert against clipping instead of
/// against intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReviewDraw {
    /// Named rows drawn, excluding the tail.
    pub rows: usize,
    /// Whether the "N of M shown" row survived.
    pub tail: bool,
}

/// The Assist review's own report line (review LT1, extended by LT1-R).
///
/// `main.rs` owns the `assist:` line and is not this agent's file, so the review
/// prints its own — the same shape `ui/panels/export.rs` uses for
/// `export frame lanes:`. It is emitted once per distinct state so a capture
/// carries the counts and the named lines without a screenshot having to be read
/// by eye.
#[must_use]
pub(crate) fn describe_review(candidate: Option<&AnalysisCandidate>, drawn: ReviewDraw) -> String {
    let Some(review) = candidate.and_then(AnalysisCandidate::lyrics_review) else {
        return "absent (this run left no lyrics-lane LT1 review artifact)".to_string();
    };
    let named: Vec<String> = review
        .entries
        .iter()
        .map(LyricReviewEntry::describe)
        .collect();
    format!(
        "unresolved={} flagged={} listed={} rows_drawn={} tail={} omitted={} counts={} \
         manifest={}/{} policy={} | {}",
        review.unresolved,
        review.flagged,
        review.entries.len(),
        drawn.rows,
        if drawn.tail { "yes" } else { "no" },
        review.omitted,
        if review.document_read {
            "document"
        } else {
            "manifest"
        },
        review.manifest_unresolved,
        review.manifest_flagged,
        if review.policy.is_empty() {
            "none"
        } else {
            review.policy.as_str()
        },
        if named.is_empty() {
            review.summary()
        } else {
            named.join(" ; ")
        }
    )
}

thread_local! {
    /// The last `assist review:` line printed, so the panel can report what it
    /// drew without printing it once per frame.
    ///
    /// Report-only state, and the narrowest thing that can hold it: the number
    /// of rows that survive the scissor is a property of the drawn frame, and
    /// `Shell`'s own fields are not this file's to add to.
    static LAST_REVIEW_REPORT: std::cell::RefCell<String> = const {
        std::cell::RefCell::new(String::new())
    };
}

/// Prints the review report line when it says something new.
fn report_review(line: String) {
    LAST_REVIEW_REPORT.with(|last| {
        let mut last = last.borrow_mut();
        if *last != line {
            println!("assist review:   {line}");
            *last = line;
        }
    });
}

/// Attaches the review where a result is staged. Both callers — a finished job
/// and the probe — go through here so the two cannot drift.
///
/// The report line is **not** printed here any more (review LT1-R, R5): it now
/// carries how many rows reached the screen, which only the panel knows. The one
/// case with no rows to count is still reported at once, because a run whose
/// review is absent has nothing to wait for a frame for.
fn stage_lyrics_review(candidate: &mut AnalysisCandidate, output_dir: &str) {
    LAST_REVIEW_REPORT.with(|last| last.borrow_mut().clear());
    let attached =
        read_lyrics_review(output_dir).is_some_and(|review| candidate.attach_lyrics_review(review));
    if !attached {
        report_review(describe_review(None, ReviewDraw::default()));
    }
}

/// `thiserror` messages are lowercase and unpunctuated; the panel prints whole
/// sentences, as the C's `analysis_bridge_result_string` table does.
fn capitalize_sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            if !out.ends_with('.') {
                out.push('.');
            }
            out
        }
    }
}

/// The first staged lyric, for the review body's one-line preview
/// (`plug.c:2460-2467`). Bounded to the C's 180 characters.
fn first_lyric(candidate: &AnalysisCandidate) -> String {
    if !candidate.available().lyrics {
        return String::new();
    }
    candidate
        .lyrics()
        .cues()
        .first()
        .map(|cue| {
            let mut end = cue.text.len().min(180);
            while end > 0 && !cue.text.is_char_boundary(end) {
                end -= 1;
            }
            cue.text[..end].to_string()
        })
        .unwrap_or_default()
}

/// What `stage_candidate_analysis_lanes` produced.
struct StagedLanes {
    lanes: Vec<AnalysisLaneReference>,
    audio_sha256: String,
}

/// `stage_candidate_analysis_lanes` (`plug.c:3502-3563`).
///
/// Provenance, not dependencies: the referenced artifact is not needed to reopen
/// evaluated project data, which is why a lane entry records a path, a digest and
/// who produced it rather than embedding anything.
///
/// Existing entries of the same kind are **replaced in place**, so re-running one
/// workflow does not push a second lyric-timing lane past the eight-entry bound
/// the `.musi` format allows.
fn stage_analysis_lanes(
    track: &Track,
    candidate: &AnalysisCandidate,
    path: &Path,
    imported_bridge: bool,
) -> Option<StagedLanes> {
    let path_text = path.to_str()?;
    if path_text.is_empty()
        || path_text.len() >= musializer_core::project::model::capacity::PATH
        || track.analysis_lanes.len() > MAX_ANALYSIS_LANES
    {
        return None;
    }
    let audio_sha256 = if track.audio_sha256.is_empty() {
        sha256_file_hex(&track.file_path).ok()?
    } else {
        track.audio_sha256.clone()
    };
    let artifact_sha256 = sha256_file_hex(path).ok()?;

    let available = candidate.available();
    #[rustfmt::skip]
    let requests: [(bool, AnalysisLaneKind, &str); 3] = [
        (available.lyrics,    AnalysisLaneKind::LyricTiming,   if imported_bridge { "imported-bridge" } else { "whisper-codex" }),
        (available.sections,  AnalysisLaneKind::MeasuredSignal, if imported_bridge { "imported-bridge" } else { "measured-sections" }),
        (available.semantics, AnalysisLaneKind::SemanticScore,  if imported_bridge { "imported-bridge" } else { "xiaomi-mimo-v2.5" }),
    ];

    let mut lanes = track.analysis_lanes.clone();
    for (present, kind, model) in requests {
        if !present {
            continue;
        }
        let reference = AnalysisLaneReference {
            kind,
            path: path_text.to_string(),
            sha256: artifact_sha256.clone(),
            audio_sha256: audio_sha256.clone(),
            provenance: Provenance {
                adapter: "external-analysis".to_string(),
                adapter_version: "1".to_string(),
                schema_version: "analysis-bridge-v1".to_string(),
                model: model.to_string(),
                // The C zeroes `provider` and writes `prompt_version`
                // (`plug.c:3546-3560`). Reproduced: a provider string invented
                // here would land in a `.musi` the frozen C never writes.
                provider: String::new(),
                prompt_version: "v1".to_string(),
            },
        };
        match lanes.iter().position(|lane| lane.kind == kind) {
            Some(index) => lanes[index] = reference,
            None => {
                if lanes.len() >= MAX_ANALYSIS_LANES {
                    return None;
                }
                lanes.push(reference);
            }
        }
    }
    Some(StagedLanes {
        lanes,
        audio_sha256,
    })
}

/// `apply_candidate_to_track` (`plug.c:3565-3623`).
///
/// The two normalizations are the interesting part and both are the oracle's:
/// the bridge measures against the container's duration and the decoder may
/// disagree by a small MP3 padding tail, so the lyric lane is re-timed onto the
/// decoded duration and the section plan is re-validated through the public
/// normalizer rather than by mutating a previously validated candidate into a
/// negative last cue.
fn apply_candidate_to_track(
    candidate: &AnalysisCandidate,
    track: &mut Track,
    now: f64,
) -> Result<(), String> {
    let available = candidate.available();
    // `lyric_editor_has_unsaved_draft` is Agent I's, and there is no draft in the
    // model yet, so this reads `false`. When the lyrics editor lands, its dirty
    // flag is the third argument and this refusal starts firing.
    if assist_ui_state::candidate_conflicts_with_lyric_draft(available.lyrics, true, false) {
        return Err(
            "Apply or discard the active lyric draft before replacing the lyric lane.".to_string(),
        );
    }

    let mut candidate = candidate.clone();
    if available.sections && !candidate.sections().is_empty() {
        candidate
            .normalize_sections(track.duration_seconds, SCENE_COUNT as u32)
            .map_err(|_| {
                "Section timing could not be normalized to the decoded track duration.".to_string()
            })?;
    }

    candidate
        .apply(
            &mut track.lyrics,
            &mut track.scene_switches,
            &mut track.semantic_events,
        )
        .map_err(|error| capitalize_sentence(&error.to_string()))?;

    // The lyric lane is normalized *after* the replacement rather than before,
    // because `AnalysisCandidate` exposes its document by shared reference only.
    // Same result: `normalize_duration` copies a validated document onto the
    // authoritative decoded duration, and the candidate's document has just been
    // validated by `apply`.
    if available.lyrics {
        let source = track.lyrics.clone();
        track
            .lyrics
            .normalize_duration(&source, track.duration_seconds)
            .map_err(|_| {
                "Lyric timing could not be normalized to the decoded track duration.".to_string()
            })?;
    }

    if available.sections {
        capture_missing_scene_cue_settings(track);
        track.cue_settings_active = false;
        track.scene_selection_pending = false;
        track.scene_switches.reset();
    }
    track.mark_dirty(now);
    Ok(())
}

/// `capture_missing_scene_cue_settings` (`plug.c:1031-1041`).
///
/// An imported plan carries no per-cue settings, so every cue adopts the track's
/// current values. Without this, playing an imported plan would drive each cue's
/// scene from a zeroed snapshot.
///
/// The C mutates the cues in place; `SceneSwitchTimeline` publishes through
/// `replace`, so the vector is rebuilt. `replace` preserves `enabled`, which is
/// what keeps the user's auto-scene opt-in out of this.
fn capture_missing_scene_cue_settings(track: &mut Track) {
    if track.scene_switches.is_empty() {
        return;
    }
    let mut cues: Vec<SceneSwitchCue> = track.scene_switches.cues().to_vec();
    let mut changed = false;
    for cue in &mut cues {
        if cue.settings.captured {
            continue;
        }
        let Some(scene) = SceneId::from_index(cue.scene_index as usize) else {
            continue;
        };
        if let Some(snapshot) = track.scene_settings.capture(scene) {
            cue.settings = snapshot;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let duration = track.duration_seconds;
    // A refusal leaves the published plan exactly as it was, which is the right
    // answer: a half-captured plan is worse than an uncaptured one.
    let _ = track
        .scene_switches
        .replace(&cues, duration, SCENE_COUNT as u32);
}

/// `resolve_lyric_reference` (`plug.c:2113-2138`).
///
/// The chosen file wins, matching the helper's own priority. A chosen file that
/// has since been moved or deleted **falls back** rather than failing the run
/// later with a path nobody can see.
#[must_use]
pub fn resolve_lyric_reference(track: Option<&Track>) -> (AssistLyricReference, String) {
    let Some(track) = track else {
        return (AssistLyricReference::None, String::new());
    };
    if let Some(chosen) = track.lyrics_reference_path.as_ref() {
        if chosen.is_file() {
            return (AssistLyricReference::Chosen, chosen.display().to_string());
        }
    }
    if let Some(sibling) = track
        .file_path
        .to_str()
        .and_then(assist_ui_state::lyric_sibling_path)
    {
        if Path::new(&sibling).is_file() {
            return (AssistLyricReference::Sibling, sibling);
        }
    }
    (AssistLyricReference::None, String::new())
}

// ---------------------------------------------------------------------------
// The probe-only states (`--ui-probe assist=`), review 4.2.
//
// The panel's three consequential bodies — a validated result awaiting Apply, a
// run in progress, and a failure — had never been in a frame, because reaching
// any of them needs a helper process, a decoded track and several seconds of
// wall clock. So they are synthesized here instead: fixed content, no spawn, no
// file read, no clock. Two runs of the same probe produce the same pixels, which
// is the only reason a capture is evidence of anything.
//
// This is invention, not a port. The frozen C has no probe at all.
// ---------------------------------------------------------------------------

/// A validated bridge document, as the helper would have written it.
///
/// Written out as a literal rather than generated, because a probe fixture that
/// is computed can drift with the thing it is supposed to hold still. Two lyric
/// cues (one flagged uncertain, so the review body draws its singular "1 timing
/// cue needs review" branch), two contiguous sections and two contiguous
/// semantic cues, so all three summary lines are present — which is also the
/// worst case for the layout review 1.13 is about.
#[rustfmt::skip]
const PROBE_BRIDGE_DOCUMENT: &str = concat!(
    "MUSIALIZER_BRIDGE\t1\n",
    // Not a real digest: `analysis_bridge::parse` is called with no expected
    // hash here, because there is no audio file in this path to hash.
    "AUDIO\t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\t60000\n",
    "LYRIC\t1\t12000\t16000\t880\tnone\td2Ugd2VyZSBuZXZlciBtZWFudCB0byBzdGF5\n",
    "LYRIC\t2\t16000\t21000\t410\tuncertain\tYW5kIHRoZSBsaWdodHMgY2FtZSB1cCBhbnl3YXk=\n",
    "SECTION\t10\t0\t30000\tspectrum\t640\tWyJjaG9ydXMiLCJsaWZ0Il0=\n",
    "SECTION\t11\t30000\t60000\tloom\t420\tWyJvdXRybyJd\n",
    "SEMANTIC\t20\t0\t30000\t720\t300\t250\t900\tYnJpZ2h0IGFuZCByaXNpbmc=\n",
    "SEMANTIC\t21\t30000\t60000\t400\t200\t-150\t800\tY2FsbSBhZnRlciB0aGUgbGlmdA==\n",
);

/// The track index a probe candidate targets: one that cannot exist.
///
/// Deliberate. A staged result whose target is gone is the reachable half of
/// review 1.13 — it greys Apply out and gives the panel a reason to print — and
/// the other half (an unfinished lyric draft) cannot be reached at all until the
/// draft editor lands. Without a blocked Apply the fix for 1.13 is not in the
/// frame, so this is the state worth photographing.
const PROBE_MISSING_TRACK: usize = usize::MAX;

/// The elapsed clock a probed running job reports: 2:05, fixed.
const PROBE_ELAPSED_SECONDS: f64 = 125.0;

/// Points a probed job's artifacts at a real directory, for the one state a
/// literal cannot synthesize.
///
/// The LT1 review list is read from `assist-manifest.json` and a `lyric-sync-v1`
/// document in the job folder, and a fixture for it is a *pair of files*, not a
/// string this file could hold. `--ui-probe` cannot carry it either: its grammar
/// lives in `cli.rs`, which is not this agent's file. So the gate points this
/// variable at a folder it wrote, exactly as it already points
/// `MUSIALIZER_ASSIST_HELPER` at a helper that is not there.
///
/// Nothing outside a probe reads it: a real job's `output_dir` comes from
/// `AssistJob`, and this is only consulted by `probe_artifacts`.
pub const PROBE_ARTIFACT_DIR_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_DIR";

/// Artifact paths for a probed job. They do not exist by default, so all three
/// Copy buttons draw disabled — which is both truthful (this job wrote nothing)
/// and the state that makes their boxes photographable beside the blocking
/// reason.
fn probe_artifacts(session: &mut AssistSession) {
    let directory = std::env::var(PROBE_ARTIFACT_DIR_VARIABLE)
        .ok()
        .filter(|path| !path.is_empty() && Path::new(path).is_dir())
        .unwrap_or_else(|| "/nonexistent/musializer-assist-probe".to_string());
    session.bridge_path = format!("{directory}/result.bridge.tsv");
    session.log_path = format!("{directory}/analysis.log");
    session.output_dir = directory;
}

/// Which mode a probed candidate was run in (review LT1-R, R11).
///
/// `probe_candidate` hard-coded [`Lanes::ALL`], so the panel state a *lyrics*
/// run produces — one lane line, and the review block directly under it — could
/// not be photographed by anyone, the gate included. That is the arrangement the
/// LT1 review surface actually ships in, and it was the one arrangement nothing
/// could take a picture of.
///
/// An environment variable for [`PROBE_ARTIFACT_DIR_VARIABLE`]'s reason: the
/// `--ui-probe` grammar lives in `cli.rs`, which is not this agent's file. An
/// unset or unrecognized value is `all`, so every existing capture is unchanged.
pub const PROBE_LANES_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_LANES";

/// The mode named by [`PROBE_LANES_VARIABLE`], or `All`.
fn probe_mode() -> AssistMode {
    match std::env::var(PROBE_LANES_VARIABLE)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "lyrics" => AssistMode::Lyrics,
        "sections" => AssistMode::Sections,
        "mimo" => AssistMode::Mimo,
        _ => AssistMode::All,
    }
}

/// The staged result a `--ui-probe assist=candidate` run reviews.
///
/// The bridge document carries all three lanes whatever the mode is, exactly as
/// a real one does; `prepare` is what drops the lanes the run was not authorized
/// for, so what a lyrics-only probe photographs is the real authority path.
fn probe_candidate(mode: AssistMode) -> Result<AnalysisCandidate, String> {
    let bridge = analysis_bridge::parse(PROBE_BRIDGE_DOCUMENT.as_bytes(), None, None)
        .map_err(|error| capitalize_sentence(&error.to_string()))?;
    let duration_seconds = bridge.duration_ms as f64 / 1000.0;
    AnalysisCandidate::prepare(
        &bridge,
        assist_ui_state::mode_lanes(mode),
        duration_seconds,
        SCENE_COUNT as u32,
    )
    .map_err(|error| capitalize_sentence(&error.to_string()))
}

/// Puts the Assist session into one of its four probeable states (review 4.2).
///
/// `now` is the transport clock the panel will read this frame, not a wall
/// clock: the running body's elapsed counter is drawn from
/// `time_seconds - started_at`, and a capture with a parked transport must
/// report the same number every run.
///
/// Nothing here spawns a process, reads a file or touches a track. The failure
/// case returns a sentence for the command line rather than half-applying.
pub(crate) fn apply_probe_state(
    workspace: &mut Workspace,
    state: crate::cli::AssistProbe,
    now: f64,
) -> Result<(), String> {
    let current = workspace.current_index();
    match state {
        crate::cli::AssistProbe::Confirm => {
            // Unchanged from the one-word grammar this replaced.
            workspace.assist.set_confirmation_pending(true);
        }
        crate::cli::AssistProbe::Candidate => {
            let mode = probe_mode();
            let mut candidate = probe_candidate(mode)?;
            let session = &mut workspace.assist;
            session.select_mode(mode);
            session.set_confirmation_pending(false);
            session.set_apply_confirmation_pending(false);
            session.candidate_first_lyric = first_lyric(&candidate);
            session.candidate_mode = mode;
            session.candidate_track = Some(PROBE_MISSING_TRACK);
            session.job_state = AssistJobState::Succeeded;
            probe_artifacts(session);
            // Same seam a finished job uses, so the review a capture shows is
            // the review a real run would show.
            stage_lyrics_review(&mut candidate, &session.output_dir);
            session.candidate = Some(candidate);
        }
        crate::cli::AssistProbe::Running => {
            let session = &mut workspace.assist;
            session.select_mode(AssistMode::All);
            session.set_confirmation_pending(false);
            session.job_state = AssistJobState::Running;
            session.job_track = current;
            session.started_at = now - PROBE_ELAPSED_SECONDS;
        }
        crate::cli::AssistProbe::Failed => {
            let session = &mut workspace.assist;
            session.select_mode(AssistMode::All);
            session.set_confirmation_pending(false);
            session.job_state = AssistJobState::Failed;
            session.job_track = current;
            // The tail of a log, joined with " | " rather than kept as lines:
            // the panel's failure surface is the one-line status row, and a
            // string with newlines in it would draw straight down over the body
            // underneath. The 250-byte truncation in `status_line` is the C's.
            session.failure_detail =
                "helper exited with status 2 | external_analysis.py:412 in run_alignment \
                 | RuntimeError: alignment model unavailable offline"
                    .to_string();
            probe_artifacts(session);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The staged-result action row, as arithmetic.
//
// Pulled out of the drawing code for this file's usual reason, and for a
// specific one: the bug review 1.13 found is a *rectangle* bug. The blocking
// reason was drawn at `discard.right + gap`, and the Copy buttons were then
// drawn from the same x at the same y, so the sentence explaining why Apply was
// greyed out was painted over by three opaque 36 px boxes — precisely when the
// user needs it. A capture cannot catch that (the picture is self-coherent, it
// just says less than it should), and a property assertion cannot pin it, so
// what follows is compared rect against rect in a unit test.
// ---------------------------------------------------------------------------

/// Where the review body's buttons and its blocking reason go (review 1.13).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CandidateActionRow {
    pub apply: UiRect,
    pub discard: UiRect,
    /// The left edge of the three Copy buttons.
    pub artifacts_x: f32,
    /// Where the reason naming the block goes, if there is one.
    pub reason: ReasonSlot,
}

/// The reason's place in the row (review 1.13).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ReasonSlot {
    /// Apply is not blocked; there is no sentence to draw.
    None,
    /// Beside the buttons, in the gap the artifact strip left behind. Up to two
    /// 13 px lines, which is what a 36 px row holds.
    Beside(UiRect),
    /// The row above the buttons, at full panel width, for a panel too narrow to
    /// give the reason a readable column beside them. The caller drops the
    /// "First staged lyric" preview when that row is where it was going to draw:
    /// why Apply cannot be pressed outranks a preview of what it would do.
    Above(UiRect),
}

/// Narrower than this and a two-line reason beside the buttons is shredded into
/// three or four words a line, so it moves to its own row instead. Sized from
/// the two reasons that exist (57 and 60 characters): at 13 px they wrap to two
/// comfortable lines at this width.
const REASON_MIN_WIDTH: f32 = 220.0;

/// The reason row's offset from the body top, which is the "First staged lyric"
/// line's own offset. Deliberately not a new row: `assist_ui_state::ui_layout`
/// reserves exactly 118 px for this body and it is checked against the frozen C
/// by `tools/differential_assist_ui.sh`, so the fix has to fit in the space the
/// oracle's layout already allows.
const REASON_ROW_OFFSET: f32 = 54.0;

/// The 13 px text height the reason rows are sized in.
const REASON_FONT_SIZE: f32 = 13.0;

// ---------------------------------------------------------------------------
// The LT1 review list.
//
// `assist_ui_state::ui_layout` reserves 118 px for the review body and is pinned
// against the frozen C by `tools/differential_assist_ui.sh`, so the named list
// cannot be squeezed into it — that body is already full to the pixel with three
// lane lines, the preview row and the 36 px button row.
//
// So the list is *extra* height, added on this side of the boundary: `ui_layout`
// is unchanged and its harness stays green, and both places that size the panel
// (`assist_timeline_height` and `assist_panel`) add the same block. It is drawn
// **below** the button row rather than above it, which is the order that
// degrades correctly: on a window too short for the whole panel, the scissor
// eats the list and leaves Apply, Discard and the counts on screen.
// ---------------------------------------------------------------------------

/// The 12 px text the review rows are drawn in.
const REVIEW_FONT_SIZE: f32 = 12.0;

/// Row pitch for the review list.
const REVIEW_ROW_HEIGHT: f32 = 15.0;

/// Rows the block draws below its heading, **including** the "+N more" tail.
///
/// Four, because the panel's own ceiling is the binding constraint:
/// `assist_ui_state::timeline_height` caps the strip at `screen - toolbar - 150`,
/// which at the supported 1280x720 leaves 88 px over the 274 px body the
/// oracle's layout asks for. Four rows plus the heading is 82 and fits; five is
/// 97 and would be drawn into the scissor, photographing as a list that had been
/// cut without saying so. `the_review_block_fits_the_height_the_panel_can
/// _actually_get` pins that arithmetic rather than trusting this paragraph.
const REVIEW_MAX_ROWS: usize = 4;

/// Where the review block starts, measured from the review body's top: after the
/// 36 px button row at +72 and its 10 px of slack.
const REVIEW_BLOCK_OFFSET: f32 = 118.0;

/// How many named lines the block shows in `max_rows` rows, and how many lines
/// that leaves for the tail count.
///
/// The tail spends one of the rows, so a list that overflows shows one name
/// fewer. Losing a name to say "there are more" is the right trade: a list
/// silently cut would read as the whole answer.
///
/// Two things changed in review LT1-R:
///
/// - **The tail is owed whenever fewer rows are drawn than there are lines to
///   check** (R4), not only when the row cap is the reason. A document with four
///   flags and two nameable ones drew two rows and no tail, so the panel's two
///   confident rows *were* the honest answer as far as the reader could tell.
/// - **`max_rows` is a parameter** (R2), because the row cap is not the binding
///   constraint at 960x640 — the panel's own scissor is, and it cut the tail
///   first. The caller measures the room it has and the tail is fitted inside
///   it, so the row that admits the truncation cannot itself be truncated.
fn review_rows(review: &LyricsReview, max_rows: usize) -> (usize, usize) {
    let total = review.total_to_check();
    let named = review.entries.len();
    if named >= total && named <= max_rows {
        return (named, 0);
    }
    let shown = named.min(max_rows.saturating_sub(1));
    (shown, total.saturating_sub(shown))
}

/// The height the review block adds to the panel, or 0 when there is no review.
///
/// Zero for a pre-LT1 run, which is what keeps every existing capture's geometry
/// byte-for-byte where it was.
fn review_block_height(candidate: Option<&AnalysisCandidate>) -> f32 {
    let Some(candidate) = candidate.filter(|candidate| candidate.available().lyrics) else {
        return 0.0;
    };
    let Some(review) = candidate.lyrics_review() else {
        return 0.0;
    };
    let (shown, hidden) = review_rows(review, REVIEW_MAX_ROWS);
    // A clear run still gets a row: it says so in words, because a blank region
    // is indistinguishable from a broken one.
    let rows = shown.max(1) + usize::from(hidden > 0);
    18.0 + rows as f32 * REVIEW_ROW_HEIGHT + 4.0
}

/// The tint a row carries: an unplaced line is a warning, a disagreement is
/// ordinary ink. They are different jobs — one line has no cue at all, the other
/// has a cue that may be in the wrong place.
fn review_row_color(kind: LyricReviewKind) -> raylib::prelude::Color {
    if kind.is_unresolved() {
        color::ui_warning()
    } else {
        color::ui_ink()
    }
}

/// The width the three Copy buttons occupy, including the gaps between them.
fn artifact_strip_width(gap: f32) -> f32 {
    AssistArtifact::ALL
        .iter()
        .enumerate()
        .map(|(index, artifact)| artifact.width() + if index == 0 { 0.0 } else { gap })
        .sum()
}

/// Lays out Apply, Discard, the Copy strip and the blocking reason so that no
/// two of them share a pixel (review 1.13).
///
/// The Copy strip is right-aligned to the panel's inner edge **whether or not**
/// a reason is showing. That costs a gap of empty row in the unblocked case and
/// buys the property that matters: a reason appearing or disappearing — which it
/// does as the user edits a lyric draft — never moves a button under the
/// pointer. It is the same argument as `core::ui::transport_bar`'s: a layout
/// that is stable under a state change beats one that packs tighter.
fn candidate_action_row(
    boundary: UiRect,
    action_y: f32,
    padding: f32,
    gap: f32,
    blocked: bool,
) -> CandidateActionRow {
    let x = boundary.x + padding;
    let row_y = action_y + 72.0;
    let apply = UiRect::new(x, row_y, 144.0, metric::UI_BUTTON_HEIGHT);
    let discard = UiRect::new(apply.x + apply.width + gap, row_y, 100.0, apply.height);
    let after_discard = discard.x + discard.width + gap;
    let inner_right = boundary.x + boundary.width - padding;

    // `.max` rather than a refusal: a panel this narrow already drops the Copy
    // buttons in `assist_artifact_actions`, which will not draw a button that
    // leaves the boundary. What must not happen is a negative-width gap being
    // handed to the reason as if it were room.
    let artifacts_x = (inner_right - artifact_strip_width(gap)).max(after_discard);
    let beside_width = artifacts_x - gap - after_discard;

    let reason = if !blocked {
        ReasonSlot::None
    } else if beside_width >= REASON_MIN_WIDTH {
        ReasonSlot::Beside(UiRect::new(
            after_discard,
            row_y,
            beside_width,
            apply.height,
        ))
    } else {
        ReasonSlot::Above(UiRect::new(
            x,
            action_y + REASON_ROW_OFFSET,
            (boundary.width - padding * 2.0).max(0.0),
            18.0,
        ))
    };
    CandidateActionRow {
        apply,
        discard,
        artifacts_x,
        reason,
    }
}

/// Greedy word wrap, with the last line ellipsized when the text outruns
/// `max_lines`.
///
/// `measure` is a parameter rather than a `&UiFonts` so the wrap can be driven
/// in a headless test; the real caller passes the real face, so what is asserted
/// here is the algorithm, not the metrics.
fn wrap_to_width(
    text: &str,
    max_width: f32,
    max_lines: usize,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<String> {
    if max_lines == 0 || max_width <= 0.0 || text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
            continue;
        }
        let candidate = format!("{current} {word}");
        if measure(&candidate) <= max_width {
            current = candidate;
        } else if lines.len() + 1 < max_lines {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            // Out of lines: everything left joins the last one, which is then
            // cut. Cutting here rather than dropping the tail is what makes the
            // ellipsis honest about there being more.
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if let Some(last) = lines.last_mut() {
        if measure(last) > max_width {
            *last = ellipsize(last, max_width, measure);
        }
    }
    lines
}

/// Cuts `text` on a character boundary until it and a trailing `...` fit.
///
/// ASCII dots rather than U+2026: the icon face is the only face with Private
/// Use Area coverage, and a glyph the UI face happens not to carry draws as an
/// empty box — which would make a truncated sentence look like a broken one.
fn ellipsize(text: &str, max_width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    const ELLIPSIS: &str = "...";
    if measure(text) <= max_width {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end -= 1;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}{ELLIPSIS}", text[..end].trim_end());
        if measure(&candidate) <= max_width {
            return candidate;
        }
    }
    ELLIPSIS.to_string()
}

/// Draws the LT1 review list: which authored lines to look at, and where
/// (tranche LT1).
///
/// The whole reason this exists rather than a count: the localizer's failures
/// are *sparse and specific*. "2 unresolved" over a fifty-line sheet asks the
/// user to replay the whole song; "UNPLACED line 26 at 1:30.6" asks them to look
/// at one place. The counts are on the summary line above; this is the part that
/// turns them into an action.
/// Where a flagged line is actually repaired (review LT1-R, R9).
///
/// A hint, not a route: the Lyrics panel is where a cue is dragged or nudged,
/// and nothing here navigates to it. Deliberately on the heading's own row so it
/// costs no row from the list, and dropped entirely when the panel is too narrow
/// for both — the names are the point, this is the footnote.
const REVIEW_FIX_HINT: &str = "Retime cues in the Lyrics panel";

/// How many review rows fit between `first_row_y` and the panel's clip edge.
///
/// Whole rows only. A row that would be cut mid-glyph is not a row the reader
/// can act on, and the scissor does not care that the arithmetic above it
/// reserved the space (review LT1-R, R2).
fn review_row_capacity(first_row_y: f32, clip_bottom: f32) -> usize {
    // `<=` rather than a negated `>`, so a NaN edge is a refusal rather than a
    // row count derived from one.
    if clip_bottom <= first_row_y || !(clip_bottom - first_row_y).is_finite() {
        return 0;
    }
    let rows = ((clip_bottom - first_row_y) / REVIEW_ROW_HEIGHT).floor();
    if rows <= 0.0 {
        0
    } else {
        rows as usize
    }
}

fn draw_lyrics_review(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    review: &LyricsReview,
    x: f32,
    y: f32,
    width: f32,
    clip_bottom: f32,
) -> ReviewDraw {
    let measure = |line: &str| widgets::measure(font, line, REVIEW_FONT_SIZE);
    let mut row_y = y + 18.0;
    // The whole block, heading included, is dropped rather than half-drawn: a
    // heading over a list the scissor ate is the one arrangement that promises
    // names and delivers none.
    let capacity = review_row_capacity(row_y, clip_bottom);
    if capacity == 0 {
        return ReviewDraw::default();
    }
    let (shown, hidden) = review_rows(review, capacity.min(REVIEW_MAX_ROWS));

    let heading = if review.policy.is_empty() {
        "Lines to check".to_string()
    } else {
        format!("Lines to check ({})", review.policy)
    };
    widgets::draw_text(d, font, &heading, x, y, REASON_FONT_SIZE, color::accent());
    let hint_width = widgets::measure(font, REVIEW_FIX_HINT, REVIEW_FONT_SIZE);
    let heading_width = widgets::measure(font, &heading, REASON_FONT_SIZE);
    if width - heading_width - hint_width >= 24.0 {
        widgets::draw_text(
            d,
            font,
            REVIEW_FIX_HINT,
            x + width - hint_width,
            y + 1.0,
            REVIEW_FONT_SIZE,
            color::ui_muted(),
        );
    }

    // Nothing to check is a state, not an absence. Without this row the panel
    // would be blank exactly where the answer goes, and a blank region is
    // indistinguishable from a broken one.
    if review.entries.is_empty() {
        let sentence = if review.is_clear() {
            "All authored lines were placed and none were flagged."
        } else if review.document_read {
            // Read, and it named nobody. Different problem, different sentence
            // (review LT1-R, R3).
            "The job's lyric document named none of them, so there is nothing to \
             point at here."
        } else {
            // Counts without names: the manifest said something is wrong and
            // the document that names it could not be read. Say which.
            "The counts above came from the job manifest; its lyric document \
             could not be read for the line names."
        };
        widgets::draw_text(
            d,
            font,
            &ellipsize(sentence, width, &measure),
            x,
            row_y,
            REVIEW_FONT_SIZE,
            color::ui_muted(),
        );
        return ReviewDraw {
            rows: 0,
            tail: false,
        };
    }

    for entry in review.entries.iter().take(shown) {
        widgets::draw_text(
            d,
            font,
            &ellipsize(&entry.describe(), width, &measure),
            x,
            row_y,
            REVIEW_FONT_SIZE,
            review_row_color(entry.kind),
        );
        row_y += REVIEW_ROW_HEIGHT;
    }
    if hidden > 0 {
        // "N of M shown" rather than "+N more": it is the form that still reads
        // correctly when the panel is short enough that N is zero, which is the
        // case this row exists for.
        let total = review.total_to_check();
        let tail = if shown == 0 {
            format!(
                "None of the {total} lines to check fit here; the full list is in the job \
                 folder's lyrics document."
            )
        } else {
            format!(
                "{shown} of {total} shown; the full list is in the job folder's lyrics document."
            )
        };
        widgets::draw_text(
            d,
            font,
            &ellipsize(&tail, width, &measure),
            x,
            row_y,
            REVIEW_FONT_SIZE,
            color::ui_muted(),
        );
    }
    ReviewDraw {
        rows: shown,
        tail: hidden > 0,
    }
}

/// Draws the sentence naming why Apply is greyed out, in the slot the row gave
/// it (review 1.13).
fn draw_blocking_reason(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    text: &str,
    slot: ReasonSlot,
) {
    let measure = |line: &str| widgets::measure(font, line, REASON_FONT_SIZE);
    let (rect, lines) = match slot {
        ReasonSlot::None => return,
        // Two 13 px lines in a 36 px row: 2 + 13 + 3 + 13 + 5.
        ReasonSlot::Beside(rect) => (rect, wrap_to_width(text, rect.width, 2, &measure)),
        ReasonSlot::Above(rect) => (rect, vec![ellipsize(text, rect.width, &measure)]),
    };
    if lines.is_empty() {
        return;
    }
    let line_height = REASON_FONT_SIZE + 3.0;
    let top = rect.y + ((rect.height - lines.len() as f32 * line_height) * 0.5).max(0.0);
    for (index, line) in lines.iter().enumerate() {
        widgets::draw_text(
            d,
            font,
            line,
            rect.x,
            top + index as f32 * line_height,
            REASON_FONT_SIZE,
            color::accent(),
        );
    }
}

// ---------------------------------------------------------------------------
// The panel.
// ---------------------------------------------------------------------------

impl Shell {
    /// The timeline height an open Assist panel asks for (`plug.c:7669-7677`).
    ///
    /// Recomputed per frame from the panel's *current* body, as the C does, which
    /// is the layout rule this repository has already paid for: a panel that
    /// reserves height it never draws pushes the scene preview down for nothing.
    /// The difference is worth 96 px of preview and — at 720p — the whole tracks
    /// rail, because the sidebar has a floor below which it cannot host the
    /// tracks panel at all.
    ///
    /// The width is the workspace's, not the window's, so an open inspector at
    /// the 960 px minimum really does wrap the mode grid and make the panel
    /// taller.
    #[must_use]
    pub(crate) fn assist_timeline_height(
        &self,
        window: (f32, f32),
        session: &AssistSession,
    ) -> f32 {
        let layout = assist_ui_state::ui_layout(
            WorkspaceFrame::assist_panel_width(window.0, self.inspector_open),
            session.panel_content(),
            session.mode().uses_lyric_reference(),
        );
        // The LT1 review list is extra height on this side of the boundary; see
        // the note above `REVIEW_FONT_SIZE`. Zero without a review, so a panel
        // that had no such artifact asks for exactly what it asked for before.
        let required = layout.required_height + review_block_height(session.candidate.as_ref());
        assist_ui_state::timeline_height(window.1, metric::HUD_BUTTON_SIZE, required)
    }

    /// The Assist panel, in the timeline strip's place (`draw_assist_panel`,
    /// `plug.c:2166-2540`).
    ///
    /// Reads the session through `input.workspace`; every click becomes an
    /// [`AssistRequest`] the frame loop drains once the drawing pair has closed.
    /// Nothing here spawns, signals, reads a file or edits a track.
    pub(crate) fn assist_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        let top = strip.y + strip.height + 28.0;
        let width = content.width - padding * 2.0;
        let available = (content.y + content.height - top - padding).max(0.0);
        let session = &input.workspace.assist;
        let panel_content = session.panel_content();
        let layout =
            assist_ui_state::ui_layout(width, panel_content, session.mode().uses_lyric_reference());
        // The box is the height the body needs, not the height the band happens
        // to have. They agree when `Shell::timeline_height` asked for this panel;
        // clamping is what keeps the panel from drawing a half-empty box when it
        // did not — which is exactly what this build looks like until the
        // `timeline_height` seam in Agent J's note is wired.
        let required = layout.required_height + review_block_height(session.candidate.as_ref());
        let boundary = UiRect::new(content.x + padding, top, width, available.min(required));
        if boundary.is_empty() {
            return;
        }
        let font = input.fonts.ui();

        widgets::fill(d, boundary, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, color::ui_warning());

        // The oracle refuses to draw below 80 px (`plug.c:3132`): a panel that
        // cannot host its own mode row would register buttons nobody can see and
        // claim the presses aimed at whatever is underneath. Saying so beats a
        // blank box — and this state is reachable today, because
        // `Shell::timeline_height` still hands Assist the plain 180 px strip.
        if boundary.height <= 80.0 {
            widgets::draw_text(
                d,
                font,
                "ASSISTED ANALYSIS",
                boundary.x + padding,
                boundary.y + 6.0,
                metric::UI_FONT_CAPTION,
                color::accent(),
            );
            widgets::draw_text(
                d,
                font,
                "needs a taller timeline than this build asks for; see Agent J's note",
                boundary.x + padding,
                boundary.y + 24.0,
                metric::UI_FONT_CAPTION,
                color::ui_warning(),
            );
            return;
        }

        // One scissor around the whole body, as the C does (`plug.c:2172-2175`),
        // so a long helper message cannot print outside the panel it belongs to.
        let mut clip = widgets::begin_scissor(
            d,
            UiRect::new(
                boundary.x + 1.0,
                boundary.y + 1.0,
                (boundary.width - 2.0).max(0.0),
                (boundary.height - 2.0).max(0.0),
            ),
            input.ui_scale,
        );

        widgets::draw_text(
            &mut clip,
            font,
            "ASSISTED ANALYSIS",
            boundary.x + padding,
            boundary.y + padding,
            metric::UI_FONT_HEADER,
            color::accent(),
        );
        widgets::draw_text(
            &mut clip,
            font,
            "Validated results stay staged until you apply them.",
            boundary.x + padding,
            boundary.y + 36.0,
            metric::UI_FONT_VALUE,
            color::ui_muted(),
        );

        // The saved/Assist-produced plan is useful only if it can be enabled
        // again after a manual scene choice disabled it. Match the oracle's
        // compact header control and keep it absent when there is no plan to
        // operate on (`plug.c:2207-2226`).
        if boundary.width >= 560.0 {
            if let Some((enabled, cue_count)) = input
                .workspace
                .current()
                .filter(|track| !track.scene_switches.is_empty())
                .map(|track| (track.scene_switches.enabled, track.scene_switches.len()))
            {
                let label = format!(
                    "Current auto scenes: {} ({cue_count})",
                    if enabled { "On" } else { "Off" }
                );
                let toggle = UiRect::new(
                    boundary.x + boundary.width - padding - 190.0,
                    boundary.y + 8.0,
                    190.0,
                    metric::UI_BUTTON_HEIGHT,
                );
                let font_size =
                    widgets::row_font_size(font, &[label.as_str()], &[toggle.width], toggle.height);
                if self
                    .widgets
                    .text_button(
                        &mut clip,
                        font,
                        widgets::widget_id(ASSIST_WIDGETS, 90),
                        toggle,
                        &label,
                        enabled,
                        ButtonStyle::Neutral,
                        Some(font_size),
                    )
                    .clicked
                {
                    commands.push(ShellCommand::SetAutoScenes(!enabled));
                }
            }
        }

        let start_block = input.workspace.assist.start_block();
        self.assist_modes(&mut clip, input, boundary, &layout, start_block);
        self.assist_status(&mut clip, input, boundary, &layout);
        self.assist_body(
            &mut clip,
            input,
            boundary,
            &layout,
            panel_content,
            padding,
            gap,
        );
    }

    /// The four workflow buttons, with their data-boundary badges
    /// (`plug.c:2229-2271`).
    ///
    /// The badge is on the button rather than only in the confirmation step so
    /// "this one leaves the machine" is visible **before** the click.
    fn assist_modes(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        layout: &AssistUiLayout,
        start_block: AssistStartBlock,
    ) {
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        let font = input.fonts.ui();
        let session = &input.workspace.assist;
        let active = session.is_active();

        let columns = layout.mode_columns.max(1);
        let button_width =
            (boundary.width - padding * 2.0 - gap * (columns - 1) as f32) / columns as f32;
        if button_width <= 0.0 {
            return;
        }
        let labels: Vec<&str> = AssistMode::ALL.iter().map(|m| m.display_name()).collect();
        let widths = vec![button_width; labels.len()];
        let font_size = widgets::row_font_size(font, &labels, &widths, metric::UI_BUTTON_HEIGHT);

        for (index, mode) in AssistMode::ALL.into_iter().enumerate() {
            let row = index / columns;
            let column = index % columns;
            let button = UiRect::new(
                boundary.x + padding + column as f32 * (button_width + gap),
                boundary.y + layout.mode_top + row as f32 * layout.mode_row_height,
                button_width,
                metric::UI_BUTTON_HEIGHT,
            );
            // The panel owns these pixels; a button drawn past its edge still
            // claims the press aimed at whatever is underneath it.
            if !boundary.contains(button) {
                continue;
            }
            let selected = (session.candidate.is_some() && session.candidate_mode == mode)
                || (active && session.mode() == mode);
            let armed = session.confirmation_pending() && !active && session.mode() == mode;

            if start_block == AssistStartBlock::Allowed {
                let id = widgets::widget_id(ASSIST_WIDGETS, index as u32);
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        id,
                        button,
                        labels[index],
                        selected,
                        ButtonStyle::Neutral,
                        Some(font_size),
                    )
                    .clicked
                {
                    // Selecting a workflow proposes it; it does not start it.
                    session.select_mode(mode);
                }
            } else {
                // The C hangs a tooltip carrying `start_block.reason()` off the
                // disabled button. There is no tooltip widget in this rewrite, so
                // the reason goes on the status line instead — which is where the
                // C also puts it when the block is a missing helper.
                self.widgets
                    .disabled_button(d, font, button, labels[index], Some(font_size));
            }
            if armed {
                d.draw_rectangle_lines_ex(widgets::rectangle(button), 2.0, color::ui_warning());
            }
            widgets::draw_text(
                d,
                font,
                mode.badge(),
                button.x + 3.0,
                button.y + 39.0,
                11.0,
                color::ui_muted(),
            );
        }
    }

    /// One line naming exactly what the panel is doing (`plug.c:2273-2338`).
    fn assist_status(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        layout: &AssistUiLayout,
    ) {
        let workspace = input.workspace;
        let session = &workspace.assist;
        let name_of = |index: Option<usize>| {
            index
                .and_then(|index| workspace.get(index))
                .map_or("missing track", Track::display_name)
        };
        let log_file_name = Path::new(&session.log_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        // The clock is the transport's, not a wall clock: a headless capture
        // parks the transport, and a status line that ticked would make two runs
        // of the same probe produce different pixels.
        let elapsed = if session.job_state == AssistJobState::Running {
            (input.time_seconds - session.started_at).max(0.0)
        } else {
            0.0
        };
        let (text, tone) = assist_ui_state::status_line(&AssistStatusInputs {
            session_mode: session.mode(),
            job_state: session.job_state,
            confirmation_pending: session.confirmation_pending(),
            helper_available: session.helper_available,
            elapsed_seconds: elapsed,
            candidate_mode: session.candidate.as_ref().map(|_| session.candidate_mode),
            candidate_track_name: name_of(session.candidate_track),
            job_track_name: name_of(session.job_track),
            current_track_name: name_of(workspace.current_index()),
            failure_detail: &session.failure_detail,
            log_file_name,
        });
        widgets::draw_text(
            d,
            input.fonts.ui(),
            &text,
            boundary.x + metric::UI_PANEL_PADDING,
            boundary.y + layout.status_y,
            16.0,
            match tone {
                AssistStatusTone::Ink => color::ui_ink(),
                AssistStatusTone::Accent => color::accent(),
                AssistStatusTone::Warning => color::ui_warning(),
                AssistStatusTone::Danger => color::ui_danger(),
                AssistStatusTone::Success => color::ui_success(),
            },
        );
    }

    /// Whichever of the six bodies `panel_content` chose (`plug.c:2340-2539`).
    #[allow(
        clippy::too_many_arguments,
        reason = "the C's own shape: the boundary, its layout, the chosen body and the two metrics"
    )]
    fn assist_body(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        layout: &AssistUiLayout,
        content: AssistPanelContent,
        padding: f32,
        gap: f32,
    ) {
        let font = input.fonts.ui();
        let session = &input.workspace.assist;
        let action_y = boundary.y + layout.content_y;
        let x = boundary.x + padding;

        match content {
            AssistPanelContent::Confirmation => {
                let mode = session.mode();
                widgets::draw_text(d, font, mode.workflow(), x, action_y, 14.0, color::ui_ink());
                widgets::draw_text(
                    d,
                    font,
                    mode.data_boundary(),
                    x,
                    action_y + 21.0,
                    14.0,
                    color::ui_muted(),
                );
                let buttons = assist_ui_state::confirmation_buttons(
                    boundary,
                    layout,
                    padding,
                    gap,
                    metric::UI_BUTTON_HEIGHT,
                );
                if layout.reference_y > 0.0 {
                    self.assist_lyric_reference(d, input, boundary, layout, &buttons);
                }
                if session.helper_available {
                    let id = widgets::widget_id(ASSIST_WIDGETS, 10);
                    if self
                        .widgets
                        .text_button(
                            d,
                            font,
                            id,
                            buttons.start,
                            "Start analysis",
                            false,
                            ButtonStyle::Neutral,
                            None,
                        )
                        .clicked
                    {
                        session.request(AssistRequest::Start);
                    }
                } else {
                    self.widgets
                        .disabled_button(d, font, buttons.start, "Start analysis", None);
                }
                let id = widgets::widget_id(ASSIST_WIDGETS, 11);
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        id,
                        buttons.cancel,
                        "Cancel",
                        false,
                        ButtonStyle::Neutral,
                        None,
                    )
                    .clicked
                {
                    session.request(AssistRequest::DismissConfirmation);
                }
            }
            AssistPanelContent::Running => {
                widgets::draw_text(
                    d,
                    font,
                    session.mode().workflow(),
                    x,
                    action_y,
                    14.0,
                    color::ui_ink(),
                );
                widgets::draw_text(
                    d,
                    font,
                    "No percentage is reported. The complete job stops at 40:00; playback remains available.",
                    x,
                    action_y + 21.0,
                    14.0,
                    color::ui_muted(),
                );
                let cancel = UiRect::new(x, action_y + 48.0, 130.0, metric::UI_BUTTON_HEIGHT);
                let id = widgets::widget_id(ASSIST_WIDGETS, 12);
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        id,
                        cancel,
                        "Cancel job",
                        false,
                        ButtonStyle::Danger,
                        None,
                    )
                    .clicked
                {
                    session.request(AssistRequest::CancelJob);
                }
            }
            AssistPanelContent::Cancelling => {
                widgets::draw_text(
                    d,
                    font,
                    if session.job_state == AssistJobState::TimingOut {
                        "The 40:00 job deadline was reached. Verifying that the complete process tree stopped."
                    } else {
                        "Waiting for the helper and its child processes to stop. Editor content is unchanged."
                    },
                    x,
                    action_y,
                    14.0,
                    color::ui_muted(),
                );
            }
            AssistPanelContent::Candidate => {
                self.assist_candidate_body(d, input, boundary, action_y, padding, gap);
            }
            AssistPanelContent::Empty => {
                let (line, tint) = if session.job_state == AssistJobState::Succeeded {
                    (session.mode().empty_result(), color::ui_ink())
                } else {
                    (
                        "Job artifacts remain available for diagnosis and support.",
                        color::ui_muted(),
                    )
                };
                widgets::draw_text(d, font, line, x, action_y, 14.0, tint);
                self.assist_artifact_actions(d, input, boundary, x, action_y + 34.0, gap);
            }
            AssistPanelContent::Ready => {}
        }
    }

    /// The lyric-sheet row (`plug.c:2349-2384`).
    ///
    /// This is the panel's reason to exist: it names the file the run will
    /// actually use, before it runs. The C's own comment is worth keeping in
    /// mind — the aligner is the best lyric path in the product, and it used to be
    /// reachable only by knowing that a sibling `<stem>.lyrics.txt` is what the
    /// helper looks for.
    fn assist_lyric_reference(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        layout: &AssistUiLayout,
        buttons: &AssistConfirmationButtons,
    ) {
        let font = input.fonts.ui();
        let session = &input.workspace.assist;
        let track = input.workspace.current();
        let (reference, path) = resolve_lyric_reference(track);
        let reference_y = boundary.y + layout.reference_y;
        let x = boundary.x + metric::UI_PANEL_PADDING;

        let name = if reference == AssistLyricReference::None {
            "none chosen".to_string()
        } else {
            Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&path)
                .to_string()
        };
        widgets::draw_text(
            d,
            font,
            &format!("Lyrics: {name}"),
            x,
            reference_y,
            14.0,
            if reference == AssistLyricReference::None {
                color::ui_muted()
            } else {
                color::ui_ink()
            },
        );
        widgets::draw_text(
            d,
            font,
            reference.summary(),
            x,
            reference_y + 18.0,
            13.0,
            color::ui_muted(),
        );
        if !buttons.reference_room {
            return;
        }
        let id = widgets::widget_id(ASSIST_WIDGETS, 20);
        if self
            .widgets
            .text_button(
                d,
                font,
                id,
                buttons.choose,
                if reference == AssistLyricReference::None {
                    "Choose lyrics..."
                } else {
                    "Replace lyrics..."
                },
                false,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            session.request(AssistRequest::ChooseLyricSheet);
        }
        // Clear exists only for a sheet the user chose. A sibling the helper found
        // is not something this panel can un-find.
        if track.is_some_and(|track| track.lyrics_reference_path.is_some()) {
            let id = widgets::widget_id(ASSIST_WIDGETS, 21);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    buttons.clear,
                    "Clear",
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                session.request(AssistRequest::ClearLyricSheet);
            }
        }
    }

    /// The staged-result review (`plug.c:2431-2525`).
    ///
    /// Every line reads `before -> after`, because "12 lyrics" does not tell
    /// anyone whether applying replaces two cues or two hundred.
    fn assist_candidate_body(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        action_y: f32,
        padding: f32,
        gap: f32,
    ) {
        let font = input.fonts.ui();
        let workspace = input.workspace;
        let session = &workspace.assist;
        let Some(candidate) = session.candidate.as_ref() else {
            return;
        };
        let target = session
            .candidate_track
            .and_then(|index| workspace.get(index));
        let x = boundary.x + padding;
        let mut line_y = action_y;
        let available = candidate.available();

        // Both blocking conditions are decided before anything is drawn, because
        // the reason may need the row the "First staged lyric" preview would
        // otherwise take (review 1.13).
        //
        // A staged lyric replacement must never clear an authored draft
        // implicitly. `draft_is_dirty` is `false` until Agent I's editor lands,
        // which is why this reads as unblocked today.
        let draft_conflict = assist_ui_state::candidate_conflicts_with_lyric_draft(
            available.lyrics,
            session.candidate_track == workspace.current_index(),
            false,
        );
        let blocked_reason = if draft_conflict {
            Some("Finish the active lyric draft before applying this result.")
        } else if target.is_none() {
            Some("The target track is no longer available. Discard this result.")
        } else {
            None
        };
        let row = candidate_action_row(boundary, action_y, padding, gap, blocked_reason.is_some());

        if available.lyrics {
            let before = target.map_or(0, |track| track.lyrics.len());
            let after = candidate.lyrics().len();
            // The counts belong on the summary line, and an LT1 run has better
            // ones than the per-cue `uncertain` flag: that flag cannot count a
            // line which never became a cue at all, and an unresolved line is
            // exactly the case a user must be told about.
            let line = match candidate.lyrics_review() {
                Some(review) => format!("Lyrics: {before} -> {after}  |  {}", review.summary()),
                None => {
                    let uncertain = candidate.uncertain_lyric_count();
                    if uncertain == 0 {
                        format!("Lyrics: {before} -> {after}  |  No timing cues flagged for review")
                    } else {
                        format!(
                            "Lyrics: {before} -> {after}  |  {uncertain} timing cue{} {} review",
                            if uncertain == 1 { "" } else { "s" },
                            if uncertain == 1 { "needs" } else { "need" }
                        )
                    }
                }
            };
            widgets::draw_text(d, font, &line, x, line_y, 14.0, color::ui_ink());
            line_y += 18.0;
        }
        if available.sections {
            let line = format!(
                "Scene changes: {} -> {}",
                target.map_or(0, |track| track.scene_switches.len()),
                candidate.sections().len()
            );
            widgets::draw_text(d, font, &line, x, line_y, 14.0, color::ui_ink());
            line_y += 18.0;
        }
        if available.semantics {
            let line = format!(
                "Feeling cues: {} -> {}",
                target.map_or(0, |track| track.semantic_events.len()),
                candidate.semantic_events().len()
            );
            widgets::draw_text(d, font, &line, x, line_y, 14.0, color::ui_ink());
            line_y += 18.0;
        }
        // Suppressed only when the reason has taken this exact row, which happens
        // when all three lanes are staged *and* the panel is too narrow to put
        // the reason beside the buttons.
        let preview_row_taken = matches!(row.reason, ReasonSlot::Above(rect) if line_y >= rect.y);
        if !session.candidate_first_lyric.is_empty() && !preview_row_taken {
            widgets::draw_text(
                d,
                font,
                &format!("First staged lyric: {}", session.candidate_first_lyric),
                x,
                line_y,
                REASON_FONT_SIZE,
                color::ui_muted(),
            );
        }

        let apply = row.apply;
        let discard = row.discard;
        let apply_label = if session.apply_confirmation_pending() {
            "Confirm apply"
        } else {
            "Apply changes"
        };
        if target.is_none() || draft_conflict {
            self.widgets
                .disabled_button(d, font, apply, apply_label, None);
        } else {
            let id = widgets::widget_id(ASSIST_WIDGETS, 30);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    apply,
                    apply_label,
                    session.apply_confirmation_pending(),
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                // Two presses, and the first one only arms. The C also pushes a
                // persistent notice on the first press; the label change carries
                // the same information without a tray entry to dismiss.
                if session.apply_confirmation_pending() {
                    session.request(AssistRequest::Apply);
                } else {
                    session.set_apply_confirmation_pending(true);
                }
            }
        }
        let id = widgets::widget_id(ASSIST_WIDGETS, 31);
        if self
            .widgets
            .text_button(
                d,
                font,
                id,
                discard,
                "Discard",
                false,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            session.request(AssistRequest::Discard);
        }
        if let Some(reason) = blocked_reason {
            draw_blocking_reason(d, font, reason, row.reason);
        }
        self.assist_artifact_actions(d, input, boundary, row.artifacts_x, row.apply.y, gap);
        // Inside the lyrics-lane guard (review LT1-R, R1): a review is a claim
        // about lyric content, and a run that staged no lyrics lane is offering
        // none. `attach_lyrics_review` already refuses one, and this is the
        // second lock on the same door — the drawing cannot outlive the lane.
        if let Some(review) = candidate.lyrics_review().filter(|_| available.lyrics) {
            let drawn = draw_lyrics_review(
                d,
                font,
                review,
                x,
                action_y + REVIEW_BLOCK_OFFSET,
                (boundary.width - padding * 2.0).max(0.0),
                // The scissor's own inner edge, which is what actually cuts a
                // row. `boundary.height` is `available.min(required)`, so at a
                // window too short for the panel this is *less* than the block
                // asked for and the list has to fit what it got.
                boundary.y + boundary.height - 1.0,
            );
            report_review(describe_review(Some(candidate), drawn));
        }
    }

    /// The three Copy buttons (`draw_assist_artifact_actions`, `plug.c:2069-2110`).
    ///
    /// A button whose artifact this job did not produce is drawn disabled rather
    /// than hidden: the run that failed is exactly the run whose log someone
    /// wants.
    fn assist_artifact_actions(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        mut x: f32,
        y: f32,
        gap: f32,
    ) {
        let font = input.fonts.ui();
        let session = &input.workspace.assist;
        let labels: Vec<&str> = AssistArtifact::ALL.iter().map(|a| a.label()).collect();
        let widths: Vec<f32> = AssistArtifact::ALL.iter().map(|a| a.width()).collect();
        let font_size = widgets::row_font_size(font, &labels, &widths, metric::UI_BUTTON_HEIGHT);

        for (index, artifact) in AssistArtifact::ALL.into_iter().enumerate() {
            let button = UiRect::new(x, y, artifact.width(), metric::UI_BUTTON_HEIGHT);
            x += artifact.width() + gap;
            if !boundary.contains(button) {
                continue;
            }
            let path = session.artifact_path(artifact);
            // Existence is checked, not assumed: a job can exit before writing a
            // bridge, and a Copy that hands over a path to nothing is worse than a
            // disabled button.
            let available = !path.is_empty()
                && match artifact {
                    AssistArtifact::Folder => Path::new(path).is_dir(),
                    _ => Path::new(path).is_file(),
                };
            if !available {
                self.widgets
                    .disabled_button(d, font, button, artifact.label(), Some(font_size));
                continue;
            }
            let id = widgets::widget_id(ASSIST_WIDGETS, 40 + index as u32);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    button,
                    artifact.label(),
                    false,
                    ButtonStyle::Neutral,
                    Some(font_size),
                )
                .clicked
            {
                session.request(AssistRequest::Copy(artifact));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::project::lyrics::LyricsDocument;
    use musializer_core::project::sha256;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "musializer-assist-panel-{}-{label}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A controller with no helper and no job, for the paths that never spawn.
    fn controller() -> AssistController {
        AssistController {
            job: None,
            nonce: 0,
            helper: None,
        }
    }

    /// The fixture's own base64, written out rather than borrowed from
    /// `project::lyrics` — which is `pub(crate)` there anyway. Duplicating the
    /// fixture generator is this repository's rule for exactly this reason: a
    /// shared encoder can hide the difference the parser is supposed to catch.
    fn b64(text: &str) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in text.as_bytes().chunks(3) {
            let bytes = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let packed =
                (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
            out.push(ALPHABET[(packed >> 18) as usize & 63] as char);
            out.push(ALPHABET[(packed >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(packed >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[packed as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    /// A bridge document for `audio`, covering 60 s with all three lanes.
    fn document(audio: &[u8]) -> String {
        let digest = sha256::digest_hex(audio);
        let reasons = b64("[\"chorus\"]");
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             LYRIC\t1\t1000\t2000\t900\tnone\t{}\n\
             SECTION\t10\t0\t60000\tspectrum\t500\t{reasons}\n\
             SEMANTIC\t20\t0\t60000\t400\t300\t-200\t900\t{}\n",
            b64("first line"),
            b64("calm")
        )
    }

    /// A workspace with one track whose audio really exists on disk, because the
    /// digest guard is the thing under test and it hashes the file.
    fn workspace_with_track(scratch: &Scratch) -> (Workspace, PathBuf) {
        let audio = scratch.join("track.wav");
        std::fs::write(&audio, b"synthetic audio").expect("audio");
        let track = Track::new(audio.clone(), 60.0, SceneId::Spectrum, 7).expect("track");
        let mut workspace = Workspace::new();
        workspace.push(track);
        (workspace, audio)
    }

    fn bridge_for(scratch: &Scratch, audio: &Path) -> PathBuf {
        let bridge = scratch.join("ok.bridge.tsv");
        std::fs::write(&bridge, document(&std::fs::read(audio).unwrap())).expect("bridge");
        bridge
    }

    #[test]
    fn the_workspace_directory_hash_is_the_oracles_djb2() {
        // 5381*33 + byte over the path, then over the eight bytes of the duration
        // (`plug.c:3262-3264`). Reproduced so the Python helper's measured and
        // model caches survive a switch between the two implementations.
        let mut expected = 5381u64;
        for byte in b"/tmp/a.wav" {
            expected = expected.wrapping_mul(33).wrapping_add(u64::from(*byte));
        }
        for byte in 60.0f64.to_ne_bytes() {
            expected = expected.wrapping_mul(33).wrapping_add(u64::from(byte));
        }
        assert_eq!(workspace_hash(Path::new("/tmp/a.wav"), 60.0), expected);
        // The duration participates, so two tracks at the same path with
        // different decoded lengths do not share a cache.
        assert_ne!(
            workspace_hash(Path::new("/tmp/a.wav"), 60.0),
            workspace_hash(Path::new("/tmp/a.wav"), 60.5)
        );
    }

    #[test]
    fn a_bridge_for_different_audio_is_refused_before_anything_is_staged() {
        let scratch = Scratch::new("digest");
        let (workspace, _) = workspace_with_track(&scratch);
        let bridge = scratch.join("wrong.bridge.tsv");
        std::fs::write(&bridge, document(b"some other audio")).expect("bridge");
        let error = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL)
            .expect_err("a bridge for other audio must not attach to this track");
        assert!(error.contains("does not match"), "{error}");
    }

    #[test]
    fn a_bridge_whose_duration_disagrees_by_more_than_the_padding_tail_is_refused() {
        let scratch = Scratch::new("duration");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);

        // 0.25 s is the allowance for an MP3 encoder-padding tail
        // (`plug.c:3466-3475`). A fifth of a second is fine; a whole one is not.
        workspace.get_mut(0).unwrap().lyrics = LyricsDocument::new(60.2).unwrap();
        assert!(load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).is_ok());

        workspace.get_mut(0).unwrap().lyrics = LyricsDocument::new(61.0).unwrap();
        let error = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).unwrap_err();
        assert!(error.contains("duration does not match"), "{error}");
    }

    #[test]
    fn an_empty_absent_or_oversized_bridge_is_refused_without_being_read() {
        let scratch = Scratch::new("bounds");
        let (workspace, _) = workspace_with_track(&scratch);
        let empty = scratch.join("empty.bridge.tsv");
        std::fs::write(&empty, b"").expect("empty");
        let error = load_candidate(&empty, &workspace.tracks()[0], Lanes::ALL).unwrap_err();
        assert!(error.contains("4 MiB input limit"), "{error}");
        // A path that is not there at all is the same refusal, not a panic.
        let error = load_candidate(&scratch.join("absent"), &workspace.tracks()[0], Lanes::ALL)
            .unwrap_err();
        assert!(error.contains("4 MiB input limit"), "{error}");
    }

    #[test]
    fn staging_hashes_the_audio_once_and_hands_the_digest_back() {
        let scratch = Scratch::new("hash-once");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);

        // The track has no digest yet, so loading computes one and returns it for
        // the caller to store — which is what stops the next run rehashing.
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");
        assert_eq!(
            loaded.audio_sha256.as_deref(),
            Some(sha256::digest_hex(b"synthetic audio").as_str())
        );
        assert_eq!(loaded.candidate.available(), Lanes::ALL);

        workspace.get_mut(0).unwrap().audio_sha256 = sha256::digest_hex(b"synthetic audio");
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");
        assert_eq!(
            loaded.audio_sha256, None,
            "a known digest is not recomputed"
        );
    }

    #[test]
    fn a_mode_only_stages_the_lanes_it_authorized() {
        let scratch = Scratch::new("lanes");
        let (workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(
            &bridge,
            &workspace.tracks()[0],
            assist_ui_state::mode_lanes(AssistMode::Sections),
        )
        .expect("stages");
        let available = loaded.candidate.available();
        assert!(available.sections);
        assert!(!available.lyrics, "a Sections run must not touch lyrics");
        assert!(!available.semantics);
        // And the "did it produce anything I asked for?" test agrees.
        assert!(assist_ui_state::result_has_changes(
            lanes_bits(assist_ui_state::mode_lanes(AssistMode::Sections)),
            lanes_bits(available)
        ));
        assert!(!assist_ui_state::result_has_changes(
            lanes_bits(assist_ui_state::mode_lanes(AssistMode::Mimo)),
            lanes_bits(available)
        ));
    }

    #[test]
    fn applying_replaces_the_lanes_and_records_their_provenance() {
        let scratch = Scratch::new("apply");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");

        workspace.assist.candidate = Some(loaded.candidate);
        workspace.assist.candidate_track = Some(0);
        workspace.assist.candidate_mode = AssistMode::All;
        workspace.assist.bridge_path = bridge.display().to_string();
        workspace.assist.job_state = AssistJobState::Succeeded;

        let notices = controller().apply(&mut workspace, 12.0);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Success);

        let track = &workspace.tracks()[0];
        assert_eq!(track.lyrics.len(), 1);
        assert_eq!(track.scene_switches.len(), 1);
        assert_eq!(track.semantic_events.len(), 1);
        assert!(track.project_dirty, "applying is an edit");
        // Provenance: one entry per applied lane, all pointing at the artifact.
        assert_eq!(track.analysis_lanes.len(), 3);
        for lane in &track.analysis_lanes {
            assert_eq!(lane.path, bridge.display().to_string());
            assert_eq!(lane.provenance.adapter, "external-analysis");
            assert_eq!(lane.provenance.schema_version, "analysis-bridge-v1");
            assert_eq!(lane.provenance.prompt_version, "v1");
            assert!(
                lane.provenance.provider.is_empty(),
                "the C leaves provider zeroed; inventing one writes a field the frozen C never does"
            );
            assert_eq!(lane.audio_sha256, sha256::digest_hex(b"synthetic audio"));
        }
        // An imported plan adopts the track's settings rather than a zeroed
        // snapshot (`capture_missing_scene_cue_settings`).
        assert!(track.scene_switches.cues()[0].settings.captured);
        // The staged result is gone, and the artifact paths are left behind.
        assert!(workspace.assist.candidate.is_none());
        assert_eq!(workspace.assist.job_state, AssistJobState::Idle);
        assert_eq!(workspace.assist.bridge_path, bridge.display().to_string());
    }

    #[test]
    fn applying_preserves_the_users_auto_scene_opt_in() {
        let scratch = Scratch::new("optin");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");
        workspace.get_mut(0).unwrap().scene_switches.enabled = true;
        workspace.assist.candidate = Some(loaded.candidate);
        workspace.assist.candidate_track = Some(0);
        workspace.assist.bridge_path = bridge.display().to_string();

        controller().apply(&mut workspace, 0.0);
        assert!(
            workspace.tracks()[0].scene_switches.enabled,
            "applying a plan must not decide the toggle for the user"
        );
        assert_eq!(workspace.tracks()[0].scene_switches.active_index(), None);
    }

    #[test]
    fn re_running_one_workflow_replaces_its_lane_rather_than_appending() {
        // Eight lane slots and three kinds: appending would let three runs of the
        // same workflow overflow the bound a `.musi` can represent.
        let scratch = Scratch::new("relane");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");

        for _ in 0..4 {
            let staged =
                stage_analysis_lanes(&workspace.tracks()[0], &loaded.candidate, &bridge, false)
                    .expect("provenance");
            workspace.get_mut(0).unwrap().analysis_lanes = staged.lanes;
        }
        assert_eq!(workspace.tracks()[0].analysis_lanes.len(), 3);
    }

    #[test]
    fn an_imported_bridge_records_that_it_was_imported() {
        // The model string is the difference between "a helper produced this
        // here" and "somebody handed us a file", and it survives into the `.musi`.
        let scratch = Scratch::new("imported");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let notices = controller()
            .import_bridge(&bridge, &mut workspace, 3.0)
            .expect("the bridge applies");
        assert_eq!(notices[0].severity, Severity::Success);
        let track = &workspace.tracks()[0];
        assert_eq!(track.lyrics.len(), 1);
        assert_eq!(track.analysis_lanes.len(), 3);
        for lane in &track.analysis_lanes {
            assert_eq!(lane.provenance.model, "imported-bridge");
        }
        // `--analysis-bridge` applies rather than staging: a batch entry point
        // with no review step must not leave the result unapplied.
        assert!(workspace.assist.candidate.is_none());
    }

    #[test]
    fn importing_without_a_track_or_with_a_bad_bridge_reports_rather_than_panics() {
        let scratch = Scratch::new("import-fail");
        let mut empty = Workspace::new();
        assert!(controller()
            .import_bridge(&scratch.join("x.tsv"), &mut empty, 0.0)
            .is_err());

        let (mut workspace, _) = workspace_with_track(&scratch);
        let wrong = scratch.join("wrong.tsv");
        std::fs::write(&wrong, document(b"other audio")).expect("bridge");
        let error = controller()
            .import_bridge(&wrong, &mut workspace, 0.0)
            .unwrap_err();
        assert!(error.contains("does not match"), "{error}");
        assert_eq!(workspace.tracks()[0].lyrics.len(), 0);
    }

    #[test]
    fn discarding_changes_nothing_and_says_so() {
        let scratch = Scratch::new("discard");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");
        workspace.assist.candidate = Some(loaded.candidate);
        workspace.assist.candidate_track = Some(0);

        let notices = controller().discard(&mut workspace);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].detail.contains("Editor content is unchanged"));
        assert!(workspace.assist.candidate.is_none());
        let track = &workspace.tracks()[0];
        assert_eq!(track.lyrics.len(), 0);
        assert!(track.analysis_lanes.is_empty());
        assert!(!track.project_dirty, "discarding is not an edit");
    }

    #[test]
    fn the_lyric_reference_prefers_a_chosen_sheet_and_falls_back_when_it_vanishes() {
        let scratch = Scratch::new("reference");
        let (mut workspace, _) = workspace_with_track(&scratch);
        assert_eq!(
            resolve_lyric_reference(workspace.current()).0,
            AssistLyricReference::None
        );

        // The helper's own sibling rule: <stem>.lyrics.txt beside the audio.
        let sibling = scratch.join("track.lyrics.txt");
        std::fs::write(&sibling, "a line").expect("sibling");
        let (reference, path) = resolve_lyric_reference(workspace.current());
        assert_eq!(reference, AssistLyricReference::Sibling);
        assert_eq!(path, sibling.display().to_string());

        let chosen = scratch.join("mine.txt");
        std::fs::write(&chosen, "my words").expect("chosen");
        workspace.get_mut(0).unwrap().lyrics_reference_path = Some(chosen.clone());
        assert_eq!(
            resolve_lyric_reference(workspace.current()),
            (AssistLyricReference::Chosen, chosen.display().to_string())
        );

        // A chosen file that has since been deleted falls back to the sibling
        // rather than failing the run with a path nobody can see.
        std::fs::remove_file(&chosen).expect("remove");
        assert_eq!(
            resolve_lyric_reference(workspace.current()).0,
            AssistLyricReference::Sibling
        );
    }

    #[test]
    fn a_lyric_sheet_is_bounded_where_the_user_can_see_it() {
        let scratch = Scratch::new("sheet-bound");
        let (mut workspace, _) = workspace_with_track(&scratch);
        let mut controller = controller();

        let empty = scratch.join("empty.txt");
        std::fs::write(&empty, b"").expect("empty");
        let notices = controller.set_lyric_sheet(&mut workspace, &empty);
        assert_eq!(notices[0].severity, Severity::Error);
        assert!(workspace.tracks()[0].lyrics_reference_path.is_none());

        let huge = scratch.join("huge.txt");
        std::fs::write(&huge, vec![b'x'; LYRIC_REFERENCE_BYTE_LIMIT as usize + 1]).expect("huge");
        let notices = controller.set_lyric_sheet(&mut workspace, &huge);
        assert_eq!(notices[0].severity, Severity::Error);
        assert!(workspace.tracks()[0].lyrics_reference_path.is_none());

        let good = scratch.join("good.txt");
        std::fs::write(&good, "a line").expect("good");
        let notices = controller.set_lyric_sheet(&mut workspace, &good);
        assert_eq!(notices[0].severity, Severity::Info);
        assert_eq!(
            workspace.tracks()[0].lyrics_reference_path.as_deref(),
            Some(good.as_path())
        );

        // Clear is the panel's other half, and it only forgets the choice.
        let outcome = controller.handle(AssistRequest::ClearLyricSheet, &mut workspace, 0.0);
        assert!(outcome.notices.is_empty());
        assert!(workspace.tracks()[0].lyrics_reference_path.is_none());
    }

    #[test]
    fn a_copy_request_only_offers_a_path_it_has() {
        let mut workspace = Workspace::new();
        let mut controller = controller();
        let outcome = controller.handle(
            AssistRequest::Copy(AssistArtifact::Log),
            &mut workspace,
            0.0,
        );
        assert_eq!(outcome.effect, None);
        workspace.assist.log_path = "/tmp/a.log".to_string();
        let outcome = controller.handle(
            AssistRequest::Copy(AssistArtifact::Log),
            &mut workspace,
            0.0,
        );
        assert_eq!(
            outcome.effect,
            Some(AssistEffect::Clipboard("/tmp/a.log".to_string()))
        );
    }

    #[test]
    fn starting_without_a_helper_or_a_track_reports_rather_than_spawning() {
        let mut workspace = Workspace::new();
        let mut controller = controller();
        // `helper_available` is false on the session, so the block is reported.
        let notices = controller.start(&mut workspace, 0.0);
        assert_eq!(notices.len(), 1);
        assert_eq!(
            notices[0].detail,
            AssistStartBlock::HelperUnavailable.reason()
        );
        assert!(!controller.is_running());

        // With a helper claimed but no track, the refusal names the real reason.
        workspace.assist.helper_available = true;
        let notices = controller.start(&mut workspace, 0.0);
        assert_eq!(notices[0].detail, "There is no track to analyse.");
    }

    #[test]
    fn a_staged_result_blocks_a_second_run() {
        let scratch = Scratch::new("blocked");
        let (mut workspace, audio) = workspace_with_track(&scratch);
        let bridge = bridge_for(&scratch, &audio);
        let loaded = load_candidate(&bridge, &workspace.tracks()[0], Lanes::ALL).expect("stages");
        workspace.assist.helper_available = true;
        workspace.assist.candidate = Some(loaded.candidate);
        workspace.assist.candidate_track = Some(0);

        let notices = controller().start(&mut workspace, 0.0);
        assert_eq!(notices[0].detail, AssistStartBlock::ResultPending.reason());
        assert!(
            workspace.assist.blocks_close(),
            "a staged result is undecided work the quit guard must weigh"
        );
    }

    #[test]
    fn dismissing_the_confirmation_starts_nothing() {
        let mut workspace = Workspace::new();
        workspace.assist.select_mode(AssistMode::Mimo);
        assert!(workspace.assist.confirmation_pending());
        let mut controller = controller();
        controller.handle(AssistRequest::DismissConfirmation, &mut workspace, 0.0);
        assert!(!workspace.assist.confirmation_pending());
        assert!(!controller.is_running());
        assert_eq!(workspace.assist.job_state, AssistJobState::Idle);
    }

    // -----------------------------------------------------------------------
    // The action row (review 1.13).
    // -----------------------------------------------------------------------

    /// Do two rectangles share a pixel? Touching edges do not count.
    fn overlaps(a: UiRect, b: UiRect) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    /// The Copy strip as one rectangle, which is what the reason must clear.
    fn artifact_strip(row: &CandidateActionRow, gap: f32) -> UiRect {
        UiRect::new(
            row.artifacts_x,
            row.apply.y,
            artifact_strip_width(gap),
            metric::UI_BUTTON_HEIGHT,
        )
    }

    /// Every panel width the shell can hand this body, in 1 px steps.
    fn width_sweep() -> impl Iterator<Item = f32> {
        (240..=1600).map(|width| width as f32)
    }

    #[test]
    fn the_blocking_reason_never_shares_a_pixel_with_a_button() {
        // This is the defect itself, as arithmetic. Before the fix the reason was
        // drawn at `discard.right + gap` and the Copy buttons started at the same
        // x and y, so every width failed this.
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        for width in width_sweep() {
            let boundary = UiRect::new(12.0, 200.0, width, 260.0);
            let row = candidate_action_row(boundary, 300.0, padding, gap, true);
            let strip = artifact_strip(&row, gap);
            let reason = match row.reason {
                ReasonSlot::None => panic!("a blocked row must offer the reason a slot"),
                ReasonSlot::Beside(rect) | ReasonSlot::Above(rect) => rect,
            };
            assert!(reason.width > 0.0, "at {width}px the reason has no room");
            for (name, rect) in [
                ("apply", row.apply),
                ("discard", row.discard),
                ("artifacts", strip),
            ] {
                assert!(
                    !overlaps(reason, rect),
                    "at {width}px the reason overlaps {name}: {reason:?} vs {rect:?}"
                );
            }
            assert!(!overlaps(row.apply, row.discard));
            assert!(!overlaps(row.discard, strip));
        }
    }

    #[test]
    fn the_reason_takes_its_own_row_only_when_it_cannot_fit_beside_the_buttons() {
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        let mut previous_beside: Option<f32> = None;
        let mut above_widths = Vec::new();
        let mut beside_widths = Vec::new();
        for width in width_sweep() {
            let boundary = UiRect::new(12.0, 200.0, width, 260.0);
            let row = candidate_action_row(boundary, 300.0, padding, gap, true);
            match row.reason {
                ReasonSlot::Above(rect) => {
                    above_widths.push(width);
                    // The full inner width, and it ends exactly where the buttons
                    // begin rather than running under them.
                    assert_eq!(rect.width, width - padding * 2.0);
                    assert_eq!(rect.y + rect.height, row.apply.y);
                }
                ReasonSlot::Beside(rect) => {
                    beside_widths.push(width);
                    assert!(rect.width >= REASON_MIN_WIDTH);
                    // Monotone in the panel width, which is what keeps a resize
                    // from making the sentence jump between the two slots more
                    // than once. `transport_bar` earned this rule.
                    if let Some(previous) = previous_beside {
                        assert!(rect.width > previous, "at {width}px the slot shrank");
                    }
                    previous_beside = Some(rect.width);
                }
                ReasonSlot::None => panic!("a blocked row must offer the reason a slot"),
            }
        }
        // One crossing, not several: every narrow width uses the row above and
        // every wide one uses the slot beside.
        assert!(!above_widths.is_empty() && !beside_widths.is_empty());
        assert!(
            above_widths
                .iter()
                .all(|narrow| beside_widths.iter().all(|wide| narrow < wide)),
            "the two slots interleave, so a resize would flap between them"
        );
    }

    #[test]
    fn an_unblocked_row_keeps_the_buttons_where_a_blocked_one_puts_them() {
        // The reason appears and disappears while the user is looking at the
        // panel. If the Copy strip moved with it, a press aimed at Copy log
        // would land on Copy folder.
        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        for width in width_sweep() {
            let boundary = UiRect::new(12.0, 200.0, width, 260.0);
            let blocked = candidate_action_row(boundary, 300.0, padding, gap, true);
            let clear = candidate_action_row(boundary, 300.0, padding, gap, false);
            assert_eq!(clear.reason, ReasonSlot::None);
            assert_eq!(clear.apply, blocked.apply);
            assert_eq!(clear.discard, blocked.discard);
            assert_eq!(clear.artifacts_x, blocked.artifacts_x);
        }
    }

    /// A measure that is not a font: 7 px a character, so an assertion about the
    /// algorithm cannot be quietly satisfied by a metric change.
    fn measure_7px(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    #[test]
    fn the_reason_wraps_within_its_slot_and_ellipsizes_only_when_it_must() {
        let text = "The target track is no longer available. Discard this result.";
        let measure: &dyn Fn(&str) -> f32 = &measure_7px;

        // The narrowest slot the row will hand out, two lines.
        let lines = wrap_to_width(text, REASON_MIN_WIDTH, 2, measure);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(
                measure(line) <= REASON_MIN_WIDTH,
                "{line:?} runs past the slot"
            );
        }
        assert!(
            lines.iter().all(|line| !line.ends_with("...")),
            "both reasons fit whole in the narrowest slot the row hands out: {lines:?}"
        );
        assert_eq!(lines.concat().replace(' ', ""), text.replace(' ', ""));

        // A sentence that does not fit says so rather than stopping mid-word.
        let long = format!("{text} {text}");
        let lines = wrap_to_width(&long, REASON_MIN_WIDTH, 2, measure);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."));
        assert!(measure(&lines[1]) <= REASON_MIN_WIDTH);

        // Wide enough, and nothing is cut.
        let lines = wrap_to_width(text, 900.0, 2, measure);
        assert_eq!(lines, vec![text.to_string()]);

        // A single row, which is the narrow-panel fallback.
        let one = ellipsize(text, 210.0, measure);
        assert!(measure(&one) <= 210.0 && one.ends_with("..."));
        assert_eq!(ellipsize(text, 900.0, measure), text);
        // Degenerate widths answer rather than loop.
        assert_eq!(ellipsize(text, 1.0, measure), "...");
        assert!(wrap_to_width(text, 0.0, 2, measure).is_empty());
        assert!(wrap_to_width(text, 200.0, 0, measure).is_empty());
    }

    // -----------------------------------------------------------------------
    // The probe states (review 4.2).
    // -----------------------------------------------------------------------

    #[test]
    fn both_reason_slots_are_reachable_at_a_supported_window_size() {
        // Which matters for the capture gate: if every supported size chose the
        // same slot, the other branch would be code nothing photographs — the
        // exact failure review 4.2 is about. The width here is the one
        // `assist_timeline_height` measures the panel with, less its padding.
        let slot_at = |window: f32, inspector_open: bool| {
            let width = WorkspaceFrame::assist_panel_width(window, inspector_open)
                - metric::UI_PANEL_PADDING * 2.0;
            let boundary = UiRect::new(0.0, 0.0, width, 300.0);
            candidate_action_row(
                boundary,
                0.0,
                metric::UI_PANEL_PADDING,
                metric::UI_CONTROL_GAP,
                true,
            )
            .reason
        };
        assert!(matches!(slot_at(1280.0, false), ReasonSlot::Beside(_)));
        assert!(matches!(slot_at(1280.0, true), ReasonSlot::Beside(_)));
        assert!(matches!(slot_at(960.0, false), ReasonSlot::Beside(_)));
        // The supported minimum with the inspector open is the narrow case, and
        // it is the one the gate should photograph for the row above.
        assert!(matches!(slot_at(960.0, true), ReasonSlot::Above(_)));
    }

    #[test]
    fn every_probe_state_reaches_the_body_it_names() {
        use crate::cli::AssistProbe;
        for (state, body) in [
            (AssistProbe::Confirm, AssistPanelContent::Confirmation),
            (AssistProbe::Candidate, AssistPanelContent::Candidate),
            (AssistProbe::Running, AssistPanelContent::Running),
            (AssistProbe::Failed, AssistPanelContent::Empty),
        ] {
            let mut workspace = Workspace::new();
            workspace.assist.helper_available = true;
            apply_probe_state(&mut workspace, state, 0.0).expect("synthesizes");
            assert_eq!(
                workspace.assist.panel_content(),
                body,
                "assist={state:?} must photograph {body:?}"
            );
        }
    }

    #[test]
    fn the_probe_candidate_is_fixed_content_with_apply_blocked() {
        let mut workspace = Workspace::new();
        apply_probe_state(&mut workspace, crate::cli::AssistProbe::Candidate, 0.0)
            .expect("synthesizes");
        let session = &workspace.assist;
        let candidate = session.candidate.as_ref().expect("staged");
        // All three lanes, which is the worst case for the layout: three summary
        // lines plus the preview line, and then the button row.
        assert!(
            candidate.available().lyrics
                && candidate.available().sections
                && candidate.available().semantics
        );
        assert_eq!(candidate.lyrics().cues().len(), 2);
        assert_eq!(candidate.uncertain_lyric_count(), 1);
        assert_eq!(candidate.sections().len(), 2);
        assert_eq!(candidate.semantic_events().len(), 2);
        assert_eq!(session.candidate_first_lyric, "we were never meant to stay");
        // The point of the state: the target is gone, so Apply is greyed and the
        // panel owes the user a sentence saying why (review 1.13).
        assert_eq!(session.candidate_track, Some(PROBE_MISSING_TRACK));
        assert!(workspace.get(PROBE_MISSING_TRACK).is_none());
    }

    // -----------------------------------------------------------------------
    // Tranche LT1: the review surface.
    // -----------------------------------------------------------------------

    /// A job folder as `external_analysis.py` leaves one: a manifest and the
    /// aligned lyric document beside it.
    fn review_job_folder(scratch: &Scratch, name: &str, manifest: &str, document: &str) -> PathBuf {
        let folder = scratch.join(name);
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join(ASSIST_MANIFEST_NAME), manifest).expect("manifest");
        std::fs::write(folder.join("lyrics.aligned.json"), document).expect("document");
        folder
    }

    const LT1_MANIFEST: &str = r#"{"schema_version":"musializer.assist-manifest/v1",
        "artifacts":{"aligned":"/nonexistent/job/lyrics.aligned.json"},
        "result_counts":{"lyrics":2,"lyrics_unresolved":1,"lyrics_review_flags":2},
        "lyric_localization":{"policy":"anchor-block-mms","policy_version":"3"}}"#;

    const LT1_DOCUMENT: &str = r#"{"localization_policy":"anchor-block-mms",
        "unresolved":[{"reference_line_index":25,"text":"hold the note until it breaks",
                       "reason":"no block placement","abstained":false,
                       "coarse_start_seconds":90.6,"coarse_end_seconds":94.2}],
        "review_flags":[
            {"reference_line_index":25,"text":"hold the note until it breaks",
             "flag":"unresolved","reason":"no block placement"},
            {"reference_line_index":0,"text":"we were never meant to stay",
             "flag":"coarse_disagreement","reason":"the two views differ by 21.6 s",
             "start_seconds":12.0,"end_seconds":16.0}]}"#;

    #[test]
    fn a_pre_lt1_job_folder_renders_the_review_it_always_rendered() {
        let scratch = Scratch::new("review-legacy");
        // No `lyrics_unresolved`, no `lyrics_review_flags`, no
        // `lyric_localization`: the manifest a job wrote before this tranche.
        let folder = review_job_folder(
            &scratch,
            "legacy",
            r#"{"schema_version":"musializer.assist-manifest/v1",
                "result_counts":{"lyrics":2,"lyrics_unmatched":0}}"#,
            r#"{"lane":"lyric_sync","lines":[]}"#,
        );
        assert!(read_lyrics_review(&folder.display().to_string()).is_none());
        assert_eq!(
            describe_review(None, ReviewDraw::default()),
            "absent (this run left no lyrics-lane LT1 review artifact)"
        );
        // And an empty or missing folder is the same answer, not a crash.
        assert!(read_lyrics_review("").is_none());
        assert!(read_lyrics_review("/nonexistent/musializer-assist-probe").is_none());
    }

    #[test]
    fn an_lt1_job_folder_is_read_into_names_counts_and_a_report_line() {
        let scratch = Scratch::new("review-flagged");
        let folder = review_job_folder(&scratch, "flagged", LT1_MANIFEST, LT1_DOCUMENT);
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert_eq!(review.unresolved, 1);
        assert_eq!(review.flagged, 2);
        assert_eq!(review.entries.len(), 2);

        let mut candidate = probe_candidate(AssistMode::All).expect("probe candidate");
        assert!(candidate.attach_lyrics_review(review));
        assert_eq!(
            describe_review(
                Some(&candidate),
                ReviewDraw {
                    rows: 2,
                    tail: false
                }
            ),
            "unresolved=1 flagged=2 listed=2 rows_drawn=2 tail=no omitted=0 counts=document \
             manifest=1/2 policy=anchor-block-mms | \
             UNPLACED line 26 proposed 1:30.6-1:34.2 \"hold the note until it breaks\" ; \
             CHECK line 1 at 0:12.0-0:16.0 \"we were never meant to stay\"",
            "the report line carries the names and the windows, not only the counts; this \
             fixture's flag records no delta, so the CHECK row has no detail to add"
        );
    }

    #[test]
    fn a_manifest_naming_a_document_elsewhere_is_read_from_the_job_folder() {
        // The manifest above names `/nonexistent/job/lyrics.aligned.json`, which
        // does not exist. The document that is read is the one in the folder the
        // supervisor chose, so a moved — or edited — manifest cannot send this
        // read out of the workspace.
        let scratch = Scratch::new("review-escape");
        let folder = review_job_folder(
            &scratch,
            "escape",
            r#"{"schema_version":"musializer.assist-manifest/v1",
                "artifacts":{"aligned":"/etc/shadow"},
                "result_counts":{"lyrics_unresolved":1,"lyrics_review_flags":2}}"#,
            LT1_DOCUMENT,
        );
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert_eq!(
            review.entries.len(),
            2,
            "the folder's own document was read"
        );
    }

    #[test]
    fn a_run_that_placed_every_line_says_so_rather_than_drawing_nothing() {
        let scratch = Scratch::new("review-clear");
        let folder = review_job_folder(
            &scratch,
            "clear",
            r#"{"schema_version":"musializer.assist-manifest/v1",
                "result_counts":{"lyrics":2,"lyrics_unresolved":0,"lyrics_review_flags":0},
                "lyric_localization":{"policy":"anchor-block-mms","policy_version":"3"}}"#,
            r#"{"localization_policy":"anchor-block-mms","unresolved":[],"review_flags":[],
                "lines":[{"reference_line_index":0,"text":"a","start_seconds":1.0}]}"#,
        );
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert!(review.is_clear());

        let mut candidate = probe_candidate(AssistMode::All).expect("probe candidate");
        assert!(candidate.attach_lyrics_review(review));
        assert_eq!(
            describe_review(Some(&candidate), ReviewDraw::default()),
            "unresolved=0 flagged=0 listed=0 rows_drawn=0 tail=no omitted=0 counts=document \
             manifest=0/0 policy=anchor-block-mms | All lines placed, none flagged"
        );
        // A row is still reserved for it: a blank region is indistinguishable
        // from a broken one, so "nothing to check" must occupy pixels.
        assert!(review_block_height(Some(&candidate)) > 0.0);
    }

    #[test]
    fn the_review_block_costs_no_height_until_there_is_one_to_draw() {
        let mut workspace = Workspace::new();
        apply_probe_state(&mut workspace, crate::cli::AssistProbe::Candidate, 0.0)
            .expect("synthesizes");
        // No `MUSIALIZER_ASSIST_PROBE_DIR`, so the probe's artifacts do not
        // exist and the panel asks for exactly what it asked for before LT1.
        // That is what keeps every pre-existing capture's geometry unchanged.
        assert_eq!(
            review_block_height(workspace.assist.candidate.as_ref()),
            0.0
        );
        assert_eq!(review_block_height(None), 0.0);
    }

    #[test]
    fn the_review_block_fits_the_height_the_panel_can_actually_get() {
        // `assist_ui_state::timeline_height` caps the strip at
        // `screen - toolbar - 150`, so at the supported 1280x720 there are 88 px
        // over the body the oracle's layout asks for. A block taller than that
        // would be drawn into the scissor and photograph as a truncated list.
        let scratch = Scratch::new("review-height");
        let mut flags = String::new();
        for index in 0..40 {
            if index > 0 {
                flags.push(',');
            }
            flags.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"line {index}","flag":"unresolved"}}"#
            ));
        }
        let folder = review_job_folder(
            &scratch,
            "many",
            LT1_MANIFEST,
            &format!(r#"{{"unresolved":[],"review_flags":[{flags}]}}"#),
        );
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert_eq!(review.flagged, 40);
        let mut candidate = probe_candidate(AssistMode::All).expect("probe candidate");
        assert!(candidate.attach_lyrics_review(review));

        // Three named rows and the "3 of 40 shown" tail: four rows plus the
        // heading.
        assert_eq!(
            review_rows(candidate.lyrics_review().unwrap(), REVIEW_MAX_ROWS),
            (3, 37)
        );
        let block = review_block_height(Some(&candidate));
        assert_eq!(
            block,
            18.0 + REVIEW_MAX_ROWS as f32 * REVIEW_ROW_HEIGHT + 4.0
        );
        let layout = assist_ui_state::ui_layout(
            WorkspaceFrame::assist_panel_width(1280.0, false) - metric::UI_PANEL_PADDING * 2.0,
            AssistPanelContent::Candidate,
            false,
        );
        let ceiling = 720.0 - metric::HUD_BUTTON_SIZE - 150.0 - 158.0;
        assert!(
            layout.required_height + block <= ceiling,
            "the review block must fit the 1280x720 ceiling: {} + {block} > {ceiling}",
            layout.required_height
        );
    }

    // -----------------------------------------------------------------------
    // Review LT1-R.
    // -----------------------------------------------------------------------

    #[test]
    fn a_sections_run_never_reads_the_previous_runs_lyric_document() {
        // R1, the reviewer's `jobs/stale`. Both artifacts are here, exactly as
        // an audio-keyed cache folder leaves them: the Sections run's manifest,
        // and the lyric document a Full-assist run wrote earlier. The panel used
        // to name four flags from a lane this run never had.
        let scratch = Scratch::new("review-stale");
        let folder = review_job_folder(
            &scratch,
            "stale",
            r#"{"schema_version":"musializer.assist-manifest/v1","mode":"sections",
                "artifacts":{"aligned":"/cache/job/lyrics.aligned.json"},
                "result_counts":{"lyrics":0,"lyrics_unresolved":0,"lyrics_review_flags":0,
                                 "sections":2,"semantics":0},
                "lyric_localization":null}"#,
            LT1_DOCUMENT,
        );
        assert!(
            read_lyrics_review(&folder.display().to_string()).is_none(),
            "a run with no lyrics lane has no lyric review, whatever is beside it"
        );
        // And the same folder under a lyrics-mode manifest still reads, so this
        // is a lane check and not an accidental refusal of everything.
        std::fs::write(
            folder.join(ASSIST_MANIFEST_NAME),
            r#"{"schema_version":"musializer.assist-manifest/v1","mode":"lyrics",
                "result_counts":{"lyrics_unresolved":1,"lyrics_review_flags":2}}"#,
        )
        .expect("manifest");
        assert!(read_lyrics_review(&folder.display().to_string()).is_some());
    }

    #[test]
    fn a_candidate_without_a_lyrics_lane_reserves_no_review_height() {
        // The drawing guard's arithmetic half (R1). A review cannot attach to a
        // sections-only candidate, so this is belt and braces — but the block's
        // height is what pushes the scene preview down, and it must not be
        // reserved for a lane the panel is not offering.
        let candidate = probe_candidate(AssistMode::Sections).expect("probe candidate");
        assert!(!candidate.available().lyrics);
        assert_eq!(review_block_height(Some(&candidate)), 0.0);
    }

    #[test]
    fn the_probe_can_stage_a_lyrics_only_candidate() {
        // R11: the arrangement the LT1 review actually ships in — one lane line
        // and the review under it — could not be photographed at all.
        let candidate = probe_candidate(AssistMode::Lyrics).expect("probe candidate");
        assert_eq!(candidate.available(), Lanes::lyrics_only());
        assert_eq!(candidate.lyrics().cues().len(), 2);
        assert!(candidate.sections().is_empty());
        assert!(candidate.semantic_events().is_empty());
        // And the variable's grammar, including the fallback that keeps every
        // existing capture on `all`.
        assert_eq!(PROBE_LANES_VARIABLE, "MUSIALIZER_ASSIST_PROBE_LANES");
    }

    #[test]
    fn a_list_shorter_than_its_count_still_gets_a_tail() {
        // R4, the reviewer's `jobs/dropped`: four flags, two of them unnameable.
        // Two rows and no tail is a picture that says "these two", and there is
        // no width or height at which that becomes true.
        let scratch = Scratch::new("review-dropped");
        let folder = review_job_folder(
            &scratch,
            "dropped",
            LT1_MANIFEST,
            r#"{"unresolved":[],"review_flags":[
                {"reference_line_index":0,"text":"we were never meant to stay",
                 "flag":"coarse_disagreement","start_seconds":12.0,"end_seconds":16.0},
                {"reference_line_index":1,"text":"and the lights came up anyway",
                 "flag":"coarse_disagreement","start_seconds":16.0,"end_seconds":21.0},
                {"reference_line_index":5,"flag":"coarse_disagreement","reason":"no text"},
                {"reference_line_index":7,"text":"","flag":"unresolved"}]}"#,
        );
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert_eq!(review.entries.len(), 2);
        assert_eq!(
            review_rows(&review, REVIEW_MAX_ROWS),
            (2, 2),
            "two named rows and a tail owning the two the panel cannot name"
        );
    }

    #[test]
    fn the_tail_is_fitted_before_the_names_are() {
        // R2, the reviewer's `jobs/many` at 960x640: the scissor ate the tail
        // first, so the most truncated panel was the one that admitted least.
        let scratch = Scratch::new("review-tail");
        let mut flags = String::new();
        for index in 0..12 {
            if index > 0 {
                flags.push(',');
            }
            flags.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"line {index}","flag":"unresolved"}}"#
            ));
        }
        let folder = review_job_folder(
            &scratch,
            "many",
            LT1_MANIFEST,
            &format!(r#"{{"unresolved":[],"review_flags":[{flags}]}}"#),
        );
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");

        // Row capacity is whole rows between the first row and the clip edge.
        assert_eq!(review_row_capacity(0.0, 60.0), 4);
        assert_eq!(review_row_capacity(0.0, 59.0), 3);
        assert_eq!(review_row_capacity(0.0, 14.9), 0);
        assert_eq!(review_row_capacity(100.0, 100.0), 0);

        // However few rows survive, the last one is always the tail, and the
        // count it names is the whole count.
        for capacity in 1..=REVIEW_MAX_ROWS {
            let (shown, hidden) = review_rows(&review, capacity);
            assert_eq!(shown, capacity - 1, "the tail keeps its row at {capacity}");
            assert_eq!(shown + hidden, 12, "and it names every line not shown");
            assert!(hidden > 0);
        }
    }

    #[test]
    fn a_list_that_fits_exactly_spends_no_row_on_a_tail() {
        let scratch = Scratch::new("review-exact");
        let folder = review_job_folder(&scratch, "exact", LT1_MANIFEST, LT1_DOCUMENT);
        let review = read_lyrics_review(&folder.display().to_string()).expect("review");
        assert_eq!(review_rows(&review, REVIEW_MAX_ROWS), (2, 0));
        assert_eq!(review_rows(&review, 2), (2, 0));
        // One row fewer than the list, and the tail appears rather than a row
        // silently going missing.
        assert_eq!(review_rows(&review, 1), (0, 2));
    }

    #[test]
    fn the_probed_running_clock_is_the_transport_clock() {
        // A capture must report the same elapsed time every run, so the clock is
        // anchored to the parked transport rather than to the wall.
        for now in [0.0, 5.0, 12.5] {
            let mut workspace = Workspace::new();
            apply_probe_state(&mut workspace, crate::cli::AssistProbe::Running, now)
                .expect("synthesizes");
            assert_eq!(
                now - workspace.assist.started_at,
                PROBE_ELAPSED_SECONDS,
                "elapsed must not depend on when the probe ran"
            );
        }
    }

    #[test]
    fn the_probed_failure_stays_on_one_line() {
        let mut workspace = Workspace::new();
        apply_probe_state(&mut workspace, crate::cli::AssistProbe::Failed, 0.0)
            .expect("synthesizes");
        let session = &workspace.assist;
        assert_eq!(session.job_state, AssistJobState::Failed);
        assert!(
            !session.failure_detail.contains('\n'),
            "a newline would draw the log tail straight down over the body"
        );
        assert!(session.failure_detail.contains("RuntimeError"));
        assert!(session.log_path.ends_with("analysis.log"));
        // Nothing the probe names exists, so all three Copy buttons draw
        // disabled rather than offering a path to nothing.
        for artifact in AssistArtifact::ALL {
            assert!(!Path::new(session.artifact_path(artifact)).exists());
        }
    }
}
