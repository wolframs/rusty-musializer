//! The cats: a few hand-placed sprites and one procedural face.
//!
//! Sprites are character maps drawn for this scene — a seated cat, a two-frame
//! gallop, a barbershop singer in a boater and a striped vest, a hooded monk —
//! and every one is drawn through [`draw`], which paints the eyes and the mouth
//! itself so a sprite blinks, looks and sings rather than being a sticker. The
//! big face ([`face`]) is geometry, not a bitmap, so it can be any size from a
//! handheld's inner screen to the whole frame.
//!
//! Letters in a map: `.` is air; `k` fur, `d` fur in shadow, `g` the inside of
//! an ear, `e` an eye and `p` its pupil, `n` the nose, `m` the mouth; `h` straw,
//! `r` red, `w` white, `b` a bow tie; `R`/`D` a robe and its fold, `f` the dark
//! inside of a hood, `o` rope.

use super::raster::{Raster, Rgb};

/// A character-map sprite. Every row has the same width (a test walks them).
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub rows: &'static [&'static str],
}

impl Sprite {
    #[must_use]
    pub fn width(&self) -> i32 {
        self.rows.first().map_or(0, |row| row.len() as i32)
    }

    #[must_use]
    pub fn height(&self) -> i32 {
        self.rows.len() as i32
    }
}

#[rustfmt::skip]
pub const SIT: Sprite = Sprite { rows: &[
    "..k..........k..",
    "..kk........kk..",
    "..kgk......kgk..",
    "..kggkkkkkkggk..",
    ".kkkkkkkkkkkkkk.",
    ".kkkkkkkkkkkkkk.",
    ".kkeekkkkkkeekk.",
    ".kkepkkkkkkpekk.",
    ".kkkkkknnkkkkkk.",
    "..kkkkkmmkkkkk..",
    "...kkkkkkkkkk..k",
    "..kkkkkkkkkkkk.k",
    ".kkkkdkkkkdkkkkk",
    ".kkkkdkkkkdkkkk.",
    ".kkk.kkkkkk.kkk.",
    "..kk..kkkk..kk..",
]};

#[rustfmt::skip]
pub const RUN_A: Sprite = Sprite { rows: &[
    "kk....................",
    ".k.................k.k",
    ".k.................kkk",
    "..k...............kkek",
    "..kkkkkkkkkkkkkkkkkkkn",
    "...kkkkkkkkkkkkkkkkkk.",
    "...kkkkkkkkkkkkkkkkk..",
    "...kkkkkkkkkkkkkkkk...",
    "..kk.............kk...",
    ".kk...............kk..",
    "kk.................kk.",
]};

#[rustfmt::skip]
pub const RUN_B: Sprite = Sprite { rows: &[
    "......................",
    "kk.................k.k",
    ".k.................kkk",
    "..k...............kkek",
    "..kkkkkkkkkkkkkkkkkkkn",
    "...kkkkkkkkkkkkkkkkkk.",
    "....kkkkkkkkkkkkkkkk..",
    "....kkkkkkkkkkkkkkk...",
    ".....kk.kk....kk.kk...",
    "......k..k....k..k....",
    "......................",
]};

#[rustfmt::skip]
pub const BARBER: Sprite = Sprite { rows: &[
    ".k..hhhhhhhh..k.",
    ".kk.hhhhhhhh.kk.",
    ".kgkrrrrrrrrkgk.",
    "hhhhhhhhhhhhhhhh",
    ".kkkkkkkkkkkkkk.",
    ".kkeekkkkkkeekk.",
    ".kkepkkkkkkpekk.",
    ".kkkkkknnkkkkkk.",
    "..kkkkkmmkkkkk..",
    "...kkkkkkkkkk...",
    "....kbbwwbbk....",
    "..rwrwrwwrwrwr..",
    ".krwrwrwwrwrwrk.",
    ".krwrwrwwrwrwrk.",
    ".krwrwrwwrwrwrk.",
    ".krwrwrwwrwrwrk.",
    "..rwrwrwwrwrwr..",
    "...kkk....kkk...",
    "...kkk....kkk...",
    "..kkkk....kkkk..",
]};

#[rustfmt::skip]
pub const MONK: Sprite = Sprite { rows: &[
    "..R..........R..",
    "..RR........RR..",
    "..RRR......RRR..",
    "..RRRRRRRRRRRR..",
    ".RRRRRRRRRRRRRR.",
    ".RRffffffffffRR.",
    ".RRffffffffffRR.",
    ".RRffeffffeffRR.",
    ".RRffffffffffRR.",
    ".RRRffffffffRRR.",
    ".RRRRRRRRRRRRRR.",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRooooRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "RRRRRRRDDRRRRRRR",
    "DDDDDDDDDDDDDDDD",
]};

/// A cat from behind, for an audience.
#[rustfmt::skip]
pub const BACK: Sprite = Sprite { rows: &[
    "..k........k..",
    "..kk......kk..",
    "..kkkkkkkkkk..",
    ".kkkkkkkkkkkk.",
    ".kkkkkkkkkkkk.",
    ".kkkkkkkkkkkk.",
    "..kkkkkkkkkk..",
    "..kkkkkkkkkk..",
    ".kkkkkkkkkkkk.",
    "kkkkkkkkkkkkkk",
]};

/// How a sprite's letters are coloured and how its face is posed.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub fur: Rgb,
    pub eye: Rgb,
    /// The outline drawn one pixel outside the silhouette; `None` for none.
    pub rim: Option<Rgb>,
    /// 0 shut (a line), 1 open.
    pub open: f32,
    /// 0 closed, 1 wide.
    pub mouth: f32,
    /// Pupil offset in sprite pixels, -1..=1.
    pub look: i32,
    /// The robe's colour, for a monk.
    pub robe: Rgb,
}

impl Look {
    #[must_use]
    pub fn cat(fur: Rgb, eye: Rgb) -> Self {
        Self {
            fur,
            eye,
            rim: None,
            open: 1.0,
            mouth: 0.0,
            look: 0,
            robe: Rgb::hex(0x60402a),
        }
    }
}

/// Mouth colours, shared with [`face`].
pub const MOUTH: Rgb = Rgb::new(112, 22, 42);
pub const TONGUE: Rgb = Rgb::new(232, 112, 140);
pub const NOSE: Rgb = Rgb::new(232, 122, 150);
const INNER_EAR: Rgb = Rgb::new(92, 48, 78);

fn letter(c: char, look: &Look) -> Option<Rgb> {
    Some(match c {
        'k' => look.fur,
        'd' => look.fur.scale(0.72),
        'g' => INNER_EAR.mix(look.fur, 0.3),
        'n' => NOSE,
        'h' => Rgb::new(232, 200, 112),
        'r' => Rgb::new(206, 42, 54),
        'w' => Rgb::new(240, 236, 222),
        'b' => Rgb::new(48, 76, 204),
        'R' => look.robe,
        'D' => look.robe.scale(0.68),
        'f' => Rgb::new(12, 9, 16),
        'o' => Rgb::new(208, 176, 96),
        // Eyes and mouth are posed below; here they are only their ground.
        'e' | 'p' => {
            if look.fur.luma() < 60 {
                look.fur
            } else {
                look.fur.scale(0.9)
            }
        }
        'm' => look.fur,
        _ => return None,
    })
}

/// Draws `sprite` with its top-left at `(x, y)`, every map cell a `scale`
/// block, mirrored when `flip`. Returns whether any pixel landed.
pub fn draw(
    raster: &mut Raster,
    sprite: Sprite,
    x: i32,
    y: i32,
    scale: i32,
    flip: bool,
    look: &Look,
) -> bool {
    let s = scale.max(1);
    let width = sprite.width();
    let column = |c: i32| if flip { width - 1 - c } else { c };
    if x >= raster.width()
        || y >= raster.height()
        || x + width * s <= 0
        || y + sprite.height() * s <= 0
    {
        return false;
    }
    if let Some(rim) = look.rim {
        for (row, cells) in sprite.rows.iter().enumerate() {
            for (c, cell) in cells.chars().enumerate() {
                if cell != '.' {
                    let cx = x + column(c as i32) * s;
                    raster.rect(cx - 1, y + row as i32 * s - 1, s + 2, s + 2, rim);
                }
            }
        }
    }
    let mut eyes: Vec<(i32, i32)> = Vec::new();
    let mut pupils: Vec<(i32, i32)> = Vec::new();
    let mut mouth: Vec<(i32, i32)> = Vec::new();
    for (row, cells) in sprite.rows.iter().enumerate() {
        for (c, cell) in cells.chars().enumerate() {
            let Some(color) = letter(cell, look) else {
                continue;
            };
            let at = (x + column(c as i32) * s, y + row as i32 * s);
            raster.rect(at.0, at.1, s, s, color);
            match cell {
                'e' => eyes.push(at),
                'p' => pupils.push(at),
                'm' => mouth.push(at),
                _ => {}
            }
        }
    }
    // Eyes: open, they are the iris with a pupil (shifted by `look`); shut, a
    // line across the pupil row.
    if look.open > 0.3 {
        for &(ex, ey) in eyes.iter().chain(&pupils) {
            raster.rect(ex, ey, s, s, look.eye);
        }
        let shift = look.look.clamp(-1, 1) * s;
        for &(px, py) in &pupils {
            raster.rect(
                px + if flip { -shift } else { shift },
                py,
                s,
                s,
                Rgb::new(10, 8, 12),
            );
        }
        if s >= 2 {
            if let Some(&(ex, ey)) = eyes.first() {
                raster.put(ex, ey, Rgb::WHITE);
            }
        }
    } else {
        // Shut: a lid line along the eyes' lowest row.
        let lowest = eyes.iter().chain(&pupils).map(|&(_, y)| y).max();
        let line = (s / 2).max(1);
        for &(px, py) in eyes.iter().chain(&pupils) {
            if Some(py) == lowest {
                raster.rect(px, py + s - line, s, line, look.eye.scale(0.8));
            }
        }
    }
    if look.mouth > 0.25 {
        for &(mx, my) in &mouth {
            let tall = ((look.mouth * 2.0).round() as i32).clamp(1, 2) * s;
            raster.rect(mx, my, s, tall, MOUTH);
            if tall > s {
                raster.rect(mx, my + tall - s / 2 - 1, s, (s / 2).max(1), TONGUE);
            }
        }
    }
    true
}

/// How the big face is posed.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub fur: Rgb,
    pub rim: Rgb,
    pub eye: Rgb,
    /// 0 shut, 1 open.
    pub open: f32,
    /// 0 closed, 1 wide open.
    pub mouth: f32,
    /// Pupils dilate to round dots: surprise.
    pub shock: bool,
    /// Pupils' sideways glance, -1..=1.
    pub look: f32,
    /// The eyes are happy arcs (`^ ^`) instead of open.
    pub happy: bool,
    pub whiskers: Rgb,
}

/// The big procedural face centred at `(cx, cy)` with head radius `radius`.
/// Returns whether it drew (a face under 3 px has nothing left to read).
pub fn face(raster: &mut Raster, cx: f32, cy: f32, radius: f32, pose: &Face) -> bool {
    let r = radius;
    if !(r.is_finite() && r >= 3.0) {
        return false;
    }
    let ears = |raster: &mut Raster, grow: f32, color: Rgb| {
        for side in [-1.0f32, 1.0] {
            raster.triangle(
                (cx + side * (0.94 * r + grow), cy - 0.28 * r),
                (cx + side * (0.80 * r + grow * 0.5), cy - 1.20 * r - grow),
                (cx + side * (0.20 * r - grow), cy - 0.74 * r - grow),
                color,
            );
        }
    };
    let rim = (r / 20.0).clamp(1.0, 4.0).round();
    ears(raster, rim, pose.rim);
    raster.ellipse(cx, cy, r + rim, r * 0.84 + rim, pose.rim);
    ears(raster, 0.0, pose.fur);
    raster.ellipse(cx, cy, r, r * 0.84, pose.fur);
    for side in [-1.0f32, 1.0] {
        raster.triangle(
            (cx + side * 0.80 * r, cy - 0.42 * r),
            (cx + side * 0.74 * r, cy - 0.98 * r),
            (cx + side * 0.40 * r, cy - 0.70 * r),
            INNER_EAR.mix(pose.fur, 0.25),
        );
    }
    for side in [-1.0f32, 1.0] {
        let (ex, ey) = (cx + side * 0.38 * r, cy - 0.06 * r);
        if pose.happy {
            // A happy squint: an upturned arc of two strokes.
            let w = 0.18 * r;
            let lift = 0.10 * r;
            let thick = (r / 24.0).max(1.0).round() as i32;
            for t in 0..thick {
                raster.line(
                    (ex - w) as i32,
                    (ey + lift) as i32 + t,
                    ex as i32,
                    (ey - lift) as i32 + t,
                    pose.eye,
                );
                raster.line(
                    ex as i32,
                    (ey - lift) as i32 + t,
                    (ex + w) as i32,
                    (ey + lift) as i32 + t,
                    pose.eye,
                );
            }
        } else if pose.open < 0.18 {
            let thick = (r * 0.05).max(1.0).round() as i32;
            raster.rect(
                (ex - 0.2 * r) as i32,
                ey as i32,
                (0.4 * r) as i32,
                thick,
                pose.eye,
            );
        } else {
            raster.ellipse(ex, ey, 0.21 * r, 0.25 * r * pose.open, pose.eye);
            let glance = pose.look.clamp(-1.0, 1.0) * 0.08 * r;
            if pose.shock {
                raster.disc(ex + glance, ey, 0.07 * r, Rgb::new(8, 8, 10));
            } else {
                raster.ellipse(
                    ex + glance,
                    ey,
                    0.065 * r,
                    0.21 * r * pose.open,
                    Rgb::new(8, 8, 10),
                );
            }
            let shine = (r * 0.06).max(1.0).round() as i32;
            raster.rect(
                (ex - 0.12 * r) as i32,
                (ey - 0.15 * r * pose.open) as i32,
                shine,
                shine,
                Rgb::WHITE,
            );
        }
    }
    raster.triangle(
        (cx - 0.10 * r, cy + 0.19 * r),
        (cx + 0.10 * r, cy + 0.19 * r),
        (cx, cy + 0.30 * r),
        NOSE,
    );
    if pose.mouth > 0.05 {
        let open = pose.mouth.clamp(0.0, 1.0);
        raster.ellipse(
            cx,
            cy + 0.44 * r,
            0.17 * r,
            0.05 * r + 0.22 * r * open,
            MOUTH,
        );
        raster.ellipse(
            cx,
            cy + 0.50 * r + 0.11 * r * open,
            0.09 * r,
            0.04 * r + 0.07 * r * open,
            TONGUE,
        );
    } else {
        let (x, y) = (cx as i32, (cy + 0.30 * r) as i32);
        let w = (0.13 * r) as i32;
        let h = (0.10 * r).max(1.0) as i32;
        raster.line(x, y, x - w, y + h, pose.rim);
        raster.line(x, y, x + w, y + h, pose.rim);
    }
    for side in [-1.0f32, 1.0] {
        for k in [-1.0f32, 0.0, 1.0] {
            raster.line(
                (cx + side * 0.32 * r) as i32,
                (cy + 0.30 * r + k * 0.06 * r) as i32,
                (cx + side * 1.06 * r) as i32,
                (cy + 0.22 * r + k * 0.17 * r) as i32,
                pose.whiskers,
            );
        }
    }
    true
}

/// The three-line ASCII cat, in the face's own glyphs.
pub const ASCII_CAT: [&str; 3] = ["/\\_/\\", "=^.^=", "/   \\"];
/// Its sleeping and blinking faces.
pub const ASCII_ASLEEP: &str = "=-.-=";
pub const ASCII_WIDE: &str = "=O.O=";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sprite_is_rectangular_and_uses_only_known_letters() {
        for sprite in [SIT, RUN_A, RUN_B, BARBER, MONK, BACK] {
            let width = sprite.width();
            assert!(width > 0);
            for row in sprite.rows {
                assert_eq!(row.len() as i32, width, "{row}");
                for c in row.chars() {
                    assert!(
                        c == '.' || letter(c, &Look::cat(Rgb::BLACK, Rgb::WHITE)).is_some(),
                        "{c}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_sprite_blinks_and_sings() {
        let mut open = Raster::new(40, 40);
        let mut shut = Raster::new(40, 40);
        let mut singing = Raster::new(40, 40);
        let mut look = Look::cat(Rgb::new(20, 18, 28), Rgb::new(200, 240, 90));
        draw(&mut open, SIT, 2, 2, 2, false, &look);
        look.open = 0.0;
        draw(&mut shut, SIT, 2, 2, 2, false, &look);
        look.open = 1.0;
        look.mouth = 1.0;
        draw(&mut singing, SIT, 2, 2, 2, false, &look);
        assert_ne!(open, shut, "a blink must change the eyes");
        assert_ne!(open, singing, "a sung note must open the mouth");
    }

    #[test]
    fn the_face_draws_at_every_size_and_its_mouth_opens() {
        let pose = Face {
            fur: Rgb::new(16, 14, 24),
            rim: Rgb::WHITE,
            eye: Rgb::new(200, 240, 90),
            open: 1.0,
            mouth: 0.0,
            shock: false,
            look: 0.0,
            happy: false,
            whiskers: Rgb::new(200, 200, 220),
        };
        for radius in [4.0f32, 12.0, 40.0, 90.0] {
            let mut raster = Raster::new(200, 200);
            assert!(face(&mut raster, 100.0, 110.0, radius, &pose));
        }
        let mut closed = Raster::new(100, 100);
        let mut open = Raster::new(100, 100);
        face(&mut closed, 50.0, 55.0, 30.0, &pose);
        face(&mut open, 50.0, 55.0, 30.0, &Face { mouth: 1.0, ..pose });
        assert_ne!(closed, open);
        assert!(!face(&mut closed, 50.0, 50.0, 1.0, &pose));
    }
}
