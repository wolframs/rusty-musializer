//! Iridescent Pool's GPU half: oil on dark water over a pebble floor, drawn in
//! one full-panel fragment shader. Every moving input comes from core state;
//! the only other per-frame input is a 24 Hz tick of track time for the
//! anti-banding dither, so two renders of one moment are the same picture.
use musializer_core::scene::settings::index::iridescent as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::iridescent_pool::{IridescentPoolState, RING_COUNT};
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/iridescent_pool.fs");

/// Uniform locations, resolved once at load rather than by name every frame.
struct Locations {
    resolution: i32,
    material_time: i32,
    seed_phase: i32,
    camera_drift: i32,
    drive: i32,
    pulse: i32,
    dither_tick: i32,
    rings: i32,
    scale: i32,
    ripple: i32,
    hue: i32,
    shimmer: i32,
    light: i32,
    spread: i32,
}

pub struct Resources {
    shader: Option<(Shader, Locations)>,
    pub report: String,
}

impl Resources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        let at = |name: &str| shader.get_shader_location(name);
        let locations = Locations {
            resolution: at("resolution"),
            material_time: at("materialTime"),
            seed_phase: at("seedPhase"),
            camera_drift: at("cameraDrift"),
            drive: at("drive"),
            pulse: at("pulse"),
            dither_tick: at("ditherTick"),
            rings: at("rings"),
            scale: at("surfaceScale"),
            ripple: at("rippleAmount"),
            hue: at("hueTurn"),
            shimmer: at("shimmer"),
            light: at("poolLight"),
            spread: at("filmSpread"),
        };
        let valid = shader.is_shader_valid() && locations.spread >= 0 && locations.rings >= 0;
        Self {
            report: if valid {
                "shader=ready, not drawn".into()
            } else {
                "shader=REFUSED, not drawn".into()
            },
            shader: valid.then_some((shader, locations)),
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
    let Some((shader, at)) = resources.shader.as_mut() else {
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
    shader.set_shader_value(
        at.resolution,
        Vector2::new(boundary.width.max(1.0), boundary.height.max(1.0)),
    );
    shader.set_shader_value(at.material_time, state.clock());
    shader.set_shader_value(at.seed_phase, state.phase());
    let drift = state.camera_drift();
    shader.set_shader_value(at.camera_drift, Vector2::new(drift[0], drift[1]));
    let drive = state.drive();
    shader.set_shader_value(at.drive, Vector2::new(drive[0], drive[1]));
    shader.set_shader_value(at.pulse, state.pulse());
    let tick = (frame.time_seconds.max(0.0) * 24.0).floor() % 1_000_000.0;
    shader.set_shader_value(at.dither_tick, tick as f32);
    let rings: [Vector4; RING_COUNT] = std::array::from_fn(|i| {
        let ring = state.ripples()[i];
        Vector4::new(ring.origin[0], ring.origin[1], ring.age, ring.strength)
    });
    shader.set_shader_value_v(at.rings, &rings);
    shader.set_shader_value(at.scale, setting(control::SCALE));
    shader.set_shader_value(at.ripple, setting(control::RIPPLE));
    shader.set_shader_value(at.hue, setting(control::HUE).to_radians());
    shader.set_shader_value(at.shimmer, setting(control::SHIMMER));
    shader.set_shader_value(at.light, setting(control::LIGHT));
    shader.set_shader_value(at.spread, setting(control::SPREAD));
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
    let live = state
        .ripples()
        .iter()
        .filter(|r| r.remaining() > 0.02)
        .count();
    resources.report = format!(
        "shader=drawn, material=oil on water, clock={:.3}, drive=[{:.2}, {:.2}], pulse={:.2}, drops={} ({} kicks, {} live), dynamics={}",
        state.clock(),
        drive[0],
        drive[1],
        state.pulse(),
        state.launches(),
        state.kicks(),
        live,
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
