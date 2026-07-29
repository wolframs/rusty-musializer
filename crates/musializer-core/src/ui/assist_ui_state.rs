//! Assist panel state including the confirmation step's lyric-sheet row.
//!
//! **Owner: Agent F.** Port of `assist_ui_state.c`/`.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! Everything the Assist panel has to decide before anything is drawn: whether a
//! run may start, which of the six panel bodies is showing, when the job has run
//! out of time, and how tall the panel has to be. Keeping it here is what lets
//! the layout be asserted without a window.
//!
//! Two policies in this file exist because the alternative was measured and found
//! wanting, so they are worth not "simplifying" later:
//!
//! - the panel never claims more certainty than it has. The lyric-reference
//!   summary for "no sheet found" says Whisper will transcribe *unless the file
//!   carries a lyrics tag*, because only ffprobe can answer that
//!   (`assist_ui_state.c:184-189`);
//! - the lyric-reference row is a parameter of the layout, not an assumption.
//!   Reserving height the panel never draws pushes the scene preview down for
//!   nothing, and at the supported 960x640 minimum the preview has only 150 px to
//!   give (`assist_ui_state.h:118-125`).

/// How long a helper run may take before the supervisor times it out
/// (`assist_ui_state.h:8`). Forty minutes: a full lyrics pass is Whisper plus a
/// Codex review, and neither is fast.
pub const ASSIST_JOB_TIMEOUT_SECONDS: f64 = 2400.0;

/// Which helper workflow a run performs (`assist_ui_state.h:10-16`).
///
/// C's `ASSIST_MODE_COUNT` is not a variant here; [`AssistMode::ALL`] carries the
/// count, which also makes every `switch` fallback in the C unreachable rather
/// than merely unreached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssistMode {
    Lyrics,
    Sections,
    Mimo,
    All,
}

impl AssistMode {
    /// Every mode, in the order the buttons are laid out.
    pub const ALL: [AssistMode; 4] = [
        AssistMode::Lyrics,
        AssistMode::Sections,
        AssistMode::Mimo,
        AssistMode::All,
    ];

    /// The button label (`assist_ui_state.c:72-82`).
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            AssistMode::Lyrics => "Timed lyrics",
            AssistMode::Sections => "Scene changes",
            AssistMode::Mimo => "MiMo feelings",
            AssistMode::All => "Full assist",
        }
    }

    /// The mode argument passed to the helper (`assist_ui_state.c:84-94`).
    #[must_use]
    pub fn argument(self) -> &'static str {
        match self {
            AssistMode::Lyrics => "lyrics",
            AssistMode::Sections => "sections",
            AssistMode::Mimo => "mimo",
            AssistMode::All => "all",
        }
    }

    /// The data-boundary badge shown on the button itself
    /// (`assist_ui_state.c:96-106`).
    ///
    /// The badge is on the button rather than only in the confirmation step so
    /// "this one leaves the machine" is visible before the click, not after.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            AssistMode::Lyrics => "LOCAL AUDIO / CODEX TEXT",
            AssistMode::Sections => "LOCAL AUDIO",
            AssistMode::Mimo => "OPENROUTER AUDIO",
            AssistMode::All => "LOCAL + REMOTE",
        }
    }

    /// What the run actually does, in one sentence (`assist_ui_state.c:108-122`).
    #[must_use]
    pub fn workflow(self) -> &'static str {
        match self {
            AssistMode::Lyrics => {
                "Workflow: local Whisper transcription, then Codex timing review."
            }
            AssistMode::Sections => {
                "Workflow: measured local audio analysis proposes scene-change sections."
            }
            AssistMode::Mimo => {
                "Workflow: MiMo describes how the music feels and stages feeling cues."
            }
            AssistMode::All => "Workflow: lyrics, measured scene changes, and MiMo feeling cues.",
        }
    }

    /// Exactly what leaves this computer (`assist_ui_state.c:124-138`).
    #[must_use]
    pub fn data_boundary(self) -> &'static str {
        match self {
            AssistMode::Lyrics => {
                "Audio stays local. Transcript evidence is sent to headless Codex."
            }
            AssistMode::Sections => {
                "Runs locally. Audio and analysis output do not leave this computer."
            }
            AssistMode::Mimo => {
                "Track audio is sent to OpenRouter for MiMo; Zero Data Retention is requested."
            }
            AssistMode::All => {
                "Transcript evidence goes to Codex; track audio goes to OpenRouter MiMo with ZDR requested."
            }
        }
    }

    /// What a validated-but-empty result means for this mode
    /// (`assist_ui_state.c:140-154`).
    ///
    /// An empty validated result is a truthful terminal outcome, so the copy names
    /// the lanes it left alone instead of reading like a failure.
    #[must_use]
    pub fn empty_result(self) -> &'static str {
        match self {
            AssistMode::Lyrics => {
                "Whisper and Codex produced no validated lyric cues. Existing lyrics were left unchanged."
            }
            AssistMode::Sections => {
                "Measured analysis produced no validated scene changes. Existing cues were left unchanged."
            }
            AssistMode::Mimo => {
                "MiMo produced no validated feeling cues. Existing semantic events were left unchanged."
            }
            AssistMode::All => {
                "The completed workflow produced no validated editor changes. Existing content was left unchanged."
            }
        }
    }

    /// Whether this mode aligns authored lyrics at all
    /// (`assist_ui_state.c:169-172`).
    ///
    /// Offering the lyric-reference control on a mode that ignores it would
    /// promise something the run does not do (`assist_ui_state.h:57-58`).
    #[must_use]
    pub fn uses_lyric_reference(self) -> bool {
        matches!(self, AssistMode::Lyrics | AssistMode::All)
    }
}

/// The supervised helper job's lifecycle (`assist_ui_state.h:18-28`).
///
/// The four "…ing" states are the ones where a process is still alive; the
/// terminal outcomes are separate so a poll can never try to cancel a job that
/// has already reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssistJobState {
    Idle,
    Running,
    Cancelling,
    TimingOut,
    Failing,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl AssistJobState {
    /// Every state, for exhaustive headless checks.
    pub const ALL: [AssistJobState; 9] = [
        AssistJobState::Idle,
        AssistJobState::Running,
        AssistJobState::Cancelling,
        AssistJobState::TimingOut,
        AssistJobState::Failing,
        AssistJobState::Succeeded,
        AssistJobState::Failed,
        AssistJobState::Cancelled,
        AssistJobState::TimedOut,
    ];

    /// Whether a process is still alive for this job (`assist_ui_state.c:6-10`).
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(
            self,
            AssistJobState::Running
                | AssistJobState::Cancelling
                | AssistJobState::TimingOut
                | AssistJobState::Failing
        )
    }

    /// Whether the run has exceeded [`ASSIST_JOB_TIMEOUT_SECONDS`]
    /// (`assist_ui_state.c:12-19`).
    ///
    /// Only `Running` has a deadline — a job already being torn down is not timed
    /// out again. Non-finite clocks and a `now` that went backwards are not
    /// evidence of an overrun, so they answer `false`.
    #[must_use]
    pub fn deadline_expired(self, started_at: f64, now: f64) -> bool {
        self == AssistJobState::Running
            && started_at.is_finite()
            && now.is_finite()
            && now >= started_at
            && now - started_at >= ASSIST_JOB_TIMEOUT_SECONDS
    }

    /// Seconds left on the deadline, never negative (`assist_ui_state.c:21-29`).
    #[must_use]
    pub fn deadline_remaining(self, started_at: f64, now: f64) -> f64 {
        if self != AssistJobState::Running
            || !started_at.is_finite()
            || !now.is_finite()
            || now < started_at
        {
            return 0.0;
        }
        let remaining = ASSIST_JOB_TIMEOUT_SECONDS - (now - started_at);
        if remaining > 0.0 {
            remaining
        } else {
            0.0
        }
    }
}

/// Why Start is unavailable, or that it is not (`assist_ui_state.h:30-35`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssistStartBlock {
    Allowed,
    HelperUnavailable,
    JobActive,
    ResultPending,
}

impl AssistStartBlock {
    /// Every block state, for exhaustive headless checks.
    pub const ALL: [AssistStartBlock; 4] = [
        AssistStartBlock::Allowed,
        AssistStartBlock::HelperUnavailable,
        AssistStartBlock::JobActive,
        AssistStartBlock::ResultPending,
    ];

    /// One truthful sentence for the blocked reason, empty when allowed
    /// (`assist_ui_state.c:41-53`).
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            AssistStartBlock::Allowed => "",
            AssistStartBlock::HelperUnavailable => {
                "Assist helper script not found in this installation."
            }
            AssistStartBlock::JobActive => {
                "Cancel the active analysis before starting another workflow."
            }
            AssistStartBlock::ResultPending => {
                "Apply or discard the staged result before starting another workflow."
            }
        }
    }
}

/// Which of the panel bodies is showing (`assist_ui_state.h:37-44`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssistPanelContent {
    Ready,
    Confirmation,
    Running,
    Cancelling,
    Candidate,
    Empty,
}

impl AssistPanelContent {
    /// Every panel body, for exhaustive headless checks.
    pub const ALL: [AssistPanelContent; 6] = [
        AssistPanelContent::Ready,
        AssistPanelContent::Confirmation,
        AssistPanelContent::Running,
        AssistPanelContent::Cancelling,
        AssistPanelContent::Candidate,
        AssistPanelContent::Empty,
    ];
}

/// Where the authored lyric text a lyrics run synchronizes against comes from
/// (`assist_ui_state.h:46-55`).
///
/// The helper's own priority is an override, then a sibling `<stem>.lyrics.txt`,
/// then an unsynchronized tag embedded in the audio. The first two are knowable
/// from here; the third is not, because reading it needs ffprobe, so it is never
/// claimed — only mentioned as still possible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AssistLyricReference {
    #[default]
    None,
    Sibling,
    Chosen,
}

impl AssistLyricReference {
    /// Every reference state, in escalating specificity.
    pub const ALL: [AssistLyricReference; 3] = [
        AssistLyricReference::None,
        AssistLyricReference::Sibling,
        AssistLyricReference::Chosen,
    ];

    /// The confirmation-step sentence describing what will be timed
    /// (`assist_ui_state.c:174-189`).
    ///
    /// The `None` wording is deliberately not "no lyrics will be used": an
    /// unsynchronized tag inside the audio container is still picked up, and only
    /// ffprobe can say whether one is there. Promising transcription outright
    /// would be a guess.
    #[must_use]
    pub fn summary(self) -> &'static str {
        match self {
            AssistLyricReference::Chosen => {
                "Your lyrics will be timed against Whisper and shown verbatim."
            }
            AssistLyricReference::Sibling => {
                "Found beside the audio. It will be timed and shown verbatim."
            }
            AssistLyricReference::None => {
                "Whisper will transcribe the words itself, unless the audio file carries a lyrics tag."
            }
        }
    }
}

/// Whether Start may be pressed, and if not, why (`assist_ui_state.c:31-39`).
///
/// The order is the order of what the user has to do about it: a live job first,
/// then a staged result, and only then the installation problem.
#[must_use]
pub fn start_block(
    helper_available: bool,
    job_state: AssistJobState,
    candidate_pending: bool,
) -> AssistStartBlock {
    if job_state.is_active() {
        return AssistStartBlock::JobActive;
    }
    if candidate_pending {
        return AssistStartBlock::ResultPending;
    }
    if !helper_available {
        return AssistStartBlock::HelperUnavailable;
    }
    AssistStartBlock::Allowed
}

/// Which panel body to draw (`assist_ui_state.c:55-70`).
///
/// A staged candidate outranks everything, including a job that is somehow
/// running again: the thing awaiting a decision is what the panel is for. A
/// terminal state with nothing staged is the empty-result body, not the ready
/// body, because "it finished and changed nothing" is information.
#[must_use]
pub fn panel_content(
    job_state: AssistJobState,
    confirmation_pending: bool,
    candidate_pending: bool,
) -> AssistPanelContent {
    if candidate_pending {
        return AssistPanelContent::Candidate;
    }
    if matches!(
        job_state,
        AssistJobState::Cancelling | AssistJobState::TimingOut | AssistJobState::Failing
    ) {
        return AssistPanelContent::Cancelling;
    }
    if job_state == AssistJobState::Running {
        return AssistPanelContent::Running;
    }
    if confirmation_pending {
        return AssistPanelContent::Confirmation;
    }
    if matches!(
        job_state,
        AssistJobState::Succeeded
            | AssistJobState::Failed
            | AssistJobState::Cancelled
            | AssistJobState::TimedOut
    ) {
        return AssistPanelContent::Empty;
    }
    AssistPanelContent::Ready
}

/// Whether a completed helper produced anything applyable
/// (`assist_ui_state.c:156-160`).
///
/// Applyable only when it produced at least one lane the selected workflow was
/// authorized to replace. Empty validated results are a truthful terminal
/// outcome, not a staged no-op (`assist_ui_state.h:105-107`).
#[must_use]
pub fn result_has_changes(authorized_lanes: u32, available_lanes: u32) -> bool {
    (authorized_lanes & available_lanes) != 0
}

/// Whether applying a staged candidate would silently discard authored lyrics
/// (`assist_ui_state.c:162-167`).
///
/// A staged lyric replacement must never clear an authored draft implicitly
/// (`assist_ui_state.h:111`).
#[must_use]
pub fn candidate_conflicts_with_lyric_draft(
    replaces_lyrics: bool,
    targets_active_track: bool,
    draft_is_dirty: bool,
) -> bool {
    replaces_lyrics && targets_active_track && draft_is_dirty
}

/// The lyric sheet the helper looks for beside the audio:
/// `<directory>/<stem>.lyrics.txt` (`assist_ui_state.c:191-217`).
///
/// The stem rule matches Python `pathlib`'s, because the helper is Python and
/// disagreeing here would mean the panel says "found" about a file the run never
/// opens: only the **final** extension is stripped, a leading dot is part of the
/// name rather than an extension, and a dot in a parent directory is not an
/// extension at all. That last rule is why the scan starts after the final `/` or
/// `\` — both, because a Windows-style path is still a string we may be handed.
///
/// The C loop bound `i > name_start + 1` is the leading-dot rule: it stops before
/// index `name_start`, so `.mp3` becomes `.mp3.lyrics.txt`, not `.lyrics.txt`.
///
/// C's capacity parameter and its "returns false when the result would not fit"
/// contract disappear with `String`; `None` is left for the empty path, which C
/// also refused.
#[must_use]
pub fn lyric_sibling_path(audio_path: &str) -> Option<String> {
    if audio_path.is_empty() {
        return None;
    }
    let bytes = audio_path.as_bytes();
    let length = bytes.len();
    let mut name_start = 0;
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'/' || *byte == b'\\' {
            name_start = i + 1;
        }
    }
    let mut stem_end = length;
    let mut i = length;
    while i > name_start + 1 {
        if bytes[i - 1] == b'.' {
            stem_end = i - 1;
            break;
        }
        i -= 1;
    }
    // Separators and dots are ASCII, so `stem_end` is always a char boundary.
    Some(format!("{}.lyrics.txt", &audio_path[..stem_end]))
}

/// Panel-relative geometry for the Assist panel (`assist_ui_state.h:69-81`).
///
/// Pixel geometry stays `f32` as in C, so a layout computed here and one computed
/// by the drawing code cannot disagree by a rounding step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AssistUiLayout {
    pub mode_columns: usize,
    pub mode_rows: usize,
    pub mode_top: f32,
    pub mode_row_height: f32,
    pub status_y: f32,
    pub content_y: f32,
    /// Where the lyric-reference line starts, relative to the panel. Zero when
    /// there is no such row, which is also what makes "is the row present?" a
    /// question the drawing code can ask without repeating the policy.
    pub reference_y: f32,
    pub required_height: f32,
}

/// Pure layout policy shared by the raylib surface and headless tests
/// (`assist_ui_state.c:219-253`).
///
/// Widths at and above the supported 960 px window keep all modes on one row;
/// narrower embedders receive a two-column grid rather than clipped buttons.
///
/// `reference_row` is a parameter rather than an assumption because the row only
/// exists for modes that use a lyric reference, and a panel that reserves height
/// it never draws pushes the scene preview down for nothing. It grows only the
/// confirmation step — the step where the user actually decides — and its buttons
/// share the label's row rather than taking one of their own, because a taller
/// panel here comes straight out of the scene preview, which at the supported
/// 960x640 minimum has only 150 px to give.
#[must_use]
pub fn ui_layout(
    panel_width: f32,
    content: AssistPanelContent,
    reference_row: bool,
) -> AssistUiLayout {
    let mode_columns: usize = if panel_width.is_finite() && panel_width >= 760.0 {
        4
    } else {
        2
    };
    let mode_rows = AssistMode::ALL.len().div_ceil(mode_columns);
    let mode_top = 60.0f32;
    let mode_row_height = 58.0f32;
    let status_y = mode_top + mode_rows as f32 * mode_row_height + 3.0;
    let content_y = status_y + 25.0;

    let mut content_height: f32 = match content {
        AssistPanelContent::Ready => 22.0,
        AssistPanelContent::Confirmation => 84.0,
        AssistPanelContent::Running => 84.0,
        AssistPanelContent::Cancelling => 44.0,
        AssistPanelContent::Candidate => 118.0,
        AssistPanelContent::Empty => 84.0,
    };
    let mut reference_y = 0.0f32;
    if reference_row && content == AssistPanelContent::Confirmation {
        reference_y = content_y + 42.0;
        content_height += 34.0;
    }
    AssistUiLayout {
        mode_columns,
        mode_rows,
        mode_top,
        mode_row_height,
        status_y,
        content_y,
        reference_y,
        required_height: content_y + content_height + 10.0,
    }
}

/// Converts a desired Assist content height into a timeline height while keeping
/// a useful scene preview at supported small-window sizes
/// (`assist_ui_state.c:255-270`).
///
/// Controls, transport, waveform lane, and their margins consume 158 px in
/// `timeline()`. At least 150 px is preserved for the scene at the small-window
/// boundary; larger windows receive the exact requested panel height.
#[must_use]
pub fn timeline_height(screen_height: f32, toolbar_height: f32, panel_height: f32) -> f32 {
    if !screen_height.is_finite()
        || !toolbar_height.is_finite()
        || !panel_height.is_finite()
        || screen_height <= 0.0
        || toolbar_height < 0.0
        || panel_height <= 0.0
    {
        return 0.0;
    }
    let desired = panel_height + 158.0;
    let mut maximum = screen_height - toolbar_height - 150.0;
    if maximum < 150.0 {
        maximum = 150.0;
    }
    if desired < maximum {
        desired
    } else {
        maximum
    }
}

// ---------------------------------------------------------------------------
// The panel's own state and geometry.
//
// **Owner: Agent J.** Everything above this line is the port of
// `assist_ui_state.c`. What follows is the state the *panel* needs, which the C
// keeps as sixteen `p->assist_*` fields on its single global `Plug`
// (`plug.c:304-317`) and composes into a status line inline at `:2274-2337`.
// Both come here rather than into the drawing code for the usual reason: a
// status line and a button rectangle that only exist inside a `BeginDrawing`
// pair cannot be asserted without a window.
// ---------------------------------------------------------------------------

use core::cell::Cell;

use crate::project::analysis_candidate::{AnalysisCandidate, Lanes};
use crate::ui::workspace_layout::UiRect;

/// Which job artifact a Copy button puts on the clipboard
/// (`draw_assist_artifact_actions`, `plug.c:2069-2110`).
///
/// The widths are the oracle's and they are not free: the three buttons share
/// one fitted label size, so changing one changes the others' typography.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssistArtifact {
    Bridge,
    Log,
    Folder,
}

impl AssistArtifact {
    /// The three, in the oracle's order.
    pub const ALL: [AssistArtifact; 3] = [
        AssistArtifact::Bridge,
        AssistArtifact::Log,
        AssistArtifact::Folder,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AssistArtifact::Bridge => "Copy result",
            AssistArtifact::Log => "Copy log",
            AssistArtifact::Folder => "Copy folder",
        }
    }

    #[must_use]
    pub fn width(self) -> f32 {
        match self {
            AssistArtifact::Bridge | AssistArtifact::Folder => 98.0,
            AssistArtifact::Log => 86.0,
        }
    }
}

/// Which lanes a mode is authorized to replace (`assist_mode_lanes`,
/// `plug.c:3387-3397`).
///
/// A free function rather than a method on [`AssistMode`] because it is the one
/// place the panel's mode vocabulary meets the candidate's lane vocabulary, and
/// naming that seam is worth more than the convenience.
#[must_use]
pub fn mode_lanes(mode: AssistMode) -> Lanes {
    match mode {
        AssistMode::Lyrics => Lanes::lyrics_only(),
        AssistMode::Sections => Lanes {
            lyrics: false,
            sections: true,
            semantics: false,
        },
        AssistMode::Mimo => Lanes {
            lyrics: false,
            sections: false,
            semantics: true,
        },
        AssistMode::All => Lanes::ALL,
    }
}

/// What a click in the Assist panel asks the frame loop to do.
///
/// Deliberately payload-free and [`Copy`]: it travels through a [`Cell`] on
/// [`AssistSession`], and a request that carried an owned path would need a
/// `RefCell` and could then be observed half-taken. Every action's operand is
/// either the session's own state or the current track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistRequest {
    /// Start the selected workflow on the current track.
    Start,
    /// The confirmation step's Cancel: forget the arming, start nothing.
    DismissConfirmation,
    /// SIGTERM the running helper's process group.
    CancelJob,
    /// Apply the staged candidate. The panel arms
    /// [`AssistSession::apply_confirmation_pending`] on the first press itself;
    /// this is only sent on the second.
    Apply,
    /// Drop the staged candidate, changing nothing.
    Discard,
    /// Open a picker for an authored lyric sheet.
    ChooseLyricSheet,
    /// Forget the chosen sheet, falling back to sibling discovery.
    ClearLyricSheet,
    /// Put an artifact path on the clipboard.
    Copy(AssistArtifact),
}

/// Everything the Assist panel draws from and asks for, in one place.
///
/// This is the C's `p->assist_*` group (`plug.c:304-317`) minus the process
/// handle, which lives with the supervisor in `musializer-runtime`. Splitting it
/// there is what keeps this half raylib-free, OS-free and testable.
///
/// # Why three fields carry a `Cell`
///
/// A panel is drawn from a `&`-borrow of the application's state, because the
/// drawing pair also needs `&mut` of the widget claim table and of the raylib
/// handle. Three pieces of state are *purely* the panel's own — which workflow
/// is selected, whether the confirmation step is armed, and whether Apply has
/// been pressed once — and a click on them must take effect in the same frame or
/// the button will not appear to respond.
///
/// So those three are [`Cell`]s the panel may write through a shared borrow,
/// plus one more holding the [`AssistRequest`] the frame loop drains *after* the
/// pair closes. Everything else is written only by the frame loop, which holds
/// `&mut`. The rule is worth stating because it is the whole safety argument:
/// **nothing that owns a process, a file or a track is behind a `Cell`.**
///
/// This is not `Rc<RefCell<_>>` shared ownership — there is exactly one
/// `AssistSession` and it has one owner. It is a one-frame intent channel.
#[derive(Clone, Debug)]
pub struct AssistSession {
    /// The selected workflow (`p->assist_mode`).
    mode: Cell<AssistMode>,
    /// The review step is armed (`p->assist_confirmation_pending`).
    confirmation_pending: Cell<bool>,
    /// Apply has been pressed once (`p->assist_apply_confirmation_pending`).
    apply_confirmation_pending: Cell<bool>,
    /// What the panel asked for this frame, drained by the frame loop.
    request: Cell<Option<AssistRequest>>,

    /// The supervised job's lifecycle (`p->assist_job_state`).
    pub job_state: AssistJobState,
    /// Which track the running job targets (`p->assist_track_index`).
    pub job_track: Option<usize>,
    /// Seconds on the application clock when the job started
    /// (`p->assist_started_at`).
    pub started_at: f64,

    /// The staged, inert result (`p->assist_candidate`).
    pub candidate: Option<AnalysisCandidate>,
    /// The mode that produced it (`p->assist_candidate_mode`).
    pub candidate_mode: AssistMode,
    /// The track it targets (`p->assist_candidate_track_index`).
    pub candidate_track: Option<usize>,
    /// The first staged lyric, kept as text so the panel does not have to reach
    /// into the candidate's document to draw one line.
    pub candidate_first_lyric: String,

    /// `p->assist_bridge_path`, `p->assist_log_path`, `p->assist_output_dir`.
    /// Empty when this job produced no such artifact.
    pub bridge_path: String,
    pub log_path: String,
    pub output_dir: String,
    /// `p->assist_failure_detail`.
    pub failure_detail: String,
    /// Whether `tools/external_analysis.py` was found (`find_assist_helper`).
    /// Resolved once at startup rather than per frame, because the C's per-frame
    /// `FileExists` probe is a syscall inside the drawing pair.
    pub helper_available: bool,
}

impl Default for AssistSession {
    fn default() -> Self {
        Self {
            mode: Cell::new(AssistMode::Lyrics),
            confirmation_pending: Cell::new(false),
            apply_confirmation_pending: Cell::new(false),
            request: Cell::new(None),
            job_state: AssistJobState::Idle,
            job_track: None,
            started_at: 0.0,
            candidate: None,
            candidate_mode: AssistMode::Lyrics,
            candidate_track: None,
            candidate_first_lyric: String::new(),
            bridge_path: String::new(),
            log_path: String::new(),
            output_dir: String::new(),
            failure_detail: String::new(),
            helper_available: false,
        }
    }
}

impl AssistSession {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn mode(&self) -> AssistMode {
        self.mode.get()
    }

    /// Selecting a workflow also arms the review step, exactly as the C's mode
    /// button does (`plug.c:2266-2269`): a mode button is not "set the mode", it
    /// is "propose this run".
    pub fn select_mode(&self, mode: AssistMode) {
        self.mode.set(mode);
        self.confirmation_pending.set(true);
    }

    #[must_use]
    pub fn confirmation_pending(&self) -> bool {
        self.confirmation_pending.get()
    }

    /// Used by `--ui-probe assist=confirm` and by the frame loop when a job ends.
    pub fn set_confirmation_pending(&self, pending: bool) {
        self.confirmation_pending.set(pending);
    }

    #[must_use]
    pub fn apply_confirmation_pending(&self) -> bool {
        self.apply_confirmation_pending.get()
    }

    pub fn set_apply_confirmation_pending(&self, pending: bool) {
        self.apply_confirmation_pending.set(pending);
    }

    /// Records what the panel asked for. Last write in a frame wins, which is
    /// correct: two Assist actions cannot be pressed in one frame, because the
    /// widget claim table awards a release to exactly one button.
    pub fn request(&self, request: AssistRequest) {
        self.request.set(Some(request));
    }

    /// Drains the pending request. Called by the frame loop once the drawing
    /// pair has closed, because every one of these blocks: a picker is modal, a
    /// spawn touches the filesystem, an apply rewrites the track.
    pub fn take_request(&self) -> Option<AssistRequest> {
        self.request.take()
    }

    /// `assist_start_block` with this session's own facts.
    #[must_use]
    pub fn start_block(&self) -> AssistStartBlock {
        start_block(
            self.helper_available,
            self.job_state,
            self.candidate.is_some(),
        )
    }

    /// `assist_panel_content` with this session's own facts.
    #[must_use]
    pub fn panel_content(&self) -> AssistPanelContent {
        panel_content(
            self.job_state,
            self.confirmation_pending.get(),
            self.candidate.is_some(),
        )
    }

    /// Whether a helper process is still alive for this session.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.job_state.is_active()
    }

    /// Whether quitting now would lose something the user has not decided about.
    ///
    /// The quit guard weighs this (`plug_confirm_close`, `plug.c:7200-7250`): a
    /// staged result is undecided work and a running job is a process tree that
    /// will be killed. A terminal state with nothing staged is neither.
    #[must_use]
    pub fn blocks_close(&self) -> bool {
        self.candidate.is_some() || self.job_state.is_active()
    }

    /// Everything a finished, discarded or applied job leaves behind, cleared.
    ///
    /// The artifact paths deliberately survive: the C keeps them so the Copy
    /// buttons still work after a failure, which is the case they exist for
    /// (`plug.c:2532-2538`).
    pub fn clear_candidate(&mut self) {
        self.candidate = None;
        self.candidate_track = None;
        self.candidate_first_lyric.clear();
        self.confirmation_pending.set(false);
        self.apply_confirmation_pending.set(false);
        self.job_state = AssistJobState::Idle;
    }

    /// The path a Copy button would put on the clipboard, or `""`.
    #[must_use]
    pub fn artifact_path(&self, artifact: AssistArtifact) -> &str {
        match artifact {
            AssistArtifact::Bridge => &self.bridge_path,
            AssistArtifact::Log => &self.log_path,
            AssistArtifact::Folder => &self.output_dir,
        }
    }
}

/// How the status line should read (`plug.c:2332-2337`).
///
/// A tone rather than a colour, because the palette is raylib-side. The mapping
/// is the C's `status_color` ladder and its order is load-bearing: a failure
/// outranks a staged result, which outranks a completed-but-empty run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistStatusTone {
    Ink,
    Accent,
    Warning,
    Danger,
    Success,
}

/// The facts the status line is composed from.
///
/// A struct rather than twelve arguments, and borrowed rather than owned so the
/// caller can hand it slices of state it already has.
#[derive(Clone, Copy, Debug)]
pub struct AssistStatusInputs<'a> {
    pub session_mode: AssistMode,
    pub job_state: AssistJobState,
    pub confirmation_pending: bool,
    pub helper_available: bool,
    /// Seconds the running job has been alive. Ignored unless running.
    pub elapsed_seconds: f64,
    pub candidate_mode: Option<AssistMode>,
    /// `track_display_name` of the candidate's target, the job's target and the
    /// current track. `"missing track"` is the C's own placeholder for an index
    /// that no longer names a track (`plug.c:2277`).
    pub candidate_track_name: &'a str,
    pub job_track_name: &'a str,
    pub current_track_name: &'a str,
    pub failure_detail: &'a str,
    /// `GetFileName(p->assist_log_path)`, or empty.
    pub log_file_name: &'a str,
}

/// One line naming exactly what the panel is doing (`plug.c:2274-2337`).
///
/// The whole `if`/`else if` ladder comes across in the C's order, because the
/// order *is* the precedence: a staged result outranks a running job, which
/// outranks an armed confirmation, which outranks a missing helper.
///
/// Two details worth not losing. The C truncates the failure detail to 250
/// characters with `%.250s` — reproduced, because an unbounded helper message
/// would run off the panel rather than wrap. And the elapsed clock is
/// `%02u:%02u`, minutes and seconds with no hours, which is honest for a job
/// that stops at 40:00.
#[must_use]
pub fn status_line(inputs: &AssistStatusInputs<'_>) -> (String, AssistStatusTone) {
    let text = if let Some(mode) = inputs.candidate_mode {
        format!(
            "{} result  |  Validated  |  {}",
            mode.display_name(),
            inputs.candidate_track_name
        )
    } else if matches!(
        inputs.job_state,
        AssistJobState::Cancelling | AssistJobState::TimingOut | AssistJobState::Failing
    ) {
        let action = match inputs.job_state {
            AssistJobState::TimingOut => "Stopping at the 40:00 job deadline",
            AssistJobState::Failing => "Verifying process-tree cleanup",
            _ => "Cancelling",
        };
        format!(
            "{action} {}  |  {}",
            inputs.session_mode.display_name(),
            inputs.job_track_name
        )
    } else if inputs.job_state == AssistJobState::Running {
        let elapsed = if inputs.elapsed_seconds.is_finite() && inputs.elapsed_seconds > 0.0 {
            inputs.elapsed_seconds
        } else {
            0.0
        };
        format!(
            "{}  |  {}  |  {:02}:{:02} elapsed",
            inputs.session_mode.display_name(),
            inputs.job_track_name,
            (elapsed / 60.0) as u32,
            (elapsed % 60.0) as u32,
        )
    } else if inputs.confirmation_pending {
        let setup = if inputs.job_state == AssistJobState::Failed {
            "Last launch failed; review and retry"
        } else {
            "Review before starting"
        };
        format!(
            "{}  |  {}  |  {setup}{}",
            inputs.session_mode.display_name(),
            inputs.current_track_name,
            if inputs.helper_available {
                ""
            } else {
                "  |  Helper unavailable"
            }
        )
    } else if !inputs.helper_available {
        AssistStartBlock::HelperUnavailable.reason().to_string()
    } else {
        match inputs.job_state {
            AssistJobState::Cancelled => {
                "Analysis cancelled  |  Editor content unchanged".to_string()
            }
            AssistJobState::TimedOut => {
                "40:00 job deadline reached  |  Editor content unchanged".to_string()
            }
            AssistJobState::Failed => {
                let detail = if inputs.failure_detail.is_empty() {
                    "The helper exited before producing a validated result."
                } else {
                    inputs.failure_detail
                };
                format!(
                    "Analysis failed  |  {}{}{}",
                    truncate_bytes(detail, 250),
                    if inputs.log_file_name.is_empty() {
                        ""
                    } else {
                        "  |  Log: "
                    },
                    inputs.log_file_name
                )
            }
            AssistJobState::Succeeded => format!(
                "{} completed  |  No editor changes found",
                inputs.session_mode.display_name()
            ),
            _ => "Ready  |  Select a workflow to review its data boundary".to_string(),
        }
    };

    let tone = if inputs.job_state == AssistJobState::Failed {
        AssistStatusTone::Danger
    } else if inputs.candidate_mode.is_some() {
        AssistStatusTone::Success
    } else if inputs.job_state == AssistJobState::Succeeded || inputs.confirmation_pending {
        AssistStatusTone::Warning
    } else if inputs.job_state.is_active() {
        AssistStatusTone::Accent
    } else {
        AssistStatusTone::Ink
    };
    (text, tone)
}

/// `%.Ns` on a UTF-8 string: at most `limit` bytes, cut on a character boundary.
///
/// C counts bytes and would happily split a multi-byte sequence; Rust cannot, so
/// the cut moves back to the nearest boundary. The difference is invisible for
/// the ASCII the helper produces and is the only safe reading of the C's bound.
fn truncate_bytes(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The confirmation step's four buttons, in panel coordinates
/// (`plug.c:2352-2408`).
///
/// `choose` and `clear` are only *drawn* when [`Self::reference_room`] is true.
/// That is not cosmetic and the C says why at `:2371-2372`: a control painted
/// past the panel edge still claims presses from whatever is underneath it,
/// which is the same click-hijack this repository already paid for once in
/// `workspace_layout`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssistConfirmationButtons {
    pub choose: UiRect,
    pub clear: UiRect,
    /// Whether `choose` and `clear` fit beside the reference label.
    pub reference_room: bool,
    pub start: UiRect,
    pub cancel: UiRect,
}

/// Places them (`plug.c:2366-2387`).
///
/// `padding`, `gap` and `button_height` are parameters rather than constants
/// because the palette and metrics live app-side; passing them keeps this
/// function pure and lets a test drive the boundary case directly.
#[must_use]
pub fn confirmation_buttons(
    panel: UiRect,
    layout: &AssistUiLayout,
    padding: f32,
    gap: f32,
    button_height: f32,
) -> AssistConfirmationButtons {
    let action_y = panel.y + layout.content_y;
    let reference_present = layout.reference_y > 0.0;
    let reference_y = panel.y + layout.reference_y;

    let choose = UiRect::new(
        panel.x + panel.width - padding - 152.0,
        reference_y - 4.0,
        152.0,
        button_height,
    );
    let clear = UiRect::new(choose.x - 84.0 - gap, choose.y, 84.0, button_height);
    let reference_room = reference_present && clear.x > panel.x + padding + 240.0;

    // With the reference row present the Start button sits 40 px below it;
    // without it, 48 px below the workflow text (`plug.c:2341`, `:2385`).
    let start_offset = if reference_present {
        layout.reference_y - layout.content_y + 40.0
    } else {
        48.0
    };
    let start = UiRect::new(
        panel.x + padding,
        action_y + start_offset,
        144.0,
        button_height,
    );
    let cancel = UiRect::new(start.x + start.width + gap, start.y, 94.0, button_height);

    AssistConfirmationButtons {
        choose,
        clear,
        reference_room,
        start,
        cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(inputs: &AssistStatusInputs<'_>) -> String {
        status_line(inputs).0
    }

    fn idle_status<'a>() -> AssistStatusInputs<'a> {
        AssistStatusInputs {
            session_mode: AssistMode::Lyrics,
            job_state: AssistJobState::Idle,
            confirmation_pending: false,
            helper_available: true,
            elapsed_seconds: 0.0,
            candidate_mode: None,
            candidate_track_name: "missing track",
            job_track_name: "missing track",
            current_track_name: "kitty.mp3",
            failure_detail: "",
            log_file_name: "",
        }
    }

    #[test]
    fn assist_start_guards_report_one_truthful_reason() {
        assert_eq!(
            start_block(false, AssistJobState::Idle, false),
            AssistStartBlock::HelperUnavailable
        );
        assert_eq!(
            start_block(true, AssistJobState::Running, false),
            AssistStartBlock::JobActive
        );
        assert_eq!(
            start_block(true, AssistJobState::Cancelling, false),
            AssistStartBlock::JobActive
        );
        assert_eq!(
            start_block(true, AssistJobState::TimingOut, false),
            AssistStartBlock::JobActive
        );
        assert_eq!(
            start_block(true, AssistJobState::Failing, false),
            AssistStartBlock::JobActive
        );
        assert_eq!(
            start_block(false, AssistJobState::Running, false),
            AssistStartBlock::JobActive
        );
        assert_eq!(
            start_block(true, AssistJobState::Failed, true),
            AssistStartBlock::ResultPending
        );
        assert_eq!(
            start_block(false, AssistJobState::Failed, true),
            AssistStartBlock::ResultPending
        );
        assert_eq!(
            start_block(true, AssistJobState::Cancelled, false),
            AssistStartBlock::Allowed
        );
        assert!(!AssistStartBlock::JobActive.reason().is_empty());
    }

    #[test]
    fn assist_panel_content_precedence_matches_the_job_lifecycle() {
        assert_eq!(
            panel_content(AssistJobState::Idle, false, false),
            AssistPanelContent::Ready
        );
        assert_eq!(
            panel_content(AssistJobState::Failed, true, false),
            AssistPanelContent::Confirmation
        );
        assert_eq!(
            panel_content(AssistJobState::Running, true, false),
            AssistPanelContent::Running
        );
        assert_eq!(
            panel_content(AssistJobState::Cancelling, true, false),
            AssistPanelContent::Cancelling
        );
        assert_eq!(
            panel_content(AssistJobState::TimingOut, true, false),
            AssistPanelContent::Cancelling
        );
        assert_eq!(
            panel_content(AssistJobState::Running, true, true),
            AssistPanelContent::Candidate
        );
        assert_eq!(
            panel_content(AssistJobState::Succeeded, false, false),
            AssistPanelContent::Empty
        );
        assert_eq!(
            panel_content(AssistJobState::Failed, false, false),
            AssistPanelContent::Empty
        );
        assert_eq!(
            panel_content(AssistJobState::TimedOut, false, false),
            AssistPanelContent::Empty
        );
    }

    #[test]
    fn assist_deadline_is_job_wide_monotonic_and_exact() {
        assert!(!AssistJobState::Running
            .deadline_expired(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS - 0.001));
        assert!(AssistJobState::Running.deadline_expired(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS));
        assert!(
            !AssistJobState::Cancelling.deadline_expired(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS)
        );
        assert!(!AssistJobState::Running.deadline_expired(10.0, 9.0));
        assert_eq!(
            AssistJobState::Running.deadline_remaining(10.0, 610.0),
            1800.0
        );
        assert_eq!(
            AssistJobState::Running
                .deadline_remaining(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS + 1.0),
            0.0
        );
    }

    #[test]
    fn assist_modes_expose_real_workflow_and_data_boundary_copy() {
        for mode in AssistMode::ALL {
            assert!(!mode.display_name().is_empty());
            assert!(!mode.argument().is_empty());
            assert!(!mode.badge().is_empty());
            assert!(!mode.workflow().is_empty());
            assert!(!mode.data_boundary().is_empty());
        }
        assert!(AssistMode::Sections.data_boundary().contains("locally"));
        assert!(AssistMode::Mimo.data_boundary().contains("OpenRouter"));
        assert!(AssistMode::Lyrics.data_boundary().contains("Codex"));
        assert!(AssistMode::Lyrics
            .empty_result()
            .contains("no validated lyric cues"));
        assert!(AssistMode::All
            .empty_result()
            .contains("no validated editor changes"));
    }

    #[test]
    fn assist_layout_fits_supported_small_and_large_windows() {
        let small = ui_layout(948.0, AssistPanelContent::Candidate, false);
        let large = ui_layout(1908.0, AssistPanelContent::Candidate, false);
        assert_eq!(small.mode_columns, 4);
        assert_eq!(small.mode_rows, 1);
        assert_eq!(large.mode_columns, 4);
        assert!(small.required_height <= 282.0);
        assert_eq!(small.required_height, large.required_height);

        // The supported minimum is 960x640 with a 50 px toolbar. The Assist panel
        // still receives enough room for its longest staged-result state.
        let timeline = timeline_height(640.0, 50.0, small.required_height);
        assert!(timeline <= 440.0);
        assert!(timeline - 158.0 >= small.required_height - 0.01);
    }

    #[test]
    fn assist_layout_degrades_to_two_columns_when_embedded_narrowly() {
        let narrow = ui_layout(620.0, AssistPanelContent::Confirmation, false);
        assert_eq!(narrow.mode_columns, 2);
        assert_eq!(narrow.mode_rows, 2);
        assert!(narrow.status_y > 175.0);
        assert!(
            narrow.required_height
                > ui_layout(948.0, AssistPanelContent::Confirmation, false).required_height
        );
    }

    #[test]
    fn assist_never_replaces_an_active_authored_lyric_draft() {
        assert!(candidate_conflicts_with_lyric_draft(true, true, true));
        assert!(!candidate_conflicts_with_lyric_draft(false, true, true));
        assert!(!candidate_conflicts_with_lyric_draft(true, false, true));
        assert!(!candidate_conflicts_with_lyric_draft(true, true, false));
    }

    #[test]
    fn assist_empty_results_are_not_applyable_candidates() {
        assert!(!result_has_changes(1, 0));
        assert!(!result_has_changes(1, 2));
        assert!(result_has_changes(1, 1));
        assert!(result_has_changes(7, 4));
    }

    #[test]
    fn assist_lyric_reference_row_only_grows_the_step_that_shows_it() {
        // The row belongs to the review step, where the user decides. Every other
        // panel state must be exactly the height it was.
        let others = [
            AssistPanelContent::Ready,
            AssistPanelContent::Running,
            AssistPanelContent::Cancelling,
            AssistPanelContent::Candidate,
            AssistPanelContent::Empty,
        ];
        for content in others {
            assert_eq!(
                ui_layout(948.0, content, true).required_height,
                ui_layout(948.0, content, false).required_height
            );
            assert_eq!(ui_layout(948.0, content, true).reference_y, 0.0);
        }

        let without = ui_layout(948.0, AssistPanelContent::Confirmation, false);
        let with = ui_layout(948.0, AssistPanelContent::Confirmation, true);
        assert_eq!(without.reference_y, 0.0);
        assert!(with.reference_y > with.content_y);
        assert!(with.required_height > without.required_height);
        // The row must sit inside the panel it grew, not past the bottom of it.
        assert!(with.reference_y + 34.0 <= with.required_height);

        // And the taller panel must still fit the supported minimum window.
        let timeline = timeline_height(640.0, 50.0, with.required_height);
        assert!(timeline - 158.0 >= with.required_height - 0.01);
    }

    #[test]
    fn assist_lyric_reference_is_offered_only_where_it_is_used() {
        assert!(AssistMode::Lyrics.uses_lyric_reference());
        assert!(AssistMode::All.uses_lyric_reference());
        // Scene changes and MiMo never read authored lyrics, so offering the
        // control there would promise something the run does not do.
        assert!(!AssistMode::Sections.uses_lyric_reference());
        assert!(!AssistMode::Mimo.uses_lyric_reference());

        for reference in AssistLyricReference::ALL {
            assert!(!reference.summary().is_empty());
        }
        // Whether an embedded lyrics tag exists needs ffprobe, so the "none" case
        // must not claim transcription outright.
        assert!(AssistLyricReference::None.summary().contains("unless"));
    }

    #[test]
    fn assist_lyric_sibling_path_matches_the_helpers_stem_rule() {
        let cases = [
            ("kitty.mp3", "kitty.lyrics.txt"),
            ("/music/kitty.mp3", "/music/kitty.lyrics.txt"),
            ("/music/a.b.mp3", "/music/a.b.lyrics.txt"),
            // No extension: pathlib's stem is the whole name.
            ("kitty", "kitty.lyrics.txt"),
            // A leading dot is part of the name, not an extension, so ".mp3"
            // becomes ".mp3.lyrics.txt" rather than ".lyrics.txt".
            (".mp3", ".mp3.lyrics.txt"),
            ("/music/.mp3", "/music/.mp3.lyrics.txt"),
            // A dot in a parent directory must not be mistaken for the extension.
            ("/my.music/kitty", "/my.music/kitty.lyrics.txt"),
            ("C:\\my.music\\kitty", "C:\\my.music\\kitty.lyrics.txt"),
        ];
        for (audio, sibling) in cases {
            assert_eq!(lyric_sibling_path(audio).as_deref(), Some(sibling));
        }

        // C also refused a NULL path, a NULL output and a zero capacity. The last
        // two are unrepresentable here; the empty path is still refused.
        assert_eq!(lyric_sibling_path(""), None);
    }

    #[test]
    fn a_mode_authorizes_exactly_the_lanes_it_produces() {
        // `assist_mode_lanes` (plug.c:3387-3397). Getting one of these wrong
        // would let a Sections run replace the lyric lane.
        assert_eq!(mode_lanes(AssistMode::Lyrics), Lanes::lyrics_only());
        assert_eq!(
            mode_lanes(AssistMode::Sections),
            Lanes {
                lyrics: false,
                sections: true,
                semantics: false
            }
        );
        assert_eq!(
            mode_lanes(AssistMode::Mimo),
            Lanes {
                lyrics: false,
                sections: false,
                semantics: true
            }
        );
        assert_eq!(mode_lanes(AssistMode::All), Lanes::ALL);
        for mode in AssistMode::ALL {
            assert!(mode_lanes(mode).is_valid(), "{mode:?} authorizes nothing");
        }
        // The lyric-reference control is offered exactly where the lane it feeds
        // is authorized.
        for mode in AssistMode::ALL {
            assert_eq!(mode.uses_lyric_reference(), mode_lanes(mode).lyrics);
        }
    }

    #[test]
    fn selecting_a_workflow_proposes_it_rather_than_merely_setting_it() {
        // plug.c:2266-2269 — a mode button arms the review step. Setting the mode
        // silently would make the badge change with no visible next step.
        let session = AssistSession::new();
        assert_eq!(session.mode(), AssistMode::Lyrics);
        assert!(!session.confirmation_pending());
        session.select_mode(AssistMode::Mimo);
        assert_eq!(session.mode(), AssistMode::Mimo);
        assert!(session.confirmation_pending());
    }

    #[test]
    fn a_request_survives_exactly_one_drain() {
        let session = AssistSession::new();
        assert_eq!(session.take_request(), None);
        session.request(AssistRequest::Start);
        session.request(AssistRequest::Copy(AssistArtifact::Log));
        assert_eq!(
            session.take_request(),
            Some(AssistRequest::Copy(AssistArtifact::Log)),
            "last write in a frame wins"
        );
        assert_eq!(session.take_request(), None, "and it is not replayed");
    }

    #[test]
    fn the_quit_guard_sees_a_staged_result_and_a_live_job() {
        let mut session = AssistSession::new();
        assert!(!session.blocks_close());
        session.job_state = AssistJobState::Running;
        assert!(session.blocks_close());
        session.job_state = AssistJobState::Failed;
        assert!(
            !session.blocks_close(),
            "a finished failure is not undecided work"
        );
    }

    #[test]
    fn clearing_a_candidate_keeps_the_artifacts_it_left_behind() {
        // plug.c:2532-2538: the Copy buttons exist for exactly the runs that went
        // wrong, so discarding the result must not take the log with it.
        let mut session = AssistSession::new();
        session.log_path = "/tmp/a/lyrics-1.log".to_string();
        session.bridge_path = "/tmp/a/lyrics-1.bridge.tsv".to_string();
        session.job_state = AssistJobState::Succeeded;
        session.candidate_track = Some(3);
        session.set_apply_confirmation_pending(true);
        session.clear_candidate();
        assert_eq!(session.job_state, AssistJobState::Idle);
        assert!(!session.apply_confirmation_pending());
        assert_eq!(session.candidate_track, None);
        assert_eq!(
            session.artifact_path(AssistArtifact::Log),
            "/tmp/a/lyrics-1.log"
        );
        assert_eq!(session.artifact_path(AssistArtifact::Folder), "");
    }

    #[test]
    fn the_status_line_follows_the_oracles_precedence() {
        // The ladder at plug.c:2274-2331, top to bottom. Each case must beat the
        // ones below it, which is why every one of these sets the state a lower
        // branch would also have matched.
        let mut inputs = idle_status();
        assert!(status(&inputs).starts_with("Ready  |  Select a workflow"));
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Ink);

        inputs.helper_available = false;
        assert_eq!(
            status(&inputs),
            AssistStartBlock::HelperUnavailable.reason()
        );

        inputs = idle_status();
        inputs.confirmation_pending = true;
        assert_eq!(
            status(&inputs),
            "Timed lyrics  |  kitty.mp3  |  Review before starting"
        );
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Warning);
        inputs.helper_available = false;
        assert!(status(&inputs).ends_with("  |  Helper unavailable"));
        inputs.helper_available = true;
        inputs.job_state = AssistJobState::Failed;
        assert!(status(&inputs).contains("Last launch failed; review and retry"));
        assert_eq!(
            status_line(&inputs).1,
            AssistStatusTone::Danger,
            "a failed launch stays red even while the retry is armed"
        );

        inputs = idle_status();
        inputs.job_state = AssistJobState::Running;
        inputs.job_track_name = "kitty.mp3";
        inputs.elapsed_seconds = 605.0;
        inputs.confirmation_pending = true;
        assert_eq!(
            status(&inputs),
            "Timed lyrics  |  kitty.mp3  |  10:05 elapsed"
        );
        // The tone ladder and the text ladder disagree here, and that is the
        // oracle's (`plug.c:2332-2337` puts `confirmation_pending` *above*
        // `active`, while `:2288` puts running above pending). A job started from
        // the review step therefore reads as running in amber, not accent, until
        // the arming is cleared. Reproduced rather than tidied: it is only a
        // colour, and two ladders that were "corrected" into one would change what
        // a running job looks like.
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Warning);
        inputs.confirmation_pending = false;
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Accent);
        inputs.confirmation_pending = true;

        inputs.job_state = AssistJobState::TimingOut;
        assert!(status(&inputs).starts_with("Stopping at the 40:00 job deadline Timed lyrics"));
        inputs.job_state = AssistJobState::Failing;
        assert!(status(&inputs).starts_with("Verifying process-tree cleanup"));
        inputs.job_state = AssistJobState::Cancelling;
        assert!(status(&inputs).starts_with("Cancelling Timed lyrics"));

        inputs.candidate_mode = Some(AssistMode::All);
        inputs.candidate_track_name = "kitty.mp3";
        assert_eq!(
            status(&inputs),
            "Full assist result  |  Validated  |  kitty.mp3",
            "a staged result outranks everything"
        );
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Success);
    }

    #[test]
    fn a_terminal_run_says_what_it_left_alone() {
        let mut inputs = idle_status();
        inputs.job_state = AssistJobState::Cancelled;
        assert_eq!(
            status(&inputs),
            "Analysis cancelled  |  Editor content unchanged"
        );
        inputs.job_state = AssistJobState::TimedOut;
        assert!(status(&inputs).starts_with("40:00 job deadline reached"));
        inputs.job_state = AssistJobState::Succeeded;
        assert_eq!(
            status(&inputs),
            "Timed lyrics completed  |  No editor changes found"
        );
        assert_eq!(status_line(&inputs).1, AssistStatusTone::Warning);
    }

    #[test]
    fn a_failure_names_its_log_and_is_bounded() {
        let mut inputs = idle_status();
        inputs.job_state = AssistJobState::Failed;
        assert_eq!(
            status(&inputs),
            "Analysis failed  |  The helper exited before producing a validated result."
        );
        inputs.failure_detail = "whisper.cpp is not installed";
        inputs.log_file_name = "lyrics-1234-0000000000000001.log";
        assert_eq!(
            status(&inputs),
            "Analysis failed  |  whisper.cpp is not installed  |  Log: \
             lyrics-1234-0000000000000001.log"
        );
        // `%.250s`: a helper that prints an essay must not run off the panel.
        let essay = "x".repeat(400);
        inputs.failure_detail = &essay;
        inputs.log_file_name = "";
        assert_eq!(status(&inputs).len(), "Analysis failed  |  ".len() + 250);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // C counts bytes. Rust cannot cut mid-sequence, so the bound moves back
        // to a boundary rather than panicking.
        let text = "aa\u{00e9}bb";
        assert_eq!(truncate_bytes(text, 2), "aa");
        assert_eq!(truncate_bytes(text, 3), "aa");
        assert_eq!(truncate_bytes(text, 4), "aa\u{00e9}");
        assert_eq!(truncate_bytes(text, 99), text);
    }

    #[test]
    fn the_reference_controls_are_dropped_rather_than_drawn_past_the_edge() {
        // plug.c:2371-2372. A control painted outside the panel still claims the
        // press, which is the click hijack `workspace_layout` was written for.
        let layout = ui_layout(948.0, AssistPanelContent::Confirmation, true);
        let wide = confirmation_buttons(
            UiRect::new(0.0, 100.0, 948.0, layout.required_height),
            &layout,
            10.0,
            8.0,
            36.0,
        );
        assert!(wide.reference_room);
        assert!(wide.choose.x + wide.choose.width <= 948.0 - 10.0 + 0.01);
        assert!(wide.clear.x + wide.clear.width < wide.choose.x);

        // A narrow embedder: the label stays, the buttons go.
        let narrow_layout = ui_layout(480.0, AssistPanelContent::Confirmation, true);
        let narrow = confirmation_buttons(
            UiRect::new(0.0, 0.0, 480.0, narrow_layout.required_height),
            &narrow_layout,
            10.0,
            8.0,
            36.0,
        );
        assert!(!narrow.reference_room);

        // Start and Cancel never overlap, at either width.
        for buttons in [wide, narrow] {
            assert!(!buttons.start.overlaps(buttons.cancel));
        }
    }

    #[test]
    fn start_sits_below_the_reference_row_only_when_there_is_one() {
        // Without the row the offset is a flat 48 px; with it, 40 px below the
        // row's own top (`plug.c:2341`, `:2385`).
        let panel = UiRect::new(0.0, 0.0, 948.0, 400.0);
        let without = ui_layout(948.0, AssistPanelContent::Confirmation, false);
        let with = ui_layout(948.0, AssistPanelContent::Confirmation, true);
        let a = confirmation_buttons(panel, &without, 10.0, 8.0, 36.0);
        let b = confirmation_buttons(panel, &with, 10.0, 8.0, 36.0);
        assert_eq!(a.start.y, without.content_y + 48.0);
        assert_eq!(b.start.y, with.reference_y + 40.0);
        assert!(b.start.y > a.start.y, "the row pushes Start down, not up");
        // And the whole block still lands inside the height the layout asked for.
        assert!(b.start.y + b.start.height <= with.required_height + 0.01);
    }
}
