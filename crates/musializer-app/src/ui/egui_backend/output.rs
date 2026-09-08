//! Platform cursor and clipboard requests from egui.

use egui::{CursorIcon, OutputCommand, PlatformOutput};
use raylib::prelude::*;

fn map_cursor(cursor: CursorIcon) -> Option<MouseCursor> {
    match cursor {
        CursorIcon::None => None,

        CursorIcon::Default
        | CursorIcon::Progress
        | CursorIcon::Wait
        | CursorIcon::Help
        | CursorIcon::Alias
        | CursorIcon::Copy => Some(MouseCursor::MOUSE_CURSOR_DEFAULT),

        CursorIcon::Text | CursorIcon::VerticalText => Some(MouseCursor::MOUSE_CURSOR_IBEAM),

        CursorIcon::Crosshair | CursorIcon::Cell | CursorIcon::ZoomIn | CursorIcon::ZoomOut => {
            Some(MouseCursor::MOUSE_CURSOR_CROSSHAIR)
        }

        CursorIcon::PointingHand
        | CursorIcon::ContextMenu
        | CursorIcon::Grab
        | CursorIcon::Grabbing => Some(MouseCursor::MOUSE_CURSOR_POINTING_HAND),

        CursorIcon::ResizeHorizontal
        | CursorIcon::ResizeColumn
        | CursorIcon::ResizeEast
        | CursorIcon::ResizeWest => Some(MouseCursor::MOUSE_CURSOR_RESIZE_EW),

        CursorIcon::ResizeVertical
        | CursorIcon::ResizeRow
        | CursorIcon::ResizeNorth
        | CursorIcon::ResizeSouth => Some(MouseCursor::MOUSE_CURSOR_RESIZE_NS),

        CursorIcon::ResizeNwSe | CursorIcon::ResizeNorthWest | CursorIcon::ResizeSouthEast => {
            Some(MouseCursor::MOUSE_CURSOR_RESIZE_NWSE)
        }

        CursorIcon::ResizeNeSw | CursorIcon::ResizeNorthEast | CursorIcon::ResizeSouthWest => {
            Some(MouseCursor::MOUSE_CURSOR_RESIZE_NESW)
        }

        CursorIcon::Move | CursorIcon::AllScroll => Some(MouseCursor::MOUSE_CURSOR_RESIZE_ALL),

        CursorIcon::NoDrop | CursorIcon::NotAllowed => Some(MouseCursor::MOUSE_CURSOR_NOT_ALLOWED),
    }
}

pub(crate) fn handle_platform_output(rl: &mut RaylibHandle, output: PlatformOutput) {
    for command in output.commands {
        match command {
            OutputCommand::CopyText(text) => rl.set_clipboard_text(&text).unwrap_or_default(),
            OutputCommand::CopyImage(_color_image) => (),
            OutputCommand::OpenUrl(_open_url) => (),
        }
    }

    match map_cursor(output.cursor_icon) {
        Some(cursor) => {
            rl.set_mouse_cursor(cursor);
            rl.show_cursor();
        }
        None => {
            rl.hide_cursor();
        }
    };
}
