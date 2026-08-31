//! What the job log says the helper died of.
//!
//! `tools/external_analysis.py` already knows precisely why a run failed and
//! says so on the one stderr line it prints before returning 1
//! (`tools/external_analysis.py:2469`): `could not start {name}: {error}`,
//! `{name} exceeded its {t}s timeout`, `{name} exited with code {n}`, or
//! whatever the refusal was. The desktop wrote that line to the job log and
//! then reported **one fixed sentence for every cause**, so "whisper.cpp is
//! missing" and "the model is corrupt" arrived as the same toast (audit B2).
//! The diagnosis existed the whole time and was thrown away one function short
//! of the user.
//!
//! Everything here is pure and total over arbitrary bytes, because the input is
//! **untrusted**: a job log carries child output, provider diagnostics and the
//! track's own lyrics. Two properties this module owes its caller, both pinned
//! by tests:
//!
//! - Nothing it returns contains a control, newline or bidi-format character.
//!   A newline in a toast is a line that overprints the one under it; a
//!   right-to-left override reverses the rest of the card including the label
//!   beside it.
//! - [`notice_detail`] is always **shorter than [`DETAIL_CAPACITY`]**. That
//!   bound is not cosmetic: [`NoticeQueue::push`](crate::ui::notice::NoticeQueue::push)
//!   *refuses* an over-long detail and the caller drops the result, so one byte
//!   too many means the failure is never reported at all — which is the same
//!   defect B2 is about, one layer down.

use crate::ui::notice::DETAIL_CAPACITY;

/// ASCII dots rather than U+2026, for `ui/panels/assist.rs::ellipsize`'s
/// reason: the icon face is the only face with Private Use Area coverage, and a
/// glyph the UI face happens not to carry draws as an empty box — which makes a
/// shortened sentence look like a broken one.
const ELLIPSIS: &str = "...";

/// The helper's one diagnostic line (`tools/external_analysis.py:2469`). Pinned
/// on the Python side by `tests/test_cross_language_pins.py`.
pub const FAILURE_PREFIX: &str = "External analysis failed: ";

/// Bytes of the helper's own sentence kept. Long enough for a tool name, a
/// numeric code and an `errno` phrase; short enough that the log path still
/// fits beside it inside [`DETAIL_CAPACITY`].
pub const CAUSE_MAX_BYTES: usize = 160;

/// Characters of one log line examined. A cause sentence longer than this is
/// not a sentence, and the cap keeps the scan's cost a function of nothing the
/// helper's children can grow.
const SCAN_LIMIT: usize = 4096;

/// The title used when the log names no cause at all — an empty log, a killed
/// process, a helper that died before it could print.
pub const UNDIAGNOSED_HEADLINE: &str = "Analysis failed";

/// The sentence that used to be shown for *every* failure (audit B2). It is
/// still right when there is genuinely nothing to say.
pub const UNDIAGNOSED_SENTENCE: &str = "The helper exited before producing a validated result.";

/// Which of the helper's four failure shapes the log line describes.
///
/// The point of the enum is the headline: a toast that says the same thing for
/// a missing binary and a corrupt model tells the user to go and read a file,
/// which is what the log path was already for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCause {
    /// `could not start {name}: {error}` — the executable is absent, is not
    /// executable, or its interpreter is missing.
    ToolMissing,
    /// `{name} exceeded its {t}s timeout`.
    ToolTimedOut,
    /// `{name} exited with code {n}`.
    ToolFailed,
    /// Anything else the helper refused with: a validation error, a bridge it
    /// would not write, a document it could not parse.
    Invalid,
}

impl FailureCause {
    /// The notice title. Distinct per cause — that *is* the fix — and under
    /// [`TITLE_CAPACITY`](crate::ui::notice::TITLE_CAPACITY).
    pub const fn headline(self) -> &'static str {
        match self {
            FailureCause::ToolMissing => "A tool the analysis needs could not run",
            FailureCause::ToolTimedOut => "A step of the analysis timed out",
            FailureCause::ToolFailed => "A step of the analysis failed",
            FailureCause::Invalid => "The analysis could not produce a valid result",
        }
    }
}

/// A cause read back out of a job log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperDiagnosis {
    pub cause: FailureCause,
    /// The helper's own words, sanitized and clipped to [`CAUSE_MAX_BYTES`].
    /// Kept verbatim otherwise — a tool name is a thing the user has to type
    /// back, so it is not capitalized or reworded.
    pub reported: String,
}

/// Reads the last cause the helper printed out of a bounded log tail.
///
/// The **last** occurrence, because the helper prints this line immediately
/// before returning 1: anything after it in the file is not a cause. Taking the
/// last one is also what makes a hostile lyric containing [`FAILURE_PREFIX`]
/// harmless — it can only be picked when the real line is absent, and then the
/// text is sanitized and bounded like any other.
#[must_use]
pub fn diagnose(log_tail: &str) -> Option<HelperDiagnosis> {
    let reported = log_tail
        .lines()
        .rev()
        .find_map(|line| line.split_once(FAILURE_PREFIX).map(|(_, rest)| rest))
        .map(|rest| sanitized(rest, CAUSE_MAX_BYTES))?;
    if reported.is_empty() {
        return None;
    }
    Some(HelperDiagnosis {
        cause: classify(&reported),
        reported,
    })
}

/// Matched against the helper's own format strings
/// (`tools/external_analysis.py:293`, `:301`, `:307`). Substrings rather than a
/// regex because the interesting half of each sentence — the tool name, the
/// code, the errno text — is interpolated and must survive verbatim.
fn classify(reported: &str) -> FailureCause {
    if reported.starts_with("could not start ") {
        FailureCause::ToolMissing
    } else if reported.contains(" exceeded its ") && reported.contains("timeout") {
        FailureCause::ToolTimedOut
    } else if reported.contains(" exited with code ") {
        FailureCause::ToolFailed
    } else {
        FailureCause::Invalid
    }
}

/// The sentence the Assist panel keeps in `failure_detail`, and the first half
/// of the toast.
///
/// A trailing full stop is added if the helper's sentence lacks one; nothing
/// else is changed. Capitalizing the first letter — which this codebase does
/// elsewhere — would rewrite `whisper-cli` into `Whisper-cli`, and a tool name
/// the user has to check on a command line is the one thing that must come
/// through untouched.
#[must_use]
pub fn failure_sentence(diagnosis: Option<&HelperDiagnosis>) -> String {
    let Some(diagnosis) = diagnosis else {
        return UNDIAGNOSED_SENTENCE.to_string();
    };
    let reported = &diagnosis.reported;
    if reported.ends_with(['.', '!', '?']) {
        reported.clone()
    } else {
        format!("{reported}.")
    }
}

/// `sentence`, plus the log path, guaranteed shorter than [`DETAIL_CAPACITY`].
///
/// The path is clipped from the **left**: a job log lives in a deep per-run
/// artifact directory, and the run's own folder name is the end of the path,
/// not the beginning. Dropped entirely rather than half-shown when there is no
/// room for a useful amount of it.
#[must_use]
pub fn notice_detail(sentence: &str, log_path: &str) -> String {
    const LIMIT: usize = DETAIL_CAPACITY - 1;
    const MARKER: &str = " Log: ";
    /// Below this a clipped path is all ellipsis and no answer.
    const PATH_MIN: usize = 24;

    let mut detail = sanitized(sentence, CAUSE_MAX_BYTES + 64);
    // Scrubbed but **not** clipped before `clipped_head` gets it: clipping from
    // the tail first and then from the front would keep neither end.
    let path = scrubbed(log_path);
    let room = LIMIT.saturating_sub(detail.len() + MARKER.len());
    if !path.is_empty() && room >= PATH_MIN {
        detail.push_str(MARKER);
        detail.push_str(&clipped_head(&path, room));
    }
    debug_assert!(detail.len() < DETAIL_CAPACITY);
    detail
}

/// Bidi-format characters, which are not `char::is_control` and so survive
/// every naive filter.
///
/// This lives in `core` rather than beside the dialog's own sanitizer because
/// two copies of a character table is exactly the drift the audit's B12 names.
/// `ui/assist_settings.rs` imports it from here.
#[must_use]
pub const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// Untrusted text made safe to draw and bounded to `max_bytes`.
///
/// Bounded in **bytes**, not characters, because the limit it has to satisfy —
/// [`DETAIL_CAPACITY`] — is a byte limit. A character bound would accept a
/// 160-character CJK sentence that is 480 bytes and see the notice refused.
fn sanitized(text: &str, max_bytes: usize) -> String {
    clipped(&scrubbed(text), max_bytes)
}

/// The safety half of [`sanitized`] with no length bound beyond [`SCAN_LIMIT`].
fn scrubbed(text: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for character in text.chars().take(SCAN_LIMIT) {
        // Deleted, not turned into a space: an override sits between two halves
        // of a word that were meant to be one.
        if is_bidi_control(character) {
            continue;
        }
        if character.is_control() || character.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(character);
    }
    out
}

/// `text` cut to `max_bytes` on a character boundary, with a trailing ellipsis
/// when anything was dropped.
fn clipped(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes < ELLIPSIS.len() {
        return String::new();
    }
    let mut end = max_bytes - ELLIPSIS.len();
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ELLIPSIS}", text[..end].trim_end())
}

/// `text` cut to `max_bytes` from the **front**, keeping the tail.
fn clipped_head(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes < ELLIPSIS.len() {
        return String::new();
    }
    let mut start = text.len() - (max_bytes - ELLIPSIS.len());
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("{ELLIPSIS}{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::notice::TITLE_CAPACITY;

    const CAUSES: [FailureCause; 4] = [
        FailureCause::ToolMissing,
        FailureCause::ToolTimedOut,
        FailureCause::ToolFailed,
        FailureCause::Invalid,
    ];

    fn log(lines: &[&str]) -> String {
        lines.join("\n")
    }

    /// The four sentences `tools/external_analysis.py` can actually print, each
    /// reaching a different headline. This is the finding: before it, all four
    /// produced one string.
    #[test]
    fn each_helper_sentence_reaches_its_own_headline() {
        let cases = [
            (
                "could not start whisper-cli: [Errno 2] No such file or directory",
                FailureCause::ToolMissing,
            ),
            (
                "whisper-cli exceeded its 2400s timeout",
                FailureCause::ToolTimedOut,
            ),
            ("codex exited with code 1", FailureCause::ToolFailed),
            (
                "the bridge names a track this audio is not",
                FailureCause::Invalid,
            ),
        ];
        let mut headlines = Vec::new();
        for (sentence, expected) in cases {
            let tail = log(&["starting", &format!("{FAILURE_PREFIX}{sentence}")]);
            let diagnosis = diagnose(&tail).expect("a cause");
            assert_eq!(diagnosis.cause, expected, "{sentence}");
            assert_eq!(diagnosis.reported, sentence, "verbatim");
            headlines.push(diagnosis.cause.headline());
        }
        headlines.sort_unstable();
        headlines.dedup();
        assert_eq!(headlines.len(), 4, "one toast per cause, not one for all");
    }

    #[test]
    fn every_headline_fits_a_notice_title() {
        for cause in CAUSES {
            assert!(cause.headline().len() < TITLE_CAPACITY, "{cause:?}");
            assert!(!cause.headline().is_empty());
        }
        assert!(UNDIAGNOSED_HEADLINE.len() < TITLE_CAPACITY);
    }

    /// The line is the *last* one, because the helper prints it immediately
    /// before exiting and a run reports many things first.
    #[test]
    fn the_last_cause_wins() {
        let tail = log(&[
            &format!("{FAILURE_PREFIX}codex exited with code 1"),
            "retrying",
            &format!("{FAILURE_PREFIX}whisper-cli exceeded its 30s timeout"),
        ]);
        let diagnosis = diagnose(&tail).expect("a cause");
        assert_eq!(diagnosis.cause, FailureCause::ToolTimedOut);
        assert_eq!(diagnosis.reported, "whisper-cli exceeded its 30s timeout");
    }

    #[test]
    fn a_log_with_no_cause_line_diagnoses_nothing() {
        assert_eq!(diagnose(""), None);
        assert_eq!(
            diagnose("Traceback (most recent call last):\n  File …"),
            None
        );
        // Present but empty is not a diagnosis either.
        assert_eq!(diagnose(&format!("{FAILURE_PREFIX}   ")), None);
        assert_eq!(
            failure_sentence(None),
            UNDIAGNOSED_SENTENCE,
            "and the old sentence is still the right one"
        );
    }

    /// A job log carries child output and the track's own lyrics. Nothing that
    /// could overprint or reverse a notice card may come back out of it.
    #[test]
    fn a_hostile_log_line_yields_drawable_text() {
        let hostile = format!(
            "{FAILURE_PREFIX}\u{202e}dessim si ledom\u{202c}\tcodex\u{0007} exited with code {}",
            "9".repeat(4000)
        );
        let diagnosis = diagnose(&hostile).expect("a cause");
        assert!(diagnosis.reported.len() <= CAUSE_MAX_BYTES);
        assert!(
            !diagnosis
                .reported
                .chars()
                .any(|character| character.is_control() || is_bidi_control(character)),
            "{:?}",
            diagnosis.reported
        );
        assert!(diagnosis.reported.ends_with(ELLIPSIS));
        assert_eq!(diagnosis.cause, FailureCause::ToolFailed);
    }

    /// A newline **ends** the cause. Everything the helper's children wrote
    /// after it is a different line and cannot be smuggled into the toast by
    /// being on the same one.
    #[test]
    fn a_newline_ends_the_cause() {
        let tail = format!("{FAILURE_PREFIX}codex exited with code 1\r\nand then some lyrics");
        let diagnosis = diagnose(&tail).expect("a cause");
        assert_eq!(diagnosis.reported, "codex exited with code 1");
        assert!(!diagnosis.reported.contains("lyrics"));
    }

    /// The bound the notice queue enforces by *refusing*. A detail one byte too
    /// long is a failure that is never reported — B2 again, one layer down.
    #[test]
    fn a_detail_always_fits_the_notice_queue() {
        let long_sentence = format!("{} exited with code 1.", "w".repeat(4000));
        let long_path = format!("/{}/assist-all-000.log", "d".repeat(4000));
        for sentence in [UNDIAGNOSED_SENTENCE, long_sentence.as_str(), ""] {
            for path in ["", "/tmp/x.log", long_path.as_str()] {
                let detail = notice_detail(sentence, path);
                assert!(
                    detail.len() < DETAIL_CAPACITY,
                    "{} bytes for {}/{}",
                    detail.len(),
                    sentence.len(),
                    path.len()
                );
                assert!(!detail.chars().any(char::is_control));
            }
        }
    }

    /// A long path is clipped from the front: the run's own folder is at the
    /// end, and that is the half a user needs to find the file.
    #[test]
    fn a_long_log_path_keeps_its_tail() {
        let path = format!("/home/{}/assist-all-042.log", "n".repeat(400));
        let detail = notice_detail("codex exited with code 1.", &path);
        assert!(detail.contains("codex exited with code 1."));
        assert!(detail.ends_with("assist-all-042.log"), "{detail}");
        assert!(detail.contains(&format!("Log: {ELLIPSIS}")));
        assert!(detail.len() < DETAIL_CAPACITY);
    }

    /// The ordinary case, end to end, with nothing dropped.
    #[test]
    fn an_ordinary_failure_reads_as_one_sentence_and_a_path() {
        let tail = log(&[
            "assist: mode=all",
            &format!(
                "{FAILURE_PREFIX}could not start whisper-cli: [Errno 2] No such file or directory"
            ),
        ]);
        let diagnosis = diagnose(&tail).expect("a cause");
        let sentence = failure_sentence(Some(&diagnosis));
        assert_eq!(
            sentence,
            "could not start whisper-cli: [Errno 2] No such file or directory."
        );
        assert_eq!(
            notice_detail(&sentence, "/tmp/mz/assist-all-000.log"),
            "could not start whisper-cli: [Errno 2] No such file or directory. \
             Log: /tmp/mz/assist-all-000.log"
        );
        assert_eq!(
            diagnosis.cause.headline(),
            "A tool the analysis needs could not run"
        );
    }

    /// A sentence already ending in punctuation is not given a second stop.
    #[test]
    fn a_sentence_is_not_double_punctuated() {
        let diagnosis = HelperDiagnosis {
            cause: FailureCause::Invalid,
            reported: "the model refused the request.".to_string(),
        };
        assert_eq!(
            failure_sentence(Some(&diagnosis)),
            "the model refused the request."
        );
    }
}
