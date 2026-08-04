//! Shared project lyric captions, drawn after every scene except Cadence.
//!
//! Port of `draw_scene_lyric_overlay` (`../musializer/src/plug.c:1219-1307`).
//! Layout stays in `musializer-core`; this file is only the application-side
//! composition that turns the layout into a box, shadow and glyph draw calls.

use musializer_core::project::caption_effects::{self, EffectInputs, ResolvedGlow};
use musializer_core::project::caption_layout::{self, CaptionLayout};
use musializer_core::project::model::{CaptionAnchor, CaptionBox, CaptionStyle};
use musializer_core::scene::LyricCue;
use musializer_runtime::draw;
use musializer_runtime::font::{FaceRef, Faces};
use musializer_runtime::halo::{self, HaloBlur};
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
/// The extra return: what the glow did this frame, for the renderer's report
/// line. `"off"` is no glow resolved, `"blurred"` a drawn halo, and
/// `"unavailable"` a resolved glow the blur pipeline could not draw — the state
/// a capture cannot distinguish from `"off"` on its own, which is why it is a
/// word and not a pixel.
#[allow(
    clippy::too_many_arguments,
    reason = "the caption crossing: draw target, cue, faces, geometry, style, drives, blur"
)]
pub fn draw_scene_lyric_overlay(
    d: &mut RaylibDrawHandle<'_>,
    lyric: Option<&LyricCue>,
    fonts: &Faces,
    boundary: Rectangle,
    pixel_scale: f32,
    style: &CaptionStyle,
    inputs: &EffectInputs,
    blur: &mut HaloBlur,
) -> &'static str {
    let Some(lyric) = lyric else {
        return "off";
    };
    let Some(font_size) = caption_font_size(boundary, &lyric.text, pixel_scale, style) else {
        return "off";
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
        return "off";
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

    // The glow ignores the caption's own scissor on purpose: a halo is
    // *supposed* to spill past the composed box, and clipping it would print
    // the box's outline into the haze. It is clipped to `boundary` instead, so
    // it cannot spill over a panel.
    let glow_status = match fx.glow {
        Some(glow) => draw_glow_halo(d, blur, &composition, &font, glow, boundary),
        None => "off",
    };

    // The soft shadow rides the same blur as the glow (operator-approved
    // follow-up to UX0-C11, 2026-08-04): its 9-tap table had the identical
    // discrete-copy pathology, hidden only because the fixture was dark on
    // dark. The buffer is *built* here, before any scissor begins — the
    // offscreen passes must run unclipped — and *composited* inside the box
    // scissor below, because unlike the glow the shadow keeps the legacy
    // decision of staying inside the caption box.
    let soft_shadow = if style.box_style == CaptionBox::Shadow && fx.shadow_blur_px > 0.0 {
        build_glyph_halo(
            d,
            blur,
            &composition,
            &font,
            (fx.shadow_blur_px * 0.55).max(0.75),
        )
    } else {
        None
    };

    // The layout's three-line ellipsis is the semantic limit; the scissor is the
    // paint limit if an imported face has unusual metrics (`plug.c:1285-1288`).
    let mut clipped = d.begin_scissor_mode(
        composition.box_rect.x as i32,
        composition.box_rect.y as i32,
        composition.box_rect.width as i32,
        composition.box_rect.height as i32,
    );
    let shadow_offset = pixel_scale.max(composition.font_size * 0.055);
    let shadow_alpha = f32::from(box_color.a) / 255.0 * fx.shadow_alpha_scale;
    // One whole-block composite rather than one per line: the blur already
    // holds every line's coverage, and per-line compositing would draw each
    // line's penumbra over the previous line's ink.
    let soft_shadow_drawn = match soft_shadow {
        Some(dest) => blur.composite_masked(
            &mut clipped,
            Rectangle::new(
                dest.x + shadow_offset,
                dest.y + shadow_offset,
                dest.width,
                dest.height,
            ),
            draw::color_alpha(box_color, shadow_alpha),
        ),
        None => false,
    };
    for line in line_positions(&composition) {
        // The hard copy: the legacy composition when `shadow_blur` is 0
        // (exactly the C's, with `shadow_opacity` 1), and the *visible*
        // fallback when the mask shader was refused — a sharp shadow a capture
        // can see beats a silently missing soft one.
        if style.box_style == CaptionBox::Shadow && !soft_shadow_drawn {
            clipped.draw_text_ex(
                font.face(),
                line.text,
                Vector2::new(
                    line.position.x + shadow_offset,
                    line.position.y + shadow_offset,
                ),
                composition.font_size,
                composition.spacing,
                draw::color_alpha(box_color, shadow_alpha),
            );
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
    drop(clipped);
    glow_status
}

/// Builds and composites the blurred glow halo (UX0-C11).
///
/// The 17-tap ring this replaces resolved into visible discrete copies of the
/// text once the radius grew — inherent to finite taps — and read thin at full
/// strength. The halo is now the glyph coverage Gaussian-blurred offscreen
/// ([`HaloBlur`]) and composited additively, so a larger radius widens one
/// halo instead of separating seventeen.
///
/// Sizing is derived from the *drawn* font size and radius in pixels — both
/// already carry `pixel_scale` — so export supersampling scales the halo with
/// everything else, and the same project and frame always produce the same
/// pixels.
fn draw_glow_halo(
    d: &mut RaylibDrawHandle<'_>,
    blur: &mut HaloBlur,
    composition: &Composition,
    font: &FaceRef<'_>,
    glow: ResolvedGlow,
    boundary: Rectangle,
) -> &'static str {
    // The authored radius names the visible reach of the haze, and a Gaussian
    // stays visible to roughly 2.5 sigma, so sigma is well under the radius.
    let Some(dest) = build_glyph_halo(
        d,
        blur,
        composition,
        font,
        (glow.radius_px * 0.55).max(0.75),
    ) else {
        return "unavailable";
    };
    let Some(texture) = blur.result() else {
        return "unavailable";
    };
    // The resolved colour's alpha byte *is* the final intensity. Compositing
    // the buffer twice is deliberate luminosity shaping, not double-counting:
    // additive blending saturates against bright pixels, and the second pass is
    // what makes 100 % strength read as hot at the core while the skirt of the
    // Gaussian stays a haze. (The old taps carried 1.5x weight headroom for the
    // same reason.)
    let tint = Color::get_color(glow.rgba);
    let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
    let mut clipped = d.begin_scissor_mode(
        boundary.x as i32,
        boundary.y as i32,
        boundary.width as i32,
        boundary.height as i32,
    );
    let mut blend = clipped.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
    for _ in 0..2 {
        draw::draw_texture_pro(
            &mut blend,
            texture,
            source,
            dest,
            Vector2::zero(),
            0.0,
            tint,
        );
    }
    "blurred"
}

/// Renders the caption's glyph coverage into the blur buffer at `sigma` screen
/// pixels and returns the on-screen rectangle the finished buffer maps onto
/// ([`HaloBlur::result`] texel grid = `down` screen pixels per texel). Shared
/// by the glow and the soft shadow, whose builds differ only in sigma and in
/// how the result is composited.
///
/// Sizing is derived from the *drawn* font size and radius in pixels — both
/// already carry `pixel_scale` — so export supersampling scales the halo with
/// everything else, and the same project and frame always produce the same
/// pixels. Must run with no scissor active (see [`HaloBlur::render`]).
fn build_glyph_halo(
    d: &mut RaylibDrawHandle<'_>,
    blur: &mut HaloBlur,
    composition: &Composition,
    font: &FaceRef<'_>,
    sigma: f32,
) -> Option<Rectangle> {
    // Everything past 3 sigma is under half a percent of peak and invisible
    // over any material; the buffer is padded exactly that far. The caller's
    // sigma floor keeps a sub-pixel radius from degenerating the kernel.
    let spread = (sigma * 3.0).ceil();
    let left = (composition.box_rect.x - spread).floor();
    let top = (composition.box_rect.y - spread).floor();
    let right = (composition.box_rect.x + composition.box_rect.width + spread).ceil();
    let bottom = (composition.box_rect.y + composition.box_rect.height + spread).ceil();
    // Blur at reduced resolution once sigma outgrows the kernel: one buffer
    // texel per `down` screen pixels, so the shader's sigma stays bounded. A
    // halo has no high frequencies, so the bilinear upsample is free quality.
    let down = (sigma / halo::MAX_SIGMA_TEXELS).ceil().max(1.0);
    let width = ((right - left) / down).ceil() as i32;
    let height = ((bottom - top) / down).ceil() as i32;
    let built = blur.render(d, width, height, sigma / down, |target| {
        for line in line_positions(composition) {
            target.draw_text_ex(
                font.face(),
                line.text,
                Vector2::new(
                    (line.position.x - left) / down,
                    (line.position.y - top) / down,
                ),
                composition.font_size / down,
                composition.spacing / down,
                Color::WHITE,
            );
        }
    });
    built.then(|| Rectangle::new(left, top, width as f32 * down, height as f32 * down))
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
