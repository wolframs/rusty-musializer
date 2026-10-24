//! Metaball Garden: a molten candy sculpture in a dark studio.
//!
//! Everything that moves is decided here, on the CPU, from the seed and the
//! frame sequence; the drawing half (`musializer-app::scenes::metaball_garden`
//! and `metaball_garden.fs`) only raymarches what it is handed. That split is
//! what lets a still, a clip and a full export agree: the replay advances this
//! state frame by frame from the start of the track and the shader is a pure
//! function of the uniforms.
//!
//! The music is felt as *mass*, not as a meter:
//!
//! - **Kicks** (the bass-transient detector Clawd measured on real mixes, see
//!   [`crate::scenes::clawd::KICK_THRESHOLD`]) hit two damped springs. One
//!   squashes the whole sculpture toward the floor and lets it bounce back up —
//!   weight. The other flings the satellite droplets and the main blobs outward
//!   so the goo splits, then pulls them back in so it re-merges — the beat
//!   *pulls the sculpture apart and lets it close again*.
//! - **Sustained bass** swells every blob's radius and loosens the goo (a larger
//!   smooth-union radius), so a heavy section looks heavier and gooier.
//! - **Track-relative loudness** is followed twice: fast for the light, slow for
//!   the section. The slow one gathers the blobs into one molten mass and dollies
//!   the camera in on loud sections, and in quiet passages lets them drift apart
//!   into separate droplets, slows the clock and dims the studio — it breathes.
//! - **Treble transients** (the same positive-excursion measure over the top of
//!   the spectrum) spawn short star glints on the blobs' surfaces.
//!
//! Response zero leaves the ambient sculpture — it still moves on `flow` — and
//! removes every musical term, including kicks and glints.
use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::metaballs as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

/// Most main blobs the `count` control can ask for.
pub const MAX_MAINS: usize = 9;
/// Satellite droplets: small spheres on wider orbits that kiss the mass and get
/// flung off it on kicks.
pub const MAX_DROPLETS: usize = 6;
/// Every sphere the shader may be handed, mains first, then droplets.
pub const MAX_BLOBS: usize = MAX_MAINS + MAX_DROPLETS;
/// Glints alive at once. A ring buffer: a new one overwrites the oldest.
pub const MAX_SPARKLES: usize = 12;
/// Beads riding the chrome ring.
pub const BEADS: usize = 3;

/// The kick gate, on the mean positive excursion of the lowest quarter of the
/// bands over their trails. Clawd's measured value (0.05 finds a real groove
/// and stays quiet through pads); reused rather than re-guessed.
pub const KICK_THRESHOLD: f32 = crate::scenes::clawd::KICK_THRESHOLD;
/// A sustained excursion is one push, not a drumroll.
const KICK_REFRACTORY_SECONDS: f32 = 0.22;
/// The glint gate, on the same excursion measure over the top third of the
/// bands. Measured with `examples/organic_probe.rs` on a real mix: hats and
/// snares cross it a few times a second in a busy section and almost never in a
/// pad, which is the density a glint wants.
pub const HAT_THRESHOLD: f32 = 0.045;
const HAT_REFRACTORY_SECONDS: f32 = 0.09;
/// A glint's whole life: a 40 ms flare and an exponential fade.
pub const SPARKLE_SECONDS: f32 = 0.6;

/// Squash spring: about three bounces a second, underdamped so it rebounds
/// (stretches) once before settling. Units: 1.0 is a full squash.
const SQUASH_HZ: f32 = 3.0;
const SQUASH_DAMPING: f32 = 0.3;
/// Burst spring: slower, so the split is visible and the re-merge overshoots
/// into a wobble.
const BURST_HZ: f32 = 1.5;
const BURST_DAMPING: f32 = 0.36;
/// Integration step for both springs. Fixed and small, so the spring's motion
/// does not depend on the frame rate it is replayed at.
const SPRING_STEP: f32 = 1.0 / 240.0;

/// The floor plane the shader draws; the sculpture floats above it.
pub const FLOOR_Y: f32 = -1.35;

/// Relative sizes of the main blobs: one mother blob and a family of smaller
/// ones, so the sculpture has several scales instead of nine equal marbles.
const MAIN_SCALE: [f32; MAX_MAINS] = [1.18, 0.92, 0.78, 0.66, 1.0, 0.58, 0.84, 0.5, 0.72];
const DROPLET_SCALE: [f32; MAX_DROPLETS] = [0.36, 0.27, 0.32, 0.22, 0.3, 0.25];

/// The candy palette, sRGB. The main mass draws from the warm family — hot
/// pink, tangerine, orchid, coral — whose blends stay clean where lobes merge
/// (pink into lime goes grey; pink into tangerine goes coral). The droplets
/// draw from the cool accents — aqua and lime — so the satellites read as
/// separate sweets against the mass.
pub const WARM: [[f32; 3]; 4] = [
    [1.0, 0.2, 0.5],
    [1.0, 0.5, 0.14],
    [0.72, 0.38, 1.0],
    [1.0, 0.36, 0.34],
];
pub const COOL: [[f32; 3]; 3] = [[0.16, 0.82, 0.95], [0.6, 0.93, 0.26], [0.36, 0.55, 1.0]];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Spring {
    x: f32,
    v: f32,
}

impl Spring {
    fn step(&mut self, dt: f32, hz: f32, damping: f32) {
        let omega = std::f32::consts::TAU * hz;
        let steps = (dt / SPRING_STEP).ceil().clamp(1.0, 240.0) as u32;
        let h = dt / steps as f32;
        for _ in 0..steps {
            // Semi-implicit Euler: stable for these stiffnesses at h = 1/240.
            self.v += (-omega * omega * self.x - 2.0 * damping * omega * self.v) * h;
            self.x += self.v * h;
        }
    }
}

/// The sculpture's camera for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: [f32; 3],
    pub target: [f32; 3],
}

/// The chrome ring: its unit plane normal, major radius and the beads on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub normal: [f32; 3],
    pub radius: f32,
    pub beads: [[f32; 4]; BEADS],
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetaballGardenState {
    seed: u64,
    clock: f64,
    bass: f32,
    light: f32,
    high: f32,
    /// Slow track-relative energy: the section, not the beat.
    energy: f32,
    squash: Spring,
    burst: Spring,
    /// Kick flash for the rim light, decays in about a quarter second.
    accent: f32,
    kick_refractory: f32,
    kick_count: u64,
    hat_refractory: f32,
    sparkle_count: u64,
    /// x, y, z, age (seconds), power.
    sparkles: [[f32; 5]; MAX_SPARKLES],
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
            energy: 0.0,
            squash: Spring::default(),
            burst: Spring::default(),
            accent: 0.0,
            kick_refractory: 0.0,
            kick_count: 0,
            hat_refractory: 0.0,
            sparkle_count: 0,
            sparkles: [[0.0, 0.0, 0.0, SPARKLE_SECONDS, 0.0]; MAX_SPARKLES],
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
    #[must_use]
    pub fn bass(&self) -> f32 {
        self.bass
    }
    #[must_use]
    pub fn energy(&self) -> f32 {
        self.energy
    }
    /// Squash displacement: positive is pressed toward the floor, negative the
    /// rebound stretch.
    #[must_use]
    pub fn squash(&self) -> f32 {
        self.squash.x.clamp(-0.7, 0.95)
    }
    /// How far the goo is split apart right now (0 at rest).
    #[must_use]
    pub fn burst(&self) -> f32 {
        self.burst.x.clamp(-0.35, 1.6)
    }
    #[must_use]
    pub fn accent(&self) -> f32 {
        self.accent
    }
    /// How hard the goo's surface is quivering, 0..1: the two springs' speeds.
    /// A kick sets the jelly rippling and it calms as the springs settle.
    #[must_use]
    pub fn wobble(&self) -> f32 {
        let tau = std::f32::consts::TAU;
        let squash = (self.squash.v.abs() / (tau * SQUASH_HZ)).min(1.0);
        let burst = (self.burst.v.abs() / (tau * BURST_HZ)).min(1.0);
        (0.6 * squash + 0.6 * burst).min(1.0)
    }
    #[must_use]
    pub fn kick_count(&self) -> u64 {
        self.kick_count
    }
    #[must_use]
    pub fn sparkle_count(&self) -> u64 {
        self.sparkle_count
    }
    /// Live glints as `[x, y, z, intensity]`; dead slots have intensity 0.
    #[must_use]
    pub fn sparkles(&self) -> [[f32; 4]; MAX_SPARKLES] {
        self.sparkles.map(|[x, y, z, age, power]| {
            let envelope = if age >= SPARKLE_SECONDS {
                0.0
            } else if age < 0.04 {
                age / 0.04
            } else {
                (-(age - 0.04) * 7.0).exp()
            };
            [x, y, z, power * envelope]
        })
    }
    /// Live glint count, for the report.
    #[must_use]
    pub fn live_sparkles(&self) -> usize {
        self.sparkles
            .iter()
            .filter(|s| s[3] < SPARKLE_SECONDS && s[4] > 0.0)
            .count()
    }

    /// The sculpture's centre: it bobs on its own buoyancy and drops when a
    /// kick squashes it.
    #[must_use]
    pub fn centre(&self) -> [f32; 3] {
        let t = self.clock as f32;
        let bob = 0.07 * (t * 0.43 + phase(self.seed)).sin();
        [0.0, bob - 0.34 * self.squash().max(0.0), 0.0]
    }

    /// Nine main blobs, even when the renderer draws fewer. Several scales, on
    /// their own slow orbits; loud sections gather them into one mass, kicks
    /// push them apart.
    #[must_use]
    pub fn blobs(&self, size: f32, orbit: f32) -> [[f32; 4]; MAX_MAINS] {
        let seed = phase(self.seed);
        let t = self.clock as f32;
        let c = self.centre();
        let gather = 1.0 - 0.32 * self.energy;
        let split = 1.0 + 0.38 * self.burst();
        let spread = (0.32 + orbit.clamp(0.0, 2.0) * 0.4) * gather * split;
        let mass = size.clamp(0.35, 1.5) * (1.0 + self.bass * 0.34);
        std::array::from_fn(|i| {
            let a = seed + i as f32 * 2.399_963;
            let q = a + t * (0.21 + i as f32 * 0.017);
            [
                c[0] + spread * q.cos(),
                c[1] + 0.5 * (q * 1.31 + a).sin() * gather + 0.12 * (t * 0.37 + a).cos(),
                c[2] + spread * 0.7 * (q * 0.83 + a).sin(),
                0.4 * MAIN_SCALE[i] * mass,
            ]
        })
    }

    /// Six satellite droplets on wider, tilted orbits. At rest they graze the
    /// mass and pull necks out of it; a kick flings them clear.
    #[must_use]
    pub fn droplets(&self, size: f32, orbit: f32) -> [[f32; 4]; MAX_DROPLETS] {
        let seed = phase(self.seed ^ 0x5bd1_e995);
        let t = self.clock as f32;
        let c = self.centre();
        let fling = 1.0 + 1.1 * self.burst();
        let reach = (1.05 + orbit.clamp(0.0, 2.0) * 0.32) * (1.0 - 0.18 * self.energy) * fling;
        let mass = size.clamp(0.35, 1.5) * (1.0 + self.bass * 0.2);
        std::array::from_fn(|j| {
            let a = seed + j as f32 * 2.094_395 + (j / 3) as f32 * 1.047;
            let q =
                a + t * (0.33 + j as f32 * 0.041) * if j.is_multiple_of(2) { 1.0 } else { -1.0 };
            let tilt = 0.45 + 0.35 * (a * 1.7).sin();
            let r = reach * (0.9 + 0.2 * (a * 3.1).cos());
            [
                c[0] + r * q.cos(),
                c[1] + r * tilt * q.sin() * 0.55 + 0.1 * (t * 0.5 + a).sin(),
                c[2] + r * 0.75 * q.sin() * (1.0 - tilt * 0.4),
                0.4 * DROPLET_SCALE[j] * mass,
            ]
        })
    }

    /// The chrome ring around the sculpture: it precesses slowly, wobbles on a
    /// kick, and three beads ride it.
    #[must_use]
    pub fn ring(&self, orbit: f32) -> Ring {
        let t = self.clock as f32;
        let s = phase(self.seed ^ 0x2545_f491);
        let tilt = 0.62 + 0.1 * (t * 0.19 + s).sin() + 0.12 * self.accent;
        let precession = s + t * 0.09;
        let normal = [
            tilt.sin() * precession.cos(),
            tilt.cos(),
            tilt.sin() * precession.sin(),
        ];
        let radius = 1.72 + 0.28 * orbit.clamp(0.0, 2.0);
        // An orthonormal basis in the ring plane.
        let helper = if normal[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = normalize(cross(helper, normal));
        let v = cross(normal, u);
        let c = self.centre();
        let beads = std::array::from_fn(|k| {
            let angle =
                s * 2.0 + t * 0.55 + k as f32 * 2.094_395 + 0.4 * (k as f32 + t * 0.2).sin();
            let (sin, cos) = angle.sin_cos();
            [
                c[0] + radius * (u[0] * cos + v[0] * sin),
                c[1] + radius * (u[1] * cos + v[1] * sin),
                c[2] + radius * (u[2] * cos + v[2] * sin),
                if k == 0 { 0.085 } else { 0.06 },
            ]
        });
        Ring {
            normal,
            radius,
            beads,
        }
    }

    /// A slow dolly: the camera drifts around the sculpture and off its axis so
    /// the frame composes instead of centring, and pushes in on loud sections.
    /// It backs off for a larger sculpture or a wider ring (`size`, `orbit`), so
    /// no setting pushes the ring out of the frame.
    #[must_use]
    pub fn camera(&self, size: f32, orbit: f32) -> Camera {
        let t = self.clock as f32;
        let s = phase(self.seed ^ 0x9e37_79b9);
        let azimuth = 0.42 * (t * 0.047 + s).sin() + 0.12 * (t * 0.019).sin();
        let elevation = 0.27 + 0.06 * (t * 0.031 + s * 0.5).sin();
        let scale = (size.clamp(0.35, 1.5) - 0.8).max(0.0) * 1.6 + orbit.clamp(0.0, 2.0) * 0.55;
        let distance = 6.05 + scale - 0.75 * self.energy + 0.12 * self.squash().max(0.0);
        let c = self.centre();
        let target = [0.32 * (t * 0.037 + s).sin(), -0.28 + 0.4 * c[1], 0.0];
        Camera {
            position: [
                target[0] + distance * elevation.cos() * azimuth.sin(),
                target[1] + distance * elevation.sin() + 0.35,
                target[2] + distance * elevation.cos() * azimuth.cos(),
            ],
            target,
        }
    }

    /// The pigment of each blob slot, sRGB: mains a seeded rotation of [`WARM`],
    /// droplets of [`COOL`], all turned by `hue`.
    #[must_use]
    pub fn tints(&self, hue_degrees: f32) -> [[f32; 3]; MAX_BLOBS] {
        let n = mix64(self.seed);
        let (warm, cool) = ((n % 4) as usize, ((n >> 8) % 3) as usize);
        let angle = hue_degrees.to_radians();
        std::array::from_fn(|i| {
            let colour = if i < MAX_MAINS {
                WARM[(warm + i) % WARM.len()]
            } else {
                COOL[(cool + i) % COOL.len()]
            };
            turn_hue(colour, angle)
        })
    }

    fn spawn_sparkles(&mut self, power: f32, size: f32, orbit: f32, mains: usize) {
        let blobs = self.blobs(size, orbit);
        let count = if power > 0.6 { 2 } else { 1 };
        for _ in 0..count {
            let n = mix64(self.seed ^ self.sparkle_count.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let pick = (n % mains.max(1) as u64) as usize;
            let b = blobs[pick];
            // A direction biased toward the camera and the key light, so a
            // glint lands where a highlight would.
            let unit = |shift: u32| ((n >> shift) & 0xffff) as f32 / 65_535.0 * 2.0 - 1.0;
            let dir = normalize([unit(8) - 0.25, unit(24) * 0.7 + 0.55, unit(40) * 0.6 + 0.85]);
            let slot = (self.sparkle_count % MAX_SPARKLES as u64) as usize;
            let reach = b[3] * 1.04;
            self.sparkles[slot] = [
                b[0] + dir[0] * reach,
                b[1] + dir[1] * reach,
                b[2] + dir[2] * reach,
                0.0,
                power,
            ];
            self.sparkle_count = self.sparkle_count.wrapping_add(1);
        }
    }
}

/// Mean positive excursion of `bands` over `trails` across a slice of the
/// spectrum: "this region just moved", the shape of spectral flux.
fn excursion(audio: &SceneAudioFrame<'_>, from: f32, to: f32) -> f32 {
    let count = audio.bands.len().min(audio.trails.len());
    if count == 0 {
        return 0.0;
    }
    let start = ((count as f32 * from) as usize).min(count - 1);
    let end = ((count as f32 * to) as usize).clamp(start + 1, count);
    let sum: f32 = audio.bands[start..end]
        .iter()
        .zip(&audio.trails[start..end])
        .map(|(band, trail)| finite(band - trail))
        .sum();
    sum / (end - start) as f32
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
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
fn follow(current: &mut f32, target: f32, dt: f32, up: f32, down: f32) {
    let rate = if target > *current { up } else { down };
    *current += (target - *current) * (1.0 - (-rate * dt).exp());
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / length, v[1] / length, v[2] / length]
}
/// Rotates an sRGB colour about the grey axis (Rodrigues), the same turn the
/// other scenes' `hue` controls make.
fn turn_hue(c: [f32; 3], angle: f32) -> [f32; 3] {
    let k = 1.0 / 3.0f32.sqrt();
    let (sin, cos) = angle.sin_cos();
    let dot = (c[0] + c[1] + c[2]) * k;
    let cross = [k * (c[2] - c[1]), k * (c[0] - c[2]), k * (c[1] - c[0])];
    std::array::from_fn(|i| (c[i] * cos + cross[i] * sin + k * dot * (1.0 - cos)).clamp(0.0, 1.0))
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
        let setting = |i| frame.setting(SceneId::MetaballGarden, i);
        let response = setting(control::RESPONSE);
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
            13.0,
            3.0,
        );
        follow(
            &mut self.light,
            finite(loud).min(1.0) * response,
            dt,
            13.0,
            3.0,
        );
        follow(
            &mut self.high,
            treble.sqrt().min(1.0) * response,
            dt,
            13.0,
            3.0,
        );
        follow(
            &mut self.energy,
            (finite(loud) * response).min(1.0),
            dt,
            0.9,
            0.35,
        );

        // Quiet passages breathe: the clock slows to 70 % in the track's own
        // quiet and runs at 130 % in its loud. Response zero pins it at 100 %.
        let pace = 0.7 + 0.6 * self.energy + 0.3 * (1.0 - response.min(1.0));
        self.clock += f64::from(dt * setting(control::FLOW) * pace);

        // Kicks: weight and the split.
        let kick = excursion(&frame.audio, 0.0, 0.25);
        self.kick_refractory = (self.kick_refractory - dt).max(0.0);
        if response > 0.0 && kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            self.kick_refractory = KICK_REFRACTORY_SECONDS;
            self.kick_count = self.kick_count.wrapping_add(1);
            let power = (kick / (2.0 * KICK_THRESHOLD)).clamp(0.45, 1.0) * response.min(1.5);
            let squash_omega = std::f32::consts::TAU * SQUASH_HZ;
            let burst_omega = std::f32::consts::TAU * BURST_HZ;
            self.squash.v += squash_omega * 0.9 * power;
            self.burst.v += burst_omega * 0.85 * power * (0.55 + 0.45 * self.energy);
            self.accent = self.accent.max(power.min(1.0));
        }
        self.squash.step(dt, SQUASH_HZ, SQUASH_DAMPING);
        self.burst.step(dt, BURST_HZ, BURST_DAMPING);
        self.accent *= (-5.0 * dt).exp();

        // Glints: age the live ones, spawn on a treble transient.
        for sparkle in &mut self.sparkles {
            sparkle[3] = (sparkle[3] + dt).min(SPARKLE_SECONDS);
        }
        let hat = excursion(&frame.audio, 0.66, 1.0);
        self.hat_refractory = (self.hat_refractory - dt).max(0.0);
        if response > 0.0 && hat >= HAT_THRESHOLD && self.hat_refractory <= 0.0 {
            self.hat_refractory = HAT_REFRACTORY_SECONDS;
            let power = (hat / (2.0 * HAT_THRESHOLD)).clamp(0.4, 1.0) * response.min(1.5);
            let mains = (setting(control::COUNT).round() as usize).clamp(3, MAX_MAINS);
            self.spawn_sparkles(
                power,
                setting(control::SIZE),
                setting(control::ORBIT),
                mains,
            );
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::MetaballGarden,
        state_version: 2,
        make_state: |seed| Box::new(MetaballGardenState::new(seed)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    fn kick_bands() -> (Vec<f32>, Vec<f32>) {
        // The lowest quarter jumps well above its trail; the rest sits still.
        let mut bands = vec![0.2; 104];
        bands[..26].fill(0.5);
        (bands, vec![0.2; 104])
    }

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
        let (bands, _) = kick_bands();
        f.audio.bands = &bands;
        loud.update(&f);
        assert_eq!(quiet, loud);
        let settings = SceneSettings::new();
        f.settings = &settings;
        f.audio.bands = &[];
        let mut absolute = MetaballGardenState::new(8);
        absolute.update(&f);
        f.dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.89, 1.0);
        let mut relative = MetaballGardenState::new(8);
        relative.update(&f);
        assert!(relative.light() < absolute.light() * 0.3);
    }

    #[test]
    fn a_kick_drops_the_sculpture_splits_the_goo_and_lets_it_close_again() {
        let settings = SceneSettings::new();
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 30.0;
        let mut state = MetaballGardenState::new(5);
        let rest_y = state.centre()[1];
        let rest_drop = state.droplets(0.8, 1.0);
        let (bands, trails) = kick_bands();
        f.audio.bands = &bands;
        f.audio.trails = &trails;
        state.update(&f);
        assert_eq!(state.kick_count(), 1);
        // Hold the excursion: the refractory makes it one push, not a drumroll.
        state.update(&f);
        assert_eq!(state.kick_count(), 1);
        f.audio.bands = &trails;
        let mut deepest = 0.0f32;
        let mut widest = 0.0f32;
        let mut rebound = 0.0f32;
        for _ in 0..90 {
            state.update(&f);
            deepest = deepest.max(state.squash());
            widest = widest.max(state.burst());
            rebound = rebound.min(state.squash());
        }
        assert!(
            deepest > 0.35,
            "a kick presses the sculpture down: {deepest}"
        );
        assert!(rebound < -0.05, "and it rebounds into a stretch: {rebound}");
        assert!(widest > 0.15, "the goo splits: {widest}");
        assert!(
            state.squash().abs() < 0.02 && state.burst().abs() < 0.05,
            "and settles"
        );
        // The split is visible in the droplets' reach while it lasts.
        let mut probe = MetaballGardenState::new(5);
        probe.burst.x = 1.0;
        let reach = |d: [[f32; 4]; MAX_DROPLETS], c: [f32; 3]| {
            d.iter()
                .map(|b| ((b[0] - c[0]).powi(2) + (b[2] - c[2]).powi(2)).sqrt())
                .sum::<f32>()
        };
        assert!(
            reach(probe.droplets(0.8, 1.0), probe.centre())
                > 1.5 * reach(rest_drop, [0.0, rest_y, 0.0])
        );
        assert!(state.centre()[1] > rest_y - 0.3);
    }

    #[test]
    fn treble_transients_spawn_glints_that_fade_and_quiet_sections_slow_the_clock() {
        let settings = SceneSettings::new();
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 30.0;
        let mut state = MetaballGardenState::new(3);
        let mut bands = vec![0.2; 104];
        bands[80..].fill(0.4);
        let trails = vec![0.2; 104];
        f.audio.bands = &bands;
        f.audio.trails = &trails;
        state.update(&f);
        assert!(state.live_sparkles() >= 1);
        assert!(state
            .sparkles()
            .iter()
            .all(|s| s.iter().all(|v| v.is_finite())));
        f.audio.bands = &trails;
        for _ in 0..30 {
            state.update(&f);
        }
        assert_eq!(state.live_sparkles(), 0, "glints are short");

        // Same flow, opposite ends of the track's own range.
        let dynamics = crate::audio::track_dynamics::TrackDynamics::from_bounds(0.1, 0.6);
        let mut quiet = MetaballGardenState::new(3);
        let mut loud = MetaballGardenState::new(3);
        f.dynamics = dynamics;
        for _ in 0..300 {
            f.audio.rms = 0.05;
            quiet.update(&f);
            f.audio.rms = 0.7;
            loud.update(&f);
        }
        assert!(loud.clock() > quiet.clock() * 1.4);
        assert!(loud.energy() > 0.9 && quiet.energy() < 0.05);
        // Loud gathers the blobs and dollies the camera in.
        let spread = |s: &MetaballGardenState| {
            let c = s.centre();
            s.blobs(0.8, 1.0)
                .iter()
                .map(|b| ((b[0] - c[0]).powi(2) + (b[2] - c[2]).powi(2)).sqrt())
                .sum::<f32>()
        };
        assert!(spread(&loud) < spread(&quiet));
        let distance = |s: &MetaballGardenState| {
            let cam = s.camera(0.8, 1.0);
            (0..3)
                .map(|i| (cam.position[i] - cam.target[i]).powi(2))
                .sum::<f32>()
                .sqrt()
        };
        assert!(distance(&loud) < distance(&quiet) - 0.5);
    }

    #[test]
    fn the_spring_does_not_depend_on_the_replay_frame_rate() {
        let mut a = Spring { x: 0.0, v: 10.0 };
        let mut b = a;
        for _ in 0..30 {
            a.step(1.0 / 30.0, SQUASH_HZ, SQUASH_DAMPING);
        }
        for _ in 0..60 {
            b.step(1.0 / 60.0, SQUASH_HZ, SQUASH_DAMPING);
        }
        assert!((a.x - b.x).abs() < 1e-4 && (a.v - b.v).abs() < 1e-3);
    }

    #[test]
    fn hue_turns_every_tint_and_geometry_stays_finite() {
        let state = MetaballGardenState::new(11);
        assert_ne!(state.tints(0.0), state.tints(90.0));
        let (a, b) = (state.tints(0.0)[0], state.tints(360.0)[0]);
        assert!(
            (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4),
            "a full turn is identity"
        );
        let ring = state.ring(1.0);
        let n = ring.normal;
        assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-4);
        for b in ring.beads {
            assert!(b.iter().all(|v| v.is_finite()));
        }
        let cam = state.camera(1.5, 2.0);
        assert!(
            cam.position[1] > FLOOR_Y + 1.0,
            "the camera stays above the floor"
        );
    }
}
