//! The pixel canvas every Pixel Cats staging draws into.
//!
//! A small RGB framebuffer and the handful of primitives pixel art needs:
//! filled rectangles, Bresenham lines, filled ellipses and triangles judged at
//! pixel centres, vertical gradients and alpha blends. Everything is integer
//! placement on the grid — a coordinate is rounded once, at the call — so a
//! frame is the same bytes on any GPU, which is the reason the scene draws on
//! the CPU at all (see the module docs one level up).
//!
//! Blends are integer arithmetic on `u8` channels with an alpha in 0..=256, so
//! "the same frame twice" is exact rather than close.

/// One opaque colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);

    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `0xRRGGBB`.
    #[must_use]
    pub const fn hex(value: u32) -> Self {
        Self::new((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// Integer Rec. 601 luma, 0..=255.
    #[must_use]
    pub fn luma(self) -> u8 {
        ((77 * u32::from(self.r) + 150 * u32::from(self.g) + 29 * u32::from(self.b)) >> 8) as u8
    }

    /// `self` toward `other` by `amount` in 0..=1.
    #[must_use]
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let a = alpha_256(amount);
        Self::new(
            blend_channel(self.r, other.r, a),
            blend_channel(self.g, other.g, a),
            blend_channel(self.b, other.b, a),
        )
    }

    /// Every channel multiplied by `factor` (clamped to 0..=4), for dimming and
    /// brightening a prop without inventing a second colour for it.
    #[must_use]
    pub fn scale(self, factor: f32) -> Self {
        let f = if factor.is_finite() {
            factor.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let channel = |c: u8| (f32::from(c) * f).round().clamp(0.0, 255.0) as u8;
        Self::new(channel(self.r), channel(self.g), channel(self.b))
    }

    /// A colour from hue (degrees), saturation and value in 0..=1.
    #[must_use]
    pub fn hsv(hue: f32, saturation: f32, value: f32) -> Self {
        let hue = if hue.is_finite() {
            hue.rem_euclid(360.0) / 60.0
        } else {
            0.0
        };
        let s = saturation.clamp(0.0, 1.0);
        let v = value.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
        let m = v - c;
        let (r, g, b) = match hue as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let channel = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self::new(channel(r), channel(g), channel(b))
    }
}

/// `amount` in 0..=1 as an integer weight in 0..=256.
#[must_use]
pub fn alpha_256(amount: f32) -> u32 {
    if amount.is_finite() {
        (amount.clamp(0.0, 1.0) * 256.0).round() as u32
    } else {
        0
    }
}

fn blend_channel(under: u8, over: u8, alpha: u32) -> u8 {
    ((u32::from(under) * (256 - alpha) + u32::from(over) * alpha + 128) >> 8) as u8
}

/// A `width` by `height` RGB framebuffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    width: i32,
    height: i32,
    pixels: Vec<Rgb>,
}

impl Raster {
    #[must_use]
    pub fn new(width: i32, height: i32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            pixels: vec![Rgb::BLACK; (width * height) as usize],
        }
    }

    #[must_use]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[Rgb] {
        &self.pixels
    }

    /// The pixel at `(x, y)`, clamped to the edge.
    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> Rgb {
        let x = x.clamp(0, self.width - 1);
        let y = y.clamp(0, self.height - 1);
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn fill(&mut self, color: Rgb) {
        self.pixels.fill(color);
    }

    pub fn put(&mut self, x: i32, y: i32, color: Rgb) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = color;
        }
    }

    /// Blends `color` over `(x, y)` with `alpha` in 0..=256.
    pub fn blend_256(&mut self, x: i32, y: i32, color: Rgb, alpha: u32) {
        if alpha == 0 || x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let index = (y * self.width + x) as usize;
        if alpha >= 256 {
            self.pixels[index] = color;
            return;
        }
        let under = self.pixels[index];
        self.pixels[index] = Rgb::new(
            blend_channel(under.r, color.r, alpha),
            blend_channel(under.g, color.g, alpha),
            blend_channel(under.b, color.b, alpha),
        );
    }

    pub fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: f32) {
        self.blend_256(x, y, color, alpha_256(alpha));
    }

    /// A filled rectangle, clipped.
    pub fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgb) {
        let (x0, y0) = (x.clamp(0, self.width), y.clamp(0, self.height));
        let (x1, y1) = (
            x.saturating_add(width).clamp(x0, self.width),
            y.saturating_add(height).clamp(y0, self.height),
        );
        for row in y0..y1 {
            let start = (row * self.width) as usize;
            self.pixels[start + x0 as usize..start + x1 as usize].fill(color);
        }
    }

    /// A translucent filled rectangle.
    pub fn rect_alpha(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgb, alpha: f32) {
        let a = alpha_256(alpha);
        if a >= 256 {
            self.rect(x, y, width, height, color);
            return;
        }
        for row in y.max(0)..(y + height).min(self.height) {
            for column in x.max(0)..(x + width).min(self.width) {
                self.blend_256(column, row, color, a);
            }
        }
    }

    /// A one-pixel outline, or `thick` pixels.
    pub fn frame(&mut self, x: i32, y: i32, width: i32, height: i32, thick: i32, color: Rgb) {
        let t = thick.max(1);
        self.rect(x, y, width, t, color);
        self.rect(x, y + height - t, width, t, color);
        self.rect(x, y, t, height, color);
        self.rect(x + width - t, y, t, height, color);
    }

    /// A Bresenham line, both ends included. Long lines are bounded so a wild
    /// coordinate cannot stall the frame.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgb) {
        let (mut x, mut y) = (x0, y0);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut error = dx + dy;
        for _ in 0..4096 {
            self.put(x, y, color);
            if x == x1 && y == y1 {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x += sx;
            }
            if twice <= dx {
                error += dx;
                y += sy;
            }
        }
    }

    /// A filled axis-aligned ellipse: every pixel whose centre is inside.
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgb) {
        self.ellipse_with(cx, cy, rx, ry, |raster, x, y| raster.put(x, y, color));
    }

    /// [`Self::ellipse`], blended.
    pub fn ellipse_alpha(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgb, alpha: f32) {
        let a = alpha_256(alpha);
        self.ellipse_with(cx, cy, rx, ry, |raster, x, y| {
            raster.blend_256(x, y, color, a)
        });
    }

    fn ellipse_with(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        mut paint: impl FnMut(&mut Self, i32, i32),
    ) {
        if !(cx.is_finite() && cy.is_finite() && rx.is_finite() && ry.is_finite()) {
            return;
        }
        let (rx, ry) = (rx.max(0.01), ry.max(0.01));
        let y0 = ((cy - ry).floor() as i32).max(0);
        let y1 = ((cy + ry).ceil() as i32).min(self.height - 1);
        let x0 = ((cx - rx).floor() as i32).max(0);
        let x1 = ((cx + rx).ceil() as i32).min(self.width - 1);
        for y in y0..=y1 {
            let v = (y as f32 + 0.5 - cy) / ry;
            for x in x0..=x1 {
                let u = (x as f32 + 0.5 - cx) / rx;
                if u * u + v * v <= 1.0 {
                    paint(self, x, y);
                }
            }
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, radius: f32, color: Rgb) {
        self.ellipse(cx, cy, radius, radius, color);
    }

    /// A filled triangle, either winding: every pixel whose centre is inside.
    pub fn triangle(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: Rgb) {
        let finite = [a.0, a.1, b.0, b.1, c.0, c.1].iter().all(|v| v.is_finite());
        if !finite {
            return;
        }
        let x0 = (a.0.min(b.0).min(c.0).floor() as i32).max(0);
        let x1 = (a.0.max(b.0).max(c.0).ceil() as i32).min(self.width - 1);
        let y0 = (a.1.min(b.1).min(c.1).floor() as i32).max(0);
        let y1 = (a.1.max(b.1).max(c.1).ceil() as i32).min(self.height - 1);
        let edge = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| {
            (x - q.0) * (p.1 - q.1) - (p.0 - q.0) * (y - q.1)
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let d1 = edge(a, b, px, py);
                let d2 = edge(b, c, px, py);
                let d3 = edge(c, a, px, py);
                let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                if !(negative && positive) {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// Rows `y0..y1` from `top` to `bottom`.
    pub fn gradient(&mut self, y0: i32, y1: i32, top: Rgb, bottom: Rgb) {
        let span = (y1 - y0).max(1);
        for y in y0.max(0)..y1.min(self.height) {
            let color = top.mix(bottom, (y - y0) as f32 / span as f32);
            self.rect(0, y, self.width, 1, color);
        }
    }

    /// Every pixel scaled toward black by `factor` (0 black, 1 unchanged).
    pub fn dim(&mut self, factor: f32) {
        for pixel in &mut self.pixels {
            *pixel = pixel.scale(factor);
        }
    }

    /// Copies `source` into this raster with its top-left at `(x, y)`, every
    /// pixel passed through `map` first.
    pub fn paste_mapped(&mut self, source: &Raster, x: i32, y: i32, map: impl Fn(Rgb) -> Rgb) {
        for row in 0..source.height {
            for column in 0..source.width {
                self.put(x + column, y + row, map(source.get(column, row)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_clip_instead_of_panicking() {
        let mut raster = Raster::new(8, 6);
        raster.rect(-4, -4, 20, 20, Rgb::WHITE);
        assert!(raster.pixels().iter().all(|&p| p == Rgb::WHITE));
        raster.line(-50, -50, 50, 50, Rgb::BLACK);
        raster.ellipse(4.0, 3.0, 100.0, 100.0, Rgb::BLACK);
        raster.triangle((-9.0, -9.0), (90.0, 0.0), (0.0, 90.0), Rgb::WHITE);
        raster.ellipse(f32::NAN, 0.0, 1.0, 1.0, Rgb::WHITE);
        raster.rect_alpha(2, 2, 100, 100, Rgb::BLACK, 0.5);
        raster.rect(7, 5, 30, 30, Rgb::WHITE);
        raster.rect(9, 7, 30, 30, Rgb::WHITE);
        raster.rect(-30, -30, 10, 10, Rgb::WHITE);
    }

    #[test]
    fn a_blend_is_exact_integer_arithmetic() {
        let mut raster = Raster::new(1, 1);
        raster.fill(Rgb::new(0, 100, 200));
        raster.blend(0, 0, Rgb::new(255, 100, 0), 0.5);
        assert_eq!(raster.get(0, 0), Rgb::new(128, 100, 100));
        raster.blend(0, 0, Rgb::WHITE, 0.0);
        assert_eq!(raster.get(0, 0), Rgb::new(128, 100, 100));
    }

    #[test]
    fn a_triangle_fills_either_winding_and_an_ellipse_its_centre() {
        let mut a = Raster::new(10, 10);
        let mut b = Raster::new(10, 10);
        a.triangle((1.0, 1.0), (9.0, 1.0), (1.0, 9.0), Rgb::WHITE);
        b.triangle((1.0, 1.0), (1.0, 9.0), (9.0, 1.0), Rgb::WHITE);
        assert_eq!(a, b);
        assert!(a.pixels().iter().filter(|&&p| p == Rgb::WHITE).count() > 20);
        let mut c = Raster::new(9, 9);
        c.disc(4.5, 4.5, 3.0, Rgb::WHITE);
        assert_eq!(c.get(4, 4), Rgb::WHITE);
        assert_eq!(c.get(0, 0), Rgb::BLACK);
    }

    #[test]
    fn hsv_lands_on_the_primaries() {
        assert_eq!(Rgb::hsv(0.0, 1.0, 1.0), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::hsv(120.0, 1.0, 1.0), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::hsv(240.0, 1.0, 1.0), Rgb::new(0, 0, 255));
        assert_eq!(Rgb::hsv(-120.0, 1.0, 1.0), Rgb::new(0, 0, 255));
        assert_eq!(Rgb::WHITE.luma(), 255);
    }
}
