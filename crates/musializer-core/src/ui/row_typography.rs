//! The shared font size for a row of buttons, and label ellipsization.
//!
//! **Owner: Agent F.** Port of `ui_row_typography.c/.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! Why this module exists (`ui_row_typography.h:9-13`): button labels used to be
//! fitted one box at a time, each shrinking its own text until it fit, with no
//! floor and no truncation. Neighbouring buttons of equal size therefore rendered
//! at unequal font sizes as soon as one label was longer than the others — the
//! single most visible defect in the workspace. A **shared** row size is the fix:
//! [`font_size`] takes the whole row and returns one size for all of it, floored
//! at [`UI_ROW_MIN_FONT_SIZE`], and anything that still does not fit ellipsizes
//! through [`truncate_label`] instead of shrinking further.
//!
//! Raylib-free on purpose, so the fitting rules are testable headlessly. Where C
//! passed a `Caption_Measure_Text` function pointer plus a `void *user_data`,
//! these take a `measure: impl Fn(&str) -> f32` closure, which carries its own
//! captured state.
//!
//! All arithmetic is `f32`, matching C's `float` and raylib's own widths.

/// Horizontal room reserved inside a button box, summed across both sides
/// (`ui_row_typography.h:18`).
pub const UI_ROW_LABEL_PADDING: f32 = 12.0;

/// A shared row size stops shrinking here; longer labels ellipsize instead
/// (`ui_row_typography.h:22`). Below roughly this size the UI font stops being
/// readable at 100% scale.
pub const UI_ROW_MIN_FONT_SIZE: f32 = 11.0;

/// Bound for a fitted label copy, including C's terminator and the ellipsis
/// (`ui_row_typography.h:25`). Passed to [`truncate_label`] as `capacity`.
pub const UI_ROW_LABEL_CAPACITY: usize = 128;

/// U+2026 HORIZONTAL ELLIPSIS (`ui_row_typography.c:8`). The UI font's curated
/// codepoint set includes it (see `caption_font_codepoints`), so it renders rather
/// than falling back to a box.
pub const ELLIPSIS: char = '\u{2026}';

/// The ellipsis' three UTF-8 bytes plus C's terminator — what `ui_row_typography.c`
/// spells `sizeof(UI_ROW_ELLIPSIS)`.
///
/// A Rust `String` has no terminator, but the byte kept in reserve for it stays
/// counted so a given `capacity` produces byte-identical output to the C.
const ELLIPSIS_BUDGET: usize = 4;

/// Largest index not exceeding `limit` that starts a UTF-8 sequence
/// (`ui_row_typography.c:12-16`).
///
/// C walked back over continuation bytes by hand and read `text[limit]`
/// deliberately, because a limit equal to the length is already a boundary.
/// `str::is_char_boundary` is the same choice of boundary, and agrees at
/// `limit == len`, with the difference that `&str` guarantees the validity the C
/// had to assume.
fn utf8_floor(text: &str, mut limit: usize) -> usize {
    while limit > 0 && !text.is_char_boundary(limit) {
        limit -= 1;
    }
    limit
}

/// Size a button of this height starts from, before any label is considered
/// (`ui_row_typography.c:18-23`).
///
/// Returns 0.0 for a non-finite or non-positive height. Tall boxes cap at 22.0 so
/// a header never dwarfs the panel it sits in.
pub fn base_font_size(box_height: f32) -> f32 {
    if !box_height.is_finite() || box_height <= 0.0 {
        return 0.0;
    }
    let size = box_height * 0.52;
    if size < 22.0 {
        size
    } else {
        22.0
    }
}

/// The largest size not exceeding `base_size` at which every label fits inside
/// its own box, floored at `min_size` (`ui_row_typography.c:25-55`).
///
/// `measure` must report the width of a label at exactly `base_size`, and must be
/// linear in font size — which holds for raylib's `MeasureTextEx` at zero
/// spacing, since the fitted size is derived by scaling one measurement.
///
/// `widths` are full box widths; [`UI_ROW_LABEL_PADDING`] is removed here. Entries
/// with an empty label or a non-finite width are skipped rather than collapsing
/// the whole row, and a measurement that comes back non-finite or non-positive is
/// distrusted the same way. A box with no usable room yields the floor, since no
/// size fits.
///
/// C's separate `count` parameter is gone: the iteration is over the shorter of
/// the two slices, with a `debug_assert` catching a caller whose row is
/// inconsistent. C also allowed a null `labels`, `widths`, or `measure` and
/// returned `base_size`; passing empty slices reaches the same answer.
pub fn font_size(
    labels: &[&str],
    widths: &[f32],
    base_size: f32,
    min_size: f32,
    measure: impl Fn(&str) -> f32,
) -> f32 {
    debug_assert_eq!(
        labels.len(),
        widths.len(),
        "a row's labels and box widths must line up"
    );
    if !base_size.is_finite() || base_size <= 0.0 {
        return 0.0;
    }

    // A caller asking for a floor above the base must not have the text grow.
    let mut floor_size = 0.0f32;
    if min_size.is_finite() && min_size > 0.0 {
        floor_size = if min_size < base_size {
            min_size
        } else {
            base_size
        };
    }

    let mut size = base_size;
    for (label, &width) in labels.iter().zip(widths) {
        // C skipped null and empty labels alike; `&str` has no null, and an empty
        // label is skipped for the same reason — it has nothing to fit.
        if label.is_empty() {
            continue;
        }
        if !width.is_finite() {
            continue;
        }

        let available = width - UI_ROW_LABEL_PADDING;
        if available <= 0.0 {
            // Early return, deliberately unclamped: no size fits a cell narrower
            // than its own padding, so the row goes straight to the floor.
            return floor_size;
        }

        let measured = measure(label);
        if !measured.is_finite() || measured <= 0.0 {
            continue;
        }
        if measured <= available {
            continue;
        }

        let fitted = base_size * (available / measured);
        if fitted < size {
            size = fitted;
        }
    }

    if size < floor_size {
        floor_size
    } else {
        size
    }
}

/// Fits `label` into `available_width`, replacing the tail with [`ELLIPSIS`] when
/// it does not fit the box or `capacity` (`ui_row_typography.c:57-100`).
///
/// Returns the fitted text and whether it was shortened. `measure` must report
/// widths at the size the label will actually be drawn at; pass `None` (as
/// `None::<fn(&str) -> f32>`) for buffer-bound truncation only, which is C's
/// null-measure path — with nothing to measure against, the first prefix that
/// fits `capacity` is accepted.
///
/// Cuts only at UTF-8 sequence boundaries. A non-finite `available_width` is
/// treated as no room at all, and a box too narrow even for the ellipsis still
/// gets the ellipsis: a missing label reads as a bug, a clipped one reads as a
/// tight box.
pub fn truncate_label(
    label: &str,
    available_width: f32,
    measure: Option<impl Fn(&str) -> f32>,
    capacity: usize,
) -> (String, bool) {
    // C's `output == NULL || capacity == 0`: nothing can be written, and that is
    // not a truncation.
    if capacity == 0 {
        return (String::new(), false);
    }
    if label.is_empty() {
        return (String::new(), false);
    }

    let length = label.len();
    let fits_buffer = length < capacity;

    if fits_buffer {
        match &measure {
            None => return (label.to_string(), false),
            Some(measure) => {
                let width = measure(label);
                if width.is_finite() && available_width.is_finite() && width <= available_width {
                    return (label.to_string(), false);
                }
            }
        }
    }

    if capacity < ELLIPSIS_BUDGET {
        // Not even room for the ellipsis. C returned an empty, terminated buffer
        // rather than a partially written one, and still reported a truncation.
        return (String::new(), true);
    }
    let prefix_limit = capacity - ELLIPSIS_BUDGET;
    let mut prefix = utf8_floor(label, length.min(prefix_limit));

    let mut output = String::new();
    loop {
        output.clear();
        output.push_str(&label[..prefix]);
        output.push(ELLIPSIS);
        if prefix == 0 {
            break;
        }
        if let Some(measure) = &measure {
            // A non-finite `available_width` matches neither branch, so the loop
            // keeps shrinking to the bare ellipsis: no room at all.
            if available_width.is_finite() {
                let width = measure(&output);
                if width.is_finite() && width <= available_width {
                    break;
                }
            }
        } else {
            break; // Buffer-bound truncation only; nothing to measure against.
        }
        prefix = utf8_floor(label, prefix - 1);
    }
    (output, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for `MeasureTextEx` at zero spacing, ported from the C tests'
    /// `Row_Measurement`: width is linear in the font size and proportional to the
    /// codepoint count, which is the property [`font_size`] relies on when it
    /// scales a measurement taken at the base size.
    ///
    /// C counted non-continuation bytes; `chars().count()` is the same number for
    /// the valid UTF-8 a `&str` guarantees.
    fn row_measure(font_size: f32, advance: f32, return_nan: bool) -> impl Fn(&str) -> f32 {
        move |text: &str| {
            if return_nan {
                return f32::NAN;
            }
            text.chars().count() as f32 * advance * font_size
        }
    }

    #[track_caller]
    fn expect_near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    #[test]
    fn ui_row_base_font_size_follows_box_height_and_caps() {
        expect_near(base_font_size(36.0), 18.72, 0.001);
        expect_near(base_font_size(30.0), 15.6, 0.001);
        // Tall boxes stop growing so a header never dwarfs the panel it sits in.
        expect_near(base_font_size(200.0), 22.0, 0.001);
        expect_near(base_font_size(0.0), 0.0, 0.001);
        expect_near(base_font_size(-10.0), 0.0, 0.001);
        expect_near(base_font_size(f32::NAN), 0.0, 0.001);
        expect_near(base_font_size(f32::INFINITY), 0.0, 0.001);
    }

    #[test]
    fn ui_row_font_size_keeps_base_when_every_label_fits() {
        let measure = row_measure(18.72, 0.5, false);
        let labels = ["Save", "Load", "Tune"];
        let widths = [200.0, 200.0, 200.0];
        let size = font_size(&labels, &widths, 18.72, UI_ROW_MIN_FONT_SIZE, measure);
        expect_near(size, 18.72, 0.001);
    }

    #[test]
    fn ui_row_font_size_shrinks_the_whole_row_to_its_longest_label() {
        // The reported defect: a track action row of equal 72px cells where only
        // "Open project" overflows. Every cell must end up at one size.
        let measure = row_measure(18.72, 0.4, false);
        let labels = ["Open project", "Save", "Save As", "Add"];
        let widths = [72.0, 72.0, 72.0, 72.0];
        let size = font_size(&labels, &widths, 18.72, UI_ROW_MIN_FONT_SIZE, measure);

        assert!(size < 18.72);
        assert!(size > UI_ROW_MIN_FONT_SIZE);
        // The binding label fits exactly, so nothing narrower had to shrink further.
        let available = 72.0 - UI_ROW_LABEL_PADDING;
        expect_near(
            "Open project".chars().count() as f32 * 0.4 * size,
            available,
            0.01,
        );
    }

    #[test]
    fn ui_row_font_size_stops_at_the_readable_floor() {
        let measure = row_measure(18.72, 0.5, false);
        let labels = ["An unreasonably long control label", "Save"];
        let widths = [72.0, 72.0];
        let size = font_size(&labels, &widths, 18.72, UI_ROW_MIN_FONT_SIZE, measure);
        expect_near(size, UI_ROW_MIN_FONT_SIZE, 0.001);
    }

    #[test]
    fn ui_row_font_size_floor_never_enlarges_a_small_box() {
        // A 20px compact button starts below the floor; the floor must not grow it.
        let measure = row_measure(10.4, 0.5, false);
        let labels = ["Way too long for this cell"];
        let widths = [40.0];
        let size = font_size(&labels, &widths, 10.4, UI_ROW_MIN_FONT_SIZE, measure);
        expect_near(size, 10.4, 0.001);
    }

    #[test]
    fn ui_row_font_size_ignores_unmeasurable_and_absent_labels() {
        let measure = row_measure(18.72, 0.5, false);
        // C's first entry was a null label; `&str` cannot be null, and the empty
        // label beside it takes the same skip.
        let labels = ["", "", "Save"];
        let widths = [72.0, 72.0, 72.0];
        expect_near(
            font_size(&labels, &widths, 18.72, UI_ROW_MIN_FONT_SIZE, &measure),
            18.72,
            0.001,
        );

        // A measurement that cannot be trusted must not collapse the row.
        let broken = row_measure(18.72, 0.5, true);
        let only = ["Open project"];
        let only_widths = [72.0];
        expect_near(
            font_size(&only, &only_widths, 18.72, UI_ROW_MIN_FONT_SIZE, broken),
            18.72,
            0.001,
        );

        // A non-finite width is skipped rather than poisoning the shared size.
        let bad_widths = [f32::NAN];
        expect_near(
            font_size(&only, &bad_widths, 18.72, UI_ROW_MIN_FONT_SIZE, &measure),
            18.72,
            0.001,
        );
    }

    #[test]
    fn ui_row_font_size_rejects_degenerate_inputs() {
        let measure = row_measure(18.72, 0.5, false);
        let labels = ["Save"];
        let widths = [72.0];

        expect_near(
            font_size(&labels, &widths, 0.0, UI_ROW_MIN_FONT_SIZE, &measure),
            0.0,
            0.001,
        );
        expect_near(
            font_size(&labels, &widths, f32::NAN, UI_ROW_MIN_FONT_SIZE, &measure),
            0.0,
            0.001,
        );
        // C's null-`labels`, null-`measure` and `count == 0` cases all returned
        // `base_size`. A required closure cannot be null, and both remaining
        // spellings are the same empty row, which reaches the same answer through
        // the loop instead of an early return.
        expect_near(
            font_size(&[], &[], 18.72, UI_ROW_MIN_FONT_SIZE, &measure),
            18.72,
            0.001,
        );

        // A cell narrower than its own padding has no size that fits.
        let airless = [UI_ROW_LABEL_PADDING];
        expect_near(
            font_size(&labels, &airless, 18.72, UI_ROW_MIN_FONT_SIZE, &measure),
            UI_ROW_MIN_FONT_SIZE,
            0.001,
        );
    }

    #[test]
    fn ui_row_truncate_label_leaves_a_fitting_label_alone() {
        let measure = row_measure(12.0, 0.5, false);
        let (output, shortened) =
            truncate_label("Save", 100.0, Some(&measure), UI_ROW_LABEL_CAPACITY);
        assert!(!shortened);
        assert_eq!(output, "Save");
    }

    #[test]
    fn ui_row_truncate_label_ellipsizes_to_the_available_width() {
        let measure = row_measure(12.0, 0.5, false);
        // Six codepoints of room at 6px each.
        let (output, shortened) = truncate_label(
            "Spectral Terrarium",
            36.0,
            Some(&measure),
            UI_ROW_LABEL_CAPACITY,
        );
        assert!(shortened);
        assert!(measure(&output) <= 36.0);
        assert_eq!(output, "Spect\u{2026}");
    }

    #[test]
    fn ui_row_truncate_label_cuts_only_at_utf8_boundaries() {
        let measure = row_measure(12.0, 0.5, false);
        // Three-byte codepoints; a naive byte cut would leave a continuation byte.
        let label = "漢字漢字";
        // C stepped `width += 3.0f` from 3.0 to 30.0; every value is exact in
        // binary, so the multiplication produces the same ten widths.
        for step in 1..=10 {
            let width = 3.0 * step as f32;
            let (output, shortened) =
                truncate_label(label, width, Some(&measure), UI_ROW_LABEL_CAPACITY);
            // C asserted the output was valid UTF-8, which a `String` guarantees.
            // The assertion with teeth is that the kept part is a whole-codepoint
            // prefix of the label.
            let kept = output.strip_suffix(ELLIPSIS).unwrap_or(output.as_str());
            assert!(
                label.starts_with(kept),
                "{output:?} is not a prefix of {label:?} plus the ellipsis"
            );
            // Four codepoints at 6px each: everything below 24px has to give.
            assert_eq!(shortened, width < 24.0);
            // The ellipsis alone is 6px, so only the narrowest boxes overhang.
            if width >= 6.0 {
                assert!(measure(&output) <= width);
            }
        }
    }

    #[test]
    fn ui_row_truncate_label_respects_the_output_buffer() {
        let measure = row_measure(12.0, 0.5, false);
        let (output, shortened) = truncate_label("Spectral Terrarium", 1000.0, Some(&measure), 8);
        assert!(shortened);
        // C's capacity counts the terminator, so seven bytes are usable.
        assert!(output.len() < 8);
        assert_eq!(output, "Spec\u{2026}");

        // Too small even for the ellipsis: an empty result, never a partially
        // written one.
        let (output, shortened) = truncate_label("Save", 1000.0, Some(&measure), 3);
        assert!(shortened);
        assert!(output.is_empty());
        // C's null output pointer with capacity 0; here only the capacity remains.
        let (output, shortened) = truncate_label("Save", 1000.0, Some(&measure), 0);
        assert!(!shortened);
        assert!(output.is_empty());
    }

    #[test]
    fn ui_row_truncate_label_marks_a_box_with_no_room() {
        let measure = row_measure(12.0, 0.5, false);

        let (output, shortened) =
            truncate_label("Save", 0.0, Some(&measure), UI_ROW_LABEL_CAPACITY);
        assert!(shortened);
        assert_eq!(output, "\u{2026}");

        let (output, shortened) =
            truncate_label("Save", f32::NAN, Some(&measure), UI_ROW_LABEL_CAPACITY);
        assert!(shortened);
        assert_eq!(output, "\u{2026}");

        let broken = row_measure(12.0, 0.5, true);
        let (output, shortened) =
            truncate_label("Save", 100.0, Some(&broken), UI_ROW_LABEL_CAPACITY);
        assert!(shortened);
        assert_eq!(output, "\u{2026}");

        // C's null label and empty label both produce an empty, unshortened result.
        let (output, shortened) = truncate_label("", 100.0, Some(&measure), UI_ROW_LABEL_CAPACITY);
        assert!(!shortened);
        assert!(output.is_empty());
    }
}
