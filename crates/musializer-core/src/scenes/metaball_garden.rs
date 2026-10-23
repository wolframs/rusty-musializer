//! A seeded, buoyant sculpture. Geometry is tiny CPU state; surface evaluation
//! and lighting are a bounded GPU pass in the drawing half. No render history.
use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::metaballs as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

#[derive(Clone, Debug, PartialEq)]
pub struct MetaballGardenState {
    seed: u64,
    clock: f64,
    bass: f32,
    light: f32,
    high: f32,
}
impl MetaballGardenState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            clock: 0.0,
            bass: 0.0,
            light: 0.0,
            high: 0.0,
        }
    }
    #[must_use]
    pub fn clock(&self) -> f32 {
        self.clock as f32
    }
    #[must_use]
    pub fn light(&self) -> f32 {
        self.light
    }
    #[must_use]
    pub fn high(&self) -> f32 {
        self.high
    }
    /// Nine bounded spheres, even when the renderer draws fewer. A shared
    /// smooth-union surface makes neighbours pull into soft necks.
    #[must_use]
    pub fn blobs(&self, size: f32, orbit: f32) -> [[f32; 4]; 9] {
        let seed = phase(self.seed);
        let t = self.clock as f32;
        std::array::from_fn(|i| {
            let a = seed + i as f32 * 2.399_963;
            let q = a + t * (0.22 + i as f32 * 0.013);
            let spread = 0.35 + orbit.clamp(0.0, 2.0) * 0.42;
            [
                spread * q.cos(),
                0.56 * (q * 1.31 + a).sin() + 0.13 * (t * 0.37 + a).cos(),
                spread * 0.65 * (q * 0.83 + a).sin(),
                size.clamp(0.35, 1.5) * (0.42 + 0.055 * a.sin()) * (1.0 + self.bass * 0.32),
            ]
        })
    }
}
fn phase(seed: u64) -> f32 {
    let mut n = seed.wrapping_add(0x9e3779b97f4a7c15);
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((n ^ (n >> 31)) >> 40) as f32 * (std::f32::consts::TAU / 16_777_216.0)
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
fn follow(current: &mut f32, target: f32, dt: f32) {
    let rate = if target > *current { 13.0 } else { 3.0 };
    *current += (target - *current) * (1.0 - (-rate * dt).exp());
}
impl SceneState for MetaballGardenState {
    fn id(&self) -> SceneId {
        SceneId::MetaballGarden
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        // Pause freezes *all* state, including followers, even if audio changes.
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        self.clock += f64::from(dt * frame.setting(self.id(), control::FLOW));
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
            .map_or(finite(inputs.rms) * 1.8, |p| p.level(finite(inputs.rms)));
        let high = frame
            .audio
            .trails
            .get(frame.audio.trails.len() * 3 / 4..)
            .unwrap_or(&[]);
        let treble = high.iter().map(|v| finite(*v)).sum::<f32>() / high.len().max(1) as f32;
        follow(
            &mut self.bass,
            finite(inputs.bass).sqrt().min(1.0) * response,
            dt,
        );
        follow(&mut self.light, finite(loud).min(1.0) * response, dt);
        follow(&mut self.high, treble.sqrt().min(1.0) * response, dt);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::MetaballGarden,
        state_version: 1,
        make_state: |seed| Box::new(MetaballGardenState::new(seed)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    #[test]
    fn sculpture_replays_with_seed_moves_in_silence_and_swallows_bass() {
        let settings = SceneSettings::new();
        let mut a = MetaballGardenState::new(29);
        let mut b = a.clone();
        let initial = a.blobs(0.8, 1.0);
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 60.0;
        for _ in 0..180 {
            a.update(&f);
            b.update(&f);
        }
        assert_eq!(a, b);
        assert_ne!(
            initial,
            a.blobs(0.8, 1.0),
            "ambient sculpture moves without audio"
        );
        assert_ne!(initial, MetaballGardenState::new(30).blobs(0.8, 1.0));
        let quiet_radius = a.blobs(0.8, 1.0)[0][3];
        f.audio.trails = &[0.7; 104];
        f.audio.rms = 0.7;
        for _ in 0..30 {
            a.update(&f);
        }
        assert!(
            a.blobs(0.8, 1.0)[0][3] > quiet_radius * 1.2,
            "bass visibly swells mass"
        );
        assert!(a.light() > 0.8 && a.high() > 0.7);
        let paused = a.clone();
        for dt in [0.0, -0.1, f32::NAN] {
            f.delta_seconds = dt;
            a.update(&f);
            assert_eq!(a, paused);
        }
    }
    #[test]
    fn response_zero_removes_music_and_track_profile_changes_the_light() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::MetaballGarden, control::RESPONSE, 0.0);
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 0.1;
        let mut quiet = MetaballGardenState::new(8);
        quiet.update(&f);
        let mut loud = MetaballGardenState::new(8);
        f.audio.rms = 0.9;
        f.audio.trails = &[1.0; 104];
        loud.update(&f);
        assert_eq!(quiet, loud);
        let settings = SceneSettings::new();
        f.settings = &settings;
        let mut absolute = MetaballGardenState::new(8);
        absolute.update(&f);
        f.dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.89, 1.0);
        let mut relative = MetaballGardenState::new(8);
        relative.update(&f);
        assert!(relative.light() < absolute.light() * 0.3);
    }
}
