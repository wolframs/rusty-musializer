//! Orbital Lattice: the drawing half.
//!
//! The deterministic half is `musializer_core::scenes::orbital_lattice`, and
//! every damped envelope this file reads was computed there.
//!
//! A convoy of twelve rings receding down a bounded travel path, each ring a
//! sixteen-node constellation linked by swaying filaments. Pearl cores and
//! selective billboard halos make the nodes read as suspended light rather than
//! exposed construction primitives.
//!
//! ## The viewport dance
//!
//! This scene draws 3D into a sub-rectangle of a 2D frame, which raylib does not
//! support directly, so it takes over the GL viewport and puts it back
//! ([`SceneViewport`]). Song Atlas and the other 3D scenes use the same shared
//! machinery from `musializer_runtime::draw`.

#![allow(dead_code)]

use musializer_core::scene::settings::index::orbital as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::orbital_lattice::{OrbitalLatticeState, NODES_PER_RING, RING_COUNT};
use musializer_runtime::draw;
use musializer_runtime::draw::{draw_billboard_rec, SceneViewport};
use raylib::prelude::{
    BlendMode, Camera3D, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibMode3DExt,
    RaylibShaderModeExt, Rectangle, Vector2, Vector3,
};

use super::spectral_terrarium::set_circle;
use super::spectrum::CircleShader;

const PI: f32 = std::f32::consts::PI;

/// `orbital_clamp01` (`scene_orbital_lattice.c:10-15`).
///
/// Note this is **not** the motion module's clamp: it has no `isfinite` test, so a
/// NaN passes straight through both comparisons. Reproduced rather than unified —
/// the two functions have the same name in C and different behaviour, and only the
/// motion one is on the path where a NaN can originate.
fn clamp01(value: f32) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// `orbital_hash` (`scene_orbital_lattice.c:17-28`).
///
/// A different function from the motion module's `orbital_hash` despite the shared
/// name: this one mixes a ring and a node rather than a single salt, so per-node
/// scatter is stable for a given seed.
fn hash(seed: u64, ring: u32, node: u32) -> u32 {
    let mut value = seed;
    value ^= u64::from(ring + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value ^= u64::from(node + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value as u32
}

/// `orbital_hash_unit` (`scene_orbital_lattice.c:30-33`).
fn hash_unit(seed: u64, ring: u32, node: u32) -> f32 {
    (hash(seed, ring, node) & 0xffff) as f32 / 65535.0
}

/// `orbital_time_phase` (`scene_orbital_lattice.c:35-41`): absolute time wrapped to
/// one turn at a fixed rate.
///
/// Absolute rather than integrated, so seeking lands on the same phase.
fn time_phase(time_seconds: f64, radians_per_second: f64) -> f32 {
    if !time_seconds.is_finite() || !radians_per_second.is_finite() {
        return 0.0;
    }
    let mut phase = (time_seconds * radians_per_second) % (2.0 * std::f64::consts::PI);
    if phase < 0.0 {
        phase += 2.0 * std::f64::consts::PI;
    }
    phase as f32
}

/// `ColorBrightness`, which the safe raylib API only exposes for images.
fn color_brightness(color: Color, factor: f32) -> Color {
    let raw = raylib_sys::Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    };
    // SAFETY: pure arithmetic over a by-value colour, no global state.
    let out = unsafe { raylib_sys::ColorBrightness(raw, factor) };
    Color::new(out.r, out.g, out.b, out.a)
}

fn average_color(left: Color, right: Color) -> Color {
    Color::new(
        ((u16::from(left.r) + u16::from(right.r)) / 2) as u8,
        ((u16::from(left.g) + u16::from(right.g)) / 2) as u8,
        ((u16::from(left.b) + u16::from(right.b)) / 2) as u8,
        ((u16::from(left.a) + u16::from(right.a)) / 2) as u8,
    )
}

/// A filament that yields to the emissive field around both endpoint pearls.
///
/// The previous seven-piece bowed tube exposed every cylinder cap at a slightly
/// different angle. In motion those caps read as intermittent rectangular kinks.
/// This link stays collinear, trims itself by each pearl's glow radius, and uses
/// two short tapers around one continuous middle span. The surrounding billboard
/// glow supplies the visual fade while the geometry never crosses the hot core.
fn draw_glow_cleared_link<D: draw::RaylibDraw3D>(
    d: &mut D,
    from: Vector3,
    to: Vector3,
    from_clearance: f32,
    to_clearance: f32,
    radius: f32,
    color: Color,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;
    let length = (dx * dx + dy * dy + dz * dz).sqrt();
    if !length.is_finite() || length <= 1.0e-5 {
        return;
    }

    // Never let two unusually broad halos erase the complete edge. Scaling the
    // clearances together preserves which endpoint has the larger glow.
    let requested = from_clearance.max(0.0) + to_clearance.max(0.0);
    let clearance_scale = if requested > length * 0.72 {
        length * 0.72 / requested
    } else {
        1.0
    };
    let start_t = from_clearance.max(0.0) * clearance_scale / length;
    let end_t = 1.0 - to_clearance.max(0.0) * clearance_scale / length;
    if end_t <= start_t {
        return;
    }

    let point = |t: f32| Vector3::new(from.x + dx * t, from.y + dy * t, from.z + dz * t);
    let span = end_t - start_t;
    let start = point(start_t);
    let full_start = point(start_t + span * 0.16);
    let full_end = point(end_t - span * 0.16);
    let end = point(end_t);
    d.draw_cylinder_ex(start, full_start, 0.0, radius, 6, color);
    d.draw_cylinder_ex(full_start, full_end, radius, radius, 6, color);
    d.draw_cylinder_ex(full_end, end, radius, 0.0, 6, color);
}

fn glow_clearance(radius: f32, amplitude: f32, pulse: f32) -> f32 {
    radius * (2.25 + amplitude * 1.75 + pulse * 0.75)
}

/// Draws Orbital Lattice into `boundary`.
///
/// The node loop indexes `node_bands` by a rotated index rather than iterating, so
/// each loop diffs cleanly against the C it came from.
#[allow(clippy::needless_range_loop)]
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    state: &OrbitalLatticeState,
    frame: &SceneFrame<'_>,
    shader: &mut CircleShader,
    boundary: Rectangle,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }
    let lattice = state.motion();

    let bass = clamp01(lattice.bass);
    let mids = clamp01(lattice.mids);
    let treble = clamp01(lattice.treble);
    let energy = clamp01(lattice.energy);
    let flux = clamp01(lattice.flux);
    let pulse = clamp01(lattice.onset_pulse);
    let radius_scale = frame.setting(SceneId::OrbitalLattice, setting::RADIUS);
    let depth_scale = frame.setting(SceneId::OrbitalLattice, setting::DEPTH);
    let node_scale = frame.setting(SceneId::OrbitalLattice, setting::NODES);
    let link_scale = frame.setting(SceneId::OrbitalLattice, setting::LINKS);
    let tilt = frame.setting(SceneId::OrbitalLattice, setting::TILT);
    let hue_shift = frame.setting(SceneId::OrbitalLattice, setting::HUE);
    // Reactivity couples the current (damped) audio into motion as additive
    // offsets, and sway drives the audio-independent wander. Neither feeds the
    // accumulating phase, so seeking still lands on the same deterministic orbit.
    let reactivity = frame.setting(SceneId::OrbitalLattice, setting::REACTIVITY);
    let sway = frame.setting(SceneId::OrbitalLattice, setting::SWAY);
    let seed_phase = hash_unit(lattice.seed, 0, 0) * 2.0 * PI;
    let hue_base =
        (lattice.hue_degrees as f32 + hue_shift + lattice.semantic_valence * 55.0 + 720.0) % 360.0;

    let background = draw::color_from_hsv(hue_base, 0.58, 0.038 + energy * 0.032);
    let horizon = draw::color_from_hsv(
        (hue_base + 26.0) % 360.0,
        0.68 + lattice.semantic_tension * 0.12,
        0.090 + energy * 0.060,
    );
    draw::atmospheric_backdrop(
        d,
        boundary,
        background,
        horizon,
        Vector2::new(
            boundary.x + boundary.width * 0.52,
            boundary.y + boundary.height * 0.49,
        ),
        boundary.width.max(boundary.height) * 0.52,
        draw::color_alpha(
            draw::color_from_hsv((hue_base + 325.0) % 360.0, 0.60, 0.27),
            0.62,
        ),
    );

    let screen_width = d.get_screen_width();
    let screen_height = d.get_screen_height();
    // Declared before the 3D handle so it is dropped after it: `EndMode3D` must
    // run while the clipped viewport is still in effect, exactly as in the C.
    let Some(viewport) = SceneViewport::begin_with_screen(boundary, screen_width, screen_height)
    else {
        return;
    };

    // Tilt swings the camera off the tunnel axis so the ring stack reads as
    // receding 3D geometry; zero restores the original head-on framing.
    let camera_orbit = lattice.camera_phase as f32;
    let camera_radius = 0.42 + mids * 0.16 + tilt * (2.5 + mids * 0.5);
    let camera_lift = (camera_orbit + seed_phase * 0.37).sin() * (0.24 + treble * 0.09)
        + tilt * (1.15 + (camera_orbit * 0.7).sin() * 0.45)
        + reactivity * flux * 0.14 * (camera_orbit * 1.3 + seed_phase).sin();
    // `Camera3D::perspective` is raylib-rs's constructor for
    // `.projection = CAMERA_PERSPECTIVE`, which the C sets by hand.
    let camera = Camera3D::perspective(
        Vector3::new(
            camera_orbit.cos() * camera_radius,
            camera_lift,
            9.65 - bass * 0.18 - pulse * 0.08,
        ),
        Vector3::new(0.0, 0.0, -7.4 * depth_scale),
        Vector3::new(0.0, 1.0, 0.0),
        50.0 + flux * 1.2 + pulse * 0.4,
    );

    {
        let mut space = d.begin_mode3D(camera);
        viewport.correct_projection_aspect();

        // The old renderer exposed its construction vocabulary — literal cubes,
        // facets and wire boxes.  Keep the lattice and its audio motion, but give
        // the nodes one coherent material: pearl-like cores suspended in a soft
        // emissive field.
        let mut lanterns = Vec::with_capacity(RING_COUNT * NODES_PER_RING);

        let breathe_phase = time_phase(frame.time_seconds, 0.55);
        let drift_x_phase = time_phase(frame.time_seconds, 0.17);
        let drift_y_phase = time_phase(frame.time_seconds, 0.13);
        // Far rings first so the nearer pearls and filaments establish the final
        // depth hierarchy.
        for ring in (0..RING_COUNT).rev() {
            let Some(ring_motion) = lattice.ring(ring) else {
                continue;
            };
            if ring_motion.visibility < 0.01 {
                continue;
            }
            let depth_t = ring_motion.depth_t;
            let z = 3.0 - ring_motion.distance * depth_scale;
            let ring_character = hash_unit(lattice.seed, ring as u32, 0xa7);

            // Beats and flux nudge each ring's rotation and breathing without ever
            // entering the accumulating twist_phase, so the swing is reactive yet
            // seek-stable. Reactivity scales the audio kick; sway the free wander.
            let twist_kick =
                reactivity * (flux * 0.55 + pulse * 0.32) * (seed_phase + ring as f32 * 0.9).sin();
            let twist = lattice.twist_phase as f32
                + ring as f32 * 0.23
                + (ring_character - 0.5) * 0.22
                + twist_kick;
            let ring_wave = (breathe_phase - ring as f32 * 0.55 + seed_phase).sin();
            let radius =
                (3.12 + bass * 0.48 + ring_wave * (0.14 + pulse * 0.10 + reactivity * 0.08))
                    * radius_scale;
            let center_x = (drift_x_phase + ring as f32 * 0.37 + seed_phase).sin() * 0.34 * sway;
            let center_y = (drift_y_phase - ring as f32 * 0.29 + seed_phase).cos() * 0.24 * sway;

            let mut first = Vector3::zero();
            let mut previous = Vector3::zero();
            let mut first_clearance = 0.0;
            let mut previous_clearance = 0.0;
            let mut first_edge = Color::BLANK;
            let mut previous_edge = Color::BLANK;
            for node in 0..NODES_PER_RING {
                let node_t = node as f32 / NODES_PER_RING as f32;
                let angle = node_t * 2.0 * PI + twist;
                // The band each node reads is rotated by the ring, so a single
                // loud band does not light up one radial spoke all the way down
                // the tunnel.
                let band_index = (node + ring * 3) % NODES_PER_RING;
                let amplitude = clamp01(lattice.node_bands[band_index]);
                let scatter = hash_unit(lattice.seed, ring as u32, node as u32) - 0.5;
                // Onsets shove every node outward together; flux ripples them out of
                // phase around the ring. Both read as the lattice "breathing" to the
                // beat and vanish smoothly when reactivity is dialed down.
                let node_bounce = reactivity
                    * (pulse * 0.24 + flux * 0.18 * (seed_phase + node_t * 2.0 * PI).sin());
                let radial = radius
                    + amplitude * (0.24 + energy * 0.42 + reactivity * 0.22)
                    + scatter * 0.10
                    + node_bounce;
                let position = Vector3::new(
                    center_x + angle.cos() * radial,
                    center_y + angle.sin() * radial,
                    z + (angle * 2.0 + seed_phase).sin() * 0.07 * (0.4 + treble),
                );

                let fog = (1.0 - depth_t * 0.66) * ring_motion.visibility;
                let hue = (hue_base + node_t * 105.0 + ring as f32 * 5.0) % 360.0;
                // A directional value bias keeps the ring from reading as one
                // flat tone even before the emissive pass.
                let shade = 0.74 + 0.26 * (angle - 2.35).cos();
                let color = draw::color_from_hsv(
                    hue,
                    0.64 + amplitude * 0.28,
                    clamp01(fog * shade * (0.72 + amplitude * 0.48)),
                );
                let color = draw::color_alpha(color, ring_motion.visibility);
                let size = (0.10 + amplitude * 0.23 + energy * 0.06 + pulse * 0.04) * node_scale;
                let pearl_radius = size * (0.48 + bass * 0.10 + amplitude * 0.10);
                lanterns.push((position, pearl_radius, color, amplitude, fog));
                let clearance = glow_clearance(pearl_radius, amplitude, pulse);
                let edge =
                    draw::color_alpha(color, fog * (0.43 + energy * 0.27) * 1.0f32.min(link_scale));

                if node == 0 {
                    first = position;
                    first_clearance = clearance;
                    first_edge = edge;
                }
                if node > 0 && link_scale > 0.001 {
                    draw_glow_cleared_link(
                        &mut space,
                        previous,
                        position,
                        previous_clearance,
                        clearance,
                        (0.0085 + energy * 0.007) * link_scale,
                        average_color(previous_edge, edge),
                    );
                }
                previous = position;
                previous_clearance = clearance;
                previous_edge = edge;
            }
            // Close the loop with the same endpoint material as every other
            // edge; a white seam advertised the ring's implementation detail.
            if link_scale > 0.001 {
                draw_glow_cleared_link(
                    &mut space,
                    previous,
                    first,
                    previous_clearance,
                    first_clearance,
                    (0.0085 + energy * 0.007) * link_scale,
                    average_color(previous_edge, first_edge),
                );
            }
        }

        // One additive pass turns the pearls into light sources.  The analytic
        // circle texture stays smooth under both preview MSAA and export
        // supersampling; depth fog keeps the tunnel readable instead of making
        let glow_texture = draw::default_texture();
        let glow_source = Rectangle::new(0.0, 0.0, 1.0, 1.0);
        set_circle(shader, 0.055, 2.8);
        space.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
            blend.draw_shader_mode(&mut shader.shader, |mut pass| {
                for (index, &(position, radius, color, amplitude, fog)) in
                    lanterns.iter().enumerate()
                {
                    // Broad glow is selective; every pearl gets a hot core, but
                    // only a stable subset and genuinely active bands illuminate
                    // the surrounding haze. This creates hierarchy and keeps the
                    // software-GL headless path comfortably inside frame budget.
                    if index % 3 == 0 || amplitude > 0.18 {
                        let diameter = radius * (6.4 + amplitude * 4.2 + pulse * 1.8);
                        draw_billboard_rec(
                            &mut pass,
                            camera,
                            glow_texture,
                            glow_source,
                            position,
                            Vector2::new(diameter, diameter),
                            draw::color_alpha(color, fog * (0.58 + amplitude * 0.42)),
                        );
                    }
                    let core = radius * (2.5 + amplitude * 1.3);
                    draw_billboard_rec(
                        &mut pass,
                        camera,
                        glow_texture,
                        glow_source,
                        position,
                        Vector2::new(core, core),
                        draw::color_alpha(Color::RAYWHITE, fog * (0.82 + amplitude * 0.18)),
                    );
                }
            });
        });
    }
    drop(viewport);

    // A faint veil of the background colour over the whole scene: it lifts the
    // black point so a quiet passage is not a black rectangle, and it is drawn
    // *after* the viewport is restored so it covers the full boundary.
    let veil = draw::color_alpha(background, 0.025 + (1.0 - energy) * 0.025);
    d.draw_rectangle_rec(boundary, veil);
    draw::vignette(d, boundary, 0.22);
}
