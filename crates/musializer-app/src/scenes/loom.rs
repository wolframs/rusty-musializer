//! Loom: the drawing half.
//!
//! The woven record and the cloth's structural rules are in
//! `musializer_core::scenes::loom`.
//!
//! **The cloth filling only part of the stage is not a framing bug.**
//! `../musializer/tools/UI_REVIEW.md` records this: the weave is revealed in
//! proportion to elapsed track time, so a capture at 15% of the track shows 15% of
//! the settled cloth. Loose warp and weft keep the future side alive without
//! pretending that part of the song has already been woven.
//!
//! Draw order, which is the composition: background, cloth backing, bare warp ahead
//! of the fell, woven warp, weft picks with their shadows, interlace stubs, the fell
//! itself, additive glints, cloth outline.

#![allow(dead_code)]

use musializer_core::scene::settings::index::loom as setting;
use musializer_core::scene::{SceneFrame, SceneId, SemanticFrame};
use musializer_core::scenes::loom::{self as loom, weave, LoomState, WeaveColumn, MAX_COLUMNS};
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, Rectangle, Vector2,
};

const PI: f32 = std::f32::consts::PI;

fn thread_color(semantic: SemanticFrame, saturation_scale: f32, brightness: f32) -> Color {
    let (hue, saturation, value) = loom::thread_hsv(semantic, saturation_scale, brightness);
    draw::color_from_hsv(hue, saturation, value)
}

/// Draws Loom into `boundary`.
#[allow(clippy::needless_range_loop)] // Row and column indices drive the weave
                                      // pattern and the crimp phase, so the index loops are what make this diffable.
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    frame: &SceneFrame<'_>,
    state: &LoomState,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    if boundary.width <= 1.0 || boundary.height <= 1.0 {
        return;
    }
    let weave_state = &state.weave;

    let density_scale = frame.setting(SceneId::Loom, setting::DENSITY);
    let weight_scale = frame.setting(SceneId::Loom, setting::WEIGHT);
    let complexity_scale = frame.setting(SceneId::Loom, setting::COMPLEXITY);
    let edge_scale = frame.setting(SceneId::Loom, setting::EDGE);
    let saturation_scale = frame.setting(SceneId::Loom, setting::SATURATION);
    let motion_scale = frame.setting(SceneId::Loom, setting::MOTION);
    let glint_scale = frame.setting(SceneId::Loom, setting::GLINTS);
    let pixel_scale = if pixel_scale > 0.0 { pixel_scale } else { 1.0 };

    // With no known duration the cloth still has to be finite, so it pretends the
    // track is one second longer than the playhead.
    let duration = if frame.duration_seconds > 0.0 {
        frame.duration_seconds
    } else {
        1.0f64.max(frame.time_seconds + 1.0)
    };
    let progress = loom::clamp01((frame.time_seconds / duration) as f32);
    let cloth = Rectangle::new(
        boundary.x + boundary.width * 0.055,
        boundary.y + boundary.height * 0.07,
        boundary.width * 0.89,
        boundary.height * 0.86,
    );
    let current = loom::semantic_at(frame, weave_state, frame.time_seconds);
    let background = draw::color_from_hsv(224.0, 0.48, 0.025 + current.energy * 0.045);
    draw::atmospheric_backdrop(
        d,
        boundary,
        background,
        draw::color_from_hsv(246.0, 0.52, 0.055 + current.energy * 0.045),
        Vector2::new(
            boundary.x + boundary.width * 0.38,
            boundary.y + boundary.height * 0.50,
        ),
        boundary.width.max(boundary.height) * 0.52,
        draw::color_alpha(thread_color(current, saturation_scale, 0.38), 0.18),
    );
    d.draw_rectangle_rec(cloth, draw::color_alpha(Color::new(13, 14, 28, 255), 0.86));

    let (columns, rows) = loom::dimensions(density_scale);
    let visible_columns = ((columns as f32 * progress).ceil() as usize).min(columns);
    let woven_width = cloth.width * progress;
    let frontier_x = cloth.x + woven_width;
    let onset_pulse = loom::clamp01(weave_state.onset_pulse);

    // Each column is a fixed sample of the song at its own time, so the finished
    // tapestry is a stable record of the whole arc rather than of this frame.
    let mut column_semantic = [SemanticFrame::default(); MAX_COLUMNS];
    let mut column_record = [WeaveColumn::default(); MAX_COLUMNS];
    for column in 0..columns {
        let column_time = (column as f64 + 0.5) / columns as f64 * duration;
        column_semantic[column] = loom::semantic_at(frame, weave_state, column_time);
        column_record[column] = weave_state.columns[weave::slot(column_time, duration)];
    }

    // Bare warp waits ahead of the fell: pale, straight, uncoloured threads the song
    // has not woven yet.
    for column in visible_columns..columns {
        let x = cloth.x + ((column as f32 + 0.5) / columns as f32) * cloth.width;
        let shade = 0.30 + loom::unit(state.seed, column as u64 * 3 + 2) * 0.18;
        d.draw_line_ex(
            Vector2::new(x, cloth.y),
            Vector2::new(x, cloth.y + cloth.height),
            0.6 * pixel_scale * weight_scale,
            draw::color_alpha(draw::color_from_hsv(224.0, 0.10, shade), 0.30),
        );
    }
    // Loose future weft keeps the unwoven side feeling like material waiting at
    // the loom, not an empty progress-meter grid. It is deliberately faint and
    // slack; the settled cloth to the left remains the visual record.
    if visible_columns < columns {
        let future_width = cloth.x + cloth.width - frontier_x;
        for row in 0..rows {
            let base_y = cloth.y + ((row as f32 + 0.5) / rows as f32) * cloth.height;
            let phase = loom::unit(state.seed, row as u64 * 11 + 700) * 2.0 * PI
                + frame.time_seconds as f32 * 0.10 * motion_scale;
            let mut previous = Vector2::new(frontier_x, base_y);
            for segment in 1..=24 {
                let t = segment as f32 / 24.0;
                let point = Vector2::new(
                    frontier_x + future_width * t,
                    base_y
                        + (t * PI).sin()
                            * (2.0 + 4.0 * (phase + row as f32 * 0.17).sin())
                            * pixel_scale,
                );
                d.draw_line_ex(
                    previous,
                    point,
                    (0.45 * pixel_scale * weight_scale).max(0.35),
                    draw::color_alpha(
                        draw::color_from_hsv(224.0 + row as f32 * 0.45, 0.18, 0.42),
                        0.14,
                    ),
                );
                previous = point;
            }
        }
    }

    // Woven warp: swaying segmented threads with crimp at every weft row. Fresh warp
    // near the fell still trembles with the live band its height maps to; settled
    // cloth further back lies still.
    for column in 0..visible_columns {
        let semantic = column_semantic[column];
        let confidence = if semantic.available {
            loom::clamp01(semantic.confidence)
        } else {
            0.0
        };
        let complexity = (0.55 + semantic.tension * 1.45) * complexity_scale;
        let x = cloth.x + ((column as f32 + 0.5) / columns as f32) * cloth.width;
        let proximity = loom::clamp01(1.0 - (frontier_x - x) / (cloth.width * 0.12));
        let phase = loom::unit(state.seed, column as u64 + 1) * 2.0 * PI;
        let color = thread_color(
            semantic,
            saturation_scale,
            0.32 + semantic.energy * 0.56 + confidence * 0.08,
        );
        let mut previous = Vector2::new(x, cloth.y);
        for segment in 1..=loom::WARP_SEGMENTS {
            let y_t = segment as f32 / loom::WARP_SEGMENTS as f32;
            let live = weave::profile_sample(&weave_state.profile, loom::band_t(y_t));
            let mut lift =
                (y_t * PI * complexity * 2.0 + phase + frame.audio.beat_phase * PI * motion_scale)
                    .sin()
                    * (1.0 + semantic.tension * 3.6)
                    * pixel_scale;
            lift += (frame.time_seconds as f32 * 24.0 + phase * 3.0 + y_t * 14.0).sin()
                * live
                * proximity
                * (2.4 + onset_pulse * 2.2)
                * pixel_scale
                * motion_scale;
            let point = Vector2::new(x + lift, cloth.y + y_t * cloth.height);
            let thickness = (0.45 + semantic.energy * 1.25)
                * (1.0 + live * proximity * 0.35)
                * pixel_scale
                * weight_scale
                * loom::crimp(y_t, rows as f32);
            d.draw_line_ex(
                previous,
                point,
                thickness,
                draw::color_alpha(color, 0.34 + confidence * 0.46),
            );
            previous = point;
        }
    }

    // Weft: each pick is drawn with a soft shadow under its lit face so the thread
    // reads as a rounded body catching light, not a flat stroke. Rows ride the
    // frequency band they map to: frozen from the column's record in settled cloth,
    // blending toward the live spectrum near the fell, and an onset beats a short
    // ripple back through the newest picks.
    if woven_width > 0.0 {
        for row in 0..rows {
            let y_t = (row as f32 + 0.5) / rows as f32;
            let band_t = loom::band_t(y_t);
            let live_band = weave::profile_sample(&weave_state.profile, band_t);
            let y = cloth.y + y_t * cloth.height;
            let mut previous = Vector2::new(cloth.x, y);
            for column in 1..=visible_columns {
                let x_t = column as f32 / columns as f32;
                let semantic = column_semantic[column - 1];
                let record = &column_record[column - 1];
                let mid_t = x_t - 0.5 / columns as f32;
                let px = cloth.x + woven_width.min(x_t * cloth.width);
                let frozen_band = if record.woven {
                    weave::profile_sample(&record.profile, band_t)
                } else {
                    live_band
                };
                let proximity = loom::clamp01(1.0 - (frontier_x - px) / (cloth.width * 0.14));
                let band = frozen_band + (live_band - frozen_band) * proximity;
                let ripple =
                    onset_pulse * (-(frontier_x - px).max(0.0) / (cloth.width * 0.06)).exp();
                let over = if (row + column) & 1 != 0 { 1.0 } else { -1.0 };
                let lift = over
                    * (0.7 + semantic.tension * 2.4)
                    * (1.0 + ripple * 1.2)
                    * pixel_scale
                    * complexity_scale;
                let point = Vector2::new(px, y + lift);
                let thickness = (0.55 + semantic.energy * 0.85 + band * 1.05)
                    * pixel_scale
                    * weight_scale
                    * loom::crimp(mid_t, columns as f32);
                let color = thread_color(
                    semantic,
                    saturation_scale,
                    0.36 + semantic.energy * 0.36 + band * 0.42 + ripple * 0.22,
                );
                let shadow = Vector2::new(0.6 * pixel_scale, 0.7 * pixel_scale);
                d.draw_line_ex(
                    Vector2::new(previous.x + shadow.x, previous.y + shadow.y),
                    Vector2::new(point.x + shadow.x, point.y + shadow.y),
                    thickness * 1.05,
                    draw::color_alpha(Color::BLACK, 0.30),
                );
                d.draw_line_ex(
                    previous,
                    point,
                    thickness,
                    draw::color_alpha(
                        color,
                        (0.42 + semantic.confidence * 0.30 + band * 0.16).min(0.9),
                    ),
                );
                previous = point;
                if point.x >= cloth.x + woven_width {
                    break;
                }
            }
        }
    }

    // Interlace: wherever the weave pattern binds warp over weft, a short bright stub
    // of the warp thread crosses back on top and catches the light. This is what makes
    // the grid read as cloth.
    let row_pitch = cloth.height / rows as f32;
    for column in 0..visible_columns {
        let semantic = column_semantic[column];
        let record = &column_record[column];
        let tension = loom::clamp01(semantic.tension);
        let x = cloth.x + ((column as f32 + 0.5) / columns as f32) * cloth.width;
        if x > cloth.x + woven_width {
            break;
        }
        let ripple = onset_pulse * (-(frontier_x - x).max(0.0) / (cloth.width * 0.06)).exp();
        let thickness = (0.50 + semantic.energy * 1.30) * pixel_scale * weight_scale;
        for row in 0..rows {
            if !loom::warp_over(row, column, tension) {
                continue;
            }
            let y_t = (row as f32 + 0.5) / rows as f32;
            let band_t = loom::band_t(y_t);
            let band = if record.woven {
                weave::profile_sample(&record.profile, band_t)
            } else {
                weave::profile_sample(&weave_state.profile, band_t)
            };
            let y = cloth.y + y_t * cloth.height;
            let half = row_pitch * 0.28;
            let lit = thread_color(
                semantic,
                saturation_scale,
                0.48 + semantic.energy * 0.44 + band * 0.26,
            );
            d.draw_line_ex(
                Vector2::new(x, y - half),
                Vector2::new(x, y + half),
                thickness,
                draw::color_alpha(
                    lit,
                    (0.40 + semantic.confidence * 0.30 + band * 0.18 + ripple * 0.30).min(0.9),
                ),
            );
        }
    }

    // The fell: a bright working edge where the next pick is beaten in, shimmering
    // with the beat and flashing as the reed strikes an onset.
    if progress < 1.0 {
        d.draw_rectangle_gradient_h(
            cloth.x.max(frontier_x - 22.0 * pixel_scale * edge_scale) as i32,
            cloth.y as i32,
            (22.0 * pixel_scale * edge_scale) as i32,
            cloth.height as i32,
            draw::color_alpha(background, 0.0),
            draw::color_alpha(
                thread_color(current, saturation_scale, 1.0),
                0.30 + onset_pulse * 0.20,
            ),
        );
        let shimmer = 0.30
            + 0.20 * motion_scale * (frame.audio.beat_phase * 2.0 * PI).sin()
            + onset_pulse * 0.38;
        d.draw_line_ex(
            Vector2::new(frontier_x, cloth.y),
            Vector2::new(frontier_x, cloth.y + cloth.height),
            ((1.6 + onset_pulse * 2.6) * pixel_scale).max(1.0),
            draw::color_alpha(
                thread_color(current, saturation_scale, 1.0),
                loom::clamp01(shimmer),
            ),
        );
    }

    // Glints land on the fabric structure itself: sequins at crossings, not sparks
    // floating over it. A burst keeps its positions for the life of one onset
    // envelope — that is what `onset_serial` is for — and fades with it instead of
    // teleporting every frame.
    let glow = onset_pulse.max(loom::clamp01((current.tension - 0.55) * 1.6));
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        if glow > 0.04 && visible_columns > 0 {
            let glints = ((5.0 + 8.0 * glow) * glint_scale).round() as i64;
            for i in 0..glints.max(0) as u64 {
                let salt = u64::from(weave_state.onset_serial)
                    .wrapping_mul(1_000_003)
                    .wrapping_add(i * 7);
                let column =
                    (loom::unit(state.seed, salt) * (visible_columns - 1) as f32 + 0.5) as usize;
                let row = (loom::unit(state.seed, salt + 1) * (rows - 1) as f32 + 0.5) as usize;
                let x = cloth.x + ((column as f32 + 0.5) / columns as f32) * cloth.width;
                let y = cloth.y + ((row as f32 + 0.5) / rows as f32) * cloth.height;
                if x > cloth.x + woven_width {
                    continue;
                }
                let color = thread_color(current, saturation_scale, 1.0);
                blend.draw_circle_v(
                    Vector2::new(x, y),
                    (0.9 + glow * 1.6 + current.tension * 1.2) * pixel_scale * glint_scale,
                    draw::color_alpha(color, 0.10 + glow * 0.30 + current.tension * 0.14),
                );
            }
        }
    });
    d.draw_rectangle_lines_ex(
        cloth,
        pixel_scale.max(1.0),
        draw::color_alpha(Color::RAYWHITE, 0.055),
    );
    draw::vignette(d, boundary, 0.18);
}
