//! Song Atlas — Tideline: the whole song engraved into a spiral of light.
//!
//! Time advances along a relief that breathes and folds with the music; flowing
//! light crosses its surface while a warm front marks the song position.
//! Frequency runs across the groove, amplitude embosses its surface.
//! Unlike framebuffer feedback, that memory is reconstructed from musical time:
//! a seek and an export show the same chart, with no warm-up or camera reset.
//!
//! SX4 (2026-09-05): original implementation, informed by an Opus 5 design
//! discussion and Le Biniou's vocabulary of flow/slow fade, without copied code
//! or assets. Settings keys/bounds/defaults are retained: height is relief,
//! width is groove width, depth is radial spacing, camera is elevation, orbit
//! and distance frame the chart, contours trace the relief, drift moves the
//! vantage around the relief. Wireframe follows the same moving surface.

use std::hash::{Hash, Hasher};

use musializer_core::scene::settings::index::atlas as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::song_atlas::{
    self as core_atlas, Slice, SongAtlasMap, SongAtlasState, BAND_COUNT,
};
use musializer_runtime::draw::Camera3D;
use musializer_runtime::draw::{self, SceneViewport};
use raylib::prelude::{Color, RaylibDrawHandle, RaylibMode3DExt, Rectangle, Vector2, Vector3};

const TAU: f32 = std::f32::consts::TAU;
const ACROSS: usize = 41;

/// An open rlgl immediate-mode batch. `rlEnd` runs on drop.
///
/// The terrain issues vertices directly because raylib's safe per-triangle call
/// would break this thousands-of-vertices pass into separate submissions.
struct Batch;

impl Batch {
    /// `mode` is an `RL_*` primitive constant.
    fn begin(mode: u32) -> Self {
        // SAFETY: opens an rlgl batch inside an active drawing context, which the
        // caller holds. `Drop` guarantees the matching `rlEnd`.
        unsafe { raylib_sys::rlBegin(mode as i32) }
        Self
    }

    /// `atlas_rl_vertex` (`scene_song_atlas.c:247-251`): colour then position, in
    /// that order, because rlgl's colour is current-state.
    fn vertex(&mut self, point: Vector3, color: Color) {
        // SAFETY: immediate-mode vertex submission inside an open batch, which
        // `self` proves.
        unsafe {
            raylib_sys::rlColor4ub(color.r, color.g, color.b, color.a);
            raylib_sys::rlVertex3f(point.x, point.y, point.z);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        // SAFETY: closes the batch opened in `begin`.
        unsafe { raylib_sys::rlEnd() }
    }
}

/// Sets the batched line width, restoring it to 1 on drop
/// (`scene_song_atlas.c:345` and `403`).
struct LineWidth;

impl LineWidth {
    fn set(width: f32) -> Self {
        // SAFETY: an rlgl state setter inside a drawing context.
        unsafe { raylib_sys::rlSetLineWidth(width) }
        Self
    }
}

impl Drop for LineWidth {
    fn drop(&mut self) {
        // SAFETY: as `set`. C restores exactly 1.0.
        unsafe { raylib_sys::rlSetLineWidth(1.0) }
    }
}

/// `ColorToHSV`, which raylib-rs only exposes on its own `Color`.
fn color_to_hsv(color: Color) -> Vector3 {
    let raw = raylib_sys::Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    };
    // SAFETY: pure arithmetic over a by-value colour.
    let out = unsafe { raylib_sys::ColorToHSV(raw) };
    Vector3::new(out.x, out.y, out.z)
}

fn mix_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

fn hue_shift(color: Color, shift: f32) -> Color {
    let hsv = color_to_hsv(color);
    let mut c = draw::color_from_hsv((hsv.x + shift).rem_euclid(360.0), hsv.y, hsv.z);
    c.a = color.a;
    c
}

fn palette(value: f32) -> Color {
    let stops = [
        Color::new(9, 23, 29, 255),
        Color::new(24, 64, 65, 255),
        Color::new(86, 135, 124, 255),
        Color::new(185, 206, 184, 255),
        Color::new(242, 232, 204, 255),
    ];
    let x = value.clamp(0.0, 1.0) * 4.0;
    let i = (x.floor() as usize).min(3);
    mix_color(stops[i], stops[i + 1], x - i as f32)
}

#[derive(Clone, Copy)]
struct ReliefVertex {
    point: Vector3,
    amplitude: f32,
    across: f32,
}

/// CPU-side rest-shape cache. Motion is evaluated after this cache on every
/// frame; no GL objects or accumulated animation history live here.
#[derive(Default)]
pub struct AtlasRenderer {
    identity: Option<u64>,
    vertices: Vec<ReliefVertex>,
    material: Vec<Color>,
    energy_floor: f32,
    energy_span: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RowMotion {
    radial_scale: f32,
    twist_sin: f32,
    twist_cos: f32,
    lift: f32,
    light: f32,
}

/// Traveling folds in fixed song coordinates. Seconds own the flow rate,
/// independent of track duration; musical energy owns its reach. Reducing the
/// phase in f64 avoids stepped movement on long tracks. No beat-phase wrap or
/// integration state can restart this field on a seek.
fn row_motion(time: f64, song_position: f32, energy: f32, bass: f32) -> RowMotion {
    let phase = (time * 1.35 - f64::from(song_position) * std::f64::consts::TAU * 2.0)
        .rem_euclid(std::f64::consts::TAU) as f32;
    let strength = 0.20 + 0.80 * energy.clamp(0.0, 1.0);
    let wave = phase.sin();
    let twist = 0.055 * strength * phase.cos();
    let (twist_sin, twist_cos) = twist.sin_cos();
    let light_phase = (time * 1.9 - f64::from(song_position) * std::f64::consts::TAU * 5.0)
        .rem_euclid(std::f64::consts::TAU) as f32;
    RowMotion {
        radial_scale: 1.0 + 0.035 * strength * wave + 0.025 * bass.clamp(0.0, 1.0),
        twist_sin,
        twist_cos,
        lift: 0.38 * strength * wave + 0.16 * bass.clamp(0.0, 1.0),
        light: (0.5 + 0.5 * light_phase.cos()).powi(5),
    }
}

fn animated_point(vertex: ReliefVertex, motion: RowMotion, band: f32) -> Vector3 {
    let edge = (vertex.across * std::f32::consts::PI).sin().max(0.0);
    let p = vertex.point;
    Vector3::new(
        (p.x * motion.twist_cos - p.z * motion.twist_sin) * motion.radial_scale,
        p.y * (1.0 + 0.65 * band) + motion.lift * (0.35 + 0.65 * edge),
        (p.x * motion.twist_sin + p.z * motion.twist_cos) * motion.radial_scale,
    )
}

fn camera_azimuth(time: f64, orbit: f32, drift: f32) -> f32 {
    orbit
        + (time * 0.045 * f64::from(drift)).rem_euclid(std::f64::consts::TAU) as f32
        + (time * 0.16).sin() as f32 * 0.06 * drift
}

fn geometry_identity(slices: &[Slice], seed: u64, height: f32, width: f32, depth: f32) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hash);
    for value in [height, width, depth] {
        value.to_bits().hash(&mut hash);
    }
    slices.len().hash(&mut hash);
    for slice in slices {
        slice.rms.to_bits().hash(&mut hash);
        for band in slice.bands {
            band.to_bits().hash(&mut hash);
        }
    }
    hash.finish()
}

fn normalized(v: Vector3) -> Vector3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(0.00001);
    Vector3::new(v.x / length, v.y / length, v.z / length)
}
fn delta(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn dot(a: Vector3, b: Vector3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// A fixed spatial kernel on the immutable map. Drawing detail cannot change
/// its coordinates or time domain: no moving-window resampling of the song.
fn amplitude(slices: &[Slice], row: usize, band: f32) -> f32 {
    let lo = band.floor() as usize;
    let fraction = band - lo as f32;
    let mut sum = 0.0;
    for (di, tw) in [
        (-4, 1.0),
        (-3, 4.0),
        (-2, 7.0),
        (-1, 10.0),
        (0, 12.0),
        (1, 10.0),
        (2, 7.0),
        (3, 4.0),
        (4, 1.0),
    ] {
        let i = (row as isize + di).clamp(0, slices.len() as isize - 1) as usize;
        for (dk, bw) in [(-2, 1.0), (-1, 4.0), (0, 6.0), (1, 4.0), (2, 1.0)] {
            let k = (lo as isize + dk).clamp(0, BAND_COUNT as isize - 1) as usize;
            let k1 = (lo as isize + dk + 1).clamp(0, BAND_COUNT as isize - 1) as usize;
            let a = slices[i].bands[k] * (1.0 - fraction) + slices[i].bands[k1] * fraction;
            sum += a * tw * bw;
        }
    }
    (sum / 896.0).clamp(0.0, 1.0)
}

fn build_geometry(
    slices: &[Slice],
    seed: u64,
    height: f32,
    width: f32,
    depth: f32,
) -> (Vec<ReliefVertex>, Vec<Color>) {
    let turns = 3.35;
    let inner = 0.86;
    let radial = 3.65 * depth.sqrt();
    let pitch = radial / turns;
    let groove = pitch * (0.88 * width.sqrt()).min(0.97);
    let phase = -1.0 + core_atlas::hash_unit(seed, 7) * 0.4;
    // Every stored slice participates even at low drawing detail. Detail adds
    // contour density, never moves the underlying audio landmarks.
    let amplitudes: Vec<f32> = (0..slices.len())
        .flat_map(|row| {
            (0..ACROSS).map(move |k| {
                amplitude(
                    slices,
                    row,
                    k as f32 / (ACROSS - 1) as f32 * (BAND_COUNT - 1) as f32,
                )
            })
        })
        .collect();
    let mut ordered = amplitudes.clone();
    ordered.sort_by(f32::total_cmp);
    let low = ordered[ordered.len() / 10];
    let high = ordered[ordered.len() * 9 / 10];
    let range = (high - low).max(0.12);
    let mut levels: Vec<f32> = slices.iter().map(|s| s.rms).collect();
    levels.sort_by(f32::total_cmp);
    let floor = levels[levels.len() / 10];
    let ceiling = levels[levels.len() * 9 / 10];
    let mut vertices = Vec::with_capacity(slices.len() * ACROSS);
    for row in 0..slices.len() {
        let u = row as f32 / (slices.len() - 1) as f32;
        let theta = phase + u * turns * TAU;
        let (sin, cos) = theta.sin_cos();
        let envelope = musializer_core::scenes::cadence::smooth(u / 0.025)
            * musializer_core::scenes::cadence::smooth((1.0 - u) / 0.035);
        let rms = (-5isize..=5)
            .map(|offset| {
                let index = (row as isize + offset).clamp(0, slices.len() as isize - 1) as usize;
                slices[index].rms * (6 - offset.abs()) as f32
            })
            .sum::<f32>()
            / 36.0;
        let level = ((rms - floor) / (ceiling - floor).max(0.15)).clamp(0.0, 1.0);
        let local_width = groove * (0.28 + 0.72 * level) * envelope;
        for k in 0..ACROSS {
            let v = k as f32 / (ACROSS - 1) as f32;
            let a = ((amplitudes[row * ACROSS + k] - low) / range).clamp(0.0, 1.0);
            let r = inner + radial * u + (v - 0.5) * local_width;
            let edge = (v * std::f32::consts::PI).sin().max(0.0);
            let relief =
                pitch * height * 0.42 * edge.powf(0.65) * (0.035 + a.powf(1.7) * 0.70) * envelope;
            vertices.push(ReliefVertex {
                point: Vector3::new(r * cos, relief, r * sin),
                amplitude: a,
                across: v,
            });
        }
    }
    let mut colors = Vec::with_capacity(vertices.len());
    let key = normalized(Vector3::new(-0.64, 0.32, -0.70));
    for row in 0..slices.len() {
        for k in 0..ACROSS {
            let v = vertices[row * ACROSS + k];
            let dt = delta(
                vertices[(row + 1).min(slices.len() - 1) * ACROSS + k].point,
                vertices[row.saturating_sub(1) * ACROSS + k].point,
            );
            let dk = delta(
                vertices[row * ACROSS + (k + 1).min(ACROSS - 1)].point,
                vertices[row * ACROSS + k.saturating_sub(1)].point,
            );
            let normal = normalized(cross(dt, dk));
            let lambert = (dot(normal, key) * 0.5 + 0.5).clamp(0.0, 1.0);
            let lip = (v.across * std::f32::consts::PI).sin().max(0.0);
            let sheen = dot(normal, normalized(Vector3::new(-0.5, 0.8, 0.2)))
                .max(0.0)
                .powf(18.0);
            let value =
                0.02 + 0.53 * lambert.powf(2.0) + 0.12 * v.amplitude + 0.30 * sheen + 0.02 * lip;
            colors.push(palette(value));
        }
    }
    (vertices, colors)
}

/// Draw the chart from map coordinates, including its spectral cross-section.
/// The same bounded ring supplies geometry when no whole-track map exists.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    renderer: &mut AtlasRenderer,
    state: &SongAtlasState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
    map: Option<&SongAtlasMap>,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }
    let get = |i| frame.setting(SceneId::SongAtlas, i);
    let height = get(setting::HEIGHT);
    let width = get(setting::WIDTH);
    let depth = get(setting::DEPTH);
    let elevation = get(setting::CAMERA);
    let contours = get(setting::CONTOURS);
    let drift = get(setting::SPEED);
    let orbit = get(setting::ORBIT).to_radians();
    let zoom = get(setting::ZOOM);
    let wire = get(setting::WIREFRAME) >= 0.5;
    let detail = get(setting::DETAIL).round().clamp(1.0, 3.0) as usize;
    let valid = map.filter(|m| m.is_valid());
    let fallback: Vec<Slice> = if valid.is_none() {
        (0..state.count())
            .filter_map(|i| state.slice(i).copied())
            .collect()
    } else {
        Vec::new()
    };
    let slices = valid.map_or(fallback.as_slice(), SongAtlasMap::slices);
    let progress = valid.map_or(1.0, |m| {
        (frame.time_seconds / m.duration_seconds()).clamp(0.0, 1.0) as f32
    });
    let energy = valid
        .and_then(|m| core_atlas::map_dynamics(m, core_atlas::map_playhead(m, frame.time_seconds)))
        .map_or(frame.audio.rms, |(e, _)| e);
    let mut shift = get(setting::COLOR);
    if get(setting::HUE_MOTION) >= 0.5 {
        shift += ((frame.time_seconds * 0.045).sin() as f32) * 16.0;
    }
    let ground = hue_shift(Color::new(6, 14, 19, 255), shift);
    draw::atmospheric_backdrop(
        d,
        boundary,
        ground,
        hue_shift(Color::new(14, 29, 33, 255), shift),
        Vector2::new(
            boundary.x + boundary.width * 0.58,
            boundary.y + boundary.height * 0.48,
        ),
        boundary.width.max(boundary.height) * 0.50,
        draw::color_alpha(hue_shift(Color::new(50, 78, 70, 255), shift), 0.34),
    );
    if slices.len() < 2 {
        return;
    }

    let identity = geometry_identity(slices, state.seed(), height, width, depth);
    if renderer.identity != Some(identity) {
        (renderer.vertices, renderer.material) =
            build_geometry(slices, state.seed(), height, width, depth);
        let mut levels: Vec<_> = slices.iter().map(|slice| slice.rms).collect();
        levels.sort_by(f32::total_cmp);
        renderer.energy_floor = levels[levels.len() / 10];
        renderer.energy_span = (levels[levels.len() * 9 / 10] - renderer.energy_floor).max(0.12);
        renderer.identity = Some(identity);
    }
    let energy = ((energy - renderer.energy_floor) / renderer.energy_span).clamp(0.0, 1.0);
    // Interpolate the spectrum at the same absolute position as the map light.
    // The bounded live ring uses its newest slice when no full map is available.
    let position = progress * (slices.len() - 1) as f32;
    let lower = (position.floor() as usize).min(slices.len() - 1);
    let upper = (lower + 1).min(slices.len() - 1);
    let mix = musializer_core::scenes::cadence::smooth(position - lower as f32);
    let bands: [f32; BAND_COUNT] = std::array::from_fn(|k| {
        (slices[lower].bands[k] + (slices[upper].bands[k] - slices[lower].bands[k]) * mix)
            .clamp(0.0, 1.0)
    });
    let bass = bands[..6].iter().sum::<f32>() / 6.0;
    let vertices = &renderer.vertices;
    let mut points = Vec::with_capacity(vertices.len());
    let mut colors = Vec::with_capacity(vertices.len());
    let flare = hue_shift(Color::new(255, 206, 126, 255), shift * 0.3);
    let flowing_ink = hue_shift(Color::new(177, 225, 203, 255), shift);
    for row in 0..slices.len() {
        let u = row as f32 / (slices.len() - 1) as f32;
        let memory = core_atlas::tideline_light(progress - u);
        let motion = row_motion(frame.time_seconds, u, energy, bass);
        for k in 0..ACROSS {
            let i = row * ACROSS + k;
            let band_position = vertices[i].across * (BAND_COUNT - 1) as f32;
            let band_index = band_position.floor() as usize;
            let band = bands[band_index]
                + (bands[(band_index + 1).min(BAND_COUNT - 1)] - bands[band_index])
                    * (band_position - band_index as f32);
            points.push(animated_point(vertices[i], motion, band));
            let warm = memory * (0.35 + 0.65 * vertices[i].amplitude) * (0.75 + energy * 0.25);
            let flowing = motion.light * (0.18 + 0.34 * energy + 0.20 * band);
            colors.push(mix_color(
                mix_color(hue_shift(renderer.material[i], shift), flowing_ink, flowing),
                flare,
                warm,
            ));
        }
    }
    let screen_width = d.get_screen_width();
    let screen_height = d.get_screen_height();
    let Some(viewport) = SceneViewport::begin_with_screen(boundary, screen_width, screen_height)
    else {
        return;
    };
    let azimuth = camera_azimuth(frame.time_seconds, orbit, drift);
    let pitch_angle = (0.90
        + (elevation - 1.0) * 0.32
        + (frame.time_seconds * 0.21).sin() as f32 * 0.035 * drift)
        .clamp(0.55, 1.22);
    let aspect = boundary.width / boundary.height;
    let distance = 11.8 * zoom * (1.15 / aspect).max(1.0);
    let camera = Camera3D::perspective(
        Vector3::new(
            azimuth.sin() * distance * pitch_angle.cos(),
            distance * pitch_angle.sin(),
            azimuth.cos() * distance * pitch_angle.cos(),
        ),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        46.0,
    );
    {
        let _space = d.begin_mode3D(camera);
        viewport.correct_projection_aspect();
        if !wire {
            let mut batch = Batch::begin(raylib_sys::RL_TRIANGLES);
            for row in 0..slices.len() - 1 {
                for k in 0..ACROSS - 1 {
                    let a = row * ACROSS + k;
                    let b = a + 1;
                    let c = a + ACROSS;
                    let e = c + 1;
                    for i in [a, c, b, b, c, e] {
                        batch.vertex(points[i], colors[i]);
                    }
                }
            }
        }
        if contours > 0.001 || wire {
            let _width = LineWidth::set((pixel_scale * 0.8).max(1.0));
            let mut batch = Batch::begin(raylib_sys::RL_LINES);
            // Engraving follows the long axis of each spectral ribbon. A handful
            // of delicate material seams, never a crosshatched polygon grid.
            let step = if wire { 3 } else { (13 / detail).max(4) };
            for k in (1..ACROSS - 1).step_by(step) {
                for row in 0..slices.len() - 1 {
                    let a = row * ACROSS + k;
                    let b = a + ACROSS;
                    let light = core_atlas::tideline_light(
                        progress - row as f32 / (slices.len() - 1) as f32,
                    );
                    let flow = row_motion(
                        frame.time_seconds,
                        row as f32 / (slices.len() - 1) as f32,
                        energy,
                        bass,
                    )
                    .light;
                    let tint = draw::color_alpha(
                        mix_color(
                            hue_shift(Color::new(155, 199, 173, 255), shift),
                            flare,
                            light,
                        ),
                        if wire {
                            0.35 + 0.40 * light + 0.20 * flow
                        } else {
                            (0.08 + 0.14 * light + 0.10 * flow) * contours
                        },
                    );
                    let mut p = points[a];
                    p.y += 0.008;
                    let mut q = points[b];
                    q.y += 0.008;
                    batch.vertex(p, tint);
                    batch.vertex(q, tint);
                }
            }
        }
    }
    drop(viewport);
    draw::vignette(d, boundary, 0.22);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion_vertex() -> ReliefVertex {
        ReliefVertex {
            point: Vector3::new(3.0, 0.2, 1.0),
            amplitude: 0.6,
            across: 0.5,
        }
    }

    fn separation(a: Vector3, b: Vector3) -> f32 {
        let difference = delta(a, b);
        dot(difference, difference).sqrt()
    }

    #[test]
    fn cached_relief_moves_visibly_within_one_second() {
        // The same rest vertex is retained across frames. Motion must survive
        // a cache hit, at a timescale a listener can actually notice.
        let vertex = motion_vertex();
        let first = row_motion(0.0, 0.2, 0.8, 0.6);
        let next = row_motion(1.0, 0.2, 0.8, 0.6);
        assert!(
            separation(
                animated_point(vertex, first, 0.5),
                animated_point(vertex, next, 0.5)
            ) > 0.15
        );
        assert!((first.light - next.light).abs() > 0.5);
        assert!((camera_azimuth(3.0, 0.0, 1.0) - camera_azimuth(0.0, 0.0, 1.0)) > 0.1);
        assert_eq!(camera_azimuth(3.0, 0.4, 0.0), 0.4);
    }

    #[test]
    fn music_changes_the_surface_not_only_its_brightness() {
        let vertex = motion_vertex();
        let quiet = row_motion(0.8, 0.2, 0.0, 0.0);
        // At an individual instant a downward fold can cancel the extra
        // spectral lift. Measure the change over motion, not that crossing.
        let average_change = (0..60)
            .map(|step| {
                let time = f64::from(step) * 0.05;
                separation(
                    animated_point(vertex, row_motion(time, 0.2, 0.0, 0.0), 0.0),
                    animated_point(vertex, row_motion(time, 0.2, 1.0, 1.0), 1.0),
                )
            })
            .sum::<f32>()
            / 60.0;
        assert!(average_change > 0.10);
        assert!(
            separation(
                animated_point(vertex, quiet, 0.0),
                animated_point(vertex, quiet, 1.0)
            ) > 0.10,
            "frequency energy must change the cross-section"
        );
    }

    #[test]
    fn folds_are_continuous_and_seek_repeatable() {
        let vertex = motion_vertex();
        for time in [0.0, 1.0, std::f64::consts::TAU / 1.35, 3600.0] {
            let point = |t| animated_point(vertex, row_motion(t, 0.2, 0.8, 0.6), 0.5);
            assert!(separation(point(time - 0.00001), point(time + 0.00001)) < 0.0001);
            let first = point(time);
            let _ = point(time + 600.0);
            assert_eq!(first, point(time));
        }
    }

    #[test]
    fn geometry_cache_tracks_audio_content_and_relief_controls() {
        let mut slices = vec![Slice::ZERO; 24];
        let first = geometry_identity(&slices, 7, 1.0, 1.0, 1.0);
        slices[10].bands[4] = 0.8;
        assert_ne!(geometry_identity(&slices, 7, 1.0, 1.0, 1.0), first);
        let (quiet, _) = build_geometry(&vec![Slice::ZERO; 24], 7, 1.0, 1.0, 1.0);
        let (sound, _) = build_geometry(&slices, 7, 1.0, 1.0, 1.0);
        assert!(
            quiet
                .iter()
                .zip(sound)
                .any(|(a, b)| (a.point.y - b.point.y).abs() > 0.001),
            "the chart must encode the spectrum, not just draw a seeded spiral"
        );
        assert_ne!(
            geometry_identity(&slices, 7, 1.0, 1.0, 1.0),
            geometry_identity(&slices, 7, 1.4, 1.0, 1.0)
        );
    }

    #[test]
    fn silent_and_constant_maps_make_finite_geometry() {
        for level in [0.0, 1.0] {
            let slices = vec![
                Slice {
                    bands: [level; BAND_COUNT],
                    rms: level,
                    flux: 0.0,
                    onset: false
                };
                24
            ];
            let (vertices, colors) = build_geometry(&slices, 7, 1.0, 1.0, 1.0);
            assert_eq!(vertices.len(), 24 * ACROSS);
            assert_eq!(colors.len(), vertices.len());
            assert!(vertices
                .iter()
                .all(|v| v.point.x.is_finite() && v.point.y.is_finite() && v.point.z.is_finite()));
        }
    }
}
