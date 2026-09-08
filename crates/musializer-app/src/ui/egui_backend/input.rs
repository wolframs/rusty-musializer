//! Raylib input conversion with explicit keyboard ownership.

use raylib::prelude::*;

use egui::{
    Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchPhase, Vec2,
    ViewportIdMap, ViewportInfo,
};

const KEYBOARD_KEYS: [KeyboardKey; 110] = [
    KeyboardKey::KEY_NULL,
    KeyboardKey::KEY_APOSTROPHE,
    KeyboardKey::KEY_COMMA,
    KeyboardKey::KEY_MINUS,
    KeyboardKey::KEY_PERIOD,
    KeyboardKey::KEY_SLASH,
    KeyboardKey::KEY_ZERO,
    KeyboardKey::KEY_ONE,
    KeyboardKey::KEY_TWO,
    KeyboardKey::KEY_THREE,
    KeyboardKey::KEY_FOUR,
    KeyboardKey::KEY_FIVE,
    KeyboardKey::KEY_SIX,
    KeyboardKey::KEY_SEVEN,
    KeyboardKey::KEY_EIGHT,
    KeyboardKey::KEY_NINE,
    KeyboardKey::KEY_SEMICOLON,
    KeyboardKey::KEY_EQUAL,
    KeyboardKey::KEY_A,
    KeyboardKey::KEY_B,
    KeyboardKey::KEY_C,
    KeyboardKey::KEY_D,
    KeyboardKey::KEY_E,
    KeyboardKey::KEY_F,
    KeyboardKey::KEY_G,
    KeyboardKey::KEY_H,
    KeyboardKey::KEY_I,
    KeyboardKey::KEY_J,
    KeyboardKey::KEY_K,
    KeyboardKey::KEY_L,
    KeyboardKey::KEY_M,
    KeyboardKey::KEY_N,
    KeyboardKey::KEY_O,
    KeyboardKey::KEY_P,
    KeyboardKey::KEY_Q,
    KeyboardKey::KEY_R,
    KeyboardKey::KEY_S,
    KeyboardKey::KEY_T,
    KeyboardKey::KEY_U,
    KeyboardKey::KEY_V,
    KeyboardKey::KEY_W,
    KeyboardKey::KEY_X,
    KeyboardKey::KEY_Y,
    KeyboardKey::KEY_Z,
    KeyboardKey::KEY_LEFT_BRACKET,
    KeyboardKey::KEY_BACKSLASH,
    KeyboardKey::KEY_RIGHT_BRACKET,
    KeyboardKey::KEY_GRAVE,
    KeyboardKey::KEY_SPACE,
    KeyboardKey::KEY_ESCAPE,
    KeyboardKey::KEY_ENTER,
    KeyboardKey::KEY_TAB,
    KeyboardKey::KEY_BACKSPACE,
    KeyboardKey::KEY_INSERT,
    KeyboardKey::KEY_DELETE,
    KeyboardKey::KEY_RIGHT,
    KeyboardKey::KEY_LEFT,
    KeyboardKey::KEY_DOWN,
    KeyboardKey::KEY_UP,
    KeyboardKey::KEY_PAGE_UP,
    KeyboardKey::KEY_PAGE_DOWN,
    KeyboardKey::KEY_HOME,
    KeyboardKey::KEY_END,
    KeyboardKey::KEY_CAPS_LOCK,
    KeyboardKey::KEY_SCROLL_LOCK,
    KeyboardKey::KEY_NUM_LOCK,
    KeyboardKey::KEY_PRINT_SCREEN,
    KeyboardKey::KEY_PAUSE,
    KeyboardKey::KEY_F1,
    KeyboardKey::KEY_F2,
    KeyboardKey::KEY_F3,
    KeyboardKey::KEY_F4,
    KeyboardKey::KEY_F5,
    KeyboardKey::KEY_F6,
    KeyboardKey::KEY_F7,
    KeyboardKey::KEY_F8,
    KeyboardKey::KEY_F9,
    KeyboardKey::KEY_F10,
    KeyboardKey::KEY_F11,
    KeyboardKey::KEY_F12,
    KeyboardKey::KEY_LEFT_SHIFT,
    KeyboardKey::KEY_LEFT_CONTROL,
    KeyboardKey::KEY_LEFT_ALT,
    KeyboardKey::KEY_LEFT_SUPER,
    KeyboardKey::KEY_RIGHT_SHIFT,
    KeyboardKey::KEY_RIGHT_CONTROL,
    KeyboardKey::KEY_RIGHT_ALT,
    KeyboardKey::KEY_RIGHT_SUPER,
    KeyboardKey::KEY_KB_MENU,
    KeyboardKey::KEY_KP_0,
    KeyboardKey::KEY_KP_1,
    KeyboardKey::KEY_KP_2,
    KeyboardKey::KEY_KP_3,
    KeyboardKey::KEY_KP_4,
    KeyboardKey::KEY_KP_5,
    KeyboardKey::KEY_KP_6,
    KeyboardKey::KEY_KP_7,
    KeyboardKey::KEY_KP_8,
    KeyboardKey::KEY_KP_9,
    KeyboardKey::KEY_KP_DECIMAL,
    KeyboardKey::KEY_KP_DIVIDE,
    KeyboardKey::KEY_KP_MULTIPLY,
    KeyboardKey::KEY_KP_SUBTRACT,
    KeyboardKey::KEY_KP_ADD,
    KeyboardKey::KEY_KP_ENTER,
    KeyboardKey::KEY_KP_EQUAL,
    KeyboardKey::KEY_BACK,
    KeyboardKey::KEY_MENU,
    KeyboardKey::KEY_VOLUME_UP,
    KeyboardKey::KEY_VOLUME_DOWN,
];

const MOUSE_BUTTONS: [MouseButton; 7] = [
    MouseButton::MOUSE_BUTTON_LEFT,
    MouseButton::MOUSE_BUTTON_RIGHT,
    MouseButton::MOUSE_BUTTON_MIDDLE,
    MouseButton::MOUSE_BUTTON_SIDE,
    MouseButton::MOUSE_BUTTON_EXTRA,
    MouseButton::MOUSE_BUTTON_FORWARD,
    MouseButton::MOUSE_BUTTON_BACK,
];

fn map_key(key: KeyboardKey) -> Option<Key> {
    match key {
        KeyboardKey::KEY_NULL => None,
        KeyboardKey::KEY_APOSTROPHE => Some(Key::Quote),
        KeyboardKey::KEY_COMMA => Some(Key::Comma),
        KeyboardKey::KEY_MINUS => Some(Key::Minus),
        KeyboardKey::KEY_PERIOD => Some(Key::Period),
        KeyboardKey::KEY_SLASH => Some(Key::Slash),
        KeyboardKey::KEY_ZERO => Some(Key::Num0),
        KeyboardKey::KEY_ONE => Some(Key::Num1),
        KeyboardKey::KEY_TWO => Some(Key::Num2),
        KeyboardKey::KEY_THREE => Some(Key::Num3),
        KeyboardKey::KEY_FOUR => Some(Key::Num4),
        KeyboardKey::KEY_FIVE => Some(Key::Num5),
        KeyboardKey::KEY_SIX => Some(Key::Num6),
        KeyboardKey::KEY_SEVEN => Some(Key::Num7),
        KeyboardKey::KEY_EIGHT => Some(Key::Num8),
        KeyboardKey::KEY_NINE => Some(Key::Num9),
        KeyboardKey::KEY_SEMICOLON => Some(Key::Semicolon),
        KeyboardKey::KEY_EQUAL => Some(Key::Equals),
        KeyboardKey::KEY_A => Some(Key::A),
        KeyboardKey::KEY_B => Some(Key::B),
        KeyboardKey::KEY_C => Some(Key::C),
        KeyboardKey::KEY_D => Some(Key::D),
        KeyboardKey::KEY_E => Some(Key::E),
        KeyboardKey::KEY_F => Some(Key::F),
        KeyboardKey::KEY_G => Some(Key::G),
        KeyboardKey::KEY_H => Some(Key::H),
        KeyboardKey::KEY_I => Some(Key::I),
        KeyboardKey::KEY_J => Some(Key::J),
        KeyboardKey::KEY_K => Some(Key::K),
        KeyboardKey::KEY_L => Some(Key::L),
        KeyboardKey::KEY_M => Some(Key::M),
        KeyboardKey::KEY_N => Some(Key::N),
        KeyboardKey::KEY_O => Some(Key::O),
        KeyboardKey::KEY_P => Some(Key::P),
        KeyboardKey::KEY_Q => Some(Key::Q),
        KeyboardKey::KEY_R => Some(Key::R),
        KeyboardKey::KEY_S => Some(Key::S),
        KeyboardKey::KEY_T => Some(Key::T),
        KeyboardKey::KEY_U => Some(Key::U),
        KeyboardKey::KEY_V => Some(Key::V),
        KeyboardKey::KEY_W => Some(Key::W),
        KeyboardKey::KEY_X => Some(Key::X),
        KeyboardKey::KEY_Y => Some(Key::Y),
        KeyboardKey::KEY_Z => Some(Key::Z),
        KeyboardKey::KEY_LEFT_BRACKET => Some(Key::OpenBracket),
        KeyboardKey::KEY_BACKSLASH => Some(Key::Backslash),
        KeyboardKey::KEY_RIGHT_BRACKET => Some(Key::CloseBracket),
        KeyboardKey::KEY_GRAVE => Some(Key::Backtick),
        KeyboardKey::KEY_SPACE => Some(Key::Space),
        KeyboardKey::KEY_ESCAPE => Some(Key::Escape),
        KeyboardKey::KEY_ENTER => Some(Key::Enter),
        KeyboardKey::KEY_TAB => Some(Key::Tab),
        KeyboardKey::KEY_BACKSPACE => Some(Key::Backspace),
        KeyboardKey::KEY_INSERT => Some(Key::Insert),
        KeyboardKey::KEY_DELETE => Some(Key::Delete),
        KeyboardKey::KEY_RIGHT => Some(Key::ArrowRight),
        KeyboardKey::KEY_LEFT => Some(Key::ArrowLeft),
        KeyboardKey::KEY_DOWN => Some(Key::ArrowDown),
        KeyboardKey::KEY_UP => Some(Key::ArrowUp),
        KeyboardKey::KEY_PAGE_UP => Some(Key::PageUp),
        KeyboardKey::KEY_PAGE_DOWN => Some(Key::PageDown),
        KeyboardKey::KEY_HOME => Some(Key::Home),
        KeyboardKey::KEY_END => Some(Key::End),
        KeyboardKey::KEY_CAPS_LOCK => None,
        KeyboardKey::KEY_SCROLL_LOCK => None,
        KeyboardKey::KEY_NUM_LOCK => None,
        KeyboardKey::KEY_PRINT_SCREEN => None,
        KeyboardKey::KEY_PAUSE => None,
        KeyboardKey::KEY_F1 => Some(Key::F1),
        KeyboardKey::KEY_F2 => Some(Key::F2),
        KeyboardKey::KEY_F3 => Some(Key::F3),
        KeyboardKey::KEY_F4 => Some(Key::F4),
        KeyboardKey::KEY_F5 => Some(Key::F5),
        KeyboardKey::KEY_F6 => Some(Key::F6),
        KeyboardKey::KEY_F7 => Some(Key::F7),
        KeyboardKey::KEY_F8 => Some(Key::F8),
        KeyboardKey::KEY_F9 => Some(Key::F9),
        KeyboardKey::KEY_F10 => Some(Key::F10),
        KeyboardKey::KEY_F11 => Some(Key::F11),
        KeyboardKey::KEY_F12 => Some(Key::F12),
        KeyboardKey::KEY_LEFT_SHIFT => Some(Key::ShiftLeft),
        KeyboardKey::KEY_LEFT_CONTROL => Some(Key::ControlLeft),
        KeyboardKey::KEY_LEFT_ALT => Some(Key::AltLeft),
        KeyboardKey::KEY_LEFT_SUPER => Some(Key::SuperLeft),
        KeyboardKey::KEY_RIGHT_SHIFT => Some(Key::ShiftRight),
        KeyboardKey::KEY_RIGHT_CONTROL => Some(Key::ControlRight),
        KeyboardKey::KEY_RIGHT_ALT => Some(Key::AltRight),
        KeyboardKey::KEY_RIGHT_SUPER => Some(Key::SuperRight),
        KeyboardKey::KEY_KB_MENU => None,
        KeyboardKey::KEY_KP_0 => Some(Key::Num0),
        KeyboardKey::KEY_KP_1 => Some(Key::Num1),
        KeyboardKey::KEY_KP_2 => Some(Key::Num2),
        KeyboardKey::KEY_KP_3 => Some(Key::Num3),
        KeyboardKey::KEY_KP_4 => Some(Key::Num4),
        KeyboardKey::KEY_KP_5 => Some(Key::Num5),
        KeyboardKey::KEY_KP_6 => Some(Key::Num6),
        KeyboardKey::KEY_KP_7 => Some(Key::Num7),
        KeyboardKey::KEY_KP_8 => Some(Key::Num8),
        KeyboardKey::KEY_KP_9 => Some(Key::Num9),
        KeyboardKey::KEY_KP_DECIMAL => None,
        KeyboardKey::KEY_KP_DIVIDE => None,
        KeyboardKey::KEY_KP_MULTIPLY => None,
        KeyboardKey::KEY_KP_SUBTRACT => None,
        KeyboardKey::KEY_KP_ADD => None,
        KeyboardKey::KEY_KP_ENTER => None,
        KeyboardKey::KEY_KP_EQUAL => None,
        KeyboardKey::KEY_BACK => None,
        KeyboardKey::KEY_MENU => None,
        KeyboardKey::KEY_VOLUME_UP => None,
        KeyboardKey::KEY_VOLUME_DOWN => None,
    }
}

fn map_mouse_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::MOUSE_BUTTON_LEFT => Some(PointerButton::Primary),
        MouseButton::MOUSE_BUTTON_RIGHT => Some(PointerButton::Secondary),
        MouseButton::MOUSE_BUTTON_MIDDLE => Some(PointerButton::Middle),
        MouseButton::MOUSE_BUTTON_SIDE => Some(PointerButton::Extra1),
        MouseButton::MOUSE_BUTTON_EXTRA => Some(PointerButton::Extra2),
        MouseButton::MOUSE_BUTTON_FORWARD => Some(PointerButton::Extra1),
        MouseButton::MOUSE_BUTTON_BACK => Some(PointerButton::Extra2),
    }
}

fn add_event_keys(events: &mut Vec<Event>, rl: &RaylibHandle, modifiers: Modifiers) {
    for key_candidate in KEYBOARD_KEYS {
        if let Some(key) = map_key(key_candidate) {
            if rl.is_key_pressed(key_candidate) {
                events.push(Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });

                if modifiers.ctrl {
                    match key {
                        Key::C => events.push(Event::Copy),
                        Key::V => {
                            if let Ok(text) = rl.get_clipboard_text() {
                                events.push(Event::Paste(text))
                            }
                        }
                        Key::X => events.push(Event::Cut),
                        _ => (),
                    }
                }
            } else if rl.is_key_pressed_repeat(key_candidate) {
                events.push(Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: true,
                    modifiers,
                });
            } else if rl.is_key_released(key_candidate) {
                events.push(Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                });
            }
        }
    }
}

fn add_event_text(events: &mut Vec<Event>, rl: &mut RaylibHandle) {
    let mut text = String::new();
    while let Some(character) = rl.get_char_pressed() {
        text.push(character);
    }

    if !text.is_empty() {
        events.push(Event::Text(text));
    }
}

fn add_event_pointer(
    events: &mut Vec<Event>,
    rl: &RaylibHandle,
    modifiers: Modifiers,
    pixels_per_point: f32,
) {
    let mouse_vector = rl.get_mouse_position();
    let mouse_pos = Pos2::new(mouse_vector.x, mouse_vector.y) / pixels_per_point;

    events.push(Event::PointerMoved(mouse_pos));

    for button_candidate in MOUSE_BUTTONS {
        if let Some(button) = map_mouse_button(button_candidate) {
            if rl.is_mouse_button_pressed(button_candidate) {
                events.push(Event::PointerButton {
                    pos: mouse_pos,
                    button,
                    pressed: true,
                    modifiers,
                });
            } else if rl.is_mouse_button_released(button_candidate) {
                events.push(Event::PointerButton {
                    pos: mouse_pos,
                    button,
                    pressed: false,
                    modifiers,
                });
            }
        }
    }

    let scroll_rl_vector = rl.get_mouse_wheel_move_v();
    let scroll_vector = Vec2::new(scroll_rl_vector.x, scroll_rl_vector.y);

    if scroll_vector != Vec2::ZERO {
        events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: scroll_vector,
            phase: TouchPhase::Move,
            modifiers,
        });
    }
}

pub(crate) fn gather_input(
    rl: &mut RaylibHandle,
    pixels_per_point: f32,
    keyboard_enabled: bool,
) -> RawInput {
    let mut viewports: ViewportIdMap<ViewportInfo> = ViewportIdMap::default();
    // Coordinates use raylib logical window units; its scissor/projection handles framebuffer DPI.
    let native_pixels_per_point = pixels_per_point;

    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(native_pixels_per_point),
            ..Default::default()
        },
    );

    let screen_size = Vec2::new(
        rl.get_screen_width() as f32 / pixels_per_point,
        rl.get_screen_height() as f32 / pixels_per_point,
    );

    let modifiers = Modifiers {
        alt: rl.is_key_down(KeyboardKey::KEY_LEFT_ALT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT),
        ctrl: rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL),
        shift: rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT),
        mac_cmd: false,
        command: rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL),
    };

    let mut events = vec![Event::WindowFocused(rl.is_window_focused())];
    if keyboard_enabled && rl.is_window_focused() {
        add_event_keys(&mut events, rl, modifiers);
        if !modifiers.command && !modifiers.alt {
            add_event_text(&mut events, rl);
        }
    }
    if rl.is_window_focused() {
        add_event_pointer(&mut events, rl, modifiers, pixels_per_point);
    } else {
        events.push(Event::PointerGone);
    }

    RawInput {
        viewport_id: egui::ViewportId::ROOT,
        viewports,
        safe_area_insets: Default::default(),
        screen_rect: Some(Rect::from_min_size(Default::default(), screen_size)),
        max_texture_side: Default::default(),
        time: Some(rl.get_time()),
        predicted_dt: Default::default(),
        modifiers,
        events,
        hovered_files: Default::default(),
        dropped_files: Default::default(),
        focused: rl.is_window_focused(),
        system_theme: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_edit_navigation_and_clipboard_keys_are_mapped() {
        for (raylib, egui) in [
            (KeyboardKey::KEY_BACKSPACE, Key::Backspace),
            (KeyboardKey::KEY_DELETE, Key::Delete),
            (KeyboardKey::KEY_LEFT, Key::ArrowLeft),
            (KeyboardKey::KEY_RIGHT, Key::ArrowRight),
            (KeyboardKey::KEY_HOME, Key::Home),
            (KeyboardKey::KEY_END, Key::End),
            (KeyboardKey::KEY_TAB, Key::Tab),
            (KeyboardKey::KEY_ENTER, Key::Enter),
            (KeyboardKey::KEY_C, Key::C),
            (KeyboardKey::KEY_V, Key::V),
            (KeyboardKey::KEY_X, Key::X),
        ] {
            assert_eq!(map_key(raylib), Some(egui));
        }
    }
}
