//! Iridescent Pool: an oil film on dark, deep water, seen from above while the
//! camera glides over it. This half is the deterministic part: the material
//! clock, the musical followers and the bounded history of drops.
//!
//! The water, the film and the pool floor are evaluated analytically on the
//! GPU (`resources/shaders/glsl330/iridescent_pool.fs`); the core remembers
//! twelve drops, not a fluid grid, so drawless still replay loses nothing.
//!
//! What the music does, and through which lane (docs/MATERIAL_SCENES.md):
//!
//! - **kicks** (the bass-excursion detector Clawd measured on real mixes)
//!   land big drops near the middle of the frame: a ring train, a hole torn
//!   in the oil that closes over seconds, and bright caustic rings on the
//!   floor below;
//! - **other transients** (onset edges and flux jumps) are rain: small drops
//!   anywhere in view;
//! - **bass** raises a long, slow swell that rolls every reflection;
//! - **track-relative loudness** adds fine chop and lifts the light.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::iridescent as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Drops kept alive at once. The replacement rule takes the slot with the
/// least wave left in it, so a burst of rain never cuts off a kick's rings.
pub const RING_COUNT: usize = 12;

/// The bass-transient gate; see `scenes::clawd::KICK_THRESHOLD` for the
/// measurement behind 0.05.
const KICK_THRESHOLD: f32 = 0.05;
const KICK_REFRACTORY: f32 = 0.25;
/// Minimum seconds between rain drops.
const DROP_COOLDOWN: f32 = 0.12;
const PULSE_DECAY: f32 = 0.3;

/// The camera's height over the water and the horizontal distance to the
/// point it looks at. The shader's `CAMERA_HEIGHT`/`CAMERA_REACH` carry the
/// same numbers; drops are placed where that camera can see them.
pub const CAMERA_HEIGHT: f32 = 3.2;
pub const CAMERA_REACH: f32 = 3.0;

/// One drop: water-plane position `(x, z)`, seconds since it landed, and
/// its strength (rain about 0.2, a full kick 1.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ripple {
    pub origin: [f32; 2],
    pub age: f32,
    pub strength: f32,
}

impl Ripple {
    /// Seconds for this drop's waves to fall to 1/e. The shader's envelope
    /// uses the same expression: bigger drops ring longer.
    #[must_use]
    pub fn decay(&self) -> f32 {
        0.6 + 1.1 * self.strength
    }
    /// How much wave this drop still carries, for slot replacement.
    #[must_use]
    pub fn remaining(&self) -> f32 {
        self.strength * (-self.age / self.decay()).exp()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IridescentPoolState {
    seed: u64,
    clock: f64,
    rings: [Ripple; RING_COUNT],
    launches: u64,
    kicks: u64,
    kick_refractory: f32,
    cooldown: f32,
    previous_hit: f32,
    previous_onset: bool,
    /// Bass pressure, track-relative loudness.
    drive: [f32; 2],
    pulse: f32,
}

fn unit(mut bits: u64) -> f32 {
    bits = bits.wrapping_add(0x9e37_79b9_7f4a_7c15);
    bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((bits ^ (bits >> 31)) >> 40) as f32 / 16_777_216.0
}

fn finite01(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Mean positive excursion of the lowest quarter of the bands over their own
/// trails: "the bass moved", over the same bands `bass_from_trails` reads.
#[must_use]
pub fn bass_excursion(audio: &SceneAudioFrame<'_>) -> f32 {
    let count = audio.bands.len().min(audio.trails.len());
    if count == 0 {
        return 0.0;
    }
    let low = (count / 4).max(1);
    let sum: f32 = audio.bands[..low]
        .iter()
        .zip(&audio.trails[..low])
        .map(|(band, trail)| (band - trail).max(0.0))
        .sum();
    let value = sum / low as f32;
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

impl IridescentPoolState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            clock: 0.0,
            rings: [Ripple {
                origin: [0.0; 2],
                age: 100.0,
                strength: 0.0,
            }; RING_COUNT],
            launches: 0,
            kicks: 0,
            kick_refractory: 0.0,
            cooldown: 0.0,
            previous_hit: 0.0,
            previous_onset: false,
            drive: [0.0; 2],
            pulse: 0.0,
        }
    }
    /// The material clock: seconds of track time scaled by Flow.
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    /// A seeded offset for the film's noise, in radians.
    #[must_use]
    pub fn phase(&self) -> f32 {
        unit(self.seed) * std::f32::consts::TAU
    }
    #[must_use]
    pub fn ripples(&self) -> &[Ripple; RING_COUNT] {
        &self.rings
    }
    /// Smoothed bass pressure and track-relative loudness.
    #[must_use]
    pub fn drive(&self) -> [f32; 2] {
        self.drive
    }
    /// The kick envelope: 1 on a full kick, decaying over ~0.3 s.
    #[must_use]
    pub fn pulse(&self) -> f32 {
        self.pulse
    }
    /// Every drop launched so far, kicks and rain.
    #[must_use]
    pub fn launches(&self) -> u64 {
        self.launches
    }
    /// The kicks among them.
    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kicks
    }

    /// Where the camera hangs over the water, `(x, z)`: a slow sideways
    /// sway and a steady glide forward, both on the material clock, so the
    /// film and the floor slide past at different depths.
    #[must_use]
    pub fn camera_drift(&self) -> [f32; 2] {
        let phase = f64::from(self.phase());
        [
            (0.8 * (self.clock * 0.021 + phase).sin()) as f32,
            (self.clock * 0.06) as f32,
        ]
    }

    /// A point on the water given as `(across, depth)` from the camera's
    /// footprint, in world `(x, z)`. The camera looks along +z, and the
    /// shader's screen-right is world -x, so `across` is negated.
    fn water_point(&self, across: f32, depth: f32) -> [f32; 2] {
        let [x, z] = self.camera_drift();
        [x - across, z + depth]
    }

    fn launch(&mut self, big: bool, hit: f32, response: f32) {
        let key = self
            .seed
            .wrapping_mul(0xd6e8_feb8_6659_fd93)
            .wrapping_add(self.launches.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let (u, v) = (unit(key), unit(key ^ 0xa076_1d64_78bd_642f));
        // Kicks land near the middle of the frame; rain anywhere in view.
        // The visible half-width grows with distance from the camera.
        let (depth, spread) = if big {
            (2.6 + 1.3 * v, 0.55)
        } else {
            (1.9 + 3.2 * v, 0.95)
        };
        let half_width = spread * 0.36 * (CAMERA_HEIGHT * CAMERA_HEIGHT + depth * depth).sqrt();
        let origin = self.water_point((u - 0.5) * 2.0 * half_width, depth);
        let strength = if big {
            (response * (0.6 + 0.4 * hit)).min(1.5)
        } else {
            (response * (0.14 + 0.22 * hit)).min(0.6)
        };
        let slot = (0..RING_COUNT)
            .min_by(|a, b| {
                self.rings[*a]
                    .remaining()
                    .total_cmp(&self.rings[*b].remaining())
            })
            .unwrap_or(0);
        self.rings[slot] = Ripple {
            origin,
            age: 0.0,
            strength,
        };
        self.launches = self.launches.wrapping_add(1);
    }
}

impl SceneState for IridescentPoolState {
    fn id(&self) -> SceneId {
        SceneId::IridescentPool
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        // Launched drops always finish, even at Flow zero.
        for ring in &mut self.rings {
            ring.age = (ring.age + dt).min(100.0);
        }
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.kick_refractory = (self.kick_refractory - dt).max(0.0);
        self.pulse *= (-dt / PULSE_DECAY).exp();
        let audio = EffectInputs::from_audio(
            0.0,
            frame.audio.rms,
            frame.audio.trails,
            frame.audio.beat_phase,
            frame.audio.spectral_flux,
        );
        let response = frame.setting(self.id(), control::RESPONSE);
        let target = [
            finite01(audio.bass).sqrt(),
            finite01(
                frame
                    .dynamics
                    .map_or(audio.rms * 1.8, |p| p.level(audio.rms)),
            ),
        ];
        // Water has inertia: a swell rises over a few beats and dies slowly.
        for (i, target) in target.into_iter().enumerate() {
            let target = response * target;
            let rate = if target > self.drive[i] { 4.0 } else { 0.9 };
            self.drive[i] += (target - self.drive[i]) * (1.0 - (-rate * dt).exp());
        }

        let kick = bass_excursion(&frame.audio);
        let hit = finite01(audio.flux * 7.0);
        if response > 0.0 && kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            let strength = (kick / (2.4 * KICK_THRESHOLD)).clamp(0.0, 1.0);
            self.launch(true, strength, response);
            self.kicks = self.kicks.wrapping_add(1);
            self.kick_refractory = KICK_REFRACTORY;
            self.pulse = self.pulse.max((response * (0.5 + 0.5 * strength)).min(1.0));
            // A kick and the onset it raises are one drop, not two.
            self.cooldown = DROP_COOLDOWN;
        }
        let edge = (frame.audio.onset && !self.previous_onset) || hit - self.previous_hit > 0.16;
        if edge && self.cooldown <= 0.0 && response > 0.0 {
            self.launch(false, hit, response);
            self.cooldown = DROP_COOLDOWN;
        }
        self.previous_hit = hit;
        self.previous_onset = frame.audio.onset;
        self.clock += f64::from(dt * frame.setting(self.id(), control::FLOW));
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::IridescentPool,
        // 2: the oil-on-water rework (twelve drops, kicks, camera glide).
        state_version: 2,
        make_state: |seed| Box::new(IridescentPoolState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    /// Six seconds of a groove: a kick on every half second, an onset on
    /// the off-beat.
    fn run(state: &mut IridescentPoolState, settings: &SceneSettings, sound: bool) {
        let trails = [0.6; 16];
        let mut kick_bands = [0.6; 16];
        for band in kick_bands.iter_mut().take(4) {
            *band = 0.78;
        }
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / 60.0;
        for i in 0..360 {
            if sound {
                frame.audio.rms = 0.6;
                frame.audio.trails = &trails;
                frame.audio.bands = if i % 30 == 0 { &kick_bands } else { &trails };
                frame.audio.onset = i % 30 == 15;
                frame.audio.spectral_flux = if frame.audio.onset { 0.15 } else { 0.0 };
            }
            state.update(&frame);
        }
    }

    #[test]
    fn replay_is_seeded_and_kicks_and_rain_leave_travelling_history() {
        let settings = SceneSettings::new();
        let mut a = IridescentPoolState::new(11);
        let mut b = a.clone();
        let mut quiet = a.clone();
        let mut other = IridescentPoolState::new(12);
        run(&mut a, &settings, true);
        run(&mut b, &settings, true);
        run(&mut quiet, &settings, false);
        run(&mut other, &settings, true);
        assert_eq!(a, b);
        assert_eq!(a.kicks, 12);
        assert_eq!(a.launches, 24, "twelve kicks and twelve rain drops");
        assert_eq!(quiet.launches, 0);
        assert_ne!(a.rings, other.rings);
        assert!(quiet.clock > 1.0);
        assert!(a.rings.iter().all(|r| r.age > 0.0 && r.strength > 0.1));
        assert!(a.drive.iter().all(|v| *v > 0.5));
    }

    #[test]
    fn kicks_are_big_drops_and_rain_is_small() {
        let settings = SceneSettings::new();
        let mut state = IridescentPoolState::new(5);
        run(&mut state, &settings, true);
        let big = state.rings.iter().filter(|r| r.strength >= 0.6).count();
        let small = state.rings.iter().filter(|r| r.strength < 0.4).count();
        assert!(big >= 4 && small >= 2, "{:?}", state.rings);
    }

    #[test]
    fn a_new_drop_replaces_the_ring_with_least_wave_left() {
        let mut state = IridescentPoolState::new(9);
        for (i, ring) in state.rings.iter_mut().enumerate() {
            *ring = Ripple {
                origin: [0.0; 2],
                age: 0.1,
                strength: 1.0,
            };
            if i == 7 {
                ring.age = 6.0;
            }
        }
        state.launch(false, 0.5, 1.0);
        assert_eq!(state.rings[7].age, 0.0);
        assert!(state.rings[7].strength < 0.4);
    }

    #[test]
    fn drops_land_where_the_camera_looks() {
        let settings = SceneSettings::new();
        let mut state = IridescentPoolState::new(21);
        run(&mut state, &settings, true);
        let [x, z] = state.camera_drift();
        for ring in &state.rings {
            let depth = ring.origin[1] - z;
            // The camera has glided forward a little since the oldest landed.
            assert!((1.6..=5.2).contains(&depth), "depth {depth}");
            let half = 0.36 * (CAMERA_HEIGHT * CAMERA_HEIGHT + depth * depth).sqrt();
            assert!((ring.origin[0] - x).abs() <= half + 0.1);
        }
    }

    #[test]
    fn held_onset_is_one_drop_and_pause_cannot_age_or_launch_it() {
        let settings = SceneSettings::new();
        let mut state = IridescentPoolState::new(3);
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        frame.audio.onset = true;
        for _ in 0..100 {
            state.update(&frame);
        }
        assert_eq!(state.launches, 1);
        let before = state.clone();
        frame.time_seconds = 400.0;
        frame.audio.spectral_flux = 1.0;
        for delta in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            frame.delta_seconds = delta;
            state.update(&frame);
            assert_eq!(state, before);
        }
    }

    #[test]
    fn response_zero_keeps_ambient_surface_without_audio_history() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SceneId::IridescentPool, control::RESPONSE, 0.0));
        let mut sound = IridescentPoolState::new(1);
        let mut quiet = sound.clone();
        run(&mut sound, &settings, true);
        run(&mut quiet, &settings, false);
        assert_eq!(sound, quiet);
        assert_eq!(sound.launches, 0);
    }

    #[test]
    fn loudness_uses_this_tracks_range() {
        let settings = SceneSettings::new();
        let mut low = IridescentPoolState::new(2);
        let mut high = low.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 0.1;
        frame.dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.7, 0.9);
        for _ in 0..40 {
            frame.audio.rms = 0.7;
            low.update(&frame);
            frame.audio.rms = 0.9;
            high.update(&frame);
        }
        assert!(high.drive[1] - low.drive[1] > 0.8);
    }
}
