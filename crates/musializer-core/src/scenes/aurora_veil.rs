//! Aurora Veil: deterministic musical envelopes for a ray-cast night sky.
//!
//! The picture is a lake under a northern sky (`aurora_veil.fs`): folded
//! curtains of light standing in perspective over a mountain ridge, mirrored
//! in the water. Everything that moves is owned here, so pausing freezes every
//! shader input and an offline replay reconstructs the state exactly.
//!
//! What the music does, and why each mapping is the one it is:
//!
//! - **Activity** is the track-relative loudness on a phrase-length follower.
//!   It decides how much of the sky is alight: a quiet passage is one calm
//!   arc low over the ridge that breathes, a loud one breaks up overhead into
//!   three folded curtains with crimson tops. Slow on purpose — an aurora that
//!   changes shape on every analyzer tick reads as a meter, not as a sky.
//! - **Bass** is weight: the lower border brightens and drops, and a pink
//!   nitrogen fringe appears under it, which is what a real substorm does.
//! - **Kicks** launch *flaming* waves: bands of brightness racing up the rays
//!   from the lower border. One accent per hit, legible at any distance.
//! - **Treble** drives the ray shimmer and the stars' twinkle, through a phase
//!   accumulator so a change of rate can never jump the pattern.
//!
//! Rendering has no history: skipped draws cannot change a later still.

use crate::audio::track_dynamics::TrackDynamics;
use crate::project::caption_effects::bass_from_trails;
use crate::scene::settings::index::aurora as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Concurrent flaming waves the shader draws. A fifth kick inside a wave's
/// lifetime replaces the oldest, which by then has nearly reached the top.
pub const PULSES: usize = 4;

/// Seconds a flaming wave takes to climb and fade. Long enough to read as
/// motion, short enough that a 120 BPM kick pattern never needs a fifth slot.
pub const PULSE_SECONDS: f32 = 1.6;

/// Seconds a shooting star takes to cross and burn out.
pub const METEOR_SECONDS: f32 = 1.1;
/// The least time between two shooting stars. One is a moment; a shower on
/// every chorus hit would be a screensaver.
const METEOR_COOLDOWN_SECONDS: f32 = 9.0;

/// The musical figures both Atmosphere scenes read, derived one way.
///
/// Neon Tunnel uses this too (`super::neon_tunnel`), so "the bass" and "a
/// kick" mean the same thing in both scenes of the family.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Features {
    /// Track-relative loudness, `0..=1`, or the absolute fallback.
    pub level: f32,
    /// Mean of the lowest quarter of the trailing spectrum.
    pub bass: f32,
    /// Mean positive excursion of the lowest quarter of the bands over their
    /// own trails: the bass *moved*, not merely the bass is present.
    pub kick: f32,
    pub flux: f32,
    pub mids: f32,
    pub treble: f32,
}

impl Features {
    #[must_use]
    pub fn of(audio: &SceneAudioFrame<'_>, dynamics: Option<TrackDynamics>) -> Self {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        let level = dynamics.map_or((audio.rms * 1.8).clamp(0.0, 1.0), |profile| {
            profile.level(audio.rms)
        });
        let count = audio.bands.len().min(audio.trails.len());
        let kick = if count == 0 {
            0.0
        } else {
            let low = (count / 4).max(1);
            audio.bands[..low]
                .iter()
                .zip(&audio.trails[..low])
                .map(|(band, trail)| (band - trail).max(0.0))
                .sum::<f32>()
                / low as f32
        };
        let (_, mids, treble) = audio.spectral_regions();
        Self {
            level: finite(level).clamp(0.0, 1.0),
            bass: finite(bass_from_trails(audio.trails)).clamp(0.0, 1.0),
            kick: finite(kick).max(0.0),
            flux: finite(audio.spectral_flux).max(0.0),
            mids: finite(mids).clamp(0.0, 1.0),
            treble: finite(treble).clamp(0.0, 1.0),
        }
    }
}

/// Bass-transient detector shared by the family: an adaptive threshold on
/// [`Features::kick`] with hysteresis, a refractory period, and a leaky
/// kick-rate estimate a scene can ride the tempo with.
///
/// Tuned with `examples/atmosphere_probe.rs` on three real mixes. The bar sits
/// at 1.7x a 0.6 s running mean of the signal (never below 0.032): on *Door,
/// No Room* that separates the half-time accents from the bass line moving
/// underneath them, with intervals clustering at one and two bars' halves.
/// The detector re-arms only after the signal falls back under 80 % of the
/// bar, so a sustained drop is one hit, not a drumroll.
#[derive(Clone, Debug, PartialEq)]
pub struct KickDetector {
    refractory: f32,
    mean: f32,
    armed: bool,
    rate: f32,
    count: u64,
    last: f32,
}

impl Default for KickDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl KickDetector {
    const MIN_BAR: f32 = 0.032;
    const MEAN_SECONDS: f32 = 0.6;
    const REFRACTORY_SECONDS: f32 = 0.2;
    /// The rate estimator halves every this many seconds without a kick.
    const RATE_HALF_LIFE: f32 = 2.0;

    #[must_use]
    pub fn new() -> Self {
        Self {
            refractory: 0.0,
            mean: 0.0,
            armed: true,
            rate: 0.0,
            count: 0,
            last: 0.0,
        }
    }

    /// Advances by `dt` with this frame's kick signal (already scaled by the
    /// scene's response). Returns the strength, `0..=1`, of a kick that fired.
    pub fn step(&mut self, signal: f32, dt: f32) -> Option<f32> {
        self.refractory = (self.refractory - dt).max(0.0);
        // Leaky integrator in kicks per second: each kick adds 1/tau and the
        // sum decays with time constant tau, so a steady rate r settles at r.
        let tau = Self::RATE_HALF_LIFE / std::f32::consts::LN_2;
        self.rate *= (-dt / tau).exp();
        let signal = if signal.is_finite() {
            signal.max(0.0)
        } else {
            0.0
        };
        let bar = (self.mean * 1.7 + 0.01).max(Self::MIN_BAR);
        if signal < bar * 0.8 {
            self.armed = true;
        }
        let mut fired = None;
        if self.armed && signal > bar && self.refractory <= 0.0 {
            self.armed = false;
            self.refractory = Self::REFRACTORY_SECONDS;
            self.count += 1;
            self.rate += 1.0 / tau;
            let strength = ((signal - bar) / (bar * 1.5) + 0.3).clamp(0.0, 1.0);
            self.last = strength;
            fired = Some(strength);
        }
        self.mean += (signal - self.mean) * (1.0 - (-dt / Self::MEAN_SECONDS).exp());
        fired
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Kicks per second, smoothed over a couple of bars.
    #[must_use]
    pub fn rate(&self) -> f32 {
        self.rate
    }

    #[must_use]
    pub fn last(&self) -> f32 {
        self.last
    }
}

/// One-pole follower with separate attack and release rates (per second).
pub fn follow(current: &mut f32, target: f32, attack: f32, release: f32, dt: f32) {
    let target = if target.is_finite() { target } else { 0.0 };
    let rate = if target > *current { attack } else { release };
    *current += (target - *current) * (1.0 - (-rate * dt).exp());
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuroraVeilState {
    clock: f64,
    shimmer: f64,
    phase: f32,
    /// Bass weight, phrase activity, mids, treble.
    music: [f32; 4],
    /// The fast half of loudness: brightness that follows a phrase's swell.
    swell: f32,
    /// Kick lift: a whole-sky brightening that decays over half a second.
    surge: f32,
    /// `[age seconds, strength]` per flaming wave.
    pulses: [[f32; 2]; PULSES],
    kick: KickDetector,
    /// `[age seconds, seed]` of the current shooting star.
    meteor: [f32; 2],
    meteor_cooldown: f32,
    meteors: u64,
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
            shimmer: 0.0,
            phase: (mixed >> 40) as f32 / 16777216.0 * std::f32::consts::TAU,
            music: [0.0; 4],
            swell: 0.0,
            surge: 0.0,
            pulses: [[PULSE_SECONDS * 4.0, 0.0]; PULSES],
            kick: KickDetector::new(),
            meteor: [METEOR_SECONDS * 4.0, 0.0],
            meteor_cooldown: 0.0,
            meteors: 0,
        }
    }

    /// Flow-scaled drift time. Music never touches it, so the curtains'
    /// shape at a given moment depends only on the settings and the clock.
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }

    /// Ray shimmer phase, wrapped to keep single precision on long tracks.
    #[must_use]
    pub fn shimmer(&self) -> f32 {
        (self.shimmer % 4096.0) as f32
    }

    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Bass weight, phrase activity (track-relative loudness), mids, treble.
    #[must_use]
    pub fn music(&self) -> [f32; 4] {
        self.music
    }

    /// Fast loudness swell and kick surge.
    #[must_use]
    pub fn lift(&self) -> [f32; 2] {
        [self.swell, self.surge]
    }

    /// Flaming waves as `(age seconds, strength)` pairs.
    #[must_use]
    pub fn pulses(&self) -> [[f32; 2]; PULSES] {
        self.pulses
    }

    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kick.count()
    }

    /// The current shooting star as `(age seconds, seed in 0..1)`; an age
    /// past [`METEOR_SECONDS`] means none is in the sky.
    #[must_use]
    pub fn meteor(&self) -> [f32; 2] {
        self.meteor
    }

    /// Shooting stars launched so far.
    #[must_use]
    pub fn meteors(&self) -> u64 {
        self.meteors
    }

    /// One CSV fragment for `examples/atmosphere_probe.rs`.
    #[must_use]
    pub fn probe_line(&self) -> String {
        let live = self
            .pulses
            .iter()
            .filter(|pulse| pulse[0] < PULSE_SECONDS)
            .count();
        format!(
            "a:{:.3}/{:.3}/{:.3}/{:.3} swell={:.3} surge={:.3} kicks={} live={} meteors={}",
            self.music[0],
            self.music[1],
            self.music[2],
            self.music[3],
            self.swell,
            self.surge,
            self.kick.count(),
            live,
            self.meteors
        )
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
        let response = frame.setting(self.id(), control::RESPONSE);
        let features = Features::of(&frame.audio, frame.dynamics);

        // Bass sits around 0.3-0.6 on real mixes, so the raw mean would pin
        // the weight near the top all song long. A soft knee above a floor
        // keeps a bass drop visibly heavier than a verse.
        let bass = ((features.bass - 0.18) / 0.5).clamp(0.0, 1.0);
        let targets = [
            bass * response,
            features.level * response,
            (features.mids * 1.6).clamp(0.0, 1.0) * response,
            (features.treble * 2.2).clamp(0.0, 1.0) * response,
        ];
        // Activity is deliberately a phrase-length follower (rise over ~1 s,
        // settle over ~3 s): the sky changes shape with the song's sections.
        let rates = [(5.0, 1.4), (1.1, 0.35), (2.5, 0.9), (6.0, 1.8)];
        for ((current, target), (attack, release)) in self.music.iter_mut().zip(targets).zip(rates)
        {
            follow(current, target, attack, release, dt);
        }
        follow(&mut self.swell, features.level * response, 7.0, 1.6, dt);

        // Shimmer runs even in silence, and faster with the hats.
        self.shimmer += f64::from(dt) * f64::from(0.35 + 1.8 * self.music[3]);

        for pulse in &mut self.pulses {
            pulse[0] = (pulse[0] + dt).min(PULSE_SECONDS * 4.0);
        }
        self.meteor[0] = (self.meteor[0] + dt).min(METEOR_SECONDS * 4.0);
        self.meteor_cooldown = (self.meteor_cooldown - dt).max(0.0);
        self.surge *= (-dt * 2.6).exp();
        if let Some(strength) = self.kick.step(features.kick * response, dt) {
            let oldest = self
                .pulses
                .iter_mut()
                .max_by(|a, b| a[0].total_cmp(&b[0]))
                .expect("PULSES > 0");
            *oldest = [0.0, strength];
            self.surge = self.surge.max(strength);
            // A strong accent in a sparse sky earns a shooting star: the
            // quiet passages are where a single streak reads as an event.
            if strength > 0.6 && self.music[1] < 0.55 && self.meteor_cooldown <= 0.0 {
                self.meteors += 1;
                let mut mixed =
                    self.meteors.wrapping_mul(0x9e3779b97f4a7c15) ^ u64::from(self.phase.to_bits());
                mixed = (mixed ^ (mixed >> 29)).wrapping_mul(0xbf58476d1ce4e5b9);
                mixed ^= mixed >> 32;
                self.meteor = [0.0, (mixed >> 40) as f32 / 16777216.0];
                self.meteor_cooldown = METEOR_COOLDOWN_SECONDS;
            }
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
        state_version: 2,
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

    /// A kick every half second: bands jump above trails for two frames.
    fn kicks(state: &mut AuroraVeilState, settings: &SceneSettings, seconds: f32) {
        let calm = [0.3; 24];
        let mut hit = [0.3; 24];
        hit[..6].fill(0.62);
        let frames = (seconds * 60.0) as usize;
        for tick in 0..frames {
            let mut frame = SceneFrame::idle(settings);
            frame.delta_seconds = 1.0 / 60.0;
            frame.audio.rms = 0.4;
            frame.audio.bands = if tick % 30 < 2 { &hit } else { &calm };
            frame.audio.trails = &calm;
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
        assert!(a.shimmer() > quiet.shimmer(), "treble quickens the rays");
        assert_ne!(initial.phase(), AuroraVeilState::new(43).phase());
        assert_eq!(a.clock(), quiet.clock(), "music cannot alter replay time");
    }

    #[test]
    fn kicks_launch_flaming_waves_and_silence_launches_none() {
        let settings = SceneSettings::new();
        let mut hits = AuroraVeilState::new(3);
        let mut quiet = hits.clone();
        kicks(&mut hits, &settings, 4.0);
        replay(&mut quiet, &settings, false);
        assert!(
            (7..=9).contains(&hits.kicks()),
            "one wave per hit, not per frame: {}",
            hits.kicks()
        );
        let live = hits
            .pulses()
            .iter()
            .filter(|pulse| pulse[0] < PULSE_SECONDS && pulse[1] > 0.3)
            .count();
        assert!(live >= 2, "a groove keeps several waves climbing at once");
        assert_eq!(quiet.kicks(), 0);
        assert!(hits.lift()[1] > quiet.lift()[1]);
    }

    #[test]
    fn a_strong_accent_in_a_sparse_sky_earns_one_shooting_star() {
        let settings = SceneSettings::new();
        let mut state = AuroraVeilState::new(11);
        // Quiet bed with a hard hit every half second: one star, then the
        // cooldown holds the rest back.
        let calm = [0.1; 24];
        let mut hit = [0.1; 24];
        hit[..6].fill(0.7);
        for tick in 0..(6 * 60) {
            let mut frame = SceneFrame::idle(&settings);
            frame.delta_seconds = 1.0 / 60.0;
            frame.audio.rms = 0.05;
            frame.audio.bands = if tick % 30 < 2 { &hit } else { &calm };
            frame.audio.trails = &calm;
            state.update(&frame);
        }
        assert!(state.kicks() >= 10);
        assert_eq!(state.meteors(), 1);
        assert!((0.0..1.0).contains(&state.meteor()[1]));
        let mut silent = AuroraVeilState::new(11);
        replay(&mut silent, &settings, false);
        assert_eq!(silent.meteors(), 0);
    }

    #[test]
    fn pause_freezes_every_uniform_even_when_audio_changes() {
        let settings = SceneSettings::new();
        let mut state = AuroraVeilState::new(7);
        replay(&mut state, &settings, true);
        kicks(&mut state, &settings, 1.0);
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
        kicks(&mut music, &settings, 2.0);
        replay(&mut quiet, &settings, false);
        // The same two seconds on the quiet side, in silence.
        for _ in 0..120 {
            let mut frame = SceneFrame::idle(&settings);
            frame.delta_seconds = 1.0 / 60.0;
            quiet.update(&frame);
        }
        assert_eq!(music, quiet);
        assert!(music.clock() > 1.0, "ambient curtains still flow");
        assert_eq!(music.kicks(), 0);
    }

    #[test]
    fn the_same_relative_phrase_has_the_same_light_on_quiet_and_loud_tracks() {
        let settings = SceneSettings::new();
        let mut quiet = AuroraVeilState::new(1);
        let mut loud = quiet.clone();
        let mut absolute = quiet.clone();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 0.25;
        for _ in 0..24 {
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

    #[test]
    fn a_sustained_excursion_is_one_hit_not_a_drumroll() {
        let mut detector = KickDetector::new();
        let mut fired = 0;
        for _ in 0..600 {
            if detector.step(0.3, 1.0 / 60.0).is_some() {
                fired += 1;
            }
        }
        assert_eq!(fired, 1, "a held drop re-armed itself");
        // Hits riding on a busy bass line still separate from it.
        let mut detector = KickDetector::new();
        let mut fired = 0;
        for tick in 0..600 {
            let signal = if tick % 30 < 3 { 0.12 } else { 0.03 };
            if detector.step(signal, 1.0 / 60.0).is_some() {
                fired += 1;
            }
        }
        assert_eq!(fired, 20, "two hits a second for ten seconds");
    }
}
