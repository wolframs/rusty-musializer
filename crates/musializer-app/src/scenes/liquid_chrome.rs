//! Liquid Chrome's stateless GPU material. All motion comes from core state.
use musializer_core::scene::settings::index::chrome as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::liquid_chrome::LiquidChromeState;
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/liquid_chrome.fs");

pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid() && shader.get_shader_location("chromeShine") >= 0;
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
    state: &LiquidChromeState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some(shader) = resources.shader.as_mut() else {
        d.draw_rectangle_rec(boundary, Color::new(8, 12, 18, 255));
        d.draw_text(
            "Liquid Chrome: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "shader=REFUSED, fallback=unavailable notice".into();
        return;
    };
    let setting = |i| frame.setting(SceneId::LiquidChrome, i);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("materialTime", state.clock());
    set("surfaceScale", setting(control::SCALE));
    set("rippleAmount", setting(control::RIPPLE));
    set("hueTurn", setting(control::HUE).to_radians());
    set("chromeShine", setting(control::SHINE));
    set("contrast", setting(control::CONTRAST));
    set("grain", setting(control::GRAIN));
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(
        location,
        Vector2::new(boundary.width.max(1.0), boundary.height.max(1.0)),
    );
    let location = shader.get_shader_location("phases");
    let phase = state.phase();
    shader.set_shader_value(
        location,
        Vector4::new(phase[0], phase[1], phase[2], phase[3]),
    );
    let location = shader.get_shader_location("drive");
    let drive = state.drive();
    shader.set_shader_value(location, Vector3::new(drive[0], drive[1], drive[2]));
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
        "shader=drawn, material=chrome, clock={:.3}, drive={:?}, dynamics={}",
        state.clock(),
        state.drive(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
