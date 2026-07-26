//! Constellation: the drawing half.
//!
//! **Owner: Agent D.** Port of the drawing half of
//! `../musializer/src/scene_constellation.c`. Node geometry, the event lane and the
//! envelope filter are in `musializer_core::scenes::constellation`.
//!
//! Draw order, which is the composition because the glow pass is additive:
//! background, nebula, web tubes, opaque node spheres, additive halos and
//! cross-flare streaks, then the atmospheric wash.
//!
//! The event colours are the point of this scene: an authored cue is teal, a
//! model-derived semantic event is amber, a lyric event is magenta, and a node with
//! no event keeps its own spectral hue. That mapping is what keeps four evidence
//! lanes visually distinct instead of merging into one glow.

// See the note in `spectral_terrarium`: nothing dispatches the scene drawing
// halves yet.
#![allow(dead_code)]

use musializer_core::scene::settings::index::constellation as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::constellation::{
    self as constellation, ConstellationState, EventFlare, Vec3 as CoreVec3, NODE_COUNT,
};
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Camera3D, Color, RaylibBlendModeExt, RaylibDraw, RaylibDraw3D, RaylibDrawHandle,
    RaylibMode3DExt, RaylibShaderModeExt, Rectangle, Vector2, Vector3,
};

use super::spectral_terrarium::{draw_billboard_rec, set_circle, SceneViewport};
use super::spectrum::CircleShader;

const TAU: f32 = std::f32::consts::TAU;

fn vec3(v: CoreVec3) -> Vector3 {
    Vector3::new(v.x, v.y, v.z)
}

/// Draws Constellation into `boundary`.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &ConstellationState,
    shader: &mut CircleShader,
    boundary: Rectangle,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }

    let motion_scale = frame.setting(SceneId::Constellation, setting::MOTION);
    let field_scale = frame.setting(SceneId::Constellation, setting::SCALE);
    let glow_scale = frame.setting(SceneId::Constellation, setting::GLOW);
    let event_duration = frame.setting(SceneId::Constellation, setting::EVENT_DURATION);
    let hue_swing = frame.setting(SceneId::Constellation, setting::HUE_SWING);
    let web = frame.setting(SceneId::Constellation, setting::WEB);

    let node_count =
        constellation::node_count(frame.setting(SceneId::Constellation, setting::DENSITY));
    let event_reach = constellation::clamp_event_reach(
        frame
            .setting(SceneId::Constellation, setting::EVENT_REACH)
            .round() as usize,
        node_count,
    );

    // Note the C's `time`: song time already multiplied by the motion setting, so
    // the setting scales the whole scene's clock rather than one term of it.
    let time = frame.time_seconds as f32 * motion_scale;
    let semantic_weight = if frame.semantic.available {
        frame.semantic.confidence
    } else {
        0.0
    };
    let base_hue = (201.0
        + constellation::unit(state.seed, 9) * 95.0
        + time * 1.8
        + frame.semantic.valence * hue_swing * semantic_weight)
        % 360.0;
    let background = draw::color_from_hsv(base_hue, 0.72, 0.035 + state.motion.energy * 0.035);
    d.draw_rectangle_rec(boundary, background);
    // A soft off-center nebula gives the star field a deep sky to sit in instead of
    // flat black.
    let nebula_center = Vector2::new(
        boundary.x + boundary.width * (0.36 + constellation::unit(state.seed, 11) * 0.28),
        boundary.y + boundary.height * (0.34 + constellation::unit(state.seed, 12) * 0.30),
    );
    let nebula_radius = boundary.width.max(boundary.height) * 0.62;
    d.draw_circle_gradient(
        nebula_center.x as i32,
        nebula_center.y as i32,
        nebula_radius,
        draw::color_alpha(
            draw::color_from_hsv(
                (base_hue + 24.0) % 360.0,
                0.66,
                0.16 + state.motion.energy * 0.10,
            ),
            0.55,
        ),
        Color::BLANK,
    );

    let Some(viewport) = SceneViewport::begin(d, boundary) else {
        return;
    };

    let orbit =
        time * (0.035 + state.motion.flux * 0.055) + constellation::unit(state.seed, 10) * TAU;
    let camera = Camera3D::perspective(
        Vector3::new(
            orbit.cos() * 8.9,
            1.2 + (orbit * 0.61).sin() * 0.8,
            orbit.sin() * 8.9,
        ),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        56.0 + state.motion.onset_pulse * 3.0,
    );

    // Positions and flares are resolved once for the whole frame, because the web
    // pass reads its neighbours' flares and the glow pass reads the same values the
    // sphere pass drew with. Fixed-size arrays, as in the C.
    let mut positions = [Vector3::zero(); NODE_COUNT];
    let mut flares = [EventFlare::default(); NODE_COUNT];
    for i in 0..node_count {
        let band = constellation::band(frame, i);
        flares[i] = constellation::event_flare(frame, i, node_count, event_duration, event_reach);
        let base = constellation::base_position(state.seed, i, node_count, time, band);
        let displacement = constellation::event_displacement(flares[i], i, state.motion.energy);
        positions[i] = Vector3::new(
            (base.x + displacement.x) * field_scale,
            (base.y + displacement.y) * field_scale,
            (base.z + displacement.z) * field_scale,
        );
    }

    d.draw_mode3D(camera, |mut m3, camera| {
        viewport.correct_aspect(&mut m3);

        // The web: each node links to its immediate neighbour and to one further
        // around the ring, which is what makes the field read as a constellation
        // rather than a shell of dots.
        if web > 0.001 {
            let long_step = if node_count > 36 { 13 } else { 7 };
            for i in 0..node_count {
                for other in [(i + 1) % node_count, (i + long_step) % node_count] {
                    let active = flares[i].strength.max(flares[other].strength);
                    let line = draw::color_from_hsv(
                        (base_hue + i as f32 * 1.7) % 360.0,
                        0.48 + active * 0.35,
                        ((0.24 + state.motion.energy * 0.18 + active * 0.50) * web).min(1.0),
                    );
                    draw::tube(
                        &mut m3,
                        positions[i],
                        positions[other],
                        0.006 + active * 0.012,
                        5,
                        draw::color_alpha(line, ((0.40 + active * 0.55) * web).min(1.0)),
                    );
                }
            }
        }

        for i in 0..node_count {
            let band = constellation::band(frame, i);
            let brightness = 0.47 + band * 0.3 + flares[i].strength * 0.52;
            let (hue, saturation, value) = constellation::event_hsv(
                flares[i].event_type,
                (base_hue + i as f32 * 2.1) % 360.0,
                brightness,
            );
            let radius = (0.045
                + band * 0.075
                + flares[i].strength * 0.16
                + state.motion.onset_pulse * 0.018)
                * glow_scale;
            m3.draw_sphere(
                positions[i],
                radius,
                draw::color_from_hsv(hue, saturation, value),
            );
        }

        // Stars glow as camera-facing soft sprites rather than wireframe shells: a
        // wide faint halo, and thin cross-flare streaks on the strongest nodes.
        let glow_texture = draw::default_texture();
        let glow_source = Rectangle::new(0.0, 0.0, 1.0, 1.0);
        set_circle(shader, 0.06, 2.6);
        m3.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
            blend.draw_shader_mode(&mut shader.shader, |mut pass| {
                for i in 0..node_count {
                    let band = constellation::band(frame, i);
                    let flare =
                        band * 0.35 + flares[i].strength * 0.85 + state.motion.onset_pulse * 0.12;
                    if flare < 0.08 || glow_scale <= 0.001 {
                        continue;
                    }
                    let (hue, saturation, value) = constellation::event_hsv(
                        flares[i].event_type,
                        (base_hue + i as f32 * 2.1) % 360.0,
                        0.9,
                    );
                    let glow = draw::color_from_hsv(hue, saturation, value);
                    let halo = (0.30 + band * 0.34 + flares[i].strength * 0.75) * glow_scale;
                    draw_billboard_rec(
                        &mut pass,
                        camera,
                        glow_texture,
                        glow_source,
                        positions[i],
                        Vector2::new(halo, halo),
                        draw::color_alpha(glow, (0.22 + flare * 0.40).min(0.60)),
                    );
                    if flare > 0.45 {
                        let streak = Vector2::new(halo * (2.4 + flare), halo * 0.30);
                        let streak_color = draw::color_alpha(glow, (flare * 0.30).min(0.40));
                        // Two crossed streaks, the second rotated 90 degrees: the
                        // classic anamorphic star flare.
                        for rotation in [0.0f32, 90.0] {
                            pass.draw_billboard_pro(
                                camera,
                                glow_texture,
                                glow_source,
                                positions[i],
                                Vector3::new(0.0, 1.0, 0.0),
                                streak,
                                Vector2::new(streak.x * 0.5, streak.y * 0.5),
                                rotation,
                                streak_color,
                            );
                        }
                    }
                }
            });
        });
    });

    viewport.end(d);
    d.draw_rectangle_rec(boundary, draw::color_alpha(background, 0.035));
}
