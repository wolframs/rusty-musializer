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
//! Text is drawn and measured through [`UiFonts`], the native-size interface bank
//! [`musializer_runtime::font`] loads, so every widget agrees with the one place
//! that knows how wide a string is. Passing the face explicitly rather than
//! reaching for `get_font_default()` inside each helper is the C's shape too
//! (`ui_font()` threaded into every `ui_widgets_*` call) and it is what makes the
//! fallback face reachable in a test.

use musializer_core::ui::row_typography;
use musializer_core::ui::{contrast, workspace_layout::UiRect};
use musializer_runtime::font::{AuthoredText, Face, GlyphRepertoire, UiFonts};
use raylib::prelude::{
    Color, RaylibDraw, RaylibDrawHandle, RaylibFont, RaylibScissorMode, RaylibScissorModeExt,
    Rectangle, Vector2,
};

use super::scale::UiScale;
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

#[derive(Clone, Copy)]
enum LabelFont<'a> {
    Ui(&'a UiFonts),
    Exact(&'a Face),
    /// Project-authored words — track names, cue text — through the
    /// glyph-complete caption atlas rather than the Latin-only chrome bank
    /// (review 1.5).
    Authored(AuthoredText<'a>),
}

/// How long the pointer must rest on a control before its tooltip appears.
///
/// Long enough that sweeping the pointer across a row does not strobe fifteen
/// boxes, short enough to feel like an answer. Not in [`metric`] because it is a
/// duration rather than a dimension, and nothing in the oracle's theme has one —
/// tooltips are an addition here, not a port.
pub const TOOLTIP_DELAY_SECONDS: f64 = 0.35;

/// A tooltip waiting to be drawn, with the control it belongs to.
///
/// Deferred rather than drawn where it is requested, because a tooltip has to
/// sit above everything: drawn in place it would be painted over by the next
/// panel, and the toolbar is the *first* thing the shell draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip {
    pub text: String,
    /// The control's box, which the tooltip is positioned against rather than
    /// against the pointer — a tip that follows the cursor is harder to read and
    /// impossible to photograph deterministically.
    pub anchor: UiRect,
}

/// Everything a widget needs to know about the pointer, as a value.
///
/// Lifted out of the draw handle because the claim rule is state that outlives a
/// frame and a rule with memory has to be testable; raylib's input is only
/// readable from a live window, which a `cargo test` run does not have.
/// `x`/`y` are logical coordinates — already through [`UiScale`] — because every
/// rectangle a widget is given is logical too.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    pub x: f32,
    pub y: f32,
    /// Held, including the frame of the press.
    pub down: bool,
    /// The press edge.
    pub pressed: bool,
    /// The release edge, true for exactly one frame.
    pub released: bool,
}

impl Pointer {
    #[must_use]
    pub fn read(d: &RaylibDrawHandle<'_>, ui_scale: UiScale) -> Self {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let position = ui_scale.mouse(d);
        Self {
            x: position.x,
            y: position.y,
            down: d.is_mouse_button_down(MOUSE_BUTTON_LEFT),
            pressed: d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT),
            released: d.is_mouse_button_released(MOUSE_BUTTON_LEFT),
        }
    }
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
    /// The physical left-button state the last drawn widget saw.
    ///
    /// A claim is only ever released by the widget that made it, so a widget that
    /// stops being drawn mid-press holds it forever and every button and slider
    /// in the application goes dead (UX0-A02: hold a slider, press `T`, the panel
    /// closes, the claim is stranded). Freeing it needs the one fact [`Widgets`]
    /// is not given — where the physical button is — so it is recorded from
    /// whichever widget happened to run rather than read at frame start. Every
    /// surface that can strand a claim draws something else in the same frame:
    /// the toolbar survives fullscreen, and the welcome and export screens have
    /// buttons of their own.
    pointer_was_down: bool,
    /// Set when any widget was interacted with this frame, so the caller can
    /// tell "the user did something" from "the user moved the mouse".
    pub interacted: bool,
    /// The control the pointer is resting on, and when it arrived.
    ///
    /// Keyed by widget id rather than by rectangle so that a control which moves —
    /// the transport button changing width as its label goes Play/Pause — does not
    /// restart its own dwell.
    hovered_id: u64,
    hovered_since: f64,
    /// This frame's tooltip, claimed by the last widget to ask. Last rather than
    /// first because later widgets are drawn on top, so the one the user can
    /// actually see is the one that should speak.
    pending_tooltip: Option<Tooltip>,
    /// Zeroed by the probe harness so a capture does not depend on how many
    /// frames it happened to run for. A tooltip nothing can photograph
    /// deterministically is a tooltip no capture reviews.
    pub tooltip_delay: f64,
    ui_scale: UiScale,
}

impl Widgets {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tooltip_delay: TOOLTIP_DELAY_SECONDS,
            ..Self::default()
        }
    }

    /// Call once per frame, before any widget.
    pub fn begin_frame(&mut self, ui_scale: UiScale) {
        self.release_stranded_claim();
        self.interacted = false;
        self.pending_tooltip = None;
        self.ui_scale = ui_scale;
    }

    /// Frees a press claimed by a widget that has since stopped being drawn
    /// (UX0-A02).
    ///
    /// The claim rule is the oracle's and is not negotiable: whoever takes the
    /// press keeps it until it releases over the same widget. What the oracle
    /// never has to survive is the widget *disappearing* mid-press, which this
    /// shell can do from the keyboard — `T` closes the inspector and `F` hides
    /// every panel, both without asking the panel first. The claim then belongs
    /// to a widget that will never be drawn again, and since `active_button_id`
    /// is one shared slot, nothing else in the application can ever be pressed.
    ///
    /// **It cannot eat a click**, because the button state it reads is the one a
    /// widget observed on the *previous* frame, and a click is delivered on the
    /// release frame itself. A claim only ever exists between a press and a
    /// release, so on every frame of a legitimate press the previous frame saw
    /// the button down — including the release frame, whose own `up` is not
    /// visible here until the frame after, by which time the owning widget has
    /// already taken the claim and returned `clicked`. If instead the release
    /// edge fell on a frame where the owner was not drawn, the owner never saw it
    /// and there is no click left to deliver.
    ///
    /// Reading the previous frame rather than the current one is the whole
    /// safety argument, which is also why this does not take a pointer argument:
    /// there would be nothing stopping a caller from handing it *this* frame's
    /// state, and a net that fires on the release frame would swallow every click
    /// in the interface.
    fn release_stranded_claim(&mut self) {
        if !self.pointer_was_down {
            self.active_button_id = 0;
        }
    }

    /// Offers a tooltip for the control `id` just drew.
    ///
    /// Called *after* the button, with the state it returned, so the widget
    /// vocabulary stays the C's and nothing that does not want a tooltip has to
    /// pass one. `text` should name the control and its shortcut — "Mute [M]" —
    /// because the tooltip is the only place a shortcut is written down now that
    /// the transport row draws icons.
    ///
    /// Returns nothing: the tooltip is drawn later, by whoever owns the frame.
    pub fn hint(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        state: ButtonState,
        id: u64,
        anchor: UiRect,
        text: &str,
    ) {
        if !state.hovered || text.is_empty() || anchor.is_empty() {
            // Only the *hovered* widget may clear the dwell, or a row of
            // neighbours would each reset it in turn and the tip would never
            // appear.
            if self.hovered_id == id {
                self.hovered_id = 0;
            }
            return;
        }
        let now = d.get_time();
        if self.hovered_id != id {
            self.hovered_id = id;
            self.hovered_since = now;
        }
        // A press dismisses the tip: the user has stopped asking what the control
        // is and started using it, and a tip hanging over a control being dragged
        // covers the thing it is explaining.
        if state.pressed {
            return;
        }
        if now - self.hovered_since >= self.tooltip_delay {
            self.pending_tooltip = Some(Tooltip {
                text: text.to_string(),
                anchor,
            });
        }
    }

    /// This frame's tooltip, if one is due.
    #[must_use]
    pub fn tooltip(&self) -> Option<&Tooltip> {
        self.pending_tooltip.as_ref()
    }

    /// `ui_widgets_button_with_id` (`ui_widgets.c:139-160`).
    ///
    /// `id` must be unique and stable across frames for the widget it names; a
    /// colliding id lets one widget release another's press.
    pub fn button(&mut self, d: &RaylibDrawHandle<'_>, id: u64, boundary: UiRect) -> ButtonState {
        self.button_at(id, boundary, Pointer::read(d, self.ui_scale))
    }

    /// [`Widgets::button`] without raylib.
    ///
    /// Split out so the claim rule — the one piece of widget state that outlives
    /// a frame, and the one a stranded press kills the whole interface with — can
    /// be driven in a unit test. Nothing here needs a window; `button` is only
    /// the four input reads.
    pub(crate) fn button_at(&mut self, id: u64, boundary: UiRect, pointer: Pointer) -> ButtonState {
        // Recorded before the empty-boundary refusal, because a control with no
        // room is still a control that ran: what the safety net needs from it is
        // where the physical button is, and that is true either way.
        self.pointer_was_down = pointer.down;
        if boundary.is_empty() {
            return ButtonState::default();
        }
        let hovered = boundary.contains_point(pointer.x, pointer.y);

        let mut clicked = false;
        if self.active_button_id == 0 {
            if hovered && pointer.pressed {
                self.active_button_id = id;
            }
        } else if self.active_button_id == id && pointer.released {
            self.active_button_id = 0;
            clicked = hovered;
        }
        if clicked {
            self.interacted = true;
        }
        ButtonState {
            hovered,
            clicked,
            pressed: hovered && pointer.down,
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
        font: &UiFonts,
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
        font: &UiFonts,
        id: u64,
        boundary: UiRect,
        hit: UiRect,
        label: &str,
        selected: bool,
        style: ButtonStyle,
        font_size: Option<f32>,
    ) -> ButtonState {
        self.button_with_label(
            d,
            LabelFont::Ui(font),
            id,
            boundary,
            hit,
            label,
            selected,
            style,
            font_size,
        )
    }

    /// [`Self::text_button_in`] for a label in the user's own words — a track
    /// name, a cue line — where the chrome bank's Latin-only repertoire would
    /// draw `?` (review 1.5).
    #[allow(clippy::too_many_arguments)]
    pub fn text_button_in_authored(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: AuthoredText<'_>,
        id: u64,
        boundary: UiRect,
        hit: UiRect,
        label: &str,
        selected: bool,
        style: ButtonStyle,
        font_size: Option<f32>,
    ) -> ButtonState {
        self.button_with_label(
            d,
            LabelFont::Authored(font),
            id,
            boundary,
            hit,
            label,
            selected,
            style,
            font_size,
        )
    }

    /// The toolbar's icon-face exception. Ordinary labels cannot call this: a
    /// raw [`Face`] here is reserved for Font Awesome glyphs, while every textual
    /// control goes through [`Self::text_button`] and the native UI bank.
    #[allow(clippy::too_many_arguments)]
    pub fn icon_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &Face,
        id: u64,
        boundary: UiRect,
        glyph: &str,
        selected: bool,
        style: ButtonStyle,
        font_size: f32,
    ) -> ButtonState {
        self.button_with_label(
            d,
            LabelFont::Exact(font),
            id,
            boundary,
            boundary,
            glyph,
            selected,
            style,
            Some(font_size),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn button_with_label(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: LabelFont<'_>,
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
        font: &UiFonts,
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
            LabelFont::Ui(font),
            boundary,
            label,
            font_size,
            0.0,
            color::ui_disabled(),
        );
    }

    /// Disabled counterpart to [`Self::icon_button`].
    pub fn disabled_icon_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &Face,
        boundary: UiRect,
        glyph: &str,
        font_size: f32,
    ) {
        if boundary.is_empty() {
            return;
        }
        let rect = rectangle(boundary);
        d.draw_rectangle_rec(rect, color::ui_surface());
        d.draw_rectangle_lines_ex(rect, 1.0, color::ui_rule());
        draw_button_label(
            d,
            LabelFont::Exact(font),
            boundary,
            glyph,
            Some(font_size),
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
        if boundary.is_empty() {
            return None;
        }
        let pointer = Pointer::read(d, self.ui_scale);
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

        let state = self.button_at(id, boundary, pointer);
        let knob_x = boundary.x + boundary.width * filled;
        d.draw_circle_v(
            Vector2::new(knob_x, boundary.y + boundary.height * 0.5),
            if state.hovered || state.pressed {
                SLIDER_KNOB_RADIUS
            } else {
                SLIDER_KNOB_RADIUS - 2.0
            },
            color::accent(),
        );

        // The claim rule matters here too: a drag that started on this slider
        // keeps control even when the pointer leaves the box, which is what makes
        // dragging past the end feel like a slider rather than a switch.
        if self.active_button_id == id && pointer.down {
            self.interacted = true;
            return Some(slider_value(
                pointer.x,
                boundary.x,
                boundary.x + boundary.width,
            ));
        }
        None
    }
}

/// Where a tooltip's box goes, given its anchor and the window.
///
/// Pure arithmetic and separate from the drawing so it can be tested without a
/// window, which is the only way the two edge rules below get checked: a tip that
/// runs off the right of the screen and a tip that would be clipped by the top are
/// both states a capture reaches only by accident.
///
/// Prefers **above** the control, because every control that has a tooltip today
/// lives in the toolbar or the timeline — near the bottom of the window — and a
/// tip drawn below them would be half off-screen.
#[must_use]
pub fn tooltip_box(anchor: UiRect, text_width: f32, window: (f32, f32)) -> UiRect {
    let width = text_width + TOOLTIP_PADDING_X * 2.0;
    let height = metric::UI_FONT_CAPTION + TOOLTIP_PADDING_Y * 2.0;

    // Centred on the control, then pushed back inside the window. Clamping the
    // left edge after centring rather than choosing a side means a tip on a
    // corner control stays adjacent to it instead of jumping across the screen.
    let centred = anchor.x + (anchor.width - width) * 0.5;
    let x = centred.clamp(
        TOOLTIP_MARGIN,
        (window.0 - width - TOOLTIP_MARGIN).max(TOOLTIP_MARGIN),
    );

    let above = anchor.y - height - TOOLTIP_GAP;
    let y = if above >= TOOLTIP_MARGIN {
        above
    } else {
        // No room above: below, and if there is no room there either, clamped —
        // an overlapping tooltip is still readable, an off-screen one is not.
        (anchor.y + anchor.height + TOOLTIP_GAP).min(window.1 - height - TOOLTIP_MARGIN)
    };
    UiRect::new(x, y, width, height)
}

/// Where the text starts inside a box [`tooltip_box`] returned.
///
/// Public for the one caller that cannot go through [`draw_tooltip`]: the lyric
/// cue lane types the cue's **own words** into its tip, and those have to go
/// through the glyph-complete authored atlas rather than the Latin-only chrome
/// bank this module draws with (UX0-A05 — the panel drew Greek cue text as rows
/// of `?` for weeks because nothing said which face served it). Exposing the
/// origin rather than the two padding constants keeps the box's interior a
/// detail of this module: a caller that wants a different face still cannot get
/// the plate's geometry wrong.
#[must_use]
pub fn tooltip_text_origin(boundary: UiRect) -> (f32, f32) {
    (
        boundary.x + TOOLTIP_PADDING_X,
        boundary.y + TOOLTIP_PADDING_Y,
    )
}

const TOOLTIP_PADDING_X: f32 = 9.0;
const TOOLTIP_PADDING_Y: f32 = 6.0;
const TOOLTIP_GAP: f32 = 6.0;
const TOOLTIP_MARGIN: f32 = 4.0;

/// Draws a tooltip above everything else.
///
/// Ink-on-white is the chrome's pairing; a tooltip inverts it — white on ink — so
/// that it reads as floating rather than as another panel. That pair is one of the
/// palette's contrast-checked ones.
pub fn draw_tooltip(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    tooltip: &Tooltip,
    window: (f32, f32),
) {
    let size = metric::UI_FONT_CAPTION;
    let text_width = measure(font, &tooltip.text, size);
    let boundary = tooltip_box(tooltip.anchor, text_width, window);
    if boundary.is_empty() {
        return;
    }
    let rect = rectangle(boundary);
    d.draw_rectangle_rec(rect, color::ui_ink());
    d.draw_rectangle_lines_ex(rect, 1.0, alpha(color::white(), 0.18));
    draw_text(
        d,
        font,
        &tooltip.text,
        boundary.x + TOOLTIP_PADDING_X,
        boundary.y + TOOLTIP_PADDING_Y,
        size,
        color::white(),
    );
}

/// The slider knob's radius when hovered, which is its largest.
///
/// Public because the knob is centred on the value's position, so at 0 and 1 half
/// of it hangs *outside* the track's rectangle. A caller placing a slider hard
/// against a neighbour has to inset by this much or the knob will overlap it — a
/// capture at 960 px with the inspector open is what found that, with a
/// full-volume knob touching the fullscreen button.
pub const SLIDER_KNOB_RADIUS: f32 = 8.0;

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
pub fn row_font_size(font: &UiFonts, labels: &[&str], widths: &[f32], box_height: f32) -> f32 {
    let base = font.native_size(row_typography::base_font_size(box_height));
    let fitted = row_typography::font_size(
        labels,
        widths,
        base,
        row_typography::UI_ROW_MIN_FONT_SIZE,
        |text| measure(font, text, base),
    );
    font.native_size(fitted)
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
pub fn measure(font: &UiFonts, text: &str, font_size: f32) -> f32 {
    font.measure_text(text, font_size, 0.0).x
}

/// Width of proportional UI text with decimal digits laid out in equal cells.
///
/// Space Grotesk remains the face — only its numeral advances become tabular,
/// like CSS's `font-variant-numeric: tabular-nums`. This is the stable-geometry
/// boundary for live timecodes: measuring `00:11.111` and `00:55.555` through
/// ordinary proportional advances makes every neighbour breathe during
/// playback, even though the strings have the same shape.
#[must_use]
pub fn measure_tabular(font: &UiFonts, text: &str, font_size: f32) -> f32 {
    let digit_width = ('0'..='9')
        .map(|digit| measure_char(font, digit, font_size))
        .fold(0.0f32, f32::max);
    tabular_width(text, digit_width, |character| {
        measure_char(font, character, font_size)
    })
}

fn tabular_width(
    text: &str,
    digit_width: f32,
    mut measure_character: impl FnMut(char) -> f32,
) -> f32 {
    text.chars()
        .map(|character| {
            if character.is_ascii_digit() {
                digit_width
            } else {
                measure_character(character)
            }
        })
        .sum()
}

fn measure_char(font: &UiFonts, character: char, font_size: f32) -> f32 {
    let mut bytes = [0u8; 4];
    measure(font, character.encode_utf8(&mut bytes), font_size)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the C's `ui_widgets_draw_label` shape; every argument is one of face, box, text, size, offset, tint"
)]
fn draw_button_label(
    d: &mut RaylibDrawHandle<'_>,
    font: LabelFont<'_>,
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
    let size = match font {
        LabelFont::Ui(font) => font_size.filter(|value| *value > 0.0).map_or_else(
            || row_font_size(font, &[label], &[boundary.width], boundary.height),
            |value| font.native_size(value),
        ),
        LabelFont::Exact(_) | LabelFont::Authored(_) => {
            font_size.filter(|value| *value > 0.0).unwrap_or(base)
        }
    };
    let available = boundary.width - row_typography::UI_ROW_LABEL_PADDING;
    // Ellipsize rather than shrink without end. A box too narrow even for the
    // ellipsis still gets the ellipsis: a missing label reads as a bug, a clipped
    // one reads as a tight box (`ui_row_typography.h:46-50`).
    let (fitted, truncated) = row_typography::truncate_label(
        label,
        available,
        Some(|text: &str| match font {
            LabelFont::Ui(font) => measure(font, text, size),
            LabelFont::Exact(font) => font.measure_text(text, size, 0.0).x,
            LabelFont::Authored(ref font) => font.measure_text(text, size, 0.0).x,
        }),
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
    let loaded = match font {
        LabelFont::Ui(font) => font.all_loaded(),
        LabelFont::Exact(font) => font.is_loaded(),
        LabelFont::Authored(ref font) => font.repertoire() != GlyphRepertoire::RaylibDefault,
    };
    let fitted = if truncated && !loaded {
        fitted.replace('\u{2026}', "...")
    } else {
        fitted
    };
    let width = match font {
        LabelFont::Ui(font) => measure(font, &fitted, size),
        LabelFont::Exact(font) => font.measure_text(&fitted, size, 0.0).x,
        LabelFont::Authored(ref font) => font.measure_text(&fitted, size, 0.0).x,
    };
    let position = Vector2::new(
        (boundary.x + (boundary.width - width) * 0.5).round(),
        (boundary.y + (boundary.height - size) * 0.5 + press_offset).round(),
    );
    match font {
        LabelFont::Ui(font) => draw_text(d, font, &fitted, position.x, position.y, size, tint),
        LabelFont::Exact(font) => d.draw_text_ex(font, &fitted, position, size, 0.0, tint),
        LabelFont::Authored(ref font) => font.draw_text(d, &fitted, position, size, 0.0, tint),
    }
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
    font: &UiFonts,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    tint: Color,
) {
    font.draw_text(
        d,
        text,
        Vector2::new(x.round(), y.round()),
        font_size,
        0.0,
        tint,
    );
}

/// Draw proportional UI text with decimal digits centred in tabular cells.
///
/// Measurement and drawing share the same cell arithmetic, so callers may use
/// [`measure_tabular`] as a fixed reservation without switching to a monospace
/// face. Non-digits retain their ordinary proportional advance.
pub fn draw_text_tabular<D: RaylibDraw>(
    d: &mut D,
    font: &UiFonts,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    tint: Color,
) {
    let digit_width = ('0'..='9')
        .map(|digit| measure_char(font, digit, font_size))
        .fold(0.0f32, f32::max);
    let mut cursor = x;
    for character in text.chars() {
        let mut bytes = [0u8; 4];
        let glyph = character.encode_utf8(&mut bytes);
        let glyph_width = measure(font, glyph, font_size);
        let cell_width = if character.is_ascii_digit() {
            digit_width
        } else {
            glyph_width
        };
        draw_text(
            d,
            font,
            glyph,
            cursor + (cell_width - glyph_width) * 0.5,
            y,
            font_size,
            tint,
        );
        cursor += cell_width;
    }
}

/// Whatever can measure and draw a run of text: the chrome bank or an authored
/// face.
///
/// [`UiFonts`] and [`AuthoredText`] already agree on these two signatures, but
/// they are unrelated types, so a helper that wants to serve both needs a name
/// for the overlap. The distinction is not cosmetic: the chrome bank is the
/// Latin-only interface subset, and project-authored words — track names, lyric
/// cues — must go through [`AuthoredText`] or a Greek or Cyrillic line draws as a
/// row of `?` (UX0-A05).
///
/// Not object-safe, because `draw_text` is generic over the draw target for the
/// same reason [`draw_text`] is — a scissor handle is a different type that
/// merely implements the same trait. Callers take it as a type parameter.
pub trait TextFace {
    /// Width and height of `text` at `size`, through the face that will draw it.
    fn measure_text(&self, text: &str, size: f32, spacing: f32) -> Vector2;

    /// Draws `text` with `position` as its top-left corner.
    fn draw_text<D: RaylibDraw>(
        &self,
        d: &mut D,
        text: &str,
        position: Vector2,
        size: f32,
        spacing: f32,
        tint: Color,
    );
}

impl TextFace for UiFonts {
    fn measure_text(&self, text: &str, size: f32, spacing: f32) -> Vector2 {
        // Named explicitly rather than through `self.`, which would resolve to
        // the inherent method and read as accidental recursion.
        UiFonts::measure_text(self, text, size, spacing)
    }

    fn draw_text<D: RaylibDraw>(
        &self,
        d: &mut D,
        text: &str,
        position: Vector2,
        size: f32,
        spacing: f32,
        tint: Color,
    ) {
        UiFonts::draw_text(self, d, text, position, size, spacing, tint);
    }
}

impl TextFace for AuthoredText<'_> {
    fn measure_text(&self, text: &str, size: f32, spacing: f32) -> Vector2 {
        AuthoredText::measure_text(self, text, size, spacing)
    }

    fn draw_text<D: RaylibDraw>(
        &self,
        d: &mut D,
        text: &str,
        position: Vector2,
        size: f32,
        spacing: f32,
        tint: Color,
    ) {
        AuthoredText::draw_text(self, d, text, position, size, spacing, tint);
    }
}

/// Draws text left-aligned from `x`, dissolving the tail into the box edge.
///
/// For a label that is longer than the room it has and must still show its
/// beginning — a lyric cue block in the timeline, which is as wide as the cue is
/// long and rarely as wide as its words. An ellipsis answers "there is more" with
/// a glyph that costs three characters of the little space there was, and a hard
/// clip reads as a rendering fault. Fading the tail says the same thing with no
/// space at all. Nothing in the oracle does this; its only overflow policy is
/// [`row_typography::truncate_label`]'s ellipsis, which stays the right answer for
/// a button.
///
/// Generic over [`TextFace`] so a lyric cue can be drawn through the authored
/// face while chrome keeps the native-size bank.
///
/// Returns the width actually drawn, so a caller can place something after the
/// text, and `0.0` when nothing was drawn at all.
///
/// **A string that fits is not faded.** A ramp on fully visible text reads as a
/// promise of more words, which is worse than no signal, so the fade only exists
/// once the string is genuinely cut.
#[allow(
    clippy::too_many_arguments,
    reason = "draw_text plus the two widths that define the cut; a struct here would hide the position"
)]
pub fn draw_text_faded<D: RaylibDraw, F: TextFace>(
    d: &mut D,
    font: &F,
    text: &str,
    x: f32,
    y: f32,
    max_width: f32,
    fade_width: f32,
    font_size: f32,
    tint: Color,
) -> f32 {
    if text.is_empty()
        || !max_width.is_finite()
        || max_width <= 0.0
        || !font_size.is_finite()
        || font_size <= 0.0
    {
        return 0.0;
    }
    // Zero letter spacing throughout, which is what [`measure`] assumes and what
    // makes the accumulated advances add up to the width of the whole string.
    let width_of = |run: &str| font.measure_text(run, font_size, 0.0).x;
    let right_edge = x + max_width;
    // No fade unless the string is genuinely cut off.
    let cut = width_of(text) > max_width;
    let fade_width = if fade_width.is_finite() {
        fade_width.clamp(0.0, max_width)
    } else {
        0.0
    };
    let base_alpha = f32::from(tint.a);

    let mut cursor = x;
    let mut drawn_to = x;
    for character in text.chars() {
        // Stop at the edge rather than measuring the rest of the line: a long
        // lyric in a narrow block would otherwise cost a measure call per
        // character, every frame, for glyphs that cannot be seen.
        if cursor >= right_edge {
            break;
        }
        let mut bytes = [0u8; 4];
        let glyph = character.encode_utf8(&mut bytes);
        let advance = width_of(glyph);
        let alpha = if cut {
            fade_alpha(cursor + advance * 0.5, right_edge, fade_width, base_alpha)
        } else {
            base_alpha
        };
        let alpha = alpha.round().clamp(0.0, 255.0) as u8;
        if alpha > 0 {
            // The pen is carried unsnapped and both faces snap `position` to the
            // physical pixel grid themselves. Snapping here as well would round
            // the advance *into* the pen, so a long line would creep sideways by
            // a fraction of a pixel per glyph instead of each glyph landing
            // within half a pixel of where the whole string would have put it.
            font.draw_text(
                d,
                glyph,
                Vector2::new(cursor, y),
                font_size,
                0.0,
                Color::new(tint.r, tint.g, tint.b, alpha),
            );
            drawn_to = cursor + advance;
        }
        cursor += advance;
    }
    drawn_to - x
}

/// The fade ramp: full alpha until `fade_width` from the edge, zero at it.
///
/// Evaluated at each glyph's horizontal centre rather than its left edge, so a
/// glyph straddling the start of the ramp is already half faded instead of
/// stepping down whole letters at a time.
fn fade_alpha(glyph_center: f32, right_edge: f32, fade_width: f32, base_alpha: f32) -> f32 {
    let remaining = right_edge - glyph_center;
    if remaining <= 0.0 {
        return 0.0;
    }
    // A zero-width ramp is a hard cut, and dividing by it would be a NaN alpha
    // rather than the sharp edge the caller asked for.
    if fade_width <= 0.0 || remaining >= fade_width {
        return base_alpha;
    }
    base_alpha * remaining / fade_width
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
    font: &UiFonts,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    spacing: f32,
    tint: Color,
) {
    font.draw_text(
        d,
        text,
        Vector2::new(x.round(), y.round()),
        font_size,
        spacing.round(),
        tint,
    );
}

/// A titled panel background: surface fill, a hairline border, and a header rule.
/// Vertical chrome [`panel`] spends on its title row before the content rect
/// begins. Public so a layout floor derived outside this file — the export
/// panel's band minimum — can count it instead of rediscovering it as a
/// mystery 27 px (review 1.4).
pub const PANEL_HEADER_HEIGHT: f32 = 27.0;

pub fn panel(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    boundary: UiRect,
    title: &str,
) -> UiRect {
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
    let rule_y = boundary.y + PANEL_HEADER_HEIGHT;
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

// ---------------------------------------------------------------------------
// Colour, for the surfaces that let a user choose one
// ---------------------------------------------------------------------------
//
// **Invented, not the oracle's.** The frozen C offers colour only as fixed
// swatch tables (`lyrics_editor_ui.c:534-539`); free picking is an operator
// decision of 2026-08-03 and there is nothing in the C to port. Everything here
// is deliberately generic — a rectangle, a hue, a packed RGBA — because a second
// caller (a scene's tint, a route colour) should not have to reinvent a hue bar.
//
// The conversions are written out rather than taken from raylib's `ColorFromHSV`
// for two reasons: raylib has no inverse, and a pure-Rust pair round-trips in a
// unit test with no window open, which is the only place this arithmetic can be
// checked at all.

/// Split a `0xRRGGBBAA` word, the packing `Color::get_color` and every swatch
/// table in this application use.
#[must_use]
pub fn unpack_rgba(packed: u32) -> (u8, u8, u8, u8) {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "each shift is masked to one byte"
    )]
    (
        (packed >> 24) as u8,
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

/// The inverse of [`unpack_rgba`].
#[must_use]
pub fn pack_rgba(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    (u32::from(red) << 24) | (u32::from(green) << 16) | (u32::from(blue) << 8) | u32::from(alpha)
}

/// Hue in degrees `[0, 360)`, saturation and value in `[0, 1]`.
///
/// Hue is **not** recoverable from a grey or a black: both have zero chroma, so
/// this reports 0 for them. A picker that re-derived its bar position from the
/// colour every frame would therefore snap to red the instant a drag reached the
/// left or bottom edge of the square, which is why the caller keeps the hue.
#[must_use]
pub fn hsv_from_rgb(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let chroma = max - min;
    let hue = if chroma <= 0.0 {
        0.0
    } else if max == red {
        60.0 * (((green - blue) / chroma) % 6.0)
    } else if max == green {
        60.0 * ((blue - red) / chroma + 2.0)
    } else {
        60.0 * ((red - green) / chroma + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let saturation = if max <= 0.0 { 0.0 } else { chroma / max };
    (hue, saturation, max)
}

/// The inverse of [`hsv_from_rgb`], clamping rather than refusing: a drag can
/// hand this a fraction a hair outside the square.
#[must_use]
pub fn rgb_from_hsv(hue: f32, saturation: f32, value: f32) -> (u8, u8, u8) {
    let hue = if hue.is_finite() {
        hue.rem_euclid(360.0)
    } else {
        0.0
    };
    let saturation = saturation.clamp(0.0, 1.0);
    let value = value.clamp(0.0, 1.0);
    let chroma = value * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let floor = value - chroma;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [0, 1] before scaling, so the product is in [0, 255]"
    )]
    let byte = |channel: f32| ((channel + floor).clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(red), byte(green), byte(blue))
}

/// A checkerboard, so a translucent fill drawn over it reads as translucent
/// rather than as a slightly different flat colour.
///
/// Takes a cell **size** rather than a cell count: the same idiom backs a 22 px
/// swatch and a 120 px alpha bar, and a count would stretch the squares into
/// stripes on the wide one.
pub fn checkerboard<D: RaylibDraw>(d: &mut D, rect: UiRect, cell: f32) {
    if rect.is_empty() || !cell.is_finite() || cell <= 0.0 {
        return;
    }
    fill(d, rect, color::ui_raised());
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "both operands are finite and positive, and the rect is a UI control"
    )]
    let (columns, rows) = (
        (rect.width / cell).ceil() as usize,
        (rect.height / cell).ceil() as usize,
    );
    for row in 0..rows {
        for column in 0..columns {
            if (row + column) % 2 == 0 {
                continue;
            }
            let x = rect.x + column as f32 * cell;
            let y = rect.y + row as f32 * cell;
            // Clipped by construction rather than by a scissor: a partial cell at
            // the far edge is one rectangle, and starting a scissor per swatch
            // would be a GPU state change per control per frame.
            fill(
                d,
                UiRect::new(
                    x,
                    y,
                    cell.min(rect.x + rect.width - x),
                    cell.min(rect.y + rect.height - y),
                ),
                color::ui_surface(),
            );
        }
    }
}

/// The saturation/value square for one hue: white at the top left, the pure hue
/// at the top right, black along the bottom.
///
/// One quad, not a per-pixel loop. `DrawRectangleGradientEx` takes its corners
/// counter-clockwise from the top left (`rshapes.c`'s `DrawRectangleGradientEx`),
/// which is exactly the square's own definition, and the GPU interpolates what a
/// software loop would otherwise write a pixel at a time.
pub fn saturation_value_field<D: RaylibDraw>(d: &mut D, rect: UiRect, hue: f32) {
    if rect.is_empty() {
        return;
    }
    let (red, green, blue) = rgb_from_hsv(hue, 1.0, 1.0);
    d.draw_rectangle_gradient_ex(
        rectangle(rect),
        Color::new(255, 255, 255, 255),
        Color::new(0, 0, 0, 255),
        Color::new(0, 0, 0, 255),
        Color::new(red, green, blue, 255),
    );
}

/// A vertical hue ramp, red at the top through to red again at the bottom.
///
/// Six segments because hue is piecewise linear in RGB: a single gradient from
/// red to red is a constant, and any fewer than six would cut a chord through
/// the colour cube and lose the secondaries.
pub fn hue_bar<D: RaylibDraw>(d: &mut D, rect: UiRect) {
    if rect.is_empty() {
        return;
    }
    for segment in 0..6usize {
        let top = rect.y + rect.height * segment as f32 / 6.0;
        let bottom = rect.y + rect.height * (segment + 1) as f32 / 6.0;
        let (r, g, b) = rgb_from_hsv(segment as f32 * 60.0, 1.0, 1.0);
        let (nr, ng, nb) = rgb_from_hsv((segment + 1) as f32 * 60.0, 1.0, 1.0);
        let start = Color::new(r, g, b, 255);
        let end = Color::new(nr, ng, nb, 255);
        d.draw_rectangle_gradient_ex(
            Rectangle::new(rect.x, top, rect.width, bottom - top),
            start,
            end,
            end,
            start,
        );
    }
}

/// Transparent to opaque, left to right, over the checkerboard.
pub fn alpha_bar<D: RaylibDraw>(d: &mut D, rect: UiRect, tint: Color) {
    if rect.is_empty() {
        return;
    }
    checkerboard(d, rect, (rect.height * 0.5).max(2.0));
    let clear = Color::new(tint.r, tint.g, tint.b, 0);
    let solid = Color::new(tint.r, tint.g, tint.b, 255);
    d.draw_rectangle_gradient_ex(rectangle(rect), clear, clear, solid, solid);
}

/// Black or white, whichever a marker drawn over `background` will be seen in.
///
/// [`contrast::ratio`] is the judge, because it is the one already checked
/// against WCAG here — but it ignores alpha by design, and a marker on a
/// 10%-opaque swatch is really sitting on the checkerboard behind it. So the
/// colour is composited over the checkerboard's mid-grey first and the tested
/// ratio then decides.
#[must_use]
pub fn contrasting_ink(background: Color) -> Color {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a convex combination of two values in [0, 255]"
    )]
    let over = |channel: u8| -> u8 {
        let alpha = f32::from(background.a) / 255.0;
        (f32::from(channel) * alpha + 127.5 * (1.0 - alpha)).round() as u8
    };
    let packed = pack_rgba(
        over(background.r),
        over(background.g),
        over(background.b),
        255,
    );
    if contrast::ratio(0x0000_00FF, packed) >= contrast::ratio(0xFFFF_FFFF, packed) {
        Color::new(0, 0, 0, 255)
    } else {
        Color::new(255, 255, 255, 255)
    }
}

/// Begin a framebuffer scissor for a logical UI rectangle. Camera transforms do
/// not affect raylib scissors, so every UI clip must cross this boundary.
pub fn begin_scissor<'a, D: RaylibDraw>(
    d: &'a mut D,
    boundary: UiRect,
    scale: UiScale,
) -> RaylibScissorMode<'a, D> {
    let physical = scale.physical_rect(boundary);
    d.begin_scissor_mode(
        physical.x as i32,
        physical.y as i32,
        physical.width as i32,
        physical.height as i32,
    )
}

/// raylib's `ColorBrightness`, which the safe API only exposes for images.
pub(crate) fn brightness(color: Color, amount: f32) -> Color {
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

pub(crate) fn alpha(color: Color, amount: f32) -> Color {
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
    /// The export panel's controls.
    pub const EXPORT: u32 = 7;
    /// The transport row's fine-seek group. A namespace of its own rather than
    /// more `TOOLBAR` indices, so that shedding the group cannot renumber the
    /// buttons that stay — a press claimed before a resize would otherwise be
    /// released by whichever control inherited its index.
    pub const SEEK: u32 = 8;
    /// The transport row's right-hand cluster: readout toggle, mute, volume,
    /// fullscreen. Separate for the same reason as SEEK.
    pub const UTILITY: u32 = 9;
    /// One per notice card's close box, indexed by the notice's own id — the
    /// id rather than the list index, so eviction cannot misroute a claim.
    pub const NOTICE: u32 = 64;
    /// The collapsed tracks strip's Save button (review 1.12, UX0-A12).
    pub const TRACK_STRIP: u32 = 65;
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
    fn tabular_digits_hold_live_timecode_geometry_without_a_monospace_face() {
        // Deliberately proportional fake glyphs: 1 is narrow, 5 is wide, and
        // punctuation keeps its own advance. Ordinary measurement would make
        // these two strings differ by 36 units.
        let measure = |character: char| match character {
            '1' => 2.0,
            '5' => 8.0,
            ':' | '.' => 1.0,
            _ => 4.0,
        };
        let digit_cell = 8.0;
        assert_eq!(
            tabular_width("00:11.111", digit_cell, measure),
            tabular_width("00:55.555", digit_cell, measure)
        );
        // Non-numeric copy is still proportional rather than being forced into
        // cells, which is what keeps Space Grotesk looking like Space Grotesk.
        assert_eq!(tabular_width("1x", digit_cell, measure), 12.0);
    }

    /// A 100 px box whose last 20 px are the fade, drawn at full opacity.
    const EDGE: f32 = 100.0;
    const RAMP: f32 = 20.0;
    const OPAQUE: f32 = 255.0;

    #[test]
    fn a_glyph_that_stops_short_of_the_ramp_keeps_the_callers_alpha() {
        assert_eq!(fade_alpha(0.0, EDGE, RAMP, OPAQUE), OPAQUE);
        assert_eq!(fade_alpha(50.0, EDGE, RAMP, OPAQUE), OPAQUE);
        // The first pixel of the ramp is still fully opaque; the fade starts here
        // rather than having already begun.
        assert_eq!(fade_alpha(EDGE - RAMP, EDGE, RAMP, OPAQUE), OPAQUE);
        // A dimmed caller keeps its own ceiling instead of being pushed to 255.
        assert_eq!(fade_alpha(50.0, EDGE, RAMP, 120.0), 120.0);
    }

    #[test]
    fn the_ramp_is_linear_and_reaches_zero_at_the_box_edge() {
        assert_eq!(fade_alpha(EDGE, EDGE, RAMP, OPAQUE), 0.0);
        assert_eq!(
            fade_alpha(EDGE - RAMP * 0.5, EDGE, RAMP, OPAQUE),
            OPAQUE * 0.5
        );
        assert_eq!(
            fade_alpha(EDGE - RAMP * 0.25, EDGE, RAMP, OPAQUE),
            OPAQUE * 0.25
        );
        assert_eq!(
            fade_alpha(EDGE - RAMP * 0.75, EDGE, RAMP, OPAQUE),
            OPAQUE * 0.75
        );
    }

    #[test]
    fn a_zero_width_fade_is_a_hard_cut() {
        assert_eq!(fade_alpha(EDGE - 0.5, EDGE, 0.0, OPAQUE), OPAQUE);
        assert_eq!(fade_alpha(EDGE, EDGE, 0.0, OPAQUE), 0.0);
        assert_eq!(fade_alpha(EDGE + 0.5, EDGE, 0.0, OPAQUE), 0.0);
    }

    #[test]
    fn a_glyph_centred_past_the_edge_is_invisible_rather_than_negative() {
        // Multiplying the ramp out without this guard gives a negative alpha,
        // which casts to a *bright* byte rather than to nothing.
        assert_eq!(fade_alpha(EDGE + 5.0, EDGE, RAMP, OPAQUE), 0.0);
        assert_eq!(fade_alpha(EDGE * 4.0, EDGE, RAMP, OPAQUE), 0.0);
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

    /// Two disjoint control boxes: the panel's, and the toolbar's.
    fn box_a() -> UiRect {
        UiRect::new(0.0, 0.0, 40.0, 20.0)
    }

    fn box_b() -> UiRect {
        UiRect::new(100.0, 0.0, 40.0, 20.0)
    }

    fn over(rect: UiRect) -> Pointer {
        Pointer {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
            ..Pointer::default()
        }
    }

    fn press(rect: UiRect) -> Pointer {
        Pointer {
            down: true,
            pressed: true,
            ..over(rect)
        }
    }

    fn hold(rect: UiRect) -> Pointer {
        Pointer {
            down: true,
            ..over(rect)
        }
    }

    fn release(rect: UiRect) -> Pointer {
        Pointer {
            released: true,
            ..over(rect)
        }
    }

    #[test]
    fn a_press_is_awarded_on_release_over_the_same_widget() {
        let mut widgets = Widgets::new();
        let scale = UiScale::default();

        widgets.begin_frame(scale);
        assert!(!widgets.button_at(1, box_a(), press(box_a())).clicked);
        widgets.begin_frame(scale);
        assert!(!widgets.button_at(1, box_a(), hold(box_a())).clicked);
        widgets.begin_frame(scale);
        assert!(widgets.button_at(1, box_a(), release(box_a())).clicked);
    }

    #[test]
    fn a_neighbour_cannot_take_a_press_another_widget_claimed() {
        let mut widgets = Widgets::new();
        let scale = UiScale::default();

        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), press(box_a()));
        // The pointer leaves A for B while held: B must stay inert, or dragging
        // past the end of a slider would become a click on whatever is next to it.
        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), hold(box_b()));
        assert!(!widgets.button_at(2, box_b(), hold(box_b())).clicked);
        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), release(box_b()));
        assert!(!widgets.button_at(2, box_b(), release(box_b())).clicked);
    }

    /// UX0-A02. Hold a slider, press `T`: the panel closes, the widget is never
    /// drawn again, and before this every control in the application was dead for
    /// the rest of the session.
    #[test]
    fn a_claim_stranded_by_a_vanishing_panel_does_not_freeze_every_other_control() {
        let mut widgets = Widgets::new();
        let scale = UiScale::default();

        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), press(box_a()));

        // The panel closes. Its widget is gone; the toolbar's is not, and it is
        // what the safety net sees the pointer through.
        widgets.begin_frame(scale);
        widgets.button_at(2, box_b(), hold(box_b()));
        widgets.begin_frame(scale);
        widgets.button_at(2, box_b(), release(box_b()));
        widgets.begin_frame(scale);
        widgets.button_at(2, box_b(), over(box_b()));

        // A fresh press on an unrelated control now behaves normally.
        widgets.begin_frame(scale);
        widgets.button_at(2, box_b(), press(box_b()));
        widgets.begin_frame(scale);
        assert!(widgets.button_at(2, box_b(), release(box_b())).clicked);
    }

    /// The safety net's failure mode would be worse than the bug: a net that fired
    /// on the release frame would swallow every click in the interface.
    #[test]
    fn the_stranded_claim_net_never_eats_a_click_the_owner_would_have_delivered() {
        let mut widgets = Widgets::new();
        let scale = UiScale::default();

        // The slowest legitimate case: one frame of press, many of hold, then a
        // release frame on which the pointer is already up.
        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), press(box_a()));
        for _ in 0..30 {
            widgets.begin_frame(scale);
            assert!(!widgets.button_at(1, box_a(), hold(box_a())).clicked);
        }
        widgets.begin_frame(scale);
        assert!(widgets.button_at(1, box_a(), release(box_a())).clicked);
    }

    /// A drag whose pointer is outside its own box is the reason the claim rule
    /// exists (`ui_widgets.c:139-160`), so the net must not cut one short.
    #[test]
    fn a_drag_that_leaves_its_own_box_keeps_the_claim() {
        let mut widgets = Widgets::new();
        let scale = UiScale::default();

        widgets.begin_frame(scale);
        widgets.button_at(1, box_a(), press(box_a()));
        for _ in 0..10 {
            widgets.begin_frame(scale);
            // Far outside, still held: exactly what dragging past the end of a
            // slider looks like.
            widgets.button_at(1, box_a(), hold(box_b()));
        }
        // Still owned, so the value is still the slider's to report.
        assert_eq!(widgets.active_button_id, 1);
        widgets.begin_frame(scale);
        // And the release still lands on the owner even though the pointer never
        // came back — no click, because the release is outside the box, but the
        // claim is spent by the widget that made it.
        assert!(!widgets.button_at(1, box_a(), release(box_b())).clicked);
        assert_eq!(widgets.active_button_id, 0);
    }

    /// The two conversions are inverses over every colour a swatch table or a
    /// picker can produce.
    ///
    /// Sampled rather than exhaustive over 16.7 M, but the sampling is on a
    /// stride coprime with 256 so it is not a lattice of round numbers: an
    /// off-by-one in the sector arithmetic shows up at the boundaries, and 60,
    /// 120, 180, 240 and 300 degrees are all hit exactly.
    #[test]
    fn hsv_and_rgb_round_trip_through_each_other() {
        for red in (0..=255u32).step_by(17) {
            for green in (0..=255u32).step_by(23) {
                for blue in (0..=255u32).step_by(29) {
                    let (r, g, b) = (red as u8, green as u8, blue as u8);
                    let (hue, saturation, value) = hsv_from_rgb(r, g, b);
                    assert!(
                        (0.0..360.0).contains(&hue),
                        "{r},{g},{b} produced hue {hue}"
                    );
                    assert!((0.0..=1.0).contains(&saturation));
                    assert!((0.0..=1.0).contains(&value));
                    let back = rgb_from_hsv(hue, saturation, value);
                    assert_eq!(back, (r, g, b), "{r},{g},{b} came back as {back:?}");
                }
            }
        }
        // The six primaries and secondaries by hue, which is what the hue bar's
        // six segments are drawn from.
        for (degrees, expected) in [
            (0.0, (255u8, 0u8, 0u8)),
            (60.0, (255, 255, 0)),
            (120.0, (0, 255, 0)),
            (180.0, (0, 255, 255)),
            (240.0, (0, 0, 255)),
            (300.0, (255, 0, 255)),
            (360.0, (255, 0, 0)),
        ] {
            assert_eq!(rgb_from_hsv(degrees, 1.0, 1.0), expected, "{degrees} deg");
        }
    }

    /// A drag can hand the conversion a fraction a hair outside the square, and
    /// a `NaN` hue is what an unwritten picker slot would hold.
    #[test]
    fn rgb_from_hsv_clamps_rather_than_wrapping_into_a_wrong_colour() {
        assert_eq!(rgb_from_hsv(0.0, 1.5, 1.5), (255, 0, 0));
        assert_eq!(rgb_from_hsv(0.0, -0.5, 1.0), (255, 255, 255));
        assert_eq!(rgb_from_hsv(-60.0, 1.0, 1.0), (255, 0, 255));
        assert_eq!(rgb_from_hsv(420.0, 1.0, 1.0), (255, 255, 0));
        assert_eq!(rgb_from_hsv(f32::NAN, 1.0, 1.0), (255, 0, 0));
    }

    #[test]
    fn packing_is_the_order_every_swatch_table_is_written_in() {
        // `0xFFFF_FFC0` in `CAPTION_TEXT_SWATCHES` is white at 75% alpha, not a
        // pale yellow: the byte order is the one `Color::get_color` reads.
        assert_eq!(unpack_rgba(0xFFFF_FFC0), (255, 255, 255, 0xC0));
        assert_eq!(unpack_rgba(0x1B2A_5AC0), (0x1B, 0x2A, 0x5A, 0xC0));
        for packed in [0x0000_0000u32, 0xFFFF_FFFF, 0x1234_5678, 0xF2BE_42FF] {
            let (r, g, b, a) = unpack_rgba(packed);
            assert_eq!(pack_rgba(r, g, b, a), packed);
        }
        let raylib_order = Color::get_color(0x1B2A_5AC0);
        assert_eq!(
            (
                raylib_order.r,
                raylib_order.g,
                raylib_order.b,
                raylib_order.a
            ),
            unpack_rgba(0x1B2A_5AC0)
        );
    }

    /// The marker on a custom swatch has to be legible on the colour the swatch
    /// is showing, including when that colour is nearly transparent and what is
    /// really behind it is the checkerboard.
    #[test]
    fn a_marker_ink_is_chosen_against_what_is_actually_behind_it() {
        assert_eq!(contrasting_ink(Color::new(255, 255, 255, 255)).r, 0);
        assert_eq!(contrasting_ink(Color::new(0, 0, 0, 255)).r, 255);
        // Black at 6% opacity is a light grey on screen, so black ink on it
        // would be the unreadable choice.
        assert_eq!(contrasting_ink(Color::new(0, 0, 0, 16)).r, 0);
        assert_eq!(contrasting_ink(Color::new(242, 190, 66, 255)).r, 0);
    }
}
