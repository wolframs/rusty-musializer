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
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibFont,
    RaylibScissorMode, RaylibScissorModeExt, Rectangle, Vector2,
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

/// `Button_Style` (`ui_widgets.h:41-44`), plus [`ButtonStyle::Primary`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Selected draws as an outline and a tint (WX-5a), never as a fill in the
    /// accent: the accent means *now*, and a selected scene tile is not now.
    Neutral,
    /// The primary action — Render — and nothing else: the one control that
    /// wears the accent as a fill, selected or not (WX-5a).
    Primary,
    /// A control that only draws a box when it is hovered or selected: the
    /// rail's icons and the transport's chips (WX-5c). Idle, it is its glyph in
    /// muted ink on whatever surface it sits on, so a column of four does not
    /// read as four buttons stacked on a panel. Selected is the same outline
    /// and tint every other selected control wears.
    Ghost,
    /// The commit inside a column card — Apply, Keep, Allow, Start analysis —
    /// that is not the frame's primary action (WX-5d §2): an ink fill with the
    /// label cut out in the surface colour, the round play button's look.
    /// **Never the accent**, so the frame's accent census stays at one: the
    /// accent fill is Render's, or the Render view's commit, and nothing else.
    #[allow(
        dead_code,
        reason = "WX-5d seam: the column views' builders are the callers"
    )]
    Strong,
    // No caller yet. It is the style for a destructive action, and the two the
    // oracle has — `Clear manual` and Cancel export — are Agents L and H's. Kept
    // rather than deleted because it is half of the C's widget vocabulary and
    // re-deriving the colour ramp later would be worse than an allow here.
    #[allow(dead_code)]
    Danger,
}

/// How a button draws in one state: its fill, its outline and its label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonLook {
    pub fill: Color,
    pub edge: Color,
    pub ink: Color,
}

/// The one table every labelled button reads (WX-5a).
///
/// Public so a surface that draws its own selectable rows — the font list, the
/// cue list — can wear exactly what a button wears rather than inventing a
/// third selected state. Pressed is applied on top by the caller, as a darker
/// fill and a 2 px edge.
#[must_use]
pub fn button_look(style: ButtonStyle, selected: bool, hovered: bool) -> ButtonLook {
    match style {
        // Selected or not: a commit is not a selection, and an ink slab that
        // turned into an outline when "selected" would stop reading as one.
        ButtonStyle::Strong => {
            let ink = color::ui_ink();
            ButtonLook {
                fill: if hovered { brightness(ink, 0.08) } else { ink },
                edge: ink,
                ink: color::ui_surface(),
            }
        }
        ButtonStyle::Primary => {
            let accent = color::accent();
            ButtonLook {
                fill: if hovered {
                    brightness(accent, 0.10)
                } else {
                    accent
                },
                edge: accent,
                ink: color::on_accent(),
            }
        }
        // An armed destructive control stays a solid slab: "this will delete"
        // is a warning, not a selection, and it has to stop the eye.
        ButtonStyle::Danger if selected => {
            let danger = color::ui_danger();
            let fill = if hovered {
                brightness(danger, 0.12)
            } else {
                danger
            };
            ButtonLook {
                fill,
                edge: danger,
                ink: contrasting_ink(fill),
            }
        }
        ButtonStyle::Danger => ButtonLook {
            fill: if hovered {
                color::ui_hover()
            } else {
                color::ui_control()
            },
            edge: alpha(color::ui_danger(), 0.72),
            ink: color::ui_ink(),
        },
        ButtonStyle::Ghost if selected => ButtonLook {
            fill: color::ui_selected(),
            edge: color::ui_selected_edge(),
            ink: color::ui_ink(),
        },
        ButtonStyle::Ghost if hovered => ButtonLook {
            fill: color::ui_hover(),
            edge: alpha(color::ui_hover(), 0.0),
            ink: color::ui_ink(),
        },
        ButtonStyle::Ghost => ButtonLook {
            fill: alpha(color::ui_hover(), 0.0),
            edge: alpha(color::ui_hover(), 0.0),
            ink: color::ui_muted(),
        },
        ButtonStyle::Neutral if selected => ButtonLook {
            fill: color::ui_selected(),
            edge: color::ui_selected_edge(),
            ink: color::ui_ink(),
        },
        ButtonStyle::Neutral => ButtonLook {
            fill: if hovered {
                color::ui_hover()
            } else {
                color::ui_control()
            },
            edge: color::ui_control_edge(),
            ink: color::ui_ink(),
        },
    }
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

/// A synthesized left-button state for `--ui-probe click=` (EX1).
///
/// Only the three edges, never the position: the pointer is parked with
/// `SetMousePosition` like `hover=` does, so a probed press lands wherever the
/// real hit test says it lands. Injecting a coordinate here as well would let a
/// probe "click" a control the physical pointer could never reach, which is the
/// one thing the check has to be unable to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerProbe {
    pub down: bool,
    pub pressed: bool,
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
    /// `--ui-probe click=`'s synthesized button state, or `None` for the real
    /// device. Set by the composition root before the shell draws and cleared by
    /// it when the press has been delivered; nothing inside the shell reads or
    /// writes it, so no control can behave differently because a probe is on.
    pointer_probe: Option<PointerProbe>,
    /// The last id to claim a press, kept after the claim is released so the
    /// probe's report line can name *what* took the click.
    ///
    /// This is the whole diagnostic value of the click probe. "The button did
    /// not fire" has two very different causes — the press never reached the
    /// control, or a control on top of it took the press — and a capture cannot
    /// tell them apart, because both leave the same picture on screen.
    last_claimed_id: u64,
    /// Input boundary paired with a scrolling surface's drawing scissor.
    input_clip: Option<UiRect>,
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
        self.input_clip = None;
        self.ui_scale = ui_scale;
    }

    /// Swaps the scale the pointer is read through, returning the one it
    /// replaces (WX-5c). The shell installs an occluded scale while it draws
    /// what an open flyout covers, and puts the plain one back for the flyout
    /// itself; see [`UiScale::occluded`].
    pub fn set_ui_scale(&mut self, ui_scale: UiScale) -> UiScale {
        std::mem::replace(&mut self.ui_scale, ui_scale)
    }

    /// Restrict new presses and hover to the visible viewport. Restore the
    /// returned boundary when leaving the scissor; active drags still release
    /// normally outside it.
    pub fn set_input_clip(&mut self, clip: Option<UiRect>) -> Option<UiRect> {
        std::mem::replace(&mut self.input_clip, clip)
    }

    /// Installs (or clears) `--ui-probe click=`'s synthesized button state.
    ///
    /// Deliberately *not* reset by [`Self::begin_frame`]: the caller sets it for
    /// exactly the frames the press occupies, and a reset here would make the
    /// order of two calls in the composition root load-bearing.
    pub fn set_pointer_probe(&mut self, probe: Option<PointerProbe>) {
        self.pointer_probe = probe;
    }

    /// The last widget id to claim a press, or `0` if none ever has.
    #[must_use]
    pub fn last_claimed_id(&self) -> u64 {
        self.last_claimed_id
    }

    /// This frame's pointer, with the click probe applied if one is installed.
    ///
    /// One seam, so a probe cannot reach half the widgets: every control in the
    /// shell goes through [`Self::button`] or [`Self::slider`], and both read
    /// here.
    ///
    /// `pub(crate)` for the surfaces that are not widgets but take presses —
    /// the scene plan lane's select-and-drag (WX-5b) — so a `click=` probe
    /// reaches them through the same seam a button's press does.
    pub(crate) fn pointer(&self, d: &RaylibDrawHandle<'_>) -> Pointer {
        let mut pointer = Pointer::read(d, self.ui_scale);
        if let Some(probe) = self.pointer_probe {
            pointer.down = probe.down;
            pointer.pressed = probe.pressed;
            pointer.released = probe.released;
        }
        pointer
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
        self.button_at(id, boundary, self.pointer(d))
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
        let hovered = boundary.contains_point(pointer.x, pointer.y)
            && self
                .input_clip
                .is_none_or(|clip| clip.contains_point(pointer.x, pointer.y));

        let mut clicked = false;
        if self.active_button_id == 0 {
            if hovered && pointer.pressed {
                self.active_button_id = id;
                self.last_claimed_id = id;
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
        let look = button_look(style, selected, state.hovered);
        let background = if state.pressed {
            brightness(look.fill, -0.08)
        } else {
            look.fill
        };
        control_box(
            d,
            boundary,
            background,
            look.edge,
            if state.pressed { 2.0 } else { 1.0 },
        );
        draw_button_label(
            d,
            font,
            boundary,
            label,
            font_size,
            if state.pressed { 1.0 } else { 0.0 },
            look.ink,
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
        control_box(d, boundary, color::ui_surface(), color::ui_rule(), 1.0);
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
        let pointer = self.pointer(d);
        let track = UiRect::new(
            boundary.x,
            boundary.y + boundary.height * 0.5 - 3.0,
            boundary.width,
            6.0,
        );
        // A filled amount is the level tone and the knob is ink (WX-5a): a
        // slider is not "now", and a row of accent sliders was half of why the
        // old accent stopped meaning anything.
        d.draw_rectangle_rec(rectangle(track), color::ui_control_edge());
        let filled = normalized.clamp(0.0, 1.0);
        d.draw_rectangle_rec(
            rectangle(UiRect::new(
                track.x,
                track.y,
                track.width * filled,
                track.height,
            )),
            color::ui_level(),
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
            color::ui_ink(),
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

impl Widgets {
    /// The transport's volume chip (WX-5c, the mockup's `70%`): the level
    /// as a fill from the left, the percentage on top, and a drag across it
    /// that sets the level like a slider. Returns the level the pointer asks
    /// for while it is dragged, as [`Self::slider`] does, so an untouched
    /// chip cannot write a value.
    ///
    /// The fill is the level tone, never the accent: a level is an amount,
    /// not *now*. While muted the percentage still shows the stored level,
    /// in the disabled ink, because mute is a toggle the user expects to undo.
    #[allow(clippy::too_many_arguments)]
    pub fn volume_chip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        id: u64,
        boundary: UiRect,
        level: f32,
        muted: bool,
    ) -> (ButtonState, Option<f32>) {
        if boundary.is_empty() {
            return (ButtonState::default(), None);
        }
        let pointer = self.pointer(d);
        let state = self.button_at(id, boundary, pointer);
        let look = button_look(ButtonStyle::Neutral, false, state.hovered || state.pressed);
        control_box(d, boundary, look.fill, look.edge, 1.0);
        let level = level.clamp(0.0, 1.0);
        let inset = 3.0;
        let fill = UiRect::new(
            boundary.x + inset,
            boundary.y + boundary.height - inset - 3.0,
            (boundary.width - inset * 2.0) * level,
            3.0,
        );
        d.draw_rectangle_rec(
            rectangle(fill),
            if muted {
                color::ui_disabled()
            } else {
                color::ui_level()
            },
        );
        let text = format!("{:.0} %", level * 100.0);
        text_label(
            d,
            font,
            UiRect::new(
                boundary.x,
                boundary.y - 1.0,
                boundary.width,
                boundary.height,
            ),
            &text,
            metric::UI_FONT_CAPTION,
            if muted {
                color::ui_disabled()
            } else {
                look.ink
            },
        );
        if self.active_button_id == id && pointer.down {
            self.interacted = true;
            return (
                state,
                Some(slider_value(
                    pointer.x,
                    boundary.x + inset,
                    boundary.x + boundary.width - inset,
                )),
            );
        }
        (state, None)
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

/// A tooltip's plate, for [`draw_tooltip`] and for a caller that types its
/// own text on it (the cue lane's tip, in the authored face).
///
/// The floating-surface shape (WX-5 follow-up): the 4 px corner and the soft
/// shadow a notice card has. The tooltip keeps its own tokens — dark in both
/// appearances, the convention a tip is read by.
pub fn tooltip_plate(d: &mut RaylibDrawHandle<'_>, boundary: UiRect) {
    floating_shadow(d, boundary);
    control_box(
        d,
        boundary,
        color::ui_tooltip_surface(),
        color::ui_tooltip_edge(),
        1.0,
    );
}

/// Draws a tooltip above everything else.
///
/// Opaque theme-specific colors keep tooltip text readable over chrome or video.
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
    tooltip_plate(d, boundary);
    draw_text(
        d,
        font,
        &tooltip.text,
        boundary.x + TOOLTIP_PADDING_X,
        boundary.y + TOOLTIP_PADDING_Y,
        size,
        color::ui_tooltip_ink(),
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
    // Capped at the type scale's row ceiling (WX-5a). The C's base grows with
    // the box to 22, which in a 54 px scene tile is a label shouting at the
    // preview; the fitting itself is the C's and is unchanged below the cap.
    let base =
        font.native_size(row_typography::base_font_size(box_height).min(metric::UI_FONT_ROW_MAX));
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

/// Width of `text` as [`draw_text_tracked`] draws it with `spacing`: the same
/// rounding, then the font's own snap of the spacing onto physical pixels.
#[must_use]
pub fn measure_tracked(font: &UiFonts, text: &str, font_size: f32, spacing: f32) -> f32 {
    font.measure_text(text, font_size, spacing.round()).x
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
        LabelFont::Exact(_) | LabelFont::Authored(_) => font_size
            .filter(|value| *value > 0.0)
            .unwrap_or(base.min(metric::UI_FONT_ROW_MAX)),
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
/// masthead rather than a label. Deliberately separate from [`draw_text`], so
/// the linearity [`measure`] depends on is never at risk. Tracked text that has
/// to fit (the lane head's label, the band's gesture hint) is measured with
/// [`measure_tracked`], never as `measure + one pixel a character`: the font
/// snaps the spacing to whole physical pixels, which at 150 % is 1.33 logical
/// a character, and the shortfall clipped the hint and ran the label into Auto.
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

/// A glyph from the icon face, centred in `boundary` exactly as a button's
/// label is (WX-5c: the round play button draws its own disc).
pub fn icon_label(
    d: &mut RaylibDrawHandle<'_>,
    face: &Face,
    boundary: UiRect,
    glyph: &str,
    size: f32,
    tint: Color,
) {
    draw_button_label(
        d,
        LabelFont::Exact(face),
        boundary,
        glyph,
        Some(size),
        0.0,
        tint,
    );
}

/// Text centred in `boundary` exactly as a button's label is.
pub fn text_label(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    boundary: UiRect,
    text: &str,
    size: f32,
    tint: Color,
) {
    draw_button_label(
        d,
        LabelFont::Ui(font),
        boundary,
        text,
        Some(size),
        0.0,
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
    // The mockup's section head (WX-5a): small, uppercase, tracked and muted,
    // so a panel says what it is without competing with what is in it.
    draw_text_tracked(
        d,
        font,
        &title.to_uppercase(),
        boundary.x + metric::UI_PANEL_PADDING,
        boundary.y + 9.0,
        metric::UI_FONT_MICRO,
        1.0,
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

/// A control's box: a fill and an outline with the type scale's corner radius
/// (WX-5a, the mockup's controls). Lanes and list rows stay square — they are
/// data laid on a time or list axis, and a rounded corner there would read as
/// a gap in it — so this is for things you press.
pub fn control_box<D: RaylibDraw>(
    d: &mut D,
    boundary: UiRect,
    fill: Color,
    edge: Color,
    thickness: f32,
) {
    rounded_box(d, boundary, metric::UI_RADIUS, fill, edge, thickness);
}

/// [`control_box`] at any corner radius: the segmented control's 6 px frame
/// (WX-5d) is the first box that is not a control's 4 px. `control_box` is this
/// at [`metric::UI_RADIUS`], arithmetic for arithmetic, so no existing control
/// moves a pixel.
pub fn rounded_box<D: RaylibDraw>(
    d: &mut D,
    boundary: UiRect,
    radius: f32,
    fill: Color,
    edge: Color,
    thickness: f32,
) {
    if boundary.is_empty() {
        return;
    }
    let rect = rectangle(boundary);
    let short = boundary.width.min(boundary.height);
    if short < radius * 4.0 {
        d.draw_rectangle_rec(rect, fill);
        d.draw_rectangle_lines_ex(rect, thickness, edge);
        return;
    }
    // raylib's roundness is the radius as a fraction of half the short side.
    let roundness = (radius * 2.0 / short).clamp(0.0, 1.0);
    d.draw_rectangle_rounded(rect, roundness, 4, fill);
    // `DrawRectangleRoundedLinesEx` strokes *outside* its rectangle
    // (`rshapes.c:1183`, `outerRadius = radius + lineThick` at `:1203`), unlike
    // `DrawRectangleLinesEx`, which strokes inside. Handed the boundary itself,
    // every rounded control grew by its stroke on each side — the AI dialog's
    // rail measured 2 px closer to its divider than the dialog reported. So the
    // stroke goes round a rectangle inset by its own width, at the radius that
    // puts its outer edge on the fill's.
    let inset = UiRect::new(
        boundary.x + thickness,
        boundary.y + thickness,
        boundary.width - thickness * 2.0,
        boundary.height - thickness * 2.0,
    );
    let inner_short = short - thickness * 2.0;
    let inner_roundness = ((radius - thickness).max(0.0) * 2.0 / inner_short).clamp(0.0, 1.0);
    d.draw_rectangle_rounded_lines_ex(rectangle(inset), inner_roundness, 4, thickness, edge);
}

/// How far a floating surface's shadow reaches past its edge, and how far it
/// drops below it. Public so a caller placing something beside a floating
/// surface can keep it out of the shadow.
pub const FLOAT_SHADOW_SPREAD: f32 = 6.0;
pub const FLOAT_SHADOW_DROP: f32 = 2.0;

/// A surface that floats over the work rather than sitting in the layout: a
/// notice card, a context menu, a pill over the preview (WX-5 follow-up).
///
/// One language with the flyout and the tooltip: the appearance's own raised
/// surface, a hairline edge, the controls' 4 px corner and a soft shadow
/// under it, so it lifts off whatever the scene is drawing. Opaque, so text on
/// it reads at the ratio the palette sweep measured (review 1.11).
///
/// The shadow is a blend and composites **premultiplied**: `BLEND_ALPHA`
/// would leave the pixels it touches translucent in a saved PNG
/// (`docs/TRAPS.md`). Its pixels round per renderer, so a capture check reads
/// the card's interior (an opaque fill) and never the shadow.
pub fn floating_surface(d: &mut RaylibDrawHandle<'_>, boundary: UiRect) {
    if boundary.is_empty() {
        return;
    }
    floating_shadow(d, boundary);
    control_box(
        d,
        boundary,
        color::float_surface(),
        color::ui_control_edge(),
        1.0,
    );
}

/// The shadow [`floating_surface`] lays under its box, on its own.
pub fn floating_shadow(d: &mut RaylibDrawHandle<'_>, boundary: UiRect) {
    if boundary.is_empty() {
        return;
    }
    // Rings of a faint black, each one pixel wider than the last: where they
    // overlap, near the edge, the shadow is darkest, and it falls off outward.
    // Premultiplied black is plain black with alpha, so the tint is unchanged.
    const RINGS: u8 = FLOAT_SHADOW_SPREAD as u8;
    const RING_ALPHA: u8 = 9;
    d.draw_blend_mode(BlendMode::BLEND_ALPHA_PREMULTIPLY, |mut blend| {
        for ring in 1..=RINGS {
            let grow = f32::from(ring);
            // Each ring reaches half as far above the surface as below it,
            // so the shadow falls under the surface rather than haloing it.
            let rect = UiRect::new(
                boundary.x - grow,
                boundary.y - grow * 0.5 + FLOAT_SHADOW_DROP,
                boundary.width + grow * 2.0,
                boundary.height + grow * 1.5,
            );
            let short = rect.width.min(rect.height);
            let roundness = ((metric::UI_RADIUS + grow) * 2.0 / short).clamp(0.0, 1.0);
            blend.draw_rectangle_rounded(
                rectangle(rect),
                roundness,
                6,
                Color::new(0, 0, 0, RING_ALPHA),
            );
        }
    });
}

/// A selected row or tile drawn by hand: the selected tint and its outline,
/// exactly what [`button_look`] gives a selected button (WX-5a). For surfaces
/// that cannot be a `text_button` because they carry more than one label.
pub fn selected_fill<D: RaylibDraw>(d: &mut D, rect: UiRect) {
    if rect.is_empty() {
        return;
    }
    d.draw_rectangle_rec(rectangle(rect), color::ui_selected());
    d.draw_rectangle_lines_ex(rectangle(rect), 1.0, color::ui_selected_edge());
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

// ---------------------------------------------------------------------------
// The column vocabulary (WX-5d)
// ---------------------------------------------------------------------------
//
// One set of pieces for every view in the right column — Direct, Lyrics, Look,
// Assist, Render — so the five read as one surface (`wx5d-design.md` §2, "The
// head"). Every number below is the approved mockup's (`build/redesign/wx5d-src/
// mock.py`'s CSS), and every widget returns the rects it drew, because each view
// prints a report line and a probe aims at a report line, never at a pixel
// measured once.
//
// Each widget is a pure `*_layout` function, tested here, and a drawing wrapper
// that calls it. The split is the file's existing one (`tooltip_box` and
// `draw_tooltip`): geometry nobody can check without a window is geometry
// nobody checks.

#[allow(
    unused_imports,
    reason = "WX-5d seam: unused until the column views call the vocabulary"
)]
pub use column_kit::*;

/// The vocabulary in a module of its own only so one `allow` covers it while
/// nothing calls it: the WX-5d seam lands it before the Lyrics and Look &
/// Assist builders do. Drop the `allow` (and fold the module back into the
/// file if you like) once the views use it.
#[allow(
    dead_code,
    reason = "WX-5d seam: the column views' builders are the callers; the tests below exercise every layout"
)]
mod column_kit {
    use super::*;

    /// The column's head: a micro title, the view's switch or actions on the
    /// right, and a hairline under it (`.head`, 40 px).
    pub const COLUMN_HEAD_HEIGHT: f32 = 40.0;
    /// Left and right insets of the head's contents (`.head`'s `padding: 0 10px 0 14px`).
    pub const COLUMN_HEAD_PADDING_LEFT: f32 = 14.0;
    pub const COLUMN_HEAD_PADDING_RIGHT: f32 = 10.0;
    /// Space between the head's title and whatever sits after it (`.head`'s `gap`).
    pub const COLUMN_HEAD_GAP: f32 = 8.0;

    /// A section head inside a body: the micro title, then a hairline to the right
    /// edge (`.gh`, 18 px).
    pub const SECTION_HEAD_HEIGHT: f32 = 18.0;
    const SECTION_HEAD_GAP: f32 = 8.0;

    /// One parameter row: label, track, value, route button (`.row`, 30 px; it was
    /// 46 px in two lines in the Tune panel).
    pub const SLIDER_ROW_HEIGHT: f32 = 30.0;
    /// Between the row's four parts (`.row`'s `gap`). Wider than the knob's
    /// hovered radius, so a knob at 0 or 1 never touches the label or the value.
    pub const SLIDER_ROW_GAP: f32 = 10.0;
    /// The value's reserved width (`.val`), right-aligned tabular figures.
    pub const SLIDER_ROW_VALUE_WIDTH: f32 = 46.0;
    /// The `~` route button's box (`.rt`).
    pub const SLIDER_ROW_ROUTE_SIZE: f32 = 22.0;
    /// The changed dot (`.cd`) and the space between it and the label's words.
    pub const SLIDER_ROW_DOT_SIZE: f32 = 5.0;
    const SLIDER_ROW_DOT_GAP: f32 = 6.0;
    const SLIDER_ROW_TRACK_HEIGHT: f32 = 4.0;
    const SLIDER_ROW_TICK_HEIGHT: f32 = 12.0;
    /// The knob at rest; one pixel more while hovered or held.
    pub const SLIDER_ROW_KNOB_RADIUS: f32 = 7.0;
    // The knob is centred on the value, so at 0 and 1 it hangs past the track
    // by its radius (AGENTS.md trap); the gap to the label and the value has
    // to be wider than the hovered knob, or it touches them.
    const _: () = assert!(SLIDER_ROW_KNOB_RADIUS + 1.0 < SLIDER_ROW_GAP);
    /// How many consecutive widget indices one [`Widgets::slider_row`] takes from
    /// its namespace: track, label, value, route. A caller laying out rows strides
    /// its `first_index` by this.
    pub const SLIDER_ROW_IDS: u32 = 4;

    /// A segmented control's frame: 24 px items inside 2 px of padding and a 1 px
    /// edge (`.seg`).
    pub const SEGMENTED_HEIGHT: f32 = 30.0;
    /// The frame's edge plus its padding: where the items start.
    pub const SEGMENTED_INSET: f32 = 3.0;
    /// Horizontal padding inside one item, either side of its label.
    pub const SEGMENTED_ITEM_PADDING: f32 = 9.0;
    const SEGMENTED_RADIUS: f32 = 6.0;

    /// A status line: a dot and a micro word (`.dot`, the Assist cards' boundary,
    /// the AI settings' READY state).
    pub const STATUS_DOT_LABEL_HEIGHT: f32 = 16.0;
    pub const STATUS_DOT_SIZE: f32 = 6.0;
    const STATUS_DOT_GAP: f32 = 6.0;

    /// A colour swatch: a 24 px disc, ringed when selected (the caption's Ink and
    /// Plate rows).
    pub const SWATCH_SIZE: f32 = 24.0;

    /// The micro type's tracking (`.micro`'s `letter-spacing: 1px`).
    const MICRO_TRACKING: f32 = 1.0;

    /// One run of a micro head: muted by default, ink for the numbers inside it
    /// (`LYRICS · 83 CUES`, where 83 is ink). Drawn uppercase.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MicroRun<'a> {
        pub text: &'a str,
        pub ink: bool,
    }

    impl<'a> MicroRun<'a> {
        #[must_use]
        pub const fn muted(text: &'a str) -> Self {
            Self { text, ink: false }
        }

        #[must_use]
        pub const fn ink(text: &'a str) -> Self {
            Self { text, ink: true }
        }
    }

    /// Width of `runs` as [`draw_micro`] draws them.
    #[must_use]
    pub fn micro_width(font: &UiFonts, runs: &[MicroRun<'_>]) -> f32 {
        runs.iter()
            .map(|run| {
                measure_tracked(
                    font,
                    &run.text.to_uppercase(),
                    metric::UI_FONT_MICRO,
                    MICRO_TRACKING,
                )
            })
            .sum()
    }

    /// Draws micro runs left to right from `(x, y)`, their top edge, and returns
    /// the width drawn.
    pub fn draw_micro(
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        x: f32,
        y: f32,
        runs: &[MicroRun<'_>],
    ) -> f32 {
        let mut cursor = x;
        for run in runs {
            let text = run.text.to_uppercase();
            draw_text_tracked(
                d,
                font,
                &text,
                cursor,
                y,
                metric::UI_FONT_MICRO,
                MICRO_TRACKING,
                if run.ink {
                    color::ui_ink()
                } else {
                    color::ui_muted()
                },
            );
            cursor += measure_tracked(font, &text, metric::UI_FONT_MICRO, MICRO_TRACKING);
        }
        cursor - x
    }

    /// The y that centres one line of `size` in a box starting at `top` that is
    /// `height` tall, on the pixel grid.
    fn centred_text_y(top: f32, height: f32, size: f32) -> f32 {
        (top + (height - size) * 0.5).round()
    }

    /// Where a column head's parts go.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ColumnHead {
        /// The whole head, [`COLUMN_HEAD_HEIGHT`] tall; its last row is the hairline.
        pub head: UiRect,
        /// The micro title.
        pub title: UiRect,
        /// What is left of the head after the title: the view's switch or its
        /// actions go here, right-aligned with [`ColumnHead::trailing_right`].
        pub trailing: UiRect,
        /// The column below the head.
        pub body: UiRect,
    }

    impl ColumnHead {
        /// A box `width` wide and `height` tall at the right end of
        /// [`Self::trailing`], vertically centred in the head. Clipped to the
        /// trailing room, so a switch too wide for a narrow column shrinks rather
        /// than covering the title.
        #[must_use]
        pub fn trailing_right(&self, width: f32, height: f32) -> UiRect {
            let width = width.min(self.trailing.width).max(0.0);
            let height = height.min(self.head.height).max(0.0);
            UiRect::new(
                (self.trailing.x + self.trailing.width - width).round(),
                (self.head.y + (self.head.height - 1.0 - height) * 0.5).round(),
                width,
                height,
            )
        }
    }

    /// [`column_head`]'s geometry, for a title `title_width` wide.
    #[must_use]
    pub fn column_head_layout(column: UiRect, title_width: f32) -> ColumnHead {
        let height = COLUMN_HEAD_HEIGHT.min(column.height.max(0.0));
        let head = UiRect::new(column.x, column.y, column.width.max(0.0), height);
        let inner_left = column.x + COLUMN_HEAD_PADDING_LEFT;
        let inner_right = (column.x + column.width - COLUMN_HEAD_PADDING_RIGHT).max(inner_left);
        let title = UiRect::new(
            inner_left,
            column.y,
            title_width.max(0.0).min(inner_right - inner_left),
            height,
        );
        let trailing_x = (title.x + title.width + COLUMN_HEAD_GAP).min(inner_right);
        ColumnHead {
            head,
            title,
            trailing: UiRect::new(trailing_x, column.y, inner_right - trailing_x, height),
            body: UiRect::new(
                column.x,
                column.y + height,
                column.width.max(0.0),
                (column.height - height).max(0.0),
            ),
        }
    }

    /// Draws the head of a column view at the top of `column` and returns where
    /// everything went. The head is filled with the surface, so it reads the same
    /// whatever the body scrolled under it.
    pub fn column_head(
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        column: UiRect,
        title: &[MicroRun<'_>],
    ) -> ColumnHead {
        let layout = column_head_layout(column, micro_width(font, title));
        if layout.head.is_empty() {
            return layout;
        }
        fill(d, layout.head, color::ui_surface());
        let y = centred_text_y(
            layout.head.y,
            layout.head.height - 1.0,
            metric::UI_FONT_MICRO,
        );
        draw_micro(d, font, layout.title.x, y, title);
        fill(
            d,
            UiRect::new(
                layout.head.x,
                layout.head.y + layout.head.height - 1.0,
                layout.head.width,
                1.0,
            ),
            color::ui_rule(),
        );
        layout
    }

    /// Where a section head's parts go.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SectionHead {
        /// The whole row, [`SECTION_HEAD_HEIGHT`] tall.
        pub rect: UiRect,
        pub title: UiRect,
        /// The hairline after the title, one pixel tall; empty when the title
        /// fills the row.
        pub rule: UiRect,
    }

    /// [`section_head`]'s geometry. Only `row`'s x, y and width are read.
    #[must_use]
    pub fn section_head_layout(row: UiRect, title_width: f32) -> SectionHead {
        let rect = UiRect::new(row.x, row.y, row.width.max(0.0), SECTION_HEAD_HEIGHT);
        let title = UiRect::new(
            row.x,
            row.y,
            title_width.max(0.0).min(rect.width),
            SECTION_HEAD_HEIGHT,
        );
        let rule_x = title.x + title.width + SECTION_HEAD_GAP;
        let right = rect.x + rect.width;
        SectionHead {
            rect,
            title,
            rule: if rule_x < right {
                UiRect::new(
                    rule_x.round(),
                    (row.y + SECTION_HEAD_HEIGHT * 0.5).floor(),
                    right - rule_x.round(),
                    1.0,
                )
            } else {
                UiRect::new(right, row.y, 0.0, 0.0)
            },
        }
    }

    /// A section head inside a column body: `TYPE ———`. Takes `row`'s x, y and
    /// width; the height is [`SECTION_HEAD_HEIGHT`].
    pub fn section_head(
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        row: UiRect,
        title: &[MicroRun<'_>],
    ) -> SectionHead {
        let layout = section_head_layout(row, micro_width(font, title));
        if layout.rect.is_empty() {
            return layout;
        }
        let y = centred_text_y(layout.rect.y, layout.rect.height, metric::UI_FONT_MICRO);
        draw_micro(d, font, layout.title.x, y, title);
        fill(d, layout.rule, color::ui_rule());
        layout
    }

    /// How a segmented control spends the width it is given.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SegmentedFit {
        /// Each item as wide as its label plus padding (`.seg`): the head's
        /// switch. Falls back to [`SegmentedFit::Fill`] when that does not fit.
        Hug,
        /// Equal items across the whole width (`.seg.fill`): a row in a body,
        /// None / Shadow / Plate.
        Fill,
    }

    /// Where a segmented control's frame and items go.
    #[derive(Clone, Debug, PartialEq)]
    pub struct SegmentedLayout {
        pub frame: UiRect,
        pub items: Vec<UiRect>,
    }

    /// The width a [`SegmentedFit::Hug`] control needs for labels this wide.
    #[must_use]
    pub fn segmented_width(label_widths: &[f32]) -> f32 {
        label_widths
            .iter()
            .map(|width| width.max(0.0) + SEGMENTED_ITEM_PADDING * 2.0)
            .sum::<f32>()
            + SEGMENTED_INSET * 2.0
    }

    /// [`segmented_width`] for labels as [`Widgets::segmented`] draws them.
    #[must_use]
    pub fn segmented_intrinsic_width(font: &UiFonts, labels: &[&str]) -> f32 {
        let widths: Vec<f32> = labels
            .iter()
            .map(|label| measure(font, label, metric::UI_FONT_CAPTION))
            .collect();
        segmented_width(&widths)
    }

    /// [`Widgets::segmented`]'s geometry. The frame is [`SEGMENTED_HEIGHT`] tall,
    /// vertically centred in `boundary` and starting at its left edge. Item edges
    /// land on whole pixels, so neighbouring items share an edge rather than
    /// splitting one between them.
    #[must_use]
    pub fn segmented_layout(
        boundary: UiRect,
        label_widths: &[f32],
        fit: SegmentedFit,
    ) -> SegmentedLayout {
        let height = SEGMENTED_HEIGHT.min(boundary.height.max(0.0));
        let y = (boundary.y + (boundary.height - height) * 0.5).round();
        let available = boundary.width.max(0.0);
        let intrinsic = segmented_width(label_widths);
        let hug = fit == SegmentedFit::Hug && intrinsic <= available;
        let frame = UiRect::new(
            boundary.x,
            y,
            if hug { intrinsic } else { available },
            height,
        );
        let count = label_widths.len();
        let inner_width = (frame.width - SEGMENTED_INSET * 2.0).max(0.0);
        let item_height = (height - SEGMENTED_INSET * 2.0).max(0.0);
        let mut items = Vec::with_capacity(count);
        let mut left = frame.x + SEGMENTED_INSET;
        let mut consumed = 0.0f32;
        for (index, width) in label_widths.iter().enumerate() {
            consumed += if hug {
                width.max(0.0) + SEGMENTED_ITEM_PADDING * 2.0
            } else {
                inner_width / count as f32
            };
            let right = if index + 1 == count {
                frame.x + SEGMENTED_INSET + inner_width
            } else {
                (frame.x + SEGMENTED_INSET + consumed).round()
            };
            items.push(UiRect::new(
                left,
                y + SEGMENTED_INSET,
                (right - left).max(0.0),
                item_height,
            ));
            left = right;
        }
        SegmentedLayout { frame, items }
    }

    /// What a segmented control drew and which item took a press.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Segmented {
        pub layout: SegmentedLayout,
        /// The item released over this frame, if any.
        pub clicked: Option<usize>,
    }

    /// One parameter row's content.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SliderRow<'a> {
        pub label: &'a str,
        /// The value as the row prints it, already formatted by the caller.
        pub value_text: &'a str,
        /// The value's position on the track, `0..=1`.
        pub normalized: f32,
        /// Where the default sits on the track, drawn as a tick; `None` for none.
        pub default: Option<f32>,
        /// Away from its default: the label turns ink and gains a dot, and the
        /// value turns ink (replacing Tune's `*`).
        pub changed: bool,
        /// `None` draws no route button; `Some(routed)` draws `~`, filled when a
        /// route drives the value.
        pub route: Option<bool>,
        /// The label column's width, dot included (118 in Look, 58 in Lyrics'
        /// Style, from the mockup).
        pub label_width: f32,
    }

    /// Where a parameter row's parts go.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SliderRowLayout {
        /// The whole row, [`SLIDER_ROW_HEIGHT`] tall.
        pub row: UiRect,
        /// The label's press area, dot included.
        pub label: UiRect,
        /// The changed dot, drawn only when the row is changed.
        pub dot: UiRect,
        /// The track's press area: the full row height across the track, so the
        /// 4 px bar is not the only thing a pointer can hit.
        pub track: UiRect,
        /// The drawn bar, centred in [`Self::track`].
        pub bar: UiRect,
        pub value: UiRect,
        /// Empty when the row has no route button.
        pub route: UiRect,
    }

    /// [`Widgets::slider_row`]'s geometry. Only `row`'s x, y and width are read.
    #[must_use]
    pub fn slider_row_layout(row: UiRect, label_width: f32, route: bool) -> SliderRowLayout {
        let row = UiRect::new(row.x, row.y, row.width.max(0.0), SLIDER_ROW_HEIGHT);
        let right = row.x + row.width;
        let label = UiRect::new(
            row.x,
            row.y,
            label_width.max(0.0).min(row.width),
            row.height,
        );
        let dot = UiRect::new(
            row.x,
            (row.y + (row.height - SLIDER_ROW_DOT_SIZE) * 0.5).round(),
            SLIDER_ROW_DOT_SIZE,
            SLIDER_ROW_DOT_SIZE,
        );
        let route_rect = if route {
            UiRect::new(
                right - SLIDER_ROW_ROUTE_SIZE,
                (row.y + (row.height - SLIDER_ROW_ROUTE_SIZE) * 0.5).round(),
                SLIDER_ROW_ROUTE_SIZE,
                SLIDER_ROW_ROUTE_SIZE,
            )
        } else {
            UiRect::new(right, row.y, 0.0, 0.0)
        };
        let value_right = if route {
            route_rect.x - SLIDER_ROW_GAP
        } else {
            right
        };
        let value = UiRect::new(
            value_right - SLIDER_ROW_VALUE_WIDTH,
            row.y,
            SLIDER_ROW_VALUE_WIDTH,
            row.height,
        );
        let track_x = label.x + label.width + SLIDER_ROW_GAP;
        let track = UiRect::new(
            track_x,
            row.y,
            (value.x - SLIDER_ROW_GAP - track_x).max(0.0),
            row.height,
        );
        let bar = UiRect::new(
            track.x,
            (row.y + (row.height - SLIDER_ROW_TRACK_HEIGHT) * 0.5).round(),
            track.width,
            SLIDER_ROW_TRACK_HEIGHT,
        );
        SliderRowLayout {
            row,
            label,
            dot,
            track,
            bar,
            value,
            route: route_rect,
        }
    }

    /// What a parameter row drew and what the pointer did to it.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SliderRowResult {
        pub layout: SliderRowLayout,
        /// The value the pointer asks for while the track is dragged, as
        /// [`Widgets::slider`] returns it: an untouched row cannot write a value.
        pub drag: Option<f32>,
        /// The track's own state, so a caller can see a double press (reset).
        pub track: ButtonState,
        /// The label: a press resets the value in Look.
        pub label: ButtonState,
        /// The value: a press opens the typed entry.
        pub value: ButtonState,
        /// The `~` button; default when the row has none.
        pub route: ButtonState,
    }

    /// The colour a status dot wears. **No accent**: a status is never *now*
    /// (WX-5a), and the Direct tab's dot is told the same (§2).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum StatusTone {
        Success,
        Warning,
        Danger,
        Ink,
        Muted,
    }

    impl StatusTone {
        #[must_use]
        pub fn color(self) -> Color {
            match self {
                StatusTone::Success => color::ui_success(),
                StatusTone::Warning => color::ui_warning(),
                StatusTone::Danger => color::ui_danger(),
                StatusTone::Ink => color::ui_ink(),
                StatusTone::Muted => color::ui_muted(),
            }
        }
    }

    /// [`status_dot_label`]'s geometry: the dot and the text box for a word
    /// `text_width` wide, left-aligned and vertically centred in `boundary`.
    /// Returns `(whole, dot, text)`; the text is clipped to the boundary.
    #[must_use]
    pub fn status_dot_label_layout(boundary: UiRect, text_width: f32) -> (UiRect, UiRect, UiRect) {
        let height = STATUS_DOT_LABEL_HEIGHT.min(boundary.height.max(0.0));
        let y = (boundary.y + (boundary.height - height) * 0.5).round();
        let dot = UiRect::new(
            boundary.x,
            (y + (height - STATUS_DOT_SIZE) * 0.5).round(),
            STATUS_DOT_SIZE,
            STATUS_DOT_SIZE,
        );
        let text_x = boundary.x + STATUS_DOT_SIZE + STATUS_DOT_GAP;
        let right = boundary.x + boundary.width;
        let text = UiRect::new(
            text_x,
            y,
            text_width.max(0.0).min((right - text_x).max(0.0)),
            height,
        );
        let whole = UiRect::new(boundary.x, y, text.x + text.width - boundary.x, height);
        (whole, dot, text)
    }

    /// A dot in `tone` and a muted micro word: `● LOCAL AUDIO`. The word is
    /// ellipsized to the boundary. Returns the rect drawn, dot to last glyph.
    pub fn status_dot_label(
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        boundary: UiRect,
        tone: StatusTone,
        text: &str,
    ) -> UiRect {
        let upper = text.to_uppercase();
        let measure_micro =
            |run: &str| measure_tracked(font, run, metric::UI_FONT_MICRO, MICRO_TRACKING);
        let room = (boundary.width - STATUS_DOT_SIZE - STATUS_DOT_GAP).max(0.0);
        let (fitted, _) = row_typography::truncate_label(
            &upper,
            room,
            Some(measure_micro),
            row_typography::UI_ROW_LABEL_CAPACITY,
        );
        let (whole, dot, text_box) = status_dot_label_layout(boundary, measure_micro(&fitted));
        if whole.is_empty() {
            return whole;
        }
        d.draw_circle_v(
            Vector2::new(dot.x + dot.width * 0.5, dot.y + dot.height * 0.5),
            STATUS_DOT_SIZE * 0.5,
            tone.color(),
        );
        draw_text_tracked(
            d,
            font,
            &fitted,
            text_box.x,
            centred_text_y(text_box.y, text_box.height, metric::UI_FONT_MICRO),
            metric::UI_FONT_MICRO,
            MICRO_TRACKING,
            color::ui_muted(),
        );
        whole
    }

    /// The checker a translucent swatch is laid over, light then dark (the
    /// mockup's `#999`/`#666`).
    const SWATCH_CHECKER: [Color; 2] = [
        Color::new(0x99, 0x99, 0x99, 0xFF),
        Color::new(0x66, 0x66, 0x66, 0xFF),
    ];

    /// `top` composited over an opaque `base`, as one opaque colour.
    ///
    /// Computed here rather than blended by the GPU: `BLEND_ALPHA` on a chrome
    /// surface leaves the pixel translucent in a saved PNG (`docs/TRAPS.md`), and
    /// a swatch is exactly what a capture check would read.
    #[must_use]
    pub fn composite_over(top: Color, base: Color) -> Color {
        let a = u16::from(top.a);
        let mix = |over: u8, under: u8| -> u8 {
            let value = (u16::from(over) * a + u16::from(under) * (255 - a) + 127) / 255;
            u8::try_from(value).unwrap_or(u8::MAX)
        };
        Color::new(
            mix(top.r, base.r),
            mix(top.g, base.g),
            mix(top.b, base.b),
            255,
        )
    }

    /// What a swatch drew and whether it was pressed.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SwatchResult {
        /// The press area, [`SWATCH_SIZE`] square; the selection ring reaches
        /// 3 px past it.
        pub rect: UiRect,
        pub state: ButtonState,
    }

    impl Widgets {
        /// A segmented control: one selected item among a few
        /// (`Cues / Style / Effects`, `None / Shadow / Plate`). Item `i` claims
        /// `widget_id(namespace, first_index + i)`.
        ///
        /// The selected item wears the selection look — tint and outline, ink
        /// label — and the rest are muted words on the frame's raised fill, so the
        /// control reads as one object with one choice in it. `selected` is an
        /// index into `labels`; `None` selects nothing.
        #[allow(
            clippy::too_many_arguments,
            reason = "a button's arguments plus the label list and its fit; the id pair has to stay visible at the call site"
        )]
        pub fn segmented(
            &mut self,
            d: &mut RaylibDrawHandle<'_>,
            font: &UiFonts,
            namespace: u32,
            first_index: u32,
            boundary: UiRect,
            labels: &[&str],
            selected: Option<usize>,
            fit: SegmentedFit,
        ) -> Segmented {
            let widths: Vec<f32> = labels
                .iter()
                .map(|label| measure(font, label, metric::UI_FONT_CAPTION))
                .collect();
            let layout = segmented_layout(boundary, &widths, fit);
            let mut clicked = None;
            if layout.frame.is_empty() {
                return Segmented { layout, clicked };
            }
            rounded_box(
                d,
                layout.frame,
                SEGMENTED_RADIUS,
                color::ui_raised(),
                color::ui_control_edge(),
                1.0,
            );
            for (index, (item, label)) in layout.items.iter().zip(labels).enumerate() {
                let id = widget_id(namespace, first_index + index as u32);
                let state = self.button(d, id, *item);
                if state.clicked {
                    clicked = Some(index);
                }
                let on = selected == Some(index);
                if on {
                    control_box(
                        d,
                        *item,
                        color::ui_selected(),
                        color::ui_selected_edge(),
                        1.0,
                    );
                } else if state.hovered {
                    control_box(d, *item, color::ui_hover(), color::ui_hover(), 1.0);
                }
                text_label(
                    d,
                    font,
                    *item,
                    label,
                    metric::UI_FONT_CAPTION,
                    if on || state.hovered {
                        color::ui_ink()
                    } else {
                        color::ui_muted()
                    },
                );
            }
            Segmented { layout, clicked }
        }

        /// One parameter row: label (with a dot when changed), a track with a
        /// tick at the default, the value, and an optional `~` route button.
        /// Claims [`SLIDER_ROW_IDS`] consecutive indices from `first_index`:
        /// track, label, value, route.
        ///
        /// Like [`Self::slider`], the caller owns the value: the row returns what
        /// the pointer asks for and writes nothing.
        pub fn slider_row(
            &mut self,
            d: &mut RaylibDrawHandle<'_>,
            font: &UiFonts,
            namespace: u32,
            first_index: u32,
            row: UiRect,
            spec: &SliderRow<'_>,
        ) -> SliderRowResult {
            let layout = slider_row_layout(row, spec.label_width, spec.route.is_some());
            let mut result = SliderRowResult {
                layout,
                drag: None,
                track: ButtonState::default(),
                label: ButtonState::default(),
                value: ButtonState::default(),
                route: ButtonState::default(),
            };
            if layout.row.is_empty() {
                return result;
            }
            let size = metric::UI_FONT_CAPTION;
            let ink = color::ui_ink();
            let muted = color::ui_muted();

            // Label: the dot, then the words, ellipsized to the column.
            result.label = self.button(d, widget_id(namespace, first_index + 1), layout.label);
            if spec.changed {
                d.draw_circle_v(
                    Vector2::new(
                        layout.dot.x + layout.dot.width * 0.5,
                        layout.dot.y + layout.dot.height * 0.5,
                    ),
                    SLIDER_ROW_DOT_SIZE * 0.5,
                    ink,
                );
            }
            let words_x = layout.label.x + SLIDER_ROW_DOT_SIZE + SLIDER_ROW_DOT_GAP;
            let words_room = (layout.label.x + layout.label.width - words_x).max(0.0);
            let (words, _) = row_typography::truncate_label(
                spec.label,
                words_room,
                Some(|text: &str| measure(font, text, size)),
                row_typography::UI_ROW_LABEL_CAPACITY,
            );
            draw_text(
                d,
                font,
                &words,
                words_x,
                centred_text_y(layout.row.y, layout.row.height, size),
                size,
                if spec.changed || result.label.hovered {
                    ink
                } else {
                    muted
                },
            );

            // Track: the trough, the filled amount, the default tick, the knob.
            let pointer = self.pointer(d);
            let track_id = widget_id(namespace, first_index);
            result.track = self.button_at(track_id, layout.track, pointer);
            let filled = spec.normalized.clamp(0.0, 1.0);
            let bar = layout.bar;
            if !bar.is_empty() {
                let roundness = 1.0;
                d.draw_rectangle_rounded(rectangle(bar), roundness, 4, color::ui_selected());
                if filled > 0.0 {
                    d.draw_rectangle_rounded(
                        rectangle(UiRect::new(bar.x, bar.y, bar.width * filled, bar.height)),
                        roundness,
                        4,
                        color::ui_level(),
                    );
                }
                if let Some(default) = spec.default {
                    let x = (bar.x + bar.width * default.clamp(0.0, 1.0)).round();
                    fill(
                        d,
                        UiRect::new(
                            x,
                            (bar.y + (bar.height - SLIDER_ROW_TICK_HEIGHT) * 0.5).round(),
                            1.0,
                            SLIDER_ROW_TICK_HEIGHT,
                        ),
                        color::ui_selected_edge(),
                    );
                }
                d.draw_circle_v(
                    Vector2::new(bar.x + bar.width * filled, bar.y + bar.height * 0.5),
                    if result.track.hovered || result.track.pressed {
                        SLIDER_ROW_KNOB_RADIUS + 1.0
                    } else {
                        SLIDER_ROW_KNOB_RADIUS
                    },
                    ink,
                );
            }
            if self.active_button_id == track_id && pointer.down {
                self.interacted = true;
                result.drag = Some(slider_value(pointer.x, bar.x, bar.x + bar.width));
            }

            // Value: right-aligned tabular figures, a hover box saying it can be typed.
            result.value = self.button(d, widget_id(namespace, first_index + 2), layout.value);
            if result.value.hovered {
                control_box(d, layout.value, color::ui_hover(), color::ui_hover(), 1.0);
            }
            let value_width = measure_tabular(font, spec.value_text, size);
            draw_text_tabular(
                d,
                font,
                spec.value_text,
                (layout.value.x + layout.value.width - value_width).max(layout.value.x),
                centred_text_y(layout.row.y, layout.row.height, size),
                size,
                if spec.changed { ink } else { muted },
            );

            // Route: `~`, always visible, filled when a route drives the value.
            if let Some(routed) = spec.route {
                result.route = self.button(d, widget_id(namespace, first_index + 3), layout.route);
                let surface = if routed {
                    color::ui_selected()
                } else if result.route.hovered {
                    color::ui_hover()
                } else {
                    color::ui_surface()
                };
                control_box(d, layout.route, surface, color::ui_rule(), 1.0);
                text_label(
                    d,
                    font,
                    layout.route,
                    "~",
                    15.0,
                    if routed || result.route.hovered {
                        ink
                    } else {
                        muted
                    },
                );
            }
            result
        }

        /// A colour swatch: a [`SWATCH_SIZE`] disc with a hairline ring, and when
        /// selected a ring of ink 2 px out. `boundary`'s top-left places it; its
        /// size is the swatch's own. A translucent colour is shown over a checker,
        /// composited on the CPU so the pixels are opaque.
        pub fn swatch(
            &mut self,
            d: &mut RaylibDrawHandle<'_>,
            id: u64,
            boundary: UiRect,
            tint: Color,
            selected: bool,
        ) -> SwatchResult {
            let rect = UiRect::new(boundary.x, boundary.y, SWATCH_SIZE, SWATCH_SIZE);
            let state = self.button(d, id, rect);
            let centre = Vector2::new(rect.x + SWATCH_SIZE * 0.5, rect.y + SWATCH_SIZE * 0.5);
            let radius = SWATCH_SIZE * 0.5;
            if selected {
                d.draw_circle_v(centre, radius + 3.0, color::ui_ink());
                d.draw_circle_v(centre, radius + 2.0, color::ui_surface());
            }
            d.draw_circle_v(
                centre,
                radius + 1.0,
                if state.hovered {
                    color::ui_selected_edge()
                } else {
                    color::ui_control_edge()
                },
            );
            if tint.a == u8::MAX {
                d.draw_circle_v(centre, radius, tint);
            } else {
                // Four quadrants of checker, each with the colour laid over it.
                for (quadrant, start) in [0.0f32, 90.0, 180.0, 270.0].into_iter().enumerate() {
                    let base = SWATCH_CHECKER[quadrant % 2];
                    d.draw_circle_sector(
                        centre,
                        radius,
                        start,
                        start + 90.0,
                        8,
                        composite_over(tint, base),
                    );
                }
            }
            SwatchResult { rect, state }
        }
    }
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
///
/// # This is the only place a namespace may be minted
///
/// It was not, and that cost the export panel's resolution row (EX1). The manual
/// event row declared `EVENT_ROW_NAMESPACE: u32 = 7` as a bare literal in its own
/// file, which is [`EXPORT`]; the row draws first, so `+ Feel`, `+ Scene` and
/// `+ Custom` claimed the press for ids 0, 1 and 2 and cashed the release before
/// the export panel's 720p, 1080p and 1440p buttons — three of four — were even
/// drawn. 2160p is index 3, has no counterpart in the row, and worked, which is
/// why the symptom read as "some sizes", not "the panel is dead". Its preset
/// picker had picked `8`, which is [`SEEK`], the same way.
///
/// Every namespace in the application is listed below, including the ones whose
/// indices are named in a panel's own file, so [`tests`] can enumerate the whole
/// table. A panel-private constant is invisible to a collision test that
/// enumerates a hand-written list — and there were three such tests, each
/// checking a different stale subset, all green.
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
    /// The manual event row's three add buttons and its Clear
    /// (`panels::events`). Was `7` — [`EXPORT`] — until EX1.
    pub const EVENT_ROW: u32 = 10;
    /// The manual event row's preset picker (`panels::events`). Was `8` —
    /// [`SEEK`] — until EX1, so its first four controls were starved by the
    /// transport row's fine-seek group and scrub bar, which draw before it.
    pub const EVENT_PRESET: u32 = 11;
    /// The welcome screen's recent-project rows (UX0-C06): two controls per row,
    /// reopen at `index * 2` and forget at `index * 2 + 1`.
    ///
    /// Separate from [`WELCOME`] rather than more of its indices, for the reason
    /// [`SEEK`] is separate from [`TOOLBAR`]: the row count follows the window
    /// height and the stored list, so folding them in would renumber `Open audio`
    /// and `Open project` whenever the list grew.
    pub const WELCOME_RECENT: u32 = 12;
    /// The scene browser's ASCII image footer (D2): Import and Clear.
    ///
    /// Not [`SCENE_BROWSER`] indices, because those are `SceneId::ALL`'s own
    /// positions — an eleventh scene would silently inherit Import's id.
    pub const SCENE_ASCII: u32 = 13;
    /// Family navigation and its scrollbar; never aliases a scene tile.
    pub const SCENE_FAMILY: u32 = 66;
    /// One per merged event marker over the waveform lane, indexed by the
    /// marker's position in the merged view (D4). A namespace of its own rather
    /// than more [`TIMELINE`] indices, because that namespace's indices are the
    /// strip's own buttons and a marker count that changes with the project
    /// would renumber them.
    ///
    /// 14 rather than the 12 it was written against: two parallel agents each
    /// minted 12 in the same wave, and this table is where that collision
    /// surfaced — at merge, not at runtime, which is the table's whole job.
    pub const TIMELINE_EVENTS: u32 = 14;
    /// The lyrics panel's four groups (`panels::lyrics::ns`). Their indices are
    /// named there; the namespaces are allocated here.
    pub const LYRICS_ACTIONS: u32 = 16;
    pub const LYRICS_FORM: u32 = 17;
    pub const LYRICS_CAPTION: u32 = 18;
    /// One per lyric cue row, keyed by cue id so a row keeps its id across a
    /// scroll.
    pub const LYRICS_ROWS: u32 = 19;
    /// The top bar's mode tabs and its Still / Contact sheet / Render
    /// (`ui::top_bar`, WX-5c). Minted in the seam so the two 5c agents never
    /// pick a number each.
    pub const TOP_BAR: u32 = 20;
    /// The rail's icons and the flyouts' own chrome (WX-5c). The flyouts'
    /// contents keep the namespaces they had in the sidebar ([`TRACKS`],
    /// [`SCENE_BROWSER`], [`SCENE_FAMILY`], [`SCENE_ASCII`]), so no claim moves.
    pub const RAIL: u32 = 21;
    /// The right column's own chrome (WX-5d, `ui::column`): its head, its
    /// grip, the view switches it owns. The views' contents keep their
    /// panels' namespaces, so no claim moves when a panel moves into it.
    pub const COLUMN: u32 = 22;
    /// The font browser pane (`panels::fonts`). It minted `0x464F_4E54`
    /// ("FONT") in its own file, which no collision test could see — the
    /// exact trap this table exists for; moved here in the WX-5d seam.
    pub const FONTS: u32 = 23;
    /// One per notice card's close box, indexed by the notice's own id — the
    /// id rather than the list index, so eviction cannot misroute a claim.
    pub const NOTICE: u32 = 64;
    /// The collapsed tracks strip's Save button (review 1.12, UX0-A12).
    pub const TRACK_STRIP: u32 = 65;
    /// The Assist panel (`panels::assist`).
    pub const ASSIST: u32 = 74;
    /// The Assist settings dialog (`ui::assist_settings`), which is modal but
    /// gets a namespace of its own for the same reason [`WELCOME`] does.
    pub const ASSIST_DIALOG: u32 = 128;
    /// The full-window blocker the Assist settings modal draws into the
    /// **shell's** bank (`assist_settings::MODAL_BLOCK_NAMESPACE`), so nothing
    /// under the modal can claim a press. Minted in that file as a bare `129`
    /// until the WX-5d seam moved it here with [`FONTS`].
    pub const ASSIST_MODAL_BLOCK: u32 = 129;

    /// Every namespace above, for the collision test and for anyone adding one.
    ///
    /// Kept as an array rather than left to a hand-written list inside the test:
    /// the list *was* hand-written, in three different files, and every one of
    /// them was missing the namespace that actually collided.
    #[allow(
        dead_code,
        reason = "the allocation table itself; only the collision tests read it, and it is the thing that has to exist rather than the thing that has to be called"
    )]
    pub const ALL: [(&str, u32); 28] = [
        ("TOOLBAR", TOOLBAR),
        ("SCENE_BROWSER", SCENE_BROWSER),
        ("TRACKS", TRACKS),
        ("INSPECTOR", INSPECTOR),
        ("TIMELINE", TIMELINE),
        ("WELCOME", WELCOME),
        ("EXPORT", EXPORT),
        ("SEEK", SEEK),
        ("UTILITY", UTILITY),
        ("EVENT_ROW", EVENT_ROW),
        ("EVENT_PRESET", EVENT_PRESET),
        ("WELCOME_RECENT", WELCOME_RECENT),
        ("SCENE_ASCII", SCENE_ASCII),
        ("SCENE_FAMILY", SCENE_FAMILY),
        ("TIMELINE_EVENTS", TIMELINE_EVENTS),
        ("LYRICS_ACTIONS", LYRICS_ACTIONS),
        ("LYRICS_FORM", LYRICS_FORM),
        ("LYRICS_CAPTION", LYRICS_CAPTION),
        ("LYRICS_ROWS", LYRICS_ROWS),
        ("TOP_BAR", TOP_BAR),
        ("RAIL", RAIL),
        ("COLUMN", COLUMN),
        ("FONTS", FONTS),
        ("NOTICE", NOTICE),
        ("TRACK_STRIP", TRACK_STRIP),
        ("ASSIST", ASSIST),
        ("ASSIST_DIALOG", ASSIST_DIALOG),
        ("ASSIST_MODAL_BLOCK", ASSIST_MODAL_BLOCK),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolled_controls_cannot_claim_hidden_pixels_or_release_as_a_click() {
        let mut widgets = Widgets::new();
        let viewport = UiRect::new(0.0, 100.0, 200.0, 100.0);
        let row = UiRect::new(10.0, 80.0, 150.0, 50.0);
        let old = widgets.set_input_clip(Some(viewport));
        let pointer = Pointer {
            x: 30.0,
            y: 90.0,
            down: true,
            pressed: true,
            released: false,
        };
        let hidden = widgets.button_at(1, row, pointer);
        assert!(!hidden.hovered);
        assert_eq!(widgets.active_button_id, 0);
        // The same row's visible portion works, but releasing above the viewport cancels.
        widgets.button_at(
            1,
            row,
            Pointer {
                y: 110.0,
                ..pointer
            },
        );
        assert_eq!(widgets.active_button_id, 1);
        let released = widgets.button_at(
            1,
            row,
            Pointer {
                down: false,
                pressed: false,
                released: true,
                ..pointer
            },
        );
        assert!(!released.clicked);
        assert_eq!(widgets.active_button_id, 0);
        widgets.set_input_clip(old);
        // Leaving the body restores interaction with fixed header/footer controls.
        assert!(widgets.button_at(2, row, pointer).hovered);
        assert_eq!(widgets.active_button_id, 2);
    }

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

    /// Every namespace in the application, checked against every other one.
    ///
    /// The predecessor of this test enumerated six of them by hand and had done
    /// so since before `EXPORT` and `SEEK` existed. It was green throughout the
    /// life of EX1's defect, in which the manual event row's `7` aliased
    /// `EXPORT` and its `8` aliased `SEEK` — because neither of the colliding
    /// pairs was in the list. So the list is now [`id::ALL`], which lives beside
    /// the constants rather than in a test, and adding a namespace without
    /// adding it here fails the count assertion below rather than passing
    /// quietly.
    #[test]
    fn widget_ids_never_collide_across_namespaces_or_indices() {
        let mut seen = std::collections::HashSet::new();
        for (name, namespace) in id::ALL {
            for index in 0..4096u32 {
                assert!(
                    seen.insert(widget_id(namespace, index)),
                    "{name} ({namespace}) collides at index {index}"
                );
            }
        }
        // Zero is the "nothing is claimed" sentinel and must be unreachable.
        assert_ne!(widget_id(0, 0), 0);
    }

    /// `id::ALL` has to *be* the table, not a subset of it.
    ///
    /// A duplicated value would be caught above; a *missing* entry would not,
    /// and a missing entry is precisely the failure mode that let EX1 through.
    /// Counting the distinct values is the cheapest check that notices a
    /// namespace someone added to the module and forgot to list.
    #[test]
    fn every_allocated_namespace_is_listed_exactly_once() {
        let values: std::collections::HashSet<u32> =
            id::ALL.iter().map(|(_, value)| *value).collect();
        assert_eq!(
            values.len(),
            id::ALL.len(),
            "two namespaces share a value in id::ALL"
        );
        // Named individually so that adding a constant to `id` without adding
        // it to `ALL` breaks here, with the name in the message.
        for (name, value) in [
            ("TOOLBAR", id::TOOLBAR),
            ("SCENE_BROWSER", id::SCENE_BROWSER),
            ("TRACKS", id::TRACKS),
            ("INSPECTOR", id::INSPECTOR),
            ("TIMELINE", id::TIMELINE),
            ("WELCOME", id::WELCOME),
            ("EXPORT", id::EXPORT),
            ("SEEK", id::SEEK),
            ("UTILITY", id::UTILITY),
            ("EVENT_ROW", id::EVENT_ROW),
            ("EVENT_PRESET", id::EVENT_PRESET),
            ("WELCOME_RECENT", id::WELCOME_RECENT),
            ("SCENE_ASCII", id::SCENE_ASCII),
            ("SCENE_FAMILY", id::SCENE_FAMILY),
            ("TIMELINE_EVENTS", id::TIMELINE_EVENTS),
            ("LYRICS_ACTIONS", id::LYRICS_ACTIONS),
            ("LYRICS_FORM", id::LYRICS_FORM),
            ("LYRICS_CAPTION", id::LYRICS_CAPTION),
            ("LYRICS_ROWS", id::LYRICS_ROWS),
            ("TOP_BAR", id::TOP_BAR),
            ("RAIL", id::RAIL),
            ("COLUMN", id::COLUMN),
            ("FONTS", id::FONTS),
            ("NOTICE", id::NOTICE),
            ("TRACK_STRIP", id::TRACK_STRIP),
            ("ASSIST", id::ASSIST),
            ("ASSIST_DIALOG", id::ASSIST_DIALOG),
            ("ASSIST_MODAL_BLOCK", id::ASSIST_MODAL_BLOCK),
        ] {
            assert!(values.contains(&value), "{name} is missing from id::ALL");
        }
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

    // --- The column vocabulary (WX-5d). Numbers are the mockup's CSS; each
    // test pins them exactly, because a property ("inside the column") passes
    // a wrong formula.

    #[test]
    fn a_column_head_puts_its_title_inside_the_padding_and_its_switch_on_the_right() {
        let head = column_head_layout(UiRect::new(1460.0, 46.0, 460.0, 1034.0), 100.0);
        assert_eq!(head.head, UiRect::new(1460.0, 46.0, 460.0, 40.0));
        assert_eq!(head.title, UiRect::new(1474.0, 46.0, 100.0, 40.0));
        assert_eq!(head.trailing, UiRect::new(1582.0, 46.0, 328.0, 40.0));
        assert_eq!(head.body, UiRect::new(1460.0, 86.0, 460.0, 994.0));
        // Right-aligned and centred over the 39 px above the hairline.
        assert_eq!(
            head.trailing_right(120.0, SEGMENTED_HEIGHT),
            UiRect::new(1790.0, 51.0, 120.0, 30.0)
        );
        // Wider than the room: clipped to it, never over the title.
        assert_eq!(head.trailing_right(900.0, 30.0).x, head.trailing.x);
    }

    #[test]
    fn a_column_too_narrow_for_its_padding_lays_out_nothing_negative() {
        let head = column_head_layout(UiRect::new(0.0, 0.0, 20.0, 30.0), 100.0);
        assert_eq!(head.head.height, 30.0);
        assert_eq!(head.title.width, 0.0);
        assert_eq!(head.trailing.width, 0.0);
        assert_eq!(head.body, UiRect::new(0.0, 30.0, 20.0, 0.0));
    }

    #[test]
    fn a_section_head_draws_its_rule_from_after_the_title_to_the_edge() {
        let section = section_head_layout(UiRect::new(1474.0, 200.0, 432.0, 99.0), 40.0);
        assert_eq!(
            section.rect,
            UiRect::new(1474.0, 200.0, 432.0, SECTION_HEAD_HEIGHT)
        );
        assert_eq!(section.title, UiRect::new(1474.0, 200.0, 40.0, 18.0));
        assert_eq!(section.rule, UiRect::new(1522.0, 209.0, 384.0, 1.0));
        // A title that fills the row leaves no rule rather than a negative one.
        let full = section_head_layout(UiRect::new(0.0, 0.0, 30.0, 18.0), 40.0);
        assert!(full.rule.is_empty());
        assert_eq!(full.title.width, 30.0);
    }

    #[test]
    fn a_hugging_segmented_control_is_its_labels_plus_padding() {
        let widths = [30.0, 40.0, 50.0];
        assert_eq!(segmented_width(&widths), 180.0);
        let layout = segmented_layout(
            UiRect::new(100.0, 50.0, 400.0, 40.0),
            &widths,
            SegmentedFit::Hug,
        );
        assert_eq!(layout.frame, UiRect::new(100.0, 55.0, 180.0, 30.0));
        assert_eq!(
            layout.items,
            vec![
                UiRect::new(103.0, 58.0, 48.0, 24.0),
                UiRect::new(151.0, 58.0, 58.0, 24.0),
                UiRect::new(209.0, 58.0, 68.0, 24.0),
            ]
        );
    }

    #[test]
    fn a_filling_segmented_control_shares_whole_pixel_edges() {
        let layout = segmented_layout(
            UiRect::new(100.0, 50.0, 200.0, 40.0),
            &[10.0, 10.0, 10.0],
            SegmentedFit::Fill,
        );
        assert_eq!(layout.frame, UiRect::new(100.0, 55.0, 200.0, 30.0));
        assert_eq!(
            layout.items,
            vec![
                UiRect::new(103.0, 58.0, 65.0, 24.0),
                UiRect::new(168.0, 58.0, 64.0, 24.0),
                UiRect::new(232.0, 58.0, 65.0, 24.0),
            ]
        );
    }

    #[test]
    fn a_hugging_control_that_does_not_fit_fills_instead() {
        let widths = [30.0, 40.0, 50.0];
        let boundary = UiRect::new(0.0, 0.0, 150.0, 30.0);
        assert_eq!(
            segmented_layout(boundary, &widths, SegmentedFit::Hug),
            segmented_layout(boundary, &widths, SegmentedFit::Fill)
        );
        let layout = segmented_layout(boundary, &widths, SegmentedFit::Hug);
        let last = layout.items.last().unwrap();
        assert_eq!(last.x + last.width, 147.0);
    }

    #[test]
    fn a_parameter_row_places_label_track_value_and_route_by_the_mockup() {
        let row = slider_row_layout(UiRect::new(1474.0, 300.0, 432.0, 0.0), 118.0, true);
        assert_eq!(
            row.row,
            UiRect::new(1474.0, 300.0, 432.0, SLIDER_ROW_HEIGHT)
        );
        assert_eq!(row.label, UiRect::new(1474.0, 300.0, 118.0, 30.0));
        assert_eq!(row.dot, UiRect::new(1474.0, 313.0, 5.0, 5.0));
        assert_eq!(row.route, UiRect::new(1884.0, 304.0, 22.0, 22.0));
        assert_eq!(row.value, UiRect::new(1828.0, 300.0, 46.0, 30.0));
        assert_eq!(row.track, UiRect::new(1602.0, 300.0, 216.0, 30.0));
        assert_eq!(row.bar, UiRect::new(1602.0, 313.0, 216.0, 4.0));

        let bare = slider_row_layout(UiRect::new(1474.0, 300.0, 432.0, 0.0), 118.0, false);
        assert!(bare.route.is_empty());
        assert_eq!(bare.value, UiRect::new(1860.0, 300.0, 46.0, 30.0));
        assert_eq!(bare.track.width, 248.0);
    }

    #[test]
    fn a_status_line_is_a_dot_then_its_word_clipped_to_the_room() {
        let (whole, dot, text) =
            status_dot_label_layout(UiRect::new(10.0, 20.0, 200.0, 30.0), 60.0);
        assert_eq!(whole, UiRect::new(10.0, 27.0, 72.0, 16.0));
        assert_eq!(dot, UiRect::new(10.0, 32.0, 6.0, 6.0));
        assert_eq!(text, UiRect::new(22.0, 27.0, 60.0, 16.0));
        let (whole, _, text) = status_dot_label_layout(UiRect::new(10.0, 20.0, 40.0, 30.0), 60.0);
        assert_eq!(text.width, 28.0);
        assert_eq!(whole.width, 40.0);
    }

    #[test]
    fn a_translucent_swatch_is_composited_to_an_opaque_colour() {
        let base = Color::new(0, 0, 255, 255);
        assert_eq!(
            composite_over(Color::new(255, 0, 0, 128), base),
            Color::new(128, 0, 127, 255)
        );
        let opaque = Color::new(12, 34, 56, 255);
        assert_eq!(composite_over(opaque, base), opaque);
        assert_eq!(composite_over(Color::new(255, 255, 255, 0), base), base);
    }

    #[test]
    fn the_strong_button_is_ink_never_the_accent_and_ignores_selection() {
        let look = button_look(ButtonStyle::Strong, false, false);
        assert_eq!(look.fill, color::ui_ink());
        assert_eq!(look.ink, color::ui_surface());
        assert_ne!(look.fill, color::accent());
        assert_eq!(button_look(ButtonStyle::Strong, true, false), look);
        assert_ne!(
            button_look(ButtonStyle::Strong, false, true).fill,
            color::accent()
        );
    }
}
