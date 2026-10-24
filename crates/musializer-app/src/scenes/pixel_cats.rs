//! Pixel Cats' drawing half: upload the CPU canvas, scale it up behind a CRT.
//!
//! Everything the picture is lives in `musializer_core::scenes::pixel_cats`,
//! which renders a canvas of about 180 rows into bytes. This file owns one
//! texture large enough for any canvas (allocated once, so a resize of the
//! preview never allocates on the GPU), uploads the drawn region into its top
//! left, and draws it integer-scaled and centred in the scene boundary through
//! `pixel_cats.fs`: nearest-neighbour always, with a scanline, an aperture grid
//! and a vignette when `crt` is up. The margin the integer factor leaves —
//! never more than one factor's worth of pixels — is black bezel.
//!
//! The six scenes share one set of these resources: only one of them draws in
//! a frame, and the texture's contents are rewritten before every draw.

use std::time::Instant;

use musializer_core::scene::settings::index::pixel as control;
use musializer_core::scene::SceneFrame;
use musializer_core::scene::SceneState;
use musializer_core::scenes::pixel_cats::{
    self, raster::Raster, PixelCatsState, PixelReport, MAX_CANVAS,
};
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/pixel_cats.fs");

pub struct PixelResources {
    texture: Option<Texture2D>,
    shader: Option<Shader>,
    raster: Raster,
    rgba: Vec<u8>,
    last: PixelReport,
    /// `shader`, `plain` (crt 0), `FALLBACK` (shader refused) or `REFUSED`
    /// (no texture: nothing but a labelled card can draw).
    path: &'static str,
    /// CPU time of the last canvas render, in microseconds. A measurement for
    /// the report, never an input to the picture.
    render_us: u128,
}

impl PixelResources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let image = Image::gen_image_color(MAX_CANVAS, MAX_CANVAS, Color::BLACK);
        let texture = rl.load_texture_from_image(thread, &image).ok();
        if let Some(texture) = &texture {
            // Nearest, always: a filtered pixel is not a pixel.
            texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_POINT);
            texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
        }
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib answers a failed compile with its default shader; a uniform
        // only this program has tells the two apart.
        let valid = shader.is_shader_valid() && shader.get_shader_location("crt") >= 0;
        Self {
            texture,
            shader: valid.then_some(shader),
            raster: Raster::new(16, 16),
            rgba: Vec::new(),
            last: PixelReport::default(),
            path: "none",
            render_us: 0,
        }
    }

    /// The `pixel cats:` report line: what the last frame staged and how it
    /// reached the screen. `none` before a Pixel Cats scene has drawn.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.last.staging.is_empty() {
            return "none".into();
        }
        format!(
            "{} crt={} cpu={:.2}ms",
            self.last.describe(),
            self.path,
            self.render_us as f64 / 1000.0
        )
    }
}

pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut PixelResources,
    state: &PixelCatsState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let scene = state.id();
    let target = frame.setting(scene, control::PIXELS);
    let crt = frame.setting(scene, control::CRT);
    let (cw, ch, factor) = pixel_cats::canvas_size(boundary.width, boundary.height, target);
    let started = Instant::now();
    let mut report = pixel_cats::render(
        state,
        frame,
        cw,
        ch,
        &mut resources.raster,
        &mut resources.rgba,
    );
    resources.render_us = started.elapsed().as_micros();
    report.canvas = (cw, ch, factor);

    d.draw_rectangle_rec(boundary, Color::BLACK);
    let Some(texture) = &mut resources.texture else {
        d.draw_text(
            "Pixel Cats: GPU texture unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.path = "REFUSED";
        resources.last = report;
        return;
    };
    if let Err(error) = texture.update_texture_rec(
        Rectangle::new(0.0, 0.0, cw as f32, ch as f32),
        &resources.rgba,
    ) {
        resources.path = "REFUSED";
        report.detail = format!("{} upload={error:?}", report.detail);
        resources.last = report;
        return;
    }
    // Integer scale, centred; whole output pixels so no column is doubled.
    let (dw, dh) = ((cw * factor) as f32, (ch * factor) as f32);
    let dest = Rectangle::new(
        (boundary.x + (boundary.width - dw) * 0.5).round(),
        (boundary.y + (boundary.height - dh) * 0.5).round(),
        dw,
        dh,
    );
    let source = Rectangle::new(0.0, 0.0, cw as f32, ch as f32);
    match (&mut resources.shader, crt > 0.0) {
        (Some(shader), true) => {
            let location = shader.get_shader_location("canvas");
            shader.set_shader_value(location, Vector2::new(cw as f32, ch as f32));
            let location = shader.get_shader_location("atlas");
            shader.set_shader_value(location, Vector2::new(MAX_CANVAS as f32, MAX_CANVAS as f32));
            let location = shader.get_shader_location("scale");
            shader.set_shader_value(location, factor as f32);
            let location = shader.get_shader_location("crt");
            shader.set_shader_value(location, crt);
            d.draw_shader_mode(shader, |mut pass| {
                pass.draw_texture_pro(&*texture, source, dest, Vector2::zero(), 0.0, Color::WHITE);
            });
            resources.path = "shader";
        }
        (shader, wants) => {
            d.draw_texture_pro(&*texture, source, dest, Vector2::zero(), 0.0, Color::WHITE);
            resources.path = if wants && shader.is_none() {
                "FALLBACK"
            } else {
                "plain"
            };
        }
    }
    resources.last = report;
}
