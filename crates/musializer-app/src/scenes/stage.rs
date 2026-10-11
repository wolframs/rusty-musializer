//! Stage: the drawing half.
//!
//! Layout arithmetic — wrapping, fitting, typed counts, the ticker's walk — is
//! in `musializer_core::scenes::stage`. This file turns one lyric line into one
//! of six staged pictures and nothing else; with no line on screen it draws its
//! ground and, if asked, the cat.
//!
//! Two rules hold for every layout, and both are about being a prop rather
//! than an effect:
//!
//! - **Nothing here reads the raw audio level.** A staged line that trembles
//!   with the mix reads as a rendering fault. The beat may nudge the sung word
//!   and the cat's tail; that is all the sound gets.
//! - **A line always ends complete.** Typed text, raised placards and the
//!   stamp all finish inside the line's reveal and then hold, so a still taken
//!   anywhere in the back half of a cue shows the whole line.

use musializer_core::project::model::CaptionFace;
use musializer_core::scene::settings::index::stage as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::cadence;
use musializer_core::scenes::stage::{self, Layout, StageState};
use musializer_runtime::draw;
use musializer_runtime::font::{FaceRef, Faces};
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibMode2DExt, Rectangle,
    Vector2,
};

use super::ascii_field::DefaultFont;

const PI: f32 = std::f32::consts::PI;

/// How much faster than Cadence's reading pace a staged line arrives.
const LANDING: f32 = 2.0;

/// What the last frame staged, for the `stage:` report line. A staged line and
/// an empty ground are different frames, and a layout that fell back to the
/// shared 64 px atlas is a soft picture no capture can tell from a soft export.
#[derive(Clone, Debug, Default)]
pub struct StageReport {
    pub layout: &'static str,
    pub words: usize,
    pub rows: usize,
    pub type_px: f32,
    pub sized_atlas: bool,
    pub familiar: bool,
}

impl StageReport {
    #[must_use]
    pub fn describe(&self) -> String {
        if self.layout.is_empty() {
            return "none".to_string();
        }
        if self.words == 0 {
            return format!(
                "layout={} line=none familiar={}",
                self.layout,
                if self.familiar { "drawn" } else { "off" }
            );
        }
        format!(
            "layout={} words={} rows={} type={:.0}px atlas={} familiar={}",
            self.layout,
            self.words,
            self.rows,
            self.type_px,
            if self.sized_atlas { "sized" } else { "shared" },
            if self.familiar { "drawn" } else { "off" }
        )
    }
}

/// The colours one frame is drawn in. Resolved once, so six layouts cannot
/// drift into six palettes.
#[derive(Clone, Copy)]
struct Palette {
    ground_top: Color,
    ground_bottom: Color,
    /// Body text and line work.
    ink: Color,
    /// The sung word, and anything a layout wants looked at.
    accent: Color,
    /// The counter-colour: a falling candle, the stamp.
    alarm: Color,
    /// A raised surface on the ground: a window, a bubble.
    panel: Color,
    /// Whether the ground is the light one, which flips how glow composites.
    daylight: bool,
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::new(
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b),
        channel(a.a, b.a),
    )
}

fn palette(hue_offset: f32, saturation: f32, light: f32) -> Palette {
    // Violet at rest, the near-complement for what should be looked at.
    let hue = (268.0 + hue_offset + 720.0) % 360.0;
    let chroma = |base: f32| (base * saturation).clamp(0.0, 1.0);
    let accent_hue = (hue + 160.0) % 360.0;
    let alarm_hue = (hue + 88.0) % 360.0;
    let night = (
        draw::color_from_hsv(hue, chroma(0.62), 0.055),
        draw::color_from_hsv((hue + 22.0) % 360.0, chroma(0.58), 0.115),
    );
    let paper = (
        draw::color_from_hsv((hue + 150.0) % 360.0, chroma(0.07), 0.955),
        draw::color_from_hsv((hue + 150.0) % 360.0, chroma(0.11), 0.89),
    );
    let daylight = light > 0.5;
    Palette {
        ground_top: mix(night.0, paper.0, light),
        ground_bottom: mix(night.1, paper.1, light),
        ink: if daylight {
            draw::color_from_hsv(hue, chroma(0.45), 0.13)
        } else {
            draw::color_from_hsv((hue + 150.0) % 360.0, chroma(0.07), 0.97)
        },
        accent: if daylight {
            draw::color_from_hsv(accent_hue, chroma(0.88), 0.62)
        } else {
            draw::color_from_hsv(accent_hue, chroma(0.72), 1.0)
        },
        alarm: if daylight {
            draw::color_from_hsv(alarm_hue, chroma(0.85), 0.78)
        } else {
            draw::color_from_hsv(alarm_hue, chroma(0.78), 1.0)
        },
        panel: if daylight {
            draw::color_from_hsv(hue, chroma(0.10), 1.0)
        } else {
            draw::color_from_hsv(hue, chroma(0.42), 0.27)
        },
        daylight,
    }
}

/// Everything a layout needs to know about the moment.
struct Moment<'a> {
    text: &'a str,
    line_id: u64,
    /// Seconds into the cue, and its length.
    elapsed: f32,
    duration: f32,
    /// 0..1 through the cue.
    position: f32,
    /// The line's own fade at its two ends.
    presence: f32,
    /// 0 at the start of the reveal, 1 when the whole line has arrived.
    reveal: f32,
    /// 1 on the beat, decaying before the next.
    pulse: f32,
    time: f32,
    seed: u64,
}

/// Rotates and scales everything drawn inside `body` about `pivot`.
///
/// One level only: `BeginMode2D` loads its own matrix rather than multiplying
/// into the current one, so a transform inside a transform silently replaces
/// the outer. Layouts that need a part turned against the whole compute that
/// part's pivot with [`turn`] and open a second, separate transform for it.
fn transformed(
    d: &mut RaylibDrawHandle<'_>,
    pivot: Vector2,
    degrees: f32,
    zoom: f32,
    body: impl FnOnce(&mut dyn Canvas),
) {
    // The ffi struct: raylib-rs's own `Camera2D` is compiled out of the
    // `nobuild` configuration this workspace links raylib through.
    let at = raylib_sys::Vector2 {
        x: pivot.x,
        y: pivot.y,
    };
    let camera = raylib_sys::Camera2D {
        offset: at,
        target: at,
        rotation: degrees,
        zoom: zoom.max(0.001),
    };
    let mut mode = d.begin_mode2D(camera);
    body(&mut mode);
}

/// A point turned about a pivot, in raylib's screen sense (clockwise).
fn turn(point: Vector2, pivot: Vector2, degrees: f32) -> Vector2 {
    let radians = degrees * PI / 180.0;
    let (sin, cos) = radians.sin_cos();
    let (x, y) = (point.x - pivot.x, point.y - pivot.y);
    Vector2::new(pivot.x + x * cos - y * sin, pivot.y + x * sin + y * cos)
}

/// The handful of draw calls the layouts use, as an object-safe trait.
///
/// `RaylibDraw` is not object safe (its methods are generic), and the layouts
/// draw through two handle types: the frame's own, and a 2D-mode guard over
/// it. Threading a generic through every helper would monomorphise this whole
/// file twice for no gain; nine methods behind a vtable is the cheaper shape.
trait Canvas {
    fn rect(&mut self, rect: Rectangle, color: Color);
    fn rounded(&mut self, rect: Rectangle, roundness: f32, color: Color);
    fn rounded_outline(&mut self, rect: Rectangle, roundness: f32, thick: f32, color: Color);
    fn outline(&mut self, rect: Rectangle, thick: f32, color: Color);
    fn line(&mut self, from: Vector2, to: Vector2, thick: f32, color: Color);
    fn circle(&mut self, centre: Vector2, radius: f32, color: Color);
    fn ellipse(&mut self, centre: Vector2, rx: f32, ry: f32, color: Color);
    fn triangle(&mut self, a: Vector2, b: Vector2, c: Vector2, color: Color);
    fn type_face(&mut self, face: &FaceRef<'_>, text: &str, at: Vector2, size: f32, color: Color);
    fn type_pixel(&mut self, text: &str, at: Vector2, size: f32, color: Color);
}

impl<D: RaylibDraw> Canvas for D {
    fn rect(&mut self, rect: Rectangle, color: Color) {
        self.draw_rectangle_rec(rect, color);
    }

    fn rounded(&mut self, rect: Rectangle, roundness: f32, color: Color) {
        self.draw_rectangle_rounded(rect, roundness, 12, color);
    }

    fn rounded_outline(&mut self, rect: Rectangle, roundness: f32, thick: f32, color: Color) {
        self.draw_rectangle_rounded_lines_ex(rect, roundness, 12, thick, color);
    }

    fn outline(&mut self, rect: Rectangle, thick: f32, color: Color) {
        self.draw_rectangle_lines_ex(rect, thick, color);
    }

    fn line(&mut self, from: Vector2, to: Vector2, thick: f32, color: Color) {
        self.draw_line_ex(from, to, thick, color);
    }

    fn circle(&mut self, centre: Vector2, radius: f32, color: Color) {
        self.draw_circle_v(centre, radius, color);
    }

    fn ellipse(&mut self, centre: Vector2, rx: f32, ry: f32, color: Color) {
        self.draw_ellipse(centre.x as i32, centre.y as i32, rx, ry, color);
    }

    fn triangle(&mut self, a: Vector2, b: Vector2, c: Vector2, color: Color) {
        // Both windings: rlgl culls a clockwise triangle without a word, and
        // which way these three points run depends on the caller's geometry.
        self.draw_triangle(a, b, c, color);
        self.draw_triangle(a, c, b, color);
    }

    fn type_face(&mut self, face: &FaceRef<'_>, text: &str, at: Vector2, size: f32, color: Color) {
        // A lyric with an interior NUL cannot cross the C string boundary; it
        // draws nothing rather than aborting the frame.
        if text.contains('\0') || text.is_empty() {
            return;
        }
        self.draw_text_ex(face.face(), text, at, size, 0.0, color);
    }

    fn type_pixel(&mut self, text: &str, at: Vector2, size: f32, color: Color) {
        if text.contains('\0') || text.is_empty() {
            return;
        }
        let font = DefaultFont::get();
        self.draw_text_ex(&font, text, at, size, size * 0.1, color);
    }
}

fn pixel_width(text: &str, size: f32) -> f32 {
    if text.contains('\0') || text.is_empty() {
        return 0.0;
    }
    use raylib::prelude::RaylibFont;
    DefaultFont::get().measure_text(text, size, size * 0.1).x
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    draw::color_alpha(color, alpha.clamp(0.0, 1.0))
}

fn splitmix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit(seed: u64, salt: u64) -> f32 {
    (splitmix(seed ^ salt.wrapping_mul(0x2545_f491_4f6c_dd1d)) & 0xffff) as f32 / 65535.0
}

/// The ground every layout stands on: a gradient, a faint grid on the night
/// ground, and grain. Grain is hashed from a 12 Hz clock rather than the frame
/// counter, so two renders of one moment lay down the same specks at any frame
/// rate (the argument Clawd's air pass makes).
fn draw_ground(
    d: &mut RaylibDrawHandle<'_>,
    boundary: Rectangle,
    colors: &Palette,
    grain: f32,
    time: f32,
    seed: u64,
    pixel_scale: f32,
) {
    d.draw_rectangle_gradient_v(
        boundary.x as i32,
        boundary.y as i32,
        boundary.width.ceil() as i32,
        boundary.height.ceil() as i32,
        colors.ground_top,
        colors.ground_bottom,
    );
    let short = boundary.width.min(boundary.height);
    if !colors.daylight {
        let step = short / 14.0;
        let rule = with_alpha(colors.ink, 0.035);
        let mut x = boundary.x + (boundary.width % step) * 0.5;
        while x < boundary.x + boundary.width {
            d.draw_line_ex(
                Vector2::new(x, boundary.y),
                Vector2::new(x, boundary.y + boundary.height),
                pixel_scale,
                rule,
            );
            x += step;
        }
        let mut y = boundary.y + (boundary.height % step) * 0.5;
        while y < boundary.y + boundary.height {
            d.draw_line_ex(
                Vector2::new(boundary.x, y),
                Vector2::new(boundary.x + boundary.width, y),
                pixel_scale,
                rule,
            );
            y += step;
        }
    }
    if grain > 0.005 {
        let tick = (f64::from(time) * 12.0).floor() as u64;
        let specks = (900.0 * grain) as u64;
        let speck = with_alpha(
            if colors.daylight {
                Color::BLACK
            } else {
                Color::WHITE
            },
            if colors.daylight { 0.07 } else { 0.09 },
        );
        let size = (short / 420.0).max(1.0);
        for index in 0..specks {
            let salt = seed ^ tick.wrapping_mul(0x5851_f42d_4c95_7f2d) ^ index;
            d.draw_rectangle_rec(
                Rectangle::new(
                    boundary.x + unit(salt, 1) * boundary.width,
                    boundary.y + unit(salt, 2) * boundary.height,
                    size,
                    size,
                ),
                speck,
            );
        }
    }
}

/// The familiar: a black cat seated at `foot`, `size` tall.
///
/// Procedural, like everything here — no artwork. On the night ground a black
/// cat would be invisible, so it carries a rim of the accent colour; on paper
/// it is simply black. The tail keeps the beat and the eyes blink on their own
/// clock, which is what makes it a character rather than a logo.
fn draw_familiar(
    d: &mut dyn Canvas,
    foot: Vector2,
    size: f32,
    colors: &Palette,
    pulse: f32,
    time: f32,
) {
    let fur = if colors.daylight {
        Color::new(18, 14, 24, 255)
    } else {
        Color::new(9, 6, 14, 255)
    };
    let rim = with_alpha(colors.accent, if colors.daylight { 0.0 } else { 0.55 });
    let body = Vector2::new(foot.x, foot.y - size * 0.27);
    let head = Vector2::new(foot.x - size * 0.02, foot.y - size * 0.66);
    let head_radius = size * 0.2;

    let shape = |d: &mut dyn Canvas, grow: f32, color: Color| {
        // Tail first, so the body sits over its root.
        let sway = (time * 1.6).sin() * 0.5 + pulse * 0.35;
        let root = Vector2::new(foot.x + size * 0.2, foot.y - size * 0.08);
        for step in 0..30 {
            let t = step as f32 / 29.0;
            let curl = t * t;
            let point = Vector2::new(
                root.x + size * (0.26 * t + 0.10 * curl * sway),
                root.y - size * (0.42 * curl) + size * 0.02 * (t * PI).sin(),
            );
            d.circle(point, size * (0.050 - 0.022 * t) + grow, color);
        }
        d.ellipse(body, size * 0.25 + grow, size * 0.29 + grow, color);
        d.circle(head, head_radius + grow, color);
        for side in [-1.0f32, 1.0] {
            let base = Vector2::new(
                head.x + side * head_radius * 0.52,
                head.y - head_radius * 0.62,
            );
            d.triangle(
                Vector2::new(
                    base.x - head_radius * 0.42 - grow,
                    base.y + head_radius * 0.25,
                ),
                Vector2::new(
                    base.x + head_radius * 0.42 + grow,
                    base.y + head_radius * 0.25,
                ),
                Vector2::new(
                    base.x + side * head_radius * 0.16,
                    base.y - head_radius * 0.72 - grow,
                ),
                color,
            );
        }
    };
    if rim.a > 0 {
        shape(d, size * 0.016, rim);
    }
    shape(d, 0.0, fur);

    // A blink every few seconds, 120 ms long.
    let blink = if (time * 0.27).fract() < 0.03 {
        0.12
    } else {
        1.0
    };
    for side in [-1.0f32, 1.0] {
        let eye = Vector2::new(
            head.x + side * head_radius * 0.42,
            head.y - head_radius * 0.02,
        );
        d.ellipse(
            eye,
            head_radius * 0.19,
            head_radius * 0.24 * blink,
            colors.accent,
        );
        d.ellipse(eye, head_radius * 0.05, head_radius * 0.20 * blink, fur);
    }
}

/// A face for `text` at `px`, through the caption's at-size atlas cache.
///
/// **One at a time.** A `FaceRef` borrows the cache; hold one while asking for
/// another and the second request panics. Every layout here measures with a
/// probe that is dropped before it asks for the face it draws with.
fn face<'f>(fonts: &'f Faces, which: CaptionFace, px: f32, text: &str) -> FaceRef<'f> {
    fonts.caption_at(which, px, text)
}

/// Words of a line with their reveal state at this moment.
struct Spoken<'a> {
    text: &'a str,
    /// 0 before the word arrives, 1 once it has.
    shown: f32,
    /// 1 while it is the word being sung.
    sung: f32,
}

fn spoken<'a>(moment: &Moment<'a>, pace: f32) -> Vec<Spoken<'a>> {
    let (words, _) = cadence::words_for_timed_cue(moment.text, f64::from(moment.duration));
    words
        .iter()
        .map(|word| {
            // A staged line is read, not followed: it lands in about half the
            // time Cadence would take to sing it through, and then holds.
            let start = word.window_start / (pace * LANDING);
            let end = word.window_end / (pace * LANDING);
            let span = (end - start).max(0.0001);
            Spoken {
                text: word.text,
                shown: stage::smooth((moment.position - start) / (span * 0.35).max(0.004)),
                sung: stage::smooth((moment.position - start) / (span * 0.2).max(0.004))
                    * (1.0 - stage::smooth((moment.position - end) / (span * 0.3).max(0.004))),
            }
        })
        .collect()
}

/// Shared by the layouts that typeset wrapped words in one face.
struct Block {
    px: f32,
    widths: Vec<f32>,
    space: f32,
    rows: Vec<(usize, usize)>,
    line_height: f32,
}

impl Block {
    #[allow(clippy::too_many_arguments)]
    fn fit(
        fonts: &Faces,
        which: CaptionFace,
        text: &str,
        words: &[Spoken<'_>],
        max_width: f32,
        max_height: f32,
        ceiling_px: f32,
        leading: f32,
        unit_space: f32,
    ) -> (Self, bool) {
        // Measured once at a size near the answer and scaled: metrics are
        // linear in size to well under a pixel, and asking the atlas cache for
        // a third size per frame would evict the one just built.
        let probe = face(fonts, which, ceiling_px, text);
        let unit_widths: Vec<f32> = words
            .iter()
            .map(|word| probe.measure_text(word.text, 100.0, 0.0).x / 100.0)
            .collect();
        let px = stage::fit_scale(
            &unit_widths,
            unit_space,
            leading,
            max_width,
            max_height,
            ceiling_px,
        );
        let widths: Vec<f32> = unit_widths.iter().map(|width| width * px).collect();
        let space = unit_space * px;
        let rows = stage::wrap_rows(&widths, space, max_width);
        (
            Self {
                px,
                widths,
                space,
                rows,
                line_height: leading * px,
            },
            probe.is_sized(),
        )
    }

    fn height(&self) -> f32 {
        self.rows.len() as f32 * self.line_height
    }

    fn width(&self) -> f32 {
        self.rows
            .iter()
            .map(|&row| stage::row_width(&self.widths, self.space, row))
            .fold(0.0, f32::max)
    }

    /// Top-left of each word, for a block whose own top-left is `origin`.
    fn positions(&self, origin: Vector2, centred_in: Option<f32>) -> Vec<Vector2> {
        let mut out = vec![Vector2::zero(); self.widths.len()];
        for (row_index, &row) in self.rows.iter().enumerate() {
            let width = stage::row_width(&self.widths, self.space, row);
            let mut x = origin.x + centred_in.map_or(0.0, |inside| (inside - width) * 0.5);
            for (slot, word_width) in out[row.0..row.1].iter_mut().zip(&self.widths[row.0..row.1]) {
                *slot = Vector2::new(x, origin.y + row_index as f32 * self.line_height);
                x += word_width + self.space;
            }
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn poster(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    bounce: f32,
    accent: f32,
    tilt: f32,
    pace: f32,
    report: &mut StageReport,
) {
    let words = spoken(moment, pace);
    let margin = boundary.width * 0.085;
    let (block, sized) = Block::fit(
        fonts,
        CaptionFace::SpaceGrotesk,
        moment.text,
        &words,
        boundary.width - margin * 2.0 - boundary.width * 0.03,
        boundary.height * 0.74,
        boundary.height * 0.36 * scale,
        0.98,
        0.27,
    );
    if block.px < 4.0 {
        return;
    }
    let font = face(fonts, CaptionFace::SpaceGrotesk, block.px, moment.text);
    let origin = Vector2::new(
        boundary.x + margin + boundary.width * 0.03,
        boundary.y + (boundary.height - block.height()) * 0.5,
    );
    let positions = block.positions(origin, None);
    let pivot = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    report.rows = block.rows.len();
    report.type_px = block.px;
    report.sized_atlas = sized && font.is_sized();
    transformed(d, pivot, tilt, 1.0, |d| {
        // The bar is the line's clock: it fills as the line is sung.
        let bar = Rectangle::new(
            boundary.x + margin,
            origin.y + block.px * 0.12,
            block.px * 0.07,
            (block.height() - block.px * 0.2) * moment.reveal.max(0.02),
        );
        d.rect(bar, with_alpha(colors.accent, moment.presence));
        for (index, word) in words.iter().enumerate() {
            if word.shown <= 0.001 {
                continue;
            }
            let lift = (1.0 - word.shown) * block.px * 0.16;
            let kick = 1.0 + 0.03 * bounce * moment.pulse * word.sung;
            let size = block.px * kick;
            let at = Vector2::new(
                positions[index].x - (size - block.px) * 0.2,
                positions[index].y + lift - (size - block.px) * 0.5,
            );
            let color = mix(colors.ink, colors.accent, word.sung * accent);
            d.type_face(
                &font,
                word.text,
                at,
                size,
                with_alpha(color, word.shown * moment.presence),
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn terminal(
    d: &mut RaylibDrawHandle<'_>,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    tilt: f32,
    pace: f32,
    pixel_scale: f32,
    report: &mut StageReport,
) {
    let window = Rectangle::new(
        boundary.x + boundary.width * 0.11,
        boundary.y + boundary.height * 0.17,
        boundary.width * 0.78,
        boundary.height * 0.66,
    );
    let bar = window.height * 0.11;
    let prompt = "> ";
    let words: Vec<&str> = moment.text.split_whitespace().collect();
    // Fitted like every other layout: a console with one small line in a
    // corner of it is a screenshot, not a staging. The pixel face is linear in
    // size, so it is measured at 100 and scaled.
    let unit: Vec<f32> = words
        .iter()
        .map(|word| pixel_width(word, 100.0) / 100.0)
        .collect();
    let unit_space = pixel_width("n", 100.0) / 100.0 * 1.2;
    let unit_prompt = pixel_width(prompt, 100.0) / 100.0;
    // Bisected here rather than through `fit_scale`: the margins and the
    // prompt are themselves in type sizes, so the measure shrinks as the type
    // grows.
    let fits = |px: f32| {
        let measure = window.width - px * (2.4 + unit_prompt);
        let widths: Vec<f32> = unit.iter().map(|width| width * px).collect();
        let rows = stage::wrap_rows(&widths, unit_space * px, measure);
        measure > 0.0
            && rows.len() as f32 * 1.45 * px <= (window.height - bar) * 0.82
            && rows
                .iter()
                .all(|&row| stage::row_width(&widths, unit_space * px, row) <= measure)
    };
    let (mut low, mut high) = (0.0f32, boundary.height * 0.15 * scale);
    if fits(high) {
        low = high;
    } else {
        for _ in 0..20 {
            let middle = (low + high) * 0.5;
            if fits(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
    }
    let px = low.max(6.0);
    let inner = window.width - px * 2.4;
    let widths: Vec<f32> = unit.iter().map(|width| width * px).collect();
    let space = unit_space * px;
    let rows = stage::wrap_rows(&widths, space, inner - unit_prompt * px);
    let total: usize = words.iter().map(|word| word.chars().count()).sum();
    let typed = stage::typed_count(total, (moment.reveal * pace).min(1.0));
    let pivot = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    let glass = if colors.daylight {
        Color::new(22, 20, 30, 255)
    } else {
        mix(colors.panel, Color::BLACK, 0.45)
    };
    let phosphor = if colors.daylight {
        mix(colors.accent, Color::WHITE, 0.55)
    } else {
        colors.accent
    };
    report.rows = rows.len();
    report.type_px = px;
    report.sized_atlas = true;
    transformed(d, pivot, tilt, 1.0, |d| {
        d.rounded(
            Rectangle::new(
                window.x + px * 0.25,
                window.y + px * 0.35,
                window.width,
                window.height,
            ),
            0.05,
            with_alpha(Color::BLACK, 0.35 * moment.presence),
        );
        d.rounded(window, 0.05, with_alpha(glass, moment.presence));
        d.rounded_outline(
            window,
            0.05,
            pixel_scale * 1.5,
            with_alpha(phosphor, 0.35 * moment.presence),
        );
        d.line(
            Vector2::new(window.x, window.y + bar),
            Vector2::new(window.x + window.width, window.y + bar),
            pixel_scale,
            with_alpha(phosphor, 0.25 * moment.presence),
        );
        for dot in 0..3 {
            d.circle(
                Vector2::new(
                    window.x + bar * (0.55 + dot as f32 * 0.42),
                    window.y + bar * 0.5,
                ),
                bar * 0.13,
                with_alpha(
                    [colors.alarm, colors.accent, colors.ink][dot],
                    0.75 * moment.presence,
                ),
            );
        }
        let title = format!("line {}", moment.line_id);
        d.type_pixel(
            &title,
            Vector2::new(window.x + bar * 1.9, window.y + bar * 0.5 - px * 0.2),
            px * 0.4,
            with_alpha(phosphor, 0.6 * moment.presence),
        );

        let mut remaining = typed;
        let start_x = window.x + px * 1.2;
        let mut pen = Vector2::new(start_x, window.y + bar + px * 0.9);
        // Where the cursor block sits: just after the last character down.
        let mut cursor = Vector2::new(start_x + pixel_width(prompt, px), pen.y);
        'rows: for (row_index, row) in rows.iter().enumerate() {
            pen.x = start_x;
            let lead = if row_index == 0 { prompt } else { "  " };
            d.type_pixel(lead, pen, px, with_alpha(phosphor, 0.6 * moment.presence));
            pen.x += pixel_width(prompt, px);
            cursor = pen;
            for index in row.0..row.1 {
                if remaining == 0 {
                    break 'rows;
                }
                let word = words[index];
                let count = word.chars().count();
                let take = remaining.min(count);
                let piece: String = word.chars().take(take).collect();
                d.type_pixel(&piece, pen, px, with_alpha(phosphor, moment.presence));
                cursor = Vector2::new(pen.x + pixel_width(&piece, px), pen.y);
                remaining -= take;
                pen.x += widths[index] + space;
            }
            pen.y += px * 1.45;
        }
        // The cursor: solid while typing, blinking once the line is down.
        let lit = typed < total || (moment.time * 2.2).fract() < 0.55;
        if lit {
            d.rect(
                Rectangle::new(cursor.x + px * 0.1, cursor.y, px * 0.5, px),
                with_alpha(phosphor, 0.9 * moment.presence),
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn chat(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    bounce: f32,
    accent: f32,
    tilt: f32,
    pace: f32,
    report: &mut StageReport,
) {
    let words = spoken(moment, pace);
    let (block, sized) = Block::fit(
        fonts,
        CaptionFace::SpaceGrotesk,
        moment.text,
        &words,
        boundary.width * 0.6,
        boundary.height * 0.56,
        boundary.height * 0.14 * scale,
        1.22,
        0.27,
    );
    if block.px < 4.0 {
        return;
    }
    let font = face(fonts, CaptionFace::SpaceGrotesk, block.px, moment.text);
    let pad = block.px * 0.62;
    let bubble_size = Vector2::new(block.width() + pad * 2.0, block.height() + pad * 1.7);
    // Even line ids speak from the right, so a verse reads as an exchange.
    let from_right = moment.line_id.is_multiple_of(2);
    let avatar_radius = boundary.height * 0.09;
    let edge = boundary.width * 0.09;
    let avatar = Vector2::new(
        if from_right {
            boundary.x + boundary.width - edge - avatar_radius
        } else {
            boundary.x + edge + avatar_radius
        },
        boundary.y + boundary.height * 0.5 + bubble_size.y * 0.5 - avatar_radius * 0.4,
    );
    let gap = avatar_radius * 1.5;
    let bubble = Rectangle::new(
        if from_right {
            avatar.x - gap - bubble_size.x
        } else {
            avatar.x + gap
        },
        boundary.y + (boundary.height - bubble_size.y) * 0.5,
        bubble_size.x,
        bubble_size.y,
    );
    // Dots first: the first 0.45 s, or a fifth of a short line.
    let dots_for = (moment.duration * 0.2).min(0.45);
    let typing = moment.elapsed < dots_for;
    let grown = stage::smooth((moment.elapsed - dots_for) / 0.16);
    let fill = if from_right {
        colors.accent
    } else {
        colors.panel
    };
    let text_ink = if from_right {
        if colors.daylight {
            Color::WHITE
        } else {
            Color::new(14, 10, 20, 255)
        }
    } else {
        colors.ink
    };
    let pivot = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    report.rows = block.rows.len();
    report.type_px = block.px;
    report.sized_atlas = sized && font.is_sized();
    transformed(d, pivot, tilt, 1.0, |d| {
        // The speaker: a cat's head in a disc. It is the same cat as the
        // familiar, so a chat line never needs the toggle to have a face.
        d.circle(
            avatar,
            avatar_radius,
            with_alpha(colors.panel, moment.presence),
        );
        let cat = Palette {
            ink: colors.ink,
            ..*colors
        };
        let head = Vector2::new(avatar.x, avatar.y + avatar_radius * 1.55);
        draw_cat_head(
            d,
            head,
            avatar_radius * 3.0,
            &cat,
            moment.time,
            moment.presence,
        );

        if typing {
            let small = Rectangle::new(
                if from_right {
                    avatar.x - gap - block.px * 2.6
                } else {
                    avatar.x + gap
                },
                avatar.y - block.px * 0.9,
                block.px * 2.6,
                block.px * 1.3,
            );
            d.rounded(small, 0.9, with_alpha(fill, moment.presence));
            for dot in 0..3 {
                let hop = ((moment.time * 7.0 - dot as f32 * 0.9).sin()).max(0.0);
                d.circle(
                    Vector2::new(
                        small.x + small.width * (0.28 + dot as f32 * 0.22),
                        small.y + small.height * 0.5 - hop * block.px * 0.16,
                    ),
                    block.px * 0.11,
                    with_alpha(text_ink, 0.75 * moment.presence),
                );
            }
            return;
        }
        // The bubble opens from the speaker's corner.
        let open = Rectangle::new(
            if from_right {
                bubble.x + bubble.width * (1.0 - grown)
            } else {
                bubble.x
            },
            bubble.y + bubble.height * (1.0 - grown),
            bubble.width * grown,
            bubble.height * grown,
        );
        if open.width > 2.0 && open.height > 2.0 {
            d.rounded(open, 0.34, with_alpha(fill, moment.presence));
        }
        if grown < 0.98 {
            return;
        }
        let positions = block.positions(Vector2::new(bubble.x + pad, bubble.y + pad * 0.8), None);
        for (index, word) in words.iter().enumerate() {
            if word.shown <= 0.001 {
                continue;
            }
            let color = if from_right {
                text_ink
            } else {
                mix(text_ink, colors.accent, word.sung * accent)
            };
            let kick = block.px * 0.03 * bounce * moment.pulse * word.sung;
            d.type_face(
                &font,
                word.text,
                Vector2::new(positions[index].x, positions[index].y - kick),
                block.px,
                with_alpha(color, word.shown * moment.presence),
            );
        }
    });
}

/// Just the head of the familiar, for the chat avatar: `size` is the height a
/// whole cat would have, so the two share proportions.
fn draw_cat_head(
    d: &mut dyn Canvas,
    foot: Vector2,
    size: f32,
    colors: &Palette,
    time: f32,
    presence: f32,
) {
    let fur = with_alpha(Color::new(12, 8, 18, 255), presence);
    let head = Vector2::new(foot.x, foot.y - size * 0.5);
    let radius = size * 0.2;
    d.circle(head, radius, fur);
    for side in [-1.0f32, 1.0] {
        let base = Vector2::new(head.x + side * radius * 0.52, head.y - radius * 0.62);
        d.triangle(
            Vector2::new(base.x - radius * 0.42, base.y + radius * 0.25),
            Vector2::new(base.x + radius * 0.42, base.y + radius * 0.25),
            Vector2::new(base.x + side * radius * 0.16, base.y - radius * 0.72),
            fur,
        );
    }
    let blink = if (time * 0.27).fract() < 0.03 {
        0.12
    } else {
        1.0
    };
    for side in [-1.0f32, 1.0] {
        let eye = Vector2::new(head.x + side * radius * 0.42, head.y - radius * 0.02);
        d.ellipse(
            eye,
            radius * 0.19,
            radius * 0.24 * blink,
            with_alpha(colors.accent, presence),
        );
        d.ellipse(eye, radius * 0.05, radius * 0.20 * blink, fur);
    }
}

#[allow(clippy::too_many_arguments)]
fn card(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    tilt: f32,
    pace: f32,
    pixel_scale: f32,
    report: &mut StageReport,
) {
    let words = spoken(moment, pace);
    let paper = Rectangle::new(
        boundary.x + boundary.width * 0.18,
        boundary.y + boundary.height * 0.16,
        boundary.width * 0.64,
        boundary.height * 0.68,
    );
    let pad = paper.height * 0.13;
    let stamp_word = words
        .last()
        .map(|word| {
            word.text
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_uppercase()
        })
        .unwrap_or_default();
    let covered = format!("{} {stamp_word}", moment.text);
    let (block, sized) = Block::fit(
        fonts,
        CaptionFace::Alegreya,
        &covered,
        &words,
        paper.width - pad * 2.0,
        paper.height * 0.6,
        boundary.height * 0.16 * scale,
        1.32,
        0.27,
    );
    if block.px < 4.0 {
        return;
    }
    let font = face(fonts, CaptionFace::Alegreya, block.px, &covered);
    // The card always leans a little; a square card on a square frame is a
    // dialog box.
    let lean = tilt - 2.0;
    let pivot = Vector2::new(paper.x + paper.width * 0.5, paper.y + paper.height * 0.5);
    let sheet = Color::new(247, 243, 232, 255);
    let type_ink = Color::new(34, 30, 38, 255);
    let total: usize = words.iter().map(|word| word.text.chars().count()).sum();
    let typed = stage::typed_count(total, (moment.reveal * pace * 1.25).min(1.0));
    let positions = block.positions(Vector2::new(paper.x + pad, paper.y + pad * 1.05), None);
    report.rows = block.rows.len();
    report.type_px = block.px;
    report.sized_atlas = sized && font.is_sized();
    transformed(d, pivot, lean, 1.0, |d| {
        d.rect(
            Rectangle::new(
                paper.x + pad * 0.14,
                paper.y + pad * 0.2,
                paper.width,
                paper.height,
            ),
            with_alpha(Color::BLACK, 0.32 * moment.presence),
        );
        d.rect(paper, with_alpha(sheet, moment.presence));
        // Ruled lines under where the type sits, and a perforated foot.
        for row in 0..block.rows.len().max(3) {
            let y = paper.y + pad * 1.05 + (row as f32 + 0.86) * block.line_height;
            if y < paper.y + paper.height - pad {
                d.line(
                    Vector2::new(paper.x + pad, y),
                    Vector2::new(paper.x + paper.width - pad, y),
                    pixel_scale,
                    with_alpha(type_ink, 0.10 * moment.presence),
                );
            }
        }
        let foot = paper.y + paper.height - pad * 0.75;
        let dash = paper.width / 46.0;
        let mut x = paper.x + pad * 0.5;
        while x < paper.x + paper.width - pad * 0.5 {
            d.line(
                Vector2::new(x, foot),
                Vector2::new(x + dash * 0.55, foot),
                pixel_scale * 1.2,
                with_alpha(type_ink, 0.28 * moment.presence),
            );
            x += dash;
        }
        let mut remaining = typed;
        for (index, word) in words.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            let count = word.text.chars().count();
            let take = remaining.min(count);
            let piece: String = word.text.chars().take(take).collect();
            d.type_face(
                &font,
                &piece,
                positions[index],
                block.px,
                with_alpha(type_ink, moment.presence),
            );
            remaining -= take;
        }
    });

    // The stamp: the line's last word, slammed on once the typing is done.
    let slam = stage::smooth((moment.reveal * pace * 1.25 - 1.0) / 0.12);
    if slam <= 0.001 || stamp_word.is_empty() {
        return;
    }
    let stamp_px = (block.px * 1.15).min(paper.height * 0.16);
    let stamp_width = font.measure_text(&stamp_word, stamp_px, 0.0).x;
    let frame = Rectangle::new(
        paper.x + paper.width - pad * 0.9 - stamp_width - stamp_px * 0.7,
        paper.y + paper.height - pad * 1.25 - stamp_px * 1.35,
        stamp_width + stamp_px * 0.7,
        stamp_px * 1.35,
    );
    let centre = turn(
        Vector2::new(frame.x + frame.width * 0.5, frame.y + frame.height * 0.5),
        pivot,
        lean,
    );
    let placed = Rectangle::new(
        centre.x - frame.width * 0.5,
        centre.y - frame.height * 0.5,
        frame.width,
        frame.height,
    );
    // The alarm colour in its daylight strength, whatever the ground: the
    // stamp is on paper.
    let ink = with_alpha(
        mix(colors.alarm, Color::new(150, 20, 30, 255), 0.35),
        0.88 * slam * moment.presence,
    );
    // Arrives large and settles: a stamp that fades in was printed, not struck.
    transformed(d, centre, lean - 7.0, 1.0 + (1.0 - slam) * 0.7, |d| {
        d.outline(placed, stamp_px * 0.085, ink);
        d.outline(
            Rectangle::new(
                placed.x + stamp_px * 0.14,
                placed.y + stamp_px * 0.14,
                placed.width - stamp_px * 0.28,
                placed.height - stamp_px * 0.28,
            ),
            stamp_px * 0.03,
            ink,
        );
        d.type_face(
            &font,
            &stamp_word,
            Vector2::new(placed.x + stamp_px * 0.35, placed.y + stamp_px * 0.17),
            stamp_px,
            ink,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn placards(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    bounce: f32,
    tilt: f32,
    pace: f32,
    hue_offset: f32,
    saturation: f32,
    pixel_scale: f32,
    report: &mut StageReport,
) {
    let words = spoken(moment, pace);
    // Signs need air between them, so the block is fitted with a wide space
    // and tall rows, then each word gets a board of its own.
    let (block, sized) = Block::fit(
        fonts,
        CaptionFace::SpaceGrotesk,
        moment.text,
        &words,
        boundary.width * 0.84,
        boundary.height * 0.62,
        boundary.height * 0.15 * scale,
        2.25,
        0.95,
    );
    if block.px < 4.0 {
        return;
    }
    let font = face(fonts, CaptionFace::SpaceGrotesk, block.px, moment.text);
    let origin = Vector2::new(
        boundary.x + boundary.width * 0.08,
        boundary.y + boundary.height * 0.12 + (boundary.height * 0.62 - block.height()) * 0.5,
    );
    let positions = block.positions(origin, Some(boundary.width * 0.84));
    report.rows = block.rows.len();
    report.type_px = block.px;
    report.sized_atlas = sized && font.is_sized();
    let floor = boundary.y + boundary.height;
    for (index, word) in words.iter().enumerate() {
        if word.shown <= 0.001 {
            continue;
        }
        let salt = moment.seed ^ moment.line_id.wrapping_mul(977) ^ index as u64;
        let board = Rectangle::new(
            positions[index].x - block.px * 0.26,
            positions[index].y - block.px * 0.2,
            block.widths[index] + block.px * 0.52,
            block.px * 1.42,
        );
        let rise = (1.0 - word.shown) * boundary.height * 0.75;
        let bob = (moment.time * 2.1 + index as f32 * 1.7).sin() * block.px * 0.035
            - moment.pulse * bounce * block.px * 0.07;
        let centre = Vector2::new(
            board.x + board.width * 0.5,
            board.y + board.height * 0.5 + rise + bob,
        );
        let lean =
            tilt + (unit(salt, 3) - 0.5) * 7.0 + (moment.time * 1.3 + index as f32).sin() * 0.8;
        let sign_hue = (268.0 + hue_offset + 160.0 + index as f32 * 47.0 + 720.0) % 360.0;
        let fill = draw::color_from_hsv(
            sign_hue,
            (0.62 * saturation).clamp(0.0, 1.0),
            if colors.daylight { 0.86 } else { 0.97 },
        );
        let lettering = Color::new(18, 14, 24, 255);
        let stick = if colors.daylight {
            Color::new(120, 96, 70, 255)
        } else {
            Color::new(196, 170, 136, 255)
        };
        transformed(d, centre, lean, 1.0, |d| {
            let placed = Rectangle::new(
                centre.x - board.width * 0.5,
                centre.y - board.height * 0.5,
                board.width,
                board.height,
            );
            d.rect(
                Rectangle::new(
                    centre.x - block.px * 0.045,
                    centre.y,
                    block.px * 0.09,
                    floor - centre.y + boundary.height * 0.2,
                ),
                with_alpha(stick, moment.presence),
            );
            d.rounded(
                Rectangle::new(
                    placed.x + block.px * 0.05,
                    placed.y + block.px * 0.07,
                    placed.width,
                    placed.height,
                ),
                0.16,
                with_alpha(Color::BLACK, 0.3 * moment.presence),
            );
            d.rounded(placed, 0.16, with_alpha(fill, moment.presence));
            d.rounded_outline(
                placed,
                0.16,
                pixel_scale * 2.0,
                with_alpha(lettering, 0.85 * moment.presence),
            );
            d.type_face(
                &font,
                word.text,
                Vector2::new(placed.x + block.px * 0.26, placed.y + block.px * 0.2),
                block.px,
                with_alpha(lettering, moment.presence),
            );
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn ticker(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    moment: &Moment<'_>,
    colors: &Palette,
    scale: f32,
    accent: f32,
    tilt: f32,
    pace: f32,
    pixel_scale: f32,
    report: &mut StageReport,
) {
    const CANDLES: u32 = 30;
    let words = spoken(moment, pace);
    let digits = "0123456789+-%";
    let covered = format!("{} {digits}", moment.text);
    let margin = boundary.width * 0.075;
    let (block, sized) = Block::fit(
        fonts,
        CaptionFace::SpaceGrotesk,
        &covered,
        &words,
        boundary.width * 0.62,
        boundary.height * 0.26,
        boundary.height * 0.115 * scale,
        1.08,
        0.27,
    );
    if block.px < 4.0 {
        return;
    }
    let font = face(fonts, CaptionFace::SpaceGrotesk, block.px, &covered);
    let positions = block.positions(
        Vector2::new(boundary.x + margin, boundary.y + boundary.height * 0.1),
        None,
    );
    let chart = Rectangle::new(
        boundary.x + margin,
        boundary.y + boundary.height * 0.42,
        boundary.width - margin * 2.0,
        boundary.height * 0.46,
    );
    // The walk is the line's own: same line, same chart, every time. It drifts
    // upward, because a chart that ends where it began is not a punchline.
    let mut prices = Vec::with_capacity(CANDLES as usize + 1);
    let mut price = 1.0f32;
    prices.push(price);
    for candle in 0..CANDLES {
        let step = stage::ticker_step(moment.seed, moment.line_id, candle);
        price *= 1.0 + 0.085 * step + 0.03;
        prices.push(price);
    }
    let shown = (moment.position * 1.08 * CANDLES as f32).min(CANDLES as f32);
    let whole = shown.floor() as usize;
    let part = shown - whole as f32;
    let now = if whole >= CANDLES as usize {
        prices[CANDLES as usize]
    } else {
        prices[whole] + (prices[whole + 1] - prices[whole]) * part
    };
    let (low, high) = prices.iter().fold((f32::MAX, f32::MIN), |(low, high), &p| {
        (low.min(p), high.max(p))
    });
    // Log prices: the walk compounds, and on a linear axis the first half of
    // a chart that doubles is a flat line of specks.
    let span = (high.ln() - low.ln()).max(0.001);
    let y_of = |p: f32| chart.y + chart.height * (1.0 - (p.ln() - low.ln()) / span * 0.92 - 0.04);
    let slot = chart.width / CANDLES as f32;
    let readout = format!("{:+.0}%", (now - 1.0) * 100.0);
    let readout_px = boundary.height * 0.2 * scale;
    // The same face as the headline, larger: the atlas cache hands out one
    // borrowed face at a time, and asking for a second while the first is
    // alive is a `RefCell` double borrow, not a second atlas.
    let readout_width = font.measure_text(&readout, readout_px, 0.0).x;
    let pivot = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    report.rows = block.rows.len();
    report.type_px = block.px;
    report.sized_atlas = sized && font.is_sized();
    transformed(d, pivot, tilt, 1.0, |d| {
        for (index, word) in words.iter().enumerate() {
            if word.shown <= 0.001 {
                continue;
            }
            d.type_face(
                &font,
                word.text,
                positions[index],
                block.px,
                with_alpha(
                    mix(colors.ink, colors.accent, word.sung * accent),
                    word.shown * moment.presence,
                ),
            );
        }
        d.type_face(
            &font,
            &readout,
            Vector2::new(
                boundary.x + boundary.width - margin - readout_width,
                boundary.y + boundary.height * 0.08,
            ),
            readout_px,
            with_alpha(
                if now >= 1.0 {
                    colors.accent
                } else {
                    colors.alarm
                },
                moment.presence,
            ),
        );
        for rule in 0..5 {
            let y = chart.y + chart.height * rule as f32 / 4.0;
            d.line(
                Vector2::new(chart.x, y),
                Vector2::new(chart.x + chart.width, y),
                pixel_scale,
                with_alpha(colors.ink, 0.10 * moment.presence),
            );
        }
        for candle in 0..CANDLES as usize {
            let grown = (shown - candle as f32).clamp(0.0, 1.0);
            if grown <= 0.0 {
                break;
            }
            let open = prices[candle];
            let close = open + (prices[candle + 1] - open) * grown;
            let rising = close >= open;
            let color = with_alpha(
                if rising { colors.accent } else { colors.alarm },
                moment.presence,
            );
            let x = chart.x + slot * (candle as f32 + 0.5);
            let (top, bottom) = (y_of(open.max(close)), y_of(open.min(close)));
            let wick = (top - bottom).abs() * 0.35 + slot * 0.12;
            d.line(
                Vector2::new(x, top - wick),
                Vector2::new(x, bottom + wick),
                pixel_scale * 1.5,
                color,
            );
            d.rect(
                Rectangle::new(
                    x - slot * 0.31,
                    top,
                    slot * 0.62,
                    (bottom - top).max(pixel_scale * 2.0),
                ),
                color,
            );
        }
    });
}

/// Draws Stage into `boundary`.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    frame: &SceneFrame<'_>,
    state: &StageState,
    boundary: Rectangle,
    pixel_scale: f32,
) -> StageReport {
    let setting = |index| frame.setting(SceneId::Stage, index);
    let layout = Layout::from_setting(setting(setting::LAYOUT));
    let mut report = StageReport {
        layout: layout.name(),
        ..StageReport::default()
    };
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return report;
    }
    let hue = setting(setting::HUE);
    let saturation = setting(setting::SATURATION);
    let scale = setting(setting::SCALE);
    let bounce = setting(setting::BOUNCE);
    let tilt = setting(setting::TILT);
    let light = setting(setting::LIGHT);
    let accent = setting(setting::ACCENT);
    let grain = setting(setting::GRAIN);
    let pace = setting(setting::PACE).max(0.1);
    let familiar = setting(setting::FAMILIAR) >= 0.5;
    let pixel_scale = if pixel_scale > 0.0 { pixel_scale } else { 1.0 };
    let colors = palette(hue, saturation, light);
    let time = frame.time_seconds as f32;
    // On the beat and decaying: a kick, not a wobble.
    let pulse = (-frame.audio.beat_phase.clamp(0.0, 1.0) * 5.0).exp();

    draw_ground(d, boundary, &colors, grain, time, state.seed, pixel_scale);

    if let Some(lyric) = frame.lyric.filter(|lyric| !lyric.text.trim().is_empty()) {
        let duration = (lyric.end_seconds - lyric.start_seconds).max(0.001) as f32;
        let elapsed = (frame.time_seconds - lyric.start_seconds).max(0.0) as f32;
        let position = (elapsed / duration).clamp(0.0, 1.0);
        let (words, span) = cadence::words_for_timed_cue(&lyric.text, f64::from(duration));
        let moment = Moment {
            text: &lyric.text,
            line_id: lyric.id,
            elapsed,
            duration,
            position,
            presence: stage::smooth(elapsed / 0.07) * stage::smooth((duration - elapsed) / 0.16),
            reveal: (position * LANDING / span.max(0.001)).clamp(0.0, 1.0),
            pulse,
            time,
            seed: state.seed,
        };
        report.words = words.len();
        match layout {
            Layout::Poster => poster(
                d,
                fonts,
                boundary,
                &moment,
                &colors,
                scale,
                bounce,
                accent,
                tilt,
                pace,
                &mut report,
            ),
            Layout::Terminal => terminal(
                d,
                boundary,
                &moment,
                &colors,
                scale,
                tilt,
                pace,
                pixel_scale,
                &mut report,
            ),
            Layout::Chat => chat(
                d,
                fonts,
                boundary,
                &moment,
                &colors,
                scale,
                bounce,
                accent,
                tilt,
                pace,
                &mut report,
            ),
            Layout::Card => card(
                d,
                fonts,
                boundary,
                &moment,
                &colors,
                scale,
                tilt,
                pace,
                pixel_scale,
                &mut report,
            ),
            Layout::Placards => placards(
                d,
                fonts,
                boundary,
                &moment,
                &colors,
                scale,
                bounce,
                tilt,
                pace,
                hue,
                saturation,
                pixel_scale,
                &mut report,
            ),
            Layout::Ticker => ticker(
                d,
                fonts,
                boundary,
                &moment,
                &colors,
                scale,
                accent,
                tilt,
                pace,
                pixel_scale,
                &mut report,
            ),
        }
    }

    if familiar {
        let size = boundary.height * 0.24;
        let foot = Vector2::new(
            boundary.x + boundary.width - size * 0.72,
            boundary.y + boundary.height - size * 0.08,
        );
        if !colors.daylight {
            // A pool of the accent under the cat, so black fur has an edge.
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
                blend.draw_circle_gradient(
                    foot.x as i32,
                    (foot.y - size * 0.4) as i32,
                    size * 0.62,
                    with_alpha(colors.accent, 0.16),
                    Color::BLANK,
                );
            });
        }
        draw_familiar(d, foot, size, &colors, pulse * bounce.min(1.0), time);
        report.familiar = true;
    }
    report
}
