//! Metaball Garden's embedded GLSL330 surface and truthful GPU diagnostics.
use musializer_core::scene::settings::index::metaballs as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::metaball_garden::MetaballGardenState;
use musializer_runtime::draw;
use raylib::prelude::*;
const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/metaball_garden.fs");
pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}
impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid()
            && shader.get_shader_location("gardenMetal") >= 0
            && shader.get_shader_location("gardenBlobs[0]") >= 0;
        Self {
            shader: valid.then_some(shader),
            report: if valid {
                "not drawn yet".into()
            } else {
                "REFUSED: Metaball Garden shader unavailable".into()
            },
        }
    }
}
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &MetaballGardenState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some(shader) = &mut resources.shader else {
        d.draw_rectangle_rec(boundary, Color::new(8, 12, 19, 255));
        d.draw_text(
            "Metaball Garden: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "REFUSED: fallback label, shader not drawn".into();
        return;
    };
    let setting = |i| frame.setting(SceneId::MetaballGarden, i);
    let blobs = state
        .blobs(setting(control::SIZE), setting(control::ORBIT))
        .map(|b| Vector4::new(b[0], b[1], b[2], b[3]));
    let location = shader.get_shader_location("gardenBlobs[0]");
    shader.set_shader_value_v(location, &blobs);
    for (name, value) in [
        ("blobCount", setting(control::COUNT).round()),
        ("gardenMetal", setting(control::METAL)),
        ("gardenGlow", setting(control::GLOW)),
        ("hueTurn", setting(control::HUE).to_radians()),
        ("musicLight", state.light()),
        ("highLight", state.high()),
        ("gardenTime", state.clock()),
    ] {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    }
    let location = shader.get_shader_location("resolution");
    shader.set_shader_value(
        location,
        Vector2::new(boundary.width.max(1.0), boundary.height.max(1.0)),
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
        )
    });
    resources.report = format!(
        "surface=shader, march=64x9 max, blobs={}, light={:.3}, treble={:.3}, clock={:.3}, dyn={}",
        setting(control::COUNT).round(),
        state.light(),
        state.high(),
        state.clock(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "none"
        }
    );
}
