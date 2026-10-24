//! Aurora Veil: one bounded GPU pass; deterministic state owns every moving input.

use musializer_core::scene::settings::index::aurora as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::aurora_veil::{AuroraVeilState, PULSE_SECONDS};
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
    let setting = |index| frame.setting(SceneId::AuroraVeil, index);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("auroraTime", state.clock());
    set("seedPhase", state.phase());
    set("shimmer", state.shimmer());
    // Grain only: a tick of track time, never a frame counter, so two renders
    // of one moment draw the same specks.
    set("trackTime", (frame.time_seconds % 3600.0) as f32);
    set("curtainHeight", setting(control::HEIGHT));
    set("folds", setting(control::FOLDS));
    set("hueTurn", setting(control::HUE));
    set("glow", setting(control::GLOW));
    set("stars", setting(control::STARS));
    set("spread", setting(control::SPREAD));
    set("response", setting(control::RESPONSE));
    let mut vec2 = |name: &str, value: Vector2| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    vec2("resolution", Vector2::new(boundary.width, boundary.height));
    let lift = state.lift();
    vec2("auroraLift", Vector2::new(lift[0], lift[1]));
    let meteor = state.meteor();
    vec2("meteor", Vector2::new(meteor[0], meteor[1]));
    let music = state.music();
    let pulses = state.pulses();
    let mut vec4 = |name: &str, value: Vector4| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    vec4(
        "auroraMusic",
        Vector4::new(music[0], music[1], music[2], music[3]),
    );
    vec4(
        "pulseAge",
        Vector4::new(pulses[0][0], pulses[1][0], pulses[2][0], pulses[3][0]),
    );
    vec4(
        "pulseStrength",
        Vector4::new(pulses[0][1], pulses[1][1], pulses[2][1], pulses[3][1]),
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
    let flaming = pulses
        .iter()
        .filter(|pulse| pulse[0] < PULSE_SECONDS && pulse[1] > 0.0)
        .count();
    resources.report = format!(
        "shader=drawn, clock={:.3}, bass={:.3}, activity={:.3}, mids={:.3}, treble={:.3}, \
         surge={:.3}, kicks={}, flaming={}, meteors={}, dynamics={}",
        state.clock(),
        music[0],
        music[1],
        music[2],
        music[3],
        lift[1],
        state.kicks(),
        flaming,
        state.meteors(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
