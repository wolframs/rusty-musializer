//! The held frame behind a scene transition, as a resource.
//!
//! A transition composites the last picture the outgoing section drew over
//! the incoming one (`core::project::transition`). This is that picture: one
//! render texture the size of the scene's boundary, refreshed by copying the
//! framebuffer the scene was just drawn into.
//!
//! ## A blit, not a second draw
//!
//! The scene has already been rasterized where it belongs — the screen in the
//! preview, the supersampled target in an export. Copying those pixels costs
//! one `glBlitFramebuffer` and cannot disagree with what was drawn; drawing
//! the scene a second time into a buffer of our own would double the frame's
//! cost and would not include whatever a scene's own offscreen passes
//! composited.
//!
//! ## Framebuffer discipline is [`crate::halo`]'s
//!
//! Allocating a render texture rebinds framebuffer 0 as a side effect, and
//! `EndTextureMode` is not the inverse of `BeginTextureMode`. So the caller's
//! framebuffer is captured before the first GL call and restored the way
//! `halo.rs` and `feedback.rs` restore it. The blit itself binds the read and
//! draw targets separately and rebinds the caller's for both afterwards; it
//! moves no viewport, projection or cached size.
//!
//! ## The buffer is the whole framebuffer's size, and the copy keeps its place
//!
//! The preview's window is multisampled, and a blit that resolves samples is
//! `GL_INVALID_OPERATION` unless its two rectangles have the same dimensions.
//! Copying the scene's rectangle to the *same* rectangle of a buffer as large
//! as the framebuffer makes that true by construction, at every panel layout,
//! and [`HeldFrame::region`] says where the scene sits in it.
//!
//! ## `rlBlitFramebuffer` takes corners, whatever its header says
//!
//! Its parameters are named `srcWidth`/`srcHeight` and it hands them to
//! `glBlitFramebuffer` as `srcX1`/`srcY1` (`rlgl.h:1891-1896`). At the origin
//! the two readings agree, which is every export; in the preview panel the
//! first version of this copied a 172 by 8 pixel sliver and reported success.
//! The call below passes corners.
//!
//! [`HeldFrame::capture`] must run with **no scissor active**: the scissor
//! test applies to a blit's destination.

use raylib::consts::{TextureFilter, TextureWrap};
use raylib::prelude::{RaylibDrawHandle, Rectangle, RenderTexture2D};

/// Same bound, same reason, as the halo's and the feedback buffer's.
const MAX_BUFFER_EDGE: i32 = 8192;

const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_BUFFER_BIT: i32 = 0x4000;

/// The last frame a scene drew, kept for a transition to composite.
pub struct HeldFrame {
    target: Option<RenderTexture2D>,
    width: i32,
    height: i32,
    /// Set when the driver refused the render texture. Named in the report
    /// line, because a transition that silently became a cut is the same
    /// picture as a plan that asked for a cut.
    refused: bool,
    /// The scene time of the held picture. `None` before the first capture,
    /// after a resize and after [`HeldFrame::reset`].
    held_seconds: Option<f64>,
    /// Where the scene's rectangle sits in the buffer, as fractions of it:
    /// left, bottom, width, height, in texture coordinates.
    region: [f32; 4],
}

impl Default for HeldFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl HeldFrame {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target: None,
            width: 0,
            height: 0,
            refused: false,
            held_seconds: None,
            region: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Forgets the held picture. Export determinism, like the feedback
    /// buffer's reset: an export of the track already on screen must not open
    /// a transition onto a frame of the preview.
    pub fn reset(&mut self) {
        self.held_seconds = None;
    }

    #[must_use]
    pub fn refused(&self) -> bool {
        self.refused
    }

    /// The scene time of the held picture, if there is one.
    #[must_use]
    pub fn held_seconds(&self) -> Option<f64> {
        self.held_seconds
    }

    /// The buffer's colour texture, whether or not it holds a picture yet.
    ///
    /// A bare ffi handle because the composite draws it through
    /// [`crate::draw::draw_texture_pro`]; the allocation stays owned here.
    #[must_use]
    pub fn texture(&self) -> Option<raylib_sys::Texture2D> {
        self.target.as_ref().map(|target| target.texture)
    }

    /// The scene's rectangle inside [`HeldFrame::texture`]: left, bottom,
    /// width and height as fractions of the texture, bottom-up like the
    /// texture itself.
    #[must_use]
    pub fn region(&self) -> [f32; 4] {
        self.region
    }

    /// Makes sure a buffer exists for `boundary`, without capturing. The intro
    /// and the outro composite black through the same quad and need something
    /// to draw it with.
    pub fn prepare(&mut self, d: &mut RaylibDrawHandle<'_>, boundary: Rectangle) -> bool {
        let caller = Caller::capture(d);
        if !self.fits(caller.width, caller.height) {
            let ready = self.ensure(caller.width, caller.height);
            caller.restore();
            if !ready {
                return false;
            }
        }
        self.region = caller.region(boundary);
        true
    }

    /// Copies what the active framebuffer holds inside `boundary` into the
    /// buffer, as the picture drawn at `time_seconds`.
    ///
    /// `boundary` is in the framebuffer's own pixels, origin top left — the
    /// rectangle the scene was drawn into. Must be called inside a drawing
    /// context, which the handle proves, and with no scissor active.
    pub fn capture(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        boundary: Rectangle,
        time_seconds: f64,
    ) -> bool {
        let (x, y) = (boundary.x as i32, boundary.y as i32);
        let (width, height) = (boundary.width as i32, boundary.height as i32);
        let caller = Caller::capture(d);
        if !self.fits(caller.width, caller.height) {
            let ready = self.ensure(caller.width, caller.height);
            caller.restore();
            if !ready {
                return false;
            }
        }
        let bottom = caller.height - (y + height);
        let Some(target) = &self.target else {
            return false;
        };
        // SAFETY: three framebuffer binds and one blit, every argument by
        // value. `caller.fbo` is the framebuffer rlgl reported as active before
        // any call here, and `target.id` is a live framebuffer this struct
        // owns. The read and draw targets are bound separately for the copy and
        // then the caller's is bound for both again, which is the state the
        // caller left: `glBlitFramebuffer` moves no viewport, projection or
        // rlgl-cached size, so there is nothing else to put back. The source
        // rectangle is flipped into GL's bottom-left origin against the
        // caller's own framebuffer height; raylib's orthographic projection is
        // top-left on the screen and in a render target alike, so one
        // conversion serves both. Colour only; the four size arguments are
        // far *corners*, not extents, and the rectangle is the same on both
        // sides so a multisampled source resolves (see the module note).
        unsafe {
            raylib_sys::rlBindFramebuffer(GL_READ_FRAMEBUFFER, caller.fbo);
            raylib_sys::rlBindFramebuffer(GL_DRAW_FRAMEBUFFER, target.id);
            raylib_sys::rlBlitFramebuffer(
                x,
                bottom,
                x + width,
                bottom + height,
                x,
                bottom,
                x + width,
                bottom + height,
                GL_COLOR_BUFFER_BIT,
            );
            raylib_sys::rlBindFramebuffer(GL_FRAMEBUFFER, caller.fbo);
        }
        self.held_seconds = Some(time_seconds);
        self.region = caller.region(boundary);
        true
    }

    fn fits(&self, width: i32, height: i32) -> bool {
        self.target.is_some() && self.width == width && self.height == height
    }

    /// Allocates the buffer, or re-allocates it when the boundary changed. A
    /// resize drops the held picture: stretched over new geometry it would be
    /// a picture of nothing that was ever on screen.
    fn ensure(&mut self, width: i32, height: i32) -> bool {
        if self.refused
            || width < 1
            || height < 1
            || width > MAX_BUFFER_EDGE
            || height > MAX_BUFFER_EDGE
        {
            return false;
        }
        self.target = None;
        self.held_seconds = None;
        // SAFETY: `LoadRenderTexture` takes its dimensions by value; a zero id
        // is its failure signal and is checked before `RenderTexture2D::from_raw`
        // — whose `Drop` is `UnloadRenderTexture` — takes ownership. It rebinds
        // the framebuffer as a side effect, which is why both callers capture
        // theirs first and restore it after. Bilinear and clamped, taken by
        // value: several kinds sample the held picture between texels and
        // outside it.
        unsafe {
            let target = raylib_sys::LoadRenderTexture(width, height);
            if target.id == 0 {
                self.refused = true;
                return false;
            }
            raylib_sys::SetTextureFilter(
                target.texture,
                TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
            );
            raylib_sys::SetTextureWrap(target.texture, TextureWrap::TEXTURE_WRAP_CLAMP as i32);
            self.target = Some(RenderTexture2D::from_raw(target));
        }
        self.width = width;
        self.height = height;
        true
    }
}

/// The framebuffer that was active when a [`HeldFrame`] call began.
struct Caller {
    fbo: u32,
    width: i32,
    height: i32,
}

impl Caller {
    fn capture(_d: &mut RaylibDrawHandle<'_>) -> Self {
        // SAFETY: a batch flush and pure getters over global renderer state,
        // valid while a drawing context is active, which the handle proves.
        // The flush puts everything already queued into the caller's
        // framebuffer before it is read from. On the default framebuffer the
        // size authority is `GetRenderWidth`/`Height`, because `EndTextureMode`
        // leaves rlgl's cached pair at the last target's dimensions
        // (`rcore.c:1109-1131`); inside a render target it is the cached pair.
        // The same split `draw::SceneViewport` makes.
        unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            let fbo = raylib_sys::rlGetActiveFramebuffer();
            let (width, height) = if fbo == 0 {
                (raylib_sys::GetRenderWidth(), raylib_sys::GetRenderHeight())
            } else {
                (
                    raylib_sys::rlGetFramebufferWidth(),
                    raylib_sys::rlGetFramebufferHeight(),
                )
            };
            Self { fbo, width, height }
        }
    }

    /// `boundary` as fractions of this framebuffer, bottom-up.
    fn region(&self, boundary: Rectangle) -> [f32; 4] {
        let (width, height) = (self.width.max(1) as f32, self.height.max(1) as f32);
        [
            boundary.x.floor() / width,
            (height - (boundary.y.floor() + boundary.height.floor())) / height,
            boundary.width.floor() / width,
            boundary.height.floor() / height,
        ]
    }

    /// Puts the caller's framebuffer back after an allocation unbound it.
    fn restore(&self) {
        // SAFETY: `HaloBlur::render`'s restore, unchanged. `EndTextureMode`
        // restores binding, viewport and projection for the default framebuffer
        // but not rlgl's cached size pair, which the two setters repair; for a
        // render target `BeginTextureMode` reads only the id and the colour
        // texture's dimensions (`rcore.c:1079-1108`) and sets all three size
        // authorities itself, so a reconstructed handle restores all of it.
        unsafe {
            if self.fbo == 0 {
                raylib_sys::EndTextureMode();
                raylib_sys::rlSetFramebufferWidth(self.width);
                raylib_sys::rlSetFramebufferHeight(self.height);
            } else {
                let face = raylib_sys::Texture2D {
                    id: 0,
                    width: self.width,
                    height: self.height,
                    mipmaps: 1,
                    format: raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
                };
                raylib_sys::BeginTextureMode(raylib_sys::RenderTexture2D {
                    id: self.fbo,
                    texture: face,
                    depth: face,
                });
            }
        }
    }
}
