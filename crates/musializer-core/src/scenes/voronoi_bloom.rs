//! Voronoi Bloom: a leaded stained-glass window with the sun behind it and
//! haze in front, so the light that comes through the glass streams toward
//! the viewer as coloured shafts.
//!
//! This half holds everything that has history — the clock, the followed
//! spectrum, the sun's drift, the bloom waves and the shatters — and the
//! drawing half (`musializer-app::scenes::voronoi_bloom` and
//! `voronoi_bloom.fs`) is a pure function of what it is handed, which is what
//! keeps a still, a clip and a full export the same picture.
//!
//! How it answers the music:
//!
//! - **Twelve followed spectral voices** light the glass as a rose: the window's
//!   centre carries the bass and its rim the treble, interpolated continuously
//!   across each cell's distance from the centre, so a bass note glows in the
//!   middle and hats glitter at the edges.
//! - **Kicks** start a *bloom wave* from a beat seed — alternately the sun and a
//!   seeded point in the window. The wave front lights every cell it passes, so
//!   the glass blooms outward cell by cell like a crystal growing.
//! - **The hardest kicks of a loud section** crack one pane: a star of fractures
//!   from the impact point flares white-gold with light leaking through, then
//!   heals over a couple of seconds. Rare on purpose (a cooldown and an energy
//!   gate), so it lands as an accent rather than a strobe.
//! - **Bass** swells the sun and thickens the haze, so the shafts grow.
//! - **The slow section energy** raises the sun and brightens it on loud
//!   sections and lets it sink and dim in quiet ones, where the glass deepens and
//!   the dust in the beams is most of what moves — it breathes.
use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::voronoi as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Bloom waves alive at once; a new one overwrites the oldest.
pub const MAX_WAVES: usize = 4;
/// Seconds a wave lives. At [`WAVE_SPEED`] it has crossed a 16:9 frame.
pub const WAVE_SECONDS: f32 = 1.8;
/// Wave front speed, in short-axis units per second.
pub const WAVE_SPEED: f32 = 0.85;
/// Seconds a crack takes to heal.
pub const SHATTER_SECONDS: f32 = 2.4;
/// The kick gate (Clawd's measured value; see [`crate::scenes::clawd::KICK_THRESHOLD`]).
pub const KICK_THRESHOLD: f32 = crate::scenes::clawd::KICK_THRESHOLD;
const KICK_REFRACTORY_SECONDS: f32 = 0.22;
/// A kick at least this strong, in a section at least [`SHATTER_ENERGY`] loud,
/// cracks a pane. Read off `examples/organic_probe.rs` on a real mix: the
/// excursion of an ordinary groove kick sits around 0.06-0.09 and a section's
/// hardest hits reach 0.13 and beyond; with the energy gate and the cooldown
/// that is a crack every twenty-odd seconds of a loud section.
pub const SHATTER_KICK: f32 = 0.13;
/// Track-relative section energy a shatter needs: a breakdown's isolated hit
/// should bloom, not break the window.
pub const SHATTER_ENERGY: f32 = 0.5;
/// The quietest a crack may follow another.
const SHATTER_COOLDOWN_SECONDS: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct VoronoiBloomState {
    seed: u64,
    clock: f64,
    voices: [f32; 12],
    bass: f32,
    light: f32,
    high: f32,
    /// Slow track-relative energy: the section, not the beat.
    energy: f32,
    /// Kick flash: the sun flares and the shafts pulse, decaying in about a
    /// quarter second.
    accent: f32,
    kick_refractory: f32,
    kick_count: u64,
    wave_count: u64,
    /// Origin x, origin y, age, power.
    waves: [[f32; 4]; MAX_WAVES],
    shatter_cooldown: f32,
    shatter_count: u64,
    /// Impact x, impact y, age, power.
    shatter: [f32; 4],
}
impl VoronoiBloomState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            clock: 0.0,
            voices: [0.0; 12],
            bass: 0.0,
            light: 0.0,
            high: 0.0,
            energy: 0.0,
            accent: 0.0,
            kick_refractory: 0.0,
            kick_count: 0,
            wave_count: 0,
            waves: [[0.0, 0.0, WAVE_SECONDS, 0.0]; MAX_WAVES],
            shatter_cooldown: 0.0,
            shatter_count: 0,
            shatter: [0.0, 0.0, SHATTER_SECONDS, 0.0],
        }
    }
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    /// All seed bits participate before narrowing to an exactly represented GPU integer.
    #[must_use]
    pub fn seed(&self) -> f32 {
        let n = self.seed ^ self.seed.rotate_left(23) ^ self.seed.rotate_right(17);
        ((n ^ n.wrapping_mul(0x9e3779b97f4a7c15).rotate_left(31)) & 0x00ff_ffff) as f32
    }
    #[must_use]
    pub fn voices(&self) -> &[f32; 12] {
        &self.voices
    }
    #[must_use]
    pub fn bass(&self) -> f32 {
        self.bass
    }
    #[must_use]
    pub fn light(&self) -> f32 {
        self.light
    }
    #[must_use]
    pub fn high(&self) -> f32 {
        self.high
    }
    #[must_use]
    pub fn energy(&self) -> f32 {
        self.energy
    }
    #[must_use]
    pub fn accent(&self) -> f32 {
        self.accent
    }
    #[must_use]
    pub fn kick_count(&self) -> u64 {
        self.kick_count
    }
    #[must_use]
    pub fn wave_count(&self) -> u64 {
        self.wave_count
    }
    #[must_use]
    pub fn shatter_count(&self) -> u64 {
        self.shatter_count
    }

    /// The sun behind the glass, in short-axis units from the frame centre (y
    /// up), and its strength. It wanders on the clock, rides higher and brighter
    /// in a loud section and sinks and dims in a quiet one; bass swells it.
    #[must_use]
    pub fn sun(&self) -> [f32; 3] {
        let t = self.clock as f32;
        let s = phase(self.seed ^ 0x51ed_270b);
        let x = 0.34 * (t * 0.061 + s).sin() + 0.08 * (t * 0.023).sin();
        let y = 0.1 + 0.12 * (t * 0.047 + s * 0.7).sin() + 0.12 * self.energy;
        let strength = 0.62
            + 0.4 * self.light
            + 0.32 * self.bass
            + 0.45 * self.accent
            + 0.35 * self.shatter_flash();
        [x, y, strength]
    }

    /// Live bloom waves as `[origin x, origin y, front radius, strength]`; dead
    /// slots have strength 0.
    #[must_use]
    pub fn waves(&self) -> [[f32; 4]; MAX_WAVES] {
        self.waves.map(|[x, y, age, power]| {
            if age >= WAVE_SECONDS {
                [x, y, 0.0, 0.0]
            } else {
                // A quick swell, then a long fade as the front travels outward.
                let envelope = (age / 0.08).min(1.0) * (1.0 - age / WAVE_SECONDS).powi(2);
                [x, y, age * WAVE_SPEED, power * envelope]
            }
        })
    }
    #[must_use]
    pub fn live_waves(&self) -> usize {
        self.waves
            .iter()
            .filter(|w| w[2] < WAVE_SECONDS && w[3] > 0.0)
            .count()
    }

    /// The live crack as `[impact x, impact y, age, strength]`; strength 0 when
    /// the glass is whole.
    #[must_use]
    pub fn shatter(&self) -> [f32; 4] {
        let [x, y, age, power] = self.shatter;
        if age >= SHATTER_SECONDS {
            [x, y, age, 0.0]
        } else {
            [x, y, age, power]
        }
    }

    fn shatter_flash(&self) -> f32 {
        let [_, _, age, power] = self.shatter();
        power * (-age * 7.0).exp()
    }

    fn seeded_point(&self, salt: u64) -> [f32; 2] {
        let n = mix64(self.seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let unit = |shift: u32| ((n >> shift) & 0xffff) as f32 / 65_535.0 * 2.0 - 1.0;
        [unit(8) * 0.62, unit(32) * 0.36]
    }
}
fn clean(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
fn follow(current: &mut f32, target: f32, dt: f32, up: f32, down: f32) {
    let rate = if target > *current { up } else { down };
    *current += (target - *current) * (1.0 - (-rate * dt).exp());
}
fn phase(seed: u64) -> f32 {
    (mix64(seed) >> 40) as f32 * (std::f32::consts::TAU / 16_777_216.0)
}
fn mix64(seed: u64) -> u64 {
    let mut n = seed.wrapping_add(0x9e3779b97f4a7c15);
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
/// Mean positive excursion of the lowest quarter of the bands over their
/// trails — the kick measure Clawd's detector uses.
fn kick_excursion(audio: &SceneAudioFrame<'_>) -> f32 {
    let count = audio.bands.len().min(audio.trails.len());
    if count == 0 {
        return 0.0;
    }
    let low = (count / 4).max(1);
    audio.bands[..low]
        .iter()
        .zip(&audio.trails[..low])
        .map(|(band, trail)| {
            let d = band - trail;
            if d.is_finite() {
                d.max(0.0)
            } else {
                0.0
            }
        })
        .sum::<f32>()
        / low as f32
}
impl SceneState for VoronoiBloomState {
    fn id(&self) -> SceneId {
        SceneId::VoronoiBloom
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        let response = frame.setting(self.id(), control::RESPONSE);
        let inputs = EffectInputs::from_audio(
            0.0,
            frame.audio.rms,
            frame.audio.trails,
            0.0,
            frame.audio.spectral_flux,
        );
        let loud = frame
            .dynamics
            .map_or(clean(inputs.rms * 1.8), |p| p.level(clean(inputs.rms)));
        follow(&mut self.light, clean(loud) * response, dt, 15.0, 2.8);
        follow(
            &mut self.bass,
            clean(inputs.bass).sqrt() * response,
            dt,
            15.0,
            2.8,
        );
        follow(
            &mut self.energy,
            (clean(loud) * response).min(1.0),
            dt,
            0.9,
            0.35,
        );
        for (i, voice) in self.voices.iter_mut().enumerate() {
            let start = i * frame.audio.trails.len() / 12;
            let end = (i + 1) * frame.audio.trails.len() / 12;
            let group = &frame.audio.trails[start..end];
            let power = group.iter().map(|v| clean(*v)).sum::<f32>() / group.len().max(1) as f32;
            follow(voice, power.sqrt() * response, dt, 15.0, 2.8);
        }
        let top = &self.voices[9..];
        follow(
            &mut self.high,
            top.iter().sum::<f32>() / top.len() as f32,
            dt,
            15.0,
            2.8,
        );

        // A quiet passage drifts at 75 % of `flow`, a loud one at 125 %.
        // Response zero pins it at 100 %.
        let pace = 0.75 + 0.5 * self.energy + 0.25 * (1.0 - response.min(1.0));
        self.clock += f64::from(dt * frame.setting(self.id(), control::FLOW) * pace);

        for wave in &mut self.waves {
            wave[2] = (wave[2] + dt).min(WAVE_SECONDS);
        }
        self.shatter[2] = (self.shatter[2] + dt).min(SHATTER_SECONDS);
        self.shatter_cooldown = (self.shatter_cooldown - dt).max(0.0);
        self.kick_refractory = (self.kick_refractory - dt).max(0.0);
        self.accent *= (-5.0 * dt).exp();

        let kick = kick_excursion(&frame.audio);
        if response > 0.0 && kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            self.kick_refractory = KICK_REFRACTORY_SECONDS;
            self.kick_count = self.kick_count.wrapping_add(1);
            let power = (kick / (2.0 * KICK_THRESHOLD)).clamp(0.45, 1.0) * response.min(1.5);
            // Beat seeds alternate: the sun, then a seeded pane.
            let origin = if self.wave_count.is_multiple_of(2) {
                let [x, y, _] = self.sun();
                [x, y]
            } else {
                self.seeded_point(self.wave_count)
            };
            let slot = (self.wave_count % MAX_WAVES as u64) as usize;
            self.waves[slot] = [origin[0], origin[1], 0.0, power];
            self.accent = self.accent.max(power.min(1.0));
            self.wave_count = self.wave_count.wrapping_add(1);

            if kick >= SHATTER_KICK && self.energy >= SHATTER_ENERGY && self.shatter_cooldown <= 0.0
            {
                // Struck near the light, where a crack has bright glass to
                // show against: between the sun and the window's heart.
                let [ox, oy] = self.seeded_point(!self.shatter_count);
                let [sx, sy, _] = self.sun();
                let (x, y) = (sx * 0.4 + ox * 0.6, sy * 0.4 + oy * 0.7);
                self.shatter = [x, y, 0.0, power.min(1.0)];
                self.shatter_count = self.shatter_count.wrapping_add(1);
                self.shatter_cooldown = SHATTER_COOLDOWN_SECONDS;
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
        id: SceneId::VoronoiBloom,
        state_version: 2,
        make_state: |seed| Box::new(VoronoiBloomState::new(seed)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    #[test]
    fn cells_replay_and_separate_low_and_high_notes() {
        let settings = SceneSettings::new();
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 60.0;
        let mut low = [0.0; 120];
        low[..20].fill(0.8);
        let mut high = [0.0; 120];
        high[100..].fill(0.8);
        let mut a = VoronoiBloomState::new(17);
        let mut replay = a.clone();
        let mut b = a.clone();
        for _ in 0..90 {
            f.audio.trails = &low;
            a.update(&f);
            replay.update(&f);
            f.audio.trails = &high;
            b.update(&f);
        }
        assert_eq!(a, replay);
        assert!(a.clock() > 0.5);
        assert!(a.voices()[0] > 0.8 && a.voices()[11] < 0.01);
        assert!(b.voices()[11] > 0.8 && b.voices()[0] < 0.01);
        assert!(a.bass() > b.bass() + 0.5);
        assert!(b.high() > a.high() + 0.5);
        assert_ne!(a.seed(), VoronoiBloomState::new(18).seed());
        let paused = a.clone();
        for dt in [0.0, -1.0, f32::INFINITY] {
            f.delta_seconds = dt;
            a.update(&f);
            assert_eq!(a, paused);
        }
    }
    #[test]
    fn zero_response_keeps_an_ambient_clock_without_spectral_flashing() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::VoronoiBloom, control::RESPONSE, 0.0);
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 0.1;
        let mut a = VoronoiBloomState::new(1);
        let mut b = a.clone();
        for _ in 0..40 {
            a.update(&f);
        }
        f.audio.rms = 1.0;
        f.audio.trails = &[0.9; 104];
        let mut kick = [0.9; 104];
        kick[..26].fill(1.0);
        f.audio.bands = &kick;
        for _ in 0..40 {
            b.update(&f);
        }
        assert_eq!(a, b);
        assert!(a.clock() > 1.0);
        assert_eq!(a.voices(), &[0.0; 12]);
        assert_eq!(a.live_waves(), 0);
    }
    #[test]
    fn kicks_bloom_outward_and_only_a_loud_hard_hit_cracks_a_pane() {
        let settings = SceneSettings::new();
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 30.0;
        let trails = vec![0.2; 104];
        let mut soft = vec![0.2; 104];
        soft[..26].fill(0.27);
        let mut hard = vec![0.2; 104];
        hard[..26].fill(0.4);
        f.audio.trails = &trails;

        // A soft kick in a quiet track blooms but does not crack.
        let mut state = VoronoiBloomState::new(9);
        f.audio.bands = &soft;
        state.update(&f);
        assert_eq!((state.kick_count(), state.live_waves()), (1, 1));
        f.audio.bands = &trails;
        state.update(&f);
        let w = state.waves()[0];
        assert!(w[3] > 0.0 && w.iter().all(|v| v.is_finite()));
        f.audio.bands = &trails;
        let mut front = 0.0f32;
        for _ in 0..20 {
            state.update(&f);
            front = front.max(state.waves()[0][2]);
        }
        assert!(front > 0.5, "the bloom travels outward: {front}");
        f.audio.bands = &hard;
        state.update(&f);
        assert_eq!(
            state.shatter_count(),
            0,
            "quiet sections bloom, never break"
        );

        // The same hard kick in a loud section cracks one pane, once.
        let dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.1, 0.5);
        f.dynamics = dynamics;
        f.audio.rms = 0.8;
        f.audio.bands = &trails;
        let mut loud = VoronoiBloomState::new(9);
        for _ in 0..120 {
            loud.update(&f);
        }
        assert!(loud.energy() > SHATTER_ENERGY);
        f.audio.bands = &hard;
        loud.update(&f);
        assert_eq!(loud.shatter_count(), 1);
        assert!(loud.shatter()[3] > 0.0);
        let bright = loud.sun()[2];
        for _ in 0..10 {
            f.audio.bands = &trails;
            loud.update(&f);
            f.audio.bands = &hard;
            loud.update(&f);
        }
        assert_eq!(loud.shatter_count(), 1, "the cooldown keeps cracks rare");
        f.audio.bands = &trails;
        for _ in 0..120 {
            loud.update(&f);
        }
        assert_eq!(loud.shatter()[3], 0.0, "the crack heals");
        assert!(loud.sun()[2] < bright, "the impact flash fades");
    }
    #[test]
    fn the_sun_rises_and_brightens_with_the_section() {
        let settings = SceneSettings::new();
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 30.0;
        f.dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.1, 0.6);
        let mut quiet = VoronoiBloomState::new(4);
        let mut loud = VoronoiBloomState::new(4);
        for _ in 0..300 {
            f.audio.rms = 0.05;
            quiet.update(&f);
            f.audio.rms = 0.7;
            loud.update(&f);
        }
        assert!(loud.clock() > quiet.clock() * 1.3);
        // Compare at the same sun-path phase: only the section term differs.
        let mut probe = quiet.clone();
        probe.energy = loud.energy;
        probe.light = loud.light;
        assert!(probe.sun()[1] > quiet.sun()[1] + 0.08);
        assert!(probe.sun()[2] > quiet.sun()[2] + 0.3);
    }
}
