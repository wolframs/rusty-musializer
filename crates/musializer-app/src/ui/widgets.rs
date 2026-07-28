//! Immediate-mode widget primitives.
//!
//! Port of the parts of `../musializer/src/ui_widgets.c` the shell actually
//! uses. Two things about it are behaviour, not style, and are reproduced
//! deliberately:
//!
//! - **A press is claimed by the first widget to see it and is only awarded on
//!   release over the same widget** (`ui_widgets_button_with_id`,
//!   `ui_widgets.c:139-160`). One `active_button_id` in [`Widgets`] holds that
//!   claim for the whole frame. This is exactly the mechanism that made the
//!   workspace-layout bug damaging: an invisible zero-height panel drew its
//!   buttons anyway, claimed the press first, and stole clicks aimed at the
//!   scene tiles painted over them. Hence
//!   [`musializer_core::ui::workspace_layout`]'s rule that a panel which cannot
//!   contain its own controls is not drawn at all.
//! - **A row of buttons agrees on one label size.** Fitting each box
//!   independently made neighbouring buttons of equal size render at unequal
//!   sizes as soon as one label was longer, which the C calls the single most
//!   visible defect in the workspace (`ui_row_typography.h:9-13`). The shared
//!   size comes from [`musializer_core::ui::row_typography`].
//!
//! Text is drawn and measured through [`Face`], the interface face
//! [`musializer_runtime::font`] loads, so every widget agrees with the one place
//! that knows how wide a string is. Passing the face explicitly rather than
//! reaching for `get_font_default()` inside each helper is the C's shape too
//! (`ui_font()` threaded into every `ui_widgets_*` call) and it is what makes the
//! fallback face reachable in a test.

use musializer_core::ui::row_typography;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::Face;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, RaylibFont, Rectangle, Vector2};

use super::theme::{color, metric};

/// Button interaction state (`Button_State`, `ui_widgets.h:34-39`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub hovered: bool,
    /// Released over the widget that claimed the press.
    pub clicked: bool,
    pub pressed: bool,
}

/// `Button_Style` (`ui_widgets.h:41-44`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Neutral,
    // No caller yet. It is the style for a destructive action, and the two the
    // oracle has — `Clear manual` and Cancel export — are Agents L and H's. Kept
    // rather than deleted because it is half of the C's widget vocabulary and
    // re-deriving the colour ramp later would be worse than an allow here.
    #[allow(dead_code)]
    Danger,
}

/// Widget state that outlives a single call.
///
/// The C keeps this inside the global `Plug` struct and passes it to every widget
/// by pointer, with no file-scope state of its own, so a hot reload cannot lose
/// it. Here it is just a field of the shell — but the single `active_button_id`
/// is the load-bearing part either way.
#[derive(Debug, Default)]
pub struct Widgets {
    active_button_id: u64,
    /// Set when any widget was interacted with this frame, so the caller can
    /// tell "the user did something" from "the user moved the mouse".
    pub interacted: bool,
}

impl Widgets {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Call once per frame, before any widget.
    pub fn begin_frame(&mut self) {
        self.interacted = false;
    }

    /// `ui_widgets_button_with_id` (`ui_widgets.c:139-160`).
    ///
    /// `id` must be unique and stable across frames for the widget it names; a
    /// colliding id lets one widget release another's press.
    pub fn button(&mut self, d: &RaylibDrawHandle<'_>, id: u64, boundary: UiRect) -> ButtonState {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        if boundary.is_empty() {
            return ButtonState::default();
        }
        let mouse = d.get_mouse_position();
        let hovered = mouse.x >= boundary.x
            && mouse.x <= boundary.x + boundary.width
            && mouse.y >= boundary.y
            && mouse.y <= boundary.y + boundary.height;

        let mut clicked = false;
        if self.active_button_id == 0 {
            if hovered && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) {
                self.active_button_id = id;
            }
        } else if self.active_button_id == id && d.is_mouse_button_released(MOUSE_BUTTON_LEFT) {
            self.active_button_id = 0;
            clicked = hovered;
        }
        if clicked {
            self.interacted = true;
        }
        ButtonState {
            hovered,
            clicked,
            pressed: hovered && d.is_mouse_button_down(MOUSE_BUTTON_LEFT),
        }
    }

    /// A labelled button (`ui_widgets_styled_text_button_sized`,
    /// `ui_widgets.c:206-229`).
    ///
    /// `font_size` of `None` lets a lone button fit itself; pass
    /// [`row_font_size`]'s result to make a row agree.
    // Eight arguments, matching `ui_widgets_styled_text_button_sized`. A builder
    // would read better at the call site but would hide the one thing that has to
    // be right — the `id`, which is what claims a press — behind a defaultable
    // field.
    #[allow(clippy::too_many_arguments)]
    pub fn text_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &Face,
        id: u64,
        boundary: UiRect,
        label: &str,
        selected: bool,
        style: ButtonStyle,
        font_size: Option<f32>,
    ) -> ButtonState {
        self.text_button_in(
            d, font, id, boundary, boundary, label, selected, style, font_size,
        )
    }

    /// A labelled button whose press area is not its drawing area.
    ///
    /// This is `button_with_id(id, GetCollisionRec(panel, item))` (`plug.c:5306`):
    /// a row in a scrolling list is drawn at its full height and clipped, but only
    /// the part still inside the panel may claim a press. Passing the same rect
    /// twice is [`Self::text_button`], which is the common case.
    #[allow(clippy::too_many_arguments)]
    pub fn text_button_in(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &Face,
        id: u64,
        boundary: UiRect,
        hit: UiRect,
        label: &str,
        selected: bool,
        style: ButtonStyle,
        font_size: Option<f32>,
    ) -> ButtonState {
        let state = self.button(d, id, hit);
        if boundary.is_empty() {
            return state;
        }
        let signal = match style {
            ButtonStyle::Danger => color::ui_danger(),
            ButtonStyle::Neutral => color::track_button_selected(),
        };
        let mut background = if selected {
            signal
        } else {
            color::track_button_background()
        };
        if state.hovered {
            background = if selected {
                brightness(signal, 0.12)
            } else {
                color::track_button_hoverover()
            };
        }
        if state.pressed {
            background = brightness(background, -0.08);
        }
        let rect = rectangle(boundary);
        d.draw_rectangle_rec(rect, background);
        d.draw_rectangle_lines_ex(
            rect,
            if state.pressed { 2.0 } else { 1.0 },
            if selected {
                signal
            } else {
                match style {
                    ButtonStyle::Danger => alpha(signal, 0.72),
                    ButtonStyle::Neutral => color::ui_rule(),
                }
            },
        );
        draw_button_label(
            d,
            font,
            boundary,
            label,
            font_size,
            if state.pressed { 1.0 } else { 0.0 },
            // The palette's WHITE, not raylib's: the pair (white on accent) is
            // one of the contrast checks in `theme`, and it can only check the
            // colour actually drawn.
            if selected {
                color::white()
            } else {
                color::ui_ink()
            },
        );
        state
    }

    /// A button that cannot be pressed, drawn so the reason is visible rather
    /// than the control absent (`ui_widgets_disabled_text_button_sized`).
    ///
    /// Drawing a disabled control instead of hiding it is the affordance rule in
    /// practice: a feature nobody can find is a feature nobody has, and a missing
    /// button teaches nothing about what would make it appear.
    pub fn disabled_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &Face,
        boundary: UiRect,
        label: &str,
        font_size: Option<f32>,
    ) {
        if boundary.is_empty() {
            return;
        }
        let rect = rectangle(boundary);
        d.draw_rectangle_rec(rect, color::ui_surface());
        d.draw_rectangle_lines_ex(rect, 1.0, color::ui_rule());
        draw_button_label(
            d,
            font,
            boundary,
            label,
            font_size,
            0.0,
            color::ui_disabled(),
        );
    }

    /// A horizontal slider returning the value the pointer asks for, or `None`
    /// when it is not being dragged this frame.
    ///
    /// Deliberately not a "return the current value" widget: the caller owns the
    /// value, so a slider that is not being touched must not be able to write one.
    pub fn slider(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        id: u64,
        boundary: UiRect,
        normalized: f32,
    ) -> Option<f32> {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        if boundary.is_empty() {
            return None;
        }
        let track = UiRect::new(
            boundary.x,
            boundary.y + boundary.height * 0.5 - 3.0,
            boundary.width,
            6.0,
        );
        d.draw_rectangle_rec(rectangle(track), color::ui_rule());
        let filled = normalized.clamp(0.0, 1.0);
        d.draw_rectangle_rec(
            rectangle(UiRect::new(
                track.x,
                track.y,
                track.width * filled,
                track.height,
            )),
            color::accent(),
        );

        let state = self.button(d, id, boundary);
        let knob_x = boundary.x + boundary.width * filled;
        d.draw_circle_v(
            Vector2::new(knob_x, boundary.y + boundary.height * 0.5),
            if state.hovered || state.pressed {
                8.0
            } else {
                6.0
            },
            color::accent(),
        );

        // The claim rule matters here too: a drag that started on this slider
        // keeps control even when the pointer leaves the box, which is what makes
        // dragging past the end feel like a slider rather than a switch.
        if self.active_button_id == id && d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
            self.interacted = true;
            let mouse = d.get_mouse_position();
            return Some(slider_value(
                mouse.x,
                boundary.x,
                boundary.x + boundary.width,
            ));
        }
        None
    }
}

/// `ui_widgets_slider_get_value` (`ui_widgets.c:295-302`).
#[must_use]
pub fn slider_value(x: f32, low: f32, high: f32) -> f32 {
    if high <= low {
        return 0.0;
    }
    let x = x.clamp(low, high);
    (x - low) / (high - low)
}

/// The shared size for a row of buttons that should agree
/// (`ui_widgets_row_font_size`, `ui_widgets.c:196-204`).
///
/// `widths` are full box widths; the padding is removed inside
/// [`row_typography::font_size`]. The measurement is taken at `base`, which is
/// what makes it linear in font size — the property `font_size` needs to scale a
/// measured width into a fitted size.
#[must_use]
pub fn row_font_size(font: &Face, labels: &[&str], widths: &[f32], box_height: f32) -> f32 {
    let base = row_typography::base_font_size(box_height);
    row_typography::font_size(
        labels,
        widths,
        base,
        row_typography::UI_ROW_MIN_FONT_SIZE,
        |text| measure(font, text, base),
    )
}

/// Text width in pixels, at zero spacing.
///
/// This is `ui_widgets_caption_measure_raylib` (`ui_widgets.c:304-309`):
/// `MeasureTextEx(font, text, size, 0.0f).x`. Zero spacing is not incidental —
/// it is what makes width linear in font size, which
/// [`row_typography::font_size`] relies on to fit a row in one pass instead of
/// searching.
///
#[must_use]
pub fn measure(font: &Face, text: &str, font_size: f32) -> f32 {
    font.measure_text(text, font_size, 0.0).x
}

#[allow(
    clippy::too_many_arguments,
    reason = "the C's `ui_widgets_draw_label` shape; every argument is one of face, box, text, size, offset, tint"
)]
fn draw_button_label(
    d: &mut RaylibDrawHandle<'_>,
    font: &Face,
    boundary: UiRect,
    label: &str,
    font_size: Option<f32>,
    press_offset: f32,
    tint: Color,
) {
    if label.is_empty() {
        return;
    }
    let base = row_typography::base_font_size(boundary.height);
    if base <= 0.0 {
        return;
    }
    let size = font_size
        .filter(|value| *value > 0.0)
        .unwrap_or_else(|| row_font_size(font, &[label], &[boundary.width], boundary.height));
    let available = boundary.width - row_typography::UI_ROW_LABEL_PADDING;
    // Ellipsize rather than shrink without end. A box too narrow even for the
    // ellipsis still gets the ellipsis: a missing label reads as a bug, a clipped
    // one reads as a tight box (`ui_row_typography.h:46-50`).
    let (fitted, truncated) = row_typography::truncate_label(
        label,
        available,
        Some(|text: &str| measure(font, text, size)),
        row_typography::UI_ROW_LABEL_CAPACITY,
    );
    // U+2026 is in the interface face's codepoint set
    // (`ui_row_typography.c:6-8`, and `font::ui_codepoint`'s General Punctuation
    // range), so with the face loaded the oracle's ellipsis is drawn as the
    // oracle wrote it. raylib's *default* face stops at ASCII 126 and renders it
    // as a missing-glyph box — a headless capture at 960x640 caught exactly that,
    // reading "Pau?" and "Exp?" on the toolbar, which no test could have since
    // the truncation itself was correct. So the substitution survives, guarded:
    // it is now reachable only on the fallback face.
    let fitted = if truncated && !font.is_loaded() {
        fitted.replace('\u{2026}', "...")
    } else {
        fitted
    };
    let width = measure(font, &fitted, size);
    draw_text(
        d,
        font,
        &fitted,
        boundary.x + (boundary.width - width) * 0.5,
        boundary.y + (boundary.height - size) * 0.5 + press_offset,
        size,
        tint,
    );
}

/// Draws text at a float position and size, which raylib's `draw_text` cannot do.
///
/// Zero letter spacing, which is what [`measure`] assumes — see its note on why
/// that is load-bearing rather than a default. Text that wants tracking goes
/// through [`draw_text_tracked`] and is not measured for fitting.
/// Generic over the draw target rather than taking a `RaylibDrawHandle`, because
/// this is also how text lands inside a scissor region — and a scissor handle is a
/// different type that merely implements the same trait.
pub fn draw_text<D: RaylibDraw>(
    d: &mut D,
    font: &Face,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    tint: Color,
) {
    d.draw_text_ex(font, text, Vector2::new(x, y), font_size, 0.0, tint);
}

/// Text with letter spacing, for the few places the C asks for tracking.
///
/// Only the welcome screen's masthead and format strip use it (`plug.c:7773`,
/// `:7828`), where the extra space is what makes a short all-caps line read as a
/// masthead rather than a label. Deliberately separate from [`draw_text`]: nothing
/// tracked is measured for fitting, so the linearity [`measure`] depends on is
/// never at risk.
#[allow(
    clippy::too_many_arguments,
    reason = "draw_text plus one spacing argument; a struct here would hide the position"
)]
pub fn draw_text_tracked(
    d: &mut RaylibDrawHandle<'_>,
    font: &Face,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    spacing: f32,
    tint: Color,
) {
    d.draw_text_ex(font, text, Vector2::new(x, y), font_size, spacing, tint);
}

/// A titled panel background: surface fill, a hairline border, and a header rule.
pub fn panel(d: &mut RaylibDrawHandle<'_>, font: &Face, boundary: UiRect, title: &str) -> UiRect {
    if boundary.is_empty() {
        return boundary;
    }
    let rect = rectangle(boundary);
    d.draw_rectangle_rec(rect, color::ui_surface());
    d.draw_rectangle_lines_ex(rect, 1.0, color::ui_rule());
    if title.is_empty() {
        return boundary;
    }
    draw_text(
        d,
        font,
        title,
        boundary.x + metric::UI_PANEL_PADDING,
        boundary.y + 8.0,
        metric::UI_FONT_CAPTION,
        color::ui_muted(),
    );
    let rule_y = boundary.y + 27.0;
    if rule_y < boundary.y + boundary.height {
        d.draw_line_ex(
            Vector2::new(boundary.x, rule_y),
            Vector2::new(boundary.x + boundary.width, rule_y),
            1.0,
            color::ui_rule(),
        );
    }
    // The content area below the header, so callers do not each re-derive it.
    UiRect::new(
        boundary.x,
        rule_y,
        boundary.width,
        (boundary.y + boundary.height - rule_y).max(0.0),
    )
}

/// `ui_widgets_format_timestamp` (`ui_widgets.c:329-335`): `MM:SS.mmm`.
#[must_use]
pub fn format_timestamp(seconds: f64) -> String {
    let seconds = if seconds < 0.0 { 0.0 } else { seconds };
    let minutes = (seconds / 60.0) as u32;
    let within_minute = seconds - f64::from(minutes) * 60.0;
    format!("{minutes:02}:{within_minute:06.3}")
}

/// A flat filled rectangle. The scrollbar thumb and any other plain block.
pub fn fill<D: RaylibDraw>(d: &mut D, rect: UiRect, tint: Color) {
    if rect.is_empty() {
        return;
    }
    d.draw_rectangle_rec(rectangle(rect), tint);
}

/// `UiRect` to raylib's `Rectangle`. The two are the same four floats; the split
/// exists so layout can be tested without raylib.
#[must_use]
pub fn rectangle(rect: UiRect) -> Rectangle {
    Rectangle::new(rect.x, rect.y, rect.width, rect.height)
}

fn brightness(color: Color, amount: f32) -> Color {
    // raylib's `ColorBrightness` for the two cases the widgets use.
    let scale = |channel: u8| -> u8 {
        let value = f32::from(channel) / 255.0;
        let value = if amount < 0.0 {
            value * (1.0 + amount)
        } else {
            value + (1.0 - value) * amount
        };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    Color::new(scale(color.r), scale(color.g), scale(color.b), color.a)
}

fn alpha(color: Color, amount: f32) -> Color {
    Color::new(
        color.r,
        color.g,
        color.b,
        (amount.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// A stable widget id from a namespace and an index.
///
/// The C hashes with djb2 (`UI_WIDGETS_DJB2_INIT`, `ui_widgets.h:46-48`) because
/// its ids are built from pointers and strings. Every id in this shell is a
/// compile-time constant plus a small index, so a shift and an add is enough —
/// and unlike a hash it cannot collide.
#[must_use]
pub const fn widget_id(namespace: u32, index: u32) -> u64 {
    ((namespace as u64) << 32) | (index as u64 + 1)
}

/// Widget id namespaces. Distinct per panel, so no two panels can name the same
/// widget — a collision would let one release another's press.
pub mod id {
    pub const TOOLBAR: u32 = 1;
    pub const SCENE_BROWSER: u32 = 2;
    pub const TRACKS: u32 = 3;
    pub const INSPECTOR: u32 = 4;
    pub const TIMELINE: u32 = 5;
    /// The welcome screen's two buttons. A namespace of its own even though the
    /// screen and the workspace are never on screen together — an id shared with a
    /// panel would let a press claimed on one be released by the other across a
    /// track load.
    pub const WELCOME: u32 = 6;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_value_clamps_to_its_own_track() {
        assert_eq!(slider_value(0.0, 0.0, 100.0), 0.0);
        assert_eq!(slider_value(50.0, 0.0, 100.0), 0.5);
        assert_eq!(slider_value(100.0, 0.0, 100.0), 1.0);
        // Dragging past either end saturates rather than extrapolating.
        assert_eq!(slider_value(-40.0, 0.0, 100.0), 0.0);
        assert_eq!(slider_value(400.0, 0.0, 100.0), 1.0);
        // A degenerate track cannot divide by zero.
        assert_eq!(slider_value(10.0, 5.0, 5.0), 0.0);
        assert_eq!(slider_value(10.0, 5.0, 1.0), 0.0);
    }

    #[test]
    fn timestamps_match_the_c_format() {
        // `%02u:%06.3f` (`ui_widgets.c:334`).
        assert_eq!(format_timestamp(0.0), "00:00.000");
        assert_eq!(format_timestamp(9.5), "00:09.500");
        assert_eq!(format_timestamp(61.25), "01:01.250");
        assert_eq!(format_timestamp(3600.0), "60:00.000");
        // A negative seek clamps rather than printing a negative minute.
        assert_eq!(format_timestamp(-5.0), "00:00.000");
    }

    #[test]
    fn widget_ids_never_collide_across_namespaces_or_indices() {
        let mut seen = std::collections::HashSet::new();
        for namespace in [
            id::TOOLBAR,
            id::SCENE_BROWSER,
            id::TRACKS,
            id::INSPECTOR,
            id::TIMELINE,
            id::WELCOME,
        ] {
            for index in 0..64u32 {
                assert!(
                    seen.insert(widget_id(namespace, index)),
                    "collision at {namespace}/{index}"
                );
            }
        }
        // Zero is the "nothing is claimed" sentinel and must be unreachable.
        assert_ne!(widget_id(0, 0), 0);
    }
}
