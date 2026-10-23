//! Liquid Chrome: a deterministic material clock and musical pressure followers.
//!
//! The surface is evaluated analytically on the GPU; there is no simulation
//! buffer to warm up during drawless still replay. Original implementation,
//! inspired by reflective liquid materials, with no third-party source.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::chrome as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidChromeState {
    phase: [f32; 4],
    clock: f64,
    drive: [f32; 3],
}

impl LiquidChromeState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut bits = seed;
        Self {
            phase: std::array::from_fn(|_| {
                bits = bits.wrapping_add(0x9e3779b97f4a7c15);
                let mut z = bits;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                ((z ^ (z >> 31)) >> 40) as f32 * (std::f32::consts::TAU / 16777216.0)
            }),
            clock: 0.0,
            drive: [0.0; 3],
        }
    }
    #[must_use]
    pub fn phase(&self) -> [f32; 4] {
        self.phase
    }
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    /// Smoothed bass pressure, track-relative loudness and transient strength.
    #[must_use]
    pub fn drive(&self) -> [f32; 3] {
        self.drive
    }
}

impl SceneState for LiquidChromeState {
    fn id(&self) -> SceneId {
        SceneId::LiquidChrome
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        let response = frame.setting(self.id(), control::RESPONSE);
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
        let target = [
            finite(audio.bass).sqrt(),
            finite(
                frame
                    .dynamics
                    .map_or(audio.rms * 1.8, |p| p.level(audio.rms)),
            ),
            finite(audio.flux * 7.0),
        ];
        for (i, target) in target.into_iter().enumerate() {
            let target = target * response;
            let rate = if target > self.drive[i] { 10.0 } else { 2.1 };
            self.drive[i] += (target - self.drive[i]) * (1.0 - (-rate * dt).exp());
        }
        self.clock += f64::from(dt * frame.setting(self.id(), control::FLOW));
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::LiquidChrome,
        state_version: 1,
        make_state: |seed| Box::new(LiquidChromeState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    fn run(state: &mut LiquidChromeState, settings: &SceneSettings, sound: bool) {
        let trails = [0.8; 16];
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / 60.0;
        if sound {
            frame.audio.trails = &trails;
            frame.audio.rms = 0.6;
            frame.audio.spectral_flux = 0.13;
        }
        for _ in 0..120 {
            state.update(&frame);
        }
    }
    #[test]
    fn replay_seed_and_music_reach_the_material() {
        let settings = SceneSettings::new();
        let mut a = LiquidChromeState::new(42);
        let mut replay = a.clone();
        let mut quiet = a.clone();
        run(&mut a, &settings, true);
        run(&mut replay, &settings, true);
        run(&mut quiet, &settings, false);
        assert_eq!(a, replay);
        assert_ne!(a.phase, LiquidChromeState::new(43).phase);
        assert!(
            a.clock > 0.5 && quiet.clock == a.clock,
            "ambient motion survives silence"
        );
        assert!(a.drive.iter().all(|v| *v > 0.5));
        assert_eq!(quiet.drive, [0.0; 3]);
    }
    #[test]
    fn pause_freezes_every_render_input_even_if_audio_and_track_time_change() {
        let settings = SceneSettings::new();
        let mut state = LiquidChromeState::new(17);
        run(&mut state, &settings, true);
        let before = state.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.time_seconds = 900.0;
        frame.audio.rms = 1.0;
        for delta in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            frame.delta_seconds = delta;
            state.update(&frame);
            assert_eq!(state, before);
        }
    }
    #[test]
    fn disabled_response_has_identical_ambient_replay() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SceneId::LiquidChrome, control::RESPONSE, 0.0));
        let mut sound = LiquidChromeState::new(8);
        let mut quiet = sound.clone();
        run(&mut sound, &settings, true);
        run(&mut quiet, &settings, false);
        assert_eq!(sound, quiet);
    }
    #[test]
    fn loudness_uses_this_tracks_range() {
        let settings = SceneSettings::new();
        let mut low = LiquidChromeState::new(2);
        let mut high = low.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 0.1;
        frame.dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.7, 0.9);
        for _ in 0..20 {
            frame.audio.rms = 0.7;
            low.update(&frame);
            frame.audio.rms = 0.9;
            high.update(&frame);
        }
        assert!(high.drive[1] - low.drive[1] > 0.8);
    }
}
