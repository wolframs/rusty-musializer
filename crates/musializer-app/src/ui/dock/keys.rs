//! What a key press sends to a terminal program: the xterm encoding that
//! shells, `less`, `vim` and the agent TUIs (Claude Code, Codex, Antigravity)
//! all expect.
//!
//! Printable text arrives separately, as egui `Text` events, which carry the
//! keyboard layout's own answer (`@` from AltGr+Q on a German keyboard). This
//! module handles only what text cannot carry: control characters, cursor and
//! editing keys, function keys, and the Ctrl/Alt chords a terminal turns into
//! bytes. A key that also produces text returns `None` here when no chord is
//! held, so a press is never sent twice.

use egui::{Key, Modifiers};

/// The xterm modifier parameter: 1 + Shift + 2·Alt + 4·Ctrl.
fn modifier_code(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl)
}

fn chorded(modifiers: Modifiers) -> bool {
    modifiers.shift || modifiers.alt || modifiers.ctrl
}

/// `CSI <final>` for cursor keys, or `SS3 <final>` in application cursor
/// mode, or `CSI 1 ; <m> <final>` with modifiers held.
fn cursor(final_byte: u8, modifiers: Modifiers, app_cursor: bool) -> Vec<u8> {
    if chorded(modifiers) {
        format!("\x1b[1;{}{}", modifier_code(modifiers), final_byte as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// `CSI <n> ~`, or `CSI <n> ; <m> ~` with modifiers held.
fn tilde(number: u8, modifiers: Modifiers) -> Vec<u8> {
    if chorded(modifiers) {
        format!("\x1b[{number};{}~", modifier_code(modifiers)).into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

/// F1–F4 are `SS3 P..S` bare and `CSI 1 ; <m> P..S` chorded.
fn ss3_function(final_byte: u8, modifiers: Modifiers) -> Vec<u8> {
    if chorded(modifiers) {
        format!("\x1b[1;{}{}", modifier_code(modifiers), final_byte as char).into_bytes()
    } else {
        vec![0x1b, b'O', final_byte]
    }
}

fn letter(key: Key) -> Option<u8> {
    let name = key.name();
    let bytes = name.as_bytes();
    (bytes.len() == 1 && bytes[0].is_ascii_uppercase()).then(|| bytes[0])
}

/// The bytes for one key press, or `None` when the press is plain text (sent
/// by its `Text` event) or means nothing to a terminal.
#[must_use]
pub fn key_bytes(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let alt_prefix = |mut bytes: Vec<u8>| {
        if modifiers.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let bytes = match key {
        Key::Enter => {
            // Shift+Enter is ESC CR, the Meta-Enter that Claude Code and
            // Codex read as "new line, do not submit".
            if modifiers.shift {
                vec![0x1b, b'\r']
            } else {
                alt_prefix(vec![b'\r'])
            }
        }
        Key::Backspace => {
            if modifiers.ctrl {
                alt_prefix(vec![0x08])
            } else {
                alt_prefix(vec![0x7f])
            }
        }
        Key::Tab => {
            if modifiers.shift {
                b"\x1b[Z".to_vec()
            } else {
                alt_prefix(vec![b'\t'])
            }
        }
        Key::Escape => vec![0x1b],
        Key::ArrowUp => cursor(b'A', modifiers, app_cursor),
        Key::ArrowDown => cursor(b'B', modifiers, app_cursor),
        Key::ArrowRight => cursor(b'C', modifiers, app_cursor),
        Key::ArrowLeft => cursor(b'D', modifiers, app_cursor),
        Key::Home => cursor(b'H', modifiers, app_cursor),
        Key::End => cursor(b'F', modifiers, app_cursor),
        Key::Insert => tilde(2, modifiers),
        Key::Delete => tilde(3, modifiers),
        Key::PageUp => tilde(5, modifiers),
        Key::PageDown => tilde(6, modifiers),
        Key::F1 => ss3_function(b'P', modifiers),
        Key::F2 => ss3_function(b'Q', modifiers),
        Key::F3 => ss3_function(b'R', modifiers),
        Key::F4 => ss3_function(b'S', modifiers),
        Key::F5 => tilde(15, modifiers),
        Key::F6 => tilde(17, modifiers),
        Key::F7 => tilde(18, modifiers),
        Key::F8 => tilde(19, modifiers),
        Key::F9 => tilde(20, modifiers),
        Key::F10 => tilde(21, modifiers),
        Key::F11 => tilde(23, modifiers),
        Key::F12 => tilde(24, modifiers),
        _ if modifiers.ctrl => {
            let control = match key {
                Key::Space | Key::Num2 => 0x00,
                Key::OpenBracket | Key::Num3 => 0x1b,
                Key::Backslash | Key::Num4 => 0x1c,
                Key::CloseBracket | Key::Num5 => 0x1d,
                Key::Num6 => 0x1e,
                Key::Slash | Key::Minus | Key::Num7 => 0x1f,
                Key::Num8 => 0x7f,
                _ => letter(key)? & 0x1f,
            };
            alt_prefix(vec![control])
        }
        // Alt+letter is ESC + the letter: readline's word motions, and the
        // Meta chords the agent TUIs bind. Text events are suppressed while
        // Alt is held (see the egui backend), so this is its only route.
        _ if modifiers.alt => {
            let byte = letter(key)?;
            let byte = if modifiers.shift {
                byte
            } else {
                byte.to_ascii_lowercase()
            };
            vec![0x1b, byte]
        }
        _ => return None,
    };
    Some(bytes)
}

/// Pasted text as a program expects it: wrapped in bracketed-paste markers
/// when the program asked for them, so an editor or an agent prompt takes it
/// as one paste rather than as typed lines; otherwise with newlines as the
/// carriage returns a keyboard would send.
#[must_use]
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // A pasted end marker would let the clipboard end the paste early and
        // type the rest as commands. Strip both markers from the payload.
        let clean = text.replace("\x1b[201~", "").replace("\x1b[200~", "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CTRL: Modifiers = Modifiers::CTRL;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const ALT: Modifiers = Modifiers::ALT;

    fn bytes(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
        key_bytes(key, modifiers, false)
    }

    #[test]
    fn control_letters_are_their_c0_codes() {
        assert_eq!(bytes(Key::C, CTRL), Some(vec![0x03]));
        assert_eq!(bytes(Key::D, CTRL), Some(vec![0x04]));
        assert_eq!(bytes(Key::Z, CTRL), Some(vec![0x1a]));
        assert_eq!(bytes(Key::A, CTRL), Some(vec![0x01]));
        assert_eq!(bytes(Key::Space, CTRL), Some(vec![0x00]));
        assert_eq!(bytes(Key::OpenBracket, CTRL), Some(vec![0x1b]));
    }

    #[test]
    fn plain_text_keys_are_left_to_their_text_events() {
        // Sending these here as well would type every letter twice.
        for key in [Key::A, Key::Space, Key::Num1, Key::Slash, Key::Period] {
            assert_eq!(bytes(key, NONE), None, "{key:?}");
            assert_eq!(bytes(key, SHIFT), None, "{key:?}");
        }
    }

    #[test]
    fn cursor_keys_follow_application_mode_and_modifiers() {
        assert_eq!(bytes(Key::ArrowUp, NONE), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            key_bytes(Key::ArrowUp, NONE, true),
            Some(b"\x1bOA".to_vec())
        );
        // Modifiers win over application mode; xterm does the same.
        assert_eq!(
            key_bytes(Key::ArrowLeft, CTRL, true),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(bytes(Key::ArrowRight, SHIFT), Some(b"\x1b[1;2C".to_vec()));
        assert_eq!(bytes(Key::Delete, NONE), Some(b"\x1b[3~".to_vec()));
        assert_eq!(bytes(Key::PageUp, SHIFT), Some(b"\x1b[5;2~".to_vec()));
        assert_eq!(bytes(Key::F1, NONE), Some(b"\x1bOP".to_vec()));
        assert_eq!(bytes(Key::F5, NONE), Some(b"\x1b[15~".to_vec()));
        assert_eq!(bytes(Key::F12, CTRL), Some(b"\x1b[24;5~".to_vec()));
    }

    #[test]
    fn editing_keys_send_what_readline_expects() {
        assert_eq!(bytes(Key::Enter, NONE), Some(b"\r".to_vec()));
        assert_eq!(bytes(Key::Enter, SHIFT), Some(b"\x1b\r".to_vec()));
        assert_eq!(bytes(Key::Backspace, NONE), Some(vec![0x7f]));
        assert_eq!(bytes(Key::Backspace, CTRL), Some(vec![0x08]));
        assert_eq!(bytes(Key::Tab, NONE), Some(b"\t".to_vec()));
        assert_eq!(bytes(Key::Tab, SHIFT), Some(b"\x1b[Z".to_vec()));
        assert_eq!(bytes(Key::Escape, NONE), Some(vec![0x1b]));
    }

    #[test]
    fn alt_letters_are_meta_prefixed() {
        assert_eq!(bytes(Key::B, ALT), Some(b"\x1bb".to_vec()));
        assert_eq!(bytes(Key::B, ALT | SHIFT), Some(b"\x1bB".to_vec()));
        assert_eq!(bytes(Key::Enter, ALT), Some(b"\x1b\r".to_vec()));
    }

    #[test]
    fn a_paste_cannot_end_its_own_bracket() {
        assert_eq!(
            paste_bytes("a\nb", true),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf ~\n", true),
            b"\x1b[200~xrm -rf ~\n\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc".to_vec());
    }
}
