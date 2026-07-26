//! The interface and caption faces.
//!
//! Port of `load_assets`' font half (`../musializer/src/plug.c:8060-8137`) and of
//! `ui_font` / `caption_face` (`plug.c:340-365`).
//!
//! Two things here are behaviour rather than decoration:
//!
//! - **The interface face is a codepoint subset of the caption face.** Space
//!   Grotesk is rasterized twice in the C: once restricted by `ui_font_codepoint`
//!   to the ranges the chrome actually draws, and once at the full curated caption
//!   set. Every requested codepoint costs atlas space whether or not the face has
//!   a glyph for it (`caption_layout.rs`'s `FONT_CODEPOINT_LIMIT`), so the subset
//!   is what keeps the interface atlas small.
//! - **The fallback is named, not silent.** A face that fails to rasterize falls
//!   back to raylib's default and says so on the trace log, because a UI that
//!   silently reverts to a 10 px bitmap face looks like a rendering bug rather
//!   than a missing asset.
//!
//! The bytes are embedded with `include_bytes!` rather than loaded from
//! `./resources/fonts/`, which is where the C reads them. That is a deliberate
//! divergence: the C's relative path is why its launcher has to `cd` into the
//! project root before exec, and an interface that loses its font because it was
//! started from the wrong directory is a failure mode worth deleting rather than
//! reproducing. The shaders are already embedded for the same reason.

use musializer_core::project::caption_layout;
use raylib::prelude::{RaylibFont, WeakFont};
use raylib::text::Font;
use raylib::{RaylibHandle, RaylibThread};

/// Rasterization size for every face (`FONT_SIZE`, `plug.c:104`).
///
/// One atlas per face at 64 px, scaled down at draw time with bilinear filtering
/// and mipmaps. Rasterizing per size would be sharper and would also mean an
/// atlas rebuild every time a row of buttons agreed on a new fitted size.
pub const FONT_SIZE: i32 = 64;

/// Space Grotesk Regular, SIL OFL 1.1. See `resources/fonts/SpaceGrotesk-OFL.txt`.
const SPACE_GROTESK: &[u8] = include_bytes!("../../../resources/fonts/SpaceGrotesk-Regular.otf");

/// Alegreya Regular, SIL OFL 1.1. See `resources/fonts/OFL.txt`.
const ALEGREYA: &[u8] = include_bytes!("../../../resources/fonts/Alegreya-Regular.ttf");

/// A face to draw with: either one this module rasterized, or raylib's default.
///
/// One concrete type rather than `Option<Font>` at every call site, because the
/// fallback has to be *drawable*, not absent. It is deliberately not a
/// [`Font`] holding `GetFontDefault()`: raylib's default face is a non-owning
/// handle into raylib's own static storage, and [`Font`]'s `Drop` calls
/// `UnloadFont`, which would free it out from under the rest of the frame. This
/// is the same hazard `draw::default_texture` carries a note about — hence
/// [`WeakFont`], whose drop is a no-op.
pub enum Face {
    Loaded(Font),
    Default(WeakFont),
}

impl Face {
    /// Whether this is a rasterized face rather than the fallback. Reported by
    /// [`Faces::describe`] so a capture carries the answer.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        matches!(self, Face::Loaded(_))
    }
}

impl AsRef<raylib_sys::Font> for Face {
    fn as_ref(&self) -> &raylib_sys::Font {
        match self {
            Face::Loaded(font) => font.as_ref(),
            Face::Default(font) => font.as_ref(),
        }
    }
}

impl AsMut<raylib_sys::Font> for Face {
    fn as_mut(&mut self) -> &mut raylib_sys::Font {
        match self {
            Face::Loaded(font) => font.as_mut(),
            Face::Default(font) => font.as_mut(),
        }
    }
}

impl RaylibFont for Face {}

/// Every face the application draws with.
///
/// Loaded once and borrowed for the rest of the run. A face loaded per frame — or
/// per scene switch — leaks a GPU atlas each time, which is the same mistake
/// `SceneRenderer` exists to prevent for shaders.
pub struct Faces {
    ui: Face,
    caption: Face,
}

impl Faces {
    /// Rasterizes the interface and caption faces, falling back to raylib's
    /// default for either one that will not load.
    ///
    /// Never fails: a missing face is a degraded interface, not a reason to
    /// refuse to start. `load_assets` in the C has the same shape.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let curated = caption_layout::font_codepoints().unwrap_or_default();

        // `ui_font_codepoint` (`plug.c:429-436`) applied to the curated set, which
        // is exactly the C's two-step: build the caption set, then filter it.
        // Filtering the curated set rather than expanding the ranges directly
        // matters — it is what keeps the interface face a strict subset of the
        // caption face, so a string that renders in one renders in the other.
        let ui_codepoints: Vec<i32> = curated
            .iter()
            .copied()
            .filter(|&codepoint| ui_codepoint(codepoint))
            .map(|codepoint| codepoint as i32)
            .collect();
        let caption_codepoints: Vec<i32> =
            curated.iter().copied().map(|point| point as i32).collect();

        let ui = rasterize(rl, thread, ".otf", SPACE_GROTESK, &ui_codepoints).map_or_else(
            || {
                // The C's own wording (`plug.c:8087-8088`).
                eprintln!("FONT: Space Grotesk UI face unavailable; using raylib default");
                default_face()
            },
            Face::Loaded,
        );
        let caption = rasterize(rl, thread, ".ttf", ALEGREYA, &caption_codepoints).map_or_else(
            || {
                eprintln!("FONT: Alegreya caption face unavailable; using raylib default");
                default_face()
            },
            Face::Loaded,
        );

        Self { ui, caption }
    }

    /// Every face is the fallback. The constructor a headless test can build
    /// without a GPU — and the state the application ends up in when the atlas
    /// build fails, so it is worth being able to name.
    #[must_use]
    pub fn fallback_only() -> Self {
        Self {
            ui: default_face(),
            caption: default_face(),
        }
    }

    /// The interface face (`ui_font`, `plug.c:340-344`).
    #[must_use]
    pub fn ui(&self) -> &Face {
        &self.ui
    }

    /// The caption face: Alegreya, at the full curated glyph set
    /// (`caption_face`'s default arm, `plug.c:361-364`).
    ///
    /// Deliberately not the interface face. The interface atlas carries only the
    /// codepoints the chrome needs, so typesetting a caption with it would drop
    /// Greek and Cyrillic without saying so.
    ///
    /// The C has a third face — Space Grotesk at the full caption set, for
    /// `MUSI_CAPTION_FACE_SPACE_GROTESK` — and a fourth for a project's imported
    /// face. Both are selected by caption style, which is not wired yet, and a
    /// 64 px atlas of 2,000 codepoints is not worth carrying for a selector
    /// nothing can reach.
    #[must_use]
    pub fn caption(&self) -> &Face {
        &self.caption
    }

    /// One line naming which faces are real, for the slice report.
    ///
    /// Evidence rather than assertion: "the font loaded" is the kind of claim a
    /// clean exit cannot support, and a fallback to the 10 px bitmap face is
    /// exactly the regression that would otherwise be noticed by eye weeks later.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "ui={}, caption={}",
            if self.ui.is_loaded() {
                "Space Grotesk"
            } else {
                "raylib default (FALLBACK)"
            },
            if self.caption.is_loaded() {
                "Alegreya"
            } else {
                "raylib default (FALLBACK)"
            },
        )
    }
}

/// `ui_font_codepoint` (`../musializer/src/plug.c:429-436`): the codepoint ranges
/// the interface itself draws.
///
/// Latin Extended-B and up covers the accented forms in a track name; the four
/// higher ranges are General Punctuation (U+2026, the ellipsis a truncated button
/// label ends in), currency symbols, letterlike symbols, and the four arrows.
#[must_use]
pub fn ui_codepoint(codepoint: u32) -> bool {
    (0x20..=0x024F).contains(&codepoint)
        || (0x2000..=0x206F).contains(&codepoint)
        || (0x20A0..=0x20CF).contains(&codepoint)
        || (0x2100..=0x214F).contains(&codepoint)
        || (0x2190..=0x2199).contains(&codepoint)
}

fn default_face() -> Face {
    // SAFETY: `GetFontDefault` returns raylib's built-in face, which lives in
    // raylib's static storage for the lifetime of the process and is valid as
    // soon as the window exists. `WeakFont`'s destructor is a no-op, so wrapping
    // it cannot unload a face raylib still owns — which is the whole reason this
    // is a `WeakFont` and not a `Font`.
    Face::Default(unsafe { WeakFont::from_raw(raylib_sys::GetFontDefault()) })
}

/// One face, or `None` if raylib could not build an atlas from these bytes.
///
/// raylib-rs's safe `load_font_from_memory` cannot be used here: it takes the
/// glyph set as a `&str` and then passes `str::len()` — a *byte* count — as the
/// codepoint count. For an ASCII string that happens to be right; for the curated
/// set, which is mostly multi-byte, it would tell raylib to read several times as
/// many `i32`s as the array holds. So the codepoints go through the ffi directly.
fn rasterize(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    file_type: &str,
    bytes: &[u8],
    codepoints: &[i32],
) -> Option<Font> {
    let _ = (rl, thread); // Proof a window exists; raylib needs one for the atlas.
    let c_file_type = std::ffi::CString::new(file_type).ok()?;

    // SAFETY: `LoadFontFromMemory` reads `bytes.len()` bytes from `bytes` and
    // `codepoints.len()` `i32`s from `codepoints`, and both lengths are passed
    // from the slices themselves, so neither read can run past its allocation.
    // `bytes` is a `'static` `include_bytes!` array and `codepoints` outlives the
    // call. Passing a null glyph array with a zero count is raylib's documented
    // "use the default 95-glyph set" request, which is the right answer when the
    // curated table is unavailable. The returned struct owns GPU and heap
    // allocations, which is why it is immediately handed to `Font::from_raw`
    // whose `Drop` is `UnloadFont`. A window must exist for the atlas upload;
    // `rl` is the proof of that.
    let raw = unsafe {
        raylib_sys::LoadFontFromMemory(
            c_file_type.as_ptr(),
            bytes.as_ptr(),
            bytes.len() as i32,
            FONT_SIZE,
            if codepoints.is_empty() {
                std::ptr::null_mut()
            } else {
                codepoints.as_ptr().cast_mut()
            },
            codepoints.len() as i32,
        )
    };
    if raw.glyphs.is_null() || raw.texture.id == 0 {
        return None;
    }

    // SAFETY: `raw` came from `LoadFontFromMemory` above and has not been
    // unloaded or copied elsewhere, so this transfers sole ownership.
    let mut font = unsafe { Font::from_raw(raw) };

    // Mipmaps and bilinear filtering, because every face is rasterized once at
    // 64 px and drawn at 13-38 px (`plug.c:8085-8086`). Without them the
    // downscale aliases badly enough that small labels look broken rather than
    // small.
    //
    // The order and the `&mut` are both load-bearing. `GenTextureMipmaps` writes
    // the new level count back through its pointer, and `SetTextureFilter` reads
    // `texture.mipmaps` to decide between `LINEAR` and `LINEAR_MIP_NEAREST`
    // (`vendor/raylib-5.5/src/rtextures.c:4380-4397`). Generating mipmaps on a
    // *copy* of the texture struct would upload them and then filter as though
    // they did not exist — the levels would sit in VRAM unused, which is the kind
    // of bug that reads as "mipmaps do nothing here".
    //
    // SAFETY: `font` owns this texture and is alive across both calls.
    // `GenTextureMipmaps` writes only through the pointer it is given, which
    // borrows the font's own texture field; `SetTextureFilter` takes the texture
    // by value and mutates raylib-side GL state only.
    unsafe {
        // Named, because `Font` implements `AsRef` for both the ffi font and its
        // texture and the inferred one would be a coin flip.
        let raw_font: &mut raylib_sys::Font = font.as_mut();
        raylib_sys::GenTextureMipmaps(&mut raw_font.texture);
        raylib_sys::SetTextureFilter(
            raw_font.texture,
            raylib_sys::TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
        );
    }
    Some(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_set_is_a_strict_subset_of_the_caption_set() {
        // The property the two-step filter buys, as a test: anything the chrome
        // can draw, a caption can draw too. If these ever diverge, a string that
        // renders in a button would render as missing-glyph boxes in a caption.
        let curated = caption_layout::font_codepoints().expect("the curated table is consistent");
        let ui: Vec<u32> = curated
            .iter()
            .copied()
            .filter(|&point| ui_codepoint(point))
            .collect();
        assert!(!ui.is_empty(), "the interface subset is empty");
        assert!(ui.len() < curated.len(), "the subset is the whole set");
        let caption: std::collections::HashSet<u32> = curated.into_iter().collect();
        for point in &ui {
            assert!(
                caption.contains(point),
                "U+{point:04X} is not in the caption set"
            );
        }
    }

    #[test]
    fn the_interface_set_carries_the_glyphs_the_chrome_actually_draws() {
        // Not an arbitrary sample. U+2026 is the ellipsis `truncate_label`
        // appends, and it is the reason the shell used to substitute "..." — the
        // default face has no glyph for it and drew a box. U+00B7 is the C's
        // "not ported" badge. A regression in the ranges shows up here rather
        // than in a screenshot.
        for point in [0x20u32, 0x41, 0xB7, 0xE9, 0x2026, 0x20AC, 0x2192] {
            assert!(
                ui_codepoint(point),
                "U+{point:04X} dropped from the interface set"
            );
        }
        // Outside every range: CJK, which the interface never draws and which
        // would cost most of the atlas.
        for point in [0x4E00u32, 0x0400, 0x1F600] {
            assert!(
                !ui_codepoint(point),
                "U+{point:04X} crept into the interface set"
            );
        }
    }

    #[test]
    fn the_embedded_faces_are_the_files_on_disk() {
        // Cheap, and it catches the one failure `include_bytes!` makes possible:
        // a path that resolves to something that is not a font at all. Both
        // magic numbers are from the OpenType spec — `OTTO` for CFF outlines,
        // 0x00010000 for TrueType.
        assert_eq!(
            &SPACE_GROTESK[..4],
            b"OTTO",
            "Space Grotesk is not a CFF OpenType file"
        );
        assert_eq!(
            &ALEGREYA[..4],
            &[0x00, 0x01, 0x00, 0x00],
            "Alegreya is not a TrueType file"
        );
    }
}
