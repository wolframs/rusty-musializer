//! Spectral Terrarium: deterministic state and update.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_spectral_terrarium.c`
//! (frozen oracle, commit `9300af9`, read-only). Drawing lives in
//! `musializer-app::scenes::spectral_terrarium`.
//!
//! The scene is a small fixed-step ecosystem: a boid flock, a drifting particle
//! field, and a ring of plants, all driven by five smoothed audio envelopes. The
//! reason it is deterministic despite being a simulation is the fixed
//! `1/60` step plus the bounded catch-up loop in [`SpectralTerrariumState::update`]:
//! the same frame sequence always produces the same number of steps.
//!
//! Two behaviours here are deliberate and easy to "fix" by accident:
//!
//! - The state carries `last_frame_time` and **reseeds the whole world** on a
//!   transport discontinuity (`scene_spectral_terrarium.c:294-298`). A seek is
//!   therefore visible as a new world, not as a fast-forwarded old one.
//! - Once the catch-up loop saturates at eight steps, the leftover accumulator is
//!   reduced modulo the step rather than kept (`:344-346`), so a long stall drops
//!   simulation time instead of paying it back over the following frames.

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

/// `scene_spectral_terrarium.c:10-15`
pub const PARTICLE_COUNT: usize = 56;
pub const PLANT_COUNT: usize = 24;
pub const CREATURE_COUNT: usize = 10;
/// Catch-up ceiling, so a stalled frame cannot make the simulation unbounded.
pub const MAX_STEPS: usize = 8;
/// `scene_spectral_terrarium.c:17`
pub const FIXED_STEP: f32 = 1.0 / 60.0;

/// Largest per-frame scene delta the update accepts (`:302`). Anything longer is
/// treated as 133.333 ms of simulation, not as a stall to be repaid.
const MAX_FRAME_DELTA: f32 = 0.133_333;

use std::f32::consts::PI;

/// A three-component position/velocity.
///
/// `musializer-core` cannot use raylib's `Vector3`, so scenes carry their own.
/// A shared small-vector type in core would suit both scene agents; see the
/// Agent D note in REWRITE_PLAN.md.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
}

/// `scene_spectral_terrarium.c:19-24`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub phase: f32,
    pub size: f32,
}

/// `scene_spectral_terrarium.c:26-32`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plant {
    pub root: Vec3,
    pub height: f32,
    pub phase: f32,
    pub lean: f32,
    /// Band index this plant reacts to, taken modulo the frame's band count.
    pub band: u8,
}

/// `scene_spectral_terrarium.c:34-40`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Creature {
    pub position: Vec3,
    pub velocity: Vec3,
    pub speed: f32,
    pub phase: f32,
    pub band: u8,
}

/// `scene_spectral_terrarium.c:42-56`
#[derive(Clone, Debug, PartialEq)]
pub struct SpectralTerrariumState {
    pub seed: u64,
    /// `-1.0` means "no frame seen yet", which is why this is not an `Option`:
    /// the sentinel is what the C compares against and the discontinuity test
    /// reads more like the oracle this way.
    pub last_frame_time: f64,
    pub simulation_time: f64,
    pub accumulator: f32,
    pub energy: f32,
    pub bass: f32,
    pub treble: f32,
    pub spectral_centroid: f32,
    pub flux: f32,
    pub onset_pulse: f32,
    pub particles: [Particle; PARTICLE_COUNT],
    pub plants: [Plant; PLANT_COUNT],
    pub creatures: [Creature; CREATURE_COUNT],
}

/// `terrarium_clamp01` (`:58-63`). Note `!isfinite` folds NaN to 0, which is what
/// keeps a poisoned band from spreading through the envelopes.
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

/// `terrarium_hash` (`:65-74`) — splitmix64's mixing function over `seed ^ salt`.
#[must_use]
fn hash(seed: u64, salt: u32) -> u32 {
    let mut value = seed ^ (u64::from(salt).wrapping_add(0x9e37_79b9_7f4a_7c15));
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value as u32
}

/// `terrarium_hash_unit` (`:76-79`) — a deterministic `[0, 1]` draw.
#[must_use]
pub fn hash_unit(seed: u64, salt: u32) -> f32 {
    (hash(seed, salt) & 0xffff) as f32 / 65535.0
}

/// `terrarium_band` (`:81-85`) — one band, wrapped by the frame's band count.
#[must_use]
pub fn band(frame: &SceneFrame<'_>, index: usize) -> f32 {
    let bands = frame.audio.bands;
    if bands.is_empty() {
        return 0.0;
    }
    clamp01(bands[index % bands.len()])
}

/// `terrarium_band_range` (`:87-96`) — mean of a clamped half-open band range.
///
/// The bound juggling reproduces the C exactly: `begin` is pulled back inside the
/// array, `end` is clipped, and an inverted range collapses to a single band.
#[must_use]
pub fn band_range(frame: &SceneFrame<'_>, mut begin: usize, mut end: usize) -> f32 {
    let bands = frame.audio.bands;
    if bands.is_empty() {
        return 0.0;
    }
    if begin >= bands.len() {
        begin = bands.len() - 1;
    }
    if end > bands.len() {
        end = bands.len();
    }
    if end <= begin {
        end = begin + 1;
    }
    let total: f32 = bands[begin..end].iter().copied().map(clamp01).sum();
    total / (end - begin) as f32
}

impl Default for SpectralTerrariumState {
    fn default() -> Self {
        Self {
            seed: 0,
            last_frame_time: -1.0,
            simulation_time: 0.0,
            accumulator: 0.0,
            energy: 0.0,
            bass: 0.0,
            treble: 0.0,
            spectral_centroid: 0.0,
            flux: 0.0,
            onset_pulse: 0.0,
            particles: [Particle::default(); PARTICLE_COUNT],
            plants: [Plant::default(); PLANT_COUNT],
            creatures: [Creature::default(); CREATURE_COUNT],
        }
    }
}

impl SpectralTerrariumState {
    /// `spectral_terrarium_init` (`:159-166`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut state = Self {
            seed,
            ..Self::default()
        };
        state.seed_world(0.0);
        state
    }

    /// `terrarium_seed_world` (`:98-157`).
    ///
    /// Called at init and again on every transport discontinuity, which is what
    /// makes a seek land in a freshly seeded world rather than a stale one.
    #[allow(clippy::needless_range_loop)] // Salts are derived from `i`; keeping the
                                          // index loop is what makes each line diffable against the C.
    pub fn seed_world(&mut self, song_time: f64) {
        let seed = self.seed;
        for i in 0..PARTICLE_COUNT {
            let salt = i as u32 * 7;
            let angle = hash_unit(seed, salt + 1) * 2.0 * PI;
            let radius = hash_unit(seed, salt + 2).sqrt() * 3.65;
            let particle = &mut self.particles[i];
            particle.position = Vec3::new(
                angle.cos() * radius,
                -1.55 + hash_unit(seed, salt + 3) * 4.4,
                angle.sin() * radius,
            );
            particle.velocity = Vec3::new(
                (hash_unit(seed, salt + 4) - 0.5) * 0.24,
                0.10 + hash_unit(seed, salt + 5) * 0.18,
                (hash_unit(seed, salt + 6) - 0.5) * 0.24,
            );
            particle.phase = hash_unit(seed, salt + 7) * 2.0 * PI;
            particle.size = 0.025 + hash_unit(seed, salt + 8) * 0.055;
        }

        for i in 0..PLANT_COUNT {
            let angle =
                (i as f32 + hash_unit(seed, i as u32 + 300) * 0.65) * 2.0 * PI / PLANT_COUNT as f32;
            let radius = 1.0 + hash_unit(seed, i as u32 + 400) * 2.45;
            let plant = &mut self.plants[i];
            plant.root = Vec3::new(angle.cos() * radius, -1.72, angle.sin() * radius);
            plant.height = 0.45 + hash_unit(seed, i as u32 + 500) * 1.35;
            plant.phase = hash_unit(seed, i as u32 + 600) * 2.0 * PI;
            plant.lean = 0.12 + hash_unit(seed, i as u32 + 700) * 0.28;
            plant.band = i as u8;
        }

        for i in 0..CREATURE_COUNT {
            let angle = hash_unit(seed, i as u32 + 800) * 2.0 * PI;
            let radius = 0.7 + hash_unit(seed, i as u32 + 900) * 2.55;
            let height = -0.85 + hash_unit(seed, i as u32 + 1000) * 2.7;
            let creature = &mut self.creatures[i];
            creature.speed =
                (0.22 + hash_unit(seed, i as u32 + 1100) * 0.42) * (0.85 + (i % 3) as f32 * 0.08);
            creature.phase = hash_unit(seed, i as u32 + 1200) * 2.0 * PI;
            creature.band = (i * 3 + 2) as u8;
            creature.position = Vec3::new(angle.cos() * radius, height, angle.sin() * radius);
            creature.velocity = Vec3::new(
                -angle.sin() * creature.speed,
                0.0,
                angle.cos() * creature.speed,
            );
        }

        // Quantized to the fixed step so a reseed lands on a step boundary and
        // the phase of every `sinf(simulation_time * ...)` term is reproducible.
        self.simulation_time = (song_time * 60.0).floor() / 60.0;
        self.accumulator = 0.0;
        self.energy = 0.0;
        self.bass = 0.0;
        self.treble = 0.0;
        self.spectral_centroid = 0.5;
        self.flux = 0.0;
        self.onset_pulse = 0.0;
    }

    /// `terrarium_simulate` (`:168-289`): one fixed step of particles and boids.
    #[allow(clippy::needless_range_loop)] // Parallel arrays indexed together.
    pub fn simulate(&mut self, step: f32, creature_speed: f32) {
        let time = self.simulation_time as f32;
        for i in 0..PARTICLE_COUNT {
            let particle = &mut self.particles[i];
            let curl = (time * 0.61 + particle.phase).sin();
            particle.position.x += (particle.velocity.x + curl * 0.08 * (0.3 + self.flux)) * step;
            particle.position.z += (particle.velocity.z - curl * 0.07 * (0.3 + self.flux)) * step;
            particle.position.y += particle.velocity.y * step * (0.55 + self.energy);
            let radius_squared = particle.position.x * particle.position.x
                + particle.position.z * particle.position.z;
            if particle.position.y > 3.05 || radius_squared > 14.6 {
                // Recycled to the floor on a golden-ratio-spaced ring rather than
                // to its seeded spot, so a long run does not re-tread one path.
                let angle = particle.phase + time * 0.13;
                let radius = 0.35 + (i as f32 * 0.618_033_9) % 1.0 * 3.0;
                particle.position = Vec3::new(angle.cos() * radius, -1.58, angle.sin() * radius);
            }
        }

        // Boids: every creature reads the *current* flock, then all velocities
        // are committed together. Updating in place would make the result depend
        // on iteration order.
        let mut next_velocity = [Vec3::default(); CREATURE_COUNT];
        for i in 0..CREATURE_COUNT {
            let creature = self.creatures[i];
            let mut separation = Vec3::default();
            let mut alignment = Vec3::default();
            let mut cohesion = Vec3::default();
            let mut neighbors = 0usize;
            for j in 0..CREATURE_COUNT {
                if i == j {
                    continue;
                }
                let other = self.creatures[j];
                let delta = Vec3::new(
                    creature.position.x - other.position.x,
                    creature.position.y - other.position.y,
                    creature.position.z - other.position.z,
                );
                let distance_squared = delta.x * delta.x + delta.y * delta.y + delta.z * delta.z;
                if distance_squared > 4.0 {
                    continue;
                }
                neighbors += 1;
                alignment.x += other.velocity.x;
                alignment.y += other.velocity.y;
                alignment.z += other.velocity.z;
                cohesion.x += other.position.x;
                cohesion.y += other.position.y;
                cohesion.z += other.position.z;
                if distance_squared < 0.72 && distance_squared > 0.0001 {
                    let inverse = 1.0 / distance_squared;
                    separation.x += delta.x * inverse;
                    separation.y += delta.y * inverse;
                    separation.z += delta.z * inverse;
                }
            }

            let mut acceleration = Vec3::default();
            if neighbors > 0 {
                let inverse = 1.0 / neighbors as f32;
                alignment.x = alignment.x * inverse - creature.velocity.x;
                alignment.y = alignment.y * inverse - creature.velocity.y;
                alignment.z = alignment.z * inverse - creature.velocity.z;
                cohesion.x = cohesion.x * inverse - creature.position.x;
                cohesion.y = cohesion.y * inverse - creature.position.y;
                cohesion.z = cohesion.z * inverse - creature.position.z;
                // Brighter mixes pull the flock tighter, which is how the boids
                // read as reacting to the music at all.
                let tightness = 0.16 + self.spectral_centroid * 0.26;
                acceleration.x += separation.x * 0.19 + alignment.x * 0.22 + cohesion.x * tightness;
                acceleration.y += separation.y * 0.19 + alignment.y * 0.22 + cohesion.y * tightness;
                acceleration.z += separation.z * 0.19 + alignment.z * 0.22 + cohesion.z * tightness;
            }

            let target_height = -0.75 + self.spectral_centroid * 3.05;
            acceleration.y += (target_height - creature.position.y) * 0.34;
            let dominant_heading = self.spectral_centroid * 2.0 * PI + (time * 0.17).sin() * 0.55;
            acceleration.x += dominant_heading.cos() * 0.12;
            acceleration.z += dominant_heading.sin() * 0.12;
            acceleration.x += -creature.position.z * 0.055;
            acceleration.z += creature.position.x * 0.055;

            let mut velocity = Vec3::new(
                creature.velocity.x + acceleration.x * step,
                creature.velocity.y + acceleration.y * step,
                creature.velocity.z + acceleration.z * step,
            );
            let speed = velocity.length();
            let desired =
                creature.speed * creature_speed * (0.78 + self.energy * 0.58 + self.flux * 0.28);
            if speed > 0.0001 {
                let limited = speed.max(desired * 0.68).min(desired * 1.35);
                let scale = limited / speed;
                velocity.x *= scale;
                velocity.y *= scale;
                velocity.z *= scale;
            }
            next_velocity[i] = velocity;
        }
        for i in 0..CREATURE_COUNT {
            let creature = &mut self.creatures[i];
            creature.velocity = next_velocity[i];
            creature.position.x += creature.velocity.x * step;
            creature.position.y += creature.velocity.y * step;
            creature.position.z += creature.velocity.z * step;
            let radius = (creature.position.x * creature.position.x
                + creature.position.z * creature.position.z)
                .sqrt();
            if radius > 3.55 {
                let scale = 3.55 / radius;
                creature.position.x *= scale;
                creature.position.z *= scale;
                creature.velocity.x *= -0.25;
                creature.velocity.z *= -0.25;
            }
            if creature.position.y < -1.42 {
                creature.position.y = -1.42;
                creature.velocity.y = creature.velocity.y.abs() * 0.45;
            } else if creature.position.y > 2.72 {
                creature.position.y = 2.72;
                creature.velocity.y = -creature.velocity.y.abs() * 0.45;
            }
        }
        self.simulation_time += f64::from(step);
    }

    /// The drawn height of a creature, which bobs on the simulation clock rather
    /// than on its own position (`terrarium_creature_position`, `:350-357`).
    #[must_use]
    pub fn creature_position(&self, creature: &Creature) -> Vec3 {
        let mut position = creature.position;
        position.y += (self.simulation_time as f32 * 1.3 + creature.phase).sin()
            * (0.035 + self.treble * 0.055);
        position
    }
}

impl SceneState for SpectralTerrariumState {
    fn id(&self) -> SceneId {
        SceneId::SpectralTerrarium
    }

    /// `spectral_terrarium_update` (`:291-348`).
    // The discontinuity clauses stay as separate comparisons in the C's order; each
    // names a different transport situation and a range check would obscure that.
    #[allow(clippy::manual_range_contains)]
    fn update(&mut self, frame: &SceneFrame<'_>) {
        use crate::scene::settings::index::terrarium as setting;

        let elapsed = if self.last_frame_time < 0.0 {
            0.0
        } else {
            frame.time_seconds - self.last_frame_time
        };
        let discontinuity = self.last_frame_time >= 0.0
            && (!elapsed.is_finite() || elapsed < -0.001 || elapsed > 0.25);
        if discontinuity {
            self.seed_world(frame.time_seconds);
        }

        let mut delta = frame.delta_seconds;
        if !delta.is_finite() || delta < 0.0 {
            delta = 0.0;
        }
        if delta > MAX_FRAME_DELTA {
            delta = MAX_FRAME_DELTA;
        }
        let simulation_speed = frame.setting(SceneId::SpectralTerrarium, setting::SIM_SPEED);
        let creature_speed = frame.setting(SceneId::SpectralTerrarium, setting::CREATURE_SPEED);
        delta *= simulation_speed;

        let bands_count = frame.audio.bands_count();
        let mut low_end = bands_count / 6;
        if low_end < 1 {
            low_end = 1;
        }
        let high_begin = bands_count * 2 / 3;
        let target_energy = clamp01(frame.audio.rms * 1.9);
        let target_bass = band_range(frame, 0, low_end);
        let target_treble = band_range(frame, high_begin, bands_count);

        // Spectral centroid over the *normalized* bands. Because the analyzer
        // normalizes per frame (see REWRITE_PLAN.md), this reads as "where the
        // energy sits", never as "how loud it is".
        let mut centroid_weight = 0.0f32;
        let mut centroid_sum = 0.0f32;
        if bands_count > 1 {
            for (i, &raw) in frame.audio.bands.iter().enumerate() {
                let value = clamp01(raw);
                centroid_sum += value * i as f32 / (bands_count - 1) as f32;
                centroid_weight += value;
            }
        }
        let target_centroid = if centroid_weight > 0.0001 {
            centroid_sum / centroid_weight
        } else {
            0.5
        };
        let target_flux = clamp01(frame.audio.spectral_flux * 5.0);
        let blend = 1.0 - (-5.5 * delta).exp();
        self.energy += (target_energy - self.energy) * blend;
        self.bass += (target_bass - self.bass) * blend;
        self.treble += (target_treble - self.treble) * blend;
        self.spectral_centroid += (target_centroid - self.spectral_centroid) * blend;
        self.flux += (target_flux - self.flux) * blend;
        self.onset_pulse *= (-6.8 * delta).exp();
        if frame.audio.onset {
            self.onset_pulse = 1.0;
        }

        self.accumulator += delta;
        let mut steps = 0usize;
        while self.accumulator >= FIXED_STEP && steps < MAX_STEPS {
            self.simulate(FIXED_STEP, creature_speed);
            self.accumulator -= FIXED_STEP;
            steps += 1;
        }
        if steps == MAX_STEPS && self.accumulator >= FIXED_STEP {
            // Deliberately drops the backlog instead of repaying it: a stall must
            // not turn into a burst of catch-up simulation on the next frames.
            self.accumulator %= FIXED_STEP;
        }
        self.last_frame_time = frame.time_seconds;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry entry (`scene_spectral_terrarium_descriptor`, `:608-616`).
pub const DESCRIPTOR: SceneDescriptor = SceneDescriptor {
    id: SceneId::SpectralTerrarium,
    state_version: 2,
    make_state: |seed| Box::new(SpectralTerrariumState::new(seed)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneAudioFrame, SceneSettings};

    fn frame_at<'a>(
        settings: &'a SceneSettings,
        time: f64,
        delta: f32,
        bands: &'a [f32],
        trails: &'a [f32],
    ) -> SceneFrame<'a> {
        let mut audio = SceneAudioFrame::from_spectrum(bands, trails);
        audio.rms = 0.4;
        SceneFrame {
            time_seconds: time,
            delta_seconds: delta,
            audio,
            ..SceneFrame::idle(settings)
        }
    }

    fn ramp(count: usize) -> Vec<f32> {
        (0..count).map(|i| i as f32 / count as f32).collect()
    }

    #[test]
    fn seeding_is_deterministic_and_bounded() {
        let a = SpectralTerrariumState::new(7);
        let b = SpectralTerrariumState::new(7);
        assert_eq!(a, b);
        assert_ne!(a.particles[0].position.x, 0.0, "seeded, not zeroed");
        assert_ne!(SpectralTerrariumState::new(8).particles[0], a.particles[0]);

        for particle in &a.particles {
            let radius = (particle.position.x.powi(2) + particle.position.z.powi(2)).sqrt();
            assert!(
                radius <= 3.66,
                "particle seeded outside the habitat: {radius}"
            );
            assert!(particle.size > 0.0 && particle.size <= 0.081);
        }
        for plant in &a.plants {
            assert_eq!(plant.root.y, -1.72, "plants are rooted in the soil");
        }
        for creature in &a.creatures {
            assert!(creature.speed > 0.0);
        }
        assert_eq!(a.spectral_centroid, 0.5, "the centroid starts centred");
        assert_eq!(a.last_frame_time, -1.0, "no frame seen yet");
    }

    #[test]
    fn the_same_frame_sequence_produces_the_same_state() {
        let settings = SceneSettings::default();
        let bands = ramp(104);
        let trails = ramp(104);
        let mut first = SpectralTerrariumState::new(11);
        let mut second = SpectralTerrariumState::new(11);
        for i in 0..240u32 {
            let frame = frame_at(
                &settings,
                f64::from(i) / 60.0,
                if i == 0 { 0.0 } else { 1.0 / 60.0 },
                &bands,
                &trails,
            );
            first.update(&frame);
            second.update(&frame);
        }
        assert_eq!(first, second);
        assert!(first.simulation_time > 3.0, "the clock advanced");
    }

    #[test]
    fn a_seek_reseeds_the_world_instead_of_fast_forwarding_it() {
        let settings = SceneSettings::default();
        let bands = ramp(64);
        let trails = ramp(64);
        let mut played = SpectralTerrariumState::new(3);
        for i in 0..120u32 {
            let frame = frame_at(
                &settings,
                f64::from(i) / 60.0,
                if i == 0 { 0.0 } else { 1.0 / 60.0 },
                &bands,
                &trails,
            );
            played.update(&frame);
        }
        // A jump of more than 0.25 s is a transport discontinuity.
        let seek = frame_at(&settings, 90.0, 0.0, &bands, &trails);
        played.update(&seek);

        let mut fresh = SpectralTerrariumState::new(3);
        fresh.seed_world(90.0);
        fresh.last_frame_time = 90.0;
        // `update` with a zero delta runs no steps, so the reseeded world is
        // exactly what a fresh reseed at that song time produces.
        assert_eq!(played.simulation_time, fresh.simulation_time);
        assert_eq!(played.particles, fresh.particles);
        assert_eq!(played.creatures, fresh.creatures);
    }

    #[test]
    fn the_catch_up_loop_is_bounded_and_drops_its_backlog() {
        let settings = SceneSettings::default();
        let bands = ramp(32);
        let trails = ramp(32);
        let mut state = SpectralTerrariumState::new(1);
        // First frame establishes `last_frame_time` without a discontinuity.
        state.update(&frame_at(&settings, 0.0, 0.0, &bands, &trails));
        let before = state.simulation_time;
        // A 10-second delta: clamped to 0.133 s, so at most eight steps.
        state.update(&frame_at(&settings, 0.1, 10.0, &bands, &trails));
        let steps = ((state.simulation_time - before) * 60.0).round() as i64;
        assert!(
            (1..=MAX_STEPS as i64).contains(&steps),
            "{steps} steps ran, the ceiling is {MAX_STEPS}"
        );
        assert!(
            state.accumulator < FIXED_STEP,
            "a saturated frame must not leave a backlog"
        );
    }

    #[test]
    fn hostile_audio_keeps_the_envelopes_in_range() {
        let settings = SceneSettings::default();
        let bands = vec![f32::NAN, -3.0, 5.0, 0.5];
        let trails = vec![0.0f32; 4];
        let mut state = SpectralTerrariumState::new(5);
        for i in 0..120u32 {
            let mut frame = frame_at(
                &settings,
                f64::from(i) / 60.0,
                if i == 0 { 0.0 } else { 1.0 / 60.0 },
                &bands,
                &trails,
            );
            frame.audio.rms = if i % 2 == 0 { -1.0 } else { f32::INFINITY };
            frame.audio.spectral_flux = f32::NAN;
            frame.audio.onset = i % 17 == 0;
            state.update(&frame);
            for value in [
                state.energy,
                state.bass,
                state.treble,
                state.spectral_centroid,
                state.flux,
                state.onset_pulse,
            ] {
                assert!((0.0..=1.0).contains(&value), "envelope escaped: {value}");
            }
        }
    }

    #[test]
    fn band_range_survives_inverted_and_out_of_range_requests() {
        let settings = SceneSettings::default();
        let bands = [0.25f32, 0.5, 1.0];
        let trails = [0.0f32; 3];
        let frame = frame_at(&settings, 0.0, 0.0, &bands, &trails);
        assert_eq!(band_range(&frame, 0, 3), (0.25 + 0.5 + 1.0) / 3.0);
        // `begin` past the end collapses onto the last band.
        assert_eq!(band_range(&frame, 9, 9), 1.0);
        // An inverted range widens to one band rather than dividing by zero.
        assert_eq!(band_range(&frame, 2, 1), 1.0);

        let empty = frame_at(&settings, 0.0, 0.0, &[], &[]);
        assert_eq!(band_range(&empty, 0, 4), 0.0);
        assert_eq!(band(&empty, 3), 0.0);
    }

    #[test]
    fn the_descriptor_matches_the_c_registry_entry() {
        assert_eq!(DESCRIPTOR.id, SceneId::SpectralTerrarium);
        assert_eq!(DESCRIPTOR.state_version, 2);
        let state = (DESCRIPTOR.make_state)(42);
        assert_eq!(state.id(), SceneId::SpectralTerrarium);
    }
}
