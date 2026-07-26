//! Font import state machine, nonce and staleness handling.
//!
//! **Owner: Agent F.** Port of `font_import_state.c`/`.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! The decisions the caption font browser has to get right, kept away from the
//! raylib surface so they can be tested without a window: when a network call is
//! allowed, when a job has run out of time, and which result belongs to the job
//! the user is actually waiting for (`font_import_state.h:8-11`).
//!
//! Downloading a face is the second network boundary in this application and the
//! weaker of the two. MiMo sends the user's audio; this sends a family name. The
//! consent is still explicit and still separate, because "I asked for a font" is
//! not "I agreed to send my music somewhere" (`font_import_state.h:13-16`).

/// A catalogue is one request against a static file (`font_import_state.h:19-20`).
pub const CATALOGUE_JOB_TIMEOUT_SECONDS: f64 = 120.0;
/// A face is three requests and a few hundred kilobytes; neither job should be
/// able to hang the panel (`font_import_state.h:19-21`).
pub const FETCH_JOB_TIMEOUT_SECONDS: f64 = 180.0;
/// How long a finished job's outcome stays on screen before the panel returns to
/// browsing. Failures are not cleared on a timer; they wait to be read
/// (`font_import_state.h:22-24`).
pub const SUCCESS_LINGER_SECONDS: f64 = 6.0;

/// Which request is in flight (`font_import_state.h:26-30`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontImportJob {
    #[default]
    None,
    Catalogue,
    Fetch,
}

impl FontImportJob {
    /// Every job kind.
    pub const ALL: [FontImportJob; 3] = [
        FontImportJob::None,
        FontImportJob::Catalogue,
        FontImportJob::Fetch,
    ];

    /// This job's budget in seconds, or `0.0` when there is no job
    /// (`font_import_state.c:14-22`).
    ///
    /// A download is allowed longer than a list, and neither borrows the other's
    /// budget.
    #[must_use]
    pub fn timeout(self) -> f64 {
        match self {
            FontImportJob::Catalogue => CATALOGUE_JOB_TIMEOUT_SECONDS,
            FontImportJob::Fetch => FETCH_JOB_TIMEOUT_SECONDS,
            FontImportJob::None => 0.0,
        }
    }
}

/// A font request's lifecycle (`font_import_state.h:32-40`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontImportJobState {
    #[default]
    Idle,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl FontImportJobState {
    /// Every state. `is_active` and `is_finished` partition all of them, and a
    /// headless test asserts nothing is ever both — otherwise a poll would try to
    /// cancel a job that has already published its result.
    pub const ALL: [FontImportJobState; 7] = [
        FontImportJobState::Idle,
        FontImportJobState::Running,
        FontImportJobState::Cancelling,
        FontImportJobState::Succeeded,
        FontImportJobState::Failed,
        FontImportJobState::Cancelled,
        FontImportJobState::TimedOut,
    ];

    /// Whether a request is still in flight (`font_import_state.c:3-6`).
    ///
    /// Cancelling counts: the worker may still be running, which is exactly why
    /// it keeps its deadline.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(
            self,
            FontImportJobState::Running | FontImportJobState::Cancelling
        )
    }

    /// Whether the request has stopped for any reason
    /// (`font_import_state.c:8-12`).
    #[must_use]
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            FontImportJobState::Succeeded
                | FontImportJobState::Failed
                | FontImportJobState::Cancelled
                | FontImportJobState::TimedOut
        )
    }
}

/// Why a request may not start, or that it may (`font_import_state.h:42-49`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontImportBlock {
    #[default]
    Allowed,
    HelperUnavailable,
    ConsentRequired,
    JobActive,
    NoFamily,
    NoTrack,
}

impl FontImportBlock {
    /// Every block state, for exhaustive headless checks.
    pub const ALL: [FontImportBlock; 6] = [
        FontImportBlock::Allowed,
        FontImportBlock::HelperUnavailable,
        FontImportBlock::ConsentRequired,
        FontImportBlock::JobActive,
        FontImportBlock::NoFamily,
        FontImportBlock::NoTrack,
    ];

    /// The one-line reason, empty when allowed (`font_import_state.c:74-91`).
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            FontImportBlock::Allowed => "",
            FontImportBlock::HelperUnavailable => {
                "The font helper is missing from this installation."
            }
            FontImportBlock::ConsentRequired => "Allow contacting Google Fonts first.",
            FontImportBlock::JobActive => "A font request is already running.",
            FontImportBlock::NoFamily => "Choose a family first.",
            FontImportBlock::NoTrack => "Open a track before importing a caption face.",
        }
    }
}

/// Which body the font browser draws (`font_import_state.h:51-60`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontImportPanel {
    /// Before the first network call, the panel explains what leaves the machine
    /// and asks. It is not a dialog to dismiss on the way to the list.
    #[default]
    Consent,
    Loading,
    Browsing,
    Fetching,
    Cancelling,
    Failed,
}

/// Whether the job has run out of time (`font_import_state.c:24-36`).
///
/// A job being cancelled still has a deadline; otherwise a worker that ignores the
/// signal keeps the panel locked forever. A finished job has none.
///
/// A clock that went backwards is not evidence that the job overran. This is
/// monotonic in practice, but a suspend/resume is not worth a spurious "the
/// download timed out" on a job that is running perfectly well.
#[must_use]
pub fn job_deadline_expired(
    job: FontImportJob,
    state: FontImportJobState,
    started_at: f64,
    now: f64,
) -> bool {
    if !state.is_active() {
        return false;
    }
    let timeout = job.timeout();
    if timeout <= 0.0 {
        return false;
    }
    if now < started_at {
        return false;
    }
    now - started_at >= timeout
}

/// Seconds left on the job's budget (`font_import_state.c:38-48`).
///
/// A backwards clock reports the full budget rather than zero, which is the same
/// refusal to treat it as an overrun that [`job_deadline_expired`] makes.
#[must_use]
pub fn job_deadline_remaining(
    job: FontImportJob,
    state: FontImportJobState,
    started_at: f64,
    now: f64,
) -> f64 {
    if !state.is_active() {
        return 0.0;
    }
    let timeout = job.timeout();
    if timeout <= 0.0 {
        return 0.0;
    }
    if now < started_at {
        return timeout;
    }
    let remaining = timeout - (now - started_at);
    if remaining > 0.0 {
        remaining
    } else {
        0.0
    }
}

/// Whether a browse may begin (`font_import_state.c:50-58`).
///
/// Fetching the family list is a network call like any other, so it is gated by
/// the same consent as a download (`font_import_state.h:72-73`). A missing helper
/// is reported ahead of consent: asking someone to approve a network call this
/// installation cannot make would be a pointless question with a misleading
/// answer.
#[must_use]
pub fn browse_block(
    helper_available: bool,
    network_allowed: bool,
    state: FontImportJobState,
) -> FontImportBlock {
    if !helper_available {
        return FontImportBlock::HelperUnavailable;
    }
    if !network_allowed {
        return FontImportBlock::ConsentRequired;
    }
    if state.is_active() {
        return FontImportBlock::JobActive;
    }
    FontImportBlock::Allowed
}

/// Whether a download may begin (`font_import_state.c:60-72`).
///
/// A track is required because the face has to belong to something: without one
/// there is nothing for it to be bundled into, and the download would be discarded
/// on exit (`font_import_state.h:78-80`).
#[must_use]
pub fn fetch_block(
    helper_available: bool,
    network_allowed: bool,
    state: FontImportJobState,
    family_selected: bool,
    track_present: bool,
) -> FontImportBlock {
    let block = browse_block(helper_available, network_allowed, state);
    if block != FontImportBlock::Allowed {
        return block;
    }
    if !track_present {
        return FontImportBlock::NoTrack;
    }
    if !family_selected {
        return FontImportBlock::NoFamily;
    }
    FontImportBlock::Allowed
}

/// Which panel body to draw (`font_import_state.c:93-113`).
///
/// Consent outranks everything. Withdrawing it while a request is in flight must
/// show the consent panel, not a progress bar for work the user has just said they
/// did not want.
///
/// A cancelled job leaves whatever was already loaded on screen: the user stopped
/// a request, they did not ask to lose the list.
#[must_use]
pub fn panel(
    network_allowed: bool,
    catalogue_loaded: bool,
    job: FontImportJob,
    state: FontImportJobState,
) -> FontImportPanel {
    if !network_allowed {
        return FontImportPanel::Consent;
    }
    if state == FontImportJobState::Cancelling {
        return FontImportPanel::Cancelling;
    }
    if state == FontImportJobState::Running {
        return if job == FontImportJob::Fetch {
            FontImportPanel::Fetching
        } else {
            FontImportPanel::Loading
        };
    }
    if matches!(
        state,
        FontImportJobState::Failed | FontImportJobState::TimedOut
    ) {
        return FontImportPanel::Failed;
    }
    if catalogue_loaded {
        FontImportPanel::Browsing
    } else {
        FontImportPanel::Loading
    }
}

/// Whether a produced artifact belongs to the job the user is waiting for
/// (`font_import_state.c:115-120`).
///
/// Anything else is a job that was cancelled or superseded while it was still
/// writing, and applying it would replace the caption face with one nobody asked
/// for (`font_import_state.h:91-94`).
///
/// Zero is the nonce no job ever carries, so it can never match and a zeroed
/// structure cannot accidentally accept a stale artifact.
#[must_use]
pub fn result_is_current(result_nonce: u64, active_nonce: u64) -> bool {
    result_nonce != 0 && result_nonce == active_nonce
}

/// The next live nonce (`font_import_state.c:122-126`).
///
/// Wrapping skips zero rather than reusing it as a live nonce. C relied on
/// unsigned wraparound here, so this uses `wrapping_add` rather than letting a
/// debug build panic on the one input where the skip matters.
#[must_use]
pub fn next_nonce(nonce: u64) -> u64 {
    let next = nonce.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

/// Whether a finished job's outcome has been on screen long enough to clear
/// (`font_import_state.c:128-136`).
///
/// Success and cancellation return to the list on their own; failure waits,
/// because the reason is the point (`font_import_state.h:98-99`). A backwards
/// clock does not expire an outcome either.
#[must_use]
pub fn outcome_expired(state: FontImportJobState, finished_at: f64, now: f64) -> bool {
    if !matches!(
        state,
        FontImportJobState::Succeeded | FontImportJobState::Cancelled
    ) {
        return false;
    }
    if now < finished_at {
        return false;
    }
    now - finished_at >= SUCCESS_LINGER_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_import_consent_gates_every_request_including_the_catalogue() {
        // Fetching the family list is a network call like any other. Letting the
        // browse happen "because it is only a list" would contact Google before
        // the user had agreed to contact anyone.
        assert_eq!(
            browse_block(true, false, FontImportJobState::Idle),
            FontImportBlock::ConsentRequired
        );
        assert_eq!(
            browse_block(true, true, FontImportJobState::Idle),
            FontImportBlock::Allowed
        );
        assert_eq!(
            fetch_block(true, false, FontImportJobState::Idle, true, true),
            FontImportBlock::ConsentRequired
        );

        // A missing helper is reported ahead of consent: asking someone to approve
        // a network call this installation cannot make would be a pointless
        // question with a misleading answer.
        assert_eq!(
            browse_block(false, false, FontImportJobState::Idle),
            FontImportBlock::HelperUnavailable
        );
        assert_eq!(
            browse_block(false, true, FontImportJobState::Idle),
            FontImportBlock::HelperUnavailable
        );

        for block in FontImportBlock::ALL {
            assert_eq!(
                block == FontImportBlock::Allowed,
                block.reason().is_empty(),
                "only the allowed state has no reason to show"
            );
        }
    }

    #[test]
    fn font_import_refuses_to_start_a_second_request_or_one_with_nowhere_to_go() {
        assert_eq!(
            browse_block(true, true, FontImportJobState::Running),
            FontImportBlock::JobActive
        );
        assert_eq!(
            browse_block(true, true, FontImportJobState::Cancelling),
            FontImportBlock::JobActive
        );
        // A finished job is not an active one: the panel must be usable again the
        // moment a request stops, not once its outcome has been dismissed.
        assert_eq!(
            browse_block(true, true, FontImportJobState::Succeeded),
            FontImportBlock::Allowed
        );
        assert_eq!(
            browse_block(true, true, FontImportJobState::Failed),
            FontImportBlock::Allowed
        );
        assert_eq!(
            browse_block(true, true, FontImportJobState::TimedOut),
            FontImportBlock::Allowed
        );

        // A face has to belong to something. Without a track there is nothing for
        // it to be bundled into, and the download would be discarded on exit.
        assert_eq!(
            fetch_block(true, true, FontImportJobState::Idle, true, false),
            FontImportBlock::NoTrack
        );
        assert_eq!(
            fetch_block(true, true, FontImportJobState::Idle, false, true),
            FontImportBlock::NoFamily
        );
        assert_eq!(
            fetch_block(true, true, FontImportJobState::Idle, true, true),
            FontImportBlock::Allowed
        );
    }

    #[test]
    fn font_import_deadlines_are_per_job_and_survive_a_clock_that_moves_backwards() {
        assert_eq!(
            FontImportJob::Catalogue.timeout(),
            CATALOGUE_JOB_TIMEOUT_SECONDS
        );
        assert_eq!(FontImportJob::Fetch.timeout(), FETCH_JOB_TIMEOUT_SECONDS);
        assert_eq!(FontImportJob::None.timeout(), 0.0);

        // A download is allowed longer than a list, and neither borrows the
        // other's budget.
        assert!(job_deadline_expired(
            FontImportJob::Catalogue,
            FontImportJobState::Running,
            100.0,
            100.0 + CATALOGUE_JOB_TIMEOUT_SECONDS
        ));
        assert!(!job_deadline_expired(
            FontImportJob::Fetch,
            FontImportJobState::Running,
            100.0,
            100.0 + CATALOGUE_JOB_TIMEOUT_SECONDS
        ));
        assert!(!job_deadline_expired(
            FontImportJob::Catalogue,
            FontImportJobState::Running,
            100.0,
            100.0 + CATALOGUE_JOB_TIMEOUT_SECONDS - 0.001
        ));

        // A job being cancelled still has a deadline; otherwise a worker that
        // ignores the signal keeps the panel locked forever.
        assert!(job_deadline_expired(
            FontImportJob::Fetch,
            FontImportJobState::Cancelling,
            0.0,
            FETCH_JOB_TIMEOUT_SECONDS + 1.0
        ));
        // A finished job has none.
        assert!(!job_deadline_expired(
            FontImportJob::Fetch,
            FontImportJobState::Succeeded,
            0.0,
            1e9
        ));

        // Suspend/resume can hand back a smaller "now". That is not a job overrun.
        assert!(!job_deadline_expired(
            FontImportJob::Fetch,
            FontImportJobState::Running,
            500.0,
            100.0
        ));
        assert_eq!(
            job_deadline_remaining(
                FontImportJob::Fetch,
                FontImportJobState::Running,
                500.0,
                100.0
            ),
            FETCH_JOB_TIMEOUT_SECONDS
        );

        assert_eq!(
            job_deadline_remaining(
                FontImportJob::Catalogue,
                FontImportJobState::Running,
                10.0,
                10.0
            ),
            CATALOGUE_JOB_TIMEOUT_SECONDS
        );
        assert_eq!(
            job_deadline_remaining(
                FontImportJob::Catalogue,
                FontImportJobState::Running,
                10.0,
                1e9
            ),
            0.0
        );
        assert_eq!(
            job_deadline_remaining(
                FontImportJob::Catalogue,
                FontImportJobState::Idle,
                10.0,
                10.0
            ),
            0.0
        );
    }

    #[test]
    fn font_import_panel_puts_consent_ahead_of_work_already_in_flight() {
        // Withdrawing consent mid-request must show the consent panel. Continuing
        // to draw a progress bar would tell the user their refusal did nothing.
        assert_eq!(
            panel(
                false,
                true,
                FontImportJob::Fetch,
                FontImportJobState::Running
            ),
            FontImportPanel::Consent
        );
        assert_eq!(
            panel(false, false, FontImportJob::None, FontImportJobState::Idle),
            FontImportPanel::Consent
        );

        assert_eq!(
            panel(
                true,
                false,
                FontImportJob::Catalogue,
                FontImportJobState::Running
            ),
            FontImportPanel::Loading
        );
        assert_eq!(
            panel(
                true,
                true,
                FontImportJob::Fetch,
                FontImportJobState::Running
            ),
            FontImportPanel::Fetching
        );
        assert_eq!(
            panel(
                true,
                true,
                FontImportJob::Fetch,
                FontImportJobState::Cancelling
            ),
            FontImportPanel::Cancelling
        );
        assert_eq!(
            panel(true, true, FontImportJob::Fetch, FontImportJobState::Failed),
            FontImportPanel::Failed
        );
        assert_eq!(
            panel(
                true,
                true,
                FontImportJob::Catalogue,
                FontImportJobState::TimedOut
            ),
            FontImportPanel::Failed
        );

        // Cancelling a download keeps the list that was already loaded. The user
        // stopped one request; they did not ask to start over.
        assert_eq!(
            panel(
                true,
                true,
                FontImportJob::Fetch,
                FontImportJobState::Cancelled
            ),
            FontImportPanel::Browsing
        );
        assert_eq!(
            panel(true, false, FontImportJob::None, FontImportJobState::Idle),
            FontImportPanel::Loading
        );
    }

    #[test]
    fn font_import_only_accepts_the_result_of_the_job_being_waited_on() {
        let mut nonce = next_nonce(0);
        assert_eq!(nonce, 1);
        assert!(result_is_current(nonce, nonce));

        // The user cancels one download and starts another. The first worker was
        // already writing; its artifact must not become the caption face.
        let superseded = nonce;
        nonce = next_nonce(nonce);
        assert!(!result_is_current(superseded, nonce));
        assert!(result_is_current(nonce, nonce));

        // Zero is never a live job, so a zeroed structure cannot accept anything.
        assert!(!result_is_current(0, 0));
        assert!(!result_is_current(0, nonce));
        assert!(!result_is_current(nonce, 0));

        // Wrapping skips zero rather than reusing it as a live nonce.
        assert_eq!(next_nonce(u64::MAX), 1);
    }

    #[test]
    fn font_import_clears_a_good_outcome_on_its_own_but_never_a_failure() {
        let finished = 1000.0;
        assert!(!outcome_expired(
            FontImportJobState::Succeeded,
            finished,
            finished
        ));
        assert!(outcome_expired(
            FontImportJobState::Succeeded,
            finished,
            finished + SUCCESS_LINGER_SECONDS
        ));
        assert!(outcome_expired(
            FontImportJobState::Cancelled,
            finished,
            finished + SUCCESS_LINGER_SECONDS
        ));

        // A failure is the one thing worth reading. It waits to be dismissed.
        assert!(!outcome_expired(FontImportJobState::Failed, finished, 1e9));
        assert!(!outcome_expired(
            FontImportJobState::TimedOut,
            finished,
            1e9
        ));
        assert!(!outcome_expired(FontImportJobState::Running, finished, 1e9));
        assert!(!outcome_expired(
            FontImportJobState::Succeeded,
            finished,
            0.0
        ));
    }

    #[test]
    fn font_import_job_activity_and_completion_partition_every_state() {
        let mut active = 0usize;
        let mut finished = 0usize;
        for state in FontImportJobState::ALL {
            let is_active = state.is_active();
            let is_finished = state.is_finished();
            // Nothing may be both, or a poll would try to cancel a job that has
            // already published its result.
            assert!(!(is_active && is_finished));
            active += usize::from(is_active);
            finished += usize::from(is_finished);
        }
        assert_eq!(active, 2);
        assert_eq!(finished, 4);
        assert!(!FontImportJobState::Idle.is_active());
        assert!(!FontImportJobState::Idle.is_finished());
    }
}
