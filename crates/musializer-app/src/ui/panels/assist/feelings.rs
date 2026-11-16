//! A staged feelings answer, looked at before Apply (J2, 2026-11-16).
//!
//! **Owner: J2.** A child of [`super`] so it shares the panel's helpers (the
//! wrap, the ellipsis, the probe seams) instead of copying them.
//!
//! The listening model's sections are an opinion, not a measurement: in the
//! lead's three real calls one song was cut well, another had a stretch only
//! slightly quieter called a quiet breakdown and a hook's return placed eight
//! seconds early. "Feeling cues: 3 → 11" asked the user to accept that blind.
//! So the staged card lists what came back, one row per section — its span,
//! the model's words and its feel drawn as three marks — and a row is a place
//! in the song: pressing it moves the playhead just inside that section, so the
//! user can listen to what the model called a breakdown.
//!
//! Everything here is drawing and one seek. The candidate stays inert; the
//! rows read [`AnalysisCandidate::feeling_sections`], which Apply never looks
//! at.

use musializer_core::project::analysis_candidate::FeelingSection;
use musializer_core::scene::events::EventType;

use super::*;

/// The row text size: the review list's, so the two blocks read as one
/// vocabulary.
const TEXT_SIZE: f32 = 12.0;
/// The head row: `SECTIONS · 11` and the three marks' names over their columns.
const HEAD_HEIGHT: f32 = 18.0;
/// The energy strip across the whole song.
const MAP_HEIGHT: f32 = 26.0;
/// One line of the notes under the head: a reused answer, and what the
/// application changed in it before staging (J3-3).
const NOTE_LINE: f32 = 15.0;
/// Lines each note may wrap to.
const NOTE_MAX_LINES: usize = 3;
/// A resting row: the span and the marks, then one line of the model's words.
const ROW_HEIGHT: f32 = 36.0;
/// One line of an open row's whole text.
const OPEN_LINE: f32 = 16.0;
/// An open row shows at most this many lines of its text; a summary is at
/// most 2 047 bytes (`SUMMARY_MAX_BYTES`), which this fits at any column width
/// the window allows only when cut, so the last line says it was.
const OPEN_MAX_LINES: usize = 6;
/// Rows on screen before the list scrolls inside the card. Five resting rows
/// keep the Apply row on screen at 1280x720 with the card revealed, at the
/// minimum column width too; six pushed it under the fold there (2026-11-16
/// capture, a 300 px column with the Apply sentence on four lines).
const LIST_MAX_ROWS: usize = 5;
/// The `Sections 1-6 of 11` line under a list that scrolls.
const TAIL_HEIGHT: f32 = 16.0;
/// One line of the sentence that says what Apply does.
pub(super) const APPLY_LINE: f32 = 18.0;
const GAP: f32 = 6.0;
/// One mark's track, and the gap between two.
const MARK_WIDTH: f32 = 34.0;
const MARK_GAP: f32 = 6.0;
/// The feel-type stripe at a row's left, and the inset the text keeps from it.
const STRIPE_WIDTH: f32 = 3.0;
const ROW_INSET: f32 = 9.0;

/// Widget index of the first feelings row, in [`ASSIST_WIDGETS`]: clear of the
/// cards (`100..104`), Auto scenes (`110..`) and the review rows (`60..`). One
/// per row on screen, never more than [`LIST_MAX_ROWS`].
pub(super) const FEELING_ROW_WIDGET: u32 = 200;

/// `MUSIALIZER_ASSIST_PROBE_FEELINGS_SCROLL=N`: the first row a probe run wants
/// on screen, the review list's seam for the same reason (a headless run has
/// no wheel it can aim at a list).
pub const PROBE_FEELINGS_SCROLL_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_FEELINGS_SCROLL";
/// `MUSIALIZER_ASSIST_PROBE_FEELINGS_OPEN=N`: a row open from the first frame,
/// for a capture of an open row. It does not seek: a press does that, and the
/// gate presses.
pub const PROBE_FEELINGS_OPEN_VARIABLE: &str = "MUSIALIZER_ASSIST_PROBE_FEELINGS_OPEN";

/// What the list keeps between frames, on [`AssistColumn`].
#[derive(Clone, Debug, Default)]
pub struct FeelingsView {
    /// Which staged answer the rest describes: its length, first id and last
    /// end. A new answer opens at the top with nothing open.
    key: Option<(usize, u64, u64)>,
    /// The first row on screen.
    first: usize,
    /// The open row, by its index in the answer.
    open: Option<usize>,
    /// The row under the playhead last frame, so the list follows the song
    /// only when the playhead moves into another section.
    now: Option<usize>,
    /// Where the rows were last frame, so the column's wheel can give way to
    /// the list's own.
    pub(super) list: UiRect,
    /// The `assist feelings:` line, printed when it changes.
    report: String,
    /// Whether a probe's open row was applied already.
    probe_opened: bool,
    /// The `staged feelings:` line (the timeline's ghosts), printed when it
    /// changes. Kept across a new answer, like `report`.
    timeline_report: String,
}

impl FeelingsView {
    /// Forgets a previous answer's place when another is staged.
    fn bind(&mut self, sections: &[FeelingSection]) {
        let key = (
            sections.len(),
            sections.first().map_or(0, |section| section.event_id),
            sections
                .last()
                .map_or(0, |section| (section.end_seconds * 1000.0).round() as u64),
        );
        if self.key != Some(key) {
            *self = Self {
                key: Some(key),
                report: std::mem::take(&mut self.report),
                probe_opened: self.probe_opened,
                timeline_report: std::mem::take(&mut self.timeline_report),
                ..Self::default()
            };
        }
    }

    /// Moves the list by whole rows, as a wheel notch does. Returns whether it
    /// moved, so the column scrolls instead when the list is at its end.
    pub(super) fn scroll(&mut self, rows: i64, count: usize) -> bool {
        let before = self.first;
        self.first = (self.first as i64 + rows).clamp(0, count.saturating_sub(1) as i64) as usize;
        self.first != before
    }
}

/// `m:ss`, with tenths only where the model put a boundary between seconds
/// (the answers do: 22.5, 55.5).
fn span_clock(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let tenths = (seconds * 10.0).round() as u64;
    let whole = tenths / 10;
    if tenths.is_multiple_of(10) {
        format!("{}:{:02}", whole / 60, whole % 60)
    } else {
        format!("{}:{:02}.{}", whole / 60, whole % 60, tenths % 10)
    }
}

/// A row's span, `0:24-0:58`: the review list's form.
pub(super) fn span_text(section: &FeelingSection) -> String {
    format!(
        "{}-{}",
        span_clock(section.start_seconds),
        span_clock(section.end_seconds)
    )
}

/// The row under `seconds`, by the lane's own rule (a cue holds from its
/// start until the next begins).
pub(super) fn row_at(sections: &[FeelingSection], seconds: f64) -> Option<usize> {
    if !seconds.is_finite() {
        return None;
    }
    sections
        .iter()
        .position(|section| section.contains(seconds))
}

/// An open row's text, wrapped; the model's words in full up to
/// [`OPEN_MAX_LINES`].
fn open_lines(section: &FeelingSection, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let text = if section.summary.trim().is_empty() {
        "The model said nothing about this section."
    } else {
        section.summary.as_str()
    };
    wrap_to_width(text, (width - ROW_INSET).max(0.0), OPEN_MAX_LINES, measure)
}

/// The notes under the head, wrapped: whether the answer is a saved one,
/// reused, and what the application changed in it before staging (J3-3:
/// `bridge_fit` joins a section shorter than a millisecond to its neighbour,
/// its words and marks dropped, and shortens words past what a cue holds; the
/// card said none of it). Empty when there is nothing to say.
fn note_lines(
    candidate: &AnalysisCandidate,
    width: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    for note in [
        candidate.reused_answer().map(str::to_string),
        candidate.repairs().map(|repairs| repairs.sentence()),
    ]
    .into_iter()
    .flatten()
    {
        lines.extend(wrap_to_width(&note, width, NOTE_MAX_LINES, measure));
    }
    lines
}

/// One row's height: resting, or open with its lines.
fn row_height(open_lines: usize) -> f32 {
    if open_lines == 0 {
        ROW_HEIGHT
    } else {
        20.0 + open_lines as f32 * OPEN_LINE + 6.0
    }
}

/// Where everything in the block goes, from the rows' heights.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FeelingsGeometry {
    pub head: UiRect,
    /// The notes under the head; zero height when there are none.
    pub notes: UiRect,
    pub map: UiRect,
    /// The rows' window.
    pub list: UiRect,
    /// `(index in the answer, rect)`, on screen, top to bottom.
    pub rows: Vec<(usize, UiRect)>,
    /// The first row on screen, after clamping.
    pub first: usize,
    pub tail: Option<UiRect>,
    /// The Apply sentence's lines.
    pub apply: UiRect,
    pub height: f32,
}

/// Lays the block out at `(x, y)` in `width`. Pure, so the arithmetic that
/// decides which rows are on screen is pinned without a window.
///
/// The window's height is fixed by the answer (up to [`LIST_MAX_ROWS`] resting
/// rows, plus the open row's extra lines), not by the scroll position, so
/// scrolling never moves the Apply row. The first row is clamped so the last
/// page is full.
pub(super) fn geometry(
    x: f32,
    y: f32,
    width: f32,
    heights: &[f32],
    requested_first: usize,
    apply_lines: usize,
    note_lines: usize,
) -> FeelingsGeometry {
    let head = UiRect::new(x, y, width, HEAD_HEIGHT);
    let notes = UiRect::new(
        x,
        head.y + HEAD_HEIGHT,
        width,
        note_lines as f32 * NOTE_LINE,
    );
    let map = UiRect::new(x, notes.y + notes.height + 2.0, width, MAP_HEIGHT);
    let list_y = map.y + MAP_HEIGHT + GAP;
    let extra = heights
        .iter()
        .map(|height| height - ROW_HEIGHT)
        .fold(0.0f32, f32::max);
    let total: f32 = heights.iter().sum();
    let window = total.min(LIST_MAX_ROWS as f32 * ROW_HEIGHT + extra);
    // The last page: the smallest first row from which the rest still fills
    // the window. A small tolerance, so a window that is exactly the rows'
    // sum does not lose its last row to rounding.
    let mut last_first = heights.len();
    let mut tail_sum = 0.0;
    while last_first > 0 && tail_sum + heights[last_first - 1] <= window + 0.01 {
        last_first -= 1;
        tail_sum += heights[last_first];
    }
    let first = requested_first.min(last_first);
    let mut rows = Vec::new();
    let mut row_y = list_y;
    for (index, height) in heights.iter().enumerate().skip(first) {
        if row_y + height > list_y + window + 0.01 {
            break;
        }
        rows.push((index, UiRect::new(x, row_y, width, *height)));
        row_y += height;
    }
    let list = UiRect::new(x, list_y, width, window);
    let mut bottom = list_y + window;
    let tail = (rows.len() < heights.len()).then(|| {
        let tail = UiRect::new(x, bottom + 2.0, width, TAIL_HEIGHT);
        bottom += 2.0 + TAIL_HEIGHT;
        tail
    });
    let apply = UiRect::new(
        x,
        bottom + if apply_lines > 0 { GAP } else { 0.0 },
        width,
        apply_lines as f32 * APPLY_LINE,
    );
    let height = apply.y + apply.height - y;
    FeelingsGeometry {
        head,
        notes,
        map,
        list,
        rows,
        first,
        tail,
        apply,
        height,
    }
}

/// The rows' heights for this frame: every row resting but the open one.
fn heights(
    sections: &[FeelingSection],
    open: Option<usize>,
    width: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<f32> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if open == Some(index) {
                row_height(open_lines(section, width, measure).len())
            } else {
                ROW_HEIGHT
            }
        })
        .collect()
}

impl Shell {
    /// The block's height for this frame, as [`Self::draw_feelings`] will draw
    /// it; `0` when the candidate stages no feelings. Called before the column
    /// is laid out, so it applies the probe's open row and a new answer's
    /// reset first: the height it returns is the height that is drawn.
    pub(super) fn feelings_height(
        &mut self,
        font: &UiFonts,
        candidate: Option<&AnalysisCandidate>,
        width: f32,
        apply_lines: usize,
    ) -> f32 {
        let Some(sections) = candidate
            .filter(|candidate| candidate.available().semantics)
            .map(AnalysisCandidate::feeling_sections)
            .filter(|sections| !sections.is_empty())
        else {
            return 0.0;
        };
        let view = &mut self.assist_column.feelings;
        view.bind(sections);
        if !view.probe_opened {
            if let Some(row) = std::env::var(PROBE_FEELINGS_OPEN_VARIABLE)
                .ok()
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|row| *row < sections.len())
            {
                view.open = Some(row);
                view.probe_opened = true;
            }
        }
        let measure = |text: &str| widgets::measure(font, text, TEXT_SIZE);
        let heights = heights(sections, view.open, width, &measure);
        let notes = candidate.map_or(0, |candidate| note_lines(candidate, width, &measure).len());
        geometry(0.0, 0.0, width, &heights, view.first, apply_lines, notes).height
    }

    /// The block: head, the energy strip, the rows and the Apply sentence.
    /// Returns its part of the report.
    #[allow(
        clippy::too_many_lines,
        reason = "one block top to bottom, as the review list is"
    )]
    pub(super) fn draw_feelings(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        apply: &[String],
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        let session = &input.workspace.assist;
        let Some(candidate) = session
            .candidate
            .as_ref()
            .filter(|candidate| candidate.available().semantics)
        else {
            return;
        };
        let sections = candidate.feeling_sections();
        if sections.is_empty() {
            return;
        }
        let measure = |text: &str| widgets::measure(font, text, TEXT_SIZE);
        let notes = note_lines(candidate, area.width, &measure);
        let duration = sections
            .last()
            .map_or(0.0, |section| section.end_seconds)
            .max(input.duration_seconds);
        // The playhead only means a place in this answer on the track it is
        // for.
        let playhead = (session.candidate_track == input.workspace.current_index())
            .then_some(input.time_seconds);
        let now = playhead.and_then(|seconds| row_at(sections, seconds));

        // The list follows the song: when the playhead moves into a section
        // that is off the list, that row comes into view. Only on a change, so
        // a wheel that moved the list away is not undone every frame.
        let view_now = self.assist_column.feelings.now;
        if now != view_now {
            self.assist_column.feelings.now = now;
            if let Some(row) = now {
                let view = &mut self.assist_column.feelings;
                let heights = heights(sections, view.open, area.width, &measure);
                let shown = geometry(
                    area.x,
                    area.y,
                    area.width,
                    &heights,
                    view.first,
                    0,
                    notes.len(),
                );
                if !shown.rows.iter().any(|(index, _)| *index == row) {
                    view.first = row;
                }
            }
        }
        if let Some(first) = std::env::var(PROBE_FEELINGS_SCROLL_VARIABLE)
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            self.assist_column.feelings.first = first;
        }

        let view = &self.assist_column.feelings;
        let heights = heights(sections, view.open, area.width, &measure);
        let layout = geometry(
            area.x,
            area.y,
            area.width,
            &heights,
            view.first,
            apply.len(),
            notes.len(),
        );
        let open = view.open;

        // The head: how many, whether it was paid for, and the marks' names
        // over their columns.
        let marks_x = area.x + area.width - (MARK_WIDTH * 3.0 + MARK_GAP * 2.0);
        let count = format!("Sections \u{00b7} {}", sections.len());
        let head_runs = [widgets::MicroRun::muted(&count)];
        widgets::draw_micro(
            d,
            font,
            layout.head.x,
            layout.head.y + (HEAD_HEIGHT - metric::UI_FONT_MICRO) * 0.5,
            &head_runs,
        );
        for (column, name) in ["energy", "tension", "mood"].iter().enumerate() {
            let centre = marks_x + column as f32 * (MARK_WIDTH + MARK_GAP) + MARK_WIDTH * 0.5;
            let width = widgets::measure(font, name, 11.0);
            widgets::draw_text(
                d,
                font,
                name,
                centre - width * 0.5,
                layout.head.y + (HEAD_HEIGHT - 11.0) * 0.5,
                11.0,
                color::ui_muted(),
            );
        }

        for (index, line) in notes.iter().enumerate() {
            widgets::draw_text(
                d,
                font,
                line,
                layout.notes.x,
                layout.notes.y + index as f32 * NOTE_LINE + 1.0,
                TEXT_SIZE,
                color::ui_muted(),
            );
        }

        // The energy strip: the whole song at a glance, a column per section
        // as tall as its energy, the one under the playhead in ink. Not a
        // control: the rows under it are, and a second target for the same
        // press would be one more thing to miss.
        let map = layout.map;
        widgets::fill(
            d,
            UiRect::new(map.x, map.y + map.height - 1.0, map.width, 1.0),
            color::ui_rule(),
        );
        if duration > 0.0 {
            for (index, section) in sections.iter().enumerate() {
                let left = map.x + (section.start_seconds / duration) as f32 * map.width;
                let right = map.x + (section.end_seconds / duration) as f32 * map.width;
                let height = (3.0 + section.energy.clamp(0.0, 1.0) * (map.height - 4.0)).round();
                let block = UiRect::new(
                    left.round() + 1.0,
                    map.y + map.height - 1.0 - height,
                    (right.round() - left.round() - 1.0).max(1.0),
                    height,
                );
                let tint = if now == Some(index) {
                    color::ui_ink()
                } else if open == Some(index) {
                    color::ui_selected_edge()
                } else {
                    color::ui_level()
                };
                widgets::fill(d, block, tint);
            }
        }

        // The rows.
        let stripe = feel_tint();
        let mut row_report = Vec::new();
        let mut pressed = None;
        for (screen, (index, row)) in layout.rows.iter().enumerate() {
            let section = &sections[*index];
            let id = widgets::widget_id(ASSIST_WIDGETS, FEELING_ROW_WIDGET + screen as u32);
            let state = self.widgets.button(d, id, *row);
            let is_open = open == Some(*index);
            if is_open {
                widgets::fill(d, *row, color::ui_selected());
            } else if now == Some(*index) {
                widgets::fill(d, *row, color::ui_cue_now());
            } else if state.hovered {
                widgets::fill(d, *row, color::ui_hover());
            }
            if is_open {
                d.draw_rectangle_lines_ex(widgets::rectangle(*row), 1.0, color::ui_selected_edge());
            }
            widgets::fill(
                d,
                UiRect::new(row.x, row.y + 3.0, STRIPE_WIDTH, row.height - 6.0),
                stripe,
            );
            let text_x = row.x + ROW_INSET;
            let span = span_text(section);
            widgets::draw_text(
                d,
                font,
                &span,
                text_x,
                row.y + 4.0,
                TEXT_SIZE,
                color::ui_ink(),
            );
            if is_open {
                let confidence = format!("sure {:.0}%", section.confidence * 100.0);
                let span_width = widgets::measure(font, &span, TEXT_SIZE);
                let room = marks_x - (text_x + span_width + 8.0) - 6.0;
                if widgets::measure(font, &confidence, 11.0) <= room {
                    widgets::draw_text(
                        d,
                        font,
                        &confidence,
                        text_x + span_width + 8.0,
                        row.y + 5.0,
                        11.0,
                        color::ui_muted(),
                    );
                }
            }
            draw_marks(d, section, marks_x, row.y + 5.0);
            if is_open {
                for (line_index, line) in
                    open_lines(section, row.width, &measure).iter().enumerate()
                {
                    widgets::draw_text(
                        d,
                        font,
                        line,
                        text_x,
                        row.y + 20.0 + line_index as f32 * OPEN_LINE + 2.0,
                        TEXT_SIZE,
                        color::ui_ink(),
                    );
                }
            } else {
                let words = if section.summary.trim().is_empty() {
                    "(no words for this section)"
                } else {
                    section.summary.as_str()
                };
                widgets::draw_text(
                    d,
                    font,
                    &ellipsize(words, row.width - ROW_INSET - 2.0, &measure),
                    text_x,
                    row.y + 20.0,
                    TEXT_SIZE,
                    color::ui_muted(),
                );
            }
            self.widgets.hint(
                d,
                state,
                id,
                *row,
                &format!(
                    "Listen from {}: the playhead moves just inside this section",
                    span_clock(section.start_seconds)
                ),
            );
            if state.clicked {
                pressed = Some(*index);
            }
            row_report.push(format!(
                "row{index}@{:.0},{:.0},{:.0},{:.0}[{:.3}-{:.3}]",
                row.x, row.y, row.width, row.height, section.start_seconds, section.end_seconds
            ));
        }
        if let Some(tail) = layout.tail {
            let last = layout.first + layout.rows.len();
            let words = format!(
                "Sections {}-{} of {}; scroll the list for the rest",
                layout.first + 1,
                last,
                sections.len()
            );
            widgets::draw_text(
                d,
                font,
                &ellipsize(&words, tail.width, &measure),
                tail.x,
                tail.y + 2.0,
                TEXT_SIZE,
                color::ui_muted(),
            );
        }
        for (index, line) in apply.iter().enumerate() {
            widgets::draw_text(
                d,
                font,
                line,
                layout.apply.x,
                layout.apply.y + index as f32 * APPLY_LINE + (APPLY_LINE - 13.0) * 0.5,
                13.0,
                color::ui_ink(),
            );
        }

        let view = &mut self.assist_column.feelings;
        view.first = layout.first;
        view.list = layout.list;
        if let Some(index) = pressed {
            view.open = Some(index);
            let section = &sections[index];
            let seconds = section.seek_seconds();
            commands.push(ShellCommand::Seek(seconds));
            println!("assist feelings nav: row={index} seek={seconds:.3}");
        }
        let report = format!(
            "sections={} first={} shown={} now={} open={} reused={} head@{:.0},{:.0},{:.0},{:.0} \
             notes@{:.0},{:.0},{:.0},{:.0} map@{:.0},{:.0},{:.0},{:.0} \
             list@{:.0},{:.0},{:.0},{:.0} apply-lines={} rows={}",
            sections.len(),
            layout.first,
            layout.rows.len(),
            now.map_or_else(|| "none".to_string(), |row| row.to_string()),
            view.open
                .map_or_else(|| "none".to_string(), |row| row.to_string()),
            if candidate.reused_answer().is_some() {
                "yes"
            } else {
                "no"
            },
            layout.head.x,
            layout.head.y,
            layout.head.width,
            layout.head.height,
            layout.notes.x,
            layout.notes.y,
            layout.notes.width,
            layout.notes.height,
            map.x,
            map.y,
            map.width,
            map.height,
            layout.list.x,
            layout.list.y,
            layout.list.width,
            layout.list.height,
            apply.len(),
            row_report.join(";"),
        );
        if report != view.report {
            println!("assist feelings: {report}");
            // The notes' words where they are drawn, for the gate (J3-3).
            println!(
                "assist feelings notes: {}",
                if notes.is_empty() {
                    "none".to_string()
                } else {
                    notes.join(" ")
                }
            );
            view.report = report;
        }
    }
}

/// Widget index of the step back on the card, in [`ASSIST_WIDGETS`]: between
/// the review rows (`60..`) and the cards (`100..`).
pub(super) const STEP_BACK_WIDGET: u32 = 95;

/// What the card that applied feeling cues says about putting them back.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct StepBackLine {
    /// The card it sits on: the one whose Apply replaced the cues.
    pub mode: AssistMode,
    /// `Some(how many)` while the step back is offered; `None` once it is
    /// gone, when `text` says why.
    pub earlier: Option<usize>,
    pub applied: usize,
    pub text: String,
    /// The report's word: `offered`, `gone:closed`, `gone:edited`.
    pub token: &'static str,
}

/// The step back's line, from the session's memory and the live track, or
/// `None` when no Apply of feeling cues left one.
pub(super) fn step_back_line(workspace: &Workspace) -> Option<StepBackLine> {
    use musializer_core::project::analysis_candidate::{StepBackGone, StepBackStatus};
    let (mode, memory) = workspace.assist.feelings_step_back.as_ref()?;
    let track = workspace
        .get(memory.track)
        .map(|track| (track.file_path.as_path(), &track.semantic_events));
    let earlier = memory.earlier.len();
    let applied = memory.applied.len();
    Some(match memory.status(track) {
        StepBackStatus::Offered => StepBackLine {
            mode: *mode,
            earlier: Some(earlier),
            applied,
            text: if earlier == 0 {
                "Before Apply there were none".to_string()
            } else {
                format!(
                    "{earlier} earlier cue{} kept",
                    if earlier == 1 { "" } else { "s" }
                )
            },
            token: "offered",
        },
        StepBackStatus::Gone(gone) => StepBackLine {
            mode: *mode,
            earlier: None,
            applied,
            text: gone.sentence().to_string(),
            token: match gone {
                StepBackGone::TrackClosed => "gone:closed",
                StepBackGone::LaneChanged => "gone:edited",
            },
        },
    })
}

/// The step back's button label.
pub(super) const STEP_BACK_LABEL: &str = "Put back";

/// The button's rect at the right end of the card's step-back row.
pub(super) fn step_back_button(font: &UiFonts, row: UiRect) -> UiRect {
    let width = widgets::measure(font, STEP_BACK_LABEL, metric::UI_FONT_CAPTION) + 20.0;
    let height = 22.0;
    UiRect::new(
        row.x + row.width - width,
        row.y + (row.height - height) * 0.5,
        width,
        height,
    )
}

/// What Apply will do, said before it is done (J2): how many earlier feeling
/// cues it replaces, what else it replaces, and what it leaves alone.
///
/// Every clause is what `apply_candidate_to_track` does, read there
/// (2026-11-16): the semantic lane is replaced whole (`AnalysisCandidate::apply`),
/// the lyric lane and the scene plan only when the run staged them, and the
/// user's own markers (`manual_events`, where `+ Feel` records too) never.
/// The semantic lane holds only cues an Assist run or a bridge import wrote.
pub(super) fn apply_sentence(
    candidate: &AnalysisCandidate,
    target: Option<&Track>,
) -> Option<String> {
    let available = candidate.available();
    if !available.semantics {
        return None;
    }
    let staged = candidate.feeling_sections().len();
    let earlier = target.map_or(0, |track| track.semantic_events.len());
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut also = Vec::new();
    if available.lyrics {
        also.push("the lyrics");
    }
    if available.sections {
        also.push("the scene plan");
    }
    let what = if earlier == 0 {
        format!(
            "Apply adds the {staged} feeling cue{} from this answer",
            plural(staged)
        )
    } else {
        format!(
            "Apply replaces the {earlier} earlier feeling cue{} with the {staged} from this answer",
            plural(earlier)
        )
    };
    let what = match also.as_slice() {
        [] => what,
        [one] => format!("{what}, and {one}"),
        [first, second, ..] => format!("{what}, {first} and {second}"),
    };
    let mut kept = vec!["your own markers"];
    if !available.sections {
        kept.push("the scene plan");
    }
    if !available.lyrics {
        kept.push("the lyrics");
    }
    let kept = match kept.as_slice() {
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => String::new(),
    };
    let back = if earlier == 0 {
        String::new()
    } else {
        " You can put the earlier cues back afterwards.".to_string()
    };
    // Always "Your own markers ...": the list starts with them.
    let kept = kept.replacen("your", "Your", 1);
    Some(format!("{what}. {kept} stay as they are.{back}"))
}

impl Shell {
    /// Draws the step back on its card's row; the press was already claimed
    /// (`state`) before the card claimed its own. Returns the report line.
    pub(super) fn draw_step_back(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        line: &StepBackLine,
        row: UiRect,
        button: Option<(UiRect, widgets::ButtonState)>,
    ) -> String {
        let measure = |text: &str| widgets::measure(font, text, 13.0);
        let text_right = button.map_or(row.x + row.width, |(rect, _)| rect.x - 8.0);
        widgets::draw_text(
            d,
            font,
            &ellipsize(&line.text, (text_right - row.x).max(0.0), &measure),
            row.x,
            row.y + 3.0,
            13.0,
            if line.earlier.is_some() {
                color::ui_ink()
            } else {
                color::ui_muted()
            },
        );
        if let Some((rect, state)) = button {
            let look = widgets::button_look(ButtonStyle::Neutral, false, state.hovered);
            widgets::rounded_box(d, rect, 4.0, look.fill, look.edge, 1.0);
            let width = widgets::measure(font, STEP_BACK_LABEL, metric::UI_FONT_CAPTION);
            widgets::draw_text(
                d,
                font,
                STEP_BACK_LABEL,
                rect.x + (rect.width - width) * 0.5,
                rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5,
                metric::UI_FONT_CAPTION,
                look.ink,
            );
            let id = widgets::widget_id(ASSIST_WIDGETS, STEP_BACK_WIDGET);
            let earlier = line.earlier.unwrap_or(0);
            self.widgets.hint(
                d,
                state,
                id,
                rect,
                &format!(
                    "Puts back the {earlier} feeling cue{} Apply replaced; the {} from the answer leave the project",
                    if earlier == 1 { "" } else { "s" },
                    line.applied
                ),
            );
        }
        format!(
            "state={} card={} earlier={} applied={} button{}",
            line.token,
            card_token(line.mode),
            line.earlier
                .map_or_else(|| "-".to_string(), |count| count.to_string()),
            line.applied,
            button.map_or_else(
                || "=none".to_string(),
                |(rect, _)| format!(
                    "@{:.0},{:.0},{:.0},{:.0}",
                    rect.x, rect.y, rect.width, rect.height
                )
            ),
        )
    }
}

/// The feel type's chartreuse (`EventType::Semantic`), the colour of a
/// feeling cue on the timeline, for the rows' stripes and the staged cues.
/// On the light appearance's pale lane it is darkened to an olive that still
/// reads as the same hue: at full lightness a 2 px dash there measured close
/// to the waveform behind it (2026-11-16 capture).
fn feel_tint() -> raylib::prelude::Color {
    let base = raylib::prelude::Color::get_color(EventType::Semantic.rgba());
    let surface = color::ui_surface();
    let light = u32::from(surface.r) + u32::from(surface.g) + u32::from(surface.b) > 3 * 160;
    if light {
        widgets::brightness(base, -0.45)
    } else {
        base
    }
}

/// Widget index of the first staged cue's tooltip on the timeline, in
/// [`ASSIST_WIDGETS`]: one per staged cue, at most the bridge's 256.
const GHOST_WIDGET: u32 = 300;
/// Where a staged cue's ring sits: at the lane's bottom, where the placed
/// heads sit at its top, so two cues at one time still show both and the tick
/// labels along the top are never covered (2026-11-16 capture: a ring under
/// `02:00.000`'s plate). "What goes" hangs from the top, "what comes" stands
/// on the bottom.
const GHOST_HEAD_LIFT: f32 = 8.0;
const GHOST_HEAD_RADIUS: f32 = 4.5;
const GHOST_RING: f32 = 1.5;
/// The staged cue's line: dashes, so it can never be read as a placed cue's
/// solid line, of either lane.
const GHOST_DASH: f32 = 4.0;
const GHOST_DASH_GAP: f32 = 3.0;

impl Shell {
    /// The staged feeling cues on the timeline, while they are staged (J2):
    /// where Apply would put them, as ghosts — a dashed line standing on a
    /// thin ring at the lane's bottom edge — beside the placed cues they would replace,
    /// which stay as they are. So the difference is a picture: the solid
    /// lines are what goes, the dashed ones what comes, and the filled heads
    /// (the user's own markers) are not touched either way.
    ///
    /// Drawn after the placed markers and before the playhead, the order
    /// `event_markers` keeps for the same reasons. `hovered` is whether a
    /// placed marker already raised its tooltip this frame.
    pub(crate) fn staged_feeling_markers(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        strip: UiRect,
        duration: f64,
        hovered: bool,
    ) {
        let session = &input.workspace.assist;
        let staged = session
            .candidate
            .as_ref()
            .filter(|candidate| candidate.available().semantics)
            .filter(|_| session.candidate_track == input.workspace.current_index())
            .map(AnalysisCandidate::feeling_sections)
            .unwrap_or_default();
        let replaced = input
            .workspace
            .current()
            .filter(|_| !staged.is_empty())
            .map_or(0, |track| track.semantic_events.len());
        let colour = feel_tint();
        let mouse = input.ui_scale.mouse(d);
        let mut drawn = 0usize;
        let mut off_screen = 0usize;
        let mut tip: Option<(UiRect, String, usize)> = None;
        {
            let mut clip = widgets::begin_scissor(d, strip, input.ui_scale);
            for (index, section) in staged.iter().enumerate() {
                if section.start_seconds < 0.0 || section.start_seconds > duration {
                    off_screen += 1;
                    continue;
                }
                let x = self.timeline.x_at(
                    section.start_seconds,
                    f64::from(strip.x),
                    f64::from(strip.width),
                ) as f32;
                if x < strip.x - 8.0 || x > strip.x + strip.width + 8.0 {
                    off_screen += 1;
                    continue;
                }
                let x = x.round();
                let head_y = strip.y + strip.height - GHOST_HEAD_LIFT;
                let line_end = head_y - GHOST_HEAD_RADIUS - 2.0;
                let mut y = strip.y;
                while y < line_end {
                    let bottom = (y + GHOST_DASH).min(line_end);
                    widgets::fill(
                        &mut *clip,
                        UiRect::new(x - 1.0, y, 2.0, bottom - y),
                        widgets::alpha(colour, 0.8),
                    );
                    y += GHOST_DASH + GHOST_DASH_GAP;
                }
                let centre = raylib::prelude::Vector2::new(x, head_y);
                clip.draw_circle_v(centre, GHOST_HEAD_RADIUS, colour);
                clip.draw_circle_v(
                    centre,
                    GHOST_HEAD_RADIUS - GHOST_RING,
                    color::ui_lane_trough(),
                );
                drawn += 1;
                let hit = UiRect::new(
                    x - GHOST_HEAD_RADIUS - 1.0,
                    head_y - GHOST_HEAD_RADIUS - 1.0,
                    GHOST_HEAD_RADIUS * 2.0 + 2.0,
                    GHOST_HEAD_RADIUS * 2.0 + 2.0,
                );
                if tip.is_none() && !hovered && hit.contains_point(mouse.x, mouse.y) {
                    tip = Some((
                        hit,
                        format!(
                            "Staged feel  \u{00b7}  {}  \u{00b7}  not applied yet",
                            widgets::format_timestamp(section.start_seconds)
                        ),
                        index,
                    ));
                }
            }
        }
        if let Some((anchor, text, index)) = tip {
            self.widgets.hint(
                d,
                widgets::ButtonState {
                    hovered: true,
                    clicked: false,
                    pressed: false,
                },
                widgets::widget_id(ASSIST_WIDGETS, GHOST_WIDGET + index as u32),
                anchor,
                &text,
            );
        }
        let report = format!(
            "staged={} ghosts={drawn} off-screen={off_screen} replaced={replaced} track={}",
            staged.len(),
            session
                .candidate_track
                .filter(|_| !staged.is_empty())
                .map_or_else(|| "none".to_string(), |index| index.to_string()),
        );
        let view = &mut self.assist_column.feelings;
        if report != view.timeline_report {
            println!("staged feelings: {report}");
            view.timeline_report = report;
        }
    }
}

/// The three marks: energy and tension as lengths in a track, valence from a
/// centre line. Energy is a solid bar and tension a comb, so the two lengths
/// are not confused down the list even though they share a scale; valence
/// is the only signed value, so it grows left or right from the middle and
/// ends in a dot, never a bar from the left that would read "a little".
fn draw_marks(d: &mut RaylibDrawHandle<'_>, section: &FeelingSection, x: f32, y: f32) {
    let track = |d: &mut RaylibDrawHandle<'_>, left: f32| {
        widgets::fill(
            d,
            UiRect::new(left, y + 2.0, MARK_WIDTH, 6.0),
            color::ui_lane_trough(),
        );
    };
    // Energy: a solid bar.
    track(d, x);
    let energy = (section.energy.clamp(0.0, 1.0) * MARK_WIDTH).round();
    if energy > 0.0 {
        widgets::fill(d, UiRect::new(x, y + 2.0, energy, 6.0), color::ui_level());
    }
    // Tension: the same scale as a comb of 2 px teeth every 3 px.
    let left = x + MARK_WIDTH + MARK_GAP;
    track(d, left);
    let tension = (section.tension.clamp(0.0, 1.0) * MARK_WIDTH).round();
    let mut tooth = 0.0;
    while tooth + 2.0 <= tension {
        widgets::fill(
            d,
            UiRect::new(left + tooth, y + 1.0, 2.0, 8.0),
            color::ui_ink(),
        );
        tooth += 3.0;
    }
    // Mood (valence, -1..1): a stem from the centre line to a dot.
    let left = x + (MARK_WIDTH + MARK_GAP) * 2.0;
    let centre = (left + MARK_WIDTH * 0.5).round();
    widgets::fill(
        d,
        UiRect::new(centre, y, 1.0, 10.0),
        color::ui_selected_edge(),
    );
    let reach = (section.valence.clamp(-1.0, 1.0) * (MARK_WIDTH * 0.5 - 3.0)).round();
    let end = centre + reach;
    let (from, to) = if reach >= 0.0 {
        (centre, end)
    } else {
        (end, centre)
    };
    if to > from {
        widgets::fill(
            d,
            UiRect::new(from, y + 4.0, to - from, 2.0),
            color::ui_ink(),
        );
    }
    d.draw_circle_v(
        raylib::prelude::Vector2::new(end + 0.5, y + 5.0),
        3.0,
        color::ui_ink(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(start: f64, end: f64) -> FeelingSection {
        FeelingSection {
            event_id: 1,
            start_seconds: start,
            end_seconds: end,
            energy: 0.5,
            tension: 0.5,
            valence: 0.0,
            confidence: 0.9,
            summary: String::new(),
        }
    }

    /// The sentence names what the run staged and nothing else, and every
    /// clause is pinned against what `apply_candidate_to_track` does by
    /// `a_feelings_apply_touches_neither_markers_nor_plan_and_steps_back_exactly`.
    #[test]
    fn the_apply_sentence_says_what_goes_and_what_stays() {
        use musializer_core::project::analysis_bridge;
        let digest = musializer_core::project::sha256::digest_hex(b"audio");
        // "a line", ["x"], "calm", "bright", in base64.
        let document = format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             LYRIC\t1\t1000\t2000\t900\tnone\tYSBsaW5l\n\
             SECTION\t10\t0\t60000\tspectrum\t500\tWyJ4Il0=\n\
             SEMANTIC\t20\t0\t30000\t400\t300\t-200\t900\tY2FsbQ==\n\
             SEMANTIC\t21\t30000\t60000\t800\t700\t600\t950\tYnJpZ2h0\n"
        );
        let bridge = analysis_bridge::parse(document.as_bytes(), None, None).unwrap();
        let prepare = |mode| {
            AnalysisCandidate::prepare(&bridge, assist_ui_state::mode_lanes(mode), 60.0, 4).unwrap()
        };
        assert_eq!(
            apply_sentence(&prepare(AssistMode::Mimo), None).as_deref(),
            Some("Apply adds the 2 feeling cues from this answer. Your own markers, the scene plan and the lyrics stay as they are.")
        );
        assert_eq!(
            apply_sentence(&prepare(AssistMode::All), None).as_deref(),
            Some("Apply adds the 2 feeling cues from this answer, the lyrics and the scene plan. Your own markers stay as they are.")
        );
        assert_eq!(apply_sentence(&prepare(AssistMode::Lyrics), None), None);
    }

    #[test]
    fn a_span_reads_seconds_and_tenths_only_where_the_model_put_them() {
        assert_eq!(span_text(&section(24.0, 58.0)), "0:24-0:58");
        assert_eq!(span_text(&section(22.5, 33.0)), "0:22.5-0:33");
        assert_eq!(span_text(&section(199.0, 260.0)), "3:19-4:20");
    }

    #[test]
    fn the_row_under_the_playhead_is_the_lanes_own_answer() {
        let sections = [section(0.0, 10.0), section(10.0, 22.5)];
        assert_eq!(row_at(&sections, 0.0), Some(0));
        assert_eq!(row_at(&sections, 9.999), Some(0));
        assert_eq!(
            row_at(&sections, 10.0),
            Some(1),
            "the boundary is the next section's"
        );
        assert_eq!(row_at(&sections, 22.5), None, "past the end is no row");
        assert_eq!(row_at(&sections, f64::NAN), None);
    }

    /// Eleven resting rows: five on screen, the window fixed, the last page
    /// full and a tail saying so; scrolling past the end lands on the last page.
    #[test]
    fn a_long_answer_scrolls_inside_a_fixed_window() {
        let heights = vec![ROW_HEIGHT; 11];
        let top = geometry(10.0, 100.0, 300.0, &heights, 0, 2, 0);
        assert_eq!(top.rows.len(), LIST_MAX_ROWS);
        assert_eq!(top.list.height, LIST_MAX_ROWS as f32 * ROW_HEIGHT);
        assert!(top.tail.is_some());
        let scrolled = geometry(10.0, 100.0, 300.0, &heights, 99, 2, 0);
        assert_eq!(scrolled.first, 6, "the last page starts at 11 - 5");
        assert_eq!(scrolled.rows.first().map(|row| row.0), Some(6));
        assert_eq!(scrolled.rows.last().map(|row| row.0), Some(10));
        assert_eq!(scrolled.height, top.height, "scrolling never moves Apply");
        assert_eq!(scrolled.apply.y, top.apply.y);
        // Rows touch, and none leaves the window.
        for pair in top.rows.windows(2) {
            assert_eq!(pair[0].1.y + pair[0].1.height, pair[1].1.y);
        }
        let last = top.rows.last().unwrap().1;
        assert!(last.y + last.height <= top.list.y + top.list.height + 0.01);
    }

    #[test]
    fn a_short_answer_shows_every_row_and_no_tail() {
        let heights = vec![ROW_HEIGHT; 4];
        let layout = geometry(0.0, 0.0, 300.0, &heights, 3, 0, 0);
        assert_eq!(layout.first, 0);
        assert_eq!(layout.rows.len(), 4);
        assert_eq!(layout.tail, None);
        assert_eq!(layout.apply.height, 0.0);
    }

    /// An open row is taller; the window grows by its extra lines, so the
    /// same number of rows stays on screen whichever row is open.
    #[test]
    fn an_open_row_grows_the_window_rather_than_pushing_rows_out() {
        let mut heights = vec![ROW_HEIGHT; 11];
        heights[2] = row_height(4);
        let layout = geometry(0.0, 0.0, 300.0, &heights, 0, 0, 0);
        assert_eq!(layout.rows.len(), LIST_MAX_ROWS);
        assert_eq!(layout.rows[2].1.height, row_height(4));
        assert_eq!(
            layout.list.height,
            LIST_MAX_ROWS as f32 * ROW_HEIGHT + row_height(4) - ROW_HEIGHT
        );
    }

    #[test]
    fn an_open_row_carries_the_whole_text_up_to_its_cap() {
        let mut long = section(0.0, 10.0);
        long.summary = "word ".repeat(200);
        let measure = |text: &str| text.chars().count() as f32 * 6.0;
        let lines = open_lines(&long, 300.0, &measure);
        assert_eq!(lines.len(), OPEN_MAX_LINES);
        assert!(lines.last().unwrap().ends_with("..."));
        let mut short = section(0.0, 10.0);
        short.summary = "A brief re-escalation.".to_string();
        assert_eq!(
            open_lines(&short, 300.0, &measure),
            ["A brief re-escalation."]
        );
    }
}
