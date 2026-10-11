//! Stage: the lyric line, staged.
//!
//! Post-legacy (2026-10-12), no oracle. Every other scene here draws the
//! *sound*; a caption rides on top and nothing in the picture knows what the
//! line says. Stage draws the **line** — as a poster, a terminal, a chat
//! bubble, a typed card with a stamp, placards, or a ticker — and a treatment
//! picks the staging per section, which can be as short as one line.
//!
//! This half is the arithmetic: which layout a setting names, how a line wraps
//! and how far along each part of it is. It keeps a seed and nothing else, so
//! what Stage draws is a function of the frame, and a still, a clip and a full
//! export agree by construction. Drawing needs font metrics and lives in
//! `musializer-app::scenes::stage`.

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneId, SceneState};

/// How a line is staged (`settings.stage.layout`). The numeric value is the
/// persisted one, so the order is a `.musi` contract: append, never insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Heavy stacked words filling the frame.
    Poster = 0,
    /// A console window typing the line behind a prompt.
    Terminal = 1,
    /// A chat bubble: typing dots, then the line.
    Chat = 2,
    /// A typed card, with the line's last word stamped across it.
    Card = 3,
    /// One placard per word, raised in turn.
    Placards = 4,
    /// A market ticker: the line as a headline over a climbing chart.
    Ticker = 5,
}

impl Layout {
    pub const ALL: [Self; 6] = [
        Self::Poster,
        Self::Terminal,
        Self::Chat,
        Self::Card,
        Self::Placards,
        Self::Ticker,
    ];

    #[must_use]
    pub fn from_setting(value: f32) -> Self {
        let index = if value.is_finite() {
            value.round() as i64
        } else {
            0
        };
        Self::ALL
            .get(usize::try_from(index).unwrap_or(0))
            .copied()
            .unwrap_or(Self::Poster)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Terminal => "terminal",
            Self::Chat => "chat",
            Self::Card => "card",
            Self::Placards => "placards",
            Self::Ticker => "ticker",
        }
    }
}

/// `scene_cadence.c`'s smoothstep, restated here so Stage does not reach into
/// another scene for three lines.
#[must_use]
pub fn smooth(value: f32) -> f32 {
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    value * value * (3.0 - 2.0 * value)
}

/// Greedy wrap of measured words into rows no wider than `max_width`.
///
/// Returns half-open word ranges, one per row. A word wider than the measure
/// gets a row to itself rather than being dropped; an empty input is no rows.
#[must_use]
pub fn wrap_rows(widths: &[f32], space: f32, max_width: f32) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0.0f32;
    for (index, &width) in widths.iter().enumerate() {
        let needed = if index == start {
            width
        } else {
            cursor + space + width
        };
        if index > start && needed > max_width {
            rows.push((start, index));
            start = index;
            cursor = width;
        } else {
            cursor = needed;
        }
    }
    if start < widths.len() {
        rows.push((start, widths.len()));
    }
    rows
}

/// The width of one wrapped row.
#[must_use]
pub fn row_width(widths: &[f32], space: f32, row: (usize, usize)) -> f32 {
    let words = &widths[row.0..row.1];
    words.iter().sum::<f32>() + space * words.len().saturating_sub(1) as f32
}

/// The largest scale at which `widths` (measured at scale 1) wrap into a block
/// no wider than `max_width` and no taller than `max_height`, where a row is
/// `line_height` tall at scale 1. Bisected: wrapping is not monotone in closed
/// form, but "fits" is monotone in scale.
#[must_use]
pub fn fit_scale(
    widths: &[f32],
    space: f32,
    line_height: f32,
    max_width: f32,
    max_height: f32,
    ceiling: f32,
) -> f32 {
    if widths.is_empty() || max_width <= 0.0 || max_height <= 0.0 || line_height <= 0.0 {
        return 0.0;
    }
    let fits = |scale: f32| {
        let scaled: Vec<f32> = widths.iter().map(|width| width * scale).collect();
        let rows = wrap_rows(&scaled, space * scale, max_width);
        rows.len() as f32 * line_height * scale <= max_height
            && rows
                .iter()
                .all(|&row| row_width(&scaled, space * scale, row) <= max_width)
    };
    let (mut low, mut high) = (0.0f32, ceiling.max(0.0));
    if fits(high) {
        return high;
    }
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if fits(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// How many characters of a line a typewriter has put down at `progress`
/// through its reveal, out of `total`. Whole characters only, and all of them
/// at 1, so a typed line never ends one character short.
#[must_use]
pub fn typed_count(total: usize, progress: f32) -> usize {
    if progress >= 1.0 {
        return total;
    }
    ((total as f32) * progress.clamp(0.0, 1.0)).floor() as usize
}

/// One step of the ticker's walk: a deterministic return in `-1..=1` for a
/// candle, from the line's own id. Not audio: a chart that redraws itself
/// every frame is noise, and one that is the same every time the line plays
/// is a prop.
#[must_use]
pub fn ticker_step(seed: u64, line: u64, candle: u32) -> f32 {
    let mut value = seed ^ line.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from(candle) << 17;
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    ((value & 0xffff) as f32 / 65535.0) * 2.0 - 1.0
}

/// Stage keeps a seed and no history.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StageState {
    pub seed: u64,
}

impl StageState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }
}

impl SceneState for StageState {
    fn id(&self) -> SceneId {
        SceneId::Stage
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::Stage,
        state_version: 1,
        make_state: |seed| Box::new(StageState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_are_numbered_in_persisted_order_and_an_unknown_one_is_the_poster() {
        for (index, layout) in Layout::ALL.into_iter().enumerate() {
            assert_eq!(layout as usize, index);
            assert_eq!(Layout::from_setting(index as f32), layout);
            assert!(!layout.name().is_empty());
        }
        assert_eq!(Layout::from_setting(99.0), Layout::Poster);
        assert_eq!(Layout::from_setting(-3.0), Layout::Poster);
        assert_eq!(Layout::from_setting(f32::NAN), Layout::Poster);
    }

    #[test]
    fn wrapping_is_greedy_and_never_drops_a_word() {
        let widths = [30.0, 40.0, 50.0, 20.0, 200.0, 10.0];
        let rows = wrap_rows(&widths, 10.0, 100.0);
        assert_eq!(rows, vec![(0, 2), (2, 4), (4, 5), (5, 6)]);
        assert_eq!(row_width(&widths, 10.0, rows[0]), 80.0);
        assert!(wrap_rows(&[], 10.0, 100.0).is_empty());
        let covered: usize = rows.iter().map(|row| row.1 - row.0).sum();
        assert_eq!(covered, widths.len());
    }

    #[test]
    fn the_fitted_scale_fills_the_box_without_leaving_it() {
        let widths = [3.0, 4.0, 5.0, 2.0];
        let scale = fit_scale(&widths, 1.0, 1.2, 100.0, 60.0, 50.0);
        assert!(scale > 0.0);
        let scaled: Vec<f32> = widths.iter().map(|width| width * scale).collect();
        let rows = wrap_rows(&scaled, scale, 100.0);
        assert!(rows.len() as f32 * 1.2 * scale <= 60.0 + 1.0e-3);
        for row in rows {
            assert!(row_width(&scaled, scale, row) <= 100.0 + 1.0e-3);
        }
        // A little larger no longer fits: the answer is the edge, not a guess.
        let larger: Vec<f32> = widths.iter().map(|width| width * scale * 1.05).collect();
        let rows = wrap_rows(&larger, scale * 1.05, 100.0);
        let fits = rows.len() as f32 * 1.2 * scale * 1.05 <= 60.0
            && rows
                .iter()
                .all(|&row| row_width(&larger, scale * 1.05, row) <= 100.0);
        assert!(!fits);
        assert_eq!(fit_scale(&[], 1.0, 1.2, 100.0, 60.0, 50.0), 0.0);
        assert_eq!(fit_scale(&widths, 1.0, 1.2, 1000.0, 1000.0, 2.0), 2.0);
    }

    #[test]
    fn a_typed_line_ends_complete() {
        assert_eq!(typed_count(10, 0.0), 0);
        assert_eq!(typed_count(10, 0.55), 5);
        assert_eq!(typed_count(10, 0.999), 9);
        assert_eq!(typed_count(10, 1.0), 10);
        assert_eq!(typed_count(0, 0.5), 0);
    }

    #[test]
    fn the_ticker_walk_is_deterministic_and_bounded() {
        for candle in 0..64 {
            let step = ticker_step(7, 3, candle);
            assert!((-1.0..=1.0).contains(&step));
            assert_eq!(step, ticker_step(7, 3, candle));
        }
        assert_ne!(ticker_step(7, 3, 5), ticker_step(7, 4, 5));
    }
}
