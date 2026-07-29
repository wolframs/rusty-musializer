//! Cue hit zones, selection rules, drag clamping, atomic bulk retiming.
//!
//! **Owner: Agent F.** Port of `lyric_lane_edit.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! Hit testing, selection and drag clamping for direct manipulation of lyric
//! cues in the timeline lane. The immediate-mode caller polls the mouse and
//! paints; this decides what the gesture *means* (`lyric_lane_edit.h:11-13`).
//!
//! Keeping it separate is what makes the rules checkable at all
//! (`lyric_lane_edit.h:15-18`): the lane is 22 px tall and its blocks are a few
//! pixels wide on a long track, so "did that press land on the trailing edge of
//! cue 7 or the body of cue 8" is not a question a screenshot can answer.
//!
//! Everything here is `f64`, matching the C's `double`, and every `isfinite`
//! guard is reproduced literally — they are all tested in the oracle.
//!
//! ## The cue list this module reads
//!
//! The C reads a whole `Lyrics_Document`. Here the lane declares only the slice
//! of it a gesture actually needs, as the [`LaneCues`] trait, so the lane rules
//! could be ported and tested before `project::lyrics` existed and so no lane
//! test has to build a full document. [`LaneCues::shift_headroom`] reproduces
//! `lyrics_shift_headroom` (`lyrics.c:387-414`), the only lyrics operation the
//! lane calls that is not a plain lookup.

use crate::project::lyrics::LyricsDocument;

use super::timeline_view::TimelineView;

/// How many cues one gesture can move together (`lyric_lane_edit.h:20-24`).
///
/// A drag has to hold the whole selection because the commit is a single atomic
/// operation; 64 is far past what anyone hand-picks. The refusal above this is
/// deliberate and load-bearing — see [`LyricLaneSelection::apply`].
pub const LYRIC_LANE_SELECTION_CAPACITY: usize = 64;

/// Pixels at each end of a block that grab the boundary instead of the body
/// (`lyric_lane_edit.h:26-27`).
pub const LYRIC_LANE_EDGE_GRAB_PIXELS: f64 = 5.0;

/// A block narrower than this offers no edge handles at all
/// (`lyric_lane_edit.h:29-32`).
///
/// Without the rule a 9 px block would be nothing but handles and could never be
/// moved, only resized — and at that width the two edges are not distinguishable
/// by hand anyway.
pub const LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS: f64 = 18.0;

/// Shortest cue a resize may produce (`lyric_lane_edit.h:34-37`).
///
/// The lyrics model only demands `end > start`, so without a floor a single
/// sloppy drag can collapse a cue to a sliver that is then impossible to grab
/// again.
pub const LYRIC_LANE_MIN_CUE_SECONDS: f64 = 0.02;

/// Pointer travel before a press counts as a drag rather than a click
/// (`lyric_lane_edit.h:39-42`).
///
/// Below it the gesture is a selection, so a click never nudges a cue by a
/// pixel's worth of time and silently dirties the project.
pub const LYRIC_LANE_DRAG_THRESHOLD_PIXELS: f64 = 3.0;

/// Minimum drawn block width, inlined in the C at `lyric_lane_edit.c:34`.
///
/// The hit test widens a too-short block to match what the drawing does, so a
/// cue too short to see is still a target. Private because it is a property of
/// the lane painter, not of the gesture vocabulary.
const MIN_DRAWN_BLOCK_PIXELS: f64 = 3.0;

/// One cue as the lane sees it. The lane needs three fields; the full model is
/// Agent B's (`project::lyrics`, from `lyrics.h:17-22`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneCue {
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// The cue list a lane gesture reads.
///
/// Agent B's `LyricsDocument` implements the three required methods in three
/// lines; the headless tests here implement them over a `Vec`. The provided
/// methods are the C's `find_cue_index` (`lyric_lane_edit.c:6-12`),
/// `lyrics_find` (`lyrics.c:489-493`) and `lyrics_shift_headroom`
/// (`lyrics.c:387-414`), so no implementor repeats them.
///
/// **Index order is the document's canonical cue order** — sorted by start, then
/// end, then id. Two rules depend on it: the hit test walks it backwards to
/// prefer the block painted on top, and a shift+click range is the contiguous
/// index range between the anchor and the click.
pub trait LaneCues {
    /// Number of cues in canonical order.
    fn cue_count(&self) -> usize;

    /// The cue at a canonical index, or `None` when out of range.
    fn cue_at(&self, index: usize) -> Option<LaneCue>;

    /// Track length. Bounds every cue and every resize.
    fn duration_seconds(&self) -> f64;

    /// Canonical index of `id` (`lyric_lane_edit.c:6-12`, where C returns
    /// `(size_t)-1`).
    fn index_of(&self, id: u64) -> Option<usize> {
        (0..self.cue_count()).find(|&index| self.cue_at(index).is_some_and(|cue| cue.id == id))
    }

    /// The cue with `id` (`lyrics.c:489-493`).
    fn find(&self, id: u64) -> Option<LaneCue> {
        self.index_of(id).and_then(|index| self.cue_at(index))
    }

    /// Largest delta a uniform shift of `ids` could take in each direction, as
    /// `(backward, forward)` (`lyrics.c:387-414`).
    ///
    /// Both outputs are `>= 0`; either may be 0 when the selection is already
    /// against that end of the track (`lyrics.h:103-106`).
    ///
    /// Reproduces `selection_resolve` (`lyrics.c:320-336`) in two respects that
    /// callers depend on:
    ///
    /// - an id that is **not** in the document rejects the whole request, because
    ///   a stale selection is a bug in the caller and silently moving the rest
    ///   would hide it (`lyrics.h:96-98`);
    /// - repeated ids are collapsed rather than counted twice — here that falls
    ///   out of taking a min and a max, where the C needs a bitmask because it
    ///   also has to mutate each cue exactly once.
    ///
    /// An empty `ids` is `None`: the C returns `LYRICS_ERROR_NOT_FOUND` for
    /// `id_count == 0` (`lyrics.c:329`), and the caller treats any error as "no
    /// headroom".
    fn shift_headroom(&self, ids: &[u64]) -> Option<(f64, f64)> {
        if ids.is_empty() {
            return None;
        }
        let mut earliest_start = self.duration_seconds();
        let mut latest_end = 0.0;
        for &id in ids {
            let cue = self.find(id)?;
            if cue.start_seconds < earliest_start {
                earliest_start = cue.start_seconds;
            }
            if cue.end_seconds > latest_end {
                latest_end = cue.end_seconds;
            }
        }
        let backward = if earliest_start > 0.0 {
            earliest_start
        } else {
            0.0
        };
        let forward = self.duration_seconds() - latest_end;
        Some((backward, if forward > 0.0 { forward } else { 0.0 }))
    }
}

/// The three lines the module comment promised.
///
/// Written here rather than in `project::lyrics` so the model stays unaware of
/// the interface: `ui` may depend on `project`, and the reverse would make the
/// document's own tests carry a lane's vocabulary. The provided methods —
/// `index_of`, `find`, `shift_headroom` — are deliberately **not** overridden
/// with the document's own faster versions: the point of the trait is that the
/// lane's rules were tested against the same implementations the application
/// runs, and a second `shift_headroom` is a second thing that can be wrong.
impl LaneCues for LyricsDocument {
    fn cue_count(&self) -> usize {
        self.len()
    }

    fn cue_at(&self, index: usize) -> Option<LaneCue> {
        self.cues().get(index).map(|cue| LaneCue {
            id: cue.id,
            start_seconds: cue.start_seconds,
            end_seconds: cue.end_seconds,
        })
    }

    fn duration_seconds(&self) -> f64 {
        LyricsDocument::duration_seconds(self)
    }
}

/// What part of a block a press landed on (`lyric_lane_edit.h:44-49`).
///
/// The C's `LYRIC_LANE_ZONE_NONE` is the `None` of the [`hit_test`] return, so
/// "nothing was hit" cannot be confused with "the body of cue 0".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LyricLaneZone {
    Body,
    StartEdge,
    EndEdge,
}

/// The cue under the pointer and what part of it (`lyric_lane_edit.h:51-54`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LyricLaneHit {
    pub zone: LyricLaneZone,
    pub id: u64,
}

/// How a click combines with the existing selection (`lyric_lane_edit.h:63-70`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LyricLaneClick {
    /// A plain click: the cue becomes the whole selection.
    #[default]
    Replace,
    /// Ctrl: add or remove this one cue, leaving the rest alone.
    Toggle,
    /// Shift: select every cue between the anchor and this one inclusive.
    Extend,
}

/// The cues one gesture acts on (`lyric_lane_edit.h:56-61`).
///
/// The fields are private on purpose. The C's fixed array is what enforces
/// [`LYRIC_LANE_SELECTION_CAPACITY`], and a `pub` `Vec` anyone could push into
/// would turn the capacity refusal in [`Self::apply`] into a suggestion. That
/// refusal is behaviour, not a limitation of C arrays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LyricLaneSelection {
    ids: Vec<u64>,
    /// The fixed end of a shift+click range. Cleared with the selection.
    anchor_id: u64,
}

impl LyricLaneSelection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The selected ids in the order they were added, which for a range is
    /// canonical document order. This is what a bulk shift is given.
    #[must_use]
    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The fixed end of a shift+click range, or 0 when there is none.
    #[must_use]
    pub fn anchor_id(&self) -> u64 {
        self.anchor_id
    }

    /// `lyric_lane_edit.c:54-58`. Clears the anchor with the selection.
    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor_id = 0;
    }

    /// `lyric_lane_edit.c:60-68`. Id 0 is never contained: it is the C's "no
    /// cue" sentinel, not a cue.
    #[must_use]
    pub fn contains(&self, id: u64) -> bool {
        if id == 0 {
            return false;
        }
        self.ids.contains(&id)
    }

    /// `lyric_lane_edit.c:70-76`. Already-present is success, not a duplicate.
    fn add(&mut self, id: u64) -> bool {
        if self.contains(id) {
            return true;
        }
        if self.ids.len() >= LYRIC_LANE_SELECTION_CAPACITY {
            return false;
        }
        self.ids.push(id);
        true
    }

    /// `lyric_lane_edit.c:78-88`. Order-preserving removal, as the C's `memmove`.
    fn remove(&mut self, id: u64) {
        if let Some(index) = self.ids.iter().position(|&held| held == id) {
            self.ids.remove(index);
        }
    }

    /// Applies a click (`lyric_lane_edit.c:90-145`).
    ///
    /// Returns `false` and leaves the selection **untouched** when the request
    /// does not fit [`LYRIC_LANE_SELECTION_CAPACITY`] or the id is not in the
    /// document, so a range drag across a thousand cues fails visibly instead of
    /// silently selecting the first sixty-four (`lyric_lane_edit.h:85-88`).
    /// Silently keeping the first sixty-four would look like a successful drag
    /// and then move the wrong set of cues.
    pub fn apply(&mut self, document: &impl LaneCues, id: u64, mode: LyricLaneClick) -> bool {
        if id == 0 {
            return false;
        }
        let Some(index) = document.index_of(id) else {
            return false;
        };

        match mode {
            LyricLaneClick::Replace => {
                self.clear();
                self.ids.push(id);
                self.anchor_id = id;
                true
            }
            LyricLaneClick::Toggle => {
                if self.contains(id) {
                    self.remove(id);
                    // Removing the anchor would leave a later shift+click
                    // extending from a cue that is no longer part of the
                    // selection.
                    if self.anchor_id == id {
                        self.anchor_id = self.ids.last().copied().unwrap_or(0);
                    }
                    return true;
                }
                if !self.add(id) {
                    return false;
                }
                self.anchor_id = id;
                true
            }
            LyricLaneClick::Extend => {
                let anchor = if self.anchor_id != 0 {
                    document.index_of(self.anchor_id)
                } else {
                    None
                };
                // Shift with nothing to extend from behaves as a plain click
                // rather than doing nothing, which is what every list control
                // does.
                let Some(anchor) = anchor else {
                    return self.apply(document, id, LyricLaneClick::Replace);
                };
                let low = anchor.min(index);
                let high = anchor.max(index);
                if high - low + 1 > LYRIC_LANE_SELECTION_CAPACITY {
                    return false;
                }
                let keep_anchor = self.anchor_id;
                self.clear();
                for i in low..=high {
                    if let Some(cue) = document.cue_at(i) {
                        self.ids.push(cue.id);
                    }
                }
                // The anchor stays put so dragging the shift+click back and
                // forth grows and shrinks one range instead of walking it along.
                self.anchor_id = keep_anchor;
                true
            }
        }
    }

    /// Drops ids the document no longer holds (`lyric_lane_edit.c:147-162`).
    ///
    /// Deleting a cue through the editing form leaves a stale id behind, and a
    /// bulk shift rejects the whole move when it sees one — correctly, but the
    /// user would just see dragging stop working with no explanation
    /// (`lyric_lane_edit.h:94-96`).
    pub fn prune(&mut self, document: &impl LaneCues) {
        self.ids.retain(|&id| document.index_of(id).is_some());
        if self.anchor_id != 0 && document.index_of(self.anchor_id).is_none() {
            self.anchor_id = self.ids.last().copied().unwrap_or(0);
        }
    }
}

/// The cue under the pointer, preferring the later one where blocks overlap,
/// because that is the one painted on top (`lyric_lane_edit.c:14-52`).
///
/// Cues whose boundary is off-screen never offer a handle for it
/// (`lyric_lane_edit.h:72-75`): an edge that scrolled out of view must not offer
/// a grab at the lane border for a boundary that is not there.
///
/// `None` is the C's `LYRIC_LANE_ZONE_NONE` with id 0.
#[must_use]
pub fn hit_test(
    document: &impl LaneCues,
    view: &TimelineView,
    lane_x: f64,
    lane_width: f64,
    pointer_x: f64,
) -> Option<LyricLaneHit> {
    if !lane_x.is_finite() || !lane_width.is_finite() || lane_width <= 0.0 {
        return None;
    }
    if !pointer_x.is_finite() {
        return None;
    }
    if pointer_x < lane_x || pointer_x > lane_x + lane_width {
        return None;
    }

    // Later cues are painted over earlier ones, so walking backwards makes the
    // hit test agree with what the eye picks out of an overlap.
    for step in (0..document.cue_count()).rev() {
        let Some(cue) = document.cue_at(step) else {
            continue;
        };
        let left = view.x_at(cue.start_seconds, lane_x, lane_width);
        let mut right = view.x_at(cue.end_seconds, lane_x, lane_width);
        if !left.is_finite() || !right.is_finite() {
            continue;
        }
        // Match the minimum drawn width so a cue too short to see is still a
        // target; the drawing does the same widening.
        if right - left < MIN_DRAWN_BLOCK_PIXELS {
            right = left + MIN_DRAWN_BLOCK_PIXELS;
        }
        if pointer_x < left || pointer_x > right {
            continue;
        }

        let mut zone = LyricLaneZone::Body;
        if right - left >= LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS {
            // Only offer a handle for a boundary that is actually on screen.
            let start_visible = left >= lane_x;
            let end_visible = right <= lane_x + lane_width;
            if start_visible && pointer_x <= left + LYRIC_LANE_EDGE_GRAB_PIXELS {
                zone = LyricLaneZone::StartEdge;
            } else if end_visible && pointer_x >= right - LYRIC_LANE_EDGE_GRAB_PIXELS {
                zone = LyricLaneZone::EndEdge;
            }
        }
        return Some(LyricLaneHit { zone, id: cue.id });
    }
    None
}

/// Largest part of the proposed move a bulk shift will accept
/// (`lyric_lane_edit.c:164-179`).
///
/// The blocks track the pointer up to the end of the track and then stop,
/// instead of following it and snapping back when the commit is rejected
/// (`lyric_lane_edit.h:100-102`). The returned value is exactly what the commit
/// accepts, which is the property that makes that true.
#[must_use]
pub fn clamp_move(
    document: &impl LaneCues,
    selection: &LyricLaneSelection,
    delta_seconds: f64,
) -> f64 {
    if selection.is_empty() {
        return 0.0;
    }
    if !delta_seconds.is_finite() {
        return 0.0;
    }
    let Some((backward, forward)) = document.shift_headroom(selection.ids()) else {
        return 0.0;
    };
    if delta_seconds < -backward {
        return -backward;
    }
    if delta_seconds > forward {
        return forward;
    }
    delta_seconds
}

/// Resolves an edge drag to the boundaries a retime should be given, as
/// `(start_seconds, end_seconds)` (`lyric_lane_edit.c:181-211`).
///
/// `None` when the cue is missing, the input is not finite, or the result would
/// not satisfy `end > start` — the C's `false`, on which the caller abandons the
/// drag rather than committing something the model will reject.
///
/// Note the trailing-edge branch's last guard (`lyric_lane_edit.c:205`): raising
/// `end` to the minimum cue length can push it past the end of the track for a
/// cue that starts within `LYRIC_LANE_MIN_CUE_SECONDS` of it, and the cue is then
/// left exactly as it was. The leading-edge branch's matching guard
/// (`lyric_lane_edit.c:199`) tests `start > end`, which the clamp above it has
/// already made unreachable — see the module tests, and the report note. It is
/// reproduced as written; do not "fix" it here, the oracle is the reference.
#[must_use]
// `!(end > start)` is the C's own spelling and is NaN-rejecting; `end <= start`
// would accept a NaN pair (`lyric_lane_edit.c:207`).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn clamp_resize(
    document: &impl LaneCues,
    id: u64,
    moving_start: bool,
    proposed_seconds: f64,
) -> Option<(f64, f64)> {
    if !proposed_seconds.is_finite() {
        return None;
    }
    let cue = document.find(id)?;

    let mut start = cue.start_seconds;
    let mut end = cue.end_seconds;
    if moving_start {
        let latest = end - LYRIC_LANE_MIN_CUE_SECONDS;
        start = proposed_seconds;
        if start < 0.0 {
            start = 0.0;
        }
        if start > latest {
            start = latest;
        }
        // A cue already shorter than the floor cannot be shortened further, but
        // must still be movable: leave it exactly as it was.
        if start > end {
            start = cue.start_seconds;
        }
    } else {
        let earliest = start + LYRIC_LANE_MIN_CUE_SECONDS;
        end = proposed_seconds;
        if end > document.duration_seconds() {
            end = document.duration_seconds();
        }
        if end < earliest {
            end = earliest;
        }
        if end > document.duration_seconds() {
            end = cue.end_seconds;
        }
    }
    if !(end > start) {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from `../musializer/tests/test_lyric_lane_edit.c`.
    const LANE_X: f64 = 20.0;
    const LANE_WIDTH: f64 = 1000.0;
    const TRACK: f64 = 100.0;

    /// The C's `EXPECT_NEAR`, which also fails on a non-finite actual value
    /// (`tests/test_support.h:63-69`).
    #[track_caller]
    fn expect_near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            actual.is_finite() && (actual - expected).abs() <= tolerance,
            "actual={actual} expected={expected} tolerance={tolerance}"
        );
    }

    /// A cue list standing in for `Lyrics_Document`: cues in canonical order and
    /// a duration.
    struct TestDocument(Vec<LaneCue>, f64);

    impl LaneCues for TestDocument {
        fn cue_count(&self) -> usize {
            self.0.len()
        }

        fn cue_at(&self, index: usize) -> Option<LaneCue> {
            self.0.get(index).copied()
        }

        fn duration_seconds(&self) -> f64 {
            self.1
        }
    }

    impl TestDocument {
        fn new(duration_seconds: f64) -> Self {
            Self(Vec::new(), duration_seconds)
        }

        /// `lyrics_insert` with `id == 0`, which allocates the next stable id
        /// (`lyrics.h:76-77`) and keeps the document in canonical order. Every
        /// fixture below inserts in ascending time before any deletion, so the
        /// ids match what the C fixtures get: 1, 2, 3, ...
        fn insert(&mut self, start_seconds: f64, end_seconds: f64) -> u64 {
            let id = self.0.len() as u64 + 1;
            self.0.push(LaneCue {
                id,
                start_seconds,
                end_seconds,
            });
            self.0.sort_by(|a, b| {
                (a.start_seconds, a.end_seconds, a.id)
                    .partial_cmp(&(b.start_seconds, b.end_seconds, b.id))
                    .expect("fixture cues are finite")
            });
            id
        }

        /// `lyrics_delete` (`lyrics.c:269-280`).
        fn delete(&mut self, id: u64) -> bool {
            let Some(index) = self.index_of(id) else {
                return false;
            };
            self.0.remove(index);
            true
        }

        /// The acceptance half of `lyrics_shift_many` (`lyrics.c:355-385`) with
        /// its `validate_cue` rules (`lyrics.c:71-79`), minus the text
        /// validation this module cannot see: all or nothing, an unknown id
        /// rejects the whole request, and every shifted cue must stay inside
        /// `[0, duration]` with `end > start`.
        ///
        /// The C test asserts "whatever `clamp_move` returns, the commit accepts
        /// it". Agent B owns the real commit, so the acceptance rule is
        /// reproduced here rather than the assertion being dropped.
        fn shift_many(&mut self, ids: &[u64], delta_seconds: f64) -> bool {
            if !delta_seconds.is_finite() || ids.is_empty() {
                return false;
            }
            let mut indices = Vec::new();
            for &id in ids {
                match self.index_of(id) {
                    Some(index) => indices.push(index),
                    None => return false,
                }
            }
            for &index in &indices {
                let cue = self.0[index];
                if !Self::valid(
                    cue.start_seconds + delta_seconds,
                    cue.end_seconds + delta_seconds,
                    self.1,
                ) {
                    return false;
                }
            }
            for &index in &indices {
                self.0[index].start_seconds += delta_seconds;
                self.0[index].end_seconds += delta_seconds;
            }
            true
        }

        /// The acceptance half of `lyrics_retime` (`lyrics.c:295-305`), which is
        /// `lyrics_update`'s `validate_cue`.
        fn retime(&mut self, id: u64, start_seconds: f64, end_seconds: f64) -> bool {
            let Some(index) = self.index_of(id) else {
                return false;
            };
            if !Self::valid(start_seconds, end_seconds, self.1) {
                return false;
            }
            self.0[index].start_seconds = start_seconds;
            self.0[index].end_seconds = end_seconds;
            true
        }

        /// `validate_cue`'s timing half (`lyrics.c:74-77`).
        fn valid(start_seconds: f64, end_seconds: f64, duration_seconds: f64) -> bool {
            start_seconds.is_finite()
                && end_seconds.is_finite()
                && start_seconds >= 0.0
                && end_seconds > start_seconds
                && end_seconds <= duration_seconds
        }
    }

    /// Five one-second cues at 10 s intervals in a 100 s track. Unzoomed, the
    /// lane is 10 px per second, so each block is 10 px wide and the numbers
    /// below are easy to check: cue 1 spans x = 120 to 130.
    fn build_lane() -> TestDocument {
        let mut document = TestDocument::new(TRACK);
        for i in 0..5 {
            let start = 10.0 + f64::from(i) * 10.0;
            document.insert(start, start + 1.0);
        }
        document
    }

    fn whole_view() -> TimelineView {
        TimelineView::new(TRACK)
    }

    #[test]
    fn lyric_lane_hit_test_finds_the_block_under_the_pointer() {
        let document = build_lane();
        let view = whole_view();

        let hit =
            hit_test(&document, &view, LANE_X, LANE_WIDTH, 125.0).expect("cue 1 is at 125 px");
        assert_eq!(hit.id, 1);
        assert_eq!(hit.zone, LyricLaneZone::Body);

        // Between blocks, and outside the lane entirely.
        assert_eq!(hit_test(&document, &view, LANE_X, LANE_WIDTH, 160.0), None);
        assert_eq!(hit_test(&document, &view, LANE_X, LANE_WIDTH, 5.0), None);
        assert_eq!(hit_test(&document, &view, LANE_X, LANE_WIDTH, 5000.0), None);
    }

    #[test]
    fn lyric_lane_hit_test_withholds_edge_handles_from_a_block_too_small_to_aim_at() {
        let document = build_lane();
        let view = whole_view();
        // A 10 px block: every pixel of it is within the 5 px grab of one edge or
        // the other, so offering handles would make it impossible to move.
        let mut x = 120.0;
        while x <= 130.0 {
            let hit = hit_test(&document, &view, LANE_X, LANE_WIDTH, x).expect("inside cue 1");
            assert_eq!(hit.id, 1);
            assert_eq!(hit.zone, LyricLaneZone::Body);
            x += 1.0;
        }
    }

    #[test]
    fn lyric_lane_hit_test_offers_edges_once_the_block_is_wide_enough() {
        let document = build_lane();
        let mut view = whole_view();
        // Zoom until cue 1 is comfortably wide. At 8x the span is 12.5 s, so the
        // lane is 80 px per second and the one-second block is 80 px.
        view.zoom(TRACK, 8.0, 10.5);
        let left = view.x_at(10.0, LANE_X, LANE_WIDTH);
        let right = view.x_at(11.0, LANE_X, LANE_WIDTH);
        expect_near(right - left, 80.0, 0.5);

        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, left + 2.0).map(|hit| hit.zone),
            Some(LyricLaneZone::StartEdge)
        );
        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, right - 2.0).map(|hit| hit.zone),
            Some(LyricLaneZone::EndEdge)
        );
        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, (left + right) * 0.5)
                .map(|hit| hit.zone),
            Some(LyricLaneZone::Body)
        );
    }

    #[test]
    fn lyric_lane_hit_test_offers_no_handle_for_a_boundary_that_scrolled_away() {
        let mut document = TestDocument::new(TRACK);
        document.insert(5.0, 60.0);

        let mut view = whole_view();
        view.zoom(TRACK, 10.0, 30.0);
        // The window is 10 s wide around 30 s, so both boundaries are off-screen.
        assert!(view.start_seconds > 5.0);
        assert!(view.start_seconds + view.span_seconds < 60.0);
        // The block fills the lane. Grabbing near the lane border must give the
        // body, not a handle for a boundary that is not being shown.
        let at_left = hit_test(&document, &view, LANE_X, LANE_WIDTH, LANE_X + 1.0)
            .expect("the block fills the lane");
        let at_right = hit_test(
            &document,
            &view,
            LANE_X,
            LANE_WIDTH,
            LANE_X + LANE_WIDTH - 1.0,
        )
        .expect("the block fills the lane");
        assert_eq!(at_left.id, 1);
        assert_eq!(at_left.zone, LyricLaneZone::Body);
        assert_eq!(at_right.zone, LyricLaneZone::Body);
    }

    #[test]
    fn lyric_lane_hit_test_prefers_the_block_painted_on_top() {
        let mut document = TestDocument::new(TRACK);
        document.insert(10.0, 30.0);
        document.insert(20.0, 40.0);
        let view = whole_view();
        // x for 25 s, inside both. The later cue is drawn last, so it is the one
        // the eye sees and must be the one the press selects.
        let x = view.x_at(25.0, LANE_X, LANE_WIDTH);
        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, x).map(|hit| hit.id),
            Some(2)
        );
        let only_first = view.x_at(15.0, LANE_X, LANE_WIDTH);
        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, only_first).map(|hit| hit.id),
            Some(1)
        );
    }

    #[test]
    fn lyric_lane_selection_replace_toggle_and_extend() {
        let document = build_lane();
        let mut selection = LyricLaneSelection::new();

        assert!(selection.apply(&document, 2, LyricLaneClick::Replace));
        assert_eq!(selection.len(), 1);
        assert!(selection.contains(2));

        // Ctrl adds without disturbing the rest.
        assert!(selection.apply(&document, 4, LyricLaneClick::Toggle));
        assert_eq!(selection.len(), 2);
        assert!(selection.contains(2));
        assert!(selection.contains(4));

        // Ctrl again removes exactly that one.
        assert!(selection.apply(&document, 2, LyricLaneClick::Toggle));
        assert_eq!(selection.len(), 1);
        assert!(!selection.contains(2));

        // Shift extends from the anchor, which the last ctrl+click left at cue 4.
        assert!(selection.apply(&document, 1, LyricLaneClick::Extend));
        assert_eq!(selection.len(), 4);
        for id in 1..=4 {
            assert!(selection.contains(id));
        }
        // Shifting the other way from the same anchor replaces the range rather
        // than accumulating: the anchor did not move.
        assert!(selection.apply(&document, 5, LyricLaneClick::Extend));
        assert_eq!(selection.len(), 2);
        assert!(selection.contains(4));
        assert!(selection.contains(5));

        // A plain click collapses back to one.
        assert!(selection.apply(&document, 3, LyricLaneClick::Replace));
        assert_eq!(selection.len(), 1);
    }

    #[test]
    fn lyric_lane_selection_rejects_what_it_cannot_hold_or_find() {
        let document = build_lane();
        let mut selection = LyricLaneSelection::new();

        assert!(!selection.apply(&document, 99, LyricLaneClick::Replace));
        assert_eq!(selection.len(), 0);
        assert!(!selection.apply(&document, 0, LyricLaneClick::Replace));

        // Shift with no anchor behaves as a plain click instead of doing nothing.
        assert!(selection.apply(&document, 3, LyricLaneClick::Extend));
        assert_eq!(selection.len(), 1);
        assert!(selection.contains(3));
    }

    #[test]
    fn lyric_lane_selection_range_beyond_capacity_leaves_the_selection_alone() {
        let mut document = TestDocument::new(1000.0);
        for i in 0..(LYRIC_LANE_SELECTION_CAPACITY + 4) {
            let start = i as f64 * 2.0;
            document.insert(start, start + 1.0);
        }
        let mut selection = LyricLaneSelection::new();
        assert!(selection.apply(&document, 1, LyricLaneClick::Replace));
        let last = document
            .cue_at(document.cue_count() - 1)
            .expect("the fixture has cues")
            .id;
        // Silently keeping the first sixty-four would look like a successful drag
        // and then move the wrong set of cues.
        assert!(!selection.apply(&document, last, LyricLaneClick::Extend));
        assert_eq!(selection.len(), 1);
        assert!(selection.contains(1));
    }

    #[test]
    fn lyric_lane_selection_prune_drops_deleted_cues() {
        let mut document = build_lane();
        let mut selection = LyricLaneSelection::new();
        assert!(selection.apply(&document, 2, LyricLaneClick::Replace));
        assert!(selection.apply(&document, 3, LyricLaneClick::Toggle));
        assert!(document.delete(3));

        // Without this, a bulk shift rejects the whole move on the stale id and
        // dragging simply stops working with nothing on screen to explain it.
        selection.prune(&document);
        assert_eq!(selection.len(), 1);
        assert!(selection.contains(2));
        assert_eq!(selection.anchor_id(), 2);
        let ids = selection.ids().to_vec();
        assert!(document.shift_many(&ids, 1.0));
    }

    #[test]
    fn lyric_lane_clamp_move_stops_at_the_ends_of_the_track() {
        let mut document = build_lane();
        let mut selection = LyricLaneSelection::new();
        assert!(selection.apply(&document, 1, LyricLaneClick::Replace));
        assert!(selection.apply(&document, 5, LyricLaneClick::Toggle));

        // Cue 1 starts at 10 s and cue 5 ends at 51 s in a 100 s track.
        expect_near(clamp_move(&document, &selection, -3.0), -3.0, 1e-9);
        expect_near(clamp_move(&document, &selection, -500.0), -10.0, 1e-9);
        expect_near(clamp_move(&document, &selection, 500.0), 49.0, 1e-9);
        expect_near(clamp_move(&document, &selection, f64::NAN), 0.0, 1e-9);

        // The clamped value is exactly what the commit accepts, which is the
        // property that stops blocks following the pointer and then snapping back.
        let clamped = clamp_move(&document, &selection, -500.0);
        let ids = selection.ids().to_vec();
        assert!(document.shift_many(&ids, clamped));

        let empty = LyricLaneSelection::new();
        expect_near(clamp_move(&document, &empty, 5.0), 0.0, 1e-9);
    }

    #[test]
    fn lyric_lane_clamp_resize_keeps_a_cue_grabbable() {
        let mut document = build_lane();

        // Dragging the trailing edge out is unrestricted until the track ends.
        let (start, end) = clamp_resize(&document, 1, false, 14.0).expect("cue 1 exists");
        expect_near(start, 10.0, 1e-9);
        expect_near(end, 14.0, 1e-9);
        let (_, end) = clamp_resize(&document, 1, false, 900.0).expect("cue 1 exists");
        expect_near(end, TRACK, 1e-9);

        // Dragging it back past the start stops at the minimum length rather than
        // collapsing the cue to something that can never be grabbed again.
        let (start, end) = clamp_resize(&document, 1, false, 2.0).expect("cue 1 exists");
        expect_near(end, 10.0 + LYRIC_LANE_MIN_CUE_SECONDS, 1e-9);
        assert!(end > start);

        // The leading edge, symmetrically.
        let (start, end) = clamp_resize(&document, 1, true, 7.5).expect("cue 1 exists");
        expect_near(start, 7.5, 1e-9);
        expect_near(end, 11.0, 1e-9);
        let (start, _) = clamp_resize(&document, 1, true, -40.0).expect("cue 1 exists");
        expect_near(start, 0.0, 1e-9);
        let (start, _) = clamp_resize(&document, 1, true, 50.0).expect("cue 1 exists");
        expect_near(start, 11.0 - LYRIC_LANE_MIN_CUE_SECONDS, 1e-9);

        // Whatever it produces, a retime must accept it.
        let (start, end) = clamp_resize(&document, 1, true, 50.0).expect("cue 1 exists");
        assert!(document.retime(1, start, end));

        assert_eq!(clamp_resize(&document, 99, true, 1.0), None);
        assert_eq!(clamp_resize(&document, 1, true, f64::NAN), None);
    }

    /// Not in the C suite. It pins the behaviour of the guard at
    /// `lyric_lane_edit.c:199`, which claims to leave a cue shorter than
    /// `LYRIC_LANE_MIN_CUE_SECONDS` "exactly as it was" but tests `start > end`,
    /// a condition the clamp to `end - MIN_CUE_SECONDS` above it has already made
    /// impossible. The observable consequence is a *negative* start, which the
    /// lyrics model rejects — so an edge drag on such a cue silently does
    /// nothing. Reproduced, not fixed: the oracle is the reference.
    #[test]
    fn lyric_lane_clamp_resize_reproduces_the_oracles_dead_short_cue_guard() {
        let mut document = TestDocument::new(TRACK);
        document.insert(0.0, 0.01);

        let (start, end) = clamp_resize(&document, 1, true, 0.005).expect("cue 1 exists");
        expect_near(start, -0.01, 1e-12);
        expect_near(end, 0.01, 1e-12);
        // ... and that is exactly the pair the model refuses.
        assert!(!document.retime(1, start, end));
    }

    /// Not in the C suite either: `hit_test`'s own degenerate-input guards
    /// (`lyric_lane_edit.c:21-22`) are only reached through a caller that has
    /// already lost, but they are the difference between an empty lane and a NaN
    /// comparison deciding a gesture.
    #[test]
    fn lyric_lane_hit_test_rejects_a_degenerate_lane() {
        let document = build_lane();
        let view = whole_view();
        assert_eq!(
            hit_test(&document, &view, f64::NAN, LANE_WIDTH, 125.0),
            None
        );
        assert_eq!(hit_test(&document, &view, LANE_X, f64::NAN, 125.0), None);
        assert_eq!(hit_test(&document, &view, LANE_X, 0.0, 125.0), None);
        assert_eq!(
            hit_test(&document, &view, LANE_X, LANE_WIDTH, f64::NAN),
            None
        );
    }

    /// `shift_headroom`'s selection-resolve behaviour, which the C exercises
    /// through `lyrics_shift_many`'s own tests rather than through the lane
    /// (`lyrics.c:320-336`, `lyrics.h:96-98`).
    #[test]
    fn lane_cues_shift_headroom_resolves_the_whole_selection_or_none_of_it() {
        let document = build_lane();
        // One unknown id rejects the request even though the others resolve.
        assert_eq!(document.shift_headroom(&[1, 99]), None);
        // An empty request is the C's LYRICS_ERROR_NOT_FOUND.
        assert_eq!(document.shift_headroom(&[]), None);
        // Repeated ids collapse rather than counting twice.
        let (backward, forward) = document.shift_headroom(&[3, 3, 3]).expect("cue 3 resolves");
        expect_near(backward, 30.0, 1e-9);
        expect_near(forward, TRACK - 31.0, 1e-9);
        // Both outputs are >= 0 even for a selection flush against an end.
        let mut edge = TestDocument::new(TRACK);
        edge.insert(0.0, TRACK);
        let (backward, forward) = edge.shift_headroom(&[1]).expect("cue 1 resolves");
        expect_near(backward, 0.0, 1e-9);
        expect_near(forward, 0.0, 1e-9);
    }

    /// The real document behind the trait.
    ///
    /// Index order is the contract two of the rules depend on — the hit test
    /// walks it backwards, and a shift+click range is a contiguous index range —
    /// so it is asserted against the document that actually keeps the cues rather
    /// than only against the `Vec` the tests above use.
    #[test]
    fn a_lyrics_document_presents_its_cues_to_the_lane_in_canonical_order() {
        use crate::project::lyrics::{LyricCue, LyricsDocument};

        let mut document = LyricsDocument::new(TRACK).expect("a positive duration");
        // Inserted out of order on purpose: the lane is promised canonical order,
        // not insertion order.
        for start in [30.0, 10.0, 20.0] {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 1.0,
                    text: format!("cue at {start}"),
                })
                .expect("the fixture cues are valid");
        }
        assert_eq!(document.cue_count(), 3);
        let starts: Vec<f64> = (0..document.cue_count())
            .map(|index| document.cue_at(index).expect("in range").start_seconds)
            .collect();
        assert_eq!(starts, vec![10.0, 20.0, 30.0]);
        assert_eq!(document.cue_at(3), None);
        expect_near(LaneCues::duration_seconds(&document), TRACK, 1e-12);

        // The ids came out of the document, so a selection over them resolves.
        let second = document.cue_at(1).expect("in range").id;
        let mut selection = LyricLaneSelection::new();
        assert!(selection.apply(&document, second, LyricLaneClick::Replace));
        let (backward, forward) = document
            .shift_headroom(selection.ids())
            .expect("a live id resolves");
        expect_near(backward, 20.0, 1e-9);
        expect_near(forward, TRACK - 21.0, 1e-9);
    }
}
