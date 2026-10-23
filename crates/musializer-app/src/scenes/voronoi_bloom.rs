//! Voronoi Bloom's bounded cellular field; resources never advance its clock.
use musializer_core::scene::settings::index::voronoi as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::voronoi_bloom::VoronoiBloomState;
use musializer_runtime::draw;
use raylib::prelude::*;
const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/voronoi_bloom.fs");
pub struct Resources {
    shader: Option<Shader>,
    pub report: String,
}
impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let valid = shader.is_shader_valid()
            && shader.get_shader_location("bloomSeed") >= 0
            && shader.get_shader_location("voices[0]") >= 0;
        Self {
            shader: valid.then_some(shader),
            report: if valid {
                "not drawn yet".into()
            } else {
                "REFUSED: Voronoi Bloom shader unavailable".into()
            },
        }
    }
}
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut Resources,
    state: &VoronoiBloomState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some(shader) = &mut resources.shader else {
        d.draw_rectangle_rec(boundary, Color::new(8, 12, 19, 255));
        d.draw_text(
            "Voronoi Bloom: shader unavailable",
            boundary.x as i32 + 16,
            boundary.y as i32 + 24,
            20,
            Color::RAYWHITE,
        );
        resources.report = "REFUSED: fallback label, shader not drawn".into();
        return;
    };
    let setting = |i| frame.setting(SceneId::VoronoiBloom, i);
    let location = shader.get_shader_location("voices[0]");
    shader.set_shader_value_v(location, state.voices());
    for (name, value) in [
        ("bloomSeed", state.seed()),
        ("bloomTime", state.clock()),
        ("cellScale", setting(control::CELLS).round()),
        ("edgeWeight", setting(control::EDGES)),
        ("bloomGlow", setting(control::GLOW)),
        ("driftAmount", setting(control::DRIFT)),
        ("facetAmount", setting(control::FACET)),
        ("hueTurn", setting(control::HUE).to_radians()),
        ("bassMass", state.bass()),
        ("musicLight", state.light()),
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
        "surface=shader, lattice=9+25, cells={}, voices=12, light={:.3}, bass={:.3}, clock={:.3}, dyn={}",
        setting(control::CELLS).round(),
        state.light(),
        state.bass(),
        state.clock(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "none"
        }
    );
}
