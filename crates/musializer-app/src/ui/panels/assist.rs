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

use musializer_core::assist::execution::{self, ContractSnapshot, ExecutionSnapshot};
use musializer_core::project::analysis_bridge;
use musializer_core::project::analysis_candidate::{
    self, AnalysisCandidate, Lanes, LyricReviewEntry, LyricReviewKind, LyricsReview,
};
use musializer_core::project::lyrics::{self, CueOrigin, LyricCue, LyricsDocument};
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
use musializer_core::ui::lyric_lane_edit::LYRIC_MIN_CUE_SECONDS;
use musializer_core::ui::notice::Severity;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::assist::env::SessionCredentials;
use musializer_runtime::assist::plan::{self, ExecutionPlan, PlanInputs};
use musializer_runtime::font::UiFonts;
use musializer_runtime::process::assist::{
    AssistJob, AssistMode as JobMode, AssistPoll, AssistSpec, AuthorizedCredential,
    LocalRuntimeOverrides, StopReason,
};
use musializer_runtime::process::font_import::find_assist_helper;
use musializer_runtime::process::reveal::{self, RevealState};
use musializer_runtime::project_files::sha256_file_hex;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::shell_layout::WorkspaceFrame;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle, Widgets};
use crate::cli::UiPanel;
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
    /// The credential imported from the environment at startup, if any.
    ///
    /// Held here rather than on `Shell` because this is the only thing in the
    /// application that may hand one to a child (§4 E1), and §3's "one owner"
    /// rule is easiest to keep when the owner is the thing that needs it. The
    /// dialog gets the fingerprint and nothing else.
    session: SessionCredentials,
    /// The route graph the **running** job was started with.
    ///
    /// §5 invariant 3: resolved once at Start and never re-resolved. Kept so the
    /// staged candidate and the report can name what actually ran without going
    /// back to the settings file, which by then may say something else.
    running_snapshot: Option<ExecutionSnapshot>,
    /// The route graph that produced the staged candidate, read back from the
    /// manifest so its model ids are the observed ones (§6).
    staged_snapshot: Option<ExecutionSnapshot>,
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
            session: SessionCredentials::empty(),
            running_snapshot: None,
            staged_snapshot: None,
        }
    }

    /// Hands over the startup credential import.
    ///
    /// Takes ownership: after this call `main` holds no copy, which is the whole
    /// of §3's "one owner, no `Clone`" rule expressed as a move. It is the only
    /// route by which a key reaches a child, and even then only through
    /// [`AuthorizedCredential`], which a local-only job cannot construct.
    pub fn set_session_credentials(&mut self, session: SessionCredentials) {
        self.session = session;
    }

    /// The fingerprint of the session credential, for the AI settings dialog.
    /// Never the key.
    #[must_use]
    pub fn session_fingerprint(&self) -> Option<String> {
        self.session.openrouter_fingerprint()
    }

    /// The route graph a running job was started with, or `None` when nothing is
    /// running.
    #[must_use]
    pub fn running_snapshot(&self) -> Option<&ExecutionSnapshot> {
        self.running_snapshot.as_ref()
    }

    /// The route graph that produced the **staged** result, with the model ids
    /// the helper observed (§6).
    ///
    /// It sits here rather than on `AnalysisCandidate` because that type is
    /// `musializer-core/src/project/`, which this tranche does not own. The
    /// candidate and this value are staged and cleared together, so the pair is
    /// as inert as the candidate is; moving the field onto the candidate itself
    /// is a one-line change for whoever next opens that file.
    #[must_use]
    pub fn staged_snapshot(&self) -> Option<&ExecutionSnapshot> {
        self.staged_snapshot.as_ref()
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

        // §5 invariant 3: the route graph is resolved **here**, once, and the
        // result is what provenance records. Pressing Start is the confirmation
        // (§5 invariant 2), so this is the one resolution that may carry
        // `boundary_confirmed`; the panel's own preview above resolves with it
        // false and shows the same routes.
        let has_reference =
            resolve_lyric_reference(workspace.current()).0 != AssistLyricReference::None;
        let session_fingerprint = self.session.openrouter_fingerprint();
        let resolved = plan::resolve(&PlanInputs {
            kind_token: job_mode(mode).argument(),
            has_lyric_reference: has_reference,
            boundary_confirmed: true,
            session_fingerprint: session_fingerprint.as_deref(),
            doctor_report: doctor_report_path().as_deref(),
        });
        // Everything that must be true before a process exists (§5 invariant 4).
        // A missing key and a constraint that leaves no endpoint are refusals,
        // not things to discover from a helper's stderr forty minutes later.
        if let Some(block) = resolved.first_block() {
            let session = &mut workspace.assist;
            session.job_state = AssistJobState::Failed;
            session.failure_detail = block.sentence();
            return vec![AssistNotice::new(
                Severity::Warning,
                "Analysis could not start",
                block.sentence(),
            )];
        }
        let snapshot_path = match plan::write_snapshot(&output_dir, &resolved.snapshot) {
            Ok(path) => path,
            Err(error) => {
                let session = &mut workspace.assist;
                session.job_state = AssistJobState::Failed;
                session.failure_detail = capitalize_sentence(&error.to_string());
                return vec![AssistNotice::new(
                    Severity::Error,
                    "Analysis could not start",
                    session.failure_detail.clone(),
                )];
            }
        };
        report_routing(&format!("start {}", resolved.describe()));

        // The credential, and only where the graph authorizes one. `stored`
        // outlives the spawn because the borrow does; the session copy is the
        // fallback, in the order §3 gives — a persisted key first, then the one
        // imported from the environment for this run.
        let stored = plan::openrouter_secret(&resolved.credential_lookup);
        let secret = stored.as_ref().or_else(|| self.session.openrouter());
        let credential = secret
            .and_then(|secret| AuthorizedCredential::for_snapshot(&resolved.snapshot, secret));

        // The dialog's local-runtime overrides reach the helper as flags, which
        // is what makes them win over an inherited `MUSIALIZER_WHISPER_BIN`
        // rather than race with it.
        let whisper_bin = resolved
            .local_runtimes
            .whisper_bin
            .as_deref()
            .map(Path::new);
        let whisper_model = resolved
            .local_runtimes
            .whisper_model
            .as_deref()
            .map(Path::new);
        let align_python = resolved
            .local_runtimes
            .align_python
            .as_deref()
            .map(Path::new);
        let spec = AssistSpec {
            helper: &helper,
            audio: &audio,
            output_dir: &output_dir,
            duration_seconds: duration,
            mode: job_mode(mode),
            lyrics_file: sheet.as_deref(),
            execution_snapshot: Some(&snapshot_path),
            credential,
            local_runtimes: LocalRuntimeOverrides {
                whisper_bin,
                whisper_model,
                align_python,
            },
        };
        match AssistJob::start(&spec, self.nonce) {
            Ok(job) => {
                self.nonce = job.artifacts().nonce;
                self.running_snapshot = Some(resolved.snapshot.clone());
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
                // §6: the staged result exposes the graph that produced it, with
                // the model ids the helper **observed** rather than the ones it
                // was asked for. Read back from the manifest for exactly that
                // reason — `running_snapshot` holds what was resolved, and the
                // difference between the two is the whole point of the field.
                self.staged_snapshot = staged_execution_snapshot(&output_dir)
                    .or_else(|| self.running_snapshot.clone());
                report_routing(&format!(
                    "staged mode={} contracts={} observed={}",
                    mode.argument(),
                    self.staged_snapshot.as_ref().map_or_else(
                        || "none".to_string(),
                        |snapshot| snapshot
                            .contracts
                            .iter()
                            .map(ContractSnapshot::compact)
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    staged_execution_snapshot(&output_dir).is_some(),
                ));
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
    /// Index of the first named row on screen (LX1-f).
    ///
    /// Zero for a list that fits, which is every capture taken before the list
    /// could scroll. Reported because "rows_drawn=3" over a 24-line list is only
    /// half an answer once the three can be any three of them.
    pub first: usize,
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
    // Bound as a pair rather than through `and_then`, so the parked counts read
    // the same candidate the review came from without an `expect` in a drawing
    // path to prove it.
    let Some((candidate, review)) =
        candidate.and_then(|candidate| Some((candidate, candidate.lyrics_review()?)))
    else {
        return "absent (this run left no lyrics-lane LT1 review artifact)".to_string();
    };
    let named: Vec<String> = review
        .entries
        .iter()
        .map(LyricReviewEntry::describe)
        .collect();
    let parked = parked_summary(candidate);
    format!(
        "unresolved={} flagged={} listed={} rows_drawn={} tail={} first={} omitted={} \
         parked={}/{}/{} counts={} manifest={}/{} policy={} | {}",
        review.unresolved,
        review.flagged,
        review.entries.len(),
        drawn.rows,
        if drawn.tail { "yes" } else { "no" },
        drawn.first,
        review.omitted,
        // parked / placeable / unresolved (LX1-f). Three numbers because the
        // gaps between them are the answer: parked < placeable is a capacity
        // refusal, placeable < unresolved is lines with no proposed time.
        parked.parked,
        parked.parked + parked.refused,
        parked.unresolved,
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

thread_local! {
    /// The last `assist review rows:` line printed. Same rule and same reason as
    /// [`LAST_REVIEW_REPORT`]: a per-frame line is not a report, it is a log.
    static LAST_REVIEW_ROWS: std::cell::RefCell<String> = const {
        std::cell::RefCell::new(String::new())
    };
}

/// Prints where the pressable review rows landed (review LT1-R, R9).
///
/// A hover state nothing can photograph is a hover state nobody reviews, and the
/// gate's pointer has to be parked on a coordinate. Publishing the rectangle the
/// panel actually used means the capture cannot drift silently away from the
/// control it thinks it is hovering — the failure `--ui-probe hover=` was
/// invented to end.
fn report_review_rows(line: &str) {
    LAST_REVIEW_ROWS.with(|last| {
        let mut last = last.borrow_mut();
        if *last != line {
            println!("assist review rows: {line}");
            last.clear();
            last.push_str(line);
        }
    });
}

thread_local! {
    /// The last `assist settings button:` line printed.
    static LAST_SETTINGS_BUTTON: std::cell::RefCell<String> = const {
        std::cell::RefCell::new(String::new())
    };
}

// ---------------------------------------------------------------------------
// Tranche P4: the resolved route graph.
//
// The confirmation step is where a user decides, so it is where the routes have
// to be legible: which model, what stays here, what leaves and under which
// policy. Resolving that reads `assist.json`, the `0600` credentials file and
// the catalog cache, which is four `stat`s and a parse — the per-frame syscall
// this file already refuses for the helper probe.
//
// So it is resolved once and cached against a key that changes exactly when the
// answer can: the workflow, whether an authored sheet was found, and whether the
// AI settings dialog is open (closing it is the only way a user can have changed
// the settings without leaving this panel). No clock, no file watch, and no stat
// per frame.
//
// `Shell`'s own fields are not this file's to add to, which is why this is a
// `thread_local!` beside the report caches rather than a member.
// ---------------------------------------------------------------------------

thread_local! {
    /// `(cache key, plan)` for the confirmation body.
    static CONFIRMATION_PLAN: std::cell::RefCell<Option<(String, ExecutionPlan)>> = const {
        std::cell::RefCell::new(None)
    };

    /// The last `assist routing:` line printed.
    static LAST_ROUTING_REPORT: std::cell::RefCell<String> = const {
        std::cell::RefCell::new(String::new())
    };
}

/// Publishes the resolved graph's own evidence.
///
/// A capture of the confirmation shows six short rows and cannot tell a resolved
/// `local-proc/whisper.cpp` from a fallback the panel invented, so the line
/// carries the routes as tokens, whether anything leaves the machine, where the
/// credential came from and what is blocking. The gate asserts against this
/// rather than against pixel positions.
fn report_routing(line: &str) {
    LAST_ROUTING_REPORT.with(|last| {
        let mut last = last.borrow_mut();
        if *last != line {
            println!("assist routing:  {line}");
            last.clear();
            last.push_str(line);
        }
    });
}

/// The execution snapshot a finished job's manifest embedded, if any.
///
/// Read from `assist-manifest.json` rather than from the `assist-execution.json`
/// this side wrote, because the two differ on purpose: the file is what was
/// **resolved**, and the manifest's copy carries the model ids the helper
/// **observed** (§6). Bounded and schema-checked like every other document this
/// panel reads from a job folder.
fn staged_execution_snapshot(output_dir: &str) -> Option<ExecutionSnapshot> {
    if output_dir.is_empty() {
        return None;
    }
    let bytes = read_bounded(&Path::new(output_dir).join(ASSIST_MANIFEST_NAME))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let embedded = manifest.get("execution_snapshot")?;
    serde_json::from_value(embedded.clone()).ok()
}

/// A doctor report to read runtime identity from, when one has been taken.
///
/// There is no persisted doctor report in an ordinary run — nothing writes one —
/// so `runtime_version` and `model_sha256` are `null` in the snapshot unless the
/// dialog's own probe seam names a file. That is the honest answer: a version
/// nobody measured is not a version, and §6 is explicit that a snapshot lacking
/// exact local model identity is not benchmark evidence.
fn doctor_report_path() -> Option<PathBuf> {
    std::env::var_os(crate::ui::assist_settings::PROBE_DOCTOR_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The route graph the confirmation is showing, resolved at most once per change
/// of the cache key above.
fn confirmation_plan(
    mode: AssistMode,
    has_lyric_reference: bool,
    settings_open: bool,
    session_fingerprint: Option<&str>,
) -> ExecutionPlan {
    let key = format!(
        "{}|{has_lyric_reference}|{settings_open}|{}",
        job_mode(mode).argument(),
        session_fingerprint.unwrap_or("-"),
    );
    CONFIRMATION_PLAN.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((cached, resolved)) = slot.as_ref() {
            if cached == &key {
                return resolved.clone();
            }
        }
        // `boundary_confirmed` is false here on purpose: this is what the job
        // *would* use, and the user has not agreed to it yet. Pressing Start is
        // the confirmation, and `AssistController::start` resolves again with it
        // true — which is also the resolution that becomes provenance (§5
        // invariant 3).
        let resolved = plan::resolve(&PlanInputs {
            kind_token: job_mode(mode).argument(),
            has_lyric_reference,
            boundary_confirmed: false,
            session_fingerprint,
            doctor_report: doctor_report_path().as_deref(),
        });
        report_routing(&format!("confirm {}", resolved.describe()));
        *slot = Some((key, resolved.clone()));
        resolved
    })
}

/// The cached plan without resolving one.
///
/// `Shell::assist_timeline_height` runs *before* the panel draws, and it has the
/// session but not the track — so it cannot ask whether an authored sheet
/// exists, which is one of the inputs. It reads the cache instead. The
/// consequence is exact and bounded: on the single frame a confirmation first
/// arms, the strip is sized without the route block and the block is clipped;
/// every frame after it is the right height. Sizing it a frame late is better
/// than resolving the graph from a function that would have to guess one of its
/// inputs.
fn cached_confirmation_plan() -> Option<ExecutionPlan> {
    CONFIRMATION_PLAN.with(|slot| slot.borrow().as_ref().map(|(_, resolved)| resolved.clone()))
}

/// Publishes the AI settings entry point's own evidence (tranche AP3).
///
/// The button has to be present in **every** panel state, and a capture cannot
/// tell "the button is there" from "the heading row happens to be blank there".
/// Naming the body it was drawn under, the panel width it had, and whether the
/// icon face was available is what makes the six-state sweep an assertion
/// instead of six pictures somebody looked at. The width is there because it is
/// the one number that would change if a later layout squeezed the heading row.
fn report_settings_button(line: &str) {
    LAST_SETTINGS_BUTTON.with(|last| {
        let mut last = last.borrow_mut();
        if *last != line {
            println!("assist settings button: {line}");
            last.clear();
            last.push_str(line);
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
    REVIEW_SCROLL.with(|first| first.set(0));
    let attached =
        read_lyrics_review(output_dir).is_some_and(|review| candidate.attach_lyrics_review(review));
    if !attached {
        report_review(describe_review(None, ReviewDraw::default()));
        return;
    }
    // LX1-f. Done here, at the one seam both the real staging path and the probe
    // go through, rather than in `apply_candidate_to_track`. The panel's counts
    // must not lie about what Apply will do, and the only way to guarantee that
    // is for the staged lane the panel counts to be the lane Apply publishes —
    // two insertion points would be two chances to drift.
    park_unresolved_proposals(candidate);
}

// ---------------------------------------------------------------------------
// LX1-f: the proposals a review row cannot reach.
//
// The complaint this answers, in the operator's words: "lyrics between
// 00:41..01:18 are just nowhere to be found for editing in the timeline (which
// is the most comfortable way to edit mistimed lyrics in the UI)."
//
// They were nowhere because a line the localizer could not place has a *coarse
// proposal window* and nothing carried it past the review list. The bridge could
// not: its grammar is cues, and widening it to carry non-cues would move the
// helper's "cues plus unresolved account for every authored line" invariant onto
// both sides of the boundary. That reasoning was sound and is unchanged — what
// was wrong was accepting its consequence.
//
// So the proposal is parked in the lane as a `CueOrigin::Potential` cue, which is
// safe for exactly one reason: `LyricsDocument::at_time` skips it, so it cannot
// reach a preview frame or an export, and `cue_shadow`/`shadowed_cues` ignore it
// in both directions. It is a handle to drag, not content. The model promotes it
// to `UserApplied` the moment anybody edits it, so dragging one into place is
// what makes it real.
// ---------------------------------------------------------------------------

/// The window given to a proposal that has a start and no end.
///
/// Three seconds, and the alternative worth naming is the one that looks more
/// principled and is wrong: [`LYRIC_MIN_CUE_SECONDS`] is 0.02 s, which is what
/// the model will accept, and a 0.02 s cue at any usable timeline zoom is under
/// one pixel wide. The whole point of parking a proposal is to give the user
/// something to *grab*, so a window nobody can hit with a mouse fails the
/// feature at its only job. Three seconds is a sung lyric line's own order of
/// magnitude, so the parked block reads as a line rather than as a tick, and it
/// is short enough that two consecutive proposals do not swallow each other.
///
/// It is not a placement and never pretends to be: the row says "proposed", the
/// lane draws it in the `Potential` colour, and nothing displays it.
pub const PROPOSAL_DEFAULT_SECONDS: f64 = 3.0;

/// How close to the end of the staged lane a proposal may start.
///
/// The same 0.25 s [`load_candidate`] allows between the bridge's duration and
/// the decoder's, and it is here for that exact reason: `apply_candidate_to_track`
/// re-times the lane onto the **decoded** duration, and `normalize_duration`
/// rejects the whole document if any cue starts at or after it. A proposal parked
/// inside the padding tail would therefore turn a working Apply into a failed
/// one — a coarse guess breaking the placements around it, which is the worst
/// possible trade.
const PROPOSAL_TAIL_GUARD_SECONDS: f64 = 0.25;

/// What happened to a run's unresolved lines (LX1-f).
///
/// Four numbers rather than "how many were parked", because they answer four
/// different questions and collapsing them is how a panel starts lying:
/// `unresolved` is what the run could not place, `parked` is what the lane now
/// carries, `unplaceable` is the lines that have no honest window at all, and
/// `refused` is the lines that had one and did not fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ParkedProposals {
    /// Review rows whose line never became a cue.
    pub unresolved: usize,
    /// Proposals now in the staged lane as `Potential` cues.
    pub parked: usize,
    /// Unresolved lines with no proposed time this side can honour. These stay
    /// review-only, and the panel still names them — a line that vanished from
    /// *both* surfaces would be worse than the state this tranche replaces.
    pub unplaceable: usize,
    /// Proposals that had a usable window and were refused as a whole, because
    /// the lane plus the proposals would exceed [`lyrics::CUE_CAPACITY`].
    pub refused: usize,
}

impl ParkedProposals {
    /// The sentence the candidate body adds to its lyrics count.
    ///
    /// Empty when there is nothing to say, so a run with no unresolved lines
    /// reads exactly as it did before this tranche.
    ///
    /// It names the double count explicitly. A parked proposal is *both* a cue
    /// in the "Lyrics: 40 -> 52" arithmetic and a row in the list below, and a
    /// reader who is not told that will read 52 placements and 12 problems as 64
    /// things — so the words "also listed below" are load-bearing, not padding.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.unresolved == 0 {
            return String::new();
        }
        if self.refused > 0 {
            return format!(
                "{} proposal{} could not be parked: the lane is at its {} cue limit",
                self.refused,
                if self.refused == 1 { "" } else { "s" },
                lyrics::CUE_CAPACITY
            );
        }
        if self.parked == 0 {
            return format!(
                "{} unresolved line{} proposed no usable time, so {} listed here only",
                self.unplaceable,
                if self.unplaceable == 1 { "" } else { "s" },
                if self.unplaceable == 1 {
                    "it is"
                } else {
                    "they are"
                }
            );
        }
        let mut text = format!(
            "{} parked on the timeline as potential cues (also listed below)",
            self.parked
        );
        if self.unplaceable > 0 {
            text.push_str(&format!(
                "; {} proposed no time and {} listed here only",
                self.unplaceable,
                if self.unplaceable == 1 { "is" } else { "are" }
            ));
        }
        text
    }
}

/// The window a proposal is parked at, or `None` when there is no honest one.
///
/// Three refusals, and each of them is a line that stays review-only rather than
/// being parked somewhere invented:
///
/// - **No proposed start.** The localizer abstained without even a coarse view,
///   so every second of the track is equally likely. Parking it at zero would put
///   a block over the intro that means nothing.
/// - **A start outside the staged lane.** A coarse view can propose past the end
///   of the audio; clamping that to the last three seconds would claim a position
///   the proposal never made. The tail guard is [`PROPOSAL_TAIL_GUARD_SECONDS`].
/// - **Text the lyric model will not hold** — empty, over 511 bytes, or carrying
///   a control character. Checked here rather than discovered by a failing
///   `insert`, because `park_potential_cues` is all-or-nothing and one bad line
///   would otherwise refuse the whole run's proposals.
fn proposal_window(entry: &LyricReviewEntry, duration_seconds: f64) -> Option<(f64, f64)> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    if lyrics::validate_text(entry.text.trim()).is_err() {
        return None;
    }
    let start = entry
        .start_seconds
        .filter(|start| start.is_finite() && *start >= 0.0)?;
    let latest = duration_seconds - PROPOSAL_TAIL_GUARD_SECONDS - LYRIC_MIN_CUE_SECONDS;
    if start > latest {
        return None;
    }
    // The helper's own end when it gave one and it is after the start; otherwise
    // the default window. A `coarse_end_seconds` at or before the start is a
    // damaged record, not a zero-length line.
    let end = match entry.end_seconds {
        Some(end) if end.is_finite() && end > start => end,
        _ => start + PROPOSAL_DEFAULT_SECONDS,
    };
    let end = end
        .min(duration_seconds - PROPOSAL_TAIL_GUARD_SECONDS)
        .max(start + LYRIC_MIN_CUE_SECONDS);
    Some((start, end))
}

/// The proposals a candidate's review offers, in review order.
fn proposal_cues(candidate: &AnalysisCandidate) -> Vec<LyricCue> {
    let Some(review) = candidate.lyrics_review() else {
        return Vec::new();
    };
    let duration = candidate.lyrics().duration_seconds();
    review
        .entries
        .iter()
        .filter(|entry| entry.kind.is_unresolved())
        .filter_map(|entry| {
            let (start_seconds, end_seconds) = proposal_window(entry, duration)?;
            Some(LyricCue {
                id: 0,
                start_seconds,
                end_seconds,
                text: entry.text.trim().to_string(),
                origin: CueOrigin::Potential,
            })
        })
        .collect()
}

/// Parks every placeable proposal in the staged lane, or none of them.
fn park_unresolved_proposals(candidate: &mut AnalysisCandidate) -> ParkedProposals {
    let cues = proposal_cues(candidate);
    if !cues.is_empty() {
        candidate.park_potential_cues(&cues);
    }
    parked_summary(candidate)
}

/// What the lane and the review together say about the run's unresolved lines.
///
/// Derived from the candidate rather than remembered from the parking call, so
/// the panel, the report line and a test all read the same document and cannot
/// disagree about it after a clone.
fn parked_summary(candidate: &AnalysisCandidate) -> ParkedProposals {
    let Some(review) = candidate.lyrics_review() else {
        return ParkedProposals::default();
    };
    let unresolved = review
        .entries
        .iter()
        .filter(|entry| entry.kind.is_unresolved())
        .count();
    let placeable = proposal_cues(candidate).len();
    let parked = candidate.potential_cue_count();
    ParkedProposals {
        unresolved,
        parked,
        unplaceable: unresolved.saturating_sub(placeable),
        // All or nothing, so the only way a placeable proposal is not in the lane
        // is a whole-run refusal.
        refused: placeable.saturating_sub(parked),
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
            // Unchanged from the one-word grammar this replaced, except that
            // the workflow is now selectable (tranche P4). It has to be: the
            // resolved route graph the confirmation draws is *different* for a
            // local `lyrics` job and a remote `mimo` one, and photographing only
            // the default would leave the branch that sends audio off the
            // machine — the one the whole confirmation exists for — with no
            // capture at all. Gated on the variable being set, so every existing
            // `assist=confirm` capture keeps the mode it had.
            if std::env::var_os(PROBE_LANES_VARIABLE).is_some() {
                workspace.assist.select_mode(probe_mode());
            }
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

// ---------------------------------------------------------------------------
// Tranche P4: the resolved-graph block on the confirmation.
//
// Extra height on this side of the boundary, exactly as the LT1 review list is
// and for the same reason: `assist_ui_state::ui_layout` reserves 84 px for the
// confirmation body and is pinned against the frozen C by
// `tools/differential_assist_ui.sh`, so this cannot be squeezed into it. Both
// places that size the panel add the same block, and it is drawn **below** the
// Start/Cancel row so a window too short for the whole panel loses the routes
// and keeps the decision.
// ---------------------------------------------------------------------------

/// The 12 px text the route rows are drawn in. Same size as the review list, so
/// the two blocks read as one vocabulary.
const ROUTE_FONT_SIZE: f32 = 12.0;

/// Row pitch for the route list.
const ROUTE_ROW_HEIGHT: f32 = 15.0;

/// Where the route block starts, measured from the confirmation body's top:
/// after the 36 px Start/Cancel row at +48 and its 10 px of slack. With a lyric
/// reference row present the buttons move down by the same amount the reference
/// row costs, and [`route_block_offset`] adds it.
const ROUTE_BLOCK_OFFSET: f32 = 94.0;

/// How far below the body top the route block starts, given the layout.
fn route_block_offset(layout: &AssistUiLayout) -> f32 {
    if layout.reference_y > 0.0 {
        ROUTE_BLOCK_OFFSET + (layout.reference_y - layout.content_y) - 8.0
    } else {
        ROUTE_BLOCK_OFFSET
    }
}

/// The height the route block adds, or 0 when there is nothing to draw it for.
///
/// Zero for every body except the confirmation, which is what keeps every
/// pre-P4 capture's geometry where it was.
fn route_block_height(content: AssistPanelContent, plan: Option<&ExecutionPlan>) -> f32 {
    if content != AssistPanelContent::Confirmation {
        return 0.0;
    }
    let Some(plan) = plan else {
        return 0.0;
    };
    // One heading, one row per composed contract, one summary sentence, and one
    // more when there is something extra to say: a downgraded `ask` policy, a
    // settings file that would not load, or a refusal.
    18.0 + (plan.snapshot.contracts.len() + 1 + route_notes(plan).len()) as f32 * ROUTE_ROW_HEIGHT
        + 4.0
}

/// The sentences below the route rows, in the order they matter.
///
/// The first is always present and is the one §5 lets the panel state as a fact
/// rather than a promise: **no applied fallback in this graph can raise a
/// boundary**, measured over the graph rather than asserted. The rest appear
/// only when there is something extra to say, and each is a thing the user would
/// otherwise have to discover from a failed run.
fn route_notes(plan: &ExecutionPlan) -> Vec<(String, raylib::prelude::Color)> {
    let mut notes = Vec::new();
    if let Some(block) = plan.first_block() {
        notes.push((block.sentence(), color::ui_danger()));
    }
    if let Some(error) = &plan.settings_error {
        notes.push((
            format!("The AI settings file could not be read ({error}); the built-in recommended routes are in use."),
            color::ui_warning(),
        ));
    }
    if !plan.ask_resolved_to_none.is_empty() {
        notes.push((
            format!(
                "Fallback \u{201c}ask\u{201d} on {} is applied as \u{201c}none\u{201d}: this build cannot pause a running job for an answer, so a failed route fails its task.",
                plan.ask_resolved_to_none
                    .iter()
                    .map(|contract| contract.token())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            color::ui_warning(),
        ));
    }
    notes
}

/// One route row's text, in the grammar the settings dialog's dry-run summary
/// uses so the two surfaces cannot describe the same route differently.
fn route_row(entry: &ContractSnapshot) -> String {
    let boundary = match entry.boundary_applied.rank() {
        0 => "stays on this machine",
        1 => "text leaves this machine",
        _ => "audio leaves this machine",
    };
    format!(
        "{}  \u{00b7}  {}  \u{00b7}  {boundary}  \u{00b7}  fallback {}",
        entry.contract.human_label(),
        if entry.model_id.is_empty() {
            entry.runtime_id.clone()
        } else {
            entry.model_id.clone()
        },
        entry.fallback_policy.token(),
    )
}

/// Draws the resolved graph under the Start/Cancel row.
///
/// Terse on purpose: one heading that says whether anything leaves the machine,
/// one row per composed task naming its model and its boundary, and the §5
/// invariant stated as the measurement it is. A user deciding whether to press
/// Start needs those four facts and nothing else — the whole matrix, the
/// suitability overlay and the constraint editors are one button away on the
/// AI settings dialog, which the heading row already carries.
fn draw_route_graph(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    boundary: UiRect,
    layout: &AssistUiLayout,
    padding: f32,
    plan: &ExecutionPlan,
) {
    let x = boundary.x + padding;
    let mut y = boundary.y + layout.content_y + route_block_offset(layout);
    let width = (boundary.width - padding * 2.0).max(0.0);
    let snapshot = &plan.snapshot;

    let leaves = snapshot.sends_audio_off_machine();
    let heading = if leaves {
        format!(
            "Resolved routes \u{2014} audio leaves this machine for {} of {} tasks",
            snapshot
                .contracts
                .iter()
                .filter(|entry| entry.boundary_applied.rank() >= 2)
                .count(),
            snapshot.contracts.len(),
        )
    } else if snapshot.has_remote_route() {
        "Resolved routes \u{2014} derived text leaves this machine; no audio does".to_string()
    } else {
        "Resolved routes \u{2014} nothing leaves this machine".to_string()
    };
    widgets::draw_text(
        d,
        font,
        &heading,
        x,
        y,
        ROUTE_FONT_SIZE,
        if leaves {
            color::ui_warning()
        } else {
            color::ui_muted()
        },
    );
    y += 18.0;

    let measure = |text: &str| widgets::measure(font, text, ROUTE_FONT_SIZE);
    for entry in &snapshot.contracts {
        widgets::draw_text(
            d,
            font,
            &ellipsize(&route_row(entry), width, &measure),
            x,
            y,
            ROUTE_FONT_SIZE,
            if entry.boundary_applied.rank() >= 2 {
                color::ui_warning()
            } else {
                color::ui_ink()
            },
        );
        y += ROUTE_ROW_HEIGHT;
    }

    // The §5 invariant-1 sentence, and it is a measurement rather than a
    // promise: `any_fallback_can_raise_boundary` walks the applied policies and
    // the whole boundary ladder. If it ever answered true this line would say
    // so, which is the only reason it is worth drawing.
    let raises = execution::any_fallback_can_raise_boundary(&snapshot.contracts);
    widgets::draw_text(
        d,
        font,
        &ellipsize(
            if raises {
                "A fallback in this graph could move data to a wider boundary."
            } else {
                "No fallback here can widen a boundary: raising one needs a new decision, and this job takes none."
            },
            width,
            &measure,
        ),
        x,
        y,
        ROUTE_FONT_SIZE,
        if raises {
            color::ui_danger()
        } else {
            color::ui_muted()
        },
    );
    y += ROUTE_ROW_HEIGHT;

    for (note, tint) in route_notes(plan) {
        widgets::draw_text(
            d,
            font,
            &ellipsize(&note, width, &measure),
            x,
            y,
            ROUTE_FONT_SIZE,
            tint,
        );
        y += ROUTE_ROW_HEIGHT;
    }
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
/// Where a flagged line is actually repaired, and how to get there
/// (review LT1-R, R9).
///
/// It shipped as a hint — "Retime cues in the Lyrics panel" — while nothing here
/// navigated. Now the rows *are* the route, so the sentence names the gesture
/// instead: icons and rows both cost discoverability, and this repository pays
/// for it in writing rather than hoping the affordance reads. Deliberately on the
/// heading's own row so it costs no row from the list, and dropped entirely when
/// the panel is too narrow for both — the names are the point, this is the
/// footnote.
const REVIEW_FIX_HINT: &str = "Click a line to open it in Lyrics";

/// Widget index of the first review row, in [`ASSIST_WIDGETS`].
///
/// Clear of the mode buttons (`0..4`), the confirmation and reference controls
/// (`10..21`), the Apply/Discard pair (`30`, `31`), the Copy strip (`40..43`) and
/// the auto-scene toggle (`90`). A colliding id would let one control release
/// another's press.
const REVIEW_ROW_WIDGET_BASE: u32 = 60;

/// The heading row's control for getting to the job folder itself (LX1-f).
///
/// The operator's complaint named this: the list "refers to file, but no
/// comfortable direct file opening". A Copy button that puts a path on the
/// clipboard is not a route to a folder, it is homework, and the artifact strip
/// has had one for as long as the panel has existed.
///
/// "Open folder" rather than "Reveal": `xdg-open` on a directory opens the
/// directory, and "reveal" is the macOS word for selecting a file inside its
/// parent, which is not what happens here.
const REVEAL_LABEL: &str = "Open folder";

/// Sized from the label at 12 px with the padding a text button adds, and short
/// enough that the fix hint still fits beside it at 1280 px.
const REVEAL_WIDTH: f32 = 92.0;

/// Two pixels under the 18 px heading band, so the box cannot touch the first
/// row's hit area.
const REVEAL_HEIGHT: f32 = 16.0;

/// Widget index of the Reveal control, in [`ASSIST_WIDGETS`].
///
/// `50` is the one gap left between the Copy strip (`40..43`) and the review rows
/// (`60..63`). A colliding id would let one control release another's press.
const REVEAL_WIDGET: u32 = 50;

/// Whether this process is a headless probe run (LX1-f).
///
/// The gate's Xvfb *is* a reachable display and its `PATH` has `xdg-open`, so
/// every guard in `reveal` except this one would pass — and a file manager
/// opening during a capture is a process this repository did not ask for, on a
/// display it does not own. Keyed on the probe variables this file already
/// defines rather than on the CLI, whose parsing lives in `cli.rs`.
fn assist_probe_run() -> bool {
    [
        PROBE_ARTIFACT_DIR_VARIABLE,
        PROBE_LANES_VARIABLE,
        PROBE_ACTIVATE_ROW_VARIABLE,
        PROBE_REVIEW_SCROLL_VARIABLE,
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// The cue that carries a review row's line, in the document the Lyrics panel
/// edits (review LT1-R, R9).
///
/// **Text, then time — never time alone.** A row whose line never became a cue
/// must not select a neighbouring one: the whole complaint R9 answers is that a
/// flagged line sends the user hunting, and landing them on somebody else's cue
/// is worse than landing them nowhere. So the match is on the authored text, and
/// the time only breaks a tie between repeated lines — which is exactly the case
/// the localizer abstains on, and therefore the case a wrong guess would be
/// least excusable in.
///
/// Compared trimmed: the helper writes the sheet's line and the editor stores
/// what was applied, and the one difference either side can introduce is
/// surrounding space.
fn review_entry_cue(entry: &LyricReviewEntry, document: &LyricsDocument) -> Option<u64> {
    let wanted = entry.text.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut best: Option<(&musializer_core::project::lyrics::LyricCue, f64)> = None;
    for cue in document.cues() {
        if cue.text.trim() != wanted {
            continue;
        }
        let distance = entry
            .start_seconds
            .map_or(0.0, |start| (cue.start_seconds - start).abs());
        // `is_none_or` is 1.82 and this tree's MSRV is 1.80.
        if best.map_or(true, |(_, previous)| distance < previous) {
            best = Some((cue, distance));
        }
    }
    best.map(|(cue, _)| cue.id)
}

/// What a review row's press did, so a test and a report line can both see it
/// (review LT1-R, R9).
///
/// An enum rather than a `bool` for the reason `BeatUpdate` is one: "nothing was
/// selected" covers three different outcomes here — a line with a cue, a line
/// without one, and a press refused because an unsaved draft is in the way — and
/// collapsing them would hide the case R9 is really about.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReviewNavigation {
    /// The line has a cue: the form is bound to it and the playhead moved to it.
    Cue { id: u64, seconds: f64 },
    /// No cue carries this line. The Lyrics panel is open at the proposed time
    /// (`None` when even the coarse view proposed nothing) and a notice names
    /// the line, because a silent no-op is indistinguishable from a broken row.
    NoCue { seconds: Option<f64> },
    /// An unsaved lyric draft blocks every context change in this interface, and
    /// this row is no exception (review 1.3's guard).
    Refused,
}

impl ReviewNavigation {
    fn describe(&self, entry: &LyricReviewEntry) -> String {
        match self {
            ReviewNavigation::Cue { id, seconds } => format!(
                "line {} -> cue {id} at {} (Lyrics panel)",
                entry.line_number,
                review_clock(*seconds)
            ),
            ReviewNavigation::NoCue {
                seconds: Some(seconds),
            } => format!(
                "line {} -> no cue; Lyrics panel at the proposed {}",
                entry.line_number,
                review_clock(*seconds)
            ),
            ReviewNavigation::NoCue { seconds: None } => format!(
                "line {} -> no cue and no proposed time; Lyrics panel, playhead unchanged",
                entry.line_number
            ),
            ReviewNavigation::Refused => format!(
                "line {} refused: an unsaved lyric draft is open",
                entry.line_number
            ),
        }
    }
}

/// The review list's own `m:ss.s`, so a tooltip and a report line read like the
/// row above them.
///
/// A duplicate of `analysis_candidate`'s private `clock`, deliberately: the
/// alternative is `widgets::format_timestamp`, whose `00:04.000` is the
/// transport's grammar and not this list's, and widening the core's function to
/// `pub` for a tooltip would export a formatting detail as an interface.
fn review_clock(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "?".to_string();
    }
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor();
    let rest = seconds - minutes * 60.0;
    format!("{minutes:.0}:{rest:04.1}")
}

/// The tooltip a review row carries, which has to tell the truth *before* the
/// click rather than after it.
///
/// A row that reads "open this cue" and then opens nothing is the silent no-op
/// R9 complains about, wearing a label. So the text is built from the same
/// resolution the press will perform.
fn review_row_hint(entry: &LyricReviewEntry, cue: Option<u64>) -> String {
    match (cue, entry.start_seconds) {
        (Some(_), _) => format!(
            "Open line {} in the Lyrics panel and select its cue",
            entry.line_number
        ),
        (None, Some(start)) => format!(
            "Open the Lyrics panel at the proposed {} — line {} has no cue yet",
            review_clock(start),
            entry.line_number
        ),
        (None, None) => format!(
            "Open the Lyrics panel — line {} has no cue and no proposed time",
            entry.line_number
        ),
    }
}

/// Row index a probe run presses, one row per process (review LT1-R, R9).
///
/// A headless run has no pointer that can *click*: `--ui-probe hover=` parks one
/// but nothing in the grammar releases a button, and the grammar lives in
/// `cli.rs`, which is not this agent's file. So the post-click frame reaches a
/// capture the same way [`PROBE_ARTIFACT_DIR_VARIABLE`] and
/// [`PROBE_LANES_VARIABLE`] reach one.
///
/// It presses the row through the same [`Shell::open_lyric_review_row`] a real
/// press goes through, so what a capture photographs is the real navigation and
/// not a picture of it.
pub const PROBE_ACTIVATE_ROW_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_ACTIVATE_ROW";

thread_local! {
    /// Whether the probe press named by [`PROBE_ACTIVATE_ROW_VARIABLE`] has been
    /// delivered. Once per process: a press repeated every frame would re-seek
    /// under playback and push one notice per frame.
    static PROBE_ROW_PRESSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Where a probe run parks the review list's scroll position (LX1-f).
///
/// [`PROBE_ACTIVATE_ROW_VARIABLE`]'s reason exactly: a headless run has no wheel
/// any more than it has a click. Without this the scrolled state of the list
/// joins the welcome screen and the three `None` fallbacks on the list of things
/// this repository shipped with nothing able to photograph them.
pub const PROBE_REVIEW_SCROLL_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_REVIEW_SCROLL";

/// The first row a probe run wants on screen.
fn probe_review_scroll() -> Option<usize> {
    std::env::var(PROBE_REVIEW_SCROLL_VARIABLE)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()
}

thread_local! {
    /// Index of the first named review row on screen.
    ///
    /// A `thread_local!` beside the report caches for the reason they are ones:
    /// `Shell`'s own fields are not this file's to add to, and `AssistSession`
    /// lives in `core::ui`, which this agent does not own either. Reset by
    /// [`stage_lyrics_review`], so a new staged result opens at the top rather
    /// than at the position the previous run was left scrolled to.
    static REVIEW_SCROLL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The row a probe run wants pressed, once.
fn probe_activated_row() -> Option<usize> {
    if PROBE_ROW_PRESSED.with(std::cell::Cell::get) {
        return None;
    }
    let row = std::env::var(PROBE_ACTIVATE_ROW_VARIABLE)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())?;
    PROBE_ROW_PRESSED.with(|pressed| pressed.set(true));
    Some(row)
}

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

/// Everything one frame of the review list did (LX1-f).
///
/// A struct rather than a widening tuple: the block now has three separable
/// outcomes — what it drew, which row was pressed, and whether the Reveal
/// control was — and a `(ReviewDraw, Option<usize>, bool)` is exactly the shape
/// a caller gets wrong.
pub(crate) struct ReviewFrame {
    pub drawn: ReviewDraw,
    pub activated: Option<usize>,
    pub reveal: bool,
}

/// The pointer state the review list needs, gathered by the caller.
///
/// `draw_lyrics_review` is called from the one place that has a [`ShellInput`],
/// and taking the two numbers rather than the input keeps this function's
/// arithmetic drivable from a test.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ReviewPointer {
    pub x: f32,
    pub y: f32,
    pub wheel: f32,
}

/// How many named rows a list of `entries` shows in `rows` rows, and where the
/// window starts (LX1-f).
///
/// The scroll offset is clamped here rather than where it is stored, because the
/// list length changes under it: a candidate discarded and another staged leaves
/// a position that was valid for a longer list. Clamping at the point of use
/// makes a stale offset unstateable instead of merely unlikely.
fn review_window(entries: usize, shown: usize, requested_first: usize) -> usize {
    requested_first.min(entries.saturating_sub(shown))
}

/// One frame of the review list, drawn and pressed.
///
/// `document` is the lyric document the *Lyrics panel* edits, not the staged
/// candidate's: the tooltip has to promise what the press will actually do, and
/// what it can do is bind a cue that exists where the user is about to be sent.
#[allow(clippy::too_many_arguments)]
fn draw_lyrics_review(
    d: &mut RaylibDrawHandle<'_>,
    widgets_state: &mut Widgets,
    font: &UiFonts,
    review: &LyricsReview,
    document: Option<&LyricsDocument>,
    pointer: ReviewPointer,
    reveal_state: RevealState,
    x: f32,
    y: f32,
    width: f32,
    clip_bottom: f32,
) -> ReviewFrame {
    let measure = |line: &str| widgets::measure(font, line, REVIEW_FONT_SIZE);
    let mut row_y = y + 18.0;
    // The whole block, heading included, is dropped rather than half-drawn: a
    // heading over a list the scissor ate is the one arrangement that promises
    // names and delivers none.
    let capacity = review_row_capacity(row_y, clip_bottom);
    if capacity == 0 {
        return ReviewFrame {
            drawn: ReviewDraw::default(),
            activated: None,
            reveal: false,
        };
    }
    let (shown, hidden) = review_rows(review, capacity.min(REVIEW_MAX_ROWS));

    let heading = if review.policy.is_empty() {
        "Lines to check".to_string()
    } else {
        format!("Lines to check ({})", review.policy)
    };
    widgets::draw_text(d, font, &heading, x, y, REASON_FONT_SIZE, color::accent());

    // The Reveal control takes the heading row's right edge, and the fix hint
    // gives way to it rather than sharing: the hint is a footnote and the button
    // is the thing the operator asked for by name.
    let reveal_rect = UiRect::new(
        x + width - REVEAL_WIDTH,
        y - 2.0,
        REVEAL_WIDTH,
        REVEAL_HEIGHT,
    );
    let reveal_id = widgets::widget_id(ASSIST_WIDGETS, REVEAL_WIDGET);
    let mut reveal = false;
    let heading_width = widgets::measure(font, &heading, REASON_FONT_SIZE);
    let reveal_drawn = width - heading_width >= REVEAL_WIDTH + 24.0;
    if reveal_drawn {
        // The hit box is registered whether or not the control is pressable, so
        // a refusal can explain itself in a tooltip. `disabled_button` takes no
        // id and therefore cannot carry one, and a greyed box that says nothing
        // is the "blank region is indistinguishable from a broken one" failure
        // wearing a border.
        let state = widgets_state.button(d, reveal_id, reveal_rect);
        if reveal_state.is_ready() {
            if widgets_state
                .text_button(
                    d,
                    font,
                    reveal_id,
                    reveal_rect,
                    REVEAL_LABEL,
                    false,
                    ButtonStyle::Neutral,
                    Some(REVIEW_FONT_SIZE),
                )
                .clicked
            {
                reveal = true;
            }
        } else {
            widgets_state.disabled_button(
                d,
                font,
                reveal_rect,
                REVEAL_LABEL,
                Some(REVIEW_FONT_SIZE),
            );
        }
        widgets_state.hint(d, state, reveal_id, reveal_rect, reveal_state.reason());
    }
    let hint_width = widgets::measure(font, REVIEW_FIX_HINT, REVIEW_FONT_SIZE);
    let hint_right = if reveal_drawn {
        reveal_rect.x - 12.0
    } else {
        x + width
    };
    if hint_right - x - heading_width - hint_width >= 24.0 {
        widgets::draw_text(
            d,
            font,
            REVIEW_FIX_HINT,
            hint_right - hint_width,
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
        return ReviewFrame {
            drawn: ReviewDraw {
                rows: 0,
                tail: false,
                first: 0,
            },
            activated: None,
            reveal,
        };
    }

    // Where the rows landed, so the headless gate can park a pointer on one
    // rather than on a coordinate somebody measured off a screenshot once.
    report_review_rows(&format!(
        "x={x:.0} y={row_y:.0} width={width:.0} pitch={REVIEW_ROW_HEIGHT:.0} \
         rows={shown} hint=\"{REVIEW_FIX_HINT}\""
    ));

    // The scroll window (LX1-f). The wheel is only read when the pointer is over
    // the rows, which is the rule every other list in this interface follows:
    // a wheel that scrolled a list the pointer was nowhere near would fight the
    // panel underneath it.
    let list = UiRect::new(x, row_y - 2.0, width, shown as f32 * REVIEW_ROW_HEIGHT);
    let mut first = REVIEW_SCROLL.with(std::cell::Cell::get);
    if pointer.wheel != 0.0 && list.contains_point(pointer.x, pointer.y) {
        // Whole rows per notch. A 15 px pitch under momentum would land the list
        // between two rows, and a half-drawn name is what the row cap exists to
        // prevent in the first place.
        let step = -(pointer.wheel.round() as i64);
        first = (first as i64 + step).max(0) as usize;
    }
    // A probe run has no wheel, so the capture takes its position from the
    // environment. Applied after the wheel rather than before, because in a
    // probe run there is no wheel to overrule.
    if let Some(parked) = probe_review_scroll() {
        first = parked;
    }
    let first = review_window(review.entries.len(), shown, first);
    REVIEW_SCROLL.with(|scroll| scroll.set(first));

    let mut activated = None;
    for (index, entry) in review.entries.iter().skip(first).take(shown).enumerate() {
        // The row is its own press area, full block width: a list row is the
        // affordance a reader already knows, and a hit box narrower than the ink
        // is a control that misses when you aim at it.
        let row = UiRect::new(x, row_y - 2.0, width, REVIEW_ROW_HEIGHT);
        let id = widgets::widget_id(ASSIST_WIDGETS, REVIEW_ROW_WIDGET_BASE + index as u32);
        let state = widgets_state.button(d, id, row);
        if state.hovered {
            // Drawn behind the text, and the same fill a track row uses, so the
            // interface has one hover vocabulary rather than a private one here.
            widgets::fill(d, row, color::track_button_hoverover());
        }
        let cue = document.and_then(|document| review_entry_cue(entry, document));
        widgets_state.hint(d, state, id, row, &review_row_hint(entry, cue));
        if state.clicked {
            // The **entry's** index, not the screen row's. The widget id is keyed
            // by screen row (the id space between the Copy strip and the
            // auto-scene toggle only has room for the four the panel can draw),
            // so the two diverge the moment the list is scrolled and the caller
            // needs the one that indexes `review.entries`.
            activated = Some(first + index);
        }
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
        //
        // LX1-f changed what it says *after* the count. It used to defer to the
        // job folder — "the full list is in the job folder's lyrics document" —
        // which is the sentence the operator called undesirable, and rightly: it
        // is an interface telling a user to go and read a JSON file. The list
        // scrolls now, so the row names the gesture and the window instead, and
        // the folder is a button on the heading row rather than a suggestion.
        let total = review.total_to_check();
        let named = review.entries.len();
        let tail = if shown == 0 {
            format!(
                "None of the {total} lines to check fit here; open the job folder to read them."
            )
        } else if named > shown {
            format!(
                "Showing {}-{} of {total}; scroll the list, or open the job folder.",
                first + 1,
                first + shown
            )
        } else {
            // Fewer names than flags: the extra lines are ones the document
            // could not name, and no amount of scrolling produces them.
            format!("{shown} of {total} shown; the rest were flagged without a name.")
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
    ReviewFrame {
        drawn: ReviewDraw {
            rows: shown,
            tail: hidden > 0,
            first,
        },
        activated,
        reveal,
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
        // The LT1 review list and the P4 route block are extra height on this
        // side of the boundary; see the notes above `REVIEW_FONT_SIZE` and
        // `ROUTE_FONT_SIZE`. Both are zero for a panel that has neither, so a
        // body without them asks for exactly what it asked for before.
        let required = layout.required_height
            + review_block_height(session.candidate.as_ref())
            + route_block_height(session.panel_content(), cached_confirmation_plan().as_ref());
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
        // Tranche P4: the resolved route graph the confirmation names. Resolved
        // here rather than inside the body so both the box height and the block
        // come from one answer, and cached so this is not four `stat`s a frame.
        let routes = (panel_content == AssistPanelContent::Confirmation).then(|| {
            confirmation_plan(
                session.mode(),
                resolve_lyric_reference(input.workspace.current()).0 != AssistLyricReference::None,
                self.assist_settings.is_open(),
                self.session_credential_fingerprint.as_deref(),
            )
        });
        // The box is the height the body needs, not the height the band happens
        // to have. They agree when `Shell::timeline_height` asked for this panel;
        // clamping is what keeps the panel from drawing a half-empty box when it
        // did not — which is exactly what this build looks like until the
        // `timeline_height` seam in Agent J's note is wired.
        let required = layout.required_height
            + review_block_height(session.candidate.as_ref())
            + route_block_height(panel_content, routes.as_ref());
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
        // The **AI settings** entry point (tranche AP3). Drawn from the heading
        // row rather than from a workflow card, and drawn before the body
        // branches, so it is present in every panel state — Ready, Confirmation,
        // Running, Cancelling, Candidate and Empty alike. A control that only
        // exists in one state is a control a user cannot rely on finding.
        //
        // It keeps its text label at every width: the icon is an addition beside
        // the words, never a replacement for them, so `Faces::icons_available`
        // being false costs decoration rather than meaning.
        //
        // There is no narrow-width second header row, and that is a measurement
        // rather than a preference: the narrowest assist panel any supported
        // window produces is 668 px (960 logical with the inspector open, via
        // `WorkspaceFrame::assist_panel_width`), where the heading, the subtitle
        // and a 152 px button all fit with room to spare. A second row would be
        // a branch nothing could reach, photograph or review. What yields
        // instead is the auto-scenes toggle below, which is the right priority:
        // the entry point must be present in every state, and that one need not.
        let settings_width = 152.0f32.min((boundary.width - padding * 2.0).max(0.0));
        let settings_row = UiRect::new(
            boundary.x + boundary.width - padding - settings_width,
            boundary.y + 8.0,
            settings_width,
            30.0,
        );
        if !settings_row.is_empty() {
            let id = widgets::widget_id(ASSIST_WIDGETS, 91);
            let state = self.widgets.text_button(
                &mut clip,
                font,
                id,
                settings_row,
                "AI settings",
                self.assist_settings.is_open(),
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            );
            self.widgets.hint(
                &clip,
                state,
                id,
                settings_row,
                "Routing, local models, Codex, OpenRouter and privacy. Opening it starts nothing.",
            );
            if input.fonts.icons_available() {
                // Drawn beside the label rather than through `icon_button`,
                // which replaces the label with a glyph. The sliders glyph is
                // the one the Tune control already uses for "settings for what
                // you are looking at", so the vocabulary stays one vocabulary.
                clip.draw_text_ex(
                    input.fonts.icons(),
                    &musializer_runtime::font::Icon::Sliders.glyph().to_string(),
                    raylib::prelude::Vector2::new(settings_row.x + 9.0, settings_row.y + 8.0),
                    15.0,
                    0.0,
                    if self.assist_settings.is_open() {
                        color::white()
                    } else {
                        color::ui_muted()
                    },
                );
            }
            if state.clicked {
                let fingerprint = self.session_credential_fingerprint.clone();
                self.assist_settings
                    .open(crate::ui::assist_settings::Section::Routing, fingerprint);
            }
            report_settings_button(&format!(
                "visible=true label=\"AI settings\" body={:?} panel-width={:.0} icons={} rect={:.0},{:.0},{:.0},{:.0}",
                panel_content,
                boundary.width,
                if input.fonts.icons_available() {
                    "on"
                } else {
                    "text-only"
                },
                settings_row.x,
                settings_row.y,
                settings_row.width,
                settings_row.height,
            ));
        }

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
        if boundary.width >= 560.0 + settings_width {
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
                    boundary.x + boundary.width - padding - settings_width - 8.0 - 190.0,
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
            commands,
        );
        if let Some(routes) = routes.as_ref() {
            draw_route_graph(&mut clip, font, boundary, &layout, padding, routes);
        }
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
        commands: &mut Vec<ShellCommand>,
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
                self.assist_candidate_body(d, input, boundary, action_y, padding, gap, commands);
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
    #[allow(
        clippy::too_many_arguments,
        reason = "the review rows emit a Seek, so the body needs the command sink the panel already threads"
    )]
    fn assist_candidate_body(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        action_y: f32,
        padding: f32,
        gap: f32,
        commands: &mut Vec<ShellCommand>,
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
                Some(review) => {
                    // LX1-f. The parked proposals are inside `after`, so without
                    // this clause the count silently includes cues the aligner
                    // explicitly failed to place — and the reader has no way to
                    // tell which part of "0 -> 52" is a placement.
                    let parked = parked_summary(candidate).summary();
                    if parked.is_empty() {
                        format!("Lyrics: {before} -> {after}  |  {}", review.summary())
                    } else {
                        format!(
                            "Lyrics: {before} -> {after}  |  {}  |  {parked}",
                            review.summary()
                        )
                    }
                }
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
            // The document the *Lyrics panel* edits, which is the one a press can
            // send the user into. Not the candidate's staged lane: until Apply
            // runs, that lane's cues do not exist anywhere the user can drag one.
            let document = workspace.current().map(|track| &track.lyrics);
            let mouse = input.ui_scale.mouse(d);
            let pointer = ReviewPointer {
                x: mouse.x,
                y: mouse.y,
                wheel: d.get_mouse_wheel_move(),
            };
            let folder = PathBuf::from(session.artifact_path(AssistArtifact::Folder));
            let reveal_state = reveal::reveal_state(&folder, assist_probe_run());
            let frame = draw_lyrics_review(
                d,
                &mut self.widgets,
                font,
                review,
                document,
                pointer,
                reveal_state,
                x,
                action_y + REVIEW_BLOCK_OFFSET,
                (boundary.width - padding * 2.0).max(0.0),
                // The scissor's own inner edge, which is what actually cuts a
                // row. `boundary.height` is `available.min(required)`, so at a
                // window too short for the panel this is *less* than the block
                // asked for and the list has to fit what it got.
                boundary.y + boundary.height - 1.0,
            );
            report_review(describe_review(Some(candidate), frame.drawn));
            if frame.reveal {
                // The refusal is a whole sentence, and it names the fallback:
                // "Copy folder" is right beside this button and does work with
                // no display, so a machine that cannot open a file manager still
                // has a route to the path.
                let (severity, title, detail) =
                    match reveal::reveal_directory(&folder, assist_probe_run()) {
                        Ok(()) => (
                            Severity::Info,
                            "Opening the job folder",
                            format!("{} was handed to your file manager.", folder.display()),
                        ),
                        Err(error) => (
                            Severity::Warning,
                            "The job folder could not be opened",
                            format!("{error}. Copy folder puts the path on the clipboard instead."),
                        ),
                    };
                self.notify(severity, title, &detail);
            }
            // A probe press only reaches rows the panel drew, so a capture cannot
            // claim to have pressed a row the scissor ate. It is offset by the
            // scroll position for the same reason: the row a capture points at is
            // the row on screen, not the row in the parse.
            let pressed = frame.activated.or_else(|| {
                probe_activated_row()
                    .filter(|row| *row < frame.drawn.rows)
                    .map(|row| frame.drawn.first + row)
            });
            if let Some(entry) = pressed.and_then(|row| review.entries.get(row)) {
                let outcome = self.open_lyric_review_row(entry, workspace, commands);
                println!("assist review nav: {}", outcome.describe(entry));
            }
        }
    }

    /// A flagged review row, pressed: the route from "this line is wrong" to the
    /// panel that fixes it (review LT1-R, R9).
    ///
    /// Three separate things, and each of them is refusable on its own:
    ///
    /// - **The panel switch** goes through [`Shell::panel`] directly, as the
    ///   Export panel's own Close button does. It is guarded by
    ///   [`Shell::lyric_draft_allows_context_change`] first, because every other
    ///   route into the Lyrics panel is — the toolbar button, a track row, an
    ///   Open — and a row that silently discarded a half-typed cue would be the
    ///   one exception.
    /// - **The selection** binds the form to a cue that carries this line, and to
    ///   nothing otherwise. [`Self::lyrics`] is entered on the current slot first:
    ///   the panel calls `enter_track` itself on its next frame, and if that call
    ///   is the *first* one it clears the draft — including the selection this
    ///   press just made.
    /// - **The seek** is a [`ShellCommand::Seek`], because the transport lives in
    ///   `main.rs`. An unresolved line seeks to the coarse proposal, which is a
    ///   guess and is labelled as one on the row; the Lyrics panel is where the
    ///   user listens and places it.
    ///
    /// The caption panes are closed on the way in. Landing on typography would be
    /// a panel switch that did not arrive.
    fn open_lyric_review_row(
        &mut self,
        entry: &LyricReviewEntry,
        workspace: &Workspace,
        commands: &mut Vec<ShellCommand>,
    ) -> ReviewNavigation {
        if !self.lyric_draft_allows_context_change(workspace) {
            return ReviewNavigation::Refused;
        }
        self.panel = UiPanel::Lyrics;
        self.lyrics.style_pane = false;
        self.lyrics.font_pane = false;
        self.lyrics.effects_pane = false;
        self.lyrics.enter_track(workspace.current_index());

        let document = workspace.current().map(|track| &track.lyrics);
        let cue = document
            .and_then(|document| review_entry_cue(entry, document).map(|id| (document, id)));
        if let Some((document, id)) = cue {
            self.lyrics.select_single(document, id);
            // The cue's own start, not the review's: the review records where the
            // job put the line, and the document records where it is now. The
            // user is going to look at the second one.
            let seconds = document
                .find(id)
                .map_or(0.0, |cue| cue.start_seconds)
                .max(0.0);
            commands.push(ShellCommand::Seek(seconds));
            return ReviewNavigation::Cue { id, seconds };
        }

        // No cue, and deliberately no substitute for one. What is left that is
        // honest: the proposed time, and the line's own words — which the user
        // needs, because the Lyrics panel has no row for a line that is not a cue
        // and would otherwise show them an empty list at a plausible timestamp.
        let seconds = entry
            .start_seconds
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        if let Some(seconds) = seconds {
            commands.push(ShellCommand::Seek(seconds));
        }
        self.notify(
            Severity::Info,
            "This line has no cue yet",
            &format!(
                "Line {} \"{}\" is {}. Add it here, or Apply the staged lyrics first.",
                entry.line_number,
                entry.text,
                entry.window()
            ),
        );
        ReviewNavigation::NoCue { seconds }
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
            session: SessionCredentials::empty(),
            running_snapshot: None,
            staged_snapshot: None,
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
                    tail: false,
                    first: 0
                }
            ),
            // `parked=0/0/1`: this fixture's one unresolved line proposes 1:30.6
            // against a 60 s staged lane, so it has no window this side can
            // honour and stays review-only (LX1-f).
            "unresolved=1 flagged=2 listed=2 rows_drawn=2 tail=no first=0 omitted=0 \
             parked=0/0/1 counts=document \
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
            "unresolved=0 flagged=0 listed=0 rows_drawn=0 tail=no first=0 omitted=0 \
             parked=0/0/0 counts=document \
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

    // -----------------------------------------------------------------------
    // Row -> Lyrics panel navigation (review LT1-R, R9).
    //
    // Driven through `Shell::open_lyric_review_row`, which is the whole press
    // minus the rectangle: the panel switch, the cue binding and the emitted
    // `Seek`. A capture can show that a row is hoverable and that the Lyrics
    // panel arrived; only a test can say *which* cue the form ended up bound to,
    // and "some cue" is precisely the wrong answer R9 is about.
    // -----------------------------------------------------------------------

    /// A workspace whose current track carries the two lines `LT1_DOCUMENT`
    /// flags, as cues — the state after Apply, which is when a user is retiming.
    ///
    /// The repeated line is deliberate: `review_entry_cue` breaks a text tie on
    /// time, and a fixture with one occurrence of everything could not tell a
    /// working tie-break from an accidental `first()`.
    fn workspace_with_cues() -> Workspace {
        use musializer_core::project::lyrics::LyricCue;

        let mut workspace = Workspace::new();
        workspace.push(
            Track::new(PathBuf::from("/tmp/lt1r.wav"), 120.0, SceneId::Spectrum, 7).expect("track"),
        );
        let document = &mut workspace.get_mut(0).expect("slot 0").lyrics;
        for (start, text) in [
            (4.0, "we were never meant to stay"),
            (12.0, "and the lights came up anyway"),
            (48.0, "we were never meant to stay"),
        ] {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 2.0,
                    text: text.to_string(),
                    origin: Default::default(),
                })
                .expect("cue");
        }
        workspace
    }

    /// The two named entries `LT1_DOCUMENT` produces: `[0]` is the UNPLACED line
    /// with a coarse proposal, `[1]` the CHECK line that has a cue.
    fn lt1_entries(scratch: &Scratch, name: &str) -> Vec<LyricReviewEntry> {
        let folder = review_job_folder(scratch, name, LT1_MANIFEST, LT1_DOCUMENT);
        read_lyrics_review(&folder.display().to_string())
            .expect("review")
            .entries
    }

    #[test]
    fn a_flagged_row_opens_the_lyrics_panel_on_that_lines_own_cue() {
        let scratch = Scratch::new("nav-cue");
        let entries = lt1_entries(&scratch, "cue");
        let workspace = workspace_with_cues();
        let mut shell = Shell::new();
        // The state a user reaches by browsing typography before the review:
        // landing on the caption panes would be a switch that did not arrive.
        shell.lyrics.style_pane = true;
        shell.lyrics.effects_pane = true;
        let mut commands = Vec::new();

        let entry = &entries[1];
        assert_eq!(entry.kind, LyricReviewKind::Disagreement);
        let outcome = shell.open_lyric_review_row(entry, &workspace, &mut commands);

        let id = workspace.get(0).expect("slot 0").lyrics.cues()[0].id;
        assert_eq!(outcome, ReviewNavigation::Cue { id, seconds: 4.0 });
        assert_eq!(shell.panel, UiPanel::Lyrics);
        assert!(!shell.lyrics.style_pane && !shell.lyrics.effects_pane);
        assert_eq!(shell.lyrics.selected_id, id);
        assert_eq!(commands, vec![ShellCommand::Seek(4.0)]);
        assert_eq!(
            outcome.describe(entry),
            "line 1 -> cue 1 at 0:04.0 (Lyrics panel)"
        );
    }

    /// The selection has to survive the *panel's* own first frame.
    ///
    /// `lyrics_panel` calls `enter_track` before it draws anything, and the first
    /// such call on a fresh editor clears the draft — which is the selection this
    /// press just made. Entering the slot inside the press is what makes the
    /// panel's call a no-op; without it the user arrives at an unbound form and
    /// the row looks like it did nothing.
    #[test]
    fn the_selection_survives_the_lyrics_panels_own_track_binding() {
        let scratch = Scratch::new("nav-survives");
        let entries = lt1_entries(&scratch, "survives");
        let workspace = workspace_with_cues();
        let mut shell = Shell::new();
        let mut commands = Vec::new();

        shell.open_lyric_review_row(&entries[1], &workspace, &mut commands);
        let selected = shell.lyrics.selected_id;
        assert_ne!(selected, 0);
        assert!(
            !shell.lyrics.enter_track(workspace.current_index()),
            "the panel's own binding must find the slot already entered"
        );
        assert_eq!(shell.lyrics.selected_id, selected);
    }

    /// An unresolved line has no cue anywhere, and must not borrow one.
    #[test]
    fn an_unresolved_row_lands_at_the_proposal_and_names_the_line() {
        let scratch = Scratch::new("nav-unresolved");
        let entries = lt1_entries(&scratch, "unresolved");
        let workspace = workspace_with_cues();
        let mut shell = Shell::new();
        let mut commands = Vec::new();

        let entry = &entries[0];
        assert_eq!(entry.kind, LyricReviewKind::Unresolved);
        let outcome = shell.open_lyric_review_row(entry, &workspace, &mut commands);

        assert_eq!(
            outcome,
            ReviewNavigation::NoCue {
                seconds: Some(90.6)
            }
        );
        assert_eq!(shell.panel, UiPanel::Lyrics);
        assert_eq!(
            shell.lyrics.selected_id, 0,
            "no cue carries this line, so nothing may be bound"
        );
        assert_eq!(commands, vec![ShellCommand::Seek(90.6)]);
        // Not a silent no-op: the words the user has to find are on screen.
        assert_eq!(shell.notices.len(), 1);
        let notice = shell.notices.notices()[0].detail.clone();
        assert!(
            notice.contains("hold the note until it breaks")
                && notice.contains("proposed 1:30.6-1:34.2"),
            "the notice must carry the line and its proposed window: {notice}"
        );
    }

    /// The one case with nothing to seek to: the coarse view proposed nothing
    /// either. The panel still opens, and the playhead is left where it was
    /// rather than sent to zero as if that meant something.
    #[test]
    fn a_row_with_no_proposed_time_opens_the_panel_and_seeks_nowhere() {
        let workspace = workspace_with_cues();
        let mut shell = Shell::new();
        let mut commands = Vec::new();
        let entry = LyricReviewEntry {
            kind: LyricReviewKind::Abstained,
            line_number: 31,
            text: "and again, and again".to_string(),
            start_seconds: None,
            end_seconds: None,
            reason: "repeated phrase could not be pinned".to_string(),
            delta_seconds: None,
        };

        let outcome = shell.open_lyric_review_row(&entry, &workspace, &mut commands);
        assert_eq!(outcome, ReviewNavigation::NoCue { seconds: None });
        assert_eq!(shell.panel, UiPanel::Lyrics);
        assert!(commands.is_empty(), "there is no honest second to seek to");
        assert_eq!(
            outcome.describe(&entry),
            "line 31 -> no cue and no proposed time; Lyrics panel, playhead unchanged"
        );
    }

    /// Every other route into the Lyrics panel is guarded by the unsaved-draft
    /// rule (review 1.3). A row that discarded a half-typed cue on the way to
    /// showing you a different one would be the exception that makes the guard
    /// worthless.
    #[test]
    fn a_row_press_is_refused_while_an_unsaved_lyric_draft_is_open() {
        let scratch = Scratch::new("nav-refused");
        let entries = lt1_entries(&scratch, "refused");
        let workspace = workspace_with_cues();
        let document = workspace.get(0).expect("slot 0").lyrics.clone();
        let mut shell = Shell::new();
        shell
            .lyrics
            .open_dirty_draft_for_test(0, &document, document.cues()[1].id);
        let dirty_id = shell.lyrics.selected_id;
        let mut commands = Vec::new();

        let outcome = shell.open_lyric_review_row(&entries[1], &workspace, &mut commands);
        assert_eq!(outcome, ReviewNavigation::Refused);
        assert_eq!(shell.panel, UiPanel::None, "the panel did not switch");
        assert_eq!(shell.lyrics.selected_id, dirty_id, "the draft is untouched");
        assert!(commands.is_empty());
    }

    /// The tie-break, and the thing it must never do: match on time alone.
    #[test]
    fn a_repeated_line_binds_the_nearest_occurrence_and_a_missing_one_binds_nothing() {
        let workspace = workspace_with_cues();
        let document = &workspace.get(0).expect("slot 0").lyrics;
        let ids: Vec<u64> = document.cues().iter().map(|cue| cue.id).collect();

        let repeated = |start: Option<f64>| LyricReviewEntry {
            kind: LyricReviewKind::Disagreement,
            line_number: 1,
            text: "we were never meant to stay".to_string(),
            start_seconds: start,
            end_seconds: start.map(|start| start + 2.0),
            reason: String::new(),
            delta_seconds: None,
        };
        // Two cues carry this text; the review's own window picks between them.
        assert_eq!(
            review_entry_cue(&repeated(Some(5.0)), document),
            Some(ids[0])
        );
        assert_eq!(
            review_entry_cue(&repeated(Some(47.0)), document),
            Some(ids[2])
        );
        // No window at all still binds — the text is the identity, the time is
        // only the tie-break — and takes the first occurrence.
        assert_eq!(review_entry_cue(&repeated(None), document), Some(ids[0]));

        // And a line no cue carries binds nothing, however close a cue is to it.
        let absent = LyricReviewEntry {
            text: "hold the note until it breaks".to_string(),
            ..repeated(Some(4.1))
        };
        assert_eq!(
            review_entry_cue(&absent, document),
            None,
            "a neighbouring cue is not this line's cue"
        );
        // Surrounding space is the one difference either side can introduce.
        let padded = LyricReviewEntry {
            text: "  we were never meant to stay\n".to_string(),
            ..repeated(Some(5.0))
        };
        assert_eq!(review_entry_cue(&padded, document), Some(ids[0]));
    }

    /// The tooltip has to promise what the press will do, or the row is a silent
    /// no-op wearing a label.
    #[test]
    fn the_row_hint_says_which_of_the_three_outcomes_this_row_has() {
        let scratch = Scratch::new("nav-hint");
        let entries = lt1_entries(&scratch, "hint");
        assert_eq!(
            review_row_hint(&entries[1], Some(3)),
            "Open line 1 in the Lyrics panel and select its cue"
        );
        assert_eq!(
            review_row_hint(&entries[1], None),
            "Open the Lyrics panel at the proposed 0:12.0 — line 1 has no cue yet"
        );
        assert_eq!(
            review_row_hint(&entries[0], None),
            "Open the Lyrics panel at the proposed 1:30.6 — line 26 has no cue yet"
        );
        let no_time = LyricReviewEntry {
            start_seconds: None,
            end_seconds: None,
            ..entries[0].clone()
        };
        assert_eq!(
            review_row_hint(&no_time, None),
            "Open the Lyrics panel — line 26 has no cue and no proposed time"
        );
    }

    #[test]
    fn the_probe_row_press_is_delivered_once_and_only_when_asked() {
        assert_eq!(
            PROBE_ACTIVATE_ROW_VARIABLE,
            "MUSIALIZER_ASSIST_PROBE_ACTIVATE_ROW"
        );
        // Unset in this process, so the seam is inert for every other test and
        // for every run that did not ask for it.
        assert_eq!(probe_activated_row(), None);
        assert!(!PROBE_ROW_PRESSED.with(std::cell::Cell::get));
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

    // -----------------------------------------------------------------------
    // Tranche LX1-f: proposals reach the timeline, and the list scrolls.
    // -----------------------------------------------------------------------

    /// The `unresolved[]`/`review_flags[]` pair a proposal-carrying job leaves.
    ///
    /// Windows are inside the probe bridge's 60 s duration on purpose: the
    /// pre-existing `LT1_DOCUMENT` proposes 1:30.6, which is *past the end of the
    /// staged lane*, and the whole point of a second fixture is that the two
    /// cases must not share one.
    const PARKED_DOCUMENT: &str = r#"{"localization_policy":"anchor-block-mms",
        "unresolved":[
            {"reference_line_index":25,"text":"hold the note until it breaks",
             "reason":"no block placement","abstained":false,
             "coarse_start_seconds":41.0,"coarse_end_seconds":44.5},
            {"reference_line_index":26,"text":"an open ended proposal",
             "reason":"no block placement","abstained":false,
             "coarse_start_seconds":50.0,"coarse_end_seconds":null},
            {"reference_line_index":30,"text":"and again, and again",
             "reason":"repeated phrase could not be pinned","abstained":true,
             "coarse_start_seconds":null,"coarse_end_seconds":null}],
        "review_flags":[
            {"reference_line_index":25,"text":"hold the note until it breaks",
             "flag":"unresolved","reason":"no block placement",
             "coarse_start_seconds":41.0},
            {"reference_line_index":26,"text":"an open ended proposal",
             "flag":"unresolved","reason":"no block placement",
             "coarse_start_seconds":50.0},
            {"reference_line_index":30,"text":"and again, and again",
             "flag":"unresolved","reason":"repeated phrase could not be pinned"}]}"#;

    const PARKED_MANIFEST: &str = r#"{"schema_version":"musializer.assist-manifest/v1",
        "mode":"lyrics",
        "result_counts":{"lyrics":2,"lyrics_unresolved":3,"lyrics_review_flags":3},
        "lyric_localization":{"policy":"anchor-block-mms","policy_version":"3"}}"#;

    /// A staged candidate with `PARKED_DOCUMENT`'s review already parked, exactly
    /// as `stage_lyrics_review` leaves one.
    fn parked_candidate(scratch: &Scratch, name: &str) -> AnalysisCandidate {
        let folder = review_job_folder(scratch, name, PARKED_MANIFEST, PARKED_DOCUMENT);
        let mut candidate = probe_candidate(AssistMode::All).expect("probe candidate");
        stage_lyrics_review(&mut candidate, &folder.display().to_string());
        candidate
    }

    #[test]
    fn an_unresolved_line_with_a_proposal_becomes_one_potential_cue_at_that_window() {
        let scratch = Scratch::new("lx1f-parked");
        let candidate = parked_candidate(&scratch, "parked");

        // Two of the three unresolved lines are placeable; the third proposed no
        // time at all. Not deduplicated, not merged.
        let parked: Vec<&LyricCue> = candidate
            .lyrics()
            .cues()
            .iter()
            .filter(|cue| cue.origin == CueOrigin::Potential)
            .collect();
        assert_eq!(parked.len(), 2);
        assert_eq!(parked[0].text, "hold the note until it breaks");
        assert!((parked[0].start_seconds - 41.0).abs() < 1e-9);
        assert!((parked[0].end_seconds - 44.5).abs() < 1e-9);

        // The window policy for a proposal with a start and no end, pinned as a
        // **literal**. Writing `50.0 + PROPOSAL_DEFAULT_SECONDS` here reads like
        // the careful version and is a tautology: it agrees with the constant
        // whatever the constant is. The negative control for this tranche moved
        // the default 3.0 -> 4.0 and this assertion passed, which is the exact
        // "copied from our own output, and would then pass forever" failure
        // `AGENTS.md` warns about. 53.0 is the number, and the separate
        // assertion below is what names why it is that number.
        assert_eq!(parked[1].text, "an open ended proposal");
        assert!((parked[1].start_seconds - 50.0).abs() < 1e-9);
        assert!(
            (parked[1].end_seconds - 53.0).abs() < 1e-9,
            "an open-ended proposal at 0:50 must be parked as 0:50-0:53, got {}",
            parked[1].end_seconds
        );
        assert!(
            (PROPOSAL_DEFAULT_SECONDS - 3.0).abs() < 1e-9,
            "the literal above is 50 + this constant; change both or neither"
        );

        // The two the bridge placed are untouched, and the counts add up.
        assert_eq!(candidate.lyrics().len(), 4);
        assert_eq!(
            parked_summary(&candidate),
            ParkedProposals {
                unresolved: 3,
                parked: 2,
                unplaceable: 1,
                refused: 0,
            }
        );
    }

    #[test]
    fn a_line_with_no_proposed_time_parks_nothing_and_is_still_named() {
        let scratch = Scratch::new("lx1f-unplaceable");
        let candidate = parked_candidate(&scratch, "unplaceable");
        let review = candidate.lyrics_review().expect("review");

        // The abstained line has no coarse view, so there is nowhere honest to
        // park it — and it must still be on the screen, because a line that
        // vanished from *both* surfaces would be worse than the state before
        // this tranche.
        let named: Vec<&str> = review
            .entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(named.contains(&"and again, and again"));
        assert!(!candidate
            .lyrics()
            .cues()
            .iter()
            .any(|cue| cue.text == "and again, and again"));
        assert_eq!(parked_summary(&candidate).unplaceable, 1);
        assert!(parked_summary(&candidate)
            .summary()
            .contains("proposed no time"));
    }

    #[test]
    fn a_proposal_outside_the_staged_lane_stays_review_only() {
        // `LT1_DOCUMENT` proposes 1:30.6 against a 60 s staged lane. Clamping
        // that into the track would claim a position the proposal never made,
        // and — worse — a proposal inside the decoder's padding tail turns a
        // working Apply into a failed one, because `normalize_duration` refuses
        // a document with any cue starting at or after the decoded duration.
        let scratch = Scratch::new("lx1f-outside");
        let folder = review_job_folder(&scratch, "outside", LT1_MANIFEST, LT1_DOCUMENT);
        let mut candidate = probe_candidate(AssistMode::All).expect("probe candidate");
        stage_lyrics_review(&mut candidate, &folder.display().to_string());
        assert_eq!(candidate.potential_cue_count(), 0);
        assert_eq!(candidate.lyrics().len(), 2);

        // And the guard is the tail, not the duration: 59.7 is inside a 60 s lane
        // and still refused, 59.5 is the last second that is not.
        let entry = |start: f64| LyricReviewEntry {
            kind: LyricReviewKind::Unresolved,
            line_number: 1,
            text: "x".to_string(),
            start_seconds: Some(start),
            end_seconds: None,
            reason: String::new(),
            delta_seconds: None,
        };
        assert_eq!(proposal_window(&entry(60.1), 60.0), None);
        assert_eq!(proposal_window(&entry(59.9), 60.0), None);
        let (start, end) = proposal_window(&entry(59.0), 60.0).expect("inside the guard");
        assert!((start - 59.0).abs() < 1e-9);
        assert!(
            (end - 59.75).abs() < 1e-9,
            "the window is clipped to the guard, not to the duration: {end}"
        );
    }

    #[test]
    fn the_whole_batch_is_refused_rather_than_half_applied_at_the_cue_limit() {
        // Reachable, not hypothetical: `REVIEW_ENTRY_CAPACITY` is 64 and
        // `CUE_CAPACITY` is 1024, so a lane the helper filled past 960 is exactly
        // where the sum bites. Half a run's proposals is worse than none — the
        // gaps in the lane would stop meaning anything.
        let mut document = String::from(
            "MUSIALIZER_BRIDGE\t1\n\
             AUDIO\t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\t60000\n",
        );
        for index in 0..1000u64 {
            let start = index * 55;
            document.push_str(&format!(
                "LYRIC\t{}\t{start}\t{}\t900\tnone\t{}\n",
                index + 1,
                start + 40,
                b64("a placed line")
            ));
        }
        // A bridge's section lane is mandatory and must reach the end of the
        // audio (`analysis_bridge.rs:505-510`), so one covering section is part
        // of the fixture rather than an oversight.
        document.push_str(&format!(
            "SECTION\t2000\t0\t60000\tspectrum\t500\t{}\n",
            b64("[\"whole\"]")
        ));
        let bridge =
            analysis_bridge::parse(document.as_bytes(), None, None).expect("the fixture parses");
        let mut candidate =
            AnalysisCandidate::prepare(&bridge, Lanes::ALL, 60.0, SCENE_COUNT as u32)
                .expect("prepare");
        assert_eq!(candidate.lyrics().len(), 1000);

        let mut unresolved = String::new();
        let mut flags = String::new();
        for index in 0..64u64 {
            if index > 0 {
                unresolved.push(',');
                flags.push(',');
            }
            let start = 1.0 + index as f64 * 0.5;
            unresolved.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"parked {index}",
                    "reason":"no block placement","abstained":false,
                    "coarse_start_seconds":{start},"coarse_end_seconds":null}}"#
            ));
            flags.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"parked {index}",
                    "flag":"unresolved","coarse_start_seconds":{start}}}"#
            ));
        }
        let scratch = Scratch::new("lx1f-capacity");
        let folder = review_job_folder(
            &scratch,
            "capacity",
            PARKED_MANIFEST,
            &format!(r#"{{"unresolved":[{unresolved}],"review_flags":[{flags}]}}"#),
        );
        let before = candidate.lyrics().clone();
        stage_lyrics_review(&mut candidate, &folder.display().to_string());

        let summary = parked_summary(&candidate);
        assert_eq!(summary.unresolved, 64);
        assert_eq!(summary.refused, 64);
        assert_eq!(summary.parked, 0, "1000 + 64 > 1024, so none of them fit");
        assert_eq!(
            candidate.lyrics(),
            &before,
            "a refused batch must leave the staged lane byte-identical"
        );
        assert!(summary.summary().contains("could not be parked"));
    }

    #[test]
    fn the_staged_preview_counts_what_apply_will_publish() {
        // The panel's "Lyrics: 0 -> N" is read off the staged lane, so the whole
        // reason parking happens at staging rather than at Apply is that these
        // two numbers cannot then disagree.
        let scratch = Scratch::new("lx1f-preview");
        let candidate = parked_candidate(&scratch, "preview");
        let staged = candidate.lyrics().len();

        let mut lyrics = LyricsDocument::new(60.0).expect("document");
        let mut sections = musializer_core::project::scene_switch::SceneSwitchTimeline::new();
        let mut events = musializer_core::project::event_timeline::EventTimeline::new();
        candidate
            .apply(&mut lyrics, &mut sections, &mut events)
            .expect("apply");
        assert_eq!(lyrics.len(), staged);
        assert_eq!(
            lyrics
                .cues()
                .iter()
                .filter(|cue| cue.origin == CueOrigin::Potential)
                .count(),
            2
        );
        // And none of them is a caption: the proposals are handles, not content.
        assert!(lyrics.at_time(42.0).is_none());
        assert!(lyrics.at_time(51.0).is_none());
    }

    #[test]
    fn the_scroll_window_is_clamped_to_the_list_it_is_scrolling() {
        // A stale offset is the failure mode: a 24-row list scrolled to 20, then
        // a 4-row candidate staged under it, would draw rows 20..23 of four.
        assert_eq!(review_window(24, 3, 0), 0);
        assert_eq!(review_window(24, 3, 5), 5);
        assert_eq!(review_window(24, 3, 21), 21);
        assert_eq!(
            review_window(24, 3, 22),
            21,
            "the last window is entries-shown"
        );
        assert_eq!(review_window(24, 3, usize::MAX), 21);
        assert_eq!(review_window(4, 4, 3), 0, "a list that fits never scrolls");
        assert_eq!(review_window(0, 3, 7), 0);
        assert_eq!(review_window(2, 3, 1), 0, "shown may exceed the list");
    }

    #[test]
    fn the_report_line_carries_the_scroll_position_and_the_parked_counts() {
        let scratch = Scratch::new("lx1f-report");
        let candidate = parked_candidate(&scratch, "report");
        let line = describe_review(
            Some(&candidate),
            ReviewDraw {
                rows: 3,
                tail: true,
                first: 4,
            },
        );
        assert!(
            line.starts_with(
                "unresolved=3 flagged=3 listed=3 rows_drawn=3 tail=yes first=4 omitted=0 \
                 parked=2/2/3 counts=document"
            ),
            "got {line}"
        );
        // `listed` is the parse and `rows_drawn` is the screen, and LX1-f must
        // not have collapsed the distinction the previous review restored.
        assert!(line.contains("listed=3 rows_drawn=3"));
    }

    #[test]
    fn the_probe_scroll_seam_is_inert_unless_it_is_asked_for() {
        assert_eq!(
            PROBE_REVIEW_SCROLL_VARIABLE,
            "MUSIALIZER_ASSIST_PROBE_REVIEW_SCROLL"
        );
        assert_eq!(probe_review_scroll(), None);
        // And a probe run never opens a file manager, whatever the display says.
        // The gate's Xvfb is a reachable display with `xdg-open` installed, so
        // this ordering is the only thing between a capture and a stray window.
        assert_eq!(
            reveal::reveal_policy(true, true, true, true),
            RevealState::Probe
        );
        assert!(!RevealState::Probe.is_ready());
    }
}
