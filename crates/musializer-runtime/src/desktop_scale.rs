//! The desktop's scale, asked of GLFW because raylib 6.0 no longer says.
//!
//! `GetWindowScaleDPI` answers `{1, 1}` unless the window was created with
//! `FLAG_WINDOW_HIGHDPI` (`rcore_desktop_glfw.c:1025-1031`), and that flag is
//! not ours to set: it makes raylib scale the whole frame itself, on top of
//! the shell's own logical units and native font atlases. So on every desktop
//! the shell's Auto scale was handed 1.0 and never knew the desktop had one.

use raylib::prelude::{RaylibHandle, RaylibThread};

/// GLFW's opaque monitor.
#[repr(C)]
struct GlfwMonitor {
    _private: [u8; 0],
}

extern "C" {
    fn glfwGetPrimaryMonitor() -> *mut GlfwMonitor;
    fn glfwGetMonitorContentScale(monitor: *mut GlfwMonitor, xscale: *mut f32, yscale: *mut f32);
}

/// The content scale of the primary monitor: 1.25 on a 125 % desktop.
///
/// On X11 this is one number for the whole display, read once from the
/// `Xft.dpi` resource when GLFW starts, so the primary monitor answers for
/// all of them. `None` when there is no monitor or the answer is not a usable
/// scale; the caller treats that as a desktop with none.
///
/// Both GLFW calls are main-thread only, and the monitor array they read is
/// reallocated on hotplug. `RaylibHandle` is `Sync`, so it proves the window
/// and nothing about the thread; `RaylibThread` is neither `Send` nor `Sync`
/// and exists only on the thread that created the window.
#[must_use]
pub fn content_scale(_window: &RaylibHandle, _thread: &RaylibThread) -> Option<(f32, f32)> {
    let (mut x, mut y) = (0.0f32, 0.0f32);
    // SAFETY: the caller borrows the live window handle, so GLFW is
    // initialized, and the thread token, so this is GLFW's main thread and
    // no monitor callback runs beside it. `glfwGetPrimaryMonitor` takes
    // nothing and returns null when no monitor is connected, which is checked
    // before the handle is used. `glfwGetMonitorContentScale` writes one
    // float through each pointer, both to locals that outlive the call, and
    // keeps neither.
    unsafe {
        let monitor = glfwGetPrimaryMonitor();
        if monitor.is_null() {
            return None;
        }
        glfwGetMonitorContentScale(monitor, &mut x, &mut y);
    }
    usable(x, y)
}

/// GLFW writes zeros when it has nothing to say; a scale has to be a finite
/// positive number before window sizes are multiplied by it.
fn usable(x: f32, y: f32) -> Option<(f32, f32)> {
    let ok = |value: f32| value.is_finite() && value > 0.0;
    (ok(x) && ok(y)).then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::usable;

    #[test]
    fn only_a_finite_positive_pair_is_a_scale() {
        assert_eq!(usable(1.25, 1.25), Some((1.25, 1.25)));
        assert_eq!(usable(0.0, 0.0), None);
        assert_eq!(usable(1.0, f32::NAN), None);
        assert_eq!(usable(f32::INFINITY, 1.0), None);
        assert_eq!(usable(-1.0, 1.0), None);
    }
}
