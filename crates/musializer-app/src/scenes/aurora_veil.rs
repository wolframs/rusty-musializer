//! Aurora Veil: one bounded GPU pass; deterministic state owns every moving input.

use musializer_core::scene::settings::index::aurora as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::aurora_veil::AuroraVeilState;
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/aurora_veil.fs");

pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib may return its valid default shader after a compile failure.
        // A scene-specific live uniform distinguishes that fallback.
        let valid = shader.is_shader_valid() && shader.get_shader_location("auroraTime") >= 0;
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
    state: &AuroraVeilState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some(shader) = &mut resources.shader else {
        d.draw_rectangle_rec(boundary, Color::new(8, 9, 20, 255));
        d.draw_text(
            "Aurora Veil: shader unavailable",
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
    set("auroraTime", state.clock());
    set("seedPhase", state.phase());
    set(
        "curtainHeight",
        frame.setting(SceneId::AuroraVeil, control::HEIGHT),
    );
    set("folds", frame.setting(SceneId::AuroraVeil, control::FOLDS));
    set("hueTurn", frame.setting(SceneId::AuroraVeil, control::HUE));
    set("glow", frame.setting(SceneId::AuroraVeil, control::GLOW));
    set("stars", frame.setting(SceneId::AuroraVeil, control::STARS));
    set(
        "spread",
        frame.setting(SceneId::AuroraVeil, control::SPREAD),
    );
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(location, Vector2::new(boundary.width, boundary.height));
    let location = shader.get_shader_location("auroraMusic");
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
        "shader=drawn, clock={:.3}, bass={:.3}, loudness={:.3}, mids={:.3}, treble={:.3}, dynamics={}",
        state.clock(),
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
