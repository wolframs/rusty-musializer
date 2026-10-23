//! Neon Tunnel: one bounded GPU pass; deterministic state owns every moving input.

use musializer_core::scene::settings::index::tunnel as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::neon_tunnel::NeonTunnelState;
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/neon_tunnel.fs");

pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib may return its valid default shader after a compile failure.
        // A scene-specific live uniform distinguishes that fallback.
        let valid = shader.is_shader_valid() && shader.get_shader_location("tunnelTravel") >= 0;
        Self {
            shader: valid.then_some(shader),
            report: if valid {
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
    let Some(shader) = &mut resources.shader else {
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
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("tunnelTravel", state.distance());
    set("seedPhase", state.phase());
    set("twist", frame.setting(SceneId::NeonTunnel, control::TWIST));
    set("sides", frame.setting(SceneId::NeonTunnel, control::SIDES));
    set("hueTurn", frame.setting(SceneId::NeonTunnel, control::HUE));
    set("glow", frame.setting(SceneId::NeonTunnel, control::GLOW));
    set("depth", frame.setting(SceneId::NeonTunnel, control::DEPTH));
    set(
        "aperture",
        frame.setting(SceneId::NeonTunnel, control::WIDTH),
    );
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(location, Vector2::new(boundary.width, boundary.height));
    let location = shader.get_shader_location("tunnelMusic");
    let music = state.music();
    shader.set_shader_value(
        location,
        Vector4::new(music[0], music[1], music[2], music[3]),
    );
    let texture = draw::default_texture();
    d.draw_shader_mode(shader, |mut pass| {
        draw::draw_texture_pro(
            &mut pass,
            texture,
            Rectangle::new(0.0, 0.0, 1.0, 1.0),
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    resources.report = format!(
        "shader=drawn, distance={:.3}, bass={:.3}, loudness={:.3}, mids={:.3}, treble={:.3}, dynamics={}",
        state.distance(),
        music[0],
        music[1],
        music[2],
        music[3],
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
