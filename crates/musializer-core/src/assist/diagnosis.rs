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

/// The listening helper's failure receipt, beside its cache in the job folder
/// (`tools/semantic_openrouter.py`, `failure_receipt_path`). Pinned by
/// `tests/test_cross_language_pins.py`.
pub const LISTENING_RECEIPT_NAME: &str = "semantic.cache.failed.json";

/// The receipt schemas the panel reads: the helper's own, and the one it
/// wrote as `tools/mimo_openrouter.py` until 2026-11-15.
pub const LISTENING_RECEIPT_SCHEMAS: [&str; 2] = [
    "musializer.semantic-failed-run/v1",
    "musializer.mimo-failed-run/v1",
];

/// The longest message a receipt carries, and the most of it the panel's
/// notice keeps ([`CAUSE_MAX_BYTES`] + 64, the bound [`notice_detail`]
/// applies). Pinned against the helper's `MESSAGE_MAX_BYTES`.
pub const RECEIPT_MESSAGE_MAX_BYTES: usize = CAUSE_MAX_BYTES + 64;

/// Bytes of a receipt read; a larger one is not a receipt.
pub const RECEIPT_MAX_BYTES: usize = 1 << 20;

/// What the listening step's own receipt says about why it failed (defect
/// J1-2, 2026-11-16).
///
/// The job log carries the same sentence behind [`FAILURE_PREFIX`], clipped at
/// [`CAUSE_MAX_BYTES`]; the receipt carries it whole, and with the two facts a
/// headline needs: which failure it was and whether it may have been billed.
#[derive(Clone, Debug, PartialEq)]
pub struct ListeningFailure {
    /// `key_rejected`, `no_credit`, `no_endpoint`, `refused`, `rate_limited`,
    /// `no_key`, `unreachable`, `not_sent`, `timed_out`, `unanswered`,
    /// `unusable_answer` or `answer_gone` (J4); anything else is read as an
    /// unknown failure.
    pub kind: String,
    /// The helper's sentence, sanitized and bounded.
    pub message: String,
    pub may_be_billed: bool,
    pub reported_cost_usd: Option<f64>,
}

impl ListeningFailure {
    /// The notice title, by what the person has to do about it.
    #[must_use]
    pub fn headline(&self) -> &'static str {
        if self.may_be_billed {
            return "The listening request may have been billed";
        }
        if self.reported_cost_usd.is_some() || self.kind == "unusable_answer" {
            return "The listening answer could not be used";
        }
        match self.kind.as_str() {
            "key_rejected" => "OpenRouter refused the key",
            "no_credit" => "The OpenRouter account needs credit",
            "no_endpoint" => "No OpenRouter endpoint fits these settings",
            "refused" => "OpenRouter refused the request",
            "rate_limited" => "OpenRouter is rate-limiting requests",
            "unreachable" => "OpenRouter could not be reached",
            "no_key" | "not_sent" => "The listening request was not sent",
            // J4-2: the card said a saved answer would be reused.
            "answer_gone" => "The saved answer is no longer there",
            _ => "The listening step failed",
        }
    }
}

/// Reads a listening receipt, when it is one and describes an attempt that
/// failed at or after `since_ns` (nanoseconds since the epoch; the job's
/// start): a receipt from an earlier job explains nothing about this one.
#[must_use]
pub fn listening_failure(receipt: &[u8], since_ns: u128) -> Option<ListeningFailure> {
    if receipt.len() > RECEIPT_MAX_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(receipt).ok()?;
    let schema = value.get("schema_version")?.as_str()?;
    if !LISTENING_RECEIPT_SCHEMAS.contains(&schema) {
        return None;
    }
    let failed = value.get("failed_ns")?.as_u64()?;
    if u128::from(failed) < since_ns {
        return None;
    }
    let message = sanitized(value.get("message")?.as_str()?, RECEIPT_MESSAGE_MAX_BYTES);
    if message.is_empty() {
        return None;
    }
    Some(ListeningFailure {
        kind: value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        message,
        may_be_billed: value
            .get("may_be_billed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        reported_cost_usd: value
            .get("reported_cost_usd")
            .and_then(serde_json::Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0),
    })
}

/// The record the listening helper writes **before** its request leaves
/// (`tools/semantic_openrouter.py`, `sent_record_path`; defect J1-3,
/// 2026-11-16). One still there is a request that was sent and never answered.
/// The one name the J1 build used; since J3-1 each attempt writes its own,
/// `semantic.cache.sent-<attempt>.json` ([`is_sent_record_name`]).
pub const SENT_RECORD_NAME: &str = "semantic.cache.sent.json";

/// Its schemas: the J1 record (one per cache), the J3 one (one per attempt)
/// and the J4 one (whose attempt is its charge's id in the ledger).
pub const SENT_RECORD_SCHEMAS: [&str; 3] = [
    "musializer.semantic-request-sent/v1",
    "musializer.semantic-request-sent/v2",
    SENT_RECORD_SCHEMA_V3,
];

/// The J4 record, whose `attempt` is its possible charge's id.
const SENT_RECORD_SCHEMA_V3: &str = "musializer.semantic-request-sent/v3";

/// Records read from one job folder at most; a folder with more is not one
/// the helper wrote.
pub const SENT_RECORDS_MAX: usize = 64;

/// Whether a job folder's entry is a sent record: the J1 name, or
/// `semantic.cache.sent-<digits>-<8 hex>.json` (the helper's
/// `ATTEMPT_PATTERN`).
#[must_use]
pub fn is_sent_record_name(name: &str) -> bool {
    if name == SENT_RECORD_NAME {
        return true;
    }
    let Some(attempt) = name
        .strip_prefix("semantic.cache.sent-")
        .and_then(|rest| rest.strip_suffix(".json"))
    else {
        return false;
    };
    is_attempt(attempt)
}

/// `<1-20 digits>-<8 lowercase hex>`: an attempt (the helper's
/// `ATTEMPT_PATTERN`).
fn is_attempt(attempt: &str) -> bool {
    let Some((time, tag)) = attempt.split_once('-') else {
        return false;
    };
    is_nanoseconds(time)
        && tag.len() == 8
        && tag
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_nanoseconds(text: &str) -> bool {
    (1..=20).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether a text is a possible charge's id in the job folder's ledger
/// (J4; the helper's `is_charge_id`): an attempt, or the id the J1 and J3
/// builds' files give a charge (`unanswered@<ns>`, `may_be_billed@<ns>`).
/// Only such ids reach the helper's argv.
#[must_use]
pub fn is_charge_id(text: &str) -> bool {
    is_attempt(text)
        || text
            .strip_prefix("unanswered@")
            .or_else(|| text.strip_prefix("may_be_billed@"))
            .is_some_and(is_nanoseconds)
}

/// The listening helper's answer cache.
pub const LISTENING_CACHE_NAME: &str = "semantic.cache.json";

/// The confirmation's line when the job folder's ledger of possible charges
/// cannot be read (J4): what it shows may not be all there is, and is not
/// read as all there is.
pub const LEDGER_UNREADABLE_SENTENCE: &str = "This track's record of earlier listening requests \
     could not be read, so this card may not show every request that may have been billed.";

/// One attempt that may have been billed and is not answered.
#[derive(Clone, Debug, PartialEq)]
pub struct PossibleCharge {
    /// Its id in the job folder's ledger (J4, [`is_charge_id`]): what the
    /// Start pressed after the confirmation showed it acknowledges.
    pub id: String,
    pub at_ns: u128,
    /// `2026-11-16 10:02 UTC`, or empty when the record has no time.
    pub at_words: String,
    /// The most it can have cost, already rounded up at the thousandth.
    pub usd_max: Option<f64>,
    /// Sent and never answered (a record), rather than answered by a status
    /// that may have been billed (a receipt).
    pub unanswered: bool,
    /// The audio it was about, when the record says (J3-2).
    pub audio_sha256: Option<String>,
    /// The request's identity as one digest (`request_key`: the cache key its
    /// answer would carry), when the record says (J3-2).
    pub request_key: Option<String>,
}

/// What the next Start would send, for telling an earlier charge for the
/// same request from one for something else that used the folder (J3-2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NextStart<'a> {
    /// The track's audio digest.
    pub audio_sha256: &'a str,
    /// The next request's `request_key`, from `semantic-status`; `None`
    /// while it is not known (asking, or the track not measured yet).
    pub request_key: Option<&'a str>,
    /// Whether the next Start reuses a saved answer and sends nothing.
    pub reuses: bool,
}

/// How an earlier possible charge relates to the next Start (J3-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeRelation {
    /// This track's audio and these settings: the next Start sends it again.
    SameRequest,
    /// This track's audio under other settings (model, providers, limits).
    OtherSettings,
    /// This track's audio; whether the settings were the same is not known.
    SameAudio,
    /// Other audio that was at this track's path (the file was replaced).
    OtherAudio,
    /// The record says nothing about what it was for (the J1 build).
    Unknown,
}

impl ChargeRelation {
    /// The words after "listening request(s)".
    fn subject(self) -> &'static str {
        match self {
            Self::SameRequest => "for this track and these settings",
            Self::OtherSettings => "for this track with other settings",
            Self::SameAudio => "for this track",
            Self::OtherAudio => "for other audio that was at this track's path",
            Self::Unknown => "that used this track's job folder",
        }
    }
}

impl PossibleCharge {
    #[must_use]
    pub fn relation(&self, next: &NextStart<'_>) -> ChargeRelation {
        let Some(audio) = self
            .audio_sha256
            .as_deref()
            .filter(|audio| !audio.is_empty())
        else {
            return ChargeRelation::Unknown;
        };
        if !audio.eq_ignore_ascii_case(next.audio_sha256) {
            return ChargeRelation::OtherAudio;
        }
        match (self.request_key.as_deref(), next.request_key) {
            (Some(this), Some(next)) if this == next => ChargeRelation::SameRequest,
            (Some(_), Some(_)) => ChargeRelation::OtherSettings,
            _ => ChargeRelation::SameAudio,
        }
    }
}

fn utc_words(value: Option<&serde_json::Value>) -> String {
    let Some(text) = value.and_then(serde_json::Value::as_str) else {
        return String::new();
    };
    let bytes = text.as_bytes();
    if text.len() >= 16 && bytes.get(10) == Some(&b'T') && text.is_char_boundary(16) {
        format!("{} {} UTC", &text[..10], &text[11..16])
    } else {
        String::new()
    }
}

/// A 64-hex digest field, or `None`.
fn digest_field(entry: &serde_json::Value, key: &str) -> Option<String> {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

fn charge(entry: &serde_json::Value, unanswered: bool) -> Option<PossibleCharge> {
    let (time, utc) = if entry.get("at_ns").is_some() {
        ("at_ns", "at_utc")
    } else if unanswered {
        ("sent_ns", "sent_utc")
    } else {
        ("failed_ns", "failed_utc")
    };
    let at_ns = entry.get(time)?.as_u64()?;
    let unanswered = entry
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map_or(unanswered, |kind| kind == "unanswered");
    // The helper's own id when it names one (a status entry, a J4 record's
    // attempt); otherwise the id the J1 and J3 builds' files give a charge.
    let attempt = (entry
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        == Some(SENT_RECORD_SCHEMA_V3))
    .then(|| entry.get("attempt").and_then(serde_json::Value::as_str))
    .flatten();
    let id = entry
        .get("id")
        .and_then(serde_json::Value::as_str)
        .or(attempt)
        .filter(|id| is_charge_id(id))
        .map_or_else(
            || {
                format!(
                    "{}@{at_ns}",
                    if unanswered {
                        "unanswered"
                    } else {
                        "may_be_billed"
                    }
                )
            },
            str::to_string,
        );
    Some(PossibleCharge {
        id,
        at_ns: u128::from(at_ns),
        at_words: utc_words(entry.get(utc)),
        usd_max: entry
            .get("usd_max")
            .and_then(serde_json::Value::as_f64)
            .filter(|usd| usd.is_finite() && *usd >= 0.0),
        unanswered,
        audio_sha256: digest_field(entry, "audio_sha256"),
        request_key: digest_field(entry, "request_key"),
    })
}

/// The sent record, when it is one: the request it describes and the earlier
/// possible charges it carried forward.
#[must_use]
pub fn sent_request(record: &[u8]) -> Option<(PossibleCharge, Vec<PossibleCharge>)> {
    if record.len() > RECEIPT_MAX_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(record).ok()?;
    if !SENT_RECORD_SCHEMAS.contains(&value.get("schema_version")?.as_str()?) {
        return None;
    }
    let this = charge(&value, true)?;
    let earlier = value
        .get("earlier")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| charge(entry, true))
                .collect()
        })
        .unwrap_or_default();
    Some((this, earlier))
}

/// The possible charges a `semantic-status` answer names (J4): every one the
/// job folder's ledger and files hold that is not acknowledged or answered,
/// oldest first, each with its id. The helper decides what is pending, in one
/// place; the confirmation shows all of them and the Start pressed after it
/// acknowledges exactly these ids. `None` when the answer names none at all
/// (an older helper), which shows nothing and acknowledges nothing.
#[must_use]
pub fn status_charges(status: &serde_json::Value) -> Option<Vec<PossibleCharge>> {
    let entries = status.get("pending_charges")?.as_array()?;
    Some(
        entries
            .iter()
            .filter(|entry| {
                entry
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_charge_id)
            })
            .filter_map(|entry| {
                // A charge with no time is still a charge: it is shown (with
                // no date) rather than left pending and never said.
                if entry
                    .get("at_ns")
                    .and_then(serde_json::Value::as_u64)
                    .is_none()
                {
                    let mut timed = entry.clone();
                    timed["at_ns"] = serde_json::Value::from(0_u64);
                    return charge(&timed, false);
                }
                charge(entry, false)
            })
            .collect(),
    )
}

/// Whether a `semantic-status` answer says the ledger could not be read.
#[must_use]
pub fn status_ledger_unreadable(status: &serde_json::Value) -> bool {
    status.get("ledger").and_then(serde_json::Value::as_str) == Some("unreadable")
}

/// `"(at most 0.073 USD)"`, or nothing when no charge names a ceiling.
fn at_most(charges: &[PossibleCharge], together: bool) -> String {
    let known: Vec<f64> = charges.iter().filter_map(|entry| entry.usd_max).collect();
    if known.is_empty() || known.len() != charges.len() {
        return String::new();
    }
    let total = crate::assist::cost::ceil_thousandths(known.iter().sum());
    format!(
        " (at most {total:.3} USD{})",
        if together { " together" } else { "" }
    )
}

/// The confirmation's sentence about earlier attempts that may have been
/// billed, or `None` when there are none (J1-3). Each says what it was for,
/// against what the next Start would send (J3-2): the same request again, or
/// an earlier one for other audio or other settings that used this folder.
#[must_use]
pub fn possible_charge_sentence(
    charges: &[PossibleCharge],
    next: &NextStart<'_>,
) -> Option<String> {
    let last = charges.last()?;
    let when = if last.at_words.is_empty() {
        String::new()
    } else {
        format!(" on {}", last.at_words)
    };
    let relations: Vec<ChargeRelation> = charges.iter().map(|entry| entry.relation(next)).collect();
    let relation = if relations.iter().all(|relation| *relation == relations[0]) {
        relations[0]
    } else {
        ChargeRelation::Unknown
    };
    let subject = relation.subject();
    // J3-7: the way out, said where the warning is: the Start pressed after
    // reading it puts it in the job folder's log, and it is not said again.
    let again = if relations.contains(&ChargeRelation::SameRequest) {
        if next.reuses {
            " This Start reuses a saved answer and sends nothing; this note then moves to the \
             job folder's log."
        } else {
            " Start sends the same request again; this note then moves to the job folder's log."
        }
    } else {
        " After Start this note moves to the job folder's log."
    };
    let sentence = if charges.len() > 1 {
        format!(
            "{} listening requests {subject} may have been sent and billed{}{}.",
            charges.len(),
            at_most(charges, true),
            if when.is_empty() {
                String::new()
            } else {
                format!("; the last was{when}")
            },
        )
    } else if last.unanswered {
        format!(
            "A listening request {subject} was started{when} and never answered; it may have \
             been sent and billed{}.",
            at_most(charges, false),
        )
    } else {
        format!(
            "A listening request {subject}{} may have been billed{}.",
            if when.is_empty() {
                String::new()
            } else {
                format!(",{when},")
            },
            at_most(charges, false),
        )
    };
    Some(format!("{sentence}{again}"))
}

/// What a stopped job adds when its request had already left: the clause
/// every stop message carries (Cancel, the deadline, a torn-down job, a
/// closed window).
#[must_use]
pub fn sent_clause(sent: &PossibleCharge) -> String {
    // J3-7: the record is written before the first byte leaves, so a helper
    // killed just after writing it had not sent anything; "had already been
    // sent" asserted more than is known.
    format!(
        "the listening request may already have been sent to OpenRouter and billed{}",
        at_most(std::slice::from_ref(sent), false)
    )
}

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

    fn receipt(kind: &str, message: &str, billed: bool, cost: Option<f64>, at: u64) -> Vec<u8> {
        serde_json::json!({
            "schema_version": "musializer.semantic-failed-run/v1",
            "failed_ns": at, "kind": kind, "message": message,
            "may_be_billed": billed, "reported_cost_usd": cost,
        })
        .to_string()
        .into_bytes()
    }

    /// J1-2: each kind the helper writes reaches its own headline, every
    /// headline fits a title, and what may have been billed decides first.
    #[test]
    fn every_listening_failure_has_a_headline_that_says_what_to_do() {
        let kinds = [
            ("key_rejected", "OpenRouter refused the key"),
            ("no_credit", "The OpenRouter account needs credit"),
            ("no_endpoint", "No OpenRouter endpoint fits these settings"),
            ("refused", "OpenRouter refused the request"),
            ("rate_limited", "OpenRouter is rate-limiting requests"),
            ("unreachable", "OpenRouter could not be reached"),
            ("no_key", "The listening request was not sent"),
            ("not_sent", "The listening request was not sent"),
            ("answer_gone", "The saved answer is no longer there"),
            ("unusable_answer", "The listening answer could not be used"),
            ("something new", "The listening step failed"),
        ];
        for (kind, headline) in kinds {
            let failure =
                listening_failure(&receipt(kind, "words.", false, None, 10), 5).expect(kind);
            assert_eq!(failure.headline(), headline, "{kind}");
            assert!(headline.len() < TITLE_CAPACITY);
        }
        let billed = listening_failure(&receipt("timed_out", "w.", true, None, 10), 5).unwrap();
        assert_eq!(
            billed.headline(),
            "The listening request may have been billed"
        );
        let paid = listening_failure(&receipt("", "w.", false, Some(0.0131), 10), 5).unwrap();
        assert_eq!(paid.headline(), "The listening answer could not be used");
        assert_eq!(paid.reported_cost_usd, Some(0.0131));
    }

    /// A receipt from before the job started, another schema, or one with no
    /// words, is not this job's reason; a long message is bounded where it
    /// is read.
    #[test]
    fn only_this_jobs_receipt_is_read_and_it_is_bounded() {
        assert!(listening_failure(&receipt("no_credit", "w.", false, None, 4), 5).is_none());
        assert!(listening_failure(&receipt("no_credit", "", false, None, 9), 5).is_none());
        assert!(listening_failure(b"not json", 0).is_none());
        let other = br#"{"schema_version":"x/v1","failed_ns":9,"message":"m"}"#;
        assert!(listening_failure(other, 0).is_none());
        let long = "é".repeat(1000);
        let failure = listening_failure(&receipt("refused", &long, false, None, 9), 0).unwrap();
        assert!(failure.message.len() <= RECEIPT_MESSAGE_MAX_BYTES);
        assert!(notice_detail(&failure.message, "/tmp/x.log").len() < DETAIL_CAPACITY);
    }

    fn sent(at: u64, usd: f64, earlier: &str) -> Vec<u8> {
        format!(
            r#"{{"schema_version":"musializer.semantic-request-sent/v1","sent_ns":{at},
                "sent_utc":"2026-11-16T10:02:33Z","usd_max":{usd},"earlier":[{earlier}]}}"#
        )
        .into_bytes()
    }

    /// A `semantic-status` answer naming these charges (J4).
    fn status(entries: &str) -> serde_json::Value {
        serde_json::from_str(&format!(
            r#"{{"schema_version":"musializer.semantic-status/v1","state":"none",
                "pending_charges":[{entries}],"ledger":"ok"}}"#
        ))
        .expect("json")
    }

    fn pending(id: &str, kind: &str, at_utc: &str, usd: f64) -> String {
        format!(
            r#"{{"id":"{id}","kind":"{kind}","at_ns":2000,"at_utc":"{at_utc}","usd_max":{usd},
                "audio_sha256":null,"request_key":null,"model":null}}"#
        )
    }

    /// J1-3, J4: a charge the status names is said before the next Start,
    /// with what it may have cost, and carries its id; a stop's clause is
    /// read from the record itself.
    #[test]
    fn a_request_sent_and_never_answered_is_said_before_the_next_start() {
        let charges = status_charges(&status(&pending(
            "1763287353000000000-0a1b2c3d",
            "unanswered",
            "2026-11-16T10:02:33Z",
            0.073,
        )))
        .expect("named");
        assert_eq!(charges.len(), 1);
        assert_eq!(charges[0].id, "1763287353000000000-0a1b2c3d");
        assert_eq!(
            possible_charge_sentence(&charges, &NextStart::default()).as_deref(),
            Some(
                "A listening request that used this track's job folder was started on \
                 2026-11-16 10:02 UTC and never answered; it may have been sent and billed (at \
                 most 0.073 USD). After Start this note moves to the job folder's log."
            )
        );
        assert_eq!(possible_charge_sentence(&[], &NextStart::default()), None);
        assert_eq!(status_charges(&serde_json::json!({"state": "none"})), None);
        let (this, _) = sent_request(&sent(2_000, 0.073, "")).expect("a record");
        assert_eq!(this.id, "unanswered@2000", "a J1 record's legacy id");
        assert_eq!(
            sent_clause(&this),
            "the listening request may already have been sent to OpenRouter and billed (at most \
             0.073 USD)"
        );
        let v3 = br#"{"schema_version":"musializer.semantic-request-sent/v3","sent_ns":5,
            "attempt":"4-0a1b2c3d","usd_max":0.01}"#;
        assert_eq!(sent_request(v3).expect("a record").0.id, "4-0a1b2c3d");
    }

    /// J4-5: however many the status names, all are shown (a count and a sum,
    /// rounded up) and all carry their ids; an entry with a malformed id is
    /// not a charge, one with no time still is.
    #[test]
    fn every_pending_charge_is_counted_and_summed_up() {
        // In the status's order: oldest first, an untimed one first of all.
        let entries: Vec<String> = [
            r#"{"id":"unanswered@7","kind":"unanswered","usd_max":0.073}"#.to_string(),
            pending("../etc", "unanswered", "", 1.0),
        ]
        .into_iter()
        .chain((0..40).map(|n| pending(&format!("{n}-0000000{}", n % 10), "unanswered", "", 0.073)))
        .chain([pending(
            "may_be_billed@2000",
            "may_be_billed",
            "2026-11-16T10:02:33Z",
            0.073,
        )])
        .collect();
        let charges = status_charges(&status(&entries.join(","))).expect("named");
        assert_eq!(
            charges.len(),
            42,
            "the malformed id is dropped, the untimed one kept"
        );
        assert!(charges.iter().all(|charge| is_charge_id(&charge.id)));
        assert_eq!(charges[0].at_words, "");
        assert_eq!(
            possible_charge_sentence(&charges, &NextStart::default()).as_deref(),
            Some(
                "42 listening requests that used this track's job folder may have been sent and \
                 billed (at most 3.066 USD together); the last was on 2026-11-16 10:02 UTC. After \
                 Start this note moves to the job folder's log."
            )
        );
        // A last one with no time says no time, rather than "the last was.".
        let untimed = status_charges(&status(
            r#"{"id":"unanswered@7","kind":"unanswered","usd_max":0.073},
               {"id":"unanswered@8","kind":"unanswered","usd_max":0.073}"#,
        ))
        .expect("named");
        assert_eq!(
            possible_charge_sentence(&untimed, &NextStart::default()).as_deref(),
            Some(
                "2 listening requests that used this track's job folder may have been sent and \
                 billed (at most 0.146 USD together). After Start this note moves to the job \
                 folder's log."
            )
        );
        let only_receipt = status_charges(&status(&pending(
            "may_be_billed@2000",
            "may_be_billed",
            "2026-11-15T20:00:00Z",
            0.0721,
        )))
        .expect("named");
        assert_eq!(
            possible_charge_sentence(&only_receipt, &NextStart::default()).as_deref(),
            Some(
                "A listening request that used this track's job folder, on 2026-11-15 20:00 \
                 UTC, may have been billed (at most 0.073 USD). After Start this note moves to the \
                 job folder's log."
            )
        );
        assert!(status_ledger_unreadable(
            &serde_json::json!({"ledger": "unreadable"})
        ));
        assert!(!status_ledger_unreadable(&status("")));
    }

    #[test]
    fn a_charge_id_is_an_attempt_or_a_legacy_id_and_nothing_else() {
        for id in [
            "1763287353000000000-0a1b2c3d",
            "1-00000000",
            "unanswered@1763287353000000000",
            "may_be_billed@2",
        ] {
            assert!(is_charge_id(id), "{id}");
        }
        for id in [
            "",
            "1-0A1B2C3D",
            "1-0a1b2c3",
            "x-0a1b2c3d",
            "charged@1",
            "unanswered@",
            "unanswered@123456789012345678901",
            "1-0a1b2c3d; rm",
            "--zdr",
        ] {
            assert!(!is_charge_id(id), "{id}");
        }
    }

    /// J3-2: a charge says what it was for, against what the next Start
    /// would send: the same request again, this track with other settings,
    /// other audio that was at this path, or (not yet known) this track.
    #[test]
    fn an_earlier_charge_says_whether_it_was_this_request() {
        let track = "a".repeat(64);
        let mine = "b".repeat(64);
        let record = |audio: &str, key: &str| {
            format!(
                r#"{{"schema_version":"musializer.semantic-request-sent/v2","sent_ns":2000,
                   "sent_utc":"2026-11-16T10:02:33Z","usd_max":0.073,
                   "audio_sha256":"{audio}","request_key":"{key}"}}"#
            )
            .into_bytes()
        };
        let next = NextStart {
            audio_sha256: &track,
            request_key: Some(&mine),
            reuses: false,
        };
        let say = |bytes: Vec<u8>, next: &NextStart<'_>| {
            let (this, _) = sent_request(&bytes).expect("a record");
            possible_charge_sentence(&[this], next).expect("a line")
        };
        assert_eq!(
            say(record(&track, &mine), &next),
            "A listening request for this track and these settings was started on 2026-11-16 \
             10:02 UTC and never answered; it may have been sent and billed (at most 0.073 USD). \
             Start sends the same request again; this note then moves to the job folder's log."
        );
        assert_eq!(
            say(
                record(&track, &mine),
                &NextStart {
                    reuses: true,
                    ..next
                }
            ),
            "A listening request for this track and these settings was started on 2026-11-16 \
             10:02 UTC and never answered; it may have been sent and billed (at most 0.073 USD). \
             This Start reuses a saved answer and sends nothing; this note then moves to the job \
             folder's log."
        );
        assert_eq!(
            say(record(&track, &"c".repeat(64)), &next),
            "A listening request for this track with other settings was started on 2026-11-16 \
             10:02 UTC and never answered; it may have been sent and billed (at most 0.073 USD). \
             After Start this note moves to the job folder's log."
        );
        assert_eq!(
            say(record(&"d".repeat(64), &mine), &next),
            "A listening request for other audio that was at this track's path was started on \
             2026-11-16 10:02 UTC and never answered; it may have been sent and billed (at most \
             0.073 USD). After Start this note moves to the job folder's log."
        );
        assert_eq!(
            say(
                record(&track, &mine),
                &NextStart {
                    request_key: None,
                    ..next
                }
            ),
            "A listening request for this track was started on 2026-11-16 10:02 UTC and never \
             answered; it may have been sent and billed (at most 0.073 USD). After Start this \
             note moves to the job folder's log."
        );
    }

    /// The names the helper writes (`ATTEMPT_PATTERN`) and nothing else.
    #[test]
    fn a_sent_records_name_is_the_helpers_and_nothing_else() {
        for name in [
            "semantic.cache.sent.json",
            "semantic.cache.sent-1763287353000000000-0a1b2c3d.json",
            "semantic.cache.sent-1-00000000.json",
        ] {
            assert!(is_sent_record_name(name), "{name}");
        }
        for name in [
            "semantic.cache.sent-.json",
            "semantic.cache.sent-123-0A1B2C3D.json",
            "semantic.cache.sent-123-0a1b2c3.json",
            "semantic.cache.sent-123456789012345678901-0a1b2c3d.json",
            ".semantic.cache.sent-1-00000000.json.x.tmp",
            "semantic.cache.sent-1-00000000.json.tmp",
            "semantic.cache.failed.json",
        ] {
            assert!(!is_sent_record_name(name), "{name}");
        }
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
