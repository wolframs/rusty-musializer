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
    self as cadence, timing, CadenceState, Focus, Word, AMBIENT_PARTICLES, INK_ALPHA_THRESHOLD,
    INK_PROBES, LAYOUT_ATTEMPTS, MAX_LAYOUT_ROWS, PARTICLES_PER_GLYPH, PARTICLE_BUDGET,
};
use musializer_runtime::draw;
use musializer_runtime::font::Face;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibFont,
    RaylibShaderModeExt, Rectangle, Vector2,
};
use raylib::shaders::Shader;

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

/// One glyph, through the SDF shader when there is one.
///
/// The shader is entered per glyph rather than once around the word because the
/// particle swarm is drawn between glyphs and must not go through it: the
/// distance-field threshold would be applied to raylib's 1x1 white shape texture,
/// which is a different picture. A word is tens of glyphs, so this is tens of
/// batch flushes per frame — measured against a scene that already draws hundreds
/// of additive circles, it does not register.
fn draw_glyph<D: RaylibDraw>(
    d: &mut D,
    faces: &mut CadenceType<'_>,
    codepoint: char,
    pen: Vector2,
    font_size: f32,
    tint: Color,
) {
    let CadenceType { text, sdf, .. } = faces;
    let text: &Face = text;
    match sdf.as_deref_mut() {
        Some(shader) => d.draw_shader_mode(shader, |mut pass| {
            pass.draw_text_codepoint(text, codepoint as i32, pen, font_size, tint);
        }),
        None => d.draw_text_codepoint(text, codepoint as i32, pen, font_size, tint),
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
/// One oracle quirk is reproduced rather than corrected: on the sixth *failed*
/// attempt the loop shrinks `font_size` once more and then exits, so the returned
/// size is 0.82x smaller than the size the slot widths were measured at. A line that
/// cannot be made to fit is therefore drawn slightly narrower than its layout
/// assumes. It is only reachable with a pathological cue, and matching the oracle
/// matters more than tidying it.
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

    for _ in 0..LAYOUT_ATTEMPTS {
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
        font_size *= 0.82;
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
fn draw_ambient(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &CadenceState,
    boundary: Rectangle,
    color: Color,
    swarm: f32,
    pixel_scale: f32,
) {
    let phase = frame.audio.beat_phase * 2.0 * PI;
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        // With no authored words there is still a typographic *field*: broad,
        // slowly breathing strokes suggest ink waiting to condense instead of
        // leaving ninety-six pinpricks in an almost black frame. Nothing here
        // invents text; the actual cue remains the only source of letterforms.
        let extent = boundary.width.min(boundary.height);
        for ribbon in 0..4 {
            let ribbon_phase = frame.time_seconds as f32 * (0.11 + ribbon as f32 * 0.018)
                + cadence::unit(state.seed, 900 + ribbon) * 2.0 * PI;
            let mut previous = Vector2::zero();
            for segment in 0..=96 {
                let t = segment as f32 / 96.0;
                let x = boundary.x + boundary.width * (0.12 + t * 0.76);
                let wave = (t * PI * (1.5 + ribbon as f32 * 0.34) + ribbon_phase).sin();
                let counter = (t * PI * 3.0 - ribbon_phase * 0.63).cos();
                let y = boundary.y
                    + boundary.height * (0.36 + ribbon as f32 * 0.085)
                    + wave * extent * (0.055 + frame.audio.rms * 0.055)
                    + counter * extent * 0.018;
                let point = Vector2::new(x, y);
                if segment > 0 {
                    let taper = (t * PI).sin().max(0.0);
                    blend.draw_line_ex(
                        previous,
                        point,
                        (1.2 + taper * (2.8 + frame.audio.rms * 3.8)) * pixel_scale * swarm,
                        draw::color_alpha(color, (0.13 + taper * 0.17) * swarm),
                    );
                }
                previous = point;
            }
        }
        for i in 0..AMBIENT_PARTICLES as u64 {
            let angle = cadence::unit(state.seed, i * 3 + 1) * 2.0 * PI + phase * 0.12;
            let radius = cadence::unit(state.seed, i * 3 + 2).sqrt()
                * boundary.width.min(boundary.height)
                * 0.38;
            let breathing =
                0.84 + 0.16 * (frame.time_seconds as f32 * 0.43 + i as f32 * 0.31).sin();
            let point = Vector2::new(
                boundary.x + boundary.width * 0.5 + angle.cos() * radius * breathing,
                boundary.y + boundary.height * 0.5 + angle.sin() * radius * breathing,
            );
            let size = (0.9 + cadence::unit(state.seed, i * 3 + 3) * 2.2 + frame.audio.rms * 3.0)
                * pixel_scale
                * swarm;
            blend.draw_circle_v(
                point,
                size,
                draw::color_alpha(color, 0.34 + frame.audio.rms * 0.32),
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
    pixel_scale: f32,
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
    particle_budget: &mut usize,
) {
    let font_size = layout.font_size;
    let ink_alpha = if focus.active {
        0.55 + focus.focus * 0.45
    } else if focus.focus >= 1.0 {
        0.62
    } else {
        cadence::smooth((focus.focus - 0.55) / 0.45) * 0.5
    };
    // The beat coil: the swarm tightens toward the letterform approaching the beat
    // and relaxes just after the phase wraps.
    let coil = 1.0 - style.beat_response * 0.15 * (frame.audio.beat_phase * PI).sin();
    let scatter_reach = boundary.width.min(boundary.height) * style.swarm * coil;
    let word_center = Vector2::new(slot.x + slot.width * 0.5, slot.y + font_size * 0.5);
    let particle_alpha = (1.0 - cadence::smooth((focus.focus - 0.60) / 0.35)) * 0.88;
    let wants_particles = focus.focus < 0.985 && particle_alpha > 0.01;

    // The lyric id seeds the whole word's particle field, so the same cue always
    // condenses the same way — including on an export pass.
    let lyric_id = frame.lyric.map_or(0, |lyric| lyric.id);
    let bands = frame.audio.bands;

    let mut cursor = slot.x;
    for (glyph_index, codepoint) in word.text.chars().enumerate() {
        let mut encoded = [0u8; 4];
        let encoded = codepoint.encode_utf8(&mut encoded);
        let glyph_advance = measure(faces.text, encoded, font_size, 0.0);
        let pen = Vector2::new(cursor, slot.y);
        // Read straight from `bands`, unclamped, exactly as the C does.
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

        if wants_particles && *particle_budget > 0 {
            // Long words get fewer particles per glyph so a long line does not cost
            // more than a short one; the global budget is the hard ceiling.
            let want = 320usize
                .checked_div(word.glyphs)
                .unwrap_or(PARTICLES_PER_GLYPH)
                .clamp(6, PARTICLES_PER_GLYPH)
                .min(*particle_budget);
            let mut ink_points = [Vector2::zero(); PARTICLES_PER_GLYPH];
            let want = glyph_ink(
                faces.ink,
                codepoint,
                state.seed,
                glyph_salt.wrapping_mul(64),
                want,
                &mut ink_points,
            );
            *particle_budget -= want;
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
                    let angle =
                        cadence::unit(state.seed, salt) * 2.0 * PI + frame.audio.beat_phase * 0.4;
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

        if ink_alpha > 0.01 {
            if focus.active && style.glow > 0.01 {
                // A single additive ghost offset to the left, which reads as a bloom
                // around the stroke rather than as a second glyph.
                let ghost = Vector2::new(pen.x - 1.5 * style.pixel_scale, pen.y);
                let ghost_tint = draw::color_alpha(style.ink, 0.16 * style.glow * focus.focus);
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
                    draw_glyph(&mut blend, faces, codepoint, ghost, font_size, ghost_tint);
                });
            }
            draw_glyph(
                d,
                faces,
                codepoint,
                pen,
                font_size,
                draw::color_alpha(style.ink, ink_alpha),
            );
        }
        cursor += glyph_advance + layout.spacing;
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
    let pixel_scale = if pixel_scale > 0.0 { pixel_scale } else { 1.0 };

    let semantic_weight = if frame.semantic.available {
        cadence::clamp01(frame.semantic.confidence)
    } else {
        0.0
    };
    let hue = (205.0
        + frame.semantic.valence * hue_swing * semantic_weight
        + frame.audio.spectral_flux * 72.0
        + 360.0)
        % 360.0;
    let background = draw::color_from_hsv(hue, 0.60, 0.030 + frame.audio.rms * 0.030);
    // The ink is the hue's near-complement, so type stays legible against the
    // background whatever the semantic lane does to the hue.
    let ink = draw::color_from_hsv(
        (hue + 118.0) % 360.0,
        0.48 + frame.semantic.tension * 0.28 * semantic_weight,
        0.94,
    );
    draw::atmospheric_backdrop(
        d,
        boundary,
        background,
        draw::color_from_hsv((hue + 24.0) % 360.0, 0.56, 0.076 + frame.audio.rms * 0.05),
        Vector2::new(
            boundary.x + boundary.width * 0.48,
            boundary.y + boundary.height * 0.52,
        ),
        boundary.width.max(boundary.height) * 0.52,
        draw::color_alpha(ink, 0.13 + frame.audio.rms * 0.10),
    );

    let Some(lyric) = frame.lyric.filter(|lyric| !lyric.text.is_empty()) else {
        draw_ambient(d, frame, state, boundary, ink, swarm, pixel_scale);
        draw::vignette(d, boundary, 0.22);
        return;
    };

    let words = cadence::words_for_cue(&lyric.text);
    if words.is_empty() {
        return;
    }

    let cue_position =
        cadence::cue_position(frame.time_seconds, lyric.start_seconds, lyric.end_seconds);
    // The whole line loosens back into particles over the cue's final beats.
    let line_hold = timing::line_hold(cue_position);
    let layout = layout(&words, faces.text, boundary, scale, spacing_scale);

    let style = WordStyle {
        ink,
        swarm,
        beat_response,
        glow,
        pixel_scale,
    };
    let mut particle_budget = PARTICLE_BUDGET;
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
            &mut particle_budget,
        );
    }
}
