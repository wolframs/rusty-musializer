//! Side-scroller: a cat running over keyboard-key rooftops at night.
//!
//! The ground is a row of keycaps that spell the line on screen (`QWERTY` with
//! none), and it scrolls faster as the track gets louder; the keys light on the
//! kicks. The cat gallops in time — its two frames swap on every half beat —
//! and jumps on the hi-hats. A keycap skyline drifts behind under a cat-head
//! moon, and the line itself is the level's banner, typed in over the HUD.

use super::super::font;
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{self, Look};
use super::super::{unit, Ask, Moment, PixelReport};
use super::{cat, say, say_centred, stars, typed_block_centred};

const SKY_TOP: Rgb = Rgb::new(24, 10, 52);
const SKY_LOW: Rgb = Rgb::new(78, 34, 116);
const MOON: Rgb = Rgb::new(250, 240, 200);
const TOWER: Rgb = Rgb::new(40, 26, 72);
const TOWER_CAP: Rgb = Rgb::new(64, 44, 104);
const WINDOW: Rgb = Rgb::new(250, 210, 90);
const KEY_SIDE: Rgb = Rgb::new(90, 90, 120);
const KEY_TOP: Rgb = Rgb::new(200, 200, 222);
const KEY_LIT: Rgb = Rgb::new(250, 120, 180);
const GOLD: Rgb = Rgb::new(255, 220, 80);

/// Ground scroll per unit of the state's travel, in canvas pixels.
const SPEED: f32 = 55.0;
/// A jump's arc, seconds.
const HOP: f32 = 0.34;

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    let ground = h - 30;
    let scroll = moment.travel * SPEED;
    raster.gradient(0, ground, SKY_TOP, SKY_LOW);
    stars(
        raster,
        moment,
        (w * h / 700).max(20) as usize,
        ground * 6 / 10,
        Rgb::new(232, 232, 255),
        scroll * 0.05,
    );

    // The cat-head moon.
    let (mx, my) = (w as f32 - 64.0, 34.0);
    for side in [-1.0f32, 1.0] {
        raster.triangle(
            (mx + side * 15.0, my - 6.0),
            (mx + side * 12.0, my - 24.0),
            (mx + side * 4.0, my - 13.0),
            MOON,
        );
    }
    raster.disc(mx, my, 16.0, MOON);
    say_centred(
        raster,
        report,
        "=^.^=",
        mx as i32,
        my as i32 - 3,
        1,
        1,
        Rgb::new(196, 176, 136),
        None,
    );

    // The keycap skyline, a third of the ground's speed.
    let far = scroll * 0.3;
    let mut x = -(far % 900.0) as i32 - 30;
    let mut index = (far / 900.0) as u64 * 40;
    while x < w + 40 {
        let key = moment.seed ^ (index % 40);
        let width = 18 + (unit(key, 21) * 16.0) as i32;
        let height = 30 + (unit(key, 22) * 52.0) as i32;
        if x + width > -2 {
            let top = ground - height;
            raster.rect(x, top, width - 2, height, TOWER);
            raster.rect(x + 2, top + 2, width - 6, 10, TOWER_CAP);
            let letter = (b'A' + (unit(key, 23) * 26.0) as u8 % 26) as char;
            font::glyph(
                raster,
                letter,
                x + width / 2 - 3,
                top + 4,
                1,
                1,
                Rgb::new(120, 100, 170),
            );
            let beat = moment.beat as i32;
            for wy in ((top + 18)..(ground - 4)).step_by(8) {
                for wx in ((x + 3)..(x + width - 5)).step_by(6) {
                    if (wx * 7 + wy * 3 + index as i32 + beat * ((wx / 6) % 2)) % 5 == 0 {
                        raster.rect(wx, wy, 2, 3, WINDOW);
                    }
                }
            }
        }
        x += width;
        index += 1;
    }

    // The ground: keycaps spelling the line.
    let letters: Vec<char> = match &moment.line {
        Some(line) => line.text.chars().filter(|c| *c != ' ').collect(),
        None => "QWERTYUIOPASDFGHJKLZXCVBNM".chars().collect(),
    };
    let pitch = 26;
    let offset = (scroll as i32).rem_euclid(pitch);
    let first = (scroll as i32).div_euclid(pitch);
    let cat_x = w / 4;
    let lit_kick = moment.kick > 0.3;
    for k in 0..(w / pitch + 2) {
        let kx = k * pitch - offset;
        let id = first + k;
        let under_cat = kx <= cat_x + 22 && kx + pitch > cat_x + 10;
        let lit = lit_kick && (id.rem_euclid(5) == 0 || under_cat);
        raster.rect(kx, ground, pitch - 2, h - ground, KEY_SIDE);
        raster.rect(
            kx + 2,
            ground + 1,
            pitch - 6,
            14,
            if lit { KEY_LIT } else { KEY_TOP },
        );
        if !letters.is_empty() {
            let letter = letters[id.rem_euclid(letters.len() as i32) as usize];
            font::glyph(
                raster,
                letter,
                kx + pitch / 2 - 4,
                ground + 4,
                1,
                1,
                Rgb::new(60, 60, 90),
            );
            report.glyphs += 1;
        }
    }

    // Bracket coins.
    for j in 0..6 {
        let cx = (j as f32 * 70.0 - scroll).rem_euclid(420.0) as i32 - 20;
        let cy = ground - 46 + (6.0 * (moment.clock * 4.0 + j as f32).sin()) as i32;
        if cx > cat_x + 24 {
            say(raster, report, "[ ]", cx, cy, 1, 1, GOLD);
        }
    }

    // The cat: a gallop in time, a jump per hi-hat.
    let jump = moment
        .ticks
        .iter()
        .chain(
            moment
                .ticks
                .is_empty()
                .then_some(&moment.onsets)
                .into_iter()
                .flatten(),
        )
        .filter(|event| event.age < HOP)
        .map(|event| (std::f32::consts::PI * event.age / HOP).sin() * 20.0)
        .fold(0.0f32, f32::max);
    let stride = (moment.beat * 2 + u64::from(moment.phase >= 0.5)).is_multiple_of(2);
    let mut look = Look::cat(Rgb::new(18, 16, 26), Rgb::new(200, 240, 90));
    look.rim = Some(Rgb::new(170, 130, 230));
    let sprite = if jump > 4.0 || stride {
        sprites::RUN_A
    } else {
        sprites::RUN_B
    };
    cat(
        raster,
        report,
        sprite,
        cat_x,
        ground - 22 - jump as i32,
        2,
        false,
        &look,
    );

    // HUD.
    let score = (moment.clock * 37.0) as u64 + moment.onset_count * 9;
    say(raster, report, "ROOT", 6, 6, 1, 1, Rgb::WHITE);
    say(
        raster,
        report,
        &format!("MEOWS {:06}", score % 1_000_000),
        6,
        16,
        1,
        1,
        GOLD,
    );
    let lives = "LIVES X9";
    say(
        raster,
        report,
        lives,
        w - 6 - font::width(lives, 1),
        6,
        1,
        1,
        Rgb::WHITE,
    );

    // The level banner: the line.
    if let Some(line) = &moment.line {
        let (scale, rows) = font::fit(&line.text, w - 24, 32, 2, 2);
        let tall = rows.len() as i32 * font::LINE * scale;
        let longest = rows
            .iter()
            .map(|r| font::width(&r.text, scale))
            .max()
            .unwrap_or(0);
        let top = 30;
        raster.rect_alpha(
            w / 2 - longest / 2 - 5,
            top - 4,
            longest + 10,
            tall + 5,
            Rgb::new(10, 4, 24),
            0.75,
        );
        raster.frame(
            w / 2 - longest / 2 - 5,
            top - 4,
            longest + 10,
            tall + 5,
            1,
            GOLD.scale(0.7),
        );
        let accent = Rgb::hsv(moment.accent_hue(48.0), 0.7, 1.0);
        typed_block_centred(
            raster,
            report,
            line,
            &rows,
            w / 2,
            top,
            scale,
            Rgb::WHITE,
            accent,
            Some(Rgb::BLACK),
        );
    }
    report.detail = format!("scroll={:.0} jump={:.0}", scroll, jump);
    Ask {
        shake: 0.4,
        ..Ask::default()
    }
}
