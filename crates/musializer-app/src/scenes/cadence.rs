//! Cadence: the drawing half.
//!
//! Word splitting, the estimated word windows and the focus envelopes are in
//! `musializer_core::scenes::cadence`.
//!
//! The scene condenses a swarm of particles onto the letterforms of the lyric line
//! currently being sung, word by word. Two details in here are the reason it reads
//! as type rather than as a blur:
//!
//! - particles land on **inked pixels of the glyph bitmap**, sampled
//!   deterministically from the seed, not on a bounding box (`:200-235`);
//! - arrivals are staggered per particle, so a word condenses organically instead of
//!   translating as one rigid clump (`:360-367`).
//!
//! `../musializer/cadence-overhauls-2026-07-26.md` describes a different Cadence and
//! is an unimplemented scratchpad; it is deliberately not ported.

#![allow(dead_code)]

use std::ffi::CString;

use musializer_core::scene::settings::index::cadence as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::cadence::{
    self as cadence, timing, CadenceState, Focus, GlyphPose, Reveal, Word, AMBIENT_PARTICLES,
    INK_ALPHA_THRESHOLD, INK_PROBES, LAYOUT_ATTEMPTS, MAX_LAYOUT_ROWS, PARTICLES_PER_GLYPH,
    PARTICLE_BUDGET,
};
use musializer_runtime::draw;
use musializer_runtime::font::{Face, SDF_FIELD_TEXELS, SDF_UNITS_PER_TEXEL};
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibFont,
    RaylibShaderModeExt, Rectangle, Vector2,
};
use raylib::shaders::{RaylibShader, Shader};

const PI: f32 = std::f32::consts::PI;

/// Whether an imported caption face is actually loaded.
///
/// The C tests `renderer->font.texture.id != 0` and falls back to
/// `GetFontDefault()` (`scene_cadence.c:449`). The caller makes that choice, because
/// only it holds both fonts; this is the predicate it should use.
#[must_use]
pub fn font_is_loaded<F: RaylibFont>(font: &F) -> bool {
    font.texture().id != 0
}

/// The two faces Cadence typesets with, and the shader that makes the first one
/// legible.
///
/// A deliberate divergence from the oracle, which hands the scene one `Font`
/// (`scene_cadence.c:449`). Cadence animates per-glyph scale continuously as a
/// word condenses, so there is no size to rasterize a bitmap atlas at — every
/// choice is wrong for most of the animation, and the oracle's 64 px atlas is
/// wrong by a factor of three at the sizes this scene actually draws. A
/// signed-distance-field atlas is scale-independent instead.
///
/// **The two faces are not interchangeable, and that is the point.** `text`
/// measures and draws; `ink` is sampled for the inked pixels particles condense
/// onto (`cadence_glyph_ink`, `scene_cadence.c:200-235`), and an SDF glyph image
/// is a distance ramp rather than coverage — the C's 96/255 alpha threshold
/// applied to it would scatter particles over the glyph's *surroundings*. So the
/// particle field keeps reading the bitmap atlas, unchanged, and only the drawn
/// letterform moves to the distance field.
pub struct CadenceType<'a> {
    /// The face `measure` and every glyph draw go through.
    pub text: &'a Face,
    /// The face whose CPU-side glyph bitmaps place particles. Always a bitmap
    /// atlas.
    pub ink: &'a Face,
    /// The SDF fragment shader. `Some` only when `text` is an SDF atlas: drawing
    /// a distance field through the normal path is a field of grey blobs, so the
    /// two travel together or not at all.
    pub sdf: Option<&'a mut Shader>,
}

impl CadenceType<'_> {
    /// Which face typeset the frame, for the report line. A capture cannot show
    /// the difference between a soft bitmap glyph and a soft *export*, and the
    /// repository rule is that a surface which can draw without its data says
    /// which one it drew.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        if self.sdf.is_some() {
            "sdf"
        } else {
            "bitmap"
        }
    }
}

/// Which of the SDF shader's two passes a glyph is drawn through.
#[derive(Clone, Copy)]
enum GlyphPass {
    /// The letterform.
    Fill,
    /// The light around it, at this strength.
    Halo(f32),
}

/// The shader's `halo` and `haloReach` uniform locations.
#[derive(Clone, Copy)]
struct HaloUniforms {
    strength: i32,
    reach: i32,
}

/// The halo's reach in the field's own units: nearly all of what the atlas
/// holds outside the outline, stopping short of where the field clips to zero.
const HALO_REACH: f32 = (SDF_FIELD_TEXELS - 1.0) * SDF_UNITS_PER_TEXEL;

/// One glyph, through the SDF shader when there is one.
///
/// The shader is entered per glyph rather than once around the word because the
/// particle swarm is drawn between glyphs and must not go through it: the
/// distance-field threshold would be applied to raylib's 1x1 white shape texture,
/// which is a different picture. A word is tens of glyphs, so this is tens of
/// batch flushes per frame — measured against a scene that already draws hundreds
/// of additive circles, it does not register.
///
/// A [`GlyphPass::Halo`] with no shader draws nothing: the bitmap atlas has no
/// field to read a halo out of.
#[allow(clippy::too_many_arguments)]
fn draw_glyph<D: RaylibDraw>(
    d: &mut D,
    faces: &mut CadenceType<'_>,
    uniforms: Option<HaloUniforms>,
    pass: GlyphPass,
    codepoint: char,
    pen: Vector2,
    font_size: f32,
    tint: Color,
) {
    let CadenceType { text, sdf, .. } = faces;
    let text: &Face = text;
    match sdf.as_deref_mut() {
        Some(shader) => {
            if let Some(uniforms) = uniforms {
                let strength = match pass {
                    GlyphPass::Fill => 0.0f32,
                    GlyphPass::Halo(strength) => strength,
                };
                shader.set_shader_value(uniforms.strength, strength);
                shader.set_shader_value(uniforms.reach, HALO_REACH);
            } else if matches!(pass, GlyphPass::Halo(_)) {
                return;
            }
            d.draw_shader_mode(shader, |mut pass| {
                pass.draw_text_codepoint(text, codepoint as i32, pen, font_size, tint);
            });
        }
        None => {
            if matches!(pass, GlyphPass::Fill) {
                d.draw_text_codepoint(text, codepoint as i32, pen, font_size, tint);
            }
        }
    }
}

/// A word's layout slot for the current frame's boundary.
///
/// The C keeps these on `Cadence_Word` itself; here they are separate because the
/// word list is computed in `musializer-core`, which has no font metrics.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    x: f32,
    y: f32,
    width: f32,
    row: usize,
}

struct Layout {
    font_size: f32,
    spacing: f32,
    slots: Vec<Slot>,
}

fn measure<F: RaylibFont>(font: &F, text: &str, font_size: f32, spacing: f32) -> f32 {
    // A lyric cue with an interior NUL cannot be measured by a C string API; an
    // unmeasurable word gets zero width rather than aborting the frame.
    match CString::new(text) {
        Ok(_) => font.measure_text(text, font_size, spacing).x,
        Err(_) => 0.0,
    }
}

/// `cadence_layout` (`scene_cadence.c:131-182`).
///
/// Wraps the whole line into centered rows so every word owns a stable slot for the
/// duration of the cue, shrinking the type until the block fits.
///
/// Every returned slot is measured at the returned size, including when an
/// oversized cue exhausts the fitting attempts. Never shrink after the final
/// measurement: that moves the type away from the particle targets (SX4).
fn layout<F: RaylibFont>(
    words: &[Word<'_>],
    font: &F,
    boundary: Rectangle,
    scale: f32,
    spacing_scale: f32,
) -> Layout {
    let mut font_size = 10.0f32.max(boundary.height * 0.20 * scale);
    let max_width = boundary.width * 0.84;
    let max_height = boundary.height * 0.76;
    let mut spacing = font_size * 0.03 * spacing_scale;
    let mut line_advance = font_size * 1.16;
    let mut rows = 1usize;
    let mut slots = vec![Slot::default(); words.len()];

    for attempt in 0..LAYOUT_ATTEMPTS {
        spacing = font_size * 0.03 * spacing_scale;
        line_advance = font_size * 1.16;
        let space_width = font_size * 0.34;
        let mut cursor = 0.0f32;
        rows = 1;
        let mut fits = true;
        for (i, word) in words.iter().enumerate() {
            slots[i].width = measure(font, word.text, font_size, spacing);
            if slots[i].width > max_width {
                fits = false;
            }
            if cursor > 0.0 && cursor + space_width + slots[i].width > max_width {
                rows += 1;
                cursor = 0.0;
            }
            slots[i].x = if cursor > 0.0 {
                cursor + space_width
            } else {
                0.0
            };
            slots[i].row = rows - 1;
            cursor = slots[i].x + slots[i].width;
        }
        if fits && rows <= MAX_LAYOUT_ROWS && rows as f32 * line_advance <= max_height {
            break;
        }
        if attempt + 1 < LAYOUT_ATTEMPTS {
            font_size *= 0.82;
        }
    }

    let mut row_extent = [0.0f32; MAX_LAYOUT_ROWS];
    for slot in &mut slots {
        if slot.row >= MAX_LAYOUT_ROWS {
            slot.row = MAX_LAYOUT_ROWS - 1;
        }
        let extent = slot.x + slot.width;
        if extent > row_extent[slot.row] {
            row_extent[slot.row] = extent;
        }
    }
    let used_rows = rows.min(MAX_LAYOUT_ROWS);
    let block_top = boundary.y + (boundary.height - used_rows as f32 * line_advance) * 0.5;
    for slot in &mut slots {
        slot.x += boundary.x + (boundary.width - row_extent[slot.row]) * 0.5;
        slot.y = block_top + slot.row as f32 * line_advance;
    }

    Layout {
        font_size,
        spacing,
        slots,
    }
}

/// `cadence_glyph_alpha_at` (`:184-194`).
///
/// # Safety
/// `image` must be a live raylib `Image` whose `data` covers
/// `width * height * bytes_per_pixel` bytes, and `(x, y)` must be inside it. Both
/// hold because the only caller checks `data`, `width` and `height` first and clamps
/// its coordinates into range.
unsafe fn glyph_alpha_at(image: raylib_sys::Image, x: i32, y: i32) -> f32 {
    let data = image.data.cast::<u8>();
    let index = y as usize * image.width as usize + x as usize;
    match image.format {
        f if f == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_GRAYSCALE as i32 => {
            f32::from(*data.add(index))
        }
        f if f == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_GRAY_ALPHA as i32 => {
            f32::from(*data.add(index * 2 + 1))
        }
        f if f == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32 => {
            f32::from(*data.add(index * 4 + 3))
        }
        _ => 0.0,
    }
}

/// `cadence_glyph_ink` (`:200-235`).
///
/// Deterministically samples points inside the glyph's inked pixels — fonts loaded
/// from TTF retain CPU-side glyph bitmaps — so particles condense onto the letterform
/// itself rather than a bounding box. Falls back to the glyph cell centre when no
/// bitmap is available, which is what happens with an atlas-only font.
fn glyph_ink<F: RaylibFont>(
    font: &F,
    codepoint: char,
    seed: u64,
    salt: u64,
    want: usize,
    out: &mut [Vector2],
) -> usize {
    let want = want.min(out.len());
    let glyphs = font.chars();
    let info = if glyphs.is_empty() {
        None
    } else {
        let index = font.get_glyph_index(codepoint);
        // `GetGlyphIndex` returns a valid index (it falls back to '?' or 0), but the
        // bound check is cheap and this indexes a raw C array.
        usize::try_from(index).ok().and_then(|i| glyphs.get(i))
    };
    let (offset_x, offset_y, image) = match info {
        Some(info) => (info.offsetX, info.offsetY, Some(info.image)),
        None => (0, 0, None),
    };
    let sampled = image.filter(|image| {
        !image.data.is_null()
            && image.width > 0
            && image.height > 0
            && (image.format == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_GRAYSCALE as i32
                || image.format
                    == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_GRAY_ALPHA as i32
                || image.format
                    == raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32)
    });

    let (width, height) = image.map_or((0, 0), |image| (image.width, image.height));
    for (k, out) in out.iter_mut().enumerate().take(want) {
        let mut point = Vector2::new(
            offset_x as f32 + width as f32 * 0.5,
            offset_y as f32 + height as f32 * 0.5,
        );
        if let Some(image) = sampled {
            for probe in 0..INK_PROBES {
                let probe_salt = salt + (k * (INK_PROBES * 2)) as u64 + (probe * 2) as u64;
                let x = (cadence::unit(seed, probe_salt) * (image.width - 1) as f32 + 0.5) as i32;
                let y =
                    (cadence::unit(seed, probe_salt + 1) * (image.height - 1) as f32 + 0.5) as i32;
                let x = x.clamp(0, image.width - 1);
                let y = y.clamp(0, image.height - 1);
                // SAFETY: `sampled` proved `data` is non-null with a supported format
                // and positive dimensions, and the coordinates are clamped inside it.
                let alpha = unsafe { glyph_alpha_at(image, x, y) };
                if alpha > INK_ALPHA_THRESHOLD {
                    point = Vector2::new(
                        offset_x as f32 + x as f32 + 0.5,
                        offset_y as f32 + y as f32 + 0.5,
                    );
                    break;
                }
            }
        }
        *out = point;
    }
    want
}

/// The instrumental idle state, with no lyric line.
#[allow(clippy::too_many_arguments)]
fn draw_ambient(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &CadenceState,
    boundary: Rectangle,
    color: Color,
    swarm: f32,
    ribbons: f32,
    pixel_scale: f32,
) {
    // Continuous ink strata. The periodic beat term meets itself at the wrap;
    // musical time supplies drift, so neither seeking nor a beat resets a field.
    let t = frame.time_seconds;
    let extent = boundary.width.min(boundary.height);
    let sway = cadence::beat_sway(frame.audio.beat_phase);
    // The follower, not the frame's `rms`: a ribbon whose width tracks the raw
    // level trembles, and the lens shapes between two crossing ribbons then
    // pulse like bubbles.
    let energy = state.energy.clamp(0.0, 1.0);
    let layers = if ribbons > 0.005 { 4usize } else { 0 };
    let segments = 180usize;
    let cross_steps = 10usize;
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        for layer in 0..layers {
            let phase = (t * (0.10 + layer as f64 * 0.012)).rem_euclid(std::f64::consts::TAU)
                as f32
                + cadence::unit(state.seed, 900 + layer as u64) * 2.0 * PI;
            let mut mesh = Vec::with_capacity(segments * cross_steps * 6);
            let point = |i: usize, j: usize| {
                let u = i as f32 / segments as f32;
                let v = j as f32 / cross_steps as f32 * 2.0 - 1.0;
                let taper = (u * PI).sin().max(0.0).powf(1.2);
                let wave = (u * PI * (1.4 + layer as f32 * 0.22) + phase).sin();
                let y = boundary.y
                    + boundary.height * (0.37 + layer as f32 * 0.085)
                    + wave * extent * (0.07 + 0.03 * energy) * swarm
                    + (u * PI * 3.0 - phase * 0.4).cos() * extent * 0.015;
                let half_width = extent
                    * (0.009 + 0.018 * energy)
                    * taper
                    * swarm
                    * ribbons.max(1.0)
                    * (1.0 + 0.03 * sway);
                let alpha = (1.0 - v * v).max(0.0).powi(3) * taper * 0.19 * ribbons.min(1.0);
                (
                    Vector2::new(
                        boundary.x + boundary.width * (0.08 + u * 0.84),
                        y + v * half_width,
                    ),
                    draw::color_alpha(color, alpha),
                )
            };
            for i in 0..segments {
                for j in 0..cross_steps {
                    let a = point(i, j);
                    let b = point(i + 1, j);
                    let c = point(i, j + 1);
                    let e = point(i + 1, j + 1);
                    mesh.extend_from_slice(&[a, c, b, b, c, e]);
                }
            }
            draw::shaded_triangles(&mut blend, &mesh);
        }
        // A sparse, slowly traveling dust layer catches the light. Fixed count,
        // absolute-time paths, and a smooth visibility envelope avoid respawns.
        for i in 0..AMBIENT_PARTICLES as u64 / 3 {
            let phase = (t * 0.035 + f64::from(cadence::unit(state.seed, i * 3 + 1)) * 6.0)
                .rem_euclid(std::f64::consts::TAU) as f32;
            let radius = cadence::unit(state.seed, i * 3 + 2).sqrt() * extent * 0.36;
            let point = Vector2::new(
                boundary.x + boundary.width * 0.5 + phase.cos() * radius * 1.4,
                boundary.y + boundary.height * 0.51 + phase.sin() * radius * 0.60,
            );
            let alpha = (0.5 + 0.5 * (phase * 2.0 + i as f32).sin()).powi(2) * 0.32;
            blend.draw_circle_v(
                point,
                (0.65 + energy * 0.25) * pixel_scale,
                draw::color_alpha(color, alpha),
            );
        }
    });
}

/// The per-frame settings `draw_word` needs, bundled to keep its argument list
/// readable; the C passes them individually.
struct WordStyle {
    ink: Color,
    swarm: f32,
    beat_response: f32,
    glow: f32,
    halo: f32,
    reveal: Reveal,
    /// The fraction of the cue the line forms across; below 1 it then holds.
    reveal_span: f32,
    energy: f32,
    uniforms: Option<HaloUniforms>,
    pixel_scale: f32,
    particles_per_glyph: usize,
}

/// One glyph of a word, placed and posed for this frame.
struct PlacedGlyph {
    codepoint: char,
    /// The settled pen position; particles condense onto this.
    pen: Vector2,
    /// The pen position and size the pose draws at.
    drawn_pen: Vector2,
    drawn_size: f32,
    alpha: f32,
}

/// `cadence_draw_word` (`:288-397`).
#[allow(clippy::too_many_arguments)]
fn draw_word(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &CadenceState,
    faces: &mut CadenceType<'_>,
    word: &Word<'_>,
    slot: Slot,
    word_index: usize,
    layout: &Layout,
    boundary: Rectangle,
    focus: Focus,
    style: &WordStyle,
) {
    let font_size = layout.font_size;
    // The active flag is a label, not an opacity switch: hard 0.55/0.62
    // branches flashed at both ends of every estimated word window.
    let position = frame.lyric.map_or(0.0, |lyric| {
        cadence::cue_position(frame.time_seconds, lyric.start_seconds, lyric.end_seconds)
    });
    let span = (word.window_end - word.window_start).max(0.0001);
    let sung = cadence::smooth((position - word.window_start) / (span * 0.22))
        * (1.0 - cadence::smooth((position - word.window_end + span * 0.22) / (span * 0.22)));
    // A line that arrived early holds at full strength: without this every word
    // of a capped reveal would sit at its unemphasised 68 % for the rest of the
    // cue, which is most of it.
    let settled = if style.reveal_span < 1.0 {
        cadence::smooth((position - style.reveal_span) / (style.reveal_span * 0.35).max(0.0001))
    } else {
        0.0
    };
    let emphasis = sung.max(settled);
    // The beat coil: the swarm tightens toward the letterform approaching the beat
    // and relaxes after it. A raised cosine, so value and velocity both meet at
    // the wrap; the half-sine this replaces turned a corner there every beat.
    let beat = 0.5 - 0.5 * (frame.audio.beat_phase * 2.0 * PI).cos();
    let coil = 1.0 - style.beat_response * 0.15 * beat;
    let scatter_reach = boundary.width.min(boundary.height) * style.swarm * coil;
    let word_center = Vector2::new(slot.x + slot.width * 0.5, slot.y + font_size * 0.5);
    let edge_alpha = frame.lyric.map_or(1.0, |lyric| {
        cadence::smooth(((frame.time_seconds - lyric.start_seconds) / 0.16) as f32)
            * cadence::smooth(((lyric.end_seconds - frame.time_seconds) / 0.16) as f32)
    });
    let particle_alpha = (1.0 - cadence::smooth((focus.focus - 0.60) / 0.35)) * 0.88 * edge_alpha;
    let wants_particles =
        style.reveal == Reveal::Condense && focus.focus < 0.985 && particle_alpha > 0.01;

    // The lyric id seeds the whole word's particle field, so the same cue always
    // condenses the same way — including on an export pass.
    let lyric_id = frame.lyric.map_or(0, |lyric| lyric.id);
    // The trailing smear where there is one: a particle sized from the live
    // band flickers with it.
    let bands = if frame.audio.trails.is_empty() {
        frame.audio.bands
    } else {
        frame.audio.trails
    };

    let glyph_count = word.text.chars().count();
    let mut placed = Vec::with_capacity(glyph_count);
    let mut cursor = slot.x;
    for (glyph_index, codepoint) in word.text.chars().enumerate() {
        let mut encoded = [0u8; 4];
        let encoded = codepoint.encode_utf8(&mut encoded);
        let glyph_advance = measure(faces.text, encoded, font_size, 0.0);
        let pen = Vector2::new(cursor, slot.y);
        let GlyphPose { alpha, scale, lift } =
            cadence::glyph_pose(style.reveal, focus.focus, glyph_index, glyph_count);
        placed.push(PlacedGlyph {
            codepoint,
            pen,
            drawn_pen: Vector2::new(
                pen.x - (scale - 1.0) * glyph_advance * 0.5,
                pen.y - (scale - 1.0) * font_size * 0.5 + lift * font_size,
            ),
            drawn_size: font_size * scale,
            alpha: alpha * (0.68 + 0.32 * emphasis) * edge_alpha,
        });
        cursor += glyph_advance + layout.spacing;
    }

    // Light first, for the whole word, then the letterforms over it: a halo
    // drawn per glyph would lay the next letter's light across this one's ink.
    for (glyph_index, glyph) in placed.iter().enumerate() {
        // Read straight from the band table, unclamped, exactly as the C does.
        let band = if bands.is_empty() {
            0.0
        } else {
            bands[(word_index + glyph_index) % bands.len()]
        };
        let glyph_salt = lyric_id
            .wrapping_add(1)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add((word_index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d))
            .wrapping_add(glyph_index as u64);
        let pen = glyph.pen;

        if wants_particles && style.particles_per_glyph > 0 {
            let want = style.particles_per_glyph;
            let mut ink_points = [Vector2::zero(); PARTICLES_PER_GLYPH];
            let want = glyph_ink(
                faces.ink,
                glyph.codepoint,
                state.seed,
                glyph_salt.wrapping_mul(64),
                want,
                &mut ink_points,
            );
            // The *ink* face's base size, because `ink_points` are coordinates in
            // its glyph bitmaps. Scaling them by the text face's base size would
            // put the particles somewhere else entirely the moment the two atlases
            // differ, which is exactly what the SDF path makes possible.
            let glyph_scale = font_size
                / if faces.ink.base_size() > 0 {
                    faces.ink.base_size() as f32
                } else {
                    1.0
                };
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
                for (k, ink_point) in ink_points.iter().enumerate().take(want) {
                    let salt = glyph_salt.wrapping_mul(64).wrapping_add(40 + k as u64);
                    let target = Vector2::new(
                        pen.x + ink_point.x * glyph_scale,
                        pen.y + ink_point.y * glyph_scale,
                    );
                    let angle = cadence::unit(state.seed, salt) * 2.0 * PI
                        + cadence::beat_sway(frame.audio.beat_phase) * 0.035;
                    let distance =
                        (0.10 + cadence::unit(state.seed, salt + 1) * 0.24) * scatter_reach;
                    let home = Vector2::new(
                        word_center.x + angle.cos() * distance,
                        word_center.y + angle.sin() * distance,
                    );
                    // Stagger arrivals so the word condenses organically instead of
                    // translating as one rigid clump.
                    let arrive = cadence::smooth(
                        focus.focus * 1.35 - cadence::unit(state.seed, salt + 2) * 0.35,
                    );
                    let mut position = Vector2::new(
                        home.x + (target.x - home.x) * arrive,
                        home.y + (target.y - home.y) * arrive,
                    );
                    let jitter = (1.0 - arrive) * (1.5 + band * 7.0) * style.pixel_scale;
                    position.x +=
                        (frame.time_seconds as f32 * 2.7 + (k + glyph_index) as f32).sin() * jitter;
                    position.y +=
                        (frame.time_seconds as f32 * 2.1 + (k + glyph_index) as f32 * 1.3).cos()
                            * jitter;
                    let size = (1.1 + band * 2.0)
                        * (1.35 - 0.55 * arrive)
                        * style.pixel_scale
                        * style.glow.max(0.25);
                    blend.draw_circle_v(
                        position,
                        size,
                        draw::color_alpha(style.ink, particle_alpha * (0.35 + 0.65 * arrive)),
                    );
                }
            });
        }

        if glyph.alpha > 0.01 && style.halo > 0.005 {
            if style.uniforms.is_some() {
                // The halo breathes with the track and lifts on the beat. Both
                // terms are smooth, so it swells rather than flashes.
                let lift = 1.0 + 0.55 * style.energy + 0.35 * style.beat_response * (1.0 - beat);
                let strength = 0.36 * style.halo * lift;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
                    draw_glyph(
                        &mut blend,
                        faces,
                        style.uniforms,
                        GlyphPass::Halo(strength),
                        glyph.codepoint,
                        glyph.drawn_pen,
                        glyph.drawn_size,
                        draw::color_alpha(style.ink, glyph.alpha * 0.55),
                    );
                });
            } else if emphasis > 0.001 {
                // The bitmap fallback has no field to read a halo from, so it
                // keeps the C's single additive ghost, offset to the left.
                let ghost = Vector2::new(
                    glyph.drawn_pen.x - 1.5 * style.pixel_scale,
                    glyph.drawn_pen.y,
                );
                let ghost_tint =
                    draw::color_alpha(style.ink, 0.16 * style.halo * focus.focus * emphasis);
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
                    draw_glyph(
                        &mut blend,
                        faces,
                        None,
                        GlyphPass::Fill,
                        glyph.codepoint,
                        ghost,
                        glyph.drawn_size,
                        ghost_tint,
                    );
                });
            }
        }
    }

    for glyph in &placed {
        if glyph.alpha > 0.01 {
            draw_glyph(
                d,
                faces,
                style.uniforms,
                GlyphPass::Fill,
                glyph.codepoint,
                glyph.drawn_pen,
                glyph.drawn_size,
                draw::color_alpha(style.ink, glyph.alpha),
            );
        }
    }
}

/// Draws Cadence into `boundary`.
///
/// `faces.text` is the caption face to typeset with; pass the default font when
/// the project's imported face is absent, which is the fallback the C makes at
/// `scene_cadence.c:449`. See [`font_is_loaded`] and [`CadenceType`].
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &CadenceState,
    faces: &mut CadenceType<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }

    let scale = frame.setting(SceneId::Cadence, setting::SCALE);
    let swarm = frame.setting(SceneId::Cadence, setting::SWARM);
    let focus_speed = frame.setting(SceneId::Cadence, setting::FOCUS);
    let beat_response = frame.setting(SceneId::Cadence, setting::BEAT);
    let glow = frame.setting(SceneId::Cadence, setting::GLOW);
    let spacing_scale = frame.setting(SceneId::Cadence, setting::SPACING);
    let hue_swing = frame.setting(SceneId::Cadence, setting::HUE_SWING);
    let hue_offset = frame.setting(SceneId::Cadence, setting::HUE);
    let saturation = frame.setting(SceneId::Cadence, setting::SATURATION);
    let halo = frame.setting(SceneId::Cadence, setting::HALO);
    let reveal = Reveal::from_setting(frame.setting(SceneId::Cadence, setting::REVEAL));
    let ribbons = frame.setting(SceneId::Cadence, setting::RIBBONS);
    let energy = state.energy.clamp(0.0, 1.0);
    let pixel_scale = if pixel_scale > 0.0 { pixel_scale } else { 1.0 };

    let semantic_weight = if frame.semantic.available {
        cadence::clamp01(frame.semantic.confidence)
    } else {
        0.0
    };
    // `hue` turns the whole palette; the ink stays the background's
    // near-complement, so type is legible wherever the wheel is turned to. The
    // C also leaned the hue on the frame's raw spectral flux, which is a
    // different number every tick and read as the picture shivering.
    let hue =
        (205.0 + hue_offset + frame.semantic.valence * hue_swing * semantic_weight + 720.0) % 360.0;
    let chroma = |base: f32| (base * saturation).clamp(0.0, 1.0);
    let background = draw::color_from_hsv(hue, chroma(0.60), 0.030 + energy * 0.030);
    let ink = draw::color_from_hsv(
        (hue + 118.0) % 360.0,
        chroma(0.48 + frame.semantic.tension * 0.28 * semantic_weight),
        0.94,
    );
    draw::atmospheric_backdrop(
        d,
        boundary,
        background,
        draw::color_from_hsv((hue + 24.0) % 360.0, chroma(0.56), 0.076 + energy * 0.05),
        Vector2::new(
            boundary.x + boundary.width * 0.48,
            boundary.y + boundary.height * 0.52,
        ),
        boundary.width.max(boundary.height) * 0.52,
        draw::color_alpha(ink, 0.13 + energy * 0.10),
    );

    draw_ambient(d, frame, state, boundary, ink, swarm, ribbons, pixel_scale);
    let Some(lyric) = frame.lyric.filter(|lyric| !lyric.text.is_empty()) else {
        draw::vignette(d, boundary, 0.22);
        return;
    };

    let (words, reveal_span) =
        cadence::words_for_timed_cue(&lyric.text, lyric.end_seconds - lyric.start_seconds);
    if words.is_empty() {
        return;
    }

    let cue_position =
        cadence::cue_position(frame.time_seconds, lyric.start_seconds, lyric.end_seconds);
    // The whole line loosens back into particles over the cue's final beats.
    let line_hold = timing::line_hold(cue_position);
    let layout = layout(&words, faces.text, boundary, scale, spacing_scale);

    let uniforms = faces.sdf.as_ref().map(|shader| HaloUniforms {
        strength: shader.get_shader_location("halo"),
        reach: shader.get_shader_location("haloReach"),
    });
    let style = WordStyle {
        ink,
        swarm,
        beat_response,
        glow,
        halo,
        reveal,
        reveal_span,
        energy,
        uniforms,
        pixel_scale,
        particles_per_glyph: (PARTICLE_BUDGET
            / words.iter().map(|w| w.glyphs).sum::<usize>().max(1))
        .min(PARTICLES_PER_GLYPH),
    };
    for (i, word) in words.iter().enumerate() {
        let focus = cadence::resolved_focus(
            word,
            cue_position,
            focus_speed,
            frame.audio.onset,
            line_hold,
        );
        draw_word(
            d,
            frame,
            state,
            faces,
            word,
            layout.slots[i],
            i,
            &layout,
            boundary,
            focus,
            &style,
        );
    }
}
