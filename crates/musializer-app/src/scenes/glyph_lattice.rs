//! Glyph Lattice's drawing half: upload the field, draw the honeycomb.
//!
//! The cells — which are lit, how hot, the ghosts, the bloom — are computed on
//! the CPU by `musializer_core::scenes::glyph_lattice::field`, a pure function
//! of the state, the frame and the boundary's aspect. This module packs them
//! into one RGBA8 atlas allocated once (two texels per cell), and draws the
//! boundary through `glyph_lattice.fs`, which turns each pixel's cell into a
//! glyph. Nothing is kept between draws but the buffers, so a still needs no
//! warmup and an export cannot inherit the preview's frames.

use std::time::Instant;

use musializer_core::scene::settings::index::glyphs as control;
use musializer_core::scene::{SceneFrame, SceneId, VoiceStyle};
use musializer_core::scenes::glyph_lattice::field::{
    self, Field, FieldReport, ATLAS_HEIGHT, ATLAS_WIDTH,
};
use musializer_core::scenes::glyph_lattice::GlyphLatticeState;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/glyph_lattice.fs");

/// The hue `glyph_lattice.fs` draws at `hue` 0, in degrees on the wheel.
const PALETTE_HUE: f32 = 174.0;

pub struct Resources {
    shader: Option<Shader>,
    atlas: Option<Texture2D>,
    field: Field,
    last: Option<FieldReport>,
    /// `drawn`, `REFUSED` (shader or texture), before anything drew `none`.
    path: &'static str,
    /// The last frame's music figures and pulse counters, for the report.
    figures: String,
    /// CPU time of the last field, in microseconds. A measurement for the
    /// report, never an input to the picture.
    field_us: u128,
    pitch_px: f32,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib answers a failed compile with its default shader; a uniform
        // only this program has tells the two apart.
        let valid = shader.is_shader_valid() && shader.get_shader_location("halfGrid") >= 0;
        let image = Image::gen_image_color(ATLAS_WIDTH as i32, ATLAS_HEIGHT as i32, Color::BLACK);
        let atlas = rl.load_texture_from_image(thread, &image).ok();
        if let Some(texture) = &atlas {
            // The shader reads exact texels and interpolates the bloom itself.
            texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_POINT);
            texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
        }
        Self {
            shader: valid.then_some(shader),
            atlas,
            field: Field::new(),
            last: None,
            path: "none",
            figures: String::new(),
            field_us: 0,
            pitch_px: 0.0,
        }
    }

    /// The `glyphs:` report line: what the last frame's lattice held. `none`
    /// before the scene has drawn.
    #[must_use]
    pub fn describe(&self) -> String {
        let Some(report) = &self.last else {
            return format!("shader={}", self.path);
        };
        let line = match &report.line {
            Some(text) => {
                let short: String = text.chars().take(24).collect();
                format!(
                    "\"{}{}\" strokes={}/{} {}",
                    short.replace('"', "'"),
                    if text.chars().count() > 24 { "…" } else { "" },
                    report.strokes_drawn,
                    report.strokes_total,
                    if report.line_live {
                        "live"
                    } else {
                        "dissolving"
                    }
                )
            }
            None => "none".into(),
        };
        format!(
            "shader={} form={}{} assembled={:.2} previous={} lattice={}x{} pitch={:.1}px \
             lit=dim {}/body {}/core {}/hot {} ghost={} echoes={} giants={} waves={} \
             ripples={} washed={} bloom={:.2} line={} {} cpu={:.2}ms",
            self.path,
            report.form,
            if report.auto { "(cycle)" } else { "" },
            report.assembled,
            report.previous.unwrap_or("none"),
            report.cols,
            report.rows,
            self.pitch_px,
            report.dim,
            report.mid,
            report.bright,
            report.hot,
            report.ghost,
            report.echoes,
            report.giants,
            report.waves,
            report.ripple_fronts.len(),
            report.washed,
            report.peak_bloom,
            line,
            self.figures,
            self.field_us as f64 / 1000.0,
        )
    }
}

pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &GlyphLatticeState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let started = Instant::now();
    let report = field::render(
        state,
        frame,
        boundary.width,
        boundary.height,
        &mut resources.field,
    );
    resources.field_us = started.elapsed().as_micros();
    let lattice = resources.field.lattice();
    let music = state.music();
    resources.figures = format!(
        "level={:.2} bass={:.2} treble={:.2} kicks={} pulses={} onsets={} dyn={}",
        music[0],
        music[1],
        music[3],
        state.kicks(),
        state.fired(),
        state.ripple_count(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "none"
        },
    );
    let short = boundary.width.min(boundary.height).max(1.0);
    resources.pitch_px = lattice.pitch * short;

    let (Some(shader), Some(atlas)) = (&mut resources.shader, &mut resources.atlas) else {
        d.draw_rectangle_rec(boundary, Color::new(4, 12, 12, 255));
        d.draw_text(
            "Glyph Lattice: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.path = "REFUSED";
        resources.last = Some(report);
        return;
    };
    let (cols, rows) = (lattice.cols(), lattice.rows());
    // Every earlier use of the atlas ended its shader mode, which flushes
    // raylib's batch, so no queued geometry can observe the upload.
    if let Err(error) = atlas.update_texture_rec(
        Rectangle::new(0.0, 0.0, (2 * cols) as f32, rows as f32),
        &resources.field.rgba,
    ) {
        resources.path = "REFUSED";
        resources.figures = format!("{} upload={error:?}", resources.figures);
        resources.last = Some(report);
        return;
    }
    let setting = |index| frame.setting(SceneId::GlyphLattice, index);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("pitch", lattice.pitch);
    set("trackTime", (frame.time_seconds % 3600.0) as f32);
    // A voiced line turns the palette to its voice's colour while it is set
    // (MV9-h); the lattice's own aqua sits at 174 degrees.
    let words = setting(control::WORDS) > 0.5;
    let voice = frame
        .lyric
        .filter(|_| words)
        .and_then(|cue| cue.voice.as_ref());
    set(
        "hueTurn",
        VoiceStyle::hue_or(voice, setting(control::HUE), PALETTE_HUE),
    );
    set("saturation", setting(control::SATURATION));
    set("glowAmount", setting(control::GLOW));
    set(
        "flicker",
        setting(control::FLICKER) * (0.4 + 0.9 * music[3]).min(1.3),
    );
    set("punch", state.punch());
    set(
        "aura",
        0.25 + 0.5 * music[1] + 0.25 * music[0] + 0.4 * state.punch(),
    );
    set("rippleCount", (state.ripple_count() % 65_536) as f32);
    set("seedSalt", (state.seed() % 65_536) as f32);
    let mut fronts = [-1.0f32; 8];
    for (slot, front) in fronts.iter_mut().zip(&report.ripple_fronts) {
        *slot = *front;
    }
    let mut vec2 = |name: &str, value: Vector2| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    vec2("resolution", Vector2::new(boundary.width, boundary.height));
    vec2("grid", Vector2::new(cols as f32, rows as f32));
    vec2(
        "halfGrid",
        Vector2::new(lattice.half_cols as f32, lattice.half_rows as f32),
    );
    vec2(
        "atlasSize",
        Vector2::new(ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32),
    );
    let location = shader.get_shader_location("ripplesA");
    shader.set_shader_value(
        location,
        Vector4::new(fronts[0], fronts[1], fronts[2], fronts[3]),
    );
    let location = shader.get_shader_location("ripplesB");
    shader.set_shader_value(
        location,
        Vector4::new(fronts[4], fronts[5], fronts[6], fronts[7]),
    );
    let source = Rectangle::new(0.0, 0.0, (2 * cols) as f32, rows as f32);
    d.draw_shader_mode(shader, |mut pass| {
        pass.draw_texture_pro(
            &*atlas,
            source,
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    resources.path = "drawn";
    resources.last = Some(report);
}
