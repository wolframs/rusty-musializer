//! Pentagram Orbits: the drawing half.
//!
//! **Owner: Agent D.** Port of the drawing half of
//! `../musializer/src/scene_pentagram.c`. The Lyness geometry, the spectral flex and
//! the hop easing are in `musializer_core::scenes::pentagram`.
//!
//! Draw order, which is the composition: background and its radial bloom in normal
//! blending, then **everything else additively** — the nest of level curves, the
//! pentagram chords and their station dots, and finally the shader-shaped sparks and
//! the golden ember at the fixed point.
//!
//! Nothing here integrates state over time. Every animated quantity is derived from
//! `frame.time_seconds`, which is exactly why seeking and offline export land on
//! identical frames.

// See the note in `spectral_terrarium`: nothing dispatches the scene drawing
// halves yet.
#![allow(dead_code)]

use musializer_core::scene::settings::index::pentagram as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::pentagram::{
    self as pentagram, PentagramState, Vec2 as CoreVec2, CURVE_CAPACITY, CURVE_SAMPLES,
    ORBIT_CAPACITY, ORBIT_PERIOD,
};
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibShaderModeExt,
    Rectangle, Vector2,
};

use super::spectral_terrarium::set_circle;
use super::spectrum::CircleShader;

const PI: f32 = std::f32::consts::PI;

fn vec2(v: CoreVec2) -> Vector2 {
    Vector2::new(v.x, v.y)
}

/// Draws Pentagram Orbits into `boundary`.
#[allow(clippy::needless_range_loop)] // Curve, orbit and sample indices drive the
                                      // angle, depth and hue arithmetic; the index loops keep this diffable against the C.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &PentagramState,
    shader: &mut CircleShader,
    boundary: Rectangle,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }

    let motion = frame.setting(SceneId::Pentagram, setting::MOTION);
    let nest_count =
        (frame.setting(SceneId::Pentagram, setting::NEST).round() as usize).min(CURVE_CAPACITY);
    let orbit_count =
        (frame.setting(SceneId::Pentagram, setting::ORBITS).round() as usize).min(ORBIT_CAPACITY);
    let glow_scale = frame.setting(SceneId::Pentagram, setting::GLOW);
    let chord_scale = frame.setting(SceneId::Pentagram, setting::CHORDS);
    // Defaults to -91 degrees, the only nonzero hue default of the ten scenes.
    let hue_shift = frame.setting(SceneId::Pentagram, setting::HUE);
    let pulse_scale = frame.setting(SceneId::Pentagram, setting::PULSE);
    let field_scale = frame.setting(SceneId::Pentagram, setting::ZOOM);

    let time = frame.time_seconds as f32;
    let flux = pentagram::clamp01(frame.audio.spectral_flux);
    let rms = pentagram::clamp01(frame.audio.rms);
    let beat_phase = pentagram::clamp01(frame.audio.beat_phase);
    let beat_pop = (1.0 - beat_phase) * (1.0 - beat_phase);
    let onset_flash = if frame.audio.onset { 1.0 } else { 0.0 };
    let drive = pentagram::clamp01(0.4 * rms + 0.6 * flux);

    let semantic_weight = if frame.semantic.available {
        frame.semantic.confidence
    } else {
        0.0
    };
    // The +1440 is the C's: it keeps the sum positive before the modulo, since
    // `hue_shift` reaches -180 and the semantic term can also go negative.
    let base_hue = (38.0
        + pentagram::unit(state.seed, 7) * 30.0
        + hue_shift
        + frame.semantic.valence * 45.0 * semantic_weight
        + 1440.0)
        % 360.0;

    let background = draw::color_from_hsv((base_hue + 226.0) % 360.0, 0.62, 0.032 + drive * 0.030);
    d.draw_rectangle_rec(boundary, background);
    let center = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    let span = boundary.width.min(boundary.height);
    d.draw_circle_gradient(
        center.x as i32,
        center.y as i32,
        span * 0.72,
        draw::color_alpha(
            draw::color_from_hsv((base_hue + 208.0) % 360.0, 0.58, 0.11 + drive * 0.07),
            0.6,
        ),
        Color::BLANK,
    );

    let rotation = time * 0.042 * motion + pentagram::unit(state.seed, 8) * 2.0 * PI;
    let cos_r = rotation.cos();
    let sin_r = rotation.sin();
    // The whole nest breathes with signal level; per-band flexing happens
    // point-by-point below via `pentagram::shape`.
    let scale = 0.44 * span / state.extent * field_scale * (1.0 + rms * 0.05 * pulse_scale);
    let core_center = CoreVec2::new(center.x, center.y);

    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        for curve in 0..nest_count {
            // Depth is measured against the full capacity, not `nest_count`, so
            // reducing the nest count removes outer curves instead of restacking them.
            let depth = (curve as f32 + 0.6) / CURVE_CAPACITY as f32;
            let trail = pentagram::band(frame.audio.trails, curve);
            // Each beat launches a brightness wave from the golden center that travels
            // outward through the nest, carried by beat phase alone.
            let ripple_distance = depth - beat_phase;
            let ripple = (-ripple_distance * ripple_distance / 0.018).exp()
                * (0.25 + flux * 0.75)
                * pulse_scale;
            let line = draw::color_from_hsv(
                (base_hue + depth * 58.0) % 360.0,
                0.58 + trail * 0.24,
                (0.30 + trail * 0.55 + ripple * 0.22 + onset_flash * 0.10).min(1.0),
            );
            let alpha = (0.32 + trail * 0.48 + ripple * 0.30 + flux * 0.10).min(0.9);
            let thickness = (span * 0.0019 * (0.60 + trail * 0.85 + ripple * 0.55)).max(1.0);
            let last_angle = (CURVE_SAMPLES - 1) as f32 * (2.0 * PI / CURVE_SAMPLES as f32);
            let mut previous = pentagram::project(
                pentagram::flex(
                    state.curves[curve][CURVE_SAMPLES - 1],
                    pentagram::shape(frame, last_angle, depth, pulse_scale) + ripple * 0.05,
                ),
                core_center,
                cos_r,
                sin_r,
                scale,
            );
            for sample in 0..CURVE_SAMPLES {
                // Samples were traced at exactly this angle in init, so the spectral
                // contour lands on the curve without any refit.
                let angle = sample as f32 * (2.0 * PI / CURVE_SAMPLES as f32);
                let disp = pentagram::shape(frame, angle, depth, pulse_scale) + ripple * 0.05;
                let point = pentagram::project(
                    pentagram::flex(state.curves[curve][sample], disp),
                    core_center,
                    cos_r,
                    sin_r,
                    scale,
                );
                blend.draw_line_ex(
                    vec2(previous),
                    vec2(point),
                    thickness,
                    draw::color_alpha(line, alpha),
                );
                previous = point;
            }
        }

        for orbit in 0..orbit_count {
            let mut points = [CoreVec2::default(); ORBIT_PERIOD];
            for step in 0..ORBIT_PERIOD {
                points[step] = state.station_point(
                    frame,
                    orbit,
                    step,
                    core_center,
                    cos_r,
                    sin_r,
                    scale,
                    pulse_scale,
                );
            }
            let trail = pentagram::band(frame.audio.trails, orbit);
            let hue = (base_hue + state.orbit_depth[orbit] * 58.0) % 360.0;
            let hops = pentagram::orbit_hops(state, orbit, time, motion);
            let active = pentagram::active_station(hops);

            if chord_scale > 0.001 {
                for edge in 0..ORBIT_PERIOD {
                    let highlight = if edge == active {
                        0.30 + flux * 0.30
                    } else {
                        0.0
                    };
                    let alpha = ((0.09 + trail * 0.14 + highlight) * chord_scale).min(0.8);
                    let thickness = (span * 0.0012 * (1.0 + highlight * 2.2)).max(1.0);
                    blend.draw_line_ex(
                        vec2(points[edge]),
                        vec2(points[(edge + 1) % ORBIT_PERIOD]),
                        thickness,
                        draw::color_alpha(draw::color_from_hsv(hue, 0.46, 0.9), alpha),
                    );
                }
            }
            for step in 0..ORBIT_PERIOD {
                blend.draw_circle_v(
                    vec2(points[step]),
                    (span * 0.0021 * (1.0 + trail * 0.8)).max(1.0),
                    draw::color_alpha(
                        draw::color_from_hsv(hue, 0.38, 0.95),
                        (0.28 + trail * 0.40).min(0.75),
                    ),
                );
            }
        }

        // Sparks and the golden fixed point render as soft shader glows on top.
        let texture = draw::default_texture();
        let source = Rectangle::new(0.0, 0.0, 1.0, 1.0);
        let origin = Vector2::zero();
        set_circle(shader, 0.05, 2.3);
        blend.draw_shader_mode(&mut shader.shader, |mut pass| {
            if glow_scale > 0.001 {
                for orbit in 0..orbit_count {
                    let band = pentagram::band(frame.audio.bands, orbit);
                    let hops = pentagram::orbit_hops(state, orbit, time, motion);
                    let active = pentagram::active_station(hops);
                    // Path position must stay monotonic: a decaying beat term added
                    // here once made sparks slide backward along their chords after
                    // every lunge. Beat energy belongs to the radial push and the size
                    // pop below, where decay is a pulse rather than a retreat.
                    let eased = pentagram::hop_ease(hops - hops.floor());
                    let from = state.station_point(
                        frame,
                        orbit,
                        active,
                        core_center,
                        cos_r,
                        sin_r,
                        scale,
                        pulse_scale,
                    );
                    let to = state.station_point(
                        frame,
                        orbit,
                        (active + 1) % ORBIT_PERIOD,
                        core_center,
                        cos_r,
                        sin_r,
                        scale,
                        pulse_scale,
                    );
                    let mut head = Vector2::new(
                        from.x + (to.x - from.x) * eased,
                        from.y + (to.y - from.y) * eased,
                    );
                    // Audio pushes the spark radially as an additive offset from the
                    // current frame only; nothing is integrated, so it stays seek-safe.
                    let push_x = head.x - center.x;
                    let push_y = head.y - center.y;
                    let push_length = (push_x * push_x + push_y * push_y).sqrt();
                    if push_length > 1.0 {
                        let push = pulse_scale * (flux * 0.55 + beat_pop * 0.45) * span * 0.016;
                        head.x += push_x / push_length * push;
                        head.y += push_y / push_length * push;
                    }
                    let hue = (base_hue + state.orbit_depth[orbit] * 58.0) % 360.0;
                    let size = span
                        * (0.048
                            + band * 0.056
                            + beat_pop * pulse_scale * 0.024
                            + onset_flash * 0.012)
                        * glow_scale;
                    let dest = Rectangle::new(head.x - size * 0.5, head.y - size * 0.5, size, size);
                    draw::draw_texture_pro(
                        &mut pass,
                        texture,
                        source,
                        dest,
                        origin,
                        0.0,
                        draw::color_alpha(
                            draw::color_from_hsv(hue, 0.42, 1.0),
                            (0.46 + band * 0.32 + flux * 0.18).min(0.9),
                        ),
                    );
                }
            }
            // The map's fixed point sits at x = y = phi: a quiet golden ember marks the
            // golden ratio at the heart of the nest. Drawn even at zero glow, with a
            // floor of 0.25, because it is the scene's anchor.
            let ember = span
                * (0.082 + rms * 0.075 + beat_pop * pulse_scale * 0.030)
                * glow_scale.max(0.25);
            let ember_dest =
                Rectangle::new(center.x - ember * 0.5, center.y - ember * 0.5, ember, ember);
            draw::draw_texture_pro(
                &mut pass,
                texture,
                source,
                ember_dest,
                origin,
                0.0,
                draw::color_alpha(
                    draw::color_from_hsv((base_hue + 6.0) % 360.0, 0.55, 1.0),
                    (0.30 + rms * 0.40).min(0.8),
                ),
            );
        });
    });
}
