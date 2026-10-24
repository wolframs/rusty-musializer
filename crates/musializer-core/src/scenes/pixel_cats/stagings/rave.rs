//! Gabber tunnel: a giant cat face at the end of a rushing tunnel.
//!
//! Square rings fly out of the centre, faster on every kick, their colours
//! turning; the face in the middle swells on the kicks, sings with the vocal,
//! goes wide-eyed on a hard kick and fires lasers from its eyes on the hardest.
//! The sung word slams in large at the top as each word arrives; words pop up
//! around the face on the onsets; the whole line runs as a subtitle.

use super::super::font;
use super::super::raster::{Raster, Rgb};
use super::super::sprites::{self, Face};
use super::super::{Ask, Moment, PixelReport};
use super::{pops, say_centred, subtitle};

pub(super) fn draw(raster: &mut Raster, moment: &Moment, report: &mut PixelReport) -> Ask {
    let (w, h) = (moment.width, moment.height);
    let short = w.min(h) as f32;
    tunnel(raster, moment);

    // The face.
    let radius = short * 0.23 * (1.0 + 0.14 * moment.kick.min(1.2));
    let (fx, fy) = (w as f32 * 0.5, h as f32 * 0.5 + short * 0.02);
    let shock = moment.kick > 0.8;
    sprites::face(
        raster,
        fx,
        fy,
        radius,
        &Face {
            fur: Rgb::new(16, 14, 24),
            rim: Rgb::WHITE,
            eye: Rgb::new(200, 240, 90),
            open: 1.0,
            mouth: (moment.vocal * 1.3 - 0.2).clamp(0.0, 1.0),
            shock,
            look: (moment.clock * 1.3).sin() * 0.6,
            happy: moment.energy < 0.25 && moment.line.is_none(),
            whiskers: Rgb::new(200, 200, 220),
        },
    );
    report.cats += 1;
    let lasers = moment.kick > 0.7;
    if lasers {
        for side in [-1.0f32, 1.0] {
            let (ex, ey) = (fx + side * 0.38 * radius, fy - 0.06 * radius);
            for t in 0..2 {
                raster.line(
                    ex as i32,
                    ey as i32 + t,
                    (fx + side * w as f32) as i32,
                    (ey - h as f32 * 0.25) as i32 + t,
                    Rgb::new(255, 60, 80),
                );
            }
        }
    }

    // The sung word, slammed in.
    if let Some(line) = &moment.line {
        let word = line.sung_word();
        if !word.is_empty() && line.typed > 0 {
            let (scale, _) = font::fit(word, w * 9 / 10, (h as f32 * 0.2) as i32, 1, 6);
            let landing = line.sung_since < 0.1;
            let s = if landing { scale + 1 } else { scale };
            say_centred(
                raster,
                report,
                word,
                w / 2,
                6,
                s,
                s,
                Rgb::WHITE,
                Some(Rgb::new(200, 0, 60)),
            );
        }
        subtitle(
            raster,
            report,
            moment,
            line,
            h - 4,
            Rgb::new(255, 244, 228),
            Rgb::hsv(moment.accent_hue(320.0), 0.7, 1.0),
        );
    }
    pops(
        raster,
        report,
        moment,
        (8, h / 6, w - 8, h - h / 4),
        moment.clock * 60.0,
        0.45,
    );
    report.detail = format!(
        "face={radius:.0} lasers={}",
        if lasers { "on" } else { "off" }
    );
    Ask {
        shake: 2.2,
        split: 0.8,
        zoom: 0.03,
        glitch: 0,
    }
}

/// The tunnel: rings at a log distance from the centre, shaped to the frame,
/// scrolling outward with the state's travel, faded toward the vanishing point.
fn tunnel(raster: &mut Raster, moment: &Moment) {
    let (w, h) = (moment.width, moment.height);
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let half = w.min(h) as f32 * 0.5;
    let (aspect_x, aspect_y) = (w as f32 * 0.5 / half, h as f32 * 0.5 / half);
    let angle = 0.25 * (moment.clock * 0.7).sin();
    let (sin, cos) = angle.sin_cos();
    let a = Rgb::hsv(moment.clock * 50.0, 0.8, 0.95);
    let b = Rgb::hsv(moment.clock * 50.0 + 180.0, 0.75, 0.28);
    let white = Rgb::new(235, 235, 245);
    let travel = moment.travel * 6.0;
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5 - cx) / aspect_x;
            let v = (y as f32 + 0.5 - cy) / aspect_y;
            let (u, v) = (u * cos - v * sin, u * sin + v * cos);
            let d = u.abs().max(v.abs()) + 2.0;
            let ring = (d.ln() * 7.0 - travel).floor() as i64;
            let base = if ring.rem_euclid(6) == 0 {
                white
            } else if ring.rem_euclid(2) == 0 {
                a
            } else {
                b
            };
            let fog = (d / 70.0).clamp(0.08, 1.0);
            raster.put(x, y, base.scale(fog));
        }
    }
}
