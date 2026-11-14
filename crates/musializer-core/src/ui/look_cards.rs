//! The Look view's look cards (D, MV9-l and MV3-a): where their parts go,
//! and the scales their sliders move on.
//!
//! A card is a column of blocks — a label, a grid of named chips, a slider
//! row, a line of text — flowed top to bottom. The app builds the block list
//! from the model each frame, asks [`layout`] for the height to reserve in
//! the Look list, and draws into the rects the same call returns, so the
//! height reserved and the rects drawn can never disagree. Nothing here draws.

use crate::project::scene_switch::MAX_GLIDE_SECONDS;
use crate::project::transition::{MAX_SECONDS, MIN_SECONDS};
use crate::ui::workspace_layout::UiRect;

/// Inside the card's frame, all round.
pub const CARD_PADDING: f32 = 10.0;
/// A chip in a grid of named choices.
pub const CHIP_HEIGHT: f32 = 26.0;
/// Between two chips, either way.
pub const CHIP_GAP: f32 = 4.0;
/// The narrowest a chip gets before the grid drops a column: wide enough
/// for "Glitch" or "Night door" at the caption size with its padding.
pub const MIN_CHIP_WIDTH: f32 = 60.0;
/// A slider row, as `widgets::SLIDER_ROW_HEIGHT` draws it.
pub const SLIDER_HEIGHT: f32 = 30.0;
/// A micro label heading a group (`ARRIVES BY`), with its trailing value.
pub const LABEL_HEIGHT: f32 = 18.0;
/// Under a label, before what it heads.
pub const LABEL_GAP: f32 = 4.0;
/// Between two groups inside a card.
pub const GROUP_GAP: f32 = 12.0;
/// A line of caption-size text.
pub const LINE_HEIGHT: f32 = 16.0;
/// A button row (the card's title with Play the cut, a scope switch).
pub const BUTTON_HEIGHT: f32 = 26.0;
/// A text field row.
pub const FIELD_HEIGHT: f32 = 28.0;

/// One block of a card, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Block {
    /// A micro label row.
    Label,
    /// A grid of `n` equal chips, as many columns as fit.
    Grid(usize),
    /// A grid of `n` chips in exactly `columns` columns.
    Columns(usize, usize),
    Slider,
    /// A line of caption text.
    Line,
    /// `n` lines of caption text.
    Lines(usize),
    /// A row of buttons, 26 px.
    Buttons,
    /// A text field row.
    Field,
    /// Empty space.
    Gap(f32),
}

/// Where one block landed. A grid's chips are in [`Placed::chips`]; every
/// other block has none.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub block: Block,
    pub rect: UiRect,
    pub chips: Vec<UiRect>,
}

/// How many columns a grid of `count` chips gets in `width`.
#[must_use]
pub fn grid_columns(width: f32, count: usize) -> usize {
    if count == 0 {
        return 1;
    }
    let fit = ((width + CHIP_GAP) / (MIN_CHIP_WIDTH + CHIP_GAP)).floor() as usize;
    fit.clamp(1, count)
}

fn grid(x: f32, y: f32, width: f32, count: usize, columns: usize) -> (Vec<UiRect>, f32) {
    let columns = columns.max(1);
    let rows = count.div_ceil(columns);
    let chip = ((width - CHIP_GAP * (columns as f32 - 1.0)) / columns as f32).max(0.0);
    let mut rects = Vec::with_capacity(count);
    for index in 0..count {
        let (row, column) = (index / columns, index % columns);
        // Whole-pixel edges, the last column flush with the right edge, so a
        // neighbour's hairline never splits across two pixels.
        let left = (x + column as f32 * (chip + CHIP_GAP)).round();
        let right = if column + 1 == columns {
            x + width
        } else {
            (x + column as f32 * (chip + CHIP_GAP) + chip).round()
        };
        rects.push(UiRect::new(
            left,
            y + row as f32 * (CHIP_HEIGHT + CHIP_GAP),
            (right - left).max(0.0),
            CHIP_HEIGHT,
        ));
    }
    let height = if rows == 0 {
        0.0
    } else {
        rows as f32 * CHIP_HEIGHT + (rows as f32 - 1.0) * CHIP_GAP
    };
    (rects, height)
}

/// The blocks flowed from `(x, y)` down a column `width` wide. Returns each
/// block's place and the total height.
#[must_use]
pub fn layout(x: f32, y: f32, width: f32, blocks: &[Block]) -> (Vec<Placed>, f32) {
    let width = width.max(0.0);
    let mut cursor = y;
    let mut placed = Vec::with_capacity(blocks.len());
    for &block in blocks {
        let (height, chips) = match block {
            Block::Label => (LABEL_HEIGHT + LABEL_GAP, Vec::new()),
            Block::Grid(count) => {
                let (chips, height) = grid(x, cursor, width, count, grid_columns(width, count));
                (height, chips)
            }
            Block::Columns(count, columns) => {
                let (chips, height) = grid(x, cursor, width, count, columns);
                (height, chips)
            }
            Block::Slider => (SLIDER_HEIGHT, Vec::new()),
            Block::Line => (LINE_HEIGHT, Vec::new()),
            Block::Lines(count) => (LINE_HEIGHT * count as f32, Vec::new()),
            Block::Buttons => (BUTTON_HEIGHT, Vec::new()),
            Block::Field => (FIELD_HEIGHT, Vec::new()),
            Block::Gap(gap) => (gap.max(0.0), Vec::new()),
        };
        let rect_height = if block == Block::Label {
            LABEL_HEIGHT
        } else {
            height
        };
        placed.push(Placed {
            block,
            rect: UiRect::new(x, cursor, width, rect_height),
            chips,
        });
        cursor += height;
    }
    (placed, cursor - y)
}

/// A card's frame around blocks of `content` height.
#[must_use]
pub fn card_height(content: f32) -> f32 {
    content + CARD_PADDING * 2.0
}

// -- the scales -------------------------------------------------------------

/// The transition length's track: logarithmic from [`MIN_SECONDS`] to
/// [`MAX_SECONDS`], because nearly every transition worth making is under two
/// seconds and a linear 30-second track would give that range a fifteenth of
/// it. 0.6 s, the default, sits at about two fifths.
#[must_use]
pub fn transition_unit(seconds: f32) -> f32 {
    let span = (MAX_SECONDS / MIN_SECONDS).ln();
    ((seconds.clamp(MIN_SECONDS, MAX_SECONDS) / MIN_SECONDS).ln() / span).clamp(0.0, 1.0)
}

/// The length at `unit` along [`transition_unit`]'s track, to the hundredth
/// of a second the readout prints.
#[must_use]
pub fn transition_seconds(unit: f32) -> f32 {
    let seconds = MIN_SECONDS * (MAX_SECONDS / MIN_SECONDS).powf(unit.clamp(0.0, 1.0));
    ((seconds * 100.0).round() / 100.0).clamp(MIN_SECONDS, MAX_SECONDS)
}

/// The glide's track: quadratic from 0 to [`MAX_GLIDE_SECONDS`], so the left
/// end is exactly none and the treatment's usual 0.8 to 3 s sit in the first
/// third rather than the first tenth.
#[must_use]
pub fn glide_unit(seconds: f32) -> f32 {
    (seconds.clamp(0.0, MAX_GLIDE_SECONDS) / MAX_GLIDE_SECONDS).sqrt()
}

/// The glide at `unit`, to the twentieth of a second.
#[must_use]
pub fn glide_seconds(unit: f32) -> f32 {
    let unit = unit.clamp(0.0, 1.0);
    let seconds = MAX_GLIDE_SECONDS * unit * unit;
    ((seconds * 20.0).round() / 20.0).clamp(0.0, MAX_GLIDE_SECONDS)
}

/// A typed length: seconds as a number, with or without a trailing `s`.
/// `None` for anything else; bounds are the caller's.
#[must_use]
pub fn parse_seconds(text: &str) -> Option<f32> {
    let text = text.trim();
    let text = text.strip_suffix('s').unwrap_or(text).trim();
    let value: f32 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

/// Where play-the-cut starts: this long before the picture starts to change.
pub const CUT_LEAD_SECONDS: f64 = 1.5;
/// And how long it keeps playing after the transition has finished.
pub const CUT_TAIL_SECONDS: f64 = 1.0;

/// Play the cut's span for a section starting at `start` that arrives by a
/// transition `seconds` long (`dip` when `half_before`: half of it plays
/// before the change). The first section's is its intro, from the top.
#[must_use]
pub fn cut_span(start: f64, seconds: f64, half_before: bool, first: bool) -> (f64, f64) {
    let seconds = seconds.max(0.0);
    let change = if half_before {
        start - seconds * 0.5
    } else {
        start
    };
    let from = if first {
        0.0
    } else {
        (change - CUT_LEAD_SECONDS).max(0.0)
    };
    let length = if half_before { seconds * 0.5 } else { seconds };
    (from, start + length + CUT_TAIL_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_fills_its_width_with_equal_chips_and_wraps() {
        // 300 px: four 60 px chips and three 4 px gaps fit (252), five do not
        // (316), so fourteen kinds take four rows of four.
        assert_eq!(grid_columns(300.0, 14), 4);
        assert_eq!(grid_columns(432.0, 14), 6);
        assert_eq!(grid_columns(40.0, 14), 1);
        assert_eq!(grid_columns(900.0, 3), 3);
        let (placed, height) = layout(10.0, 100.0, 300.0, &[Block::Grid(14)]);
        assert_eq!(placed[0].chips.len(), 14);
        assert_eq!(placed[0].chips[0], UiRect::new(10.0, 100.0, 72.0, 26.0));
        assert_eq!(placed[0].chips[3].x + placed[0].chips[3].width, 310.0);
        assert_eq!(placed[0].chips[4], UiRect::new(10.0, 130.0, 72.0, 26.0));
        assert_eq!(height, 4.0 * 26.0 + 3.0 * 4.0);
        // No two chips overlap.
        for (i, a) in placed[0].chips.iter().enumerate() {
            for b in &placed[0].chips[i + 1..] {
                assert!(
                    a.x + a.width <= b.x
                        || b.x + b.width <= a.x
                        || a.y + a.height <= b.y
                        || b.y + b.height <= a.y,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn blocks_flow_down_and_the_height_is_their_sum() {
        let blocks = [
            Block::Buttons,
            Block::Gap(12.0),
            Block::Label,
            Block::Slider,
            Block::Line,
            Block::Columns(3, 3),
        ];
        let (placed, height) = layout(0.0, 50.0, 200.0, &blocks);
        assert_eq!(placed[0].rect, UiRect::new(0.0, 50.0, 200.0, 26.0));
        assert_eq!(placed[2].rect, UiRect::new(0.0, 88.0, 200.0, 18.0));
        assert_eq!(placed[3].rect.y, 88.0 + 22.0);
        assert_eq!(placed[5].chips.len(), 3);
        assert_eq!(height, 26.0 + 12.0 + 22.0 + 30.0 + 16.0 + 26.0);
    }

    #[test]
    fn the_transition_track_is_logarithmic_and_round_trips_its_readout() {
        assert_eq!(transition_unit(MIN_SECONDS), 0.0);
        assert_eq!(transition_unit(MAX_SECONDS), 1.0);
        assert_eq!(transition_seconds(0.0), 0.05);
        assert_eq!(transition_seconds(1.0), 30.0);
        // The default sits near two fifths, and every readout a drag can
        // produce survives a trip through the track.
        assert!((transition_unit(0.6) - 0.388).abs() < 0.001);
        for hundredths in [5, 10, 45, 60, 100, 160, 350, 1000, 3000] {
            let seconds = hundredths as f32 / 100.0;
            let back = transition_seconds(transition_unit(seconds));
            assert!((back - seconds).abs() < 0.006, "{seconds} -> {back}");
        }
    }

    #[test]
    fn the_glide_track_starts_at_none_and_reaches_thirty() {
        assert_eq!(glide_seconds(0.0), 0.0);
        assert_eq!(glide_seconds(1.0), 30.0);
        assert_eq!(glide_unit(0.0), 0.0);
        assert_eq!(glide_seconds(glide_unit(2.6)), 2.6);
        assert_eq!(glide_seconds(0.5), 7.5);
    }

    #[test]
    fn typed_seconds_take_an_optional_unit() {
        assert_eq!(parse_seconds("0.45"), Some(0.45));
        assert_eq!(parse_seconds(" 2 s "), Some(2.0));
        assert_eq!(parse_seconds("2s"), Some(2.0));
        assert_eq!(parse_seconds("fast"), None);
        assert_eq!(parse_seconds("nan"), None);
    }

    #[test]
    fn play_the_cut_starts_before_the_change_and_ends_after_it() {
        let (from, to) = cut_span(72.0, 0.6, false, false);
        assert_eq!(from, 70.5);
        assert!((to - 73.6).abs() < 1e-9);
        // A dip starts half its length early.
        assert_eq!(cut_span(72.0, 1.0, true, false), (70.0, 73.5));
        // The intro plays from the top.
        assert_eq!(cut_span(0.0, 3.5, false, true), (0.0, 4.5));
        assert_eq!(cut_span(1.0, 0.6, false, false).0, 0.0);
    }
}
