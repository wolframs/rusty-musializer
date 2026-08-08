//! ASCII Field: the drawing half.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_ascii_field.c`'s `draw`
//! (frozen at `9300af9`, read-only). The scrolling spectrogram, the colour
//! pipeline and the glyph animation are all in
//! `musializer_core::scenes::ascii_field` and its `ascii_art` submodule, because
//! they are arithmetic and therefore testable; only the raylib calls are here.
//!
//! A CRT terminal: a navy surface with phosphor scanlines, a grid of animated
//! glyphs, chromatic-aberration ghosts on the detailed cells, and a slow sweep
//! line. With no imported image the glyph grid *is* the spectrogram; with one, the
//! image supplies the glyphs and the spectrogram tints them.
//!
//! Formulas and draw-call order are kept recognizable against the C on purpose.

// Nothing dispatches this yet: `main.rs` still calls Spectrum directly and the
// scene registry that will call all ten is the integration owner's. Remove this
// allow once the frame loop dispatches by `SceneId` — every item here is reached
// from `draw`.
#![allow(dead_code)]

use musializer_core::scene::settings::index::ascii as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::ascii_field::ascii_art::{self, Cell, EdgeOrientation, Grid, Rgba};
use musializer_core::scenes::ascii_field::{
    audio_band, clamp01, color_byte, main_color, seed_phase, AsciiFieldState, MAX_COLUMNS, MAX_ROWS,
};
use musializer_runtime::draw;
use raylib::prelude::{
    Color, RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt, Rectangle, Vector2,
};
use raylib::text::RaylibFont;

const PI: f32 = std::f32::consts::PI;

/// raylib's built-in font, borrowed rather than owned.
///
/// `ascii_grid_font` (`scene_ascii_field.c:154-160`) deliberately uses
/// `GetFontDefault()` rather than the bundled caption face: the caption face is
/// literary and proportional, while raylib's built-in pixel face is monospaced and
/// heavier at tiny sizes, which is what keeps individual ASCII samples legible
/// after video encoding.
///
/// A newtype rather than `raylib::text::WeakFont` because that type cannot be
/// constructed outside its crate. It owns nothing and unloads nothing.
///
/// Shared with `scenes::phosphor_dream`, which needs the same monospaced face
/// for the same reason. One type rather than two identical ones so the `unsafe`
/// inventory in `AGENTS.md` keeps naming a single island.
pub(crate) struct DefaultFont(raylib_sys::Font);

impl DefaultFont {
    pub(crate) fn get() -> Self {
        // SAFETY: a getter over raylib's global default font, which exists for as
        // long as the window does. The handle is never unloaded by this type.
        Self(unsafe { raylib_sys::GetFontDefault() })
    }
}

impl AsRef<raylib_sys::Font> for DefaultFont {
    fn as_ref(&self) -> &raylib_sys::Font {
        &self.0
    }
}

impl AsMut<raylib_sys::Font> for DefaultFont {
    fn as_mut(&mut self) -> &mut raylib_sys::Font {
        &mut self.0
    }
}

impl RaylibFont for DefaultFont {}

fn to_color(rgba: Rgba) -> Color {
    Color::new(rgba.r, rgba.g, rgba.b, rgba.a)
}

/// `ascii_scissor` (`scene_ascii_field.c:130-152`).
///
/// Rounds *inward* — ceil on the near edges, floor on the far ones — so the
/// scissor never covers a pixel the boundary only partly contains.
fn scissor(boundary: Rectangle) -> Option<(i32, i32, i32, i32)> {
    let right = boundary.x + boundary.width;
    let bottom = boundary.y + boundary.height;
    if !boundary.x.is_finite()
        || !boundary.y.is_finite()
        || !right.is_finite()
        || !bottom.is_finite()
        || boundary.width <= 1.0
        || boundary.height <= 1.0
        || boundary.x < i32::MIN as f32
        || boundary.y < i32::MIN as f32
        || right > i32::MAX as f32
        || bottom > i32::MAX as f32
    {
        return None;
    }
    let left_i = boundary.x.ceil() as i32;
    let top_i = boundary.y.ceil() as i32;
    let right_i = right.floor() as i32;
    let bottom_i = bottom.floor() as i32;
    if right_i <= left_i || bottom_i <= top_i {
        return None;
    }
    Some((left_i, top_i, right_i - left_i, bottom_i - top_i))
}

/// `ascii_measure_glyph` (`scene_ascii_field.c:108-114`).
///
/// A codepoint that is not a valid `char` measures as a square of the font size,
/// which is C's `CodepointToUTF8` failure path.
fn measure_glyph(font: &DefaultFont, glyph: u32, font_size: f32) -> Vector2 {
    match char::from_u32(glyph) {
        Some(character) => {
            let mut buffer = [0u8; 4];
            font.measure_text(character.encode_utf8(&mut buffer), font_size, 0.0)
        }
        None => Vector2::new(font_size, font_size),
    }
}

/// `ascii_draw_glyph` (`scene_ascii_field.c:116-128`): centres the measured glyph
/// on `center`.
fn draw_glyph<D: RaylibDraw>(
    d: &mut D,
    font: &DefaultFont,
    glyph: u32,
    center: Vector2,
    font_size: f32,
    measured: Vector2,
    color: Color,
) {
    let position = Vector2::new(center.x - measured.x * 0.5, center.y - measured.y * 0.5);
    d.draw_text_codepoint(font, glyph as i32, position, font_size, color);
}

/// Draws ASCII Field into `boundary`.
///
/// `grid` is the imported image, if any — C's `renderer->ascii_cells` plus its two
/// dimensions (`scene.h:59-61`). `None` is the live spectrogram path.
///
/// The cell loops keep C's index walk rather than iterating: `source_x`/`source_y`
/// advance by a Bresenham step over the *imported* grid while `row`/`column` walk
/// the *drawn* grid, and the two are different sizes.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    state: &AsciiFieldState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
    grid: Option<&Grid>,
) {
    let Some((scissor_x, scissor_y, scissor_width, scissor_height)) = scissor(boundary) else {
        return;
    };

    let energy = clamp01(frame.audio.rms * 2.0);
    let flux = clamp01(frame.audio.spectral_flux * 5.0);
    let pulse = if frame.audio.onset { 1.0f32 } else { 0.0 };
    // Wrapped at 4096 s so the sine arguments stay in a range where a float still
    // has resolution; a three-hour set would otherwise visibly stop animating.
    let time = if frame.time_seconds.is_finite() {
        (frame.time_seconds % 4096.0) as f32
    } else {
        0.0
    };
    let phase_offset = seed_phase(state.seed());
    let semantic_weight = if frame.semantic.available {
        clamp01(frame.semantic.confidence)
    } else {
        0.0
    };
    let semantic_tension = if frame.semantic.available {
        clamp01(frame.semantic.tension) * semantic_weight
    } else {
        0.0
    };
    let semantic_valence = if frame.semantic.available {
        clamp01((frame.semantic.valence + 1.0) * 0.5) * semantic_weight
    } else {
        0.0
    };
    let pixel_scale = if pixel_scale > 0.0 { pixel_scale } else { 1.0 };
    let motion_scale = frame.setting(SceneId::AsciiField, setting::MOTION);
    let cycling_scale = frame.setting(SceneId::AsciiField, setting::CYCLING);
    let scanline_scale = frame.setting(SceneId::AsciiField, setting::SCANLINES);
    let split_scale = frame.setting(SceneId::AsciiField, setting::SPLIT);
    let gain = frame.setting(SceneId::AsciiField, setting::GAIN);
    let tint = frame.setting(SceneId::AsciiField, setting::TINT);

    // Deep navy CRT surface: flat enough to preserve image contrast, but with
    // deterministic phosphor bands so an empty dark source does not become an
    // undifferentiated black rectangle.
    d.draw_rectangle_rec(boundary, Color::new(5, 4, 18, 255));
    let background_step = (6.0 * pixel_scale).max(2.0);
    let mut y = boundary.y;
    while y < boundary.y + boundary.height {
        d.draw_rectangle_rec(
            Rectangle::new(boundary.x, y, boundary.width, (2.0 * pixel_scale).max(1.0)),
            Color::new(0, 0, 0, color_byte(34.0 * scanline_scale)),
        );
        y += background_step;
    }

    let mut clipped = d.begin_scissor_mode(scissor_x, scissor_y, scissor_width, scissor_height);
    let imported = grid.filter(|grid| grid.is_populated());
    // `columns`/`rows` address the source grid and `draw_columns`/`draw_rows` the
    // drawn one; they differ only for an imported image bigger than the drawn
    // bound (`scene_ascii_field.c:260-264`).
    //
    // The two branches take their dimensions from different places on purpose. An
    // imported picture keeps the grid it was converted at, because that grid
    // carries the *image's* aspect and refitting it to the frame's would stretch
    // the picture. Only the live spectrogram — which has no aspect of its own —
    // is fitted to the frame.
    let (columns, rows, draw_columns, draw_rows) = match imported {
        Some(grid) => (
            grid.columns(),
            grid.rows(),
            grid.columns().min(MAX_COLUMNS),
            grid.rows().min(MAX_ROWS),
        ),
        None => {
            let Some((columns, rows)) = ascii_art::live_grid(boundary.width, boundary.height)
            else {
                return;
            };
            (columns, rows, columns, rows)
        }
    };
    let margin = (9.0 * pixel_scale).max(boundary.width.min(boundary.height) * 0.035);
    if boundary.width <= margin * 2.0 || boundary.height <= margin * 2.0 {
        return;
    }
    let drawing_area = Rectangle::new(
        boundary.x + margin,
        boundary.y + margin,
        boundary.width - margin * 2.0,
        boundary.height - margin * 2.0,
    );
    let Some(layout) = ascii_art::grid_layout(
        drawing_area.width,
        drawing_area.height,
        draw_columns,
        draw_rows,
        1.0,
    ) else {
        return;
    };
    let origin_x = drawing_area.x + layout.offset_x;
    let origin_y = drawing_area.y + layout.offset_y;
    let font = DefaultFont::get();
    let font_size = layout.cell_height * 1.15;
    let field_box = Rectangle::new(
        origin_x - 2.0 * pixel_scale,
        origin_y - 2.0 * pixel_scale,
        layout.field_width + 4.0 * pixel_scale,
        layout.field_height + 4.0 * pixel_scale,
    );
    clipped.draw_rectangle_rec(field_box, Color::new(2, 5, 16, 126));
    clipped.draw_rectangle_lines_ex(field_box, pixel_scale.max(1.0), Color::new(0, 240, 255, 34));

    let mut source_y = 0usize;
    let mut y_error = 0usize;
    let y_step = rows / draw_rows;
    let y_remainder = rows % draw_rows;
    for row in 0..draw_rows {
        let mut source_x = 0usize;
        let mut x_error = 0usize;
        let x_step = columns / draw_columns;
        let x_remainder = columns % draw_columns;
        for column in 0..draw_columns {
            // The history is a fixed 54x96 grid whatever the drawn grid's size, so
            // the lookup spreads the drawn cells across all of it. It is a
            // rescale, not an index — `row < draw_rows` gives `history_row < 54`
            // for *any* `draw_rows` — which is why a portrait frame may draw more
            // rows than the history holds and merely repeat sampled ones.
            let history_row = row * MAX_ROWS / draw_rows;
            let history_column = column * MAX_COLUMNS / draw_columns;
            let spectrum_density = state.density(history_row, history_column);
            let mut blended = Cell {
                glyph: u32::from(b'#'),
                foreground: Rgba {
                    r: 0,
                    g: 220,
                    b: 255,
                    a: 255,
                },
                luminance: spectrum_density,
                edge_strength: 0.0,
                edge_orientation: EdgeOrientation::None,
            };
            if let Some(grid) = imported {
                if let Some(cell) = grid.cell(source_y, source_x) {
                    blended = *cell;
                }
                blended.luminance = clamp01(blended.luminance * 0.78 + spectrum_density * 0.42);
                blended.foreground.g =
                    color_byte(f32::from(blended.foreground.g) * 0.82 + spectrum_density * 46.0);
                blended.foreground.b =
                    color_byte(f32::from(blended.foreground.b) * 0.82 + spectrum_density * 64.0);
            } else {
                // Band hue spread stretches the phosphor palette across the
                // frequency axis: zero is a single-hue CRT, one sweeps
                // bass-to-treble through the whole spectrum.
                let live = draw::color_from_hsv(
                    (188.0
                        + column as f32 / draw_columns as f32 * (36.0 + tint * 270.0)
                        + semantic_valence * 28.0)
                        % 360.0,
                    0.78,
                    0.30 + spectrum_density.powf(0.75) * 0.70,
                );
                blended.foreground = Rgba {
                    r: live.r,
                    g: live.g,
                    b: live.b,
                    a: color_byte((0.34 + spectrum_density * 1.30) * 255.0),
                };
            }
            let cell = &blended;
            let band = audio_band(frame.audio.bands, column, draw_columns);
            let horizontal_t = (column as f32 + 0.5) / draw_columns as f32;
            let vertical_t = (row as f32 + 0.5) / draw_rows as f32;
            // Two waves at different rates and different diagonals, so the field
            // ripples rather than pulsing as one rigid sheet.
            let phase = phase_offset + horizontal_t * PI * 4.0 - vertical_t * PI * 1.7;
            let glyph_wave = (time * motion_scale * (0.66 + energy * 0.18) + phase).sin();
            let counter_wave = (time * motion_scale * 0.43 - vertical_t * PI * 3.2
                + horizontal_t * PI * 0.8
                + phase_offset)
                .sin();
            let column_drift = glyph_wave
                * layout.cell_height
                * (0.035 + band * 0.060 + energy * 0.018)
                * motion_scale;
            let row_drift = counter_wave
                * layout.cell_width
                * (0.012 + flux * 0.030 + pulse * 0.016)
                * motion_scale;
            let center_x = origin_x + (column as f32 + 0.5) * layout.cell_width + row_drift;
            let center_y = origin_y + (row as f32 + 0.5) * layout.cell_height + column_drift;

            let edge = clamp01(cell.edge_strength * 4.0);
            let luminance = clamp01(cell.luminance);
            let color = to_color(main_color(cell, band, energy, semantic_tension, gain));

            let glyph_activity =
                clamp01((energy * 0.48 + band * 0.36 + flux * 0.11 + pulse * 0.24) * cycling_scale);
            // Cycling scales the animation *clock*, not just its amplitude, so
            // zero freezes the glyphs rather than merely calming them.
            let glyph = ascii_art::animated_glyph(
                cell,
                row,
                column,
                frame.time_seconds * f64::from(cycling_scale),
                glyph_activity,
                state.seed(),
            );
            if glyph != u32::from(b' ') && glyph != 0 {
                let center = Vector2::new(center_x, center_y);
                let measured = measure_glyph(&font, glyph, font_size);
                let cell_hash = state.seed()
                    ^ (row as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    ^ (column as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                // Ghosting on the detailed cells, plus a deterministic 1-in-32
                // sprinkle over the mid-tones so the aberration reads as a whole
                // failing tube rather than an outline effect.
                let chromatic_detail =
                    edge > 0.18 || luminance > 0.62 || ((cell_hash >> 59) == 0 && luminance > 0.24);
                if chromatic_detail {
                    let split = pixel_scale
                        * (0.42 + flux * 1.20 + pulse * 1.10 + semantic_tension * 0.65)
                        * split_scale;
                    let ghost_alpha = color_byte(
                        f32::from(color.a) * (0.13 + flux * 0.10 + pulse * 0.07) * split_scale,
                    );
                    let magenta = Color::new(
                        255,
                        color_byte(18.0 + semantic_valence * 28.0),
                        110,
                        ghost_alpha,
                    );
                    let cyan = Color::new(0, 240, 255, ghost_alpha);
                    draw_glyph(
                        &mut clipped,
                        &font,
                        glyph,
                        Vector2::new(center.x - split, center.y),
                        font_size,
                        measured,
                        magenta,
                    );
                    draw_glyph(
                        &mut clipped,
                        &font,
                        glyph,
                        Vector2::new(center.x + split, center.y),
                        font_size,
                        measured,
                        cyan,
                    );
                }
                draw_glyph(
                    &mut clipped,
                    &font,
                    glyph,
                    center,
                    font_size,
                    measured,
                    color,
                );
            }

            let mut x_advance = x_step;
            x_error += x_remainder;
            if x_error >= draw_columns {
                x_advance += 1;
                x_error -= draw_columns;
            }
            source_x += x_advance;
        }
        let mut y_advance = y_step;
        y_error += y_remainder;
        if y_error >= draw_rows {
            y_advance += 1;
            y_error -= draw_rows;
        }
        source_y += y_advance;
    }

    // Two-pixel dark bands plus a faint phosphor lip survive downsampling and
    // ordinary web-video compression better than the old alpha-12 hairlines.
    let mut y = field_box.y;
    while y < field_box.y + field_box.height {
        clipped.draw_rectangle_rec(
            Rectangle::new(
                field_box.x,
                y,
                field_box.width,
                (2.0 * pixel_scale).max(1.0),
            ),
            Color::new(0, 0, 0, color_byte((42.0 + energy * 10.0) * scanline_scale)),
        );
        clipped.draw_rectangle_rec(
            Rectangle::new(
                field_box.x,
                y + (2.0 * pixel_scale).max(1.0),
                field_box.width,
                pixel_scale.max(1.0),
            ),
            Color::new(0, 240, 255, color_byte(10.0 * scanline_scale)),
        );
        y += background_step;
    }
    let mut sweep_phase = (time * (0.055 + energy * 0.045) + phase_offset / (2.0 * PI)) % 1.0;
    if sweep_phase < 0.0 {
        sweep_phase += 1.0;
    }
    let sweep_y = field_box.y + sweep_phase * field_box.height;
    clipped.draw_rectangle_rec(
        Rectangle::new(field_box.x, sweep_y, field_box.width, pixel_scale.max(1.0)),
        Color::new(
            0,
            240,
            255,
            color_byte((18.0 + energy * 24.0 + pulse * 25.0) * scanline_scale),
        ),
    );
}
