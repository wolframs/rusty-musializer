//! A render texture to draw into mid-frame, and back out of, safely.
//!
//! Post-legacy (MV9). Three new things draw offscreen in the middle of a
//! frame: the bloom of the finish pass, its blur, and a scene nested inside
//! another (the threshold's doorway). Each needs what `halo.rs`,
//! `feedback.rs` and `transition.rs` each needed — a buffer, a way in, and a
//! way back to whatever framebuffer the caller was drawing into — so it is one
//! type here instead of a fourth copy of the same unsafe blocks.
//!
//! ## Framebuffer discipline is [`crate::halo`]'s, exactly
//!
//! raylib texture modes do not nest and `EndTextureMode` is not the inverse of
//! `BeginTextureMode`: it always returns to the *default* framebuffer and never
//! restores rlgl's cached size pair (`rcore.c:1109-1131`). So
//! [`Canvas::draw_into`] captures the caller's framebuffer before its first GL
//! call and restores it on every exit path — `EndTextureMode` plus the explicit
//! size pair for the screen, a reconstructed `BeginTextureMode` for a render
//! target (every export frame, and a nested scene's own offscreen passes,
//! which therefore return to *this* canvas rather than to the screen).
//!
//! The size authority on the default framebuffer is `GetRenderWidth`/`Height`
//! rather than rlgl's cached pair, because the cached pair may hold the last
//! target's dimensions: the same split `draw::SceneViewport` and
//! `transition.rs` make.
//!
//! Like every other offscreen pass here, [`Canvas::draw_into`] must run with
//! **no scissor active**: a scissor rect is global GL state in framebuffer
//! coordinates and would clip the offscreen pass with a rectangle meant for
//! the screen. The body may open its own.

use raylib::consts::{TextureFilter, TextureWrap};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, RenderTexture2D};

/// Same bound, same reason, as the halo's and the feedback buffer's.
const MAX_BUFFER_EDGE: i32 = 8192;

/// One render texture, drawn into by a closure.
pub struct Canvas {
    target: Option<RenderTexture2D>,
    width: i32,
    height: i32,
    refused: bool,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target: None,
            width: 0,
            height: 0,
            refused: false,
        }
    }

    /// Whether the driver refused the buffer. Sticky: a refusal is named on a
    /// report line rather than retried every frame.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.refused
    }

    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// The colour texture, upright in GL's bottom-up convention like every
    /// render texture: draw it with a negative source height.
    #[must_use]
    pub fn texture(&self) -> Option<raylib_sys::Texture2D> {
        self.target.as_ref().map(|target| target.texture)
    }

    /// Clears the buffer to `clear`, runs `body` drawing into it in **buffer**
    /// coordinates (origin top left, `width` by `height`), and puts the
    /// caller's framebuffer back. Returns whether the buffer was drawn.
    ///
    /// Must be called inside a drawing context, which the handle proves, and
    /// with no scissor active.
    pub fn draw_into(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        width: i32,
        height: i32,
        clear: Color,
        body: impl FnOnce(&mut RaylibDrawHandle<'_>),
    ) -> bool {
        if self.refused
            || width < 1
            || height < 1
            || width > MAX_BUFFER_EDGE
            || height > MAX_BUFFER_EDGE
        {
            return false;
        }
        let caller = Caller::capture(d);
        if !self.ensure(width, height) {
            caller.restore();
            return false;
        }
        let Some(target) = &self.target else {
            caller.restore();
            return false;
        };
        // SAFETY: binds a framebuffer this struct owns, whose colour texture
        // has the dimensions just ensured. `BeginTextureMode` flushes the batch
        // before switching and sets the viewport, projection and rlgl's size
        // pair to the target's; `caller.restore` below undoes all three on
        // every path that reaches here.
        unsafe { raylib_sys::BeginTextureMode(**target) };
        d.clear_background(clear);
        body(d);
        caller.restore();
        true
    }

    /// Allocates the buffer, or re-allocates it when the size changed.
    fn ensure(&mut self, width: i32, height: i32) -> bool {
        if self.target.is_some() && self.width == width && self.height == height {
            return true;
        }
        self.target = None;
        // SAFETY: `LoadRenderTexture` takes its dimensions by value; a zero id
        // is its failure signal and is checked before `RenderTexture2D::from_raw`
        // — whose `Drop` is `UnloadRenderTexture` — takes ownership. It rebinds
        // the framebuffer as a side effect, which is why `draw_into` captures
        // the caller's binding before this runs and restores it on every exit
        // path. The filter and wrap calls take the colour texture by value:
        // bilinear because every reader samples between texels, clamp because a
        // blur reads past the edge and wrap-around would pull the opposite
        // edge's light in.
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

/// The framebuffer that was active when a [`Canvas::draw_into`] began.
struct Caller {
    fbo: u32,
    width: i32,
    height: i32,
}

impl Caller {
    fn capture(_d: &mut RaylibDrawHandle<'_>) -> Self {
        // SAFETY: a batch flush and pure getters over global renderer state,
        // valid while a drawing context is active, which the handle proves. The
        // flush puts everything already queued into the caller's framebuffer
        // before anything is rebound. `transition.rs`'s capture, unchanged.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_canvas_has_nothing_to_offer_and_has_refused_nothing() {
        let canvas = Canvas::new();
        assert!(canvas.texture().is_none());
        assert!(!canvas.refused());
        assert_eq!(canvas.size(), (0, 0));
    }
}
