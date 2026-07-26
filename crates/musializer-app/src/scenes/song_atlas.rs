//! Song Atlas: the drawing half.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_song_atlas.c`'s `draw`
//! (frozen at `9300af9`, read-only). The live ring, the map's validity and
//! interpolation, and the render-sampling arithmetic are all in
//! `musializer_core::scenes::song_atlas`.
//!
//! A lit heightfield of the song's spectrum, flown over from above: frequency
//! across, time into the distance, amplitude as terrain height. Two surface
//! sources with the same geometry — the whole-song map when there is one, the
//! rolling live ring when there is not — plus a batched contour pass that makes it
//! read as a survey map rather than a hillside.
//!
//! ## Why this uses raw rlgl
//!
//! The terrain is thousands of triangles per frame and hundreds of contour lines.
//! C batches them through `rlBegin(RL_TRIANGLES)`/`rlVertex3f`, and the safe
//! raylib API has no equivalent — `draw_triangle_3D` would issue each triangle
//! separately. [`Batch`] wraps the immediate-mode calls so the `rlEnd` cannot be
//! forgotten.
//!
//! Formulas and draw-call order are kept recognizable against the C on purpose.

// Nothing dispatches this yet: `main.rs` still calls Spectrum directly and the
// scene registry that will call all ten is the integration owner's. Remove this
// allow once the frame loop dispatches by `SceneId` — every item here is reached
// from `draw`.
#![allow(dead_code)]

use musializer_core::scene::settings::index::atlas as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::song_atlas::{
    render_distance, render_sample_count, render_sample_index, Slice, SongAtlasMap, SongAtlasState,
    BAND_COUNT, BASE_SLICES, MAX_DETAIL, SLICE_SPACING,
};
use musializer_runtime::draw;
use raylib::prelude::{
    Camera3D, Color, RaylibDraw, RaylibDrawHandle, RaylibMode3DExt, Rectangle, Vector3,
};

use musializer_core::scenes::song_atlas as core_atlas;
use musializer_runtime::draw::SceneViewport;

const PI: f32 = std::f32::consts::PI;
const DEG2RAD: f32 = PI / 180.0;

/// The terrain palette (`scene_song_atlas.c:228-231`): bass through treble, with a
/// warm summit for the peaks.
const BASS: Color = Color::new(20, 72, 156, 255);
const MIDDLE: Color = Color::new(42, 205, 184, 255);
const TREBLE: Color = Color::new(210, 73, 176, 255);
const SUMMIT: Color = Color::new(255, 224, 172, 255);
/// The warm survey line drawn on a latched onset.
const LANDMARK: Color = Color::new(255, 219, 150, 255);
/// The teal frequency meridian.
const MERIDIAN: Color = Color::new(93, 219, 205, 255);

fn clamp01(value: f32) -> f32 {
    // `atlas_clamp01` (`scene_song_atlas.c:32-37`); no `isfinite` test, as in C.
    if value < 0.0 {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// An open rlgl immediate-mode batch. `rlEnd` runs on drop.
///
/// This is the one place in Agent C's files that issues vertices directly. Like
/// [`SceneViewport`], it is machinery that belongs in `musializer_runtime::draw`
/// and is here only because that crate is not Agent C's to edit.
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

/// One vertex of the heightfield (`atlas_vertex`, `scene_song_atlas.c:184-195`).
///
/// `terrain_profile` is the reason the terrain has a ridge rather than a flat
/// plateau: the mid frequencies get up to 2.35 units of height and the extremes
/// only 0.72, so the surface arches across the frequency axis.
fn vertex(slice: &Slice, band: usize, age: f32, scroll_phase: f32) -> Vector3 {
    let across = band as f32 / (BAND_COUNT - 1) as f32;
    let amplitude = slice.bands[band];
    let terrain_profile = 0.42 + 0.58 * (across * PI).sin();
    Vector3::new(
        (across - 0.5) * 9.4,
        -1.68 + amplitude * (0.72 + terrain_profile * 2.35),
        1.40 - (age + scroll_phase) * SLICE_SPACING,
    )
}

/// `atlas_live_vertex` (`scene_song_atlas.c:197-203`).
fn live_vertex(slice: &Slice, band: usize, source_age: usize, scroll_phase: f32) -> Vector3 {
    vertex(
        slice,
        band,
        render_distance(source_age as f32 + scroll_phase),
        0.0,
    )
}

/// `atlas_complete_vertex` (`scene_song_atlas.c:452-457`).
fn complete_vertex(slice: &Slice, band: usize, map_distance: f32) -> Vector3 {
    vertex(slice, band, render_distance(map_distance), 0.0)
}

/// `atlas_mix_color` (`scene_song_atlas.c:205-214`).
fn mix_color(from: Color, to: Color, amount: f32) -> Color {
    let amount = clamp01(amount);
    let channel =
        |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount) as u8;
    Color::new(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
        channel(from.a, to.a),
    )
}

/// `atlas_hue_shift` (`scene_song_atlas.c:216-223`): rotates hue, preserving alpha.
fn hue_shift(color: Color, color_shift: f32) -> Color {
    let hsv = color_to_hsv(color);
    let mut shifted = draw::color_from_hsv((hsv.x + color_shift + 720.0) % 360.0, hsv.y, hsv.z);
    shifted.a = color.a;
    shifted
}

/// `atlas_color` (`scene_song_atlas.c:225-245`).
///
/// Frequency picks the base hue, amplitude and flux blend toward the summit, and
/// `age` darkens with distance — so the near terrain is exposed and the far
/// terrain falls away into the background instead of needing a fog plane.
fn terrain_color(slice: &Slice, band: usize, age: f32, color_shift: f32) -> Color {
    let across = band as f32 / (BAND_COUNT - 1) as f32;
    let frequency = if across < 0.5 {
        mix_color(BASS, MIDDLE, across * 2.0)
    } else {
        mix_color(MIDDLE, TREBLE, (across - 0.5) * 2.0)
    };
    let amplitude = slice.bands[band];
    let mut color = mix_color(
        frequency,
        SUMMIT,
        clamp01(amplitude * 0.52 + slice.flux * 0.14),
    );
    let depth = 1.0 - age / (BASE_SLICES - 1) as f32;
    let exposure = 0.12 + depth * 0.68 + amplitude * 0.24;
    let scale = clamp01(exposure);
    color.r = (f32::from(color.r) * scale) as u8;
    color.g = (f32::from(color.g) * scale) as u8;
    color.b = (f32::from(color.b) * scale) as u8;
    hue_shift(color, color_shift)
}

/// `atlas_light_color` (`scene_song_atlas.c:253-263`).
fn light_color(color: Color, light: f32) -> Color {
    let light = 0.26 + clamp01(light) * 0.86;
    Color::new(
        (f32::from(color.r) * light).min(255.0) as u8,
        (f32::from(color.g) * light).min(255.0) as u8,
        (f32::from(color.b) * light).min(255.0) as u8,
        color.a,
    )
}

/// `atlas_triangle_light` (`scene_song_atlas.c:265-285`).
///
/// The absolute dot product, so a triangle facing away is lit like one facing
/// toward: the terrain is a single-sided surface and a signed test would leave
/// half of it black. A degenerate triangle takes a neutral 0.45.
fn triangle_light(a: Vector3, b: Vector3, c: Vector3, light_direction: Vector3) -> f32 {
    let first = Vector3::new(b.x - a.x, b.y - a.y, b.z - a.z);
    let second = Vector3::new(c.x - a.x, c.y - a.y, c.z - a.z);
    let normal = Vector3::new(
        first.y * second.z - first.z * second.y,
        first.z * second.x - first.x * second.z,
        first.x * second.y - first.y * second.x,
    );
    let normal_length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    let light_length = (light_direction.x * light_direction.x
        + light_direction.y * light_direction.y
        + light_direction.z * light_direction.z)
        .sqrt();
    if normal_length <= 0.00001 || light_length <= 0.00001 {
        return 0.45;
    }
    let dot = (normal.x * light_direction.x
        + normal.y * light_direction.y
        + normal.z * light_direction.z)
        / (normal_length * light_length);
    clamp01(dot.abs())
}

/// `atlas_lit_triangle` (`scene_song_atlas.c:287-295`): one flat light per
/// triangle, applied to all three vertex colours.
#[allow(clippy::too_many_arguments)]
fn lit_triangle(
    batch: &mut Batch,
    a: Vector3,
    b: Vector3,
    c: Vector3,
    ca: Color,
    cb: Color,
    cc: Color,
    light_direction: Vector3,
) {
    let light = triangle_light(a, b, c, light_direction);
    batch.vertex(a, light_color(ca, light));
    batch.vertex(b, light_color(cb, light));
    batch.vertex(c, light_color(cc, light));
}

/// The live fallback surface (`atlas_draw_surface`, `scene_song_atlas.c:297-404`).
#[allow(clippy::too_many_arguments)]
fn draw_live_surface(
    atlas: &SongAtlasState,
    scroll_phase: f32,
    pixel_scale: f32,
    contour_scale: f32,
    color_shift: f32,
    wireframe: bool,
    detail_level: usize,
    light_direction: Vector3,
) {
    if atlas.count() < 2 {
        return;
    }
    let available = atlas.count();
    let render_count = render_sample_count(available, detail_level);
    if render_count < 2 {
        return;
    }
    // `age` counts backward from the newest slice, so the ring index is
    // `count - age - 1`.
    let slice_at = |age: usize| atlas.slice(available - age - 1);

    if !wireframe {
        let mut batch = Batch::begin(raylib_sys::RL_TRIANGLES);
        for sample in 1..render_count {
            let Some(newer_age) = render_sample_index(0, available, render_count, sample - 1)
            else {
                continue;
            };
            let Some(older_age) = render_sample_index(0, available, render_count, sample) else {
                continue;
            };
            let (Some(newer), Some(older)) = (slice_at(newer_age), slice_at(older_age)) else {
                continue;
            };
            for band in 0..BAND_COUNT - 1 {
                let a = live_vertex(newer, band, newer_age, scroll_phase);
                let b = live_vertex(newer, band + 1, newer_age, scroll_phase);
                let c = live_vertex(older, band, older_age, scroll_phase);
                let dd = live_vertex(older, band + 1, older_age, scroll_phase);
                let newer_depth = render_distance(newer_age as f32);
                let older_depth = render_distance(older_age as f32);
                let ca = terrain_color(newer, band, newer_depth, color_shift);
                let cb = terrain_color(newer, band + 1, newer_depth, color_shift);
                let cc = terrain_color(older, band, older_depth, color_shift);
                let cd = terrain_color(older, band + 1, older_depth, color_shift);
                // Counter-clockwise from above: the atlas camera lives above the
                // heightfield, so upward-facing terrain must survive back-face
                // culling on every OpenGL target.
                lit_triangle(&mut batch, a, b, c, ca, cb, cc, light_direction);
                lit_triangle(&mut batch, b, dd, c, cb, cd, cc, light_direction);
            }
        }
    }

    if contour_scale <= 0.001 {
        return;
    }

    // A single batched contour pass replaces hundreds of tiny cylinders. The
    // fixed-cadence cross-lines and sparse frequency meridians read as a map;
    // latched onsets become warm survey lines instead of arbitrary cubes.
    let _line_width = LineWidth::set(1.0f32.max(pixel_scale * contour_scale));
    let mut batch = Batch::begin(raylib_sys::RL_LINES);
    let row_step = if wireframe { 1usize } else { 5 };
    let band_step = if wireframe { 1usize } else { 5 };
    let mut sample = 0usize;
    while sample < render_count {
        if let Some(source_age) = render_sample_index(0, available, render_count, sample) {
            if let Some(slice) = slice_at(source_age) {
                let line = draw::color_alpha(
                    Color::RAYWHITE,
                    (if wireframe { 0.16 } else { 0.08 })
                        + 0.22 * (1.0 - sample as f32 / render_count as f32),
                );
                for band in 0..BAND_COUNT - 1 {
                    batch.vertex(live_vertex(slice, band, source_age, scroll_phase), line);
                    batch.vertex(live_vertex(slice, band + 1, source_age, scroll_phase), line);
                }
            }
        }
        sample += row_step;
    }
    let mut band = if wireframe { 0usize } else { 2 };
    while band + 1 < BAND_COUNT {
        let line = draw::color_alpha(
            hue_shift(MERIDIAN, color_shift),
            if wireframe { 0.32 } else { 0.18 },
        );
        for sample in 1..render_count {
            let Some(newer_age) = render_sample_index(0, available, render_count, sample - 1)
            else {
                continue;
            };
            let Some(older_age) = render_sample_index(0, available, render_count, sample) else {
                continue;
            };
            let (Some(newer), Some(older)) = (slice_at(newer_age), slice_at(older_age)) else {
                continue;
            };
            batch.vertex(live_vertex(newer, band, newer_age, scroll_phase), line);
            batch.vertex(live_vertex(older, band, older_age, scroll_phase), line);
        }
        band += band_step;
    }
    for sample in 0..render_count {
        let Some(source_age) = render_sample_index(0, available, render_count, sample) else {
            continue;
        };
        let Some(slice) = slice_at(source_age) else {
            continue;
        };
        if !slice.onset {
            continue;
        }
        let landmark =
            draw::color_alpha(hue_shift(LANDMARK, color_shift), 0.24 + slice.flux * 0.52);
        for band in 0..BAND_COUNT - 1 {
            let mut a = live_vertex(slice, band, source_age, scroll_phase);
            let mut b = live_vertex(slice, band + 1, source_age, scroll_phase);
            // Lifted clear of the surface so the survey line is not z-fought by
            // the terrain it marks.
            a.y += 0.025;
            b.y += 0.025;
            batch.vertex(a, landmark);
            batch.vertex(b, landmark);
        }
    }
}

/// `atlas_map_playhead_vertex` (`scene_song_atlas.c:432-450`).
///
/// The playhead sits *between* two slices, so its vertex is the interpolation of
/// the same band on both, each offset by its own fractional distance.
fn playhead_vertex(map: &SongAtlasMap, playhead: f32, band: usize) -> Vector3 {
    let slices = map.slices();
    let lower = playhead.floor() as usize;
    let last = slices.len() - 1;
    if lower >= last {
        return vertex(&slices[last], band, 0.0, 0.0);
    }
    let amount = playhead - lower as f32;
    let a = vertex(&slices[lower], band, -amount / MAX_DETAIL as f32, 0.0);
    let b = vertex(
        &slices[lower + 1],
        band,
        (1.0 - amount) / MAX_DETAIL as f32,
        0.0,
    );
    Vector3::new(
        a.x + (b.x - a.x) * amount,
        a.y + (b.y - a.y) * amount,
        a.z + (b.z - a.z) * amount,
    )
}

/// The whole-song surface (`atlas_draw_complete_surface`,
/// `scene_song_atlas.c:459-555`).
#[allow(clippy::too_many_arguments)]
fn draw_complete_surface(
    map: &SongAtlasMap,
    time_seconds: f64,
    pixel_scale: f32,
    contour_scale: f32,
    color_shift: f32,
    wireframe: bool,
    detail_level: usize,
    light_direction: Vector3,
) {
    if !map.is_valid() {
        return;
    }
    let slices = map.slices();
    let playhead = core_atlas::map_playhead(map, time_seconds);
    // Ten slices' worth of already-played terrain is kept behind the playhead, so
    // the recent past is visible without drawing the whole song every frame.
    let history = 10 * MAX_DETAIL;
    let first = if playhead > history as f32 {
        playhead.floor() as usize - history
    } else {
        0
    };
    let available = slices.len() - first;
    let sample_count = render_sample_count(available, detail_level);
    if sample_count < 2 {
        return;
    }

    if !wireframe {
        let mut batch = Batch::begin(raylib_sys::RL_TRIANGLES);
        for sample in 0..sample_count - 1 {
            let Some(row) = render_sample_index(first, available, sample_count, sample) else {
                continue;
            };
            let Some(next_row) = render_sample_index(first, available, sample_count, sample + 1)
            else {
                continue;
            };
            let near = &slices[row];
            let far = &slices[next_row];
            let near_distance = row as f32 - playhead;
            let far_distance = next_row as f32 - playhead;
            let near_depth = near_distance.max(0.0) / MAX_DETAIL as f32;
            let far_depth = far_distance.max(0.0) / MAX_DETAIL as f32;
            for band in 0..BAND_COUNT - 1 {
                let a = complete_vertex(near, band, near_distance);
                let b = complete_vertex(near, band + 1, near_distance);
                let c = complete_vertex(far, band, far_distance);
                let dd = complete_vertex(far, band + 1, far_distance);
                let ca = terrain_color(near, band, near_depth, color_shift);
                let cb = terrain_color(near, band + 1, near_depth, color_shift);
                let cc = terrain_color(far, band, far_depth, color_shift);
                let cd = terrain_color(far, band + 1, far_depth, color_shift);
                lit_triangle(&mut batch, a, b, c, ca, cb, cc, light_direction);
                lit_triangle(&mut batch, b, dd, c, cb, cd, cc, light_direction);
            }
        }
    }

    if contour_scale <= 0.001 {
        return;
    }

    let _line_width = LineWidth::set(1.0f32.max(pixel_scale * contour_scale));
    let mut batch = Batch::begin(raylib_sys::RL_LINES);
    for sample in 0..sample_count {
        let Some(row) = render_sample_index(first, available, sample_count, sample) else {
            continue;
        };
        let distance = row as f32 - playhead;
        // Solid mode draws every eighth cross-line plus every onset; wireframe
        // draws them all.
        if !wireframe && row % 8 != 0 && !slices[row].onset {
            continue;
        }
        let line = if slices[row].onset {
            draw::color_alpha(
                hue_shift(LANDMARK, color_shift),
                0.28 + slices[row].flux * 0.50,
            )
        } else {
            draw::color_alpha(Color::RAYWHITE, 0.12)
        };
        for band in 0..BAND_COUNT - 1 {
            batch.vertex(complete_vertex(&slices[row], band, distance), line);
            batch.vertex(complete_vertex(&slices[row], band + 1, distance), line);
        }
    }
    let band_step = if wireframe { 1usize } else { 5 };
    let mut band = if wireframe { 0usize } else { 2 };
    while band + 1 < BAND_COUNT {
        let line = draw::color_alpha(
            hue_shift(MERIDIAN, color_shift),
            if wireframe { 0.30 } else { 0.16 },
        );
        for sample in 0..sample_count - 1 {
            let Some(row) = render_sample_index(first, available, sample_count, sample) else {
                continue;
            };
            let Some(next_row) = render_sample_index(first, available, sample_count, sample + 1)
            else {
                continue;
            };
            let near_distance = row as f32 - playhead;
            let far_distance = next_row as f32 - playhead;
            batch.vertex(complete_vertex(&slices[row], band, near_distance), line);
            batch.vertex(complete_vertex(&slices[next_row], band, far_distance), line);
        }
        band += band_step;
    }
    // The playhead itself, in warm white, drawn last so it sits on top.
    let playhead_color = Color::new(255, 238, 196, 255);
    for band in 0..BAND_COUNT - 1 {
        batch.vertex(playhead_vertex(map, playhead, band), playhead_color);
        batch.vertex(playhead_vertex(map, playhead, band + 1), playhead_color);
    }
}

/// Draws Song Atlas into `boundary`.
///
/// `map` is the whole-track analysis prepared when the track loaded
/// (`renderer->song_atlas_map`, `scene.h:62`). An absent or invalid map falls back
/// to the live ring in `state`.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    state: &SongAtlasState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
    map: Option<&SongAtlasMap>,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }
    let energy = state.camera_energy();
    let flux = state.camera_flux();
    let height_scale = frame.setting(SceneId::SongAtlas, setting::HEIGHT);
    let width_scale = frame.setting(SceneId::SongAtlas, setting::WIDTH);
    let depth_scale = frame.setting(SceneId::SongAtlas, setting::DEPTH);
    let camera_scale = frame.setting(SceneId::SongAtlas, setting::CAMERA);
    let contour_scale = frame.setting(SceneId::SongAtlas, setting::CONTOURS);
    let mut color_shift = frame.setting(SceneId::SongAtlas, setting::COLOR);
    let drift_scale = frame.setting(SceneId::SongAtlas, setting::SPEED);
    let orbit_degrees = frame.setting(SceneId::SongAtlas, setting::ORBIT);
    let zoom_scale = frame.setting(SceneId::SongAtlas, setting::ZOOM);
    let wireframe = frame.setting(SceneId::SongAtlas, setting::WIREFRAME) >= 0.5;
    let detail_level =
        (frame.setting(SceneId::SongAtlas, setting::DETAIL).round() as usize).clamp(1, MAX_DETAIL);
    let hue_motion = frame.setting(SceneId::SongAtlas, setting::HUE_MOTION) >= 0.5;

    // A valid map is the source of the hue's dynamics too, so the colour sweep
    // follows the song's own shape rather than the preview analyzer's smoothing.
    let valid_map = map.filter(|map| map.is_valid());
    if hue_motion {
        let mut hue_energy = frame.audio.rms;
        let mut hue_flux = frame.audio.spectral_flux;
        if let Some(map) = valid_map {
            let playhead = core_atlas::map_playhead(map, frame.time_seconds);
            if let Some((energy, flux)) = core_atlas::map_dynamics(map, playhead) {
                hue_energy = energy;
                hue_flux = flux;
            }
        }
        let hue_energy = clamp01(hue_energy);
        let hue_flux = clamp01(hue_flux);
        let hue_wave = (frame.time_seconds as f32 * (0.70 + hue_energy * 0.55)).sin();
        color_shift += (frame.time_seconds as f32 * 12.0
            + hue_wave * (14.0 + hue_energy * 34.0)
            + hue_flux * 82.0)
            % 360.0;
    }
    let seed_phase = musializer_core::scenes::song_atlas::hash_unit(state.seed(), 7) * 2.0 * PI;
    let semantic_weight = if frame.semantic.available {
        frame.semantic.confidence
    } else {
        0.0
    };
    let hue = (205.0
        + musializer_core::scenes::song_atlas::hash_unit(state.seed(), 2) * 100.0
        + frame.semantic.valence * 70.0 * semantic_weight
        + color_shift
        + 720.0)
        % 360.0;
    let background = draw::color_from_hsv(hue, 0.70, 0.038 + energy * 0.025);
    let horizon = draw::color_from_hsv((hue + 24.0) % 360.0, 0.64, 0.075 + energy * 0.035);
    d.draw_rectangle_gradient_v(
        boundary.x as i32,
        boundary.y as i32,
        boundary.width as i32,
        boundary.height as i32,
        background,
        horizon,
    );

    let screen_width = d.get_screen_width();
    let screen_height = d.get_screen_height();
    // Declared before the 3D handle so `EndMode3D` runs before the viewport is
    // restored, matching the C's order.
    let Some(viewport) = SceneViewport::begin_with_screen(boundary, screen_width, screen_height)
    else {
        return;
    };

    let journey = frame.time_seconds as f32 * 0.025 * drift_scale + seed_phase;
    let mut target_z = -5.4f32;
    if let Some(map) = valid_map {
        // With a map the camera looks a bounded distance ahead of the playhead
        // rather than at a fixed point, so the framing tightens toward the end of
        // the song instead of staring past it.
        let playhead = core_atlas::map_playhead(map, frame.time_seconds);
        let remaining = ((map.len() - 1) as f32 - playhead) / MAX_DETAIL as f32;
        let focus_slices = (remaining * 0.45).clamp(4.0, 18.0);
        target_z = 1.40 - focus_slices * SLICE_SPACING;
    }
    target_z *= depth_scale;
    let mut camera = Camera3D::perspective(
        Vector3::new(
            journey.sin() * 0.52,
            (4.62 + (journey * 0.47).cos() * 0.14 + energy * 0.20) * camera_scale,
            7.35,
        ),
        Vector3::new(
            (journey * 0.31).sin() * 0.34,
            -0.58 * height_scale,
            target_z,
        ),
        Vector3::new(0.0, 1.0, 0.0),
        49.0 + flux * 2.2,
    );

    // Orbit rotates the vantage horizontally around the focus point; distance
    // dollies it radially in or out. The solid terrain is z-buffered, so any
    // azimuth reads cleanly, and orbit 0 / distance 1 leaves the framing intact.
    let orbit = orbit_degrees * DEG2RAD;
    let off_x = camera.position.x - camera.target.x;
    let off_y = camera.position.y - camera.target.y;
    let off_z = camera.position.z - camera.target.z;
    let orbit_cos = orbit.cos();
    let orbit_sin = orbit.sin();
    camera.position.x = camera.target.x + (off_x * orbit_cos + off_z * orbit_sin) * zoom_scale;
    camera.position.y = camera.target.y + off_y * zoom_scale;
    camera.position.z = camera.target.z + (-off_x * orbit_sin + off_z * orbit_cos) * zoom_scale;

    {
        let _space = d.begin_mode3D(camera);
        viewport.correct_projection_aspect();
        // The world itself is scaled, not the vertices: one model-view scale keeps
        // the terrain arithmetic identical whatever the width/height/depth dials
        // say.
        viewport.scale_modelview(width_scale, height_scale, depth_scale);

        // A slowly circling key light, so the terrain's relief changes over the
        // song instead of being lit identically for an hour.
        let light_direction = Vector3::new(
            (journey * 0.73 + seed_phase).cos() * 0.42,
            0.78 + energy * 0.20,
            (journey * 0.73 + seed_phase).sin() * 0.42,
        );

        match valid_map {
            Some(map) => draw_complete_surface(
                map,
                frame.time_seconds,
                pixel_scale,
                contour_scale,
                color_shift,
                wireframe,
                detail_level,
                light_direction,
            ),
            None => draw_live_surface(
                state,
                state.scroll_phase(frame.time_seconds),
                pixel_scale,
                contour_scale,
                color_shift,
                wireframe,
                detail_level,
                light_direction,
            ),
        }
    }
    drop(viewport);

    d.draw_rectangle_rec(boundary, draw::color_alpha(background, 0.045));
}
