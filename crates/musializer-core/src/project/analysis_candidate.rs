//! Staged analysis results awaiting Apply or Discard.
//!
//! **Owner: Agent B.** Port of `../musializer/src/analysis_candidate.c/.h`.
//!
//! This module is the "model output is **staged**" invariant made into a type.
//! Nothing in the editor changes because a job finished: a completed run becomes an
//! [`AnalysisCandidate`], which is validated, bounded, and inert until someone calls
//! [`AnalysisCandidate::apply`]. Discarding it is dropping it.
//!
//! Two authority rules are enforced rather than trusted:
//!
//! - a candidate only ever *contains* lanes the caller authorized, because
//!   [`AnalysisCandidate::prepare`] skips the rest;
//! - [`AnalysisCandidate::apply`] re-checks that nothing available exceeds what was
//!   authorized, so a candidate that was tampered with between preparation and
//!   application is refused rather than applied.

use serde_json::Value;

use crate::project::analysis_bridge::AnalysisBridge;
use crate::project::event_timeline::EventTimeline;
use crate::project::lyrics::LyricsDocument;
use crate::project::scene_switch::{SceneSwitchCue, SceneSwitchTimeline, CAPACITY};
use crate::scene::events::{EventRecord, EventType};
use crate::scene::settings::SettingsSnapshot;

/// Namespaces a bridge cue id into the semantic event lane
/// (`analysis_candidate.c:68`, the ASCII bytes of `MIMO`).
///
/// The lanes are independently authored, so an id from one must not collide with an
/// id from another. XOR rather than addition so the mapping is reversible.
pub const SEMANTIC_ID_SALT: u64 = 0x4D49_4D4F_0000_0000;

/// Which evidence lanes a run may touch (`Analysis_Candidate_Lane`,
/// `analysis_candidate.h:15-22`).
///
/// C carries these as a bitmask and has to check for unknown bits
/// (`lanes_are_valid`, `analysis_candidate.c:7-10`). Three named booleans make that
/// check unrepresentable; all that remains is "at least one".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lanes {
    pub lyrics: bool,
    pub sections: bool,
    pub semantics: bool,
}

impl Lanes {
    /// Every lane, which is what an unrestricted Assist run authorizes.
    pub const ALL: Lanes = Lanes {
        lyrics: true,
        sections: true,
        semantics: true,
    };

    #[must_use]
    pub const fn lyrics_only() -> Self {
        Self {
            lyrics: true,
            sections: false,
            semantics: false,
        }
    }

    /// At least one lane. An empty authority is a caller bug, not a no-op run.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.lyrics || self.sections || self.semantics
    }

    /// True when `self` contains a lane `authority` does not.
    #[must_use]
    pub fn exceeds(self, authority: Lanes) -> bool {
        (self.lyrics && !authority.lyrics)
            || (self.sections && !authority.sections)
            || (self.semantics && !authority.semantics)
    }
}

/// Why a candidate could not be prepared or applied
/// (`Analysis_Candidate_Result`, `analysis_candidate.h:38-47`).
///
/// C's `ERROR_SCHEMA` has no counterpart: a Rust [`AnalysisBridge`] can only exist
/// by having parsed as v1, so there is no version left to disagree about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisCandidateError {
    #[error("invalid lane authority")]
    Authority,
    #[error("invalid duration")]
    Duration,
    #[error("invalid lyric candidate")]
    Lyrics,
    #[error("invalid scene candidate")]
    Sections,
    #[error("invalid semantic candidate")]
    Semantics,
}

// ---------------------------------------------------------------------------
// The LT1 lyric-review surface.
//
// Invention, not a port: the frozen C has no anchor→block localizer and so no
// notion of an authored line that could not be placed. Tranche LT1 made the
// helper write two additive arrays — `unresolved[]` and `review_flags[]` in a
// `lyric-sync-v1` document, counted in `assist-manifest.json` — and this is the
// bounded reader for them.
//
// Two rules, and they are the ones a later session will otherwise undo:
//
// - **An unresolved line is never a cue.** Nothing below feeds `prepare`, the
//   bridge, or `LyricsDocument`. It is review text and only review text; the
//   helper already refuses to write a lane whose cues plus unresolved do not
//   account for every alignable authored line, and this side does not get a
//   second opinion about that.
// - **Absence is not zero.** A pre-LT1 artifact carries none of these keys, and
//   `parse_review_manifest` returns `None` for it, so the panel renders exactly
//   what it rendered before LT1. A run that really did place everything says so
//   with counts of zero, which is a different thing and looks different.
// ---------------------------------------------------------------------------

/// `assist-manifest.json`'s own version tag (`external_analysis.py:1742`).
pub const MANIFEST_SCHEMA_VERSION: &str = "musializer.assist-manifest/v1";

/// The largest review artifact this module will parse.
///
/// Bounded for `analysis_bridge::MAX_INPUT_BYTES`'s reason: the helper is an
/// independent tool and nothing about its output is trusted. Larger than the
/// bridge's 4 MiB because an aligned document also carries per-word CTC
/// evidence the panel never reads.
pub const REVIEW_INPUT_MAX_BYTES: usize = 8 * 1024 * 1024;

/// How many named review entries a candidate carries.
///
/// A run that flags more than this has a systemic problem the counts already
/// state, and the difference between "64 lines to check" and "600" does not
/// change what the user does next.
pub const REVIEW_ENTRY_CAPACITY: usize = 64;

/// Characters kept of an authored line and of a helper's reason.
pub const REVIEW_TEXT_MAX_CHARS: usize = 160;

/// Why a line is in the review list.
///
/// Never the aligner's own score: the 2026-08-04 adjudication measured it at a
/// median of 0.139 on confirmed-correct lines against 0.142 on confirmed-wrong
/// ones, so a score-derived flag would be noise wearing a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LyricReviewKind {
    /// The line never became a cue at all.
    Unresolved,
    /// The line abstained: a repeated phrase whose occurrence the global order
    /// could not pin. Also never a cue, but for a reason the user can act on
    /// differently — the text is right, the occurrence is not known.
    Abstained,
    /// The line *is* a cue, but the coarse Whisper proposal and the anchor/block
    /// placement disagree by more than the review tolerance.
    Disagreement,
}

impl LyricReviewKind {
    /// The tag drawn at the head of the row. Fixed width in words, not pixels,
    /// so the eye can skip a column of them.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            LyricReviewKind::Unresolved => "UNPLACED",
            LyricReviewKind::Abstained => "AMBIGUOUS",
            LyricReviewKind::Disagreement => "CHECK",
        }
    }

    /// Whether this line failed to become a cue.
    #[must_use]
    pub const fn is_unresolved(self) -> bool {
        matches!(
            self,
            LyricReviewKind::Unresolved | LyricReviewKind::Abstained
        )
    }
}

/// One line the user should look at, by name and time range.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricReviewEntry {
    pub kind: LyricReviewKind,
    /// The authored sheet's line number as a human counts them, so 1-based —
    /// the artifact's `reference_line_index` is 0-based.
    pub line_number: u64,
    pub text: String,
    /// Where to look. For a placed line this is its own window; for an
    /// unresolved one it is the coarse proposal, which is the only clue there
    /// is, and `None` when even that was absent.
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    /// The helper's own sentence.
    pub reason: String,
}

impl LyricReviewEntry {
    /// `1:30.7-1:34.9`, or `not placed` when nothing proposed a window.
    #[must_use]
    pub fn window(&self) -> String {
        match (self.start_seconds, self.end_seconds) {
            (Some(start), Some(end)) => format!("{}-{}", clock(start), clock(end)),
            (Some(start), None) => clock(start),
            _ => "not placed".to_string(),
        }
    }

    /// One row of the review list, and the same text the report line carries so
    /// a capture and a screenshot cannot disagree.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{} line {} {} \"{}\"",
            self.kind.label(),
            self.line_number,
            self.window(),
            self.text
        )
    }
}

/// `m:ss.s`, which is how the transport reads elsewhere in the interface.
fn clock(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "?".to_string();
    }
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor();
    let rest = seconds - minutes * 60.0;
    format!("{minutes:.0}:{rest:04.1}")
}

/// What `assist-manifest.json` says about the run's review surface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewManifest {
    pub unresolved: usize,
    pub flagged: usize,
    /// `lyric_localization.policy`, empty on the per-cue lane.
    pub policy: String,
    pub policy_version: String,
    /// The **file names** of the lyric documents to try, best first: the
    /// acoustic result before the coarse proposal.
    ///
    /// Names rather than the manifest's own absolute paths, deliberately. The
    /// caller resolves them against the job folder it already trusts, so a
    /// manifest that names `/etc/shadow` or `../../..` cannot send a reader
    /// anywhere — the escape is unstateable rather than filtered.
    pub documents: Vec<String>,
}

/// The staged run's review surface: counts for the summary, named lines for the
/// detail.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricsReview {
    /// Authored lines that never became cues.
    pub unresolved: usize,
    /// Lines a human should check. A superset of `unresolved`: every unresolved
    /// line is also flagged, and cross-view disagreements are flagged on top.
    pub flagged: usize,
    /// The named list, bounded by [`REVIEW_ENTRY_CAPACITY`].
    pub entries: Vec<LyricReviewEntry>,
    /// Flagged lines that did not fit in `entries`.
    pub omitted: usize,
    pub policy: String,
    pub policy_version: String,
}

impl LyricsReview {
    /// The counts alone, from a manifest whose documents could not be read.
    /// Still worth showing: "2 unresolved" points at a problem even without the
    /// names.
    #[must_use]
    pub fn from_manifest(manifest: &ReviewManifest) -> Self {
        Self {
            unresolved: manifest.unresolved,
            flagged: manifest.flagged,
            entries: Vec::new(),
            omitted: manifest.flagged,
            policy: manifest.policy.clone(),
            policy_version: manifest.policy_version.clone(),
        }
    }

    /// Nothing to look at. The distinction that matters is against *absence*:
    /// this is a run that placed every line, not a run whose artifact predates
    /// the question.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.unresolved == 0 && self.flagged == 0
    }

    /// The counts, for the panel's summary line.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_clear() {
            "All lines placed, none flagged".to_string()
        } else {
            format!(
                "{} unresolved, {} flagged for review",
                self.unresolved, self.flagged
            )
        }
    }

    /// Fills the named list from a `lyric-sync-v1` document.
    ///
    /// Returns whether the document contributed anything, so a caller trying the
    /// aligned lane and then the coarse one knows when to stop.
    ///
    /// The document's own counters win over the manifest's when both exist:
    /// the manifest counts what the document contains, so a disagreement means
    /// the manifest is stale and the document is the artifact being read.
    pub fn read_document(&mut self, bytes: &[u8]) -> bool {
        let Some(root) = json_value(bytes) else {
            return false;
        };
        let Some(root) = root.as_object() else {
            return false;
        };
        if let Some(policy) = root.get("localization_policy").and_then(Value::as_str) {
            self.policy = truncated(policy);
        }
        if let Some(version) = root
            .get("localization_policy_version")
            .and_then(Value::as_str)
        {
            self.policy_version = truncated(version);
        }

        let unresolved = root.get("unresolved").and_then(Value::as_array);
        let flags = root.get("review_flags").and_then(Value::as_array);
        if unresolved.is_none() && flags.is_none() {
            // A pre-LT1 document, or the no-reference per-cue lane. The
            // manifest's counts stand; inventing entries from `lines[]` alone
            // would be this side deciding what needs review.
            return false;
        }
        let unresolved = unresolved.map_or(&[][..], Vec::as_slice);
        self.unresolved = unresolved.len();

        let mut entries: Vec<LyricReviewEntry> = Vec::new();
        match flags {
            Some(flags) => {
                self.flagged = flags.len();
                for flag in flags {
                    if let Some(entry) = review_entry_from_flag(flag, unresolved) {
                        entries.push(entry);
                    }
                }
            }
            None => {
                // `review_flags[]` is the LT1 surface, but a document that has
                // `unresolved[]` and per-cue `review_flagged` and nothing else is
                // still readable — that pair is what the schema requires, and the
                // aggregate array is a convenience over it.
                for record in unresolved {
                    if let Some(entry) = review_entry_from_unresolved(record) {
                        entries.push(entry);
                    }
                }
                let flagged_cues = root
                    .get("lines")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .filter(|line| {
                        line.get("review_flagged").and_then(Value::as_bool) == Some(true)
                    });
                for line in flagged_cues {
                    if let Some(entry) = review_entry_from_line(line) {
                        entries.push(entry);
                    }
                }
                self.flagged = entries.len();
            }
        }

        // Unresolved lines first, then the disagreements, each in authored
        // order. The unresolved ones are the ones with no cue to jump to, so
        // they are the ones a truncated list must not lose.
        entries.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.line_number.cmp(&b.line_number)));
        self.omitted = self
            .flagged
            .saturating_sub(entries.len().min(REVIEW_ENTRY_CAPACITY));
        entries.truncate(REVIEW_ENTRY_CAPACITY);
        self.entries = entries;
        true
    }
}

/// Reads `assist-manifest.json`.
///
/// `None` for anything that is not an LT1-aware manifest, which is what keeps a
/// pre-LT1 job folder rendering exactly as it did before this tranche. The three
/// markers are checked rather than the schema version, because the manifest
/// version did **not** change: the fields are additive.
#[must_use]
pub fn parse_review_manifest(bytes: &[u8]) -> Option<ReviewManifest> {
    let root = json_value(bytes)?;
    if root.get("schema_version").and_then(Value::as_str)? != MANIFEST_SCHEMA_VERSION {
        return None;
    }
    let counts = root.get("result_counts");
    let unresolved = counts
        .and_then(|counts| counts.get("lyrics_unresolved"))
        .and_then(Value::as_u64);
    let flagged = counts
        .and_then(|counts| counts.get("lyrics_review_flags"))
        .and_then(Value::as_u64);
    let localization = root
        .get("lyric_localization")
        .filter(|value| !value.is_null());
    if unresolved.is_none() && flagged.is_none() && localization.is_none() {
        return None;
    }

    let mut documents = Vec::new();
    for key in ["aligned", "sync"] {
        let name = root
            .get("artifacts")
            .and_then(|artifacts| artifacts.get(key))
            .and_then(Value::as_str)
            .and_then(plain_file_name);
        if let Some(name) = name {
            if !documents.contains(&name) {
                documents.push(name);
            }
        }
    }

    Some(ReviewManifest {
        unresolved: bounded_count(unresolved),
        flagged: bounded_count(flagged),
        policy: localization
            .and_then(|block| block.get("policy"))
            .and_then(Value::as_str)
            .map(truncated)
            .unwrap_or_default(),
        policy_version: localization
            .and_then(|block| block.get("policy_version"))
            .and_then(Value::as_str)
            .map(truncated)
            .unwrap_or_default(),
        documents,
    })
}

fn json_value(bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() || bytes.len() > REVIEW_INPUT_MAX_BYTES {
        return None;
    }
    serde_json::from_slice(bytes).ok()
}

/// The last component of a helper-recorded path, and only when it is an
/// ordinary name. Both separators, because the manifest records whatever the
/// producing platform wrote.
fn plain_file_name(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    if name.is_empty() || name == "." || name == ".." || name.len() > 128 {
        return None;
    }
    Some(name.to_string())
}

fn bounded_count(value: Option<u64>) -> usize {
    usize::try_from(value.unwrap_or(0)).unwrap_or(usize::MAX)
}

fn truncated(text: &str) -> String {
    let mut end = text.len();
    if text.chars().count() > REVIEW_TEXT_MAX_CHARS {
        end = text
            .char_indices()
            .nth(REVIEW_TEXT_MAX_CHARS)
            .map_or(text.len(), |(index, _)| index);
        return format!("{}...", &text[..end]);
    }
    text[..end].to_string()
}

/// A number the panel can print: finite and not negative.
fn seconds_at(value: &Value, key: &str) -> Option<f64> {
    let seconds = value.get(key).and_then(Value::as_f64)?;
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// `reference_line_index` is 0-based; a lyric sheet's first line is line 1.
fn line_number(value: &Value) -> Option<u64> {
    Some(value.get("reference_line_index").and_then(Value::as_u64)? + 1)
}

fn text_at(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(truncated)
        .unwrap_or_default()
}

/// One `review_flags[]` record, with the matching `unresolved[]` record for the
/// two things a flag does not carry: whether the line abstained, and the end of
/// the coarse window.
fn review_entry_from_flag(flag: &Value, unresolved: &[Value]) -> Option<LyricReviewEntry> {
    let number = line_number(flag)?;
    let text = text_at(flag, "text");
    if text.is_empty() {
        return None;
    }
    match flag.get("flag").and_then(Value::as_str)? {
        "unresolved" => {
            let record = unresolved
                .iter()
                .find(|record| line_number(record) == Some(number));
            let abstained = record
                .and_then(|record| record.get("abstained"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(LyricReviewEntry {
                kind: if abstained {
                    LyricReviewKind::Abstained
                } else {
                    LyricReviewKind::Unresolved
                },
                line_number: number,
                text,
                start_seconds: record
                    .and_then(|record| seconds_at(record, "coarse_start_seconds"))
                    .or_else(|| seconds_at(flag, "coarse_start_seconds")),
                end_seconds: record.and_then(|record| seconds_at(record, "coarse_end_seconds")),
                reason: text_at(flag, "reason"),
            })
        }
        "coarse_disagreement" => Some(LyricReviewEntry {
            kind: LyricReviewKind::Disagreement,
            line_number: number,
            text,
            start_seconds: seconds_at(flag, "start_seconds"),
            end_seconds: seconds_at(flag, "end_seconds"),
            reason: text_at(flag, "reason"),
        }),
        // An unknown flag kind from a newer helper is still a line to look at;
        // dropping it would make the list quieter than the count.
        _ => Some(LyricReviewEntry {
            kind: LyricReviewKind::Disagreement,
            line_number: number,
            text,
            start_seconds: seconds_at(flag, "start_seconds"),
            end_seconds: seconds_at(flag, "end_seconds"),
            reason: text_at(flag, "reason"),
        }),
    }
}

fn review_entry_from_unresolved(record: &Value) -> Option<LyricReviewEntry> {
    let line_number = line_number(record)?;
    let text = text_at(record, "text");
    if text.is_empty() {
        return None;
    }
    Some(LyricReviewEntry {
        kind: if record
            .get("abstained")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            LyricReviewKind::Abstained
        } else {
            LyricReviewKind::Unresolved
        },
        line_number,
        text,
        start_seconds: seconds_at(record, "coarse_start_seconds"),
        end_seconds: seconds_at(record, "coarse_end_seconds"),
        reason: text_at(record, "reason"),
    })
}

fn review_entry_from_line(line: &Value) -> Option<LyricReviewEntry> {
    let line_number = line_number(line)?;
    let text = text_at(line, "text");
    if text.is_empty() {
        return None;
    }
    Some(LyricReviewEntry {
        kind: LyricReviewKind::Disagreement,
        line_number,
        text,
        start_seconds: seconds_at(line, "start_seconds"),
        end_seconds: seconds_at(line, "end_seconds"),
        reason: text_at(line, "status"),
    })
}

/// Validated, mode-bounded, **inert** analysis state (`Analysis_Candidate`,
/// `analysis_candidate.h:24-36`).
#[derive(Clone, Debug)]
pub struct AnalysisCandidate {
    authorized: Lanes,
    available: Lanes,
    lyrics: LyricsDocument,
    sections: SceneSwitchTimeline,
    semantic_events: EventTimeline,
    uncertain_lyric_count: usize,
    semantic_note_count: usize,
    /// The LT1 review surface, when the job left an artifact describing one.
    /// Additive: `None` is a pre-LT1 run and renders as one.
    lyrics_review: Option<LyricsReview>,
}

impl AnalysisCandidate {
    #[must_use]
    pub fn authorized(&self) -> Lanes {
        self.authorized
    }

    /// Which authorized lanes the run actually produced. The Assist panel shows this,
    /// because "authorized but empty" and "not authorized" mean different things to a
    /// user deciding whether to apply.
    #[must_use]
    pub fn available(&self) -> Lanes {
        self.available
    }

    #[must_use]
    pub fn lyrics(&self) -> &LyricsDocument {
        &self.lyrics
    }

    #[must_use]
    pub fn sections(&self) -> &SceneSwitchTimeline {
        &self.sections
    }

    #[must_use]
    pub fn semantic_events(&self) -> &EventTimeline {
        &self.semantic_events
    }

    /// How many staged lyric cues the helper flagged as estimated. Worth surfacing
    /// before Apply: an estimated window is timing the user may want to check.
    #[must_use]
    pub fn uncertain_lyric_count(&self) -> usize {
        self.uncertain_lyric_count
    }

    /// How many free-form interpretation notes came with the run. They are not
    /// applied to anything — they are for the user to read.
    #[must_use]
    pub fn semantic_note_count(&self) -> usize {
        self.semantic_note_count
    }

    /// The LT1 review surface, or `None` when the run left no artifact
    /// describing one.
    ///
    /// `None` and a review whose counts are zero are **different answers** and
    /// the panel draws them differently: the first is an artifact that predates
    /// the question, the second is a run that placed every authored line.
    #[must_use]
    pub fn lyrics_review(&self) -> Option<&LyricsReview> {
        self.lyrics_review.as_ref()
    }

    /// Attaches the review read from the finished job's artifacts.
    ///
    /// Inert by construction: this cannot add, move or remove a cue, and
    /// [`Self::apply`] never looks at it. An unresolved line stays out of
    /// `lyrics` because the helper never put it there — nothing here is a second
    /// chance for one.
    ///
    /// Refuses, rather than storing, when the candidate has no lyrics lane: a
    /// review of a lane that was not staged would be a claim about content the
    /// user is not being offered.
    pub fn attach_lyrics_review(&mut self, review: LyricsReview) -> bool {
        if !self.available.lyrics {
            return false;
        }
        self.lyrics_review = Some(review);
        true
    }

    /// `analysis_candidate_prepare` (`analysis_candidate.c:91-130`).
    ///
    /// Converts a parsed bridge into candidate state. Nothing in the destination
    /// editor is touched; that is [`Self::apply`]'s job, and only if the user asks.
    ///
    /// `duration_seconds` is the **decoder's** authoritative duration, which may
    /// differ from the bridge's by rounding — see [`Self::normalize_sections`].
    pub fn prepare(
        bridge: &AnalysisBridge,
        authorized: Lanes,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<Self, AnalysisCandidateError> {
        if !authorized.is_valid() {
            return Err(AnalysisCandidateError::Authority);
        }
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || scene_count == 0 {
            return Err(AnalysisCandidateError::Duration);
        }

        let mut candidate = Self {
            authorized,
            available: Lanes::default(),
            lyrics: LyricsDocument::new(duration_seconds)
                .map_err(|_| AnalysisCandidateError::Duration)?,
            sections: SceneSwitchTimeline::new(),
            semantic_events: EventTimeline::new(),
            uncertain_lyric_count: 0,
            semantic_note_count: 0,
            lyrics_review: None,
        };

        if authorized.lyrics && bridge.lyrics_present() {
            bridge
                .lyrics
                .validate()
                .map_err(|_| AnalysisCandidateError::Lyrics)?;
            candidate.lyrics = bridge.lyrics.clone();
            candidate.available.lyrics = true;
            candidate.uncertain_lyric_count = bridge.uncertain_lyric_count();
        }

        if authorized.sections && bridge.sections_present() {
            if bridge.sections.len() > CAPACITY {
                return Err(AnalysisCandidateError::Sections);
            }
            let cues: Vec<SceneSwitchCue> = bridge
                .sections
                .iter()
                .map(|section| SceneSwitchCue {
                    id: section.id,
                    start_seconds: section.start_ms as f64 / 1000.0,
                    end_seconds: section.end_ms as f64 / 1000.0,
                    scene_index: section.recommended_scene.index() as u32,
                    strength: f32::from(section.transition_strength_milli) / 1000.0,
                    settings: SettingsSnapshot::default(),
                })
                .collect();
            candidate
                .sections
                .replace(&cues, duration_seconds, scene_count)
                .map_err(|_| AnalysisCandidateError::Sections)?;
            candidate.available.sections = true;
        }

        if authorized.semantics {
            candidate.semantic_note_count = bridge.semantic_notes.len();
            if bridge.semantic_cues_present() {
                for cue in &bridge.semantic_cues {
                    // Namespaced so a MiMo cue id cannot masquerade as a manually
                    // authored event's. `0` is reserved, hence the nudge to 1.
                    let mut id = cue.id ^ SEMANTIC_ID_SALT;
                    if id == 0 {
                        id = 1;
                    }
                    candidate
                        .semantic_events
                        .record(EventRecord {
                            timestamp_seconds: cue.start_ms as f64 / 1000.0,
                            id,
                            event_type: EventType::Semantic as u32,
                            value_count: 4,
                            values: [
                                f32::from(cue.energy_milli) / 1000.0,
                                f32::from(cue.tension_milli) / 1000.0,
                                f32::from(cue.valence_milli) / 1000.0,
                                f32::from(cue.confidence_milli) / 1000.0,
                            ],
                        })
                        .map_err(|_| AnalysisCandidateError::Semantics)?;
                }
                candidate.available.semantics = true;
            }
        }

        Ok(candidate)
    }

    /// `analysis_candidate_normalize_sections` (`analysis_candidate.c:132-163`).
    ///
    /// Revalidates the sections lane against the decoder's authoritative duration by
    /// stretching the final cue to reach it. The candidate is left **unchanged** when
    /// that would make the last cue invalid — for example when decoder padding ends
    /// before that cue's start — because a half-normalized plan is worse than an
    /// un-normalized one.
    pub fn normalize_sections(
        &mut self,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), AnalysisCandidateError> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || scene_count == 0 {
            return Err(AnalysisCandidateError::Duration);
        }
        if !self.available.sections || self.sections.is_empty() {
            return Ok(());
        }
        let mut cues = self.sections.cues().to_vec();
        let last = cues.len() - 1;
        cues[last].end_seconds = duration_seconds;

        let mut normalized = SceneSwitchTimeline::new();
        normalized
            .replace(&cues, duration_seconds, scene_count)
            .map_err(|_| AnalysisCandidateError::Sections)?;
        normalized.enabled = self.sections.enabled;
        self.sections = normalized;
        Ok(())
    }

    /// `analysis_candidate_apply` (`analysis_candidate.c:165-200`).
    ///
    /// Applies only lanes that are both authorized **and** present. Suggestions for
    /// sections preserve the user's existing auto-scene opt-in: applying a plan does
    /// not decide for them that auto scenes are on.
    ///
    /// Every source was validated during preparation, so these replacements cannot
    /// fail for content. The independently revisioned documents go first, and the
    /// section timeline is published last, so a failure cannot leave the section plan
    /// pointing at lyrics that were never applied.
    pub fn apply(
        &self,
        lyrics: &mut LyricsDocument,
        sections: &mut SceneSwitchTimeline,
        semantic_events: &mut EventTimeline,
    ) -> Result<(), AnalysisCandidateError> {
        if !self.authorized.is_valid() || self.available.exceeds(self.authorized) {
            return Err(AnalysisCandidateError::Authority);
        }
        if self.available.lyrics {
            lyrics
                .replace(&self.lyrics)
                .map_err(|_| AnalysisCandidateError::Lyrics)?;
        }
        if self.available.semantics {
            semantic_events
                .replace(&self.semantic_events)
                .map_err(|_| AnalysisCandidateError::Semantics)?;
        }
        if self.available.sections {
            let enabled = sections.enabled;
            *sections = self.sections.clone();
            sections.enabled = enabled;
            sections.reset();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::analysis_bridge::{self, AnalysisBridgeError};
    use crate::project::lyrics::base64_encode;
    use crate::project::sha256;
    use crate::scene::SceneId;
    use crate::scene::SCENE_COUNT;

    const SCENES: u32 = SCENE_COUNT as u32;
    const DURATION: f64 = 60.0;

    fn b64(text: &str) -> String {
        base64_encode(text.as_bytes())
    }

    /// A bridge document with all three lanes, built inline the way the C suite
    /// builds its fixtures.
    fn document() -> String {
        let digest = sha256::digest_hex(b"audio");
        let reasons = b64("[\"chorus\"]");
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             LYRIC\t1\t1000\t2000\t900\tnone\t{}\n\
             LYRIC\t2\t2000\t3000\t-1\tuncertain\t{}\n\
             SECTION\t10\t0\t30000\tspectrum\t500\t{reasons}\n\
             SECTION\t11\t30000\t60000\tloom\t750\t{reasons}\n\
             SEMANTIC\t20\t0\t30000\t400\t300\t-200\t900\t{}\n\
             SEMANTIC\t21\t30000\t60000\t800\t700\t600\t950\t{}\n",
            b64("first line"),
            b64("second line"),
            b64("calm"),
            b64("bright")
        )
    }

    fn bridge() -> AnalysisBridge {
        analysis_bridge::parse(document().as_bytes(), None, None).expect("the fixture parses")
    }

    fn notes_document() -> String {
        let digest = sha256::digest_hex(b"audio");
        let reasons = b64("[\"x\"]");
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n\
             SEMANTIC_NOTE\t2\t{}\nSEMANTIC_NOTE\t3\t{}\n",
            b64("a thought"),
            b64("another")
        )
    }

    #[test]
    fn preparing_stages_every_authorized_lane_and_touches_nothing() {
        let bridge = bridge();
        let candidate = AnalysisCandidate::prepare(&bridge, Lanes::ALL, DURATION, SCENES).unwrap();
        assert_eq!(candidate.available(), Lanes::ALL);
        assert_eq!(candidate.lyrics().len(), 2);
        assert_eq!(candidate.sections().len(), 2);
        assert_eq!(candidate.semantic_events().len(), 2);
        assert_eq!(candidate.uncertain_lyric_count(), 1);
        assert_eq!(candidate.semantic_note_count(), 0);
        assert_eq!(
            candidate.sections().cues()[1].scene_index,
            SceneId::Loom.index() as u32
        );
        assert_eq!(candidate.sections().cues()[1].strength, 0.75);
    }

    #[test]
    fn an_unauthorized_lane_is_never_even_staged() {
        let bridge = bridge();
        let candidate =
            AnalysisCandidate::prepare(&bridge, Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        assert_eq!(
            candidate.available(),
            Lanes {
                lyrics: true,
                sections: false,
                semantics: false
            }
        );
        assert!(
            candidate.sections().is_empty(),
            "a lane the run was not authorized for must not exist in the candidate"
        );
        assert!(candidate.semantic_events().is_empty());
    }

    #[test]
    fn an_empty_authority_is_a_caller_bug() {
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::default(), DURATION, SCENES).unwrap_err(),
            AnalysisCandidateError::Authority
        );
    }

    #[test]
    fn an_unusable_duration_or_registry_is_refused() {
        for (duration, scenes) in [
            (0.0, SCENES),
            (-1.0, SCENES),
            (f64::NAN, SCENES),
            (f64::INFINITY, SCENES),
            (DURATION, 0),
        ] {
            assert_eq!(
                AnalysisCandidate::prepare(&bridge(), Lanes::ALL, duration, scenes).unwrap_err(),
                AnalysisCandidateError::Duration,
                "{duration} {scenes}"
            );
        }
    }

    #[test]
    fn semantic_ids_are_namespaced_away_from_manual_ones() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        for event in candidate.semantic_events().events() {
            assert_ne!(event.id, 20);
            assert_ne!(event.id, 21);
            assert_ne!(event.id, 0);
            assert_eq!(
                event.id ^ SEMANTIC_ID_SALT,
                if event.timestamp_seconds == 0.0 {
                    20
                } else {
                    21
                },
                "the mapping is reversible, so a staged event can be traced back"
            );
        }
    }

    #[test]
    fn semantic_values_are_scaled_from_thousandths() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let first = &candidate.semantic_events().events()[0];
        assert_eq!(first.values, [0.4, 0.3, -0.2, 0.9]);
        // And they land inside the project model's per-position bounds.
        assert!((0.0..=1.0).contains(&first.values[0]));
        assert!((-1.0..=1.0).contains(&first.values[2]));
    }

    #[test]
    fn free_form_notes_are_counted_but_never_applied_to_anything() {
        let bridge = analysis_bridge::parse(notes_document().as_bytes(), None, None).unwrap();
        let candidate = AnalysisCandidate::prepare(&bridge, Lanes::ALL, DURATION, SCENES).unwrap();
        assert_eq!(candidate.semantic_note_count(), 2);
        assert!(
            !candidate.available().semantics,
            "notes are for the user to read, not events to apply"
        );
        assert!(candidate.semantic_events().is_empty());
    }

    #[test]
    fn applying_replaces_only_available_lanes() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();

        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        sections
            .replace(
                &[SceneSwitchCue {
                    id: 99,
                    start_seconds: 0.0,
                    end_seconds: DURATION,
                    scene_index: 3,
                    strength: 0.5,
                    settings: SettingsSnapshot::default(),
                }],
                DURATION,
                SCENES,
            )
            .unwrap();
        let mut semantic_events = EventTimeline::new();

        candidate
            .apply(&mut lyrics, &mut sections, &mut semantic_events)
            .unwrap();
        assert_eq!(lyrics.len(), 2);
        assert_eq!(
            sections.cues()[0].id,
            99,
            "an unauthorized lane leaves the editor's own plan alone"
        );
        assert!(semantic_events.is_empty());
    }

    #[test]
    fn applying_sections_preserves_the_users_auto_scene_opt_in() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut semantic_events = EventTimeline::new();

        for opted_in in [false, true] {
            let mut sections = SceneSwitchTimeline::new();
            sections.enabled = opted_in;
            candidate
                .apply(&mut lyrics, &mut sections, &mut semantic_events)
                .unwrap();
            assert_eq!(sections.len(), 2);
            assert_eq!(
                sections.enabled, opted_in,
                "applying a plan must not decide the toggle for the user"
            );
            assert_eq!(sections.active_index(), None, "and the cursor is reset");
        }
    }

    #[test]
    fn applying_advances_the_destination_revisions() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        let mut semantic_events = EventTimeline::new();
        let lyric_revision = lyrics.revision();
        let event_revision = semantic_events.revision();
        candidate
            .apply(&mut lyrics, &mut sections, &mut semantic_events)
            .unwrap();
        assert_eq!(lyrics.revision(), lyric_revision + 1);
        assert_eq!(semantic_events.revision(), event_revision + 1);
    }

    #[test]
    fn a_candidate_carrying_more_than_it_was_authorized_for_is_refused() {
        // The available/authorized re-check at apply time exists for exactly this:
        // state that was tampered with between preparation and application.
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        candidate.available.sections = true;
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        let mut semantic_events = EventTimeline::new();
        assert_eq!(
            candidate
                .apply(&mut lyrics, &mut sections, &mut semantic_events)
                .unwrap_err(),
            AnalysisCandidateError::Authority
        );
        assert_eq!(lyrics.len(), 0, "and nothing was applied on the way out");
    }

    #[test]
    fn normalizing_stretches_the_last_section_to_the_decoded_duration() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        candidate.sections.enabled = true;
        // The decoder found 60.02 s where the helper reported 60.
        candidate.normalize_sections(60.02, SCENES).unwrap();
        assert_eq!(
            candidate.sections().cues().last().unwrap().end_seconds,
            60.02
        );
        assert_eq!(candidate.sections().cues()[0].end_seconds, 30.0);
        assert!(
            candidate.sections().enabled,
            "normalization is not a reason to change the toggle"
        );
    }

    #[test]
    fn normalizing_leaves_the_candidate_alone_when_it_cannot_succeed() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let before = candidate.sections().clone();
        // A duration that ends before the last cue starts cannot be normalized into a
        // valid plan; a half-normalized one would be worse than none.
        assert_eq!(
            candidate.normalize_sections(10.0, SCENES).unwrap_err(),
            AnalysisCandidateError::Sections
        );
        assert_eq!(candidate.sections(), &before);
        for (duration, scenes) in [(0.0, SCENES), (f64::NAN, SCENES), (DURATION, 0)] {
            assert_eq!(
                candidate.normalize_sections(duration, scenes).unwrap_err(),
                AnalysisCandidateError::Duration
            );
        }
        assert_eq!(candidate.sections(), &before);
    }

    #[test]
    fn normalizing_a_candidate_without_sections_is_a_no_op() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        assert!(candidate.normalize_sections(60.02, SCENES).is_ok());
        assert!(candidate.sections().is_empty());
    }

    #[test]
    fn a_bridge_whose_sections_do_not_fit_the_decoded_duration_is_refused() {
        // The bridge covers 60 s; preparing against a 30 s decode cannot produce a
        // contiguous plan, and that is a staged-lane failure rather than something to
        // truncate silently.
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, 30.0, SCENES).unwrap_err(),
            AnalysisCandidateError::Sections
        );
    }

    #[test]
    fn a_bridge_naming_a_scene_outside_the_registry_cannot_be_staged() {
        // `scene_count` smaller than the registry is how a build with fewer scenes
        // would see a newer plan.
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, 1).unwrap_err(),
            AnalysisCandidateError::Sections
        );
    }

    #[test]
    fn a_bridge_that_never_parsed_cannot_be_staged_at_all() {
        // Restating the boundary: candidates are built from parsed bridges, so every
        // malformed document is rejected before this module sees it.
        let malformed = document().replace("MUSIALIZER_BRIDGE\t1", "MUSIALIZER_BRIDGE\t2");
        assert_eq!(
            analysis_bridge::parse(malformed.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Header
        );
    }

    // -----------------------------------------------------------------------
    // The LT1 review surface.
    // -----------------------------------------------------------------------

    /// A manifest as `external_analysis.py:1741-1768` writes it.
    fn manifest(counts: &str, localization: &str) -> String {
        format!(
            r#"{{"schema_version":"musializer.assist-manifest/v1","mode":"lyrics",
                "artifacts":{{"aligned":"/tmp/job/lyrics.aligned.json",
                              "sync":"/tmp/job/lyrics.sync.json"}},
                "result_counts":{{"lyrics":51,"lyrics_unmatched":2{counts}}},
                "lyric_localization":{localization}}}"#
        )
    }

    /// Two unresolved lines (one of them an abstention) and one cue whose views
    /// disagree, which is three flags: `review_flags` includes the unresolved.
    const REVIEW_DOCUMENT: &str = r#"{
        "schema_version": "musializer.lyric-sync/v1",
        "lane": "lyric_sync",
        "localization_policy": "anchor-block-mms",
        "localization_policy_version": "3",
        "lines": [
            {"reference_line_index": 4, "text": "rawr.", "start_seconds": 134.6,
             "end_seconds": 136.0, "review_flagged": true}
        ],
        "unresolved": [
            {"reference_line_index": 25, "text": "we were never meant to stay",
             "reason": "no block placement", "abstained": false,
             "coarse_start_seconds": 90.6, "coarse_end_seconds": 94.2},
            {"reference_line_index": 30, "text": "and again",
             "reason": "repeated phrase could not be pinned", "abstained": true,
             "coarse_start_seconds": null, "coarse_end_seconds": null}
        ],
        "review_flags": [
            {"reference_line_index": 4, "text": "rawr.", "flag": "coarse_disagreement",
             "reason": "the coarse Whisper proposal and the anchor/block placement differ by 21.6 s",
             "start_seconds": 134.6, "end_seconds": 136.0,
             "coarse_start_seconds": 156.2, "delta_seconds": -21.6},
            {"reference_line_index": 25, "text": "we were never meant to stay",
             "flag": "unresolved", "reason": "no block placement",
             "start_seconds": null, "end_seconds": null,
             "coarse_start_seconds": 90.6, "delta_seconds": null},
            {"reference_line_index": 30, "text": "and again",
             "flag": "unresolved", "reason": "repeated phrase could not be pinned",
             "start_seconds": null, "end_seconds": null,
             "coarse_start_seconds": null, "delta_seconds": null}
        ],
        "statistics": {"reference_lines": 51, "matched_lines": 49,
                       "estimated_lines": 0, "unmatched_lines": 2,
                       "reference_tokens": 300, "unresolved_lines": 2,
                       "review_flagged_lines": 3}
    }"#;

    #[test]
    fn a_manifest_without_the_lt1_keys_is_not_a_review_at_all() {
        // The whole point of returning `None` here: a job folder written before
        // this tranche must render exactly as it did, and "0 unresolved" is a
        // claim its artifact cannot support.
        let pre_lt1 = r#"{"schema_version":"musializer.assist-manifest/v1","mode":"lyrics",
            "artifacts":{"aligned":"/tmp/job/lyrics.aligned.json"},
            "result_counts":{"lyrics":51,"lyrics_unmatched":0,"sections":4,"semantics":0}}"#;
        assert_eq!(parse_review_manifest(pre_lt1.as_bytes()), None);
        // And so are the obvious refusals.
        assert_eq!(parse_review_manifest(b""), None);
        assert_eq!(parse_review_manifest(b"not json"), None);
        assert_eq!(
            parse_review_manifest(br#"{"schema_version":"something.else/v1"}"#),
            None
        );
    }

    #[test]
    fn a_manifest_carrying_only_zero_counts_is_a_review_that_says_so() {
        let manifest = parse_review_manifest(
            manifest(",\"lyrics_unresolved\":0,\"lyrics_review_flags\":0", "null").as_bytes(),
        )
        .expect("the count keys alone mark an LT1 manifest");
        assert_eq!(manifest.unresolved, 0);
        assert_eq!(manifest.flagged, 0);
        let review = LyricsReview::from_manifest(&manifest);
        assert!(review.is_clear());
        assert_eq!(review.summary(), "All lines placed, none flagged");
    }

    #[test]
    fn manifest_document_names_cannot_escape_the_job_folder() {
        // The names are resolved against a directory the caller already trusts,
        // so a manifest naming an absolute path or a traversal must arrive as a
        // plain file name or not at all.
        let manifest = parse_review_manifest(
            r#"{"schema_version":"musializer.assist-manifest/v1",
                "artifacts":{"aligned":"/etc/shadow","sync":"../../../etc/passwd"},
                "result_counts":{"lyrics_unresolved":0}}"#
                .as_bytes(),
        )
        .expect("manifest");
        assert_eq!(manifest.documents, vec!["shadow", "passwd"]);
        assert!(manifest.documents.iter().all(|name| !name.contains('/')));
    }

    #[test]
    fn a_document_names_every_flagged_line_with_a_window_to_look_at() {
        let manifest = parse_review_manifest(
            manifest(
                ",\"lyrics_unresolved\":2,\"lyrics_review_flags\":3",
                r#"{"policy":"anchor-block-mms","policy_version":"3"}"#,
            )
            .as_bytes(),
        )
        .expect("manifest");
        assert_eq!(
            manifest.documents,
            vec!["lyrics.aligned.json", "lyrics.sync.json"]
        );

        let mut review = LyricsReview::from_manifest(&manifest);
        assert!(review.read_document(REVIEW_DOCUMENT.as_bytes()));
        assert_eq!(review.unresolved, 2);
        assert_eq!(review.flagged, 3);
        assert_eq!(review.omitted, 0);
        assert_eq!(review.policy, "anchor-block-mms");
        assert!(!review.is_clear());
        assert_eq!(review.summary(), "2 unresolved, 3 flagged for review");

        // Unresolved first, then the abstention, then the disagreement — the
        // lines with no cue to jump to are the ones a short list must keep.
        let described: Vec<String> = review
            .entries
            .iter()
            .map(LyricReviewEntry::describe)
            .collect();
        assert_eq!(
            described,
            vec![
                "UNPLACED line 26 1:30.6-1:34.2 \"we were never meant to stay\"",
                "AMBIGUOUS line 31 not placed \"and again\"",
                "CHECK line 5 2:14.6-2:16.0 \"rawr.\"",
            ],
            "line numbers are 1-based, and an unresolved line shows the coarse window"
        );
    }

    #[test]
    fn a_document_with_no_aggregate_flag_array_is_read_from_its_parts() {
        // `unresolved[]` plus per-cue `review_flagged` is what the schema
        // requires; `review_flags[]` is the convenience over it.
        let document = r#"{
            "localization_policy": "anchor-block-mms",
            "lines": [{"reference_line_index": 4, "text": "rawr.",
                       "start_seconds": 134.6, "end_seconds": 136.0,
                       "review_flagged": true, "status": "aligned"},
                      {"reference_line_index": 6, "text": "quiet one",
                       "start_seconds": 140.0, "end_seconds": 141.0,
                       "review_flagged": false}],
            "unresolved": [{"reference_line_index": 25, "text": "we were never meant to stay",
                            "reason": "no block placement"}]
        }"#;
        let mut review = LyricsReview::default();
        assert!(review.read_document(document.as_bytes()));
        assert_eq!(review.unresolved, 1);
        assert_eq!(review.flagged, 2);
        assert_eq!(review.entries.len(), 2);
        assert_eq!(review.entries[0].kind, LyricReviewKind::Unresolved);
        assert_eq!(review.entries[1].kind, LyricReviewKind::Disagreement);
        assert_eq!(review.entries[1].line_number, 5);
    }

    #[test]
    fn a_pre_lt1_document_leaves_the_review_untouched() {
        let mut review = LyricsReview::default();
        assert!(
            !review.read_document(
                br#"{"lane":"lyric_sync","lines":[{"reference_line_index":0,"text":"a"}]}"#
            ),
            "a document with neither array is not an LT1 lane"
        );
        assert_eq!(review, LyricsReview::default());
        assert!(!review.read_document(b"{"));
        assert!(!review.read_document(&vec![b'x'; REVIEW_INPUT_MAX_BYTES + 1]));
    }

    #[test]
    fn the_named_list_is_bounded_and_says_how_many_it_dropped() {
        let mut flags = String::new();
        let mut unresolved = String::new();
        for index in 0..(REVIEW_ENTRY_CAPACITY + 5) {
            if index > 0 {
                flags.push(',');
                unresolved.push(',');
            }
            flags.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"line {index}","flag":"unresolved","reason":"x"}}"#
            ));
            unresolved.push_str(&format!(
                r#"{{"reference_line_index":{index},"text":"line {index}","reason":"x"}}"#
            ));
        }
        let document = format!(r#"{{"unresolved":[{unresolved}],"review_flags":[{flags}]}}"#);
        let mut review = LyricsReview::default();
        assert!(review.read_document(document.as_bytes()));
        assert_eq!(review.flagged, REVIEW_ENTRY_CAPACITY + 5);
        assert_eq!(review.entries.len(), REVIEW_ENTRY_CAPACITY);
        assert_eq!(review.omitted, 5);
    }

    #[test]
    fn text_and_timestamps_from_the_helper_are_bounded_before_they_are_drawn() {
        let long = "l".repeat(REVIEW_TEXT_MAX_CHARS * 4);
        let document = format!(
            r#"{{"unresolved":[],"review_flags":[
                {{"reference_line_index":0,"text":"{long}","flag":"coarse_disagreement",
                  "reason":"{long}","start_seconds":-4.0,"end_seconds":"soon"}}]}}"#
        );
        let mut review = LyricsReview::default();
        assert!(review.read_document(document.as_bytes()));
        let entry = &review.entries[0];
        assert!(entry.text.chars().count() <= REVIEW_TEXT_MAX_CHARS + 3);
        assert!(entry.reason.chars().count() <= REVIEW_TEXT_MAX_CHARS + 3);
        assert_eq!(
            entry.window(),
            "not placed",
            "a negative timestamp, or one that is not a number at all, is refused \
             rather than drawn"
        );
    }

    #[test]
    fn a_review_can_only_attach_to_a_candidate_that_has_a_lyrics_lane() {
        let mut review = LyricsReview::default();
        assert!(review.read_document(REVIEW_DOCUMENT.as_bytes()));

        let mut with_lyrics =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        assert!(
            with_lyrics.lyrics_review().is_none(),
            "absent until attached"
        );
        assert!(with_lyrics.attach_lyrics_review(review.clone()));
        assert_eq!(with_lyrics.lyrics_review().map(|r| r.flagged), Some(3));

        let sections_only = Lanes {
            lyrics: false,
            sections: true,
            semantics: false,
        };
        let mut without =
            AnalysisCandidate::prepare(&bridge(), sections_only, DURATION, SCENES).unwrap();
        assert!(!without.attach_lyrics_review(review));
        assert!(without.lyrics_review().is_none());
    }

    #[test]
    fn attaching_a_review_cannot_turn_an_unresolved_line_into_a_cue() {
        // The invariant LT1 rests on, asserted rather than assumed: the review
        // is inert, and `apply` never reads it.
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let staged_before = candidate.lyrics().clone();
        let mut review = LyricsReview::default();
        assert!(review.read_document(REVIEW_DOCUMENT.as_bytes()));
        assert!(candidate.attach_lyrics_review(review));
        assert_eq!(candidate.lyrics(), &staged_before);
        assert_eq!(candidate.lyrics().len(), 2);

        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        let mut semantic_events = EventTimeline::new();
        candidate
            .apply(&mut lyrics, &mut sections, &mut semantic_events)
            .unwrap();
        assert_eq!(
            lyrics.len(),
            2,
            "three review entries must not have become cues"
        );
    }
}
