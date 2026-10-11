//! Cadence's word timing, raylib-free.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_cadence_timing.c` and
//! `.h`. One of the three modules the C project extracted so it could be tested
//! without a window; keep it pure. Its C tests are ported at the bottom of this
//! file from `../musializer/tests/test_scene_cadence_timing.c`.
//!
//! **These are estimates over line-level cues, not measured word timestamps.**
//! Cadence reads the timed-lyric lane, and that lane times *lines*; splitting a
//! line's span by glyph count is a derived guess. Keeping that distinction is why
//! the lyric lane stays separate from the measured-audio lane.
//!
//! `../musializer/cadence-overhauls-2026-07-26.md` proposes changing all of this.
//! It is an unimplemented scratchpad and is deliberately **not** ported.

/// The line loosens back into particles over the final 1/9 of the cue
/// (`scene_cadence_timing.h:12`).
pub const LINE_DISSOLVE_SPAN: f32 = 9.0;

/// Above this hold a word is drawn as settled type; below it, as a swarm
/// (`scene_cadence_timing.h:16`).
pub const HOLD_LEGIBLE: f32 = 0.5;

/// `cadence_timing_clamp01` (`scene_cadence_timing.c:5-10`).
///
/// Written as `!(value > 0.0)` in the C specifically so NaN lands on 0 rather
/// than propagating into a focus value; reproduced with the same shape.
// The negated comparison *is* the NaN filter, not a stylistic slip: `value <= 0.0`
// is false for NaN and would let it through.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn clamp01(value: f32) -> f32 {
    if !(value > 0.0) {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// `cadence_timing_smoothstep` (`:12-16`).
#[must_use]
pub fn smoothstep(value: f32) -> f32 {
    let value = clamp01(value);
    value * value * (3.0 - 2.0 * value)
}

/// Dissolve factor for the line as a whole: 1 for most of the cue, falling to 0
/// across its final beats (`:18-21`).
#[must_use]
pub fn line_hold(cue_position: f32) -> f32 {
    smoothstep((1.0 - cue_position) * LINE_DISSOLVE_SPAN)
}

/// Dissolve factor for one word (`:23-29`).
///
/// A word keeps its own moment, then joins the line's exit continuously.
/// Assigning `line_hold` on the first frame after its window used to drop
/// opacity in one step for late words (SX4). The last word remains exempt;
/// the renderer supplies its short absolute-time cue-edge fade.
#[must_use]
pub fn word_hold(cue_position: f32, window_end: f32, line_hold: f32) -> f32 {
    if cue_position < window_end {
        return 1.0;
    }
    let release = smoothstep((cue_position - window_end) / 0.04);
    if release >= 1.0 {
        line_hold
    } else {
        1.0 + (line_hold - 1.0) * release
    }
}

/// Splits `[0, 1]` across `glyph_counts.len()` words in proportion to glyph count
/// plus one, which is what gives longer words longer moments (`:31-50`).
///
/// Returns `None` for an empty request, matching the C's `false`. The `+1` per
/// word is the breath between words, and it is also what keeps a zero-glyph word
/// from collapsing the division.
#[must_use]
pub fn assign_windows(glyph_counts: &[u32]) -> Option<Vec<(f32, f32)>> {
    if glyph_counts.is_empty() {
        return None;
    }
    let mut total: u64 = glyph_counts.iter().map(|&count| u64::from(count) + 1).sum();
    if total == 0 {
        total = 1;
    }
    let mut windows = Vec::with_capacity(glyph_counts.len());
    let mut before: u64 = 0;
    for &count in glyph_counts {
        let weight = u64::from(count) + 1;
        windows.push((
            before as f32 / total as f32,
            (before + weight) as f32 / total as f32,
        ));
        before += weight;
    }
    // The final window closes exactly at the end of the cue by construction.
    let last = windows.len() - 1;
    windows[last].1 = 1.0;
    Some(windows)
}

/// How long one glyph's worth of a line takes to form at the slowest
/// (post-legacy, 2026-10-12).
///
/// The windows above split the *whole cue*, which is right for a line sung
/// across its span and wrong for two words held for six seconds: the second
/// word would not start gathering until three seconds in, and the frame is an
/// empty swarm for most of that. About nine glyphs a second is an unhurried
/// reading pace, so a line never takes longer than that to arrive and then
/// simply stays.
pub const REVEAL_SECONDS_PER_GLYPH: f32 = 0.11;

/// Below this a capped reveal would read as a cut rather than as an arrival.
pub const REVEAL_MIN_SECONDS: f32 = 0.45;

/// The fraction of a cue its words form across: 1 for a line that fills its
/// span, less for a short line on a long cue.
///
/// Uses the same `glyphs + 1` weight as [`assign_windows`], so the cap and the
/// split agree about how long a word is. A cue with no positive duration reads
/// as 1, the same answer `cue_position` gives it.
#[must_use]
pub fn reveal_span(glyph_counts: &[u32], duration_seconds: f64) -> f32 {
    if duration_seconds.is_nan() || duration_seconds <= 0.0 || glyph_counts.is_empty() {
        return 1.0;
    }
    let weight: u64 = glyph_counts.iter().map(|&count| u64::from(count) + 1).sum();
    let seconds = (weight as f32 * REVEAL_SECONDS_PER_GLYPH).max(REVEAL_MIN_SECONDS);
    (f64::from(seconds) / duration_seconds).min(1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_line_on_a_long_cue_arrives_early_and_a_full_line_is_untouched() {
        // "haul away" held for six seconds: both words are up inside 1.1 s.
        let span = reveal_span(&[4, 4], 6.0);
        assert!((span - 1.1 / 6.0).abs() < 1.0e-4, "{span}");
        // Eight words in three seconds already outrun the cap.
        assert_eq!(reveal_span(&[3, 5, 7, 2, 6, 6, 4, 5], 3.0), 1.0);
        // A single short word still takes long enough to be seen arriving.
        let one = reveal_span(&[1], 10.0);
        assert!((one - REVEAL_MIN_SECONDS / 10.0).abs() < 1.0e-5);
        assert_eq!(reveal_span(&[4], 0.0), 1.0);
        assert_eq!(reveal_span(&[], 5.0), 1.0);
    }

    /// The reported case from the C test: eleven ordinary words and a two-letter
    /// closer, so the short final word gets a narrow slice at the very end of the
    /// cue — exactly where the line's dissolve lives.
    const LONG_LINE: [u32; 12] = [5, 4, 7, 6, 5, 3, 8, 4, 6, 5, 7, 2];

    fn long_line_windows() -> Vec<(f32, f32)> {
        assign_windows(&LONG_LINE).expect("a non-empty line always assigns")
    }

    /// Port of `cadence_timing_windows_tile_the_cue_and_close_at_its_end`.
    #[test]
    fn windows_tile_the_cue_and_close_at_its_end() {
        let windows = long_line_windows();
        assert!((windows[0].0 - 0.0).abs() < 0.0001);
        assert_eq!(windows[LONG_LINE.len() - 1].1, 1.0);
        for (i, &(start, end)) in windows.iter().enumerate() {
            assert!(end > start, "word {i} has an empty window");
            if i > 0 {
                assert!(
                    (start - windows[i - 1].1).abs() < 0.0001,
                    "gap before word {i}"
                );
            }
        }
        // Longer words get longer moments; that is the whole point of the weighting.
        let long = windows[6].1 - windows[6].0;
        let short = windows[11].1 - windows[11].0;
        assert!(long > short);
    }

    /// Port of `cadence_timing_line_hold_dissolves_only_at_the_end`.
    #[test]
    fn the_line_hold_dissolves_only_at_the_end() {
        assert!((line_hold(0.0) - 1.0).abs() < 0.0001);
        assert!((line_hold(0.5) - 1.0).abs() < 0.0001);
        // The dissolve begins with 1/9 of the cue left.
        assert!((line_hold(1.0 - 1.0 / LINE_DISSOLVE_SPAN) - 1.0).abs() < 0.0001);
        assert!(line_hold(0.95) < 1.0);
        assert!(line_hold(1.0).abs() < 0.0001);
        // Crosses legibility at 17/18, which is what used to make the final word
        // stop being drawn as settled type.
        assert!(line_hold(17.0 / 18.0 - 0.001) > HOLD_LEGIBLE);
        assert!(line_hold(17.0 / 18.0 + 0.001) < HOLD_LEGIBLE);
        // Degenerate inputs do not produce a NaN hold that would poison focus.
        assert_eq!(line_hold(f32::NAN), 0.0);
        assert!((line_hold(-5.0) - 1.0).abs() < 0.0001);
        assert_eq!(line_hold(5.0), 0.0);
    }

    /// Port of `cadence_timing_keeps_the_final_word_legible_through_its_own_window`.
    #[test]
    fn the_final_word_stays_legible_through_its_own_window() {
        let windows = long_line_windows();
        let (start, end) = windows[LONG_LINE.len() - 1];
        // The reachability condition from the report: this line's final window
        // opens after the dissolve has already started.
        assert!(start > 1.0 - 1.0 / LINE_DISSOLVE_SPAN);

        // Sampling the interior rather than only the endpoints is the point: the
        // old behaviour failed in the middle, not at a boundary.
        for step in 0..=20 {
            let position = start + (end - start) * (step as f32 / 20.0) * 0.999;
            let line = line_hold(position);
            assert_eq!(word_hold(position, end, line), 1.0);
            if position > 17.0 / 18.0 {
                assert!(line < HOLD_LEGIBLE, "the line hold alone would have failed");
            }
        }
    }

    /// Port of `cadence_timing_still_dissolves_words_that_are_already_sung`.
    #[test]
    fn words_already_sung_still_dissolve_with_the_line() {
        let windows = long_line_windows();
        let position = 0.99f32;
        let line = line_hold(position);
        assert!(line < HOLD_LEGIBLE);
        for &(_, end) in &windows[..LONG_LINE.len() - 1] {
            if end + 0.04 <= position {
                assert!((word_hold(position, end, line) - line).abs() < 1.0e-6);
            }
        }
        // A word whose window has not opened yet is likewise exempt, so nothing is
        // dissolved before it has had its moment.
        let last_end = windows[LONG_LINE.len() - 1].1;
        assert_eq!(word_hold(0.10, last_end, line_hold(0.10)), 1.0);
    }

    #[test]
    fn late_words_join_the_dissolve_without_an_opacity_step() {
        let end = 0.95;
        let before = word_hold(end - 0.00001, end, line_hold(end - 0.00001));
        let after = word_hold(end + 0.00001, end, line_hold(end + 0.00001));
        assert!((after - before).abs() < 0.00001);
        assert!(word_hold(0.97, end, line_hold(0.97)) < 1.0);
    }

    /// Port of `cadence_timing_assign_windows_rejects_unusable_input`.
    #[test]
    fn assigning_windows_rejects_unusable_input() {
        assert!(assign_windows(&[]).is_none());

        // A single word owns the entire cue.
        let one = assign_windows(&[3]).unwrap();
        assert_eq!(one[0], (0.0, 1.0));

        // Zero-glyph words still get a slice from the +1 breath weight rather
        // than collapsing the division.
        let empty = assign_windows(&[0, 0]).unwrap();
        assert!((empty[0].1 - 0.5).abs() < 0.0001);
        assert_eq!(empty[1].1, 1.0);
    }

    #[test]
    fn smoothstep_is_clamped_at_both_ends() {
        assert_eq!(smoothstep(-1.0), 0.0);
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert_eq!(smoothstep(2.0), 1.0);
        assert_eq!(smoothstep(f32::NAN), 0.0);
        assert!((smoothstep(0.5) - 0.5).abs() < 1.0e-6);
    }
}
