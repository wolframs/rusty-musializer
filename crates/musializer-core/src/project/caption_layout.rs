//! Caption line breaking, bounded before anything reaches the renderer.
//!
//! **Owner: Agent B.** Port of `../musializer/src/caption_layout.c/.h`.
//!
//! Lyric cues are deliberately bounded here rather than in the renderer, and the
//! layout model is raylib-free, so **preview and export use the same rules**. That
//! is the whole design: a caption typeset against a preview window has to break
//! into the same lines at export resolution, and it can only do that if the wrap
//! decisions are made from measured widths in one place.
//!
//! # For Agent F, who draws this
//!
//! The output shape is deliberately small: [`CaptionLayout`] is up to three
//! [`CaptionLine`]s, each carrying its own text, its measured `width`, and a
//! `centered_offset` from the left edge of a `max_width` box. There is nothing to
//! interpret and no state to carry between frames. Widths are in whatever unit the
//! `measure` callback returned, which for the renderer means pixels at the current
//! frame size — the *fractions* live in
//! [`crate::project::model::CaptionStyle`] and are converted to a `max_width`
//! before this is called.

/// Longest source text, in **bytes** (`caption_layout.h:9`, capacity 512 minus its
/// NUL).
pub const SOURCE_MAX_BYTES: usize = 511;
/// Hard ceiling on caption lines (`caption_layout.h:10`).
///
/// Three, and the third always ends in a visible ellipsis when content is lost.
pub const MAX_LINES: usize = 3;
/// Longest laid-out line, in bytes: the source plus room for the ellipsis
/// (`caption_layout.h:11`).
pub const LINE_MAX_BYTES: usize = SOURCE_MAX_BYTES + 4;
/// Hard ceiling on the glyph request passed to raylib's `LoadFontEx`
/// (`caption_layout.h:15`).
///
/// The curated set below is intentionally much smaller than "load all of Unicode",
/// because every requested codepoint costs atlas space whether the face has a glyph
/// for it or not.
pub const FONT_CODEPOINT_LIMIT: usize = 2048;

/// The ellipsis the third line ends with, `U+2026`.
const ELLIPSIS: &str = "\u{2026}";

/// Why a caption could not be laid out (`Caption_Layout_Result`,
/// `caption_layout.h:33-43`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptionLayoutError {
    #[error("caption source text exceeds its fixed capacity")]
    SourceTooLong,
    #[error("caption source text is not valid UTF-8")]
    InvalidUtf8,
    #[error("caption source text contains a control character")]
    InvalidText,
    #[error("caption source text is empty once whitespace is collapsed")]
    Empty,
    #[error("caption width is not a finite positive measurement")]
    Width,
    #[error("the measurement callback returned a value that is not usable")]
    Measurement,
    #[error("the caption box is too narrow for any content")]
    TooNarrow,
}

/// One laid-out line (`Caption_Layout_Line`, `caption_layout.h:19-25`).
///
/// C also stores `byte_length`; `text.len()` is the same number, so it is not
/// duplicated here.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionLine {
    pub text: String,
    /// As returned by the measurement callback.
    pub width: f32,
    /// Offset from the left edge of a `max_width` caption box, never negative.
    pub centered_offset: f32,
}

/// A laid-out caption (`Caption_Layout`, `caption_layout.h:27-31`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptionLayout {
    pub lines: Vec<CaptionLine>,
    /// True when content was dropped and the last line ends in an ellipsis.
    pub ellipsized: bool,
}

/// Unicode whitespace, as `codepoint_is_space` defines it
/// (`caption_layout.c:85-96`).
///
/// Deliberately a fixed list rather than a property lookup: it has to give the same
/// answer in the C build and this one, and a table that grows with a Unicode
/// version would rewrap existing captions.
#[must_use]
pub fn codepoint_is_space(codepoint: u32) -> bool {
    matches!(
        codepoint,
        0x0009 | 0x000A | 0x000B | 0x000C | 0x000D | 0x0020 | 0x0085 | 0x00A0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000
    )
}

/// `normalize_text` (`caption_layout.c:98-135`): trims and collapses whitespace.
///
/// Runs of any whitespace become one `U+0020`, leading whitespace is dropped, and a
/// trailing run is dropped by never being emitted. A control character that is not
/// whitespace rejects the whole caption rather than being stripped — the same rule
/// the lyric model applies, for the same reason: what renders should be what the
/// author can see.
fn normalize_text(text: &str) -> Result<String, CaptionLayoutError> {
    if text.len() > SOURCE_MAX_BYTES {
        return Err(CaptionLayoutError::SourceTooLong);
    }
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        let codepoint = u32::from(character);
        let is_space = codepoint_is_space(codepoint);
        if (codepoint < 0x20 || (0x7F..=0x9F).contains(&codepoint)) && !is_space {
            return Err(CaptionLayoutError::InvalidText);
        }
        if is_space {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
            }
            out.push(character);
            pending_space = false;
        }
    }
    if out.is_empty() {
        return Err(CaptionLayoutError::Empty);
    }
    Ok(out)
}

/// `measure_bytes` (`caption_layout.c:137-153`).
///
/// A non-finite or negative measurement is an error rather than a zero, because a
/// zero would silently fit and produce a line nobody can read.
fn measure(text: &str, measure: &mut dyn FnMut(&str) -> f32) -> Result<f32, CaptionLayoutError> {
    if text.len() >= LINE_MAX_BYTES {
        return Err(CaptionLayoutError::SourceTooLong);
    }
    let measured = measure(text);
    if !measured.is_finite() || measured < 0.0 {
        return Err(CaptionLayoutError::Measurement);
    }
    Ok(measured)
}

/// `measure_with_ellipsis` (`caption_layout.c:155-172`).
fn measure_with_ellipsis(
    text: &str,
    measure_text: &mut dyn FnMut(&str) -> f32,
) -> Result<f32, CaptionLayoutError> {
    if text.len() + ELLIPSIS.len() + 1 > LINE_MAX_BYTES {
        return Err(CaptionLayoutError::SourceTooLong);
    }
    let mut candidate = String::with_capacity(text.len() + ELLIPSIS.len());
    candidate.push_str(text);
    candidate.push_str(ELLIPSIS);
    let measured = measure_text(&candidate);
    if !measured.is_finite() || measured < 0.0 {
        return Err(CaptionLayoutError::Measurement);
    }
    Ok(measured)
}

/// `store_line` (`caption_layout.c:174-184`).
fn store_line(layout: &mut CaptionLayout, text: &str, width: f32, max_width: f32) {
    layout.lines.push(CaptionLine {
        text: text.to_owned(),
        width,
        centered_offset: ((max_width - width) * 0.5).max(0.0),
    });
}

/// The best prefix of `remaining` that fits, as a byte offset.
///
/// `word` is the offset of the last space that fits, `any` the offset of the last
/// codepoint boundary that fits, and `word_consumed` includes the space itself so
/// the caller can skip it. A word break is preferred whenever one exists; a
/// mid-word break only happens for a word too long for an empty line.
struct BestFit {
    word: usize,
    word_width: f32,
    word_consumed: usize,
    any: usize,
    any_width: f32,
}

/// `caption_layout_utf8` (`caption_layout.c:202-331`).
///
/// Greedy wrapping at word boundaries, splitting a word only at a UTF-8 codepoint
/// boundary and only when it cannot fit on an empty line. If content exceeds three
/// lines, the third always ends in a visible `U+2026` and
/// [`CaptionLayout::ellipsized`] is true — a caption that quietly lost its last
/// clause is worse than one that says so.
///
/// The output is produced atomically: on any error nothing is returned, so a caller
/// holding a previous layout keeps it.
pub fn layout_utf8(
    text: &str,
    max_width: f32,
    measure_text: &mut dyn FnMut(&str) -> f32,
) -> Result<CaptionLayout, CaptionLayoutError> {
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err(CaptionLayoutError::Width);
    }
    let normalized = normalize_text(text)?;

    let mut layout = CaptionLayout::default();
    let mut position = 0usize;
    while position < normalized.len() && layout.lines.len() < MAX_LINES {
        let remaining = &normalized[position..];
        let full_width = measure(remaining, measure_text)?;
        if full_width <= max_width {
            store_line(&mut layout, remaining, full_width, max_width);
            position = normalized.len();
            break;
        }

        let last_line = layout.lines.len() + 1 == MAX_LINES;
        if last_line {
            // The ellipsis alone must fit, or there is no honest way to show that
            // content was dropped.
            let ellipsis_width = measure_with_ellipsis("", measure_text)?;
            if ellipsis_width > max_width {
                return Err(CaptionLayoutError::TooNarrow);
            }
            let best = best_fit(remaining, max_width, ellipsis_width, true, measure_text)?;
            let (prefix, width) = if best.word > 0 {
                (best.word, best.word_width)
            } else {
                (best.any, best.any_width)
            };
            let mut line = String::with_capacity(prefix + ELLIPSIS.len());
            line.push_str(&remaining[..prefix]);
            line.push_str(ELLIPSIS);
            store_line(&mut layout, &line, width, max_width);
            layout.ellipsized = true;
            position = normalized.len();
            break;
        }

        let best = best_fit(remaining, max_width, 0.0, false, measure_text)?;
        if best.word > 0 {
            store_line(
                &mut layout,
                &remaining[..best.word],
                best.word_width,
                max_width,
            );
            position += best.word_consumed;
        } else if best.any > 0 {
            store_line(
                &mut layout,
                &remaining[..best.any],
                best.any_width,
                max_width,
            );
            position += best.any;
        } else {
            // Not even one codepoint fits.
            return Err(CaptionLayoutError::TooNarrow);
        }
        while normalized[position..].starts_with(' ') {
            position += 1;
        }
    }

    if position != normalized.len() || layout.lines.is_empty() {
        return Err(CaptionLayoutError::TooNarrow);
    }
    Ok(layout)
}

/// The scan shared by the wrapping and ellipsizing branches
/// (`caption_layout.c:241-272` and `:281-313`, which are the same loop with and
/// without the ellipsis).
fn best_fit(
    remaining: &str,
    max_width: f32,
    empty_width: f32,
    with_ellipsis: bool,
    measure_text: &mut dyn FnMut(&str) -> f32,
) -> Result<BestFit, CaptionLayoutError> {
    let mut best = BestFit {
        word: 0,
        word_width: 0.0,
        word_consumed: 0,
        any: 0,
        any_width: empty_width,
    };
    for (at, character) in remaining.char_indices() {
        let candidate_end = at + character.len_utf8();
        if character == ' ' && at > 0 {
            let width = if with_ellipsis {
                measure_with_ellipsis(&remaining[..at], measure_text)?
            } else {
                measure(&remaining[..at], measure_text)?
            };
            if width <= max_width {
                best.word = at;
                best.word_width = width;
                best.word_consumed = candidate_end;
            }
        } else {
            let width = if with_ellipsis {
                measure_with_ellipsis(&remaining[..candidate_end], measure_text)?
            } else {
                measure(&remaining[..candidate_end], measure_text)?
            };
            if width <= max_width {
                best.any = candidate_end;
                best.any_width = width;
            }
        }
    }
    Ok(best)
}

/// The curated codepoint ranges the caption atlas requests
/// (`caption_codepoint_ranges`, `caption_layout.c:12-41`).
///
/// Latin including decomposed accents, Greek, Cyrillic, punctuation and currency,
/// and a small deliberate symbol set — chosen because the bundled Alegreya actually
/// has glyphs for them, rather than whole blocks that would only produce
/// missing-glyph boxes.
#[rustfmt::skip]
pub const FONT_CODEPOINT_RANGES: &[(u32, u32)] = &[
    // Basic Latin, Latin-1, Latin Extended A/B, and IPA Extensions.
    (0x0020, 0x007E),
    (0x00A0, 0x024F),
    // Decomposed accents; important for canonically equivalent lyric text.
    (0x0300, 0x036F),
    // Greek/Coptic, Cyrillic, and Cyrillic Supplement.
    (0x0370, 0x052F),
    // Latin Extended Additional and Greek Extended.
    (0x1E00, 0x1FFF),
    // General Punctuation and Currency Symbols (includes U+2026, the ellipsis).
    (0x2000, 0x206F),
    (0x20A0, 0x20CF),
    (0x2116, 0x2116), // numero sign
    (0x2122, 0x2122), // trademark
    (0x2190, 0x2199), // common arrows
    (0x2212, 0x2212), // mathematical minus
    (0x221E, 0x221E), // infinity
    (0x2248, 0x2248), // approximately equal
    (0x2260, 0x2260), // not equal
    (0x2264, 0x2265), // less/greater than or equal
    (0x25A0, 0x25A1), // squares
    (0x25B2, 0x25B2), // up triangle
    (0x25B6, 0x25B6), // play triangle
    (0x25BC, 0x25BC), // down triangle
    (0x25C0, 0x25C0), // reverse triangle
    (0x25C6, 0x25C6), // diamond
];

/// `caption_font_codepoint_count` (`caption_layout.c:333-345`).
///
/// `None` when the table is internally inconsistent or would exceed
/// [`FONT_CODEPOINT_LIMIT`], which is a bug in the table rather than a runtime
/// condition — but it is checked, because the alternative is an over-large atlas
/// request at startup.
#[must_use]
pub fn font_codepoint_count() -> Option<usize> {
    let mut count = 0usize;
    for (first, last) in FONT_CODEPOINT_RANGES {
        if last < first {
            return None;
        }
        let span = (last - first) as usize + 1;
        if span > FONT_CODEPOINT_LIMIT - count {
            return None;
        }
        count += span;
    }
    Some(count)
}

/// `caption_font_codepoints` (`caption_layout.c:347-374`): the sorted, deduplicated
/// codepoint list to hand raylib.
///
/// The ranges must be strictly ascending and non-overlapping; that is asserted
/// rather than assumed, because a duplicate codepoint in a `LoadFontEx` request is
/// a silently wasted atlas slot.
#[must_use]
pub fn font_codepoints() -> Option<Vec<u32>> {
    let required = font_codepoint_count()?;
    if required == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(required);
    let mut previous = 0u32;
    for (index, (first, last)) in FONT_CODEPOINT_RANGES.iter().enumerate() {
        if index > 0 && *first <= previous {
            return None;
        }
        out.extend(*first..=*last);
        previous = *last;
    }
    (out.len() == required).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic stand-in for a real font: every character is one unit wide.
    ///
    /// Character counting rather than byte counting is the point — it means the
    /// tests exercise the same wrap decisions for `"→"` as for `"a"`, which is what
    /// a proportional font would not let them isolate.
    fn monospace(text: &str) -> f32 {
        text.chars().count() as f32
    }

    fn lay_out(text: &str, max_width: f32) -> Result<CaptionLayout, CaptionLayoutError> {
        layout_utf8(text, max_width, &mut monospace)
    }

    fn lines(layout: &CaptionLayout) -> Vec<&str> {
        layout.lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn text_that_fits_becomes_one_line() {
        let layout = lay_out("hello world", 20.0).unwrap();
        assert_eq!(lines(&layout), vec!["hello world"]);
        assert!(!layout.ellipsized);
        assert_eq!(layout.lines[0].width, 11.0);
        assert_eq!(layout.lines[0].centered_offset, 4.5);
    }

    #[test]
    fn whitespace_is_collapsed_and_trimmed() {
        let layout = lay_out("  hello \t\n world  ", 40.0).unwrap();
        assert_eq!(lines(&layout), vec!["hello world"]);
        // Every kind of Unicode space collapses, not just ASCII.
        let layout = lay_out("a\u{00a0}\u{2009}\u{3000}b", 40.0).unwrap();
        assert_eq!(lines(&layout), vec!["a b"]);
    }

    #[test]
    fn control_characters_reject_the_whole_caption() {
        for text in ["bell\u{7}", "\u{7f}del", "esc\u{1b}[0m", "null\u{0}"] {
            assert_eq!(
                lay_out(text, 40.0).unwrap_err(),
                CaptionLayoutError::InvalidText,
                "{text:?}"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_only_text_is_refused() {
        assert_eq!(lay_out("", 40.0).unwrap_err(), CaptionLayoutError::Empty);
        assert_eq!(lay_out("   ", 40.0).unwrap_err(), CaptionLayoutError::Empty);
        assert_eq!(
            lay_out("\u{2009}\u{00a0}", 40.0).unwrap_err(),
            CaptionLayoutError::Empty
        );
    }

    #[test]
    fn source_length_is_bounded_in_bytes() {
        let fits = "a".repeat(SOURCE_MAX_BYTES);
        assert!(lay_out(&fits, f32::from(u16::MAX)).is_ok());
        let over = "a".repeat(SOURCE_MAX_BYTES + 1);
        assert_eq!(
            lay_out(&over, 9999.0).unwrap_err(),
            CaptionLayoutError::SourceTooLong
        );
        // And in bytes, not characters: 171 three-byte characters is 513 bytes.
        let multibyte = "→".repeat(171);
        assert_eq!(multibyte.chars().count(), 171);
        assert_eq!(
            lay_out(&multibyte, 9999.0).unwrap_err(),
            CaptionLayoutError::SourceTooLong
        );
    }

    #[test]
    fn a_non_finite_or_non_positive_width_is_refused() {
        for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                lay_out("hello", width).unwrap_err(),
                CaptionLayoutError::Width,
                "{width}"
            );
        }
    }

    #[test]
    fn a_measurement_that_is_not_a_length_is_refused() {
        for value in [f32::NAN, f32::INFINITY, -1.0] {
            let mut broken = |_: &str| value;
            assert_eq!(
                layout_utf8("hello", 10.0, &mut broken).unwrap_err(),
                CaptionLayoutError::Measurement,
                "{value}"
            );
        }
    }

    #[test]
    fn wrapping_prefers_word_boundaries_and_drops_the_space() {
        let layout = lay_out("alpha beta gamma", 11.0).unwrap();
        assert_eq!(lines(&layout), vec!["alpha beta", "gamma"]);
        assert!(!layout.ellipsized);
        // The space between lines is consumed, never rendered as leading space.
        assert!(layout.lines[1].text.starts_with('g'));
    }

    #[test]
    fn a_word_too_long_for_an_empty_line_splits_at_a_codepoint_boundary() {
        let layout = lay_out("→→→→→→", 2.0).unwrap();
        // Two characters per line, and every line is valid UTF-8 — the split
        // never lands inside a multi-byte sequence.
        assert_eq!(lines(&layout), vec!["→→", "→→", "→→"]);
        for line in &layout.lines {
            assert!(line.text.chars().all(|character| character == '→'));
        }
    }

    #[test]
    fn content_past_three_lines_is_ellipsized_visibly() {
        let layout = lay_out("one two three four five six seven eight", 9.0).unwrap();
        assert_eq!(layout.lines.len(), 3);
        assert!(layout.ellipsized, "the caption must say it lost content");
        let last = &layout.lines[2].text;
        assert!(last.ends_with(ELLIPSIS), "{last:?}");
        assert!(layout.lines[2].width <= 9.0);
    }

    #[test]
    fn the_ellipsis_replaces_a_whole_word_where_one_fits() {
        let layout = lay_out("aa bb cc dd ee ff gg hh ii jj", 5.0).unwrap();
        assert!(layout.ellipsized);
        let last = &layout.lines[2].text;
        // A word boundary is preferred, so the ellipsis follows a complete word
        // rather than cutting one in half.
        assert!(last.ends_with(ELLIPSIS));
        assert!(
            !last.trim_end_matches(ELLIPSIS).ends_with(' '),
            "no trailing space before the ellipsis: {last:?}"
        );
    }

    #[test]
    fn a_box_too_narrow_for_the_ellipsis_is_refused() {
        // The ellipsis measures 1 unit here, so a box narrower than that has no
        // honest rendering at all.
        assert_eq!(
            lay_out("aaaa bbbb cccc dddd eeee", 0.5).unwrap_err(),
            CaptionLayoutError::TooNarrow
        );
    }

    #[test]
    fn a_box_that_fits_only_the_ellipsis_yields_just_the_ellipsis() {
        // best_any stays 0 and the ellipsis width is the fallback, which is the C
        // behaviour: one visible character saying content was dropped.
        let layout = lay_out("aaaa bbbb cccc", 1.0).unwrap();
        assert!(layout.ellipsized);
        assert_eq!(layout.lines.last().unwrap().text, ELLIPSIS);
    }

    #[test]
    fn the_centered_offset_is_never_negative() {
        // A line wider than the box would give a negative offset, which would draw
        // the text off the left edge instead of merely overflowing the right.
        let mut wide = |_: &str| 100.0f32;
        let layout = layout_utf8("x", 100.0, &mut wide).unwrap();
        assert_eq!(layout.lines[0].centered_offset, 0.0);
    }

    #[test]
    fn every_line_stays_within_the_box() {
        let text = "the quick brown fox jumps over the lazy dog near the river bank";
        for width in [8.0f32, 12.0, 20.0, 40.0, 63.0] {
            let Ok(layout) = lay_out(text, width) else {
                continue;
            };
            for line in &layout.lines {
                assert!(
                    line.width <= width,
                    "line {:?} is {} wide in a {width} box",
                    line.text,
                    line.width
                );
            }
            assert!(layout.lines.len() <= MAX_LINES);
        }
    }

    #[test]
    fn layout_is_deterministic_so_preview_and_export_agree() {
        let text = "the quick brown fox jumps over the lazy dog";
        let first = lay_out(text, 17.0).unwrap();
        let second = lay_out(text, 17.0).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn the_curated_codepoint_set_is_sorted_bounded_and_free_of_duplicates() {
        let count = font_codepoint_count().expect("the table is consistent");
        assert!(count > 0 && count <= FONT_CODEPOINT_LIMIT);
        let codepoints = font_codepoints().expect("the table produces a list");
        assert_eq!(codepoints.len(), count);
        assert!(
            codepoints.windows(2).all(|pair| pair[0] < pair[1]),
            "strictly ascending, so no atlas slot is wasted on a duplicate"
        );
        // The ellipsis must be requested, or an ellipsized caption renders a box.
        assert!(codepoints.contains(&0x2026));
        // As must the space, or nothing wraps.
        assert!(codepoints.contains(&0x0020));
    }
}
