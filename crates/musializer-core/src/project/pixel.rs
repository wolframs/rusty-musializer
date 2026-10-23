//! The pixel cartridge: a whole video drawn as if a handheld console drew it.
//!
//! Post-legacy (2026-10-24). A model made a video for the operator's
//! chiptune-gabber track in Python: every frame drawn at 320x180, upscaled with
//! a pixel grid, a four-shade green world that "unlocks" into full colour, and
//! kicks answered with shake and colour split instead of flashes. Nothing in it
//! was specific to its scenes, so it belongs here, where it works on every
//! scene: a [`Pixel`] look rides on the [`super::look::Finish`] for the whole
//! video, and a section may switch it off or change it ([`PixelFrame`]).
//!
//! ## Where it sits, and why there
//!
//! The pixel pass is the first step of the finish: after the grade, the
//! transition, the caption and the fade to black. A cartridge has one
//! framebuffer, so everything on screen goes through it — a caption, a
//! `flash` to white and an `iris` out of black included. Drawn anywhere
//! earlier, a transition's smooth edge or a caption's anti-aliased serif would
//! sit on top of the pixels at full resolution, and that is the "cheap mosaic
//! filter" look this exists to avoid.
//!
//! ## Crisp at every size
//!
//! A cell is a **whole number** of output pixels ([`PixelGrid`]), computed on
//! the export's *logical* size and multiplied by the supersample factor, so a
//! 2x supersampled export resolves to exactly the cells a 1x render draws, and
//! the box downsample in `render::resolve` never straddles a cell edge.
//! `pixels` is the short edge's count, so a 9:16 export has the same pixel size
//! as a 16:9 one.
//!
//! ## Deterministic by construction
//!
//! The dither is an ordered (Bayer) matrix over integer cell indices; the
//! shake is an integer hash of a tick of *track time*; the hit strength is a
//! stateless function of the frame's own bands and trails, which a still's
//! replay reproduces. Nothing reads a frame counter, a clock or an RNG.
//!
//! The shaders are `resources/shaders/glsl330/post_pixel.fs` (the console:
//! downsample, palette, dither) and `post_pixel_display.fs` (the screen:
//! upscale, split, grid, scanlines, curvature, glow). The CPU functions below
//! restate their arithmetic so tests pin the semantics.

use super::look::rgb;

/// Most colours a pixel palette may have: the size of a 16-colour home
/// computer's set.
pub const MAX_COLOURS: usize = 16;
/// Fewest: a ramp needs a dark end and a light end.
pub const MIN_COLOURS: usize = 2;

/// Bounds a [`Pixel`] is validated against. One place, read by the codec, the
/// treatment parser, `--vocabulary` and the Tune panel.
pub mod pixel_bounds {
    /// Pixels across the frame's short edge. 48 is a postage stamp, 720 is
    /// barely a filter at 1080p; the classic handheld is 144, a 16:9
    /// cartridge 180.
    pub const PIXELS: (u16, u16) = (48, 720);
    pub const LEVELS: (u8, u8) = (2, 16);
    pub const UNIT: (f32, f32) = (0.0, 1.0);
}

/// The pixel count a look turned on without one starts from: a 320x180
/// frame at 16:9, the source video's own.
pub const DEFAULT_PIXELS: u16 = 180;

/// How a palette holds the picture's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matching {
    /// Pixels only: every colour the scene drew survives, averaged per cell.
    None,
    /// The colours are a ramp, darkest first, and the picture's luminance
    /// picks one — how a four-shade handheld screen shows a colour world.
    Ramp,
    /// Each cell takes the nearest colour of the set, dithered — how a home
    /// computer's fixed palette shows one.
    Nearest,
    /// Each channel is held to `levels` steps: a console's colour depth, with
    /// no fixed set.
    Levels,
}

impl Matching {
    pub const ALL: [Self; 4] = [Self::None, Self::Ramp, Self::Nearest, Self::Levels];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ramp => "ramp",
            Self::Nearest => "nearest",
            Self::Levels => "levels",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|matching| matching.name() == name)
    }

    /// Whether this matching reads a colour set.
    #[must_use]
    pub fn uses_colours(self) -> bool {
        matches!(self, Self::Ramp | Self::Nearest)
    }

    /// The integer the shaders switch on.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// The named colour sets. Order and names are a treatment compatibility
/// surface; append only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelPalette {
    /// Four shades of handheld green, darkest first: the classic pea-soup
    /// screen.
    Handheld,
    /// Four greys, a later handheld's screen.
    Grey,
    /// Sixteen colours of an 8-bit home computer, matched by nearest colour.
    Home,
    /// Sixteen colours of a fantasy console: brighter and warmer than the home
    /// computer's, matched by nearest colour.
    Fantasy,
}

impl PixelPalette {
    pub const ALL: [Self; 4] = [Self::Handheld, Self::Grey, Self::Home, Self::Fantasy];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Handheld => "handheld",
            Self::Grey => "grey",
            Self::Home => "home",
            Self::Fantasy => "fantasy",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|palette| palette.name() == name)
    }

    #[must_use]
    pub fn matching(self) -> Matching {
        match self {
            Self::Handheld | Self::Grey => Matching::Ramp,
            Self::Home | Self::Fantasy => Matching::Nearest,
        }
    }

    /// One line on how it looks, for `--vocabulary` and the Tune panel.
    #[must_use]
    pub fn look(self) -> &'static str {
        match self {
            Self::Handheld => "four shades of handheld green, by luminance",
            Self::Grey => "four greys, by luminance",
            Self::Home => "sixteen colours of an 8-bit home computer, nearest colour",
            Self::Fantasy => "sixteen bright fantasy-console colours, nearest colour",
        }
    }

    /// The colours. A ramp is darkest first.
    #[must_use]
    pub fn colours(self) -> &'static [u32] {
        match self {
            // The handheld's four greens as a popular emulator shows them
            // rather than as measured: the real top two shades are nearly one
            // colour, and a scene that reaches them flattens into soup.
            Self::Handheld => &[0x081820, 0x346856, 0x88c070, 0xe0f8d0],
            Self::Grey => &[0x1c1c1c, 0x5a5a5a, 0xa5a5a5, 0xe8e8e8],
            // A measured home-computer palette, in its hardware order (the
            // order is irrelevant to nearest matching).
            Self::Home => &[
                0x000000, 0xffffff, 0x68372b, 0x70a4b2, 0x6f3d86, 0x588d43, 0x352879, 0xb8c76f,
                0x6f4f25, 0x433900, 0x9a6759, 0x444444, 0x6c6c6c, 0x9ad284, 0x6c5eb5, 0x959595,
            ],
            Self::Fantasy => &[
                0x000000, 0x1d2b53, 0x7e2553, 0x008751, 0xab5236, 0x5f574f, 0xc2c3c7, 0xfff1e8,
                0xff004d, 0xffa300, 0xffec27, 0x00e436, 0x29adff, 0x83769c, 0xff77a8, 0xffccaa,
            ],
        }
    }
}

/// The pixel look: a frame drawn at a low resolution, its colours held to a
/// palette with ordered dithering, shown through a screen.
///
/// `pixels == 0` is "off", which is the default and what nothing serializes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pixel {
    /// Pixels across the frame's short edge, or 0 for no pixel look.
    pub pixels: u16,
    pub matching: Matching,
    /// `0xRRGGBB`. Only the first `colour_count` are meaningful, and only for
    /// [`Matching::Ramp`] (darkest first) and [`Matching::Nearest`].
    pub colours: [u32; MAX_COLOURS],
    pub colour_count: u8,
    /// Steps per channel for [`Matching::Levels`], `2..=16`. Carried, and
    /// validated, whatever the matching, so a section can switch to it.
    pub levels: u8,
    /// Ordered dithering, `0..=1`: 0 rounds each cell to its nearest step, 1
    /// spreads the in-between tones across a 4x4 pattern.
    pub dither: f32,
    /// The pixel grid, `0..=1`: a dark line along each cell's top and left
    /// edge, like the gaps between a handheld's LCD cells.
    pub grid: f32,
    /// CRT scanlines, `0..=1`: each row of cells darkens toward its top and
    /// bottom, so it reads as a beam.
    pub scanlines: f32,
    /// Screen curvature, `0..=1`. Bends the picture into a tube and darkens
    /// outside it; 0 keeps every pixel straight.
    pub curvature: f32,
    /// Phosphor glow, `0..=1`: lit cells bleed soft light into their
    /// neighbours and across the grid.
    pub glow: f32,
    /// On a kick, the red and blue of the picture part sideways by whole
    /// pixels, `0..=1`.
    pub split: f32,
    /// On a kick, the screen jumps by whole pixels, `0..=1`.
    pub shake: f32,
    /// On a kick, the picture punches in, `0..=1`.
    pub zoom: f32,
}

impl Default for Pixel {
    fn default() -> Self {
        Self::OFF
    }
}

/// How far a full-strength split parts the channels at a full hit, in cells.
pub const MAX_SPLIT_CELLS: f32 = 2.5;
/// How far a full-strength shake throws the screen at a full hit, in cells.
pub const MAX_SHAKE_CELLS: f32 = 3.0;
/// How far a full-strength zoom punches in at a full hit: 8 % larger.
pub const MAX_ZOOM: f32 = 0.08;
/// The shake's direction changes this many times a second of *track time*:
/// fast enough to read as a jolt, slow enough that a 24 fps export and a
/// 60 fps preview see the same jolts.
pub const SHAKE_TICKS_PER_SECOND: f64 = 30.0;

/// A ramp's luminance is read through this gamma before it picks a shade.
/// Most scenes are a few bright things on a near-black ground, and a straight
/// four-way split of luminance would spend three shades on what is barely
/// there and leave the ground and its glow one flat dark green.
pub const RAMP_GAMMA: f32 = 0.75;
/// How far the dither spreads a colour before nearest matching, at
/// `dither == 1`: about the distance between neighbouring colours of a
/// 16-colour set.
pub const NEAREST_SPREAD: f32 = 0.28;

impl Pixel {
    /// No pixel look.
    pub const OFF: Self = Self {
        pixels: 0,
        matching: Matching::None,
        colours: [0; MAX_COLOURS],
        colour_count: 0,
        // Eight steps a channel, 512 colours: a 16-bit console's depth, the
        // step after a fixed palette.
        levels: 8,
        dither: 0.0,
        grid: 0.0,
        scanlines: 0.0,
        curvature: 0.0,
        glow: 0.0,
        split: 0.0,
        shake: 0.0,
        zoom: 0.0,
    };

    /// The look a pixel block starts from when it is turned on: 180 handheld
    /// greens, lightly dithered, a light grid, a few scanlines and a little
    /// glow, standing still. The hit responses are off until asked for, so a
    /// look turned on in the Tune panel does not start moving the picture.
    ///
    /// The dither is a third rather than a half because a video is mostly
    /// flat grounds: at a half, every mid-toned backdrop becomes a full-screen
    /// checkerboard; at a third, only tones near the middle of a step break
    /// up, which is where a pixel artist would dither by hand.
    #[must_use]
    pub fn on() -> Self {
        Self {
            pixels: DEFAULT_PIXELS,
            dither: 0.35,
            grid: 0.25,
            scanlines: 0.25,
            glow: 0.35,
            ..Self::OFF
        }
        .with_palette(PixelPalette::Handheld)
    }

    /// The cartridge: [`Self::on`] with the source video's answer to a kick —
    /// a jolt, a punch-in and the colours parting.
    #[must_use]
    pub fn cartridge() -> Self {
        Self {
            split: 0.6,
            shake: 0.5,
            zoom: 0.4,
            ..Self::on()
        }
    }

    /// This look with a named palette's colours and matching.
    #[must_use]
    pub fn with_palette(mut self, palette: PixelPalette) -> Self {
        let colours = palette.colours();
        self.colours = [0; MAX_COLOURS];
        self.colours[..colours.len()].copy_from_slice(colours);
        self.colour_count = colours.len() as u8;
        self.matching = palette.matching();
        self
    }

    /// This look with no colour set: `Matching::None` or `Matching::Levels`.
    #[must_use]
    pub fn without_colours(mut self, matching: Matching) -> Self {
        debug_assert!(!matching.uses_colours());
        self.colours = [0; MAX_COLOURS];
        self.colour_count = 0;
        self.matching = matching;
        self
    }

    /// The named palette this look's colours are, if they are one.
    #[must_use]
    pub fn named_palette(&self) -> Option<PixelPalette> {
        PixelPalette::ALL.into_iter().find(|palette| {
            palette.matching() == self.matching && palette.colours() == self.colours()
        })
    }

    /// Words for the palette, for the report line and the Tune panel:
    /// a named palette's name, `none`, `levels6`, or `ramp4`/`nearest7` for a
    /// listed set.
    #[must_use]
    pub fn palette_label(&self) -> String {
        if let Some(palette) = self.named_palette() {
            return palette.name().to_owned();
        }
        match self.matching {
            Matching::None => "none".to_owned(),
            Matching::Levels => format!("levels{}", self.levels),
            other => format!("{}{}", other.name(), self.colour_count),
        }
    }

    #[must_use]
    pub fn is_on(&self) -> bool {
        self.pixels > 0
    }

    /// The meaningful colours.
    #[must_use]
    pub fn colours(&self) -> &[u32] {
        &self.colours[..usize::from(self.colour_count).min(MAX_COLOURS)]
    }

    /// Whether this is a value a project may hold. Off is always valid, and
    /// is normalised to [`Self::OFF`] by every constructor here.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        use pixel_bounds::*;
        if !self.is_on() {
            return *self == Self::OFF;
        }
        let unit = |value: f32| value.is_finite() && (UNIT.0..=UNIT.1).contains(&value);
        let colours_fit = if self.matching.uses_colours() {
            (MIN_COLOURS..=MAX_COLOURS).contains(&usize::from(self.colour_count))
        } else {
            self.colour_count == 0
        };
        (PIXELS.0..=PIXELS.1).contains(&self.pixels)
            && colours_fit
            && self.colours().iter().all(|colour| *colour <= 0x00ff_ffff)
            && self.colours[usize::from(self.colour_count).min(MAX_COLOURS)..]
                .iter()
                .all(|colour| *colour == 0)
            && (LEVELS.0..=LEVELS.1).contains(&self.levels)
            && [
                self.dither,
                self.grid,
                self.scanlines,
                self.curvature,
                self.glow,
                self.split,
                self.shake,
                self.zoom,
            ]
            .into_iter()
            .all(unit)
    }

    /// The CPU statement of `post_pixel.fs`'s colour step: one cell's
    /// averaged colour, held to this look's palette at the cell's place in the
    /// dither pattern. Colours are `0..=1` sRGB, as the framebuffer holds them.
    #[must_use]
    pub fn quantize(&self, colour: [f32; 3], cell_x: i32, cell_y: i32) -> [f32; 3] {
        let colour = colour.map(|channel| {
            if channel.is_finite() {
                channel.clamp(0.0, 1.0)
            } else {
                0.0
            }
        });
        let threshold = 0.5 + (bayer4(cell_x, cell_y) - 0.5) * self.dither.clamp(0.0, 1.0);
        match self.matching {
            Matching::None => colour,
            Matching::Levels => {
                // Steps evenly spaced in the square root of the value, not in
                // the value: most scenes here are a few bright things on a
                // near-black ground, and eight linear steps put the first one
                // at 14 % — every glow and trail below it becomes a speckle of
                // black and dark grey. In root space the first step is 2 %.
                let steps = f32::from(self.levels.max(2) - 1);
                colour.map(|channel| {
                    let step = ((channel.sqrt() * steps + threshold).floor() / steps).min(1.0);
                    step * step
                })
            }
            Matching::Ramp => {
                let colours = self.colours();
                if colours.is_empty() {
                    return colour;
                }
                let luma = (0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2])
                    .clamp(0.0, 1.0)
                    .powf(RAMP_GAMMA);
                let top = colours.len() - 1;
                let index = ((luma * top as f32 + threshold).floor().max(0.0) as usize).min(top);
                rgb(colours[index])
            }
            Matching::Nearest => {
                let colours = self.colours();
                if colours.is_empty() {
                    return colour;
                }
                let offset = (threshold - 0.5) * NEAREST_SPREAD * 2.0;
                let probe = colour.map(|channel| channel + offset);
                let mut best = rgb(colours[0]);
                let mut best_distance = f32::INFINITY;
                for &packed in colours {
                    let candidate = rgb(packed);
                    let distance = colour_distance(probe, candidate);
                    if distance < best_distance {
                        best_distance = distance;
                        best = candidate;
                    }
                }
                best
            }
        }
    }

    /// How far the channels part this frame, in whole cells.
    #[must_use]
    pub fn split_cells(&self, hit: f32) -> i32 {
        (self.split * hit.clamp(0.0, 1.0) * MAX_SPLIT_CELLS).round() as i32
    }

    /// How far the picture punches in this frame: a scale of at least 1.
    #[must_use]
    pub fn zoom_scale(&self, hit: f32) -> f32 {
        1.0 + self.zoom * hit.clamp(0.0, 1.0) * MAX_ZOOM
    }

    /// Where the screen has jumped to this frame, in whole cells: an integer
    /// hash of the tick of track time, so a still and a clip of one moment
    /// jolt alike.
    #[must_use]
    pub fn shake_cells(&self, hit: f32, time_seconds: f64) -> (i32, i32) {
        let amplitude = self.shake * hit.clamp(0.0, 1.0) * MAX_SHAKE_CELLS;
        if amplitude < 0.5 {
            return (0, 0);
        }
        let tick = (time_seconds.max(0.0) * SHAKE_TICKS_PER_SECOND).floor() as u64 as u32;
        let unit = |salt: u32| {
            let bits = hash(tick ^ salt);
            (f64::from(bits & 0xffff) / 32767.5 - 1.0) as f32
        };
        (
            (unit(0x5ba1_e2d3) * amplitude).round() as i32,
            (unit(0x1c37_9e41) * amplitude).round() as i32,
        )
    }
}

/// A weighted squared distance between two colours: green counts most, as it
/// does to the eye, so nearest matching keeps a picture's brightness.
fn colour_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    0.30 * dr * dr + 0.59 * dg * dg + 0.11 * db * db
}

/// The 4x4 ordered-dither threshold of a cell, in `(0, 1)`: the classic Bayer
/// matrix, read by integer cell index so it never moves with the frame.
#[must_use]
pub fn bayer4(x: i32, y: i32) -> f32 {
    (bayer_index(x, y, 2) as f32 + 0.5) / 16.0
}

/// The 8x8 threshold, for the dissolve between two sections' looks: 64 steps,
/// so a two-second dissolve at 30 fps has a new cell turning on most frames.
#[must_use]
pub fn bayer8(x: i32, y: i32) -> f32 {
    (bayer_index(x, y, 3) as f32 + 0.5) / 64.0
}

/// The recursive Bayer index over `bits` levels: the finest 2x2 pattern is the
/// most significant, which is what spreads consecutive thresholds furthest
/// apart. `post_pixel*.fs` computes the same loop.
fn bayer_index(x: i32, y: i32, bits: u32) -> u32 {
    let (x, y) = (x as u32, y as u32);
    let mut value = 0u32;
    for level in 0..bits {
        let xb = (x >> level) & 1;
        let yb = (y >> level) & 1;
        value = value * 4 + (((xb ^ yb) << 1) | yb);
    }
    value
}

/// A 32-bit integer hash (multiply-xorshift). Integers have no last bit to
/// tear on, which is the `fract` trap in `AGENTS.md`.
#[must_use]
pub fn hash(value: u32) -> u32 {
    let mut h = value.wrapping_mul(0x8da6_b343);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// A kick's strength in this frame, `0..=1`: the mean positive excursion of
/// the lowest quarter of the bands over their own trails — the measure
/// Clawd's kick detector reads — eased between a floor and a ceiling.
///
/// Stateless on purpose. The trails already decay behind the bands, so the
/// excursion rises on the hit and falls away after it like an envelope, and a
/// still's replay reproduces it exactly because the analyzer is replayed; an
/// envelope kept in the renderer would not be.
#[must_use]
pub fn hit_strength(bands: &[f32], trails: &[f32]) -> f32 {
    let count = bands.len().min(trails.len());
    if count == 0 {
        return 0.0;
    }
    let low = (count / 4).max(1);
    let excursion = bands[..low]
        .iter()
        .zip(&trails[..low])
        .map(|(band, trail)| (band - trail).max(0.0))
        .sum::<f32>()
        / low as f32;
    if !excursion.is_finite() {
        return 0.0;
    }
    let t = ((excursion - HIT_FLOOR) / (HIT_CEILING - HIT_FLOOR)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Below this excursion nothing moves: Clawd's [`crate::scenes::clawd::KICK_THRESHOLD`]
/// region, a little under it, so a soft kick still nudges.
pub const HIT_FLOOR: f32 = 0.035;
/// At this excursion a hit is full strength.
pub const HIT_CEILING: f32 = 0.16;

/// The cell geometry of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelGrid {
    /// A cell's edge in framebuffer pixels: a whole number, at least 1.
    pub cell: u32,
    /// Cells across and down, partial cells at the edges included.
    pub columns: u32,
    pub rows: u32,
    /// How far the grid starts before the frame's left and top edges, in
    /// framebuffer pixels, so the partial cells are shared between both sides
    /// and the picture's centre is a cell corner or a cell centre.
    pub offset_x: u32,
    pub offset_y: u32,
}

impl PixelGrid {
    /// The grid for a `width` x `height` framebuffer rectangle drawn at
    /// `scale` framebuffer pixels per output pixel (an export's supersample
    /// factor; 1 in the preview).
    ///
    /// The cell is worked out on the **output** size and then multiplied by
    /// the scale, which is what makes a 2x export's cells exactly twice a 1x
    /// render's, on the same boundaries — so the box downsample lands every
    /// cell on whole output pixels. A scale that is not a whole number falls
    /// back to the framebuffer size, which is still crisp, only not
    /// identical.
    #[must_use]
    pub fn new(pixels: u16, width: u32, height: u32, scale: f32) -> Self {
        let pixels = u32::from(pixels.max(1));
        let factor = if scale.is_finite() && scale >= 1.0 && (scale - scale.round()).abs() < 1.0e-3
        {
            scale.round() as u32
        } else {
            1
        };
        let (out_width, out_height) = ((width / factor).max(1), (height / factor).max(1));
        let short = out_width.min(out_height);
        let out_cell = ((short as f32 / pixels as f32).round() as u32).max(1);
        let columns = out_width.div_ceil(out_cell);
        let rows = out_height.div_ceil(out_cell);
        Self {
            cell: out_cell * factor,
            columns,
            rows,
            offset_x: (columns * out_cell - out_width) / 2 * factor,
            offset_y: (rows * out_cell - out_height) / 2 * factor,
        }
    }

    /// The cell a framebuffer pixel (counted from the rectangle's top left)
    /// falls in.
    #[must_use]
    pub fn cell_of(&self, x: u32, y: u32) -> (u32, u32) {
        (
            (x + self.offset_x) / self.cell,
            (y + self.offset_y) / self.cell,
        )
    }
}

/// The pixel look in force on one frame: one look, or a dissolve from one to
/// another while a section arrives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelFrame {
    pub from: Pixel,
    pub to: Pixel,
    /// `0` is all `from`, `1` all `to`. Each cell crosses over when this
    /// passes its place in the 8x8 Bayer order, so the change sweeps the
    /// screen as a dither — the way a cartridge cross-fades.
    pub mix: f32,
}

impl PixelFrame {
    /// One look, not dissolving.
    #[must_use]
    pub fn steady(pixel: Pixel) -> Self {
        Self {
            from: pixel,
            to: pixel,
            mix: 1.0,
        }
    }

    /// Whether the pass has anything to draw.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.to.is_on() || (self.mix < 1.0 && self.from.is_on())
    }

    /// Whether a cell shows `to` rather than `from`.
    #[must_use]
    pub fn shows_to(&self, cell_x: i32, cell_y: i32) -> bool {
        self.mix >= 1.0 || bayer8(cell_x, cell_y) < self.mix
    }

    /// The look whose grid the frame is cut into. Two looks of different
    /// pixel counts cannot share one grid, so it changes halfway; an off side
    /// has no grid at all.
    #[must_use]
    pub fn grid_look(&self) -> &Pixel {
        if !self.from.is_on() || (self.to.is_on() && self.mix >= 0.5) {
            &self.to
        } else {
            &self.from
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_palette_is_a_valid_look_and_names_round_trip() {
        for palette in PixelPalette::ALL {
            let look = Pixel::on().with_palette(palette);
            assert!(look.is_valid(), "{}", palette.name());
            assert_eq!(PixelPalette::from_name(palette.name()), Some(palette));
            assert_eq!(look.named_palette(), Some(palette));
            assert_eq!(look.palette_label(), palette.name());
        }
        assert_eq!(PixelPalette::from_name("gameboy"), None);
        for matching in Matching::ALL {
            assert_eq!(Matching::from_name(matching.name()), Some(matching));
        }
        // A ramp is darkest first, or every picture would come out inverted.
        for palette in [PixelPalette::Handheld, PixelPalette::Grey] {
            let luma = |packed: u32| {
                let [r, g, b] = rgb(packed);
                0.2126 * r + 0.7152 * g + 0.0722 * b
            };
            let colours = palette.colours();
            assert!(colours.windows(2).all(|pair| luma(pair[0]) < luma(pair[1])));
        }
    }

    #[test]
    fn off_is_the_default_valid_and_normalised() {
        assert_eq!(Pixel::default(), Pixel::OFF);
        assert!(Pixel::OFF.is_valid());
        assert!(!Pixel::OFF.is_on());
        // An "off" carrying leftover members is not a value a file may hold:
        // off is spelled by leaving the block out.
        assert!(!Pixel {
            dither: 0.5,
            ..Pixel::OFF
        }
        .is_valid());
        assert!(Pixel::on().is_valid() && Pixel::cartridge().is_valid());
    }

    #[test]
    fn bounds_refuse_what_the_codec_must_not_load() {
        let good = Pixel::on();
        for bad in [
            Pixel { pixels: 47, ..good },
            Pixel {
                pixels: 721,
                ..good
            },
            Pixel { levels: 1, ..good },
            Pixel { levels: 17, ..good },
            Pixel {
                dither: 1.5,
                ..good
            },
            Pixel {
                glow: f32::NAN,
                ..good
            },
            Pixel {
                colour_count: 1,
                ..good
            },
            Pixel {
                colour_count: 17,
                ..good
            },
            Pixel {
                matching: Matching::Levels,
                ..good
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
        let mut wide = good;
        wide.colours[0] = 0x0100_0000;
        assert!(!wide.is_valid(), "a colour is 24-bit");
        let mut stray = good;
        stray.colours[10] = 0x123456;
        assert!(
            !stray.is_valid(),
            "a colour past the count is garbage, not data"
        );
        assert!(good.without_colours(Matching::Levels).is_valid());
        assert!(good.without_colours(Matching::None).is_valid());
    }

    #[test]
    fn the_bayer_matrices_are_the_classic_ones_and_permutations() {
        let expected: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(bayer_index(x, y, 2), expected[(y * 4 + x) as usize]);
            }
        }
        let mut seen = [false; 64];
        for y in 0..8 {
            for x in 0..8 {
                seen[bayer_index(x, y, 3) as usize] = true;
            }
        }
        assert!(seen.iter().all(|hit| *hit), "every 8x8 threshold once");
        // Periodic, and defined for negative cells (a shake can reach them).
        assert_eq!(bayer4(-1, -1), bayer4(3, 3));
        assert_eq!(bayer8(9, 2), bayer8(1, 2));
    }

    #[test]
    fn a_ramp_maps_luminance_to_shades_and_dither_only_moves_the_in_between() {
        let hard = Pixel {
            dither: 0.0,
            ..Pixel::on()
        };
        let shades: Vec<[f32; 3]> = PixelPalette::Handheld
            .colours()
            .iter()
            .map(|colour| rgb(*colour))
            .collect();
        assert_eq!(hard.quantize([0.0, 0.0, 0.0], 0, 0), shades[0]);
        assert_eq!(hard.quantize([1.0, 1.0, 1.0], 3, 1), shades[3]);
        // Every output is one of the four, whatever goes in.
        let dithered = Pixel {
            dither: 1.0,
            ..hard
        };
        let mut used = std::collections::BTreeSet::new();
        for step in 0..=40 {
            let v = step as f32 / 40.0;
            for (x, y) in [(0, 0), (1, 0), (2, 3), (3, 1)] {
                let out = dithered.quantize([v, v * 0.8, v * 0.3], x, y);
                assert!(shades.contains(&out), "{out:?}");
                used.insert(shades.iter().position(|shade| *shade == out).unwrap());
            }
        }
        assert_eq!(used.len(), 4, "the whole ramp is reachable");
        // A mid grey dithers: across one 4x4 tile it lands on two shades.
        let tile: std::collections::BTreeSet<usize> = (0..16)
            .map(|i| {
                let out = dithered.quantize([0.3, 0.3, 0.3], i % 4, i / 4);
                shades.iter().position(|shade| *shade == out).unwrap()
            })
            .collect();
        assert_eq!(tile.len(), 2, "{tile:?}");
        // Without dither the same grey is one shade everywhere.
        let flat: std::collections::BTreeSet<usize> = (0..16)
            .map(|i| {
                let out = hard.quantize([0.3, 0.3, 0.3], i % 4, i / 4);
                shades.iter().position(|shade| *shade == out).unwrap()
            })
            .collect();
        assert_eq!(flat.len(), 1);
    }

    #[test]
    fn nearest_picks_the_closest_colour_and_levels_hold_each_channel() {
        let home = Pixel {
            dither: 0.0,
            ..Pixel::on().with_palette(PixelPalette::Home)
        };
        assert_eq!(home.quantize([0.98, 0.98, 0.98], 0, 0), rgb(0xffffff));
        assert_eq!(home.quantize([0.02, 0.0, 0.03], 0, 0), rgb(0x000000));
        let set: Vec<[f32; 3]> = home.colours().iter().map(|c| rgb(*c)).collect();
        for step in 0..30 {
            let v = step as f32 / 29.0;
            let out = Pixel {
                dither: 1.0,
                ..home
            }
            .quantize([v, 1.0 - v, 0.5], step, 2);
            assert!(set.contains(&out));
        }
        let levels = Pixel {
            levels: 3,
            dither: 0.0,
            ..Pixel::on().without_colours(Matching::Levels)
        };
        // Three steps in root space: 0, a quarter, 1.
        assert_eq!(levels.quantize([0.01, 0.3, 0.9], 0, 0), [0.0, 0.25, 1.0]);
        // A step value is a fixed point at full dither.
        let dithered = Pixel {
            dither: 1.0,
            ..levels
        };
        for i in 0..16 {
            assert_eq!(
                dithered.quantize([0.25, 0.0, 1.0], i % 4, i / 4),
                [0.25, 0.0, 1.0]
            );
        }
        // Eight steps keep a dim glow: the first is 2 %, not 14 %.
        let eight = Pixel::on().without_colours(Matching::Levels);
        assert_eq!(eight.levels, 8);
        let dim = eight.quantize([0.03, 0.03, 0.03], 0, 0);
        assert!(dim[0] > 0.0 && dim[0] < 0.05, "{dim:?}");
        let none = Pixel::on().without_colours(Matching::None);
        assert_eq!(none.quantize([0.2, 0.3, 0.4], 1, 1), [0.2, 0.3, 0.4]);
    }

    #[test]
    fn a_cell_is_whole_pixels_and_a_supersampled_grid_is_the_same_grid() {
        let grid = PixelGrid::new(180, 1920, 1080, 1.0);
        assert_eq!((grid.cell, grid.columns, grid.rows), (6, 320, 180));
        assert_eq!((grid.offset_x, grid.offset_y), (0, 0));
        let doubled = PixelGrid::new(180, 3840, 2160, 2.0);
        assert_eq!(
            (doubled.cell, doubled.columns, doubled.rows),
            (12, 320, 180)
        );
        // A count that does not divide the frame: whole cells anyway, the
        // partial ones shared between both edges, and the 2x grid lands on
        // exactly the 1x boundaries — even offsets — so the box downsample
        // never straddles a cell.
        let odd = PixelGrid::new(160, 1920, 1080, 1.0);
        assert_eq!(odd.cell, 7);
        assert_eq!(odd.columns, 275);
        assert_eq!(odd.offset_x, 2);
        let odd2 = PixelGrid::new(160, 3840, 2160, 2.0);
        assert_eq!(odd2.cell, 14);
        assert_eq!((odd2.columns, odd2.rows), (odd.columns, odd.rows));
        assert_eq!(
            (odd2.offset_x, odd2.offset_y),
            (odd.offset_x * 2, odd.offset_y * 2)
        );
        // The short edge sets the size: a portrait export has the same pixel.
        let portrait = PixelGrid::new(180, 1080, 1920, 1.0);
        assert_eq!(
            (portrait.cell, portrait.columns, portrait.rows),
            (6, 180, 320)
        );
        // A preview too small for the count draws one pixel per cell, never 0.
        assert_eq!(PixelGrid::new(720, 300, 200, 1.0).cell, 1);
        assert_eq!(grid.cell_of(0, 0), (0, 0));
        assert_eq!(grid.cell_of(1919, 1079), (319, 179));
    }

    #[test]
    fn the_hit_is_a_stateless_kick_envelope() {
        let quiet = [0.3f32; 16];
        assert_eq!(hit_strength(&quiet, &quiet), 0.0, "no excursion, no hit");
        let mut kick = quiet;
        for band in kick.iter_mut().take(4) {
            *band += 0.3;
        }
        assert_eq!(hit_strength(&kick, &quiet), 1.0);
        // Treble alone is not a kick.
        let mut hat = quiet;
        for band in hat.iter_mut().skip(8) {
            *band += 0.3;
        }
        assert_eq!(hit_strength(&hat, &quiet), 0.0);
        assert_eq!(hit_strength(&[], &[]), 0.0);
        let mid = [0.3 + (HIT_FLOOR + HIT_CEILING) / 2.0; 16];
        let half = hit_strength(&mid, &quiet);
        assert!((half - 0.5).abs() < 1.0e-3, "{half}");
    }

    #[test]
    fn the_hit_responses_are_whole_cells_and_deterministic() {
        let look = Pixel::cartridge();
        assert_eq!(look.split_cells(0.0), 0);
        assert_eq!(look.split_cells(1.0), 2);
        assert_eq!(look.zoom_scale(0.0), 1.0);
        assert!(look.zoom_scale(1.0) > 1.03);
        assert_eq!(look.shake_cells(0.0, 12.3), (0, 0), "no hit, no jolt");
        // The same moment jolts the same way; the next tick, differently.
        assert_eq!(look.shake_cells(1.0, 12.3), look.shake_cells(1.0, 12.3));
        let mut moves = std::collections::BTreeSet::new();
        for tick in 0..60 {
            let (x, y) = look.shake_cells(1.0, f64::from(tick) / SHAKE_TICKS_PER_SECOND);
            assert!(x.abs() <= 2 && y.abs() <= 2, "{x},{y}");
            moves.insert((x, y));
        }
        assert!(moves.len() > 6, "a shake goes somewhere: {moves:?}");
        assert_eq!(Pixel::on().shake_cells(1.0, 3.0), (0, 0), "off until asked");
    }

    #[test]
    fn a_dissolve_crosses_each_cell_once_in_bayer_order() {
        let frame = PixelFrame {
            from: Pixel::on(),
            to: Pixel::on().with_palette(PixelPalette::Fantasy),
            mix: 0.0,
        };
        assert!((0..64).all(|i| !frame.shows_to(i % 8, i / 8)));
        let half = PixelFrame { mix: 0.5, ..frame };
        let count = (0..64).filter(|i| half.shows_to(i % 8, i / 8)).count();
        assert_eq!(count, 32);
        assert!((0..64).all(|i| PixelFrame { mix: 1.0, ..frame }.shows_to(i % 8, i / 8)));
        assert!(PixelFrame::steady(Pixel::OFF).grid_look() == &Pixel::OFF);
        assert!(!PixelFrame::steady(Pixel::OFF).is_on());
        let unlocking = PixelFrame {
            from: Pixel::on(),
            to: Pixel::OFF,
            mix: 0.7,
        };
        assert!(unlocking.is_on());
        assert_eq!(
            unlocking.grid_look(),
            &Pixel::on(),
            "an off side has no grid"
        );
    }
}
