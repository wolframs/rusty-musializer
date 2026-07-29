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
use musializer_core::project::analysis_candidate::{AnalysisCandidate, Lanes};
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
use musializer_runtime::process::assist::{
    AssistJob, AssistMode as JobMode, AssistPoll, AssistSpec, StopReason,
};
use musializer_runtime::process::font_import::find_assist_helper;
use musializer_runtime::project_files::sha256_file_hex;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt};

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
            Ok(loaded) => {
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
        assist_ui_state::timeline_height(window.1, metric::HUD_BUTTON_SIZE, layout.required_height)
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
        // The panel's requests travel on the session rather than as
        // `ShellCommand`s, because `ShellCommand` lives in `ui/shell.rs`, which no
        // leaf agent in this fan-out may edit. See Agent J's note.
        let _ = commands;
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
        let boundary = UiRect::new(
            content.x + padding,
            top,
            width,
            available.min(layout.required_height),
        );
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
        let mut clip = d.begin_scissor_mode(
            boundary.x as i32 + 1,
            boundary.y as i32 + 1,
            (boundary.width - 2.0).max(0.0) as i32,
            (boundary.height - 2.0).max(0.0) as i32,
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

        if available.lyrics {
            let uncertain = candidate.uncertain_lyric_count();
            let line = if uncertain == 0 {
                format!(
                    "Lyrics: {} -> {}  |  No timing cues flagged for review",
                    target.map_or(0, |track| track.lyrics.len()),
                    candidate.lyrics().len()
                )
            } else {
                format!(
                    "Lyrics: {} -> {}  |  {uncertain} timing cue{} {} review",
                    target.map_or(0, |track| track.lyrics.len()),
                    candidate.lyrics().len(),
                    if uncertain == 1 { "" } else { "s" },
                    if uncertain == 1 { "needs" } else { "need" }
                )
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
        if !session.candidate_first_lyric.is_empty() {
            widgets::draw_text(
                d,
                font,
                &format!("First staged lyric: {}", session.candidate_first_lyric),
                x,
                line_y,
                13.0,
                color::ui_muted(),
            );
        }

        let apply = UiRect::new(x, action_y + 72.0, 144.0, metric::UI_BUTTON_HEIGHT);
        let discard = UiRect::new(apply.x + apply.width + gap, apply.y, 100.0, apply.height);
        // A staged lyric replacement must never clear an authored draft
        // implicitly. `draft_is_dirty` is `false` until Agent I's editor lands,
        // which is why this reads as unblocked today.
        let draft_conflict = assist_ui_state::candidate_conflicts_with_lyric_draft(
            available.lyrics,
            session.candidate_track == workspace.current_index(),
            false,
        );
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
        if draft_conflict {
            widgets::draw_text(
                d,
                font,
                "Finish the active lyric draft before applying this result.",
                discard.x + discard.width + gap,
                discard.y + 10.0,
                13.0,
                color::accent(),
            );
        } else if target.is_none() {
            widgets::draw_text(
                d,
                font,
                "The target track is no longer available. Discard this result.",
                discard.x + discard.width + gap,
                discard.y + 10.0,
                13.0,
                color::accent(),
            );
        }
        self.assist_artifact_actions(
            d,
            input,
            boundary,
            discard.x + discard.width + gap,
            apply.y,
            gap,
        );
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
}
