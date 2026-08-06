//! Copy, cut and paste for a lyric-lane selection (LX1).
//!
//! Not from the oracle, which has no clipboard at all. The arithmetic is here
//! rather than in the panel for the reason every other rule in this directory
//! is: "where does a three-cue paste land when the playhead is 1.2 s from the
//! end of the track" is a question with an off-by-one in it, and a capture of
//! the lane cannot tell a correct answer from a plausible one.
//!
//! ## What a paste preserves, and what it does not
//!
//! **Relative timing, yes. Absolute timing, no. Provenance, no.**
//!
//! The offsets between copied cues are the thing worth carrying — a chorus
//! pasted onto its second occurrence should keep its internal rhythm. The
//! absolute position is by definition being replaced, and the *origin* is
//! replaced with it: the user chose where this goes, so the result is
//! [`CueOrigin::UserApplied`] however the source cue was placed. A paste that
//! kept "AI inferred (certain)" would be asserting a machine had checked a
//! placement no machine has ever seen.

use crate::project::lyrics::{CueOrigin, LyricCue, LyricsDocument, CUE_CAPACITY};

use super::lyric_lane_edit::{LYRIC_LANE_SELECTION_CAPACITY, LYRIC_MIN_CUE_SECONDS};

/// One cue on the clipboard, timed relative to the earliest one copied.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardCue {
    /// Seconds after the earliest copied cue's start. `0.0` for that cue.
    pub offset_seconds: f64,
    pub duration_seconds: f64,
    pub text: String,
}

/// A session-only clipboard. Never persisted: it holds text the user has not
/// committed anywhere, and a clipboard that outlives the process is a way to
/// paste a line from a project you have since decided against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricClipboard {
    cues: Vec<ClipboardCue>,
    span_seconds: f64,
}

/// Why a paste produced nothing.
///
/// An enum rather than an `Option` because the four cases want four different
/// sentences in the notice tray, and "nothing happened" is the message this
/// interface has a rule against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PasteRefusal {
    #[error("the clipboard is empty")]
    Empty,
    #[error("the paste would exceed the document's cue capacity")]
    Capacity,
    #[error("the copied cues are longer than the track")]
    LongerThanTrack,
    #[error("the paste target is not a valid time")]
    InvalidTarget,
}

impl LyricClipboard {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    /// Total time from the first copied start to the last copied end.
    #[must_use]
    pub fn span_seconds(&self) -> f64 {
        self.span_seconds
    }

    /// Copies the cues named by `ids`, in canonical document order.
    ///
    /// Order comes from the document rather than from `ids`, so a selection
    /// built by shift-clicking backwards pastes the same way as one built
    /// forwards. Unknown ids are skipped rather than refusing the whole copy:
    /// a selection can go stale between the keypress and the read, and losing
    /// four of five cues is better than losing the gesture.
    ///
    /// `None` when nothing usable was named, so an empty clipboard is never
    /// silently installed over a full one.
    #[must_use]
    pub fn copy(document: &LyricsDocument, ids: &[u64]) -> Option<Self> {
        if ids.is_empty() || ids.len() > LYRIC_LANE_SELECTION_CAPACITY {
            return None;
        }
        let picked: Vec<&LyricCue> = document
            .cues()
            .iter()
            .filter(|cue| ids.contains(&cue.id))
            .collect();
        let first = picked.first()?;
        let origin = first.start_seconds;
        if !origin.is_finite() {
            return None;
        }
        let mut span: f64 = 0.0;
        let cues: Vec<ClipboardCue> = picked
            .iter()
            .map(|cue| {
                let offset = cue.start_seconds - origin;
                let duration = (cue.end_seconds - cue.start_seconds).max(LYRIC_MIN_CUE_SECONDS);
                if offset + duration > span {
                    span = offset + duration;
                }
                ClipboardCue {
                    offset_seconds: offset,
                    duration_seconds: duration,
                    text: cue.text.clone(),
                }
            })
            .collect();
        if cues.is_empty() || !span.is_finite() {
            return None;
        }
        Some(Self {
            cues,
            span_seconds: span,
        })
    }

    /// The cues a paste at `at_seconds` would insert, or why it cannot.
    ///
    /// Returned rather than applied so the caller can validate and notify
    /// without this module knowing what a notice is — the same shape
    /// [`crate::ui::lyric_lane_edit`] uses for a drag.
    ///
    /// **The anchor slides back to fit.** Pasting near the end of a track is the
    /// normal case (an outro repeats), and refusing it because the last cue
    /// would overhang by 200 ms would be pedantry: the paste lands flush against
    /// the end instead. It is refused only when the copied block is longer than
    /// the whole track, where there is no honest answer.
    pub fn paste_at(
        &self,
        document: &LyricsDocument,
        at_seconds: f64,
    ) -> Result<Vec<LyricCue>, PasteRefusal> {
        if self.cues.is_empty() {
            return Err(PasteRefusal::Empty);
        }
        if !at_seconds.is_finite() || at_seconds < 0.0 {
            return Err(PasteRefusal::InvalidTarget);
        }
        if document.len() + self.cues.len() > CUE_CAPACITY {
            return Err(PasteRefusal::Capacity);
        }
        let duration = document.duration_seconds();
        if !duration.is_finite() || duration <= 0.0 {
            return Err(PasteRefusal::InvalidTarget);
        }
        if self.span_seconds > duration {
            return Err(PasteRefusal::LongerThanTrack);
        }
        let anchor = at_seconds.min(duration - self.span_seconds).max(0.0);

        Ok(self
            .cues
            .iter()
            .map(|cue| {
                let start = anchor + cue.offset_seconds;
                // Clamped rather than trusted: the anchor slide above makes the
                // last end land on `duration` exactly, and f64 addition can put
                // it a bit past. A cue one ulp over the end fails validation and
                // would lose the whole paste.
                let end = (start + cue.duration_seconds).min(duration);
                LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: end,
                    text: cue.text.clone(),
                    // See the module comment: the user chose this position.
                    origin: CueOrigin::UserApplied,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(duration: f64, spans: &[(f64, f64, &str)]) -> LyricsDocument {
        let mut document = LyricsDocument::new(duration).expect("a positive duration");
        for &(start, end, text) in spans {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: end,
                    text: text.to_owned(),
                    origin: CueOrigin::InferredCertain,
                })
                .expect("a valid fixture cue");
        }
        document
    }

    #[test]
    fn a_copy_keeps_the_gaps_between_the_cues_it_took() {
        let source = document(
            60.0,
            &[
                (10.0, 11.0, "one"),
                (12.0, 13.5, "two"),
                (20.0, 21.0, "far"),
            ],
        );
        let ids: Vec<u64> = source.cues()[..2].iter().map(|cue| cue.id).collect();
        let clipboard = LyricClipboard::copy(&source, &ids).expect("two cues copied");
        assert_eq!(clipboard.len(), 2);
        assert_eq!(clipboard.cues[0].offset_seconds, 0.0);
        assert_eq!(clipboard.cues[1].offset_seconds, 2.0);
        assert_eq!(clipboard.span_seconds(), 3.5);

        let pasted = clipboard.paste_at(&source, 40.0).expect("room at 40s");
        assert_eq!(pasted.len(), 2);
        assert_eq!(pasted[0].start_seconds, 40.0);
        assert_eq!(pasted[1].start_seconds, 42.0);
        assert_eq!(pasted[1].end_seconds, 43.5);
        assert_eq!(pasted[0].text, "one");
    }

    #[test]
    fn a_paste_is_always_user_applied_whatever_it_was_copied_from() {
        // The whole reason the colour code is worth drawing: a pasted cue is
        // placed by a human, and must never inherit a machine's confidence.
        let source = document(60.0, &[(10.0, 11.0, "one")]);
        let ids = vec![source.cues()[0].id];
        let clipboard = LyricClipboard::copy(&source, &ids).unwrap();
        let pasted = clipboard.paste_at(&source, 30.0).unwrap();
        assert_eq!(pasted[0].origin, CueOrigin::UserApplied);
        assert_eq!(source.cues()[0].origin, CueOrigin::InferredCertain);
    }

    #[test]
    fn a_copy_takes_the_documents_order_not_the_selections() {
        let source = document(60.0, &[(10.0, 11.0, "first"), (20.0, 21.0, "second")]);
        let forwards: Vec<u64> = source.cues().iter().map(|cue| cue.id).collect();
        let backwards: Vec<u64> = forwards.iter().rev().copied().collect();
        assert_eq!(
            LyricClipboard::copy(&source, &forwards),
            LyricClipboard::copy(&source, &backwards),
            "a selection built backwards must paste the same way"
        );
    }

    #[test]
    fn a_paste_near_the_end_slides_back_instead_of_being_refused() {
        // Pasting an outro onto the last bar is the normal case, and the copied
        // block overhanging by a fraction of a second is what always happens.
        let source = document(60.0, &[(10.0, 14.0, "outro")]);
        let ids = vec![source.cues()[0].id];
        let clipboard = LyricClipboard::copy(&source, &ids).unwrap();

        let pasted = clipboard.paste_at(&source, 58.0).expect("slides back");
        assert_eq!(pasted[0].start_seconds, 56.0);
        assert_eq!(pasted[0].end_seconds, 60.0);

        // Past the end entirely: still flush, still not a refusal.
        let pasted = clipboard.paste_at(&source, 1_000.0).expect("clamped");
        assert_eq!(pasted[0].end_seconds, 60.0);

        // But a block longer than the track has no honest landing place.
        let long = document(3.0, &[(0.0, 3.0, "whole track")]);
        let ids = vec![long.cues()[0].id];
        let oversized = LyricClipboard::copy(&long, &ids).unwrap();
        let short = document(2.0, &[]);
        assert_eq!(
            oversized.paste_at(&short, 0.0),
            Err(PasteRefusal::LongerThanTrack)
        );
    }

    #[test]
    fn every_pasted_cue_is_something_the_document_will_accept() {
        // The property that matters: a paste is applied cue by cue through the
        // model's validating insert, and one rejection mid-way leaves the
        // document half-pasted with no undo. So sweep the anchors.
        let source = document(30.0, &[(1.0, 2.0, "a"), (3.0, 4.0, "b"), (5.0, 5.02, "c")]);
        let ids: Vec<u64> = source.cues().iter().map(|cue| cue.id).collect();
        let clipboard = LyricClipboard::copy(&source, &ids).unwrap();
        let mut anchor = 0.0f64;
        while anchor <= 40.0 {
            let mut destination = document(30.0, &[]);
            let pasted = clipboard
                .paste_at(&destination, anchor)
                .unwrap_or_else(|error| panic!("refused at {anchor}: {error}"));
            for cue in pasted {
                destination
                    .insert(cue)
                    .unwrap_or_else(|error| panic!("invalid cue from anchor {anchor}: {error}"));
            }
            assert!(destination.validate().is_ok(), "anchor {anchor}");
            anchor += 0.13;
        }
    }

    #[test]
    fn a_paste_that_would_overflow_the_document_is_refused_before_it_starts() {
        let mut source = LyricsDocument::new(4000.0).unwrap();
        for index in 0..CUE_CAPACITY {
            source
                .insert(LyricCue {
                    id: 0,
                    start_seconds: index as f64,
                    end_seconds: index as f64 + 0.5,
                    text: "x".into(),
                    origin: CueOrigin::default(),
                })
                .unwrap();
        }
        let ids = vec![source.cues()[0].id];
        let clipboard = LyricClipboard::copy(&source, &ids).unwrap();
        assert_eq!(
            clipboard.paste_at(&source, 0.0),
            Err(PasteRefusal::Capacity)
        );
    }

    #[test]
    fn a_stale_selection_copies_what_still_exists_rather_than_nothing() {
        let source = document(60.0, &[(10.0, 11.0, "one")]);
        let live = source.cues()[0].id;
        let clipboard = LyricClipboard::copy(&source, &[live, 9_999]).expect("one survivor");
        assert_eq!(clipboard.len(), 1);
        // But a selection with nothing live in it installs no clipboard at all.
        assert_eq!(LyricClipboard::copy(&source, &[9_999]), None);
        assert_eq!(LyricClipboard::copy(&source, &[]), None);
    }

    #[test]
    fn an_empty_clipboard_and_a_broken_target_refuse_with_their_own_reasons() {
        let source = document(60.0, &[(10.0, 11.0, "one")]);
        assert_eq!(
            LyricClipboard::default().paste_at(&source, 1.0),
            Err(PasteRefusal::Empty)
        );
        let ids = vec![source.cues()[0].id];
        let clipboard = LyricClipboard::copy(&source, &ids).unwrap();
        assert_eq!(
            clipboard.paste_at(&source, f64::NAN),
            Err(PasteRefusal::InvalidTarget)
        );
        assert_eq!(
            clipboard.paste_at(&source, -1.0),
            Err(PasteRefusal::InvalidTarget)
        );
    }
}
