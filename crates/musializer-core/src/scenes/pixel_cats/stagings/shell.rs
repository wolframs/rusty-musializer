//! Root shell: a lyric editor in a terminal, and the music writes the tags.
//!
//! Every line is typed into `LYRICS.TXT` behind a line number, the lines
//! already sung scroll up above it, and a cat at the keyboard taps along on
//! the hi-hats. Above a line the music may write a stage direction, decided
//! as the line starts (`PixelCatsState::tag_for`) — and the tag takes the text
//! over: `[BELTED]` sets it large and red and shakes it on the kicks,
//! `[WHISPERED]` lets it fade, `[FALSETTO]` stretches it tall and wobbling,
//! `[GABBER]` cycles its colours and bounces its letters, and a line that asks
//! for `sudo` or `root` is answered `ACCESS GRANTED`. A line may name its own
//! tag by starting with one (`[BELTED] I typed belted`).
//!
//! With no line on screen the editor shows its prompt and an ASCII meter of
//! the spectrum under an `[INSTRUMENTAL]` tag, so an instrumental stretch
//! still moves.

use super::super::font::{self, Row};
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{self, Look};
use super::super::{mix, unit, Ask, Line, Moment, PixelReport};
use super::{blinking, cat, cursor_lit, say, say_centred};

const DESK: Rgb = Rgb::new(16, 40, 48);
const DESK_DOT: Rgb = Rgb::new(24, 54, 62);
const WINDOW: Rgb = Rgb::new(22, 24, 36);
const TITLE: Rgb = Rgb::new(70, 80, 140);
const TEXT: Rgb = Rgb::new(232, 232, 244);
const DIM: Rgb = Rgb::new(118, 122, 150);
const NUMBER: Rgb = Rgb::new(80, 84, 112);
const TAG: Rgb = Rgb::new(255, 210, 80);
const GREEN: Rgb = Rgb::new(130, 240, 150);

/// One row of the editor: a number (or none for a continuation), its text.
struct Entry {
    number: Option<usize>,
    text: String,
    color: Rgb,
}

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    let portrait = h > w;
    raster.fill(DESK);
    for y in (0..h).step_by(4) {
        for x in ((y / 4 % 2) * 2..w).step_by(4) {
            raster.put(x, y, DESK_DOT);
        }
    }
    let (wx, wy) = (6, 6);
    let (ww, wh) = if portrait {
        (w - 12, (h as f32 * 0.70) as i32)
    } else {
        (w - 12 - 64, h - 12)
    };
    // A BELTED line shakes the whole window on its kicks.
    let belted = moment
        .line
        .as_ref()
        .is_some_and(|l| l.tag.as_deref() == Some("BELTED"));
    let tick = (moment.time * 30.0) as u64;
    let jolt = if belted && moment.kick > 0.25 {
        ((unit(mix(moment.seed ^ tick), 1) * 2.0 - 1.0) * moment.kick * 2.0).round() as i32
    } else {
        0
    };
    raster.rect(wx + 3, wy + 3, ww, wh, Rgb::new(6, 12, 16));
    raster.rect(wx + jolt, wy, ww, wh, WINDOW);
    raster.rect(wx + jolt, wy, ww, 10, TITLE);
    say(
        raster,
        report,
        "LYRICS.TXT - CATEDIT",
        wx + jolt + 4,
        wy + 2,
        1,
        1,
        TEXT,
    );
    for (k, color) in [
        Rgb::new(240, 90, 90),
        Rgb::new(240, 200, 90),
        Rgb::new(110, 220, 110),
    ]
    .into_iter()
    .enumerate()
    {
        raster.rect(wx + jolt + ww - 10 - k as i32 * 8, wy + 3, 5, 5, color);
    }

    let content_x = wx + jolt + 20;
    let content_w = ww - 24;
    let columns = ((content_w + 1) / font::ADVANCE).max(4) as usize;
    let top = wy + 14;
    let bottom = wy + wh - 4;

    // The history, newest last, numbered from the first line this editor saw.
    let mut entries: Vec<Entry> = Vec::new();
    for (index, sung) in moment.history.iter().enumerate() {
        if let Some(tag) = &sung.tag {
            entries.push(Entry {
                number: None,
                text: format!("[{tag}]"),
                color: TAG.scale(0.7),
            });
        }
        for (k, row) in font::wrap(&sung.text, columns).into_iter().enumerate() {
            entries.push(Entry {
                number: (k == 0).then_some(index + 1),
                text: row.text,
                color: DIM,
            });
        }
    }
    let number = moment.history.len() + 1;

    // How tall the current line is, so the history scrolls to make room.
    let current = moment
        .line
        .as_ref()
        .map(|line| Current::layout(line, content_w));
    let current_height = match &current {
        Some(layout) => {
            layout.height
                + if layout.line_tag.is_some() {
                    font::LINE
                } else {
                    0
                }
        }
        None => font::LINE * 2,
    };
    let history_rows = ((bottom - top - current_height) / font::LINE).max(0) as usize;
    let skip = entries.len().saturating_sub(history_rows);
    let mut y = top;
    for entry in entries.iter().skip(skip) {
        if let Some(n) = entry.number {
            say(
                raster,
                report,
                &format!("{n:2}"),
                wx + jolt + 3,
                y,
                1,
                1,
                NUMBER,
            );
        }
        say(raster, report, &entry.text, content_x, y, 1, 1, entry.color);
        y += font::LINE;
    }

    let mut ask = Ask::default();
    match (&moment.line, current) {
        (Some(line), Some(layout)) => {
            say(
                raster,
                report,
                &format!("{number:2}"),
                wx + jolt + 3,
                y + if layout.line_tag.is_some() {
                    font::LINE
                } else {
                    0
                },
                1,
                1,
                NUMBER,
            );
            ask = layout.draw(
                raster,
                report,
                moment,
                line,
                content_x,
                y,
                (wx + jolt, wy, ww, wh),
            );
        }
        _ if moment
            .history
            .last()
            .is_some_and(|sung| sung.tag.as_deref() == Some("LOGOUT")) =>
        {
            // Logged out: a login prompt nobody answers, and the ASCII cat
            // asleep in the middle of the screen.
            let end = say(raster, report, "LOGIN: ", content_x, y, 1, 1, GREEN);
            if cursor_lit(moment, false) {
                raster.rect(content_x + end + 1, y, 5, 7, GREEN);
            }
            let scale = if content_w >= font::width("/\\_/\\", 3) + 20 {
                3
            } else {
                2
            };
            let middle = (y + font::LINE + bottom) / 2 - font::LINE * scale;
            let centre = content_x + content_w / 2;
            say_centred(
                raster,
                report,
                sprites::ASCII_CAT[0],
                centre,
                middle,
                scale,
                scale,
                GREEN,
                None,
            );
            say_centred(
                raster,
                report,
                sprites::ASCII_ASLEEP,
                centre,
                middle + (font::LINE - 1) * scale,
                scale,
                scale,
                GREEN,
                None,
            );
            report.cats += 1;
        }
        _ if moment.quiet < 2.5 && !moment.history.is_empty() => {
            // Between two lines: the prompt waits for the next one.
            let end = say(raster, report, "CAT@CATOS:~$ ", content_x, y, 1, 1, GREEN);
            if cursor_lit(moment, false) {
                raster.rect(content_x + end + 1, y, 5, 7, GREEN);
            }
        }
        _ => idle(raster, report, moment, content_x, y, content_w, bottom),
    }
    // Logged out: the cat sleeps once the session has shut down.
    let asleep = match &moment.line {
        Some(line) => {
            line.tag.as_deref() == Some("LOGOUT")
                && line.complete()
                && line.elapsed - line.revealed_at > LOGOUT_SLEEP
        }
        None => moment
            .history
            .last()
            .is_some_and(|sung| sung.tag.as_deref() == Some("LOGOUT")),
    };

    // The cat at its keyboard.
    let scale = if portrait { 2 } else { 3 };
    let size = 16 * scale;
    let (cx, cy) = if portrait {
        (w / 2 - size / 2, wy + wh + (h - wy - wh - size) / 2 - 4)
    } else {
        (w - 6 - size - 4, h - 18 - size)
    };
    let desk_y = cy + size - 2;
    raster.rect(cx - 8, desk_y, size + 16, 8, Rgb::new(60, 60, 82));
    for k in 0..((size + 12) / 6) {
        raster.rect(cx - 6 + k * 6, desk_y + 2, 5, 3, Rgb::new(150, 150, 172));
    }
    let mut look = Look::cat(Rgb::new(18, 16, 26), Rgb::new(200, 240, 90));
    look.rim = Some(Rgb::new(140, 200, 220));
    look.open = if asleep || blinking(moment, 3) {
        0.0
    } else {
        1.0
    };
    look.look = if portrait { 0 } else { -1 };
    cat(raster, report, sprites::SIT, cx, cy, scale, false, &look);
    // Paws: one taps on each hi-hat, the other on each kick.
    let tap = moment.ticks.iter().any(|e| e.age < 0.07);
    let thump = moment.kicks.iter().any(|e| e.age < 0.07);
    let paw = Rgb::new(18, 16, 26);
    raster.rect(
        cx + 2 * scale + if tap { 2 } else { 0 },
        desk_y - if tap { 2 } else { 0 },
        2 * scale,
        scale,
        paw,
    );
    raster.rect(
        cx + 12 * scale - if thump { 2 } else { 0 },
        desk_y - if thump { 2 } else { 0 },
        2 * scale,
        scale,
        paw,
    );
    if asleep {
        // Z, z, Z rising from the sleeping cat, each on its own clock.
        for k in 0..3 {
            let age = (moment.clock - k as f32 * 0.7).rem_euclid(2.1);
            let scale = 1 + (k % 2);
            let zx = cx + size - 4 + (age * 8.0) as i32;
            let zy = cy - 2 - (age * 12.0) as i32;
            say(
                raster,
                report,
                "Z",
                zx,
                zy,
                scale,
                scale,
                TEXT.scale(1.0 - age / 2.4),
            );
        }
    }
    report.detail = format!(
        "history={} rows={}{}",
        moment.history.len(),
        entries.len() - skip,
        if asleep { " asleep" } else { "" }
    );
    ask
}

/// Seconds after a logout line lands before the cat falls asleep: the three
/// shutdown messages' worth.
const LOGOUT_SLEEP: f32 = 1.6;

/// What the session prints as it shuts down.
const SHUTDOWN: [&str; 3] = [
    "SAVING 9 LIVES ...... OK",
    "UNMOUNTING /DEV/MEOW  OK",
    "RELEASING ROOT ...... OK",
];

/// With no line: the prompt, and the spectrum as a column meter of `#`.
fn idle(
    raster: &mut Raster,
    report: &mut PixelReport,
    moment: &Moment,
    x: i32,
    y: i32,
    width: i32,
    bottom: i32,
) {
    say(raster, report, "[INSTRUMENTAL]", x, y, 1, 1, TAG);
    let prompt = "CAT@CATOS:~$ ";
    let end = say(raster, report, prompt, x, y + font::LINE, 1, 1, GREEN);
    if cursor_lit(moment, false) {
        raster.rect(x + end + 1, y + font::LINE, 5, 7, GREEN);
    }
    let columns = (width / font::ADVANCE).max(1) as usize;
    let rows = ((bottom - (y + font::LINE * 3)) / (font::LINE - 1)).clamp(1, 8);
    let base = bottom - font::GLYPH_HEIGHT;
    let bands = &moment.bands;
    for column in 0..columns {
        let level = if bands.is_empty() {
            0.0
        } else {
            let a = column * bands.len() / columns;
            let b = ((column + 1) * bands.len() / columns)
                .max(a + 1)
                .min(bands.len());
            bands[a..b].iter().sum::<f32>() / (b - a) as f32
        };
        let height = (level * rows as f32 * 1.15).round() as i32;
        for k in 0..height.min(rows) {
            let color = if k >= rows - 1 {
                Rgb::new(255, 90, 90)
            } else if k >= rows * 2 / 3 {
                TAG
            } else {
                GREEN
            };
            font::glyph(
                raster,
                '#',
                x + column as i32 * font::ADVANCE,
                base - k * (font::LINE - 1),
                1,
                1,
                color,
            );
            report.glyphs += 1;
        }
    }
}

/// How the current line is set, from its tag.
struct Current {
    line_tag: Option<String>,
    sx: i32,
    sy: i32,
    rows: Vec<Row>,
    height: i32,
}

impl Current {
    fn layout(line: &Line, width: i32) -> Self {
        let (sx, sy) = match line.tag.as_deref() {
            Some("BELTED") => {
                let (scale, _) = font::fit(&line.text, width, 60, 3, 3);
                (scale.max(2), scale.max(2))
            }
            Some("GABBER") => (2, 2),
            Some("FALSETTO") => (1, 3),
            Some("WHISPERED") | Some("LOGOUT") => (1, 1),
            // Plain type is set large when it fits, so the line being typed
            // reads across a room and the history under it stays small.
            _ => {
                let (scale, _) = font::fit(&line.text, width, 40, 3, 2);
                (scale, scale)
            }
        };
        let columns = ((width + sx) / (font::ADVANCE * sx)).max(1) as usize;
        let rows = font::wrap(&line.text, columns);
        let height = rows.len() as i32 * (font::LINE * sy).max(font::LINE);
        Self {
            line_tag: line.tag.clone(),
            sx,
            sy,
            rows,
            height,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        raster: &mut Raster,
        report: &mut PixelReport,
        moment: &Moment,
        line: &Line,
        x: i32,
        mut y: i32,
        window: (i32, i32, i32, i32),
    ) -> Ask {
        let mut ask = Ask::default();
        if let Some(tag) = &self.line_tag {
            say(raster, report, &format!("[{tag}]"), x, y, 1, 1, TAG);
            y += font::LINE;
        }
        // A key change lifts the page as the line starts.
        let key_change = self
            .line_tag
            .as_deref()
            .is_some_and(|tag| tag.contains("KEY"));
        if key_change {
            y -= ((line.elapsed / 0.25).clamp(0.0, 1.0) * 2.0).round() as i32;
        }
        let accent_hue = moment.accent_hue(48.0);
        let accent = Rgb::hsv(accent_hue, 0.7, 1.0);
        let mut cursor = (x, y);
        let tick = (moment.time * 30.0) as u64;
        for (row_index, row) in self.rows.iter().enumerate() {
            let ry = y + row_index as i32 * (font::LINE * self.sy).max(font::LINE);
            let mut pen = x;
            for (offset, c) in row.text.chars().enumerate() {
                let index = row.start + offset;
                if index >= line.typed {
                    break;
                }
                let sung = line
                    .words
                    .get(line.sung)
                    .is_some_and(|&(a, b)| index >= a && index < b);
                let i = index as f32;
                let (color, dx, dy) = match self.line_tag.as_deref() {
                    Some("BELTED") => {
                        let jitter = if moment.kick > 0.3 {
                            ((unit(mix(moment.seed ^ tick ^ index as u64), 2) * 2.0 - 1.0) * 1.4)
                                .round() as i32
                        } else {
                            0
                        };
                        (if sung { accent } else { Rgb::new(255, 64, 72) }, jitter, 0)
                    }
                    Some("WHISPERED") => {
                        let fade = 0.25 + 0.55 * (i / line.total.max(1) as f32);
                        (
                            DIM.mix(WINDOW, fade),
                            0,
                            ((moment.clock * 1.3 + i * 0.4).sin() * 0.6).round() as i32,
                        )
                    }
                    Some("FALSETTO") => (
                        if sung {
                            accent
                        } else {
                            Rgb::new(255, 146, 222)
                        },
                        0,
                        ((moment.clock * 9.0 + i * 0.6).sin() * 2.2).round() as i32,
                    ),
                    Some("GABBER") => (
                        Rgb::hsv(moment.clock * 220.0 + i * 24.0, 0.7, 1.0),
                        0,
                        -((moment.kick * 3.0 * (i * 1.7).sin().abs()).round() as i32),
                    ),
                    Some("ROOT") => (if sung { accent } else { GREEN }, 0, 0),
                    _ if key_change => (
                        if sung {
                            accent
                        } else {
                            Rgb::new(120, 230, 255)
                        },
                        0,
                        0,
                    ),
                    _ => (if sung { accent } else { TEXT }, 0, 0),
                };
                if c != ' ' {
                    if self.sx > 1 {
                        font::glyph(
                            raster,
                            c,
                            pen + dx + 1,
                            ry + dy + 1,
                            self.sx,
                            self.sy,
                            Rgb::BLACK,
                        );
                    }
                    if font::glyph(raster, c, pen + dx, ry + dy, self.sx, self.sy, color) {
                        report.glyphs += 1;
                    }
                }
                pen += font::ADVANCE * self.sx;
            }
            if line.typed > row.start {
                cursor = (pen, ry);
            }
        }
        if cursor_lit(moment, !line.complete()) {
            raster.rect(
                cursor.0 + 1,
                cursor.1,
                5 * self.sx.min(2),
                7 * self.sy.min(2),
                TEXT,
            );
        }
        match self.line_tag.as_deref() {
            Some("BELTED") | Some("GABBER") => {
                ask.shake = 1.0;
                ask.split = 0.5;
            }
            Some("LOGOUT") if line.complete() => {
                let since = line.elapsed - line.revealed_at;
                let mut sy = cursor.1 + font::LINE + 2;
                for (k, message) in SHUTDOWN.iter().enumerate() {
                    let start = 0.15 + k as f32 * 0.45;
                    if since < start {
                        break;
                    }
                    let shown = (((since - start) * 40.0) as usize).min(message.len());
                    say(raster, report, &message[..shown], x, sy, 1, 1, GREEN);
                    sy += font::LINE;
                }
            }
            Some("ROOT") if line.complete() => {
                let (wx, wy, ww, wh) = window;
                let since = line.elapsed;
                // Under the line it answers, never over it.
                let by = (cursor.1 + font::LINE * self.sy.max(1) + 10).min(wy + wh - 30);
                let bx = wx + ww / 2;
                let stamp = "ACCESS GRANTED";
                let scale = if font::width(stamp, 2) + 16 < ww {
                    2
                } else {
                    1
                };
                let bw = font::width(stamp, scale) + 12;
                raster.rect(
                    bx - bw / 2,
                    by - 6,
                    bw,
                    14 * scale + 12,
                    Rgb::new(8, 20, 10),
                );
                raster.frame(bx - bw / 2, by - 6, bw, 14 * scale + 12, 1, GREEN);
                say_centred(raster, report, stamp, bx, by, scale, scale, GREEN, None);
                if moment.phase < 0.75 || since > 2.0 {
                    say_centred(
                        raster,
                        report,
                        "=^.^=",
                        bx,
                        by + 9 * scale,
                        scale,
                        scale,
                        TAG,
                        None,
                    );
                }
            }
            _ => {}
        }
        ask
    }
}
