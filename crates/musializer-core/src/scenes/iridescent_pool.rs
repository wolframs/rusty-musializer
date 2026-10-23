//! Iridescent Pool: bounded transient rings over a seeded thin-film surface.
//!
//! The core remembers six wave launches, not a CPU fluid grid. Their ages and
//! origins travel with deterministic state even when the renderer skips draws.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::iridescent as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

pub const RING_COUNT: usize = 6;
/// Short-axis surface coordinates, age in seconds, and launch strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ripple {
    pub origin: [f32; 2],
    pub age: f32,
    pub strength: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IridescentPoolState {
    seed: u64,
    clock: f64,
    rings: [Ripple; RING_COUNT],
    launches: u64,
    cooldown: f32,
    previous_hit: f32,
    previous_onset: bool,
    drive: [f32; 2],
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
            cooldown: 0.0,
            previous_hit: 0.0,
            previous_onset: false,
            drive: [0.0; 2],
        }
    }
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    #[must_use]
    pub fn phase(&self) -> f32 {
        unit(self.seed) * std::f32::consts::TAU
    }
    #[must_use]
    pub fn ripples(&self) -> &[Ripple; RING_COUNT] {
        &self.rings
    }
    #[must_use]
    pub fn drive(&self) -> [f32; 2] {
        self.drive
    }
    #[must_use]
    pub fn launches(&self) -> u64 {
        self.launches
    }
}

fn unit(mut bits: u64) -> f32 {
    bits = bits.wrapping_add(0x9e3779b97f4a7c15);
    bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((bits ^ (bits >> 31)) >> 40) as f32 / 16777216.0
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
        self.clock += f64::from(dt * frame.setting(self.id(), control::FLOW));
        for ring in &mut self.rings {
            ring.age = (ring.age + dt).min(100.0);
        }
        self.cooldown = (self.cooldown - dt).max(0.0);
        let audio = EffectInputs::from_audio(
            0.0,
            frame.audio.rms,
            frame.audio.trails,
            frame.audio.beat_phase,
            frame.audio.spectral_flux,
        );
        let finite = |x: f32| {
            if x.is_finite() {
                x.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let response = frame.setting(self.id(), control::RESPONSE);
        let target = [
            finite(audio.bass).sqrt(),
            finite(
                frame
                    .dynamics
                    .map_or(audio.rms * 1.8, |p| p.level(audio.rms)),
            ),
        ];
        for (i, target) in target.into_iter().enumerate() {
            let target = response * target;
            let rate = if target > self.drive[i] { 9.0 } else { 1.7 };
            self.drive[i] += (target - self.drive[i]) * (1.0 - (-rate * dt).exp());
        }
        let hit = finite(audio.flux * 7.0);
        let edge = (frame.audio.onset && !self.previous_onset) || hit - self.previous_hit > 0.16;
        if edge && self.cooldown <= 0.0 && response > 0.0 {
            let key = self
                .seed
                .wrapping_add(self.launches.wrapping_mul(0x9e3779b97f4a7c15));
            self.rings[self.launches as usize % RING_COUNT] = Ripple {
                origin: [
                    (unit(key) - 0.5) * 1.0,
                    (unit(key ^ 0xa0761d6478bd642f) - 0.5) * 0.8,
                ],
                age: 0.0,
                strength: response * (0.3 + 0.7 * hit),
            };
            self.launches = self.launches.wrapping_add(1);
            self.cooldown = 0.16;
        }
        self.previous_hit = hit;
        self.previous_onset = frame.audio.onset;
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::IridescentPool,
        state_version: 1,
        make_state: |seed| Box::new(IridescentPoolState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    fn run(state: &mut IridescentPoolState, settings: &SceneSettings, sound: bool) {
        let trails = [0.6; 16];
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / 60.0;
        for i in 0..360 {
            if sound {
                frame.audio.rms = 0.6;
                frame.audio.trails = &trails;
                frame.audio.onset = i % 30 == 0;
                frame.audio.spectral_flux = if frame.audio.onset { 0.15 } else { 0.0 };
            }
            state.update(&frame);
        }
    }
    #[test]
    fn replay_is_seeded_and_transients_leave_travelling_history() {
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
        assert_eq!(a.launches, 12);
        assert_eq!(quiet.launches, 0);
        assert_ne!(a.rings, other.rings);
        assert!(quiet.clock > 1.0);
        assert!(a.rings.iter().all(|r| r.age > 0.1 && r.strength > 0.5));
        assert!(a.drive.iter().all(|v| *v > 0.5));
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
        assert_eq!(sound.clock, quiet.clock);
        assert_eq!(sound.drive, quiet.drive);
        assert_eq!(sound.rings, quiet.rings);
        assert_eq!(sound.launches, 0);
    }
}
