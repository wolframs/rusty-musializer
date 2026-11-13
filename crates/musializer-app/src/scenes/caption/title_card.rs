//! Title cards (post-legacy, MV2): a title line and, under it, who made the
//! music — drawn by the shared caption path after every scene, Cadence and
//! Stage included, so a title no longer needs a scene of its own.
//!
//! The look is built in (`caption_styles::title_look`): the project's face and
//! ink, set at a little over twice a caption's size, centred, over a soft dark
//! band that keeps it legible on any scene. The artist line is small, tracked
//! capitals with the `@handle` after it in a lighter weight of the same ink,
//! and a hairline rule between the two lines draws itself out from the centre.
//! The title condenses out of a soft haze while it rises into place; the
//! artist line follows a beat later. A named style is written over that look.
//!
//! Nothing here decides *whether* there is a card: only an authored entry on
//! the title lane is ever drawn (operator, 2026-10-12).

use musializer_core::project::caption_effects::{self, EffectInputs, ResolvedGlow};
use musializer_core::project::caption_layout;
use musializer_core::project::caption_styles::{self, StyleName, TitleCard};
use musializer_core::project::model::{CaptionBox, CaptionStyle};
use musializer_runtime::font::Faces;
use musializer_runtime::halo::HaloBlur;
use raylib::consts::BlendMode;
use raylib::prelude::{
    Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, Rectangle, Vector2,
};

use super::{
    anchor_axes, axis_offset, build_glyph_halo, draw_glow_halo, line_positions, short_edge,
    Composition,
};

/// The separator between the artist's name and the handle.
const SEPARATOR: &str = "  ·  ";
/// The title's lines may wrap once; a title longer than two lines is a
/// sentence, and the ellipsis says so.
const TITLE_MAX_LINES: usize = 2;

/// What one frame drew, for the report line: the text **as laid out** — not
/// the entry's fields — so a card drawn from the wrong entry, or with its
/// artist line lost, reads differently from the card it was given.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleDrawn {
    /// The laid-out title lines, joined with ` / `.
    pub title: String,
    /// The artist line as drawn, `None` when the card has no artist anywhere.
    pub artist: Option<String>,
    /// Whether the line names an artist (a handle alone does not).
    pub has_artist: bool,
    pub style: Option<StyleName>,
    /// The card's block, title to artist line, in frame pixels.
    pub rect: Rectangle,
    /// How far the card has arrived, `0..=1`.
    pub presence: f32,
    /// What the title's glow did: `off`, `blurred` or `unavailable`.
    pub glow: &'static str,
}

fn with_alpha(color: Color, by: f32) -> Color {
    Color::new(
        color.r,
        color.g,
        color.b,
        (f32::from(color.a) * by.clamp(0.0, 1.0)).round() as u8,
    )
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Draws the card the title lane has at this moment, if it has one.
#[allow(
    clippy::too_many_arguments,
    reason = "the caption crossing: draw target, card, artist, faces, geometry, style, drives, blur"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one card's passes in drawing order: band, shadow, glow, haze, ink, rule, artist"
)]
pub fn draw_title_card(
    d: &mut RaylibDrawHandle<'_>,
    card: &TitleCard,
    project_artist: Option<&str>,
    fonts: &Faces,
    boundary: Rectangle,
    pixel_scale: f32,
    base: &CaptionStyle,
    inputs: &EffectInputs,
    blur: &mut HaloBlur,
) -> Option<TitleDrawn> {
    if !pixel_scale.is_finite()
        || pixel_scale <= 0.0
        || boundary.width < 240.0 * pixel_scale
        || boundary.height < 160.0 * pixel_scale
    {
        return None;
    }
    let named = card
        .style
        .and_then(|name| caption_styles::find(&base.styles, name));
    let style = named.and(card.style);
    let look = caption_styles::title_look(base, named);
    let fx_style = &look.effects;
    let presence = caption_effects::presence(
        fx_style,
        card.start_seconds,
        card.end_seconds,
        inputs.time_seconds,
    );
    if presence <= 0.0 {
        return None;
    }
    // The artist line arrives a beat after the title and leaves with it.
    let delay =
        (fx_style.entrance_seconds * 0.45).min((card.end_seconds - card.start_seconds) * 0.25);
    let artist_presence = caption_effects::presence(
        fx_style,
        card.start_seconds + delay,
        card.end_seconds,
        inputs.time_seconds,
    )
    .min(presence);
    let rule_presence = smooth((presence - 0.25) / 0.75).min(artist_presence.max(presence * 0.6));

    let short = short_edge(boundary);
    let title_size = (20.0 * pixel_scale).max(short * look.size_scale as f32);
    let title_spacing = pixel_scale + title_size * 0.01;
    let maximum =
        (boundary.width * look.width_scale as f32).min(boundary.width - 24.0 * pixel_scale);

    // Measure first, one face at a time (`Faces::caption_at` lends one).
    let layout = {
        let font = fonts.caption_at(look.face, title_size, &card.title);
        caption_layout::layout_utf8_lines(&card.title, maximum, TITLE_MAX_LINES, &mut |line| {
            font.measure_text(line, title_size, title_spacing).x
        })
        .ok()?
    };
    let title_width = layout
        .lines
        .iter()
        .map(|line| line.width)
        .fold(0.0f32, f32::max);

    // The display name in tracked capitals; the handle as written, because a
    // handle's case is the handle.
    let name = card
        .artist
        .as_deref()
        .or(project_artist.filter(|artist| !artist.trim().is_empty()))
        .map(str::to_uppercase);
    let handle = card.handle.clone();
    let has_artist = name.is_some();
    let artist_text = match (&name, &handle) {
        (Some(name), Some(handle)) => Some(format!("{name}{SEPARATOR}{handle}")),
        (Some(name), None) => Some(name.clone()),
        (None, Some(handle)) => Some(handle.clone()),
        (None, None) => None,
    };
    let mut artist_size = (11.0 * pixel_scale).max(title_size * caption_styles::TITLE_ARTIST_RATIO);
    let tracking = |size: f32| size * 0.16;
    // Widths of the name (with its separator) and of the whole line.
    let mut artist_widths = (0.0f32, 0.0f32);
    if let Some(text) = &artist_text {
        let font = fonts.caption_at(look.face, artist_size, text);
        let measure = |size: f32| {
            let lead = match (&name, &handle) {
                (Some(name), Some(_)) => format!("{name}{SEPARATOR}"),
                (Some(name), None) => name.clone(),
                (None, _) => String::new(),
            };
            let spacing = tracking(size);
            let lead_width = if lead.is_empty() {
                0.0
            } else {
                font.measure_text(&lead, size, spacing).x + spacing
            };
            (lead_width, font.measure_text(text, size, spacing).x)
        };
        artist_widths = measure(artist_size);
        // A long name shrinks to the measure rather than running off it.
        if artist_widths.1 > maximum {
            artist_size = (artist_size * maximum / artist_widths.1).max(artist_size * 0.6);
            artist_widths = measure(artist_size);
        }
    }

    // The block: title lines, a gap, the rule, a gap, the artist line.
    let title_advance = title_size * 1.06;
    let title_height = title_size + layout.lines.len().saturating_sub(1) as f32 * title_advance;
    let gap = title_size * 0.30;
    let rule_height = (1.4 * pixel_scale).max(title_size * 0.014);
    let artist_block = if artist_text.is_some() {
        gap + rule_height + gap * 0.85 + artist_size
    } else {
        0.0
    };
    let block_width = title_width.max(artist_widths.1).min(boundary.width);
    let block_height = title_height + artist_block;
    let (horizontal, vertical) = anchor_axes(look.anchor);
    let margin = short * look.margin_scale as f32;
    let edge_margin = margin.max(12.0 * pixel_scale);
    let x = (boundary.x + axis_offset(horizontal, boundary.width, block_width, edge_margin)).clamp(
        boundary.x,
        boundary.x + (boundary.width - block_width).max(0.0),
    );
    let y = (boundary.y + axis_offset(vertical, boundary.height, block_height, margin)).clamp(
        boundary.y,
        boundary.y + (boundary.height - block_height).max(0.0),
    );
    let rect = Rectangle::new(x, y, block_width, block_height);

    let ink = Color::get_color(look.text_rgba);
    let backing = Color::get_color(look.box_rgba);
    let align_x = |width: f32| match horizontal {
        value if value < 0 => x,
        value if value > 0 => x + block_width - width,
        _ => x + (block_width - width) * 0.5,
    };

    // The backing, under everything.
    match look.box_style {
        CaptionBox::Shadow => {
            // A soft band across the frame, darkest behind the card and gone
            // a title's height above and below it. Premultiplied, so the
            // frame stays opaque in a saved still (`BLEND_ALPHA` trap).
            let pad = title_size * 1.15;
            let peak = f32::from(backing.a) / 255.0 * 0.62 * presence;
            let alpha = (peak * 255.0).round() as u8;
            let premultiplied = |by: f32| {
                Color::new(
                    (f32::from(backing.r) * by).round() as u8,
                    (f32::from(backing.g) * by).round() as u8,
                    (f32::from(backing.b) * by).round() as u8,
                    (by * 255.0).round() as u8,
                )
            };
            let dark = premultiplied(f32::from(alpha) / 255.0);
            let top = (y - pad).max(boundary.y);
            let middle = y + block_height * 0.5;
            let bottom = (y + block_height + pad).min(boundary.y + boundary.height);
            let (left, width) = (boundary.x as i32, boundary.width.ceil() as i32);
            d.draw_blend_mode(BlendMode::BLEND_ALPHA_PREMULTIPLY, |mut blend| {
                blend.draw_rectangle_gradient_v(
                    left,
                    top as i32,
                    width,
                    (middle - top).ceil() as i32,
                    Color::BLANK,
                    dark,
                );
                blend.draw_rectangle_gradient_v(
                    left,
                    middle as i32,
                    width,
                    (bottom - middle).ceil() as i32,
                    dark,
                    Color::BLANK,
                );
            });
        }
        CaptionBox::Plate => {
            let plate = Rectangle::new(
                x - title_size * 0.7,
                y - title_size * 0.45,
                block_width + title_size * 1.4,
                block_height + title_size * 0.9,
            );
            let roundness = fx_style.plate_roundness as f32;
            d.draw_rectangle_rounded(plate, roundness, 8, with_alpha(backing, presence));
            d.draw_rectangle_rounded_lines_ex(
                plate,
                roundness,
                8,
                pixel_scale,
                with_alpha(ink, 0.28 * presence),
            );
        }
        CaptionBox::None => {}
    }

    // The title rises into place as it condenses.
    let rise = (1.0 - presence) * title_size * 0.22;
    let title = Composition {
        box_rect: Rectangle::new(x, y + rise, block_width, title_height),
        font_size: title_size,
        spacing: title_spacing,
        vertical_padding: 0.0,
        line_advance: title_advance,
        align: horizontal,
        layout,
    };
    let fx = caption_effects::resolve(fx_style, inputs, title_size);
    let glow = {
        let font = fonts.caption_at(look.face, title_size, &card.title);
        if look.box_style == CaptionBox::Shadow && fx.shadow_blur_px > 0.0 {
            if let Some(dest) =
                build_glyph_halo(d, blur, &title, &font, (fx.shadow_blur_px * 0.55).max(0.75))
            {
                let offset = pixel_scale.max(title_size * 0.04);
                blur.composite_masked(
                    d,
                    Rectangle::new(dest.x + offset, dest.y + offset, dest.width, dest.height),
                    with_alpha(backing, fx.shadow_alpha_scale * presence),
                );
            }
        }
        let glow = match fx.glow {
            Some(mut glow) => {
                let alpha = ((glow.rgba & 0xFF) as f32 * presence).round() as u32;
                glow.rgba = (glow.rgba & 0xFFFF_FF00) | alpha.min(255);
                draw_glow_halo(d, blur, &title, &font, glow, boundary)
            }
            None => "off",
        };
        if presence < 1.0 {
            let haze = ((1.0 - presence) * presence * 4.0 * 0.85 * 255.0).round() as u32;
            if haze > 0 {
                draw_glow_halo(
                    d,
                    blur,
                    &title,
                    &font,
                    ResolvedGlow {
                        rgba: (look.text_rgba & 0xFFFF_FF00) | haze.min(255),
                        radius_px: title_size * (0.12 + 0.5 * (1.0 - presence)),
                    },
                    boundary,
                );
            }
        }
        let ink_now = with_alpha(ink, presence);
        for line in line_positions(&title) {
            d.draw_text_ex(
                font.face(),
                line.text,
                line.position,
                title_size,
                title_spacing,
                ink_now,
            );
        }
        glow
    };

    let mut drawn_artist = None;
    if let Some(text) = &artist_text {
        // The rule, drawing itself out from the card's axis.
        let rule_y = y + title_height + gap;
        let full = (artist_widths.1 * 0.42)
            .max(title_size * 1.4)
            .min(block_width);
        let width = full * rule_presence;
        if width >= 1.0 {
            let rule_x = match horizontal {
                value if value < 0 => x,
                value if value > 0 => x + block_width - width,
                _ => x + (block_width - width) * 0.5,
            };
            d.draw_rectangle_rec(
                Rectangle::new(rule_x, rule_y, width, rule_height),
                with_alpha(ink, 0.55 * rule_presence),
            );
        }
        let artist_rise = (1.0 - artist_presence) * artist_size * 0.7;
        let line_y = rule_y + rule_height + gap * 0.85 + artist_rise;
        let line_x = align_x(artist_widths.1);
        let spacing = tracking(artist_size);
        let font = fonts.caption_at(look.face, artist_size, text);
        match (&name, &handle) {
            (Some(name), Some(handle)) => {
                d.draw_text_ex(
                    font.face(),
                    &format!("{name}{SEPARATOR}"),
                    Vector2::new(line_x, line_y),
                    artist_size,
                    spacing,
                    with_alpha(ink, 0.94 * artist_presence),
                );
                d.draw_text_ex(
                    font.face(),
                    handle,
                    Vector2::new(line_x + artist_widths.0, line_y),
                    artist_size,
                    spacing,
                    with_alpha(ink, 0.6 * artist_presence),
                );
            }
            _ => {
                d.draw_text_ex(
                    font.face(),
                    text,
                    Vector2::new(line_x, line_y),
                    artist_size,
                    spacing,
                    with_alpha(ink, 0.94 * artist_presence),
                );
            }
        }
        drawn_artist = Some(text.clone());
    }

    Some(TitleDrawn {
        title: title
            .layout
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" / "),
        artist: drawn_artist,
        has_artist,
        style,
        rect,
        presence,
        glow,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_artist_line_arrives_after_the_title_and_never_before() {
        let look = caption_styles::title_look(&CaptionStyle::default(), None);
        let fx = &look.effects;
        let delay = fx.entrance_seconds * 0.45;
        for time in [0.1, 0.4, 0.8, 1.2, 3.0] {
            let title = caption_effects::presence(fx, 0.0, 6.0, time);
            let artist = caption_effects::presence(fx, delay, 6.0, time).min(title);
            assert!(artist <= title, "{time}: {artist} > {title}");
        }
        assert!(caption_effects::presence(fx, delay, 6.0, 0.3) < 0.05);
        assert_eq!(caption_effects::presence(fx, 0.0, 6.0, 3.0), 1.0);
    }

    #[test]
    fn colours_scale_their_alpha_and_nothing_else() {
        let colour = with_alpha(Color::new(10, 20, 30, 200), 0.5);
        assert_eq!((colour.r, colour.g, colour.b, colour.a), (10, 20, 30, 100));
        assert_eq!(with_alpha(Color::WHITE, 2.0).a, 255);
    }
}
