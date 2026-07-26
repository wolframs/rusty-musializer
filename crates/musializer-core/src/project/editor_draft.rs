//! Draft editing state distinct from canonical project values.
//!
//! **Owner: Agent B.** Port of `../musializer/src/editor_draft.c/.h`.
//!
//! One question, asked before a workspace context change: would this discard an
//! authored lyric edit? The answer has to **fail safe**, so a selection that no
//! longer resolves counts as dirty rather than clean — if the document changed
//! underneath an open editor, the one thing we must not do is decide on the user's
//! behalf that there was nothing to lose.

/// What the caller knows about one open lyric editor
/// (`Editor_Lyric_Draft_State`, `editor_draft.h:7-19`).
///
/// The canonical values are modelled as one `Option` rather than C's
/// `canonical_exists` flag plus three fields, because "exists but the text pointer
/// is null" is a state C has to check for and Rust cannot construct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricDraftState {
    /// A cue being composed, which has nothing canonical to compare against.
    pub is_new: bool,
    /// `0` means nothing is selected.
    pub selected_id: u64,
    /// The stored cue the draft was opened from, if it is still in the document.
    pub canonical: Option<LyricDraftValues>,
    pub draft: LyricDraftValues,
}

/// The three fields an edit can change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricDraftValues {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

impl LyricDraftState {
    /// `editor_lyric_draft_is_dirty` (`editor_draft.c:5-15`).
    ///
    /// Three rules, in this order and for these reasons:
    ///
    /// 1. a new cue is always dirty — there is nothing to compare it to, and
    ///    discarding it would lose everything the user typed;
    /// 2. nothing selected is never dirty — there is no edit to lose;
    /// 3. a selection whose canonical cue has vanished is dirty, because the
    ///    document changed while the editor was open and the safe assumption is
    ///    that the draft still matters.
    ///
    /// Timings are compared exactly, not with a tolerance: a one-sample nudge is a
    /// real edit, and rounding it away would silently discard it.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        if self.is_new {
            return true;
        }
        if self.selected_id == 0 {
            return false;
        }
        let Some(canonical) = &self.canonical else {
            return true;
        };
        canonical.start_seconds != self.draft.start_seconds
            || canonical.end_seconds != self.draft.end_seconds
            || canonical.text != self.draft.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(start: f64, end: f64, text: &str) -> LyricDraftValues {
        LyricDraftValues {
            start_seconds: start,
            end_seconds: end,
            text: text.to_owned(),
        }
    }

    fn clean() -> LyricDraftState {
        LyricDraftState {
            is_new: false,
            selected_id: 7,
            canonical: Some(values(1.0, 2.0, "a line")),
            draft: values(1.0, 2.0, "a line"),
        }
    }

    #[test]
    fn an_untouched_draft_is_clean() {
        assert!(!clean().is_dirty());
    }

    #[test]
    fn a_new_cue_is_always_dirty() {
        let state = LyricDraftState {
            is_new: true,
            ..LyricDraftState::default()
        };
        assert!(state.is_dirty(), "there is nothing to compare it against");
    }

    #[test]
    fn nothing_selected_is_never_dirty() {
        let state = LyricDraftState {
            selected_id: 0,
            ..clean()
        };
        assert!(!state.is_dirty());
    }

    #[test]
    fn a_vanished_canonical_cue_fails_safe() {
        let state = LyricDraftState {
            canonical: None,
            ..clean()
        };
        assert!(
            state.is_dirty(),
            "the document changed under an open editor; assume the draft matters"
        );
    }

    #[test]
    fn any_of_the_three_fields_differing_is_dirty() {
        for draft in [
            values(1.5, 2.0, "a line"),
            values(1.0, 2.5, "a line"),
            values(1.0, 2.0, "a different line"),
            values(1.0, 2.0, "a line "),
        ] {
            let state = LyricDraftState { draft, ..clean() };
            assert!(state.is_dirty());
        }
    }

    #[test]
    fn timings_are_compared_exactly() {
        let state = LyricDraftState {
            draft: values(1.0 + f64::EPSILON, 2.0, "a line"),
            ..clean()
        };
        assert!(
            state.is_dirty(),
            "a one-sample nudge is a real edit, not rounding noise"
        );
    }
}
