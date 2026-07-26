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

use raylib::prelude::{Camera3D, Color, RaylibDraw, Rectangle, Vector2, Vector3};

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

// ---------------------------------------------------------------------------
// 3D panel machinery.
//
// Hoisted here by the integration owner. Agents C and D independently ported the
// same viewport block out of the C's 3D scenes and both asked for it to live in
// one place: Orbital Lattice, Song Atlas, Spectral Terrarium and Constellation all
// need it, and Song Atlas was importing it across a scene-module boundary, which
// reads wrong and would have drifted.
// ---------------------------------------------------------------------------

/// Restricts 3D rendering to `boundary` without changing the composition inside it.
///
/// Port of the viewport block the C repeats in every 3D scene
/// (`scene_spectral_terrarium.c:555-604`, `scene_constellation.c:180-318`). Three
/// things have to happen together and none of them is optional:
///
/// 1. the GL viewport is narrowed to the panel's pixels, so 3D geometry cannot
///    spill over the UI;
/// 2. rlgl's notion of the framebuffer size is narrowed with it, because
///    `BeginMode3D` derives its projection aspect from that;
/// 3. the projection is then rescaled in X by `full_aspect / viewport_aspect`, so
///    the scene is framed by the *window's* aspect rather than the panel's. Without
///    this a narrow preview panel stretches the world instead of cropping it.
///
/// The batch is flushed before and after, because rlgl defers draw calls and a
/// pending batch would otherwise be rasterized under the wrong viewport.
pub struct SceneViewport {
    saved_width: i32,
    saved_height: i32,
    width: i32,
    height: i32,
}

impl SceneViewport {
    /// Narrows the viewport to `boundary`, or returns `None` when the result would
    /// be degenerate — the same early returns the C takes.
    pub fn begin<D: RaylibDraw>(_d: &mut D, boundary: Rectangle) -> Option<Self> {
        // SAFETY: pure raylib getters over global window state.
        let (screen_width, screen_height) =
            unsafe { (raylib_sys::GetScreenWidth(), raylib_sys::GetScreenHeight()) };
        Self::begin_with_screen(boundary, screen_width, screen_height)
    }

    /// [`SceneViewport::begin`] with the screen size supplied rather than read.
    ///
    /// For a caller whose draw handle is already borrowed, and for tests.
    pub fn begin_with_screen(
        boundary: Rectangle,
        screen_width: i32,
        screen_height: i32,
    ) -> Option<Self> {
        // SAFETY: pure rlgl getters over global renderer state, valid while a
        // drawing context is active — which every constructor path requires.
        let (saved_width, saved_height, active_framebuffer) = unsafe {
            (
                raylib_sys::rlGetFramebufferWidth(),
                raylib_sys::rlGetFramebufferHeight(),
                raylib_sys::rlGetActiveFramebuffer(),
            )
        };
        if saved_width < 1 || saved_height < 1 {
            return None;
        }
        // When drawing to the default framebuffer, boundary coordinates are in
        // screen space rather than framebuffer space; a supersampled export path
        // renders into a render texture where the two already agree.
        let (coordinate_width, coordinate_height) = if active_framebuffer == 0 {
            if screen_width < 1 || screen_height < 1 {
                return None;
            }
            (screen_width as f32, screen_height as f32)
        } else {
            (saved_width as f32, saved_height as f32)
        };
        let scale_x = saved_width as f32 / coordinate_width;
        let scale_y = saved_height as f32 / coordinate_height;
        let x = (boundary.x * scale_x).round() as i32;
        let width = (boundary.width * scale_x).round() as i32;
        let height = (boundary.height * scale_y).round() as i32;
        // GL's origin is bottom-left, the boundary's is top-left.
        let top = ((boundary.y + boundary.height) * scale_y).round() as i32;
        let y = saved_height - top;
        if width < 1 || height < 1 {
            return None;
        }

        // SAFETY: rlgl state changes on the render thread while a drawing context is
        // active. The batch is flushed first so nothing already queued is rasterized
        // under the new viewport, and `end` restores every value this touches.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlViewport(x, y, width, height);
            raylib_sys::rlSetFramebufferWidth(width);
            raylib_sys::rlSetFramebufferHeight(height);
        }
        Some(Self {
            saved_width,
            saved_height,
            width,
            height,
        })
    }

    /// Rescales the projection so the panel crops the world instead of stretching
    /// it. Call this immediately inside `BeginMode3D`, as the C does.
    pub fn correct_aspect<D: raylib::prelude::RaylibDraw3D>(&self, _d: &mut D) {
        let viewport_aspect = self.width as f32 / self.height as f32;
        let full_aspect = self.saved_width as f32 / self.saved_height as f32;
        // SAFETY: matrix-stack calls on the active rlgl context, balanced so the
        // mode is left back on MODELVIEW exactly as `BeginMode3D` left it.
        unsafe {
            raylib_sys::rlMatrixMode(raylib_sys::RL_PROJECTION as i32);
            raylib_sys::rlScalef(full_aspect / viewport_aspect, 1.0, 1.0);
            raylib_sys::rlMatrixMode(raylib_sys::RL_MODELVIEW as i32);
        }
    }

    /// `correct_aspect` without a draw handle, for callers already inside a 3D mode
    /// whose handle is borrowed elsewhere.
    ///
    /// Identical arithmetic; the caller carries the responsibility that
    /// `correct_aspect`'s `_d` parameter otherwise proves.
    pub fn correct_projection_aspect(&self) {
        let viewport_aspect = self.width as f32 / self.height as f32;
        let full_aspect = self.saved_width as f32 / self.saved_height as f32;
        // SAFETY: matrix-stack calls on the active rlgl context, balanced so the
        // mode is left back on MODELVIEW exactly as `BeginMode3D` left it.
        unsafe {
            raylib_sys::rlMatrixMode(raylib_sys::RL_PROJECTION as i32);
            raylib_sys::rlScalef(full_aspect / viewport_aspect, 1.0, 1.0);
            raylib_sys::rlMatrixMode(raylib_sys::RL_MODELVIEW as i32);
        }
    }

    /// Scales the modelview matrix. Song Atlas stretches its terrain this way
    /// rather than scaling every vertex.
    ///
    /// Must be called inside an active 3D mode.
    pub fn scale_modelview(&self, x: f32, y: f32, z: f32) {
        // SAFETY: an rlgl matrix call inside an active 3D mode, which the caller is
        // required to have entered.
        unsafe { raylib_sys::rlScalef(x, y, z) }
    }

    /// Restores the full viewport. Call after `EndMode3D`.
    ///
    /// Consuming `self` runs [`Drop`], which does the actual restoring, so an
    /// explicit call and an early `return` behave identically.
    pub fn end<D: RaylibDraw>(self, _d: &mut D) {}
}

/// Restoring on drop is deliberate, and it is the reason this type exists rather
/// than a pair of functions.
///
/// A narrowed viewport is global renderer state. If a scene returns early — and
/// several do, on a degenerate boundary or an empty band list — a forgotten
/// restore does not corrupt that scene, it corrupts **everything drawn afterwards
/// in the frame**, including the UI. That is a miserable bug to find from a
/// screenshot. Agents C and D ported this block independently and split on exactly
/// this point: one used `Drop`, the other an explicit `end`. Both are right, so
/// this has both.
///
/// The invariant a caller must uphold: a `SceneViewport` is only constructible
/// inside an active drawing context, and it must be dropped inside that same
/// context. Every current caller drops it within the same `draw` call, which is
/// the only shape the borrow of the draw handle in `begin` permits anyway.
impl Drop for SceneViewport {
    fn drop(&mut self) {
        // SAFETY: restores exactly the values captured in `begin`, after flushing
        // the batch drawn under the narrowed viewport. Valid because the drawing
        // context that `begin` required is still active — see the note above.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            raylib_sys::rlSetFramebufferWidth(self.saved_width);
            raylib_sys::rlSetFramebufferHeight(self.saved_height);
            raylib_sys::rlViewport(0, 0, self.saved_width, self.saved_height);
        }
    }
}

/// `DrawBillboardRec` over the non-owning default texture.
///
/// The safe API takes an owned `Texture2D`, which would unload raylib's default
/// texture on drop — the same reason `runtime::draw` wraps `DrawTexturePro`.
pub fn draw_billboard_rec<D: raylib::prelude::RaylibDraw3D>(
    _d: &mut D,
    camera: Camera3D,
    texture: raylib_sys::Texture2D,
    source: Rectangle,
    position: Vector3,
    size: Vector2,
    tint: Color,
) {
    // SAFETY: an immediate-mode draw call with a valid texture id and by-value
    // geometry, inside an active 3D drawing context proven by `_d`.
    unsafe {
        raylib_sys::DrawBillboardRec(
            camera.into(),
            texture,
            source.into(),
            position.into(),
            size.into(),
            tint.into(),
        );
    }
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
