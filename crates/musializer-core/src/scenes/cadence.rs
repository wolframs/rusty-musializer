//! Cadence: deterministic word splitting, timing, and focus envelopes.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_cadence.c`, with the
//! extracted timing in [`timing`] from `scene_cadence_timing.c/.h`. Layout and
//! drawing need font metrics, so they live in
//! `musializer-app::scenes::cadence`.
//!
//! Cadence is the scene that reads the **timed lyric lane**. That lane times
//! lines; the per-word windows here are a derived estimate over a line's span
//! (`scene_cadence.c:105-107`), never measured word timestamps, and the two must
//! not be confused. The state itself is only a seed: everything Cadence draws is
//! a function of the frame, which is what makes preview and export agree.
//!
//! `../musializer/cadence-overhauls-2026-07-26.md` is an unimplemented scratchpad
//! describing a *different* Cadence. It is deliberately not ported.

pub mod timing;

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneId, SceneState};

/// A full 511-byte cue can hold 256 one-byte words separated by spaces, so the
/// word bound is derived from the lyric contract rather than guessed
/// (`scene_cadence.c:8-11`, `../musializer/src/lyrics.h:12`).
pub const LYRICS_TEXT_CAPACITY: usize = 512;
pub const MAX_WORDS: usize = LYRICS_TEXT_CAPACITY.div_ceil(2);
/// `scene_cadence.c:12-21`
pub const AMBIENT_PARTICLES: usize = 96;
pub const MAX_LAYOUT_ROWS: usize = 24;
pub const LAYOUT_ATTEMPTS: usize = 6;
pub const PARTICLES_PER_GLYPH: usize = 20;
/// Global per-frame particle ceiling, so a pathological cue degrades to plain
/// text instead of unbounded draw submissions.
pub const PARTICLE_BUDGET: usize = 1400;
pub const INK_PROBES: usize = 8;
pub const INK_ALPHA_THRESHOLD: f32 = 96.0;

/// One word of a cue, with its estimated singing window (`scene_cadence.c:24-36`).
///
/// The layout slot the C keeps in the same struct lives in the drawing half
/// instead, because it needs font metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Word<'a> {
    pub text: &'a str,
    pub glyphs: usize,
    /// Estimated singing window, normalized to `[0, 1]` across the cue span.
    pub window_start: f32,
    pub window_end: f32,
}

/// `scene_cadence.c:38-40` — Cadence keeps no history at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CadenceState {
    pub seed: u64,
}

/// `cadence_clamp01` (`:42-47`).
#[must_use]
pub fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `cadence_smooth` (`:49-53`).
#[must_use]
pub fn smooth(value: f32) -> f32 {
    let value = clamp01(value);
    value * value * (3.0 - 2.0 * value)
}

/// `cadence_mix` (`:55-63`).
#[must_use]
pub fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

/// `cadence_unit` (`:65-68`).
///
/// Note this one hashes `seed ^ salt` with **no** golden-ratio offset, unlike the
/// otherwise identical helpers in Constellation, Loom and Pentagram. Adding one
/// here would move every particle in the scene.
#[must_use]
pub fn unit(seed: u64, salt: u64) -> f32 {
    (mix(seed ^ salt) & 0xffff) as f32 / 65535.0
}

/// `cadence_split_words` (`:84-103`).
///
/// Splits on bytes `<= 0x20`, exactly as the C does. That is safe for UTF-8
/// because every continuation byte is `>= 0x80`, so a split never lands inside a
/// codepoint. Stops at [`MAX_WORDS`].
///
/// Glyph counts are codepoint counts. The C walks the string with
/// `GetCodepointNext` and falls back to one byte on a malformed sequence; a Rust
/// `&str` is already valid UTF-8, so `chars().count()` is the same number.
#[must_use]
pub fn split_words(text: &str) -> Vec<Word<'_>> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() && words.len() < MAX_WORDS {
        while at < bytes.len() && bytes[at] <= 0x20 {
            at += 1;
        }
        if at >= bytes.len() {
            break;
        }
        let start = at;
        while at < bytes.len() && bytes[at] > 0x20 {
            at += 1;
        }
        let slice = &text[start..at];
        words.push(Word {
            text: slice,
            glyphs: slice.chars().count(),
            window_start: 0.0,
            window_end: 0.0,
        });
    }
    words
}

/// `cadence_assign_windows` (`:108-120`).
///
/// A cue with more than [`MAX_WORDS`] words leaves every window at zero, which is
/// what the C's early return does. [`split_words`] already stops at the bound, so
/// the guard is unreachable in practice and is kept for shape.
pub fn assign_windows(words: &mut [Word<'_>]) {
    if words.is_empty() || words.len() > MAX_WORDS {
        return;
    }
    let counts: Vec<u32> = words.iter().map(|word| word.glyphs as u32).collect();
    let Some(windows) = timing::assign_windows(&counts) else {
        return;
    };
    for (word, (start, end)) in words.iter_mut().zip(windows) {
        word.window_start = start;
        word.window_end = end;
    }
}

/// Splits a cue and assigns its windows in one step — what a caller almost always
/// wants.
#[must_use]
pub fn words_for_cue(text: &str) -> Vec<Word<'_>> {
    let mut words = split_words(text);
    assign_windows(&mut words);
    words
}

/// Where the playhead sits inside a cue, as `[0, 1]` (`:442-445`).
///
/// A zero-or-negative duration reads as 1.0 — a finished cue — rather than as a
/// division by zero.
#[must_use]
pub fn cue_position(time_seconds: f64, start_seconds: f64, end_seconds: f64) -> f32 {
    let duration = end_seconds - start_seconds;
    if duration > 0.0 {
        ((time_seconds - start_seconds) / duration).clamp(0.0, 1.0) as f32
    } else {
        1.0
    }
}

/// A word's focus envelope and whether it is currently being sung.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// 0 = loose particle cloud hovering at the slot, 1 = settled legible type.
    pub focus: f32,
    pub active: bool,
}

/// A continuous, monotone gathering envelope across the estimated word window.
///
/// Onsets must not assign particle positions: the old one-frame 0.93 floor
/// formed a word on a hit and scattered it again on the next frame. Audio may
/// light the ink, but the cue clock owns its formation (SX4).
#[must_use]
pub fn word_focus(word: &Word<'_>, cue_position: f32, focus_speed: f32, _onset: bool) -> Focus {
    let span = (word.window_end - word.window_start).max(0.0001);
    let lead = span.min(0.12) * 0.22;
    let gather = span / (2.4 + focus_speed.max(0.0) * 1.8);
    Focus {
        focus: smooth((cue_position - word.window_start + lead) / (gather + lead)),
        active: cue_position >= word.window_start && cue_position < word.window_end,
    }
}

/// A periodic beat displacement. Both value and velocity meet at the wrap;
/// multiplying raw phase by an angle instead teleports particles every beat.
#[must_use]
pub fn beat_sway(phase: f32) -> f32 {
    (phase * std::f32::consts::TAU).sin()
}

/// The focus and legibility a word is drawn with, after the line's dissolve is
/// applied (`:455-469`).
///
/// The hold is applied *per word*, not per line — see [`timing::word_hold`] for
/// why that distinction is load-bearing.
#[must_use]
pub fn resolved_focus(
    word: &Word<'_>,
    cue_position: f32,
    focus_speed: f32,
    onset: bool,
    line_hold: f32,
) -> Focus {
    let raw = word_focus(word, cue_position, focus_speed, onset);
    let hold = timing::word_hold(cue_position, word.window_end, line_hold);
    Focus {
        focus: raw.focus * hold,
        active: raw.active && hold > timing::HOLD_LEGIBLE,
    }
}

impl CadenceState {
    /// `cadence_init` (`:237-241`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }
}

impl SceneState for CadenceState {
    fn id(&self) -> SceneId {
        SceneId::Cadence
    }

    // No `update`: the C descriptor sets none (`:473-480`). Cadence is a pure
    // function of the frame plus its seed.

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry entry (`scene_cadence_descriptor`, `:473-480`).
pub const DESCRIPTOR: SceneDescriptor = SceneDescriptor {
    id: SceneId::Cadence,
    state_version: 1,
    make_state: |seed| Box::new(CadenceState::new(seed)),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_handles_runs_of_whitespace_and_control_bytes() {
        let words = split_words("  hello\tworld \n  again ");
        let texts: Vec<&str> = words.iter().map(|word| word.text).collect();
        assert_eq!(texts, vec!["hello", "world", "again"]);
        assert!(split_words("").is_empty());
        assert!(split_words("   \t\n ").is_empty());
    }

    #[test]
    fn glyph_counts_are_codepoints_not_bytes() {
        // The C counts codepoints via GetCodepointNext; a multi-byte word must
        // not be over-weighted in the window split.
        let words = split_words("naïve über");
        assert_eq!(words[0].glyphs, 5, "naïve is five codepoints, six bytes");
        assert_eq!(words[1].glyphs, 4);
        assert_eq!(words[0].text, "naïve");
    }

    #[test]
    fn splitting_stops_at_the_word_bound() {
        let text = "a ".repeat(MAX_WORDS + 40);
        assert_eq!(split_words(&text).len(), MAX_WORDS);
    }

    #[test]
    fn windows_tile_the_cue_after_assignment() {
        let words = words_for_cue("one two three");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].window_start, 0.0);
        assert_eq!(words[2].window_end, 1.0);
        for pair in words.windows(2) {
            assert!((pair[0].window_end - pair[1].window_start).abs() < 1.0e-6);
        }
        // "three" is longest, so it owns the widest slice.
        let widths: Vec<f32> = words
            .iter()
            .map(|word| word.window_end - word.window_start)
            .collect();
        assert!(widths[2] > widths[0]);
    }

    #[test]
    fn cue_position_clamps_and_survives_a_degenerate_cue() {
        assert_eq!(cue_position(5.0, 4.0, 6.0), 0.5);
        assert_eq!(cue_position(1.0, 4.0, 6.0), 0.0, "before the cue");
        assert_eq!(cue_position(9.0, 4.0, 6.0), 1.0, "after the cue");
        assert_eq!(
            cue_position(4.0, 4.0, 4.0),
            1.0,
            "zero-length cue reads as done"
        );
        assert_eq!(
            cue_position(4.0, 6.0, 4.0),
            1.0,
            "inverted cue reads as done"
        );
    }

    #[test]
    fn focus_rises_through_a_word_and_holds_afterwards() {
        let words = words_for_cue("gather round now");
        let word = &words[1];
        let before = word_focus(word, 0.0, 1.0, false);
        assert!(!before.active);
        assert!(before.focus <= 0.14, "faint pre-gather only");

        let during = word_focus(
            word,
            (word.window_start + word.window_end) * 0.5,
            1.0,
            false,
        );
        assert!(during.active);
        assert!(during.focus > before.focus);

        let after = word_focus(word, 1.0, 1.0, false);
        assert!(!after.active, "a finished word is no longer being sung");
        assert_eq!(after.focus, 1.0, "but it holds as settled type");
    }

    #[test]
    fn onset_cannot_teleport_a_gathering_word() {
        let word = &words_for_cue("hit")[0];
        for step in 0..1000 {
            let p = step as f32 / 1000.0;
            assert_eq!(
                word_focus(word, p, 1.0, false),
                word_focus(word, p, 1.0, true)
            );
        }
    }

    #[test]
    fn gathering_has_no_jump_at_word_boundaries() {
        for word in words_for_cue("gather round now") {
            for boundary in [word.window_start, word.window_end] {
                let before = word_focus(&word, boundary - 0.00001, 1.0, false).focus;
                let after = word_focus(&word, boundary + 0.00001, 1.0, false).focus;
                assert!((after - before).abs() < 0.001);
            }
            let mut previous = 0.0;
            for step in 0..1000 {
                let focus = word_focus(&word, step as f32 / 999.0, 1.0, false).focus;
                assert!(focus >= previous);
                previous = focus;
            }
        }
        assert!((beat_sway(1.0 - 0.00001) - beat_sway(0.00001)).abs() < 0.001);
    }

    #[test]
    fn the_final_word_settles_despite_the_line_dissolve() {
        // The regression the C test pins, exercised through the scene-level
        // resolution rather than the timing module alone.
        let words = words_for_cue("the quick brown fox jumped over the lazy dog and then it");
        let last = words.last().unwrap();
        assert!(
            last.window_start > 1.0 - 1.0 / timing::LINE_DISSOLVE_SPAN,
            "the final window must open inside the dissolve for this to be a test"
        );
        let position = (last.window_start + last.window_end) * 0.5;
        let resolved = resolved_focus(last, position, 1.0, false, timing::line_hold(position));
        assert!(resolved.active, "the last word must still read as type");
        assert!(resolved.focus > timing::HOLD_LEGIBLE);
    }

    #[test]
    fn a_word_finished_early_follows_the_line_out() {
        let words = words_for_cue("early word here and more and more and more and more");
        let position = 0.99f32;
        let hold = timing::line_hold(position);
        assert!(hold < timing::HOLD_LEGIBLE);
        let first = &words[0];
        assert!(first.window_end <= position);
        let resolved = resolved_focus(first, position, 1.0, false, hold);
        assert_eq!(resolved.focus, hold, "focus 1.0 scaled by the line's exit");
    }

    #[test]
    fn the_unit_hash_has_no_golden_ratio_offset() {
        // Guards against "harmonizing" this with the other scenes' helpers.
        assert_eq!(unit(0, 0), (mix(0) & 0xffff) as f32 / 65535.0);
    }

    #[test]
    fn the_descriptor_matches_the_c_registry_entry() {
        assert_eq!(DESCRIPTOR.id, SceneId::Cadence);
        assert_eq!(DESCRIPTOR.state_version, 1);
        assert_eq!(MAX_WORDS, 256);
    }
}
