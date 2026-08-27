//! Frame-persistence feedback: the classic visualizer trick, as a resource.
//!
//! A feedback buffer holds the *light* a scene emitted over the last fraction
//! of a second. Each frame the previous buffer is redrawn into the other half
//! of a ping-pong pair — faded, and optionally zoomed and rotated about a point
//! — and this frame's emissive drawing is laid over it; the caller then
//! composites the result under its crisp foreground. Moving light leaves a
//! streak, a transient leaves a ghost, and nothing that stays still gets
//! smeared.
//!
//! The family is well known (Le Biniou's whole effect vocabulary is built on
//! it); nothing here is ported from it. This is the mechanism, written against
//! the constraints this repository already has:
//!
//! ## Everything is per second, never per frame
//!
//! `retain` is `0.5^(delta / half_life)` and `zoom` is `rate^delta`, computed by
//! the caller from the scene clock. A per-frame constant would make a 30 fps
//! export smear half as far as a 60 fps preview of the same music — the same
//! defect class as the freewheeling clock the cats had.
//!
//! ## The buffer is premultiplied, so one accumulation serves both composites
//!
//! Contents are premultiplied RGBA over a transparent clear: `rgb` already
//! carries its own alpha. That is what makes the fade a plain tint multiply
//! (scaling `rgb` and `a` by the same factor keeps the premultiplication), and
//! it lets the caller pick its composite from the frame rather than from the
//! buffer's format — `BLEND_ALPHA_PREMULTIPLY` paints the trail *over* a light
//! background, where additive light saturates toward white and vanishes, and
//! `BLEND_ADD_COLORS` adds it as light over a dark one. Both read the same
//! texels. (Straight alpha cannot do this: raylib's `BLEND_ALPHA` uses `srcA`
//! as both the colour factor and the alpha factor, so re-drawing a straight
//! buffer through it squares the coverage on every frame and the trail collapses
//! toward black in a handful of frames.)
//!
//! ## Render-target discipline is [`crate::halo`]'s, exactly
//!
//! `EndTextureMode` is not the inverse of `BeginTextureMode` — it restores the
//! viewport and `currentFbo` but leaves rlgl's cached framebuffer size at the
//! *target's* dimensions (`rcore.c:1109-1131`, `rcore.c:3537`), and nothing
//! later in a frame puts it back. And a caller may already be inside a texture
//! mode (every export frame is), which `EndTextureMode` would end behind its
//! back. So this module captures the active framebuffer before its first GL
//! call and restores it on every exit path, screen and render target alike. See
//! `halo.rs`'s module note for the full reasoning and the damage.
//!
//! Like the halo, `accumulate` must run with **no scissor active**: a scissor
//! rect is global GL state in framebuffer coordinates and would clip the
//! offscreen pass with a rectangle meant for the screen.

use raylib::consts::{TextureFilter, TextureWrap};
use raylib::math::Vector2;
use raylib::prelude::{BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle};
use raylib::prelude::{Rectangle, RenderTexture2D};

use crate::draw;

/// Refusing absurd buffer requests beats letting a corrupt boundary allocate
/// one. Same constant, same reason, as the halo's.
const MAX_BUFFER_EDGE: i32 = 8192;

/// How the previous frame is carried into this one.
///
/// Every field is already resolved for *this frame's delta* by the caller: the
/// buffer does no time arithmetic of its own, so there is exactly one place a
/// per-frame constant could sneak in and it is the scene that owns the clock.
#[derive(Clone, Copy, Debug)]
pub struct Carry {
    /// Fraction of the previous buffer that survives, `0..=1`. Normally
    /// `0.5f32.powf(delta / half_life)`.
    pub retain: f32,
    /// Scale applied to the previous buffer about [`Carry::centre`]. Above 1 the
    /// trail creeps outward (light blooming away from the flower), below 1 it
    /// draws inward.
    pub zoom: f32,
    /// Degrees the previous buffer turns about [`Carry::centre`], for swirl.
    pub rotation_degrees: f32,
    /// The zoom/rotation pivot, in **buffer** pixels.
    pub centre: Vector2,
}

/// A ping-pong pair of render textures holding one scene's accumulated light.
///
/// Created empty and allocated on first use, because the size follows the scene
/// boundary — which is one thing under the preview panel and another under a
/// supersampled export target, and changes whenever the window does.
pub struct FeedbackBuffer {
    /// The accumulated trail, after the last [`FeedbackBuffer::accumulate`].
    front: Option<RenderTexture2D>,
    /// This frame's target, which becomes `front` on the swap.
    back: Option<RenderTexture2D>,
    width: i32,
    height: i32,
    /// Set when an allocation was refused. A refusal is a supported state — the
    /// scene draws exactly as it did before feedback existed — but it must be
    /// *named*, because a frame with no trails and a frame whose buffer
    /// silently failed to build are the same picture.
    refused: bool,
    /// False when `front` holds nothing worth carrying: before the first
    /// accumulate, after a resize, and after [`FeedbackBuffer::reset`].
    carries: bool,
    /// Whether the last accumulate built something [`FeedbackBuffer::result`]
    /// may be read from.
    built: bool,
}

impl Default for FeedbackBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            front: None,
            back: None,
            width: 0,
            height: 0,
            refused: false,
            carries: false,
            built: false,
        }
    }

    /// Throws the accumulated light away, so the next frame starts from black.
    ///
    /// **A correctness requirement, not a convenience.** An export must
    /// reproduce frame for frame, and a preview's trails sitting in the buffer
    /// when encoding starts would put a picture of whatever the user was
    /// looking at into frame zero. Called when an export session begins, when a
    /// still's replay begins, and whenever the drawn scene or track changes.
    ///
    /// A **preview seek** deliberately does not reset: the light from before the
    /// jump decays inside a second, nothing downstream can observe it, and a
    /// scrub is a continuous gesture that would otherwise clear the buffer on
    /// every frame of the drag.
    pub fn reset(&mut self) {
        self.carries = false;
        self.built = false;
    }

    /// True once an allocation has been refused — the scene then draws its
    /// ordinary frame and says so.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.refused
    }

    /// Fades the previous frame into the other buffer, lays `draw_fresh` over
    /// it, and swaps. Returns whether a buffer is now readable.
    ///
    /// `draw_fresh` draws in **buffer** coordinates (origin at the buffer's top
    /// left, `width`x`height` across) and picks its own blend mode; its colours
    /// must be premultiplied — see the module note. Must be called inside an
    /// active drawing context, which the handle proves, and with no scissor
    /// active.
    pub fn accumulate(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        width: i32,
        height: i32,
        carry: Carry,
        draw_fresh: impl FnOnce(&mut RaylibDrawHandle<'_>),
    ) -> bool {
        self.built = false;
        if self.refused
            || width < 1
            || height < 1
            || width > MAX_BUFFER_EDGE
            || height > MAX_BUFFER_EDGE
            || !carry.retain.is_finite()
            || !carry.zoom.is_finite()
            || !carry.rotation_degrees.is_finite()
        {
            return false;
        }
        // SAFETY: pure rlgl getters over global renderer state, valid while a
        // drawing context is active, which the handle proves. The batch is
        // flushed first so geometry already queued for the caller's framebuffer
        // is rasterized there and not under a later binding. Identical to
        // `HaloBlur::render`'s capture, and for the same reasons.
        let (previous_fbo, previous_width, previous_height) = unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            (
                raylib_sys::rlGetActiveFramebuffer(),
                raylib_sys::rlGetFramebufferWidth(),
                raylib_sys::rlGetFramebufferHeight(),
            )
        };
        let built = self.pass(d, width, height, carry, draw_fresh);
        // SAFETY: restores the framebuffer captured above, unconditionally —
        // even a refused allocation has already unbound the caller's
        // framebuffer, because `rlLoadFramebuffer` binds 0 as a side effect.
        // `EndTextureMode` restores the binding, viewport, projection and
        // `currentFbo` when the caller was the default framebuffer but *not*
        // rlgl's cached size pair, which is what the two setters after it
        // repair; when the caller was a render target, `BeginTextureMode` reads
        // only `id` and the colour texture's dimensions (`rcore.c:1079-1108`),
        // so a reconstructed handle carrying those three restores all of it.
        // This is `HaloBlur::render`'s restore, unchanged.
        unsafe {
            if previous_fbo == 0 {
                raylib_sys::EndTextureMode();
                raylib_sys::rlSetFramebufferWidth(previous_width);
                raylib_sys::rlSetFramebufferHeight(previous_height);
            } else {
                raylib_sys::BeginTextureMode(raylib_sys::RenderTexture2D {
                    id: previous_fbo,
                    texture: raylib_sys::Texture2D {
                        id: 0,
                        width: previous_width,
                        height: previous_height,
                        mipmaps: 1,
                        format: raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
                    },
                    depth: raylib_sys::Texture2D {
                        id: 0,
                        width: previous_width,
                        height: previous_height,
                        mipmaps: 1,
                        format: raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
                    },
                });
            }
        }
        if built {
            std::mem::swap(&mut self.front, &mut self.back);
            self.carries = true;
            self.built = true;
        }
        built
    }

    /// The accumulated trail, premultiplied RGBA, `accumulate`'s dimensions.
    ///
    /// A bare ffi handle because the composite tints it through
    /// [`draw::draw_texture_pro`]; the allocation stays owned here.
    #[must_use]
    pub fn result(&self) -> Option<raylib_sys::Texture2D> {
        if !self.built {
            return None;
        }
        self.front.as_ref().map(|target| target.texture)
    }

    fn pass(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        width: i32,
        height: i32,
        carry: Carry,
        draw_fresh: impl FnOnce(&mut RaylibDrawHandle<'_>),
    ) -> bool {
        if !self.ensure(width, height) {
            return false;
        }
        let (Some(front), Some(back)) = (self.front.as_ref(), self.back.as_ref()) else {
            return false;
        };
        let previous = front.texture;
        // SAFETY: binds an id this struct owns, whose colour texture has the
        // dimensions just ensured. `BeginTextureMode` flushes the batch before
        // switching, and `accumulate` restores the caller's target on every
        // exit path after this point.
        unsafe { raylib_sys::BeginTextureMode(**back) };
        // Transparent, not opaque black: the buffer is a coverage-carrying
        // overlay, and an opaque clear would make the composite paint a black
        // rectangle over the scene wherever no light has been.
        d.clear_background(Color::new(0, 0, 0, 0));
        if self.carries && carry.retain > 0.0 {
            // A render texture's rows are stored bottom-up, so every draw *of*
            // one takes a negative source height.
            let source = Rectangle::new(0.0, 0.0, width as f32, -(height as f32));
            let zoom = carry.zoom.max(0.0);
            let dest = Rectangle::new(
                carry.centre.x,
                carry.centre.y,
                width as f32 * zoom,
                height as f32 * zoom,
            );
            // `draw_texture_pro` puts the *origin* point of the scaled source at
            // `dest`'s position and rotates about it, so scaling the pivot by
            // the same factor is what keeps `centre` a fixed point.
            let origin = Vector2::new(carry.centre.x * zoom, carry.centre.y * zoom);
            let fade = (carry.retain.clamp(0.0, 1.0) * 255.0).round() as u8;
            let mut pass = d.begin_blend_mode(BlendMode::BLEND_ALPHA_PREMULTIPLY);
            draw::draw_texture_pro(
                &mut pass,
                previous,
                source,
                dest,
                origin,
                carry.rotation_degrees,
                // Premultiplied content scales by one factor on all four
                // channels; anything else would break the invariant the
                // composite reads.
                Color::new(fade, fade, fade, fade),
            );
        }
        draw_fresh(d);
        true
    }

    /// Allocates the pair, or re-allocates it when the boundary changed.
    ///
    /// Exact match rather than grow-and-crop: the buffer's texel grid *is* the
    /// composite's, and a resize discards the accumulated light anyway, which is
    /// the honest answer — a trail carried across a window resize would be a
    /// picture of the old geometry stretched over the new one.
    fn ensure(&mut self, width: i32, height: i32) -> bool {
        if self.width == width && self.height == height && self.front.is_some() {
            return true;
        }
        self.front = None;
        self.back = None;
        self.carries = false;
        // SAFETY: `LoadRenderTexture` takes its dimensions by value and returns
        // an id-carrying struct; a zero id is its failure signal and is checked
        // before the wrapper — whose `Drop` is `UnloadRenderTexture` — takes
        // ownership. It rebinds the framebuffer as a side effect, which is why
        // `accumulate` captures the caller's binding before this runs and
        // restores it on every exit path. The filter and wrap calls take the
        // colour texture by value: bilinear because the zoom resamples between
        // texels every frame and a nearest-neighbour trail crawls in visible
        // steps, clamp because a zoom below 1 samples outside the buffer and
        // wrap-around would pull the opposite edge's light into the frame.
        unsafe {
            let front = raylib_sys::LoadRenderTexture(width, height);
            if front.id == 0 {
                self.refused = true;
                return false;
            }
            let back = raylib_sys::LoadRenderTexture(width, height);
            if back.id == 0 {
                raylib_sys::UnloadRenderTexture(front);
                self.refused = true;
                return false;
            }
            for target in [&front, &back] {
                raylib_sys::SetTextureFilter(
                    target.texture,
                    TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
                );
                raylib_sys::SetTextureWrap(target.texture, TextureWrap::TEXTURE_WRAP_CLAMP as i32);
            }
            self.front = Some(RenderTexture2D::from_raw(front));
            self.back = Some(RenderTexture2D::from_raw(back));
        }
        self.width = width;
        self.height = height;
        true
    }
}

/// The retention factor for a half-life, over a delta. Time-based by
/// construction: this is the one place the exponent lives, so a caller cannot
/// accidentally write a per-frame constant.
///
/// A non-positive or non-finite half-life means "keep nothing", which is the
/// safe answer — a trail that never fades is a smear that never clears.
#[must_use]
pub fn retention(half_life_seconds: f32, delta_seconds: f32) -> f32 {
    if !half_life_seconds.is_finite()
        || half_life_seconds <= 0.0
        || !delta_seconds.is_finite()
        || delta_seconds <= 0.0
    {
        return 0.0;
    }
    0.5f32.powf(delta_seconds / half_life_seconds)
}

/// A per-second rate turned into this frame's alpha, clamped to `0..=1`.
///
/// The companion to [`retention`]: a deposit that is *not* scaled by the delta
/// makes a 30 fps export accumulate half as much light per second as a 60 fps
/// preview of the same music, which is the export-versus-preview drift this
/// scene has already been bitten by once.
#[must_use]
pub fn deposit(rate_per_second: f32, delta_seconds: f32) -> f32 {
    if !rate_per_second.is_finite() || !delta_seconds.is_finite() || delta_seconds <= 0.0 {
        return 0.0;
    }
    (rate_per_second * delta_seconds).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defining property: half the light is gone after one half-life,
    /// whatever the frame rate that got there.
    #[test]
    fn retention_is_a_half_life_not_a_frame_count() {
        assert!((retention(0.35, 0.35) - 0.5).abs() < 1.0e-6);
        // Two 30 fps steps must equal one 15 fps step, or preview and export
        // disagree about how far a streak reaches.
        let fast = retention(0.35, 1.0 / 60.0);
        let slow = retention(0.35, 1.0 / 30.0);
        assert!((fast * fast - slow).abs() < 1.0e-6, "{fast} {slow}");
    }

    #[test]
    fn retention_refuses_degenerate_input() {
        for (half_life, delta) in [(0.0, 0.016), (-1.0, 0.016), (f32::NAN, 0.016), (0.35, 0.0)] {
            assert_eq!(retention(half_life, delta), 0.0, "{half_life} {delta}");
        }
    }

    /// Equilibrium under `A <- A * retain + (1 - A * retain) * deposit` must
    /// land in the same place at 30 and 60 fps — that equality is the whole
    /// argument for scaling the deposit by the delta.
    #[test]
    fn equilibrium_is_frame_rate_independent() {
        let settle = |delta: f32| {
            let retain = retention(0.35, delta);
            let alpha = deposit(6.0, delta);
            let mut level = 0.0f32;
            for _ in 0..600 {
                let carried = level * retain;
                level = carried + (1.0 - carried) * alpha;
            }
            level
        };
        let at60 = settle(1.0 / 60.0);
        let at30 = settle(1.0 / 30.0);
        assert!((at60 - at30).abs() < 0.05, "{at60} vs {at30}");
    }

    #[test]
    fn deposit_is_bounded_and_refuses_degenerate_input() {
        assert!((deposit(6.0, 1.0 / 60.0) - 0.1).abs() < 1.0e-6);
        assert_eq!(deposit(6.0, 1.0), 1.0);
        assert_eq!(deposit(f32::NAN, 0.016), 0.0);
        assert_eq!(deposit(6.0, 0.0), 0.0);
    }

    /// A fresh buffer has nothing to hand out, and a reset takes it back.
    /// Neither path touches the GPU, which is what makes them testable at all.
    #[test]
    fn an_unaccumulated_buffer_offers_no_result() {
        let mut buffer = FeedbackBuffer::new();
        assert!(buffer.result().is_none());
        assert!(!buffer.refused());
        buffer.reset();
        assert!(buffer.result().is_none());
    }
}
