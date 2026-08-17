//! Pulse Field: the drawing half.
//!
//! The deterministic half is `musializer_core::scenes::pulse_field`, and it is
//! one number.
//!
//! A stack of rose curves, drawn outermost first so the additive blend builds up
//! toward the centre. Everything animated is derived from `frame.time_seconds`,
//! not from accumulated state, which is what makes a seeked preview and an export
//! agree.
//!
//! A broad, low-alpha echo and a tighter bright stroke give the rose a continuous
//! light material without sacrificing its spare line language.

#![allow(dead_code)]
// The sweep clamp is C's two `if`s rather than a `clamp` call, so the line diffs
// against the line it came from.
#![allow(clippy::manual_clamp)]

use musializer_core::scene::settings::index::pulse as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::pulse_field::PulseFieldState;
use musializer_runtime::draw;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, Rectangle, Vector2,
};

/// Segments per rose curve (`scene_pulse_field.c:99`).
const SEGMENTS: i32 = 112;
/// raylib's `DEG2RAD`.
const DEG2RAD: f32 = std::f32::consts::PI / 180.0;

/// Draws Pulse Field into `boundary`.
///
/// `pixel_scale` is physical target pixels per logical output pixel: line weight
/// uses it so supersampling changes sampling, not composition
/// (`../musializer/src/scene.h:64-66`).
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    state: &PulseFieldState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    let scale = frame.setting(SceneId::PulseField, setting::SCALE);
    let ring_setting = frame.setting(SceneId::PulseField, setting::RINGS);
    let motion = frame.setting(SceneId::PulseField, setting::MOTION);
    let arc = frame.setting(SceneId::PulseField, setting::ARC);
    let weight = frame.setting(SceneId::PulseField, setting::WEIGHT);
    let petal_setting = frame.setting(SceneId::PulseField, setting::PETALS);
    let hue_shift = frame.setting(SceneId::PulseField, setting::HUE);
    let bloom = frame.setting(SceneId::PulseField, setting::GLOW);

    let center = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    let extent = boundary.width.min(boundary.height) * 0.45 * scale;
    let bands = frame.audio.bands;
    let count = bands.len();
    if count == 0 {
        return;
    }

    let requested_rings = (ring_setting + 0.5).floor() as usize;
    let rings = count.min(requested_rings);
    if rings == 0 {
        return;
    }
    let semantic_hue = if frame.semantic.available {
        frame.semantic.valence * 60.0 * frame.semantic.confidence
    } else {
        0.0
    };
    let interpretation = if frame.semantic.available {
        frame.semantic.tension * frame.semantic.confidence
    } else {
        0.0
    };
    let (bass, _, _) = frame.audio.spectral_regions();
    // Zero keeps the audio-driven fold (bass/treble balance chooses the rose);
    // any explicit petal count pins the silhouette for a consistent look.
    let fold = if petal_setting >= 0.5 {
        petal_setting.round() as i32
    } else {
        3 + (frame.audio.spectral_balance() * 6.0).round() as i32
    };
    let rotation = state.rotation
        + frame.time_seconds as f32 * motion * 12.0
        + frame.audio.spectral_flux * motion * 45.0
        + interpretation * motion * 14.0;

    let ground_hue = (rotation + semantic_hue + hue_shift + 720.0) % 360.0;
    draw::atmospheric_backdrop(
        d,
        boundary,
        draw::color_from_hsv(ground_hue, 0.62, 0.016 + frame.audio.rms * 0.018),
        draw::color_from_hsv((ground_hue + 34.0) % 360.0, 0.66, 0.045 + bass * 0.035),
        center,
        boundary.width.max(boundary.height) * 0.48,
        draw::color_alpha(
            draw::color_from_hsv((ground_hue + 318.0) % 360.0, 0.58, 0.15),
            0.46 + bass * 0.12,
        ),
    );

    // raylib-rs models raylib's Begin/End mode pairs as scoped closures rather
    // than as bare calls, so C's `BeginBlendMode`/`EndBlendMode` pair becomes this
    // block. The contents and their order are unchanged.
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
        // A bass-breathing bloom anchors the rose's heart so the center never
        // reads as an empty hole between petal passes.
        if bloom > 0.001 {
            let bloom_radius = extent * (0.26 + bass * 0.44) * bloom;
            let bloom_color = draw::color_from_hsv(
                (rotation + semantic_hue + hue_shift + 720.0) % 360.0,
                0.58,
                0.85,
            );
            blend.draw_circle_gradient(
                center.x as i32,
                center.y as i32,
                bloom_radius,
                draw::color_alpha(
                    bloom_color,
                    0.24 + bass * 0.36 + frame.audio.spectral_flux * 0.22,
                ),
                Color::BLANK,
            );
        }

        // Outermost ring first: additive blending makes the order visible, so the
        // reverse loop is behaviour rather than style.
        for i in (1..=rings).rev() {
            let band_index = (i - 1) * count / rings;
            let amplitude = bands[band_index];
            // Each ring breathes at its own rate, so the stack never pulses as one
            // rigid object. Computed in double as C does before the cast.
            let phase = (frame.time_seconds * f64::from(motion) * (0.25 + 0.015 * i as f64)) as f32;
            let wobble =
                (phase * 2.0 * std::f32::consts::PI + i as f32).sin() * extent * 0.025 * amplitude;
            let base_radius = extent * (i as f32 / rings as f32) + wobble;
            let start_angle = rotation + i as f32 * 7.5;
            let mut sweep = (220.0 + amplitude * 140.0) * arc;
            if sweep < 20.0 {
                sweep = 20.0;
            }
            if sweep > 356.0 {
                sweep = 356.0;
            }
            let thickness = (1.0 + amplitude * 5.6) * pixel_scale * weight;
            let color = draw::color_from_hsv(
                (i as f32 / rings as f32 * 280.0 + rotation + semantic_hue + hue_shift + 720.0)
                    % 360.0,
                0.72,
                0.95,
            );
            let rose_depth = 0.045 + amplitude * 0.17 + interpretation * 0.06;
            let mut previous = Vector2::zero();
            for segment in 0..=SEGMENTS {
                let amount = segment as f32 / SEGMENTS as f32;
                let degrees = start_angle + sweep * amount;
                let theta = degrees * DEG2RAD;
                let petals = (fold as f32 * theta + i as f32 * 0.19).cos();
                let spiral =
                    (theta * 0.5 + frame.audio.beat_phase * 2.0 * std::f32::consts::PI).sin();
                let radius = base_radius * (1.0 + petals * rose_depth)
                    + spiral * extent * 0.018 * interpretation;
                let point = Vector2::new(
                    center.x + theta.cos() * radius,
                    center.y + theta.sin() * radius,
                );
                if segment > 0 {
                    // A broad faint echo makes the rose feel drawn in light,
                    // without turning every ring into the same glowing rope.
                    if bloom > 0.001 {
                        blend.draw_line_ex(
                            previous,
                            point,
                            thickness * (2.4 + bloom * 0.8),
                            draw::color_alpha(color, (0.028 + amplitude * 0.070) * bloom),
                        );
                    }
                    blend.draw_line_ex(
                        previous,
                        point,
                        thickness,
                        draw::color_alpha(color, 0.24 + amplitude * 0.68),
                    );
                }
                previous = point;
            }
        }
    });
    draw::vignette(d, boundary, 0.18);
}
