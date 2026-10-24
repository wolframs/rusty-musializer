//! Fluid Ink: persistent, music-stirred dye in a two-dimensional fluid.
//!
//! Original Rust implementation of semi-Lagrangian advection, pressure
//! projection and vorticity confinement, inspired by the visible WebGL Fluid
//! demo (https://webdev.teknium.io/d/gl/demos/webgl-fluid-ink/). Method:
//! Jos Stam, Stable Fluids (1999), https://doi.org/10.1145/311535.311548.
//! No third-party implementation is copied or loaded.
//!
//! The solver lives here, rather than in GPU feedback, so skipped drawing during
//! offline still/clip replay cannot lose the ink's history. Fixed 60 Hz substeps
//! bound cost after a stall and freeze exactly on pause. Rendering only uploads
//! the resulting dye; it never advances the simulation.
//!
//! ## What the music does (2026-10-24 rework)
//!
//! The first version pushed three continuously orbiting jets harder when the
//! track was louder, so every moment looked like every other: a vessel full of
//! evenly mixed colour. Now each musical gesture has its own visible act:
//!
//! - **A kick** fires a burst: one strong impulse from a point on a ring around
//!   the frame, aimed past the centre. The projection turns an impulse into a
//!   vortex pair, so a kick reads as a mushroom of fresh ink rolling inward. The
//!   ink arrives *hot* (the fourth dye channel), which the display draws
//!   white-hot and blooming and which cools in half a second. Placement walks a
//!   golden-angle sequence, so consecutive bursts never land on each other.
//! - **Sustained bass** builds pressure, a slow follower that spins the whole
//!   vessel and makes the next burst bigger. The spin direction reverses each
//!   time the pressure has fully released, so two drops do not turn alike.
//! - **A transient that is not a kick** (snare, hat, consonant) flicks a thin,
//!   fast spray of complementary sparks across the last burst.
//! - **Quiet** breathes: forces fall to two slow ribbons that puff on a
//!   six-second breath, the water thickens, and ink lingers longer.
//!
//! Every envelope advances per fixed substep from the frame's audio, never per
//! drawn frame, which is what keeps 30 Hz and 60 Hz replays bit-identical.

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::fluid as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 144;
const CELLS: usize = WIDTH * HEIGHT;
/// Dye is twice as fine as velocity, so thin pigment folds survive the solve.
pub const DYE_WIDTH: usize = WIDTH * 2;
pub const DYE_HEIGHT: usize = HEIGHT * 2;
const DYE_CELLS: usize = DYE_WIDTH * DYE_HEIGHT;
const STEP: f32 = 1.0 / 60.0;
/// The most track time one update may bank: six substeps. A preview frame
/// that stalls hands the next one a long delta, and the first version's
/// 0.25 s allowance (fifteen substeps) could make that frame stall too, and
/// the next, until the preview spiralled (measured 2026-10-24: 1021 steps in
/// 600 frames, 46 stalls, worst 319 ms). Every export at 10 fps and up is a
/// whole number of substeps under this cap, so its frames do not change.
const MAX_BANKED: f32 = 6.0 * STEP;
/// Red-black successive over-relaxation, warm-started from the previous
/// step's pressure. The first version ran 32 cold Jacobi sweeps; carried
/// pressure plus over-relaxation removes more divergence in fewer sweeps.
const PRESSURE_SWEEPS: usize = 20;
const SOR_OMEGA: f32 = 1.72;
const PRESSURE_CARRY: f32 = 0.8;
/// The heat channel's half-life in track seconds: a kick's white-hot core has
/// cooled to its pigment by the next beat at ordinary tempi.
const HEAT_HALF_LIFE: f32 = 0.42;
/// Mean positive excursion of the lowest quarter of the bands over their own
/// trails. Clawd's kick detector, measured on real material (`clawd.rs`,
/// `KICK_THRESHOLD`): 0.05 finds a real groove and stays quiet through pads.
const KICK_THRESHOLD: f32 = 0.05;
const KICK_REFRACTORY: f32 = 0.2;
const SPRAY_THRESHOLD: f32 = 0.42;
const SPRAY_REFRACTORY: f32 = 0.11;
const GOLDEN_ANGLE: f32 = 2.399_963;
const VELOCITY_LIMIT: f32 = 160.0;
const DYE_LIMIT: f32 = 5.0;
/// Dye channels: red, green and blue pigment, then heat.
pub type Dye = [f32; 4];
type Velocity = [f32; 2];

/// The musical envelopes, as the solver last saw them. The display reads the
/// same values, so a still's light agrees with its ink.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InkMusic {
    /// Track-relative loudness, fast attack and slow release.
    pub level: f32,
    /// Bass weight from the trailing smear.
    pub bass: f32,
    /// Slow follower of sustained bass, `0..=1`.
    pub pressure: f32,
    /// The last kick's envelope: `1` at a strong burst, gone in ~0.3 s.
    pub kick: f32,
    pub kicks: u64,
    /// Transient envelope for the non-kick sprays.
    pub spray: f32,
    pub sprays: u64,
    /// Quietness, `0` in a loud passage and near `1` in a quiet one.
    pub breath: f32,
    /// Signed spin of the pressure swirl.
    pub swirl: f32,
}

pub struct FluidInkState {
    phase: f32,
    velocity: Vec<Velocity>,
    velocity_next: Vec<Velocity>,
    dye: Vec<Dye>,
    dye_next: Vec<Dye>,
    dye_corrected: Vec<Dye>,
    backtrace: Vec<Velocity>,
    pressure: Vec<f32>,
    divergence: Vec<f32>,
    curl: Vec<f32>,
    accumulator: f32,
    /// Fluid time, scaled by Flow speed: the emitters' paths.
    clock: f32,
    /// Track seconds since the scene began: palette and light drift.
    seconds: f32,
    steps: u64,
    music: InkMusic,
    kick_refractory: f32,
    spray_refractory: f32,
    /// Whether the pressure has built since it last released fully, which is
    /// when the swirl reverses.
    pressure_armed: bool,
    last_burst: [f32; 2],
    last_burst_direction: [f32; 2],
    workers: usize,
}

impl FluidInkState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let phase = (seed % 997) as f32 * 0.017;
        let mut state = Self {
            phase,
            velocity: vec![[0.0; 2]; CELLS],
            velocity_next: vec![[0.0; 2]; CELLS],
            dye: vec![[0.0; 4]; DYE_CELLS],
            dye_next: vec![[0.0; 4]; DYE_CELLS],
            dye_corrected: vec![[0.0; 4]; DYE_CELLS],
            backtrace: vec![[0.0; 2]; DYE_CELLS],
            pressure: vec![0.0; CELLS],
            divergence: vec![0.0; CELLS],
            curl: vec![0.0; CELLS],
            accumulator: 0.0,
            clock: 0.0,
            seconds: 0.0,
            steps: 0,
            music: InkMusic {
                swirl: if seed.is_multiple_of(2) { 1.0 } else { -1.0 },
                ..InkMusic::default()
            },
            kick_refractory: 0.0,
            spray_refractory: 0.0,
            pressure_armed: false,
            last_burst: [0.5, 0.5],
            last_burst_direction: [1.0, 0.0],
            // Threads only split rows; every cell is computed from the same
            // inputs whatever the count, so the count cannot change a pixel.
            workers: std::thread::available_parallelism().map_or(1, |n| n.get().min(6)),
        };
        // An already wetted vessel invites playing before the first beat:
        // broad seeded eddies and a few clouds of the opening palette.
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let fx = x as f32;
                let fy = y as f32;
                state.velocity[y * WIDTH + x] = [
                    9.0 * (fy * 0.061 + phase).cos() + 5.0 * (fx * 0.083 + fy * 0.097).cos(),
                    -8.0 * (fx * 0.052 - phase).cos() - 4.0 * (fx * 0.083 + fy * 0.097).cos(),
                ];
            }
        }
        for i in 0..4 {
            let a = i as f32 * std::f32::consts::FRAC_PI_2 + phase;
            state.splat(
                [0.5 + 0.24 * a.cos(), 0.5 + 0.22 * a.sin()],
                [30.0 * a.sin(), -30.0 * a.cos()],
                hue_color(state.palette_hue(i as f32 * 40.0 - 40.0)),
                0.0,
                0.06,
                0.9,
            );
        }
        state.project();
        state
    }

    /// Pigment and heat, `DYE_WIDTH * DYE_HEIGHT` cells, row 0 at the top.
    #[must_use]
    pub fn dye(&self) -> &[Dye] {
        &self.dye
    }
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }
    #[must_use]
    pub fn music(&self) -> InkMusic {
        self.music
    }

    /// Where the light that backs the vessel sits, in frame units (`0..1`,
    /// y down). It wanders slowly and is drawn by the display; it lives here
    /// so a still and the video frame put it in the same place.
    #[must_use]
    pub fn light(&self) -> [f32; 2] {
        let t = self.seconds;
        [
            0.5 + 0.2 * (t * 0.043 + self.phase).sin(),
            0.3 + 0.07 * (t * 0.061 + 1.3 * self.phase).sin(),
        ]
    }

    /// Track seconds the scene has run, for the display's grain clock.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        self.seconds
    }

    /// Share of the vessel holding visible pigment, sampled on a sparse grid.
    /// The report prints it: a vessel at 100 % has lost its negative space.
    #[must_use]
    pub fn ink_coverage(&self) -> f32 {
        let mut inked = 0usize;
        let mut total = 0usize;
        for y in (0..DYE_HEIGHT).step_by(4) {
            for x in (0..DYE_WIDTH).step_by(4) {
                let d = self.dye[y * DYE_WIDTH + x];
                total += 1;
                if d[0].max(d[1]).max(d[2]) > 0.12 {
                    inked += 1;
                }
            }
        }
        inked as f32 / total.max(1) as f32
    }

    /// The palette's centre hue in degrees, plus `offset`: an analogous window
    /// drifting slowly and leaning warmer under bass pressure. Neighbouring
    /// hues mix into a third vivid hue; the first version's lime, orchid and
    /// coral mixed into olive and grey wherever they met.
    fn palette_hue(&self, offset: f32) -> f32 {
        205.0
            + 26.0 * (self.seconds * std::f32::consts::TAU / 110.0 + self.phase).sin()
            + 34.0 * self.music.pressure
            + offset
    }

    fn splat(
        &mut self,
        position: [f32; 2],
        force: Velocity,
        color: [f32; 3],
        heat: f32,
        radius: f32,
        amount: f32,
    ) {
        // Radius is relative to the short axis: circles in the 16:9 simulation.
        let r = radius * HEIGHT as f32;
        let cx = position[0] * WIDTH as f32;
        let cy = position[1] * HEIGHT as f32;
        if force != [0.0, 0.0] {
            let x0 = (cx - 3.0 * r).max(1.0) as usize;
            let x1 = ((cx + 3.0 * r).max(0.0) as usize).min(WIDTH - 2);
            let y0 = (cy - 3.0 * r).max(1.0) as usize;
            let y1 = ((cy + 3.0 * r).max(0.0) as usize).min(HEIGHT - 2);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let weight = (-(dx * dx + dy * dy) / (r * r)).exp();
                    let i = y * WIDTH + x;
                    for (v, f) in self.velocity[i].iter_mut().zip(force) {
                        *v = (*v + weight * f).clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT);
                    }
                }
            }
        }
        if amount <= 0.0 {
            return;
        }
        // Pigment lives on its own finer grid; the force radius is unchanged.
        let (r, cx, cy) = (r * 2.0, cx * 2.0, cy * 2.0);
        let x0 = (cx - 3.0 * r).max(0.0) as usize;
        let x1 = ((cx + 3.0 * r).max(0.0) as usize).min(DYE_WIDTH - 1);
        let y0 = (cy - 3.0 * r).max(0.0) as usize;
        let y1 = ((cy + 3.0 * r).max(0.0) as usize).min(DYE_HEIGHT - 1);
        let source = [color[0], color[1], color[2], heat];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let weight = (-(dx * dx + dy * dy) / (r * r)).exp() * amount;
                for (d, c) in self.dye[y * DYE_WIDTH + x].iter_mut().zip(source) {
                    *d = (*d + weight * c).min(DYE_LIMIT);
                }
            }
        }
    }

    fn project(&mut self) {
        for p in &mut self.pressure {
            *p *= PRESSURE_CARRY;
        }
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                self.divergence[i] = 0.5
                    * (self.velocity[i + 1][0] - self.velocity[i - 1][0]
                        + self.velocity[i + WIDTH][1]
                        - self.velocity[i - WIDTH][1]);
            }
        }
        let p = &mut self.pressure;
        for _ in 0..PRESSURE_SWEEPS {
            for color in 0..2 {
                for y in 1..HEIGHT - 1 {
                    let mut x = if (1 + y) % 2 == color { 1 } else { 2 };
                    while x < WIDTH - 1 {
                        let i = y * WIDTH + x;
                        let gauss = (p[i - 1] + p[i + 1] + p[i - WIDTH] + p[i + WIDTH]
                            - self.divergence[i])
                            * 0.25;
                        p[i] += SOR_OMEGA * (gauss - p[i]);
                        x += 2;
                    }
                }
            }
            // Zero normal pressure gradient at the vessel's walls.
            for x in 1..WIDTH - 1 {
                p[x] = p[WIDTH + x];
                p[(HEIGHT - 1) * WIDTH + x] = p[(HEIGHT - 2) * WIDTH + x];
            }
            for y in 0..HEIGHT {
                p[y * WIDTH] = p[y * WIDTH + 1];
                p[y * WIDTH + WIDTH - 1] = p[y * WIDTH + WIDTH - 2];
            }
        }
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                let v = &mut self.velocity[i];
                v[0] = (v[0] - 0.5 * (p[i + 1] - p[i - 1])).clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT);
                v[1] = (v[1] - 0.5 * (p[i + WIDTH] - p[i - WIDTH]))
                    .clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT);
            }
        }
        for x in 0..WIDTH {
            self.velocity[x] = [0.0; 2];
            self.velocity[(HEIGHT - 1) * WIDTH + x] = [0.0; 2];
        }
        for y in 0..HEIGHT {
            self.velocity[y * WIDTH] = [0.0; 2];
            self.velocity[y * WIDTH + WIDTH - 1] = [0.0; 2];
        }
    }

    /// Advances the musical envelopes by one fixed substep and returns the
    /// kick and spray strengths that fired on it (zero when none did).
    fn listen(&mut self, frame: &SceneFrame<'_>) -> (f32, f32) {
        let loud = frame
            .dynamics
            .map_or(frame.audio.rms * 1.8, |p| p.level(frame.audio.rms))
            .clamp(0.0, 1.0);
        let bass = EffectInputs::from_audio(0.0, frame.audio.rms, frame.audio.trails, 0.0, 0.0)
            .bass
            .clamp(0.0, 1.0);
        let m = &mut self.music;
        m.level = follow(m.level, loud, 0.06, 1.1);
        m.bass = follow(m.bass, bass, 0.05, 0.45);
        // Pressure is the *sustained* part: it needs seconds of bass and
        // loudness together, and lets go a little faster than it builds.
        m.pressure = follow(m.pressure, (m.bass * 1.6 * m.level).min(1.0), 3.2, 1.8);
        m.breath = follow(m.breath, 1.0 - m.level, 1.4, 0.5);
        m.kick *= (-STEP / 0.12).exp();
        m.spray *= (-STEP / 0.09).exp();
        if m.pressure > 0.45 {
            self.pressure_armed = true;
        } else if self.pressure_armed && m.pressure < 0.12 {
            self.pressure_armed = false;
            m.swirl = -m.swirl;
        }
        self.kick_refractory = (self.kick_refractory - STEP).max(0.0);
        self.spray_refractory = (self.spray_refractory - STEP).max(0.0);
        let kick = raw_kick(&frame.audio);
        let mut fired_kick = 0.0;
        if kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            self.kick_refractory = KICK_REFRACTORY;
            fired_kick = (kick / (3.0 * KICK_THRESHOLD)).clamp(0.4, 1.0);
            m.kick = m.kick.max(fired_kick);
            m.kicks += 1;
        }
        let flux = (frame.audio.spectral_flux * 6.0).clamp(0.0, 1.0);
        let mut fired_spray = 0.0;
        if fired_kick == 0.0 && flux >= SPRAY_THRESHOLD && self.spray_refractory <= 0.0 {
            self.spray_refractory = SPRAY_REFRACTORY;
            fired_spray = flux;
            m.spray = m.spray.max(flux);
            m.sprays += 1;
        }
        (fired_kick, fired_spray)
    }

    fn step(&mut self, frame: &SceneFrame<'_>) {
        let setting = |i| frame.setting(SceneId::FluidInk, i);
        let response = setting(control::RESPONSE);
        let flow = setting(control::FLOW);
        let radius = setting(control::RADIUS);
        let dt = STEP * flow;
        let (kick, spray) = self.listen(frame);
        let m = self.music;
        let t = self.clock;

        // Two slow ribbons: the ambient voice that keeps a quiet passage
        // alive. Thin sources pushed along their own path are stretched into
        // filaments by the shear; fat ones read as comets.
        let breath = 0.65 + 0.35 * (self.seconds * std::f32::consts::TAU / 6.0).sin();
        for j in 0..2 {
            let a = j as f32 * std::f32::consts::PI + self.phase;
            let speed = 0.105 + j as f32 * 0.031;
            let q = t * speed + a;
            let p = [0.5 + 0.3 * q.cos(), 0.5 + 0.27 * (q * 1.37 + a).sin()];
            let tangent = [-0.3 * q.sin(), 0.27 * 1.37 * (q * 1.37 + a).cos()];
            let norm = (tangent[0] * tangent[0] + tangent[1] * tangent[1])
                .sqrt()
                .max(1.0e-4);
            let push = 16.0 + 44.0 * response * m.level;
            let amount = (0.028 + 0.045 * response * m.level) * breath.max(m.level);
            self.splat(
                p,
                [tangent[0] / norm * push, tangent[1] / norm * push],
                hue_color(self.palette_hue(if j == 0 { -20.0 } else { 40.0 })),
                0.0,
                radius * 0.55,
                amount * flow,
            );
        }

        if kick > 0.0 && response > 0.0 {
            let k = m.kicks as f32;
            let angle = self.phase * 3.0 + k * GOLDEN_ANGLE;
            let p = [0.5 + 0.31 * angle.cos(), 0.5 + 0.29 * angle.sin()];
            // Aimed past the centre, twisting alternately, so bursts spiral.
            let twist = if m.kicks.is_multiple_of(2) {
                0.55
            } else {
                -0.55
            } + 0.25 * (k * 0.7).sin();
            let to_centre = [0.5 - p[0], 0.5 - p[1]];
            let n = (to_centre[0].powi(2) + to_centre[1].powi(2))
                .sqrt()
                .max(1.0e-4);
            let (s, c) = twist.sin_cos();
            let d = [
                (to_centre[0] * c - to_centre[1] * s) / n,
                (to_centre[0] * s + to_centre[1] * c) / n,
            ];
            // A kick in a quiet passage is a puff, not a punch: the detector
            // reads excursion, which a soft intro has as much of as a drop.
            let gain = response.min(2.0) * (0.3 + 0.7 * m.level);
            let power = 120.0 * (0.45 + 0.55 * kick) * (1.0 + 0.7 * m.pressure) * gain;
            let hue = self.palette_hue([-45.0, 15.0, 75.0][(m.kicks % 3) as usize]);
            let size = radius * (0.95 + 0.55 * kick) * (1.0 + 0.3 * m.pressure);
            self.splat(
                p,
                [d[0] * power, d[1] * power],
                hue_color(hue),
                0.0,
                size,
                (0.4 + 0.45 * kick) * gain.min(1.0),
            );
            // The heat is a core, not the whole cloud: the impulse winds it
            // into the leading ring, where it reads as the burst's bright edge.
            self.splat(
                p,
                [0.0, 0.0],
                [0.0; 3],
                1.0,
                size * 0.45,
                2.2 * kick * gain.min(1.0),
            );
            self.last_burst = p;
            self.last_burst_direction = d;
        }

        if spray > 0.0 && response > 0.0 {
            // A thin flick across the last burst, in the complementary hue:
            // small, hot, fast, gone in a breath.
            let d = self.last_burst_direction;
            let side = if m.sprays.is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let across = [-d[1] * side, d[0] * side];
            let p = [
                self.last_burst[0] + d[0] * 0.08,
                self.last_burst[1] + d[1] * 0.08,
            ];
            let gain = response.min(2.0);
            self.splat(
                p,
                [across[0] * 150.0 * gain, across[1] * 150.0 * gain],
                hue_color(self.palette_hue(180.0)),
                0.9 * spray,
                radius * 0.32,
                0.45 * spray * gain.min(1.0),
            );
        }

        // Sustained bass spins the vessel: solid-body rotation under a soft
        // falloff, divergence-free, so the projection keeps all of it.
        let spin = m.pressure * response * 13.0 * m.swirl;
        if spin.abs() > 1.0e-3 {
            let (cx, cy) = (WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5);
            let scale = 1.0 / (HEIGHT as f32 * 0.5);
            for y in 1..HEIGHT - 1 {
                for x in 1..WIDTH - 1 {
                    let dx = (x as f32 - cx) * scale;
                    let dy = (y as f32 - cy) * scale;
                    let falloff = (-(dx * dx + dy * dy) * 0.55).exp();
                    let v = &mut self.velocity[y * WIDTH + x];
                    v[0] -= dt * spin * dy * falloff;
                    v[1] += dt * spin * dx * falloff;
                }
            }
        }

        // Advect velocity through itself, then restore curl lost to
        // interpolation. Quiet thickens the water: eddies settle instead of
        // churning on with nothing driving them.
        let damping = (-(0.22 + 0.55 * m.breath) * dt).exp();
        {
            // One thread: 37k cells are cheaper than the spawns would be.
            let velocity = &self.velocity;
            rows_parallel(
                &mut self.velocity_next,
                WIDTH,
                1,
                |y, row: &mut [Velocity]| {
                    if y == 0 || y == HEIGHT - 1 {
                        row.fill([0.0; 2]);
                        return;
                    }
                    for x in 1..WIDTH - 1 {
                        let v = velocity[y * WIDTH + x];
                        let value = sample(velocity, x as f32 - dt * v[0], y as f32 - dt * v[1]);
                        row[x] = value.map(|v| v * damping);
                    }
                    row[0] = [0.0; 2];
                    row[WIDTH - 1] = [0.0; 2];
                },
            );
        }
        std::mem::swap(&mut self.velocity, &mut self.velocity_next);
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                self.curl[i] = 0.5
                    * (self.velocity[i + 1][1]
                        - self.velocity[i - 1][1]
                        - self.velocity[i + WIDTH][0]
                        + self.velocity[i - WIDTH][0]);
            }
        }
        // Transients sharpen the curls for a moment: a snare crinkles the ink.
        let curl_strength = setting(control::CURL) * (7.0 + 6.0 * m.spray * response.min(1.0));
        for y in 2..HEIGHT - 2 {
            for x in 2..WIDTH - 2 {
                let i = y * WIDTH + x;
                let nx = self.curl[i + 1].abs() - self.curl[i - 1].abs();
                let ny = self.curl[i + WIDTH].abs() - self.curl[i - WIDTH].abs();
                let scale = curl_strength * dt * self.curl[i] / (nx * nx + ny * ny + 1.0e-5).sqrt();
                self.velocity[i][0] =
                    (self.velocity[i][0] + ny * scale).clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT);
                self.velocity[i][1] =
                    (self.velocity[i][1] - nx * scale).clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT);
            }
        }
        self.project();

        // Ink lingers in a quiet passage; heat always cools on its own clock.
        let life = setting(control::LIFETIME) * (1.0 + 0.9 * m.breath);
        let fade = [
            (-std::f32::consts::LN_2 * STEP / life).exp(),
            (-std::f32::consts::LN_2 * STEP / life).exp(),
            (-std::f32::consts::LN_2 * STEP / life).exp(),
            (-std::f32::consts::LN_2 * STEP / HEAT_HALF_LIFE).exp(),
        ];
        self.advect_dye(dt, fade);
        self.clock += dt;
        self.seconds += STEP;
        self.steps += 1;
    }

    /// Midpoint backtrace followed by limited MacCormack correction. Plain
    /// bilinear transport loses a thin ribbon every frame; undoing its round
    /// trip error retains that ribbon without inventing negative pigment or
    /// overshooting the four source texels (which would ring at ink edges).
    ///
    /// The two passes are split by rows across threads. Each cell reads only
    /// the previous buffers, so the split changes nothing but the wall clock.
    fn advect_dye(&mut self, dt: f32, fade: [f32; 4]) {
        let velocity = &self.velocity;
        let dye = &self.dye;
        rows_parallel_pair(
            &mut self.dye_next,
            &mut self.backtrace,
            DYE_WIDTH,
            self.workers,
            |y, next: &mut [Dye], back: &mut [Velocity]| {
                for x in 0..DYE_WIDTH {
                    let (vx, vy) = (x as f32 * 0.5, y as f32 * 0.5);
                    let v = sample(velocity, vx, vy);
                    let mid = sample(velocity, vx - 0.5 * dt * v[0], vy - 0.5 * dt * v[1]);
                    let p = [x as f32 - 2.0 * dt * mid[0], y as f32 - 2.0 * dt * mid[1]];
                    back[x] = p;
                    next[x] = sample_grid(dye, p[0], p[1], DYE_WIDTH, DYE_HEIGHT);
                }
            },
        );
        let dye_next = &self.dye_next;
        let backtrace = &self.backtrace;
        rows_parallel(
            &mut self.dye_corrected,
            DYE_WIDTH,
            self.workers,
            |y, out: &mut [Dye]| {
                for (x, cell) in out.iter_mut().enumerate() {
                    let i = y * DYE_WIDTH + x;
                    let v = sample(velocity, x as f32 * 0.5, y as f32 * 0.5);
                    let reverse = sample_grid(
                        dye_next,
                        x as f32 + 2.0 * dt * v[0],
                        y as f32 + 2.0 * dt * v[1],
                        DYE_WIDTH,
                        DYE_HEIGHT,
                    );
                    let p = backtrace[i];
                    let x0 = (p[0].max(0.0) as usize).min(DYE_WIDTH - 2);
                    let y0 = (p[1].max(0.0) as usize).min(DYE_HEIGHT - 2);
                    let j = y0 * DYE_WIDTH + x0;
                    let (a, b, c, d) = (
                        dye[j],
                        dye[j + 1],
                        dye[j + DYE_WIDTH],
                        dye[j + DYE_WIDTH + 1],
                    );
                    *cell = std::array::from_fn(|ch| {
                        let lo = a[ch].min(b[ch]).min(c[ch]).min(d[ch]);
                        let hi = a[ch].max(b[ch]).max(c[ch]).max(d[ch]);
                        (dye_next[i][ch] + 0.5 * (dye[i][ch] - reverse[ch])).clamp(lo, hi)
                            * fade[ch]
                    });
                }
            },
        );
        std::mem::swap(&mut self.dye, &mut self.dye_corrected);
    }
}

/// One-pole follower with separate rise and fall time constants, advanced
/// by one fixed substep.
fn follow(current: f32, target: f32, rise: f32, fall: f32) -> f32 {
    let tau = if target > current { rise } else { fall };
    current + (target - current) * (1.0 - (-STEP / tau).exp())
}

/// The bass transient: mean positive excursion of the lowest quarter of the
/// bands over their trails (the shape `clawd::ClawdState::raw_kick` measured).
fn raw_kick(audio: &SceneAudioFrame<'_>) -> f32 {
    let count = audio.bands.len().min(audio.trails.len());
    if count == 0 {
        return 0.0;
    }
    let low = (count / 4).max(1);
    audio.bands[..low]
        .iter()
        .zip(&audio.trails[..low])
        .map(|(band, trail)| (band - trail).max(0.0))
        .sum::<f32>()
        / low as f32
}

/// Bands per worker in the shared queue. A worker the scheduler parks while
/// the machine is busy then holds back one small band, not a whole share, and
/// the others drain the rest of the queue.
const BANDS_PER_WORKER: usize = 4;

/// Calls `f(row_index, row)` for every row of `out`, on up to `workers`
/// threads (the caller's among them) pulling bands of rows from one queue.
fn rows_parallel<T: Send>(
    out: &mut [T],
    width: usize,
    workers: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) {
    if workers <= 1 {
        for (y, row) in out.chunks_mut(width).enumerate() {
            f(y, row);
        }
        return;
    }
    let height = out.len() / width;
    let band = height.div_ceil(workers * BANDS_PER_WORKER).max(1);
    let queue = std::sync::Mutex::new(out.chunks_mut(band * width).enumerate());
    let work = || loop {
        let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
        let Some((index, rows)) = next else { break };
        for (r, row) in rows.chunks_mut(width).enumerate() {
            f(index * band + r, row);
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..workers {
            let work = &work;
            spawn_small(scope, work);
        }
        work();
    });
}

/// Spawns a scoped worker on a 256 KiB stack. The solver spawns a few hundred
/// workers a second and none recurses; the default 2 MiB stack is mapped and
/// unmapped for nothing each time.
fn spawn_small<'scope, 'env>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    work: impl FnOnce() + Send + 'scope,
) {
    if std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn_scoped(scope, work)
        .is_err()
    {
        // `spawn_scoped` hands the closure back to nobody on failure, so a
        // refused thread is a lost band. Treat it as the fatal resource
        // exhaustion `scope.spawn` would have panicked on anyway.
        panic!("fluid_ink: could not spawn a solver worker");
    }
}

/// [`rows_parallel`] over two equally shaped outputs at once.
fn rows_parallel_pair<A: Send, B: Send>(
    a: &mut [A],
    b: &mut [B],
    width: usize,
    workers: usize,
    f: impl Fn(usize, &mut [A], &mut [B]) + Sync,
) {
    debug_assert_eq!(a.len(), b.len());
    let height = a.len() / width;
    let band = height.div_ceil(workers.max(1) * BANDS_PER_WORKER).max(1);
    let queue = std::sync::Mutex::new(
        a.chunks_mut(band * width)
            .zip(b.chunks_mut(band * width))
            .enumerate(),
    );
    let work = || loop {
        let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
        let Some((index, (rows_a, rows_b))) = next else {
            break;
        };
        for (r, (row_a, row_b)) in rows_a
            .chunks_mut(width)
            .zip(rows_b.chunks_mut(width))
            .enumerate()
        {
            f(index * band + r, row_a, row_b);
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..workers {
            let work = &work;
            spawn_small(scope, work);
        }
        work();
    });
}

/// Bilinear interpolation, clamped at closed boundaries.
fn sample<const N: usize>(field: &[[f32; N]], x: f32, y: f32) -> [f32; N] {
    sample_grid(field, x, y, WIDTH, HEIGHT)
}

fn sample_grid<const N: usize>(
    field: &[[f32; N]],
    x: f32,
    y: f32,
    width: usize,
    height: usize,
) -> [f32; N] {
    let x = x.clamp(0.0, (width - 1) as f32);
    let y = y.clamp(0.0, (height - 1) as f32);
    let x0 = (x as usize).min(width - 2);
    let y0 = (y as usize).min(height - 2);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let i = y0 * width + x0;
    std::array::from_fn(|c| {
        let top = field[i][c] + (field[i + 1][c] - field[i][c]) * fx;
        let bottom = field[i + width][c] + (field[i + width + 1][c] - field[i + width][c]) * fx;
        top + (bottom - top) * fy
    })
}

/// A saturated pigment at `hue` degrees. Not full saturation: a touch of
/// white in every pigment keeps two overlapping inks from cancelling to black
/// in any channel.
fn hue_color(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let floor = 0.08;
    [
        floor + (1.0 - floor) * r,
        floor + (1.0 - floor) * g,
        floor + (1.0 - floor) * b,
    ]
}

impl SceneState for FluidInkState {
    fn id(&self) -> SceneId {
        SceneId::FluidInk
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        self.accumulator += frame.delta_seconds.min(MAX_BANKED);
        while self.accumulator + 1.0e-7 >= STEP {
            self.step(frame);
            self.accumulator = (self.accumulator - STEP).max(0.0);
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::FluidInk,
        // 2: four dye channels (heat), the musical envelopes and SOR pressure.
        state_version: 2,
        make_state: |seed| Box::new(FluidInkState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;
    fn advance(state: &mut FluidInkState, settings: &SceneSettings, rms: f32, frames: usize) {
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = STEP;
        frame.audio.rms = rms;
        frame.audio.spectral_flux = rms * 0.15;
        for _ in 0..frames {
            state.update(&frame);
        }
    }

    /// A frame whose low bands stand `excursion` above their trails: a kick
    /// to [`raw_kick`], nothing at all to the flux-only `advance`.
    fn kick_frame<'a>(
        settings: &'a SceneSettings,
        bands: &'a [f32],
        trails: &'a [f32],
    ) -> SceneFrame<'a> {
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = STEP;
        frame.audio.rms = 0.5;
        frame.audio.bands = bands;
        frame.audio.trails = trails;
        frame
    }

    #[test]
    fn projection_removes_divergence_in_a_forced_field() {
        let mut s = FluidInkState::new(7);
        s.splat([0.45, 0.55], [70.0, -35.0], [0.0; 3], 0.0, 0.08, 0.0);
        let divergence = |s: &FluidInkState| -> f32 {
            let mut sum = 0.0;
            for y in 2..HEIGHT - 2 {
                for x in 2..WIDTH - 2 {
                    let i = y * WIDTH + x;
                    let d = s.velocity[i + 1][0] - s.velocity[i - 1][0] + s.velocity[i + WIDTH][1]
                        - s.velocity[i - WIDTH][1];
                    sum += d * d;
                }
            }
            sum
        };
        let before = divergence(&s);
        s.project();
        assert!(
            divergence(&s) < before * 0.5,
            "pressure must remove most of the compression: before={before}, after={}",
            divergence(&s)
        );
    }

    #[test]
    fn replay_is_seeded_audio_driven_and_bounded() {
        let settings = SceneSettings::new();
        let mut a = FluidInkState::new(9);
        let mut b = FluidInkState::new(9);
        let mut quiet = FluidInkState::new(9);
        advance(&mut a, &settings, 0.7, 90);
        advance(&mut b, &settings, 0.7, 90);
        advance(&mut quiet, &settings, 0.0, 90);
        assert_eq!(a.dye, b.dye);
        let difference: f32 = a
            .dye
            .iter()
            .zip(&quiet.dye)
            .flat_map(|(a, b)| a.iter().zip(b))
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            difference > 100.0,
            "audio must change the transported dye, got {difference}"
        );
        assert!(a
            .dye
            .iter()
            .flatten()
            .all(|d| d.is_finite() && (0.0..=DYE_LIMIT).contains(d)));
        assert!(a
            .velocity
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= VELOCITY_LIMIT));
        assert_ne!(FluidInkState::new(10).dye, FluidInkState::new(9).dye);
    }

    #[test]
    fn worker_count_cannot_change_a_cell() {
        let settings = SceneSettings::new();
        let mut one = FluidInkState::new(4);
        one.workers = 1;
        let mut many = FluidInkState::new(4);
        many.workers = 7;
        advance(&mut one, &settings, 0.6, 40);
        advance(&mut many, &settings, 0.6, 40);
        assert_eq!(one.dye, many.dye);
        assert_eq!(one.velocity, many.velocity);
    }

    #[test]
    fn a_kick_fires_a_hot_burst_that_cools() {
        let settings = SceneSettings::new();
        let quiet_bands = [0.2f32; 16];
        let kick_bands: Vec<f32> = (0..16).map(|i| if i < 4 { 0.45 } else { 0.2 }).collect();
        let mut s = FluidInkState::new(3);
        let heat = |s: &FluidInkState| s.dye.iter().map(|d| d[3]).sum::<f32>();
        assert_eq!(heat(&s), 0.0, "the wetted vessel starts cold");
        s.update(&kick_frame(&settings, &kick_bands, &quiet_bands));
        assert_eq!(s.music().kicks, 1);
        assert!(s.music().kick > 0.9);
        let hot = heat(&s);
        assert!(hot > 50.0, "a kick must lay down heat, got {hot}");
        // Held excursion is one push, not a drumroll.
        for _ in 0..5 {
            s.update(&kick_frame(&settings, &kick_bands, &quiet_bands));
        }
        assert_eq!(s.music().kicks, 1);
        for _ in 0..60 {
            s.update(&kick_frame(&settings, &quiet_bands, &quiet_bands));
        }
        assert!(
            heat(&s) < hot * 0.3,
            "heat must cool within a second: {} of {hot}",
            heat(&s)
        );
        assert!(s.music().kick < 0.01);
    }

    #[test]
    fn response_zero_ignores_kicks() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SceneId::FluidInk, control::RESPONSE, 0.0));
        let low = [0.2f32; 16];
        let high: Vec<f32> = (0..16).map(|i| if i < 4 { 0.45 } else { 0.2 }).collect();
        let mut s = FluidInkState::new(3);
        s.update(&kick_frame(&settings, &high, &low));
        assert_eq!(s.music().kicks, 1, "detection still listens");
        assert_eq!(s.dye.iter().map(|d| d[3]).sum::<f32>(), 0.0);
    }

    #[test]
    fn sustained_bass_builds_pressure_and_quiet_breathes() {
        let settings = SceneSettings::new();
        let trails = [0.9f32; 16];
        let mut loud = SceneFrame::idle(&settings);
        loud.delta_seconds = STEP;
        loud.audio.rms = 0.6;
        loud.audio.trails = &trails;
        loud.audio.bands = &trails;
        let mut s = FluidInkState::new(5);
        for _ in 0..60 {
            s.update(&loud);
        }
        let early = s.music().pressure;
        for _ in 0..300 {
            s.update(&loud);
        }
        let built = s.music().pressure;
        assert!(early < 0.4 && built > 0.7, "pressure {early} -> {built}");
        assert!(s.music().breath < 0.2);
        let swirl = s.music().swirl;
        let mut silent = SceneFrame::idle(&settings);
        silent.delta_seconds = STEP;
        for _ in 0..600 {
            s.update(&silent);
        }
        assert!(s.music().pressure < 0.12);
        assert!(s.music().breath > 0.8, "breath {}", s.music().breath);
        assert_eq!(s.music().swirl, -swirl, "a full release reverses the spin");
    }

    #[test]
    fn pause_and_drawless_replay_keep_the_fluid_history() {
        let settings = SceneSettings::new();
        let mut a = FluidInkState::new(1);
        let mut b = FluidInkState::new(1);
        advance(&mut a, &settings, 0.4, 30);
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 30.0;
        frame.audio.rms = 0.4;
        frame.audio.spectral_flux = 0.06;
        for _ in 0..15 {
            b.update(&frame);
        }
        assert_eq!(
            a.dye, b.dye,
            "30 and 60 Hz must take the same fixed substeps"
        );
        let before = a.dye.clone();
        for delta in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            frame.delta_seconds = delta;
            a.update(&frame);
            assert_eq!(a.dye, before);
        }
    }
}
