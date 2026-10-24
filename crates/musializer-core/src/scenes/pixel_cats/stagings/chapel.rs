//! Rose window: monk cats under stained glass, the line on a chant staff.
//!
//! A rose window turns slowly in the stone, its glass brightening with the
//! vocal, a cat's face in its centre whose eyes glow with it; light falls from
//! it to a row of hooded monk cats whose eyes glow too, between candles that
//! flicker. The line is laid out on a parchment staff: each word appears as it
//! is typed, under a square neume at a pitch hashed from the word, and the
//! sung word's note lights.

use super::super::font;
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{self, Face, Look};
use super::super::{mix, unit, Ask, Line, Moment, PixelReport};
use super::{blinking, cat, risers, say_centred};

const STONE: Rgb = Rgb::new(34, 36, 54);
const MORTAR: Rgb = Rgb::new(24, 26, 40);
const LEAD: Rgb = Rgb::new(20, 16, 26);
const PARCHMENT: Rgb = Rgb::new(222, 206, 164);
const STAFF: Rgb = Rgb::new(190, 40, 40);
const INK: Rgb = Rgb::new(140, 20, 30);
const NEUME: Rgb = Rgb::new(20, 16, 20);

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    let portrait = h > w;

    // Stone.
    raster.fill(STONE);
    for y in (0..h).step_by(8) {
        raster.rect(0, y, w, 1, MORTAR);
        let offset = if (y / 8) % 2 == 0 { 0 } else { 8 };
        for x in (offset..w).step_by(16) {
            raster.rect(x, y, 1, 8, Rgb::new(26, 28, 44));
        }
    }
    for cx in [18, w - 18] {
        raster.rect(cx - 8, 0, 16, h, Rgb::new(50, 50, 70));
        raster.rect(cx - 8, 0, 2, h, Rgb::new(70, 70, 92));
    }

    // Layout, bottom up when it has to be: the monks keep the floor, the
    // staff sits on them, and a line long enough to need a second stave takes
    // its room from the window rather than pushing the monks off the screen.
    let staves = moment
        .line
        .as_ref()
        .map_or(1, |line| stave_words(line, w).len().min(3) as i32);
    let monk_y = h - 58;
    let mut radius = if portrait {
        w as f32 * 0.30
    } else {
        h as f32 * 0.20
    };
    let mut wcy = if portrait {
        h as f32 * 0.17
    } else {
        h as f32 * 0.27
    };
    let mut banner_top = (wcy + radius * 1.12 + 4.0) as i32;
    if banner_top + staves * 34 > monk_y - 2 {
        banner_top = monk_y - 2 - staves * 34;
        radius = radius.min(((banner_top - 8) as f32 / 2.24).max(10.0));
        wcy = banner_top as f32 - 4.0 - radius * 1.12;
    }
    let wcx = w as f32 * 0.5;
    let bright = 0.62 + 0.5 * moment.vocal;
    let hue0 = moment.clock * 12.0;
    let turn = moment.clock * 0.15;
    rose(raster, wcx, wcy, radius, hue0, bright, turn);
    let glow = (0.45 + moment.vocal).clamp(0.3, 1.0);
    sprites::face(
        raster,
        wcx,
        wcy + radius * 0.24 * 0.15,
        radius * 0.24,
        &Face {
            fur: LEAD,
            rim: LEAD,
            eye: Rgb::new(200, 240, 90).scale(glow),
            open: if blinking(moment, 40) { 0.0 } else { 1.0 },
            mouth: 0.0,
            shock: false,
            look: 0.0,
            happy: false,
            whiskers: Rgb::new(60, 56, 70),
        },
    );

    // Light from the window to the floor.
    let beam = Rgb::hsv(hue0 + 40.0, 0.25, 1.0);
    let strength = 0.18 + 0.25 * moment.vocal;
    let from = wcy + radius * 0.4;
    let reach = (h as f32 - from).max(1.0);
    for y in (from as i32).max(0)..h {
        let t = (y as f32 - from) / reach;
        for (k, spread) in [-40.0f32, 0.0, 40.0].into_iter().enumerate() {
            let centre = wcx + spread * 0.2 + t * spread;
            let half = 6.0 + t * 22.0;
            for x in ((centre - half) as i32).max(0)..((centre + half) as i32).min(w) {
                let d = 1.0 - (x as f32 - centre).abs() / half;
                let a = d * 0.5 * strength * if k == 1 { 1.0 } else { 0.85 };
                raster.blend(x, y, beam, a);
            }
        }
    }

    // The chant staff.
    let staves = match &moment.line {
        Some(line) => staff(raster, report, moment, line, banner_top),
        None => {
            blank_staff(raster, 22, banner_top, w - 44, 1);
            1
        }
    };

    // Monks and candles.
    let count = ((w - 40) / 38).clamp(3, 9);
    let pitch = (w - 40) as f32 / count as f32;
    let eyes = Rgb::new(255, 230, 120).scale((0.4 + 0.9 * moment.vocal).clamp(0.25, 1.4));
    for k in 0..count {
        let x = (20.0 + pitch * (k as f32 + 0.5)) as i32 - 16;
        let sway = ((moment.clock * 2.0 + k as f32 * 0.9).sin()).round() as i32;
        let bob = if moment.energy > 0.7 {
            -((3.0
                * (std::f32::consts::PI * moment.phase + k as f32 * 0.3)
                    .sin()
                    .abs())
            .round() as i32)
        } else {
            0
        };
        let mut look = Look::cat(LEAD, eyes);
        look.robe = Rgb::new(96, 62, 40);
        look.open = if blinking(moment, 50 + k as u64) {
            0.0
        } else {
            1.0
        };
        cat(
            raster,
            report,
            sprites::MONK,
            x + sway,
            monk_y + bob,
            2,
            false,
            &look,
        );
    }
    // A note drifts up from a monk on each onset, slowly: a hymn, not a rave.
    let hoods: Vec<(i32, i32)> = (0..count)
        .map(|k| ((20.0 + pitch * (k as f32 + 0.5)) as i32 - 2, monk_y - 4))
        .collect();
    let note = font::NOTE.to_string();
    risers(
        raster,
        report,
        moment,
        &hoods,
        &note,
        Rgb::new(255, 224, 150),
        1.4,
        18.0,
    );
    for k in 0..=count {
        let x = (20.0 + pitch * k as f32) as i32 - 1;
        let y = h - 14;
        let flicker = 0.7 + 0.3 * (moment.time * 17.0 + k as f32 * 2.3).sin();
        raster.rect(x, y, 3, 10, Rgb::new(230, 220, 200));
        raster.rect(x, y - 4, 3, 4, Rgb::new(255, 190, 80).scale(flicker));
        raster.rect(x + 1, y - 6, 1, 2, Rgb::new(255, 240, 160).scale(flicker));
    }
    report.detail = format!("window={radius:.0} staves={staves} monks={count}");
    Ask::default()
}

/// The stained glass: four rings of segments in jewel colours behind lead.
fn rose(raster: &mut Raster, cx: f32, cy: f32, radius: f32, hue0: f32, bright: f32, turn: f32) {
    let tau = std::f32::consts::TAU;
    let x0 = ((cx - radius * 1.1) as i32).max(0);
    let x1 = ((cx + radius * 1.1) as i32).min(raster.width() - 1);
    let y0 = ((cy - radius * 1.1) as i32).max(0);
    let y1 = ((cy + radius * 1.1) as i32).min(raster.height() - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let rn = (dx * dx + dy * dy).sqrt() / radius;
            if rn >= 1.08 {
                continue;
            }
            if rn >= 1.0 {
                raster.put(x, y, Rgb::new(70, 64, 80));
                continue;
            }
            let ringf = rn * 4.0;
            let ring = (ringf as usize).min(3);
            let segments = [6.0f32, 12.0, 12.0, 24.0][ring];
            let direction = if ring.is_multiple_of(2) { 1.0 } else { -1.0 };
            let angle = (dy.atan2(dx) + turn * direction).rem_euclid(tau);
            let segf = angle / tau * segments;
            let seg = segf as u32;
            let lead = ringf.fract() < 0.10 || segf.fract() < 0.07 * (segments / 12.0).sqrt();
            if lead {
                raster.put(x, y, LEAD);
                continue;
            }
            let hue =
                hue0 + ring as f32 * 47.0 + (seg % 3) as f32 * 120.0 + (seg % 2) as f32 * 25.0;
            let saturation = 0.75 + 0.2 * (seg % 2) as f32;
            let value =
                ((0.55 + 0.35 * ((seg as usize + ring) % 2) as f32) * bright).clamp(0.0, 1.0);
            raster.put(x, y, Rgb::hsv(hue, saturation, value));
        }
    }
}

/// The line's words on staves: greedy, each word at least as wide as its
/// neume pair.
fn stave_words(line: &Line, width: i32) -> Vec<Vec<usize>> {
    let inner = width - 44 - 16;
    let mut staves: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0;
    for index in 0..line.words.len() {
        let width = font::width(line.word(index), 1).max(10) + 8;
        if used + width > inner && !staves.last().is_some_and(Vec::is_empty) {
            staves.push(Vec::new());
            used = 0;
        }
        if let Some(stave) = staves.last_mut() {
            stave.push(index);
        }
        used += width;
    }
    staves
}

fn blank_staff(raster: &mut Raster, x: i32, y: i32, width: i32, staves: i32) {
    raster.rect(x, y, width, 32 * staves + 2 * (staves - 1), PARCHMENT);
    raster.rect(x, y, width, 1, Rgb::new(150, 120, 70));
    raster.rect(
        x,
        y + 32 * staves + 2 * (staves - 1) - 1,
        width,
        1,
        Rgb::new(150, 120, 70),
    );
    for s in 0..staves {
        for k in 0..4 {
            raster.rect(x + 6, y + s * 34 + 4 + k * 4, width - 12, 1, STAFF);
        }
    }
}

/// Lays the line's words on one or more staves. Returns how many staves.
fn staff(
    raster: &mut Raster,
    report: &mut PixelReport,
    moment: &Moment,
    line: &Line,
    top: i32,
) -> i32 {
    let w = moment.width;
    let inner = w - 44 - 16;
    let staves = stave_words(line, w);
    let count = staves.len().min(3) as i32;
    blank_staff(raster, 22, top, w - 44, count);
    for (s, words) in staves.iter().take(3).enumerate() {
        let y = top + s as i32 * 34;
        let widths: Vec<i32> = words
            .iter()
            .map(|&i| font::width(line.word(i), 1).max(10))
            .collect();
        let total: i32 = widths.iter().sum();
        let gap = if words.len() > 1 {
            ((inner - total) / (words.len() as i32 - 1)).clamp(6, 40)
        } else {
            0
        };
        let span = total + gap * (words.len() as i32 - 1).max(0);
        let mut x = w / 2 - span / 2;
        for (&index, &width) in words.iter().zip(&widths) {
            let (start, _) = line.words[index];
            if line.typed <= start {
                break;
            }
            let word = line.word(index);
            let pitch = (unit(mix(moment.seed ^ line.id), index as u64) * 6.0) as i32;
            let ny = y + 4 + (6 - pitch) * 2;
            let sung = index == line.sung;
            let note = if sung {
                Rgb::hsv(moment.accent_hue(42.0), 0.8, 1.0)
            } else {
                NEUME
            };
            raster.rect(x, ny - 1, 4, 4, note);
            if index % 2 == 0 {
                raster.rect(x + 4, ny - 3, 4, 4, note);
            }
            let ink = if sung { Rgb::new(200, 30, 40) } else { INK };
            let lift = if sung {
                -((moment.pulse * 1.5).round() as i32)
            } else {
                0
            };
            say_centred(
                raster,
                report,
                word,
                x + width / 2,
                y + 21 + lift,
                1,
                1,
                ink,
                None,
            );
            x += width + gap;
        }
    }
    count
}
