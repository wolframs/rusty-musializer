//! The solids Glyph Lattice draws, as wire graphs.
//!
//! Every form is a set of points and the edges between them, scaled to a
//! circumradius of one, so one camera frames them all alike. Points carry a
//! fourth coordinate for the tesseract; the others leave it at zero. A wire is
//! also a graph: a kick's pulse runs along it by hop count from a vertex, and a
//! form assembles outward the same way, so "the light travels the edges" is
//! one rule for every solid.

use std::f32::consts::{PI, TAU};

/// The solids, in the order the `solid` setting names them (1-based there;
/// 0 is the automatic cycle). Persisted through that index: append, never
/// reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
    Cube,
    Octahedron,
    Dodecahedron,
    Icosahedron,
    /// The four-dimensional cube, turning through the fourth axis.
    Tesseract,
    /// A trefoil torus knot drawn as one closed stroke.
    Knot,
}

impl Form {
    pub const ALL: [Form; 6] = [
        Form::Cube,
        Form::Octahedron,
        Form::Dodecahedron,
        Form::Icosahedron,
        Form::Tesseract,
        Form::Knot,
    ];

    /// The form a `solid` setting names, or `None` for 0, the automatic cycle.
    #[must_use]
    pub fn from_setting(value: f32) -> Option<Form> {
        if !value.is_finite() || value < 0.5 {
            return None;
        }
        let index = (value.round() as usize).clamp(1, Self::ALL.len()) - 1;
        Some(Self::ALL[index])
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|form| *form == self).unwrap_or(0)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Form::Cube => "cube",
            Form::Octahedron => "octahedron",
            Form::Dodecahedron => "dodecahedron",
            Form::Icosahedron => "icosahedron",
            Form::Tesseract => "tesseract",
            Form::Knot => "knot",
        }
    }

    /// Hops a pulse travels per second. The solids' edges are all one length,
    /// so this is edges per second; the knot's hops are its short segments,
    /// so it needs many more to cross half its loop in about the same time.
    #[must_use]
    pub fn pulse_speed(self) -> f32 {
        match self {
            Form::Knot => KNOT_POINTS as f32 * 0.5 / 0.95,
            Form::Tesseract => 4.6,
            _ => 3.4,
        }
    }

    /// The width of a pulse's bright head, in hops.
    #[must_use]
    pub fn pulse_width(self) -> f32 {
        match self {
            Form::Knot => KNOT_POINTS as f32 * 0.03,
            _ => 0.22,
        }
    }

    /// Stroke weight relative to the cube's: a solid with many edges draws
    /// them thinner, so its faces stay open.
    #[must_use]
    pub fn stroke_scale(self) -> f32 {
        match self {
            Form::Icosahedron | Form::Dodecahedron => 0.74,
            Form::Tesseract => 0.8,
            Form::Octahedron => 0.92,
            Form::Cube | Form::Knot => 1.0,
        }
    }

    /// Hops a comet runs before it burns out: most of a solid's girth, or
    /// half the knot's loop.
    #[must_use]
    pub fn walk_length(self) -> usize {
        match self {
            Form::Knot => KNOT_POINTS / 2,
            Form::Tesseract | Form::Icosahedron | Form::Dodecahedron => 6,
            _ => 5,
        }
    }

    #[must_use]
    pub fn wire(self) -> Wire {
        match self {
            Form::Cube => cube(),
            Form::Octahedron => octahedron(),
            Form::Dodecahedron => dodecahedron(),
            Form::Icosahedron => icosahedron(),
            Form::Tesseract => tesseract(),
            Form::Knot => knot(),
        }
    }
}

/// Points along the knot's closed stroke.
pub const KNOT_POINTS: usize = 168;

/// A form as a graph: points, edges, and which edges draw as one stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    /// x, y, z and the tesseract's w; circumradius one.
    pub points: Vec<[f32; 4]>,
    pub edges: Vec<[u16; 2]>,
    /// Consecutive runs of `edges` drawn as one stroke, so the joints inside a
    /// run do not count as two strokes crossing. A solid's edges are each
    /// their own stroke; the knot is one.
    pub strokes: Vec<std::ops::Range<usize>>,
}

impl Wire {
    fn of_edges(points: Vec<[f32; 4]>, edges: Vec<[u16; 2]>) -> Self {
        let strokes = (0..edges.len()).map(|edge| edge..edge + 1).collect();
        Self {
            points,
            edges,
            strokes,
        }
    }

    /// Hop counts from `source` to every point (`u16::MAX` when unreachable).
    #[must_use]
    pub fn hops_from(&self, source: usize) -> Vec<u16> {
        let mut hops = vec![u16::MAX; self.points.len()];
        if source < hops.len() {
            self.flood(&mut hops, source);
        }
        hops
    }

    /// Hop counts from `source`, and from the first point of every part of
    /// the graph `source` cannot reach, so a pulse or an assembly covers the
    /// whole form whatever wire it is given.
    #[must_use]
    pub fn hops_covering(&self, source: usize) -> Vec<u16> {
        let mut hops = self.hops_from(source);
        while let Some(start) = hops.iter().position(|hop| *hop == u16::MAX) {
            self.flood(&mut hops, start);
        }
        hops
    }

    fn flood(&self, hops: &mut [u16], source: usize) {
        hops[source] = 0;
        let mut queue = std::collections::VecDeque::from([source]);
        while let Some(point) = queue.pop_front() {
            let next = hops[point].saturating_add(1);
            for [a, b] in &self.edges {
                let (a, b) = (usize::from(*a), usize::from(*b));
                let neighbour = if a == point {
                    b
                } else if b == point {
                    a
                } else {
                    continue;
                };
                if hops[neighbour] == u16::MAX {
                    hops[neighbour] = next;
                    queue.push_back(neighbour);
                }
            }
        }
    }

    /// A seeded walk of up to `steps` edges from `source` that never takes an
    /// edge twice: each entry is an edge and whether it is walked from its
    /// first point to its second. The knot's walk runs around its loop.
    #[must_use]
    pub fn walk(&self, source: usize, steps: usize, salt: u64) -> Vec<(usize, bool)> {
        let mut walk = Vec::with_capacity(steps);
        let mut used = vec![false; self.edges.len()];
        let mut at = source;
        let mut state = salt;
        for _ in 0..steps {
            let options: Vec<(usize, bool, usize)> = self
                .edges
                .iter()
                .enumerate()
                .filter(|(index, _)| !used[*index])
                .filter_map(|(index, [a, b])| {
                    let (a, b) = (usize::from(*a), usize::from(*b));
                    if a == at {
                        Some((index, true, b))
                    } else if b == at {
                        Some((index, false, a))
                    } else {
                        None
                    }
                })
                .collect();
            if options.is_empty() {
                break;
            }
            state = state
                .wrapping_mul(0x5851_f42d_4c95_7f2d)
                .wrapping_add(0x1405_7b7e_f767_814f);
            let (edge, forward, next) = options[((state >> 33) % options.len() as u64) as usize];
            used[edge] = true;
            walk.push((edge, forward));
            at = next;
        }
        walk
    }

    /// The largest hop count from `source`: how far a pulse has to go.
    #[must_use]
    pub fn reach_from(&self, source: usize) -> u16 {
        self.hops_covering(source).into_iter().max().unwrap_or(0)
    }
}

fn edges_at_distance(points: &[[f32; 4]], length: f32) -> Vec<[u16; 2]> {
    let mut edges = Vec::new();
    for a in 0..points.len() {
        for b in a + 1..points.len() {
            let d2: f32 = (0..4).map(|i| (points[a][i] - points[b][i]).powi(2)).sum();
            if (d2.sqrt() - length).abs() < length * 1.0e-3 {
                edges.push([a as u16, b as u16]);
            }
        }
    }
    edges
}

fn cube() -> Wire {
    let s = 1.0 / 3.0f32.sqrt();
    let mut points = Vec::new();
    for i in 0..8 {
        let sign = |bit: u32| if i & (1 << bit) != 0 { s } else { -s };
        points.push([sign(0), sign(1), sign(2), 0.0]);
    }
    let edges = edges_at_distance(&points, 2.0 * s);
    Wire::of_edges(points, edges)
}

fn octahedron() -> Wire {
    let mut points = Vec::new();
    for axis in 0..3 {
        for sign in [1.0, -1.0] {
            let mut point = [0.0; 4];
            point[axis] = sign;
            points.push(point);
        }
    }
    let edges = edges_at_distance(&points, 2.0f32.sqrt());
    Wire::of_edges(points, edges)
}

fn dodecahedron() -> Wire {
    // The cube's corners, plus three golden rectangles' worth.
    let phi = (1.0 + 5.0f32.sqrt()) * 0.5;
    let norm = 3.0f32.sqrt();
    let mut points = Vec::new();
    for i in 0..8 {
        let sign = |bit: u32| if i & (1 << bit) != 0 { 1.0 } else { -1.0 };
        points.push([sign(0) / norm, sign(1) / norm, sign(2) / norm, 0.0]);
    }
    for a in [1.0, -1.0] {
        for b in [1.0, -1.0] {
            points.push([0.0, a / phi / norm, b * phi / norm, 0.0]);
            points.push([a / phi / norm, b * phi / norm, 0.0, 0.0]);
            points.push([a * phi / norm, 0.0, b / phi / norm, 0.0]);
        }
    }
    let edges = edges_at_distance(&points, 2.0 / phi / norm);
    Wire::of_edges(points, edges)
}

fn icosahedron() -> Wire {
    let phi = (1.0 + 5.0f32.sqrt()) * 0.5;
    let norm = (1.0 + phi * phi).sqrt();
    let mut points = Vec::new();
    for a in [1.0, -1.0] {
        for b in [phi, -phi] {
            points.push([0.0, a / norm, b / norm, 0.0]);
            points.push([a / norm, b / norm, 0.0, 0.0]);
            points.push([b / norm, 0.0, a / norm, 0.0]);
        }
    }
    let edges = edges_at_distance(&points, 2.0 / norm);
    Wire::of_edges(points, edges)
}

fn tesseract() -> Wire {
    let mut points = Vec::new();
    for i in 0..16 {
        let sign = |bit: u32| if i & (1 << bit) != 0 { 0.5 } else { -0.5 };
        points.push([sign(0), sign(1), sign(2), sign(3)]);
    }
    let edges = edges_at_distance(&points, 1.0);
    Wire::of_edges(points, edges)
}

fn knot() -> Wire {
    // A (2, 3) torus knot: the trefoil, three lobes and three crossings.
    let mut points = Vec::with_capacity(KNOT_POINTS);
    for i in 0..KNOT_POINTS {
        let phi = TAU * i as f32 / KNOT_POINTS as f32;
        let r = (3.0 * phi).cos() + 2.1;
        points.push([
            r * (2.0 * phi).cos() / 3.1,
            r * (2.0 * phi).sin() / 3.1,
            -(3.0 * phi).sin() * 0.55,
            0.0,
        ]);
    }
    let edges: Vec<[u16; 2]> = (0..KNOT_POINTS)
        .map(|i| [i as u16, ((i + 1) % KNOT_POINTS) as u16])
        .collect();
    let count = edges.len();
    // One stroke: the whole loop, so its own joints never read as crossings.
    let strokes = std::iter::once(0..count).collect();
    Wire {
        points,
        edges,
        strokes,
    }
}

/// An orientation: a 3x3 rotation, row-major.
pub type Rotation = [[f32; 3]; 3];

/// The rotation for yaw (about y), pitch (about x) and roll (about z),
/// applied after the base pose that shows a cube corner-on: a body diagonal
/// pointed at the camera, which is the silhouette the scene opens on.
#[must_use]
pub fn orientation(yaw: f32, pitch: f32, roll: f32) -> Rotation {
    // Corner-on: turn 45 degrees about y, then tip by atan(1/sqrt 2) about x.
    let base = mul(rot_x(0.615_479_7), rot_y(PI * 0.25));
    mul(rot_z(roll), mul(rot_x(pitch), mul(rot_y(yaw), base)))
}

fn rot_x(a: f32) -> Rotation {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(a: f32) -> Rotation {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(a: f32) -> Rotation {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn mul(a: Rotation, b: Rotation) -> Rotation {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Turns a point through the fourth axis by `angle` (in the xw and zw planes,
/// the second at a slower rate so the turn never repeats exactly) and
/// projects it into three dimensions. A perspective in w is what makes the
/// inner cube swell through the outer one and turn the solid inside out.
#[must_use]
pub fn fold_4d(point: [f32; 4], angle: f32) -> [f32; 3] {
    let (s1, c1) = angle.sin_cos();
    let (s2, c2) = (angle * 0.61).sin_cos();
    let x = point[0] * c1 - point[3] * s1;
    let w = point[0] * s1 + point[3] * c1;
    let z = point[2] * c2 - w * s2;
    let w = point[2] * s2 + w * c2;
    let k = 1.55 / (1.55 - w) * 0.74;
    [x * k, point[1] * k, z * k]
}

/// Rotates a point.
#[must_use]
pub fn rotate(rotation: &Rotation, p: [f32; 3]) -> [f32; 3] {
    [
        rotation[0][0] * p[0] + rotation[0][1] * p[1] + rotation[0][2] * p[2],
        rotation[1][0] * p[0] + rotation[1][1] * p[1] + rotation[1][2] * p[2],
        rotation[2][0] * p[0] + rotation[2][1] * p[1] + rotation[2][2] * p[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_has_the_edges_its_solid_has() {
        let counts = [
            (Form::Cube, 8, 12),
            (Form::Octahedron, 6, 12),
            (Form::Dodecahedron, 20, 30),
            (Form::Icosahedron, 12, 30),
            (Form::Tesseract, 16, 32),
            (Form::Knot, KNOT_POINTS, KNOT_POINTS),
        ];
        for (form, points, edges) in counts {
            let wire = form.wire();
            assert_eq!(wire.points.len(), points, "{form:?} points");
            assert_eq!(wire.edges.len(), edges, "{form:?} edges");
            let radius = wire
                .points
                .iter()
                .map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2] + p[3] * p[3]).sqrt())
                .fold(0.0f32, f32::max);
            assert!((0.5..=1.001).contains(&radius), "{form:?} radius {radius}");
            let covered: usize = wire.strokes.iter().map(|range| range.len()).sum();
            assert_eq!(covered, wire.edges.len());
        }
    }

    #[test]
    fn hops_walk_the_graph() {
        let cube = Form::Cube.wire();
        let hops = cube.hops_from(0);
        assert_eq!(hops.iter().filter(|h| **h == 1).count(), 3);
        assert_eq!(cube.reach_from(0), 3);
        assert_eq!(Form::Octahedron.wire().reach_from(0), 2);
        assert_eq!(Form::Tesseract.wire().reach_from(0), 4);
        assert_eq!(
            usize::from(Form::Knot.wire().reach_from(0)),
            KNOT_POINTS / 2
        );
        assert_eq!(Form::Dodecahedron.wire().reach_from(0), 5);
        // Covering reaches every part of a disconnected wire.
        let mut split = Form::Cube.wire();
        split.edges.retain(|[a, b]| (a & 1) == (b & 1));
        assert!(split.hops_from(0).contains(&u16::MAX));
        assert!(!split.hops_covering(0).contains(&u16::MAX));
    }

    #[test]
    fn a_walk_follows_edges_without_repeating_one() {
        for form in Form::ALL {
            let wire = form.wire();
            for salt in 0..20u64 {
                let walk = wire.walk(salt as usize % wire.points.len(), form.walk_length(), salt);
                assert!(!walk.is_empty());
                let mut at = salt as usize % wire.points.len();
                let mut seen = std::collections::HashSet::new();
                for (edge, forward) in walk {
                    assert!(seen.insert(edge), "{form:?} repeats an edge");
                    let [a, b] = wire.edges[edge];
                    let (from, to) = if forward { (a, b) } else { (b, a) };
                    assert_eq!(usize::from(from), at, "{form:?} jumps");
                    at = usize::from(to);
                }
            }
        }
    }

    #[test]
    fn the_setting_names_forms_and_zero_is_the_cycle() {
        assert_eq!(Form::from_setting(0.0), None);
        assert_eq!(Form::from_setting(1.0), Some(Form::Cube));
        assert_eq!(Form::from_setting(6.0), Some(Form::Knot));
        assert_eq!(Form::from_setting(f32::NAN), None);
        for form in Form::ALL {
            assert_eq!(Form::from_setting(form.index() as f32 + 1.0), Some(form));
        }
    }

    #[test]
    fn rotations_preserve_length() {
        let r = orientation(0.7, -0.3, 1.9);
        let p = rotate(&r, [0.3, -0.5, 0.8]);
        let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((length - (0.98f32).sqrt()).abs() < 1.0e-5);
    }
}
