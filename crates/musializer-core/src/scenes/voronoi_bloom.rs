//! A spectral stained-glass field: deterministic motion and twelve independently
//! followed voices. The drawing half evaluates a bounded lattice Voronoi field.
use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::voronoi as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

#[derive(Clone, Debug, PartialEq)]
pub struct VoronoiBloomState {
    seed: u64,
    clock: f64,
    voices: [f32; 12],
    bass: f32,
    light: f32,
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
}
fn clean(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
fn follow(current: &mut f32, target: f32, dt: f32) {
    let rate = if target > *current { 15.0 } else { 2.8 };
    *current += (target - *current) * (1.0 - (-rate * dt).exp());
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
            .map_or(clean(inputs.rms * 1.8), |p| p.level(clean(inputs.rms)));
        follow(&mut self.light, clean(loud) * response, dt);
        follow(&mut self.bass, clean(inputs.bass).sqrt() * response, dt);
        for (i, voice) in self.voices.iter_mut().enumerate() {
            let start = i * frame.audio.trails.len() / 12;
            let end = (i + 1) * frame.audio.trails.len() / 12;
            let group = &frame.audio.trails[start..end];
            let power = group.iter().map(|v| clean(*v)).sum::<f32>() / group.len().max(1) as f32;
            follow(voice, power.sqrt() * response, dt);
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
        state_version: 1,
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
        for _ in 0..40 {
            b.update(&f);
        }
        assert_eq!(a, b);
        assert!(a.clock() > 1.0);
        assert_eq!(a.voices(), &[0.0; 12]);
    }
}
