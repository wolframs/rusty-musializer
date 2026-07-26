//! Orbital Lattice: the drawing half.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_orbital_lattice.c`'s
//! `draw` (frozen at `9300af9`, read-only). The deterministic half is
//! `musializer_core::scenes::orbital_lattice`, and every damped envelope this
//! file reads was computed there.
//!
//! A convoy of twelve rings receding down a bounded travel path, each ring a
//! sixteen-node cube lattice linked by swaying tubes. The camera path is a
//! function of seed and transport time only; audio moves the geometry.
//!
//! Formulas and draw-call order are kept recognizable against the C on purpose.
//!
//! ## The viewport dance
//!
//! This scene draws 3D into a sub-rectangle of a 2D frame, which raylib does not
//! support directly, so it takes over the GL viewport and puts it back
//! ([`SceneViewport`]). That is genuinely shared machinery — Song Atlas uses it
//! and several of Agent D's scenes will too — and it belongs in
//! `musializer_runtime::draw` next to [`draw::tube`]. It is here only because
//! `musializer-runtime` is not Agent C's to edit; see the note in
//! REWRITE_PLAN.md asking the integration owner to hoist it.

// Nothing dispatches this yet: `main.rs` still calls Spectrum directly and the
// scene registry that will call all ten is the integration owner's. Remove this
// allow once the frame loop dispatches by `SceneId` — every item here is reached
// from `draw`, and `SceneViewport` is also used by Song Atlas.
#![allow(dead_code)]

use musializer_core::scene::settings::index::orbital as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::orbital_lattice::{OrbitalLatticeState, NODES_PER_RING, RING_COUNT};
use musializer_runtime::draw;
use raylib::prelude::{
    Camera3D, Color, RaylibDraw, RaylibDrawHandle, RaylibMode3DExt, Rectangle, Vector3,
};
// Aliased because `musializer_runtime::draw` has its own `RaylibDraw3D` — the
// narrow capability `draw::tube` needs. Both must be in scope: this one for the
// cube calls, that one for the tube.
use raylib::prelude::RaylibDraw3D as RlDraw3D;

const PI: f32 = std::f32::consts::PI;

/// Segments per swaying link (`scene_orbital_lattice.c:45`).
const LINK_SEGMENTS: i32 = 7;

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

/// A borrowed GL viewport restricted to one sub-rectangle of the frame.
///
/// Port of the viewport preamble and epilogue in `scene_orbital_lattice.c:128-159`
/// and `302-308`. Two things there are load-bearing and neither is obvious:
///
/// - **UI boundaries are top-left logical window coordinates; GL viewports are
///   bottom-left framebuffer pixels.** Render textures are already in framebuffer
///   pixels, so only the default framebuffer needs the DPI scale.
/// - **rlgl's framebuffer size must be changed too**, not just the viewport,
///   because its batch and stereo paths treat it as authoritative.
///
/// Restores on drop, so a `?` or an early return cannot leave the viewport
/// clipped for the UI drawn afterwards. Declare it *before* the `Mode3D` handle so
/// `EndMode3D` runs first, matching the C's order.
pub struct SceneViewport {
    saved_width: i32,
    saved_height: i32,
    width: i32,
    height: i32,
}

impl SceneViewport {
    /// Clips the GL viewport to `boundary`, or returns `None` for anything
    /// degenerate — which is every one of C's early returns in that preamble.
    #[must_use]
    pub fn begin(boundary: Rectangle, screen_width: i32, screen_height: i32) -> Option<Self> {
        // SAFETY: rlgl getters over global state, valid inside a drawing context,
        // which every caller of this module is already required to hold.
        let (saved_width, saved_height, active_framebuffer) = unsafe {
            (
                raylib_sys::rlGetFramebufferWidth(),
                raylib_sys::rlGetFramebufferHeight(),
                raylib_sys::rlGetActiveFramebuffer(),
            )
        };
        if saved_width < 1 || saved_height < 1 {
            return None;
        }

        let mut coordinate_width = saved_width as f32;
        let mut coordinate_height = saved_height as f32;
        if active_framebuffer == 0 {
            if screen_width < 1 || screen_height < 1 {
                return None;
            }
            coordinate_width = screen_width as f32;
            coordinate_height = screen_height as f32;
        }

        let scale_x = saved_width as f32 / coordinate_width;
        let scale_y = saved_height as f32 / coordinate_height;
        let viewport_x = (boundary.x * scale_x).round() as i32;
        let width = (boundary.width * scale_x).round() as i32;
        let height = (boundary.height * scale_y).round() as i32;
        let viewport_top = ((boundary.y + boundary.height) * scale_y).round() as i32;
        let viewport_y = saved_height - viewport_top;
        if width < 1 || height < 1 {
            return None;
        }

        // SAFETY: rlgl state setters. `rlDrawRenderBatchActive` first, because the
        // pending 2D batch must be flushed *before* the viewport it was recorded
        // against changes.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlViewport(viewport_x, viewport_y, width, height);
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

    /// Corrects the projection for this sub-viewport
    /// (`scene_orbital_lattice.c:183-190`).
    ///
    /// raylib 5.5's `BeginMode3D` reads its own full-target aspect rather than
    /// rlgl's framebuffer dimensions, so the perspective it sets up is wrong for a
    /// clipped viewport. Only the X axis is corrected, which centres and
    /// proportions it without touching the field of view.
    ///
    /// Call this immediately after entering 3D mode.
    pub fn correct_projection_aspect(&self) {
        let target_aspect = self.width as f32 / self.height as f32;
        let full_aspect = self.saved_width as f32 / self.saved_height as f32;
        // SAFETY: rlgl matrix stack calls inside an active 3D mode, which the
        // caller is required to have entered.
        unsafe {
            raylib_sys::rlMatrixMode(raylib_sys::RL_PROJECTION as i32);
            raylib_sys::rlScalef(full_aspect / target_aspect, 1.0, 1.0);
            raylib_sys::rlMatrixMode(raylib_sys::RL_MODELVIEW as i32);
        }
    }

    /// Scales the model-view matrix, for scenes that stretch their whole world.
    pub fn scale_modelview(&self, x: f32, y: f32, z: f32) {
        // SAFETY: an rlgl matrix call inside an active 3D mode.
        unsafe { raylib_sys::rlScalef(x, y, z) }
    }
}

impl Drop for SceneViewport {
    fn drop(&mut self) {
        // SAFETY: the mirror image of `begin`. The flush comes first because the
        // 3D batch was recorded against the clipped viewport and must be drawn
        // before the full-frame contract is restored for subsequent 2D UI.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlSetFramebufferWidth(self.saved_width);
            raylib_sys::rlSetFramebufferHeight(self.saved_height);
            raylib_sys::rlViewport(0, 0, self.saved_width, self.saved_height);
        }
    }
}

/// `orbital_draw_swaying_link` (`scene_orbital_lattice.c:43-60`).
///
/// A parabolic arch of tubes rather than one straight line: `arch` peaks at the
/// midpoint, so links bow outward and the lattice reads as suspended rather than
/// welded.
fn draw_swaying_link<D: draw::RaylibDraw3D>(
    d: &mut D,
    from: Vector3,
    to: Vector3,
    radius: f32,
    sway: f32,
    phase: f32,
    color: Color,
) {
    let mut previous = from;
    for segment in 1..=LINK_SEGMENTS {
        let t = segment as f32 / LINK_SEGMENTS as f32;
        let arch = 4.0 * t * (1.0 - t);
        let point = Vector3::new(
            from.x + (to.x - from.x) * t,
            from.y + (to.y - from.y) * t + arch * sway,
            from.z + (to.z - from.z) * t + arch * sway * 0.45 * (phase + t * PI).sin(),
        );
        draw::tube(d, previous, point, radius, 6, color);
        previous = point;
    }
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

    let background = draw::color_from_hsv(
        hue_base,
        0.58 + lattice.semantic_tension * 0.16,
        0.055 + energy * 0.035,
    );
    d.draw_rectangle_rec(boundary, background);

    let screen_width = d.get_screen_width();
    let screen_height = d.get_screen_height();
    // Declared before the 3D handle so it is dropped after it: `EndMode3D` must
    // run while the clipped viewport is still in effect, exactly as in the C.
    let Some(viewport) = SceneViewport::begin(boundary, screen_width, screen_height) else {
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
            10.9 - bass * 0.28 - pulse * 0.12,
        ),
        Vector3::new(0.0, 0.0, -8.5 * depth_scale),
        Vector3::new(0.0, 1.0, 0.0),
        55.0 + flux * 2.3 + pulse * 0.8,
    );

    {
        let mut space = d.begin_mode3D(camera);
        viewport.correct_projection_aspect();

        let breathe_phase = time_phase(frame.time_seconds, 0.55);
        let drift_x_phase = time_phase(frame.time_seconds, 0.17);
        let drift_y_phase = time_phase(frame.time_seconds, 0.13);
        // Far rings first: painting back to front is what makes the near cubes
        // occlude cleanly under additive facets and wire overlays.
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

                let fog = (1.0 - depth_t * 0.78) * ring_motion.visibility;
                let hue = (hue_base + node_t * 105.0 + ring as f32 * 5.0) % 360.0;
                // Directional key light from the upper left so cubes shade around
                // the ring instead of rendering as one flat tone.
                let shade = 0.74 + 0.26 * (angle - 2.35).cos();
                let color = draw::color_from_hsv(
                    hue,
                    0.64 + amplitude * 0.28,
                    clamp01(fog * shade * (0.60 + amplitude * 0.44)),
                );
                let color = draw::color_alpha(color, ring_motion.visibility);
                let size = (0.10 + amplitude * 0.23 + energy * 0.06 + pulse * 0.04) * node_scale;
                let cube_size = Vector3::new(
                    size * (0.88 + bass * 0.30),
                    size * (1.0 + amplitude * 0.55),
                    size * (1.45 + treble * 0.65),
                );
                space.draw_cube_v(position, cube_size, color);
                let facet_position = Vector3::new(
                    position.x - size * 0.10,
                    position.y + size * 0.13,
                    position.z + size * 0.08,
                );
                let facet_size =
                    Vector3::new(cube_size.x * 0.48, cube_size.y * 0.42, cube_size.z * 0.36);
                space.draw_cube_v(
                    facet_position,
                    facet_size,
                    draw::color_alpha(color_brightness(color, 0.34), 0.34 + amplitude * 0.34),
                );
                if ring_motion.distance < 11.0 || amplitude > 0.55 {
                    space.draw_cube_wires_v(
                        position,
                        cube_size,
                        draw::color_alpha(Color::RAYWHITE, fog * 0.32),
                    );
                }

                if node == 0 {
                    first = position;
                }
                if node > 0 && link_scale > 0.001 {
                    let edge = draw::color_alpha(
                        color,
                        fog * (0.22 + energy * 0.20) * 1.0f32.min(link_scale),
                    );
                    draw_swaying_link(
                        &mut space,
                        previous,
                        position,
                        (0.0085 + energy * 0.007) * link_scale,
                        (0.025 + flux * 0.11) * (angle + breathe_phase).sin(),
                        angle + seed_phase,
                        edge,
                    );
                }
                previous = position;
            }
            // The closing link back to node 0, drawn white rather than in the
            // ring's hue so the ring reads as a closed loop.
            if link_scale > 0.001 {
                draw_swaying_link(
                    &mut space,
                    previous,
                    first,
                    (0.0085 + energy * 0.007) * link_scale,
                    (0.025 + flux * 0.11) * (twist + breathe_phase).sin(),
                    twist,
                    draw::color_alpha(
                        Color::RAYWHITE,
                        1.0f32.min(link_scale) * (1.0 - depth_t) * ring_motion.visibility * 0.24,
                    ),
                );
            }
        }
    }
    drop(viewport);

    // A faint veil of the background colour over the whole scene: it lifts the
    // black point so a quiet passage is not a black rectangle, and it is drawn
    // *after* the viewport is restored so it covers the full boundary.
    let veil = draw::color_alpha(background, 0.08 + (1.0 - energy) * 0.07);
    d.draw_rectangle_rec(boundary, veil);
}
