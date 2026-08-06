//! How overlapping lyric cues share one lane's height (LX1).
//!
//! Not from the oracle. The C draws every cue as a full-height block at its own
//! x range, so a stack of overlapping lines is one amber rectangle with hidden
//! edges inside it — you cannot see how many cues are there, let alone grab the
//! one you want. Two cues is survivable; the operator's track has runs of four.
//!
//! This module decides, for a whole document, which **row** of the lane each cue
//! draws in and how many **shade steps** darker it is. It is raylib-free for the
//! usual reason: "did cue 7 end up behind cue 8" is not a question a screenshot
//! can answer, and the row assignment is exactly the kind of arithmetic that
//! looks right in a capture while being wrong at the one width nobody tried.
//!
//! ## The three rules
//!
//! 1. **Clusters, not pairs.** Rows are assigned per maximal run of connected
//!    overlaps. Deciding per pair would let one cue's row change because a cue it
//!    does not touch appeared, and the lane would reshuffle while the user drags.
//! 2. **A cluster fans out only when three cues are live at once.** The
//!    operator's rule, and it keeps the common case — a line whose tail laps into
//!    the next — looking exactly as it did.
//! 3. **Row 0 is the cue that will actually show.** Rows are assigned walking the
//!    cluster *backwards*, because
//!    [`crate::project::lyrics::LyricsDocument::at_time`] resolves an overlap as
//!    "the last one in canonical order", so the last one is the line the preview
//!    and the export will use. Brightest and topmost therefore means "this is
//!    what you will see", and every step down is a line that is being covered.
//!    Assigning forwards would have made brightness track document order, which
//!    is not a thing the user can see or cares about.

use crate::project::lyrics::{CueOrigin, LyricsDocument};

/// How many shade steps deep the darkening goes before it stops.
///
/// At 20 % per step, step 5 is 33 % of the original brightness — past that a
/// block stops reading as its own colour and the state code goes with it. Deeper
/// rows keep the last shade rather than fading to black, because a cue nobody
/// can see the colour of is worse than two cues sharing one.
pub const LYRIC_STACK_MAX_SHADE_STEPS: usize = 4;

/// Fraction of brightness removed per step (the operator's 20 %).
pub const LYRIC_STACK_SHADE_PER_STEP: f32 = 0.20;

/// Live cues before a cluster fans out into rows.
///
/// Two overlapping cues are legible as one block with a seam; three are not.
pub const LYRIC_STACK_FAN_THRESHOLD: usize = 3;

/// Shortest a stacked block may become before the lane refuses to add rows.
///
/// Below this a block is a line rather than a target: [`LYRIC_LANE_EDGE_GRAB_PIXELS`]
/// worth of handle at each end needs something to be a handle *of*.
///
/// [`LYRIC_LANE_EDGE_GRAB_PIXELS`]: super::lyric_lane_edit::LYRIC_LANE_EDGE_GRAB_PIXELS
pub const LYRIC_STACK_MIN_BLOCK_PIXELS: f32 = 9.0;

/// Where one cue draws inside the lane, and how dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LyricStackSlot {
    /// 0 is the top row and the cue that [`LyricsDocument::at_time`] would hand
    /// a frame. Larger is further down and further behind.
    pub row: usize,
    /// How many 20 % steps to darken by. Equal to `row`, clamped to
    /// [`LYRIC_STACK_MAX_SHADE_STEPS`] — kept as its own field so a caller
    /// cannot accidentally use an uncapped row as a brightness multiplier.
    pub shade_steps: usize,
}

/// Row assignment for a whole document, indexed by canonical cue order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LyricStack {
    slots: Vec<LyricStackSlot>,
    rows: usize,
}

impl LyricStack {
    /// Assigns every cue a row and a shade.
    ///
    /// [`CueOrigin::Potential`] cues take part exactly like any other: a parked
    /// proposal sitting on top of a real cue is *the* case the operator hit, and
    /// hiding it from the stacker would put it back on top of the line it is
    /// asking about.
    #[must_use]
    pub fn build(document: &LyricsDocument) -> Self {
        let spans: Vec<(f64, f64)> = document
            .cues()
            .iter()
            .map(|cue| (cue.start_seconds, cue.end_seconds))
            .collect();
        Self::from_spans(&spans)
    }

    /// [`Self::build`] without a document, so the rules can be driven from a
    /// table. Spans must be in the document's canonical order.
    #[must_use]
    // `!(a < b)` rather than `a >= b`: a document can hold a non-finite span
    // before it validates, and the negated form puts a NaN on the *safe* side --
    // it closes the cluster instead of extending it forever.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn from_spans(spans: &[(f64, f64)]) -> Self {
        let mut slots = vec![LyricStackSlot::default(); spans.len()];
        let mut rows = 1usize;

        let mut cluster_start = 0usize;
        let mut cluster_end_seconds = f64::NEG_INFINITY;
        for index in 0..=spans.len() {
            // A cluster closes when the next cue starts at or after every
            // preceding end, or when the list runs out.
            let breaks = match spans.get(index) {
                None => true,
                Some(&(start, _)) => !(start < cluster_end_seconds),
            };
            if breaks && index > cluster_start {
                let used = assign_cluster(
                    &spans[cluster_start..index],
                    &mut slots[cluster_start..index],
                );
                rows = rows.max(used);
                cluster_start = index;
                cluster_end_seconds = f64::NEG_INFINITY;
            } else if breaks {
                cluster_start = index;
            }
            if let Some(&(_, end)) = spans.get(index) {
                if end > cluster_end_seconds {
                    cluster_end_seconds = end;
                }
            }
        }

        Self { slots, rows }
    }

    /// The slot for a canonical index. Out of range yields the default, which is
    /// the flat single-row look — a caller that lost sync draws the lane it used
    /// to draw rather than nothing.
    #[must_use]
    pub fn slot(&self, index: usize) -> LyricStackSlot {
        self.slots.get(index).copied().unwrap_or_default()
    }

    /// Most rows any one cluster needed. `1` when nothing fans out.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Whether anything fanned out at all, for a report line.
    #[must_use]
    pub fn is_flat(&self) -> bool {
        self.rows <= 1
    }
}

/// Greedy interval colouring over one cluster, walked backwards.
///
/// Returns the number of rows used. Backwards is rule 3: the last cue in
/// canonical order is the one `at_time` resolves to, and it gets row 0.
// `!(end > claimed)` for `from_spans`'s reason: a NaN end must not be handed a
// row it then owns for the rest of the cluster.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn assign_cluster(spans: &[(f64, f64)], slots: &mut [LyricStackSlot]) -> usize {
    // How many cues are live at once anywhere in the cluster. Below the
    // threshold the whole cluster stays flat, so a cue's row cannot change
    // because of a neighbour it does not overlap.
    if peak_multiplicity(spans) < LYRIC_STACK_FAN_THRESHOLD {
        for slot in slots.iter_mut() {
            *slot = LyricStackSlot::default();
        }
        return 1;
    }

    // `row_start[r]` is the earliest start still claimed by row r. Walking
    // backwards, a row is free for this cue when the row's current earliest
    // start is at or after this cue's end.
    let mut row_start: Vec<f64> = Vec::new();
    for index in (0..spans.len()).rev() {
        let (start, end) = spans[index];
        let row = row_start
            .iter()
            .position(|&claimed| !(end > claimed))
            .unwrap_or_else(|| {
                row_start.push(f64::INFINITY);
                row_start.len() - 1
            });
        row_start[row] = start;
        slots[index] = LyricStackSlot {
            row,
            shade_steps: row.min(LYRIC_STACK_MAX_SHADE_STEPS),
        };
    }
    row_start.len()
}

/// The largest number of spans covering any single instant in the cluster.
fn peak_multiplicity(spans: &[(f64, f64)]) -> usize {
    // Every overlap change happens at some cue's start, so sampling the starts
    // finds the peak without sorting a second time.
    let mut peak = 0usize;
    for &(probe, _) in spans {
        let live = spans
            .iter()
            .filter(|&&(start, end)| start <= probe && probe < end)
            .count();
        if live > peak {
            peak = live;
        }
    }
    peak
}

/// The pixel geometry one lane hands a stacked block: `(y_offset, height)`
/// relative to the lane's own content box.
///
/// Bottom-aligned on purpose. Every block ends at the lane's bottom edge and the
/// deeper ones start lower, so the stack reads as sheets of paper slid out from
/// under each other — and the *top* edge, which is the one the eye counts, is
/// the one that moves. Aligning the tops instead would have made the row count
/// legible only at the bottom, where the playhead crosses.
#[must_use]
pub fn row_geometry(lane_height: f32, rows: usize, row: usize) -> (f32, f32) {
    if !lane_height.is_finite() || lane_height <= 0.0 || rows <= 1 {
        return (0.0, lane_height.max(0.0));
    }
    let rows = rows.max(1);
    // Every extra row costs one step of height, and the step shrinks rather than
    // the block going under the grabbable floor. A cluster deeper than the lane
    // can seat therefore crowds instead of vanishing.
    let spare = (lane_height - LYRIC_STACK_MIN_BLOCK_PIXELS).max(0.0);
    let step = spare / (rows - 1) as f32;
    let offset = step * row as f32;
    ((offset).min(lane_height), (lane_height - offset).max(1.0))
}

/// Brightness multiplier for a shade step, for a caller that darkens a colour.
///
/// `1.0` at step 0, and never below the cap's value, so a deep stack stays a
/// recognisable colour instead of turning into holes in the lane.
#[must_use]
pub fn shade_multiplier(steps: usize) -> f32 {
    let steps = steps.min(LYRIC_STACK_MAX_SHADE_STEPS);
    (1.0 - LYRIC_STACK_SHADE_PER_STEP).powi(steps as i32)
}

/// Whether a document has any cue whose text a frame will never show, for the
/// panel's report line. Not a stacking question, but the same walk answers it
/// and the caller already has the document.
#[must_use]
pub fn potential_cue_count(document: &LyricsDocument) -> usize {
    document
        .cues()
        .iter()
        .filter(|cue| cue.origin == CueOrigin::Potential)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(spans: &[(f64, f64)]) -> LyricStack {
        LyricStack::from_spans(spans)
    }

    #[test]
    fn a_document_with_no_overlap_stays_one_flat_row() {
        let stack = stack(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)]);
        assert!(stack.is_flat());
        for index in 0..3 {
            assert_eq!(stack.slot(index), LyricStackSlot::default());
        }
        // Touching ends are not an overlap: `at_time` hands the second cue back
        // at exactly 1.0, so nothing is being covered.
        assert_eq!(stack.rows(), 1);
    }

    #[test]
    fn two_overlapping_cues_do_not_fan_out() {
        // The operator's threshold. A line whose tail laps into the next one is
        // the common case and must keep looking the way it looks today.
        let stack = stack(&[(0.0, 2.0), (1.0, 3.0)]);
        assert!(stack.is_flat());
        assert_eq!(stack.slot(0).row, 0);
        assert_eq!(stack.slot(1).row, 0);
    }

    #[test]
    fn three_live_cues_fan_out_with_the_displayed_one_on_top() {
        // All three cover 2.5s. `at_time` returns the last in canonical order,
        // so index 2 is the line that reaches a frame and must be row 0.
        let stack = stack(&[(0.0, 3.0), (1.0, 4.0), (2.0, 5.0)]);
        assert_eq!(stack.rows(), 3);
        assert_eq!(stack.slot(2).row, 0, "the cue that displays is on top");
        assert_eq!(stack.slot(1).row, 1);
        assert_eq!(stack.slot(0).row, 2);
        assert_eq!(stack.slot(0).shade_steps, 2);
    }

    #[test]
    fn a_cluster_that_fans_out_does_not_disturb_its_neighbours() {
        // Rule 1. The pile at 0-5 needs three rows; the two cues at 20+ do not
        // touch it and must not be dragged into its geometry.
        let stack = stack(&[
            (0.0, 3.0),
            (1.0, 4.0),
            (2.0, 5.0),
            (20.0, 21.0),
            (21.0, 22.0),
        ]);
        assert_eq!(stack.rows(), 3);
        assert_eq!(stack.slot(3).row, 0);
        assert_eq!(stack.slot(4).row, 0);
        assert_eq!(stack.slot(3).shade_steps, 0);
    }

    #[test]
    fn a_row_is_reused_once_its_occupant_has_ended() {
        // Greedy colouring's whole point: four cues, but never more than three
        // live, so three rows carry all of them. Without reuse this would need
        // four and every block would be a quarter of the lane.
        let stack = stack(&[(0.0, 3.0), (1.0, 4.0), (2.0, 5.0), (4.5, 6.0)]);
        assert_eq!(stack.rows(), 3);
        // Walking backwards, the last cue takes row 0 and the one it overlaps
        // (2.0-5.0) is pushed down.
        assert_eq!(stack.slot(3).row, 0);
        assert!(stack.slot(2).row > 0);
    }

    #[test]
    fn shade_steps_stop_before_a_block_stops_reading_as_a_colour() {
        let deep: Vec<(f64, f64)> = (0..8).map(|index| (f64::from(index) * 0.1, 10.0)).collect();
        let stack = stack(&deep);
        assert_eq!(stack.rows(), 8);
        let deepest = stack.slot(0);
        assert_eq!(deepest.row, 7);
        assert_eq!(deepest.shade_steps, LYRIC_STACK_MAX_SHADE_STEPS);
        // And the multiplier is bounded with it, rather than reaching 0.
        assert!(shade_multiplier(deepest.shade_steps) > 0.4);
        assert_eq!(shade_multiplier(0), 1.0);
        assert_eq!(
            shade_multiplier(99),
            shade_multiplier(LYRIC_STACK_MAX_SHADE_STEPS)
        );
    }

    #[test]
    fn row_geometry_tiles_the_lane_and_keeps_every_block_grabbable() {
        // Sweep the heights a resizable lane can produce against every row count
        // it can ask for. The invariant is that no block leaves the lane and none
        // collapses to nothing -- a block with no pixels is a cue that cannot be
        // clicked, which is the failure the fan-out is supposed to prevent.
        let mut height = 16.0f32;
        while height <= 80.0 {
            for rows in 1..=8usize {
                for row in 0..rows {
                    let (offset, block) = row_geometry(height, rows, row);
                    assert!(offset >= 0.0, "h={height} rows={rows} row={row}");
                    assert!(block > 0.0, "h={height} rows={rows} row={row}");
                    assert!(
                        offset + block <= height + 0.01,
                        "a stacked block left the lane: h={height} rows={rows} row={row}"
                    );
                }
                // Bottom-aligned: every row ends on the lane's bottom edge.
                let (offset, block) = row_geometry(height, rows, rows - 1);
                assert!((offset + block - height).abs() <= 0.01 || rows == 1);
            }
            height += 1.0;
        }
        // A single row is the whole lane, which is what keeps the flat case
        // byte-identical to what the lane drew before this module existed.
        assert_eq!(row_geometry(33.0, 1, 0), (0.0, 33.0));
    }

    #[test]
    fn degenerate_geometry_is_refused_rather_than_producing_a_negative_block() {
        assert_eq!(row_geometry(f32::NAN, 3, 1), (0.0, 0.0));
        assert_eq!(row_geometry(-4.0, 3, 1), (0.0, 0.0));
        assert_eq!(row_geometry(0.0, 3, 1), (0.0, 0.0));
    }

    #[test]
    fn a_zero_length_document_and_a_single_cue_both_stay_flat() {
        assert!(stack(&[]).is_flat());
        assert!(stack(&[(0.0, 1.0)]).is_flat());
        assert_eq!(stack(&[]).slot(0), LyricStackSlot::default());
    }

    #[test]
    fn identical_spans_stack_rather_than_hiding_each_other() {
        // The exact case the operator's assist run produces: several proposals
        // parked on the same coarse window. Drawn flat they are one block and
        // only the last is reachable.
        let stack = stack(&[(10.0, 12.0), (10.0, 12.0), (10.0, 12.0)]);
        assert_eq!(stack.rows(), 3);
        let mut rows: Vec<usize> = (0..3).map(|index| stack.slot(index).row).collect();
        rows.sort_unstable();
        assert_eq!(rows, vec![0, 1, 2], "three identical cues need three rows");
    }
}
