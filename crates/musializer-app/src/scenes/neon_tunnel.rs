//! Neon Tunnel: one bounded GPU pass over a small spectrum-history texture.
//!
//! The state owns the history (`core::scenes::neon_tunnel`); this module only
//! copies it into an RGBA8 texture each draw, newest row first, and binds it
//! as the quad's own texture so the shader reads it as `texture0`. Nothing is
//! kept between draws, so a still needs no drawing warmup and an export cannot
//! inherit a preview's frames.

use musializer_core::scene::settings::index::tunnel as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::neon_tunnel::{
    NeonTunnelState, HISTORY_BANDS, HISTORY_CHANNELS, HISTORY_SLOTS,
};
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/neon_tunnel.fs");

pub struct Resources {
    shader: Option<Shader>,
    history: Option<Texture2D>,
    pixels: Vec<u8>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib may return its valid default shader after a compile failure.
        // A scene-specific live uniform distinguishes that fallback.
        let valid = shader.is_shader_valid() && shader.get_shader_location("tunnelTravel") >= 0;
        let image =
            Image::gen_image_color(HISTORY_CHANNELS as i32, HISTORY_SLOTS as i32, Color::BLACK);
        let history = rl.load_texture_from_image(thread, &image).ok();
        if let Some(texture) = &history {
            // Bilinear in time is the point (the wave moves continuously);
            // across bands it smooths the circumference. The shader samples
            // the flash and loudness columns at texel centres, so they never
            // blend into the bands.
            texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_BILINEAR);
            texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
        }
        let ready = valid && history.is_some();
        Self {
            shader: valid.then_some(shader),
            history,
            pixels: vec![0; HISTORY_CHANNELS * HISTORY_SLOTS * 4],
            report: if ready {
                "not drawn yet"
            } else {
                "shader=REFUSED"
            }
            .into(),
        }
    }
}

pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &NeonTunnelState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let (Some(shader), Some(history)) = (&mut resources.shader, &mut resources.history) else {
        d.draw_rectangle_rec(boundary, Color::new(8, 9, 20, 255));
        d.draw_text(
            "Neon Tunnel: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "shader=REFUSED, fallback=labelled unavailable scene".into();
        return;
    };
    for age in 0..HISTORY_SLOTS {
        let slot = state.history_slot(age);
        let row = &mut resources.pixels[age * HISTORY_CHANNELS * 4..][..HISTORY_CHANNELS * 4];
        for (pixel, value) in row.chunks_exact_mut(4).zip(slot) {
            let byte = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixel.copy_from_slice(&[byte, byte, byte, 255]);
        }
    }
    // Every earlier use of this texture ended its shader mode, which flushes
    // raylib's batch, so no queued geometry can observe the upload.
    if let Err(error) = history.update_texture(&resources.pixels) {
        resources.report = format!("REFUSED: history upload {error:?}");
        return;
    }
    let setting = |index| frame.setting(SceneId::NeonTunnel, index);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("tunnelTravel", state.distance());
    set("seedPhase", state.phase());
    set("historyLag", state.history_lag());
    // Grain only: a tick of track time, never a frame counter.
    set("trackTime", (frame.time_seconds % 3600.0) as f32);
    set("twist", setting(control::TWIST));
    set("sides", setting(control::SIDES));
    set("hueTurn", setting(control::HUE));
    set("glow", setting(control::GLOW));
    set("depth", setting(control::DEPTH));
    set("aperture", setting(control::WIDTH));
    set("response", setting(control::RESPONSE));
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(location, Vector2::new(boundary.width, boundary.height));
    let music = state.music();
    let location = shader.get_shader_location("tunnelMusic");
    shader.set_shader_value(
        location,
        Vector4::new(music[0], music[1], music[2], music[3]),
    );
    let accents = state.accents();
    let location = shader.get_shader_location("accents");
    shader.set_shader_value(location, Vector3::new(accents[0], accents[1], accents[2]));
    d.draw_shader_mode(shader, |mut pass| {
        pass.draw_texture_pro(
            &*history,
            Rectangle::new(0.0, 0.0, HISTORY_CHANNELS as f32, HISTORY_SLOTS as f32),
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    let floor = state.history_slot(0)[0];
    let ceiling = state.history_slot(0)[HISTORY_BANDS - 1];
    resources.report = format!(
        "shader=drawn, distance={:.3}, bass={:.3}, loudness={:.3}, mids={:.3}, treble={:.3}, \
         surge={:.3}, punch={:.3}, cruise={:.3}, kicks={}, history=floor {:.2}/ceiling {:.2}, \
         dynamics={}",
        state.distance(),
        music[0],
        music[1],
        music[2],
        music[3],
        accents[0],
        accents[1],
        state.cruise(),
        state.kicks(),
        floor,
        ceiling,
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
