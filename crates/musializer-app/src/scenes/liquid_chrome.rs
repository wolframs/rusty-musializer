//! Liquid Chrome's GPU half: a mercury sea and a chrome ring, marched in one
//! full-panel fragment shader. Every moving input comes from core state; the
//! only other per-frame input is a 24 Hz tick of track time for the grain, so
//! two renders of one moment draw the same specks.
use musializer_core::scene::settings::index::chrome as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::liquid_chrome::{LiquidChromeState, DROP_COUNT, IMPACT_COUNT};
use musializer_runtime::draw;
use raylib::prelude::*;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/liquid_chrome.fs");

/// Uniform locations, resolved once at load rather than by name every frame.
struct Locations {
    resolution: i32,
    phases: i32,
    drive: i32,
    pulse: i32,
    material_time: i32,
    camera_orbit: i32,
    grain_tick: i32,
    impacts: i32,
    drops: i32,
    scale: i32,
    ripple: i32,
    hue: i32,
    shine: i32,
    contrast: i32,
    grain: i32,
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
            phases: at("phases"),
            drive: at("drive"),
            pulse: at("pulse"),
            material_time: at("materialTime"),
            camera_orbit: at("cameraOrbit"),
            grain_tick: at("grainTick"),
            impacts: at("impacts"),
            drops: at("drops"),
            scale: at("surfaceScale"),
            ripple: at("rippleAmount"),
            hue: at("hueTurn"),
            shine: at("chromeShine"),
            contrast: at("contrast"),
            grain: at("grain"),
        };
        let valid = shader.is_shader_valid() && locations.shine >= 0 && locations.impacts >= 0;
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
    state: &LiquidChromeState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    let Some((shader, at)) = resources.shader.as_mut() else {
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
    shader.set_shader_value(
        at.resolution,
        Vector2::new(boundary.width.max(1.0), boundary.height.max(1.0)),
    );
    let phase = state.phase();
    shader.set_shader_value(
        at.phases,
        Vector4::new(phase[0], phase[1], phase[2], phase[3]),
    );
    let drive = state.drive();
    shader.set_shader_value(at.drive, Vector3::new(drive[0], drive[1], drive[2]));
    shader.set_shader_value(at.pulse, state.pulse());
    shader.set_shader_value(at.material_time, state.clock());
    let orbit = state.camera_orbit();
    shader.set_shader_value(at.camera_orbit, Vector2::new(orbit[0], orbit[1]));
    // Track time, not a frame counter: grain resampled per frame crawls at
    // 60 fps and strobes at 24, and a still must draw its video frame's specks.
    let tick = (frame.time_seconds.max(0.0) * 24.0).floor() % 1_000_000.0;
    shader.set_shader_value(at.grain_tick, tick as f32);
    let impacts: [Vector2; IMPACT_COUNT] =
        std::array::from_fn(|i| Vector2::new(state.impacts()[i].age, state.impacts()[i].strength));
    shader.set_shader_value_v(at.impacts, &impacts);
    let drops: [Vector4; DROP_COUNT] = std::array::from_fn(|i| {
        let drop = state.droplets()[i];
        Vector4::new(drop.origin[0], drop.origin[1], drop.age, drop.strength)
    });
    shader.set_shader_value_v(at.drops, &drops);
    shader.set_shader_value(at.scale, setting(control::SCALE));
    shader.set_shader_value(at.ripple, setting(control::RIPPLE));
    shader.set_shader_value(at.hue, setting(control::HUE).to_radians());
    shader.set_shader_value(at.shine, setting(control::SHINE));
    shader.set_shader_value(at.contrast, setting(control::CONTRAST));
    shader.set_shader_value(at.grain, setting(control::GRAIN));
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
    let live = |age: f32, strength: f32, life: f32| strength > 0.0 && age < life;
    resources.report = format!(
        "shader=drawn, material=mercury sea, clock={:.3}, drive=[{:.2}, {:.2}, {:.2}], pulse={:.2}, kicks={} ({} live), droplets={} ({} live), dynamics={}",
        state.clock(),
        drive[0],
        drive[1],
        drive[2],
        state.pulse(),
        state.kicks(),
        state
            .impacts()
            .iter()
            .filter(|i| live(i.age, i.strength, musializer_core::scenes::liquid_chrome::IMPACT_LIFE))
            .count(),
        state.splashes(),
        state
            .droplets()
            .iter()
            .filter(|d| live(d.age, d.strength, musializer_core::scenes::liquid_chrome::DROP_LIFE))
            .count(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "absolute"
        }
    );
}
