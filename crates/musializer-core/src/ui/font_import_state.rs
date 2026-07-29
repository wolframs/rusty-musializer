//! Font import state machine, nonce and staleness handling, and the browser
//! pane's own view state and geometry.
//!
//! **Owner: Agent F, then Agent K.** Port of `font_import_state.c`/`.h`, and of
//! the raylib-free half of `draw_font_browser` (`lyrics_editor_ui.c:650-833`)
//! from the frozen C oracle at `../musializer` (commit `9300af9`, read-only).
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

// ---------------------------------------------------------------------------
// The browser pane's view state and geometry.
//
// `lyrics_editor_ui.c` keeps these in `Lyric_Editor` (`lyrics_editor_ui.h:64-72`)
// and computes the rectangles inline. They are here instead for the reason this
// crate exists: a pane that refuses to draw below a threshold, and a list window
// that has to agree with the row loop that fills it, are both assertable without
// a window — and the C's own comment on `FONT_BROWSER_MIN_HEIGHT` records that
// the neighbouring style form "spent a release refusing to draw at every size
// because its threshold was a guess".
// ---------------------------------------------------------------------------

/// Below this the pane draws one sentence instead of a browser
/// (`FONT_BROWSER_MIN_HEIGHT`, `lyrics_editor_ui.c:583`).
pub const BROWSER_MIN_HEIGHT: f32 = 150.0;
/// `FONT_BROWSER_MIN_WIDTH` (`:584`).
pub const BROWSER_MIN_WIDTH: f32 = 420.0;
/// `FONT_BROWSER_ROW_HEIGHT` (`:585`). The stride; a row is drawn 2 px shorter.
pub const BROWSER_ROW_HEIGHT: f32 = 26.0;
/// The C's `indices[64]` (`:728`), which caps how many rows one frame collects
/// however tall the pane is. It is what keeps scrolling a catalogue of eighteen
/// hundred families from ever walking more than a screenful.
pub const BROWSER_MAX_VISIBLE_ROWS: usize = 64;
/// The longest query the search field will hold, in bytes.
///
/// `font_query` is `char[FONT_CATALOGUE_FAMILY_CAPACITY]` and the input guard is
/// `length + 1 < sizeof(...)` (`lyrics_editor_ui.c:610`), so 63 bytes fit. A
/// query longer than the longest family name it could match is not a limit
/// anybody reaches by accident.
pub const BROWSER_QUERY_MAX_BYTES: usize = 63;

/// Where the browser's parts go inside the pane it was given
/// (`draw_font_browser`, `lyrics_editor_ui.c:713-816`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrowserLayout {
    /// The search field. 160 px narrower than the pane, which is the room the
    /// "N of M" count needs on the same line.
    pub search: (f32, f32, f32, f32),
    /// The scrolling list: everything left between the search field and the
    /// action row.
    pub list: (f32, f32, f32, f32),
    /// The "Download and use" button.
    pub action: (f32, f32, f32, f32),
    /// How many rows fit, already capped at [`BROWSER_MAX_VISIBLE_ROWS`].
    pub visible_rows: usize,
}

impl BrowserLayout {
    /// `None` when the pane is too small to host a browser at all
    /// (`lyrics_editor_ui.c:654-658`), which is the layout rule this repository
    /// has already paid for: a control that does not fit is not drawn, it is
    /// replaced by a sentence saying why.
    #[must_use]
    pub fn measure(x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        if height < BROWSER_MIN_HEIGHT || width < BROWSER_MIN_WIDTH {
            return None;
        }
        let list_top = y + 36.0;
        let list_bottom = y + height - 38.0;
        let visible_rows = if list_bottom > list_top {
            ((list_bottom - list_top) / BROWSER_ROW_HEIGHT) as usize
        } else {
            0
        };
        Some(Self {
            search: (x, y, width - 160.0, 28.0),
            list: (x, list_top, width, (list_bottom - list_top).max(0.0)),
            action: (x, y + height - 32.0, 150.0, 30.0),
            visible_rows: visible_rows.min(BROWSER_MAX_VISIBLE_ROWS),
        })
    }
}

/// The browser pane's own state between frames
/// (`Lyric_Editor`'s font fields, `lyrics_editor_ui.h:59-72`).
///
/// **The consent flag lives here and is deliberately not persisted.** Nothing in
/// this type is serialized, `Default` starts it `false`, and there is no setter
/// that takes a stored value — consent to contact a third party is asked once per
/// run, not remembered until somebody withdraws it. That is the oracle's rule too
/// (`p->font_network_allowed` is a plain `Plug` field, written only by
/// `font_service_allow_network`, `plug.c:1879-1882`), and the consent panel says
/// so in as many words: "Musializer asks again next time it starts."
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserView {
    query: String,
    /// Whether the search field has keyboard focus.
    pub query_active: bool,
    first: usize,
    /// The chosen family, **by name**.
    ///
    /// The C stores a catalogue index and a validity flag, and its own comment
    /// says why that is wrong: "A catalogue refresh can renumber every row, so
    /// the family name is re-resolved rather than the index being trusted across
    /// a reload" (`lyrics_editor_ui.h:68-71`). The implementation never does the
    /// re-resolution — `can_import` only bounds-checks the index
    /// (`lyrics_editor_ui.c:813-814`) — so a refresh between choosing and
    /// pressing downloads a *different family*. Storing the name is the comment's
    /// stated intent and closes it; see the Agent K note in `REWRITE_PLAN.md`.
    selected: Option<String>,
    network_allowed: bool,
}

impl BrowserView {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The search text. Always valid UTF-8 and always ASCII, because
    /// [`Self::type_char`] is the only way in.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Accepts one typed character (`font_query_input`,
    /// `lyrics_editor_ui.c:599-615`).
    ///
    /// Printable ASCII only, and not because of laziness: a family name is ASCII
    /// by the helper's own validation, so anything else could never match and is
    /// simply not accepted into the query. Returns whether it was taken.
    pub fn type_char(&mut self, character: char) -> bool {
        if !('\u{20}'..='\u{7E}').contains(&character) {
            return false;
        }
        if self.query.len() >= BROWSER_QUERY_MAX_BYTES {
            return false;
        }
        self.query.push(character);
        // A narrowed query can leave the window scrolled past the last match.
        self.first = 0;
        true
    }

    /// Deletes one **character**, not one byte (`font_query_backspace`,
    /// `lyrics_editor_ui.c:587-597`).
    ///
    /// The C steps back over UTF-8 continuation bytes for this. Nothing
    /// non-ASCII can get into the query, so the case is unreachable — it is
    /// reproduced anyway because the day somebody relaxes [`Self::type_char`],
    /// half a code point in a `String` is a panic rather than a mojibake.
    pub fn backspace(&mut self) {
        self.query.pop();
        self.first = 0;
    }

    /// The first visible row.
    #[must_use]
    pub fn first(&self) -> usize {
        self.first
    }

    /// A wheel notch, three rows at a time (`lyrics_editor_ui.c:745-750`).
    ///
    /// Positive is up. The floor is applied here; the ceiling needs the match
    /// count and belongs to [`Self::clamp_window`].
    pub fn scroll(&mut self, wheel: f32) {
        if wheel == 0.0 {
            return;
        }
        let moved = self.first as i64 - (wheel * 3.0) as i64;
        self.first = moved.max(0) as usize;
    }

    /// Pulls the window back inside the match list (`lyrics_editor_ui.c:751`).
    ///
    /// Called every frame, because the list shortens under the window whenever
    /// the query narrows.
    pub fn clamp_window(&mut self, matched: usize, visible: usize) {
        if self.first + visible > matched {
            self.first = matched.saturating_sub(visible);
        }
    }

    /// The chosen family, or `None`.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn select(&mut self, family: &str) {
        self.selected = Some(family.to_string());
    }

    /// Forgets the choice, which is what a cleared import has to do: leaving a
    /// selection pointing at a family the project no longer carries is how a
    /// second press downloads something nobody asked for.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Whether this run has been allowed to contact the network.
    #[must_use]
    pub fn network_allowed(&self) -> bool {
        self.network_allowed
    }

    /// Records the consent. One-way and one-run: there is deliberately no
    /// `withdraw`, and deliberately no way to set this from a stored value.
    pub fn allow_network(&mut self) {
        self.network_allowed = true;
    }
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
    fn the_browser_refuses_to_draw_below_the_size_it_was_measured_against() {
        // The threshold is measured against the pane the 960x640 minimum window
        // actually produces, not guessed — the C's comment records that the
        // style form next door "spent a release refusing to draw at every size
        // because its threshold was a guess".
        assert!(BrowserLayout::measure(0.0, 0.0, BROWSER_MIN_WIDTH - 1.0, 400.0).is_none());
        assert!(BrowserLayout::measure(0.0, 0.0, 900.0, BROWSER_MIN_HEIGHT - 1.0).is_none());
        assert!(BrowserLayout::measure(0.0, 0.0, BROWSER_MIN_WIDTH, BROWSER_MIN_HEIGHT).is_some());
    }

    #[test]
    fn the_browsers_list_window_never_overlaps_its_search_field_or_its_action_row() {
        // The failure this catches is the one a capture would show as rows
        // printed through the "Download and use" button: the list takes what is
        // left *after* both, and `visible_rows` has to agree with it or the row
        // loop draws past the bottom.
        for height in [150.0f32, 213.0, 400.0, 2000.0] {
            for width in [420.0f32, 640.0, 1600.0] {
                let layout = BrowserLayout::measure(11.0, 23.0, width, height)
                    .expect("above both thresholds");
                let (_, search_y, search_width, search_height) = layout.search;
                let (_, list_y, _, list_height) = layout.list;
                let (_, action_y, _, _) = layout.action;
                assert!(
                    search_y + search_height <= list_y,
                    "{width}x{height}: the list starts inside the search field"
                );
                assert!(
                    list_y + list_height <= action_y,
                    "{width}x{height}: the list runs into the action row"
                );
                assert!(
                    search_width < width,
                    "{width}x{height}: the search field left no room for the count"
                );
                assert!(
                    layout.visible_rows as f32 * BROWSER_ROW_HEIGHT <= list_height + 0.001,
                    "{width}x{height}: {} rows do not fit in {list_height}px",
                    layout.visible_rows
                );
                assert!(layout.visible_rows <= BROWSER_MAX_VISIBLE_ROWS);
            }
        }
        // A pane tall enough for a thousand rows still collects at most a
        // screenful, which is what keeps an 1,800-family catalogue cheap.
        let tall = BrowserLayout::measure(0.0, 0.0, 640.0, 20_000.0).expect("measure");
        assert_eq!(tall.visible_rows, BROWSER_MAX_VISIBLE_ROWS);
    }

    #[test]
    fn the_search_field_takes_ascii_only_and_stops_at_the_length_it_can_hold() {
        let mut view = BrowserView::new();
        assert!(view.type_char('S'));
        assert!(view.type_char('p'));
        assert!(view.type_char(' '));
        assert_eq!(view.query(), "Sp ");

        // A family name is ASCII by the helper's own validation, so a character
        // that could never match is not accepted rather than accepted and
        // ignored — the difference is whether the field appears broken.
        for refused in ['\u{e9}', '\u{4e00}', '\n', '\t', '\u{7f}'] {
            assert!(!view.type_char(refused), "{refused:?} was accepted");
        }
        assert_eq!(view.query(), "Sp ");

        view.backspace();
        assert_eq!(view.query(), "Sp");
        // Backspacing an empty field is a no-op, not an underflow.
        view.backspace();
        view.backspace();
        view.backspace();
        assert_eq!(view.query(), "");

        while view.type_char('A') {}
        assert_eq!(view.query().len(), BROWSER_QUERY_MAX_BYTES);
    }

    #[test]
    fn the_list_window_follows_the_matches_when_the_query_narrows() {
        // Typing shortens the match list under a window that may be scrolled
        // past its new end. Without the reset and the clamp the browser shows an
        // empty list and looks like it lost the catalogue.
        let mut view = BrowserView::new();
        view.scroll(-30.0); // ninety rows down
        assert_eq!(view.first(), 90);
        view.clamp_window(100, 10);
        assert_eq!(view.first(), 90);
        view.clamp_window(12, 10);
        assert_eq!(view.first(), 2);
        view.clamp_window(3, 10);
        assert_eq!(view.first(), 0);

        view.scroll(-10.0);
        assert!(view.first() > 0);
        view.type_char('x');
        assert_eq!(view.first(), 0, "a narrowed query must return to the top");
        view.scroll(-10.0);
        view.backspace();
        assert_eq!(view.first(), 0);

        // Scrolling up past the top saturates rather than wrapping to a vast
        // index, which is what an unsigned subtraction would have done.
        view.scroll(-1.0);
        view.scroll(100.0);
        assert_eq!(view.first(), 0);
    }

    #[test]
    fn the_selection_is_a_family_name_so_a_refresh_cannot_retarget_it() {
        // The C keeps an index into a catalogue that a refresh renumbers, which
        // its own header comment says it must not do. A name survives the
        // refresh or stops matching; it never silently becomes a different face.
        let mut view = BrowserView::new();
        assert_eq!(view.selected(), None);
        view.select("Space Mono");
        assert_eq!(view.selected(), Some("Space Mono"));
        view.clear_selection();
        assert_eq!(view.selected(), None);
    }

    #[test]
    fn network_consent_starts_refused_every_run_and_cannot_be_restored() {
        // The whole point of the flag. It is asked once per run and never
        // written anywhere, so a fresh view is always a fresh question — which
        // is what the consent panel promises in as many words.
        let mut view = BrowserView::new();
        assert!(!view.network_allowed());
        view.allow_network();
        assert!(view.network_allowed());
        // A new run is a new question. There is no constructor, setter or
        // serialization that could carry the answer across one.
        assert!(!BrowserView::new().network_allowed());
        assert!(!BrowserView::default().network_allowed());

        // And the panel that consent gates is the consent panel, whatever else
        // is in flight.
        assert_eq!(
            panel(
                view.network_allowed(),
                false,
                FontImportJob::None,
                FontImportJobState::Idle
            ),
            FontImportPanel::Loading
        );
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
