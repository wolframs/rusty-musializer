//! SX5: six hooked ribbons, with a fixed weave and audio-carried light.
//!
//! Constructed from a circular arc and two straight runs, rotated in sixths.
//! The geometry study used https://openai.com/brand/ as a visual reference;
//! there is no image, SVG path or external implementation in the renderer.
//! Endpoint overlaps belong below the receiving strand. Music can change the
//! material, depth and uniform scale, never the crossing order or central hole.
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;
use std::f32::consts::{PI, TAU};

pub const STRANDS: usize = 6;
pub const SEGMENTS: usize = 160;
pub const HALF_WIDTH: f32 = 0.064;

/// Local coordinates in units of the mark's radius. The tiny quadratic corner
/// removes a miter spike without rounding away the mark's straight inner runs.
pub fn centerline(strand: usize, t: f32) -> [f32; 2] {
    let t = t.clamp(0.0, 1.0);
    let p = if t < 0.56 {
        let a = (-36.0 - 144.0 * t / 0.56).to_radians();
        [10.76 + 5.285 * a.cos(), 6.075 + 5.285 * a.sin()]
    } else if t < 0.75 {
        let u = (t - 0.56) / 0.19;
        [5.475, 6.075 + (11.75 - 6.075) * u]
    } else if t < 0.77 {
        let u = (t - 0.75) / 0.02;
        let a = [5.475, 11.75];
        let b = [5.475, 11.9];
        let c = [5.605, 11.975];
        std::array::from_fn(|i| {
            (1.0 - u).powi(2) * a[i] + 2.0 * u * (1.0 - u) * b[i] + u * u * c[i]
        })
    } else {
        let u = (t - 0.77) / 0.23;
        [5.605 + (12.0 - 5.605) * u, 11.975 + (15.668 - 11.975) * u]
    };
    let a = strand as f32 * TAU / STRANDS as f32;
    let (s, c) = a.sin_cos();
    let (x, y) = ((p[0] - 12.0) / 12.0, (p[1] - 12.0) / 12.0);
    [x * c - y * s, x * s + y * c]
}

/// A shared radial deformation, applied to every ribbon vertex. Adjacent
/// strands therefore still meet exactly. Its factor is positive and constant
/// along each ray, so even maximum settings cannot reverse a ray or close the
/// central hole (unlike independently translating six lobes).
pub fn breathe(p: [f32; 2], bands: &[f32; STRANDS], response: f32) -> [f32; 2] {
    let angle = p[1].atan2(p[0]);
    let mut weight = 0.0;
    let mut drive = 0.0;
    for (i, band) in bands.iter().enumerate() {
        let w = (0.5 + 0.5 * (angle - i as f32 * TAU / STRANDS as f32).cos()).powi(6);
        weight += w;
        drive += w * unit(*band);
    }
    let scale = 1.0 + 0.075 * response.clamp(0.0, 2.0) * drive / weight.max(0.001);
    [p[0] * scale, p[1] * scale]
}

/// Ends tuck beneath the adjacent ribbon. Sorting by this fixed depth is
/// deliberate: changing the order with audio would make the knot pop inside out.
pub fn depth(t: f32) -> f32 {
    (PI * t).sin() - 0.85 * (-((t - 0.08) / 0.10).powi(2)).exp()
}

fn unit(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KnotState {
    pub bands: [f32; STRANDS],
    pub energy: f32,
    pub beat: f32,
    pub time: f32,
    pub phase: f32,
}
impl KnotState {
    pub fn new(seed: u64) -> Self {
        Self {
            bands: [0.0; STRANDS],
            energy: 0.0,
            beat: 0.0,
            time: 0.0,
            phase: (seed % 65536) as f32 / 65536.0 * TAU,
        }
    }
}
impl SceneState for KnotState {
    fn id(&self) -> SceneId {
        SceneId::OpenAiKnot
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        // Absolute scene time makes seeks and a frozen frame stable. Followers
        // are replayed by the existing deterministic preview/export machinery.
        self.time = if frame.time_seconds.is_finite() {
            frame.time_seconds.max(0.0) as f32
        } else {
            0.0
        };
        let dt = if frame.delta_seconds.is_finite() {
            frame.delta_seconds.clamp(0.0, 0.1)
        } else {
            0.0
        };
        let n = frame.audio.bands.len();
        for (i, out) in self.bands.iter_mut().enumerate() {
            let start = n * i / STRANDS;
            let end = n * (i + 1) / STRANDS;
            let target = if end > start {
                (frame.audio.bands[start..end]
                    .iter()
                    .map(|x| unit(*x).powi(2))
                    .sum::<f32>()
                    / (end - start) as f32)
                    .sqrt()
            } else {
                0.0
            };
            let rate = if target > *out { 18.0 } else { 4.0 };
            *out += (target - *out) * (1.0 - (-dt * rate).exp());
        }
        self.energy = self.bands.iter().sum::<f32>() / STRANDS as f32;
        self.beat *= (-dt * 5.0).exp();
        if frame.audio.onset && dt > 0.0 {
            self.beat = self.beat.max(unit(frame.audio.spectral_flux * 4.0));
        }
    }
}
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::OpenAiKnot,
        state_version: 1,
        make_state: |seed| Box::new(KnotState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    #[test]
    fn authored_arc_and_straight_run_landmarks_stay_pinned() {
        for (t, expected) in [
            (0.21, [-0.103_333_33, -0.934_166_67]),
            (0.56, [-0.543_75, -0.493_75]),
            (0.75, [-0.543_75, -0.020_833_334]),
            (1.0, [0.0, 0.305_666_66]),
        ] {
            let actual = centerline(0, t);
            for axis in 0..2 {
                assert!((actual[axis] - expected[axis]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn sixfold_geometry_preserves_hole_and_bounds() {
        for i in 0..STRANDS {
            for j in 0..=SEGMENTS {
                let t = j as f32 / SEGMENTS as f32;
                let p = centerline(i, t);
                for bands in [[1.0; STRANDS], [1.0, 0.0, 1.0, 0.0, 1.0, 0.0]] {
                    let expanded = breathe(p, &bands, 2.0);
                    assert!(expanded[0].hypot(expanded[1]) >= p[0].hypot(p[1]));
                    assert!(expanded[0].hypot(expanded[1]) <= p[0].hypot(p[1]) * 1.151);
                }
                let r = p[0].hypot(p[1]);
                assert!(r - HALF_WIDTH > 0.18, "central opening collapsed: {p:?}");
                assert!(r + HALF_WIDTH < 1.08, "silhouette escaped: {p:?}");
                let q = centerline((i + 1) % STRANDS, t);
                assert!((q[0] - (p[0] * 0.5 - p[1] * 3.0_f32.sqrt() * 0.5)).abs() < 1e-5);
            }
        }
    }
    #[test]
    fn followers_are_selective_deterministic_and_release() {
        let settings = SceneSettings::default();
        let mut frame = SceneFrame::idle(&settings);
        let mut audio = [0.0; 120];
        audio[40..60].fill(1.0);
        frame.audio.bands = &audio;
        frame.delta_seconds = 1.0 / 60.0;
        let mut a = KnotState::new(42);
        let mut b = a.clone();
        for i in 0..60 {
            frame.time_seconds = i as f64 / 60.0;
            a.update(&frame);
            b.update(&frame);
        }
        assert_eq!(a, b);
        assert!(a.bands[2] > 0.99);
        assert_eq!(a.bands[0], 0.0);
        assert_eq!(a.bands[5], 0.0);
        frame.audio.bands = &[];
        for _ in 0..180 {
            a.update(&frame);
        }
        assert!(a.energy < 0.0001);
    }
    #[test]
    fn bad_audio_and_zero_dt_do_not_poison_state() {
        let settings = SceneSettings::default();
        let mut f = SceneFrame::idle(&settings);
        f.audio.bands = &[f32::NAN, f32::INFINITY, -1.0];
        f.delta_seconds = 0.016;
        let mut s = KnotState::new(0);
        s.update(&f);
        assert_eq!(s.energy, 0.0);
        f.audio.bands = &[1.0; 120];
        f.delta_seconds = 0.0;
        let before = s.clone();
        s.update(&f);
        assert_eq!(s, before);
    }
}
