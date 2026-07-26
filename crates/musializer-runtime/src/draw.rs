//! Drawing primitives shared by every scene.
//!
//! **Shared contract.** Consumed by Agents C, D and E. Port of
//! `../musializer/src/scene_draw.h`, plus the small ffi wrappers the scenes need
//! that `raylib`'s safe API does not cover.
//!
//! `scene_draw.h` exists because GL line primitives are implementation-defined
//! and commonly rasterize as a single aliased pixel. Both scene agents call
//! [`tube`] rather than each inventing line drawing.
//!
//! ## The drawing-context rule
//!
//! Every function here issues an immediate-mode raylib draw call, which is only
//! valid between `BeginDrawing` and `EndDrawing`. In this codebase that means:
//! **call these only while you hold a `RaylibDrawHandle`** (or a mode handle
//! derived from one). The handle is the proof, so these take `&mut impl
//! RaylibDraw` where they can, and where the raw ffi has no safe counterpart
//! they take the handle anyway purely to make the requirement checkable.

use raylib::prelude::{Color, RaylibDraw, Rectangle, Vector2, Vector3};

/// A non-owning handle to raylib's built-in 1x1 white texture.
///
/// Several scenes draw shader-shaped quads by stretching this
/// (`../musializer/src/scene_spectrum.c:77`). It must **not** be wrapped in
/// `raylib::texture::Texture2D`: that type unloads on drop, and unloading
/// raylib's default texture would take the renderer down with it.
#[must_use]
pub fn default_texture() -> raylib_sys::Texture2D {
    raylib_sys::Texture2D {
        // SAFETY: a pure getter over rlgl's global state. Valid once the window
        // exists, which every caller of this module already requires.
        id: unsafe { raylib_sys::rlGetTextureIdDefault() },
        width: 1,
        height: 1,
        mipmaps: 1,
        format: raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
    }
}

/// A small camera-space tube, used wherever a 3D scene wants a visible line.
///
/// Port of `scene_draw_tube` (`../musializer/src/scene_draw.h:12-24`). A tube has
/// stable world-space thickness, participates in depth testing, and is smoothed
/// by both preview MSAA and the deterministic offline supersampling pass — none
/// of which a GL line does.
///
/// Silently does nothing for a non-finite or non-positive radius, or a
/// degenerate segment, exactly as C does. `sides` is raised to 3 if lower.
pub fn tube<D: RaylibDraw3D>(
    d: &mut D,
    start: Vector3,
    end: Vector3,
    radius: f32,
    sides: i32,
    color: Color,
) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let dz = end.z - start.z;
    if !radius.is_finite() || radius <= 0.0 || dx * dx + dy * dy + dz * dz <= 1.0e-10 {
        return;
    }
    let sides = sides.max(3);
    d.draw_cylinder_ex(start, end, radius, radius, sides, color);
}

/// The 3D drawing capability [`tube`] needs, so scenes can pass whatever mode
/// handle they are inside.
pub trait RaylibDraw3D {
    fn draw_cylinder_ex(
        &mut self,
        start: Vector3,
        end: Vector3,
        start_radius: f32,
        end_radius: f32,
        sides: i32,
        color: Color,
    );
}

impl<T: raylib::prelude::RaylibDraw3D> RaylibDraw3D for T {
    fn draw_cylinder_ex(
        &mut self,
        start: Vector3,
        end: Vector3,
        start_radius: f32,
        end_radius: f32,
        sides: i32,
        color: Color,
    ) {
        raylib::prelude::RaylibDraw3D::draw_cylinder_ex(
            self,
            start,
            end,
            start_radius,
            end_radius,
            sides,
            color,
        );
    }
}

/// Draws a source rectangle of a raw texture into a destination rectangle.
///
/// Wraps `DrawTexturePro` for the non-owning [`default_texture`] case, which the
/// safe API cannot express without taking ownership.
///
/// The `_d` handle is unused at runtime; it is the compile-time proof that a
/// drawing context is active.
pub fn draw_texture_pro<D: RaylibDraw>(
    _d: &mut D,
    texture: raylib_sys::Texture2D,
    source: Rectangle,
    dest: Rectangle,
    origin: Vector2,
    rotation: f32,
    tint: Color,
) {
    // SAFETY: an immediate-mode draw call with a valid texture id and by-value
    // geometry. The `_d` borrow proves BeginDrawing is active, which is the only
    // precondition rlgl has here.
    unsafe {
        raylib_sys::DrawTexturePro(
            texture,
            to_ffi_rect(source),
            to_ffi_rect(dest),
            to_ffi_vec2(origin),
            rotation,
            to_ffi_color(tint),
        );
    }
}

/// Draws a raw texture scaled about its top-left corner.
///
/// Wraps `DrawTextureEx` for the non-owning [`default_texture`] case.
pub fn draw_texture_ex<D: RaylibDraw>(
    _d: &mut D,
    texture: raylib_sys::Texture2D,
    position: Vector2,
    rotation: f32,
    scale: f32,
    tint: Color,
) {
    // SAFETY: as `draw_texture_pro`.
    unsafe {
        raylib_sys::DrawTextureEx(
            texture,
            to_ffi_vec2(position),
            rotation,
            scale,
            to_ffi_color(tint),
        );
    }
}

/// HSV to RGB, matching raylib's `ColorFromHSV` bit for bit.
///
/// Scenes compute hue per band and must agree with the C renderer, so this calls
/// raylib rather than reimplementing the conversion.
#[must_use]
pub fn color_from_hsv(hue: f32, saturation: f32, value: f32) -> Color {
    // SAFETY: a pure arithmetic function with no global state.
    let raw = unsafe { raylib_sys::ColorFromHSV(hue, saturation, value) };
    Color::new(raw.r, raw.g, raw.b, raw.a)
}

/// Scales a colour's alpha, matching raylib's `ColorAlpha`.
#[must_use]
pub fn color_alpha(color: Color, alpha: f32) -> Color {
    // SAFETY: pure arithmetic.
    let raw = unsafe { raylib_sys::ColorAlpha(to_ffi_color(color), alpha) };
    Color::new(raw.r, raw.g, raw.b, raw.a)
}

fn to_ffi_rect(rect: Rectangle) -> raylib_sys::Rectangle {
    raylib_sys::Rectangle {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn to_ffi_vec2(v: Vector2) -> raylib_sys::Vector2 {
    raylib_sys::Vector2 { x: v.x, y: v.y }
}

fn to_ffi_color(color: Color) -> raylib_sys::Color {
    raylib_sys::Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}
