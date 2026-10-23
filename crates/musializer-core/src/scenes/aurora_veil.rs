//! Aurora Veil: deterministic musical envelopes for a GPU curtain of light.
//!
//! Original scene inspired by the visible React Bits Aurora and Audio Borealis
//! demos. Rendering has no history: skipped draws cannot change a later still.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::aurora as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

#[derive(Clone, Debug, PartialEq)]
pub struct AuroraVeilState {
    clock: f64,
    phase: f32,
    music: [f32; 4],
}

impl AuroraVeilState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut mixed = seed.wrapping_add(0x9e3779b97f4a7c15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
        mixed ^= mixed >> 31;
        Self {
            clock: 0.0,
            phase: (mixed >> 40) as f32 / 16777216.0 * std::f32::consts::TAU,
            music: [0.0; 4],
        }
    }

    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }

    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Bass, track-relative loudness, middle spectrum, high spectrum.
    #[must_use]
    pub fn music(&self) -> [f32; 4] {
        self.music
    }
}

impl SceneState for AuroraVeilState {
    fn id(&self) -> SceneId {
        SceneId::AuroraVeil
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        self.clock += f64::from(dt) * f64::from(frame.setting(self.id(), control::FLOW));
        let inputs = EffectInputs::from_audio(
            0.0,
            frame.audio.rms,
            frame.audio.trails,
            frame.audio.beat_phase,
            frame.audio.spectral_flux,
        );
        let loud = frame.dynamics.map_or(frame.audio.rms * 1.8, |profile| {
            profile.level(frame.audio.rms)
        });
        let (_, mids, treble) = frame.audio.spectral_regions();
        let response = frame.setting(self.id(), control::RESPONSE);
        for (current, value) in self.music.iter_mut().zip([inputs.bass, loud, mids, treble]) {
            let target = if value.is_finite() {
                value.clamp(0.0, 1.0).sqrt() * response
            } else {
                0.0
            };
            // A slow release lets a phrase open a whole curtain rather than
            // making every analyzer tick blink a line of pixels.
            let rate = if target > *current { 4.5 } else { 1.1 };
            *current += (target - *current) * (1.0 - (-rate * dt).exp());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::AuroraVeil,
        state_version: 1,
        make_state: |seed| Box::new(AuroraVeilState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    fn replay(state: &mut AuroraVeilState, settings: &SceneSettings, audible: bool) {
        let low = [0.04; 24];
        let high = [0.72; 24];
        for tick in 0..180 {
            let mut frame = SceneFrame::idle(settings);
            frame.delta_seconds = 1.0 / 60.0;
            if audible {
                frame.audio.rms = if tick % 60 < 24 { 0.65 } else { 0.12 };
                frame.audio.bands = if tick % 60 < 24 { &high } else { &low };
                frame.audio.trails = &high;
            }
            state.update(&frame);
        }
    }

    #[test]
    fn replay_uses_seed_time_and_music_without_drawing() {
        let settings = SceneSettings::new();
        let mut a = AuroraVeilState::new(42);
        let initial = a.clone();
        let mut b = a.clone();
        let mut quiet = a.clone();
        replay(&mut a, &settings, true);
        replay(&mut b, &settings, true);
        replay(&mut quiet, &settings, false);
        assert_eq!(a, b, "identical input sequences reproduce every uniform");
        assert!(a.clock() > initial.clock());
        assert!(a.music()[0] > quiet.music()[0] + 0.5);
        assert!(a.music()[1] > quiet.music()[1] + 0.2);
        assert_ne!(initial.phase(), AuroraVeilState::new(43).phase());
        assert_eq!(a.clock(), quiet.clock(), "music cannot alter replay time");
    }

    #[test]
    fn pause_freezes_every_uniform_even_when_audio_changes() {
        let settings = SceneSettings::new();
        let mut state = AuroraVeilState::new(7);
        replay(&mut state, &settings, true);
        let before = state.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.audio.rms = 1.0;
        frame.time_seconds = 900.0;
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            frame.delta_seconds = dt;
            state.update(&frame);
            assert_eq!(state, before);
        }
    }

    #[test]
    fn response_zero_is_an_audio_independent_ambient_scene() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::AuroraVeil, control::RESPONSE, 0.0);
        let mut music = AuroraVeilState::new(7);
        let mut quiet = music.clone();
        replay(&mut music, &settings, true);
        replay(&mut quiet, &settings, false);
        assert_eq!(music, quiet);
        assert!(music.clock() > 1.0, "ambient curtains still flow");
    }

    #[test]
    fn the_same_relative_phrase_has_the_same_light_on_quiet_and_loud_tracks() {
        use crate::audio::track_dynamics::TrackDynamics;
        let settings = SceneSettings::new();
        let mut quiet = AuroraVeilState::new(1);
        let mut loud = quiet.clone();
        let mut absolute = quiet.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 0.25;
        for _ in 0..12 {
            frame.audio.rms = 0.2;
            frame.dynamics = TrackDynamics::from_bounds(0.1, 0.3);
            quiet.update(&frame);
            frame.audio.rms = 0.6;
            frame.dynamics = TrackDynamics::from_bounds(0.3, 0.9);
            loud.update(&frame);
            frame.dynamics = None;
            absolute.update(&frame);
        }
        assert!(
            (quiet.music()[1] - loud.music()[1]).abs() < 1.0e-5,
            "track normalization must preserve a phrase across mastering levels"
        );
        assert!(
            absolute.music()[1] > loud.music()[1] + 0.25,
            "removing the profile must exercise the absolute fallback"
        );
    }
}
