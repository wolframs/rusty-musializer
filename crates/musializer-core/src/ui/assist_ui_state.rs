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

#[cfg(test)]
mod tests {
    use super::*;

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
}
