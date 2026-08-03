//! The notice queue: transient toasts and persistent failure cards.
//!
//! **Owner: Agent F.** Port of `ui_notice.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! The queue is bounded at [`NOTICE_CAPACITY`] and never allocates unboundedly,
//! because the interesting case is a burst: an export that fails once per frame
//! must not push the one notice the user has to act on out of the list. Hence the
//! overflow policy in [`NoticeQueue::push`] — a full queue evicts its
//! *lowest-priority* member, and refuses the newcomer outright when everything
//! present outranks it. Chatter is dropped; actionable failures survive.
//!
//! C's fixed `char` buffers become [`String`], but the length limits stay as
//! validation rather than as allocation bounds. They are measured in **bytes**
//! (`str::len`), because the C capacities count bytes including the terminator;
//! a `chars().count()` check would accept multi-byte strings the C rejects and
//! quietly diverge from the oracle.
//!
//! `revision` is the only thing the drawing code should watch: it changes exactly
//! when the visible list changes, which is what lets a redraw be skipped.

/// Notices held at once (`ui_notice.h:7`).
pub const NOTICE_CAPACITY: usize = 16;
/// Title byte limit, including C's terminator (`ui_notice.h:8`), so a valid title
/// satisfies `title.len() < TITLE_CAPACITY`.
pub const TITLE_CAPACITY: usize = 80;
/// Detail byte limit, including C's terminator (`ui_notice.h:9`).
pub const DETAIL_CAPACITY: usize = 320;
/// Path byte limit, including C's terminator (`ui_notice.h:10`). 1025 is
/// `PATH_MAX` plus the NUL.
pub const PATH_CAPACITY: usize = 1025;

/// Notice severity (`ui_notice.h:12-13`).
///
/// `Default` is `Info` because C's zero-initialised `Ui_Notice_Spec` gets
/// `UI_NOTICE_INFO == 0`. C's trailing `UI_NOTICE_SEVERITY_COUNT` has no Rust
/// counterpart, which is also why the range check in `ui_notice.c:44` disappears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Severity {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl Severity {
    /// The badge text drawn on the notice card (`ui_notice.c:113-123`).
    ///
    /// User-visible strings; note `Success` reads `"DONE"`, not `"SUCCESS"`. C's
    /// `"NOTICE"` fallback covered `UI_NOTICE_SEVERITY_COUNT` and an out-of-range
    /// cast, neither of which can occur here.
    pub const fn label(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Success => "DONE",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }
}

/// Seconds a routine notice stays on screen. The one number the shell used for
/// everything.
pub const INFO_DWELL_SECONDS: f64 = 6.0;
/// A warning is read, not glanced at: something is off but nothing failed, and
/// six seconds is not enough to finish the sentence and decide whether to care.
pub const WARNING_DWELL_SECONDS: f64 = 14.0;

/// How long a notice of a given severity stays on screen.
///
/// **Where the fix for review 1.11 (UX0-A11) lives.** All ~51 `notify` call sites
/// in the shell hard-coded `persistent: false, 6.0` while this queue supported
/// persistence correctly, so an export that failed twenty minutes into a render
/// was gone long before the user came back to the machine. Deriving the lifetime
/// from the severity puts the policy at the seam instead of in fifty-one places
/// that would each have to be found and changed again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dwell {
    pub persistent: bool,
    /// Ignored when `persistent`, but still finite and non-negative so
    /// [`NoticeQueue::push`]'s validation accepts it either way.
    pub duration_seconds: f64,
}

impl Severity {
    /// The lifetime a notice of this severity gets unless a caller overrides it.
    ///
    /// Only [`Severity::Error`] is persistent. A failure is the one class of
    /// notice the user has to *act* on, and it is also the class most likely to
    /// arrive while nobody is looking at the screen; everything else is a
    /// confirmation of something the user just did and can expire.
    pub const fn dwell(self) -> Dwell {
        match self {
            Severity::Error => Dwell {
                persistent: true,
                duration_seconds: 0.0,
            },
            Severity::Warning => Dwell {
                persistent: false,
                duration_seconds: WARNING_DWELL_SECONDS,
            },
            Severity::Info | Severity::Success => Dwell {
                persistent: false,
                duration_seconds: INFO_DWELL_SECONDS,
            },
        }
    }
}

/// What a clipped last line ends with.
pub const DETAIL_ELLIPSIS: &str = "\u{2026}";

/// Wraps a notice's detail to `max_width`, at most `max_lines`, ellipsizing the
/// last line when there is more text than fits.
///
/// **Review 1.11 (UX0-A11).** Detail text was drawn as one unwrapped, unclipped
/// line on a card at most 380 px wide, while the strings the application actually
/// produces run 90 to 150+ characters — so a real failure message ran off the
/// card, across the preview, and past the window edge. Wrapping it is not enough
/// on its own: without a line cap one notice could take the whole preview, so the
/// remainder is dropped *visibly*, with an ellipsis, rather than silently.
///
/// `measure` is the caller's text measurement, which is why this is here and not
/// beside the drawing: the shell's font bank needs a window, and the wrapping
/// rules — the long word with no break in it, the line that fits exactly, the
/// ellipsis that must itself fit — are exactly what a capture cannot check.
///
/// A word wider than `max_width` on its own is broken by characters rather than
/// left to overflow. That is the case a naive wrapper gets wrong and the one this
/// interface reaches every time a detail names a path.
pub fn wrap_detail<M>(text: &str, max_width: f32, max_lines: usize, mut measure: M) -> Vec<String>
where
    M: FnMut(&str) -> f32,
{
    let mut lines: Vec<String> = Vec::new();
    if text.is_empty() || max_lines == 0 || !max_width.is_finite() || max_width <= 0.0 {
        return lines;
    }

    // `split_whitespace` rather than `split(' ')`: a detail carrying a newline or
    // a tab would otherwise measure as one enormous word and be hard-broken.
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let mut current = String::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if current.is_empty() {
            if measure(word) <= max_width {
                current.push_str(word);
                index += 1;
            } else {
                // Always consumes at least one character, so this terminates even
                // when a single glyph is wider than the card.
                let take = longest_prefix(word, max_width, &mut measure);
                lines.push(word[..take].to_string());
                words[index] = &word[take..];
            }
            continue;
        }
        let candidate = format!("{current} {word}");
        if measure(&candidate) <= max_width {
            current = candidate;
            index += 1;
        } else {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }

    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.pop() {
            lines.push(ellipsize(&last, max_width, &mut measure));
        }
    }
    lines
}

/// The longest prefix of `word` that fits, in bytes, never zero.
fn longest_prefix<M>(word: &str, max_width: f32, measure: &mut M) -> usize
where
    M: FnMut(&str) -> f32,
{
    let mut fits = 0;
    for (offset, character) in word.char_indices() {
        let end = offset + character.len_utf8();
        if measure(&word[..end]) > max_width {
            break;
        }
        fits = end;
    }
    if fits == 0 {
        // Nothing fits at all. Emit one character anyway rather than looping
        // forever; the caller's clip is what keeps it honest.
        word.chars()
            .next()
            .map_or(word.len(), |character| character.len_utf8())
    } else {
        fits
    }
}

/// `line` with an ellipsis, shortened until the pair fits.
fn ellipsize<M>(line: &str, max_width: f32, measure: &mut M) -> String
where
    M: FnMut(&str) -> f32,
{
    let mut end = line.len();
    loop {
        let candidate = format!("{}{DETAIL_ELLIPSIS}", line[..end].trim_end());
        if measure(&candidate) <= max_width || end == 0 {
            return candidate;
        }
        // Step back one character, not one byte.
        end = line[..end]
            .char_indices()
            .next_back()
            .map_or(0, |(offset, _)| offset);
    }
}

/// Outcome of a queue operation (`ui_notice.h:39-42`), in C's declaration order
/// so [`NoticeResult::result_string`] stays the same table.
///
/// [`NoticeResult::Dropped`] is **not** an error: it reports a successful
/// application of the overflow policy. Folding it into an error type would make
/// "the queue is busy with worse news" indistinguishable from "your call was
/// wrong".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeResult {
    Ok,
    /// The queue was full and every notice in it outranked the newcomer.
    Dropped,
    /// A required string was absent. In C this also covered a null queue or spec
    /// pointer; here only a missing title can produce it.
    ErrorNull,
    ErrorInvalid,
    ErrorStringTooLong,
    ErrorNotFound,
    ErrorIdExhausted,
}

impl NoticeResult {
    /// The exact C message for each variant (`ui_notice.c:105-111`). These reach
    /// logs and the status line, so they are a compatibility surface.
    pub const fn result_string(self) -> &'static str {
        match self {
            NoticeResult::Ok => "ok",
            NoticeResult::Dropped => "notice dropped by overflow policy",
            NoticeResult::ErrorNull => "null argument",
            NoticeResult::ErrorInvalid => "invalid notice or queue",
            NoticeResult::ErrorStringTooLong => "notice string exceeds fixed capacity",
            NoticeResult::ErrorNotFound => "notice not found",
            NoticeResult::ErrorIdExhausted => "notice id space exhausted",
        }
    }
}

/// A queued notice (`ui_notice.h:14-22`).
#[derive(Clone, Debug)]
pub struct Notice {
    pub id: u64,
    pub severity: Severity,
    pub persistent: bool,
    /// Zero for a persistent notice; otherwise counts down in [`NoticeQueue::tick`].
    pub remaining_seconds: f64,
    pub title: String,
    pub detail: String,
    /// A file the user can go look at — empty when there is none. Its presence
    /// alone raises the notice's priority, because a notice pointing at a log is
    /// one the user can act on.
    pub path: String,
}

impl Notice {
    /// A failure the user can do something about (`ui_notice.c:30-33`): an error
    /// that either sticks around or names a path. C's null check disappears with
    /// the pointer.
    pub fn is_actionable_failure(&self) -> bool {
        self.severity == Severity::Error && (self.persistent || !self.path.is_empty())
    }
}

/// A request to enqueue a notice (`ui_notice.h:23-30`).
///
/// `title` is [`Option`] because C distinguishes the two failures: a null title
/// is [`NoticeResult::ErrorNull`], while an *empty* one is
/// [`NoticeResult::ErrorStringTooLong`] (`ui_notice.c:47-48`, via `bounded`'s
/// `empty` flag). `detail` and `path` may be empty; C accepted null for those and
/// stored an empty string, which `""` models exactly.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoticeSpec<'a> {
    pub severity: Severity,
    pub persistent: bool,
    /// Ignored when `persistent`; otherwise must be finite and strictly positive.
    pub duration_seconds: f64,
    pub title: Option<&'a str>,
    pub detail: &'a str,
    pub path: &'a str,
}

/// Eviction rank (`ui_notice.c:22-29`). Higher survives.
///
/// The ladder is not simply severity-ordered: an INFO or SUCCESS that is
/// persistent or carries a path (2) outranks a plain WARNING (1). That is
/// intentional — "here is the file I wrote" is more useful than "heads up".
fn priority(severity: Severity, persistent: bool, has_path: bool) -> u32 {
    if severity == Severity::Error && (persistent || has_path) {
        return 5;
    }
    if severity == Severity::Error {
        return 4;
    }
    if severity == Severity::Warning && (persistent || has_path) {
        return 3;
    }
    if persistent || has_path {
        return 2;
    }
    if severity == Severity::Warning {
        1
    } else {
        0
    }
}

/// The bounded notice list (`ui_notice.h:31-38`).
#[derive(Clone, Debug)]
pub struct NoticeQueue {
    notices: Vec<Notice>,
    next_id: u64,
    revision: u64,
    evicted_count: u64,
    dropped_count: u64,
}

impl Default for NoticeQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl NoticeQueue {
    /// `ui_notice_queue_init` (`ui_notice.c:34-39`): `next_id` and `revision` both
    /// start at 1, so 0 can mean "no id" and "never rendered".
    pub fn new() -> Self {
        Self {
            notices: Vec::new(),
            next_id: 1,
            revision: 1,
            evicted_count: 0,
            dropped_count: 0,
        }
    }

    /// The notices in insertion order, oldest first.
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// C's `count`.
    pub fn len(&self) -> usize {
        self.notices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notices.is_empty()
    }

    /// Changes exactly when the visible list changes; never 0 after
    /// initialisation, so 0 remains usable as "not yet observed".
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The id the next accepted notice will be given. 0 once the space is spent.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// How many notices the overflow policy has removed to make room.
    pub fn evicted_count(&self) -> u64 {
        self.evicted_count
    }

    /// How many incoming notices the overflow policy has refused.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count
    }

    /// `bump` (`ui_notice.c:5`): wraps but skips 0, so the sentinel stays
    /// distinguishable after 2^64 changes.
    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.revision = 1;
        }
    }

    /// `ui_notice_push` (`ui_notice.c:40-72`). Returns the C result plus the id
    /// the notice was given, which is `Some` only for [`NoticeResult::Ok`] —
    /// mirroring C's `out` parameter, which it zeroes on every other path.
    ///
    /// Validation order is C's, and it matters: an invalid duration is reported
    /// before an over-long title.
    pub fn push(&mut self, spec: &NoticeSpec) -> (NoticeResult, Option<u64>) {
        // `ui_notice.c:44-46`. C also range-checked the severity enum and
        // guarded `count > CAPACITY` against a corrupted queue; the enum and the
        // private `Vec` make both unreachable here.
        if !spec.duration_seconds.is_finite() || (!spec.persistent && spec.duration_seconds <= 0.0)
        {
            return (NoticeResult::ErrorInvalid, None);
        }
        // `ui_notice.c:47-48`: a missing title is a different bug from a title
        // that does not fit, and the two results say so.
        let title = match spec.title {
            None => return (NoticeResult::ErrorNull, None),
            Some(title) if title.is_empty() || title.len() >= TITLE_CAPACITY => {
                return (NoticeResult::ErrorStringTooLong, None)
            }
            Some(title) => title,
        };
        // `ui_notice.c:49-50`. Byte lengths, not character counts: see the module
        // docs.
        if spec.detail.len() >= DETAIL_CAPACITY || spec.path.len() >= PATH_CAPACITY {
            return (NoticeResult::ErrorStringTooLong, None);
        }
        if self.next_id == 0 {
            return (NoticeResult::ErrorIdExhausted, None);
        }

        let incoming = priority(spec.severity, spec.persistent, !spec.path.is_empty());
        if self.notices.len() == NOTICE_CAPACITY {
            // `ui_notice.c:53-64`: find the lowest-priority resident, preferring
            // the *oldest* on a tie because the scan only replaces `lowest` on a
            // strict `<`.
            let mut candidate = 0;
            let mut lowest = priority(
                self.notices[0].severity,
                self.notices[0].persistent,
                !self.notices[0].path.is_empty(),
            );
            for (index, notice) in self.notices.iter().enumerate().skip(1) {
                let p = priority(notice.severity, notice.persistent, !notice.path.is_empty());
                if p < lowest {
                    lowest = p;
                    candidate = index;
                }
            }
            // Strictly greater: an equal-priority newcomer wins, so a stream of
            // like-for-like notices still shows the most recent one.
            if lowest > incoming {
                self.dropped_count += 1;
                return (NoticeResult::Dropped, None);
            }
            self.notices.remove(candidate);
            self.evicted_count += 1;
        }

        let id = self.next_id;
        // `ui_notice.c:67`: the id space is spent rather than reused, and the next
        // push fails with `ErrorIdExhausted` instead of handing out a duplicate.
        self.next_id = if id == u64::MAX { 0 } else { id + 1 };
        self.notices.push(Notice {
            id,
            severity: spec.severity,
            persistent: spec.persistent,
            remaining_seconds: if spec.persistent {
                0.0
            } else {
                spec.duration_seconds
            },
            title: title.to_string(),
            detail: spec.detail.to_string(),
            path: spec.path.to_string(),
        });
        self.bump();
        (NoticeResult::Ok, Some(id))
    }

    /// `ui_notice_dismiss` (`ui_notice.c:73-81`). Id 0 is invalid, not merely
    /// absent, because it is the sentinel a caller gets when a push failed.
    pub fn dismiss(&mut self, id: u64) -> NoticeResult {
        if id == 0 {
            return NoticeResult::ErrorInvalid;
        }
        if let Some(index) = self.notices.iter().position(|notice| notice.id == id) {
            self.notices.remove(index);
            self.bump();
            return NoticeResult::Ok;
        }
        NoticeResult::ErrorNotFound
    }

    /// `ui_notice_tick` (`ui_notice.c:82-94`): ages the timed notices by
    /// `delta_seconds` and removes the expired ones.
    ///
    /// Persistent notices are not touched at all — not even decremented — so a
    /// failure card outlives any amount of elapsed time. `revision` advances only
    /// if something was actually removed.
    pub fn tick(&mut self, delta_seconds: f64) -> NoticeResult {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return NoticeResult::ErrorInvalid;
        }
        let mut changed = false;
        let mut index = 0;
        while index < self.notices.len() {
            if !self.notices[index].persistent {
                self.notices[index].remaining_seconds -= delta_seconds;
                // `<= 0`, so a notice ticked by exactly its duration expires now.
                if self.notices[index].remaining_seconds <= 0.0 {
                    self.notices.remove(index);
                    changed = true;
                    continue;
                }
            }
            index += 1;
        }
        if changed {
            self.bump();
        }
        NoticeResult::Ok
    }

    /// `ui_notice_clear` (`ui_notice.c:95-98`). Clearing an empty queue is not a
    /// change, so it does not advance `revision`.
    pub fn clear(&mut self) {
        if !self.notices.is_empty() {
            self.notices.clear();
            self.bump();
        }
    }

    /// `ui_notice_find` (`ui_notice.c:99-104`). Id 0 never matches.
    pub fn find(&self, id: u64) -> Option<&Notice> {
        if id == 0 {
            return None;
        }
        self.notices.iter().find(|notice| notice.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of the C tests' `timed` helper.
    fn timed(severity: Severity, title: &str) -> NoticeSpec<'_> {
        NoticeSpec {
            severity,
            duration_seconds: 2.0,
            title: Some(title),
            ..NoticeSpec::default()
        }
    }

    /// Port of the C tests' `failure` helper: the actionable case, persistent and
    /// carrying a log path.
    fn failure(title: &str) -> NoticeSpec<'_> {
        NoticeSpec {
            severity: Severity::Error,
            persistent: true,
            title: Some(title),
            detail: "Previous state preserved",
            path: "/tmp/job.log",
            ..NoticeSpec::default()
        }
    }

    /// A fixed-pitch stand-in for the real font, so the wrapping tests state
    /// character counts rather than pixel widths nobody can check by reading.
    fn monospace(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn severity_decides_how_long_a_notice_lives() {
        // Review 1.11 (UX0-A11). The table, pinned: a 20-minute export that fails
        // while the user is away has to still be on screen when they come back,
        // and a "Saved" toast must not be.
        assert_eq!(
            Severity::Error.dwell(),
            Dwell {
                persistent: true,
                duration_seconds: 0.0
            }
        );
        assert!(!Severity::Warning.dwell().persistent);
        assert!(Severity::Warning.dwell().duration_seconds > INFO_DWELL_SECONDS);
        for severity in [Severity::Info, Severity::Success] {
            assert_eq!(
                severity.dwell(),
                Dwell {
                    persistent: false,
                    duration_seconds: INFO_DWELL_SECONDS
                }
            );
        }
        // And the queue accepts every one of them: a persistent spec still has to
        // carry a finite duration or `push` rejects it as invalid.
        let mut queue = NoticeQueue::new();
        for severity in [
            Severity::Info,
            Severity::Success,
            Severity::Warning,
            Severity::Error,
        ] {
            let dwell = severity.dwell();
            let result = queue.push(&NoticeSpec {
                severity,
                persistent: dwell.persistent,
                duration_seconds: dwell.duration_seconds,
                title: Some("Title"),
                detail: "",
                path: "",
            });
            assert_eq!(result.0, NoticeResult::Ok, "{severity:?} was refused");
        }
        // An hour later only the failure is left.
        assert_eq!(queue.tick(3600.0), NoticeResult::Ok);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.notices()[0].severity, Severity::Error);
    }

    #[test]
    fn wrapping_handles_the_degenerate_inputs_without_producing_a_line() {
        assert!(wrap_detail("", 100.0, 3, monospace).is_empty());
        assert!(wrap_detail("something", 100.0, 0, monospace).is_empty());
        assert!(wrap_detail("something", 0.0, 3, monospace).is_empty());
        assert!(wrap_detail("something", f32::NAN, 3, monospace).is_empty());
    }

    #[test]
    fn a_detail_that_already_fits_is_left_exactly_alone() {
        assert_eq!(wrap_detail("short", 100.0, 3, monospace), vec!["short"]);
        // Exactly the width, which is the boundary a `<` would get wrong: ten
        // characters at 10 px each against a 100 px card.
        assert_eq!(
            wrap_detail("abcdefghij", 100.0, 3, monospace),
            vec!["abcdefghij"]
        );
        // And one character more is two lines rather than an overflow.
        assert_eq!(
            wrap_detail("abcdefghij k", 100.0, 3, monospace),
            vec!["abcdefghij", "k"]
        );
    }

    #[test]
    fn a_long_detail_wraps_and_says_where_it_was_cut() {
        // 150 characters is the size the real failure strings reach; the card
        // this is measured against fits 34 of them.
        let detail = "the render helper exited with status 1 and the previous \
             output file was preserved unchanged at its original path on disk xy";
        assert_eq!(detail.len(), 123);
        let lines = wrap_detail(detail, 340.0, 2, monospace);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(
                monospace(line) <= 340.0,
                "a wrapped line is still too wide: {line:?}"
            );
        }
        assert!(
            lines[1].ends_with(DETAIL_ELLIPSIS),
            "the clipped remainder has to be visible: {:?}",
            lines[1]
        );
        // Unbounded, the same text is more than two lines — so the cap is doing
        // something and the ellipsis is not decorative.
        assert!(wrap_detail(detail, 340.0, 16, monospace).len() > 2);
    }

    #[test]
    fn a_word_with_no_break_in_it_is_broken_rather_than_left_to_overflow() {
        // Every detail that names a path reaches this.
        let path = "/home/somebody/Music/very-long-project-directory/track-01.musi";
        let lines = wrap_detail(path, 200.0, 4, monospace);
        assert!(lines.len() > 1, "the token was not broken at all");
        for line in &lines {
            assert!(monospace(line) <= 200.0, "{line:?} overflows the card");
        }
        // Nothing is lost except through the ellipsis: the pieces rejoin.
        assert_eq!(lines.concat(), path);

        // A card narrower than one glyph still terminates and still emits.
        let lines = wrap_detail("abc", 1.0, 4, monospace);
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn ui_notice_expiry_preserves_persistent_failures() {
        let mut q = NoticeQueue::new();
        let transient = timed(Severity::Success, "Saved");
        let error = failure("Render failed");
        let (result, transient_id) = q.push(&transient);
        assert_eq!(result, NoticeResult::Ok);
        let transient_id = transient_id.unwrap();
        let (result, error_id) = q.push(&error);
        assert_eq!(result, NoticeResult::Ok);
        let error_id = error_id.unwrap();
        assert_eq!(q.tick(2.0), NoticeResult::Ok);
        assert!(q.find(transient_id).is_none());
        let kept = q.find(error_id).expect("the failure must survive expiry");
        assert!(kept.is_actionable_failure());
        assert_eq!(kept.detail, "Previous state preserved");
    }

    #[test]
    fn ui_notice_dismiss_and_clear_advance_revision() {
        let mut q = NoticeQueue::new();
        let item = timed(Severity::Info, "One");
        let (result, id) = q.push(&item);
        assert_eq!(result, NoticeResult::Ok);
        let id = id.unwrap();
        let revision = q.revision();
        assert_eq!(q.dismiss(id), NoticeResult::Ok);
        assert_eq!(q.len(), 0);
        assert_ne!(q.revision(), revision);
        assert_eq!(q.dismiss(id), NoticeResult::ErrorNotFound);
        assert_eq!(q.push(&item).0, NoticeResult::Ok);
        q.clear();
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn ui_notice_invalid_input_is_atomic() {
        let mut q = NoticeQueue::new();
        let revision = q.revision();
        let mut item = timed(Severity::Info, "Good");
        item.duration_seconds = f64::NAN;
        assert_eq!(q.push(&item).0, NoticeResult::ErrorInvalid);
        item.duration_seconds = 1.0;
        item.title = Some("");
        // An empty title is "does not fit" rather than "absent": C's `bounded`
        // rejects it through the same path as an over-long one.
        assert_eq!(q.push(&item).0, NoticeResult::ErrorStringTooLong);
        assert_eq!(q.len(), 0);
        assert_eq!(q.revision(), revision);
        assert_eq!(q.tick(-1.0), NoticeResult::ErrorInvalid);
    }

    #[test]
    fn ui_notice_overflow_eviction_prefers_transient_chatter() {
        let mut q = NoticeQueue::new();
        let error = failure("Important");
        let (result, error_id) = q.push(&error);
        assert_eq!(result, NoticeResult::Ok);
        let error_id = error_id.unwrap();
        for _ in 1..NOTICE_CAPACITY {
            let info = timed(Severity::Info, "Update");
            assert_eq!(q.push(&info).0, NoticeResult::Ok);
        }
        let newest = timed(Severity::Success, "Done");
        assert_eq!(q.push(&newest).0, NoticeResult::Ok);
        assert!(q.find(error_id).is_some());
        assert_eq!(q.evicted_count(), 1);
        assert_eq!(q.dropped_count(), 0);
    }

    #[test]
    fn ui_notice_overflow_drops_low_priority_before_failures() {
        let mut q = NoticeQueue::new();
        for _ in 0..NOTICE_CAPACITY {
            let item = failure("Failure");
            assert_eq!(q.push(&item).0, NoticeResult::Ok);
        }
        let oldest = q.notices()[0].id;
        let info = timed(Severity::Info, "Routine");
        assert_eq!(q.push(&info).0, NoticeResult::Dropped);
        assert!(q.find(oldest).is_some());
        assert_eq!(q.dropped_count(), 1);
    }

    #[test]
    fn ui_notice_overflow_preserves_most_recent_actionable_failures() {
        let mut q = NoticeQueue::new();
        for _ in 0..NOTICE_CAPACITY {
            let item = failure("Failure");
            assert_eq!(q.push(&item).0, NoticeResult::Ok);
        }
        let oldest = q.notices()[0].id;
        let newest = failure("Newest failure");
        let (result, newest_id) = q.push(&newest);
        assert_eq!(result, NoticeResult::Ok);
        assert!(q.find(oldest).is_none());
        assert!(q.find(newest_id.unwrap()).is_some());
        assert_eq!(q.evicted_count(), 1);
    }

    #[test]
    fn ui_notice_rejects_unterminated_bounded_strings() {
        let mut q = NoticeQueue::new();
        // The C test hands `push` an 80-byte buffer with no NUL in it. A Rust
        // `String` is never unterminated, so the equivalent defect is a title
        // whose byte length reaches the capacity that the terminator needed.
        let title = "x".repeat(TITLE_CAPACITY);
        let item = timed(Severity::Info, &title);
        assert_eq!(q.push(&item).0, NoticeResult::ErrorStringTooLong);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn title_capacity_counts_bytes_not_characters() {
        // Not a C test: it guards the porting trap. 40 two-byte characters are 80
        // bytes, which the C rejects, while `chars().count() == 40` would pass.
        let mut q = NoticeQueue::new();
        let title = "é".repeat(TITLE_CAPACITY / 2);
        assert_eq!(title.chars().count(), 40);
        assert_eq!(title.len(), TITLE_CAPACITY);
        assert_eq!(
            q.push(&timed(Severity::Info, &title)).0,
            NoticeResult::ErrorStringTooLong
        );
        // One character shorter leaves room for C's terminator.
        let title = "é".repeat(TITLE_CAPACITY / 2 - 1);
        assert_eq!(q.push(&timed(Severity::Info, &title)).0, NoticeResult::Ok);
    }
}
