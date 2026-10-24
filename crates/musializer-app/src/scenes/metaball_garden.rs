//! Metaball Garden's embedded GLSL330 sculpture and truthful GPU diagnostics.
//!
//! Every position here is read from the core state; this module only lays it
//! out as uniforms. The report line names what the frame actually held — the
//! drawn blob count, kicks heard, live glints, the squash and split springs —
//! because a sculpture that stopped hearing kicks still photographs as a
//! plausible sculpture.
use musializer_core::scene::settings::index::metaballs as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::metaball_garden::{
    MetaballGardenState, MAX_BLOBS, MAX_DROPLETS, MAX_MAINS,
};
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
fn v4(v: [f32; 4]) -> Vector4 {
    Vector4::new(v[0], v[1], v[2], v[3])
}
fn v3(v: [f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
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
    let (size, orbit) = (setting(control::SIZE), setting(control::ORBIT));
    let mains = (setting(control::COUNT).round() as usize).clamp(3, MAX_MAINS);
    let drops = mains.min(MAX_DROPLETS);
    let main_blobs = state.blobs(size, orbit);
    let droplets = state.droplets(size, orbit);
    let tints = state.tints(setting(control::HUE));
    // Mains first, then droplets, packed so the shader's loop stops at the
    // first unused slot.
    let mut blobs = [Vector4::zero(); MAX_BLOBS];
    let mut colours = [Vector4::zero(); MAX_BLOBS];
    for (slot, (blob, tint)) in main_blobs[..mains]
        .iter()
        .zip(&tints[..mains])
        .chain(
            droplets[..drops]
                .iter()
                .zip(&tints[MAX_MAINS..MAX_MAINS + drops]),
        )
        .enumerate()
    {
        blobs[slot] = v4(*blob);
        colours[slot] = Vector4::new(tint[0], tint[1], tint[2], 1.0);
    }
    let total = mains + drops;
    let centre = state.centre();
    let ring = state.ring(orbit);
    let camera = state.camera(size, orbit);
    let squash = state.squash();
    // Everything the march may meet sits inside this sphere about the centre;
    // the squash stretches the goo by at most ~18 %, the goo radius pads necks.
    let goo_radius = (0.2 + 0.12 * state.bass()) * (size / 0.8).clamp(0.6, 1.6);
    let reach = blobs[..total]
        .iter()
        .map(|b| {
            ((b.x - centre[0]).powi(2) + (b.y - centre[1]).powi(2) + (b.z - centre[2]).powi(2))
                .sqrt()
                + b.w
        })
        .fold(0.0f32, f32::max)
        * 1.2
        + goo_radius;
    let bound = reach.max(ring.radius + 0.15);

    let location = shader.get_shader_location("gardenBlobs[0]");
    shader.set_shader_value_v(location, &blobs);
    let location = shader.get_shader_location("gardenTints[0]");
    shader.set_shader_value_v(location, &colours);
    let location = shader.get_shader_location("beads[0]");
    shader.set_shader_value_v(location, &ring.beads.map(v4));
    let location = shader.get_shader_location("sparkles[0]");
    shader.set_shader_value_v(location, &state.sparkles().map(v4));
    for (name, value) in [
        (
            "sculpt",
            Vector4::new(centre[0], centre[1], centre[2], squash),
        ),
        (
            "ringPlane",
            Vector4::new(ring.normal[0], ring.normal[1], ring.normal[2], ring.radius),
        ),
    ] {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, value);
    }
    for (name, value) in [("camPos", camera.position), ("camTarget", camera.target)] {
        let location = shader.get_shader_location(name);
        shader.set_shader_value(location, v3(value));
    }
    for (name, value) in [
        ("blobTotal", total as f32),
        ("bound", bound),
        ("gooRadius", goo_radius),
        ("gardenMetal", setting(control::METAL)),
        ("gardenGlow", setting(control::GLOW)),
        ("musicLight", state.light()),
        ("sectionEnergy", state.energy()),
        ("highLight", state.high()),
        ("accent", state.accent()),
        ("wobble", state.wobble()),
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
        "surface=shader, march=112/40/20, blobs={mains}+{drops}, kicks={}, glints={} live, \
         squash={squash:.3}, burst={:.3}, light={:.3}, energy={:.3}, treble={:.3}, clock={:.3}, dyn={}",
        state.kick_count(),
        state.live_sparkles(),
        state.burst(),
        state.light(),
        state.energy(),
        state.high(),
        state.clock(),
        if frame.dynamics.is_some() {
            "track"
        } else {
            "none"
        }
    );
}
