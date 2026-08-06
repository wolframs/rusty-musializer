//! The offscreen Gaussian blur behind the caption glow halo (UX0-C11).
//!
//! The old glow was 17 additive re-draws of the whole caption offset in two
//! rings, and it had the failure inherent to finite taps: past a modest radius
//! the "halo" resolves into seventeen visible copies of the text. A real halo
//! is a blur, and a blur needs an offscreen buffer, so this module renders the
//! glyph coverage into a render texture, blurs it with a two-pass separable
//! Gaussian (`resources/shaders/glsl330/halo_blur.fs`, embedded), and hands the
//! result back for the caption renderer to composite — additively for the
//! glow, or as coverage alpha through `halo_mask.fs` for the soft shadow,
//! which has the same discrete-copy pathology in its tap table and rides the
//! same blur (operator-approved follow-up, 2026-08-04).
//!
//! ## Why this drives `BeginTextureMode` by hand
//!
//! raylib texture modes do not nest: `EndTextureMode` always returns to the
//! *default* framebuffer (`rcore.c:1110-1131`), never to whatever was bound
//! before. The caption overlay draws both to the screen (preview) and inside
//! the export's supersampled render target — from the same call site — so a
//! safe `begin_texture_mode` guard here would end the export's texture mode
//! behind its back and quietly redirect the rest of the frame to the screen.
//! Instead this module captures the active framebuffer with
//! `rlGetActiveFramebuffer` / `rlGetFramebufferWidth` / `rlGetFramebufferHeight`
//! (the same triple `draw::SceneViewport` already trusts), runs its passes
//! through raw `BeginTextureMode` calls — each of which flushes the batch and
//! rebinds everything it touches — and restores the caller's framebuffer on
//! every exit path: `EndTextureMode` when the caller was the screen, or a
//! reconstructed `BeginTextureMode` when the caller was a render target, which
//! reads only the id and the colour texture's dimensions (`rcore.c:1079-1108`).
//!
//! ## `EndTextureMode` is not the inverse of `BeginTextureMode`
//!
//! `BeginTextureMode` moves **three** size authorities to the target
//! (`rcore.c:1079-1107`): the GL viewport, rlgl's cached
//! `framebufferWidth`/`framebufferHeight` pair, and `CORE.Window.currentFbo`.
//! `EndTextureMode` (`rcore.c:1109-1131`) restores only two of them — it calls
//! `SetupViewport(CORE.Window.render.*)` (`rcore.c:3537`), which rewrites the
//! viewport and the projection, and it resets `currentFbo` by hand, but it never
//! touches rlgl's pair. Nothing later in a frame does either: `BeginDrawing`
//! does not set it, and only a window resize does (`rcore_desktop_glfw.c:1762`).
//!
//! So a blur that ends with a bare `EndTextureMode` leaves rlgl reporting the
//! *blur buffer's* dimensions for the rest of the session, and the readers of
//! that pair — `draw::SceneViewport`, and this module's own capture — then scale
//! a panel boundary by a ratio like 0.15. The observed damage was a scene panel
//! that drew nothing (the narrowed rect rounded below one pixel) or an entire
//! interface squeezed into the window's bottom-left corner (the restored
//! viewport pinned to the small rect, at GL's bottom-left origin), persisting
//! across every later frame. The restore below therefore puts the pair back
//! explicitly; the render-target path needs no such thing, because the
//! reconstructed `BeginTextureMode` sets all three authorities itself.
//!
//! ## Determinism
//!
//! The buffers are cleared and fully redrawn on every build, and every
//! dimension is derived from the caption's resolved font size and radius in
//! pixels — never from window constants — so the same project and frame
//! produce the same pixels, in preview and under export supersampling alike.
//!
//! ## The blur stays in luminance, not alpha
//!
//! The glyph buffer is cleared to opaque black and the text is drawn white, so
//! coverage lives in RGB with alpha pinned at 1 throughout. Blurring an
//! RGBA-over-transparent buffer through raylib's non-premultiplied `BLEND_ALPHA`
//! squares the coverage (both the colour and alpha factors are `srcA`), which
//! steepens the falloff and fringes the edge — the classic defect. A luminance
//! buffer has no alpha channel for that defect to live in, and the additive
//! composite (`dst += luma * tint.rgb * tint.a`) never needs one.

use raylib::consts::{TextureFilter, TextureWrap};
use raylib::math::Vector2;
use raylib::prelude::{
    Color, RaylibDraw, RaylibDrawHandle, RaylibShader, RaylibShaderModeExt, Rectangle,
};
use raylib::shaders::Shader;
use raylib::{RaylibHandle, RaylibThread};

use crate::draw;

/// The largest Gaussian sigma, in buffer texels, the shader's 17 unit-spaced
/// taps cover without a visible truncation edge (3.2 sigma). Callers downsample
/// their buffer by `ceil(sigma_px / MAX_SIGMA_TEXELS)` so the on-screen sigma
/// can grow without the kernel growing — a quarter-resolution halo upsampled
/// bilinearly is indistinguishable from a full-resolution one, because a halo
/// has no high frequencies by construction.
pub const MAX_SIGMA_TEXELS: f32 = 2.5;

/// Refusing absurd buffer requests beats letting a corrupt radius allocate one.
const MAX_BUFFER_EDGE: i32 = 8192;

struct BlurShader {
    shader: Shader,
    direction_location: i32,
    sigma_location: i32,
}

struct Buffers {
    /// Pass 1 target (glyph coverage) and pass 3 target (the finished halo).
    glyphs: raylib::texture::RenderTexture2D,
    /// Pass 2 target (horizontally blurred coverage).
    scratch: raylib::texture::RenderTexture2D,
    width: i32,
    height: i32,
}

/// Owns the blur shader and the two ping-pong buffers, living alongside the
/// other GPU resources in the scene renderer: loaded once, resized only when a
/// caption's region changes, never leaked per frame.
pub struct HaloBlur {
    shader: Option<BlurShader>,
    refusal: Option<&'static str>,
    /// The luminance-as-alpha composite for the soft shadow. Separate from the
    /// blur shader because the buffer serves two composites: the glow adds the
    /// luminance directly, the shadow needs it converted into coverage alpha
    /// under normal blending — an opaque black buffer drawn dark with normal
    /// blending would paint the whole rectangle.
    mask: Option<Shader>,
    mask_refusal: Option<&'static str>,
    /// At most two cached buffer pairs, matched by exact dimensions — one for
    /// the glow's sigma class and one for the soft shadow's, which commonly
    /// differ within a single frame. A single pair would be recreated twice
    /// per frame whenever both effects are authored; two is the same policy as
    /// the two cached caption atlases, and for the same reason.
    buffers: Vec<Buffers>,
    /// Index of the pair the last [`HaloBlur::render`] built into, which is
    /// the one [`HaloBlur::result`] and [`HaloBlur::composite_masked`] read.
    current: usize,
}

impl HaloBlur {
    /// GLSL 330 only, like every other first-party shader: the oracle hard-codes
    /// `GLSL_VERSION` to 330 and nothing here targets GLES.
    const SOURCE: &'static str = include_str!("../../../resources/shaders/glsl330/halo_blur.fs");
    const MASK_SOURCE: &'static str =
        include_str!("../../../resources/shaders/glsl330/halo_mask.fs");

    /// Compiles the blur shader. A refusal is a supported state — the caption
    /// then draws no halo and [`HaloBlur::describe`] says why — because a glow
    /// is decoration and a startup abort over decoration is the wrong trade.
    #[must_use]
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        // raylib answers a failed compile with the default shader rather than an
        // error, so validity is a question about the id, not a `Result`.
        let mask_shader = rl.load_shader_from_memory(thread, None, Some(Self::MASK_SOURCE));
        let (mask, mask_refusal) = if mask_shader.is_shader_valid() {
            (Some(mask_shader), None)
        } else {
            (
                None,
                Some("the driver refused the GLSL 330 halo mask shader"),
            )
        };
        let shader = rl.load_shader_from_memory(thread, None, Some(Self::SOURCE));
        if !shader.is_shader_valid() {
            return Self {
                shader: None,
                refusal: Some("the driver refused the GLSL 330 halo blur shader"),
                mask,
                mask_refusal,
                buffers: Vec::new(),
                current: 0,
            };
        }
        let direction_location = shader.get_shader_location("direction");
        let sigma_location = shader.get_shader_location("sigmaTexels");
        if direction_location < 0 || sigma_location < 0 {
            return Self {
                shader: None,
                refusal: Some("the halo blur shader compiled but its uniforms are missing"),
                mask,
                mask_refusal,
                buffers: Vec::new(),
                current: 0,
            };
        }
        Self {
            shader: Some(BlurShader {
                shader,
                direction_location,
                sigma_location,
            }),
            refusal: None,
            mask,
            mask_refusal,
            buffers: Vec::new(),
            current: 0,
        }
    }

    /// Whether a halo can be built at all. `false` means the shader was
    /// refused; the caption reports the fallback rather than pretending.
    #[must_use]
    pub fn available(&self) -> bool {
        self.shader.is_some()
    }

    /// One phrase for the report line: the mechanism when it works, the reason
    /// when it cannot. A refused *mask* shader is named too — the glow still
    /// works then, but the soft shadow has fallen back to its hard copy, and
    /// the two frames are otherwise plausible pictures of each other.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        if let Some(reason) = self.refusal {
            return reason;
        }
        if let Some(reason) = self.mask_refusal {
            return reason;
        }
        "rt-blur"
    }

    /// Builds the blurred halo: renders `draw_source` into a `width`x`height`
    /// buffer, Gaussian-blurs it with `sigma_texels`, and leaves the result
    /// readable through [`HaloBlur::result`]. Returns whether a halo was built.
    ///
    /// Must be called inside an active drawing context (the handle is the
    /// proof), with **no scissor active**: a scissor rect is global GL state in
    /// framebuffer coordinates and would clip the offscreen passes with a
    /// rectangle meant for the screen. Safe whether the caller is drawing to
    /// the default framebuffer or inside a texture mode — the caller's
    /// framebuffer is restored on every exit path after the first GL call.
    pub fn render(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        width: i32,
        height: i32,
        sigma_texels: f32,
        draw_source: impl FnOnce(&mut RaylibDrawHandle<'_>),
    ) -> bool {
        if self.shader.is_none()
            || width < 1
            || height < 1
            || width > MAX_BUFFER_EDGE
            || height > MAX_BUFFER_EDGE
            || !sigma_texels.is_finite()
            || sigma_texels <= 0.0
        {
            return false;
        }
        // SAFETY: pure rlgl getters over global renderer state, valid while a
        // drawing context is active, which the handle proves. The batch is
        // flushed first so geometry already queued for the caller's framebuffer
        // is rasterized there and not under a later binding.
        let (previous_fbo, previous_width, previous_height) = unsafe {
            raylib_sys::rlDrawRenderBatchActive();
            (
                raylib_sys::rlGetActiveFramebuffer(),
                raylib_sys::rlGetFramebufferWidth(),
                raylib_sys::rlGetFramebufferHeight(),
            )
        };
        let built = self.render_passes(d, width, height, sigma_texels, draw_source);
        // SAFETY: restores the framebuffer captured above, unconditionally —
        // even a failed buffer allocation has already unbound the caller's
        // framebuffer, because `rlLoadFramebuffer` binds 0 as a side effect.
        // `EndTextureMode` restores the binding, viewport, projection and
        // `currentFbo` when the caller was the default framebuffer, but *not*
        // rlgl's cached framebuffer size — see the module note above; leaving it
        // at the blur buffer's dimensions corrupts every later frame's scene
        // viewport, which is what the two size calls after it repair. When the
        // caller was a render target, `BeginTextureMode` reads only `id` and the
        // colour texture's dimensions (`rcore.c:1079-1108`), so a reconstructed
        // handle with those three fields restores the binding, viewport,
        // projection, the size pair and raylib's `currentFbo` bookkeeping (which
        // scissor Y-flips depend on) exactly.
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
        built
    }

    /// The finished halo: white-on-black luminance, `render`'s dimensions.
    /// A bare ffi handle because the composite tints it through
    /// [`draw::draw_texture_pro`]; the allocation stays owned here.
    #[must_use]
    pub fn result(&self) -> Option<raylib_sys::Texture2D> {
        self.buffers
            .get(self.current)
            .map(|buffers| buffers.glyphs.texture)
    }

    /// Composites the built halo as a *mask* under normal blending: the
    /// buffer's luminance becomes the drawn alpha and the tint's colour the
    /// paint (`halo_mask.fs`), which is what the soft shadow needs — a shadow
    /// is an occlusion, not a light, so adding it like the glow would brighten
    /// the very pixels it should darken. Returns whether it drew; `false`
    /// means the mask shader was refused (named by [`HaloBlur::describe`]) or
    /// nothing has been built, and the caller falls back visibly.
    pub fn composite_masked<D: RaylibDraw>(
        &mut self,
        d: &mut D,
        dest: Rectangle,
        tint: Color,
    ) -> bool {
        let (Some(mask), Some(buffers)) = (self.mask.as_mut(), self.buffers.get(self.current))
        else {
            return false;
        };
        let texture = buffers.glyphs.texture;
        let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
        let mut pass = d.begin_shader_mode(mask);
        draw::draw_texture_pro(&mut pass, texture, source, dest, Vector2::zero(), 0.0, tint);
        true
    }

    fn render_passes(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        width: i32,
        height: i32,
        sigma_texels: f32,
        draw_source: impl FnOnce(&mut RaylibDrawHandle<'_>),
    ) -> bool {
        if !self.ensure_buffers(width, height) {
            return false;
        }
        let current = self.current;
        let (Some(buffers), Some(blur)) = (self.buffers.get_mut(current), self.shader.as_mut())
        else {
            return false;
        };
        let full = Rectangle::new(0.0, 0.0, width as f32, height as f32);
        // A render texture's rows are stored bottom-up, so every draw *of* one
        // uses a negative source height. Applied at each of the two reads and
        // again by the caller's composite, the content stays in its authored
        // orientation the whole way — and a caption's halo is not vertically
        // symmetric, so a missed flip here is visible over any descender.
        let flipped = Rectangle::new(0.0, 0.0, width as f32, -(height as f32));

        // Pass 1: glyph coverage, white on opaque black.
        // SAFETY: binds an id this struct owns and whose colour texture has the
        // dimensions just ensured; `BeginTextureMode` flushes the batch before
        // switching, and the caller of `render` restores the previous target.
        unsafe { raylib_sys::BeginTextureMode(*buffers.glyphs) };
        d.clear_background(Color::BLACK);
        draw_source(d);

        // Pass 2: horizontal blur into the scratch buffer.
        blur.shader.set_shader_value(
            blur.direction_location,
            Vector2::new(1.0 / width as f32, 0.0),
        );
        blur.shader
            .set_shader_value(blur.sigma_location, sigma_texels);
        // SAFETY: as pass 1; the previous pass's pending quads are flushed into
        // the glyph buffer by this call before the target changes.
        unsafe { raylib_sys::BeginTextureMode(*buffers.scratch) };
        d.clear_background(Color::BLACK);
        {
            let mut pass = d.begin_shader_mode(&mut blur.shader);
            draw::draw_texture_pro(
                &mut pass,
                buffers.glyphs.texture,
                flipped,
                full,
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
        }

        // Pass 3: vertical blur back into the glyph buffer, now the result.
        blur.shader.set_shader_value(
            blur.direction_location,
            Vector2::new(0.0, 1.0 / height as f32),
        );
        // SAFETY: as pass 2.
        unsafe { raylib_sys::BeginTextureMode(*buffers.glyphs) };
        d.clear_background(Color::BLACK);
        {
            let mut pass = d.begin_shader_mode(&mut blur.shader);
            draw::draw_texture_pro(
                &mut pass,
                buffers.scratch.texture,
                flipped,
                full,
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
        }
        true
    }

    /// Selects a cached buffer pair of exactly `width`x`height`, creating one
    /// (and evicting the oldest beyond two) when none matches. Exact match
    /// rather than grow-only, because the dimensions define the composite's
    /// texel grid: a buffer larger than the request would need sub-rect
    /// bookkeeping in three places, and a caption's sigma classes change a
    /// handful of times per session, not per frame.
    fn ensure_buffers(&mut self, width: i32, height: i32) -> bool {
        if let Some(index) = self
            .buffers
            .iter()
            .position(|buffers| buffers.width == width && buffers.height == height)
        {
            self.current = index;
            return true;
        }
        // Evicts the oldest pair first (their `Drop` is `UnloadRenderTexture`);
        // the batch was flushed before `render_passes` was entered.
        while self.buffers.len() >= 2 {
            self.buffers.remove(0);
        }
        // SAFETY: `LoadRenderTexture` takes its dimensions by value and returns
        // an id-carrying struct; a zero id is its failure signal and is checked
        // before the wrapper — whose `Drop` is `UnloadRenderTexture` — takes
        // ownership. It rebinds the framebuffer as a side effect, which is why
        // `render` captures the caller's binding before this runs and restores
        // it on every exit path. The filter and wrap calls receive the colour
        // texture by value: bilinear because both blur passes and the final
        // composite sample between texels, clamp because the blur kernel reads
        // up to 8 texels past the edge and wrap-around would pull the opposite
        // edge's glyphs into the halo.
        unsafe {
            let glyphs = raylib_sys::LoadRenderTexture(width, height);
            if glyphs.id == 0 {
                return false;
            }
            let scratch = raylib_sys::LoadRenderTexture(width, height);
            if scratch.id == 0 {
                raylib_sys::UnloadRenderTexture(glyphs);
                return false;
            }
            for target in [&glyphs, &scratch] {
                raylib_sys::SetTextureFilter(
                    target.texture,
                    TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
                );
                raylib_sys::SetTextureWrap(target.texture, TextureWrap::TEXTURE_WRAP_CLAMP as i32);
            }
            self.buffers.push(Buffers {
                glyphs: raylib::texture::RenderTexture2D::from_raw(glyphs),
                scratch: raylib::texture::RenderTexture2D::from_raw(scratch),
                width,
                height,
            });
        }
        self.current = self.buffers.len() - 1;
        true
    }
}
