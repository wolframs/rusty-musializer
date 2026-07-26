//! Pentagram Orbits: the Lyness phase portrait, traced deterministically.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_pentagram.c`. Drawing lives
//! in `musializer-app::scenes::pentagram`.
//!
//! The scene draws the phase portrait of the Lyness recurrence
//!
//! ```text
//! y[k+1] = (1 + y[k]) / y[k-1]
//! ```
//!
//! whose phase-plane map `(x, y) -> (y, (1 + y)/x)` returns every positive orbit
//! to its start after exactly five steps (Zamolodchikov periodicity of the A2
//! Y-system). Each orbit is five stations on one level curve of the conserved
//! quantity `K = (x+1)(y+1)(x+y+1)/(x*y)`; the chords between consecutive stations
//! inscribe a pentagram in that oval. Music drives the hop around the five-cycle,
//! so every traversal closes exactly.
//!
//! Everything here is computed once at init from the seed and is then a pure
//! function of the frame — nothing is integrated over time, which is exactly why
//! seeking and offline export land on identical frames.
//!
//! One setting default matters and is not a typo: `settings.pentagram.hue`
//! defaults to **-91 degrees**, the only nonzero hue default of the ten scenes.

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

/// `scene_pentagram.c:18-23`
pub const CURVE_CAPACITY: usize = 12;
pub const CURVE_SAMPLES: usize = 96;
pub const ORBIT_CAPACITY: usize = 24;
pub const ORBIT_PERIOD: usize = 5;

/// `ln(phi)`. The unique fixed point of the Lyness map is `x = y = phi`, which in
/// centered log coordinates places the whole nest around the origin
/// (`scene_pentagram.c:27`).
pub const LOG_PHI: f32 = 0.481_211_83;

/// How far above `min(ln K)` the outermost traced level curve sits (`:29`).
pub const LEVEL_SPREAD: f32 = 1.75;

/// A point in the centered log phase plane. See the note on `Vec3` in
/// `spectral_terrarium`: core has no shared vector type yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// `scene_pentagram.c:31-41`
#[derive(Clone, Debug, PartialEq)]
pub struct PentagramState {
    pub seed: u64,
    /// Level curves of `K` in centered log coordinates, innermost first.
    pub curves: [[Vec2; CURVE_SAMPLES]; CURVE_CAPACITY],
    /// Five phase-plane stations per orbit, in visiting order.
    pub stations: [[Vec2; ORBIT_PERIOD]; ORBIT_CAPACITY],
    pub orbit_offset: [f32; ORBIT_CAPACITY],
    pub orbit_rate: [f32; ORBIT_CAPACITY],
    pub orbit_depth: [f32; ORBIT_CAPACITY],
    pub extent: f32,
}

/// `pentagram_clamp01` (`:43-48`).
#[must_use]
pub fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `pentagram_mix` (`:50-58`).
#[must_use]
pub fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

/// `pentagram_unit` (`:60-64`).
#[must_use]
pub fn unit(seed: u64, salt: u64) -> f32 {
    (mix(seed ^ salt.wrapping_add(0x9e37_79b9_7f4a_7c15)) & 0xffff) as f32 / 65535.0
}

/// `ln K` with `x = e^u`, `y = e^v` (`pentagram_log_invariant`, `:66-75`).
///
/// Every term is a softplus or log-sum-exp minus a linear part, so the function is
/// convex with a single minimum at `u = v = ln(phi)`. A ray from that minimum
/// crosses each level set exactly once, which is the property [`level_radius`]
/// depends on.
#[must_use]
pub fn log_invariant(u: f32, v: f32) -> f32 {
    let x = u.exp();
    let y = v.exp();
    (1.0 + x).ln() + (1.0 + y).ln() + (1.0 + x + y).ln() - u - v
}

/// Distance from the fixed point along a ray at which `ln K` reaches `target`
/// (`pentagram_level_radius`, `:77-97`).
///
/// A doubling search followed by 34 bisections. Convexity guarantees a unique
/// crossing, so no refinement beyond bisection is needed.
#[must_use]
pub fn level_radius(cos_a: f32, sin_a: f32, target: f32) -> f32 {
    let mut low = 0.0f32;
    let mut high = 0.25f32;
    while high < 8.0 && log_invariant(LOG_PHI + cos_a * high, LOG_PHI + sin_a * high) < target {
        low = high;
        high *= 2.0;
    }
    for _ in 0..34 {
        let mid = 0.5 * (low + high);
        if log_invariant(LOG_PHI + cos_a * mid, LOG_PHI + sin_a * mid) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// `pentagram_level_target` (`:99-102`).
#[must_use]
pub fn level_target(depth: f32, floor_level: f32) -> f32 {
    floor_level + LEVEL_SPREAD * clamp01(depth).powf(1.35)
}

/// `pentagram_band` (`:158-162`) — one band or trail value, wrapped.
#[must_use]
pub fn band(values: &[f32], index: usize) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    clamp01(values[index % values.len()])
}

/// `pentagram_project` (`:164-171`) — rotate about the origin, scale, translate.
#[must_use]
pub fn project(point: Vec2, center: Vec2, cos_r: f32, sin_r: f32, scale: f32) -> Vec2 {
    Vec2::new(
        center.x + (point.x * cos_r - point.y * sin_r) * scale,
        center.y + (point.x * sin_r + point.y * cos_r) * scale,
    )
}

/// Smoothed band energy sampled by angle around the nest (`pentagram_trail_at`,
/// `:179-201`).
///
/// Two things here are deliberate. The mirror map folds the circle so bass and
/// treble meet seamlessly instead of jumping at an angular seam. The triangular
/// window low-passes the contour: per-band jitter would scribble the curve with one
/// wiggle per band, while the windowed average yields a few smooth spectral lobes
/// that breathe with the mix.
#[must_use]
pub fn trail_at(trails: &[f32], angle: f32) -> f32 {
    let count = trails.len() as i32;
    if count <= 0 {
        return 0.0;
    }
    let mut turns = angle / (2.0 * std::f32::consts::PI);
    turns -= turns.floor();
    let mirrored = 1.0 - (2.0 * turns - 1.0).abs();
    let position = mirrored * (count - 1) as f32;
    let window = 1.0f32.max(count as f32 / 6.0);
    let mut total = 0.0f32;
    let mut weight_sum = 0.0f32;
    let first = (position - window).floor() as i32;
    let last = (position + window).ceil() as i32;
    for band in first..=last {
        let weight = 1.0 - (band as f32 - position).abs() / (window + 1.0);
        if weight <= 0.0 {
            continue;
        }
        let mut index = if band < 0 { -band } else { band };
        if index >= count {
            // Reflect at the top edge, mirroring the fold at the bottom.
            index = 2 * (count - 1) - index;
        }
        if index < 0 || index >= count {
            continue;
        }
        total += clamp01(trails[index as usize]) * weight;
        weight_sum += weight;
    }
    if weight_sum > 0.0 {
        total / weight_sum
    } else {
        0.0
    }
}

/// Radial displacement, in log-space units, that bends the invariant geometry into
/// the shape of the current spectrum (`pentagram_shape`, `:207-214`).
///
/// Outer structures flex more than inner ones, and spectral flux sharpens the
/// excursion on hits. Everything is a pure function of the current frame, so
/// seeking and export stay exact.
#[must_use]
pub fn shape(frame: &SceneFrame<'_>, angle: f32, depth: f32, coupling: f32) -> f32 {
    let trail = trail_at(frame.audio.trails, angle);
    let flux = clamp01(frame.audio.spectral_flux);
    coupling * trail * (0.12 + 0.38 * clamp01(depth)) * (0.75 + flux * 0.50)
}

/// `pentagram_flex` (`:216-222`) — push a point radially by `disp`.
#[must_use]
pub fn flex(point: Vec2, disp: f32) -> Vec2 {
    let radius = (point.x * point.x + point.y * point.y).sqrt();
    if radius <= 0.0005 || !disp.is_finite() {
        return point;
    }
    let factor = (radius + disp) / radius;
    Vec2::new(point.x * factor, point.y * factor)
}

/// Dwell on a station, then hop (`pentagram_hop_ease`, `:238-242`).
///
/// A smoothstep of a smoothstep keeps the spark parked near integer positions and
/// quick across the chord between them.
#[must_use]
pub fn hop_ease(fraction: f32) -> f32 {
    let smooth = fraction * fraction * (3.0 - 2.0 * fraction);
    smooth * smooth * (3.0 - 2.0 * smooth)
}

/// Position along the five-cycle for one orbit at one moment (`:354-356`,
/// `:386-393`).
///
/// A pure function of seed and time, which is what makes a seek land on the same
/// frame the export produces.
#[must_use]
pub fn orbit_hops(state: &PentagramState, orbit: usize, time: f32, motion: f32) -> f32 {
    time * motion * (0.42 + state.orbit_rate[orbit] * 0.38) + state.orbit_offset[orbit]
}

/// The station a spark is currently leaving (`:356`).
///
/// The C casts a possibly-negative `float` through `uint64_t`, which is undefined
/// for negatives; `motion` and `orbit_offset` are both non-negative so `hops`
/// cannot be negative in practice. The `max(0.0)` here makes that explicit rather
/// than relying on it.
#[must_use]
pub fn active_station(hops: f32) -> usize {
    (hops.max(0.0) as u64 % ORBIT_PERIOD as u64) as usize
}

impl Default for PentagramState {
    fn default() -> Self {
        Self {
            seed: 0,
            curves: [[Vec2::default(); CURVE_SAMPLES]; CURVE_CAPACITY],
            stations: [[Vec2::default(); ORBIT_PERIOD]; ORBIT_CAPACITY],
            orbit_offset: [0.0; ORBIT_CAPACITY],
            orbit_rate: [0.0; ORBIT_CAPACITY],
            orbit_depth: [0.0; ORBIT_CAPACITY],
            extent: 0.0,
        }
    }
}

impl PentagramState {
    /// `pentagram_init` (`:104-156`).
    ///
    /// Traces every level curve at fixed angles, then seeds one phase point per
    /// orbit and lets the recurrence itself place the remaining four stations. The
    /// samples are traced at exactly the angles the draw loop later asks for, so the
    /// spectral contour lands on the curve without any refit.
    #[must_use]
    #[allow(clippy::needless_range_loop)] // Curve and sample indices drive the
                                          // angle and salt arithmetic; the index loops are what make this diffable.
    pub fn new(seed: u64) -> Self {
        let mut state = Self {
            seed,
            ..Self::default()
        };

        let floor_level = log_invariant(LOG_PHI, LOG_PHI);
        let mut extent = 0.0f32;
        for curve in 0..CURVE_CAPACITY {
            let depth = (curve as f32 + 0.6) / CURVE_CAPACITY as f32;
            let target = level_target(depth, floor_level);
            for sample in 0..CURVE_SAMPLES {
                let angle = sample as f32 * (2.0 * std::f32::consts::PI / CURVE_SAMPLES as f32);
                let cos_a = angle.cos();
                let sin_a = angle.sin();
                let radius = level_radius(cos_a, sin_a, target);
                let mut point = Vec2::new(cos_a * radius, sin_a * radius);
                if !point.x.is_finite() || !point.y.is_finite() {
                    point = Vec2::default();
                }
                state.curves[curve][sample] = point;
                extent = extent.max(point.x.abs().max(point.y.abs()));
            }
        }
        state.extent = extent.max(0.5);

        for orbit in 0..ORBIT_CAPACITY {
            let salt = orbit as u64 * 4;
            let depth = unit(seed, salt + 1);
            let target = level_target(0.10 + 0.88 * depth, floor_level);
            let angle = unit(seed, salt + 2) * 2.0 * std::f32::consts::PI;
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            let radius = level_radius(cos_a, sin_a, target);
            let mut x = (LOG_PHI + cos_a * radius).exp();
            let mut y = (LOG_PHI + sin_a * radius).exp();
            for step in 0..ORBIT_PERIOD {
                let mut station = Vec2::new(x.ln() - LOG_PHI, y.ln() - LOG_PHI);
                if !station.x.is_finite() || !station.y.is_finite() {
                    station = Vec2::default();
                }
                state.stations[orbit][step] = station;
                let next = (1.0 + y) / x;
                x = y;
                y = next;
            }
            state.orbit_offset[orbit] = unit(seed, salt + 3) * ORBIT_PERIOD as f32;
            state.orbit_rate[orbit] = 0.75 + unit(seed, salt + 4) * 0.5;
            state.orbit_depth[orbit] = depth;
        }
        state
    }

    /// One station of one orbit, flexed by the current spectrum and projected to
    /// screen space (`pentagram_station_point`, `:224-234`).
    #[must_use]
    // The projection parameters travel together in the C as loose arguments too.
    // Bundling them into a struct is a reasonable cleanup once the artistry is
    // refactored; until the application works, matching the oracle's shape wins.
    #[allow(clippy::too_many_arguments)]
    pub fn station_point(
        &self,
        frame: &SceneFrame<'_>,
        orbit: usize,
        step: usize,
        center: Vec2,
        cos_r: f32,
        sin_r: f32,
        scale: f32,
        coupling: f32,
    ) -> Vec2 {
        let station = self.stations[orbit][step];
        let disp = shape(
            frame,
            station.y.atan2(station.x),
            self.orbit_depth[orbit],
            coupling,
        );
        project(flex(station, disp), center, cos_r, sin_r, scale)
    }
}

impl SceneState for PentagramState {
    fn id(&self) -> SceneId {
        SceneId::Pentagram
    }

    // No `update`: the C descriptor sets none (`:436-443`). Every animated quantity
    // is derived from `frame.time_seconds`, which is what keeps export exact.

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry entry (`scene_pentagram_descriptor`, `:436-443`).
pub const DESCRIPTOR: SceneDescriptor = SceneDescriptor {
    id: SceneId::Pentagram,
    state_version: 1,
    make_state: |seed| Box::new(PentagramState::new(seed)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::{descriptor, index};
    use crate::scene::{SceneAudioFrame, SceneSettings};

    #[test]
    fn the_fixed_point_is_the_minimum_of_the_invariant() {
        // The whole construction rests on this: ln K has a single minimum at
        // u = v = ln(phi), so a ray from there crosses each level set once.
        let floor_level = log_invariant(LOG_PHI, LOG_PHI);
        for du in [-0.5f32, -0.1, 0.1, 0.5] {
            for dv in [-0.5f32, -0.1, 0.1, 0.5] {
                assert!(
                    log_invariant(LOG_PHI + du, LOG_PHI + dv) > floor_level,
                    "({du}, {dv}) sits below the claimed minimum"
                );
            }
        }
        // At the fixed point K = phi^5, so ln K = 5 ln(phi). That closed form is
        // worth pinning: it is the cheapest check that LOG_PHI is the right
        // constant and that the invariant is transcribed correctly.
        assert!(
            (floor_level - 5.0 * LOG_PHI).abs() < 1.0e-5,
            "ln K at the fixed point is {floor_level}, expected {}",
            5.0 * LOG_PHI
        );
    }

    #[test]
    fn the_recurrence_closes_after_exactly_five_steps() {
        // Zamolodchikov periodicity is what makes every chord traversal close, so
        // this is the property the scene's geometry actually depends on.
        let state = PentagramState::new(1234);
        for orbit in 0..ORBIT_CAPACITY {
            let first = state.stations[orbit][0];
            // Step the map five times from the first station and land back on it.
            let mut x = (first.x + LOG_PHI).exp();
            let mut y = (first.y + LOG_PHI).exp();
            for _ in 0..ORBIT_PERIOD {
                let next = (1.0 + y) / x;
                x = y;
                y = next;
            }
            let closed = Vec2::new(x.ln() - LOG_PHI, y.ln() - LOG_PHI);
            assert!(
                (closed.x - first.x).abs() < 1.0e-3 && (closed.y - first.y).abs() < 1.0e-3,
                "orbit {orbit} did not close: {closed:?} vs {first:?}"
            );
        }
    }

    #[test]
    fn every_station_lies_on_its_orbits_level_curve() {
        let state = PentagramState::new(77);
        for orbit in 0..ORBIT_CAPACITY {
            let mut levels = Vec::new();
            for step in 0..ORBIT_PERIOD {
                let station = state.stations[orbit][step];
                levels.push(log_invariant(station.x + LOG_PHI, station.y + LOG_PHI));
            }
            let first = levels[0];
            for (step, level) in levels.iter().enumerate() {
                assert!(
                    (level - first).abs() < 1.0e-3,
                    "orbit {orbit} step {step}: K drifted from {first} to {level}"
                );
            }
        }
    }

    #[test]
    fn level_curves_are_nested_and_finite() {
        let state = PentagramState::new(5);
        let mut previous = 0.0f32;
        for curve in 0..CURVE_CAPACITY {
            let mut maximum = 0.0f32;
            for sample in 0..CURVE_SAMPLES {
                let point = state.curves[curve][sample];
                assert!(point.x.is_finite() && point.y.is_finite());
                maximum = maximum.max((point.x * point.x + point.y * point.y).sqrt());
            }
            assert!(
                maximum > previous,
                "curve {curve} is not outside curve {}",
                curve - 1
            );
            previous = maximum;
        }
        assert!(state.extent >= 0.5);
    }

    #[test]
    fn init_is_deterministic_and_seed_sensitive() {
        assert_eq!(PentagramState::new(3), PentagramState::new(3));
        let a = PentagramState::new(3);
        let b = PentagramState::new(4);
        assert_ne!(a.stations, b.stations);
        // Curves do not depend on the seed at all, only the orbits do.
        assert_eq!(a.curves[0][0], b.curves[0][0]);
    }

    #[test]
    fn the_trail_contour_is_seamless_at_the_angular_wrap() {
        // The mirror fold exists so bass and treble meet without a visible seam.
        let trails: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        let before = trail_at(&trails, 2.0 * std::f32::consts::PI - 0.0001);
        let after = trail_at(&trails, 0.0001);
        assert!(
            (before - after).abs() < 0.01,
            "seam at the wrap: {before} vs {after}"
        );
        // The fold means the halfway angle reads the top band, not the middle one.
        let folded = trail_at(&trails, std::f32::consts::PI);
        assert!(folded > trail_at(&trails, 0.0));
        assert_eq!(trail_at(&[], 1.0), 0.0);
    }

    #[test]
    fn the_trail_window_low_passes_a_single_spike() {
        // One loud band must not scribble a wiggle into the curve.
        let mut trails = vec![0.0f32; 64];
        trails[32] = 1.0;
        let at_spike = trail_at(&trails, std::f32::consts::PI * 0.5);
        assert!(
            at_spike < 0.2,
            "a lone band should be smeared, not tracked: {at_spike}"
        );
    }

    #[test]
    fn hop_easing_dwells_then_hops() {
        assert_eq!(hop_ease(0.0), 0.0);
        assert_eq!(hop_ease(1.0), 1.0);
        assert!((hop_ease(0.5) - 0.5).abs() < 1.0e-6);
        // Parked near the ends: a quarter of the way through, less than 10% moved.
        assert!(hop_ease(0.25) < 0.10);
        assert!(hop_ease(0.75) > 0.90);
        // And monotonic, which is what keeps a spark from sliding backward.
        let mut previous = -1.0f32;
        for step in 0..=100 {
            let value = hop_ease(step as f32 / 100.0);
            assert!(value >= previous, "hop easing went backwards at {step}");
            previous = value;
        }
    }

    #[test]
    fn flexing_ignores_the_origin_and_non_finite_displacement() {
        let origin = Vec2::new(0.0, 0.0);
        assert_eq!(flex(origin, 1.0), origin);
        let point = Vec2::new(1.0, 0.0);
        assert_eq!(flex(point, f32::NAN), point);
        assert_eq!(flex(point, 1.0), Vec2::new(2.0, 0.0));
    }

    #[test]
    fn the_shape_displacement_is_a_pure_function_of_the_frame() {
        let settings = SceneSettings::default();
        let trails: Vec<f32> = (0..64).map(|i| (i % 7) as f32 / 7.0).collect();
        let bands = trails.clone();
        let mut audio = SceneAudioFrame::from_spectrum(&bands, &trails);
        audio.spectral_flux = 0.3;
        let frame = SceneFrame {
            time_seconds: 12.5,
            audio,
            ..SceneFrame::idle(&settings)
        };
        let first = shape(&frame, 1.0, 0.5, 1.0);
        let second = shape(&frame, 1.0, 0.5, 1.0);
        assert_eq!(first, second);
        // Outer structures flex more than inner ones.
        assert!(shape(&frame, 1.0, 1.0, 1.0) > shape(&frame, 1.0, 0.0, 1.0));
        // Zero coupling means no flex at all.
        assert_eq!(shape(&frame, 1.0, 1.0, 0.0), 0.0);
    }

    #[test]
    fn hops_stay_in_the_five_cycle() {
        let state = PentagramState::new(21);
        for orbit in 0..ORBIT_CAPACITY {
            for step in 0..200 {
                let hops = orbit_hops(&state, orbit, step as f32 * 0.25, 1.0);
                assert!(hops >= 0.0, "hops must never go negative: {hops}");
                assert!(active_station(hops) < ORBIT_PERIOD);
            }
        }
        // Motion at zero freezes the phase at the seeded offset.
        let frozen = orbit_hops(&state, 0, 999.0, 0.0);
        assert_eq!(frozen, state.orbit_offset[0]);
    }

    #[test]
    fn the_hue_default_is_the_oracles_unusual_minus_ninety_one() {
        // Duplicated from the settings test on purpose: this is the value a reader
        // is most likely to "tidy" to zero, and it is visible if they do.
        assert_eq!(
            descriptor(SceneId::Pentagram, index::pentagram::HUE)
                .unwrap()
                .default_value,
            -91.0
        );
    }

    #[test]
    fn the_descriptor_matches_the_c_registry_entry() {
        assert_eq!(DESCRIPTOR.id, SceneId::Pentagram);
        assert_eq!(DESCRIPTOR.state_version, 1);
    }
}
