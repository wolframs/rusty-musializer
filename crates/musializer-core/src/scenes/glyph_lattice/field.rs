//! The lattice: which cells a frame lights, how hot they run and how they
//! bloom.
//!
//! The honeycomb is fixed to the frame, not to the pixels: its pitch is a
//! fraction of the frame's short side (`density` cells across it), so a
//! preview, a 1080p export and a supersampled one light the same cells and
//! only the glyphs get sharper. Rows are offset by half a cell, which is what
//! makes it a honeycomb.
//!
//! Each frame the solids, their ghosts and the line are projected and drawn
//! as thick strokes into the cells: brightness falls off from the stroke's
//! centre line, so a cell's tier — edge, body, core — is a measured distance,
//! not a texture. Strokes are summed across and maxed within, so the joints
//! of one stroke stay even while two strokes crossing pool into heat.
//! Pulses ride a stroke as a path distance from their vertex. The bloom is the
//! lit field blurred on the lattice itself, which keeps it a property of the
//! cells rather than of the screen.
//!
//! The result is packed for the GPU two texels per cell: (lit, heat, ghost,
//! wash) in the left half of the atlas and (bloom, warmth) in the right.

use super::shapes::{self, Form, Rotation, Wire};
use super::{strokes, GlyphLatticeState, ASSEMBLE_SECONDS, FADE_SECONDS, RIPPLES};
use super::{RIPPLE_SECONDS, RIPPLE_SPEED};
use crate::scene::settings::index::glyphs as control;
use crate::scene::{SceneFrame, SceneId};

/// The GPU atlas the packed field is uploaded into, allocated once.
pub const ATLAS_WIDTH: usize = 1024;
pub const ATLAS_HEIGHT: usize = 512;
/// The largest lattice the atlas holds: two texels per cell across.
pub const MAX_COLS: usize = ATLAS_WIDTH / 2 - 1;
pub const MAX_ROWS: usize = ATLAS_HEIGHT - 1;
/// Row spacing in pitches: sqrt(3) / 2.
pub const ROW: f32 = 0.866_025_4;
/// A radius-one solid at `size` 1 spans this fraction of the short side.
const RADIUS: f32 = 0.32;
/// Camera distance in solid radii; close enough that near edges read nearer.
const CAMERA: f32 = 4.0;
/// Pulses (and the assembling front) one instance can carry.
const WAVES: usize = super::MAX_PULSES + 1;

/// The honeycomb for one frame shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lattice {
    pub half_cols: i32,
    pub half_rows: i32,
    /// Horizontal cell pitch in short sides.
    pub pitch: f32,
    /// The frame's width and height in short sides.
    pub frame: [f32; 2],
}

impl Lattice {
    /// The lattice for a `width` by `height` frame with `density` cells across
    /// its short side. Only the aspect matters, so every resolution of one
    /// shape lights the same cells. A shape too long for the atlas at this
    /// density coarsens the pitch until it fits.
    #[must_use]
    pub fn new(width: f32, height: f32, density: f32) -> Self {
        let w = if width.is_finite() && width >= 1.0 {
            width
        } else {
            1.0
        };
        let h = if height.is_finite() && height >= 1.0 {
            height
        } else {
            1.0
        };
        let short = w.min(h);
        let frame = [w / short, h / short];
        let density = if density.is_finite() {
            density.clamp(16.0, 400.0)
        } else {
            120.0
        };
        let max_half_cols = (MAX_COLS as i32 - 1) / 2;
        let max_half_rows = (MAX_ROWS as i32 - 1) / 2;
        let pitch = (1.0 / density)
            .max(frame[0] * 0.5 / (max_half_cols - 1) as f32)
            .max(frame[1] * 0.5 / ((max_half_rows - 1) as f32 * ROW));
        let half_cols = ((frame[0] * 0.5 / pitch).ceil() as i32 + 1).min(max_half_cols);
        let half_rows = ((frame[1] * 0.5 / (pitch * ROW)).ceil() as i32 + 1).min(max_half_rows);
        Self {
            half_cols,
            half_rows,
            pitch,
            frame,
        }
    }

    #[must_use]
    pub fn cols(&self) -> usize {
        (2 * self.half_cols + 1) as usize
    }

    #[must_use]
    pub fn rows(&self) -> usize {
        (2 * self.half_rows + 1) as usize
    }

    /// A cell's centre in short sides from the frame's centre, y down.
    #[must_use]
    pub fn centre(&self, col: usize, row: usize) -> [f32; 2] {
        let c = col as i32 - self.half_cols;
        let r = row as i32 - self.half_rows;
        [
            (c as f32 + 0.5 * r.rem_euclid(2) as f32) * self.pitch,
            r as f32 * self.pitch * ROW,
        ]
    }

    /// The cell nearest a point, as (col, row), if it is on the lattice.
    #[must_use]
    pub fn nearest(&self, point: [f32; 2]) -> Option<(usize, usize)> {
        let rf = point[1] / (self.pitch * ROW);
        let mut best: Option<(f32, i32, i32)> = None;
        for r in [rf.floor() as i32, rf.floor() as i32 + 1] {
            let offset = 0.5 * r.rem_euclid(2) as f32;
            let c = (point[0] / self.pitch - offset).round() as i32;
            let x = (c as f32 + offset) * self.pitch;
            let y = r as f32 * self.pitch * ROW;
            let d = (point[0] - x).powi(2) + (point[1] - y).powi(2);
            if best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, c, r));
            }
        }
        let (_, c, r) = best?;
        let (col, row) = (c + self.half_cols, r + self.half_rows);
        (col >= 0 && row >= 0 && (col as usize) < self.cols() && (row as usize) < self.rows())
            .then_some((col as usize, row as usize))
    }
}

/// A pulse as the rasterizer sees it: a bright head at `front` (in the
/// stroke's path units) and a tail behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wave {
    front: f32,
    width: f32,
    tail: f32,
    strength: f32,
    /// A front spreading through the whole graph (the assembly), whose path
    /// along an edge is `min(da + t, db + 1 - t)`; otherwise a path that runs
    /// each edge one way, `da` to `db` (a comet's walk, the line's sweep).
    graph: bool,
}

/// A path endpoint no wave ever reaches: the edge is not on this walk.
const OFF_PATH: f32 = 1.0e6;

impl Wave {
    fn at(&self, path: f32) -> f32 {
        let x = path - self.front;
        let shape = if x > 0.0 {
            (-(x / self.width).powi(2)).exp()
        } else {
            (x / self.tail).exp()
        };
        shape * self.strength
    }
}

/// One drawn piece of a stroke, already projected.
#[derive(Clone, Copy, Debug)]
struct Seg {
    a: [f32; 2],
    b: [f32; 2],
    /// Brightness at each end (depth and band).
    ka: f32,
    kb: f32,
    /// Half width at each end, in short sides.
    wa: f32,
    wb: f32,
    /// The piece's ends as parameters along its whole edge.
    ta: f32,
    tb: f32,
    /// Path distance of the edge's two ends along each wave's path.
    paths: [[f32; 2]; WAVES],
    /// Heat the whole piece carries at its core, besides its waves: a word
    /// igniting, or warming on a kick.
    heat: f32,
}

impl Seg {
    fn path(&self, wave: usize, graph: bool, t_edge: f32) -> f32 {
        let [da, db] = self.paths[wave];
        if graph {
            (da + t_edge).min(db + 1.0 - t_edge)
        } else {
            da + (db - da) * t_edge
        }
    }
}

/// Which layer a drawing goes to, and how strongly.
#[derive(Clone, Copy, Debug)]
struct Weights {
    main: f32,
    ghost: f32,
}

/// What a frame's lattice held: the evidence the `glyphs:` report line prints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldReport {
    pub form: &'static str,
    pub auto: bool,
    /// How far the current solid has assembled, `0..=1`.
    pub assembled: f32,
    /// The solid exhaling into the ghosts, if one is.
    pub previous: Option<&'static str>,
    pub cols: usize,
    pub rows: usize,
    /// Lit cells by tier: edge (dim), body (mid), core (bright), and hot.
    pub dim: usize,
    pub mid: usize,
    pub bright: usize,
    pub hot: usize,
    pub ghost: usize,
    /// Cells a ripple is washing over.
    pub washed: usize,
    pub waves: usize,
    pub echoes: usize,
    pub giants: usize,
    /// The line being set, its strokes drawn and in all, and whether its cue
    /// is the one playing.
    pub line: Option<String>,
    pub strokes_drawn: usize,
    pub strokes_total: usize,
    pub line_live: bool,
    /// Each live ripple's radius in short sides, newest last; for the shader,
    /// which turns a glyph over as a ripple passes it.
    pub ripple_fronts: Vec<f32>,
    pub peak_bloom: f32,
}

/// The lattice's buffers, kept between frames so a frame allocates nothing.
#[derive(Clone, Debug)]
pub struct Field {
    lattice: Lattice,
    lit: Vec<f32>,
    heat: Vec<f32>,
    ghost: Vec<f32>,
    wash: Vec<f32>,
    bloom: Vec<f32>,
    warm: Vec<f32>,
    /// The wide bloom: light pooling where strokes cross and pulses run.
    pool: Vec<f32>,
    /// Extra halo source the line's strokes write: letters are thinner than
    /// a solid's edges and would otherwise throw less glow.
    halo: Vec<f32>,
    /// How strongly the strokes being drawn write `halo` (the line's only).
    halo_gain: f32,
    sum: Vec<f32>,
    max: Vec<f32>,
    pulse: Vec<f32>,
    cross: Vec<f32>,
    group_value: Vec<f32>,
    group_heat: Vec<f32>,
    touched: Vec<u32>,
    scratch: Vec<f32>,
    /// The packed atlas region: `2 * cols` texels by `rows`, RGBA8.
    pub rgba: Vec<u8>,
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

impl Field {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lattice: Lattice::new(16.0, 9.0, 120.0),
            lit: Vec::new(),
            heat: Vec::new(),
            ghost: Vec::new(),
            wash: Vec::new(),
            bloom: Vec::new(),
            warm: Vec::new(),
            pool: Vec::new(),
            halo: Vec::new(),
            halo_gain: 0.0,
            sum: Vec::new(),
            max: Vec::new(),
            pulse: Vec::new(),
            cross: Vec::new(),
            group_value: Vec::new(),
            group_heat: Vec::new(),
            touched: Vec::new(),
            scratch: Vec::new(),
            rgba: Vec::new(),
        }
    }

    #[must_use]
    pub fn lattice(&self) -> Lattice {
        self.lattice
    }

    /// Lit level of each cell, row-major, `0..=1`.
    #[must_use]
    pub fn lit(&self) -> &[f32] {
        &self.lit
    }

    #[must_use]
    pub fn heat(&self) -> &[f32] {
        &self.heat
    }

    #[must_use]
    pub fn ghost(&self) -> &[f32] {
        &self.ghost
    }

    #[must_use]
    pub fn wash(&self) -> &[f32] {
        &self.wash
    }

    #[must_use]
    pub fn bloom(&self) -> &[f32] {
        &self.bloom
    }

    fn reset(&mut self, lattice: Lattice) {
        self.lattice = lattice;
        let cells = lattice.cols() * lattice.rows();
        for buffer in [
            &mut self.lit,
            &mut self.heat,
            &mut self.ghost,
            &mut self.wash,
            &mut self.bloom,
            &mut self.warm,
            &mut self.pool,
            &mut self.halo,
            &mut self.sum,
            &mut self.max,
            &mut self.pulse,
            &mut self.cross,
            &mut self.group_value,
            &mut self.group_heat,
            &mut self.scratch,
        ] {
            buffer.clear();
            buffer.resize(cells, 0.0);
        }
        self.touched.clear();
    }

    /// Draws one piece into the current group (main layer) and the ghosts.
    fn splat(&mut self, seg: &Seg, waves: &[Wave], weights: Weights) {
        let lattice = self.lattice;
        let cols = lattice.cols();
        let reach = seg.wa.max(seg.wb);
        let row_h = lattice.pitch * ROW;
        let (y0, y1) = (
            seg.a[1].min(seg.b[1]) - reach,
            seg.a[1].max(seg.b[1]) + reach,
        );
        let r_lo = ((y0 / row_h).floor() as i32).max(-lattice.half_rows);
        let r_hi = ((y1 / row_h).ceil() as i32).min(lattice.half_rows);
        let d = [seg.b[0] - seg.a[0], seg.b[1] - seg.a[1]];
        let len2 = d[0] * d[0] + d[1] * d[1];
        for r in r_lo..=r_hi {
            let y = r as f32 * row_h;
            // The stretch of this row the capsule can reach.
            let (t0, t1) = if d[1].abs() < 1.0e-6 {
                if (y - seg.a[1]).abs() > reach {
                    continue;
                }
                (0.0, 1.0)
            } else {
                let ta = (y - reach - seg.a[1]) / d[1];
                let tb = (y + reach - seg.a[1]) / d[1];
                let (lo, hi) = (ta.min(tb).max(0.0), ta.max(tb).min(1.0));
                if lo > hi {
                    continue;
                }
                (lo, hi)
            };
            let xa = seg.a[0] + d[0] * t0;
            let xb = seg.a[0] + d[0] * t1;
            let offset = 0.5 * r.rem_euclid(2) as f32;
            let c_lo = (((xa.min(xb) - reach) / lattice.pitch - offset).floor() as i32)
                .max(-lattice.half_cols);
            let c_hi = (((xa.max(xb) + reach) / lattice.pitch - offset).ceil() as i32)
                .min(lattice.half_cols);
            let row = (r + lattice.half_rows) as usize;
            for c in c_lo..=c_hi {
                let p = [(c as f32 + offset) * lattice.pitch, y];
                let t = if len2 > 1.0e-12 {
                    (((p[0] - seg.a[0]) * d[0] + (p[1] - seg.a[1]) * d[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = [seg.a[0] + d[0] * t, seg.a[1] + d[1] * t];
                let dist = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                let width = seg.wa + (seg.wb - seg.wa) * t;
                if dist >= width {
                    continue;
                }
                let u = dist / width;
                let profile = 1.0 - smoothstep(0.38, 1.0, u);
                let k = seg.ka + (seg.kb - seg.ka) * t;
                let value = profile * k;
                let index = row * cols + (c + lattice.half_cols) as usize;
                if weights.ghost > 0.0 {
                    let g = value * weights.ghost;
                    if g > self.ghost[index] {
                        self.ghost[index] = g;
                    }
                }
                if weights.main <= 0.0 {
                    continue;
                }
                let main = value * weights.main;
                if self.halo_gain > 0.0 {
                    let h = main * self.halo_gain;
                    if h > self.halo[index] {
                        self.halo[index] = h;
                    }
                }
                let mut hot = seg.heat;
                if !waves.is_empty() {
                    let te = seg.ta + (seg.tb - seg.ta) * t;
                    for (i, wave) in waves.iter().enumerate() {
                        hot = hot.max(wave.at(seg.path(i, wave.graph, te)));
                    }
                }
                if hot > 0.0 {
                    hot *= (1.0 - smoothstep(0.3, 0.85, u)) * weights.main;
                }
                if self.group_value[index] == 0.0 && self.group_heat[index] == 0.0 {
                    self.touched.push(index as u32);
                }
                if main > self.group_value[index] {
                    self.group_value[index] = main.max(1.0e-6);
                }
                if hot > self.group_heat[index] {
                    self.group_heat[index] = hot;
                }
            }
        }
    }

    /// Ends a stroke: its cells join the sums, and two strokes over one cell
    /// start to read as a crossing.
    fn flush(&mut self) {
        for &index in &self.touched {
            let i = index as usize;
            let value = self.group_value[i];
            // Squares, so a stroke's soft edge adds little to a crossing.
            self.sum[i] += value * value;
            if value > self.max[i] {
                self.max[i] = value;
            }
            if self.group_heat[i] > self.pulse[i] {
                self.pulse[i] = self.group_heat[i];
            }
            self.group_value[i] = 0.0;
            self.group_heat[i] = 0.0;
        }
        self.touched.clear();
    }

    /// A soft spot of heat, for a knot crossing itself.
    fn heat_spot(&mut self, at: [f32; 2], radius: f32, strength: f32) {
        let lattice = self.lattice;
        let cols = lattice.cols();
        let row_h = lattice.pitch * ROW;
        let reach = radius * 1.8;
        let r_lo = (((at[1] - reach) / row_h).floor() as i32).max(-lattice.half_rows);
        let r_hi = (((at[1] + reach) / row_h).ceil() as i32).min(lattice.half_rows);
        for r in r_lo..=r_hi {
            let offset = 0.5 * r.rem_euclid(2) as f32;
            let c_lo =
                (((at[0] - reach) / lattice.pitch - offset).floor() as i32).max(-lattice.half_cols);
            let c_hi =
                (((at[0] + reach) / lattice.pitch - offset).ceil() as i32).min(lattice.half_cols);
            for c in c_lo..=c_hi {
                let p = [(c as f32 + offset) * lattice.pitch, r as f32 * row_h];
                let d2 = (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2);
                let value = strength * (-d2 / (radius * radius)).exp();
                let index =
                    (r + lattice.half_rows) as usize * cols + (c + lattice.half_cols) as usize;
                if value > self.cross[index] {
                    self.cross[index] = value;
                }
            }
        }
    }

    /// Two passes of a box blur of `radius` cells, from `source` into
    /// `target`, on the lattice's rows and columns.
    fn blur(&mut self, which: Blur, radius: usize) {
        let (cols, rows) = (self.lattice.cols(), self.lattice.rows());
        let source = match which {
            Blur::Bloom => std::mem::take(&mut self.bloom),
            Blur::Warm => std::mem::take(&mut self.warm),
            Blur::Pool => std::mem::take(&mut self.pool),
        };
        let mut a = source;
        let mut b = std::mem::take(&mut self.scratch);
        let window = (2 * radius + 1) as f32;
        for _ in 0..2 {
            for row in 0..rows {
                let line = &a[row * cols..(row + 1) * cols];
                let out = &mut b[row * cols..(row + 1) * cols];
                let mut acc: f32 = line[..radius.min(cols)].iter().sum();
                for col in 0..cols {
                    if col + radius < cols {
                        acc += line[col + radius];
                    }
                    out[col] = acc / window;
                    if col >= radius {
                        acc -= line[col - radius];
                    }
                }
            }
            for col in 0..cols {
                let mut acc: f32 = (0..radius.min(rows)).map(|row| b[row * cols + col]).sum();
                for row in 0..rows {
                    if row + radius < rows {
                        acc += b[(row + radius) * cols + col];
                    }
                    a[row * cols + col] = acc / window;
                    if row >= radius {
                        acc -= b[(row - radius) * cols + col];
                    }
                }
            }
        }
        self.scratch = b;
        match which {
            Blur::Bloom => self.bloom = a,
            Blur::Warm => self.warm = a,
            Blur::Pool => self.pool = a,
        }
    }
}

#[derive(Clone, Copy)]
enum Blur {
    Bloom,
    Warm,
    Pool,
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix64(value: u64) -> u64 {
    super::mix64(value)
}

/// Where and how one solid is drawn.
#[derive(Clone, Copy, Debug)]
struct Pose {
    rotation: Rotation,
    fourth: f32,
    scale: f32,
    offset: [f32; 2],
}

impl Pose {
    /// Screen position (short sides from the centre, y down), depth `-1..1`
    /// toward the camera, and the perspective factor.
    fn project(&self, point: [f32; 4], form: Form) -> ([f32; 2], f32, f32) {
        let p3 = if form == Form::Tesseract {
            shapes::fold_4d(point, self.fourth)
        } else {
            [point[0], point[1], point[2]]
        };
        let p = shapes::rotate(&self.rotation, p3);
        let persp = CAMERA / (CAMERA - p[2]);
        (
            [
                self.offset[0] + p[0] * persp * RADIUS * self.scale,
                self.offset[1] - p[1] * persp * RADIUS * self.scale,
            ],
            p[2].clamp(-1.0, 1.0),
            persp,
        )
    }
}

/// Everything a frame's drawing reads from the settings and the state.
struct Look {
    response: f32,
    level: f32,
    bass: f32,
    /// Half width of a solid's stroke, in short sides.
    stroke: f32,
    ghosts: f32,
    size: f32,
}

fn band_at(state: &GlyphLatticeState, u: f32) -> f32 {
    let bands = state.bands();
    let x = u.clamp(0.0, 1.0) * (bands.len() - 1) as f32;
    let i = (x.floor() as usize).min(bands.len() - 2);
    let f = x - i as f32;
    bands[i] * (1.0 - f) + bands[i + 1] * f
}

impl Field {
    /// Draws one solid. `waves` light it; `assemble` reveals it outward from
    /// a vertex. Returns the projected chain for the knot's crossings.
    #[allow(clippy::too_many_arguments)]
    fn draw_solid(
        &mut self,
        state: &GlyphLatticeState,
        look: &Look,
        form: Form,
        wire: &Wire,
        pose: &Pose,
        weights: Weights,
        width: f32,
        waves: &[(Wave, Vec<[f32; 2]>)],
        assemble: Option<&[u16]>,
        assemble_front: f32,
    ) -> Vec<[f32; 2]> {
        let projected: Vec<([f32; 2], f32, f32)> = wire
            .points
            .iter()
            .map(|point| pose.project(*point, form))
            .collect();
        let rb = look.response.min(1.0);
        let wave_list: Vec<Wave> = waves.iter().map(|(wave, _)| *wave).collect();
        let edges = wire.edges.len().max(1);
        for stroke in &wire.strokes {
            for edge_index in stroke.clone() {
                let [ia, ib] = wire.edges[edge_index];
                let (ia, ib) = (usize::from(ia), usize::from(ib));
                let (mut t0, mut t1) = (0.0f32, 1.0f32);
                if let Some(hops) = assemble {
                    let (ha, hb) = (f32::from(hops[ia]), f32::from(hops[ib]));
                    let near = ha.min(hb);
                    let reveal = (assemble_front - near).clamp(0.0, 1.0);
                    if reveal <= 0.0 {
                        continue;
                    }
                    if ha <= hb {
                        t1 = reveal;
                    } else {
                        t0 = 1.0 - reveal;
                    }
                }
                // The spectrum by height on the solid: lower edges carry the
                // bass. The knot carries it along its length instead.
                let u = if form == Form::Knot {
                    1.0 - (2.0 * edge_index as f32 / edges as f32 - 1.0).abs()
                } else {
                    let pa = wire.points[ia];
                    let pb = wire.points[ib];
                    let y = (pa[1] + pb[1]) * 0.5 + (pa[3] + pb[3]) * 0.25;
                    (y + 1.0) * 0.5
                };
                let band = band_at(state, u);
                let bright = 0.86 + (0.72 + 0.45 * band.powf(0.8) - 0.86) * rb;
                let thick = 1.0 + (0.84 + 0.4 * band - 1.0) * rb;
                let (pa, za, sa) = projected[ia];
                let (pb, zb, sb) = projected[ib];
                let at = |t: f32| [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t];
                let depth = |z: f32| 0.6 + 0.4 * (z + 1.0) * 0.5;
                let (za0, zb0) = (za + (zb - za) * t0, za + (zb - za) * t1);
                let (sa0, sb0) = (sa + (sb - sa) * t0, sa + (sb - sa) * t1);
                let mut paths = [[0.0f32; 2]; WAVES];
                for (slot, (_, ends)) in paths.iter_mut().zip(waves) {
                    *slot = ends[edge_index];
                }
                let seg = Seg {
                    a: at(t0),
                    b: at(t1),
                    ka: bright * depth(za0),
                    kb: bright * depth(zb0),
                    wa: width * form.stroke_scale() * thick * sa0,
                    wb: width * form.stroke_scale() * thick * sb0,
                    ta: t0,
                    tb: t1,
                    paths,
                    heat: 0.0,
                };
                self.splat(&seg, &wave_list, weights);
            }
            self.flush();
        }
        projected.into_iter().map(|(p, _, _)| p).collect()
    }
}

/// Builds the lattice for one frame of `state` in a `width` by `height`
/// output. Pure: the same state, frame and aspect give the same cells.
pub fn render(
    state: &GlyphLatticeState,
    frame: &SceneFrame<'_>,
    width: f32,
    height: f32,
    field: &mut Field,
) -> FieldReport {
    let id = SceneId::GlyphLattice;
    let setting = |index| frame.setting(id, index);
    let lattice = Lattice::new(width, height, setting(control::DENSITY));
    field.reset(lattice);
    let music = state.music();
    let response = setting(control::RESPONSE).clamp(0.0, 2.0);
    let look = Look {
        response,
        level: music[0],
        bass: music[1],
        stroke: setting(control::STROKE)
            * lattice.pitch
            * (2.7 + 0.45 * music[0] + 0.35 * music[1] + 0.3 * state.punch()),
        ghosts: setting(control::GHOSTS).clamp(0.0, 1.0),
        size: setting(control::SIZE) * (1.0 + 0.045 * state.punch()),
    };
    let (yaw, drift, fourth) = state.angles();
    let pose_of = |yaw: f32, drift: f32, fourth: f32, scale: f32, offset: [f32; 2]| Pose {
        rotation: shapes::orientation(
            yaw,
            0.2 * (drift * 0.71).sin(),
            0.12 * (drift * 0.43 + 1.3).sin(),
        ),
        fourth,
        scale,
        offset,
    };
    let presence = state.presence();
    let mut report = FieldReport {
        form: state.form().name(),
        auto: setting(control::SOLID) < 0.5,
        cols: lattice.cols(),
        rows: lattice.rows(),
        line_live: state.line_live(),
        ..FieldReport::default()
    };

    // The giants: two other solids, large and slow, turning in the dark at
    // the frame's long ends. Ghost layer only.
    if look.ghosts > 0.0 {
        let cycle = state.cycle();
        let at = cycle.iter().position(|f| *f == state.form()).unwrap_or(0);
        let landscape = lattice.frame[0] >= lattice.frame[1];
        for (k, step) in [(0usize, 2usize), (1, 4)] {
            let form = cycle[(at + step) % cycle.len()];
            let side = if k == 0 { -1.0 } else { 1.0 };
            let offset = if landscape {
                [side * lattice.frame[0] * 0.37, 0.06 * side]
            } else {
                [0.05 * side, side * lattice.frame[1] * 0.36]
            };
            let pose = pose_of(
                -yaw * 0.45 + 2.1 * k as f32 + drift * 0.3,
                drift * 1.3 + 2.0 * k as f32,
                fourth * 0.5 + k as f32,
                1.55,
                offset,
            );
            let wire = form.wire();
            let weights = Weights {
                main: 0.0,
                ghost: look.ghosts * (0.3 + 0.3 * look.level),
            };
            field.draw_solid(
                state,
                &look,
                form,
                &wire,
                &pose,
                weights,
                look.stroke * 0.85,
                &[],
                None,
                0.0,
            );
            report.giants += 1;
        }
    }

    let form = state.form();
    let wire = form.wire();
    let solid_scale = look.size * (1.0 + 0.32 * presence);

    // The solid it just was, exhaling: growing and fading into the ghosts.
    if let Some(previous) = state.previous() {
        let age = state.previous_age();
        if age < FADE_SECONDS {
            let g = (1.0 - age / FADE_SECONDS) as f32;
            let pose = pose_of(
                yaw,
                drift,
                fourth,
                solid_scale * (1.0 + 0.45 * (1.0 - g)),
                [0.0, 0.0],
            );
            let old = previous.wire();
            field.draw_solid(
                state,
                &look,
                previous,
                &old,
                &pose,
                Weights {
                    main: 0.0,
                    ghost: g * g * 0.9,
                },
                look.stroke * 0.9,
                &[],
                None,
                0.0,
            );
            report.previous = Some(previous.name());
        }
    }

    // Echoes: the pose a moment ago, so a turn or a kick's lurch smears.
    let age = state.form_age();
    if look.ghosts > 0.0 && age > 0.5 {
        for (ago, weight) in [(0.08f32, 0.55f32), (0.19, 0.38), (0.33, 0.24)] {
            let (y, d, f) = state.angles_ago(ago);
            let pose = pose_of(y, d, f, solid_scale, [0.0, 0.0]);
            field.draw_solid(
                state,
                &look,
                form,
                &wire,
                &pose,
                Weights {
                    main: 0.0,
                    ghost: look.ghosts * weight * (0.45 + 0.55 * look.level),
                },
                look.stroke * 0.85,
                &[],
                None,
                0.0,
            );
            report.echoes += 1;
        }
    }

    // The solid itself, its pulses, and its assembly.
    let assembly_source = (mix64(state.seed() ^ ((form.index() as u64 + 1) * 0x2545_f491))
        % wire.points.len() as u64) as usize;
    let assembly_hops = wire.hops_covering(assembly_source);
    let reach = f32::from(assembly_hops.iter().copied().max().unwrap_or(0));
    let progress = (age / ASSEMBLE_SECONDS).clamp(0.0, 1.0) as f32;
    report.assembled = progress;
    let assembling = progress < 1.0;
    let assemble_front = progress * (reach + 1.0);
    let mut waves: Vec<(Wave, Vec<[f32; 2]>)> = Vec::new();
    if assembling {
        let ends = wire
            .edges
            .iter()
            .map(|[a, b]| {
                [
                    f32::from(assembly_hops[usize::from(*a)]),
                    f32::from(assembly_hops[usize::from(*b)]),
                ]
            })
            .collect();
        waves.push((
            Wave {
                front: assemble_front,
                width: form.pulse_width() * 1.2,
                tail: form.pulse_width() * 0.6,
                strength: 0.95,
                graph: true,
            },
            ends,
        ));
    }
    // Each pulse is a comet: one bright head running a seeded walk along the
    // edges from its vertex, never doubling back, with a streak behind it.
    let born_after = state.clock() - age;
    for pulse in state.pulses() {
        if pulse.born < born_after || waves.len() >= WAVES {
            continue;
        }
        let pulse_age = (state.clock() - pulse.born) as f32;
        let salt = mix64(state.seed() ^ pulse.id.wrapping_mul(0x9e37_79b9));
        let source = (salt % wire.points.len() as u64) as usize;
        let walk = wire.walk(source, form.walk_length(), salt);
        let front = pulse_age * form.pulse_speed();
        let width = form.pulse_width();
        let tail = width * 2.6;
        let fade = (1.0 - (front - walk.len() as f32) / (tail * 2.0)).clamp(0.0, 1.0)
            * (-pulse_age * 0.25).exp();
        if fade <= 0.0 || walk.is_empty() {
            continue;
        }
        let mut ends = vec![[OFF_PATH; 2]; wire.edges.len()];
        for (step, (edge, forward)) in walk.iter().enumerate() {
            let (near, far) = (step as f32, step as f32 + 1.0);
            ends[*edge] = if *forward { [near, far] } else { [far, near] };
        }
        waves.push((
            Wave {
                front,
                width,
                tail,
                strength: (0.4 + 0.7 * pulse.strength).min(1.0) * fade,
                graph: false,
            },
            ends,
        ));
    }
    report.waves = waves.len();
    let solid_weights = Weights {
        main: 1.0 - presence,
        ghost: presence * 0.8,
    };
    let pose = pose_of(yaw, drift, fourth, solid_scale, [0.0, 0.0]);
    let chain = field.draw_solid(
        state,
        &look,
        form,
        &wire,
        &pose,
        solid_weights,
        look.stroke,
        &waves,
        assembling.then_some(assembly_hops.as_slice()),
        assemble_front,
    );
    // The knot crossing itself is the one crossing a single stroke has.
    if form == Form::Knot && solid_weights.main > 0.05 && !assembling {
        let n = chain.len();
        for i in 0..n {
            let (a, b) = (chain[i], chain[(i + 1) % n]);
            for j in i + 3..n {
                // Neighbours along the loop meet at a joint, not a crossing.
                if n - (j - i) < 3 {
                    continue;
                }
                let (c, d) = (chain[j], chain[(j + 1) % n]);
                if let Some(x) = intersect(a, b, c, d) {
                    field.heat_spot(x, look.stroke * 1.3, 0.75 * solid_weights.main);
                }
            }
        }
    }

    // The line, set in stroke capitals.
    if let Some(line) = state.line().filter(|_| presence > 0.004) {
        draw_line(state, frame, field, &look, line, presence, &mut report);
    }

    finish(state, frame, field, &look, &mut report);
    report
}

fn intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denom = r[0] * s[1] - r[1] * s[0];
    if denom.abs() < 1.0e-12 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / denom;
    let u = (q[0] * r[1] - q[1] * r[0]) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}

/// Sets the remembered line. The whole line stands in its place in the
/// ghost tone from the cue's start, then lights word by word in reading order
/// over the start of the cue, each word igniting hot and cooling, so nothing
/// ever appears anywhere but where it ends up. Its junctions and sharp corners
/// gather warm light like a solid's vertices and run hot on the kicks, which
/// also sweep left to right through it. It tilts gently in depth and
/// dissolves into the ghosts once its cue is over.
fn draw_line(
    state: &GlyphLatticeState,
    frame: &SceneFrame<'_>,
    field: &mut Field,
    look: &Look,
    line: &super::LineMemory,
    presence: f32,
    report: &mut FieldReport,
) {
    let lattice = field.lattice;
    // Landscape sets a wide block; portrait uses its height for more rows.
    let (box_w, box_h) = (lattice.frame[0] * 0.86, (lattice.frame[1] * 0.58).min(1.05));
    let (set, scale) = strokes::set_line(&line.text, box_w / box_h);
    if set.strokes.is_empty() {
        return;
    }
    // Grid units to short sides, with a cap so one short word is a sign,
    // not a wall.
    let unit = (scale * box_h).min(0.27 / strokes::CAP);
    let (_, drift, _) = state.angles();
    let sway = 0.35 + 0.65 * look.level;
    let yaw = 0.2 * sway * (drift * 0.9).sin();
    let pitch = 0.1 * sway * (drift * 0.67 + 1.0).sin();
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let punch = state.punch();
    let grow = 1.0 + 0.035 * punch;
    let place = |p: [f32; 2]| -> ([f32; 2], f32) {
        let (x, y) = (p[0] * unit * grow, p[1] * unit * grow);
        // Yaw about the vertical, then pitch about the horizontal.
        let (x1, z1) = (x * cy, -x * sy);
        let (y2, z2) = (y * cp - z1 * sp, y * sp + z1 * cp);
        let persp = 2.4 / (2.4 - z2);
        ([x1 * persp, -y2 * persp], persp)
    };
    let duration = (line.end - line.start).max(0.05);
    let span = (duration * 0.38).clamp(0.25, 1.15);
    let since = frame.time_seconds - line.start;
    report.strokes_total = set.strokes.len();
    report.line = Some(line.text.clone());
    let live = state.line_live();
    // How far each word has lit (0 waits in the ghost tone) and how hot it
    // still is from lighting.
    let per_word = span / set.words.max(1) as f64;
    let word_light = |word: usize| -> (f32, f32) {
        if !live {
            return (1.0, 0.0);
        }
        let age = since - per_word * word as f64;
        let lit = (age / 0.12).clamp(0.0, 1.0) as f32;
        let flash = if age >= 0.0 {
            (-(age as f32) / 0.32).exp()
        } else {
            0.0
        };
        (lit, flash)
    };
    let width = (lattice.pitch * 1.7).max(strokes::CAP * unit * 0.135)
        * frame.setting(SceneId::GlyphLattice, control::STROKE).sqrt();
    // Kicks sweep left to right through the letters.
    let mut waves = Vec::new();
    let block = set.width.max(1.0);
    for pulse in state.pulses() {
        let age = (state.clock() - pulse.born) as f32;
        let front = age * block * 1.6 - set.width * 0.5 - 2.0;
        if front > set.width * 0.5 + 6.0 || waves.len() >= WAVES {
            continue;
        }
        waves.push(Wave {
            front,
            width: 1.4,
            tail: 2.4,
            strength: pulse.strength * 0.8,
            graph: false,
        });
    }
    // Large letters glow like a solid's edges; small ones less, or the halo
    // bridges the gaps between them and the line goes soft.
    let cap_cells = strokes::CAP * unit / lattice.pitch;
    let size_gain = (cap_cells / 28.0).clamp(0.35, 1.0);
    field.halo_gain = size_gain;
    for stroke in &set.strokes {
        let (lit, flash) = word_light(stroke.word);
        let weights = if live {
            Weights {
                main: presence * lit,
                ghost: presence * (1.0 - lit) * 0.85,
            }
        } else {
            Weights {
                main: 0.0,
                ghost: presence * 0.95,
            }
        };
        if lit > 0.0 && live {
            report.strokes_drawn += 1;
        }
        let heat = (0.85 * flash).max(0.3 * punch);
        // Each word breathes with a band of the spectrum, as a solid's edges
        // do: the line's first word low, its last high.
        let band = band_at(
            state,
            stroke.word as f32 / set.words.saturating_sub(1).max(1) as f32,
        );
        let rb = look.response.min(1.0);
        let bright = 1.12 + (0.92 + 0.42 * band.powf(0.8) - 1.12) * rb;
        for pair in stroke.points.windows(2) {
            let (a, sa) = place(pair[0]);
            let (b, sb) = place(pair[1]);
            let mut paths = [[0.0f32; 2]; WAVES];
            for slot in paths.iter_mut().take(waves.len()) {
                *slot = [pair[0][0], pair[1][0]];
            }
            let seg = Seg {
                a,
                b,
                ka: bright * sa.min(1.15),
                kb: bright * sb.min(1.15),
                wa: width * sa,
                wb: width * sb,
                ta: 0.0,
                tb: 1.0,
                paths,
                heat,
            };
            field.splat(&seg, &waves, weights);
        }
        field.flush();
    }
    field.halo_gain = 0.0;
    // The letters' vertices: warm where strokes meet, hot on the kick.
    if live {
        for (point, word) in strokes::joints(&set) {
            let (lit, _) = word_light(word);
            if lit <= 0.0 {
                continue;
            }
            let (at, persp) = place(point);
            let strength =
                (0.46 + 0.55 * punch).min(0.95) * lit * presence * (0.55 + 0.45 * size_gain);
            field.heat_spot(at, width * persp * 1.2, strength);
        }
    }
}

/// Combines the strokes, adds the ripples' wash, blooms, counts and packs.
fn finish(
    state: &GlyphLatticeState,
    frame: &SceneFrame<'_>,
    field: &mut Field,
    look: &Look,
    report: &mut FieldReport,
) {
    let lattice = field.lattice;
    let (cols, rows) = (lattice.cols(), lattice.rows());
    for i in 0..cols * rows {
        let max = field.max[i];
        let extra = (field.sum[i] - max * max).max(0.0);
        field.lit[i] = (max + 0.08 * extra).clamp(0.0, 1.0);
        // Two strokes meeting warm a cell; three or more make it hot. A
        // solid's vertices glow, the place where many edges cross pools.
        let overlap = (extra * 0.26 - 0.14).clamp(0.0, 1.0);
        field.heat[i] = field.pulse[i]
            .max(overlap)
            .max(field.cross[i])
            .clamp(0.0, 1.0);
    }

    // Ripples from the centre: a faint ring of light crossing the whole
    // lattice, lit or not.
    let fronts: Vec<(f32, f32)> = state
        .ripples()
        .iter()
        .map(|(born, strength)| {
            let age = state.clock() - born;
            let fade = (1.0 - age / RIPPLE_SECONDS).clamp(0.0, 1.0) as f32;
            (age as f32 * RIPPLE_SPEED, strength * fade * fade)
        })
        .collect();
    report.ripple_fronts = fronts.iter().map(|(front, _)| *front).collect();
    debug_assert!(report.ripple_fronts.len() <= RIPPLES);
    if !fronts.is_empty() {
        for row in 0..rows {
            for col in 0..cols {
                let p = lattice.centre(col, row);
                let rho = (p[0] * p[0] + p[1] * p[1]).sqrt();
                let mut wash = 0.0f32;
                for (front, strength) in &fronts {
                    let x = (rho - front) / 0.024;
                    if x.abs() < 3.0 {
                        wash = wash.max(strength * (-x * x).exp());
                    }
                }
                field.wash[row * cols + col] = wash;
            }
        }
    }

    // Bloom on the lattice: core light and heat, blurred twice.
    let glow = frame.setting(SceneId::GlyphLattice, control::GLOW).max(0.0);
    for i in 0..cols * rows {
        let lit = field.lit[i];
        let halo = field.halo[i];
        field.bloom[i] =
            lit * lit * 0.4 + halo * halo * 0.55 + field.heat[i] * 0.5 + field.wash[i] * 0.05;
        field.warm[i] = field.heat[i];
        field.pool[i] = field.heat[i] * 1.2 + lit * lit * 0.03;
    }
    let radius = ((0.01 + 0.01 * glow + 0.01 * look.bass) / lattice.pitch)
        .round()
        .clamp(1.0, 24.0) as usize;
    field.blur(Blur::Bloom, radius);
    field.blur(Blur::Warm, radius);
    let wide = ((0.04 + 0.03 * glow + 0.03 * look.bass) / lattice.pitch)
        .round()
        .clamp(2.0, 48.0) as usize;
    field.blur(Blur::Pool, wide);
    let gain = 0.95 + 0.45 * state.punch() + 0.3 * look.bass;
    let pool_gain = 2.2 + 1.2 * state.punch() + 1.4 * look.bass;
    let mut peak = 0.0f32;
    for i in 0..cols * rows {
        field.bloom[i] = (field.bloom[i] * gain).clamp(0.0, 1.0);
        field.warm[i] = (field.warm[i] * 2.4).clamp(0.0, 1.0);
        field.pool[i] = (field.pool[i] * pool_gain).clamp(0.0, 1.0);
        peak = peak.max(field.bloom[i]);
    }
    report.peak_bloom = peak;

    for i in 0..cols * rows {
        let lit = field.lit[i];
        if lit > 0.06 {
            if field.heat[i] > 0.45 {
                report.hot += 1;
            } else if lit > 0.72 {
                report.bright += 1;
            } else if lit > 0.34 {
                report.mid += 1;
            } else {
                report.dim += 1;
            }
        } else if field.ghost[i] > 0.06 {
            report.ghost += 1;
        }
        if field.wash[i] > 0.1 {
            report.washed += 1;
        }
    }

    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let stride = 2 * cols * 4;
    field.rgba.clear();
    field.rgba.resize(stride * rows, 0);
    for row in 0..rows {
        for col in 0..cols {
            let i = row * cols + col;
            let a = row * stride + col * 4;
            field.rgba[a..a + 4].copy_from_slice(&[
                q(field.lit[i]),
                q(field.heat[i]),
                q(field.ghost[i]),
                q(field.wash[i]),
            ]);
            let b = row * stride + (cols + col) * 4;
            field.rgba[b..b + 4].copy_from_slice(&[
                q(field.bloom[i]),
                q(field.warm[i]),
                q(field.pool[i]),
                255,
            ]);
        }
    }
}
