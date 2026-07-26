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
/// A word that has not finished its own window is exempt from the line's exit.
/// That exemption is a bug fix the oracle already paid for: the last word's window
/// ends at exactly 1.0, so the line dissolve and the final word's own moment
/// overlap by construction, and applying the line hold to it scaled its focus
/// toward zero precisely while it was due to settle into legible type.
#[must_use]
pub fn word_hold(cue_position: f32, window_end: f32, line_hold: f32) -> f32 {
    if cue_position < window_end {
        return 1.0;
    }
    line_hold
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

#[cfg(test)]
mod tests {
    use super::*;

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
            if end <= position {
                assert_eq!(word_hold(position, end, line), line);
            }
        }
        // A word whose window has not opened yet is likewise exempt, so nothing is
        // dissolved before it has had its moment.
        let last_end = windows[LONG_LINE.len() - 1].1;
        assert_eq!(word_hold(0.10, last_end, line_hold(0.10)), 1.0);
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
