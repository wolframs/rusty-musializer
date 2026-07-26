//! Orbital Lattice motion: the audio-independent camera path and the damped
//! audio envelopes the geometry is drawn from.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_orbital_lattice_motion.c`
//! and `.h` (frozen at `9300af9`, read-only).
//!
//! This module is raylib-free in C and stays raylib-free here. That is the whole
//! reason the scene is testable: the C suite exercises it through
//! `../musializer/tests/test_scene_orbital_lattice_motion.c`, and those five
//! tests are ported at the bottom of this file.
//!
//! The design invariant worth not breaking (`scene_orbital_lattice_motion.h:37-39`):
//! **phase is a function of seed and transport time only.** Audio animates
//! bounded geometry and colour envelopes, never phase velocity, so a seeked
//! preview and an uninterrupted export land on the same camera path instead of
//! integrating two different histories.

// C's discontinuity and bounds tests are kept as chains of comparisons rather
// than range `contains` calls, so each guard diffs against the line it came from.
#![allow(clippy::manual_range_contains)]

/// `ORBITAL_LATTICE_RING_COUNT` (`scene_orbital_lattice_motion.h:9`).
pub const RING_COUNT: usize = 12;
/// `ORBITAL_LATTICE_NODES_PER_RING` (`scene_orbital_lattice_motion.h:10`).
pub const NODES_PER_RING: usize = 16;
/// `ORBITAL_LATTICE_RING_SPACING` (`scene_orbital_lattice_motion.h:13`).
pub const RING_SPACING: f32 = 2.25;
/// `ORBITAL_LATTICE_PATH_LENGTH` = `RING_COUNT*RING_SPACING`
/// (`scene_orbital_lattice_motion.h:14-15`). Exactly 27.0 in both float and
/// double, which is why the wrap period below can be spelled either way.
pub const PATH_LENGTH: f32 = RING_COUNT as f32 * RING_SPACING;

const TAU: f64 = 2.0 * std::f64::consts::PI;

/// Per-frame input (`Orbital_Lattice_Motion_Input`,
/// `scene_orbital_lattice_motion.h:17-30`).
///
/// C's `bands` + `bands_count` pair becomes one slice; a null `bands` is an
/// empty slice.
#[derive(Clone, Copy, Debug, Default)]
pub struct MotionInput<'a> {
    pub time_seconds: f64,
    pub delta_seconds: f32,
    pub bands: &'a [f32],
    pub rms: f32,
    pub spectral_flux: f32,
    pub onset: bool,
    pub semantic_available: bool,
    pub semantic_valence: f32,
    pub semantic_tension: f32,
    pub semantic_confidence: f32,
    pub motion_rate: f32,
}

/// The damping targets for one frame (`Orbital_Lattice_Targets`,
/// `scene_orbital_lattice_motion.c:9-18`).
#[derive(Clone, Copy, Debug, Default)]
struct Targets {
    energy: f32,
    bass: f32,
    mids: f32,
    treble: f32,
    flux: f32,
    semantic_valence: f32,
    semantic_tension: f32,
    node_bands: [f32; NODES_PER_RING],
}

/// The whole scene state (`Orbital_Lattice_Motion`,
/// `scene_orbital_lattice_motion.h:32-57`).
///
/// Fields are public because the drawing half reads every one of them and the C
/// struct is equally open. Bounded by construction: `node_bands` is a fixed
/// array, and nothing here grows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub seed: u64,
    pub initialized: bool,
    pub last_time_seconds: f64,

    // Absolute, fixed-rate phases. See the module comment.
    pub camera_phase: f64,
    pub travel_phase: f64,
    pub twist_phase: f64,
    pub hue_degrees: f64,

    // Deliberately distinct roles: bass shapes the structure, mids shape its
    // rings, treble articulates nodes, and flux controls restrained accents.
    pub energy: f32,
    pub bass: f32,
    pub mids: f32,
    pub treble: f32,
    pub flux: f32,
    pub onset_pulse: f32,
    pub onset_hold_seconds: f32,
    pub semantic_valence: f32,
    pub semantic_tension: f32,
    pub node_bands: [f32; NODES_PER_RING],
}

/// One sampled ring (`Orbital_Lattice_Ring_Motion`,
/// `scene_orbital_lattice_motion.h:59-63`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingMotion {
    pub distance: f32,
    pub depth_t: f32,
    pub visibility: f32,
}

fn clamp01(value: f32) -> f32 {
    // `orbital_clamp01` (motion.c:20-25). Note that a NaN maps to 0.0 rather
    // than propagating, which is what keeps a bad analyzer frame from poisoning
    // the whole lattice.
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

fn clamp_signed(value: f32) -> f32 {
    // `orbital_clamp_signed` (motion.c:27-33).
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-1.0, 1.0)
}

fn wrap(value: f64, period: f64) -> f64 {
    // `orbital_wrap` (motion.c:35-40).
    if !value.is_finite() || !period.is_finite() || period <= 0.0 {
        return 0.0;
    }
    let value = value % period;
    if value < 0.0 {
        value + period
    } else {
        value
    }
}

fn hash(seed: u64, salt: u32) -> u32 {
    // `orbital_hash` (motion.c:42-51). splitmix64's finalizer over a salted
    // seed. `wrapping_*` is not a liberty taken here: C's unsigned arithmetic
    // wraps by definition and this is the same computation.
    let mut value = seed ^ (u64::from(salt).wrapping_add(0x9e37_79b9_7f4a_7c15));
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value as u32
}

fn hash_unit(seed: u64, salt: u32) -> f64 {
    // `orbital_hash_unit` (motion.c:53-56). Only the low 16 bits are used.
    f64::from(hash(seed, salt) & 0xffff) / 65535.0
}

/// `orbital_band_mean` (`motion.c:58-71`), with C's index repair kept verbatim.
///
/// The repair order matters and looks odd on purpose: `begin` is pulled inside
/// the array, `end` is clipped to the length, and only then is an empty range
/// widened to one element — so a `begin` at the last band still averages
/// something rather than dividing by zero.
fn band_mean(bands: &[f32], mut begin: usize, mut end: usize) -> f32 {
    let count = bands.len();
    if count == 0 {
        return 0.0;
    }
    if begin >= count {
        begin = count - 1;
    }
    if end > count {
        end = count;
    }
    if end <= begin {
        end = begin + 1;
    }

    let mut total = 0.0f64;
    for &band in &bands[begin..end] {
        total += f64::from(clamp01(band));
    }
    (total / (end - begin) as f64) as f32
}

/// `orbital_targets` (`motion.c:73-111`).
fn targets(input: &MotionInput<'_>) -> Targets {
    let mut targets = Targets {
        energy: clamp01(input.rms * 1.65),
        flux: clamp01(input.spectral_flux * 4.0),
        ..Targets::default()
    };

    let count = input.bands.len();
    if count > 0 {
        let mut bass_end = count / 5;
        if bass_end < 1 {
            bass_end = 1;
        }
        let mut treble_begin = count * 3 / 5;
        if treble_begin < bass_end {
            treble_begin = bass_end;
        }
        if treble_begin >= count {
            treble_begin = count - 1;
        }
        targets.bass = band_mean(input.bands, 0, bass_end);
        targets.mids = band_mean(input.bands, bass_end, treble_begin);
        targets.treble = band_mean(input.bands, treble_begin, count);

        for (node, slot) in targets.node_bands.iter_mut().enumerate() {
            let begin = node * count / NODES_PER_RING;
            let mut end = (node + 1) * count / NODES_PER_RING;
            if end <= begin {
                end = begin + 1;
            }
            *slot = band_mean(input.bands, begin, end);
        }
    }

    if input.semantic_available {
        let confidence = clamp01(input.semantic_confidence);
        targets.semantic_valence = clamp_signed(input.semantic_valence) * confidence;
        targets.semantic_tension = clamp01(input.semantic_tension) * confidence;
    }
    targets
}

/// `orbital_damp` (`motion.c:113-119`): an exponential approach with separate
/// attack and release rates.
fn damp(current: f32, target: f32, attack_rate: f32, release_rate: f32, delta: f32) -> f32 {
    let rate = if target > current {
        attack_rate
    } else {
        release_rate
    };
    let blend = 1.0 - (-rate * delta).exp();
    current + (target - current) * blend
}

/// `orbital_smoothstep` (`motion.c:121-126`).
fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    if edge0 == edge1 {
        return if value >= edge1 { 1.0 } else { 0.0 };
    }
    let amount = clamp01((value - edge0) / (edge1 - edge0));
    amount * amount * (3.0 - 2.0 * amount)
}

/// The audio-independent phase speeds (`motion.c:229-232`), named once so the
/// rebase path and the incremental path cannot drift apart.
const CAMERA_SPEED: f64 = 0.060;
const TRAVEL_SPEED: f64 = 0.38;
const TWIST_SPEED: f64 = 0.18;
const HUE_SPEED: f64 = 1.5;

impl Motion {
    /// `orbital_lattice_motion_init` (`motion.c:166-171`): everything zeroed, then
    /// the seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            initialized: false,
            last_time_seconds: 0.0,
            camera_phase: 0.0,
            travel_phase: 0.0,
            twist_phase: 0.0,
            hue_degrees: 0.0,
            energy: 0.0,
            bass: 0.0,
            mids: 0.0,
            treble: 0.0,
            flux: 0.0,
            onset_pulse: 0.0,
            onset_hold_seconds: 0.0,
            semantic_valence: 0.0,
            semantic_tension: 0.0,
            node_bands: [0.0; NODES_PER_RING],
        }
    }

    /// Sanitized transport time: negative, zero and non-finite all read as 0
    /// (`motion.c:133-134`, `motion.c:180-181`).
    fn sanitized_time(time_seconds: f64) -> f64 {
        if time_seconds.is_finite() && time_seconds > 0.0 {
            time_seconds
        } else {
            0.0
        }
    }

    /// A non-positive or non-finite motion rate reads as 1 (`motion.c:135-136`).
    fn sanitized_motion_rate(motion_rate: f32) -> f64 {
        if motion_rate.is_finite() && motion_rate > 0.0 {
            f64::from(motion_rate)
        } else {
            1.0
        }
    }

    /// Recomputes every phase from seed and absolute time.
    fn set_phases(&mut self, time: f64, motion_rate: f64) {
        self.camera_phase = wrap(
            hash_unit(self.seed, 1) * TAU + time * CAMERA_SPEED * motion_rate,
            TAU,
        );
        self.travel_phase = wrap(
            hash_unit(self.seed, 2) * f64::from(PATH_LENGTH) + time * TRAVEL_SPEED * motion_rate,
            f64::from(PATH_LENGTH),
        );
        self.twist_phase = wrap(
            hash_unit(self.seed, 3) * TAU + time * TWIST_SPEED * motion_rate,
            TAU,
        );
        self.hue_degrees = wrap(
            205.0 + hash_unit(self.seed, 4) * 110.0 + time * HUE_SPEED * motion_rate,
            360.0,
        );
    }

    /// `orbital_lattice_motion_rebase` (`motion.c:128-164`): snap straight to the
    /// targets instead of damping toward them.
    ///
    /// This is what makes a seek land on the same orbit as uninterrupted
    /// playback — there is no history to carry across the discontinuity.
    fn rebase(&mut self, input: &MotionInput<'_>, targets: &Targets) {
        let time = Self::sanitized_time(input.time_seconds);
        let motion_rate = Self::sanitized_motion_rate(input.motion_rate);
        self.set_phases(time, motion_rate);

        self.energy = targets.energy;
        self.bass = targets.bass;
        self.mids = targets.mids;
        self.treble = targets.treble;
        self.flux = targets.flux;
        self.semantic_valence = targets.semantic_valence;
        self.semantic_tension = targets.semantic_tension;
        self.node_bands = targets.node_bands;
        self.onset_hold_seconds = if input.onset { 0.09 } else { 0.0 };
        self.onset_pulse = if input.onset { 1.0 } else { 0.0 };
        self.last_time_seconds = time;
        self.initialized = true;
    }

    /// `orbital_lattice_motion_update` (`motion.c:173-247`).
    pub fn update(&mut self, input: &MotionInput<'_>) {
        let targets = targets(input);

        let time = Self::sanitized_time(input.time_seconds);
        let elapsed = time - self.last_time_seconds;
        let mut delta = if input.delta_seconds.is_finite() && input.delta_seconds > 0.0 {
            input.delta_seconds
        } else {
            0.0
        };
        // Four separate discontinuity tests, all from `motion.c:185-187`. The
        // last one catches a paused transport that still moved: time jumped but
        // no frame delta was reported.
        let discontinuity = !self.initialized
            || elapsed < -0.001
            || elapsed > 0.25
            || delta > 0.20
            || (elapsed.abs() > 0.02 && delta == 0.0);
        if discontinuity {
            self.rebase(input, &targets);
            return;
        }
        if delta > 0.10 {
            delta = 0.10;
        }
        self.last_time_seconds = time;
        if delta <= 0.0 {
            return;
        }

        self.energy = damp(self.energy, targets.energy, 5.5, 2.4, delta);
        self.bass = damp(self.bass, targets.bass, 4.2, 2.0, delta);
        self.mids = damp(self.mids, targets.mids, 5.0, 2.6, delta);
        self.treble = damp(self.treble, targets.treble, 7.0, 3.6, delta);
        self.flux = damp(self.flux, targets.flux, 6.5, 2.8, delta);
        self.semantic_valence = damp(
            self.semantic_valence,
            targets.semantic_valence,
            1.4,
            1.1,
            delta,
        );
        self.semantic_tension = damp(
            self.semantic_tension,
            targets.semantic_tension,
            1.5,
            1.1,
            delta,
        );
        for (node, slot) in self.node_bands.iter_mut().enumerate() {
            *slot = damp(*slot, targets.node_bands[node], 8.0, 4.0, delta);
        }

        if input.onset {
            self.onset_hold_seconds = 0.09;
        }
        let onset_target = if self.onset_hold_seconds > 0.0 {
            1.0
        } else {
            0.0
        };
        self.onset_pulse = damp(self.onset_pulse, onset_target, 18.0, 3.0, delta);
        self.onset_hold_seconds = (self.onset_hold_seconds - delta).max(0.0);

        // Phase is recomputed from absolute time, not integrated. See the module
        // comment: audio changes damped geometry, colour and scale, but never the
        // rate at which phase accumulates.
        self.set_phases(time, Self::sanitized_motion_rate(input.motion_rate));
    }

    /// `orbital_lattice_motion_ring` (`motion.c:249-270`).
    ///
    /// Rings fade out at the far boundary and back in at the near boundary, so
    /// wrapping never resets the whole lattice in one visible jump. `None` is
    /// C's `false`: an uninitialized motion or an out-of-range ring.
    #[must_use]
    pub fn ring(&self, ring_index: usize) -> Option<RingMotion> {
        if !self.initialized || ring_index >= RING_COUNT {
            return None;
        }
        let distance = wrap(
            ring_index as f64 * f64::from(RING_SPACING) + self.travel_phase,
            f64::from(PATH_LENGTH),
        ) as f32;
        let near_fade = smoothstep(0.0, 0.80, distance);
        let far_fade = 1.0 - smoothstep(PATH_LENGTH - 3.0, PATH_LENGTH, distance);
        Some(RingMotion {
            distance,
            depth_t: distance / PATH_LENGTH,
            visibility: near_fade * far_fade,
        })
    }
}

#[cfg(test)]
mod tests {
    //! The five tests from
    //! `../musializer/tests/test_scene_orbital_lattice_motion.c`, ported with
    //! their assertions and tolerances intact.

    use super::*;

    fn motion_input(time: f64, delta: f32, bands: &[f32]) -> MotionInput<'_> {
        MotionInput {
            time_seconds: time,
            delta_seconds: delta,
            bands,
            ..MotionInput::default()
        }
    }

    fn wrapped_distance(first: f64, second: f64, period: f64) -> f64 {
        let difference = (second - first).abs();
        if difference > period * 0.5 {
            period - difference
        } else {
            difference
        }
    }

    #[test]
    fn orbital_motion_damps_distinct_frequency_roles() {
        let silence = [0.0f32; 48];
        let mut shaped = [0.0f32; 48];
        for (i, value) in shaped.iter_mut().enumerate() {
            *value = if i < 9 {
                1.0
            } else if i < 28 {
                0.55
            } else {
                0.2
            };
        }
        let mut motion = Motion::new(42);
        motion.update(&motion_input(0.0, 0.0, &silence));

        let mut input = motion_input(1.0 / 60.0, 1.0 / 60.0, &shaped);
        input.rms = 0.6;
        input.spectral_flux = 0.2;
        input.onset = true;
        motion.update(&input);

        assert!(motion.bass > motion.mids);
        assert!(motion.mids > motion.treble);
        assert!(motion.bass > 0.0 && motion.bass < 1.0);
        assert!(motion.energy > 0.0 && motion.energy < 0.99);
        assert!(motion.flux > 0.0 && motion.flux < 0.8);
        assert!(motion.onset_pulse > 0.0 && motion.onset_pulse < 1.0);
        assert!(motion.node_bands[0] > motion.node_bands[8]);
        assert!(motion.node_bands[8] > motion.node_bands[15]);
    }

    #[test]
    fn orbital_motion_keeps_audio_independent_phase_without_late_song_jumps() {
        let mut bands = [0.0f32; 64];
        let mut motion = Motion::new(77);
        motion.update(&motion_input(180.0, 0.0, &bands));

        let mut previous_camera = motion.camera_phase;
        let mut previous_travel = motion.travel_phase;
        let mut previous_twist = motion.twist_phase;
        let mut previous_hue = motion.hue_degrees;
        for frame in 1..=240u32 {
            let loud = frame & 1 == 1;
            for band in bands.iter_mut() {
                *band = if loud { 1.0 } else { 0.0 };
            }
            let mut input = motion_input(180.0 + f64::from(frame) / 60.0, 1.0 / 60.0, &bands);
            input.rms = if loud { 1.0 } else { 0.0 };
            input.spectral_flux = if loud { 1.0 } else { 0.0 };
            motion.update(&input);

            // A full-scale audio swing every frame must not move phase by more
            // than one frame's worth of its own fixed rate.
            assert!(wrapped_distance(previous_camera, motion.camera_phase, TAU) < 0.002);
            assert!(
                wrapped_distance(previous_travel, motion.travel_phase, f64::from(PATH_LENGTH))
                    < 0.017
            );
            assert!(wrapped_distance(previous_twist, motion.twist_phase, TAU) < 0.007);
            assert!(wrapped_distance(previous_hue, motion.hue_degrees, 360.0) < 0.09);
            previous_camera = motion.camera_phase;
            previous_travel = motion.travel_phase;
            previous_twist = motion.twist_phase;
            previous_hue = motion.hue_degrees;
        }
    }

    #[test]
    fn orbital_motion_is_deterministic_and_rebases_seeks() {
        let mut bands = [0.0f32; 32];
        for (i, band) in bands.iter_mut().enumerate() {
            *band = i as f32 / 31.0;
        }
        let mut first = Motion::new(0x1234_5678_9abc_def0);
        let mut second = Motion::new(0x1234_5678_9abc_def0);
        for frame in 0..90u32 {
            let mut input = motion_input(
                f64::from(frame) / 30.0,
                if frame == 0 { 0.0 } else { 1.0 / 30.0 },
                &bands,
            );
            input.rms = 0.4;
            input.spectral_flux = 0.08;
            input.onset = frame == 30;
            first.update(&input);
            second.update(&input);
        }
        assert_eq!(
            first, second,
            "the same input sequence gives the same state"
        );

        // A seek rebases, so a fresh instance and a played-forward one agree.
        let mut fresh = Motion::new(0x1234_5678_9abc_def0);
        let mut seek = motion_input(41.25, 0.0, &bands);
        seek.rms = 0.7;
        seek.spectral_flux = 0.15;
        first.update(&seek);
        fresh.update(&seek);
        assert_eq!(first, fresh, "a seek discards history entirely");

        // And the rebased phase equals the phase 41.25 s of uninterrupted
        // playback would have reached.
        let mut uninterrupted = Motion::new(0x1234_5678_9abc_def0);
        for frame in 0..=2475u32 {
            let time = f64::from(frame) / 60.0;
            let mut input = motion_input(time, if frame == 0 { 0.0 } else { 1.0 / 60.0 }, &bands);
            input.rms = 0.2 + 0.5 * (frame % 17) as f32 / 16.0;
            input.spectral_flux = 0.15 * (frame % 11) as f32 / 10.0;
            uninterrupted.update(&input);
        }
        assert!(wrapped_distance(first.camera_phase, uninterrupted.camera_phase, TAU) < 0.000001);
        assert!(
            wrapped_distance(
                first.travel_phase,
                uninterrupted.travel_phase,
                f64::from(PATH_LENGTH)
            ) < 0.000001
        );
        assert!(wrapped_distance(first.twist_phase, uninterrupted.twist_phase, TAU) < 0.000001);
        assert!(wrapped_distance(first.hue_degrees, uninterrupted.hue_degrees, 360.0) < 0.000001);
    }

    #[test]
    fn orbital_motion_is_stable_across_common_frame_rates() {
        let silence = [0.0f32; 64];
        let mut bands = [0.0f32; 64];
        for (i, band) in bands.iter_mut().enumerate() {
            *band = 0.25 + (i % 7) as f32 * 0.08;
        }
        let mut thirty = Motion::new(9001);
        let mut sixty = Motion::new(9001);

        let start = motion_input(0.0, 0.0, &silence);
        thirty.update(&start);
        sixty.update(&start);
        for frame in 1..=120u32 {
            let mut input = motion_input(f64::from(frame) / 60.0, 1.0 / 60.0, &bands);
            input.rms = 0.42;
            input.spectral_flux = 0.09;
            sixty.update(&input);
        }
        for frame in 1..=60u32 {
            let mut input = motion_input(f64::from(frame) / 30.0, 1.0 / 30.0, &bands);
            input.rms = 0.42;
            input.spectral_flux = 0.09;
            thirty.update(&input);
        }

        assert!((thirty.energy - sixty.energy).abs() < 0.0001);
        assert!((thirty.bass - sixty.bass).abs() < 0.0001);
        assert!(wrapped_distance(thirty.camera_phase, sixty.camera_phase, TAU) < 0.0001);
        assert!(
            wrapped_distance(
                thirty.travel_phase,
                sixty.travel_phase,
                f64::from(PATH_LENGTH)
            ) < 0.0002
        );
        assert!(wrapped_distance(thirty.twist_phase, sixty.twist_phase, TAU) < 0.0002);
    }

    #[test]
    fn orbital_ring_convoy_wraps_one_faded_ring_instead_of_every_ring() {
        let mut motion = Motion::new(5);
        motion.update(&motion_input(0.0, 0.0, &[]));
        motion.travel_phase = f64::from(PATH_LENGTH) - 0.05;

        let mut faded = 0usize;
        let mut distances = [0.0f32; RING_COUNT];
        // The index is the ring number the sampler is asked for, not just a
        // position in `distances`, so the range loop is the honest shape here.
        #[allow(clippy::needless_range_loop)]
        for i in 0..RING_COUNT {
            let ring = motion.ring(i).expect("an initialized ring is in range");
            distances[i] = ring.distance;
            if ring.visibility < 0.1 {
                faded += 1;
            }
            assert!(ring.distance >= 0.0 && ring.distance < PATH_LENGTH);
            assert!(ring.depth_t >= 0.0 && ring.depth_t < 1.0);
            assert!(ring.visibility >= 0.0 && ring.visibility <= 1.0);
        }
        assert!((1..=3).contains(&faded), "{faded} rings faded, wanted 1..3");
        for i in 1..RING_COUNT {
            let mut spacing = distances[i] - distances[i - 1];
            if spacing < 0.0 {
                spacing += PATH_LENGTH;
            }
            assert!((spacing - RING_SPACING).abs() < 0.0001);
        }
        assert!(motion.ring(RING_COUNT).is_none());
    }

    #[test]
    fn an_uninitialized_motion_has_no_rings() {
        // C's `!motion->initialized` guard (`motion.c:254-255`). A scene that is
        // drawn before its first update must draw nothing rather than a lattice
        // sitting at phase zero.
        assert!(Motion::new(1).ring(0).is_none());
    }
}
