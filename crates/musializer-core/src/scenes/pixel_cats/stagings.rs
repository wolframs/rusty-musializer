//! The six stagings, and what they share.
//!
//! Each staging is one function from the [`Moment`] to a picture, and each
//! honours the same three rules:
//!
//! - **The line is the subject.** It is typed in, readable at 1x, and finished
//!   inside its reveal ([`Line::new`]); the sung word takes the accent colour.
//! - **The music moves things, it does not flash them.** Kicks squash, bounce
//!   and shake; vocals open mouths and light windows; hi-hats make cats jump.
//! - **No line still makes a picture.** An instrumental stretch gets the
//!   staging's idle business — an ASCII meter, a boot logo, a running cat.

mod boot;
mod chapel;
mod quartet;
mod rave;
mod runner;
mod shell;

use super::font::{self, Row};
use super::raster::{Raster, Rgb};
use super::sprites;
use super::{unit, Ask, Line, Moment, PixelReport, Staging};

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    match moment.staging {
        Staging::Boot => boot::draw(raster, moment, report),
        Staging::Shell => shell::draw(raster, moment, report),
        Staging::Runner => runner::draw(raster, moment, report),
        Staging::Quartet => quartet::draw(raster, moment, report),
        Staging::Chapel => chapel::draw(raster, moment, report),
        Staging::Rave => rave::draw(raster, moment, report),
    }
}

/// Text, counted.
#[allow(clippy::too_many_arguments)]
fn say(
    raster: &mut Raster,
    report: &mut PixelReport,
    text: &str,
    x: i32,
    y: i32,
    sx: i32,
    sy: i32,
    color: Rgb,
) -> i32 {
    report.glyphs += font::text(raster, text, x, y, sx, sy, color);
    font::width(text, sx)
}

/// Text centred on `cx`, with an optional hard shadow.
#[allow(clippy::too_many_arguments)]
fn say_centred(
    raster: &mut Raster,
    report: &mut PixelReport,
    text: &str,
    cx: i32,
    y: i32,
    sx: i32,
    sy: i32,
    color: Rgb,
    shadow: Option<Rgb>,
) {
    let x = cx - font::width(text, sx) / 2;
    match shadow {
        Some(shadow) => {
            report.glyphs += font::shadowed(raster, text, x, y, sx, sy, color, shadow);
        }
        None => {
            report.glyphs += font::text(raster, text, x, y, sx, sy, color);
        }
    }
}

/// One wrapped row of the line, only as far as the typewriter has reached,
/// the sung word in `accent`. Returns the pen's x after the last character
/// drawn, for a cursor.
#[allow(clippy::too_many_arguments)]
fn typed_row(
    raster: &mut Raster,
    report: &mut PixelReport,
    line: &Line,
    row: &Row,
    x: i32,
    y: i32,
    sx: i32,
    sy: i32,
    ink: Rgb,
    accent: Rgb,
    shadow: Option<Rgb>,
) -> i32 {
    let sung = line.words.get(line.sung).copied();
    let mut pen = x;
    for (offset, c) in row.text.chars().enumerate() {
        let index = row.start + offset;
        if index >= line.typed {
            break;
        }
        let lit = sung.is_some_and(|(a, b)| index >= a && index < b);
        if c != ' ' {
            if let Some(shadow) = shadow {
                font::glyph(
                    raster,
                    c,
                    pen + (sx / 2).max(1),
                    y + (sy / 2).max(1),
                    sx,
                    sy,
                    shadow,
                );
            }
            if font::glyph(raster, c, pen, y, sx, sy, if lit { accent } else { ink }) {
                report.glyphs += 1;
            }
        }
        pen += font::ADVANCE * sx;
    }
    pen
}

/// The whole line typed into rows centred on `cx` from `y`.
#[allow(clippy::too_many_arguments)]
fn typed_block_centred(
    raster: &mut Raster,
    report: &mut PixelReport,
    line: &Line,
    rows: &[Row],
    cx: i32,
    y: i32,
    scale: i32,
    ink: Rgb,
    accent: Rgb,
    shadow: Option<Rgb>,
) -> (i32, i32) {
    let mut cursor = (cx, y);
    for (index, row) in rows.iter().enumerate() {
        let x = cx - font::width(&row.text, scale) / 2;
        let ry = y + index as i32 * font::LINE * scale;
        if line.typed > row.start {
            let end = typed_row(
                raster, report, line, row, x, ry, scale, scale, ink, accent, shadow,
            );
            cursor = (end, ry);
        }
    }
    cursor
}

/// Whether a blinking cursor is lit: solid while typing, on the beat's first
/// half once the line is down.
fn cursor_lit(moment: &Moment, typing: bool) -> bool {
    typing || moment.phase < 0.5
}

/// A cat's blink: about every three seconds, 140 ms long, each cat on its own
/// offset so a row of them never blinks in unison.
fn blinking(moment: &Moment, cat: u64) -> bool {
    let period = 2.6 + unit(moment.seed ^ cat, 7) * 1.6;
    let offset = unit(moment.seed ^ cat, 8) * period;
    ((moment.time + offset) % period) < 0.14
}

/// The words that pop on an onset when no line is on screen.
const CAT_SOUNDS: [&str; 8] = [
    "MEOW!", "MRRP", "NYA~", "=^.^=", "MEOW", "PRRT", "MEW?", "MROW",
];

/// Pops: one short word per recent onset, rising and shrinking as it ages,
/// at a place and in a colour hashed from the onset itself — so the same
/// onset always pops the same way. The sung word when a line is on screen,
/// a cat sound otherwise. Returns how many drew.
fn pops(
    raster: &mut Raster,
    report: &mut PixelReport,
    moment: &Moment,
    area: (i32, i32, i32, i32),
    hue_base: f32,
    life: f32,
) -> usize {
    if !moment.pops {
        return 0;
    }
    let mut drew = 0;
    for event in moment.onsets.iter().filter(|event| event.age < life) {
        let word = match &moment.line {
            Some(line) if !line.sung_word().is_empty() && unit(event.key, 3) < 0.6 => {
                line.sung_word().to_string()
            }
            _ => CAT_SOUNDS
                [(unit(event.key, 4) * CAT_SOUNDS.len() as f32) as usize % CAT_SOUNDS.len()]
            .to_string(),
        };
        let age = event.age / life;
        let scale = if age < 0.65 {
            2 + (unit(event.key, 5) * 1.99) as i32
        } else {
            1
        };
        let width = font::width(&word, scale);
        let (x0, y0, x1, y1) = area;
        let x = (x0 as f32 + unit(event.key, 1) * (x1 - x0 - width).max(1) as f32) as i32;
        let y = (y0 as f32 + unit(event.key, 2) * (y1 - y0).max(1) as f32 - age * 10.0) as i32;
        let color = Rgb::hsv(hue_base + unit(event.key, 6) * 360.0, 0.65, 1.0);
        report.glyphs += font::shadowed(
            raster,
            &word,
            x,
            y,
            scale,
            scale,
            color,
            Rgb::new(10, 6, 18),
        );
        drew += 1;
    }
    report.pops += drew;
    drew
}

/// A dark box with a typed line in it, for the stagings that subtitle.
fn subtitle(
    raster: &mut Raster,
    report: &mut PixelReport,
    moment: &Moment,
    line: &Line,
    y_bottom: i32,
    ink: Rgb,
    accent: Rgb,
) {
    let width = moment.width - 12;
    let (scale, rows) = font::fit(&line.text, width, 30, 2, 2);
    let tall = rows.len() as i32 * font::LINE * scale - (font::LINE - font::GLYPH_HEIGHT) * scale;
    let longest = rows
        .iter()
        .map(|row| font::width(&row.text, scale))
        .max()
        .unwrap_or(0);
    let top = y_bottom - tall - 4;
    raster.rect_alpha(
        moment.width / 2 - longest / 2 - 4,
        top - 3,
        longest + 8,
        tall + 6,
        Rgb::new(6, 4, 12),
        0.72,
    );
    typed_block_centred(
        raster,
        report,
        line,
        &rows,
        moment.width / 2,
        top,
        scale,
        ink,
        accent,
        None,
    );
}

/// Draws a cat sprite and counts it.
#[allow(clippy::too_many_arguments)]
fn cat(
    raster: &mut Raster,
    report: &mut PixelReport,
    sprite: sprites::Sprite,
    x: i32,
    y: i32,
    scale: i32,
    flip: bool,
    look: &sprites::Look,
) {
    if sprites::draw(raster, sprite, x, y, scale, flip, look) {
        report.cats += 1;
    }
}

/// Small risers: one `text` per recent onset, rising from one of `anchors`
/// (chosen by the onset's own hash) and drifting sideways as it goes, gone
/// after `life` seconds. Notes over singers, a score over a runner. Returns
/// how many drew.
#[allow(clippy::too_many_arguments)]
fn risers(
    raster: &mut Raster,
    report: &mut PixelReport,
    moment: &Moment,
    anchors: &[(i32, i32)],
    text: &str,
    color: Rgb,
    life: f32,
    rise: f32,
) -> usize {
    if !moment.pops || anchors.is_empty() {
        return 0;
    }
    let mut drew = 0;
    for event in moment.onsets.iter().filter(|event| event.age < life) {
        let (ax, ay) = anchors[(event.key % anchors.len() as u64) as usize];
        let t = event.age / life;
        let x = ax + ((t * 6.0 + unit(event.key, 9) * 6.3).sin() * 3.0) as i32;
        let y = ay - (t * rise) as i32;
        let tint = color.mix(Rgb::hsv(unit(event.key, 6) * 360.0, 0.5, 1.0), 0.35);
        report.glyphs += font::shadowed(raster, text, x, y, 1, 1, tint, Rgb::new(12, 8, 18));
        drew += 1;
    }
    report.pops += drew;
    drew
}

/// Twinkling stars over `0..height` rows, hashed from their index; each
/// brightens and dims on its own phase.
fn stars(raster: &mut Raster, moment: &Moment, count: usize, height: i32, color: Rgb, drift: f32) {
    let w = moment.width as f32;
    for index in 0..count as u64 {
        let x = (unit(moment.seed ^ index, 11) * w - drift).rem_euclid(w) as i32;
        let y = (unit(moment.seed ^ index, 12) * height as f32) as i32;
        let twinkle = 0.5 + 0.5 * (moment.clock * 3.0 + unit(moment.seed ^ index, 13) * 6.3).sin();
        if twinkle > 0.3 {
            raster.blend(x, y, color, twinkle);
        }
    }
}
