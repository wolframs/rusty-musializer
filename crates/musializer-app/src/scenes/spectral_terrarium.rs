//! Spectral Terrarium: the drawing half.
//!
//! **Owner: Agent D.** Port of the drawing half of
//! `../musializer/src/scene_spectral_terrarium.c`. The deterministic simulation is
//! in `musializer_core::scenes::spectral_terrarium`.
//!
//! Formulas and draw-call order are kept recognizable against the C, because
//! additive blending makes order visible and order is therefore behaviour.
//!
//! Draw order, which is the composition: background, soil, floor ring, onset
//! ripple, plants, particles, creatures, additive creature glow, glass rings, then
//! the atmospheric wash on top.
//!
//! This module also carries [`SceneViewport`], the sub-rectangle 3D viewport dance
//! that Constellation needs too. It belongs in `musializer_runtime::draw` next to
//! `tube()` — both scene agents need it and Agent C's 3D scenes will want it as
//! well — but that file is the integration owner's. See the Agent D note in
//! REWRITE_PLAN.md.

// Nothing calls the scene drawing halves yet: `main.rs` still drives only
// Spectrum, and the registry that will dispatch all ten is the integration
// owner's. Remove this once a scene selector exists.
#![allow(dead_code)]

use musializer_core::scene::settings::index::terrarium as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::spectral_terrarium::{
    self as terrarium, SpectralTerrariumState, CREATURE_COUNT, PARTICLE_COUNT, PLANT_COUNT,
};
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Camera3D, Color, RaylibBlendModeExt, RaylibDraw, RaylibDraw3D, RaylibDrawHandle,
    RaylibMode3DExt, RaylibShaderModeExt, Rectangle, Vector2, Vector3,
};

use super::spectrum::CircleShader;

const PI: f32 = std::f32::consts::PI;

/// Restricts 3D rendering to `boundary` without changing the composition inside it.
///
/// Port of the viewport block the C repeats in every 3D scene
/// (`scene_spectral_terrarium.c:555-604`, `scene_constellation.c:180-318`). Three
/// things have to happen together and none of them is optional:
///
/// 1. the GL viewport is narrowed to the panel's pixels, so 3D geometry cannot
///    spill over the UI;
/// 2. rlgl's notion of the framebuffer size is narrowed with it, because
///    `BeginMode3D` derives its projection aspect from that;
/// 3. the projection is then rescaled in X by `full_aspect / viewport_aspect`, so
///    the scene is framed by the *window's* aspect rather than the panel's. Without
///    this a narrow preview panel stretches the world instead of cropping it.
///
/// The batch is flushed before and after, because rlgl defers draw calls and a
/// pending batch would otherwise be rasterized under the wrong viewport.
pub struct SceneViewport {
    saved_width: i32,
    saved_height: i32,
    width: i32,
    height: i32,
}

impl SceneViewport {
    /// Narrows the viewport to `boundary`, or returns `None` when the result would
    /// be degenerate — the same early returns the C takes.
    pub fn begin<D: RaylibDraw>(_d: &mut D, boundary: Rectangle) -> Option<Self> {
        // SAFETY: pure rlgl/raylib getters over global renderer state, valid while a
        // drawing context is active. The `_d` borrow is that proof.
        let (saved_width, saved_height, active_framebuffer, screen_width, screen_height) = unsafe {
            (
                raylib_sys::rlGetFramebufferWidth(),
                raylib_sys::rlGetFramebufferHeight(),
                raylib_sys::rlGetActiveFramebuffer(),
                raylib_sys::GetScreenWidth(),
                raylib_sys::GetScreenHeight(),
            )
        };
        if saved_width < 1 || saved_height < 1 {
            return None;
        }
        // When drawing to the default framebuffer, boundary coordinates are in
        // screen space rather than framebuffer space; a supersampled export path
        // renders into a render texture where the two already agree.
        let (coordinate_width, coordinate_height) = if active_framebuffer == 0 {
            if screen_width < 1 || screen_height < 1 {
                return None;
            }
            (screen_width as f32, screen_height as f32)
        } else {
            (saved_width as f32, saved_height as f32)
        };
        let scale_x = saved_width as f32 / coordinate_width;
        let scale_y = saved_height as f32 / coordinate_height;
        let x = (boundary.x * scale_x).round() as i32;
        let width = (boundary.width * scale_x).round() as i32;
        let height = (boundary.height * scale_y).round() as i32;
        // GL's origin is bottom-left, the boundary's is top-left.
        let top = ((boundary.y + boundary.height) * scale_y).round() as i32;
        let y = saved_height - top;
        if width < 1 || height < 1 {
            return None;
        }

        // SAFETY: rlgl state changes on the render thread while a drawing context is
        // active. The batch is flushed first so nothing already queued is rasterized
        // under the new viewport, and `end` restores every value this touches.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlViewport(x, y, width, height);
            raylib_sys::rlSetFramebufferWidth(width);
            raylib_sys::rlSetFramebufferHeight(height);
        }
        Some(Self {
            saved_width,
            saved_height,
            width,
            height,
        })
    }

    /// Rescales the projection so the panel crops the world instead of stretching
    /// it. Call this immediately inside `BeginMode3D`, as the C does.
    pub fn correct_aspect<D: RaylibDraw3D>(&self, _d: &mut D) {
        let viewport_aspect = self.width as f32 / self.height as f32;
        let full_aspect = self.saved_width as f32 / self.saved_height as f32;
        // SAFETY: matrix-stack calls on the active rlgl context, balanced so the
        // mode is left back on MODELVIEW exactly as `BeginMode3D` left it.
        unsafe {
            raylib_sys::rlMatrixMode(raylib_sys::RL_PROJECTION as i32);
            raylib_sys::rlScalef(full_aspect / viewport_aspect, 1.0, 1.0);
            raylib_sys::rlMatrixMode(raylib_sys::RL_MODELVIEW as i32);
        }
    }

    /// Restores the full viewport. Must be called after `EndMode3D`.
    pub fn end<D: RaylibDraw>(self, _d: &mut D) {
        // SAFETY: restores exactly the values captured in `begin`, after flushing
        // the batch drawn under the narrowed viewport.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlSetFramebufferWidth(self.saved_width);
            raylib_sys::rlSetFramebufferHeight(self.saved_height);
            raylib_sys::rlViewport(0, 0, self.saved_width, self.saved_height);
        }
    }
}

/// Sets the circle shader's two uniforms.
///
/// Agent C's `CircleShader` keeps its setters private to its own module, so this
/// goes through the public location fields. Worth promoting to methods there
/// eventually; not worth editing another agent's file for.
pub(crate) fn set_circle(shader: &mut CircleShader, radius: f32, power: f32) {
    shader
        .shader
        .set_shader_value(shader.radius_location, radius);
    shader.shader.set_shader_value(shader.power_location, power);
}

/// `DrawBillboardRec` over the non-owning default texture.
///
/// The safe API takes an owned `Texture2D`, which would unload raylib's default
/// texture on drop — the same reason `runtime::draw` wraps `DrawTexturePro`.
pub(crate) fn draw_billboard_rec<D: RaylibDraw3D>(
    _d: &mut D,
    camera: Camera3D,
    texture: raylib_sys::Texture2D,
    source: Rectangle,
    position: Vector3,
    size: Vector2,
    tint: Color,
) {
    // SAFETY: an immediate-mode draw call with a valid texture id and by-value
    // geometry, inside an active 3D drawing context proven by `_d`.
    unsafe {
        raylib_sys::DrawBillboardRec(
            camera.into(),
            texture,
            source.into(),
            position.into(),
            size.into(),
            tint.into(),
        );
    }
}

fn vec3(v: terrarium::Vec3) -> Vector3 {
    Vector3::new(v.x, v.y, v.z)
}

/// Draws Spectral Terrarium into `boundary`.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &SpectralTerrariumState,
    shader: &mut CircleShader,
    boundary: Rectangle,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }

    let motion_scale = frame.setting(SceneId::SpectralTerrarium, setting::MOTION);
    let growth_scale = frame.setting(SceneId::SpectralTerrarium, setting::GROWTH);
    let particle_scale = frame.setting(SceneId::SpectralTerrarium, setting::PARTICLES);
    let glass_opacity = frame.setting(SceneId::SpectralTerrarium, setting::GLASS_OPACITY);
    let density = frame.setting(SceneId::SpectralTerrarium, setting::DENSITY);
    let creature_glow = frame.setting(SceneId::SpectralTerrarium, setting::CREATURE_GLOW);

    let semantic_weight = if frame.semantic.available {
        frame.semantic.confidence
    } else {
        0.0
    };
    // The habitat's hue drifts with song time and flux, offset per seed so two
    // terrariums never share a palette.
    let hue = (145.0
        + terrarium::hash_unit(state.seed, 1500) * 125.0
        + frame.time_seconds as f32 * motion_scale * (1.2 + state.flux * 2.5)
        + frame.semantic.valence * 75.0 * semantic_weight)
        % 360.0;
    let background = draw::color_from_hsv(hue, 0.68, 0.035 + state.energy * 0.035);
    d.draw_rectangle_rec(boundary, background);

    let Some(viewport) = SceneViewport::begin(d, boundary) else {
        return;
    };

    let orbit = frame.time_seconds as f32 * 0.075 * motion_scale
        + terrarium::hash_unit(state.seed, 1600) * 2.0 * PI;
    let camera = Camera3D::perspective(
        Vector3::new(
            orbit.cos() * 7.6,
            3.3 + (orbit * 0.7).sin() * 0.45,
            orbit.sin() * 7.6,
        ),
        Vector3::new(0.0, -0.15, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        52.0 + state.flux * 5.0 + state.onset_pulse * 2.0,
    );
    d.draw_mode3D(camera, |mut m3, camera| {
        viewport.correct_aspect(&mut m3);
        draw_world(
            &mut m3,
            frame,
            state,
            shader,
            camera,
            hue,
            WorldScales {
                growth: growth_scale,
                particles: particle_scale,
                glass_opacity,
                density,
                creature_glow,
            },
        );
    });

    viewport.end(d);
    // A faint wash of the background over the whole panel: atmospheric depth, and
    // it also hides the seam at the viewport edge.
    d.draw_rectangle_rec(boundary, draw::color_alpha(background, 0.035));
}

/// The five habitat settings, bundled only to keep [`draw_world`]'s argument list
/// readable; the C passes them individually.
struct WorldScales {
    growth: f32,
    particles: f32,
    glass_opacity: f32,
    density: f32,
    creature_glow: f32,
}

/// `terrarium_draw_world` (`scene_spectral_terrarium.c:359-525`).
#[allow(clippy::needless_range_loop)] // Item indices drive per-item hue and salt
                                      // arithmetic, so keeping them is what makes each line diffable against the C.
fn draw_world<D>(
    d: &mut D,
    frame: &SceneFrame<'_>,
    state: &SpectralTerrariumState,
    shader: &mut CircleShader,
    camera: Camera3D,
    hue: f32,
    scales: WorldScales,
) where
    D: RaylibDraw3D + RaylibDraw + RaylibBlendModeExt + RaylibShaderModeExt + Sized,
{
    let soil = draw::color_from_hsv((hue + 115.0) % 360.0, 0.55, 0.16 + state.bass * 0.08);
    d.draw_cylinder(Vector3::new(0.0, -1.82, 0.0), 4.15, 3.9, 0.22, 48, soil);
    d.draw_circle_3D(
        Vector3::new(0.0, -1.69, 0.0),
        4.0,
        Vector3::new(1.0, 0.0, 0.0),
        90.0,
        draw::color_alpha(draw::color_from_hsv(hue, 0.52, 0.28), 0.55),
    );
    // Each onset rings the soil like a struck bell: the pulse decays while its
    // ripple expands outward across the floor.
    if state.onset_pulse > 0.03 {
        let ripple_radius = 0.45 + (1.0 - state.onset_pulse) * 3.6;
        d.draw_circle_3D(
            Vector3::new(0.0, -1.66, 0.0),
            ripple_radius,
            Vector3::new(1.0, 0.0, 0.0),
            90.0,
            draw::color_alpha(
                draw::color_from_hsv((hue + 45.0) % 360.0, 0.42, 0.85),
                state.onset_pulse * 0.55,
            ),
        );
    }

    let plant_count = ((PLANT_COUNT as f32 * scales.density).ceil() as usize).min(PLANT_COUNT);
    let particle_count =
        ((PARTICLE_COUNT as f32 * scales.density).ceil() as usize).min(PARTICLE_COUNT);
    let creature_count =
        ((CREATURE_COUNT as f32 * scales.density).ceil() as usize).min(CREATURE_COUNT);

    for i in 0..plant_count {
        let plant = &state.plants[i];
        let amplitude = terrarium::band(frame, plant.band as usize);
        let height = plant.height * (0.72 + amplitude * 0.75 + state.bass * 0.18) * scales.growth;
        let sway = (state.simulation_time as f32 * (0.75 + state.flux) + plant.phase).sin()
            * plant.lean
            * (0.45 + amplitude);
        let middle = Vector3::new(
            plant.root.x + sway * 0.42,
            plant.root.y + height * 0.54,
            plant.root.z + plant.phase.cos() * sway * 0.25,
        );
        let tip = Vector3::new(
            plant.root.x + sway,
            plant.root.y + height,
            plant.root.z + plant.phase.sin() * sway * 0.55,
        );
        let stem = draw::color_from_hsv(
            (hue + 72.0 + i as f32 * 2.7) % 360.0,
            0.72,
            0.38 + amplitude * 0.48,
        );
        let stem_radius = 0.012 + amplitude * 0.012;
        draw::tube(
            d,
            vec3(plant.root),
            middle,
            stem_radius,
            6,
            draw::color_alpha(stem, 0.78),
        );
        draw::tube(d, middle, tip, stem_radius * 0.72, 6, stem);
        // A pair of leaf blades branching from mid-stem turns a bare stalk into a
        // plant; they sway with the stem and open with amplitude.
        let leaf_angle = terrarium::hash_unit(state.seed, i as u32 + 1300) * 2.0 * PI;
        let leaf_length = (0.16 + plant.height * 0.14) * (0.7 + amplitude * 0.5) * scales.growth;
        let leaf = draw::color_from_hsv(
            (hue + 95.0 + i as f32 * 3.1) % 360.0,
            0.68,
            0.34 + amplitude * 0.40,
        );
        for blade in 0..2 {
            let direction = leaf_angle + blade as f32 * PI + sway * (0.5 + blade as f32 * 0.3);
            let leaf_tip = Vector3::new(
                middle.x + direction.cos() * leaf_length,
                middle.y + leaf_length * (0.45 + amplitude * 0.30),
                middle.z + direction.sin() * leaf_length,
            );
            draw::tube(
                d,
                middle,
                leaf_tip,
                stem_radius * 0.55,
                5,
                draw::color_alpha(leaf, 0.85),
            );
        }
        d.draw_sphere(
            tip,
            0.055 + amplitude * 0.14 + state.onset_pulse * 0.025,
            draw::color_from_hsv(
                (hue + 145.0 + i as f32 * 8.0) % 360.0,
                0.62,
                0.55 + amplitude * 0.42,
            ),
        );
    }

    for i in 0..particle_count {
        let particle = &state.particles[i];
        let shimmer = 0.5 + 0.5 * (state.simulation_time as f32 * 2.1 + particle.phase).sin();
        let color = draw::color_from_hsv(
            (hue + 35.0 + i as f32 * 4.7) % 360.0,
            0.38,
            0.48 + shimmer * 0.45,
        );
        d.draw_sphere(
            vec3(particle.position),
            particle.size * (0.8 + state.treble * 1.3) * scales.particles,
            draw::color_alpha(color, 0.38 + shimmer * 0.52),
        );
    }

    for i in 0..creature_count {
        let creature = &state.creatures[i];
        let amplitude = terrarium::band(frame, creature.band as usize);
        let position = state.creature_position(creature);
        let velocity_length = creature.velocity.length();
        let tangent = if velocity_length > 0.0001 {
            Vector3::new(
                creature.velocity.x / velocity_length,
                creature.velocity.y / velocity_length,
                creature.velocity.z / velocity_length,
            )
        } else {
            Vector3::new(1.0, 0.0, 0.0)
        };
        let length = 0.18 + amplitude * 0.32;
        let head = Vector3::new(
            position.x + tangent.x * length,
            position.y,
            position.z + tangent.z * length,
        );
        let color = draw::color_from_hsv(
            (hue + 190.0 + i as f32 * 13.0) % 360.0,
            0.58,
            0.58 + amplitude * 0.38,
        );
        // A fading wake behind each swimmer makes the flock's motion legible even
        // in a still frame.
        let wake = Vector3::new(
            position.x - tangent.x * (0.30 + velocity_length * 0.22),
            position.y - tangent.y * (0.30 + velocity_length * 0.22) * 0.4,
            position.z - tangent.z * (0.30 + velocity_length * 0.22),
        );
        draw::tube(
            d,
            wake,
            vec3(position),
            0.008 + amplitude * 0.006,
            5,
            draw::color_alpha(color, 0.30),
        );
        draw::tube(
            d,
            vec3(position),
            head,
            0.014 + amplitude * 0.012,
            6,
            draw::color_alpha(color, 0.8),
        );
        // Beating side fins keyed to the simulation clock give each body a stroke
        // cycle instead of a rigid dart shape.
        let side = Vector3::new(-tangent.z, 0.0, tangent.x);
        let flap =
            (state.simulation_time as f32 * 6.5 + creature.phase).sin() * (0.5 + amplitude * 0.5);
        let fin_length = 0.09 + amplitude * 0.07;
        for fin in 0..2 {
            let fin_side = if fin == 0 { 1.0 } else { -1.0 };
            let fin_tip = Vector3::new(
                position.x + side.x * fin_side * fin_length,
                position.y + flap * fin_side * fin_length * 0.8,
                position.z + side.z * fin_side * fin_length,
            );
            draw::tube(
                d,
                vec3(position),
                fin_tip,
                0.007,
                4,
                draw::color_alpha(color, 0.62),
            );
        }
        d.draw_sphere(head, 0.07 + amplitude * 0.09, color);
        d.draw_sphere(
            vec3(position),
            0.045 + state.energy * 0.035,
            draw::color_alpha(Color::RAYWHITE, 0.72),
        );
    }

    // Creature glow: soft additive camera-facing sprites lift the flock out of the
    // dark habitat the way lanternfish read in deep water.
    if scales.creature_glow > 0.001 {
        let glow_texture = draw::default_texture();
        let glow_source = Rectangle::new(0.0, 0.0, 1.0, 1.0);
        set_circle(shader, 0.06, 2.8);
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
            blend.draw_shader_mode(&mut shader.shader, |mut pass| {
                for i in 0..creature_count {
                    let creature = &state.creatures[i];
                    let amplitude = terrarium::band(frame, creature.band as usize);
                    let position = state.creature_position(creature);
                    let color =
                        draw::color_from_hsv((hue + 190.0 + i as f32 * 13.0) % 360.0, 0.52, 0.9);
                    let halo =
                        (0.26 + amplitude * 0.30 + state.onset_pulse * 0.10) * scales.creature_glow;
                    draw_billboard_rec(
                        &mut pass,
                        camera,
                        glow_texture,
                        glow_source,
                        vec3(position),
                        Vector2::new(halo, halo),
                        draw::color_alpha(color, 0.30 + amplitude * 0.25),
                    );
                }
            });
        });
    }

    // Sparse latitude rings imply a glass habitat without hiding its contents.
    let glass = draw::color_alpha(
        draw::color_from_hsv((hue + 25.0) % 360.0, 0.2, 0.9),
        scales.glass_opacity,
    );
    for ring in 0..4 {
        let y = -1.35 + ring as f32 * 1.15;
        let radius = (16.0f32 - (y + 1.65) * (y + 1.65)).max(0.0).sqrt();
        d.draw_circle_3D(
            Vector3::new(0.0, y, 0.0),
            radius,
            Vector3::new(1.0, 0.0, 0.0),
            90.0,
            glass,
        );
    }
}
