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

/// One timed line (`Lyric_Cue`, `lyrics.h:17-22`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricCue {
    /// `0` asks [`LyricsDocument::insert`] to allocate a stable id.
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
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

        self.cues[index] = LyricCue {
            id,
            start_seconds: original.start_seconds,
            end_seconds: split_seconds,
            text: left_text.to_owned(),
        };
        self.sort_cues();
        let right = LyricCue {
            id: 0,
            start_seconds: split_seconds,
            end_seconds: original.end_seconds,
            text: right_text.to_owned(),
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
            if time_seconds < cue.end_seconds {
                active = Some(cue);
            }
        }
        active
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
        }
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
}
