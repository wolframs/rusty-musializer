//! A video's look: the palette a section is held to, and the finish over
//! every frame.
//!
//! Post-legacy (2026-10-21, MV9-a/b). The "Door, No Room" video was a
//! Musializer render with a GPU Python pass on top, and most of what that pass
//! did was not specific to the song: it held every scene to one palette and
//! finished every frame the same way. Both belong in the renderer, so a
//! preview, a still and an export are one picture again.
//!
//! ## A grade belongs to a section, and is applied before the transition
//!
//! [`Grade`] rides on a cue of the scene plan, like its transition. The
//! renderer applies it to what the scene just drew and *then* composites a
//! transition over that, which is what makes two things true at once without
//! any blending of grades: the held picture of the outgoing section already
//! carries its own grade, and a transition's own light — a `flash` blowing out
//! to white — is never mapped through a palette, so it cannot come out
//! lavender (which is what the external pass did, and had to detect).
//!
//! ## The finish belongs to the video
//!
//! [`Finish`] is one block for the whole track, applied last: after the
//! caption and after a fade to black, so the type gets the same bloom and
//! grain as the picture under it. Its grain is a function of the frame time,
//! never of a frame counter or an ambient RNG, so an export, a still and a
//! clip of the same moment draw the same specks.
//!
//! The pixel cartridge ([`super::pixel`]) is part of the finish: the first
//! step of it, before the bloom.
//!
//! Everything here is data and arithmetic. The shaders that read it are
//! `resources/shaders/glsl330/post_grade.fs` and `post_finish.fs`; the CPU
//! reference functions below ([`Grade::apply`], [`shoulder`]) restate their
//! formulas so the semantics are pinned by tests rather than only by pictures.

use super::pixel::Pixel;

/// Most stops a gradient map may have.
pub const MAX_STOPS: usize = 8;
/// Fewest: a map needs a dark end and a light end.
pub const MIN_STOPS: usize = 2;

/// A section's palette lock.
///
/// The picture's luminance is mapped through a gradient of `stops` (evenly
/// spaced, darkest first) and mixed over the scene's own colour by `map`.
/// Independently, hues within `shift_width` degrees of `shift_from` are
/// turned by `shift_by` — the narrow "greens lean teal" correction a palette
/// with no green in it wants, which a gradient map at partial strength cannot
/// make on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    /// `0xRRGGBB`, darkest first. Only the first `stop_count` are meaningful.
    pub stops: [u32; MAX_STOPS],
    pub stop_count: u8,
    /// How much of the mapped picture replaces the scene's own colour, `0..=1`.
    pub map: f32,
    /// Centre of the hue band to turn, in degrees `0..360`.
    pub shift_from: f32,
    /// How far to turn it, `-180..=180`. Zero is no shift.
    pub shift_by: f32,
    /// Half-width of the band, in degrees, `1..=90`. The turn falls off as a
    /// Gaussian of the distance from `shift_from`.
    pub shift_width: f32,
}

/// Bounds a [`Grade`] is validated against. One place, read by the codec, the
/// treatment parser and `--vocabulary`.
pub mod grade_bounds {
    pub const MAP: (f32, f32) = (0.0, 1.0);
    pub const SHIFT_FROM: (f32, f32) = (0.0, 360.0);
    pub const SHIFT_BY: (f32, f32) = (-180.0, 180.0);
    pub const SHIFT_WIDTH: (f32, f32) = (1.0, 90.0);
}

impl Default for Grade {
    fn default() -> Self {
        Self::from_palette(Palette::NightDoor, 0.0)
    }
}

impl Grade {
    /// A grade from a named palette at strength `map`, with no hue shift.
    #[must_use]
    pub fn from_palette(palette: Palette, map: f32) -> Self {
        let colours = palette.stops();
        let mut stops = [0u32; MAX_STOPS];
        stops[..colours.len()].copy_from_slice(colours);
        Self {
            stops,
            stop_count: colours.len() as u8,
            map,
            shift_from: 125.0,
            shift_by: 0.0,
            shift_width: 32.0,
        }
    }

    /// The meaningful stops.
    #[must_use]
    pub fn stops(&self) -> &[u32] {
        &self.stops[..usize::from(self.stop_count).min(MAX_STOPS)]
    }

    /// Whether this is a value a plan may hold.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        use grade_bounds::*;
        let within = |value: f32, (low, high): (f32, f32)| {
            value.is_finite() && (low..=high).contains(&value)
        };
        (MIN_STOPS..=MAX_STOPS).contains(&usize::from(self.stop_count))
            && self.stops().iter().all(|stop| *stop <= 0x00ff_ffff)
            && within(self.map, MAP)
            && within(self.shift_from, SHIFT_FROM)
            && within(self.shift_by, SHIFT_BY)
            && within(self.shift_width, SHIFT_WIDTH)
    }

    /// Whether it changes a picture at all.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.map <= 0.0 && self.shift_by == 0.0
    }

    /// The gradient's colour at `t` in `0..=1`, linear between neighbouring
    /// stops. The shader reads the same stops the same way.
    #[must_use]
    pub fn gradient(&self, t: f32) -> [f32; 3] {
        let stops = self.stops();
        if stops.is_empty() {
            return [t, t, t];
        }
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let span = (stops.len() - 1).max(1) as f32;
        let position = t * span;
        let index = (position.floor() as usize).min(stops.len().saturating_sub(2));
        let fraction = (position - index as f32).clamp(0.0, 1.0);
        let a = rgb(stops[index]);
        let b = rgb(stops[(index + 1).min(stops.len() - 1)]);
        [
            a[0] + (b[0] - a[0]) * fraction,
            a[1] + (b[1] - a[1]) * fraction,
            a[2] + (b[2] - a[2]) * fraction,
        ]
    }

    /// The CPU statement of `post_grade.fs`: hue shift first, then the map.
    /// Colours are `0..=1` sRGB, as the framebuffer holds them.
    #[must_use]
    pub fn apply(&self, colour: [f32; 3]) -> [f32; 3] {
        let shifted = if self.shift_by == 0.0 {
            colour
        } else {
            let (hue, saturation, value) = rgb_to_hsv(colour);
            let distance = hue_distance(hue, self.shift_from);
            let weight = (-(distance / self.shift_width).powi(2)).exp()
                * if saturation > 0.05 { 1.0 } else { 0.0 };
            hsv_to_rgb(
                hue + weight * self.shift_by,
                saturation * (1.0 - 0.25 * weight),
                value,
            )
        };
        if self.map <= 0.0 {
            return shifted;
        }
        let luma =
            (0.2126 * shifted[0] + 0.7152 * shifted[1] + 0.0722 * shifted[2]).clamp(0.0, 1.0);
        // A gentle gamma: the scenes that want a map are mostly dark, and a
        // straight map spends most of its stops on what is barely there.
        let mapped = self.gradient(luma.powf(0.85));
        let m = self.map.clamp(0.0, 1.0);
        [
            shifted[0] * (1.0 - m) + mapped[0] * m,
            shifted[1] * (1.0 - m) + mapped[1] * m,
            shifted[2] * (1.0 - m) + mapped[2] * m,
        ]
    }
}

/// The named palettes a treatment may ask for by name instead of listing
/// stops. Order and names are a treatment compatibility surface; append only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    /// The "Door, No Room" palette: near-black teal, slate, violet, lavender,
    /// gold, warm white — the cover's room, door and light.
    NightDoor,
    /// Black, oxblood, ember orange, gold, cream.
    Ember,
    /// Ink blue, steel, ice, white.
    Ice,
    /// Black to white.
    Mono,
    /// Brown-black, umber, sand, paper.
    Sepia,
    /// Deep purple, magenta, coral, peach — a synthwave dusk.
    Dusk,
}

impl Palette {
    pub const ALL: [Self; 6] = [
        Self::NightDoor,
        Self::Ember,
        Self::Ice,
        Self::Mono,
        Self::Sepia,
        Self::Dusk,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NightDoor => "night-door",
            Self::Ember => "ember",
            Self::Ice => "ice",
            Self::Mono => "mono",
            Self::Sepia => "sepia",
            Self::Dusk => "dusk",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|palette| palette.name() == name)
    }

    /// The stops, darkest first.
    #[must_use]
    pub fn stops(self) -> &'static [u32] {
        match self {
            // Measured off the compositor that made the video: the same
            // seven stops, re-spaced evenly (the original's positions were
            // 0, .07, .20, .38, .58, .78, 1 — close enough to even that the
            // re-spacing moves no frame of that video visibly).
            Self::NightDoor => &[
                0x05080b, 0x0b1a1e, 0x1d2f3a, 0x4a3f72, 0x9d8ad0, 0xe9c27a, 0xfff7e6,
            ],
            Self::Ember => &[0x070302, 0x3a0d0a, 0xa83a12, 0xf0a030, 0xfff0d0],
            Self::Ice => &[0x03060c, 0x13294a, 0x5a86b0, 0xbfe0f4, 0xffffff],
            Self::Mono => &[0x000000, 0xffffff],
            Self::Sepia => &[0x0b0806, 0x3e2c1e, 0x9c7a55, 0xe8d6b4, 0xfff8ec],
            Self::Dusk => &[0x0a0614, 0x3a1450, 0xa02a7a, 0xf06a5a, 0xffd2a8],
        }
    }
}

/// The finish over every frame of the video.
///
/// All zero is "no finish", which is the default and what nothing serializes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    /// How much bloom is added, `0..=2`.
    pub bloom: f32,
    /// The brightness bloom starts from, `0..=1`.
    pub bloom_threshold: f32,
    /// The bloom's spread as a fraction of the frame's short edge, `0.002..=0.08`.
    pub bloom_radius: f32,
    /// Highlight roll-off, `0..=1`: 0 is a hard clip, 1 starts rolling off at
    /// [`SHOULDER_KNEE_AT_ONE`]. Paper reads as paper instead of a white hole.
    pub shoulder: f32,
    /// Split toning strength, `0..=1`: shadows lean toward `shadow_rgb`,
    /// highlights toward `highlight_rgb`.
    pub tone: f32,
    pub shadow_rgb: u32,
    pub highlight_rgb: u32,
    /// Corner darkening, `0..=1`.
    pub vignette: f32,
    /// Film grain, `0..=1`.
    pub grain: f32,
    /// The pixel cartridge (2026-10-24): the frame drawn at a low resolution
    /// through a palette and a screen, before everything above. Off by
    /// default, and never written when off. A section may override it
    /// ([`super::pixel::PixelFrame`]).
    pub pixel: Pixel,
}

/// Bounds a [`Finish`] is validated against.
pub mod finish_bounds {
    pub const BLOOM: (f32, f32) = (0.0, 2.0);
    pub const BLOOM_THRESHOLD: (f32, f32) = (0.0, 1.0);
    pub const BLOOM_RADIUS: (f32, f32) = (0.002, 0.08);
    pub const UNIT: (f32, f32) = (0.0, 1.0);
}

/// Where the shoulder's knee sits at full strength.
pub const SHOULDER_KNEE_AT_ONE: f32 = 0.72;

impl Default for Finish {
    fn default() -> Self {
        Self::NONE
    }
}

impl Finish {
    /// No finish at all.
    pub const NONE: Self = Self {
        bloom: 0.0,
        bloom_threshold: 0.74,
        bloom_radius: 0.012,
        shoulder: 0.0,
        tone: 0.0,
        shadow_rgb: 0x0b2a30,
        highlight_rgb: 0xffd9a0,
        vignette: 0.0,
        grain: 0.0,
        pixel: Pixel::OFF,
    };

    /// The finish the "Door, No Room" compositor applied, restated in these
    /// units. A starting point a treatment can name (`"finish": "film"`).
    pub const FILM: Self = Self {
        bloom: 0.45,
        bloom_threshold: 0.74,
        bloom_radius: 0.016,
        shoulder: 0.85,
        tone: 0.5,
        shadow_rgb: 0x0b2a30,
        highlight_rgb: 0xffd9a0,
        vignette: 0.45,
        grain: 0.4,
        pixel: Pixel::OFF,
    };

    /// The pixel cartridge as a whole finish (`"finish": "cartridge"`): the
    /// look [`Pixel::cartridge`] names, framed by a soft vignette. No bloom
    /// and no grain — the screen's own glow does the first, and film grain
    /// over whole pixels reads as a dirty lens rather than a screen.
    #[must_use]
    pub fn cartridge() -> Self {
        Self {
            vignette: 0.3,
            pixel: Pixel::cartridge(),
            ..Self::NONE
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.bloom > 0.0
            || self.shoulder > 0.0
            || self.tone > 0.0
            || self.vignette > 0.0
            || self.grain > 0.0
            || self.pixel.is_on()
    }

    /// Whether the finish's own shader has work: everything but the pixel
    /// look, which has its own pass.
    #[must_use]
    pub fn shades(&self) -> bool {
        Self {
            pixel: Pixel::OFF,
            ..*self
        }
        .is_active()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        use finish_bounds::*;
        let within = |value: f32, (low, high): (f32, f32)| {
            value.is_finite() && (low..=high).contains(&value)
        };
        within(self.bloom, BLOOM)
            && within(self.bloom_threshold, BLOOM_THRESHOLD)
            && within(self.bloom_radius, BLOOM_RADIUS)
            && within(self.shoulder, UNIT)
            && within(self.tone, UNIT)
            && within(self.vignette, UNIT)
            && within(self.grain, UNIT)
            && self.shadow_rgb <= 0x00ff_ffff
            && self.highlight_rgb <= 0x00ff_ffff
            && self.pixel.is_valid()
    }

    /// The shoulder's knee for this strength: 1.0 (no roll-off) at zero.
    #[must_use]
    pub fn knee(&self) -> f32 {
        1.0 - (1.0 - SHOULDER_KNEE_AT_ONE) * self.shoulder.clamp(0.0, 1.0)
    }
}

/// The highlight shoulder `post_finish.fs` applies per channel: identity below
/// `knee`, then a rational roll-off that approaches but never passes
/// `knee + 0.2`. Continuous with slope 1 at the knee, so no band appears there.
#[must_use]
pub fn shoulder(value: f32, knee: f32) -> f32 {
    if value <= knee || knee >= 1.0 {
        return value;
    }
    let over = value - knee;
    knee + over / (1.0 + over / 0.2)
}

/// `0xRRGGBB` as `0..=1` floats.
#[must_use]
pub fn rgb(packed: u32) -> [f32; 3] {
    [
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >> 8) & 0xff) as f32 / 255.0,
        (packed & 0xff) as f32 / 255.0,
    ]
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= 1.0e-5 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let saturation = if max > 1.0e-5 { delta / max } else { 0.0 };
    (hue, saturation, max)
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let c = value * saturation;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0e-4)
    }

    #[test]
    fn every_named_palette_is_a_valid_grade_and_names_round_trip() {
        for palette in Palette::ALL {
            let grade = Grade::from_palette(palette, 0.8);
            assert!(grade.is_valid(), "{}", palette.name());
            assert_eq!(Palette::from_name(palette.name()), Some(palette));
            // Darkest first: a reversed palette would invert every picture.
            let first = grade.gradient(0.0);
            let last = grade.gradient(1.0);
            assert!(
                first.iter().sum::<f32>() < last.iter().sum::<f32>(),
                "{}",
                palette.name()
            );
        }
        assert_eq!(Palette::from_name("nope"), None);
    }

    #[test]
    fn the_gradient_hits_its_stops_and_interpolates_between_them() {
        let grade = Grade::from_palette(Palette::Mono, 1.0);
        assert!(close(grade.gradient(0.0), [0.0, 0.0, 0.0]));
        assert!(close(grade.gradient(1.0), [1.0, 1.0, 1.0]));
        assert!(close(grade.gradient(0.25), [0.25, 0.25, 0.25]));
        let door = Grade::from_palette(Palette::NightDoor, 1.0);
        // Seven stops: t = 4/6 lands exactly on the fifth (lavender).
        assert!(close(door.gradient(4.0 / 6.0), rgb(0x9d8ad0)));
        // Out-of-range and non-finite input clamp rather than extrapolate.
        assert!(close(door.gradient(-3.0), rgb(0x05080b)));
        assert!(close(door.gradient(f32::NAN), rgb(0x05080b)));
    }

    #[test]
    fn an_identity_grade_changes_nothing() {
        let grade = Grade::from_palette(Palette::NightDoor, 0.0);
        assert!(grade.is_identity());
        for colour in [[0.1, 0.7, 0.2], [0.9, 0.9, 0.9], [0.0, 0.0, 0.0]] {
            assert!(close(grade.apply(colour), colour));
        }
    }

    #[test]
    fn the_hue_shift_moves_greens_and_leaves_lavender_alone() {
        let grade = Grade {
            map: 0.0,
            shift_from: 125.0,
            shift_by: 52.0,
            shift_width: 32.0,
            ..Grade::from_palette(Palette::NightDoor, 0.0)
        };
        let green = grade.apply([0.2, 0.8, 0.25]);
        let (hue, _, _) = rgb_to_hsv(green);
        assert!(hue > 160.0 && hue < 185.0, "green leaned to teal: {hue}");
        let lavender = [0.72, 0.6, 1.0];
        assert!(
            close(grade.apply(lavender), lavender),
            "violet is far from the band"
        );
        // Grey has no hue to turn.
        assert!(close(grade.apply([0.5, 0.5, 0.5]), [0.5, 0.5, 0.5]));
    }

    #[test]
    fn grade_bounds_refuse_what_the_codec_must_not_load() {
        let good = Grade::from_palette(Palette::Ember, 0.5);
        assert!(good.is_valid());
        for bad in [
            Grade { map: 1.01, ..good },
            Grade {
                map: f32::NAN,
                ..good
            },
            Grade {
                shift_width: 0.0,
                ..good
            },
            Grade {
                shift_by: 181.0,
                ..good
            },
            Grade {
                stop_count: 1,
                ..good
            },
            Grade {
                stop_count: 9,
                ..good
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
        let mut wide = good;
        wide.stops[0] = 0x0100_0000;
        assert!(!wide.is_valid(), "a stop is 24-bit");
    }

    #[test]
    fn the_shoulder_is_continuous_monotone_and_bounded() {
        let knee = Finish::FILM.knee();
        assert!(knee > 0.7 && knee < 0.8);
        let mut last = 0.0;
        for step in 0..=200 {
            let value = step as f32 / 100.0;
            let out = shoulder(value, knee);
            assert!(out >= last - 1.0e-6, "monotone at {value}");
            assert!(out < knee + 0.2 + 1.0e-6, "bounded at {value}");
            last = out;
        }
        assert_eq!(shoulder(0.5, knee), 0.5, "identity below the knee");
        // Slope 1 at the knee: no visible band where the roll-off begins.
        let epsilon = 1.0e-3;
        let slope = (shoulder(knee + epsilon, knee) - knee) / epsilon;
        assert!((slope - 1.0).abs() < 0.01, "{slope}");
        assert_eq!(Finish::NONE.knee(), 1.0, "no finish, no shoulder");
    }

    #[test]
    fn no_finish_is_inactive_and_film_is_valid() {
        assert!(!Finish::NONE.is_active());
        assert!(Finish::NONE.is_valid());
        assert!(Finish::FILM.is_active());
        assert!(Finish::FILM.is_valid());
        assert!(!Finish {
            grain: 1.5,
            ..Finish::FILM
        }
        .is_valid());
        assert!(!Finish {
            bloom_radius: 0.0,
            ..Finish::FILM
        }
        .is_valid());
    }
}
