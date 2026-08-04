//! Shared project lyric captions, drawn after every scene except Cadence.
//!
//! Port of `draw_scene_lyric_overlay` (`../musializer/src/plug.c:1219-1307`).
//! Layout stays in `musializer-core`; this file is only the application-side
//! composition that turns the layout into a box, shadow and glyph draw calls.

use musializer_core::project::caption_effects::{self, EffectInputs, GLOW_TAPS, SHADOW_TAPS};
use musializer_core::project::caption_layout::{self, CaptionLayout};
use musializer_core::project::model::{CaptionAnchor, CaptionBox, CaptionStyle};
use musializer_core::scene::LyricCue;
use musializer_runtime::draw;
use musializer_runtime::font::Faces;
use raylib::consts::BlendMode;
use raylib::prelude::{
    Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt, Rectangle,
    Vector2,
};

#[derive(Clone, Debug)]
struct Composition {
    layout: CaptionLayout,
    box_rect: Rectangle,
    font_size: f32,
    spacing: f32,
    vertical_padding: f32,
    line_advance: f32,
}

fn axis_offset(alignment: i8, available: f32, content: f32, margin: f32) -> f32 {
    match alignment {
        value if value < 0 => margin,
        value if value > 0 => available - content - margin,
        _ => (available - content) * 0.5,
    }
}

fn anchor_axes(anchor: CaptionAnchor) -> (i8, i8) {
    match anchor {
        CaptionAnchor::BottomLeft => (-1, 1),
        CaptionAnchor::BottomCenter => (0, 1),
        CaptionAnchor::BottomRight => (1, 1),
        CaptionAnchor::MiddleLeft => (-1, 0),
        CaptionAnchor::MiddleCenter => (0, 0),
        CaptionAnchor::MiddleRight => (1, 0),
        CaptionAnchor::TopLeft => (-1, -1),
        CaptionAnchor::TopCenter => (0, -1),
        CaptionAnchor::TopRight => (1, -1),
    }
}

/// The pixel size this caption will be drawn at, or `None` when there is no
/// caption to draw (`plug.c:1219-1307`).
///
/// Split out of [`compose`] because it is the *only* input the at-size atlas
/// needs and it depends on nothing that has to be measured first. The face has to
/// be chosen before the first measurement — measuring through one atlas and
/// drawing through another drifts every line width — so this runs first, the
/// atlas is resolved from it, and `compose` is then handed the same number.
fn caption_font_size(
    boundary: Rectangle,
    text: &str,
    pixel_scale: f32,
    style: &CaptionStyle,
) -> Option<f32> {
    if text.is_empty()
        || !pixel_scale.is_finite()
        || pixel_scale <= 0.0
        || boundary.width < 240.0 * pixel_scale
        || boundary.height < 160.0 * pixel_scale
    {
        return None;
    }
    Some((20.0 * pixel_scale).max(boundary.height * style.size_scale as f32))
}

fn compose(
    boundary: Rectangle,
    text: &str,
    pixel_scale: f32,
    style: &CaptionStyle,
    font_size: f32,
    measure: &mut dyn FnMut(&str, f32, f32) -> f32,
) -> Option<Composition> {
    let spacing = pixel_scale;
    let (horizontal_padding, vertical_padding) = match style.box_style {
        CaptionBox::Plate => (font_size * 0.7, font_size * 0.34),
        CaptionBox::None | CaptionBox::Shadow => (font_size * 0.12, font_size * 0.08),
    };
    let maximum = (boundary.width * style.width_scale as f32)
        .min(boundary.width - 2.0 * (horizontal_padding + 12.0 * pixel_scale));
    let layout =
        caption_layout::layout_utf8(text, maximum, &mut |line| measure(line, font_size, spacing))
            .ok()?;
    let widest = layout
        .lines
        .iter()
        .map(|line| line.width)
        .fold(0.0f32, f32::max);
    let line_advance = font_size * 1.12;
    let text_height = font_size + (layout.lines.len().saturating_sub(1)) as f32 * line_advance;
    let box_width = (boundary.width - 24.0 * pixel_scale).min(widest + horizontal_padding * 2.0);
    let box_height = text_height + vertical_padding * 2.0;
    let (horizontal, vertical) = anchor_axes(style.anchor);
    let margin = boundary.height * style.margin_scale as f32;
    let edge_margin = margin.max(12.0 * pixel_scale);
    let box_rect = Rectangle::new(
        boundary.x + axis_offset(horizontal, boundary.width, box_width, edge_margin),
        boundary.y + axis_offset(vertical, boundary.height, box_height, margin),
        box_width,
        box_height,
    );
    Some(Composition {
        layout,
        box_rect,
        font_size,
        spacing,
        vertical_padding,
        line_advance,
    })
}

/// Draws the current project cue in the shared caption style.
///
/// Takes the whole face bank rather than one face because the face is chosen
/// *by size*: the caption is drawn at `max(20 * pixel_scale, boundary.height *
/// size_scale)`, which reaches 200 px on a large window and further under export
/// supersampling, and the shared 64 px atlas magnified that far is a blur. See
/// [`Faces::caption_at`], which quantizes the size, caches two atlases and falls
/// back — visibly, in the report line — when it cannot build one.
pub fn draw_scene_lyric_overlay(
    d: &mut RaylibDrawHandle<'_>,
    lyric: Option<&LyricCue>,
    fonts: &Faces,
    boundary: Rectangle,
    pixel_scale: f32,
    style: &CaptionStyle,
    inputs: &EffectInputs,
) {
    let Some(lyric) = lyric else {
        return;
    };
    let Some(font_size) = caption_font_size(boundary, &lyric.text, pixel_scale, style) else {
        return;
    };
    // One resolution, held across both the measurement and the draw. Two calls
    // could hand back two different atlases if a resize landed between them, and
    // a line measured at one size and drawn at another either clips or floats.
    let font = fonts.caption_at(style.face, font_size, &lyric.text);
    let Some(composition) = compose(
        boundary,
        &lyric.text,
        pixel_scale,
        style,
        font_size,
        &mut |text, size, spacing| font.measure_text(text, size, spacing).x,
    ) else {
        return;
    };
    // Deterministic per frame: the drives read the same figures the scenes do,
    // so an export reproduces the preview's pulse exactly.
    let fx = caption_effects::resolve(&style.effects, inputs, font_size);
    let text_color = Color::get_color(style.text_rgba);
    let box_color = Color::get_color(style.box_rgba);
    if style.box_style == CaptionBox::Plate {
        d.draw_rectangle_rounded(composition.box_rect, fx.plate_roundness, 8, box_color);
        // Rounded like the fill it traces. The C outlined its rounded plate
        // with a sharp `DrawRectangleLinesEx` (`plug.c:1281-1285`) — the
        // operator called that one out by name, and the C is legacy.
        d.draw_rectangle_rounded_lines_ex(
            composition.box_rect,
            fx.plate_roundness,
            8,
            pixel_scale,
            draw::color_alpha(text_color, 0.28),
        );
    }

    // The glow ignores the scissor on purpose: a halo is *supposed* to spill
    // past the composed box, and clipping it would print the box's outline into
    // the haze. The taps are still bounded — radius is a fraction of the font
    // size — so nothing reaches far.
    if let Some(glow) = fx.glow {
        let glow_alpha = f32::from((glow.rgba & 0xFF) as u8) / 255.0;
        let glow_color = Color::get_color(glow.rgba);
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut blend| {
            for line in line_positions(&composition) {
                for tap in GLOW_TAPS {
                    blend.draw_text_ex(
                        font.face(),
                        line.text,
                        Vector2::new(
                            line.position.x + tap.dx * glow.radius_px,
                            line.position.y + tap.dy * glow.radius_px,
                        ),
                        composition.font_size,
                        composition.spacing,
                        draw::color_alpha(glow_color, glow_alpha * tap.weight),
                    );
                }
            }
        });
    }

    // The layout's three-line ellipsis is the semantic limit; the scissor is the
    // paint limit if an imported face has unusual metrics (`plug.c:1285-1288`).
    let mut clipped = d.begin_scissor_mode(
        composition.box_rect.x as i32,
        composition.box_rect.y as i32,
        composition.box_rect.width as i32,
        composition.box_rect.height as i32,
    );
    for line in line_positions(&composition) {
        if style.box_style == CaptionBox::Shadow {
            let offset = pixel_scale.max(composition.font_size * 0.055);
            let anchor = Vector2::new(line.position.x + offset, line.position.y + offset);
            let base_alpha = f32::from(box_color.a) / 255.0 * fx.shadow_alpha_scale;
            if fx.shadow_blur_px > 0.0 {
                // The soft shadow: the same colour budget spread over a ring
                // of taps, so blurring never darkens the composite.
                for tap in SHADOW_TAPS {
                    clipped.draw_text_ex(
                        font.face(),
                        line.text,
                        Vector2::new(
                            anchor.x + tap.dx * fx.shadow_blur_px,
                            anchor.y + tap.dy * fx.shadow_blur_px,
                        ),
                        composition.font_size,
                        composition.spacing,
                        draw::color_alpha(box_color, base_alpha * tap.weight),
                    );
                }
            } else {
                // Legacy: one hard copy, exactly the C's composition when the
                // effects block is default (`shadow_opacity` 1).
                clipped.draw_text_ex(
                    font.face(),
                    line.text,
                    anchor,
                    composition.font_size,
                    composition.spacing,
                    draw::color_alpha(box_color, base_alpha),
                );
            }
        }
        clipped.draw_text_ex(
            font.face(),
            line.text,
            line.position,
            composition.font_size,
            composition.spacing,
            text_color,
        );
    }
}

struct PlacedLine<'a> {
    text: &'a str,
    position: Vector2,
}

/// Each laid-out line and where it is drawn — the arithmetic the glow, shadow
/// and ink passes must agree on, so it exists once.
fn line_positions(composition: &Composition) -> impl Iterator<Item = PlacedLine<'_>> {
    composition
        .layout
        .lines
        .iter()
        .enumerate()
        .map(move |(index, line)| PlacedLine {
            text: &line.text,
            position: Vector2::new(
                composition.box_rect.x + (composition.box_rect.width - line.width) * 0.5,
                composition.box_rect.y
                    + composition.vertical_padding
                    + index as f32 * composition.line_advance,
            ),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(boundary: Rectangle, text: &str, scale: f32, style: &CaptionStyle) -> Composition {
        let font_size =
            caption_font_size(boundary, text, scale, style).expect("the boundary holds a caption");
        compose(
            boundary,
            text,
            scale,
            style,
            font_size,
            &mut |line, size, _| line.chars().count() as f32 * size * 0.5,
        )
        .expect("the synthetic caption fits")
    }

    #[test]
    fn long_utf8_is_three_lines_and_visibly_ellipsized() {
        let style = CaptionStyle {
            anchor: CaptionAnchor::TopLeft,
            width_scale: 0.28,
            ..CaptionStyle::default()
        };
        let composition = measured(
            Rectangle::new(0.0, 0.0, 1280.0, 720.0),
            "Καλημέρα κόσμε — мир продолжает звучать, while a final clause must not disappear silently",
            1.0,
            &style,
        );
        assert_eq!(composition.layout.lines.len(), 3);
        assert!(composition.layout.ellipsized);
        assert!(composition.layout.lines[2].text.ends_with('…'));
        assert!(composition.box_rect.x >= 12.0);
        assert!(
            composition.box_rect.y < 100.0,
            "the non-default top anchor won"
        );
    }

    #[test]
    fn all_three_box_modes_have_their_oracle_padding() {
        let boundary = Rectangle::new(0.0, 0.0, 1280.0, 720.0);
        let base = CaptionStyle::default();
        let mut widths = Vec::new();
        for box_style in [CaptionBox::None, CaptionBox::Shadow, CaptionBox::Plate] {
            let composition = measured(
                boundary,
                "same words",
                1.0,
                &CaptionStyle {
                    box_style,
                    ..base.clone()
                },
            );
            widths.push(composition.box_rect.width);
        }
        assert_eq!(
            widths[0], widths[1],
            "none and shadow share compact padding"
        );
        assert!(
            widths[2] > widths[1],
            "the plate pays for its larger padding"
        );
    }

    #[test]
    fn export_pixel_scale_changes_pixels_not_composition() {
        let style = CaptionStyle {
            anchor: CaptionAnchor::MiddleRight,
            ..CaptionStyle::default()
        };
        let preview = measured(
            Rectangle::new(0.0, 0.0, 1280.0, 720.0),
            "resolution independent",
            1.0,
            &style,
        );
        let export = measured(
            Rectangle::new(0.0, 0.0, 2560.0, 1440.0),
            "resolution independent",
            2.0,
            &style,
        );
        for (left, right) in [
            (preview.box_rect.x, export.box_rect.x),
            (preview.box_rect.y, export.box_rect.y),
            (preview.box_rect.width, export.box_rect.width),
            (preview.box_rect.height, export.box_rect.height),
            (preview.font_size, export.font_size),
        ] {
            assert!(
                (right - left * 2.0).abs() < 0.001,
                "{left} did not scale to {right}"
            );
        }
    }
}
