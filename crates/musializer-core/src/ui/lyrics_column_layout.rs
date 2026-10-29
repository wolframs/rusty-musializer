//! The Lyrics view's layout in the right column (WX-5d §3.2).
//!
//! **Owner: the Lyrics builder.** The cue card, the list's 34 px rows, the
//! footer, and the Style and Effects panes' groups, raylib-free so the gate can
//! aim at rects from the `lyrics column:` and `caption style:` lines rather than
//! at pixels measured once.
//!
//! Every number here is the approved mockup's (`build/redesign/wx5d-src/
//! mock.py`, the `lyrics_cues` and `lyrics_style_1280` pages): body padding
//! 12/14, a 6 px card with 12 px padding and 9 px between its rows, 30 px
//! controls, 34 px list rows one pixel apart. Labels are fixed English words, so
//! button widths are constants too; a label that outgrows its box is fitted by
//! the drawing side (`widgets::text_button` shrinks its type), never moved.
//!
//! # Two densities
//!
//! The mockup's card is 271 px tall, and at 960x640 a column body of 554 px
//! then holds the card and under three rows. The design note promises the card
//! and at least four rows there (§3.2, check 5), so a body shorter than
//! [`COMPACT_BODY_HEIGHT`] tightens the card's gaps and its text field rather
//! than dropping a control. Every control keeps its size; only air goes.
//!
//! # Scrolling panes
//!
//! Style and Effects are taller than the shortest body (Style is about 560 px
//! before a picker or an import opens in it), so both are laid out as one
//! content column that scrolls. [`scrolled`] is the only place an offset is
//! applied, and the drawing side clips both pixels and presses to the body.

use super::workspace_layout::UiRect;

// ---------------------------------------------------------------------------
// Shared metrics
// ---------------------------------------------------------------------------

/// The body's horizontal padding (`.body`'s `padding: 12px 14px`).
pub const BODY_PAD_X: f32 = 14.0;
/// The body's vertical padding.
pub const BODY_PAD_Y: f32 = 12.0;
/// Between the card, the list head and the list (`.body`'s `gap`).
pub const BODY_GAP: f32 = 10.0;
/// Every button, field row and slider row (`.btn`, `.row`).
pub const CONTROL_HEIGHT: f32 = 30.0;
/// The card's corner radius (`.card`).
pub const CARD_RADIUS: f32 = 6.0;
/// A column narrower than this drops the words from the set-to-playhead
/// buttons and moves Delete to the Apply row (§3.2).
pub const NARROW_COLUMN: f32 = 400.0;
/// A body shorter than this uses the compact card (see the module comment).
pub const COMPACT_BODY_HEIGHT: f32 = 600.0;

/// One list row (§3.2: "34 px rows").
pub const ROW_HEIGHT: f32 = 34.0;
/// Row to row: the rows sit one pixel apart (`gap: 1px`).
pub const ROW_STRIDE: f32 = ROW_HEIGHT + 1.0;
/// The current cue's tick: 2 px wide, inset 7 px top and bottom.
pub const TICK_WIDTH: f32 = 2.0;
pub const TICK_INSET: f32 = 7.0;
/// Inside a row: left padding, the origin dot, the time column.
pub const ROW_PAD_X: f32 = 10.0;
pub const ROW_DOT: f32 = 6.0;
pub const ROW_TIME_WIDTH: f32 = 76.0;
pub const ROW_INNER_GAP: f32 = 10.0;

/// The legend's dot rows (`.micro` at 11 px in a 16 px line) and their gap.
pub const LEGEND_ROW: f32 = 16.0;
pub const LEGEND_ROW_GAP: f32 = 4.0;

/// Section heads inside a scrolling pane (`.gh`, 18 px, 6 px above).
pub const SECTION_HEAD: f32 = 18.0;
pub const SECTION_MARGIN: f32 = 6.0;
/// Between items in a pane body (`.body` on the Style page: `gap: 8px`).
pub const PANE_GAP: f32 = 8.0;
/// The label column in Style and Effects rows (`labw=58`).
pub const PANE_LABEL: f32 = 58.0;

// ---------------------------------------------------------------------------
// Cues
// ---------------------------------------------------------------------------

/// Button widths on the card and in the footer. Constants because the labels
/// are; see the module comment.
pub mod width {
    pub const STEP: f32 = 26.0;
    pub const TAP: f32 = 48.0;
    pub const SPLIT: f32 = 54.0;
    pub const MERGE: f32 = 62.0;
    pub const APPLY: f32 = 64.0;
    pub const DISCARD: f32 = 72.0;
    pub const DELETE: f32 = 64.0;
    pub const TIME_LABEL: f32 = 42.0;
    pub const TIME_FIELD_NARROW: f32 = 104.0;
    pub const TIME_FIELD_WIDE: f32 = 118.0;
    pub const PLAYHEAD_WORD: f32 = 92.0;
    pub const FOLLOW: f32 = 150.0;
    pub const ADD_WIDE: f32 = 184.0;
    pub const ADD_NARROW: f32 = 146.0;
    pub const FILE: f32 = 70.0;
    pub const LIST_HEAD: f32 = 22.0;
}

/// How tight the card is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Density {
    pub card_pad: f32,
    pub card_gap: f32,
    pub text_height: f32,
    pub footer_pad: f32,
    pub footer_gap: f32,
    pub body_pad_y: f32,
}

impl Density {
    pub const NORMAL: Self = Self {
        card_pad: 12.0,
        card_gap: 9.0,
        text_height: 40.0,
        footer_pad: 10.0,
        footer_gap: 10.0,
        body_pad_y: BODY_PAD_Y,
    };
    pub const COMPACT: Self = Self {
        card_pad: 10.0,
        card_gap: 5.0,
        text_height: 34.0,
        footer_pad: 8.0,
        footer_gap: 6.0,
        body_pad_y: 8.0,
    };

    /// The density for a body this tall.
    #[must_use]
    pub fn for_body(height: f32) -> Self {
        if height < COMPACT_BODY_HEIGHT {
            Self::COMPACT
        } else {
            Self::NORMAL
        }
    }
}

/// One START or END row on the card.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeRow {
    pub row: UiRect,
    pub label: UiRect,
    /// The click-to-type time.
    pub field: UiRect,
    pub minus: UiRect,
    pub plus: UiRect,
    /// "Set to the playhead": a crosshair, with the word when wide.
    pub playhead: UiRect,
}

/// The cue card: everything that edits the selected cue.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CardLayout {
    pub card: UiRect,
    /// `CUE 22 OF 83 · ● USER`.
    pub title: UiRect,
    pub previous: UiRect,
    pub next: UiRect,
    /// The cue's words, in the authored face.
    pub text: UiRect,
    pub start: TimeRow,
    pub end: TimeRow,
    /// The hairline above the timing ladder.
    pub rule: UiRect,
    pub tap: UiRect,
    pub split: UiRect,
    pub merge: UiRect,
    pub undo: UiRect,
    pub redo: UiRect,
    pub apply: UiRect,
    pub discard: UiRect,
    pub delete: UiRect,
    /// The shortcut hint right of Discard; empty in a narrow column, where
    /// Delete takes that place.
    pub hint: UiRect,
    /// A status line under the Apply row (an armed tap run, an overlap
    /// warning); empty when there is nothing to say.
    pub status: UiRect,
}

/// The list's head and its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListLayout {
    /// `ALL CUES`.
    pub head: UiRect,
    /// The `follows the playhead` chip.
    pub follow: UiRect,
    /// Where rows go; [`row_rect`] places one.
    pub rows: UiRect,
    /// Whole rows that fit.
    pub visible: usize,
}

/// The footer under the list: the origin legend and the document buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FooterLayout {
    pub footer: UiRect,
    /// The hairline along its top.
    pub rule: UiRect,
    pub legend: UiRect,
    pub add: UiRect,
    pub import: UiRect,
    pub export: UiRect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CuesLayout {
    pub card: CardLayout,
    pub list: ListLayout,
    pub footer: FooterLayout,
    /// Whether the set-to-playhead buttons carry their word and Delete sits on
    /// the ladder row.
    pub wide: bool,
}

/// What the Cues pane's layout depends on besides the body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CuesSpec {
    /// One or two; the drawing side measures whether four labels fit in one.
    pub legend_rows: usize,
    /// Whether the card has a status line to show.
    pub status_line: bool,
}

/// The legend's height for `rows` rows.
#[must_use]
pub fn legend_height(rows: usize) -> f32 {
    let rows = rows.max(1) as f32;
    rows * LEGEND_ROW + (rows - 1.0) * LEGEND_ROW_GAP
}

fn time_row(x: f32, y: f32, width: f32, wide: bool) -> TimeRow {
    let field_width = if wide {
        width::TIME_FIELD_WIDE
    } else {
        width::TIME_FIELD_NARROW
    };
    let label = UiRect::new(x, y, width::TIME_LABEL, CONTROL_HEIGHT);
    let field = UiRect::new(x + width::TIME_LABEL + 8.0, y, field_width, CONTROL_HEIGHT);
    let minus = UiRect::new(field.x + field.width + 8.0, y, width::STEP, CONTROL_HEIGHT);
    let plus = UiRect::new(minus.x + width::STEP + 4.0, y, width::STEP, CONTROL_HEIGHT);
    let playhead = UiRect::new(
        plus.x + width::STEP + 8.0,
        y,
        if wide {
            width::PLAYHEAD_WORD
        } else {
            width::STEP
        },
        CONTROL_HEIGHT,
    );
    TimeRow {
        row: UiRect::new(x, y, width, CONTROL_HEIGHT),
        label,
        field,
        minus,
        plus,
        playhead,
    }
}

/// The Cues pane in `body`, the column below its head.
#[must_use]
pub fn cues_layout(body: UiRect, spec: CuesSpec) -> CuesLayout {
    let density = Density::for_body(body.height);
    let wide = body.width >= NARROW_COLUMN;
    let x = body.x + BODY_PAD_X;
    let content_width = (body.width - BODY_PAD_X * 2.0).max(0.0);
    let right = x + content_width;
    let bottom = body.y + body.height;

    // The card, top down.
    let card_top = body.y + density.body_pad_y;
    let pad = density.card_pad;
    let gap = density.card_gap;
    let inner_x = x + pad;
    let inner_width = (content_width - pad * 2.0).max(0.0);
    let inner_right = inner_x + inner_width;
    let mut y = card_top + pad;
    let next = UiRect::new(inner_right - width::STEP, y, width::STEP, CONTROL_HEIGHT);
    let previous = UiRect::new(next.x - 4.0 - width::STEP, y, width::STEP, CONTROL_HEIGHT);
    let title = UiRect::new(
        inner_x,
        y,
        (previous.x - 8.0 - inner_x).max(0.0),
        CONTROL_HEIGHT,
    );
    y += CONTROL_HEIGHT + gap;
    let text = UiRect::new(inner_x, y, inner_width, density.text_height);
    y += density.text_height + gap;
    let start = time_row(inner_x, y, inner_width, wide);
    y += CONTROL_HEIGHT + gap;
    let end = time_row(inner_x, y, inner_width, wide);
    y += CONTROL_HEIGHT + gap;
    let rule = UiRect::new(inner_x, y, inner_width, 1.0);
    y += 1.0 + gap;
    let tap = UiRect::new(inner_x, y, width::TAP, CONTROL_HEIGHT);
    let split = UiRect::new(tap.x + tap.width + 4.0, y, width::SPLIT, CONTROL_HEIGHT);
    let merge = UiRect::new(split.x + split.width + 4.0, y, width::MERGE, CONTROL_HEIGHT);
    let undo = UiRect::new(merge.x + merge.width + 4.0, y, width::STEP, CONTROL_HEIGHT);
    let redo = UiRect::new(undo.x + undo.width + 2.0, y, width::STEP, CONTROL_HEIGHT);
    let ladder_y = y;
    y += CONTROL_HEIGHT + gap;
    let apply = UiRect::new(inner_x, y, width::APPLY, CONTROL_HEIGHT);
    let discard = UiRect::new(
        apply.x + apply.width + 8.0,
        y,
        width::DISCARD,
        CONTROL_HEIGHT,
    );
    let (delete, hint) = if wide {
        let hint_x = discard.x + discard.width + 8.0;
        (
            UiRect::new(
                inner_right - width::DELETE,
                ladder_y,
                width::DELETE,
                CONTROL_HEIGHT,
            ),
            UiRect::new(hint_x, y, (inner_right - hint_x).max(0.0), CONTROL_HEIGHT),
        )
    } else {
        (
            UiRect::new(
                inner_right - width::DELETE,
                y,
                width::DELETE,
                CONTROL_HEIGHT,
            ),
            UiRect::new(inner_right, y, 0.0, 0.0),
        )
    };
    y += CONTROL_HEIGHT;
    let status = if spec.status_line {
        y += gap;
        let line = UiRect::new(inner_x, y, inner_width, LEGEND_ROW);
        y += LEGEND_ROW;
        line
    } else {
        UiRect::new(inner_x, y, 0.0, 0.0)
    };
    let card = UiRect::new(x, card_top, content_width, y + pad - card_top);

    // The footer, bottom up.
    let legend_h = legend_height(spec.legend_rows);
    let footer_height = density.footer_pad * 2.0 + CONTROL_HEIGHT + density.footer_gap + legend_h;
    let footer = UiRect::new(body.x, bottom - footer_height, body.width, footer_height);
    let legend = UiRect::new(x, footer.y + density.footer_pad, content_width, legend_h);
    let button_y = legend.y + legend_h + density.footer_gap;
    let export = UiRect::new(right - width::FILE, button_y, width::FILE, CONTROL_HEIGHT);
    let import = UiRect::new(
        export.x - 4.0 - width::FILE,
        button_y,
        width::FILE,
        CONTROL_HEIGHT,
    );
    let add = UiRect::new(
        x,
        button_y,
        if wide {
            width::ADD_WIDE
        } else {
            width::ADD_NARROW
        },
        CONTROL_HEIGHT,
    );

    // The list between them.
    let head_y = card.y + card.height + BODY_GAP;
    let head = UiRect::new(x, head_y, 120.0_f32.min(content_width), width::LIST_HEAD);
    let follow = UiRect::new(
        right - width::FOLLOW,
        head_y,
        width::FOLLOW,
        width::LIST_HEAD,
    );
    let rows_top = head_y + width::LIST_HEAD + BODY_GAP;
    let rows_bottom = footer.y - density.body_pad_y;
    let rows = UiRect::new(
        x,
        rows_top,
        content_width,
        (rows_bottom - rows_top).max(0.0),
    );
    let visible = ((rows.height + 1.0) / ROW_STRIDE).floor().max(0.0) as usize;

    CuesLayout {
        card: CardLayout {
            card,
            title,
            previous,
            next,
            text,
            start,
            end,
            rule,
            tap,
            split,
            merge,
            undo,
            redo,
            apply,
            discard,
            delete,
            hint,
            status,
        },
        list: ListLayout {
            head,
            follow,
            rows,
            visible,
        },
        footer: FooterLayout {
            footer,
            rule: UiRect::new(footer.x, footer.y, footer.width, 1.0),
            legend,
            add,
            import,
            export,
        },
        wide,
    }
}

/// Row `index` of the visible window.
#[must_use]
pub fn row_rect(rows: UiRect, index: usize) -> UiRect {
    UiRect::new(
        rows.x,
        rows.y + index as f32 * ROW_STRIDE,
        rows.width,
        ROW_HEIGHT,
    )
}

/// The parts of one row: the origin dot, the time, the words, and the tick
/// the current cue wears.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowParts {
    pub dot: UiRect,
    pub time: UiRect,
    pub text: UiRect,
    pub tick: UiRect,
}

/// [`RowParts`] for a row rect.
#[must_use]
pub fn row_parts(row: UiRect) -> RowParts {
    let dot = UiRect::new(
        row.x + ROW_PAD_X,
        (row.y + (row.height - ROW_DOT) * 0.5).round(),
        ROW_DOT,
        ROW_DOT,
    );
    let time = UiRect::new(
        dot.x + ROW_DOT + ROW_INNER_GAP,
        row.y,
        ROW_TIME_WIDTH,
        row.height,
    );
    let text_x = time.x + ROW_TIME_WIDTH + ROW_INNER_GAP;
    RowParts {
        dot,
        time,
        text: UiRect::new(
            text_x,
            row.y,
            (row.x + row.width - ROW_PAD_X - text_x).max(0.0),
            row.height,
        ),
        tick: UiRect::new(
            row.x,
            row.y + TICK_INSET,
            TICK_WIDTH,
            (row.height - TICK_INSET * 2.0).max(0.0),
        ),
    }
}

/// The first row the list shows.
///
/// Following, the list keeps `focus` in its middle — a lyric sheet scrolling
/// with the song; not following, it stays where the wheel left it. Either way
/// the window never runs past the last cue.
#[must_use]
pub fn list_first(
    focus: usize,
    visible: usize,
    count: usize,
    follow: bool,
    held_first: usize,
) -> usize {
    let first = if follow {
        focus.saturating_sub(visible / 2)
    } else {
        held_first
    };
    first.min(count.saturating_sub(visible))
}

/// The cue whose span holds `seconds`: the first one, as the list has always
/// marked it. `spans` are `(start, end)` in document order.
#[must_use]
pub fn live_cue(spans: &[(f64, f64)], seconds: f64) -> Option<usize> {
    spans
        .iter()
        .position(|&(start, end)| start <= seconds && seconds < end)
}

/// The cue nearest `seconds`, measured to the nearer end of its span. A tie
/// goes to the earlier cue, so the answer never depends on float noise in the
/// order of a scan.
#[must_use]
pub fn nearest_cue(spans: &[(f64, f64)], seconds: f64) -> Option<usize> {
    if !seconds.is_finite() {
        return None;
    }
    let mut best: Option<(usize, f64)> = None;
    for (index, &(start, end)) in spans.iter().enumerate() {
        let distance = if seconds < start {
            start - seconds
        } else if seconds >= end {
            seconds - end
        } else {
            0.0
        };
        if best.is_none_or(|(_, held)| distance < held) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

/// The cue the preview draws as a sample caption (§3.2): with a caption pane
/// open and no cue live at the playhead, the nearest one; otherwise none.
///
/// A live cue always wins, so the sample can never stand in for the real line,
/// and with Cues open there is no sample at all: the card is editing words,
/// not their look.
#[must_use]
pub fn sample_cue(caption_pane_open: bool, spans: &[(f64, f64)], seconds: f64) -> Option<usize> {
    if !caption_pane_open || live_cue(spans, seconds).is_some() {
        return None;
    }
    nearest_cue(spans, seconds)
}

// ---------------------------------------------------------------------------
// Style and Effects
// ---------------------------------------------------------------------------

/// A face chip: "Aa" in the face, its name under it (`height:44px`).
pub const FACE_CHIP_HEIGHT: f32 = 44.0;
pub const FACE_CHIP_GAP: f32 = 6.0;
/// The 3x3 anchor frame (`width:112px;height:63px`).
pub const PLACE_WIDTH: f32 = 112.0;
pub const PLACE_HEIGHT: f32 = 63.0;
pub const PLACE_DOT: f32 = 8.0;
/// The swatch row's preferred gap; it shrinks to fit a narrow column.
pub const SWATCH_GAP: f32 = 7.0;
pub const SWATCH_MIN_GAP: f32 = 4.0;
pub const SWATCH: f32 = 24.0;
/// The free picker, inline under its row.
pub const PICKER_HEIGHT: f32 = 200.0;
/// The import progress under the face chips: family and Cancel, the sweep,
/// the status line.
pub const PROGRESS_HEIGHT: f32 = 58.0;
/// The drive meter under each segmented item.
pub const METER_HEIGHT: f32 = 3.0;
pub const METER_GAP: f32 = 3.0;
/// An open `~ Tune` editor, inline under its row.
pub const TUNE_HEIGHT: f32 = 244.0;
/// Two lines of micro type for the closing note.
pub const NOTE_HEIGHT: f32 = 34.0;

/// Moves a rect by the pane's scroll offset. The one place an offset is
/// applied, so nothing is scrolled twice or not at all.
#[must_use]
pub fn scrolled(rect: UiRect, offset: f32) -> UiRect {
    UiRect::new(rect.x, rect.y - offset, rect.width, rect.height)
}

/// The furthest a pane may scroll.
#[must_use]
pub fn max_scroll(content_height: f32, body: UiRect) -> f32 {
    (content_height - body.height).max(0.0)
}

/// A labelled row: the 58 px label and what follows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LabelledRow {
    pub row: UiRect,
    pub label: UiRect,
    pub control: UiRect,
}

fn labelled(x: f32, y: f32, width: f32) -> LabelledRow {
    LabelledRow {
        row: UiRect::new(x, y, width, CONTROL_HEIGHT),
        label: UiRect::new(x, y, PANE_LABEL, CONTROL_HEIGHT),
        control: UiRect::new(
            x + PANE_LABEL + 8.0,
            y,
            (width - PANE_LABEL - 8.0).max(0.0),
            CONTROL_HEIGHT,
        ),
    }
}

/// `count` swatches across a row's control area, each [`SWATCH`] square and
/// vertically centred, the gap shrinking from [`SWATCH_GAP`] to
/// [`SWATCH_MIN_GAP`] before any swatch is dropped.
#[must_use]
pub fn swatch_rects(area: UiRect, count: usize) -> Vec<UiRect> {
    if count == 0 {
        return Vec::new();
    }
    let room = (area.width - SWATCH * count as f32) / (count.saturating_sub(1).max(1)) as f32;
    let gap = room.clamp(SWATCH_MIN_GAP, SWATCH_GAP);
    let fits = ((area.width + gap) / (SWATCH + gap)).floor().max(0.0) as usize;
    let y = (area.y + (area.height - SWATCH) * 0.5).round();
    (0..count.min(fits))
        .map(|index| UiRect::new(area.x + index as f32 * (SWATCH + gap), y, SWATCH, SWATCH))
        .collect()
}

/// The free colour picker, inline under the row whose `+` opened it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PickerLayout {
    pub card: UiRect,
    pub heading: UiRect,
    pub done: UiRect,
    /// The saturation/value square.
    pub field: UiRect,
    pub hue: UiRect,
    pub alpha: UiRect,
    pub readout: UiRect,
}

fn picker(x: f32, y: f32, width: f32) -> PickerLayout {
    let inner_x = x + 10.0;
    let inner_width = (width - 20.0).max(0.0);
    let top = y + 10.0;
    PickerLayout {
        card: UiRect::new(x, y, width, PICKER_HEIGHT),
        heading: UiRect::new(inner_x, top, (inner_width - 64.0).max(0.0), 22.0),
        done: UiRect::new(inner_x + inner_width - 56.0, top, 56.0, 22.0),
        field: UiRect::new(inner_x, top + 30.0, 120.0, 120.0),
        hue: UiRect::new(inner_x + 128.0, top + 30.0, 16.0, 120.0),
        alpha: UiRect::new(inner_x, top + 158.0, 144.0, 18.0),
        readout: UiRect::new(
            inner_x + 156.0,
            top + 30.0,
            (inner_width - 156.0).max(0.0),
            34.0,
        ),
    }
}

/// The import in flight, under the face chips.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProgressLayout {
    pub block: UiRect,
    pub family: UiRect,
    pub cancel: UiRect,
    /// The indeterminate sweep's track.
    pub sweep: UiRect,
    pub status: UiRect,
}

fn progress(x: f32, y: f32, width: f32) -> ProgressLayout {
    let cancel = UiRect::new(x + width - 64.0, y, 64.0, 26.0);
    ProgressLayout {
        block: UiRect::new(x, y, width, PROGRESS_HEIGHT),
        family: UiRect::new(x, y, (cancel.x - 8.0 - x).max(0.0), 26.0),
        cancel,
        sweep: UiRect::new(x, y + 32.0, width, 4.0),
        status: UiRect::new(x, y + 42.0, width, 16.0),
    }
}

/// What the Style pane's layout depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleSpec {
    /// Faces on offer: two, or three with an imported one.
    pub faces: usize,
    /// An import is in flight.
    pub progress: bool,
    /// Tuner rows under BACKING: Shadow has two, Plate one, None none.
    pub tuners: usize,
    /// Which picker is open, if any.
    pub picker: Option<PickerAt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerAt {
    Ink,
    Plate,
}

/// The Style pane, in content coordinates: `y` grows from the body's top as if
/// nothing scrolled. Pass each rect through [`scrolled`] to draw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleLayout {
    pub type_head: UiRect,
    /// One per face, then `+ Import` last.
    pub face_chips: Vec<UiRect>,
    pub progress: Option<ProgressLayout>,
    pub size: LabelledRow,
    pub width: LabelledRow,
    pub colour_head: UiRect,
    pub ink: LabelledRow,
    pub ink_picker: Option<PickerLayout>,
    pub plate: LabelledRow,
    pub plate_picker: Option<PickerLayout>,
    pub backing_head: UiRect,
    pub backing: UiRect,
    pub tuners: Vec<LabelledRow>,
    pub place_head: UiRect,
    pub place_frame: UiRect,
    /// Press areas, row-major from the top; each a third of the frame.
    pub anchors: [UiRect; 9],
    /// The drawn dots, in the same order.
    pub anchor_dots: [UiRect; 9],
    pub inset: LabelledRow,
    /// The anchor's name, under Inset.
    pub place_word: UiRect,
    pub note: UiRect,
    /// Everything above, plus the bottom padding.
    pub content_height: f32,
}

fn section(x: f32, y: &mut f32, width: f32, first: bool) -> UiRect {
    if !first {
        *y += SECTION_MARGIN;
    }
    let head = UiRect::new(x, *y, width, SECTION_HEAD);
    *y += SECTION_HEAD + PANE_GAP;
    head
}

/// The Style pane for a body `body`.
#[must_use]
pub fn style_layout(body: UiRect, spec: StyleSpec) -> StyleLayout {
    let x = body.x + BODY_PAD_X;
    let w = (body.width - BODY_PAD_X * 2.0).max(0.0);
    let mut y = body.y + BODY_PAD_Y;

    let type_head = section(x, &mut y, w, true);
    let chips = spec.faces.clamp(2, 3) + 1;
    let chip_width = (w - FACE_CHIP_GAP * (chips - 1) as f32) / chips as f32;
    let mut face_chips = Vec::with_capacity(chips);
    for index in 0..chips {
        let left = (x + index as f32 * (chip_width + FACE_CHIP_GAP)).round();
        let right = if index + 1 == chips {
            x + w
        } else {
            (x + (index + 1) as f32 * (chip_width + FACE_CHIP_GAP) - FACE_CHIP_GAP).round()
        };
        face_chips.push(UiRect::new(left, y, right - left, FACE_CHIP_HEIGHT));
    }
    y += FACE_CHIP_HEIGHT + PANE_GAP;
    let progress = spec.progress.then(|| {
        let block = progress(x, y, w);
        y += PROGRESS_HEIGHT + PANE_GAP;
        block
    });
    let size = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let width_row = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;

    let colour_head = section(x, &mut y, w, false);
    let ink = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let ink_picker = (spec.picker == Some(PickerAt::Ink)).then(|| {
        let block = picker(x, y, w);
        y += PICKER_HEIGHT + PANE_GAP;
        block
    });
    let plate = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let plate_picker = (spec.picker == Some(PickerAt::Plate)).then(|| {
        let block = picker(x, y, w);
        y += PICKER_HEIGHT + PANE_GAP;
        block
    });

    let backing_head = section(x, &mut y, w, false);
    let backing = UiRect::new(x, y, w, CONTROL_HEIGHT);
    y += CONTROL_HEIGHT + PANE_GAP;
    let mut tuners = Vec::with_capacity(spec.tuners);
    for _ in 0..spec.tuners.min(2) {
        tuners.push(labelled(x, y, w));
        y += CONTROL_HEIGHT + PANE_GAP;
    }

    let place_head = section(x, &mut y, w, false);
    let place_frame = UiRect::new(x, y, PLACE_WIDTH, PLACE_HEIGHT);
    let mut anchors = [UiRect::default(); 9];
    let mut anchor_dots = [UiRect::default(); 9];
    let cell_w = PLACE_WIDTH / 3.0;
    let cell_h = PLACE_HEIGHT / 3.0;
    for cell in 0..9 {
        let (column, row) = ((cell % 3) as f32, (cell / 3) as f32);
        anchors[cell] = UiRect::new(
            (x + column * cell_w).round(),
            (y + row * cell_h).round(),
            cell_w.floor(),
            cell_h.floor(),
        );
        anchor_dots[cell] = UiRect::new(
            x + 14.0 + column * 42.0,
            y + 10.0 + row * 22.0,
            PLACE_DOT,
            PLACE_DOT,
        );
    }
    let beside = x + PLACE_WIDTH + 12.0;
    let beside_width = (x + w - beside).max(0.0);
    let inset = LabelledRow {
        row: UiRect::new(beside, y, beside_width, CONTROL_HEIGHT),
        label: UiRect::new(beside, y, 40.0, CONTROL_HEIGHT),
        control: UiRect::new(
            beside + 48.0,
            y,
            (beside_width - 48.0).max(0.0),
            CONTROL_HEIGHT,
        ),
    };
    let place_word = UiRect::new(beside, y + CONTROL_HEIGHT + 4.0, beside_width, 16.0);
    y += PLACE_HEIGHT + PANE_GAP + SECTION_MARGIN;
    let note = UiRect::new(x, y, w, NOTE_HEIGHT);
    y += NOTE_HEIGHT + BODY_PAD_Y;

    StyleLayout {
        type_head,
        face_chips,
        progress,
        size,
        width: width_row,
        colour_head,
        ink,
        ink_picker,
        plate,
        plate_picker,
        backing_head,
        backing,
        tuners,
        place_head,
        place_frame,
        anchors,
        anchor_dots,
        inset,
        place_word,
        note,
        content_height: y - body.y,
    }
}

/// Where an open `~ Tune` editor's parts go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TuneLayout {
    pub card: UiRect,
    pub heading: UiRect,
    pub clamp: UiRect,
    pub reset: UiRect,
    pub meter_caption: UiRect,
    pub meter: UiRect,
    /// The anchor pairs' top edges; each pair is 35 px
    /// (`mapping_editor::anchor_pair`).
    pub quiet_y: f32,
    pub loud_y: f32,
    pub curve_y: f32,
    pub graph: UiRect,
    pub x: f32,
    pub width: f32,
}

fn tune(x: f32, y: f32, width: f32) -> TuneLayout {
    // Indented under its row, as Look's route editor is (`margin: 4px 0 8px 11px`).
    let card = UiRect::new(x + 11.0, y, (width - 11.0).max(0.0), TUNE_HEIGHT);
    let inner_x = card.x + 10.0;
    let inner_width = (card.width - 20.0).max(0.0);
    let top = card.y + 10.0;
    TuneLayout {
        card,
        heading: UiRect::new(inner_x, top, (inner_width - 112.0).max(0.0), 22.0),
        clamp: UiRect::new(inner_x + inner_width - 108.0, top, 52.0, 22.0),
        reset: UiRect::new(inner_x + inner_width - 52.0, top, 52.0, 22.0),
        meter_caption: UiRect::new(inner_x, top + 28.0, inner_width, 12.0),
        meter: UiRect::new(inner_x, top + 42.0, inner_width, 7.0),
        quiet_y: top + 54.0,
        loud_y: top + 94.0,
        curve_y: top + 134.0,
        graph: UiRect::new(inner_x, top + 162.0, inner_width, 60.0),
        x: inner_x,
        width: inner_width,
    }
}

/// One drive group in Effects: the segmented choice with its meters, then the
/// row its `~` hangs on, then the editor if open.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriveGroup {
    pub head: UiRect,
    pub drive: UiRect,
    /// The strip under the segmented control where each source's live level
    /// is drawn under its own item.
    pub meters: UiRect,
    pub amount: LabelledRow,
    pub tune: Option<TuneLayout>,
}

/// What the Effects pane's layout depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectsSpec {
    pub glow_picker: bool,
    pub tune_pulse: bool,
    pub tune_hue: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectsLayout {
    pub glow_head: UiRect,
    pub glow: LabelledRow,
    pub radius: LabelledRow,
    pub colour: LabelledRow,
    pub picker: Option<PickerLayout>,
    pub pulse: DriveGroup,
    pub hue: DriveGroup,
    pub note: UiRect,
    pub content_height: f32,
}

fn drive_group(x: f32, y: &mut f32, w: f32, open: bool) -> DriveGroup {
    let head = section(x, y, w, false);
    let drive = UiRect::new(x, *y, w, CONTROL_HEIGHT);
    let meters = UiRect::new(x, *y + CONTROL_HEIGHT + METER_GAP, w, METER_HEIGHT);
    *y += CONTROL_HEIGHT + METER_GAP + METER_HEIGHT + PANE_GAP;
    let amount = labelled(x, *y, w);
    *y += CONTROL_HEIGHT + PANE_GAP;
    let tune = open.then(|| {
        let block = tune(x, *y, w);
        *y += TUNE_HEIGHT + PANE_GAP;
        block
    });
    DriveGroup {
        head,
        drive,
        meters,
        amount,
        tune,
    }
}

/// The Effects pane for a body `body`, in content coordinates.
#[must_use]
pub fn effects_layout(body: UiRect, spec: EffectsSpec) -> EffectsLayout {
    let x = body.x + BODY_PAD_X;
    let w = (body.width - BODY_PAD_X * 2.0).max(0.0);
    let mut y = body.y + BODY_PAD_Y;
    let glow_head = section(x, &mut y, w, true);
    let glow = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let radius = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let colour = labelled(x, y, w);
    y += CONTROL_HEIGHT + PANE_GAP;
    let picker_block = spec.glow_picker.then(|| {
        let block = picker(x, y, w);
        y += PICKER_HEIGHT + PANE_GAP;
        block
    });
    let pulse = drive_group(x, &mut y, w, spec.tune_pulse);
    let hue = drive_group(x, &mut y, w, spec.tune_hue);
    y += SECTION_MARGIN;
    let note = UiRect::new(x, y, w, NOTE_HEIGHT);
    y += NOTE_HEIGHT + BODY_PAD_Y;
    EffectsLayout {
        glow_head,
        glow,
        radius,
        colour,
        picker: picker_block,
        pulse,
        hue,
        note,
        content_height: y - body.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The column bodies the design note's table gives (§1), below the 46 px
    /// top bar and the 40 px head: 960x640, 1280x720, 1920x1080.
    const BODIES: [UiRect; 3] = [
        UiRect::new(640.0, 86.0, 320.0, 554.0),
        UiRect::new(922.0, 86.0, 358.0, 634.0),
        UiRect::new(1460.0, 86.0, 460.0, 994.0),
    ];

    /// The legend fits one row from the 358 px column up; at 320 it wraps.
    fn spec() -> CuesSpec {
        CuesSpec {
            legend_rows: 1,
            status_line: false,
        }
    }

    fn spec_for(body: UiRect) -> CuesSpec {
        CuesSpec {
            legend_rows: if body.width < 358.0 { 2 } else { 1 },
            ..spec()
        }
    }

    #[test]
    fn the_card_is_the_mockups_269_px_inside_its_border_and_its_parts_sit_where_it_drew_them() {
        let layout = cues_layout(BODIES[1], spec());
        let card = layout.card;
        assert_eq!(card.card, UiRect::new(936.0, 98.0, 330.0, 269.0));
        assert_eq!(card.title, UiRect::new(948.0, 110.0, 242.0, 30.0));
        assert_eq!(card.previous, UiRect::new(1198.0, 110.0, 26.0, 30.0));
        assert_eq!(card.next, UiRect::new(1228.0, 110.0, 26.0, 30.0));
        assert_eq!(card.text, UiRect::new(948.0, 149.0, 306.0, 40.0));
        assert_eq!(card.start.field, UiRect::new(998.0, 198.0, 104.0, 30.0));
        assert_eq!(card.start.playhead, UiRect::new(1174.0, 198.0, 26.0, 30.0));
        assert_eq!(card.end.row.y, 237.0);
        assert_eq!(card.rule, UiRect::new(948.0, 276.0, 306.0, 1.0));
        assert_eq!(card.tap, UiRect::new(948.0, 286.0, 48.0, 30.0));
        assert_eq!(card.redo, UiRect::new(1152.0, 286.0, 26.0, 30.0));
        assert_eq!(card.apply, UiRect::new(948.0, 325.0, 64.0, 30.0));
        // Narrow: Delete on the Apply row, at its right end, and no hint.
        assert_eq!(card.delete, UiRect::new(1190.0, 325.0, 64.0, 30.0));
        assert!(card.hint.is_empty());
        assert!(!layout.wide);
    }

    #[test]
    fn a_wide_column_puts_delete_on_the_ladder_and_words_on_the_playhead_buttons() {
        let layout = cues_layout(BODIES[2], spec());
        assert!(layout.wide);
        assert_eq!(layout.card.delete.y, layout.card.tap.y);
        assert_eq!(layout.card.start.playhead.width, width::PLAYHEAD_WORD);
        assert_eq!(layout.card.start.field.width, width::TIME_FIELD_WIDE);
        assert!(!layout.card.hint.is_empty());
        // Everything on the card stays inside it.
        let card = layout.card;
        for rect in [
            card.title,
            card.previous,
            card.next,
            card.text,
            card.start.playhead,
            card.end.playhead,
            card.redo,
            card.delete,
            card.hint,
        ] {
            assert!(card.card.contains(rect), "{rect:?}");
        }
    }

    #[test]
    fn every_size_keeps_the_card_and_at_least_four_rows() {
        // 960x640 is the design's check 5: the card and at least four rows.
        let small = cues_layout(BODIES[0], spec_for(BODIES[0]));
        assert_eq!(small.card.card.height, 235.0, "the compact card");
        assert_eq!(small.list.visible, 4);
        assert_eq!(cues_layout(BODIES[1], spec()).list.visible, 6);
        assert_eq!(cues_layout(BODIES[2], spec()).list.visible, 16);
        for body in BODIES {
            let layout = cues_layout(body, spec_for(body));
            let last = row_rect(layout.list.rows, layout.list.visible - 1);
            assert!(last.y + last.height <= layout.footer.footer.y, "{body:?}");
            assert!(layout.list.follow.x > layout.list.head.x + layout.list.head.width);
            assert!(body.contains(layout.footer.footer), "{body:?}");
            assert!(layout.footer.add.x + layout.footer.add.width < layout.footer.import.x);
        }
    }

    #[test]
    fn a_status_line_grows_the_card_by_one_line_and_the_list_gives_way() {
        let without = cues_layout(BODIES[1], spec());
        let with = cues_layout(
            BODIES[1],
            CuesSpec {
                status_line: true,
                ..spec()
            },
        );
        assert_eq!(
            with.card.card.height - without.card.card.height,
            LEGEND_ROW + Density::NORMAL.card_gap
        );
        assert_eq!(with.card.status.height, LEGEND_ROW);
        assert!(with.list.visible < without.list.visible);
    }

    #[test]
    fn rows_are_34_px_one_pixel_apart_with_the_tick_inset_7() {
        let rows = UiRect::new(936.0, 400.0, 330.0, 300.0);
        assert_eq!(row_rect(rows, 0), UiRect::new(936.0, 400.0, 330.0, 34.0));
        assert_eq!(row_rect(rows, 2), UiRect::new(936.0, 470.0, 330.0, 34.0));
        let parts = row_parts(row_rect(rows, 0));
        assert_eq!(parts.tick, UiRect::new(936.0, 407.0, 2.0, 20.0));
        assert_eq!(parts.dot, UiRect::new(946.0, 414.0, 6.0, 6.0));
        assert_eq!(parts.time, UiRect::new(962.0, 400.0, 76.0, 34.0));
        assert_eq!(parts.text, UiRect::new(1048.0, 400.0, 208.0, 34.0));
    }

    #[test]
    fn the_list_follows_the_focus_and_never_runs_past_the_end() {
        assert_eq!(list_first(21, 6, 83, true, 0), 18);
        assert_eq!(list_first(1, 6, 83, true, 40), 0);
        assert_eq!(list_first(82, 6, 83, true, 0), 77);
        // Not following: where the wheel left it, clamped.
        assert_eq!(list_first(21, 6, 83, false, 40), 40);
        assert_eq!(list_first(21, 6, 83, false, 90), 77);
        assert_eq!(list_first(0, 6, 3, true, 0), 0);
    }

    #[test]
    fn the_sample_is_the_nearest_cue_only_when_none_is_live_and_a_caption_pane_is_open() {
        let spans = [(10.0, 12.0), (20.0, 22.5), (30.0, 31.0)];
        assert_eq!(live_cue(&spans, 21.0), Some(1));
        assert_eq!(live_cue(&spans, 22.5), None);
        assert_eq!(nearest_cue(&spans, 25.0), Some(1));
        assert_eq!(nearest_cue(&spans, 27.0), Some(2));
        assert_eq!(nearest_cue(&spans, 0.0), Some(0));
        assert_eq!(nearest_cue(&spans, 99.0), Some(2));
        // A tie (16 is 4 from both) goes to the earlier cue.
        assert_eq!(nearest_cue(&spans, 16.0), Some(0));
        assert_eq!(nearest_cue(&[], 16.0), None);
        assert_eq!(nearest_cue(&spans, f64::NAN), None);
        assert_eq!(sample_cue(true, &spans, 25.0), Some(1));
        assert_eq!(sample_cue(true, &spans, 21.0), None, "a live cue wins");
        assert_eq!(sample_cue(false, &spans, 25.0), None, "Cues shows none");
    }

    #[test]
    fn style_groups_follow_the_mockup_and_a_picker_opens_under_its_own_row() {
        let body = BODIES[1];
        let base = StyleSpec {
            faces: 2,
            progress: false,
            tuners: 2,
            picker: None,
        };
        let layout = style_layout(body, base);
        assert_eq!(layout.type_head, UiRect::new(936.0, 98.0, 330.0, 18.0));
        assert_eq!(layout.face_chips.len(), 3);
        assert_eq!(layout.face_chips[0], UiRect::new(936.0, 124.0, 106.0, 44.0));
        assert_eq!(layout.face_chips[2].x + layout.face_chips[2].width, 1266.0);
        assert_eq!(layout.size.row.y, 176.0);
        assert_eq!(layout.colour_head.y, 258.0);
        assert_eq!(layout.backing_head.y, 366.0);
        assert_eq!(layout.place_head.y, 512.0);
        assert_eq!(layout.place_frame, UiRect::new(936.0, 538.0, 112.0, 63.0));
        assert_eq!(layout.content_height, 575.0);
        for cell in 0..9 {
            assert!(layout.place_frame.contains(layout.anchors[cell]));
            assert!(
                layout.anchors[cell].contains(layout.anchor_dots[cell]),
                "{cell}"
            );
        }

        let picked = style_layout(
            body,
            StyleSpec {
                picker: Some(PickerAt::Ink),
                ..base
            },
        );
        let picker = picked.ink_picker.expect("the ink picker");
        assert_eq!(picker.card.y, layout.ink.row.y + CONTROL_HEIGHT + PANE_GAP);
        assert_eq!(
            picked.plate.row.y,
            layout.plate.row.y + PICKER_HEIGHT + PANE_GAP
        );
        assert!(picked.plate_picker.is_none());
        for part in [
            picker.heading,
            picker.done,
            picker.field,
            picker.hue,
            picker.alpha,
        ] {
            assert!(picker.card.contains(part), "{part:?}");
        }

        let busy = style_layout(
            body,
            StyleSpec {
                faces: 3,
                progress: true,
                ..base
            },
        );
        assert_eq!(busy.face_chips.len(), 4);
        let block = busy.progress.expect("the progress block");
        assert_eq!(block.block.y, 176.0);
        assert_eq!(busy.size.row.y, 176.0 + PROGRESS_HEIGHT + PANE_GAP);
    }

    #[test]
    fn swatches_shrink_their_gap_before_any_is_dropped() {
        let wide = UiRect::new(0.0, 0.0, 300.0, 30.0);
        let rects = swatch_rects(wide, 8);
        assert_eq!(rects.len(), 8);
        assert_eq!(rects[1].x - rects[0].x, SWATCH + SWATCH_GAP);
        assert_eq!(rects[0].y, 3.0);
        // The 960 column's row: 292 - 66 = 226 for eight swatches.
        let narrow = UiRect::new(0.0, 0.0, 226.0, 30.0);
        let rects = swatch_rects(narrow, 8);
        assert_eq!(rects.len(), 8);
        assert!(rects[7].x + SWATCH <= 226.0);
        assert!(rects[1].x - rects[0].x >= SWATCH + SWATCH_MIN_GAP);
        // Too narrow even at the minimum gap: the tail is dropped, never overlapped.
        assert_eq!(swatch_rects(UiRect::new(0.0, 0.0, 100.0, 30.0), 8).len(), 3);
    }

    #[test]
    fn effects_open_a_tune_editor_under_its_own_row() {
        let body = BODIES[1];
        let closed = effects_layout(body, EffectsSpec::default());
        assert_eq!(closed.glow_head.y, 98.0);
        assert!(closed.pulse.tune.is_none());
        assert_eq!(
            closed.pulse.meters.y,
            closed.pulse.drive.y + CONTROL_HEIGHT + METER_GAP
        );
        let open = effects_layout(
            body,
            EffectsSpec {
                tune_pulse: true,
                ..EffectsSpec::default()
            },
        );
        let tune = open.pulse.tune.expect("the pulse editor");
        assert_eq!(
            tune.card.y,
            open.pulse.amount.row.y + CONTROL_HEIGHT + PANE_GAP
        );
        assert_eq!(open.hue.head.y, closed.hue.head.y + TUNE_HEIGHT + PANE_GAP);
        assert!(tune.card.contains(tune.graph));
        assert!(tune.graph.y >= tune.curve_y + 22.0);
        assert_eq!(
            open.content_height - closed.content_height,
            TUNE_HEIGHT + PANE_GAP
        );
    }

    #[test]
    fn scrolling_moves_rects_and_never_past_the_content() {
        let rect = UiRect::new(10.0, 300.0, 20.0, 30.0);
        assert_eq!(scrolled(rect, 120.0), UiRect::new(10.0, 180.0, 20.0, 30.0));
        let body = UiRect::new(0.0, 86.0, 320.0, 554.0);
        assert_eq!(max_scroll(669.0, body), 115.0);
        assert_eq!(max_scroll(400.0, body), 0.0);
    }
}
