//! The export's 16-bit render target (EX3).
//!
//! ## Why
//!
//! A slow dark gradient in an exported video showed contour steps, and they were
//! already in the frame handed to FFmpeg: `LoadRenderTexture` is RGBA8, so every
//! draw into the export target was rounded to 8 bits as it landed. Measured on
//! stills of the shader scenes (aurora, tunnel, iridescent, glyphs), 93–98 % of
//! the steps inside smooth dark regions are exactly one code: the precision is
//! lost at the write, not amplified by some intermediate pass, so the fix is a
//! deeper write plus a dither when the frame is finally quantized to 8 bits
//! (`musializer_core::render::resolve::LinearResolver::resolve_deep`).
//!
//! ## Why `GL_RGBA16` and not `GL_RGBA16F`
//!
//! The queue proposed half floats, which raylib can allocate itself. They are
//! the wrong format here: a float target does not clamp, so additive glow piles
//! up past 1.0 and a later alpha blend or a finish pass that reads the frame
//! sees light the 8-bit preview never had. `GL_RGBA16` is unsigned-normalized
//! like RGBA8 — every blend clamps to `0..=1` exactly as it does in the preview
//! — and only the precision changes. raylib has no pixel format for it, so the
//! colour texture is allocated by raylib as RGBA8 and its storage re-specified
//! here with one `glTexImage2D`; the framebuffer around it is raylib's own
//! (`rlLoadFramebuffer`, `rlFramebufferAttach`, `rlFramebufferComplete`).
//! `GL_RGBA16` is a required colour-renderable format in desktop GL 3.0+, so
//! NVIDIA and llvmpipe both have it; a driver that refuses it anyway is caught
//! by the completeness check and the export falls back to RGBA8, saying so.
//!
//! ## Reading it back
//!
//! As `GL_UNSIGNED_SHORT`, exactly, rather than through raylib's half-float
//! path: a colour the 8-bit render would have produced is stored as
//! `code * 257` and has to come back as exactly that, so the resolve can pass
//! it through untouched and a flat fill stays flat.
//!
//! The texture's `format` field says `R16G16B16A16`, raylib's nearest label.
//! Nothing here reads it, and raylib's own readers would request half floats,
//! which GL converts correctly — never wrongly — from unorm storage.

use std::ffi::{c_char, c_void};

use raylib::prelude::RenderTexture2D;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA: u32 = 0x1908;
const GL_RGBA16: i32 = 0x805B;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_NO_ERROR: u32 = 0;

type TexImage2d = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type GetTexImage = unsafe extern "C" fn(u32, i32, u32, u32, *mut c_void);
type GetError = unsafe extern "C" fn() -> u32;
type GetFramebufferAttachmentParameteriv = unsafe extern "C" fn(u32, u32, u32, *mut i32);

const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE: u32 = 0x8212;

unsafe extern "C" {
    /// GLFW's loader for the current context, linked from raylib's bundled
    /// GLFW (`rglfw.c`). Returns null for a name the context does not have.
    fn glfwGetProcAddress(procname: *const c_char) -> *const c_void;
}

/// The longest edge a deep target is attempted at: an 8K Master frame's 2x
/// target is 15360 wide, which no allocation here should try.
const MAX_EDGE: i32 = 8192;

/// Why a deep target was not made. Every one falls back to RGBA8, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeepRefusal {
    #[error("the GL context has no {0}")]
    MissingEntryPoint(&'static str),
    #[error("{0}x{1} is past the deep target's {MAX_EDGE} px edge")]
    TooLarge(i32, i32),
    #[error("no framebuffer object could be created")]
    Framebuffer,
    #[error("the driver refused 16-bit storage (GL error 0x{0:04x})")]
    Storage(u32),
    #[error("the driver reports the 16-bit framebuffer incomplete")]
    Incomplete,
    #[error("disabled by MUSIALIZER_EXPORT_DEPTH=8")]
    Disabled,
}

/// Why a deep target could not be read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("the GL context has no {0}")]
    MissingEntryPoint(&'static str),
    #[error("the readback failed (GL error 0x{0:04x})")]
    Gl(u32),
}

/// Looks a GL entry point up by name in the current context.
fn entry_point(name: &'static [u8]) -> Option<*const c_void> {
    debug_assert_eq!(name.last(), Some(&0), "a C string");
    // SAFETY: `name` is a NUL-terminated static byte string (asserted above in
    // debug builds, and every caller passes a `b"...\0"` literal). GLFW reads
    // it and returns a function pointer or null; it requires a current
    // context, which every caller has because it runs between `InitWindow`
    // and `CloseWindow` on the main thread.
    let pointer = unsafe { glfwGetProcAddress(name.as_ptr().cast()) };
    (!pointer.is_null()).then_some(pointer)
}

/// Drains GL's error flags, so the next `glGetError` names our call's.
fn clear_errors(get_error: GetError) {
    for _ in 0..16 {
        // SAFETY: `glGetError` takes nothing and returns a code; the pointer
        // came from the current context's loader.
        if unsafe { get_error() } == GL_NO_ERROR {
            return;
        }
    }
}

/// Whether `MUSIALIZER_EXPORT_DEPTH=8` asks for the old RGBA8 target — the
/// escape hatch for a driver that renders a 16-bit target wrongly, and the
/// before half of every before-and-after measurement.
#[must_use]
pub fn disabled_by_environment() -> bool {
    std::env::var_os("MUSIALIZER_EXPORT_DEPTH").is_some_and(|value| value == "8")
}

/// A render target whose colour attachment is `GL_RGBA16`.
///
/// # Errors
/// [`DeepRefusal`]; nothing is left allocated on any of them.
pub fn load(width: i32, height: i32) -> Result<RenderTexture2D, DeepRefusal> {
    if disabled_by_environment() {
        return Err(DeepRefusal::Disabled);
    }
    if width <= 0 || height <= 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(DeepRefusal::TooLarge(width, height));
    }
    let tex_image =
        entry_point(b"glTexImage2D\0").ok_or(DeepRefusal::MissingEntryPoint("glTexImage2D"))?;
    let get_error =
        entry_point(b"glGetError\0").ok_or(DeepRefusal::MissingEntryPoint("glGetError"))?;
    // SAFETY: both pointers came from the current context's loader for exactly
    // these names, and the types are the GL 1.0 signatures of those names
    // (`void glTexImage2D(GLenum, GLint, GLint, GLsizei, GLsizei, GLint,
    // GLenum, GLenum, const void*)`, `GLenum glGetError(void)`).
    let (tex_image, get_error) = unsafe {
        (
            std::mem::transmute::<*const c_void, TexImage2d>(tex_image),
            std::mem::transmute::<*const c_void, GetError>(get_error),
        )
    };

    // SAFETY: the same sequence as raylib's `LoadRenderTexture`
    // (`rtextures.c:4251-4287`), with three differences: the colour texture's
    // storage is re-specified as `GL_RGBA16` before it is attached, the depth
    // renderbuffer is attached first so a refusal in between cannot strand
    // it, and completeness is *checked* rather than logged. Every call takes ids and
    // dimensions by value and no pointer crosses except the null data pointer
    // `glTexImage2D` is documented to accept (it allocates without
    // uploading). The texture is bound only between `rlEnableTexture` and
    // `rlDisableTexture`; the framebuffer binding `rlLoadFramebuffer` and
    // `rlFramebufferComplete` leave is the default one, which is what a caller
    // outside a texture mode expects. Once the three ids exist they are
    // handed to `RenderTexture2D::from_raw`, whose `Drop` is
    // `UnloadRenderTexture` — so every failure after that point frees them by
    // dropping it.
    unsafe {
        let id = raylib_sys::rlLoadFramebuffer();
        if id == 0 {
            return Err(DeepRefusal::Framebuffer);
        }
        let colour = raylib_sys::rlLoadTexture(
            std::ptr::null(),
            width,
            height,
            raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
            1,
        );
        let depth = raylib_sys::rlLoadTextureDepth(width, height, true);
        // Attached before anything below can refuse: `rlUnloadFramebuffer`
        // finds the depth renderbuffer only through the framebuffer's
        // attachment, so one that was never attached would outlive a refused
        // target.
        if depth != 0 {
            raylib_sys::rlFramebufferAttach(
                id,
                depth,
                raylib_sys::rlFramebufferAttachType::RL_ATTACHMENT_DEPTH as i32,
                raylib_sys::rlFramebufferAttachTextureType::RL_ATTACHMENT_RENDERBUFFER as i32,
                0,
            );
        }
        let face = |id: u32, format: i32| raylib_sys::Texture2D {
            id,
            width,
            height,
            mipmaps: 1,
            format,
        };
        let target = RenderTexture2D::from_raw(raylib_sys::RenderTexture2D {
            id,
            texture: face(
                colour,
                raylib_sys::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R16G16B16A16 as i32,
            ),
            // raylib's own label for its depth renderbuffer (`rtextures.c:4272`).
            depth: face(depth, 19),
        });
        if colour == 0 || depth == 0 {
            return Err(DeepRefusal::Framebuffer);
        }

        clear_errors(get_error);
        raylib_sys::rlEnableTexture(colour);
        tex_image(
            GL_TEXTURE_2D,
            0,
            GL_RGBA16,
            width,
            height,
            0,
            GL_RGBA,
            GL_UNSIGNED_SHORT,
            std::ptr::null(),
        );
        let error = get_error();
        raylib_sys::rlDisableTexture();
        if error != GL_NO_ERROR {
            return Err(DeepRefusal::Storage(error));
        }

        raylib_sys::rlFramebufferAttach(
            id,
            colour,
            raylib_sys::rlFramebufferAttachType::RL_ATTACHMENT_COLOR_CHANNEL0 as i32,
            raylib_sys::rlFramebufferAttachTextureType::RL_ATTACHMENT_TEXTURE2D as i32,
            0,
        );
        if !raylib_sys::rlFramebufferComplete(id) {
            return Err(DeepRefusal::Incomplete);
        }
        Ok(target)
    }
}

/// Reads a target made by [`load`] back as RGBA16, bottom row first, into
/// `pixels` (resized to `width * height * 4`).
///
/// The caller must have ended the texture mode that drew into it — which
/// flushes raylib's batch, so the texture holds the whole frame.
///
/// # Errors
/// [`ReadError`]; `pixels` is then unspecified.
pub fn read(target: &RenderTexture2D, pixels: &mut Vec<u16>) -> Result<(), ReadError> {
    let get_tex_image =
        entry_point(b"glGetTexImage\0").ok_or(ReadError::MissingEntryPoint("glGetTexImage"))?;
    let get_error =
        entry_point(b"glGetError\0").ok_or(ReadError::MissingEntryPoint("glGetError"))?;
    // SAFETY: as in `load`: loader pointers for exactly these names, typed
    // with their GL signatures (`void glGetTexImage(GLenum, GLint, GLenum,
    // GLenum, void*)`).
    let (get_tex_image, get_error) = unsafe {
        (
            std::mem::transmute::<*const c_void, GetTexImage>(get_tex_image),
            std::mem::transmute::<*const c_void, GetError>(get_error),
        )
    };
    let width = target.texture.width.max(0) as usize;
    let height = target.texture.height.max(0) as usize;
    pixels.clear();
    pixels.resize(width * height * 4, 0);
    // SAFETY: `pixels` holds exactly `width * height * 4` `u16`s, which is
    // what `glGetTexImage` writes for level 0 of a `width` x `height` texture
    // in `GL_RGBA`/`GL_UNSIGNED_SHORT`: a row is `width * 8` bytes, a multiple
    // of 4, so neither the default pack alignment of 4 nor raylib's 1 adds
    // padding. The texture id is the target's own colour attachment, alive for
    // the borrow; it is bound only for the call and unbound after.
    // `rlDrawRenderBatchActive` first, so nothing queued is still on its way
    // into the texture.
    unsafe {
        raylib_sys::rlDrawRenderBatchActive();
        clear_errors(get_error);
        raylib_sys::rlEnableTexture(target.texture.id);
        get_tex_image(
            GL_TEXTURE_2D,
            0,
            GL_RGBA,
            GL_UNSIGNED_SHORT,
            pixels.as_mut_ptr().cast(),
        );
        let error = get_error();
        raylib_sys::rlDisableTexture();
        if error != GL_NO_ERROR {
            return Err(ReadError::Gl(error));
        }
    }
    Ok(())
}

/// Whether the **currently bound** framebuffer `fbo` stores more than 8 bits
/// of red — a target made by [`load`].
///
/// For the held copies a grade, a finish or a transition takes of the frame
/// (`transition::HeldFrame`): a copy into RGBA8 would round the deep frame
/// back to 8 bits halfway through, and every pass after it would carry the
/// contours the deep target exists to remove. The default framebuffer (`0`,
/// the preview's window) is always reported as not deep, without asking.
#[must_use]
pub fn is_deep_framebuffer(fbo: u32) -> bool {
    if fbo == 0 {
        return false;
    }
    let Some(query) = entry_point(b"glGetFramebufferAttachmentParameteriv\0") else {
        return false;
    };
    // SAFETY: a loader pointer for exactly this name, typed with its GL 3.0
    // signature (`void glGetFramebufferAttachmentParameteriv(GLenum, GLenum,
    // GLenum, GLint*)`).
    let query =
        unsafe { std::mem::transmute::<*const c_void, GetFramebufferAttachmentParameteriv>(query) };
    let mut red_bits = 0i32;
    // SAFETY: a state query on the framebuffer the caller has bound (every
    // caller passes the id rlgl reports as active), writing one `GLint` into
    // a live local. No binding changes.
    unsafe {
        query(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
            &mut red_bits,
        );
    }
    red_bits > 8
}
