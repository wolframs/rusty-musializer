//! Neon Tunnel: replayable forward travel and spectrum-separated rib lighting.
//!
//! The tunnel is projected analytically in a bounded fragment shader. This
//! module owns its time and envelopes so pausing freezes every shader input.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::tunnel as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

#[derive(Clone, Debug, PartialEq)]
pub struct NeonTunnelState {
    distance: f64,
    phase: f32,
    music: [f32; 4],
}

impl NeonTunnelState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut mixed = seed.wrapping_add(0xd1b54a32d192ed03);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
        mixed ^= mixed >> 31;
        Self {
            distance: 0.0,
            phase: (mixed >> 40) as f32 / 16777216.0 * std::f32::consts::TAU,
            music: [0.0; 4],
        }
    }

    #[must_use]
    pub fn distance(&self) -> f32 {
        self.distance as f32
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

impl SceneState for NeonTunnelState {
    fn id(&self) -> SceneId {
        SceneId::NeonTunnel
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        self.distance += f64::from(dt) * f64::from(frame.setting(self.id(), control::SPEED)) * 2.4;
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
            let rate = if target > *current { 11.0 } else { 3.2 };
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
        id: SceneId::NeonTunnel,
        state_version: 1,
        make_state: |seed| Box::new(NeonTunnelState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    fn advance(state: &mut NeonTunnelState, settings: &SceneSettings, high: bool) {
        let mut bands = [0.02; 40];
        if high {
            bands[32..].fill(0.81);
        } else {
            bands[..10].fill(0.81);
        }
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / 60.0;
        frame.audio.rms = 0.4;
        frame.audio.bands = &bands;
        frame.audio.trails = &bands;
        for _ in 0..120 {
            state.update(&frame);
        }
    }

    #[test]
    fn seeded_replay_and_spectral_ribs_have_independent_evidence() {
        let settings = SceneSettings::new();
        let mut bass = NeonTunnelState::new(23);
        let mut copy = bass.clone();
        let mut treble = bass.clone();
        advance(&mut bass, &settings, false);
        advance(&mut copy, &settings, false);
        advance(&mut treble, &settings, true);
        assert_eq!(bass, copy);
        assert!(bass.distance() > 2.0, "ribs travel without drawing");
        assert!(bass.music()[0] > treble.music()[0] + 0.6);
        assert!(treble.music()[3] > bass.music()[3] + 0.6);
        assert_eq!(bass.music()[1], treble.music()[1], "equal loudness");
        assert_ne!(bass.phase(), NeonTunnelState::new(24).phase());
    }

    #[test]
    fn pause_freezes_travel_and_light() {
        let settings = SceneSettings::new();
        let mut state = NeonTunnelState::new(9);
        advance(&mut state, &settings, false);
        let before = state.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.audio.rms = 1.0;
        frame.time_seconds = 700.0;
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            frame.delta_seconds = dt;
            state.update(&frame);
            assert_eq!(state, before);
        }
    }

    #[test]
    fn zero_response_preserves_ambient_travel_and_speed_zero_parks_it() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::NeonTunnel, control::RESPONSE, 0.0);
        let mut low = NeonTunnelState::new(9);
        let mut high = low.clone();
        advance(&mut low, &settings, false);
        advance(&mut high, &settings, true);
        assert_eq!(low, high, "audio cannot leak through zero response");
        assert!(low.distance() > 2.0);
        settings.set(SceneId::NeonTunnel, control::SPEED, 0.0);
        let distance = low.distance();
        advance(&mut low, &settings, false);
        assert_eq!(low.distance(), distance);
    }

    #[test]
    fn the_same_relative_phrase_has_the_same_light_on_quiet_and_loud_tracks() {
        use crate::audio::track_dynamics::TrackDynamics;
        let settings = SceneSettings::new();
        let mut quiet = NeonTunnelState::new(1);
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
