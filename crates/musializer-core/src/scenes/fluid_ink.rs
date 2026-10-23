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

use crate::project::caption_effects::EffectInputs;
use crate::scene::settings::index::fluid as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 144;
const CELLS: usize = WIDTH * HEIGHT;
/// Dye is twice as fine as velocity, so thin pigment folds survive the solve.
pub const DYE_WIDTH: usize = WIDTH * 2;
pub const DYE_HEIGHT: usize = HEIGHT * 2;
const DYE_CELLS: usize = DYE_WIDTH * DYE_HEIGHT;
const STEP: f32 = 1.0 / 60.0;
const PRESSURE_STEPS: usize = 32;
type Dye = [f32; 3];
type Velocity = [f32; 2];

pub struct FluidInkState {
    seed: u64,
    velocity: Vec<Velocity>,
    velocity_next: Vec<Velocity>,
    dye: Vec<Dye>,
    dye_next: Vec<Dye>,
    dye_corrected: Vec<Dye>,
    backtrace: Vec<Velocity>,
    pressure: Vec<f32>,
    pressure_next: Vec<f32>,
    divergence: Vec<f32>,
    curl: Vec<f32>,
    accumulator: f32,
    clock: f32,
    steps: u64,
}

impl FluidInkState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut state = Self {
            seed,
            velocity: vec![[0.0; 2]; CELLS],
            velocity_next: vec![[0.0; 2]; CELLS],
            dye: vec![[0.0; 3]; DYE_CELLS],
            dye_next: vec![[0.0; 3]; DYE_CELLS],
            dye_corrected: vec![[0.0; 3]; DYE_CELLS],
            backtrace: vec![[0.0; 2]; DYE_CELLS],
            pressure: vec![0.0; CELLS],
            pressure_next: vec![0.0; CELLS],
            divergence: vec![0.0; CELLS],
            curl: vec![0.0; CELLS],
            accumulator: 0.0,
            clock: 0.0,
            steps: 0,
        };
        // An already wetted canvas invites playing even before the first beat.
        // The seeded stream function supplies broad initial eddies.
        let phase = (seed % 997) as f32 * 0.017;
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let fx = x as f32;
                let fy = y as f32;
                state.velocity[y * WIDTH + x] = [
                    13.0 * (fy * 0.074 + phase).cos() + 7.0 * (fx * 0.11 + fy * 0.13).cos(),
                    -11.0 * (fx * 0.063 - phase).cos() - 5.9 * (fx * 0.11 + fy * 0.13).cos(),
                ];
            }
        }
        for i in 0..5 {
            let a = i as f32 * 1.2566 + phase;
            state.splat(
                [0.5 + 0.27 * a.cos(), 0.5 + 0.28 * a.sin()],
                [25.0 * a.sin(), -25.0 * a.cos()],
                palette(i as f32 * 0.23),
                0.065,
                1.0,
            );
        }
        state.project();
        state
    }

    #[must_use]
    pub fn dye(&self) -> &[Dye] {
        &self.dye
    }
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    fn splat(&mut self, position: [f32; 2], force: Velocity, color: Dye, radius: f32, amount: f32) {
        // Radius is relative to the short axis: circles in the 16:9 simulation.
        let r = radius * HEIGHT as f32;
        let cx = position[0] * WIDTH as f32;
        let cy = position[1] * HEIGHT as f32;
        let x0 = (cx - 3.0 * r).max(1.0) as usize;
        let x1 = ((cx + 3.0 * r) as usize).min(WIDTH - 2);
        let y0 = (cy - 3.0 * r).max(1.0) as usize;
        let y1 = ((cy + 3.0 * r) as usize).min(HEIGHT - 2);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let weight = (-(dx * dx + dy * dy) / (r * r)).exp() * amount;
                let i = y * WIDTH + x;
                for (v, f) in self.velocity[i].iter_mut().zip(force) {
                    *v = (*v + weight * f).clamp(-160.0, 160.0);
                }
            }
        }
        // Pigment lives on its own finer grid; the force radius is unchanged.
        let (r, cx, cy) = (r * 2.0, cx * 2.0, cy * 2.0);
        let x0 = (cx - 3.0 * r).max(0.0) as usize;
        let x1 = ((cx + 3.0 * r) as usize).min(DYE_WIDTH - 1);
        let y0 = (cy - 3.0 * r).max(0.0) as usize;
        let y1 = ((cy + 3.0 * r) as usize).min(DYE_HEIGHT - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let weight = (-(dx * dx + dy * dy) / (r * r)).exp() * amount;
                for (d, c) in self.dye[y * DYE_WIDTH + x].iter_mut().zip(color) {
                    *d = (*d + weight * c).min(5.0);
                }
            }
        }
    }

    fn project(&mut self) {
        self.pressure.fill(0.0);
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                self.divergence[i] = 0.5
                    * (self.velocity[i + 1][0] - self.velocity[i - 1][0]
                        + self.velocity[i + WIDTH][1]
                        - self.velocity[i - WIDTH][1]);
            }
        }
        for _ in 0..PRESSURE_STEPS {
            for y in 1..HEIGHT - 1 {
                for x in 1..WIDTH - 1 {
                    let i = y * WIDTH + x;
                    self.pressure_next[i] = (self.pressure[i - 1]
                        + self.pressure[i + 1]
                        + self.pressure[i - WIDTH]
                        + self.pressure[i + WIDTH]
                        - self.divergence[i])
                        * 0.25;
                }
            }
            // Zero normal pressure gradient at the vessel's walls.
            for x in 1..WIDTH - 1 {
                self.pressure_next[x] = self.pressure_next[WIDTH + x];
                self.pressure_next[(HEIGHT - 1) * WIDTH + x] =
                    self.pressure_next[(HEIGHT - 2) * WIDTH + x];
            }
            for y in 0..HEIGHT {
                self.pressure_next[y * WIDTH] = self.pressure_next[y * WIDTH + 1];
                self.pressure_next[y * WIDTH + WIDTH - 1] =
                    self.pressure_next[y * WIDTH + WIDTH - 2];
            }
            std::mem::swap(&mut self.pressure, &mut self.pressure_next);
        }
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                self.velocity[i][0] -= 0.5 * (self.pressure[i + 1] - self.pressure[i - 1]);
                self.velocity[i][1] -= 0.5 * (self.pressure[i + WIDTH] - self.pressure[i - WIDTH]);
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

    fn step(&mut self, frame: &SceneFrame<'_>) {
        let setting = |i| frame.setting(SceneId::FluidInk, i);
        let response = setting(control::RESPONSE);
        let loud = frame
            .dynamics
            .map_or(frame.audio.rms * 1.8, |p| p.level(frame.audio.rms))
            .clamp(0.0, 1.0);
        let bass = EffectInputs::from_audio(0.0, frame.audio.rms, frame.audio.trails, 0.0, 0.0)
            .bass
            .clamp(0.0, 1.0);
        let hit = (frame.audio.spectral_flux * 7.0).clamp(0.0, 1.0);
        let flow = setting(control::FLOW);
        let dt = STEP * flow;
        let phase = (self.seed % 997) as f32 * 0.017;
        let t = self.clock;
        // Three moving jets: continuous narrow ink ribbons plus musical pushes.
        // Forces, not arbitrary screen displacement, are injected into the fluid.
        for j in 0..3 {
            let a = j as f32 * 2.0944 + phase;
            let speed = 0.24 + j as f32 * 0.065;
            let q = t * speed + a;
            let p = [0.5 + 0.31 * q.cos(), 0.5 + 0.32 * (q * 1.37 + a).sin()];
            let force = [-(q * 1.7 + a).sin() * 24.0, (q * 1.3 - a).cos() * 24.0];
            let musical = match j {
                0 => bass,
                1 => loud,
                _ => hit,
            };
            let strength = (0.16 + response * musical * 0.7) * flow;
            let radius = setting(control::RADIUS) * (0.65 + 0.6 * bass * response);
            self.splat(
                p,
                force,
                palette(j as f32 * 0.31 + t * 0.025),
                radius,
                strength,
            );
        }
        // Advect velocity through itself, then restore curl lost to interpolation.
        for y in 1..HEIGHT - 1 {
            for x in 1..WIDTH - 1 {
                let i = y * WIDTH + x;
                let v = self.velocity[i];
                let value = sample(&self.velocity, x as f32 - dt * v[0], y as f32 - dt * v[1]);
                self.velocity_next[i] = value.map(|v| v * (-0.22 * dt).exp());
            }
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
        let curl_strength = setting(control::CURL) * 7.0;
        for y in 2..HEIGHT - 2 {
            for x in 2..WIDTH - 2 {
                let i = y * WIDTH + x;
                let nx = self.curl[i + 1].abs() - self.curl[i - 1].abs();
                let ny = self.curl[i + WIDTH].abs() - self.curl[i - WIDTH].abs();
                let scale = curl_strength * dt * self.curl[i] / (nx * nx + ny * ny + 1.0e-5).sqrt();
                self.velocity[i][0] = (self.velocity[i][0] + ny * scale).clamp(-160.0, 160.0);
                self.velocity[i][1] = (self.velocity[i][1] - nx * scale).clamp(-160.0, 160.0);
            }
        }
        self.project();
        let fade = (-std::f32::consts::LN_2 * STEP / setting(control::LIFETIME)).exp();
        // Midpoint backtrace followed by limited MacCormack correction. Plain
        // bilinear transport loses a thin ribbon every frame; undoing its round
        // trip error retains that ribbon without inventing negative pigment or
        // overshooting the four source texels (which would ring at ink edges).
        for y in 0..DYE_HEIGHT {
            for x in 0..DYE_WIDTH {
                let i = y * DYE_WIDTH + x;
                let (vx, vy) = (x as f32 * 0.5, y as f32 * 0.5);
                let v = sample(&self.velocity, vx, vy);
                let mid = sample(&self.velocity, vx - 0.5 * dt * v[0], vy - 0.5 * dt * v[1]);
                let p = [x as f32 - 2.0 * dt * mid[0], y as f32 - 2.0 * dt * mid[1]];
                self.backtrace[i] = p;
                self.dye_next[i] = sample_grid(&self.dye, p[0], p[1], DYE_WIDTH, DYE_HEIGHT);
            }
        }
        for y in 0..DYE_HEIGHT {
            for x in 0..DYE_WIDTH {
                let i = y * DYE_WIDTH + x;
                let v = sample(&self.velocity, x as f32 * 0.5, y as f32 * 0.5);
                let reverse = sample_grid(
                    &self.dye_next,
                    x as f32 + 2.0 * dt * v[0],
                    y as f32 + 2.0 * dt * v[1],
                    DYE_WIDTH,
                    DYE_HEIGHT,
                );
                let p = self.backtrace[i];
                let x0 = (p[0].max(0.0) as usize).min(DYE_WIDTH - 2);
                let y0 = (p[1].max(0.0) as usize).min(DYE_HEIGHT - 2);
                let j = y0 * DYE_WIDTH + x0;
                for (c, back) in reverse.iter().enumerate() {
                    let neighbors = [
                        self.dye[j][c],
                        self.dye[j + 1][c],
                        self.dye[j + DYE_WIDTH][c],
                        self.dye[j + DYE_WIDTH + 1][c],
                    ];
                    let lo = neighbors.iter().copied().fold(f32::INFINITY, f32::min);
                    let hi = neighbors.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    self.dye_corrected[i][c] =
                        (self.dye_next[i][c] + 0.5 * (self.dye[i][c] - back)).clamp(lo, hi) * fade;
                }
            }
        }
        std::mem::swap(&mut self.dye, &mut self.dye_corrected);
        self.clock += dt;
        self.steps += 1;
    }
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

// Lime, turquoise and orchid: chromatic separation survives mixed dye.
fn palette(phase: f32) -> Dye {
    let colors = [
        [0.65, 1.0, 0.025],
        [0.02, 0.8, 1.0],
        [0.6, 0.04, 1.0],
        [1.0, 0.18, 0.12],
    ];
    let p = phase.rem_euclid(1.0) * 4.0;
    let i = p as usize;
    std::array::from_fn(|c| colors[i][c] + (colors[(i + 1) % 4][c] - colors[i][c]) * (p - i as f32))
}

impl SceneState for FluidInkState {
    fn id(&self) -> SceneId {
        SceneId::FluidInk
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        self.accumulator += frame.delta_seconds.min(0.25);
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
        state_version: 1,
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
    #[test]
    fn projection_removes_divergence_in_a_forced_field() {
        let mut s = FluidInkState::new(7);
        s.splat([0.45, 0.55], [70.0, -35.0], [0.0; 3], 0.08, 1.0);
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
            divergence(&s) < before * 0.8,
            "pressure must measurably remove compression: before={before}, after={}",
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
            .all(|d| d.is_finite() && (0.0..=5.0).contains(d)));
        assert!(a
            .velocity
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 170.0));
        assert_ne!(FluidInkState::new(10).dye, FluidInkState::new(9).dye);
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
