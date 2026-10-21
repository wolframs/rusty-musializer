//! Threshold's drawing half (MV9-d): one shader pass over the boundary.
//!
//! The room is ray-traced per pixel in `resources/shaders/glsl330/threshold.fs`;
//! this file resolves the settings and the camera, binds the nested scene the
//! renderer drew offscreen (or nothing, for the doorway's own fog), and says
//! what it drew. The nesting itself — drawing another scene into a buffer
//! mid-frame — is the renderer's, because only the renderer can draw a scene.

use musializer_core::scene::settings::index::threshold as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::threshold::{self, ThresholdState};
use musializer_runtime::draw;
use raylib::prelude::{
    Color, RaylibDraw, RaylibDrawHandle, RaylibShader, RaylibShaderModeExt, Rectangle, Shader,
    Vector2, Vector3,
};

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/threshold.fs");

/// The shader and the report.
pub struct ThresholdResources {
    shader: Option<Shader>,
    /// What the last frame drew, for the `threshold:` report line.
    pub report: String,
}

impl ThresholdResources {
    pub fn load(rl: &mut raylib::RaylibHandle, thread: &raylib::RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid();
        Self {
            shader: valid.then_some(shader),
            report: if valid {
                "not drawn yet".to_string()
            } else {
                "shader=REFUSED".to_string()
            },
        }
    }
}

/// Draws the room into `boundary`. `beyond` is the nested scene's picture,
/// drawn this frame at the boundary's size, when the doorway shows one.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut ThresholdResources,
    state: &ThresholdState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    beyond: Option<raylib_sys::Texture2D>,
) {
    let setting = |index| frame.setting(SceneId::Threshold, index);
    let camera = threshold::camera(
        setting(control::DISTANCE),
        setting(control::SWAY),
        frame.time_seconds,
    );
    let theta = threshold::door_angle(setting(control::OPEN));
    // The light breathes with the followed loudness; at the threshold's own
    // default it never goes dark.
    let light = setting(control::LIGHT) * (0.8 + 0.4 * state.glow().min(1.0));
    let (cx, cy, height) = threshold::beyond_map(&camera, boundary.width, boundary.height);
    let nested = state.beyond().map(|instance| instance.id());
    let Some(shader) = resources.shader.as_mut() else {
        // No room without its shader. Say so rather than draw a plausible
        // dark rectangle.
        d.draw_rectangle_rec(boundary, Color::new(10, 12, 16, 255));
        resources.report = "shader=REFUSED, drew nothing".to_string();
        return;
    };
    let texture = beyond.unwrap_or_else(draw::default_texture);
    let mut set = |name: &str, value: f32| {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    };
    set("theta", theta);
    set("light", light);
    set("haze", setting(control::HAZE));
    set("mist", setting(control::MIST));
    set("motes", setting(control::MOTES));
    set("presences", setting(control::PRESENCES));
    set("hueTurn", setting(control::HUE));
    set("reflection", setting(control::REFLECTION));
    set("time", frame.time_seconds as f32);
    set("seed", (state.seed() % 977) as f32);
    set("hasBeyond", if beyond.is_some() { 1.0 } else { 0.0 });
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(location, Vector2::new(boundary.width, boundary.height));
    let location = shader.get_shader_location("cam");
    shader.set_shader_value(location, Vector3::new(camera.x, camera.y, camera.z));
    let location = shader.get_shader_location("look");
    shader.set_shader_value(location, Vector2::new(camera.yaw, camera.pitch));
    let location = shader.get_shader_location("beyondMap");
    shader.set_shader_value(location, Vector3::new(cx, cy, height));
    // Flipped source: fragTexCoord.y is 1 at the top, which the shader relies
    // on, and the nested picture (a render texture) samples upright.
    let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
    d.draw_shader_mode(shader, |mut pass| {
        draw::draw_texture_pro(
            &mut pass,
            texture,
            source,
            boundary,
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    });
    resources.report = format!(
        "distance={:.2} (camera z {:.2}), open={:.2}, light={:.2}, beyond={}, presences={}",
        setting(control::DISTANCE),
        camera.z,
        setting(control::OPEN),
        light,
        match (nested, beyond.is_some()) {
            (Some(scene), true) => format!("{} (live)", scene.stable_name()),
            (Some(scene), false) => format!("{} (NOT DRAWN)", scene.stable_name()),
            (None, _) => "haze".to_string(),
        },
        setting(control::PRESENCES).round(),
    );
}
