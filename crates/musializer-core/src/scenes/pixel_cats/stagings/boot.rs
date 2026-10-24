//! Boot screen: a handheld powering on.
//!
//! The logo drops in from the top of the screen and lands; a shine crosses it;
//! an ASCII cat assembles under it one row at a time and blinks; `PRESS START`
//! keeps the beat. With a line on screen the line *is* the logo — the whole
//! line when it fits large, else its first word large and the rest typed
//! beneath. With none it is `MEOW`, dropped when the section starts.

use super::super::font;
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{ASCII_ASLEEP, ASCII_CAT, ASCII_WIDE};
use super::super::{ease_out, Ask, Moment, PixelReport};
use super::{blinking, say_centred, typed_block_centred};

const BG: Rgb = Rgb::new(196, 214, 124);
const INK: Rgb = Rgb::new(16, 48, 28);
const MID: Rgb = Rgb::new(52, 98, 58);
const SOFT: Rgb = Rgb::new(138, 168, 50);
const SHINE: Rgb = Rgb::new(236, 244, 196);

/// Seconds the logo takes to fall into place.
const DROP: f32 = 0.55;

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    raster.fill(BG);
    // The screen's own faint inner frame.
    raster.frame(3, 3, w - 6, h - 6, 1, SOFT.mix(BG, 0.4));

    // What the logo says, and what is typed under it.
    let (logo, rest, since) = match &moment.line {
        Some(line) => {
            let (scale, _) = font::fit(&line.text, w * 9 / 10, h * 36 / 100, 2, 6);
            if scale >= 3 || line.words.len() <= 1 {
                (line.text.clone(), None, line.elapsed)
            } else {
                let first = line.word(0).to_string();
                (first, Some(line), line.elapsed)
            }
        }
        None => ("MEOW".to_string(), None, moment.age),
    };
    let (scale, rows) = font::fit(&logo, w * 9 / 10, h * 36 / 100, 2, 6);
    let block = rows.len() as i32 * font::LINE * scale - (font::LINE - font::GLYPH_HEIGHT) * scale;
    let target = (h as f32 * 0.17) as i32;
    let fall = ease_out(since / DROP);
    let landed = since >= DROP;
    let bounce = if landed {
        -((moment.pulse * 2.0).round() as i32)
    } else {
        0
    };
    let top = target - ((1.0 - fall) * (target + block + 6) as f32) as i32 + bounce;
    for (index, row) in rows.iter().enumerate() {
        say_centred(
            raster,
            report,
            &row.text,
            w / 2,
            top + index as i32 * font::LINE * scale,
            scale,
            scale,
            INK,
            Some(SOFT),
        );
    }
    // The shine: one diagonal band across the landed logo.
    let sweep = (since - DROP) / 0.45;
    if (0.0..=1.0).contains(&sweep) {
        let left = w / 2
            - rows
                .iter()
                .map(|r| font::width(&r.text, scale))
                .max()
                .unwrap_or(0)
                / 2;
        let span = (w - 2 * left) + block;
        let band = left as f32 + sweep * span as f32 - block as f32 * 0.5;
        for y in top.max(0)..(top + block).min(h) {
            for x in left.max(0)..(w - left).min(w) {
                let d = x as f32 + (y - top) as f32 * 0.6 - band;
                if d.abs() < scale as f32 * 1.5 && raster.get(x, y) == INK {
                    raster.put(x, y, SHINE);
                }
            }
        }
    }
    let mut below = top + block + 7;
    match rest {
        Some(line) => {
            // The rest of the line, typed under the logo it began.
            let start = line.words.get(1).map_or(line.total, |w| w.0);
            let rest_text: String = line.text.chars().skip(start).collect();
            let (small, rest_rows) = font::fit(&rest_text, w - 16, h / 5, 3, 2);
            let shifted: Vec<font::Row> = rest_rows
                .into_iter()
                .map(|row| font::Row {
                    text: row.text,
                    start: row.start + start,
                })
                .collect();
            if landed {
                typed_block_centred(
                    raster,
                    report,
                    line,
                    &shifted,
                    w / 2,
                    below,
                    small,
                    MID,
                    INK,
                    None,
                );
            }
            below += shifted.len() as i32 * font::LINE * small + 4;
        }
        None => {
            if landed {
                say_centred(raster, report, "CATOS", w / 2, below, 1, 1, MID, None);
            }
            below += font::LINE + 4;
        }
    }

    // The ASCII cat, one row at a time after the logo lands.
    let cat_scale = if h >= 150 { 2 } else { 1 };
    let cat_top = below.max((h as f32 * 0.52) as i32);
    let bob = if moment.phase < 0.22 { -1 } else { 0 };
    for (index, text) in ASCII_CAT.iter().enumerate() {
        if since < DROP + 0.2 + index as f32 * 0.32 {
            break;
        }
        let face = if index == 1 {
            if moment.kick > 0.65 {
                ASCII_WIDE
            } else if blinking(moment, 1) {
                ASCII_ASLEEP
            } else {
                text
            }
        } else {
            text
        };
        say_centred(
            raster,
            report,
            face,
            w / 2,
            cat_top + bob + index as i32 * (font::LINE - 1) * cat_scale,
            cat_scale,
            cat_scale,
            INK,
            None,
        );
    }
    if since >= DROP + 0.2 {
        report.cats += 1;
    }

    // Footer: the prompt keeps the beat.
    if moment.phase < 0.5 && landed {
        say_centred(
            raster,
            report,
            "PRESS START",
            w / 2,
            h - 24,
            1,
            1,
            MID,
            None,
        );
    }
    say_centred(raster, report, "(C) CATOS", w / 2, h - 13, 1, 1, SOFT, None);
    report.detail = format!(
        "logo={} scale={} landed={}",
        logo.chars().count(),
        scale,
        if landed { "yes" } else { "falling" }
    );
    Ask {
        shake: 0.0,
        split: 0.0,
        zoom: 0.0,
        glitch: 0,
    }
}
