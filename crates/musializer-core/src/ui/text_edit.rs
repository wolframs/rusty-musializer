//! Caret, selection and editing policy for a single-line text field.
//!
//! **Owner: Agent I.** Scaffolded by the integration owner because `ui/mod.rs`
//! is a shared file, and because the split is the one this crate exists for:
//! every *decision* — where the caret lands, what a click selects, what a
//! keystroke does to the buffer — belongs here, raylib-free and headlessly
//! tested, while `musializer_app::ui::text_input` draws it and reads raylib's
//! keyboard.
//!
//! # There is an oracle for the *rules*, and none for the caret
//!
//! The scaffold's comment said the frozen C has no text entry at all. That is
//! not quite right and the difference matters, so it is recorded here rather
//! than rediscovered: `lyric_editor_ui_text_input_update`
//! (`lyrics_editor_ui.c:177-217`) **is** text entry. What it has is an
//! append-only field — every character lands at the end, `KEY_BACKSPACE` removes
//! the last code point (`:166-175`), `KEY_ESCAPE` defocuses, `Ctrl+V` appends
//! the clipboard through `lyrics_text_append`, and the drawn caret is a rule at
//! `MeasureTextEx(whole string).x` (`:1380-1385`), which is to say it can only
//! ever be at the end. There is no way to fix a typo in the middle of a cue
//! except to delete back to it.
//!
//! So the *acceptance rules* are the oracle's and are reproduced exactly, while
//! caret movement, selection and cut/copy are invention. AGENTS.md's rule
//! applies to the second half: decide, build it, record the divergence. See the
//! `#### Agent I` note in `REWRITE_PLAN.md`.
//!
//! What is **not** negotiable is what the text is allowed to be:
//! [`crate::project::lyrics::validate_text`] and
//! [`crate::project::lyrics::TEXT_MAX_BYTES`] already define that, they are
//! checked against the C, and a field that lets a user type something the model
//! will later reject is a bug in the field. The test
//! `every_buffer_the_lyric_field_can_produce_is_one_the_model_accepts` is that
//! sentence as an assertion.
//!
//! Byte offsets, not character counts: the buffer is UTF-8 and a caret must
//! never land inside a code point. `str::is_char_boundary` is the guard, and it
//! is applied on the way *in* — [`TextEdit::set_caret`] snaps rather than
//! panicking, because an offset arriving from a pixel measurement has no reason
//! to be a boundary.

use crate::project::lyrics::TEXT_MAX_BYTES;

/// Why an edit was refused.
///
/// A refusal always leaves the buffer byte-for-byte as it was. That is
/// `lyrics_text_append`'s contract (`lyrics.c:81-130`) and it is the reason a
/// paste cannot land half of a multi-byte sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEditError {
    /// The result would be longer than [`TextRules::max_bytes`].
    TooLong,
    /// The insertion held a character the field does not accept — a control
    /// character other than the three that flatten to a space, or a non-ASCII
    /// character in an ASCII-only field.
    Rejected,
    /// There was nothing to insert once the empty insertion was rejected.
    Empty,
}

/// What a field will accept.
///
/// Deliberately data rather than a trait: there are two fields in this
/// application and both are describable by two numbers, so a trait object would
/// buy nothing and cost the ability to compare two rule sets in a test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRules {
    /// Longest buffer, in **bytes**. Never a character count: the model's limit
    /// is a byte limit, and so is the C buffer it came from.
    pub max_bytes: usize,
    /// Refuse anything outside printable ASCII.
    pub ascii_only: bool,
}

impl TextRules {
    /// A lyric cue's text (`LYRICS_TEXT_CAPACITY - 1`, `lyrics.h:14`).
    ///
    /// Tab is **not** accepted even though
    /// [`validate_text`](crate::project::lyrics::validate_text) permits one in
    /// stored text. That is a narrowing, not a widening, so the field still
    /// cannot produce anything the model refuses — and a single-line field with
    /// no tab stops has nothing to do with one anyway. A tab arriving in a paste
    /// still flattens to a space, exactly as `lyrics_text_append` does it.
    pub const LYRIC_CUE: Self = Self {
        max_bytes: TEXT_MAX_BYTES,
        ascii_only: false,
    };

    /// A font-family search box (`font_query_input`, `lyrics_editor_ui.c:599-615`).
    ///
    /// ASCII-only for the oracle's stated reason: "a family name is ASCII by the
    /// helper's own validation, so anything else could never match".
    #[must_use]
    pub const fn ascii_query(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ascii_only: true,
        }
    }

    /// One typed character, or `None` when the field refuses it.
    ///
    /// The three whitespace controls are **not** accepted here: a keystroke that
    /// produced one would be Enter or Tab, which are commands rather than text.
    /// They flatten to a space only on the paste path, where they are part of a
    /// larger fragment the user did mean to insert.
    #[must_use]
    fn accepts(self, codepoint: char) -> bool {
        let value = codepoint as u32;
        if value < 0x20 || value == 0x7F {
            return false;
        }
        !self.ascii_only || value <= 0x7E
    }
}

/// How a movement or a click treats the existing selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    /// Collapse to a caret at the new position, the plain case.
    Collapse,
    /// Keep the anchor and move only the caret — shift+arrow, shift+click, and
    /// the drag half of a click-drag.
    Extend,
}

/// Where a movement puts the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One code point. Never one byte: a caret inside a code point is the bug
    /// this whole module is written in byte offsets to avoid.
    Left,
    Right,
    /// To the start of the previous word, the way every text field does it: skip
    /// the run of separators, then the run of word characters.
    WordLeft,
    WordRight,
    Home,
    End,
}

/// A single-line editable buffer with a caret and a selection.
///
/// The selection is the ordered pair `(anchor, caret)`; both are byte offsets
/// into `text` and both are always on a character boundary. Which of the two is
/// smaller is what tells shift+arrow which end to move, so they are kept as a
/// pair rather than normalised to `(start, end)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    rules: TextRules,
    text: String,
    caret: usize,
    anchor: usize,
}

impl TextEdit {
    #[must_use]
    pub fn new(rules: TextRules) -> Self {
        Self {
            rules,
            text: String::new(),
            caret: 0,
            anchor: 0,
        }
    }

    #[must_use]
    pub fn rules(&self) -> TextRules {
        self.rules
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selection as `(start, end)`, ordered, both on character boundaries.
    /// `start == end` means there is no selection, only a caret.
    #[must_use]
    pub fn selection(&self) -> (usize, usize) {
        if self.anchor <= self.caret {
            (self.anchor, self.caret)
        } else {
            (self.caret, self.anchor)
        }
    }

    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        let (start, end) = self.selection();
        &self.text[start..end]
    }

    /// Replaces the whole buffer and parks the caret at the end.
    ///
    /// Used to bind the field to a cue, where the text came out of the model and
    /// is valid by construction. Text that would not survive
    /// [`TextRules::accepts`] is refused rather than silently cleaned: the only
    /// way to get one here is a bug upstream, and hiding it would make the field
    /// disagree with the document it is showing.
    pub fn set_text(&mut self, text: &str) -> Result<(), TextEditError> {
        if text.len() > self.rules.max_bytes {
            return Err(TextEditError::TooLong);
        }
        if text.chars().any(|codepoint| !self.rules.accepts(codepoint)) {
            return Err(TextEditError::Rejected);
        }
        self.text.clear();
        self.text.push_str(text);
        self.caret = self.text.len();
        self.anchor = self.caret;
        Ok(())
    }

    /// Empties the buffer and the selection.
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
    }

    /// Snaps `offset` to the nearest character boundary at or below it and puts
    /// the caret there.
    ///
    /// Snapping rather than asserting is deliberate: the two callers are a pixel
    /// measurement and a paste, and neither has any reason to hand over a
    /// boundary. A `&str` index that is not one panics, which would turn a
    /// mis-measured click into a crash.
    pub fn set_caret(&mut self, offset: usize, select: Select) {
        let mut offset = offset.min(self.text.len());
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        self.caret = offset;
        if select == Select::Collapse {
            self.anchor = offset;
        }
    }

    /// Drops the selection, leaving the caret where it is.
    pub fn collapse(&mut self) {
        self.anchor = self.caret;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Selects the word under `offset`, which is what a double-click means.
    ///
    /// A double-click in a run of separators selects that run, not the
    /// neighbouring word — the same rule, applied to the other class of
    /// character, and it is what makes double-clicking a gap between two words
    /// do something predictable.
    pub fn select_word_at(&mut self, offset: usize) {
        self.set_caret(offset, Select::Collapse);
        if self.text.is_empty() {
            return;
        }
        // A caret past the last character belongs to the last character's run.
        let probe = if self.caret == self.text.len() {
            self.previous_boundary(self.caret)
        } else {
            self.caret
        };
        let Some(kind) = self.text[probe..].chars().next().map(is_word_char) else {
            return;
        };
        let mut start = probe;
        while start > 0 {
            let previous = self.previous_boundary(start);
            match self.text[previous..].chars().next() {
                Some(codepoint) if is_word_char(codepoint) == kind => start = previous,
                _ => break,
            }
        }
        let mut end = probe;
        while let Some(codepoint) = self.text[end..].chars().next() {
            if is_word_char(codepoint) != kind {
                break;
            }
            end += codepoint.len_utf8();
        }
        self.anchor = start;
        self.caret = end;
    }

    /// Moves the caret. With [`Select::Extend`] the anchor stays put, which is
    /// what makes shift+arrow grow and shrink one selection instead of walking
    /// it along.
    ///
    /// One case is worth naming because every text field gets it wrong at least
    /// once: a **plain** Left or Right with a live selection collapses to the
    /// near edge of it rather than moving one code point from the caret. Typing
    /// a word, selecting it and pressing Right must land after the word, not
    /// after the word plus one.
    pub fn move_caret(&mut self, motion: Motion, select: Select) {
        if select == Select::Collapse && self.has_selection() {
            let (start, end) = self.selection();
            match motion {
                Motion::Left => {
                    self.caret = start;
                    self.anchor = start;
                    return;
                }
                Motion::Right => {
                    self.caret = end;
                    self.anchor = end;
                    return;
                }
                _ => {}
            }
        }
        let target = match motion {
            Motion::Left => self.previous_boundary(self.caret),
            Motion::Right => self.next_boundary(self.caret),
            Motion::WordLeft => self.word_start_before(self.caret),
            Motion::WordRight => self.word_end_after(self.caret),
            Motion::Home => 0,
            Motion::End => self.text.len(),
        };
        self.caret = target;
        if select == Select::Collapse {
            self.anchor = target;
        }
    }

    /// Deletes the selection, returning whether there was one.
    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        let (start, end) = self.selection();
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    /// Backspace: the selection if there is one, otherwise the code point before
    /// the caret.
    ///
    /// One code point, not one byte — the C walks back over UTF-8 continuation
    /// bytes for exactly this reason (`lyrics_editor_ui.c:166-175`), and its
    /// comment on the font query's copy says it plainly: "one press deletes one
    /// character rather than half of one".
    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.caret == 0 {
            return false;
        }
        let start = self.previous_boundary(self.caret);
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    /// Delete-forward: the selection if there is one, otherwise the code point
    /// after the caret. The oracle has no Delete key; this is invention.
    pub fn delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let end = self.next_boundary(self.caret);
        if end == self.caret {
            return false;
        }
        self.text.replace_range(self.caret..end, "");
        true
    }

    /// One typed character at the caret, replacing the selection.
    pub fn insert_char(&mut self, codepoint: char) -> Result<(), TextEditError> {
        if !self.rules.accepts(codepoint) {
            return Err(TextEditError::Rejected);
        }
        let mut encoded = [0u8; 4];
        let encoded = codepoint.encode_utf8(&mut encoded);
        // Checked against the buffer the deletion would leave, not the current
        // one, so replacing a long selection with one character always fits.
        let (start, end) = self.selection();
        if self.text.len() - (end - start) + encoded.len() > self.rules.max_bytes {
            return Err(TextEditError::TooLong);
        }
        self.delete_selection();
        self.text.insert_str(self.caret, encoded);
        self.caret += encoded.len();
        self.anchor = self.caret;
        Ok(())
    }

    /// A whole fragment at the caret, replacing the selection: the paste path.
    ///
    /// Reproduces `lyrics_text_append` (`lyrics.c:81-130`) in the two respects a
    /// user can observe, and diverges in one:
    ///
    /// - **Line breaks and tabs flatten to single spaces**, and the returned
    ///   `bool` says whether any did, so the caller can say "Pasted as one line"
    ///   the way the C does (`lyrics_editor_ui.c:197-201`).
    /// - **All or nothing.** Any other control character, or a result past
    ///   `max_bytes`, refuses the whole paste and leaves the buffer exactly as it
    ///   was. Truncating to fit is specifically not allowed — the C's comment
    ///   explains why, and it is the same reason here: a cut sequence would fail
    ///   the validation every stored cue must pass.
    /// - **The divergence:** the C appends to the end because that is where its
    ///   only caret is. Here the fragment lands at the caret and replaces the
    ///   selection, which is what a caret is for.
    pub fn insert(&mut self, addition: &str) -> Result<bool, TextEditError> {
        let mut staged = String::with_capacity(addition.len());
        let mut flattened = false;
        for codepoint in addition.chars() {
            let value = codepoint as u32;
            if matches!(codepoint, '\n' | '\r' | '\t') {
                flattened = true;
                staged.push(' ');
                continue;
            }
            // The C tests bytes; every byte of a multi-byte sequence is >= 0x80,
            // so testing the code point is the same rule with the same answers.
            if value < 0x20 || value == 0x7F {
                return Err(TextEditError::Rejected);
            }
            if self.rules.ascii_only && value > 0x7E {
                return Err(TextEditError::Rejected);
            }
            staged.push(codepoint);
        }
        if staged.is_empty() {
            return Err(TextEditError::Empty);
        }
        let (start, end) = self.selection();
        if self.text.len() - (end - start) + staged.len() > self.rules.max_bytes {
            return Err(TextEditError::TooLong);
        }
        self.delete_selection();
        self.text.insert_str(self.caret, &staged);
        self.caret += staged.len();
        self.anchor = self.caret;
        Ok(flattened)
    }

    /// The selection, removed and returned: the cut half of cut/copy/paste.
    pub fn cut(&mut self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let taken = self.selected_text().to_string();
        self.delete_selection();
        Some(taken)
    }

    /// Byte offset the caret should take for a pointer at `x`, measured from the
    /// left edge of the drawn text.
    ///
    /// `measure` is the same function the drawing uses, threaded in rather than
    /// assumed, because the only correct answer is the one that agrees with the
    /// glyphs actually on screen. The boundary chosen is the nearest one, so
    /// clicking the left half of a character puts the caret before it — which is
    /// what makes click-and-drag select the character the pointer started on.
    ///
    /// Linear in the number of characters and linear in `measure`, so quadratic
    /// overall. That is fine and is a deliberate choice over a cached advance
    /// table: the buffer is at most 511 bytes and this runs on a click, and a
    /// cache would be one more thing that can disagree with the renderer.
    #[must_use]
    pub fn offset_at_x(&self, x: f32, measure: &dyn Fn(&str) -> f32) -> usize {
        if x <= 0.0 || self.text.is_empty() {
            return 0;
        }
        let mut best = 0usize;
        let mut best_distance = x.abs();
        let mut offset = 0usize;
        while offset < self.text.len() {
            offset = self.next_boundary(offset);
            let distance = (measure(&self.text[..offset]) - x).abs();
            if distance < best_distance {
                best_distance = distance;
                best = offset;
            }
        }
        best
    }

    /// Pixels from the left edge of the drawn text to a caret at `offset`.
    #[must_use]
    pub fn x_at_offset(&self, offset: usize, measure: &dyn Fn(&str) -> f32) -> f32 {
        let mut offset = offset.min(self.text.len());
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        measure(&self.text[..offset])
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        if offset == 0 {
            return 0;
        }
        offset -= 1;
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        if offset >= self.text.len() {
            return self.text.len();
        }
        offset += 1;
        while offset < self.text.len() && !self.text.is_char_boundary(offset) {
            offset += 1;
        }
        offset
    }

    fn word_start_before(&self, offset: usize) -> usize {
        let mut at = offset.min(self.text.len());
        while at > 0 {
            let previous = self.previous_boundary(at);
            match self.text[previous..].chars().next() {
                Some(codepoint) if !is_word_char(codepoint) => at = previous,
                _ => break,
            }
        }
        while at > 0 {
            let previous = self.previous_boundary(at);
            match self.text[previous..].chars().next() {
                Some(codepoint) if is_word_char(codepoint) => at = previous,
                _ => break,
            }
        }
        at
    }

    fn word_end_after(&self, offset: usize) -> usize {
        let mut at = offset.min(self.text.len());
        while let Some(codepoint) = self.text[at..].chars().next() {
            if is_word_char(codepoint) {
                break;
            }
            at += codepoint.len_utf8();
        }
        while let Some(codepoint) = self.text[at..].chars().next() {
            if !is_word_char(codepoint) {
                break;
            }
            at += codepoint.len_utf8();
        }
        at
    }
}

/// What counts as part of a word for the word-motion keys.
///
/// Alphanumeric plus the apostrophe, because "don't" is one word in a lyric and
/// ctrl+left stopping inside it would be wrong far more often than it was right.
/// Everything else — space, punctuation, the em dash — separates.
fn is_word_char(codepoint: char) -> bool {
    codepoint.is_alphanumeric() || codepoint == '\''
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lyrics::{validate_text, LyricsError};

    fn field() -> TextEdit {
        TextEdit::new(TextRules::LYRIC_CUE)
    }

    fn typed(text: &str) -> TextEdit {
        let mut edit = field();
        for codepoint in text.chars() {
            edit.insert_char(codepoint).expect("plain text is accepted");
        }
        edit
    }

    #[test]
    fn typing_lands_at_the_caret_and_the_caret_follows() {
        let mut edit = typed("wide");
        assert_eq!(edit.text(), "wide");
        assert_eq!(edit.caret(), 4);
        edit.move_caret(Motion::Home, Select::Collapse);
        edit.insert_char('a').unwrap();
        assert_eq!(edit.text(), "awide");
        assert_eq!(edit.caret(), 1);
    }

    #[test]
    fn the_caret_never_lands_inside_a_code_point() {
        // Four two-byte characters and one four-byte one. Walking right by one
        // "character" must move by 2, 2, 2, 2 then 4 bytes, and every offset the
        // field ever reports must be a boundary.
        let mut edit = typed("äöüß🎵");
        assert_eq!(edit.text().len(), 12);
        edit.move_caret(Motion::Home, Select::Collapse);
        let mut seen = vec![edit.caret()];
        while edit.caret() < edit.text().len() {
            edit.move_caret(Motion::Right, Select::Collapse);
            seen.push(edit.caret());
        }
        assert_eq!(seen, vec![0, 2, 4, 6, 8, 12]);
        for offset in seen {
            assert!(edit.text().is_char_boundary(offset));
        }

        // And a byte offset from nowhere in particular snaps rather than panics.
        for offset in 0..=13usize {
            edit.set_caret(offset, Select::Collapse);
            assert!(edit.text().is_char_boundary(edit.caret()));
        }
    }

    #[test]
    fn backspace_removes_one_character_not_one_byte() {
        // The C's own reason for walking continuation bytes
        // (`lyrics_editor_ui.c:166-175`).
        let mut edit = typed("über");
        assert!(edit.backspace());
        assert!(edit.backspace());
        assert!(edit.backspace());
        assert_eq!(edit.text(), "ü");
        assert!(edit.backspace());
        assert_eq!(edit.text(), "");
        assert!(!edit.backspace());
    }

    #[test]
    fn a_plain_arrow_collapses_a_selection_to_its_near_edge() {
        let mut edit = typed("one two");
        edit.select_all();
        edit.move_caret(Motion::Left, Select::Collapse);
        assert_eq!(edit.caret(), 0);
        assert!(!edit.has_selection());

        edit.select_all();
        edit.move_caret(Motion::Right, Select::Collapse);
        assert_eq!(edit.caret(), 7);
        assert!(!edit.has_selection());
    }

    #[test]
    fn shift_arrow_grows_and_shrinks_one_selection() {
        let mut edit = typed("abcd");
        edit.move_caret(Motion::Home, Select::Collapse);
        edit.move_caret(Motion::Right, Select::Extend);
        edit.move_caret(Motion::Right, Select::Extend);
        assert_eq!(edit.selection(), (0, 2));
        assert_eq!(edit.selected_text(), "ab");
        // Back the other way shrinks the same selection rather than starting a
        // new one on the other side.
        edit.move_caret(Motion::Left, Select::Extend);
        assert_eq!(edit.selection(), (0, 1));
        edit.move_caret(Motion::Left, Select::Extend);
        assert!(!edit.has_selection());
    }

    #[test]
    fn word_motion_skips_separators_then_the_word() {
        let mut edit = typed("hold  on, don't go");
        edit.move_caret(Motion::Home, Select::Collapse);
        edit.move_caret(Motion::WordRight, Select::Collapse);
        assert_eq!(&edit.text()[..edit.caret()], "hold");
        edit.move_caret(Motion::WordRight, Select::Collapse);
        assert_eq!(&edit.text()[..edit.caret()], "hold  on");
        edit.move_caret(Motion::WordRight, Select::Collapse);
        // The apostrophe is part of the word, so ctrl+right does not stop inside
        // "don't".
        assert_eq!(&edit.text()[..edit.caret()], "hold  on, don't");
        edit.move_caret(Motion::WordLeft, Select::Collapse);
        assert_eq!(&edit.text()[..edit.caret()], "hold  on, ");
    }

    #[test]
    fn double_click_selects_a_word_or_the_run_of_gaps_it_landed_in() {
        let mut edit = typed("hold  on");
        edit.select_word_at(2);
        assert_eq!(edit.selected_text(), "hold");
        edit.select_word_at(5);
        assert_eq!(edit.selected_text(), "  ");
        edit.select_word_at(7);
        assert_eq!(edit.selected_text(), "on");
        // Past the end belongs to the last run rather than selecting nothing.
        edit.select_word_at(8);
        assert_eq!(edit.selected_text(), "on");
        // An empty buffer has no word, and must not panic looking for one.
        let mut empty = field();
        empty.select_word_at(4);
        assert_eq!(empty.selected_text(), "");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut edit = typed("hold on");
        edit.set_caret(0, Select::Collapse);
        edit.set_caret(4, Select::Extend);
        assert_eq!(edit.selected_text(), "hold");
        edit.insert_char('w').unwrap();
        assert_eq!(edit.text(), "w on");
        assert_eq!(edit.caret(), 1);
        assert!(!edit.has_selection());
    }

    #[test]
    fn a_paste_flattens_line_breaks_and_says_that_it_did() {
        // `lyrics_text_append`'s behaviour (`lyrics.c:81-130`) and the notice the
        // C shows for it (`lyrics_editor_ui.c:197-201`).
        let mut edit = field();
        assert_eq!(edit.insert("hold on"), Ok(false));
        edit.clear();
        assert_eq!(edit.insert("hold\non\ttight\r\n"), Ok(true));
        assert_eq!(edit.text(), "hold on tight  ");
    }

    #[test]
    fn a_paste_is_all_or_nothing() {
        let mut edit = typed("kept");
        // A control character anywhere refuses the whole fragment.
        assert_eq!(edit.insert("no\u{7}pe"), Err(TextEditError::Rejected));
        assert_eq!(edit.text(), "kept");
        assert_eq!(edit.insert(""), Err(TextEditError::Empty));
        assert_eq!(edit.text(), "kept");
        // Too long refuses rather than truncating, which is what would cut a
        // multi-byte sequence in half.
        let long = "x".repeat(TEXT_MAX_BYTES);
        assert_eq!(edit.insert(&long), Err(TextEditError::TooLong));
        assert_eq!(edit.text(), "kept");
    }

    #[test]
    fn the_limit_is_measured_against_what_the_edit_would_leave() {
        let mut edit = field();
        edit.set_text(&"x".repeat(TEXT_MAX_BYTES)).unwrap();
        assert_eq!(edit.insert_char('y'), Err(TextEditError::TooLong));
        // Replacing a selection frees its bytes first, so a full buffer can still
        // be typed over.
        edit.select_all();
        edit.insert_char('y').unwrap();
        assert_eq!(edit.text(), "y");
    }

    #[test]
    fn cut_returns_the_selection_and_removes_it() {
        let mut edit = typed("hold on");
        assert_eq!(edit.cut(), None);
        edit.set_caret(5, Select::Collapse);
        edit.move_caret(Motion::End, Select::Extend);
        assert_eq!(edit.cut().as_deref(), Some("on"));
        assert_eq!(edit.text(), "hold ");
    }

    #[test]
    fn a_click_takes_the_nearest_boundary_to_the_pointer() {
        // A stub measurer: every character is ten pixels wide, so the boundary
        // between character 2 and 3 is at x = 20 and anything past 25 belongs to
        // the next one.
        let edit = typed("abcdef");
        let measure = |text: &str| text.chars().count() as f32 * 10.0;
        assert_eq!(edit.offset_at_x(-5.0, &measure), 0);
        assert_eq!(edit.offset_at_x(0.0, &measure), 0);
        assert_eq!(edit.offset_at_x(14.0, &measure), 1);
        assert_eq!(edit.offset_at_x(16.0, &measure), 2);
        assert_eq!(edit.offset_at_x(1000.0, &measure), 6);
        assert_eq!(edit.x_at_offset(3, &measure), 30.0);
        // A non-boundary offset measures the boundary below it rather than
        // slicing a code point and panicking.
        let wide = typed("äb");
        assert_eq!(wide.x_at_offset(1, &measure), 0.0);
    }

    #[test]
    fn the_ascii_query_field_refuses_what_could_never_match() {
        // `font_query_input`'s rule (`lyrics_editor_ui.c:608`).
        let mut query = TextEdit::new(TextRules::ascii_query(64));
        query.insert_char('S').unwrap();
        assert_eq!(query.insert_char('ä'), Err(TextEditError::Rejected));
        assert_eq!(query.insert("Grötesk"), Err(TextEditError::Rejected));
        assert_eq!(query.text(), "S");
    }

    /// The one property the field exists to hold.
    ///
    /// AGENTS.md: a field that lets a user type something the model will later
    /// reject is a bug in the field. `validate_text` is checked against the C, so
    /// this is the seam between the invention above and the parity below it.
    ///
    /// The single exception is the empty buffer, which `validate_text` refuses
    /// (`LyricsError::InvalidCue`) and the field must allow — you cannot type a
    /// first character into a buffer you were not allowed to empty. Apply is what
    /// refuses it, and the panel disables Apply for it.
    #[test]
    fn every_buffer_the_lyric_field_can_produce_is_one_the_model_accepts() {
        let mut edit = field();
        // Everything a keystroke can produce, across the interesting ranges: the
        // control boundary, ASCII, Latin-1, the BMP and an astral plane.
        let candidates: Vec<char> = (0u32..0x100)
            .chain([0x2019, 0x2026, 0x4E2D, 0x1F3B5])
            .filter_map(char::from_u32)
            .collect();
        let mut accepted = 0usize;
        for codepoint in candidates {
            edit.clear();
            if edit.insert_char(codepoint).is_ok() {
                accepted += 1;
                assert_eq!(
                    validate_text(edit.text()),
                    Ok(()),
                    "the field accepted U+{:04X}, which the model refuses",
                    codepoint as u32
                );
            }
        }
        assert!(accepted > 200, "only {accepted} characters were typeable");

        // And through the paste path, which is the one that can insert several
        // characters at once and is therefore the one that can build an invalid
        // buffer out of valid pieces.
        for fragment in [
            "hold on",
            "hold\non",
            "  ",
            "ä\u{2019}b",
            &"x".repeat(TEXT_MAX_BYTES),
            &"ä".repeat(TEXT_MAX_BYTES / 2),
        ] {
            edit.clear();
            if edit.insert(fragment).is_ok() {
                assert_eq!(validate_text(edit.text()), Ok(()), "pasted {fragment:?}");
            }
        }

        // The named exception, spelled out so a later reader does not "fix" it.
        edit.clear();
        assert_eq!(validate_text(edit.text()), Err(LyricsError::InvalidCue));
    }

    #[test]
    fn set_text_refuses_what_the_field_could_not_have_produced() {
        let mut edit = field();
        assert_eq!(edit.set_text("hold on"), Ok(()));
        assert_eq!(edit.caret(), 7);
        assert_eq!(edit.set_text("bad\u{1}"), Err(TextEditError::Rejected));
        assert_eq!(
            edit.set_text(&"x".repeat(TEXT_MAX_BYTES + 1)),
            Err(TextEditError::TooLong)
        );
        // A refusal leaves the buffer alone.
        assert_eq!(edit.text(), "hold on");
    }
}
