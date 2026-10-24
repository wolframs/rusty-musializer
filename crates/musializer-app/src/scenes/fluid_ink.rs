//! Fluid Ink's drawing half: upload deterministic dye, light it in one pass.
//!
//! The CPU solver owns all history. This resource is a reusable float texture
//! and a display shader, so a still needs no drawing warmup and an export
//! cannot inherit an earlier preview's ink.
//!
//! The texture is an atlas: the full-resolution dye on top and a five-level
//! box pyramid of it in a strip underneath (`ATLAS_LEVELS`). Bloom, the light
//! shafts and the out-of-focus depth layer all read the pyramid, which is what
//! keeps the whole look to one texture, one sampler and one draw. A second
//! sampler would need an `unsafe` bind (`runtime::post::bind_sampler` is the
//! precedent), and GPU mipmaps would need `GenTextureMipmaps`, which logs a
//! line on every call — sixty a second.

use musializer_core::scene::settings::index::fluid as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::fluid_ink::{
    Dye, FluidInkState, DYE_HEIGHT as HEIGHT, DYE_WIDTH as WIDTH,
};
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/fluid_ink.fs");

const ATLAS_WIDTH: usize = WIDTH;
const ATLAS_HEIGHT: usize = HEIGHT + HEIGHT / 2;
/// Pyramid levels 1..=5 as (x, y) origins in the atlas; level `k` is
/// `WIDTH >> k` by `HEIGHT >> k`. The shader's `rectOf` restates this table.
const ATLAS_LEVELS: [(usize, usize); 5] = [
    (0, HEIGHT),
    (WIDTH / 2, HEIGHT),
    (WIDTH / 2 + WIDTH / 4, HEIGHT),
    (WIDTH / 2 + WIDTH / 4 + WIDTH / 8, HEIGHT),
    (WIDTH / 2 + WIDTH / 4 + WIDTH / 8 + WIDTH / 16, HEIGHT),
];

const UNIFORMS: [&str; 13] = [
    "inkGlow",
    "relief",
    "hueTurn",
    "aspect",
    "lightPos",
    "kick",
    "pressure",
    "level",
    "breath",
    "spray",
    "seconds",
    "resolution",
    "swirl",
];

pub struct FluidResources {
    texture: Option<Texture2D>,
    shader: Option<Shader>,
    locations: [i32; UNIFORMS.len()],
    atlas: Vec<Dye>,
    bytes: Vec<u8>,
    pub report: String,
}

impl FluidResources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let mut image =
            Image::gen_image_color(ATLAS_WIDTH as i32, ATLAS_HEIGHT as i32, Color::BLACK);
        // Float storage: the shader derives surface normals from the dye's
        // gradient, and 8-bit steps facet a gentle slope at 4x magnification.
        image.set_format(PixelFormat::PIXELFORMAT_UNCOMPRESSED_R32G32B32A32);
        let texture = rl.load_texture_from_image(thread, &image).ok();
        if let Some(texture) = &texture {
            texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_BILINEAR);
            texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
        }
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let locations = UNIFORMS.map(|name| shader.get_shader_location(name));
        let valid = shader.is_shader_valid() && locations[0] >= 0;
        Self {
            texture,
            shader: valid.then_some(shader),
            locations,
            atlas: vec![[0.0; 4]; ATLAS_WIDTH * ATLAS_HEIGHT],
            bytes: vec![0; ATLAS_WIDTH * ATLAS_HEIGHT * 16],
            report: "not drawn yet".into(),
        }
    }
}

/// Copies the dye into the atlas and box-filters the pyramid under it.
fn build_atlas(dye: &[Dye], atlas: &mut [Dye]) {
    for y in 0..HEIGHT {
        atlas[y * ATLAS_WIDTH..y * ATLAS_WIDTH + WIDTH]
            .copy_from_slice(&dye[y * WIDTH..(y + 1) * WIDTH]);
    }
    let (mut sx, mut sy, mut sw, mut sh) = (0, 0, WIDTH, HEIGHT);
    for &(ox, oy) in &ATLAS_LEVELS {
        let (w, h) = (sw / 2, sh / 2);
        for y in 0..h {
            for x in 0..w {
                let a = (sy + 2 * y) * ATLAS_WIDTH + sx + 2 * x;
                let b = a + ATLAS_WIDTH;
                let (p, q, r, s) = (atlas[a], atlas[a + 1], atlas[b], atlas[b + 1]);
                atlas[(oy + y) * ATLAS_WIDTH + ox + x] =
                    std::array::from_fn(|c| 0.25 * (p[c] + q[c] + r[c] + s[c]));
            }
        }
        (sx, sy, sw, sh) = (ox, oy, w, h);
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
    build_atlas(state.dye(), &mut resources.atlas);
    for (out, cell) in resources.bytes.chunks_exact_mut(16).zip(&resources.atlas) {
        for (slot, value) in out.chunks_exact_mut(4).zip(cell) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
    }
    // Each earlier use of this texture ended its shader mode, which flushes
    // raylib's batch. No queued geometry can observe this upload retroactively.
    if let Err(error) = texture.update_texture(&resources.bytes) {
        resources.report = format!("REFUSED: dye upload {error:?}");
        return;
    }
    let setting = |i| frame.setting(SceneId::FluidInk, i);
    let music = state.music();
    let light = state.light();
    let at = resources.locations;
    shader.set_shader_value(at[0], setting(control::GLOW));
    shader.set_shader_value(at[1], setting(control::RELIEF));
    shader.set_shader_value(at[2], setting(control::HUE).to_radians());
    shader.set_shader_value(at[3], boundary.width / boundary.height.max(1.0));
    shader.set_shader_value(at[4], Vector2::new(light[0], light[1]));
    shader.set_shader_value(at[5], music.kick);
    shader.set_shader_value(at[6], music.pressure);
    shader.set_shader_value(at[7], music.level);
    shader.set_shader_value(at[8], music.breath);
    shader.set_shader_value(at[9], music.spray);
    shader.set_shader_value(at[10], state.seconds());
    shader.set_shader_value(at[11], Vector2::new(boundary.width, boundary.height));
    shader.set_shader_value(at[12], music.swirl);
    d.draw_shader_mode(shader, |mut pass| {
        pass.draw_texture_pro(
            texture,
            Rectangle::new(0.0, 0.0, ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32),
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    resources.report = format!(
        "solver=Rust, grid={}x{}, steps={}, dye=uploaded, display=shader, dynamics={}, \
         kicks={}, sprays={}, pressure={:.2}, breath={:.2}, ink={:.0}%",
        WIDTH,
        HEIGHT,
        state.steps(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        },
        music.kicks,
        music.sprays,
        music.pressure,
        music.breath,
        state.ink_coverage() * 100.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pyramid_fits_its_strip_and_averages() {
        let (x, _) = ATLAS_LEVELS[4];
        assert!(x + (WIDTH >> 5) <= ATLAS_WIDTH);
        assert_eq!(ATLAS_HEIGHT - HEIGHT, HEIGHT >> 1);
        let mut dye = vec![[0.0; 4]; WIDTH * HEIGHT];
        dye[0] = [4.0, 0.0, 0.0, 8.0];
        let mut atlas = vec![[0.0; 4]; ATLAS_WIDTH * ATLAS_HEIGHT];
        build_atlas(&dye, &mut atlas);
        assert_eq!(atlas[0], dye[0]);
        assert_eq!(atlas[HEIGHT * ATLAS_WIDTH], [1.0, 0.0, 0.0, 2.0]);
        let (x2, y2) = ATLAS_LEVELS[1];
        assert_eq!(atlas[y2 * ATLAS_WIDTH + x2], [0.25, 0.0, 0.0, 0.5]);
    }
}
