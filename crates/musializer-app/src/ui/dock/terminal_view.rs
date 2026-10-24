//! Draws a [`Session`]'s grid with egui and turns egui input into the bytes
//! the program expects (WX-2).
//!
//! **One galley per run, not per cell.** A run is a stretch of one row that
//! shares a colour and holds only ASCII. egui's bundled monospace face (Hack)
//! advances every ASCII glyph by exactly one cell, so a run's characters land
//! on the grid by themselves. Anything else (box drawing from a fallback,
//! an emoji, a CJK glyph twice as wide) is placed on its own at its cell's x.
//! That way a fallback glyph with a different advance cannot push the rest of
//! the row off the grid. The galleys are cached by egui, so an unchanged
//! screen costs almost nothing to redraw.

use musializer_runtime::terminal::alacritty_terminal;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use egui::{Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use musializer_runtime::terminal::{self, GridSize, Session};

use super::keys;
use crate::ui::theme::color32;

/// The type size, in egui points. 13 fits 80 columns into a 640-point dock.
pub const FONT_SIZE: f32 = 13.0;

/// Lines per wheel notch, as most terminals do.
const WHEEL_LINES: f32 = 3.0;

fn color(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// What one frame of the view did.
#[derive(Default)]
pub struct ViewOutput {
    pub has_focus: bool,
    /// Ctrl+` pressed while the terminal held the keyboard. The dock hides,
    /// which the composition root does, so the key means one thing everywhere.
    pub toggle_dock: bool,
    /// Enter pressed on a terminal whose program has exited.
    pub restart: bool,
}

#[derive(Default)]
pub struct TerminalView {
    /// A paste is announced by the `V` key event that precedes it; this is
    /// whether that key was Ctrl+Shift+V (paste) or Ctrl+V (`^V`).
    paste_armed: bool,
    wheel: f32,
    /// Ask for focus on the next frame, for a tab that was just opened.
    pub focus_next: bool,
}

struct Metrics {
    font: FontId,
    cell: Vec2,
}

fn metrics(ctx: &egui::Context) -> Metrics {
    let font = FontId::monospace(FONT_SIZE);
    let (width, height) =
        ctx.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    Metrics {
        font,
        cell: Vec2::new(width.max(1.0), height.max(1.0)),
    }
}

impl TerminalView {
    pub fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, session: &mut Session) -> ViewOutput {
        let mut output = ViewOutput::default();
        let rect = ui.available_rect_before_wrap();
        let response = ui.interact(rect, id, Sense::click_and_drag());
        ui.advance_cursor_after_rect(rect);
        let metrics = metrics(ui.ctx());
        let ppp = ui.ctx().pixels_per_point();
        let size = GridSize {
            columns: ((rect.width() / metrics.cell.x).floor() as u16).max(2),
            lines: ((rect.height() / metrics.cell.y).floor() as u16).max(1),
            cell_width: (metrics.cell.x * ppp).round() as u16,
            cell_height: (metrics.cell.y * ppp).round() as u16,
        };
        session.resize(size);
        let activity = session.poll();
        if let Some(text) = activity.clipboard {
            ui.ctx().copy_text(text);
        }
        if self.focus_next {
            response.request_focus();
            self.focus_next = false;
        }
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.clicked_elsewhere() && response.has_focus() {
            response.surrender_focus();
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }

        self.mouse(ui, &response, rect, &metrics, session);
        output.has_focus = response.has_focus();
        if output.has_focus {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
            self.keyboard(ui, session, &mut output);
        }
        if ui
            .ctx()
            .input(|input| input.pointer.is_decidedly_dragging())
            && response.dragged()
        {
            ui.ctx().request_repaint();
        }
        paint(ui, rect, &metrics, session, output.has_focus);
        output
    }

    fn point_at(
        &self,
        pos: Pos2,
        rect: Rect,
        metrics: &Metrics,
        session: &Session,
    ) -> (Point, Side) {
        let term = session.term().lock();
        let offset = term.grid().display_offset() as i32;
        let columns = term.columns();
        let lines = term.screen_lines();
        drop(term);
        let x = ((pos.x - rect.left()) / metrics.cell.x).max(0.0);
        let y = ((pos.y - rect.top()) / metrics.cell.y).max(0.0);
        let column = (x.floor() as usize).min(columns.saturating_sub(1));
        let row = (y.floor() as usize).min(lines.saturating_sub(1)) as i32;
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (Point::new(Line(row - offset), Column(column)), side)
    }

    fn mouse(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        metrics: &Metrics,
        session: &mut Session,
    ) {
        // Selection: press-drag selects characters, a double click a word.
        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() {
                let (point, side) = self.point_at(pos, rect, metrics, session);
                session.term().lock().selection =
                    Some(Selection::new(SelectionType::Simple, point, side));
            } else if response.dragged() {
                let (point, side) = self.point_at(pos, rect, metrics, session);
                if let Some(selection) = session.term().lock().selection.as_mut() {
                    selection.update(point, side);
                }
            } else if response.double_clicked() {
                let (point, side) = self.point_at(pos, rect, metrics, session);
                let mut term = session.term().lock();
                let mut selection = Selection::new(SelectionType::Semantic, point, side);
                selection.update(point, side);
                term.selection = Some(selection);
            } else if response.clicked() {
                session.term().lock().selection = None;
            }
        }

        if !response.hovered() {
            self.wheel = 0.0;
            return;
        }
        let notches: f32 = ui.ctx().input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum()
        });
        self.wheel += notches * WHEEL_LINES;
        let lines = self.wheel.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.wheel -= lines as f32;
        let mode = *session.term().lock().mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // A full-screen program gets arrow keys, as alacritty does, so
            // the wheel scrolls `less` and the agent transcripts.
            let key = if lines > 0 {
                Key::ArrowUp
            } else {
                Key::ArrowDown
            };
            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            if let Some(bytes) = keys::key_bytes(key, Modifiers::NONE, app_cursor) {
                let mut all = Vec::new();
                for _ in 0..lines.unsigned_abs() {
                    all.extend_from_slice(&bytes);
                }
                session.write(all);
            }
        } else {
            session.term().lock().scroll_display(Scroll::Delta(lines));
        }
    }

    fn keyboard(&mut self, ui: &egui::Ui, session: &mut Session, output: &mut ViewOutput) {
        let mode = *session.term().lock().mode();
        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let exited = session.exit().is_some();
        let mut sent = false;
        // Taken out of the frame's input, so nothing built after the dock
        // (the Editor, whose Escape closes it) sees a key the terminal used.
        let events = ui
            .ctx()
            .input_mut(|input| std::mem::take(&mut input.events));
        let mut keep = Vec::with_capacity(events.len());
        for event in events {
            match &event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let (key, modifiers) = (*key, *modifiers);
                    if modifiers.ctrl && key == Key::Backtick {
                        output.toggle_dock = true;
                        continue;
                    }
                    if exited {
                        output.restart |= key == Key::Enter;
                        continue;
                    }
                    if modifiers.ctrl && modifiers.shift && key == Key::C {
                        if let Some(text) = session.term().lock().selection_to_string() {
                            ui.ctx().copy_text(text);
                        }
                        continue;
                    }
                    if modifiers.ctrl && key == Key::V {
                        self.paste_armed = modifiers.shift;
                        if modifiers.shift {
                            continue;
                        }
                    }
                    if let Some(bytes) = keys::key_bytes(key, modifiers, app_cursor) {
                        session.write(bytes);
                        sent = true;
                    }
                }
                egui::Event::Key { pressed: false, .. } => {}
                egui::Event::Text(text) => {
                    if !exited {
                        session.write(text.clone().into_bytes());
                        sent = true;
                    }
                }
                egui::Event::Paste(text) => {
                    if self.paste_armed && !exited {
                        let bracketed = mode.contains(TermMode::BRACKETED_PASTE);
                        session.write(keys::paste_bytes(text, bracketed));
                        sent = true;
                    }
                    self.paste_armed = false;
                }
                egui::Event::Copy | egui::Event::Cut => {}
                _ => keep.push(event),
            }
        }
        ui.ctx().input_mut(|input| input.events = keep);
        if sent {
            // Typing returns to the live screen, as every terminal does.
            session.term().lock().scroll_display(Scroll::Bottom);
        }
    }
}

/// The resolved colours of one cell.
fn cell_colors(
    fg: Color,
    bg: Color,
    flags: Flags,
    selected: bool,
    overrides: &alacritty_terminal::term::color::Colors,
) -> (Rgb, Rgb) {
    let mut fg = terminal::resolve(fg, overrides);
    let bg_rgb = terminal::resolve(bg, overrides);
    if flags.contains(Flags::DIM) {
        fg = terminal::dim(fg);
    }
    let (mut fg, mut bg) = (fg, bg_rgb);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if selected {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(Flags::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

fn paint(ui: &egui::Ui, rect: Rect, metrics: &Metrics, session: &Session, focused: bool) {
    let painter = ui.painter_at(rect);
    let term = session.term().lock();
    let content = term.renderable_content();
    let overrides = content.colors;
    let background = terminal::resolve(Color::Named(NamedColor::Background), overrides);
    painter.rect_filled(rect, 0.0, color(background));
    let offset = content.display_offset as i32;
    let origin = rect.min;
    let cell = metrics.cell;
    let at = |line: i32, column: usize| {
        Pos2::new(
            origin.x + column as f32 * cell.x,
            origin.y + (line + offset) as f32 * cell.y,
        )
    };

    // Text runs are flushed whenever the style changes, the row changes, or a
    // non-ASCII glyph needs placing on its own.
    let mut run = String::new();
    let mut run_start: Option<(i32, usize, Rgb, bool)> = None;
    let flush = |run: &mut String, start: &mut Option<(i32, usize, Rgb, bool)>| {
        if let Some((line, column, fg, underline)) = start.take() {
            let text = run.trim_end();
            if !text.is_empty() {
                let pos = at(line, column);
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    text,
                    metrics.font.clone(),
                    color(fg),
                );
                if underline {
                    let y = pos.y + cell.y - 1.5;
                    let width = text.chars().count() as f32 * cell.x;
                    painter.line_segment(
                        [Pos2::new(pos.x, y), Pos2::new(pos.x + width, y)],
                        Stroke::new(1.0, color(fg)),
                    );
                }
            }
        }
        run.clear();
    };

    let selection = content.selection;
    let cursor = content.cursor;
    for indexed in content.display_iter {
        let point = indexed.point;
        let cell_ref = indexed.cell;
        let flags = cell_ref.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER)
            || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let selected = selection.is_some_and(|range| range.contains(point));
        let (fg, bg) = cell_colors(cell_ref.fg, cell_ref.bg, flags, selected, overrides);
        let line = point.line.0;
        let column = point.column.0;
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        if bg != background {
            painter.rect_filled(
                Rect::from_min_size(at(line, column), Vec2::new(cell.x * width, cell.y)),
                0.0,
                color(bg),
            );
        }
        let underline = flags.intersects(Flags::ALL_UNDERLINES);
        let ch = cell_ref.c;
        let contiguous = run_start.is_some_and(|(l, c, f, u)| {
            l == line && c + run.chars().count() == column && f == fg && u == underline
        });
        if ch.is_ascii() && !ch.is_ascii_control() {
            if !contiguous {
                flush(&mut run, &mut run_start);
                run_start = Some((line, column, fg, underline));
            }
            run.push(ch);
        } else {
            flush(&mut run, &mut run_start);
            if ch != ' ' && ch != '\0' {
                let centre = at(line, column) + Vec2::new(cell.x * width * 0.5, 0.0);
                painter.text(
                    centre,
                    egui::Align2::CENTER_TOP,
                    ch.to_string(),
                    metrics.font.clone(),
                    color(fg),
                );
            }
        }
    }
    flush(&mut run, &mut run_start);

    if cursor.shape != CursorShape::Hidden && session.exit().is_none() {
        let pos = at(cursor.point.line.0, cursor.point.column.0);
        let cursor_color = color(terminal::resolve(
            Color::Named(NamedColor::Cursor),
            overrides,
        ));
        let block = Rect::from_min_size(pos, cell);
        match (focused, cursor.shape) {
            (true, CursorShape::Beam) => {
                painter.rect_filled(
                    Rect::from_min_size(pos, Vec2::new(2.0, cell.y)),
                    0.0,
                    cursor_color,
                );
            }
            (true, CursorShape::Underline) => {
                painter.rect_filled(
                    Rect::from_min_size(pos + Vec2::new(0.0, cell.y - 2.0), Vec2::new(cell.x, 2.0)),
                    0.0,
                    cursor_color,
                );
            }
            (true, _) => {
                painter.rect_filled(block, 0.0, cursor_color);
                let ch = term.grid()[cursor.point].c;
                if ch != ' ' && ch != '\0' {
                    painter.text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        ch.to_string(),
                        metrics.font.clone(),
                        color(background),
                    );
                }
            }
            (false, _) => {
                painter.rect_stroke(
                    block.shrink(0.5),
                    0.0,
                    Stroke::new(1.0, cursor_color),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
    drop(term);
    if let Some(code) = session.exit() {
        let message = match code {
            Some(code) => format!("exited with status {code} · Enter starts it again"),
            None => "ended by a signal · Enter starts it again".to_string(),
        };
        let bar = Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - cell.y - 8.0),
            Vec2::new(rect.width(), cell.y + 8.0),
        );
        // The terminal is dark in both appearances — its sixteen colours are
        // tuned to a dark ground — so its status bar takes the dark palette's
        // roles rather than the current appearance's (WX-5a).
        let dark = &crate::ui::theme::palette::DARK;
        painter.rect_filled(bar, 0.0, color32::of(dark.ui_hover));
        painter.text(
            bar.left_center() + Vec2::new(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            message,
            metrics.font.clone(),
            color32::of(dark.ui_muted),
        );
    }
}

/// The text on screen, row by row, for the report line and the gate.
#[must_use]
pub fn screen_text(session: &Session) -> String {
    let term = session.term().lock();
    let grid = term.grid();
    let mut out = String::new();
    for line in 0..grid.screen_lines() {
        let row = &grid[Line(line as i32)];
        let text: String = (0..grid.columns())
            .map(|column| row[Column(column)].c)
            .collect();
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}
