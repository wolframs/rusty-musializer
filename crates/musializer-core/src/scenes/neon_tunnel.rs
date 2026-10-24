//! Neon Tunnel: replayable forward travel through a lit, wet neon tunnel.
//!
//! The tunnel is ray cast analytically in one fragment pass
//! (`neon_tunnel.fs`). This module owns its travel, its envelopes and a short
//! **spectrum history**, so pausing freezes every shader input and an offline
//! replay reconstructs every frame.
//!
//! The history is what makes the rings *be* the music. Each neon ring shows
//! the spectrum around its circumference — bass on the floor, treble on the
//! ceiling — as it was when a light-speed wave fired from the camera reached
//! it, so a kick lights the nearest ring at once and then visibly races away
//! down the tunnel. The near ring is immediate, which is what makes the hit
//! felt; the trail is what makes it seen.
//!
//! Musically: kicks surge the travel, punch the field of view and flash the
//! exit light; the cruise speed rides the kick rate, so a busier groove flies
//! faster; loudness lights the haze; a quiet passage slows to a glide with the
//! rings down to embers.

use super::aurora_veil::{follow, Features, KickDetector};
use crate::scene::settings::index::tunnel as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Spectrum groups around a ring's circumference (mirrored left/right).
pub const HISTORY_BANDS: usize = 16;
/// Channels per history slot: the bands, then the kick flash, then loudness.
pub const HISTORY_CHANNELS: usize = HISTORY_BANDS + 2;
/// Slots of history; at [`HISTORY_SLOT_SECONDS`] that is a little over a
/// second, which is how long the wave takes to reach the fog.
pub const HISTORY_SLOTS: usize = 64;
/// History resolution in track seconds, independent of the frame rate, so a
/// 30 fps export and a 60 Hz preview sample the same moments.
pub const HISTORY_SLOT_SECONDS: f32 = 1.0 / 60.0;

#[derive(Clone, Debug, PartialEq)]
pub struct NeonTunnelState {
    distance: f64,
    phase: f32,
    /// Bass, track-relative loudness, mids, treble.
    music: [f32; 4],
    /// Kick-driven forward lunge, decays over ~0.4 s.
    surge: f32,
    /// Field-of-view punch envelope.
    punch: f32,
    /// Exit-light flash envelope.
    flash: f32,
    /// Tempo factor from the kick rate, slow.
    cruise: f32,
    kick: KickDetector,
    /// Ring buffer of `HISTORY_SLOTS` samples of `HISTORY_CHANNELS` values.
    history: Vec<f32>,
    /// Index of the newest slot.
    head: usize,
    /// Track seconds this state has heard, so slots are counted in time
    /// rather than accumulated per frame (which would drift with the rate).
    elapsed: f64,
    /// Slots written so far, `floor(elapsed / HISTORY_SLOT_SECONDS)`.
    written: u64,
    /// Last frame's sample, so a slot written between two frames is
    /// interpolated rather than duplicated.
    previous: [f32; HISTORY_CHANNELS],
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
            surge: 0.0,
            punch: 0.0,
            flash: 0.0,
            cruise: 0.0,
            kick: KickDetector::new(),
            history: vec![0.0; HISTORY_SLOTS * HISTORY_CHANNELS],
            head: 0,
            elapsed: 0.0,
            written: 0,
            previous: [0.0; HISTORY_CHANNELS],
        }
    }

    /// Distance travelled. Never wrapped: no single period divides every
    /// repeat the shader uses (ring spacing is continuous in `depth`), and at
    /// the fastest setting a long track reaches only ~10^4, where single
    /// precision still resolves a thousandth of a ring.
    #[must_use]
    pub fn distance(&self) -> f32 {
        self.distance as f32
    }

    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Bass, track-relative loudness, mids, treble.
    #[must_use]
    pub fn music(&self) -> [f32; 4] {
        self.music
    }

    /// Kick envelopes: `[surge, punch, flash]`.
    #[must_use]
    pub fn accents(&self) -> [f32; 3] {
        [self.surge, self.punch, self.flash]
    }

    #[must_use]
    pub fn cruise(&self) -> f32 {
        self.cruise
    }

    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kick.count()
    }

    /// Fraction of a slot since the newest one was written: the shader's
    /// sub-slot offset, so the history scrolls continuously.
    #[must_use]
    pub fn history_lag(&self) -> f32 {
        (self.elapsed / f64::from(HISTORY_SLOT_SECONDS) - self.written as f64).clamp(0.0, 1.0)
            as f32
    }

    /// Slot `age` (0 = newest) as `HISTORY_CHANNELS` values in `0..=1`.
    #[must_use]
    pub fn history_slot(&self, age: usize) -> &[f32] {
        let index = (self.head + HISTORY_SLOTS - age % HISTORY_SLOTS) % HISTORY_SLOTS;
        &self.history[index * HISTORY_CHANNELS..(index + 1) * HISTORY_CHANNELS]
    }

    /// One CSV fragment for `examples/atmosphere_probe.rs`.
    #[must_use]
    pub fn probe_line(&self) -> String {
        format!(
            "t:{:.3}/{:.3}/{:.3}/{:.3} surge={:.3} punch={:.3} cruise={:.3} kicks={} rate={:.2}",
            self.music[0],
            self.music[1],
            self.music[2],
            self.music[3],
            self.surge,
            self.punch,
            self.cruise,
            self.kick.count(),
            self.kick.rate()
        )
    }

    fn sample(&self, bands: &[f32], response: f32) -> [f32; HISTORY_CHANNELS] {
        let mut sample = [0.0; HISTORY_CHANNELS];
        if !bands.is_empty() {
            for (group, value) in sample[..HISTORY_BANDS].iter_mut().enumerate() {
                let start = group * bands.len() / HISTORY_BANDS;
                let end = ((group + 1) * bands.len() / HISTORY_BANDS)
                    .max(start + 1)
                    .min(bands.len());
                let mean = bands[start..end].iter().copied().sum::<f32>() / (end - start) as f32;
                // Analyzer bands sit low; the upper groups lower still. A
                // per-group tilt lets the ceiling light as readily as the floor.
                let tilt = 1.4 + 1.6 * group as f32 / (HISTORY_BANDS - 1) as f32;
                let level = if mean.is_finite() { mean } else { 0.0 };
                *value = ((level * tilt).clamp(0.0, 1.0) * response).clamp(0.0, 1.0);
            }
        }
        sample[HISTORY_BANDS] = self.flash.clamp(0.0, 1.0);
        sample[HISTORY_BANDS + 1] = self.music[1].clamp(0.0, 1.0);
        sample
    }

    fn push(&mut self, sample: &[f32; HISTORY_CHANNELS]) {
        self.head = (self.head + 1) % HISTORY_SLOTS;
        let at = self.head * HISTORY_CHANNELS;
        self.history[at..at + HISTORY_CHANNELS].copy_from_slice(sample);
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
        let response = frame.setting(self.id(), control::RESPONSE);
        let features = Features::of(&frame.audio, frame.dynamics);

        let bass = ((features.bass - 0.18) / 0.5).clamp(0.0, 1.0);
        let targets = [
            bass * response,
            features.level * response,
            (features.mids * 1.6).clamp(0.0, 1.0) * response,
            (features.treble * 2.2).clamp(0.0, 1.0) * response,
        ];
        let rates = [(11.0, 3.2), (3.0, 0.8), (6.0, 2.0), (12.0, 3.5)];
        for ((current, target), (attack, release)) in self.music.iter_mut().zip(targets).zip(rates)
        {
            follow(current, target, attack, release, dt);
        }

        self.surge *= (-dt * 2.8).exp();
        self.punch *= (-dt * 7.0).exp();
        self.flash *= (-dt * 4.5).exp();
        if let Some(strength) = self.kick.step(features.kick * response, dt) {
            self.surge = self.surge.max(strength);
            self.punch = self.punch.max(strength);
            self.flash = self.flash.max(strength);
        }
        // Two kicks a second (120 BPM on the beat) is "cruising"; the factor
        // saturates so a drum fill cannot send the camera to warp.
        let tempo = (self.kick.rate() / 2.0).clamp(0.0, 1.5);
        follow(&mut self.cruise, tempo, 0.6, 0.25, dt);

        // Quiet passages glide at half the set speed; loudness, the cruise
        // and a kick's surge add on top. Speed zero still parks everything.
        let pace = 0.5 + 0.8 * self.music[1] + 0.6 * self.cruise + 1.4 * self.surge;
        self.distance += f64::from(dt)
            * f64::from(frame.setting(self.id(), control::SPEED))
            * 2.4
            * f64::from(pace);

        let current = self.sample(frame.audio.bands, response);
        let slot = f64::from(HISTORY_SLOT_SECONDS);
        let from = self.elapsed;
        self.elapsed += f64::from(dt);
        // The epsilon keeps a frame that ends exactly on a slot boundary from
        // landing on either side of it depending on how its delta rounded.
        let due = (self.elapsed / slot + 1.0e-6).floor() as u64;
        while self.written < due {
            self.written += 1;
            let boundary = self.written as f64 * slot;
            let into = ((boundary - from) / f64::from(dt)).clamp(0.0, 1.0) as f32;
            let mut sample = [0.0; HISTORY_CHANNELS];
            for ((out, old), new) in sample.iter_mut().zip(self.previous).zip(current) {
                *out = old + (new - old) * into;
            }
            self.push(&sample);
        }
        self.previous = current;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::NeonTunnel,
        state_version: 2,
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

    fn kicks(state: &mut NeonTunnelState, settings: &SceneSettings, seconds: f32, fps: f32) {
        let calm = [0.3; 40];
        let mut hit = [0.3; 40];
        hit[..10].fill(0.62);
        let frames = (seconds * fps) as usize;
        let period = (fps / 2.0) as usize;
        for tick in 0..frames {
            let mut frame = SceneFrame::idle(settings);
            frame.delta_seconds = 1.0 / fps;
            frame.audio.rms = 0.4;
            frame.audio.bands = if tick % period < 2 { &hit } else { &calm };
            frame.audio.trails = &calm;
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
        // The history carries the spectrum's shape: floor lit for bass,
        // ceiling for treble.
        let floor = |s: &NeonTunnelState| s.history_slot(0)[0];
        let ceiling = |s: &NeonTunnelState| s.history_slot(0)[HISTORY_BANDS - 1];
        assert!(floor(&bass) > floor(&treble) + 0.5);
        assert!(ceiling(&treble) > ceiling(&bass) + 0.5);
    }

    #[test]
    fn a_kick_fires_down_the_tunnel_through_the_history() {
        let settings = SceneSettings::new();
        let mut state = NeonTunnelState::new(5);
        kicks(&mut state, &settings, 1.02, 60.0);
        assert!(state.kicks() >= 2);
        // The newest flash is near the front of the history, and an older
        // copy of the same kick is further down: the wave travels.
        let flash: Vec<f32> = (0..HISTORY_SLOTS)
            .map(|age| state.history_slot(age)[HISTORY_BANDS])
            .collect();
        let near = flash[..8].iter().copied().fold(0.0, f32::max);
        let far = flash[24..40].iter().copied().fold(0.0, f32::max);
        assert!(near > 0.3 && far > 0.3, "near {near} far {far}");
        assert!(state.accents()[1] > 0.0);
    }

    #[test]
    fn history_samples_track_time_not_frames() {
        // A 30 fps replay and a 60 fps replay of the same music write the
        // same number of slots, so the wave reaches the same ring at the same
        // moment in an export as in the preview.
        let settings = SceneSettings::new();
        let mut slow = NeonTunnelState::new(5);
        let mut fast = slow.clone();
        kicks(&mut slow, &settings, 2.0, 30.0);
        kicks(&mut fast, &settings, 2.0, 60.0);
        assert_eq!(slow.head, fast.head);
        assert!((slow.history_lag() - fast.history_lag()).abs() < 1.0e-3);
    }

    #[test]
    fn pause_freezes_travel_and_light() {
        let settings = SceneSettings::new();
        let mut state = NeonTunnelState::new(9);
        advance(&mut state, &settings, false);
        kicks(&mut state, &settings, 1.0, 60.0);
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
        kicks(&mut low, &settings, 1.0, 60.0);
        advance(&mut high, &settings, true);
        kicks(&mut high, &settings, 1.0, 60.0);
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
