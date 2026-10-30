//! Clipboard reads with the nullable GLFW result checked before dereferencing.

use std::ffi::CStr;
use std::str::Utf8Error;

use raylib::prelude::RaylibHandle;

/// Copy clipboard text on the window thread. An absent selection is empty.
///
/// raylib-rs 6.0's wrapper passes GLFW's null result to `CStr::from_ptr`.
/// Keep the copy inside this call, before another clipboard read can replace it.
pub fn text(_window: &RaylibHandle) -> Result<String, Utf8Error> {
    // SAFETY: the caller borrows the live window handle on its UI thread.
    // GLFW owns the NUL-terminated buffer until the next clipboard call; null
    // means no text selection. We check it and copy before returning.
    unsafe {
        let pointer = raylib_sys::GetClipboardText();
        if pointer.is_null() {
            Ok(String::new())
        } else {
            CStr::from_ptr(pointer).to_str().map(str::to_owned)
        }
    }
}
