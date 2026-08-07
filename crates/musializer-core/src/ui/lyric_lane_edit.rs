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

use super::{timed_lane, timeline_view::TimelineView};

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

/// Shortest cue **any edit here may produce** (`lyric_lane_edit.h:34-37`).
///
/// The lyrics model only demands `end > start`, so without a floor a single
/// sloppy drag can collapse a cue to a sliver that is then impossible to grab
/// again.
///
/// Named without `LANE` because it is no longer only the lane's: the editing
/// form's timing rows clamp against the same number (review 1.1, which found the
/// form using a 1 ms floor while the lane used this one). It governs *edits the
/// application makes*, never what a document may contain —
/// [`crate::project::lyrics::LyricsDocument`] still loads a shorter cue, and
/// tightening it there would make a currently-openable `.musi` unopenable.
pub const LYRIC_MIN_CUE_SECONDS: f64 = 0.02;

/// Pointer travel before a press counts as a drag rather than a click
/// (`lyric_lane_edit.h:39-42`).
///
/// Below it the gesture is a selection, so a click never nudges a cue by a
/// pixel's worth of time and silently dirties the project.
pub const LYRIC_LANE_DRAG_THRESHOLD_PIXELS: f64 = timed_lane::DRAG_THRESHOLD_PIXELS;

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
        let Some(block) = timed_lane::block_geometry(
            view,
            lane_x,
            lane_width,
            cue.start_seconds,
            cue.end_seconds,
        ) else {
            continue;
        };
        if !block.contains_x(pointer_x) {
            continue;
        }

        let mut zone = LyricLaneZone::Body;
        if block.true_width() >= LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS {
            // Only offer a handle for a boundary that is actually on screen.
            if block.start_visible && pointer_x <= block.true_left + LYRIC_LANE_EDGE_GRAB_PIXELS {
                zone = LyricLaneZone::StartEdge;
            } else if block.end_visible
                && pointer_x >= block.true_right - LYRIC_LANE_EDGE_GRAB_PIXELS
            {
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
/// cue that starts within `LYRIC_MIN_CUE_SECONDS` of it, and the cue is then
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
        let latest = end - LYRIC_MIN_CUE_SECONDS;
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
        let earliest = start + LYRIC_MIN_CUE_SECONDS;
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

/// The editing form's START row, clamped so it can never invert (review 1.1).
///
/// Not from the oracle: the C writes two sequential `if`s over a `double` and
/// merely produces a slightly out-of-range value, but the Rust port reached for
/// `f64::clamp`, **which panics when `min > max`**. A cue shorter than
/// [`LYRIC_MIN_CUE_SECONDS`] is loadable — `validate_cue` asks only for
/// `end > start`, and so does TSV import — so `clamp(0.0, end - gap)` with a
/// sub-gap cue took the whole process down on the next frame that drew the row.
///
/// Total for every input, including NaN, because the panic it replaces was
/// reachable from a file on disk rather than from a mistake in this crate. The
/// order matters and is the fix: the ceiling is applied first and the floor
/// second, so a cue that is *already* shorter than the gap keeps its start at 0
/// rather than being pushed negative into a pair the model would refuse.
#[must_use]
pub fn clamp_form_start(value: f64, end_seconds: f64) -> f64 {
    value.min(end_seconds - LYRIC_MIN_CUE_SECONDS).max(0.0)
}

/// The editing form's END row, the same way (review 1.1).
///
/// The mirror-image panic: `clamp(start + gap, duration)` inverts for a cue that
/// starts within the gap of the end of the track. The track length wins here, as
/// 0 wins in [`clamp_form_start`], because `end <= duration` is what the model
/// checks and `end > start` then follows from `start < duration`.
#[must_use]
pub fn clamp_form_end(value: f64, start_seconds: f64, duration_seconds: f64) -> f64 {
    value
        .max(start_seconds + LYRIC_MIN_CUE_SECONDS)
        .min(duration_seconds)
}

// ---------------------------------------------------------------------------
// The timing loop: nudge ladder, typed times, play-and-tap stamping
// ---------------------------------------------------------------------------
//
// **Invented, not the oracle's** (UX0-B02/B04, UX0-C03). The frozen C has one
// fixed 0.1 s nudge button pair per time row (`lyrics_editor_ui.c:219-250`), no
// typed times and no stamping at all. Everything below is here rather than in
// the panel for the usual reason: "did that tap land 20 ms after the previous
// one, or 20 ms before it" is not a question a screenshot can answer, and the
// tap loop is the one surface in this editor a user judges by feel.

/// The cue-timing nudge step, in seconds, for the modifier keys held.
///
/// Deliberately the same *shape* as the transport's own ladder
/// ([`crate::ui::transport_bar::seek_step_seconds`]) — Control is fine, Shift is
/// coarse, and **fine wins when both are held** — at a tenth of its scale,
/// because a cue boundary is placed against a syllable and a playhead is placed
/// against a section. Sharing the shape rather than the numbers is the point: a
/// user who learned Ctrl on the transport two rows above does not have to learn
/// a second meaning for it here, and the smaller step stays the recoverable
/// mistake at both scales.
#[must_use]
pub fn cue_nudge_step_seconds(fine: bool, coarse: bool) -> f64 {
    match (fine, coarse) {
        (true, _) => 0.01,
        (false, true) => 1.0,
        (false, false) => 0.1,
    }
}

/// Reads a typed cue time back out of the form's own `MM:SS.mmm` readout.
///
/// The inverse of `musializer_app::ui::widgets::format_timestamp`
/// (`ui_widgets.c:329-335`). The two live in different crates because the
/// formatter is chrome and the parser is an edit — a mistyped parse writes a
/// wrong number into a `.musi` file, so it belongs where it can be tested
/// without a GPU. `lyrics.rs`'s `a_typed_time_round_trips_through_the_forms_own_readout`
/// pins the pair against the real formatter so the split cannot drift.
///
/// Accepts what a user will actually type rather than only what the readout
/// prints: `01:23.456`, `1:23.4`, `1:23`, `83.456` and `83` all mean the same
/// instant. Refuses anything else — including a negative sign, an hour field and
/// a seconds field of 60 or more — because a silently-clamped typed time is a
/// control that lies about what it took.
#[must_use]
pub fn parse_cue_timestamp(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || text.len() > 32 {
        return None;
    }
    // A bare `+`/`-` would otherwise reach the float parse below and make
    // `-1:30` mean 90 seconds, or `1e3` mean a thousand.
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b':' || byte == b'.')
    {
        return None;
    }
    let (minutes, rest) = match text.split_once(':') {
        None => (0.0, text),
        Some((minutes, rest)) => {
            if minutes.is_empty() || rest.contains(':') {
                return None;
            }
            (minutes.parse::<u32>().ok().map(f64::from)?, rest)
        }
    };
    if rest.is_empty() {
        return None;
    }
    let seconds = rest.parse::<f64>().ok()?;
    // `60.0` is a minute the user meant to type as `01:00`, and accepting it
    // makes two different strings mean one instant in a field whose whole job is
    // to be unambiguous. Only enforced where a minutes field was actually
    // written: `83.456` is a legitimate way to type 1:23.456.
    if !seconds.is_finite() || seconds < 0.0 || (text.contains(':') && seconds >= 60.0) {
        return None;
    }
    let total = minutes * 60.0 + seconds;
    total.is_finite().then_some(total)
}

/// Why a tap did not stamp anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapRefusal {
    /// Tap mode is not armed. Not an error the user needs told about.
    NotArmed,
    /// The document has no cues to place. Tapping stamps the times of lines that
    /// already exist; it never invents text.
    NoCues,
    /// The run is over — every cue in the armed order has been stamped and
    /// closed.
    Finished,
    /// The playhead has not moved [`LYRIC_MIN_CUE_SECONDS`] past the previous
    /// stamp. Refused rather than clamped: two taps inside 20 ms is a double
    /// keypress, and silently accepting one would leave a cue nobody can see and
    /// nobody meant.
    TooSoon,
    /// The playhead is at or past the end of the track, so there is no room for
    /// a cue at all.
    PastEnd,
}

impl core::fmt::Display for TapRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotArmed => "Tap mode is not armed.",
            Self::NoCues => "There are no cues to stamp. Add or import lines first.",
            Self::Finished => "Every line in this run has been stamped.",
            Self::TooSoon => "That tap landed less than 20 ms after the previous one.",
            Self::PastEnd => "The playhead is at the end of the track.",
        })
    }
}

/// One accepted tap: the retimings it asks for, and where the run now stands.
///
/// A tap is up to **two** retimings, and that pairing is the whole feature
/// rather than an optimisation. Stamping a line's start is also the answer to
/// where the previous line ends (review 1.14), so the previous cue is closed at
/// the same instant the next one opens — which is what makes a run of taps
/// produce contiguous captions instead of a row of cues with whatever durations
/// they happened to arrive with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TapStamp {
    /// `(id, start_seconds, end_seconds)`, in the order they must be applied.
    pub retimes: Vec<(u64, f64, f64)>,
    /// The cue whose start this tap placed, or `0` when the tap only closed the
    /// final line.
    pub opened_id: u64,
    /// Lines still waiting for a start after this tap.
    pub remaining: usize,
    /// Whether this tap ended the run.
    pub finished: bool,
}

/// Play-and-tap stamping (UX0-C03).
///
/// The instrument. Arm it, start playback, and press the tap key once per line:
/// each press closes the line before and opens the next one at the playhead.
///
/// ## Why the order is captured at arming time
///
/// [`LyricTap::order`] is a snapshot of the cue ids taken when the run is armed,
/// and the cursor is an index into it. It cannot be recomputed from the document
/// on each tap, because a stamp *moves* a cue and the document sorts by start
/// time: stamping line 1 at 0:10 while lines 2-4 still sit at their imported
/// 0:01-0:03 re-sorts it behind them, and a cursor that walked canonical order
/// would then hand out line 2 twice and never reach line 1 again. The snapshot
/// makes that unstateable rather than guarded against.
///
/// Ids rather than indices for the same reason one layer down: an id survives
/// the re-sort, and a cue deleted mid-run is skipped by
/// [`LaneCues::find`] returning `None` rather than silently stamping its
/// neighbour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricTap {
    armed: bool,
    order: Vec<u64>,
    cursor: usize,
    offset_seconds: f64,
    stamped: usize,
    /// The cue the previous tap opened, and the instant it opened at, so the
    /// next tap can close it without re-reading a document that may not have
    /// been written yet. The panel defers its edits by a frame, so the document
    /// a tap reads is one edit behind — trusting it here would close the
    /// previous line at its *old* end.
    open_id: u64,
    open_start: f64,
}

/// Largest tap offset the control will take, either way, in seconds.
///
/// A quarter of a second each way covers the two things an offset is for — a
/// user who taps consistently early or late, and the display/audio latency of
/// the machine — without becoming a way to move a cue somewhere it does not
/// belong. Past that, drag it.
pub const LYRIC_TAP_OFFSET_LIMIT_SECONDS: f64 = 0.25;

/// One offset step, in seconds. Ten milliseconds is roughly the smallest
/// difference a listener can hear on a transient.
pub const LYRIC_TAP_OFFSET_STEP_SECONDS: f64 = 0.01;

impl LyricTap {
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    #[must_use]
    pub fn offset_seconds(&self) -> f64 {
        self.offset_seconds
    }

    /// Lines stamped so far in this run.
    #[must_use]
    pub fn stamped(&self) -> usize {
        self.stamped
    }

    /// How many lines the armed run holds in total.
    #[must_use]
    pub fn total(&self) -> usize {
        self.order.len()
    }

    /// The cue the next tap will open, or `None` when the run is spent.
    #[must_use]
    pub fn target_id(&self) -> Option<u64> {
        self.order.get(self.cursor).copied()
    }

    /// One-based position of the next line, for a readout.
    #[must_use]
    pub fn position(&self) -> usize {
        (self.cursor + 1).min(self.order.len().max(1))
    }

    /// Arms a run over every cue in the document, starting at the first one that
    /// begins at or after `from_seconds`.
    ///
    /// Starting from the playhead rather than from the top is what makes the
    /// loop usable on a long track: a user fixing the second verse arms it
    /// there, taps the eight lines they care about and stops, rather than
    /// re-stamping forty lines to reach them.
    ///
    /// Returns `false` and stays disarmed when there is nothing to stamp.
    pub fn arm(&mut self, document: &impl LaneCues, from_seconds: f64) -> bool {
        // Every exit below is a *disarmed* one until the new run is built. A
        // refused arm that left the previous run in place would keep an order of
        // ids from another document loaded, and the next tap would stamp
        // whichever cue here happened to share an id with one there.
        self.disarm();
        let count = document.cue_count();
        if count == 0 {
            return false;
        }
        let from = if from_seconds.is_finite() {
            from_seconds
        } else {
            0.0
        };
        let mut order = Vec::with_capacity(count);
        for index in 0..count {
            if let Some(cue) = document.cue_at(index) {
                order.push(cue.id);
            }
        }
        // The first line whose *start* is not already behind the playhead. A
        // line the playhead is sitting inside is still the one a user means to
        // re-place, which is why this is `>=` against the start rather than a
        // containment test.
        let cursor = (0..count)
            .find(|index| {
                document
                    .cue_at(*index)
                    .is_some_and(|cue| cue.start_seconds >= from)
            })
            .unwrap_or(0);
        if order.is_empty() {
            return false;
        }
        self.armed = true;
        self.order = order;
        self.cursor = cursor;
        self.stamped = 0;
        self.open_id = 0;
        self.open_start = 0.0;
        true
    }

    /// Ends the run, keeping the offset the user dialled in.
    ///
    /// The offset survives on purpose: it is a property of this person tapping
    /// on this machine, not of the run, and making them re-find it every time
    /// they re-arm is how a calibration control becomes one nobody uses.
    pub fn disarm(&mut self) {
        self.armed = false;
        self.order.clear();
        self.cursor = 0;
        self.stamped = 0;
        self.open_id = 0;
        self.open_start = 0.0;
    }

    /// Moves the offset by `steps` of [`LYRIC_TAP_OFFSET_STEP_SECONDS`],
    /// saturating at [`LYRIC_TAP_OFFSET_LIMIT_SECONDS`].
    pub fn adjust_offset(&mut self, steps: i32) {
        let proposed = self.offset_seconds + f64::from(steps) * LYRIC_TAP_OFFSET_STEP_SECONDS;
        self.offset_seconds = if proposed.is_finite() {
            proposed.clamp(
                -LYRIC_TAP_OFFSET_LIMIT_SECONDS,
                LYRIC_TAP_OFFSET_LIMIT_SECONDS,
            )
        } else {
            0.0
        };
    }

    /// One tap at `at_seconds` on the transport clock.
    ///
    /// Pure: it decides what the tap *means* and hands back retimings for the
    /// caller to enqueue, exactly as [`hit_test`] does for the pointer. Nothing
    /// here writes to the document, so a refusal cannot leave half a stamp
    /// behind.
    ///
    /// The stamped instant is `at_seconds + offset`, floored at 0 and at
    /// [`LYRIC_MIN_CUE_SECONDS`] past the previous stamp. The cue keeps its own
    /// duration as a provisional end, so an imported document's authored line
    /// lengths survive until the next tap tightens them — a fixed default would
    /// throw away real information on the very documents this loop exists for.
    pub fn tap(
        &mut self,
        document: &impl LaneCues,
        at_seconds: f64,
    ) -> Result<TapStamp, TapRefusal> {
        if !self.armed {
            return Err(TapRefusal::NotArmed);
        }
        if self.order.is_empty() {
            return Err(TapRefusal::NoCues);
        }
        let duration = document.duration_seconds();
        if !at_seconds.is_finite() || !duration.is_finite() || duration <= 0.0 {
            return Err(TapRefusal::PastEnd);
        }
        let at = (at_seconds + self.offset_seconds).max(0.0);
        if at >= duration - LYRIC_MIN_CUE_SECONDS {
            return Err(TapRefusal::PastEnd);
        }
        if self.open_id != 0 && at < self.open_start + LYRIC_MIN_CUE_SECONDS {
            return Err(TapRefusal::TooSoon);
        }

        // Skip past ids the document no longer has: a cue deleted between arming
        // and this tap must cost the run one line, not misroute the stamp onto
        // whichever cue inherited its position.
        while self.cursor < self.order.len() && document.find(self.order[self.cursor]).is_none() {
            self.cursor += 1;
        }

        let closes = (self.open_id != 0)
            .then(|| document.find(self.open_id))
            .flatten()
            .map(|_| (self.open_id, self.open_start, at));

        let Some(&target) = self.order.get(self.cursor) else {
            // The run is spent, so this tap is the out point of the last line —
            // the N+1st press that closes a run of N. Without it the final
            // caption would keep whatever end it arrived with, which is the one
            // line a tap run would otherwise never fix.
            let Some((id, start, end)) = closes else {
                return Err(TapRefusal::Finished);
            };
            self.open_id = 0;
            self.armed = false;
            return Ok(TapStamp {
                retimes: vec![(id, start, end)],
                opened_id: 0,
                remaining: 0,
                finished: true,
            });
        };
        let Some(cue) = document.find(target) else {
            return Err(TapRefusal::Finished);
        };

        // The cue's own length, carried forward as a provisional end. Clamped
        // into the track and floored at the minimum so the retime the caller
        // enqueues is one the model will accept.
        let held = (cue.end_seconds - cue.start_seconds).max(LYRIC_MIN_CUE_SECONDS);
        let end = (at + held).min(duration).max(at + LYRIC_MIN_CUE_SECONDS);
        let end = end.min(duration);

        let mut retimes = Vec::with_capacity(2);
        if let Some(close) = closes {
            retimes.push(close);
        }
        retimes.push((target, at, end));

        self.cursor += 1;
        self.stamped += 1;
        self.open_id = target;
        self.open_start = at;
        Ok(TapStamp {
            retimes,
            opened_id: target,
            remaining: self.order.len().saturating_sub(self.cursor),
            finished: false,
        })
    }
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
        expect_near(end, 10.0 + LYRIC_MIN_CUE_SECONDS, 1e-9);
        assert!(end > start);

        // The leading edge, symmetrically.
        let (start, end) = clamp_resize(&document, 1, true, 7.5).expect("cue 1 exists");
        expect_near(start, 7.5, 1e-9);
        expect_near(end, 11.0, 1e-9);
        let (start, _) = clamp_resize(&document, 1, true, -40.0).expect("cue 1 exists");
        expect_near(start, 0.0, 1e-9);
        let (start, _) = clamp_resize(&document, 1, true, 50.0).expect("cue 1 exists");
        expect_near(start, 11.0 - LYRIC_MIN_CUE_SECONDS, 1e-9);

        // Whatever it produces, a retime must accept it.
        let (start, end) = clamp_resize(&document, 1, true, 50.0).expect("cue 1 exists");
        assert!(document.retime(1, start, end));

        assert_eq!(clamp_resize(&document, 99, true, 1.0), None);
        assert_eq!(clamp_resize(&document, 1, true, f64::NAN), None);
    }

    /// Review 1.1: the shape that used to panic the process.
    ///
    /// Every pair here is one `validate_cue` accepts, so every one of them is
    /// loadable from a `.musi`, a TSV import or an aligner — and each one made
    /// `f64::clamp` assert `min <= max` in the editing form's timing rows.
    #[test]
    fn a_sub_millisecond_cue_no_longer_panics_the_timing_rows() {
        const TRACK: f64 = 100.0;
        // A 0.5 ms cue against the start of the track: `clamp(0.0, -0.0005)`.
        let start = clamp_form_start(0.0, 0.000_5);
        expect_near(start, 0.0, 1e-12);
        assert!(
            start < 0.000_5,
            "the pair the model gets must keep end > start"
        );
        // ... and both nudge buttons on the same cue.
        expect_near(clamp_form_start(0.1, 0.000_5), 0.0, 1e-12);
        expect_near(clamp_form_start(-0.1, 0.000_5), 0.0, 1e-12);

        // A 0.5 ms cue against the end of the track: `clamp(100.0005, 100.0)`.
        let end = clamp_form_end(TRACK, TRACK - 0.000_5, TRACK);
        expect_near(end, TRACK, 1e-12);
        assert!(end > TRACK - 0.000_5);
        expect_near(
            clamp_form_end(TRACK + 5.0, TRACK - 0.000_5, TRACK),
            TRACK,
            1e-12,
        );
        expect_near(clamp_form_end(0.0, TRACK - 0.000_5, TRACK), TRACK, 1e-12);

        // A sub-gap cue mid-track, from both rows: the floor widens it rather
        // than inverting, and neither row can produce `start >= end`.
        expect_near(clamp_form_start(9.999_5, 9.999_5), 9.979_5, 1e-12);
        expect_near(clamp_form_end(10.0, 10.0, TRACK), 10.02, 1e-12);

        // Degenerate values reach these rows through the same draft fields, and a
        // NaN is what `f64::clamp` panics on second.
        assert!(clamp_form_start(f64::NAN, 5.0).is_finite());
        assert!(clamp_form_end(f64::NAN, 5.0, TRACK).is_finite());
        expect_near(clamp_form_start(1.0, f64::NAN), 1.0, 1e-12);
        expect_near(clamp_form_end(1.0, f64::NAN, TRACK), 1.0, 1e-12);
    }

    /// The form and the lane now clamp against the same floor (review 1.1).
    ///
    /// They did not: the form used 1 ms and the lane 20 ms, so a resize drag and
    /// a `-0.1` press disagreed about the shortest cue the editor allows.
    #[test]
    fn the_form_and_the_lane_agree_on_the_minimum_cue_length() {
        let document = build_lane();
        // Cue 1 is 10 s to 11 s. Dragging its leading edge past the end stops at
        // the floor ...
        let (start, _) = clamp_resize(&document, 1, true, 50.0).expect("cue 1 exists");
        expect_near(start, 11.0 - LYRIC_MIN_CUE_SECONDS, 1e-9);
        // ... and so does typing the same move into the form's START row.
        expect_near(
            clamp_form_start(50.0, 11.0),
            11.0 - LYRIC_MIN_CUE_SECONDS,
            1e-9,
        );

        let (_, end) = clamp_resize(&document, 1, false, 2.0).expect("cue 1 exists");
        expect_near(end, 10.0 + LYRIC_MIN_CUE_SECONDS, 1e-9);
        expect_near(
            clamp_form_end(2.0, 10.0, TRACK),
            10.0 + LYRIC_MIN_CUE_SECONDS,
            1e-9,
        );
    }

    /// Not in the C suite. It pins the behaviour of the guard at
    /// `lyric_lane_edit.c:199`, which claims to leave a cue shorter than
    /// `LYRIC_MIN_CUE_SECONDS` "exactly as it was" but tests `start > end`,
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
                    origin: Default::default(),
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

    // -----------------------------------------------------------------------
    // The timing loop (UX0-B02/B04, UX0-C03)
    // -----------------------------------------------------------------------

    impl TestDocument {
        /// Applies what a [`TapStamp`] asked for, keeping canonical order — the
        /// same thing `LyricsDocument::retime` does one crate layer up. The tap
        /// tests need it because the defect they exist for only appears once a
        /// stamp has actually moved a cue past its neighbours.
        fn commit(&mut self, stamp: &TapStamp) {
            for (id, start, end) in &stamp.retimes {
                let cue = self
                    .0
                    .iter_mut()
                    .find(|cue| cue.id == *id)
                    .expect("a retime names a live cue");
                cue.start_seconds = *start;
                cue.end_seconds = *end;
            }
            self.0.sort_by(|left, right| {
                left.start_seconds
                    .partial_cmp(&right.start_seconds)
                    .expect("finite starts")
                    .then(left.id.cmp(&right.id))
            });
        }

        fn remove(&mut self, id: u64) {
            self.0.retain(|cue| cue.id != id);
        }

        fn span(&self, id: u64) -> (f64, f64) {
            let cue = self.find(id).expect("a live cue");
            (cue.start_seconds, cue.end_seconds)
        }
    }

    /// Four lines imported at the top of the track, as an aligner or a TSV
    /// import leaves them.
    fn tap_fixture() -> TestDocument {
        let mut document = TestDocument::new(TRACK);
        document.insert(0.0, 1.0);
        document.insert(1.0, 2.0);
        document.insert(2.0, 3.0);
        document.insert(3.0, 4.0);
        document
    }

    #[test]
    fn the_nudge_ladder_teaches_the_same_modifiers_as_the_transport() {
        expect_near(cue_nudge_step_seconds(false, false), 0.1, 1e-12);
        expect_near(cue_nudge_step_seconds(true, false), 0.01, 1e-12);
        expect_near(cue_nudge_step_seconds(false, true), 1.0, 1e-12);
        // Fine wins when both are held, exactly as `seek_step_seconds` decides
        // it. The two ladders differ in scale and agree in shape, and this is
        // the assertion that keeps them agreeing.
        expect_near(cue_nudge_step_seconds(true, true), 0.01, 1e-12);
        // Strictly increasing coarseness, so no two rungs can silently become
        // the same step.
        assert!(
            cue_nudge_step_seconds(true, false) < cue_nudge_step_seconds(false, false)
                && cue_nudge_step_seconds(false, false) < cue_nudge_step_seconds(false, true)
        );
    }

    #[test]
    fn a_typed_time_takes_every_shape_a_user_would_write_it_in() {
        expect_near(
            parse_cue_timestamp("01:23.456").expect("mm:ss.mmm"),
            83.456,
            1e-9,
        );
        expect_near(parse_cue_timestamp("1:23.4").expect("m:ss.m"), 83.4, 1e-9);
        expect_near(parse_cue_timestamp("1:23").expect("m:ss"), 83.0, 1e-9);
        expect_near(
            parse_cue_timestamp("83.456").expect("bare seconds"),
            83.456,
            1e-9,
        );
        expect_near(
            parse_cue_timestamp("83").expect("bare whole seconds"),
            83.0,
            1e-9,
        );
        expect_near(
            parse_cue_timestamp("  0:00.000  ").expect("padded"),
            0.0,
            1e-9,
        );
        expect_near(
            parse_cue_timestamp("10:00").expect("ten minutes"),
            600.0,
            1e-9,
        );
    }

    #[test]
    fn a_typed_time_refuses_rather_than_guessing() {
        // Every one of these parses as *something* under a looser reading, and
        // each would write a number the user did not type into a `.musi` file.
        for text in [
            "", "   ", "abc", "-1:30", // a negative cue time is not a thing
            "+30",   // ditto, and `f64::parse` takes it
            "1e3",   // `f64::parse` takes this as 1000 seconds
            "1:2:3", // an hours field this readout never prints
            ":30",   // no minutes
            "1:",    // no seconds
            "1:60",  // the minute the user meant to write as 2:00
            "1:99.5", "NaN", "inf", "1,5",
        ] {
            assert!(
                parse_cue_timestamp(text).is_none(),
                "{text:?} should not parse"
            );
        }
    }

    #[test]
    fn arming_starts_at_the_playhead_rather_than_at_the_top_of_the_track() {
        let document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        assert_eq!(tap.target_id(), Some(1));
        assert_eq!(tap.total(), 4);

        // Armed from inside line 2: line 2 is still the one to re-place, because
        // a user parked inside a line means that line.
        assert!(tap.arm(&document, 1.5));
        assert_eq!(tap.target_id(), Some(3));
        assert!(tap.arm(&document, 1.0));
        assert_eq!(tap.target_id(), Some(2));

        // Past every cue, the run wraps to the top rather than arming an empty
        // one: an armed run that can never stamp is a control that does nothing.
        assert!(tap.arm(&document, 90.0));
        assert_eq!(tap.target_id(), Some(1));

        assert!(!tap.arm(&TestDocument::new(TRACK), 0.0));
        assert!(!tap.is_armed());
    }

    #[test]
    fn each_tap_closes_the_line_before_it_and_opens_the_next() {
        let mut document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));

        // The first tap has nothing to close, so it is one retime.
        let first = tap.tap(&document, 10.0).expect("the first tap stamps");
        assert_eq!(first.retimes.len(), 1);
        assert_eq!(first.opened_id, 1);
        assert_eq!(first.remaining, 3);
        assert!(!first.finished);
        // The cue kept its own 1.0 s length as a provisional end.
        expect_near(first.retimes[0].1, 10.0, 1e-9);
        expect_near(first.retimes[0].2, 11.0, 1e-9);
        document.commit(&first);

        // The second closes the first at exactly where the second begins, which
        // is review 1.14's rule and the reason a run of taps produces contiguous
        // captions.
        let second = tap.tap(&document, 14.0).expect("the second tap stamps");
        assert_eq!(second.retimes.len(), 2);
        assert_eq!(second.retimes[0].0, 1);
        expect_near(second.retimes[0].2, 14.0, 1e-9);
        assert_eq!(second.retimes[1].0, 2);
        expect_near(second.retimes[1].1, 14.0, 1e-9);
        document.commit(&second);
        expect_near(document.span(1).1, 14.0, 1e-9);
        expect_near(document.span(2).0, 14.0, 1e-9);
    }

    #[test]
    fn a_run_survives_the_re_sort_its_own_stamps_cause() {
        // The defect the arming-time order snapshot exists for. Stamping line 1
        // at 0:50 puts it *behind* lines 2-4, which still sit at 1-4 s. A cursor
        // walking canonical order would hand out line 2 next, then line 3, then
        // line 4, and never reach line 1 — or hand out line 2 twice.
        let mut document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));

        let mut opened = Vec::new();
        for at in [50.0, 52.0, 54.0, 56.0] {
            let stamp = tap.tap(&document, at).expect("a tap inside the track");
            opened.push(stamp.opened_id);
            document.commit(&stamp);
        }
        assert_eq!(opened, vec![1, 2, 3, 4], "every line stamped exactly once");
        expect_near(document.span(1).0, 50.0, 1e-9);
        expect_near(document.span(4).0, 56.0, 1e-9);

        // The N+1st tap is the last line's out point, and it ends the run.
        let close = tap.tap(&document, 58.0).expect("the closing tap");
        assert!(close.finished);
        assert_eq!(close.opened_id, 0);
        assert_eq!(close.retimes, vec![(4, 56.0, 58.0)]);
        assert!(!tap.is_armed());
        document.commit(&close);
        expect_near(document.span(4).1, 58.0, 1e-9);
    }

    #[test]
    fn a_double_keypress_is_refused_rather_than_collapsing_a_cue() {
        let mut document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        let first = tap.tap(&document, 10.0).expect("the first tap");
        document.commit(&first);

        // Inside the minimum gap: a bounced key, not a line.
        assert_eq!(
            tap.tap(&document, 10.0 + LYRIC_MIN_CUE_SECONDS * 0.5),
            Err(TapRefusal::TooSoon)
        );
        // And the refusal cost the run nothing — the same line is still next.
        assert_eq!(tap.target_id(), Some(2));
        assert!(tap.tap(&document, 10.0 + LYRIC_MIN_CUE_SECONDS).is_ok());
    }

    #[test]
    fn the_offset_moves_the_stamp_and_saturates_rather_than_running_away() {
        let document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        tap.adjust_offset(-10);
        expect_near(tap.offset_seconds(), -0.1, 1e-9);
        let stamp = tap.tap(&document, 10.0).expect("an offset tap");
        expect_near(stamp.retimes[0].1, 9.9, 1e-9);

        // Saturating, both ways, and the limit is a real bound rather than a
        // suggestion: an offset large enough to move a cue somewhere it does not
        // belong is a drag, not a calibration.
        tap.adjust_offset(10_000);
        expect_near(tap.offset_seconds(), LYRIC_TAP_OFFSET_LIMIT_SECONDS, 1e-9);
        tap.adjust_offset(-10_000);
        expect_near(tap.offset_seconds(), -LYRIC_TAP_OFFSET_LIMIT_SECONDS, 1e-9);

        // A negative offset can never stamp before the track starts.
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        tap.adjust_offset(-25);
        let stamp = tap.tap(&document, 0.05).expect("a tap near zero");
        expect_near(stamp.retimes[0].1, 0.0, 1e-9);
    }

    #[test]
    fn the_offset_outlives_the_run_but_the_cursor_does_not() {
        let document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        tap.adjust_offset(-8);
        let _ = tap.tap(&document, 10.0).expect("one stamp");
        assert_eq!(tap.stamped(), 1);
        tap.disarm();
        assert!(!tap.is_armed());
        assert_eq!(tap.stamped(), 0);
        assert_eq!(tap.target_id(), None);
        // The calibration is a property of the person and the machine, not of
        // the run.
        expect_near(tap.offset_seconds(), -0.08, 1e-9);
    }

    #[test]
    fn a_cue_deleted_mid_run_costs_one_line_rather_than_misrouting_the_stamp() {
        let mut document = tap_fixture();
        let mut tap = LyricTap::default();
        assert!(tap.arm(&document, 0.0));
        let first = tap.tap(&document, 10.0).expect("the first tap");
        document.commit(&first);
        // Line 2 goes away between taps. The next tap must reach line 3, not
        // stamp whichever cue now occupies index 1.
        document.remove(2);
        let second = tap.tap(&document, 12.0).expect("the next live line");
        assert_eq!(second.opened_id, 3);
        // And the dead line is not closed, because there is nothing to close.
        assert_eq!(second.retimes.len(), 2);
        assert_eq!(second.retimes[0].0, 1);
    }

    #[test]
    fn tapping_refuses_where_it_cannot_produce_a_cue_the_model_would_take() {
        let document = tap_fixture();
        let mut tap = LyricTap::default();
        assert_eq!(tap.tap(&document, 1.0), Err(TapRefusal::NotArmed));

        let empty = TestDocument::new(TRACK);
        assert!(!tap.arm(&empty, 0.0));
        assert_eq!(tap.tap(&empty, 1.0), Err(TapRefusal::NotArmed));

        assert!(tap.arm(&document, 0.0));
        // At the very end of the track there is no room for a cue at all, and a
        // clamped stamp would be a zero-length one the model refuses.
        assert_eq!(tap.tap(&document, TRACK), Err(TapRefusal::PastEnd));
        assert_eq!(tap.tap(&document, f64::NAN), Err(TapRefusal::PastEnd));
        assert_eq!(
            tap.tap(&document, TRACK - LYRIC_MIN_CUE_SECONDS * 0.5),
            Err(TapRefusal::PastEnd)
        );
        // A stamp just inside the end is fine, and its provisional end stops at
        // the track length rather than running past it.
        let stamp = tap.tap(&document, TRACK - 0.5).expect("a tap inside");
        expect_near(stamp.retimes[0].2, TRACK, 1e-9);
    }

    #[test]
    fn every_stamp_a_tap_produces_is_one_the_lane_would_accept() {
        // The contract between this module and the model: a retime a tap hands
        // back must never be one `LyricsDocument::retime` refuses. Swept rather
        // than spot-checked, because the arithmetic has three clamps in it.
        let mut document = tap_fixture();
        let mut tap = LyricTap::default();
        for offset_steps in [-25, -7, 0, 3, 25] {
            assert!(tap.arm(&document, 0.0));
            tap.disarm();
            tap.adjust_offset(offset_steps - (tap.offset_seconds() * 100.0).round() as i32);
            assert!(tap.arm(&document, 0.0));
            for step in 0..6 {
                let at = f64::from(step) * 0.5;
                match tap.tap(&document, at) {
                    Ok(stamp) => {
                        for (_, start, end) in &stamp.retimes {
                            assert!(start.is_finite() && end.is_finite());
                            assert!(*start >= 0.0, "start {start} below zero");
                            assert!(*end <= TRACK + 1e-9, "end {end} past the track");
                            assert!(
                                *end >= *start + LYRIC_MIN_CUE_SECONDS - 1e-9,
                                "span {start}..{end} shorter than the minimum"
                            );
                        }
                        document.commit(&stamp);
                    }
                    // `NotArmed` is reachable here and is not a fault: a run
                    // that reaches its closing tap disarms itself.
                    Err(
                        TapRefusal::TooSoon
                        | TapRefusal::PastEnd
                        | TapRefusal::Finished
                        | TapRefusal::NotArmed,
                    ) => {}
                    Err(other) => panic!("unexpected refusal {other:?}"),
                }
            }
        }
    }
}
