//! Spectrum: the drawing half.
//!
//! Port of `../musializer/src/scene_spectrum.c`. Spectrum has no deterministic
//! state at all — the C descriptor sets no `update` (`scene_spectrum.c:149-154`),
//! so it is a pure function of the frame.
//!
//! Formulas and draw-call order are kept recognizable against the C on purpose.
//! Refactoring the artistry comes after the Rust application works; until then a
//! line here should be diffable against the line it came from.
//!
//! The three passes, in order, because additive blending makes order visible:
//! 1. opaque bars (plus their floor reflection, if enabled);
//! 2. an additive vertical smear from each band's trail to its current peak;
//! 3. an additive bright core at each bar's head.

use musializer_core::scene::settings::index::spectrum as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibShader,
    RaylibShaderModeExt, Rectangle, Vector2,
};

/// Uniform locations resolved once, so the per-band loop does not look them up.
pub struct CircleShader {
    pub shader: raylib::shaders::Shader,
    pub radius_location: i32,
    pub power_location: i32,
}

impl CircleShader {
    /// The GLSL 330 variant is the only one the C build actually reaches:
    /// `GLSL_VERSION` is hard-coded to 330, so `glsl120/circle.fs` ships but is
    /// unreachable there.
    pub const SOURCE: &'static str =
        include_str!("../../../../resources/shaders/glsl330/circle.fs");

    pub fn load(
        rl: &mut raylib::RaylibHandle,
        thread: &raylib::RaylibThread,
    ) -> Result<Self, String> {
        let shader = rl.load_shader_from_memory(thread, None, Some(Self::SOURCE));
        let radius_location = shader.get_shader_location("radius");
        let power_location = shader.get_shader_location("power");
        if radius_location < 0 || power_location < 0 {
            return Err(format!(
                "circle shader compiled but its uniforms are missing (radius={radius_location}, power={power_location})"
            ));
        }
        Ok(Self {
            shader,
            radius_location,
            power_location,
        })
    }

    /// Public because three of Agent D's scenes set these too; going through
    /// the raw `radius_location` field instead was a workaround for this being
    /// private.
    pub fn set_radius(&mut self, radius: f32) {
        self.shader.set_shader_value(self.radius_location, radius);
    }

    pub fn set_power(&mut self, power: f32) {
        self.shader.set_shader_value(self.power_location, power);
    }
}

/// Draws Spectrum into `boundary`.
///
/// The band loops index `bands` and `trails` by position rather than iterating,
/// deliberately: they are parallel arrays read together with a shared index, and
/// keeping `for i in 0..bands_count` makes each loop diff cleanly against the C
/// it came from. Once the artistry is refactored (after the application works),
/// zipping is the better shape.
///
/// `pixel_scale` is physical target pixels per logical output pixel: fixed-pixel
/// details use it so supersampling changes sampling, not composition
/// (`../musializer/src/scene.h:64-66`).
#[allow(clippy::needless_range_loop)]
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    shader: &mut CircleShader,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    let bands_count = frame.audio.bands_count();
    if bands_count == 0 || frame.audio.trails.len() != bands_count {
        return;
    }
    let bands = frame.audio.bands;
    let trails = frame.audio.trails;

    let amplitude_scale = frame.setting(SceneId::Spectrum, setting::AMPLITUDE);
    let trail_scale = frame.setting(SceneId::Spectrum, setting::TRAIL);
    let saturation_scale = frame.setting(SceneId::Spectrum, setting::SATURATION);
    let glow_softness = frame.setting(SceneId::Spectrum, setting::GLOW_SOFTNESS);
    let hue_swing = frame.setting(SceneId::Spectrum, setting::HUE_SWING);
    let core_glow = frame.setting(SceneId::Spectrum, setting::CORE_GLOW);
    let bar_taper = frame.setting(SceneId::Spectrum, setting::BAR_TAPER);
    let reflection = frame.setting(SceneId::Spectrum, setting::REFLECTION);

    let cell_width = boundary.width / bands_count as f32;
    // Reflection lifts the bars onto a polished floor strip; zero keeps the
    // original bottom-edge composition.
    let floor_drop = boundary.height * 0.13 * 1.0f32.min(reflection * 2.0);
    let base_y = boundary.y + boundary.height - floor_drop;

    let semantic_weight = if frame.semantic.available {
        frame.semantic.confidence
    } else {
        0.0
    };
    let semantic_hue = frame.semantic.valence * hue_swing * semantic_weight;
    let saturation =
        ((0.75 + frame.semantic.tension * 0.2 * semantic_weight) * saturation_scale).min(1.0);
    let value = 1.0f32;

    let band_color = |i: usize| -> Color {
        let hue = i as f32 / bands_count as f32;
        draw::color_from_hsv(
            (hue * 360.0 + semantic_hue + 360.0) % 360.0,
            saturation,
            value,
        )
    };
    let bar_top = |t: f32| base_y - boundary.height * 2.0 / 3.0 * t;

    // Pass 1: the opaque bars.
    for i in 0..bands_count {
        let t = (bands[i] * amplitude_scale).min(1.0);
        let color = band_color(i);
        let x = boundary.x + i as f32 * cell_width + cell_width / 2.0;
        let start_pos = Vector2::new(x, bar_top(t));
        let end_pos = Vector2::new(x, base_y);
        let thick = cell_width / 3.0 * t.powf(bar_taper);
        d.draw_line_ex(start_pos, end_pos, thick, color);

        if reflection > 0.001 && floor_drop > 1.0 {
            let mirror_height = floor_drop.min(boundary.height * 2.0 / 3.0 * t * 0.45);
            d.draw_rectangle_gradient_v(
                (start_pos.x - thick * 0.5) as i32,
                base_y as i32,
                thick.max(1.0) as i32,
                mirror_height as i32,
                draw::color_alpha(color, (0.28 + reflection * 0.45).min(0.55)),
                draw::color_alpha(color, 0.0),
            );
        }
    }
    if reflection > 0.001 && floor_drop > 1.0 {
        d.draw_line_ex(
            Vector2::new(boundary.x, base_y),
            Vector2::new(boundary.x + boundary.width, base_y),
            pixel_scale.max(1.0),
            draw::color_alpha(
                Color::RAYWHITE,
                (0.14 + frame.audio.rms * 0.20) * 1.0f32.min(reflection * 2.0),
            ),
        );
    }

    let texture = draw::default_texture();
    let source = Rectangle::new(0.0, 0.0, 1.0, 1.0);
    let origin = Vector2::zero();

    // raylib-rs models raylib's Begin/End mode pairs as scoped closures rather
    // than as bare calls, so the C sequence becomes this nesting. The order and
    // the uniform values are unchanged; only the bracketing is Rust's.
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        // Pass 2: a soft vertical smear from each band's previous peak to its
        // current one. Stretching the full radial glow — rather than slicing it
        // at the equator — keeps both ends rounded and removes the hard bright
        // line a half-disc otherwise leaves cutting across the head of each bar.
        shader.set_radius(0.3);
        shader.set_power(glow_softness);
        blend.draw_shader_mode(&mut shader.shader, |mut pass| {
            for i in 0..bands_count {
                let start = (trails[i] * amplitude_scale).min(1.0);
                let end = (bands[i] * amplitude_scale).min(1.0);
                let color = band_color(i);
                let x = boundary.x + i as f32 * cell_width + cell_width / 2.0;
                let start_y = bar_top(start);
                let end_y = bar_top(end);
                let radius = cell_width * 3.0 * end.sqrt() * trail_scale;
                let smear_top = start_y.min(end_y);
                let smear_bottom = start_y.max(end_y);
                let dest = Rectangle::new(
                    x - radius * 0.5,
                    smear_top - radius * 0.5,
                    radius,
                    (smear_bottom - smear_top) + radius,
                );
                draw::draw_texture_pro(&mut pass, texture, source, dest, origin, 0.0, color);
            }
        });

        // Pass 3: a small bright core at each bar's head. A tighter radius and a
        // higher power than the smear, which is what makes it read as a core.
        shader.set_radius(0.07);
        shader.set_power(glow_softness + 2.0);
        blend.draw_shader_mode(&mut shader.shader, |mut pass| {
            for i in 0..bands_count {
                let t = (bands[i] * amplitude_scale).min(1.0);
                let color = band_color(i);
                let center = Vector2::new(
                    boundary.x + i as f32 * cell_width + cell_width / 2.0,
                    bar_top(t),
                );
                let radius = cell_width * 6.0 * t.sqrt() * core_glow;
                let position = Vector2::new(center.x - radius, center.y - radius);
                draw::draw_texture_ex(&mut pass, texture, position, 0.0, 2.0 * radius, color);
            }
        });
    });
}
