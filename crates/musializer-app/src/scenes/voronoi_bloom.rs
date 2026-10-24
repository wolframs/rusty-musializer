//! Voronoi Bloom's backlit stained glass; resources never advance its clock.
//!
//! The report names what the frame held — kicks heard, bloom waves alive, a
//! crack and its age — because a window that stopped hearing the music still
//! photographs as a plausible window.
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
    let waves = state.waves().map(|w| Vector4::new(w[0], w[1], w[2], w[3]));
    let location = shader.get_shader_location("waves[0]");
    shader.set_shader_value_v(location, &waves);
    let sun = state.sun();
    let location = shader.get_shader_location("sun");
    shader.set_shader_value(location, Vector3::new(sun[0], sun[1], sun[2]));
    let crack = state.shatter();
    let location = shader.get_shader_location("shatter");
    shader.set_shader_value(
        location,
        Vector4::new(crack[0], crack[1], crack[2], crack[3]),
    );
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
        ("sectionEnergy", state.energy()),
        ("trebleLight", state.high()),
        ("accent", state.accent()),
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
        "surface=shader, lattice=9+25, shafts=28x9, cells={}, voices=12, kicks={}, waves={} live, \
         crack={}, sun={:.2}@({:.2},{:.2}), light={:.3}, bass={:.3}, energy={:.3}, clock={:.3}, dyn={}",
        setting(control::CELLS).round(),
        state.kick_count(),
        state.live_waves(),
        if crack[3] > 0.0 {
            format!("{:.2}s", crack[2])
        } else {
            "none".into()
        },
        sun[2],
        sun[0],
        sun[1],
        state.light(),
        state.bass(),
        state.energy(),
        state.clock(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "none"
        }
    );
}
