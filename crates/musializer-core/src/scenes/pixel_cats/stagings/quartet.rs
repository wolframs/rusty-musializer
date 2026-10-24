//! Barbershop: a quartet of cats in boaters on a curtained stage.
//!
//! Four singers — black, ginger, grey and cream — bob in pairs on the beat and
//! open their mouths with the vocal, each on its own phase so the harmony
//! looks like four voices. Behind them a drum kit's bass drum swells on the
//! kicks and its cymbals jump on the hats, the barber poles spin faster as the
//! track gets louder, and the line is typed onto a marquee whose bulbs chase
//! on the beat. A tiny terminal on the drum riser echoes the sung word.

use super::super::font;
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{self, Look};
use super::super::{Ask, Moment, PixelReport};
use super::{blinking, cat, say, say_centred, typed_block_centred};

const VALANCE: Rgb = Rgb::new(122, 16, 30);
const GOLD: Rgb = Rgb::new(224, 182, 82);
const FLOOR: Rgb = Rgb::new(110, 70, 40);
const PLANK: Rgb = Rgb::new(80, 48, 26);
const CREAM: Rgb = Rgb::new(250, 236, 206);

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    let portrait = h > w;
    let floor = h - 30;

    // Curtains: a fold per few columns, lit from the front.
    for x in 0..w {
        let fold = 0.55 + 0.45 * (x as f32 * 0.28).sin();
        let color = Rgb::new(
            (150.0 * fold + 40.0) as u8,
            (20.0 * fold + 6.0) as u8,
            (30.0 * fold + 12.0) as u8,
        );
        raster.rect(x, 0, 1, floor, color);
    }
    raster.rect(0, 0, w, 12, VALANCE);
    for x in (0..w + 16).step_by(16) {
        raster.ellipse(x as f32 + 8.0, 12.0, 8.0, 4.0, VALANCE);
    }
    raster.rect(0, 0, w, 2, GOLD);
    // Floor planks.
    raster.rect(0, floor, w, h - floor, FLOOR);
    for (k, dy) in [4, 9, 15, 22, 29].into_iter().enumerate() {
        raster.rect(0, floor + dy, w, 1, PLANK);
        let seam = (k as i32 * 53 + 17) % w.max(1);
        raster.rect(seam, floor + dy - 4, 1, 4, PLANK);
    }
    raster.rect(0, floor, w, 2, Rgb::new(170, 120, 70));

    // Barber poles: stripes climb faster as the track gets louder.
    let speed = 1.0 + 2.5 * moment.energy;
    for px in [8, w - 20] {
        raster.rect(px - 1, 30, 14, 4, Rgb::new(200, 200, 210));
        raster.rect(px - 1, floor - 10, 14, 4, Rgb::new(200, 200, 210));
        for y in 34..(floor - 10) {
            for column in 0..12 {
                let phase =
                    (y as f32 + column as f32 * 0.9 - moment.clock * 30.0 * speed).rem_euclid(18.0);
                let base = if phase < 6.0 {
                    Rgb::new(210, 40, 50)
                } else if !(9.0..15.0).contains(&phase) {
                    Rgb::new(240, 240, 240)
                } else {
                    Rgb::new(40, 70, 200)
                };
                let shade = 0.62 + 0.38 * (column as f32 / 11.0 * std::f32::consts::PI).sin();
                raster.put(px + column, y, base.scale(shade));
            }
        }
    }
    // Two follow spots, swaying.
    for (k, base) in [w / 5, w * 4 / 5].into_iter().enumerate() {
        let sway = (moment.clock * 0.7 + k as f32 * 2.0).sin() * 18.0;
        let foot = base as f32 + (w as f32 * 0.5 - base as f32) * 0.3 + sway;
        raster_cone(raster, base as f32, foot, floor as f32 + 6.0, 46.0);
    }

    // The drum kit and its riser terminal.
    let kit_x = w / 2;
    let kit_y = if portrait { floor - 78 } else { floor - 46 };
    raster.rect(kit_x - 26, kit_y - 46, 52, 38, Rgb::new(60, 60, 72));
    raster.rect(kit_x - 23, kit_y - 43, 46, 30, Rgb::new(8, 24, 12));
    let screen: Vec<String> = match &moment.line {
        Some(line) => vec![
            "> SING".into(),
            line.sung_word().chars().take(7).collect(),
            if moment.phase < 0.5 {
                "#_".into()
            } else {
                "#".into()
            },
        ],
        None => vec![
            "> TUNE".into(),
            "C MAJ".into(),
            if moment.phase < 0.5 {
                "_".into()
            } else {
                String::new()
            },
        ],
    };
    for (k, text) in screen.iter().enumerate() {
        say(
            raster,
            report,
            text,
            kit_x - 21,
            kit_y - 41 + k as i32 * 9,
            1,
            1,
            Rgb::new(120, 255, 140),
        );
    }
    raster.rect(kit_x - 6, kit_y - 8, 12, 4, Rgb::new(60, 60, 72));
    let crash = moment.ticks.iter().any(|e| e.age < 0.09);
    raster.rect(
        kit_x - 40,
        kit_y - 14 - i32::from(crash),
        18,
        2,
        Rgb::new(230, 190, 80),
    );
    raster.rect(
        kit_x + 22,
        kit_y - 18 + i32::from(crash),
        18,
        2,
        Rgb::new(230, 190, 80),
    );
    raster.rect(kit_x - 32, kit_y - 12, 1, 30, Rgb::new(150, 150, 160));
    raster.rect(kit_x + 31, kit_y - 16, 1, 34, Rgb::new(150, 150, 160));
    let radius = 16.0 + 2.5 * moment.kick.min(1.2);
    raster.disc(
        kit_x as f32,
        kit_y as f32 + 12.0,
        radius + 2.0,
        Rgb::new(200, 30, 40),
    );
    raster.disc(
        kit_x as f32,
        kit_y as f32 + 12.0,
        radius,
        Rgb::new(236, 226, 200),
    );
    say_centred(
        raster,
        report,
        "=^.^=",
        kit_x,
        kit_y + 9,
        1,
        1,
        Rgb::new(200, 30, 40),
        None,
    );

    // The quartet.
    let furs = [
        (Rgb::new(18, 16, 26), Rgb::new(200, 240, 90)),
        (Rgb::new(214, 128, 48), Rgb::new(40, 120, 60)),
        (Rgb::new(128, 128, 142), Rgb::new(60, 180, 220)),
        (Rgb::new(236, 214, 170), Rgb::new(70, 110, 200)),
    ];
    let xs: [i32; 4] = if portrait {
        [0, 1, 2, 3].map(|k| (w as f32 * (k as f32 + 0.5) / 4.0) as i32 - 16)
    } else {
        [-114, -70, 38, 82].map(|dx| w / 2 + dx)
    };
    for (k, (&x, &(fur, eye))) in xs.iter().zip(furs.iter()).enumerate() {
        let lean = (std::f32::consts::PI * (moment.phase + (k % 2) as f32 * 0.5))
            .sin()
            .abs();
        let bob = -((2.0 + 2.0 * moment.energy) * lean).round() as i32;
        let mut look = Look::cat(fur, eye);
        look.rim = Some(Rgb::new(255, 222, 160));
        look.open = if blinking(moment, 10 + k as u64) {
            0.0
        } else {
            1.0
        };
        look.mouth = (moment.vocal * (0.8 + 0.4 * (moment.clock * 9.0 + k as f32 * 1.7).sin())
            - 0.12)
            .clamp(0.0, 1.0);
        look.look = if k < 2 { 1 } else { -1 };
        cat(
            raster,
            report,
            sprites::BARBER,
            x,
            floor - 40 + bob,
            2,
            false,
            &look,
        );
    }

    // The marquee.
    let board_w = (w - 40).min(260);
    let board_h = if portrait { 44 } else { 30 };
    let (bx, by) = (w / 2 - board_w / 2, 18);
    raster.rect(bx, by, board_w, board_h, Rgb::new(40, 10, 22));
    raster.frame(bx, by, board_w, board_h, 2, GOLD);
    let mut bulb = 0;
    let mut place = |raster: &mut Raster, x: i32, y: i32| {
        let lit = (bulb + moment.beat as i32) % 3 == 0;
        raster.rect(
            x,
            y,
            2,
            2,
            if lit {
                Rgb::new(255, 244, 180)
            } else {
                Rgb::new(120, 90, 40)
            },
        );
        bulb += 1;
    };
    for x in (bx + 2..bx + board_w - 2).step_by(6) {
        place(raster, x, by - 3);
        place(raster, x, by + board_h + 1);
    }
    match &moment.line {
        Some(line) => {
            let (scale, rows) = font::fit(&line.text, board_w - 12, board_h - 8, 3, 2);
            let tall =
                rows.len() as i32 * font::LINE * scale - (font::LINE - font::GLYPH_HEIGHT) * scale;
            let accent = Rgb::hsv(moment.accent_hue(46.0), 0.75, 1.0);
            typed_block_centred(
                raster,
                report,
                line,
                &rows,
                w / 2,
                by + (board_h - tall) / 2,
                scale,
                CREAM,
                accent,
                None,
            );
        }
        None => {
            say_centred(
                raster,
                report,
                "LIVE TONIGHT",
                w / 2,
                by + 6,
                1,
                1,
                CREAM,
                None,
            );
            say_centred(
                raster,
                report,
                "THE CAT QUARTET",
                w / 2,
                by + 16,
                1,
                1,
                GOLD,
                None,
            );
        }
    }
    report.detail = format!("poles={speed:.2} drum={radius:.1}");
    Ask {
        shake: 0.3,
        ..Ask::default()
    }
}

/// A soft spotlight cone from `(top_x, -4)` to a floor ellipse at `foot`.
fn raster_cone(raster: &mut Raster, top_x: f32, foot: f32, floor: f32, spread: f32) {
    let light = Rgb::new(255, 240, 200);
    let h = floor.max(1.0);
    for y in 0..(floor as i32) {
        let t = (y as f32 + 4.0) / (h + 4.0);
        let centre = top_x + (foot - top_x) * t;
        let half = 3.0 + spread * t;
        for x in ((centre - half) as i32)..=((centre + half) as i32) {
            raster.blend(x, y, light, 0.13);
        }
    }
    raster.ellipse_alpha(foot, floor, spread, 4.0, light, 0.18);
}
