//! The single-line text field: pixels, keyboard, and the caret's blink.
//!
//! **Owner: Agent I.** The policy half — caret movement, selection, what a
//! keystroke does to the buffer — is [`musializer_core::ui::text_edit`], so this
//! file holds only drawing and raylib input. That split is what makes the
//! editing rules assertable without a window.
//!
//! Scaffolded by the integration owner because `ui/mod.rs` is shared. This lives
//! beside `widgets` rather than inside it: a text field is the first widget here
//! with focus, a caret and a repeat timer, and folding that state into
//! [`super::widgets::Widgets`] — which every other widget shares — would give
//! every button a field's worth of state it does not want.
//!
//! # What is the oracle's here, and what is not
//!
//! The frozen C's field is append-only: characters land at the end, backspace
//! takes the last one, and the drawn caret is a rule at the full string width
//! (`lyrics_editor_ui.c:1380-1385`), so it can only ever be at the end. Three
//! behaviours *are* reproduced exactly because a user or the model can see them:
//!
//! - **Focus follows the press**, set on every left press to whether the pointer
//!   was inside the box, so clicking anywhere else defocuses
//!   (`lyrics_editor_ui.c:1366-1368`).
//! - **A chord must not also type its letter**, and characters are drained
//!   either way so a chord cannot leave one queued for the next frame
//!   (`:204-207`).
//! - **Escape defocuses** rather than reverting (`:182`).
//!
//! Everything else — a caret you can put somewhere, a selection, cut and copy,
//! word motion, and the horizontal scroll that keeps the caret on screen — is
//! invention, recorded in `REWRITE_PLAN.md`'s `#### Agent I` note.

use musializer_core::ui::text_edit::{Motion, Select, TextEdit, TextEditError, TextRules};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::AuthoredText;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle, Vector2};

use super::theme::color;
use super::widgets;

/// Horizontal inset of the text from the box's edges, and the width the caret
/// keeps clear of them when it scrolls into view.
const TEXT_INSET: f32 = 8.0;

/// What one frame of a field did, for the caller to turn into notices.
///
/// A struct rather than direct notice pushes: the notice queue belongs to the
/// shell, and a widget that reached into it could not be drawn anywhere else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextFieldEvent {
    /// The buffer changed this frame.
    pub changed: bool,
    /// A paste flattened line breaks or tabs into spaces. The C says so with an
    /// info notice, and so does the caller (`lyrics_editor_ui.c:197-201`).
    pub flattened: bool,
    /// A paste was refused and the buffer was left exactly as it was.
    pub rejected: Option<TextEditError>,
    /// The clipboard could not be read at all — no owner, or not UTF-8.
    pub clipboard_unavailable: bool,
}

/// A single-line text field.
#[derive(Clone, Debug)]
pub struct TextField {
    /// The buffer and its caret. Public because the panel binds it to a cue and
    /// reads it back on Apply; every *mutation* still goes through the policy.
    pub edit: TextEdit,
    focused: bool,
    /// A press that landed inside the box is dragging out a selection. Tracked
    /// here rather than through the widget claim for the same reason the timeline
    /// scrubber does it: a drag that leaves the box must keep selecting.
    dragging: bool,
    /// Pixels of the text scrolled off the left edge.
    scroll: f32,
    /// When the caret was last moved or the buffer last changed. The blink is
    /// measured from here so the caret is solid while you are typing rather than
    /// disappearing mid-word.
    blink_epoch: f64,
}

impl TextField {
    #[must_use]
    pub fn new(rules: TextRules) -> Self {
        Self {
            edit: TextEdit::new(rules),
            focused: false,
            dragging: false,
            scroll: 0.0,
            blink_epoch: 0.0,
        }
    }

    #[must_use]
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Focuses or defocuses without a click. Used when a new cue is begun, which
    /// the C also does (`lyric_editor_ui_begin_new` sets `text_active = true`).
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.dragging = false;
        }
    }

    /// Binds the field to a string that came out of the model.
    ///
    /// Refused text clears the field rather than being shown: the only way to get
    /// text here that the field would not accept is a bug upstream, and drawing
    /// it would let Apply write back something the caller never saw.
    pub fn bind(&mut self, text: &str) {
        if self.edit.set_text(text).is_err() {
            self.edit.clear();
        }
        self.scroll = 0.0;
        self.dragging = false;
    }

    /// Draws the field and applies this frame's input to it.
    ///
    /// Returns what happened, for the caller's notices. The field claims no
    /// widget id: it is not a button, and taking the shared `active_button_id`
    /// for the length of a selection drag would freeze every other control the
    /// way a stranded claim does (`ui_widgets.c`, and AGENTS.md's note on it).
    ///
    /// The face is the caller's choice — the lyric editor passes the caption
    /// atlas so a user's own words are legible while typed (review 1.5); a
    /// chrome caller wraps its bank with [`AuthoredText::from_ui`]. Every
    /// metric below — hit test, caret x, selection rect, scroll — derives from
    /// the one `measure` closure, so face and measurement cannot drift apart.
    #[allow(
        clippy::too_many_arguments,
        reason = "the widget shape used everywhere in this shell: target, face, box, size, placeholder"
    )]
    pub fn draw_with_face(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: AuthoredText<'_>,
        boundary: UiRect,
        font_size: f32,
        placeholder: &str,
        enabled: bool,
    ) -> TextFieldEvent {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let mut event = TextFieldEvent::default();
        if boundary.is_empty() {
            return event;
        }
        let inner = UiRect::new(
            boundary.x + TEXT_INSET,
            boundary.y,
            (boundary.width - TEXT_INSET * 2.0).max(0.0),
            boundary.height,
        );
        let measure = |text: &str| font.measure_text(text, font_size, 0.0).x;

        if !enabled {
            self.focused = false;
            self.dragging = false;
        } else {
            let scale = super::scale::UiScale::new(font.scale()).unwrap_or_default();
            let mouse = scale.mouse(d);
            let inside = boundary.contains_point(mouse.x, mouse.y);
            // The C sets focus from every press, not only from one inside the
            // box, which is what makes clicking elsewhere defocus
            // (`lyrics_editor_ui.c:1366-1368`).
            if d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) {
                self.focused = inside;
                if inside {
                    let offset = self
                        .edit
                        .offset_at_x(mouse.x - inner.x + self.scroll, &measure);
                    // A second press inside an existing selection is a
                    // double-click for our purposes: raylib has no double-click
                    // signal, and a word select on the second press of a pair is
                    // close enough to be useful and cheap enough to be honest.
                    if self.edit.has_selection() && self.selection_holds(offset) {
                        self.edit.select_word_at(offset);
                    } else {
                        let select = if d.is_key_down(raylib::consts::KeyboardKey::KEY_LEFT_SHIFT)
                            || d.is_key_down(raylib::consts::KeyboardKey::KEY_RIGHT_SHIFT)
                        {
                            Select::Extend
                        } else {
                            Select::Collapse
                        };
                        self.edit.set_caret(offset, select);
                    }
                    self.dragging = true;
                    self.blink_epoch = d.get_time();
                }
            }
            if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
                self.dragging = false;
            }
            if self.dragging {
                let offset = self
                    .edit
                    .offset_at_x(mouse.x - inner.x + self.scroll, &measure);
                self.edit.set_caret(offset, Select::Extend);
            }
            if self.focused {
                event = self.keyboard(d);
                if event.changed {
                    self.blink_epoch = d.get_time();
                }
            }
        }

        // Scroll last, so the caret the user just moved is the one kept on
        // screen.
        let caret_x = self.edit.x_at_offset(self.edit.caret(), &measure);
        let width = measure(self.edit.text());
        if inner.width > 0.0 {
            if caret_x - self.scroll > inner.width - 2.0 {
                self.scroll = caret_x - inner.width + 2.0;
            }
            if caret_x - self.scroll < 0.0 {
                self.scroll = caret_x;
            }
            // Never leave a gap at the right when the text would fill it.
            self.scroll = self.scroll.min((width - inner.width).max(0.0)).max(0.0);
        }

        widgets::fill(d, boundary, color::ui_raised());
        d.draw_rectangle_lines_ex(
            widgets::rectangle(boundary),
            if self.focused { 2.0 } else { 1.0 },
            if self.focused {
                color::accent()
            } else {
                color::ui_rule()
            },
        );

        let text_y = boundary.y + (boundary.height - font_size) * 0.5;
        if inner.width <= 0.0 || inner.height <= 0.0 {
            return event;
        }
        let scale = super::scale::UiScale::new(font.scale()).unwrap_or_default();
        let mut clip = widgets::begin_scissor(d, inner, scale);
        if self.edit.is_empty() && !placeholder.is_empty() {
            font.draw_text(
                &mut clip,
                placeholder,
                Vector2::new(inner.x, text_y),
                font_size,
                0.0,
                color::ui_muted(),
            );
        } else {
            // The selection is painted behind the glyphs rather than over them,
            // so the text stays the ink colour and stays legible — an accent
            // block over dark text is the version that cannot be read.
            if self.edit.has_selection() {
                let (start, end) = self.edit.selection();
                let left = self.edit.x_at_offset(start, &measure) - self.scroll;
                let right = self.edit.x_at_offset(end, &measure) - self.scroll;
                widgets::fill(
                    &mut clip,
                    UiRect::new(
                        inner.x + left,
                        boundary.y + 4.0,
                        (right - left).max(1.0),
                        boundary.height - 8.0,
                    ),
                    // A pale wash of the accent, the same tone the cue list uses
                    // for its selected row.
                    raylib::prelude::Color::new(231, 236, 250, 255),
                );
            }
            font.draw_text(
                &mut clip,
                self.edit.text(),
                Vector2::new(inner.x - self.scroll, text_y),
                font_size,
                0.0,
                color::ui_ink(),
            );
        }
        // The C's blink, `((int)(GetTime()*2.0) & 1) == 0` (`:1380`), measured
        // from the last edit so the caret does not vanish mid-keystroke.
        let phase = (clip.get_time() - self.blink_epoch) * 2.0;
        if self.focused && (phase as i64) % 2 == 0 {
            let x = inner.x + caret_x - self.scroll;
            clip.draw_line_ex(
                Vector2::new(x, boundary.y + 5.0),
                Vector2::new(x, boundary.y + boundary.height - 5.0),
                1.0,
                color::accent(),
            );
        }
        drop(clip);
        event
    }

    fn selection_holds(&self, offset: usize) -> bool {
        let (start, end) = self.edit.selection();
        offset >= start && offset <= end
    }

    /// This frame's keys. Only reached while the field has focus.
    fn keyboard(&mut self, d: &mut RaylibDrawHandle<'_>) -> TextFieldEvent {
        use raylib::consts::KeyboardKey as Key;

        let mut event = TextFieldEvent::default();
        let control = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
        let shift = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
        let select = if shift {
            Select::Extend
        } else {
            Select::Collapse
        };
        // Held keys repeat: a field where holding backspace deletes exactly one
        // character is a field nobody can use. raylib reports the repeat edge
        // separately from the first press.
        let pressed = |d: &RaylibDrawHandle<'_>, key: Key| {
            d.is_key_pressed(key) || d.is_key_pressed_repeat(key)
        };

        if d.is_key_pressed(Key::KEY_ESCAPE) {
            self.focused = false;
            self.dragging = false;
            return event;
        }

        if control {
            if d.is_key_pressed(Key::KEY_A) {
                self.edit.select_all();
            }
            if d.is_key_pressed(Key::KEY_C) && self.edit.has_selection() {
                let selected = self.edit.selected_text().to_string();
                let _ = d.set_clipboard_text(&selected);
            }
            if d.is_key_pressed(Key::KEY_X) {
                if let Some(taken) = self.edit.cut() {
                    let _ = d.set_clipboard_text(&taken);
                    event.changed = true;
                }
            }
            if d.is_key_pressed(Key::KEY_V) {
                match d.get_clipboard_text() {
                    Ok(clipboard) => match self.edit.insert(&clipboard) {
                        Ok(flattened) => {
                            event.changed = true;
                            event.flattened = flattened;
                        }
                        Err(error) => event.rejected = Some(error),
                    },
                    // tinyfd's `GetClipboardText` returns "" rather than failing,
                    // so the C's paste of an empty clipboard reports
                    // LYRICS_ERROR_NULL. Here an unreadable clipboard is its own
                    // outcome, because "there is nothing on the clipboard" and
                    // "the clipboard is not UTF-8" deserve different sentences.
                    Err(_) => event.clipboard_unavailable = true,
                }
            }
        }

        if pressed(d, Key::KEY_BACKSPACE) && self.edit.backspace() {
            event.changed = true;
        }
        if pressed(d, Key::KEY_DELETE) && self.edit.delete() {
            event.changed = true;
        }

        let motion = if control {
            (Motion::WordLeft, Motion::WordRight)
        } else {
            (Motion::Left, Motion::Right)
        };
        if pressed(d, Key::KEY_LEFT) {
            self.edit.move_caret(motion.0, select);
            self.blink_epoch = d.get_time();
        }
        if pressed(d, Key::KEY_RIGHT) {
            self.edit.move_caret(motion.1, select);
            self.blink_epoch = d.get_time();
        }
        if pressed(d, Key::KEY_HOME) {
            self.edit.move_caret(Motion::Home, select);
            self.blink_epoch = d.get_time();
        }
        if pressed(d, Key::KEY_END) {
            self.edit.move_caret(Motion::End, select);
            self.blink_epoch = d.get_time();
        }

        // Characters are drained whether or not they are used, so a chord cannot
        // leave one queued for the next frame (`lyrics_editor_ui.c:204-207`).
        while let Some(codepoint) = d.get_char_pressed() {
            if control {
                continue;
            }
            if self.edit.insert_char(codepoint).is_ok() {
                event.changed = true;
            }
        }
        event
    }
}
