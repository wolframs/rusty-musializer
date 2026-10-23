//! Iridescent Pool's stateless GPU material. All motion comes from core state.
use musializer_core::scene::settings::index::iridescent as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::iridescent_pool::IridescentPoolState;
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/iridescent_pool.fs");

pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid() && shader.get_shader_location("filmSpread") >= 0;
        Self {
            shader: valid.then_some(shader),
            report: if valid {
                "shader=ready, not drawn".into()
            } else {
                "shader=REFUSED, not drawn".into()
            },
        }
    }
}

pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &IridescentPoolState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some(shader) = resources.shader.as_mut() else {
        d.draw_rectangle_rec(boundary, Color::new(8, 12, 18, 255));
        d.draw_text(
            "Iridescent Pool: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "shader=REFUSED, fallback=unavailable notice".into();
        return;
    };
    let setting = |i| frame.setting(SceneId::IridescentPool, i);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("materialTime", state.clock());
    set("surfaceScale", setting(control::SCALE));
    set("rippleAmount", setting(control::RIPPLE));
    set("hueTurn", setting(control::HUE).to_radians());
    set("shimmer", setting(control::SHIMMER));
    set("poolLight", setting(control::LIGHT));
    set("filmSpread", setting(control::SPREAD));
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(
        location,
        Vector2::new(boundary.width.max(1.0), boundary.height.max(1.0)),
    );
    let location = shader.get_shader_location("drive");
    let drive = state.drive();
    shader.set_shader_value(location, Vector2::new(drive[0], drive[1]));
    let location = shader.get_shader_location("seedPhase");
    shader.set_shader_value(location, state.phase());
    for (i, ring) in state.ripples().iter().enumerate() {
        let location = shader.get_shader_location(&format!("ring{i}"));
        shader.set_shader_value(
            location,
            Vector4::new(ring.origin[0], ring.origin[1], ring.age, ring.strength),
        );
    }
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
        "shader=drawn, material=iridescent, clock={:.3}, drive={:?}, dynamics={}",
        state.clock(),
        state.drive(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
