//! Liquid Chrome: a mercury sea under a studio sky, with a chrome ring standing
//! in it. This half is the deterministic part: the material clock, the musical
//! followers and the bounded history of waves the music has launched.
//!
//! The surface itself is evaluated analytically on the GPU
//! (`resources/shaders/glsl330/liquid_chrome.fs`); there is no simulation
//! buffer, so drawless still replay loses nothing. Original implementation,
//! inspired by reflective liquid-metal materials, with no third-party source.
//!
//! What the music does, and through which lane (docs/MATERIAL_SCENES.md):
//!
//! - **bass** (the shared `EffectInputs` definition) raises the heavy swell, so
//!   a bassline is felt as weight rolling through the metal;
//! - **kicks** (a bass-transient detector, the shape Clawd measured on real
//!   mixes) launch a wave front outward from the ring's footprint and flash
//!   the studio lights;
//! - **other transients** (onset edges and flux jumps) drop small ripples on
//!   the sea, like rain on mercury;
//! - **track-relative loudness** brightens the gels and adds chop, so a quiet
//!   passage settles into a slow, glassy mirror.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::chrome as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Kick fronts kept alive at once. A front has faded to nothing well before
/// four more kicks can arrive (lifetime 2.6 s against a 0.25 s refractory
/// would allow ten, but the replacement rule takes the weakest, not the oldest).
pub const IMPACT_COUNT: usize = 4;
/// Transient droplets kept alive at once.
pub const DROP_COUNT: usize = 6;

/// How long a kick front is drawn, in seconds. The shader's envelope reaches
/// zero here; the core uses it to know which slot is free.
pub const IMPACT_LIFE: f32 = 2.6;
/// How long a droplet's rings are drawn, in seconds.
pub const DROP_LIFE: f32 = 1.6;

/// The bass-transient gate: mean positive excursion of the lowest quarter of
/// the bands over their own trails. 0.05 is the value Clawd measured on real
/// mixes (`scenes::clawd::KICK_THRESHOLD`): it finds a real bass groove and
/// stays quiet through pads.
const KICK_THRESHOLD: f32 = 0.05;
/// Minimum seconds between kicks; a sustained excursion is one push.
const KICK_REFRACTORY: f32 = 0.25;
/// Minimum seconds between droplets, so a busy hat line is rain, not foam.
const DROP_COOLDOWN: f32 = 0.11;
/// Seconds for the kick light flash to fall to 1/e.
const PULSE_DECAY: f32 = 0.22;

/// Where the ring stands on the sea plane, `(x, z)`. The shader's
/// `RING_CENTRE` carries the same numbers; the camera circles this point.
pub const RING_CENTRE_XZ: [f32; 2] = [0.0, 7.4];
/// Radians of orbit per second of material clock: at the default Flow a full
/// circle takes about seven minutes, so one song sees the ring from most
/// sides without the camera ever visibly travelling.
const ORBIT_RATE: f64 = 0.022;

/// One kick's wave front: seconds since launch and launch strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    pub age: f32,
    pub strength: f32,
}

/// One droplet: sea-plane position (x across, z into the frame), seconds
/// since it landed, and its strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Droplet {
    pub origin: [f32; 2],
    pub age: f32,
    pub strength: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidChromeState {
    seed: u64,
    phase: [f32; 4],
    clock: f64,
    /// Bass pressure, track-relative loudness, transient strength.
    drive: [f32; 3],
    pulse: f32,
    impacts: [Impact; IMPACT_COUNT],
    drops: [Droplet; DROP_COUNT],
    kicks: u64,
    splashes: u64,
    kick_refractory: f32,
    drop_cooldown: f32,
    previous_onset: bool,
    previous_hit: f32,
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

/// The bass transient: mean positive excursion of the lowest quarter of the
/// bands over their own trails. The same shape as spectral flux, scoped to the
/// bands `bass_from_trails` calls bass, so "the bass level" and "the bass
/// moved" are measured over the same region.
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

impl LiquidChromeState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            phase: std::array::from_fn(|i| {
                unit(seed.wrapping_add(i as u64 * 0x2545_f491_4f6c_dd1d)) * std::f32::consts::TAU
            }),
            clock: 0.0,
            drive: [0.0; 3],
            pulse: 0.0,
            impacts: [Impact {
                age: IMPACT_LIFE,
                strength: 0.0,
            }; IMPACT_COUNT],
            drops: [Droplet {
                origin: [0.0; 2],
                age: DROP_LIFE,
                strength: 0.0,
            }; DROP_COUNT],
            kicks: 0,
            splashes: 0,
            kick_refractory: 0.0,
            drop_cooldown: 0.0,
            previous_onset: false,
            previous_hit: 0.0,
        }
    }
    /// Seeded phases for the swell trains and the camera drift.
    #[must_use]
    pub fn phase(&self) -> [f32; 4] {
        self.phase
    }
    /// The material clock: seconds of track time scaled by Flow.
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    /// Smoothed bass pressure, track-relative loudness and transient strength.
    #[must_use]
    pub fn drive(&self) -> [f32; 3] {
        self.drive
    }
    /// The kick flash: 1 on a full-strength kick, decaying over ~0.2 s.
    #[must_use]
    pub fn pulse(&self) -> f32 {
        self.pulse
    }
    #[must_use]
    pub fn impacts(&self) -> &[Impact; IMPACT_COUNT] {
        &self.impacts
    }
    #[must_use]
    pub fn droplets(&self) -> &[Droplet; DROP_COUNT] {
        &self.drops
    }
    /// The camera: radians round the ring and distance from it. Both follow
    /// the material clock, so Flow zero holds the shot.
    #[must_use]
    pub fn camera_orbit(&self) -> [f32; 2] {
        let angle = f64::from(self.phase[1]) + self.clock * ORBIT_RATE;
        let distance = 6.7 + 0.8 * (self.clock * 0.047 + f64::from(self.phase[2])).sin();
        [
            angle.rem_euclid(std::f64::consts::TAU) as f32,
            distance as f32,
        ]
    }
    /// A point on the sea given relative to the camera — `across` to its
    /// right as the shader draws it, `depth` along its heading — in world
    /// `(x, z)`. Mirrors the shader's camera basis.
    fn sea_point(&self, across: f32, depth: f32) -> [f32; 2] {
        let [angle, distance] = self.camera_orbit();
        let (s, c) = angle.sin_cos();
        let camera = [
            RING_CENTRE_XZ[0] - s * distance,
            RING_CENTRE_XZ[1] - c * distance,
        ];
        // cross(forward, up) with forward (s, 0, c) is (-c, 0, s).
        [
            camera[0] + s * depth - c * across,
            camera[1] + c * depth + s * across,
        ]
    }
    /// Kicks detected so far; reported, so a track that moves the sea and
    /// never launches a front says so.
    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kicks
    }
    /// Droplets launched so far.
    #[must_use]
    pub fn splashes(&self) -> u64 {
        self.splashes
    }

    /// The slot with the least wave left in it: a fresh kick never overwrites
    /// a stronger front that is still crossing the frame.
    fn weakest_impact(&self) -> usize {
        let remaining = |i: &Impact| i.strength * (1.0 - i.age / IMPACT_LIFE).max(0.0);
        (0..IMPACT_COUNT)
            .min_by(|a, b| remaining(&self.impacts[*a]).total_cmp(&remaining(&self.impacts[*b])))
            .unwrap_or(0)
    }

    fn weakest_drop(&self) -> usize {
        let remaining = |d: &Droplet| d.strength * (1.0 - d.age / DROP_LIFE).max(0.0);
        (0..DROP_COUNT)
            .min_by(|a, b| remaining(&self.drops[*a]).total_cmp(&remaining(&self.drops[*b])))
            .unwrap_or(0)
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
        let target = [
            finite01(audio.bass).sqrt(),
            finite01(
                frame
                    .dynamics
                    .map_or(audio.rms * 1.8, |p| p.level(audio.rms)),
            ),
            finite01(audio.flux * 7.0),
        ];
        // Bass and loudness rise quickly and fall slowly, so a swell carries
        // past the note that raised it — weight, not tremble.
        let release = [1.4, 0.9, 3.0];
        for (i, target) in target.into_iter().enumerate() {
            let target = target * response;
            let rate = if target > self.drive[i] {
                7.0
            } else {
                release[i]
            };
            self.drive[i] += (target - self.drive[i]) * (1.0 - (-rate * dt).exp());
        }

        // Launched waves always finish travelling, even at Flow zero.
        for impact in &mut self.impacts {
            impact.age = (impact.age + dt).min(IMPACT_LIFE);
        }
        for drop in &mut self.drops {
            drop.age = (drop.age + dt).min(DROP_LIFE);
        }
        self.pulse *= (-dt / PULSE_DECAY).exp();
        self.kick_refractory = (self.kick_refractory - dt).max(0.0);
        self.drop_cooldown = (self.drop_cooldown - dt).max(0.0);

        let kick = bass_excursion(&frame.audio);
        if response > 0.0 && kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            let strength = (response * (kick / (2.4 * KICK_THRESHOLD)).clamp(0.4, 1.0)).min(1.5);
            let slot = self.weakest_impact();
            self.impacts[slot] = Impact { age: 0.0, strength };
            self.pulse = self.pulse.max(strength.min(1.0));
            self.kicks = self.kicks.wrapping_add(1);
            self.kick_refractory = KICK_REFRACTORY;
        }

        let hit = finite01(audio.flux * 7.0);
        let edge = (frame.audio.onset && !self.previous_onset) || hit - self.previous_hit > 0.16;
        if edge && response > 0.0 && self.drop_cooldown <= 0.0 {
            let key = self
                .seed
                .wrapping_mul(0xd6e8_feb8_6659_fd93)
                .wrapping_add(self.splashes.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            // Somewhere on the visible sea: across the frame, between the
            // foreground and just past the ring. Stored in world space, so a
            // ring of ripples stays put while the camera moves on.
            let depth = 2.4 + unit(key ^ 0xa076_1d64_78bd_642f) * 7.5;
            let across = (unit(key) - 0.5) * (1.6 + 0.55 * depth);
            let origin = self.sea_point(across, depth);
            let slot = self.weakest_drop();
            self.drops[slot] = Droplet {
                origin,
                age: 0.0,
                strength: (response * (0.35 + 0.65 * hit)).min(1.5),
            };
            self.splashes = self.splashes.wrapping_add(1);
            self.drop_cooldown = DROP_COOLDOWN;
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
        id: SceneId::LiquidChrome,
        // 2: the sea-and-ring rework (kick fronts and droplets in the state).
        state_version: 2,
        make_state: |seed| Box::new(LiquidChromeState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    /// Two seconds of a groove: a kick every half second (bands jump above
    /// their trails in the bass), and an onset between kicks.
    fn run(state: &mut LiquidChromeState, settings: &SceneSettings, sound: bool) {
        let trails = [0.6; 16];
        let mut kick_bands = [0.6; 16];
        for band in kick_bands.iter_mut().take(4) {
            *band = 0.78;
        }
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / 60.0;
        for i in 0..120 {
            if sound {
                frame.audio.trails = &trails;
                frame.audio.bands = if i % 30 == 0 { &kick_bands } else { &trails };
                frame.audio.rms = 0.6;
                frame.audio.onset = i % 30 == 15;
                frame.audio.spectral_flux = if frame.audio.onset { 0.13 } else { 0.0 };
            }
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
        assert!(a.drive[0] > 0.5 && a.drive[1] > 0.5, "{:?}", a.drive);
        assert_eq!(a.kicks, 4, "one front per kick");
        assert_eq!(a.splashes, 4, "one droplet per onset");
        assert!(a.impacts.iter().any(|i| i.strength > 0.0 && i.age < 0.6));
        assert_eq!(quiet.drive, [0.0; 3]);
        assert_eq!((quiet.kicks, quiet.splashes), (0, 0));
    }

    #[test]
    fn droplets_land_on_the_visible_sea_and_differ_by_seed() {
        let settings = SceneSettings::new();
        let mut a = LiquidChromeState::new(5);
        let mut b = LiquidChromeState::new(6);
        run(&mut a, &settings, true);
        run(&mut b, &settings, true);
        assert_ne!(a.drops, b.drops);
        // In front of the camera, within the frame's spread at that depth.
        let [angle, distance] = a.camera_orbit();
        let camera = [
            RING_CENTRE_XZ[0] - angle.sin() * distance,
            RING_CENTRE_XZ[1] - angle.cos() * distance,
        ];
        for drop in a.drops.iter().filter(|d| d.strength > 0.0) {
            let to = [drop.origin[0] - camera[0], drop.origin[1] - camera[1]];
            let depth = to[0] * angle.sin() + to[1] * angle.cos();
            let across = -to[0] * angle.cos() + to[1] * angle.sin();
            assert!((2.3..=10.0).contains(&depth), "depth {depth}");
            assert!(across.abs() <= 0.8 + 0.28 * depth + 0.2, "across {across}");
        }
    }

    #[test]
    fn the_camera_circles_the_ring_with_the_material_clock() {
        let mut state = LiquidChromeState::new(3);
        let start = state.camera_orbit();
        state.clock = 100.0;
        let later = state.camera_orbit();
        assert!((later[0] - start[0]).abs() > 1.0);
        assert!((5.8..=7.6).contains(&later[1]));
    }

    #[test]
    fn a_sustained_excursion_is_one_kick_per_refractory() {
        let settings = SceneSettings::new();
        let mut state = LiquidChromeState::new(1);
        let trails = [0.2; 16];
        let bands = [0.5; 16];
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        frame.audio.trails = &trails;
        frame.audio.bands = &bands;
        for _ in 0..60 {
            state.update(&frame);
        }
        // One second of continuous excursion at a 0.25 s refractory.
        assert_eq!(state.kicks, 4);
        assert!(state.pulse > 0.0 && state.pulse <= 1.0);
    }

    #[test]
    fn a_new_kick_replaces_the_weakest_front() {
        let mut state = LiquidChromeState::new(9);
        state.impacts = [
            Impact {
                age: 0.1,
                strength: 1.0,
            },
            Impact {
                age: 2.0,
                strength: 1.0,
            },
            Impact {
                age: 0.2,
                strength: 0.9,
            },
            Impact {
                age: 0.3,
                strength: 0.8,
            },
        ];
        assert_eq!(state.weakest_impact(), 1);
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
        frame.audio.onset = true;
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
    fn launched_waves_finish_at_flow_zero() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SceneId::LiquidChrome, control::FLOW, 0.0));
        let mut state = LiquidChromeState::new(4);
        run(&mut state, &settings, true);
        assert_eq!(state.clock, 0.0);
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 0.1;
        for _ in 0..40 {
            state.update(&frame);
        }
        assert!(state.impacts.iter().all(|i| i.age >= IMPACT_LIFE));
        assert!(state.drops.iter().all(|d| d.age >= DROP_LIFE));
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
