//! Shared project lyric captions, drawn after every scene except Cadence.
//!
//! Port of `draw_scene_lyric_overlay` (`../musializer/src/plug.c:1219-1307`).
//! Layout stays in `musializer-core`; this file is only the application-side
//! composition that turns the layout into a box, shadow and glyph draw calls.

use musializer_core::project::caption_layout::{self, CaptionLayout};
use musializer_core::project::model::{CaptionAnchor, CaptionBox, CaptionStyle};
use musializer_core::scene::LyricCue;
use musializer_runtime::draw;
use musializer_runtime::font::Face;
use raylib::prelude::{
    Color, RaylibDraw, RaylibDrawHandle, RaylibFont, RaylibScissorModeExt, Rectangle, Vector2,
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

fn compose(
    boundary: Rectangle,
    text: &str,
    pixel_scale: f32,
    style: &CaptionStyle,
    measure: &mut dyn FnMut(&str, f32, f32) -> f32,
) -> Option<Composition> {
    if text.is_empty()
        || !pixel_scale.is_finite()
        || pixel_scale <= 0.0
        || boundary.width < 240.0 * pixel_scale
        || boundary.height < 160.0 * pixel_scale
    {
        return None;
    }
    let font_size = (20.0 * pixel_scale).max(boundary.height * style.size_scale as f32);
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
pub fn draw_scene_lyric_overlay(
    d: &mut RaylibDrawHandle<'_>,
    lyric: Option<&LyricCue>,
    font: &Face,
    boundary: Rectangle,
    pixel_scale: f32,
    style: &CaptionStyle,
) {
    let Some(lyric) = lyric else {
        return;
    };
    let Some(composition) = compose(
        boundary,
        &lyric.text,
        pixel_scale,
        style,
        &mut |text, size, spacing| font.measure_text(text, size, spacing).x,
    ) else {
        return;
    };
    let text_color = Color::get_color(style.text_rgba);
    let box_color = Color::get_color(style.box_rgba);
    if style.box_style == CaptionBox::Plate {
        d.draw_rectangle_rounded(composition.box_rect, 0.12, 8, box_color);
        d.draw_rectangle_lines_ex(
            composition.box_rect,
            pixel_scale,
            draw::color_alpha(text_color, 0.28),
        );
    }

    // The layout's three-line ellipsis is the semantic limit; the scissor is the
    // paint limit if an imported face has unusual metrics (`plug.c:1285-1288`).
    let mut clipped = d.begin_scissor_mode(
        composition.box_rect.x as i32,
        composition.box_rect.y as i32,
        composition.box_rect.width as i32,
        composition.box_rect.height as i32,
    );
    for (index, line) in composition.layout.lines.iter().enumerate() {
        let position = Vector2::new(
            composition.box_rect.x + (composition.box_rect.width - line.width) * 0.5,
            composition.box_rect.y
                + composition.vertical_padding
                + index as f32 * composition.line_advance,
        );
        if style.box_style == CaptionBox::Shadow {
            let offset = pixel_scale.max(composition.font_size * 0.055);
            clipped.draw_text_ex(
                font,
                &line.text,
                Vector2::new(position.x + offset, position.y + offset),
                composition.font_size,
                composition.spacing,
                box_color,
            );
        }
        clipped.draw_text_ex(
            font,
            &line.text,
            position,
            composition.font_size,
            composition.spacing,
            text_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(boundary: Rectangle, text: &str, scale: f32, style: &CaptionStyle) -> Composition {
        compose(boundary, text, scale, style, &mut |line, size, _| {
            line.chars().count() as f32 * size * 0.5
        })
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
