//! Timed lyric cues and their editorial timing.
//!
//! **Owner: Agent B.** Port of `../musializer/src/lyrics.c/.h`.
//!
//! This model owns transcribed or written lyric content and its editorial
//! timing. Semantic interpretation belongs in a separate analysis lane — that
//! separation is one of the invariants the rewrite carries over, not a filing
//! convenience.
//!
//! Two C results have no Rust counterpart because Rust removes the failure mode
//! rather than handling it: `LYRICS_ERROR_NULL` (no null pointers) and
//! `LYRICS_ERROR_SCHEMA` (the document's schema version is a constant, not a
//! field a caller can corrupt). `LYRICS_ERROR_ALLOCATION` is also gone: the C
//! bridge importer heap-allocates a whole 512 KiB staging document because it
//! must not touch the destination before it validates, and a `Vec` stages the
//! same way without a fallible allocation in the API.

use core::cmp::Ordering;
use core::fmt;

/// Maximum cues in one document (`lyrics.h:11`).
pub const CUE_CAPACITY: usize = 1024;
/// Maximum cue text in **UTF-8 bytes** (`lyrics.h:12`, capacity 512 minus its
/// NUL).
///
/// Bytes, not characters. A `chars().count()` check here would accept documents
/// the C rejects and the schema forbids (`project-v1.schema.json:6`).
pub const TEXT_MAX_BYTES: usize = 511;
/// Bridge format version (`lyrics.h:13`).
pub const BRIDGE_VERSION: u64 = 1;
/// Largest bridge document the importer will look at (`lyrics.h:14-15`).
pub const BRIDGE_MAX_BYTES: usize = 64 + CUE_CAPACITY * (64 + 4 * (TEXT_MAX_BYTES + 1).div_ceil(3));

/// Why a lyric operation failed (`Lyrics_Result`, `lyrics.h:34-51`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LyricsError {
    #[error("invalid document duration")]
    Duration,
    #[error("lyrics capacity exceeded")]
    Capacity,
    #[error("invalid lyric cue")]
    InvalidCue,
    #[error("invalid UTF-8 lyric text")]
    InvalidUtf8,
    #[error("lyric text exceeds fixed capacity")]
    TextTooLong,
    #[error("duplicate lyric cue id")]
    DuplicateId,
    #[error("lyric cue id space exhausted")]
    IdExhausted,
    #[error("lyric cue not found")]
    NotFound,
    #[error("lyric cues are not canonically ordered")]
    Order,
    #[error("lyric cues are not adjacent")]
    NotAdjacent,
    #[error("malformed lyrics bridge")]
    BridgeFormat,
}

/// A validation failure with the cue it concerns (`Lyrics_Validation`,
/// `lyrics.h:53-57`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LyricsValidation {
    pub error: LyricsError,
    pub index: usize,
    pub related_index: usize,
}

/// User-facing: the notice tray shows this verbatim, so it names the cue
/// (1-based, as the editor numbers rows) rather than the variant.
impl fmt::Display for LyricsValidation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cue {}: {}", self.index + 1, self.error)
    }
}

impl std::error::Error for LyricsValidation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Where a cue's timing came from, and how much it should be trusted (LX1).
///
/// Not from the oracle. The C has one kind of cue, so its editor could not tell
/// a line the user placed by ear from one an aligner guessed at, and after an
/// assist run every block in the lane was the same amber. That is the defect
/// this enum exists for: the operator's complaint was not that the placements
/// were wrong, it was that nothing on screen said *which* ones to check.
///
/// The order is deliberate — it runs from most to least trustworthy, so
/// `PartialOrd` sorts a review list the way a user would read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CueOrigin {
    /// A human placed or confirmed this timing. The default, because everything
    /// the editor creates and everything a pre-LX1 file contains is one: a cue
    /// that reached a `.musi` before this field existed was reviewed by whoever
    /// saved it, and calling that "unknown" would flag a whole finished project.
    #[default]
    UserApplied,
    /// An aligner placed it and the cross-view check agreed.
    InferredCertain,
    /// An aligner placed it and something disagreed — the coarse and fine views
    /// differ by more than the review tolerance, or the line is a repeated
    /// phrase whose occurrence was resolved weakly. The text is right; the
    /// second is the question.
    InferredAmbiguous,
    /// **Not a placement.** A line the localizer could not pin at all, parked at
    /// whatever coarse window proposed it so the user has something to drag.
    ///
    /// A `Potential` cue is editing scaffolding, never content: [`LyricsDocument::at_time`]
    /// refuses to hand one to a frame, so it cannot reach the preview or an
    /// export until a human moves it — and moving it is what promotes it.
    Potential,
}

impl CueOrigin {
    /// The persisted token, and the one a report line prints.
    ///
    /// Short and lower-case because it is a file field first: the label below is
    /// for the interface and may be rewritten freely, this may not.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            CueOrigin::UserApplied => "user",
            CueOrigin::InferredCertain => "certain",
            CueOrigin::InferredAmbiguous => "ambiguous",
            CueOrigin::Potential => "potential",
        }
    }

    /// The reverse of [`CueOrigin::token`]. `None` is a hard parse error in the
    /// codec, which does not tolerate unknown input anywhere else either.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "user" => Some(CueOrigin::UserApplied),
            "certain" => Some(CueOrigin::InferredCertain),
            "ambiguous" => Some(CueOrigin::InferredAmbiguous),
            "potential" => Some(CueOrigin::Potential),
            _ => None,
        }
    }

    /// What a tooltip and a legend call it — the operator's own words.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            CueOrigin::UserApplied => "User applied",
            CueOrigin::InferredCertain => "AI inferred (certain)",
            CueOrigin::InferredAmbiguous => "AI inferred (ambiguous)",
            CueOrigin::Potential => "Potential cue (highly uncertain)",
        }
    }

    /// Whether a frame may show this cue's text.
    ///
    /// One predicate rather than a `== Potential` at each call site, because
    /// "does this reach the screen" is the question, and a future origin that
    /// also should not reach it must not have to find every comparison.
    #[must_use]
    pub const fn is_displayable(self) -> bool {
        !matches!(self, CueOrigin::Potential)
    }
}

/// One timed line (`Lyric_Cue`, `lyrics.h:17-22`), plus LX1's origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricCue {
    /// `0` asks [`LyricsDocument::insert`] to allocate a stable id.
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    /// How this timing came to be (LX1). Defaults to [`CueOrigin::UserApplied`],
    /// which is what makes every constructor and every older file unchanged.
    pub origin: CueOrigin,
}

/// The part of one cue's span that a later cue takes over (review 1.14).
///
/// Produced by [`LyricsDocument::cue_shadow`]. Not from the oracle: the C has no
/// notion of this, which is exactly why a swallowed line was invisible in it too.
#[derive(Clone, Debug, PartialEq)]
pub struct CueShadow {
    /// Canonical index of the cue that loses time, and its id.
    pub index: usize,
    pub id: u64,
    /// The **first** cue that takes part of the span, for naming one culprit in a
    /// one-line warning. There may be more; the intervals below are all of them.
    pub shadowed_by_index: usize,
    pub shadowed_by_id: u64,
    /// No instant of the span resolves to this cue: the line never displays.
    pub fully: bool,
    /// The hidden intervals, merged and in order, clipped to the cue's own span.
    /// Never empty — a cue with nothing hidden has no `CueShadow` at all.
    pub hidden: Vec<(f64, f64)>,
}

impl CueShadow {
    /// The first instant the cue stops displaying. What the form's warning names.
    #[must_use]
    pub fn from_seconds(&self) -> f64 {
        self.hidden.first().map_or(f64::NAN, |span| span.0)
    }
}

/// Canonical order: `(start, end, id)` (`lyrics.c:19-28`).
///
/// The id tiebreak makes the order total, so no two cues in a valid document ever
/// compare equal and every sort of a valid document is deterministic.
fn compare(left: &LyricCue, right: &LyricCue) -> Ordering {
    left.start_seconds
        .partial_cmp(&right.start_seconds)
        .unwrap_or(Ordering::Equal)
        .then(
            left.end_seconds
                .partial_cmp(&right.end_seconds)
                .unwrap_or(Ordering::Equal),
        )
        .then(left.id.cmp(&right.id))
}

/// `validate_text` (`lyrics.c:30-69`), for text that is already known-UTF-8.
///
/// The C function does three things: bounds the byte length, rejects empty text,
/// and rejects control characters other than tab. Its UTF-8 pass is exactly what
/// `str` already guarantees — it rejects overlongs (`>= 0xC2` for two-byte
/// leads), surrogates, and anything past `U+10FFFF`, which is
/// `core::str::from_utf8`'s acceptance set. So on a `&str` only the other two
/// checks remain.
///
/// One deliberate difference: an interior `U+0000` is rejected here as a control
/// character, where C's NUL-terminated buffer would silently treat it as the end
/// of the text and accept the prefix. Truncating a lyric on a byte the user
/// cannot see is worse than refusing it.
pub fn validate_text(text: &str) -> Result<(), LyricsError> {
    if text.len() > TEXT_MAX_BYTES {
        return Err(LyricsError::TextTooLong);
    }
    if text.is_empty() {
        return Err(LyricsError::InvalidCue);
    }
    if text
        .bytes()
        .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0)
    {
        return Err(LyricsError::InvalidCue);
    }
    Ok(())
}

/// The same rule for bytes off a wire, where invalid UTF-8 is possible.
pub fn validate_text_bytes(text: &[u8]) -> Result<&str, LyricsError> {
    if text.len() > TEXT_MAX_BYTES {
        return Err(LyricsError::TextTooLong);
    }
    let text = core::str::from_utf8(text).map_err(|_| LyricsError::InvalidUtf8)?;
    validate_text(text).map(|()| text)
}

/// Appends `addition` to `text` in full or not at all (`lyrics_text_append`,
/// `lyrics.c:81-130`).
///
/// Returns whether line breaks or tabs were flattened to single spaces, so the
/// caller can tell the user what it did. Truncating to fit is specifically not
/// allowed: it would cut a multi-byte sequence in half and the result would fail
/// the validation every stored cue must pass. A rejected paste leaves `text`
/// byte-for-byte as it was.
///
/// Any other control character — including `0x7F`, which stored text may
/// legitimately contain (`validate_text` does not check for it) — rejects the
/// whole paste rather than being stripped, so what lands in the cue is always
/// what the user can see.
pub fn text_append(text: &mut String, addition: &str) -> Result<bool, LyricsError> {
    if text.len() > TEXT_MAX_BYTES {
        return Err(LyricsError::TextTooLong);
    }
    let mut staged = String::with_capacity(addition.len());
    let mut flattened = false;
    for byte in addition.bytes() {
        let byte = match byte {
            b'\n' | b'\r' | b'\t' => {
                flattened = true;
                b' '
            }
            byte if byte < 0x20 || byte == 0x7F => return Err(LyricsError::InvalidCue),
            byte => byte,
        };
        // Every replacement above is single-byte ASCII, so it can never land
        // inside a multi-byte sequence.
        staged.push(byte as char);
        if staged.len() > TEXT_MAX_BYTES {
            return Err(LyricsError::TextTooLong);
        }
    }
    if staged.is_empty() {
        return Err(LyricsError::InvalidCue);
    }
    if text.len() + staged.len() > TEXT_MAX_BYTES {
        return Err(LyricsError::TextTooLong);
    }

    // The joined text has to satisfy exactly the contract a stored cue does,
    // which is what catches a paste splicing two halves of a sequence.
    let mut combined = text.clone();
    combined.push_str(&staged);
    validate_text(&combined)?;
    *text = combined;
    Ok(flattened)
}

/// A validated, ordered set of lyric cues (`Lyrics_Document`, `lyrics.h:24-32`).
///
/// `duration_seconds` is part of the persistence contract and bounds every cue,
/// which is why there is no way to construct a document without one.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricsDocument {
    duration_seconds: f64,
    next_id: u64,
    revision: u64,
    cues: Vec<LyricCue>,
}

impl LyricsDocument {
    /// `lyrics_document_init` (`lyrics.c:172-184`).
    pub fn new(duration_seconds: f64) -> Result<Self, LyricsError> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(LyricsError::Duration);
        }
        Ok(Self {
            duration_seconds,
            next_id: 1,
            revision: 1,
            cues: Vec::new(),
        })
    }

    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    #[must_use]
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn cues(&self) -> &[LyricCue] {
        &self.cues
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Used by the `.musi` codec, which reads `next_id` from the file and must be
    /// able to reconstruct a document exactly as written before validating it.
    pub(crate) fn set_next_id(&mut self, next_id: u64) {
        self.next_id = next_id;
    }

    /// Used by the `.musi` codec: cues arrive in file order and are validated as a
    /// whole afterwards, exactly as the C parser does (`project_io.c:220-221`),
    /// rather than being inserted one at a time through the editing path.
    pub(crate) fn push_unvalidated(&mut self, cue: LyricCue) {
        self.cues.push(cue);
    }

    /// Used by the `.musi` codec: `duration_seconds` is not a lyrics field in the
    /// file, it is copied from `audio.duration_seconds` after parsing
    /// (`project_io.c:306`).
    pub(crate) fn set_duration_unvalidated(&mut self, duration_seconds: f64) {
        self.duration_seconds = duration_seconds;
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.revision = 1;
        }
    }

    fn validate_cue(&self, cue: &LyricCue) -> Result<(), LyricsError> {
        if cue.id == 0
            || !cue.start_seconds.is_finite()
            || !cue.end_seconds.is_finite()
            || cue.start_seconds < 0.0
            || cue.end_seconds <= cue.start_seconds
            || cue.end_seconds > self.duration_seconds
        {
            return Err(LyricsError::InvalidCue);
        }
        validate_text(&cue.text)
    }

    /// `lyrics_document_validate` (`lyrics.c:186-214`).
    pub fn validate(&self) -> Result<(), LyricsValidation> {
        let fail = |error, index, related_index| LyricsValidation {
            error,
            index,
            related_index,
        };
        if !self.duration_seconds.is_finite() || self.duration_seconds <= 0.0 {
            return Err(fail(LyricsError::Duration, 0, 0));
        }
        if self.cues.len() > CUE_CAPACITY {
            return Err(fail(LyricsError::Capacity, self.cues.len(), 0));
        }
        for (index, cue) in self.cues.iter().enumerate() {
            if let Err(error) = self.validate_cue(cue) {
                return Err(fail(error, index, 0));
            }
            if index > 0 && compare(&self.cues[index - 1], cue) != Ordering::Less {
                return Err(fail(LyricsError::Order, index, index - 1));
            }
            if let Some(previous) = self.cues[..index]
                .iter()
                .position(|previous| previous.id == cue.id)
            {
                return Err(fail(LyricsError::DuplicateId, index, previous));
            }
            // `next_id == 0` means the id space is exhausted, and then every id
            // below it is legitimately in use.
            if self.next_id != 0 && cue.id >= self.next_id {
                return Err(fail(LyricsError::InvalidCue, index, 0));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn index_of(&self, id: u64) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.cues.iter().position(|cue| cue.id == id)
    }

    /// `lyrics_find` (`lyrics.c:489-493`).
    #[must_use]
    pub fn find(&self, id: u64) -> Option<&LyricCue> {
        self.index_of(id).map(|index| &self.cues[index])
    }

    /// `lyrics_insert` (`lyrics.c:216-246`).
    ///
    /// `cue.id == 0` allocates a deterministic, never-reused stable id. An
    /// explicit nonzero id supports import and persistence round-trips, and
    /// advances `next_id` past itself so a later allocation cannot collide.
    ///
    /// # The origin rule (LX1)
    ///
    /// This is the only mutator that takes a [`CueOrigin`] from its caller,
    /// because it is the only one an *importer* uses. Every other editing
    /// operation on this document — [`Self::update`], [`Self::shift_many`],
    /// [`Self::split`], [`Self::merge`] — promotes what it touches to
    /// [`CueOrigin::UserApplied`], and that is the point rather than a
    /// side effect: the colour code in the lane is only worth reading if
    /// "still amber" reliably means "nobody has looked at this yet". A drag
    /// that quietly left a cue marked *inferred* would make the whole signal
    /// untrustworthy in the one direction that matters.
    ///
    /// Promotion lives here, on the model, and not at the call sites, because
    /// there are six call sites and one of them will be added by somebody who
    /// has not read this paragraph.
    pub fn insert(&mut self, cue: LyricCue) -> Result<u64, LyricsError> {
        if self.cues.len() >= CUE_CAPACITY {
            return Err(LyricsError::Capacity);
        }
        let mut candidate = cue;
        if candidate.id == 0 {
            if self.next_id == 0 {
                return Err(LyricsError::IdExhausted);
            }
            candidate.id = self.next_id;
        }
        self.validate_cue(&candidate)?;
        if self.index_of(candidate.id).is_some() {
            return Err(LyricsError::DuplicateId);
        }

        let mut next_id = self.next_id;
        if candidate.id >= next_id && next_id != 0 {
            next_id = if candidate.id == u64::MAX {
                0
            } else {
                candidate.id + 1
            };
        }
        let id = candidate.id;
        let at = self
            .cues
            .partition_point(|existing| compare(existing, &candidate) == Ordering::Less);
        self.cues.insert(at, candidate);
        self.next_id = next_id;
        self.bump_revision();
        Ok(id)
    }

    /// `lyrics_update` (`lyrics.c:248-267`).
    pub fn update(
        &mut self,
        id: u64,
        start_seconds: f64,
        end_seconds: f64,
        text: &str,
    ) -> Result<(), LyricsError> {
        let index = self.index_of(id).ok_or(LyricsError::NotFound)?;
        validate_text(text)?;
        let replacement = LyricCue {
            id,
            start_seconds,
            end_seconds,
            text: text.to_owned(),
            // Promoted, per the rule on [`LyricsDocument::insert`]: a human just
            // decided what this cue's timing is.
            origin: CueOrigin::UserApplied,
        };
        self.validate_cue(&replacement)?;
        self.cues[index] = replacement;
        self.sort_cues();
        self.bump_revision();
        Ok(())
    }

    /// `lyrics_delete` (`lyrics.c:269-280`).
    pub fn delete(&mut self, id: u64) -> Result<(), LyricsError> {
        let index = self.index_of(id).ok_or(LyricsError::NotFound)?;
        self.cues.remove(index);
        self.bump_revision();
        Ok(())
    }

    /// `lyrics_nudge` (`lyrics.c:282-293`).
    pub fn nudge(&mut self, id: u64, delta_seconds: f64) -> Result<(), LyricsError> {
        if !delta_seconds.is_finite() {
            return Err(LyricsError::InvalidCue);
        }
        let index = self.index_of(id).ok_or(LyricsError::NotFound)?;
        let (start, end, text) = {
            let cue = &self.cues[index];
            (
                cue.start_seconds + delta_seconds,
                cue.end_seconds + delta_seconds,
                cue.text.clone(),
            )
        };
        self.update(id, start, end, &text)
    }

    /// `lyrics_retime` (`lyrics.c:295-305`): moves one cue's boundaries without
    /// touching its text. This is what dragging a block's edge commits.
    pub fn retime(
        &mut self,
        id: u64,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<(), LyricsError> {
        let index = self.index_of(id).ok_or(LyricsError::NotFound)?;
        let text = self.cues[index].text.clone();
        self.update(id, start_seconds, end_seconds, &text)
    }

    /// Resolves a selection to cue indices, collapsing repeats
    /// (`selection_resolve`, `lyrics.c:320-336`).
    ///
    /// An id that is not in the document rejects the whole request, because a
    /// stale selection is a bug in the caller and silently moving the rest would
    /// hide it. An empty selection is `NotFound`, not a successful no-op.
    fn selection(&self, ids: &[u64]) -> Result<Vec<usize>, LyricsError> {
        if ids.is_empty() {
            return Err(LyricsError::NotFound);
        }
        let mut selected = Vec::with_capacity(ids.len());
        for id in ids {
            let index = self.index_of(*id).ok_or(LyricsError::NotFound)?;
            if !selected.contains(&index) {
                selected.push(index);
            }
        }
        Ok(selected)
    }

    /// Restores canonical order after a bulk edit (`sort_cues`,
    /// `lyrics.c:342-353`).
    fn sort_cues(&mut self) {
        self.cues.sort_by(compare);
    }

    /// `lyrics_shift_many` (`lyrics.c:355-385`): moves every listed cue by the
    /// same delta, in full or not at all.
    ///
    /// Dragging a multi-cue selection has to be one operation: applying it cue by
    /// cue would leave the document half-moved at the first cue that hits zero or
    /// the end of the track, and there is no undo to recover with.
    pub fn shift_many(&mut self, ids: &[u64], delta_seconds: f64) -> Result<(), LyricsError> {
        if !delta_seconds.is_finite() {
            return Err(LyricsError::InvalidCue);
        }
        let selected = self.selection(ids)?;

        // Individual validity is the only thing that can fail: the selection keeps
        // its internal order under a uniform shift, unselected cues never move,
        // and no two cues can compare equal, so the sort below always produces a
        // strictly ordered document.
        for index in &selected {
            let candidate = LyricCue {
                start_seconds: self.cues[*index].start_seconds + delta_seconds,
                end_seconds: self.cues[*index].end_seconds + delta_seconds,
                ..self.cues[*index].clone()
            };
            self.validate_cue(&candidate)?;
        }
        for index in &selected {
            self.cues[*index].start_seconds += delta_seconds;
            self.cues[*index].end_seconds += delta_seconds;
            // A drag is the gesture that turns a proposal into a placement, so
            // it is also the gesture that has to say so. See
            // [`LyricsDocument::insert`] for why promotion lives on the model's
            // editing operations rather than at each call site.
            //
            // Guarded on a real delta: a drag the clamp refused moved nothing,
            // and marking a whole selection reviewed because someone pushed it
            // against the end of the track would be the colour code lying.
            if delta_seconds != 0.0 {
                self.cues[*index].origin = CueOrigin::UserApplied;
            }
        }
        self.sort_cues();
        self.bump_revision();
        Ok(())
    }

    /// `lyrics_shift_headroom` (`lyrics.c:387-414`): the largest delta
    /// [`Self::shift_many`] would accept in each direction.
    ///
    /// Lets a drag be clamped as it happens instead of snapping back on release.
    /// Both values are `>= 0`; either may be `0` when the selection is already
    /// against that end of the track.
    pub fn shift_headroom(&self, ids: &[u64]) -> Result<(f64, f64), LyricsError> {
        let selected = self.selection(ids)?;
        let mut earliest_start = self.duration_seconds;
        let mut latest_end = 0.0f64;
        for index in selected {
            earliest_start = earliest_start.min(self.cues[index].start_seconds);
            latest_end = latest_end.max(self.cues[index].end_seconds);
        }
        Ok((
            earliest_start.max(0.0),
            (self.duration_seconds - latest_end).max(0.0),
        ))
    }

    /// `lyrics_split` (`lyrics.c:416-452`): keeps `id` on the left cue and
    /// allocates a new id for the right one. One logical edit, one revision bump.
    pub fn split(
        &mut self,
        id: u64,
        split_seconds: f64,
        left_text: &str,
        right_text: &str,
    ) -> Result<u64, LyricsError> {
        if self.cues.len() >= CUE_CAPACITY {
            return Err(LyricsError::Capacity);
        }
        if self.next_id == 0 {
            return Err(LyricsError::IdExhausted);
        }
        let index = self.index_of(id).ok_or(LyricsError::NotFound)?;
        let original = self.cues[index].clone();
        if !split_seconds.is_finite()
            || split_seconds <= original.start_seconds
            || split_seconds >= original.end_seconds
        {
            return Err(LyricsError::InvalidCue);
        }
        validate_text(left_text)?;
        validate_text(right_text)?;

        // Both halves are promoted: the user chose where the seam goes, which is
        // a timing decision about each of them.
        self.cues[index] = LyricCue {
            id,
            start_seconds: original.start_seconds,
            end_seconds: split_seconds,
            text: left_text.to_owned(),
            origin: CueOrigin::UserApplied,
        };
        self.sort_cues();
        let right = LyricCue {
            id: 0,
            start_seconds: split_seconds,
            end_seconds: original.end_seconds,
            text: right_text.to_owned(),
            origin: CueOrigin::UserApplied,
        };
        match self.insert(right) {
            Ok(right_id) => Ok(right_id),
            Err(error) => {
                let left_index = self.index_of(id).expect("the left cue is still present");
                self.cues[left_index] = original;
                self.sort_cues();
                Err(error)
            }
        }
    }

    /// `lyrics_merge` (`lyrics.c:454-487`): joins two consecutive cues in
    /// canonical order, keeping the first cue's id and spanning both timings.
    pub fn merge(
        &mut self,
        first_id: u64,
        second_id: u64,
        separator: &str,
    ) -> Result<(), LyricsError> {
        let first = self.index_of(first_id).ok_or(LyricsError::NotFound)?;
        let second = self.index_of(second_id).ok_or(LyricsError::NotFound)?;
        if second != first + 1 {
            return Err(LyricsError::NotAdjacent);
        }
        // An empty separator is legal even though empty *text* is not
        // (`lyrics.c:463`).
        if !separator.is_empty() {
            validate_text(separator)?;
        }
        let combined_length =
            self.cues[first].text.len() + separator.len() + self.cues[second].text.len();
        if combined_length > TEXT_MAX_BYTES {
            return Err(LyricsError::TextTooLong);
        }

        let tail = self.cues[second].clone();
        let merged = &mut self.cues[first];
        merged.text.push_str(separator);
        merged.text.push_str(&tail.text);
        if tail.end_seconds > merged.end_seconds {
            merged.end_seconds = tail.end_seconds;
        }
        // The survivor carries text and a span the user just composed, so its
        // provenance is theirs whatever the two halves were.
        merged.origin = CueOrigin::UserApplied;
        self.cues.remove(second);
        self.sort_cues();
        self.bump_revision();
        Ok(())
    }

    /// `lyrics_document_replace` (`lyrics.c:501-512`): validates the source before
    /// replacing the destination, and advances the destination's revision once.
    pub fn replace(&mut self, source: &LyricsDocument) -> Result<(), LyricsValidation> {
        source.validate()?;
        let next = self.revision.wrapping_add(1);
        let revision = if next == 0 { 1 } else { next };
        self.duration_seconds = source.duration_seconds;
        self.next_id = source.next_id;
        self.cues.clear();
        self.cues.extend_from_slice(&source.cues);
        self.revision = revision;
        Ok(())
    }

    /// `lyrics_document_normalize_duration` (`lyrics.c:514-542`).
    ///
    /// Copies a validated document onto an authoritative decoded-audio duration.
    /// Cues crossing the tail are clamped; cues beginning at or after the new end
    /// reject the whole operation, because clamping those would produce a
    /// zero-length cue rather than a shorter one.
    pub fn normalize_duration(
        &mut self,
        source: &LyricsDocument,
        duration_seconds: f64,
    ) -> Result<(), LyricsValidation> {
        let fail = |error| LyricsValidation {
            error,
            index: 0,
            related_index: 0,
        };
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(fail(LyricsError::Duration));
        }
        source.validate()?;
        if source
            .cues
            .iter()
            .any(|cue| cue.start_seconds >= duration_seconds)
        {
            return Err(fail(LyricsError::Duration));
        }

        let next = self.revision.wrapping_add(1);
        let revision = if next == 0 { 1 } else { next };
        self.next_id = source.next_id;
        self.cues.clear();
        self.cues.extend_from_slice(&source.cues);
        self.duration_seconds = duration_seconds;
        for cue in &mut self.cues {
            if cue.end_seconds > duration_seconds {
                cue.end_seconds = duration_seconds;
            }
        }
        self.revision = revision;
        Ok(())
    }

    /// `lyrics_at_time` (`lyrics.c:544-556`): the most recently started active
    /// cue, or `None` outside all cues. Overlaps are legal in this lane, which is
    /// why "most recently started" rather than "the one" is the rule.
    ///
    /// **This is the display resolver**, and the single place a cue becomes a
    /// frame ([`crate::project::frame_lanes`] is its only non-test caller). So it
    /// is also where LX1's proposals are filtered: a [`CueOrigin::Potential`] cue
    /// is a parked guess, and letting one reach a caption would put a line the
    /// aligner explicitly failed to place on screen — and, worse, into an export,
    /// where it would look exactly like a placement that had been checked.
    ///
    /// Skipping rather than stopping: a proposal parked over a real cue must not
    /// hide the real one, which is what `continue` buys over letting the scan
    /// treat it as the active line.
    #[must_use]
    pub fn at_time(&self, time_seconds: f64) -> Option<&LyricCue> {
        if !time_seconds.is_finite() || time_seconds < 0.0 || self.validate().is_err() {
            return None;
        }
        let mut active = None;
        for cue in &self.cues {
            if cue.start_seconds > time_seconds {
                break;
            }
            if !cue.origin.is_displayable() {
                continue;
            }
            if time_seconds < cue.end_seconds {
                active = Some(cue);
            }
        }
        active
    }

    /// Which part of the cue at `index` [`Self::at_time`] will never hand back
    /// (review 1.14), or `None` when all of it displays.
    ///
    /// Overlaps are legal here and `at_time` resolves them as "the last one in
    /// canonical order that is active", so an overlapping cue does not merge or
    /// alternate — it **replaces** the one under it for the length of the
    /// overlap, in the preview and in an export alike. Nothing in the editor said
    /// so: the lane drew both blocks identically and a user proofreading saw a
    /// line missing with nothing pointing at why.
    ///
    /// Additive signalling only. This reports what `at_time` already decides and
    /// must never change that decision — export determinism depends on it.
    #[must_use]
    // `!(to > from)` is NaN-rejecting where `to <= from` would let a non-finite
    // pair through as an interval; a document can hold one before it validates.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn cue_shadow(&self, index: usize) -> Option<CueShadow> {
        let cue = self.cues.get(index)?;
        // A proposal is not on screen to begin with, so it can neither lose time
        // nor take it. Reporting one as shadowed would hatch a block whose whole
        // point is that it is *not* content yet, and reporting one as the
        // shadower would accuse a real cue of being hidden by a guess (LX1).
        if !cue.origin.is_displayable() {
            return None;
        }
        let (start, end) = (cue.start_seconds, cue.end_seconds);

        let mut hidden: Vec<(f64, f64)> = Vec::new();
        let mut by = None;
        // Canonical order is `(start, end, id)`, so every cue after this one both
        // starts no earlier *and* wins the `at_time` scan against it. That is the
        // whole rule: later index, later word.
        for (later_index, later) in self.cues.iter().enumerate().skip(index + 1) {
            if later.start_seconds >= end {
                break;
            }
            if !later.origin.is_displayable() {
                continue;
            }
            let from = later.start_seconds.max(start);
            let to = later.end_seconds.min(end);
            if !(to > from) {
                continue;
            }
            if by.is_none() {
                by = Some((later_index, later.id));
            }
            match hidden.last_mut() {
                // Touching or overlapping runs merge, so "fully shadowed" is a
                // question about one interval rather than about a list.
                Some(previous) if from <= previous.1 => previous.1 = previous.1.max(to),
                _ => hidden.push((from, to)),
            }
        }

        let (shadowed_by_index, shadowed_by_id) = by?;
        let fully = hidden.len() == 1 && hidden[0].0 <= start && hidden[0].1 >= end;
        Some(CueShadow {
            index,
            id: cue.id,
            shadowed_by_index,
            shadowed_by_id,
            fully,
            hidden,
        })
    }

    /// Every shadowed cue in the document, in canonical order (review 1.14).
    ///
    /// One pass for a whole frame: the lane asks once and looks up by id, rather
    /// than asking per block and walking the document again each time.
    #[must_use]
    pub fn shadowed_cues(&self) -> Vec<CueShadow> {
        (0..self.cues.len())
            .filter_map(|index| self.cue_shadow(index))
            .collect()
    }

    /// `lyrics_bridge_export` (`lyrics.c:603-648`).
    ///
    /// The derived UI bridge, **not** the canonical persistence format:
    /// `MUSIALIZER-LYRICS-BRIDGE<TAB>1<TAB>duration_ms<LF>` then one
    /// `id<TAB>start_ms<TAB>end_ms<TAB>base64_utf8_text<LF>` per cue. Canonical and
    /// locale-independent by construction.
    ///
    /// C reports a `required_size` including its trailing NUL; the Rust equivalent
    /// is `returned.len() + 1`.
    pub fn bridge_export(&self) -> Result<String, LyricsError> {
        self.validate().map_err(|failure| failure.error)?;
        if self.duration_seconds > u64::MAX as f64 / 1000.0 {
            return Err(LyricsError::Duration);
        }
        let duration_ms = seconds_to_milliseconds(self.duration_seconds);
        if duration_ms == 0 {
            return Err(LyricsError::Duration);
        }
        let mut out = format!("MUSIALIZER-LYRICS-BRIDGE\t{BRIDGE_VERSION}\t{duration_ms}\n");
        for cue in &self.cues {
            let start_ms = seconds_to_milliseconds(cue.start_seconds);
            let end_ms = seconds_to_milliseconds(cue.end_seconds);
            if end_ms <= start_ms || end_ms > duration_ms {
                return Err(LyricsError::InvalidCue);
            }
            out.push_str(&format!("{}\t{start_ms}\t{end_ms}\t", cue.id));
            out.push_str(&base64_encode(cue.text.as_bytes()));
            out.push('\n');
        }
        Ok(out)
    }

    /// `lyrics_bridge_import` (`lyrics.c:709-753`): strict and atomic.
    ///
    /// Everything is staged into a fresh document and published through
    /// [`Self::replace`], so a malformed bridge leaves the destination untouched.
    pub fn bridge_import(&mut self, input: &[u8]) -> Result<(), LyricsError> {
        if input.is_empty() || input.len() > BRIDGE_MAX_BYTES || input.contains(&0) {
            return Err(LyricsError::BridgeFormat);
        }
        const PREFIX: &[u8] = b"MUSIALIZER-LYRICS-BRIDGE\t";
        let mut cursor = input
            .strip_prefix(PREFIX)
            .ok_or(LyricsError::BridgeFormat)?;

        let version = take_u64_field(&mut cursor, b'\t').ok_or(LyricsError::BridgeFormat)?;
        let duration_ms = take_u64_field(&mut cursor, b'\n').ok_or(LyricsError::BridgeFormat)?;
        if version != BRIDGE_VERSION || duration_ms == 0 {
            return Err(LyricsError::BridgeFormat);
        }

        let mut staged =
            LyricsDocument::new(duration_ms as f64 / 1000.0).map_err(|_| LyricsError::Duration)?;
        while !cursor.is_empty() {
            let id = take_u64_field(&mut cursor, b'\t').ok_or(LyricsError::BridgeFormat)?;
            let start_ms = take_u64_field(&mut cursor, b'\t').ok_or(LyricsError::BridgeFormat)?;
            let end_ms = take_u64_field(&mut cursor, b'\t').ok_or(LyricsError::BridgeFormat)?;
            if id == 0 {
                return Err(LyricsError::BridgeFormat);
            }
            let line_end = cursor
                .iter()
                .position(|byte| *byte == b'\n')
                .ok_or(LyricsError::BridgeFormat)?;
            let encoded = &cursor[..line_end];
            if encoded.is_empty() {
                return Err(LyricsError::BridgeFormat);
            }
            let decoded =
                base64_decode(encoded, TEXT_MAX_BYTES).map_err(|_| LyricsError::BridgeFormat)?;
            let text = core::str::from_utf8(&decoded).map_err(|_| LyricsError::BridgeFormat)?;
            staged.insert(LyricCue {
                id,
                start_seconds: start_ms as f64 / 1000.0,
                end_seconds: end_ms as f64 / 1000.0,
                text: text.to_owned(),
                // The bridge grammar has no origin column and gains none: it is
                // the round trip through an *external editor*, so what comes
                // back is by definition what a person decided. Widening the
                // wire format would only let a third party assert provenance
                // this application has no way to check.
                origin: CueOrigin::default(),
            })?;
            cursor = &cursor[line_end + 1..];
        }
        self.replace(&staged).map_err(|failure| failure.error)
    }
}

/// `seconds_to_milliseconds` (`lyrics.c:565-568`): round-half-up, not truncation.
fn seconds_to_milliseconds(seconds: f64) -> u64 {
    (seconds * 1000.0 + 0.5).floor() as u64
}

/// `parse_uint64_field` (`lyrics.c:650-665`): digits then exactly `delimiter`.
///
/// Leading zeros are accepted here (unlike the analysis bridge's stricter
/// `parse_u64`), and overflow is refused rather than wrapped.
fn take_u64_field(cursor: &mut &[u8], delimiter: u8) -> Option<u64> {
    let mut value: u64 = 0;
    let mut digits = 0usize;
    let mut at = 0usize;
    while at < cursor.len() && cursor[at].is_ascii_digit() {
        let digit = u64::from(cursor[at] - b'0');
        value = value.checked_mul(10)?.checked_add(digit)?;
        digits += 1;
        at += 1;
    }
    if digits == 0 || cursor.get(at) != Some(&delimiter) {
        return None;
    }
    *cursor = &cursor[at + 1..];
    Some(value)
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `base64_encode` (`lyrics.c:573-601`), standard alphabet with `=` padding.
pub(crate) fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(4 * input.len().div_ceil(3));
    let mut chunks = input.chunks_exact(3);
    for chunk in &mut chunks {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        for shift in [18, 12, 6, 0] {
            out.push(BASE64_ALPHABET[((bits >> shift) & 63) as usize] as char);
        }
    }
    match chunks.remainder() {
        [first] => {
            let bits = u32::from(*first) << 16;
            out.push(BASE64_ALPHABET[((bits >> 18) & 63) as usize] as char);
            out.push(BASE64_ALPHABET[((bits >> 12) & 63) as usize] as char);
            out.push_str("==");
        }
        [first, second] => {
            let bits = (u32::from(*first) << 16) | (u32::from(*second) << 8);
            for shift in [18, 12, 6] {
                out.push(BASE64_ALPHABET[((bits >> shift) & 63) as usize] as char);
            }
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Why a base64 field was refused. Shared with
/// [`crate::project::analysis_bridge`], which reports the three cases separately
/// (`analysis_bridge.c:107-135`) where the lyrics bridge collapses them into one
/// format error (`lyrics.c:677-707`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base64Error {
    /// Not canonical base64: bad character, bad length, or padding in the middle.
    Format,
    /// Decodes to more than the destination allows.
    TooLarge,
    /// Decodes to bytes containing `U+0000`, which no text field may hold.
    EmbeddedNul,
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// `base64_decode` (`lyrics.c:677-707`), including its refusal of non-canonical
/// nonzero padding bits — the check that stops two spellings decoding to the same
/// bytes.
pub(crate) fn base64_decode(input: &[u8], max_bytes: usize) -> Result<Vec<u8>, Base64Error> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if input.len() % 4 != 0 {
        return Err(Base64Error::Format);
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    for (group, quad) in input.chunks_exact(4).enumerate() {
        let final_group = (group + 1) * 4 == input.len();
        let a = base64_value(quad[0]).ok_or(Base64Error::Format)?;
        let b = base64_value(quad[1]).ok_or(Base64Error::Format)?;
        let c = if quad[2] == b'=' {
            None
        } else {
            Some(base64_value(quad[2]).ok_or(Base64Error::Format)?)
        };
        let d = if quad[3] == b'=' {
            None
        } else {
            Some(base64_value(quad[3]).ok_or(Base64Error::Format)?)
        };
        // Padding is only ever a suffix, and `==` may not be followed by data.
        if (c.is_none() && d.is_some()) || ((c.is_none() || d.is_none()) && !final_group) {
            return Err(Base64Error::Format);
        }
        if (c.is_none() && (b & 15) != 0) || (d.is_none() && c.is_some_and(|c| (c & 3) != 0)) {
            return Err(Base64Error::Format);
        }
        let bits = (u32::from(a) << 18)
            | (u32::from(b) << 12)
            | (u32::from(c.unwrap_or(0)) << 6)
            | u32::from(d.unwrap_or(0));
        let produced = 1 + usize::from(c.is_some()) + usize::from(d.is_some());
        if out.len() + produced > max_bytes {
            return Err(Base64Error::TooLarge);
        }
        out.push((bits >> 16) as u8);
        if c.is_some() {
            out.push((bits >> 8) as u8);
        }
        if d.is_some() {
            out.push(bits as u8);
        }
    }
    if out.contains(&0) {
        return Err(Base64Error::EmbeddedNul);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The TSV import transaction (D3)
// ---------------------------------------------------------------------------

/// Why an imported `.lyrics.tsv` was not accepted.
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeImportRefusal {
    /// The destination track has no usable length to import against.
    NoTrackLength(LyricsError),
    /// The bytes are not a bridge document.
    Format(LyricsError),
    /// The cues are a bridge document, but they do not fit this track.
    DoesNotFit {
        failure: LyricsValidation,
        /// The length the file was written against, for the message.
        source_duration_seconds: f64,
    },
}

impl fmt::Display for BridgeImportRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrackLength(error) => write!(f, "{error}"),
            Self::Format(error) => write!(f, "{error}"),
            Self::DoesNotFit { failure, .. } => write!(f, "{failure}"),
        }
    }
}

impl core::error::Error for BridgeImportRefusal {}

/// Reads a `.lyrics.tsv` and re-bases it onto a track of `duration_seconds` (D3).
///
/// Pure, and separated from the file and dialog plumbing for the reason
/// everything in this crate is: the interesting half of an import is what it
/// *refuses*, and a refusal path is where a difference hides. A test can hand
/// this a truncated file, a file from a different song, and one with a cue that
/// starts past the end, without a window or a picker.
///
/// Transactional in two layers rather than one. [`LyricsDocument::bridge_import`]
/// already stages into a fresh document, so malformed bytes touch nothing — but
/// a *well-formed* file carries the duration of the track it was exported from,
/// and adopting that would silently re-length this track's document. So the
/// staged cues go through [`LyricsDocument::normalize_duration`] onto the real
/// length, which clamps a cue crossing the tail and refuses one that begins at
/// or after the end. That refusal is deliberate: clamping those produces
/// zero-length cues rather than shorter ones, which is a different edit from the
/// one the user asked for.
///
/// Nothing is written to the caller's document; the accepted result comes back
/// for the caller to publish through [`LyricsDocument::replace`] once it has put
/// the old one on the undo stack.
pub fn import_bridge_document(
    bytes: &[u8],
    duration_seconds: f64,
) -> Result<LyricsDocument, BridgeImportRefusal> {
    let mut staged =
        LyricsDocument::new(duration_seconds).map_err(BridgeImportRefusal::NoTrackLength)?;
    staged
        .bridge_import(bytes)
        .map_err(BridgeImportRefusal::Format)?;
    let source_duration_seconds = staged.duration_seconds();
    let mut normalized =
        LyricsDocument::new(duration_seconds).map_err(BridgeImportRefusal::NoTrackLength)?;
    normalized
        .normalize_duration(&staged, duration_seconds)
        .map_err(|failure| BridgeImportRefusal::DoesNotFit {
            failure,
            source_duration_seconds,
        })?;
    Ok(normalized)
}

// ---------------------------------------------------------------------------
// Undo (UX0-B03)
// ---------------------------------------------------------------------------

/// How many undo steps one track's editor keeps.
///
/// Every entry is a whole document, so the ceiling is real memory: a full 1024
/// cues of 511 bytes is about 600 KiB, and 64 of those is under 40 MiB in the
/// worst case a `.musi` file can express — with a realistic 60-line lyric it is
/// a few hundred kilobytes. That is the price of the design decision below, and
/// it is worth stating rather than discovering.
pub const LYRIC_HISTORY_DEPTH: usize = 64;

/// A bounded undo/redo stack over whole lyric documents (UX0-B03).
///
/// ## Why snapshots rather than inverse edits
///
/// The obvious design is an inverse-edit log — store `Retime { id, old_span }`
/// beside every `Retime` and replay it backwards. It is rejected here, and the
/// reason is worth writing down because it will look like an easy simplification
/// to a later session.
///
/// An inverse has to reproduce **everything** the forward edit touched, and the
/// operations in this model touch more than they name. `split` allocates an id
/// from `next_id` and promotes both halves' [`CueOrigin`]; `merge` destroys a
/// cue, so its inverse has to resurrect an id that `insert` would not hand back;
/// `update` and `retime` promote origin to [`CueOrigin::UserApplied`], so their
/// inverse must restore a provenance the edit deliberately overwrote — and
/// provenance is load-bearing here rather than decorative, since `at_time`
/// refuses to display a `Potential` cue (LX1). An inverse log that gets any one
/// of those wrong produces a document that is *valid* and quietly different from
/// the one the user had, which is the failure mode no test written from the
/// forward direction catches.
///
/// A snapshot cannot be wrong about any of them. It restores cues, `next_id` and
/// the duration exactly, and the round-trip test below is an equality rather
/// than a list of properties someone had to think of.
///
/// `revision` is the one field that deliberately does **not** round-trip:
/// [`LyricsDocument::replace`] advances it, because it is a monotone change
/// counter that everything downstream uses to notice the document moved. An undo
/// *is* a change, and a caller that cached a frame against revision 7 must not
/// be told it is still looking at revision 7.
#[derive(Clone, Debug, Default)]
pub struct LyricHistory {
    /// States to go back to, oldest first. Each carries the name of the edit
    /// that left it behind.
    undo: Vec<(String, LyricsDocument)>,
    redo: Vec<(String, LyricsDocument)>,
}

impl LyricHistory {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers `before` as the state one `label` edit takes you back to.
    ///
    /// Called with the document as it stood *before* the edit lands, once per
    /// user action rather than once per model operation — a cut of five cues is
    /// five `delete`s and one undo step, because five presses of Ctrl+Z to
    /// reverse one press of Ctrl+X is not undo, it is arithmetic.
    pub fn record(&mut self, label: impl Into<String>, before: &LyricsDocument) {
        // A new edit invalidates the branch that was ahead of it. Keeping the
        // redo stack would let one press of Ctrl+Y jump the document to a state
        // that never followed from what is on screen.
        self.redo.clear();
        self.undo.push((label.into(), before.clone()));
        if self.undo.len() > LYRIC_HISTORY_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Forgets everything. Called when the editor binds to another track: cue
    /// ids restart at 1 in every document, so one track's history restores
    /// another's cues (the defect `LyricEditor::owner_slot` exists for, one
    /// layer up).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// What the next Ctrl+Z will take back, for a control's label or tooltip.
    #[must_use]
    pub fn next_undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(label, _)| label.as_str())
    }

    #[must_use]
    pub fn next_redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(label, _)| label.as_str())
    }

    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Steps `document` back one edit, returning that edit's label.
    ///
    /// `None` means there was nothing to undo. An `Err` means the stored state
    /// failed validation on the way back in, which cannot happen for a state
    /// this document was ever in — it is reported rather than unwrapped because
    /// silently doing nothing is exactly the "control that lies" failure this
    /// repository keeps finding.
    pub fn undo(
        &mut self,
        document: &mut LyricsDocument,
    ) -> Option<Result<String, LyricsValidation>> {
        let (label, previous) = self.undo.pop()?;
        let current = document.clone();
        Some(match document.replace(&previous) {
            Ok(()) => {
                self.redo.push((label.clone(), current));
                Ok(label)
            }
            Err(failure) => {
                // Put it back, so a refused undo costs the user nothing.
                self.undo.push((label, previous));
                Err(failure)
            }
        })
    }

    /// Steps `document` forward again, returning the label of the edit redone.
    pub fn redo(
        &mut self,
        document: &mut LyricsDocument,
    ) -> Option<Result<String, LyricsValidation>> {
        let (label, next) = self.redo.pop()?;
        let current = document.clone();
        Some(match document.replace(&next) {
            Ok(()) => {
                self.undo.push((label.clone(), current));
                Ok(label)
            }
            Err(failure) => {
                self.redo.push((label, next));
                Err(failure)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> LyricsDocument {
        LyricsDocument::new(60.0).unwrap()
    }

    fn cue(id: u64, start: f64, end: f64, text: &str) -> LyricCue {
        LyricCue {
            id,
            start_seconds: start,
            end_seconds: end,
            text: text.to_owned(),
            origin: CueOrigin::default(),
        }
    }

    /// LX1. A parked proposal must be invisible to everything downstream of the
    /// editor, and must not be *made* visible by the thing that hides it either.
    #[test]
    fn a_potential_cue_never_reaches_a_frame_and_never_shadows_one() {
        let mut covered = document();
        let real = covered
            .insert(cue(0, 10.0, 20.0, "the line that is placed"))
            .unwrap();
        let proposal = covered
            .insert(LyricCue {
                origin: CueOrigin::Potential,
                ..cue(0, 12.0, 18.0, "the line nobody could pin")
            })
            .unwrap();

        // The proposal sits *inside* the real cue and later in canonical order,
        // so under the plain `at_time` rule — last one active wins — it would
        // replace the real line for six seconds of the song, in the preview and
        // in an export alike. That is the failure this filter exists for.
        for time in [12.0, 15.0, 17.999] {
            let shown = covered.at_time(time).expect("the real cue still shows");
            assert_eq!(
                shown.id, real,
                "a proposal displaced a placed line at {time}"
            );
        }
        // And outside the real cue's span a proposal shows nothing at all,
        // rather than showing itself.
        covered.delete(real).unwrap();
        assert_eq!(covered.at_time(15.0), None);

        // Neither direction of the shadow report mentions it: a proposal is not
        // hidden content, and a placed line is not hidden *by* a guess.
        let mut pair = document();
        pair.insert(cue(0, 10.0, 20.0, "placed")).unwrap();
        pair.insert(LyricCue {
            origin: CueOrigin::Potential,
            ..cue(0, 10.0, 20.0, "proposed")
        })
        .unwrap();
        assert!(
            pair.shadowed_cues().is_empty(),
            "a proposal exactly covering a cue must not be reported as hiding it"
        );
        let _ = proposal;
    }

    /// LX1. The colour code is only worth reading if amber reliably means
    /// "nobody has looked at this yet", so every editing operation has to clear
    /// it. A gap here is silent: the lane would keep drawing a reviewed cue as
    /// unreviewed forever.
    #[test]
    fn every_editing_operation_promotes_a_cue_to_user_applied() {
        let inferred = |start: f64, end: f64, text: &str| LyricCue {
            origin: CueOrigin::InferredAmbiguous,
            ..cue(0, start, end, text)
        };

        // update
        let mut updated = document();
        let id = updated.insert(inferred(1.0, 2.0, "one")).unwrap();
        updated.update(id, 1.5, 2.5, "one").unwrap();
        assert_eq!(updated.find(id).unwrap().origin, CueOrigin::UserApplied);

        // shift_many, and its refusal to promote a drag that moved nothing
        let mut shifted = document();
        let id = shifted.insert(inferred(1.0, 2.0, "one")).unwrap();
        shifted.shift_many(&[id], 0.0).unwrap();
        assert_eq!(
            shifted.find(id).unwrap().origin,
            CueOrigin::InferredAmbiguous,
            "a zero-delta shift moved nothing and must not claim a review"
        );
        shifted.shift_many(&[id], 3.0).unwrap();
        assert_eq!(shifted.find(id).unwrap().origin, CueOrigin::UserApplied);

        // split: both halves
        let mut halves = document();
        let id = halves.insert(inferred(1.0, 5.0, "one two")).unwrap();
        let right = halves.split(id, 3.0, "one", "two").unwrap();
        assert_eq!(halves.find(id).unwrap().origin, CueOrigin::UserApplied);
        assert_eq!(halves.find(right).unwrap().origin, CueOrigin::UserApplied);

        // merge: the survivor
        let mut joined = document();
        let first = joined.insert(inferred(1.0, 2.0, "one")).unwrap();
        let second = joined.insert(inferred(2.0, 3.0, "two")).unwrap();
        joined.merge(first, second, " ").unwrap();
        assert_eq!(joined.find(first).unwrap().origin, CueOrigin::UserApplied);
    }

    /// Review 1.14. Each case is checked against `at_time` itself rather than
    /// against a hand-read expectation, because the two agreeing is the only
    /// thing that makes the warning true.
    #[test]
    fn a_cue_a_later_one_talks_over_is_reported_as_shadowed() {
        let mut document = document();
        // 1 is buried whole, 2 loses its tail, 3 is clear.
        document.insert(cue(0, 10.0, 12.0, "buried")).unwrap();
        document.insert(cue(0, 10.0, 14.0, "on top")).unwrap();
        document.insert(cue(0, 20.0, 24.0, "cut short")).unwrap();
        document.insert(cue(0, 22.0, 26.0, "cuts in")).unwrap();
        document.insert(cue(0, 40.0, 42.0, "clear")).unwrap();

        let shadowed = document.shadowed_cues();
        assert_eq!(shadowed.len(), 2);

        // Identical starts: canonical order breaks the tie by end, then id, and
        // the loser never displays at all.
        let buried = &shadowed[0];
        assert_eq!(buried.id, 1);
        assert!(buried.fully);
        assert_eq!(buried.shadowed_by_id, 2);
        assert_eq!(buried.hidden, vec![(10.0, 12.0)]);
        for time in [10.0, 11.0, 11.999] {
            assert_ne!(document.at_time(time).unwrap().id, 1);
        }

        let cut = &shadowed[1];
        assert_eq!(cut.id, 3);
        assert!(!cut.fully);
        assert_eq!(cut.shadowed_by_id, 4);
        assert_eq!(cut.hidden, vec![(22.0, 24.0)]);
        assert!((cut.from_seconds() - 22.0).abs() < 1e-12);
        assert_eq!(document.at_time(21.0).unwrap().id, 3);
        assert_eq!(document.at_time(23.0).unwrap().id, 4);

        // The clear cue and the two that do the shadowing report nothing.
        for index in [1, 3, 4] {
            assert_eq!(document.cue_shadow(index), None);
        }
        assert_eq!(document.cue_shadow(99), None);
    }

    #[test]
    fn a_cue_that_reappears_between_two_overlaps_is_not_fully_shadowed() {
        let mut document = document();
        document.insert(cue(0, 10.0, 20.0, "long line")).unwrap();
        document.insert(cue(0, 11.0, 13.0, "first cut")).unwrap();
        document.insert(cue(0, 15.0, 17.0, "second cut")).unwrap();

        let shadow = document.cue_shadow(0).expect("the long line loses time");
        assert!(!shadow.fully);
        // Two separate holes, not one span from the first to the last.
        assert_eq!(shadow.hidden, vec![(11.0, 13.0), (15.0, 17.0)]);
        assert_eq!(document.at_time(14.0).unwrap().id, 1);

        // Abutting overlaps merge into one hole rather than two, which is what
        // keeps "fully" a question about a single span.
        let mut abutting = LyricsDocument::new(60.0).unwrap();
        abutting.insert(cue(0, 10.0, 20.0, "long line")).unwrap();
        abutting.insert(cue(0, 12.0, 16.0, "first cut")).unwrap();
        abutting.insert(cue(0, 16.0, 20.0, "second cut")).unwrap();
        let shadow = abutting
            .cue_shadow(0)
            .expect("the long line loses its tail");
        assert_eq!(shadow.hidden, vec![(12.0, 20.0)]);
        assert!(!shadow.fully);
        assert_eq!(abutting.at_time(11.0).unwrap().id, 1);
        assert_eq!(abutting.at_time(13.0).unwrap().id, 2);
        assert_eq!(abutting.at_time(17.0).unwrap().id, 3);
    }

    #[test]
    fn cues_that_only_touch_are_not_shadowed() {
        let mut document = document();
        // `at_time` is right-open, so a cue ending exactly where the next starts
        // still displays for its whole span. Warning about this would train the
        // user to ignore the warning.
        document.insert(cue(0, 10.0, 12.0, "first")).unwrap();
        document.insert(cue(0, 12.0, 14.0, "second")).unwrap();
        assert!(document.shadowed_cues().is_empty());
        assert_eq!(document.at_time(11.999).unwrap().id, 1);
        assert_eq!(document.at_time(12.0).unwrap().id, 2);
    }

    #[test]
    fn a_document_needs_a_finite_positive_duration() {
        assert_eq!(LyricsDocument::new(0.0).unwrap_err(), LyricsError::Duration);
        assert_eq!(
            LyricsDocument::new(-1.0).unwrap_err(),
            LyricsError::Duration
        );
        assert_eq!(
            LyricsDocument::new(f64::NAN).unwrap_err(),
            LyricsError::Duration
        );
        assert_eq!(
            LyricsDocument::new(f64::INFINITY).unwrap_err(),
            LyricsError::Duration
        );
        let document = document();
        assert_eq!(document.next_id(), 1);
        assert_eq!(document.revision(), 1);
    }

    #[test]
    fn cue_bounds_are_checked_field_by_field() {
        let mut document = document();
        let cases = [
            (cue(1, -0.1, 1.0, "a"), LyricsError::InvalidCue),
            (cue(1, 1.0, 1.0, "a"), LyricsError::InvalidCue),
            (cue(1, 2.0, 1.0, "a"), LyricsError::InvalidCue),
            (cue(1, 0.0, 60.001, "a"), LyricsError::InvalidCue),
            (cue(1, f64::NAN, 1.0, "a"), LyricsError::InvalidCue),
            (cue(1, 0.0, f64::INFINITY, "a"), LyricsError::InvalidCue),
            (cue(1, 0.0, 1.0, ""), LyricsError::InvalidCue),
            (cue(1, 0.0, 1.0, "line\nbreak"), LyricsError::InvalidCue),
            (
                cue(1, 0.0, 1.0, &"x".repeat(TEXT_MAX_BYTES + 1)),
                LyricsError::TextTooLong,
            ),
        ];
        for (candidate, expected) in cases {
            assert_eq!(document.insert(candidate.clone()).unwrap_err(), expected);
        }
        assert!(document.is_empty());
        // A tab is the one control character stored text may contain.
        assert!(document.insert(cue(0, 0.0, 1.0, "with\ttab")).is_ok());
        // Exactly at the byte ceiling is accepted.
        assert!(document
            .insert(cue(0, 1.0, 2.0, &"x".repeat(TEXT_MAX_BYTES)))
            .is_ok());
    }

    #[test]
    fn text_length_counts_utf8_bytes_not_characters() {
        // 170 three-byte characters is 510 bytes and fits; 171 is 513 and does not,
        // even though both are far below 511 *characters*.
        let fits = "→".repeat(170);
        let over = "→".repeat(171);
        assert_eq!(fits.len(), 510);
        assert_eq!(over.len(), 513);
        assert!(validate_text(&fits).is_ok());
        assert_eq!(
            validate_text(&over).unwrap_err(),
            LyricsError::TextTooLong,
            "a chars().count() check would have accepted this"
        );
    }

    #[test]
    fn insert_allocates_ids_and_keeps_canonical_order() {
        let mut document = document();
        assert_eq!(document.insert(cue(0, 4.0, 5.0, "second")).unwrap(), 1);
        assert_eq!(document.insert(cue(0, 1.0, 2.0, "first")).unwrap(), 2);
        assert_eq!(document.next_id(), 3);
        let texts: Vec<&str> = document
            .cues()
            .iter()
            .map(|cue| cue.text.as_str())
            .collect();
        assert_eq!(texts, vec!["first", "second"]);
        assert!(document.validate().is_ok());
    }

    #[test]
    fn an_explicit_id_advances_next_id_past_itself() {
        let mut document = document();
        document.insert(cue(500, 1.0, 2.0, "a")).unwrap();
        assert_eq!(document.next_id(), 501);
        assert_eq!(
            document.insert(cue(500, 3.0, 4.0, "b")).unwrap_err(),
            LyricsError::DuplicateId
        );
        assert_eq!(document.insert(cue(0, 3.0, 4.0, "b")).unwrap(), 501);
    }

    #[test]
    fn overlapping_cues_are_legal_in_this_lane() {
        // The opposite of parameter cues, which may not overlap
        // (`project-v1.schema.json:394`).
        let mut document = document();
        document.insert(cue(0, 1.0, 5.0, "a")).unwrap();
        document.insert(cue(0, 2.0, 6.0, "b")).unwrap();
        assert!(document.validate().is_ok());
        assert_eq!(document.at_time(3.0).unwrap().text, "b");
        assert_eq!(document.at_time(5.5).unwrap().text, "b");
        assert_eq!(document.at_time(0.5), None);
        assert_eq!(document.at_time(6.0), None);
    }

    #[test]
    fn capacity_is_refused_rather_than_grown() {
        let mut document = LyricsDocument::new(2000.0).unwrap();
        for index in 0..CUE_CAPACITY {
            document
                .insert(cue(0, index as f64, index as f64 + 0.5, "x"))
                .unwrap();
        }
        assert_eq!(
            document.insert(cue(0, 1500.0, 1500.5, "x")).unwrap_err(),
            LyricsError::Capacity
        );
    }

    #[test]
    fn shift_many_is_all_or_nothing() {
        let mut document = document();
        let first = document.insert(cue(0, 1.0, 2.0, "a")).unwrap();
        let second = document.insert(cue(0, 3.0, 4.0, "b")).unwrap();
        let revision = document.revision();

        // The second cue would pass but the first would go negative.
        assert_eq!(
            document.shift_many(&[first, second], -2.0).unwrap_err(),
            LyricsError::InvalidCue
        );
        assert_eq!(document.cues()[0].start_seconds, 1.0);
        assert_eq!(document.cues()[1].start_seconds, 3.0);
        assert_eq!(document.revision(), revision);

        document.shift_many(&[first, second], 2.0).unwrap();
        assert_eq!(document.cues()[0].start_seconds, 3.0);
        assert_eq!(document.cues()[1].start_seconds, 5.0);
        assert_eq!(document.revision(), revision + 1);
    }

    #[test]
    fn shift_many_collapses_repeats_and_rejects_stale_ids() {
        let mut document = document();
        let id = document.insert(cue(0, 1.0, 2.0, "a")).unwrap();
        document.shift_many(&[id, id, id], 1.0).unwrap();
        assert_eq!(document.cues()[0].start_seconds, 2.0);
        assert_eq!(
            document.shift_many(&[id, 999], 1.0).unwrap_err(),
            LyricsError::NotFound
        );
        assert_eq!(
            document.shift_many(&[], 1.0).unwrap_err(),
            LyricsError::NotFound
        );
        assert_eq!(
            document.shift_many(&[id], f64::NAN).unwrap_err(),
            LyricsError::InvalidCue
        );
    }

    #[test]
    fn shift_headroom_matches_what_shift_many_accepts() {
        let mut document = document();
        let first = document.insert(cue(0, 5.0, 6.0, "a")).unwrap();
        let second = document.insert(cue(0, 50.0, 55.0, "b")).unwrap();
        let (backward, forward) = document.shift_headroom(&[first, second]).unwrap();
        assert_eq!(backward, 5.0);
        assert_eq!(forward, 5.0);

        let mut at_limit = document.clone();
        assert!(at_limit.shift_many(&[first, second], forward).is_ok());
        let mut past_limit = document.clone();
        assert!(past_limit
            .shift_many(&[first, second], forward + 0.001)
            .is_err());
        let mut back_at_limit = document.clone();
        assert!(back_at_limit
            .shift_many(&[first, second], -backward)
            .is_ok());
    }

    #[test]
    fn split_keeps_the_left_id_and_allocates_the_right() {
        let mut document = document();
        let id = document.insert(cue(0, 1.0, 5.0, "left right")).unwrap();
        let right = document.split(id, 3.0, "left", "right").unwrap();
        assert_eq!(document.len(), 2);
        assert_eq!(document.cues()[0].id, id);
        assert_eq!(document.cues()[0].end_seconds, 3.0);
        assert_eq!(document.cues()[1].id, right);
        assert_eq!(document.cues()[1].start_seconds, 3.0);
        assert_eq!(document.cues()[1].end_seconds, 5.0);
    }

    #[test]
    fn split_refuses_boundaries_outside_the_cue_and_restores_on_failure() {
        let mut document = document();
        let id = document.insert(cue(0, 1.0, 5.0, "text")).unwrap();
        let before = document.clone();
        for split in [1.0, 5.0, 0.5, 9.0, f64::NAN] {
            assert_eq!(
                document.split(id, split, "a", "b").unwrap_err(),
                LyricsError::InvalidCue
            );
            assert_eq!(document, before);
        }
        // An invalid right-hand text must also leave the document intact.
        assert!(document.split(id, 3.0, "a", "").is_err());
        assert_eq!(document, before);
    }

    #[test]
    fn merge_requires_adjacency_in_canonical_order() {
        let mut document = document();
        let first = document.insert(cue(0, 1.0, 2.0, "one")).unwrap();
        let second = document.insert(cue(0, 2.0, 3.0, "two")).unwrap();
        let third = document.insert(cue(0, 3.0, 4.0, "three")).unwrap();
        assert_eq!(
            document.merge(first, third, " ").unwrap_err(),
            LyricsError::NotAdjacent
        );
        assert_eq!(
            document.merge(second, first, " ").unwrap_err(),
            LyricsError::NotAdjacent
        );
        document.merge(first, second, " ").unwrap();
        assert_eq!(document.len(), 2);
        assert_eq!(document.cues()[0].text, "one two");
        assert_eq!(document.cues()[0].end_seconds, 3.0);
        assert_eq!(document.cues()[0].id, first);
    }

    #[test]
    fn merge_refuses_to_exceed_the_text_ceiling() {
        let mut document = document();
        let first = document.insert(cue(0, 1.0, 2.0, &"a".repeat(300))).unwrap();
        let second = document.insert(cue(0, 2.0, 3.0, &"b".repeat(300))).unwrap();
        assert_eq!(
            document.merge(first, second, "").unwrap_err(),
            LyricsError::TextTooLong
        );
        assert_eq!(document.len(), 2);
    }

    #[test]
    fn text_append_flattens_breaks_and_rejects_other_controls() {
        let mut text = String::from("hello");
        assert!(!text_append(&mut text, " world").unwrap());
        assert_eq!(text, "hello world");
        assert!(text_append(&mut text, "\nagain").unwrap());
        assert_eq!(text, "hello world again");

        let before = text.clone();
        assert_eq!(
            text_append(&mut text, "bell\u{7}").unwrap_err(),
            LyricsError::InvalidCue
        );
        assert_eq!(
            text_append(&mut text, "\u{7f}").unwrap_err(),
            LyricsError::InvalidCue
        );
        assert_eq!(
            text_append(&mut text, "").unwrap_err(),
            LyricsError::InvalidCue
        );
        assert_eq!(text, before, "a rejected paste must leave the draft alone");
    }

    #[test]
    fn text_append_refuses_to_truncate_to_fit() {
        let mut text = "a".repeat(TEXT_MAX_BYTES - 2);
        let before = text.clone();
        assert_eq!(
            text_append(&mut text, "abc").unwrap_err(),
            LyricsError::TextTooLong
        );
        assert_eq!(text, before);
        assert!(text_append(&mut text, "ab").is_ok());
        assert_eq!(text.len(), TEXT_MAX_BYTES);
    }

    #[test]
    fn normalize_duration_clamps_the_tail_and_rejects_orphans() {
        let mut source = LyricsDocument::new(60.0).unwrap();
        source.insert(cue(0, 10.0, 20.0, "kept")).unwrap();
        source.insert(cue(0, 55.0, 59.0, "clamped")).unwrap();

        let mut destination = LyricsDocument::new(1.0).unwrap();
        destination.normalize_duration(&source, 57.0).unwrap();
        assert_eq!(destination.duration_seconds(), 57.0);
        assert_eq!(destination.cues()[1].end_seconds, 57.0);

        // A cue starting at or after the new end cannot be clamped, only refused.
        let mut destination = LyricsDocument::new(1.0).unwrap();
        assert_eq!(
            destination
                .normalize_duration(&source, 54.0)
                .unwrap_err()
                .error,
            LyricsError::Duration
        );
        assert!(destination.is_empty());
    }

    #[test]
    fn replace_advances_the_revision_and_validates_first() {
        let mut destination = document();
        destination.insert(cue(0, 1.0, 2.0, "old")).unwrap();
        let revision = destination.revision();

        let mut source = document();
        source.insert(cue(0, 5.0, 6.0, "new")).unwrap();
        destination.replace(&source).unwrap();
        assert_eq!(destination.revision(), revision + 1);
        assert_eq!(destination.cues()[0].text, "new");
    }

    #[test]
    fn the_bridge_round_trips_through_export_and_import() {
        let mut source = document();
        source.insert(cue(0, 1.0, 2.5, "first line")).unwrap();
        source.insert(cue(0, 3.0, 4.0, "héllo — wörld")).unwrap();
        let exported = source.bridge_export().unwrap();
        assert!(exported.starts_with("MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n"));

        let mut destination = LyricsDocument::new(1.0).unwrap();
        destination.bridge_import(exported.as_bytes()).unwrap();
        assert_eq!(destination.len(), 2);
        assert_eq!(destination.cues()[0].text, "first line");
        assert_eq!(destination.cues()[1].text, "héllo — wörld");
        assert_eq!(destination.duration_seconds(), 60.0);
    }

    #[test]
    fn malformed_bridges_leave_the_destination_untouched() {
        let mut original = document();
        original.insert(cue(0, 1.0, 2.0, "keep me")).unwrap();

        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"nope\n".to_vec(),
            b"MUSIALIZER-LYRICS-BRIDGE\t2\t60000\n".to_vec(),
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t0\n".to_vec(),
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n0\t1000\t2000\naGk=\n".to_vec(),
            // Missing newline on the record line.
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\taGk=".to_vec(),
            // Not base64.
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\t!!!!\n".to_vec(),
            // Empty text field.
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\t\n".to_vec(),
            // A cue past the declared duration.
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t1000\n1\t100\t9000\taGk=\n".to_vec(),
            // An embedded NUL anywhere in the document.
            b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n\0".to_vec(),
        ];
        for case in cases {
            let mut document = original.clone();
            assert!(
                document.bridge_import(&case).is_err(),
                "accepted {:?}",
                String::from_utf8_lossy(&case)
            );
            assert_eq!(document, original);
        }
    }

    #[test]
    fn base64_rejects_non_canonical_padding_bits() {
        // "aGk=" is "hi"; "aGl=" sets padding bits that a canonical encoder never
        // produces, so accepting it would give one byte string two spellings.
        assert_eq!(base64_decode(b"aGk=", 64).unwrap(), b"hi");
        assert_eq!(base64_decode(b"aGl=", 64), Err(Base64Error::Format));
        assert_eq!(base64_decode(b"aQ==", 64).unwrap(), b"i");
        assert_eq!(base64_decode(b"aR==", 64), Err(Base64Error::Format));
        assert_eq!(base64_decode(b"a===", 64), Err(Base64Error::Format));
        assert_eq!(base64_decode(b"aGk", 64), Err(Base64Error::Format));
        assert_eq!(base64_decode(b"aG==aGk=", 64), Err(Base64Error::Format));
        assert_eq!(base64_decode(b"AA==", 64), Err(Base64Error::EmbeddedNul));
        assert_eq!(base64_decode(b"aGVsbG8=", 4), Err(Base64Error::TooLarge));
    }

    #[test]
    fn base64_encoding_matches_the_c_alphabet_and_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"h"), "aA==");
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b"hi!"), "aGkh");
        for input in ["", "a", "ab", "abc", "abcd", "héllo — wörld"] {
            assert_eq!(
                base64_decode(base64_encode(input.as_bytes()).as_bytes(), 4096).unwrap(),
                input.as_bytes(),
                "round trip {input}"
            );
        }
    }

    #[test]
    fn milliseconds_round_half_up() {
        assert_eq!(seconds_to_milliseconds(0.0005), 1);
        assert_eq!(seconds_to_milliseconds(0.0004), 0);
        assert_eq!(seconds_to_milliseconds(1.2345), 1235);
    }

    /// These strings reach the notice tray and `ProjectError::Build` verbatim,
    /// so a Display that regressed to a bare variant name is a UI bug.
    #[test]
    fn errors_display_as_sentences_not_variant_names() {
        let all = [
            LyricsError::Duration,
            LyricsError::Capacity,
            LyricsError::InvalidCue,
            LyricsError::InvalidUtf8,
            LyricsError::TextTooLong,
            LyricsError::DuplicateId,
            LyricsError::IdExhausted,
            LyricsError::NotFound,
            LyricsError::Order,
            LyricsError::NotAdjacent,
            LyricsError::BridgeFormat,
        ];
        for error in all {
            let shown = error.to_string();
            assert_ne!(
                shown,
                format!("{error:?}"),
                "{error:?} shows its variant name"
            );
            assert!(
                shown.contains(' ') && shown.chars().next().unwrap().is_lowercase(),
                "{error:?} should render a sentence fragment, got {shown:?}"
            );
        }
        assert_eq!(
            LyricsError::NotAdjacent.to_string(),
            "lyric cues are not adjacent"
        );
        let validation = LyricsValidation {
            error: LyricsError::NotAdjacent,
            index: 2,
            related_index: 3,
        };
        assert_eq!(validation.to_string(), "cue 3: lyric cues are not adjacent");
        assert_eq!(
            LyricsError::Order.to_string(),
            "lyric cues are not canonically ordered"
        );
    }
}

#[cfg(test)]
mod bridge_import_tests {
    use super::*;

    fn exported(duration: f64, cues: &[(f64, f64, &str)]) -> String {
        let mut document = LyricsDocument::new(duration).unwrap();
        for (start, end, text) in cues {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: *start,
                    end_seconds: *end,
                    text: (*text).to_owned(),
                    origin: CueOrigin::UserApplied,
                })
                .unwrap();
        }
        document.bridge_export().unwrap()
    }

    #[test]
    fn a_valid_file_round_trips_including_text_no_ascii_codec_would_survive() {
        // Base64 over UTF-8 is the whole reason the bridge is not a plain TSV,
        // and a Greek line is the case UX0-A05 was about one layer up.
        let body = exported(
            60.0,
            &[
                (1.0, 2.0, "the first line"),
                (2.0, 3.5, "Ελληνικά, кириллица, and a tab-free line"),
                (10.0, 12.0, "one\"with\"quotes and a \\ backslash"),
            ],
        );
        let imported = import_bridge_document(body.as_bytes(), 60.0).expect("a valid file");
        assert_eq!(imported.len(), 3);
        assert_eq!(
            imported.cues()[1].text,
            "Ελληνικά, кириллица, and a tab-free line"
        );
        assert!((imported.cues()[2].start_seconds - 10.0).abs() < 1e-9);
        // Round-tripping again is byte-identical, which is what makes the codec
        // safe to use as an interchange format at all.
        assert_eq!(imported.bridge_export().unwrap(), body);
    }

    #[test]
    fn an_invalid_file_is_refused_and_names_the_format() {
        for (name, bytes) in [
            ("empty", b"".as_slice()),
            ("plain text", b"00:01.000\tthe first line\n".as_slice()),
            (
                "a truncated header",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\n".as_slice(),
            ),
            (
                "the wrong version",
                b"MUSIALIZER-LYRICS-BRIDGE\t2\t60000\n".as_slice(),
            ),
            (
                "a row with no newline",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\tYWJj".as_slice(),
            ),
            (
                "a row with no text",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\t\n".as_slice(),
            ),
            (
                "a zero cue id",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n0\t1000\t2000\tYWJj\n".as_slice(),
            ),
            (
                "text that is not base64",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\tnot base64!\n".as_slice(),
            ),
            (
                "an embedded NUL",
                b"MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n1\t1000\t2000\tYWJj\n\0".as_slice(),
            ),
        ] {
            let refusal = import_bridge_document(bytes, 60.0)
                .expect_err(&format!("{name} should be refused"));
            assert!(
                matches!(refusal, BridgeImportRefusal::Format(_)),
                "{name} was refused as {refusal:?} rather than a format error"
            );
            assert!(!refusal.to_string().is_empty());
        }
    }

    #[test]
    fn a_file_from_another_track_is_re_based_or_refused_but_never_silently_adopted() {
        // Adopting the *file's* duration is the defect this guards: it would
        // silently re-length the destination, and every cue past the real end
        // would then be unreachable from the timeline.
        let body = exported(60.0, &[(1.0, 2.0, "early"), (50.0, 58.0, "late")]);

        // A longer destination keeps every cue and takes its own length.
        let longer = import_bridge_document(body.as_bytes(), 120.0).expect("fits");
        assert!((longer.duration_seconds() - 120.0).abs() < 1e-9);
        assert_eq!(longer.len(), 2);

        // A destination that cuts across the last cue clamps it rather than
        // dropping it.
        let clipped = import_bridge_document(body.as_bytes(), 55.0).expect("clamps");
        assert!((clipped.cues()[1].end_seconds - 55.0).abs() < 1e-9);

        // A destination shorter than a cue's *start* refuses the whole import,
        // because clamping that produces a zero-length cue rather than a
        // shorter one — a different edit from the one asked for.
        let refusal = import_bridge_document(body.as_bytes(), 30.0).expect_err("refused");
        match refusal {
            BridgeImportRefusal::DoesNotFit {
                source_duration_seconds,
                ..
            } => assert!((source_duration_seconds - 60.0).abs() < 1e-9),
            other => panic!("expected a fit refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_track_with_no_length_refuses_before_it_reads_a_byte() {
        let body = exported(60.0, &[(1.0, 2.0, "a line")]);
        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                import_bridge_document(body.as_bytes(), duration),
                Err(BridgeImportRefusal::NoTrackLength(_))
            ));
        }
    }

    #[test]
    fn an_empty_document_exports_and_imports_as_a_header_and_nothing_else() {
        // The one shape that reads as a failure and is not: a track whose lyrics
        // were deliberately cleared.
        let body = exported(60.0, &[]);
        assert_eq!(body, "MUSIALIZER-LYRICS-BRIDGE\t1\t60000\n");
        let imported = import_bridge_document(body.as_bytes(), 60.0).expect("valid");
        assert!(imported.is_empty());
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    /// Four cues in four different provenances.
    ///
    /// The provenances are the point rather than colour: origin is the field an
    /// inverse-edit log would most plausibly forget, it is invisible in a span
    /// comparison, and `at_time` refuses to display a `Potential` cue — so
    /// losing it across an undo turns a reviewable proposal into a caption on
    /// screen (LX1).
    fn seeded() -> LyricsDocument {
        let mut document = LyricsDocument::new(60.0).unwrap();
        for (index, (start, end, origin)) in [
            (0.0, 1.0, CueOrigin::InferredCertain),
            (2.0, 3.0, CueOrigin::InferredAmbiguous),
            (4.0, 6.0, CueOrigin::Potential),
            (8.0, 9.0, CueOrigin::UserApplied),
        ]
        .into_iter()
        .enumerate()
        {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: end,
                    text: format!("line {}", index + 1),
                    origin,
                })
                .unwrap();
        }
        document
    }

    /// The comparison a round trip is graded on: the cue vector element for
    /// element — ids, spans, text and origin — plus `next_id` and the duration.
    #[track_caller]
    fn same_content(left: &LyricsDocument, right: &LyricsDocument) {
        assert_eq!(left.cues(), right.cues(), "cues differ");
        assert_eq!(left.next_id(), right.next_id(), "next_id differs");
        assert!(
            (left.duration_seconds() - right.duration_seconds()).abs() < 1e-12,
            "duration differs"
        );
    }

    /// Applies one of every edit kind the editor can produce, recording a
    /// snapshot before each, and returns the state after every step.
    fn drive(document: &mut LyricsDocument, history: &mut LyricHistory) -> Vec<LyricsDocument> {
        let mut states = vec![document.clone()];
        /// One named model operation, so the sweep below reads as a table.
        type Step = (&'static str, fn(&mut LyricsDocument));
        let steps: [Step; 8] = [
            ("Move", |d| d.shift_many(&[1, 2], 0.25).unwrap()),
            ("Resize", |d| d.retime(3, 4.5, 7.0).unwrap()),
            ("Stamp", |d| d.retime(4, 20.0, 21.5).unwrap()),
            ("Edit text", |d| {
                d.update(2, 2.25, 3.25, "rewritten").unwrap();
            }),
            ("Split", |d| {
                d.split(3, 5.5, "left half", "right half").unwrap();
            }),
            ("Merge", |d| {
                let (first, second) = (d.cues()[0].id, d.cues()[1].id);
                d.merge(first, second, " ").unwrap();
            }),
            ("Insert", |d| {
                d.insert(LyricCue {
                    id: 0,
                    start_seconds: 30.0,
                    end_seconds: 31.0,
                    text: "a new line".into(),
                    origin: CueOrigin::UserApplied,
                })
                .unwrap();
            }),
            ("Delete", |d| {
                let id = d.cues().last().unwrap().id;
                d.delete(id).unwrap();
            }),
        ];
        for (label, edit) in steps {
            history.record(label, document);
            edit(document);
            states.push(document.clone());
        }
        states
    }

    #[test]
    fn undoing_every_edit_kind_walks_the_document_back_state_for_state() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        let states = drive(&mut document, &mut history);
        assert_eq!(history.undo_depth(), states.len() - 1);

        // Every intermediate is checked, not only the destination: a stack that
        // lands on the right document by passing through wrong states in
        // between is still broken, and only a per-step comparison says so.
        for expected in states.iter().rev().skip(1) {
            let label = history
                .undo(&mut document)
                .expect("a step to undo")
                .expect("a state this document was in is always valid");
            assert!(!label.is_empty());
            same_content(&document, expected);
        }
        assert!(!history.can_undo());
        assert!(history.undo(&mut document).is_none());
        same_content(&document, &seeded());
    }

    #[test]
    fn redoing_walks_it_forward_through_exactly_the_same_states() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        let states = drive(&mut document, &mut history);
        while history.can_undo() {
            history.undo(&mut document).unwrap().unwrap();
        }
        for expected in states.iter().skip(1) {
            history
                .redo(&mut document)
                .expect("a step to redo")
                .expect("valid");
            same_content(&document, expected);
        }
        assert!(!history.can_redo());
        same_content(&document, states.last().unwrap());
    }

    #[test]
    fn the_revision_advances_on_an_undo_because_an_undo_is_a_change() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        history.record("Move", &document);
        document.shift_many(&[1], 1.0).unwrap();
        let after_edit = document.revision();
        history.undo(&mut document).unwrap().unwrap();
        assert!(
            document.revision() > after_edit,
            "revision went {after_edit} -> {} across an undo",
            document.revision()
        );
    }

    #[test]
    fn a_new_edit_after_an_undo_drops_the_branch_that_was_ahead() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        history.record("Move", &document);
        document.shift_many(&[1], 1.0).unwrap();
        history.undo(&mut document).unwrap().unwrap();
        assert!(history.can_redo());

        history.record("Delete", &document);
        document.delete(1).unwrap();
        assert!(
            !history.can_redo(),
            "redo must not survive a new edit, or one Ctrl+Y jumps the document to a state that never followed from what is on screen"
        );
    }

    #[test]
    fn the_stack_is_bounded_and_forgets_the_oldest_first() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        for step in 0..(LYRIC_HISTORY_DEPTH + 10) {
            history.record(format!("step {step}"), &document);
            let end = 9.0 + f64::from(u32::try_from(step).unwrap()) * 0.001;
            document.retime(4, 8.0, end).unwrap();
        }
        assert_eq!(history.undo_depth(), LYRIC_HISTORY_DEPTH);
        assert_eq!(
            history.next_undo_label(),
            Some(format!("step {}", LYRIC_HISTORY_DEPTH + 9).as_str())
        );
        // Walking to the floor never panics and never restores a state from
        // outside the window.
        while history.can_undo() {
            history.undo(&mut document).unwrap().unwrap();
        }
        assert_eq!(document.len(), seeded().len());
    }

    #[test]
    fn clearing_drops_both_directions_because_ids_restart_in_every_document() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        history.record("Move", &document);
        document.shift_many(&[1], 1.0).unwrap();
        history.undo(&mut document).unwrap().unwrap();
        assert!(history.can_redo() && !history.can_undo());
        history.clear();
        assert!(!history.can_redo() && !history.can_undo());
    }

    #[test]
    fn the_labels_name_what_each_direction_will_do() {
        let mut document = seeded();
        let mut history = LyricHistory::new();
        history.record("Move 2 cues", &document);
        document.shift_many(&[1, 2], 0.5).unwrap();
        assert_eq!(history.next_undo_label(), Some("Move 2 cues"));
        assert_eq!(history.next_redo_label(), None);
        assert_eq!(
            history.undo(&mut document).unwrap().unwrap(),
            "Move 2 cues".to_owned()
        );
        assert_eq!(history.next_undo_label(), None);
        assert_eq!(history.next_redo_label(), Some("Move 2 cues"));
    }
}
