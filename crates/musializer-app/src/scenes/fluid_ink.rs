//! Fluid Ink's drawing half: upload deterministic dye, light it in one pass.
//!
//! The CPU solver owns all history. This resource is just a reusable RGBA8
//! texture and a display shader, so a still needs no drawing warmup and an
//! export cannot inherit an earlier preview's ink.

use musializer_core::scene::settings::index::fluid as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::fluid_ink::{FluidInkState, DYE_HEIGHT as HEIGHT, DYE_WIDTH as WIDTH};
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/fluid_ink.fs");

pub struct FluidResources {
    texture: Option<Texture2D>,
    shader: Option<Shader>,
    pixels: Vec<u8>,
    transfer: [u8; 4096],
    pub report: String,
}

impl FluidResources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let image = Image::gen_image_color(WIDTH as i32, HEIGHT as i32, Color::BLACK);
        let texture = rl.load_texture_from_image(thread, &image).ok();
        if let Some(texture) = &texture {
            texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_BILINEAR);
            texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
        }
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid() && shader.get_shader_location("inkGlow") >= 0;
        Self {
            texture,
            shader: valid.then_some(shader),
            pixels: vec![255; WIDTH * HEIGHT * 4],
            // A bounded transfer table removes nearly half a million exp calls
            // from every live frame. Its quantization is below one RGBA8 step.
            transfer: std::array::from_fn(|i| {
                ((1.0 - (-(i as f32 / 4095.0 * 5.0) * 0.65).exp()) * 255.0).round() as u8
            }),
            report: "not drawn yet".into(),
        }
    }
}

pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut FluidResources,
    state: &FluidInkState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let (Some(texture), Some(shader)) = (&mut resources.texture, &mut resources.shader) else {
        d.draw_rectangle_rec(boundary, Color::new(8, 10, 17, 255));
        d.draw_text(
            "Fluid Ink: GPU resources unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "REFUSED: texture or display shader unavailable".into();
        return;
    };
    for (pixel, dye) in resources.pixels.chunks_exact_mut(4).zip(state.dye()) {
        for c in 0..3 {
            pixel[c] =
                resources.transfer[(dye[c].clamp(0.0, 5.0) * (4095.0 / 5.0)).round() as usize];
        }
        pixel[3] = 255;
    }
    // Each earlier use of this texture ended its shader mode, which flushes
    // raylib's batch. No queued geometry can observe this upload retroactively.
    if let Err(error) = texture.update_texture(&resources.pixels) {
        resources.report = format!("REFUSED: dye upload {error:?}");
        return;
    }
    let setting = |i| frame.setting(SceneId::FluidInk, i);
    let location = shader.get_shader_location("inkGlow");
    shader.set_shader_value(location, setting(control::GLOW));
    let location = shader.get_shader_location("relief");
    shader.set_shader_value(location, setting(control::RELIEF));
    let location = shader.get_shader_location("hueTurn");
    shader.set_shader_value(location, setting(control::HUE).to_radians());
    let location = shader.get_shader_location("aspect");
    shader.set_shader_value(location, boundary.width / boundary.height.max(1.0));
    d.draw_shader_mode(shader, |mut pass| {
        pass.draw_texture_pro(
            texture,
            Rectangle::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    resources.report = format!(
        "solver=Rust, grid={}x{}, steps={}, dye=uploaded, display=shader, dynamics={}",
        WIDTH,
        HEIGHT,
        state.steps(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
