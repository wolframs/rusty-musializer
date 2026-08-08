//! Phosphor Dream: the drawing half.
//!
//! **No oracle.** The arithmetic lives in
//! [`musializer_core::scenes::phosphor_dream`] — fields, alphabets, crossfade,
//! the audio followers — and only the raylib calls are here, which is the same
//! split every other scene uses. See that module for what this scene is and what
//! changed on the way in from the Python renderer it is adapted from.
//!
//! ## The CRT, and why it is built this way
//!
//! The source pushes each finished frame through a CPU chain: two-tier bloom,
//! chromatic aberration on the glow, scanlines, a rolling refresh band and a
//! filmic rolloff. At 1920x1080 in numpy that costs a large fraction of the
//! nine minutes a two-minute render takes, which is fine offline and impossible
//! at 60 fps. Each piece is therefore re-sited rather than reproduced:
//!
//! - **Bloom** goes through the [`HaloBlur`] the caption glow already uses — a
//!   real separable Gaussian offscreen, at a downsampled resolution. Its source
//!   is *not* a second pass of the glyphs: it is one flat quad per cell. A bloom
//!   has no high frequencies by construction (the same argument `halo.rs` makes
//!   about its own buffer scale), so blurring cell coverage and blurring the
//!   glyphs inside those cells give the same halo, and the quad version costs no
//!   font work at all.
//! - **Chromatic aberration** is the same blurred buffer composited twice more,
//!   offset either way and tinted to the red and blue channels. The source
//!   splits the glow only, keeping the glyphs themselves readable; that is worth
//!   keeping, and it falls out of compositing rather than needing a pass.
//! - **The filmic rolloff is not reproduced.** The source applies it to
//!   `frame + 0.9 * glow` — the *bloomed* signal, which reaches about 2 — where
//!   it stops a hot core clipping to a flat white plate. Reaching that signal
//!   here would need a second full-frame render target. Running the same curve
//!   on the bare cell value instead, which is what the first version did, is not
//!   a cheaper approximation of it: it is a 40 % dimmer, and it made Plasma two
//!   green blobs on black. Additive blending saturating against white does
//!   approximately the same job at the top of the range.
//! - **Scanlines, the rolling band and the vignette** are rectangle draws, as
//!   they are in ASCII Field.
//!
//! What this loses against the source is the shoulder above `bloom` 1.6, where a
//! saturated core goes flat white a little sooner than the film curve would take
//! it. Recorded rather than hidden.

use musializer_core::scene::settings::index::phosphor as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::phosphor_dream::{
    self as field, FieldGrid, FieldParams, Ink, PhosphorDreamState, Shape,
};
use musializer_runtime::draw;
use musializer_runtime::halo::HaloBlur;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt,
    Rectangle, Vector2,
};
use raylib::text::RaylibFont;

use super::ascii_field::DefaultFont;

/// Rows the field draws at `density` 1.0, and the cell aspect that goes with it.
///
/// The source's 54 rows of 12x20 cells, which is where 1920x1080 gives exactly
/// its 160x54 grid. Kept as a row count over the **short** axis rather than as a
/// fixed 160x54, so a 9:16 export gets a portrait field instead of a band across
/// a quarter of the frame — the defect EX2 fixed in ASCII Field, and the same
/// arithmetic.
const ROWS_AT_UNIT_DENSITY: f32 = 54.0;
/// Cell width over cell height. The source's 12/20.
const CELL_ASPECT: f32 = 0.6;

/// The most cells this scene will evaluate and draw in one frame.
///
/// A ceiling rather than a guess: the grid derives from the frame's short axis,
/// so a very wide window at low density asks for an unbounded column count, and
/// the per-cell cost is real. 32,000 is comfortably above the 8,640 the source
/// renders and above what a 4K export at density 2.0 needs; past it the cells
/// grow rather than multiplying.
const MAX_CELLS: usize = 32_000;

/// The bloom buffer's longest edge, in texels.
///
/// Deliberately small. The blur is the expensive pass and its output is
/// low-frequency, so a quarter-resolution buffer upsampled bilinearly is
/// indistinguishable from a full one — `halo.rs` makes the same argument about
/// its own scale, and this is the number that keeps the shader's 17 taps
/// covering a wide haze without the kernel growing.
const BLOOM_BUFFER_EDGE: i32 = 320;

/// Sigma of the bloom, in texels of the buffer above.
///
/// Two values because the source runs two tiers — a tight halo around the
/// glyphs and a wide atmospheric haze — and one Gaussian cannot be both. Built
/// as two composites of one buffer at different scales rather than as two
/// blurs, which is where the cost would double.
const BLOOM_SIGMA: f32 = 2.1;

/// Everything the draw needs that is not the frame.
///
/// Both buffers are owned by the renderer and reused: the grid is up to 32,000
/// cells and the blur owns GPU allocations, and rebuilding either per frame is
/// the kind of cost that is invisible in a preview and shows up as a stutter
/// halfway through a long export.
pub struct PhosphorResources {
    pub grid: FieldGrid,
    pub bloom: HaloBlur,
    /// What the last frame's bloom did — `blurred`, `off`, or `unavailable` —
    /// for the report line. A frame drawn with no bloom and a frame whose bloom
    /// silently failed to build are the same picture, which is the distinction
    /// this repository has paid for more than once.
    pub bloom_status: &'static str,
    /// The last resolved grid and field, for the report line.
    pub last: String,
}

impl PhosphorResources {
    pub fn load(rl: &mut raylib::RaylibHandle, thread: &raylib::RaylibThread) -> Self {
        Self {
            grid: FieldGrid::new(),
            bloom: HaloBlur::load(rl, thread),
            bloom_status: "none",
            last: "none".to_string(),
        }
    }

    /// One line for the slice report.
    ///
    /// Names the field, the alphabet and the grid, because every one of them is
    /// a thing a capture cannot state on its own: two fields in the same
    /// alphabet photograph as the same kind of picture, and a grid that silently
    /// collapsed to its minimum still draws something plausible.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{} bloom={}", self.last, self.bloom_status)
    }
}

/// The grid geometry for a boundary.
///
/// Returned rather than computed twice, because the bloom pass and the glyph
/// pass must agree on it exactly — a half-cell disagreement puts the halo off
/// the glyphs it belongs to, which reads as a badly registered CRT rather than
/// as a bug.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    columns: usize,
    rows: usize,
    cell_width: f32,
    cell_height: f32,
    origin_x: f32,
    origin_y: f32,
}

impl Geometry {
    fn resolve(boundary: Rectangle, density: f32) -> Option<Self> {
        if !boundary.width.is_finite()
            || !boundary.height.is_finite()
            || boundary.width < 8.0
            || boundary.height < 8.0
        {
            return None;
        }
        let density = if density.is_finite() && density > 0.0 {
            density
        } else {
            1.0
        };
        let short_axis = boundary.width.min(boundary.height);
        let mut cell_height = short_axis / (ROWS_AT_UNIT_DENSITY / density);
        // Grow the cell rather than the count when the ask is unbounded. The
        // loop terminates because each step multiplies the cell edge.
        for _ in 0..8 {
            let width = (cell_height * CELL_ASPECT).max(1.0);
            let columns = (boundary.width / width).floor().max(1.0) as usize;
            let rows = (boundary.height / cell_height).floor().max(1.0) as usize;
            if columns * rows <= MAX_CELLS {
                break;
            }
            cell_height *= 1.35;
        }
        let cell_width = (cell_height * CELL_ASPECT).max(1.0);
        let cell_height = cell_height.max(1.0);
        let columns = (boundary.width / cell_width).floor().max(1.0) as usize;
        let rows = (boundary.height / cell_height).floor().max(1.0) as usize;
        if columns * rows > MAX_CELLS {
            return None;
        }
        // Centre the field in the boundary: the cell count is a floor, so there
        // is up to one cell of slack on each axis and letting it all fall on the
        // right edge reads as the picture being nailed to one corner.
        let field_width = columns as f32 * cell_width;
        let field_height = rows as f32 * cell_height;
        Some(Self {
            columns,
            rows,
            cell_width,
            cell_height,
            origin_x: boundary.x + (boundary.width - field_width) * 0.5,
            origin_y: boundary.y + (boundary.height - field_height) * 0.5,
        })
    }

    fn cell_rect(&self, row: usize, column: usize) -> Rectangle {
        Rectangle::new(
            self.origin_x + column as f32 * self.cell_width,
            self.origin_y + row as f32 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }
}

/// Resolves one cell to a colour: the shaped value, lifted and clipped.
///
/// The `1.25` is the source's, and so is the *absence* of anything else here.
///
/// An earlier version ran the source's filmic rolloff — `x / (1 + 0.55x)` and a
/// mild gamma — on this value, on the reasoning that a per-cell curve is cheaper
/// than a full-frame pass. That was wrong, and a capture is what said so: the
/// source applies that curve to `frame + 0.9 * glow`, a signal that reaches
/// about 2, where its job is to stop a bloomed core clipping to a flat white
/// plate. Applied to a bare cell value that rarely passes 0.8, the same curve is
/// simply a 40 % dimmer, and it turned Plasma into two green blobs on black.
///
/// So the rolloff is **not reproduced**. The compression it provided at the top
/// of the range is approximately what additive blending does to the bloom
/// composite anyway, since that saturates against white in the framebuffer.
fn cell_color(cell: &field::GridCell, gain: f32) -> Color {
    let value = (cell.value * 1.25 * gain).clamp(0.0, 1.0);
    let rgb = draw::color_from_hsv(cell.hue * 360.0, cell.saturation, value);
    Color::new(rgb.r, rgb.g, rgb.b, 255)
}

/// Builds the bloom source and blurs it. Must run with **no scissor active**.
///
/// Called from `scene_host` before the scene's clip is opened, for the reason
/// [`HaloBlur::render`] documents: a scissor rect is global GL state in
/// framebuffer coordinates and would clip the offscreen passes with a rectangle
/// meant for the screen.
///
/// The source is one flat quad per cell rather than a second pass of the glyphs.
/// See the module note — a Gaussian this wide cannot tell the two apart, and the
/// quad version costs no font work.
pub fn prepare(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut PhosphorResources,
    state: &PhosphorDreamState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let scene = SceneId::PhosphorDream;
    let density = frame.setting(scene, setting::DENSITY);
    let bloom_strength = frame.setting(scene, setting::BLOOM);
    let gain = 1.0;

    let Some(geometry) = Geometry::resolve(boundary, density) else {
        resources.grid = FieldGrid::new();
        resources.bloom_status = "off";
        resources.last = "grid=none".to_string();
        return;
    };

    let params = FieldParams::resolve(state, frame, CELL_ASPECT);
    field::evaluate_grid(
        &mut resources.grid,
        state,
        &params,
        geometry.columns,
        geometry.rows,
    );
    // The brightest and mean cell go on the line beside the envelopes, because
    // "the scene is coupled to the track" and "the scene is dark" are separate
    // claims and a capture makes them look like one. A field whose peak sits at
    // 0.3 is underexposed no matter how good the coupling is.
    let (mean, peak) = {
        let count = geometry.columns * geometry.rows;
        let mut sum = 0.0f32;
        let mut peak = 0.0f32;
        for row in 0..geometry.rows {
            for column in 0..geometry.columns {
                if let Some(cell) = resources.grid.cell(row, column) {
                    sum += cell.value;
                    peak = peak.max(cell.value);
                }
            }
        }
        (sum / count.max(1) as f32, peak)
    };
    resources.last = format!(
        "field={} ramp={} grid={}x{} amp={:.2} bass={:.2} mean={:.2} peak={:.2}{}",
        params.field.name(),
        params.ramp_name(),
        geometry.columns,
        geometry.rows,
        params.amplitude,
        params.bass,
        mean,
        peak,
        match params.previous {
            Some((previous, _)) =>
                format!(" fading-from={} mix={:.2}", previous.name(), params.mix),
            None => String::new(),
        }
    );

    if bloom_strength <= 0.0 {
        resources.bloom_status = "off";
        return;
    }
    if !resources.bloom.available() {
        resources.bloom_status = "unavailable";
        return;
    }

    // The buffer keeps the boundary's aspect so the halo is not stretched, and
    // its longest edge is capped: the blur is the expensive pass and its output
    // has no detail to lose.
    let aspect = boundary.width / boundary.height.max(1.0);
    let (buffer_width, buffer_height) = if aspect >= 1.0 {
        (
            BLOOM_BUFFER_EDGE,
            (BLOOM_BUFFER_EDGE as f32 / aspect).round().max(16.0) as i32,
        )
    } else {
        (
            (BLOOM_BUFFER_EDGE as f32 * aspect).round().max(16.0) as i32,
            BLOOM_BUFFER_EDGE,
        )
    };

    let grid = &resources.grid;
    let scale_x = buffer_width as f32 / geometry.columns as f32;
    let scale_y = buffer_height as f32 / geometry.rows as f32;
    let built = resources
        .bloom
        .render(d, buffer_width, buffer_height, BLOOM_SIGMA, |target| {
            for row in 0..geometry.rows {
                for column in 0..geometry.columns {
                    let Some(cell) = grid.cell(row, column) else {
                        continue;
                    };
                    if cell.ink.is_blank() {
                        continue;
                    }
                    let color = cell_color(cell, gain);
                    // Ink coverage matters: a blank-ish glyph in a bright cell
                    // should not bloom like a solid block, or the haze loses the
                    // field's structure and becomes a flat wash.
                    let coverage = ink_coverage(cell.ink);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let inset = (1.0 - coverage) * 0.5;
                    target.draw_rectangle_rec(
                        Rectangle::new(
                            (column as f32 + inset) * scale_x,
                            (row as f32 + inset) * scale_y,
                            (scale_x * coverage).max(1.0),
                            (scale_y * coverage).max(1.0),
                        ),
                        color,
                    );
                }
            }
        });
    resources.bloom_status = if built { "blurred" } else { "unavailable" };
}

/// Roughly how much of its cell a glyph covers, `0..1`.
///
/// Used only to weight the bloom. Approximate on purpose — the exact coverage of
/// `@` versus `#` is not a thing a Gaussian this wide can show, but the
/// difference between a period and a filled block very much is.
fn ink_coverage(ink: Ink) -> f32 {
    match ink {
        Ink::Blank => 0.0,
        Ink::Char(character) => match character {
            '.' | '\'' | '`' | ',' | '·' => 0.18,
            ':' | ';' | '-' | '_' | '~' => 0.28,
            '+' | '=' | 'x' | '*' | '×' => 0.42,
            'o' | 'X' | '1' => 0.52,
            '#' | '%' | '$' | '&' | '8' | 'B' | 'O' | '0' | 'Æ' => 0.72,
            '@' => 0.86,
            _ => 0.45,
        },
        Ink::Shape(shape) => match shape {
            Shape::Fill(quarters) => (f32::from(quarters) / 4.0).clamp(0.0, 1.0),
            Shape::Disc
            | Shape::Square {
                filled: true,
                large: true,
            } => 0.80,
            Shape::Diamond { filled: true } => 0.62,
            Shape::Square {
                filled: true,
                large: false,
            } => 0.42,
            Shape::RingLarge
            | Shape::Square {
                filled: false,
                large: true,
            } => 0.50,
            Shape::RingSmall
            | Shape::Square {
                filled: false,
                large: false,
            } => 0.30,
            Shape::Diamond { filled: false } => 0.40,
            Shape::Hatch { .. } => 0.55,
            Shape::Bars(count) => f32::from(count) * 0.16,
            Shape::Star4 => 0.44,
            Shape::Star6 => 0.50,
        },
    }
}

/// Draws Phosphor Dream into `boundary`.
///
/// [`prepare`] must have run for this frame first — it owns grid evaluation and
/// the offscreen bloom, both of which have to happen before the scene's scissor
/// is opened.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut PhosphorResources,
    state: &PhosphorDreamState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    let scene = SceneId::PhosphorDream;
    let pixel_scale = if pixel_scale.is_finite() && pixel_scale > 0.0 {
        pixel_scale
    } else {
        1.0
    };
    let density = frame.setting(scene, setting::DENSITY);
    let bloom_strength = frame.setting(scene, setting::BLOOM);
    let aberration = frame.setting(scene, setting::ABERRATION);
    let scanlines = frame.setting(scene, setting::SCANLINES);
    let reactivity = frame.setting(scene, setting::REACTIVITY);
    let titles_on = frame.setting(scene, setting::TITLES) >= 0.5;

    // The tube itself. Near-black rather than black so an empty field still
    // reads as a screen that is on.
    d.draw_rectangle_rec(boundary, Color::new(4, 3, 10, 255));

    let Some(geometry) = Geometry::resolve(boundary, density) else {
        return;
    };
    if resources.grid.columns() != geometry.columns || resources.grid.rows() != geometry.rows {
        // `prepare` did not run, or ran against a different boundary. Drawing a
        // stale grid stretched over a new geometry is a plausible picture of the
        // wrong frame, so it draws nothing and the report line says `grid=none`.
        return;
    }

    let time = if frame.time_seconds.is_finite() {
        (frame.time_seconds % 4096.0) as f32
    } else {
        0.0
    };
    let energy = state.amplitude();

    // The word the cue surfaces, if the toggle is on and there is one. Nothing
    // is surfaced with no cue: the source hardcodes its own six words, and
    // putting a canned "BREATHE" over somebody's track is not a thing this
    // application should do on their behalf.
    let surfaced = titles_on
        .then(|| frame.lyric.map(|cue| cue.text.trim()))
        .flatten()
        .filter(|text| !text.is_empty())
        .filter(|text| field::fit_line(text, geometry.columns, geometry.rows).is_some());

    let font = DefaultFont::get();
    let font_size = geometry.cell_height * 1.05;
    let stroke = (pixel_scale * 1.2).max(1.0);

    {
        let mut clipped = d.begin_scissor_mode(
            boundary.x as i32,
            boundary.y as i32,
            boundary.width as i32,
            boundary.height as i32,
        );
        for row in 0..geometry.rows {
            for column in 0..geometry.columns {
                let Some(cell) = resources.grid.cell(row, column) else {
                    continue;
                };
                let on_word = surfaced.is_some_and(|text| {
                    field::line_ink(text, geometry.columns, geometry.rows, row, column)
                });
                // A word surfaces by the field around it dimming and the letters
                // going bright and white, which is what makes it read as coming
                // *out of* the field rather than sitting on top of it.
                let (ink, color) = if let Some(text) = surfaced {
                    let _ = text;
                    if on_word {
                        (pick_word_ink(cell.ink), Color::new(255, 255, 255, 255))
                    } else {
                        let dimmed = cell_color(cell, 0.30);
                        (cell.ink, dimmed)
                    }
                } else {
                    (cell.ink, cell_color(cell, 1.0))
                };
                if ink.is_blank() {
                    continue;
                }
                let rect = geometry.cell_rect(row, column);
                draw_ink(&mut clipped, &font, ink, rect, font_size, stroke, color);
            }
        }
    }

    // The bloom, composited additively over the glyphs. The colour split is the
    // same buffer twice more, offset either way into the red and blue channels —
    // the source splits the glow and leaves the glyphs sharp, which is what
    // keeps the field readable while still reading as a failing tube.
    if bloom_strength > 0.0 {
        if let Some(texture) = resources.bloom.result() {
            let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
            let level =
                |scale: f32| -> u8 { (255.0 * (bloom_strength * scale).clamp(0.0, 1.0)) as u8 };
            let split = pixel_scale * 3.0 * aberration;
            let mut clipped = d.begin_scissor_mode(
                boundary.x as i32,
                boundary.y as i32,
                boundary.width as i32,
                boundary.height as i32,
            );
            let mut blend = clipped.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
            // Tight core, then the wide haze at a lower level: the source's two
            // tiers, built from one buffer by compositing it at two strengths
            // rather than by blurring twice.
            let white = Color::new(255, 255, 255, level(0.55));
            draw::draw_texture_pro(
                &mut blend,
                texture,
                source,
                boundary,
                Vector2::zero(),
                0.0,
                white,
            );
            if aberration > 0.0 {
                let red = Color::new(255, 0, 0, level(0.30));
                let blue = Color::new(0, 90, 255, level(0.30));
                let mut shifted = boundary;
                shifted.x = boundary.x + split;
                draw::draw_texture_pro(
                    &mut blend,
                    texture,
                    source,
                    shifted,
                    Vector2::zero(),
                    0.0,
                    red,
                );
                shifted.x = boundary.x - split;
                draw::draw_texture_pro(
                    &mut blend,
                    texture,
                    source,
                    shifted,
                    Vector2::zero(),
                    0.0,
                    blue,
                );
            }
        }
    }

    if scanlines > 0.0 {
        let mut clipped = d.begin_scissor_mode(
            boundary.x as i32,
            boundary.y as i32,
            boundary.width as i32,
            boundary.height as i32,
        );
        // Two-pixel dark bands. The same shape ASCII Field settled on, and for
        // the same reason: alpha-12 hairlines vanish under downsampling and
        // ordinary web-video compression, so the effect would survive the
        // preview and disappear from the export.
        let step = (geometry.cell_height * 0.5).max(2.0 * pixel_scale);
        let band = (2.0 * pixel_scale).max(1.0);
        let alpha = ((46.0 + energy * 14.0) * scanlines).clamp(0.0, 255.0) as u8;
        let mut y = boundary.y;
        while y < boundary.y + boundary.height {
            clipped.draw_rectangle_rec(
                Rectangle::new(boundary.x, y, boundary.width, band),
                Color::new(0, 0, 0, alpha),
            );
            y += step;
        }

        // The rolling refresh band — a CRT filmed on a camcorder. One slow wave
        // over the frame height, brightening rather than darkening, because the
        // artefact is the camera's shutter catching the beam mid-scan.
        let phase = (time * 0.33).rem_euclid(1.0);
        let height = boundary.height * 0.22;
        let top = boundary.y + phase * (boundary.height + height) - height;
        let steps = 12;
        for index in 0..steps {
            let u = (index as f32 + 0.5) / steps as f32;
            let strength = (u * std::f32::consts::PI).sin();
            let slab = Rectangle::new(
                boundary.x,
                top + u * height,
                boundary.width,
                (height / steps as f32).max(1.0),
            );
            clipped.draw_rectangle_rec(
                slab,
                Color::new(
                    120,
                    170,
                    255,
                    (10.0 * strength * scanlines * (0.6 + reactivity * 0.4)) as u8,
                ),
            );
        }
    }
}

/// The glyph a surfaced word's letters are drawn with.
///
/// The letters keep the field's own alphabet rather than switching to a solid
/// block, which is the whole point of the effect in the source: the word is
/// built from the same characters as everything around it. A blank cell inside a
/// letter would put a hole in it, so that one case is promoted.
fn pick_word_ink(ink: Ink) -> Ink {
    if ink.is_blank() {
        Ink::Char('#')
    } else {
        ink
    }
}

/// Draws one cell's ink.
#[allow(clippy::too_many_arguments)]
fn draw_ink<D: RaylibDraw>(
    d: &mut D,
    font: &DefaultFont,
    ink: Ink,
    rect: Rectangle,
    font_size: f32,
    stroke: f32,
    color: Color,
) {
    let center = Vector2::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    match ink {
        Ink::Blank => {}
        Ink::Char(character) => {
            let mut buffer = [0u8; 4];
            let text = character.encode_utf8(&mut buffer);
            let measured = font.measure_text(text, font_size, 0.0);
            d.draw_text_codepoint(
                font,
                character as i32,
                Vector2::new(center.x - measured.x * 0.5, center.y - measured.y * 0.5),
                font_size,
                color,
            );
        }
        Ink::Shape(shape) => draw_shape(d, shape, rect, center, stroke, color),
    }
}

/// The 4x4 dither patterns behind `░▒▓█`, one row per nibble, bit 3 leftmost.
///
/// The shade blocks are the only entries in any alphabet that can cover a whole
/// cell, so they are the only ones that can make *neighbouring* cells merge —
/// and the first version drew them as flat rectangles at a matching alpha, which
/// did exactly that. A capture of Plasma in the `blocks` alphabet came out as
/// three featureless teal blobs on black: correct coverage, and no longer
/// recognisable as a grid of characters, which is the entire point of the scene.
///
/// Real shade blocks are dither patterns, and that texture is what keeps the
/// cell boundaries visible when a whole region is bright. Reproduced here at 4x4
/// rather than at the font's own resolution because 4x4 is enough to read as
/// texture at the sizes this draws at, and because a regular pattern beats a
/// random one: the same cell must dither the same way every frame or the field
/// boils.
#[rustfmt::skip]
const SHADE_PATTERNS: [[u8; 4]; 3] = [
    // 25 % — sparse and evenly spread, so it reads as tone rather than as dots
    // in a line.
    [0b1000, 0b0010, 0b0001, 0b0100],
    // 50 % — checkerboard.
    [0b1010, 0b0101, 0b1010, 0b0101],
    // 75 % — the 25 % pattern inverted, so the three tones step evenly.
    [0b0111, 0b1101, 0b1110, 0b1011],
];

/// Draws one shade block: a solid rect at full coverage, a dither below it.
fn draw_shade_block<D: RaylibDraw>(d: &mut D, quarters: u8, rect: Rectangle, color: Color) {
    let quarters = quarters.clamp(1, 4);
    if quarters >= 4 {
        d.draw_rectangle_rec(rect, color);
        return;
    }
    let sub_width = rect.width * 0.25;
    let sub_height = rect.height * 0.25;
    // Below about four pixels a cell has no room for a 4x4 pattern, and drawing
    // one would land every sub-square on the same pixel and read as noise. A
    // flat rect at matching alpha is the honest answer there — the cells are too
    // small to merge visibly anyway.
    if sub_width < 1.0 || sub_height < 1.0 {
        let coverage = f32::from(quarters) / 4.0;
        let alpha = (f32::from(color.a) * coverage) as u8;
        d.draw_rectangle_rec(rect, Color::new(color.r, color.g, color.b, alpha));
        return;
    }
    let pattern = SHADE_PATTERNS[usize::from(quarters) - 1];
    for (row, bits) in pattern.iter().enumerate() {
        for column in 0..4u32 {
            if bits & (1 << (3 - column)) == 0 {
                continue;
            }
            d.draw_rectangle_rec(
                Rectangle::new(
                    rect.x + column as f32 * sub_width,
                    rect.y + row as f32 * sub_height,
                    sub_width.max(1.0),
                    sub_height.max(1.0),
                ),
                color,
            );
        }
    }
}

/// Draws a synthesized shape into its cell.
///
/// These stand in for the block elements and geometric characters raylib's
/// built-in face has no glyph for. Drawn from geometry, so they stay sharp at
/// any size — including under export supersampling, where a bitmap atlas glyph
/// magnified past its native size is the blur the caption work already had to
/// answer for once.
fn draw_shape<D: RaylibDraw>(
    d: &mut D,
    shape: Shape,
    rect: Rectangle,
    center: Vector2,
    stroke: f32,
    color: Color,
) {
    let edge = rect.width.min(rect.height);
    match shape {
        Shape::Fill(quarters) => draw_shade_block(d, quarters, rect, color),
        Shape::Disc => d.draw_circle_v(center, edge * 0.34, color),
        Shape::RingSmall => {
            d.draw_circle_lines(center.x as i32, center.y as i32, edge * 0.18, color);
        }
        Shape::RingLarge => {
            d.draw_circle_lines(center.x as i32, center.y as i32, edge * 0.34, color);
        }
        Shape::Diamond { filled } => {
            let radius = edge * 0.38;
            if filled {
                d.draw_poly(center, 4, radius, 45.0, color);
            } else {
                d.draw_poly_lines_ex(center, 4, radius, 45.0, stroke, color);
            }
        }
        Shape::Square { filled, large } => {
            let side = edge * if large { 0.62 } else { 0.34 };
            let box_rect = Rectangle::new(center.x - side * 0.5, center.y - side * 0.5, side, side);
            if filled {
                d.draw_rectangle_rec(box_rect, color);
            } else {
                d.draw_rectangle_lines_ex(box_rect, stroke, color);
            }
        }
        Shape::Hatch { back } => {
            let inset = edge * 0.18;
            let left = rect.x + inset;
            let right = rect.x + rect.width - inset;
            let top = rect.y + inset;
            let bottom = rect.y + rect.height - inset;
            for index in 0..3 {
                let t = (index as f32 + 0.5) / 3.0;
                let (from, to) = if back {
                    (
                        Vector2::new(left, top + (bottom - top) * t),
                        Vector2::new(left + (right - left) * t, top),
                    )
                } else {
                    (
                        Vector2::new(left, bottom - (bottom - top) * t),
                        Vector2::new(left + (right - left) * t, bottom),
                    )
                };
                d.draw_line_ex(from, to, stroke, color);
            }
        }
        Shape::Bars(count) => {
            let count = count.max(1);
            let span = rect.height * 0.5;
            for index in 0..count {
                let t = (f32::from(index) + 0.5) / f32::from(count);
                let y = center.y - span * 0.5 + span * t;
                d.draw_line_ex(
                    Vector2::new(rect.x + rect.width * 0.18, y),
                    Vector2::new(rect.x + rect.width * 0.82, y),
                    stroke,
                    color,
                );
            }
        }
        Shape::Star4 | Shape::Star6 => {
            let spokes = if matches!(shape, Shape::Star4) { 4 } else { 6 };
            let radius = edge * 0.40;
            for index in 0..spokes {
                let angle = std::f32::consts::TAU * index as f32 / spokes as f32
                    + std::f32::consts::FRAC_PI_4;
                let (sin, cos) = angle.sin_cos();
                d.draw_line_ex(
                    center,
                    Vector2::new(center.x + cos * radius, center.y + sin * radius),
                    stroke,
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fills_a_landscape_frame_at_the_sources_own_dimensions() {
        // 1920x1080 at density 1.0 is the source's 160x54 exactly, which is the
        // check that the row-count-over-short-axis rule reduces to its fixed
        // grid on the shape it was written for.
        let geometry = Geometry::resolve(Rectangle::new(0.0, 0.0, 1920.0, 1080.0), 1.0).unwrap();
        assert_eq!(geometry.rows, 54);
        assert_eq!(geometry.columns, 160);
    }

    #[test]
    fn a_portrait_frame_gets_a_portrait_field_rather_than_a_band() {
        // The EX2 defect: a fixed 160x54 min-fitted into 1080x1920 draws across
        // about a quarter of the height. The field must fill it.
        let boundary = Rectangle::new(0.0, 0.0, 1080.0, 1920.0);
        let geometry = Geometry::resolve(boundary, 1.0).unwrap();
        let covered = geometry.rows as f32 * geometry.cell_height / boundary.height;
        assert!(
            covered > 0.95,
            "the field covers only {covered} of a portrait frame"
        );
        assert!(
            geometry.rows > geometry.columns,
            "portrait frame, portrait grid"
        );
    }

    #[test]
    fn the_cell_count_is_capped_rather_than_unbounded() {
        // A very large frame at maximum density is where the per-cell cost would
        // run away. The cells grow instead.
        for (width, height, density) in [
            (3840.0, 2160.0, 2.0),
            (7680.0, 2160.0, 2.0),
            (1920.0, 1080.0, 2.0),
            (640.0, 360.0, 0.5),
        ] {
            let geometry =
                Geometry::resolve(Rectangle::new(0.0, 0.0, width, height), density).unwrap();
            assert!(
                geometry.columns * geometry.rows <= MAX_CELLS,
                "{width}x{height} @ {density} asked for {} cells",
                geometry.columns * geometry.rows
            );
            assert!(geometry.columns >= 1 && geometry.rows >= 1);
        }
    }

    #[test]
    fn a_degenerate_boundary_is_refused_rather_than_dividing_by_zero() {
        assert!(Geometry::resolve(Rectangle::new(0.0, 0.0, 0.0, 100.0), 1.0).is_none());
        assert!(Geometry::resolve(Rectangle::new(0.0, 0.0, 100.0, 2.0), 1.0).is_none());
        assert!(Geometry::resolve(Rectangle::new(0.0, 0.0, f32::NAN, 100.0), 1.0).is_none());
        // A zero or wild density falls back to 1.0 rather than producing no grid.
        assert!(Geometry::resolve(Rectangle::new(0.0, 0.0, 800.0, 600.0), 0.0).is_some());
        assert!(Geometry::resolve(Rectangle::new(0.0, 0.0, 800.0, 600.0), f32::NAN).is_some());
    }

    #[test]
    fn the_field_is_centred_in_its_boundary() {
        // The cell count is a floor, so there is slack; letting it all fall on
        // one edge reads as the picture being nailed to a corner.
        let boundary = Rectangle::new(100.0, 50.0, 977.0, 543.0);
        let geometry = Geometry::resolve(boundary, 1.0).unwrap();
        let left = geometry.origin_x - boundary.x;
        let right = (boundary.x + boundary.width)
            - (geometry.origin_x + geometry.columns as f32 * geometry.cell_width);
        assert!((left - right).abs() < 0.01, "{left} vs {right}");
        assert!(left >= 0.0 && right >= -0.01);
    }

    /// The glyph colour must use most of the range a cell value spans.
    ///
    /// This is the regression test for the rolloff mistake: the curve that used
    /// to sit here mapped a full-brightness cell to about 0.6, and every field
    /// read as underexposed. A capture caught it; this catches it next time.
    #[test]
    fn a_bright_cell_actually_reaches_full_brightness() {
        let bright = field::GridCell {
            value: 0.8,
            hue: 0.33,
            saturation: 1.0,
            ink: Ink::Char('@'),
        };
        let color = cell_color(&bright, 1.0);
        let peak = color.r.max(color.g).max(color.b);
        assert_eq!(
            peak, 255,
            "a 0.8 cell must clip to full after the 1.25 lift, not land at {peak}"
        );
        let dim = field::GridCell {
            value: 0.1,
            ..bright
        };
        let dim_peak = {
            let c = cell_color(&dim, 1.0);
            c.r.max(c.g).max(c.b)
        };
        assert!(
            (25..=45).contains(&dim_peak),
            "a 0.1 cell should stay dim but visible, got {dim_peak}"
        );
    }

    #[test]
    fn every_ink_has_a_bloom_coverage_and_only_blank_is_zero() {
        for ramp in field::RAMPS {
            for ink in ramp {
                let coverage = ink_coverage(*ink);
                assert!((0.0..=1.0).contains(&coverage), "{ink:?} -> {coverage}");
                assert_eq!(
                    coverage == 0.0,
                    ink.is_blank(),
                    "{ink:?}: only a blank cell may bloom nothing"
                );
            }
        }
    }

    /// The three dither patterns must actually step 25/50/75, or the shade
    /// ladder has a flat rung and the `blocks` alphabet loses a tone.
    #[test]
    fn the_shade_patterns_step_evenly_and_are_not_striped() {
        for (index, pattern) in SHADE_PATTERNS.iter().enumerate() {
            let lit: u32 = pattern.iter().map(|bits| bits.count_ones()).sum();
            assert_eq!(
                lit as usize,
                (index + 1) * 4,
                "pattern {index} covers {lit}/16, expected {}/16",
                (index + 1) * 4
            );
            // No row may be empty or full: either would draw as a stripe rather
            // than as tone, which is what the flat rectangle did wrong.
            for (row, bits) in pattern.iter().enumerate() {
                let count = bits.count_ones();
                assert!(
                    count > 0 && count < 4,
                    "pattern {index} row {row} is a stripe ({count}/4)"
                );
            }
        }
        // 25 % and 75 % must be complements, so the ladder is symmetric.
        for (low, high) in SHADE_PATTERNS[0].iter().zip(SHADE_PATTERNS[2].iter()) {
            assert_eq!(low & high, 0, "the 25 % and 75 % patterns overlap");
            assert_eq!(low | high, 0b1111, "the 25 % and 75 % patterns leave a gap");
        }
    }

    #[test]
    fn a_blank_cell_inside_a_surfaced_letter_is_promoted_so_words_have_no_holes() {
        assert!(!pick_word_ink(Ink::Blank).is_blank());
        assert_eq!(pick_word_ink(Ink::Char('@')), Ink::Char('@'));
    }
}
