//! The interface and caption faces.
//!
//! Port of `load_assets`' font half (`../musializer/src/plug.c:8060-8137`), of
//! `ui_font` / `caption_face` (`plug.c:340-365`), and of
//! `caption_imported_font_load` / `_unload` (`plug.c:371-427`).
//!
//! The four logical face slots match the oracle, but the interface slot is a
//! native-size bank rather than one scaled texture. Space Grotesk is rasterized
//! at every shell size over the interface subset and once at 64 px over the full
//! curated caption set; Alegreya is rasterized once, and a project's imported
//! face is rasterized on demand and keyed by the path it came from.
//! [`Faces::caption`] is the seam that picks among caption faces, and it is the
//! whole of `caption_face`.
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

use std::borrow::Cow;
use std::cell::Cell;
use std::path::{Path, PathBuf};

use musializer_core::project::caption_layout;
use musializer_core::project::model::CaptionFace;
use raylib::prelude::{Color, RaylibDraw, RaylibFont, Vector2, WeakFont};
use raylib::text::Font;
use raylib::{RaylibHandle, RaylibThread};

/// Rasterization size for captions and icons (`FONT_SIZE`, `plug.c:104`).
///
/// Caption styles can request continuous sizes, so those faces retain the
/// oracle's 64 px mipmapped atlas. The interface uses [`UI_FONT_SIZES`] instead.
pub const FONT_SIZE: i32 = 64;

/// Native raster sizes for interface text.
///
/// The shell's fixed typography uses 11--19, 24, 28, 34, 38 and 84 px. Fitted
/// button rows can land anywhere from their 11 px floor through the 22 px cap,
/// so that interval is complete rather than sampled. Every resolved UI draw is
/// quantized down to one of these sizes and rendered 1:1 from its matching
/// atlas. That removes the old 64 px atlas's discrete mip-level changes and
/// makes integer pixel snapping meaningful.
pub const UI_FONT_SIZES: [i32; 17] = [
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24, 28, 34, 38, 84,
];

/// Largest face this build will rasterize
/// (`CAPTION_IMPORTED_FONT_BYTE_LIMIT`, `plug.c:369`).
///
/// Well above any Google Fonts family — the biggest CJK entries are a few
/// megabytes — and far below a size that would make the atlas build a hang.
pub const IMPORTED_FACE_BYTE_LIMIT: u64 = 32 * 1024 * 1024;

/// Space Grotesk Regular, SIL OFL 1.1. See `resources/fonts/SpaceGrotesk-OFL.txt`.
const SPACE_GROTESK: &[u8] = include_bytes!("../../../resources/fonts/SpaceGrotesk-Regular.otf");

/// Alegreya Regular, SIL OFL 1.1. See `resources/fonts/OFL.txt`.
const ALEGREYA: &[u8] = include_bytes!("../../../resources/fonts/Alegreya-Regular.ttf");

/// Font Awesome 4, SIL OFL 1.1. See `resources/fonts/FontAwesome-OFL.txt`.
///
/// A fourth face, and the first thing in this build that is **not** in the frozen
/// C — the oracle's chrome is text-labelled throughout. It is here because a
/// transport row is the one place where a glyph is more legible than its name at
/// the sizes the row shrinks to, and because every icon it draws is paired with a
/// tooltip that spells the label out, so nothing is only available as a picture.
const FONT_AWESOME: &[u8] = include_bytes!("../../../resources/fonts/FontAwesome.otf");

/// The icons the chrome draws, and the only codepoints the icon atlas carries.
///
/// An enum rather than bare `char` constants because the atlas is built from
/// [`Icon::ALL`]: adding a variant and forgetting to rasterize it would draw a
/// missing-glyph box, and that failure would look like a font bug rather than a
/// one-line omission. The same list is therefore both the vocabulary and the
/// atlas request, which is the only way they cannot drift.
///
/// Codepoints are Font Awesome 4's Private Use Area assignments. They are not
/// Unicode semantics — U+F04B is "play" only because this font says so — so the
/// names here are the contract and the numbers are an implementation detail of
/// the vendored face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    /// The render/export control, drawn as a film reel.
    Film,
    VolumeOff,
    VolumeDown,
    VolumeUp,
    /// Enter fullscreen.
    Expand,
    /// Leave fullscreen.
    Compress,
    /// Jump to the start of the track.
    StepBackward,
    /// Nudge the position backwards.
    ChevronLeft,
    /// Nudge the position forwards.
    ChevronRight,
    /// The tuning inspector.
    Sliders,
    /// The lyrics panel.
    FileText,
    /// The assist panel.
    Magic,
    /// The diagnostic readout toggle.
    Info,
}

impl Icon {
    /// Every icon, which is exactly what the atlas is built over.
    pub const ALL: [Icon; 15] = [
        Icon::Play,
        Icon::Pause,
        Icon::Film,
        Icon::VolumeOff,
        Icon::VolumeDown,
        Icon::VolumeUp,
        Icon::Expand,
        Icon::Compress,
        Icon::StepBackward,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Sliders,
        Icon::FileText,
        Icon::Magic,
        Icon::Info,
    ];

    /// The Font Awesome 4 codepoint.
    #[must_use]
    pub fn codepoint(self) -> u32 {
        match self {
            Icon::Play => 0xF04B,
            Icon::Pause => 0xF04C,
            Icon::Film => 0xF008,
            Icon::VolumeOff => 0xF026,
            Icon::VolumeDown => 0xF027,
            Icon::VolumeUp => 0xF028,
            Icon::Expand => 0xF065,
            Icon::Compress => 0xF066,
            Icon::StepBackward => 0xF048,
            Icon::ChevronLeft => 0xF053,
            Icon::ChevronRight => 0xF054,
            Icon::Sliders => 0xF1DE,
            Icon::FileText => 0xF0F6,
            Icon::Magic => 0xF0D0,
            Icon::Info => 0xF05A,
        }
    }

    /// The codepoint as a `char`, which is what a draw call needs.
    ///
    /// Infallible by construction: every value above is a valid scalar in the
    /// Private Use Area, and the test at the bottom of this module proves it for
    /// all of [`Icon::ALL`] rather than leaving it to inspection.
    #[must_use]
    pub fn glyph(self) -> char {
        char::from_u32(self.codepoint()).unwrap_or('\u{FFFD}')
    }
}

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

/// One native-size interface atlas selected for a draw or measurement.
///
/// Deliberately does not implement [`RaylibFont`]. Shell code must resolve UI
/// typography through [`UiFonts`] instead of accidentally treating the bank as
/// the old single face; changing `Faces::ui()` to this type makes the compiler
/// enumerate every raw-face assumption in the application.
#[derive(Clone, Copy)]
struct UiFontRef<'a> {
    face: &'a Face,
    size: f32,
}

/// Space Grotesk rasterized at every size the application chrome can use.
///
/// `Cell` records coverage for the headless report. It is diagnostic state, not
/// drawing state: resolving the same label twice returns the same face and size,
/// so preview/export determinism does not depend on it.
pub struct UiFonts {
    faces: Vec<(i32, Face)>,
    scale: f32,
    used_mask: Cell<u32>,
    non_native_requests: Cell<u64>,
}

impl UiFonts {
    fn new(faces: Vec<(i32, Face)>, scale: f32) -> Self {
        debug_assert_eq!(faces.len(), UI_FONT_SIZES.len());
        Self {
            faces,
            scale,
            used_mask: Cell::new(0),
            non_native_requests: Cell::new(0),
        }
    }

    /// Largest native size not exceeding `requested`, with the bank's endpoints
    /// as clamps. Fitted rows use this to quantize *before* measuring, preserving
    /// their guarantee that every label fits.
    #[must_use]
    pub fn native_size(&self, requested: f32) -> f32 {
        native_ui_size(requested) as f32
    }

    /// Physical pixels per logical UI unit for this atlas bank.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Resolves a requested design size to its native atlas and records whether
    /// a caller bypassed size quantization.
    #[must_use]
    fn resolve(&self, requested: f32) -> UiFontRef<'_> {
        let native = native_ui_size(requested);
        let index = UI_FONT_SIZES
            .iter()
            .position(|&size| size == native)
            .expect("native UI size must belong to the raster bank");
        self.used_mask.set(self.used_mask.get() | (1u32 << index));
        if !requested.is_finite() || (requested - native as f32).abs() > 0.001 {
            self.non_native_requests
                .set(self.non_native_requests.get().saturating_add(1));
        }
        UiFontRef {
            face: &self.faces[index].1,
            size: native as f32,
        }
    }

    /// Measures through the same native atlas and size the draw call will use.
    #[must_use]
    pub fn measure_text(&self, text: &str, requested: f32, spacing: f32) -> Vector2 {
        let resolved = self.resolve(requested);
        let spacing = snap_logical(spacing, self.scale);
        resolved.face.measure_text(text, resolved.size, spacing)
    }

    /// Draws native-size UI text on integer pixel origins.
    pub fn draw_text<D: RaylibDraw>(
        &self,
        d: &mut D,
        text: &str,
        position: Vector2,
        requested: f32,
        spacing: f32,
        tint: Color,
    ) {
        let resolved = self.resolve(requested);
        let position = Vector2::new(
            snap_logical(position.x, self.scale),
            snap_logical(position.y, self.scale),
        );
        d.draw_text_ex(
            resolved.face,
            text,
            position,
            resolved.size,
            snap_logical(spacing, self.scale),
            tint,
        );
    }

    #[must_use]
    pub fn all_loaded(&self) -> bool {
        self.faces.iter().all(|(_, face)| face.is_loaded())
    }

    #[must_use]
    pub fn loaded_count(&self) -> usize {
        self.faces
            .iter()
            .filter(|(_, face)| face.is_loaded())
            .count()
    }

    /// Coverage evidence for captures: the sizes actually used and whether any
    /// caller asked the draw layer to rescale instead of quantizing first.
    #[must_use]
    pub fn usage_report(&self) -> String {
        let mask = self.used_mask.get();
        let used = UI_FONT_SIZES
            .iter()
            .enumerate()
            .filter_map(|(index, size)| ((mask & (1u32 << index)) != 0).then_some(size.to_string()))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "scale={:.2} native-sizes=[{used}] non-native-requests={}",
            self.scale,
            self.non_native_requests.get()
        )
    }
}

fn snap_logical(value: f32, scale: f32) -> f32 {
    (value * scale).round() / scale
}

/// Pure size policy shared by the GPU bank and its tests.
#[must_use]
pub fn native_ui_size(requested: f32) -> i32 {
    if !requested.is_finite() {
        return UI_FONT_SIZES[0];
    }
    UI_FONT_SIZES
        .iter()
        .copied()
        .rev()
        .find(|&size| size as f32 <= requested + 0.001)
        .unwrap_or(UI_FONT_SIZES[0])
}

/// Every logical face slot the application draws with.
///
/// Loaded once and borrowed for the rest of the run. A face loaded per frame — or
/// per scene switch — leaks a GPU atlas each time, which is the same mistake
/// `SceneRenderer` exists to prevent for shaders.
pub struct Faces {
    ui: UiFonts,
    caption: Face,
    /// Space Grotesk at the **full** curated caption set — `p->caption_alt_font`
    /// (`plug.c:353`). Not the interface face: that one carries only the
    /// codepoints the chrome draws, so typesetting a caption with it would drop
    /// Greek and Cyrillic without saying so (`plug.c:346-349`).
    caption_alt: Face,
    /// Font Awesome at the fifteen codepoints in [`Icon::ALL`], and nothing else.
    ///
    /// Fifteen glyphs rather than the face's ~600: an atlas costs space per
    /// *requested* codepoint whether or not the face has a glyph, and this face is
    /// asked for so little that the atlas is smaller than any of the other three by
    /// two orders of magnitude.
    icons: Face,
    /// The project's imported face, and the path it was rasterized from
    /// (`p->caption_imported_font` / `_path`, `plug.c:371-427`).
    ///
    /// The path is the *key*, not a label: `caption_imported_font_load` returns
    /// early when it is asked for the face it already holds, which is what stops
    /// a per-frame reload from leaking an atlas every frame.
    imported: Option<(PathBuf, Face)>,
}

impl Faces {
    /// Rasterizes the interface and both built-in caption faces, falling back to
    /// raylib's default for any one that will not load.
    ///
    /// Never fails: a missing face is a degraded interface, not a reason to
    /// refuse to start. `load_assets` in the C has the same shape.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        Self::load_with_ui_scale(rl, thread, 1.0)
    }

    /// Load the shell bank for `ui_scale` physical pixels per logical unit.
    pub fn load_with_ui_scale(rl: &mut RaylibHandle, thread: &RaylibThread, ui_scale: f32) -> Self {
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

        let ui = load_ui_fonts(rl, thread, &ui_codepoints, ui_scale);
        let caption = rasterize(rl, thread, ".ttf", ALEGREYA, &caption_codepoints).map_or_else(
            || {
                eprintln!("FONT: Alegreya caption face unavailable; using raylib default");
                default_face()
            },
            Face::Loaded,
        );
        // The second Space Grotesk atlas. It costs a full 2,000-codepoint atlas
        // and it is the only thing that makes `MUSI_CAPTION_FACE_SPACE_GROTESK`
        // mean anything: falling back to the interface face here would quietly
        // typeset a Cyrillic lyric as missing-glyph boxes.
        let caption_alt = rasterize(rl, thread, ".otf", SPACE_GROTESK, &caption_codepoints)
            .map_or_else(
                || {
                    eprintln!(
                        "FONT: Space Grotesk caption face unavailable; captions asking for it \
                         will fall back to Alegreya"
                    );
                    default_face()
                },
                Face::Loaded,
            );

        // The icon atlas. A failure here is the one fallback that is *not*
        // survivable by drawing something slightly wrong: raylib's default face has
        // no Private Use Area glyphs at all, so every icon button would draw an
        // empty box. `Face::is_loaded` is what the toolbar consults to fall back to
        // text labels instead, and `describe` reports it either way.
        let icon_codepoints: Vec<i32> = Icon::ALL
            .iter()
            .map(|icon| icon.codepoint() as i32)
            .collect();
        let icons = rasterize(rl, thread, ".otf", FONT_AWESOME, &icon_codepoints).map_or_else(
            || {
                eprintln!("FONT: icon face unavailable; the chrome will use text labels");
                default_face()
            },
            Face::Loaded,
        );

        Self {
            ui,
            caption,
            caption_alt,
            icons,
            imported: None,
        }
    }

    /// Rebuild only the shell bank when a window crosses a scale rung. Caption
    /// and imported faces are project assets and remain untouched.
    pub fn set_ui_scale(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, ui_scale: f32) {
        if (self.ui.scale() - ui_scale).abs() < 0.001 {
            return;
        }
        let curated = caption_layout::font_codepoints().unwrap_or_default();
        let codepoints: Vec<i32> = curated
            .iter()
            .copied()
            .filter(|&codepoint| ui_codepoint(codepoint))
            .map(|codepoint| codepoint as i32)
            .collect();
        self.ui = load_ui_fonts(rl, thread, &codepoints, ui_scale);
    }

    /// Every built-in face is the fallback. The constructor a headless test can
    /// build without a GPU — and the state the application ends up in when the
    /// atlas build fails, so it is worth being able to name.
    #[must_use]
    pub fn fallback_only() -> Self {
        Self {
            ui: UiFonts::new(
                UI_FONT_SIZES
                    .iter()
                    .copied()
                    .map(|size| (size, default_face()))
                    .collect(),
                1.0,
            ),
            caption: default_face(),
            caption_alt: default_face(),
            icons: default_face(),
            imported: None,
        }
    }

    /// The native-size interface face bank.
    #[must_use]
    pub fn ui(&self) -> &UiFonts {
        &self.ui
    }

    /// The icon face, or the fallback if it would not rasterize.
    ///
    /// Callers must check [`Face::is_loaded`] before drawing a glyph through it.
    /// This is the one face where the fallback cannot approximate the real thing:
    /// raylib's default has no Private Use Area coverage, so an icon drawn through
    /// it is an empty box rather than a slightly wrong letterform.
    #[must_use]
    pub fn icons(&self) -> &Face {
        &self.icons
    }

    /// Whether icon buttons can be drawn at all.
    ///
    /// The seam that makes the text fallback reachable in a test, rather than only
    /// on a machine where the atlas build fails.
    #[must_use]
    pub fn icons_available(&self) -> bool {
        self.icons.is_loaded()
    }

    /// The face a caption style asks for, with a defined fallback.
    ///
    /// This is `caption_face` (`plug.c:350-364`) exactly, including its fallback
    /// rule: a style naming a face this build could not rasterize gets
    /// **Alegreya**, not raylib's default. raylib's bitmap face has none of the
    /// curated glyph coverage and would silently drop every accent.
    ///
    /// Threaded explicitly rather than reached for through a `GuiSetFont`-style
    /// implicit face, which is what makes the fallback reachable in a test.
    #[must_use]
    pub fn caption_for(&self, face: CaptionFace) -> &Face {
        match face {
            CaptionFace::SpaceGrotesk if self.caption_alt.is_loaded() => &self.caption_alt,
            CaptionFace::Imported => self
                .imported
                .as_ref()
                .map_or(&self.caption, |(_, face)| face),
            _ => &self.caption,
        }
    }

    /// The caption default: Alegreya at the full curated set
    /// (`caption_face`'s final arm, `plug.c:360-363`).
    ///
    /// Deliberately **not** the interface face, whose atlas carries only the
    /// codepoints the chrome needs. A scene drawing a *project's* captions must
    /// go through [`Faces::caption_for`] with that track's
    /// [`CaptionFace`] instead; this is the answer for a caller that has no
    /// style to consult, which today is the cadence scene's own overlay.
    #[must_use]
    pub fn caption(&self) -> &Face {
        self.caption_for(CaptionFace::Alegreya)
    }

    /// The face for **project-authored** text: cue lines, the lyric text field,
    /// track names.
    ///
    /// Not `caption_for`: this deliberately ignores the project's
    /// [`CaptionFace`] choice, including an imported one. A caption style is a
    /// statement about how the *video* should look, and the editor has to stay
    /// legible while the user is still deciding — an imported display face with
    /// no Cyrillic would otherwise make the field unreadable in exactly the
    /// session where the user is typing Cyrillic into it. So authored text gets
    /// a built-in face over the full curated set and the style applies to the
    /// rendered frame.
    ///
    /// The ladder, and why it is in this order:
    ///
    /// | State | Serves | `name()` | Repertoire |
    /// | --- | --- | --- | --- |
    /// | Alegreya rasterized | Alegreya, 64 px | `caption` | Curated |
    /// | Alegreya failed, Space Grotesk full-set rasterized | Space Grotesk, 64 px | `caption-alt` | Curated |
    /// | both caption atlases failed, chrome bank intact | the UI bank | `ui` | InterfaceSubset |
    /// | nothing rasterized | raylib default | `default` | RaylibDefault |
    ///
    /// The last two rungs are degraded and say so. Dropping to the chrome bank
    /// loses Greek and Cyrillic — it is the defect UX0-A05 fixed, kept only as
    /// the answer to "both caption atlases are gone", where the alternative is
    /// raylib's 10 px bitmap face and even less coverage. A panel drawing through
    /// this must report [`AuthoredText::name`] and a missing-glyph count, per the
    /// repository rule that a surface which can draw without its data has to say
    /// which one it drew.
    #[must_use]
    pub fn authored(&self) -> AuthoredText<'_> {
        let scale = self.ui.scale();
        if self.caption.is_loaded() {
            return AuthoredText {
                source: AuthoredSource::Caption(&self.caption),
                scale,
                repertoire: GlyphRepertoire::Curated,
                name: "caption",
            };
        }
        if self.caption_alt.is_loaded() {
            return AuthoredText {
                source: AuthoredSource::Caption(&self.caption_alt),
                scale,
                repertoire: GlyphRepertoire::Curated,
                name: "caption-alt",
            };
        }
        AuthoredText::from_ui(&self.ui)
    }

    /// The path the imported face was rasterized from, or `None`
    /// (`p->caption_imported_font_path`).
    #[must_use]
    pub fn imported_path(&self) -> Option<&Path> {
        self.imported.as_ref().map(|(path, _)| path.as_path())
    }

    /// Rasterizes an already-verified face (`caption_imported_font_load`,
    /// `plug.c:383-427`).
    ///
    /// **The caller owns the promise that these bytes matched their recorded
    /// digest.** This only decides whether raylib can make an atlas out of them,
    /// and answers honestly when it cannot — which is the same division of labour
    /// the C draws, and the reason nothing here re-reads a manifest.
    ///
    /// Idempotent for a path already loaded, because the alternative is an atlas
    /// leaked on every frame that asks.
    pub fn load_imported(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        path: &Path,
    ) -> bool {
        if self.imported_path() == Some(path) {
            return true;
        }
        self.clear_imported();

        if !imported_face_size_is_usable(path) {
            // The C's own wording (`plug.c:407`).
            eprintln!("FONT: imported caption face is not a usable size");
            return false;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        let codepoints: Vec<i32> = caption_layout::font_codepoints()
            .unwrap_or_default()
            .into_iter()
            .map(|point| point as i32)
            .collect();
        if codepoints.is_empty() {
            return false;
        }
        // The stored name is content-addressed, so the extension is the only
        // thing that tells raylib which loader to use. A face that arrived
        // without one is tried as a TrueType, which is what every path that
        // writes here produces (`plug.c:411-415`).
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map_or_else(|| ".ttf".to_string(), |value| format!(".{value}"));

        match rasterize(rl, thread, &extension, &bytes, &codepoints) {
            None => false,
            Some(font) => {
                self.imported = Some((path.to_path_buf(), Face::Loaded(font)));
                true
            }
        }
    }

    /// Drops the imported face and the path that named it
    /// (`caption_imported_font_unload`, `plug.c:371-378`).
    ///
    /// The GPU atlas goes with it: [`Font`]'s `Drop` is `UnloadFont`.
    pub fn clear_imported(&mut self) {
        self.imported = None;
    }

    /// One line naming which faces are real, for the slice report.
    ///
    /// Evidence rather than assertion: "the font loaded" is the kind of claim a
    /// clean exit cannot support, and a fallback to the 10 px bitmap face is
    /// exactly the regression that would otherwise be noticed by eye weeks later.
    /// The imported slot reports its *path*, because "an imported face is loaded"
    /// and "the imported face the project names is loaded" are different claims.
    #[must_use]
    pub fn describe(&self) -> String {
        let fallback = "raylib default (FALLBACK)";
        format!(
            "ui={}, {}, caption={}, caption-alt={}, icons={}, imported={}, authored={}",
            if self.ui.all_loaded() {
                format!("Space Grotesk ({} native sizes)", UI_FONT_SIZES.len())
            } else {
                format!(
                    "{}/{} Space Grotesk sizes loaded ({fallback})",
                    self.ui.loaded_count(),
                    UI_FONT_SIZES.len()
                )
            },
            self.ui.usage_report(),
            if self.caption.is_loaded() {
                "Alegreya"
            } else {
                fallback
            },
            if self.caption_alt.is_loaded() {
                "Space Grotesk"
            } else {
                fallback
            },
            // Named separately because its fallback is not a degraded face but a
            // *different interface*: the toolbar draws text labels instead, and a
            // reader of this line should be able to tell which one they are looking
            // at in a capture.
            if self.icons.is_loaded() {
                format!("Font Awesome ({} glyphs)", Icon::ALL.len())
            } else {
                format!("{fallback}, chrome falls back to text labels")
            },
            self.imported_path()
                .map_or_else(|| "none".to_string(), |path| path.display().to_string()),
            // Which face serves *user-typed* text. Separate from `caption=`
            // because the two answer different questions: `caption=` is what a
            // rendered frame typesets with, `authored=` is what the editor can
            // legibly show the user while they type it.
            self.authored().name(),
        )
    }
}

fn load_ui_fonts(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    codepoints: &[i32],
    requested_scale: f32,
) -> UiFonts {
    let scale = if requested_scale.is_finite() {
        requested_scale.clamp(1.0, 2.0)
    } else {
        1.0
    };
    UiFonts::new(
        UI_FONT_SIZES
            .iter()
            .copied()
            .map(|logical_size| {
                let pixel_size = (logical_size as f32 * scale).round() as i32;
                let face = rasterize_at(
                    rl,
                    thread,
                    ".otf",
                    SPACE_GROTESK,
                    codepoints,
                    pixel_size,
                    false,
                )
                .map_or_else(
                    || {
                        eprintln!(
                            "FONT: Space Grotesk UI face unavailable at {pixel_size}px \
                             ({logical_size}px logical, {scale:.2}x); using raylib default"
                        );
                        default_face()
                    },
                    Face::Loaded,
                );
                (logical_size, face)
            })
            .collect(),
        scale,
    )
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

/// Whether the curated caption table asked for this codepoint.
///
/// Reads [`caption_layout::FONT_CODEPOINT_RANGES`] directly rather than
/// materializing [`caption_layout::font_codepoints`], because this is called per
/// character of every authored string drawn in a frame and the table is 21
/// ascending ranges.
#[must_use]
fn curated_codepoint(codepoint: u32) -> bool {
    caption_layout::FONT_CODEPOINT_RANGES
        .iter()
        .any(|&(first, last)| (first..=last).contains(&codepoint))
}

/// The glyph repertoire a face's atlas was **requested over**.
///
/// This is deliberately about the atlas request, not about the outlines the
/// typeface happens to contain, because the request is what the UX0-A05 defect
/// was: the interface atlas is [`ui_codepoint`]-filtered, so a Greek or Cyrillic
/// lyric drawn through it comes out as missing-glyph boxes — in the one editor
/// whose whole job is editing that lyric. A codepoint the atlas never asked for
/// cannot be drawn no matter how complete the font file is, and that is a
/// property this crate can decide without a GPU, which is what makes
/// [`GlyphRepertoire::missing`] assertable in a headless test.
///
/// It does **not** model an imported face: that atlas is requested over the full
/// curated set, so the request is complete even when the file's coverage is not.
/// Whether a user's own `.ttf` has a Greek alpha is the user's business and is
/// not something this build can answer before rasterizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphRepertoire {
    /// The full curated caption set — Latin, Greek, Cyrillic, punctuation.
    Curated,
    /// [`ui_codepoint`] applied to the curated set: chrome only.
    InterfaceSubset,
    /// raylib's built-in bitmap face: printable ASCII and nothing else.
    RaylibDefault,
}

impl GlyphRepertoire {
    /// Whether a face over this repertoire can draw `ch`.
    ///
    /// Line and tab controls answer `true`: they are layout instructions rather
    /// than glyphs, and counting them as missing would make every multi-line
    /// lyric report a defect it does not have.
    #[must_use]
    pub fn covers(self, ch: char) -> bool {
        if matches!(ch, '\n' | '\r' | '\t') {
            return true;
        }
        let codepoint = ch as u32;
        match self {
            // raylib's default atlas is the 95 printable ASCII glyphs
            // (`rtext.c`'s `LoadFontDefault`).
            GlyphRepertoire::RaylibDefault => (0x20..=0x7E).contains(&codepoint),
            GlyphRepertoire::InterfaceSubset => {
                curated_codepoint(codepoint) && ui_codepoint(codepoint)
            }
            GlyphRepertoire::Curated => curated_codepoint(codepoint),
        }
    }

    /// How many characters of `text` this repertoire cannot draw.
    ///
    /// Occurrences, not distinct codepoints: "one missing glyph" and "forty
    /// missing glyphs" describe different pictures, and the report line exists to
    /// tell them apart.
    #[must_use]
    pub fn missing(self, text: &str) -> usize {
        text.chars().filter(|&ch| !self.covers(ch)).count()
    }

    /// The truncation marker a face over this repertoire can actually draw.
    ///
    /// U+2026 is inside the curated General Punctuation range *and* inside
    /// [`ui_codepoint`]'s, so both real atlases carry it; only raylib's bitmap
    /// fallback needs the three-period spelling. Choosing per repertoire rather
    /// than hard-coding `"..."` everywhere is what keeps a truncated Cyrillic row
    /// from ending in the one character the row was truncated to avoid.
    #[must_use]
    pub fn ellipsis(self) -> &'static str {
        match self {
            GlyphRepertoire::RaylibDefault => "...",
            _ => "\u{2026}",
        }
    }
}

/// Fit `text` into `max_width`, ending in `ellipsis` when it does not fit.
///
/// `measure` must be the *same* face and size the caller will draw with, which is
/// why this takes a closure rather than a face: a row measured with the chrome
/// atlas and drawn with the caption atlas would either clip or leave a gap, and
/// the two atlases have different advance widths for the same string.
///
/// Truncation walks `char_indices` and never a byte offset. A byte-sliced
/// Cyrillic string panics, and a byte-sliced UTF-8 string sliced *successfully*
/// at the wrong boundary is worse — it is the same class of defect as this
/// module's `str::len()`-as-codepoint-count note.
pub fn ellipsize_with<'t, F>(
    text: &'t str,
    max_width: f32,
    ellipsis: &str,
    mut measure: F,
) -> Cow<'t, str>
where
    F: FnMut(&str) -> f32,
{
    if max_width <= 0.0 {
        return Cow::Borrowed("");
    }
    if measure(text) <= max_width {
        return Cow::Borrowed(text);
    }
    // Longest prefix whose ellipsized form still fits. Linear from the end
    // rather than a binary search because advance widths are not guaranteed
    // monotone under kerning, and a row is tens of characters.
    let mut best: Option<String> = None;
    for (index, _) in text.char_indices() {
        if index == 0 {
            continue;
        }
        let candidate = format!("{}{ellipsis}", text[..index].trim_end());
        if measure(&candidate) <= max_width {
            best = Some(candidate);
        } else {
            break;
        }
    }
    match best {
        Some(fitted) => Cow::Owned(fitted),
        // Not even one character plus the marker fits. The marker alone still
        // says "there is text here", which an empty rect does not.
        None if measure(ellipsis) <= max_width => Cow::Owned(ellipsis.to_string()),
        None => Cow::Borrowed(""),
    }
}

/// The face that draws **project-authored** text, and the evidence of which one.
///
/// The chrome keeps the [`UiFonts`] bank: its atlas is scale-matched, native-size
/// and small precisely because it carries no Greek, Cyrillic or accented-extended
/// coverage, and every string the chrome draws is a literal in this repository.
/// Anything a *user* typed — a cue line, the lyric text field, a track name — goes
/// through here instead, because the user's alphabet is not this repository's
/// choice to make.
///
/// The seam is a type rather than a `&Face` so that measurement, drawing,
/// truncation and the missing-glyph count all come from one object. A caret
/// measured with one face and drawn against text laid out by another drifts, and
/// that drift grows with the line, so the caret lands nowhere near the character
/// the user clicked.
#[derive(Clone, Copy)]
pub struct AuthoredText<'a> {
    source: AuthoredSource<'a>,
    scale: f32,
    repertoire: GlyphRepertoire,
    name: &'static str,
}

#[derive(Clone, Copy)]
enum AuthoredSource<'a> {
    /// A 64 px mipmapped caption atlas over the full curated set.
    ///
    /// Resolution-independent by construction: the shell draws in logical units
    /// under a `Camera2D` whose zoom is the UI scale, so a logical 15 px row is
    /// 15, 18.75 or 22.5 physical pixels sampled *down* from a 64 px atlas. There
    /// is no rung at which this magnifies a 1x bitmap, which is the failure the
    /// native-size UI bank exists to avoid.
    Caption(&'a Face),
    /// The chrome bank, used only when neither caption atlas rasterized.
    Ui(&'a UiFonts),
}

impl<'a> AuthoredText<'a> {
    /// Which face served authored text: `caption`, `caption-alt`, `ui` or
    /// `default`. Reported by panels so a capture carries the answer rather than
    /// requiring someone to recognize Alegreya by eye.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The repertoire behind [`AuthoredText::missing`].
    #[must_use]
    pub fn repertoire(&self) -> GlyphRepertoire {
        self.repertoire
    }

    /// Characters of `text` the serving face cannot draw. Zero is the claim the
    /// headless gate asserts.
    #[must_use]
    pub fn missing(&self, text: &str) -> usize {
        self.repertoire.missing(text)
    }

    /// The truncation marker this face can draw.
    #[must_use]
    pub fn ellipsis(&self) -> &'static str {
        self.repertoire.ellipsis()
    }

    /// Measures through the face and size that will draw.
    #[must_use]
    pub fn measure_text(&self, text: &str, size: f32, spacing: f32) -> Vector2 {
        match self.source {
            AuthoredSource::Caption(face) => {
                face.measure_text(text, size, snap_logical(spacing, self.scale))
            }
            AuthoredSource::Ui(bank) => bank.measure_text(text, size, spacing),
        }
    }

    /// Physical pixels per logical unit, for a caller that needs the shell's
    /// [`crate::font::UiFonts::scale`] without also holding the chrome bank.
    ///
    /// Deliberately the *same* number either arm, because it describes the
    /// window rather than the atlas: a text field converts the pointer into
    /// logical units with it, and that conversion must not change when the face
    /// behind the field does.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Wrap the chrome bank as authored text.
    ///
    /// The seam that lets a widget take an [`AuthoredText`] without every caller
    /// having to have a [`Faces`] in hand: chrome that happens to flow through
    /// the same widget keeps the native-size bank, and the repertoire reported is
    /// the honest [`GlyphRepertoire::InterfaceSubset`] rather than a claim of
    /// full coverage.
    #[must_use]
    pub fn from_ui(bank: &'a UiFonts) -> Self {
        Self {
            source: AuthoredSource::Ui(bank),
            scale: bank.scale(),
            repertoire: if bank.all_loaded() {
                GlyphRepertoire::InterfaceSubset
            } else {
                GlyphRepertoire::RaylibDefault
            },
            name: if bank.all_loaded() { "ui" } else { "default" },
        }
    }

    /// Fit `text` into `max_width` through this face.
    #[must_use]
    pub fn ellipsize<'t>(
        &self,
        text: &'t str,
        max_width: f32,
        size: f32,
        spacing: f32,
    ) -> Cow<'t, str> {
        ellipsize_with(text, max_width, self.ellipsis(), |candidate| {
            self.measure_text(candidate, size, spacing).x
        })
    }

    /// Draws authored text on the same integer pixel grid the chrome uses.
    pub fn draw_text<D: RaylibDraw>(
        &self,
        d: &mut D,
        text: &str,
        position: Vector2,
        size: f32,
        spacing: f32,
        tint: Color,
    ) {
        match self.source {
            AuthoredSource::Caption(face) => {
                let position = Vector2::new(
                    snap_logical(position.x, self.scale),
                    snap_logical(position.y, self.scale),
                );
                d.draw_text_ex(
                    face,
                    text,
                    position,
                    size,
                    snap_logical(spacing, self.scale),
                    tint,
                );
            }
            AuthoredSource::Ui(bank) => bank.draw_text(d, text, position, size, spacing, tint),
        }
    }
}

/// The size gate from `caption_imported_font_load` (`plug.c:405-409`), split out
/// because it is the half of that function reachable without a window.
///
/// Zero bytes is what a worker killed mid-write leaves behind, and anything over
/// [`IMPORTED_FACE_BYTE_LIMIT`] is what would turn an atlas build into a hang.
/// A directory or a missing file is neither, and both answer `false`.
#[must_use]
pub fn imported_face_size_is_usable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= IMPORTED_FACE_BYTE_LIMIT
    })
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
    rasterize_at(rl, thread, file_type, bytes, codepoints, FONT_SIZE, true)
}

fn rasterize_at(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    file_type: &str,
    bytes: &[u8],
    codepoints: &[i32],
    pixel_size: i32,
    generate_mipmaps: bool,
) -> Option<Font> {
    let _ = (rl, thread); // Proof a window exists; raylib needs one for the atlas.
    let c_file_type = std::ffi::CString::new(file_type).ok()?;

    // SAFETY: `LoadFontFromMemory` reads `bytes.len()` bytes from `bytes` and
    // `codepoints.len()` `i32`s from `codepoints`, and both lengths are passed
    // from the slices themselves, so neither read can run past its allocation.
    // Both borrows outlive the call, which is what matters here and is all that
    // matters: `bytes` is an `include_bytes!` array for the three built-in faces
    // and a heap `Vec` read from disk for an imported one, and raylib copies out
    // what it needs before returning either way — it keeps no pointer into the
    // buffer. Passing a null glyph array with a zero count is raylib's documented
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
            pixel_size,
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

    // Caption faces retain the oracle's mipmapped 64 px atlas because their
    // project style may request any continuous size. UI faces are native-size
    // atlases and deliberately skip mipmaps: selecting another level would
    // recreate the discontinuity this bank exists to remove.
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
        if generate_mipmaps {
            raylib_sys::GenTextureMipmaps(&mut raw_font.texture);
        }
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
    fn ui_native_sizes_cover_every_fixed_and_fitted_shell_size() {
        assert!(UI_FONT_SIZES.windows(2).all(|pair| pair[0] < pair[1]));
        for size in 11..=22 {
            assert!(
                UI_FONT_SIZES.contains(&size),
                "fitted row size {size} has no native atlas"
            );
        }
        for size in [24, 28, 34, 38, 84] {
            assert!(
                UI_FONT_SIZES.contains(&size),
                "fixed shell size {size} has no native atlas"
            );
        }
    }

    #[test]
    fn ui_size_resolution_quantizes_down_and_reports_bypasses() {
        assert_eq!(native_ui_size(10.0), 11);
        assert_eq!(native_ui_size(18.72), 18);
        assert_eq!(native_ui_size(22.99), 22);
        assert_eq!(native_ui_size(23.99), 22);
        assert_eq!(native_ui_size(24.0), 24);
        assert_eq!(native_ui_size(100.0), 84);
        assert_eq!(native_ui_size(f32::NAN), 11);

        let faces = Faces::fallback_only();
        let _ = faces.ui.resolve(15.0);
        let _ = faces.ui.resolve(18.72);
        assert_eq!(
            faces.ui.usage_report(),
            "scale=1.00 native-sizes=[15,18] non-native-requests=1"
        );
    }

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
    fn the_caption_seam_falls_back_to_alegreya_and_never_to_the_bitmap_face() {
        // `caption_face`'s rule (`plug.c:350-364`), asserted on the one
        // configuration a headless test can build: nothing rasterized. Every
        // style must still resolve, and must resolve to the *same* face — the
        // Alegreya slot — rather than to raylib's default by a different route.
        //
        // This is the check that would have caught a `caption()` that returned
        // the interface face for `SpaceGrotesk`: the interface atlas has no
        // Greek, so a caption typeset with it silently loses glyphs.
        let faces = Faces::fallback_only();
        for face in [
            CaptionFace::Alegreya,
            CaptionFace::SpaceGrotesk,
            CaptionFace::Imported,
        ] {
            assert!(
                std::ptr::eq(faces.caption_for(face), faces.caption()),
                "{face:?} did not fall back to the caption default"
            );
        }
        assert_eq!(faces.imported_path(), None);
        // `Imported` with nothing imported is a state the model forbids
        // (`CaptionStyle::validate`) but the renderer still has to survive,
        // because the style and the asset are written in two steps.
        assert!(!faces.caption_for(CaptionFace::Imported).is_loaded());
    }

    #[test]
    fn describe_names_all_four_slots_so_a_capture_carries_the_answer() {
        let faces = Faces::fallback_only();
        let line = faces.describe();
        for key in ["ui=", "caption=", "caption-alt=", "imported="] {
            assert!(line.contains(key), "{key} missing from {line:?}");
        }
        // A fallback must be loud. `tools/headless_check.sh` greps this line, and
        // a silent revert to the 10 px bitmap face is exactly the regression it
        // exists to catch.
        assert!(line.contains("FALLBACK"));
        assert!(line.contains("imported=none"));
    }

    #[test]
    fn an_imported_face_that_is_not_a_usable_size_is_refused_without_a_window() {
        // The size gate runs before raylib is involved at all
        // (`plug.c:405-409`), which is what makes it testable here. A zero-byte
        // file is what a worker killed mid-write leaves behind, and a file over
        // the ceiling is the one that would turn the atlas build into a hang.
        let mut faces = Faces::fallback_only();
        let directory =
            std::env::temp_dir().join(format!("musializer-face-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("scratch");
        let empty = directory.join("empty.ttf");
        std::fs::write(&empty, b"").expect("write");

        // `load_imported` needs a `RaylibHandle` it can only get from a window, so
        // the reachable half here is the refusal path — which is the half with the
        // interesting rules. That the caller must own the digest promise is the
        // other half, and it is the caller's test.
        assert!(!imported_face_size_is_usable(&empty));
        assert!(!imported_face_size_is_usable(&directory.join("absent.ttf")));
        assert!(!imported_face_size_is_usable(&directory));
        std::fs::write(&empty, b"not a font, but a plausible size").expect("write");
        assert!(imported_face_size_is_usable(&empty));

        // Clearing an empty slot is a no-op rather than a panic, which is what
        // `font_service_clear_import` (`plug.c:1914-1923`) relies on: it clears
        // unconditionally, without first asking whether anything is loaded.
        faces.clear_imported();
        assert_eq!(faces.imported_path(), None);

        let _ = std::fs::remove_dir_all(&directory);
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
        assert_eq!(
            &FONT_AWESOME[..4],
            b"OTTO",
            "Font Awesome is not a CFF OpenType file"
        );
    }

    /// The UX0-A05 defect, as a number.
    ///
    /// The seeded fixture's line is the case that was reported: the preview
    /// typeset it perfectly while the cue list and the text field one panel below
    /// showed rows of `?` with only the em dash surviving. That asymmetry is
    /// exactly this assertion — the em dash is U+2014, inside General
    /// Punctuation, which is one of the four ranges `ui_codepoint` keeps.
    #[test]
    fn authored_text_through_the_chrome_repertoire_loses_greek_and_cyrillic() {
        const REPORTED: &str = "Καλημέρα κόσμε — мир продолжает звучать";

        assert_eq!(GlyphRepertoire::Curated.missing(REPORTED), 0);

        // The negative control, and it is the defect rather than a synthetic
        // perturbation: serve the same string from the chrome atlas and count.
        let dropped = GlyphRepertoire::InterfaceSubset.missing(REPORTED);
        assert_eq!(
            dropped,
            REPORTED
                .chars()
                .filter(|ch| !ch.is_ascii() && *ch != '\u{2014}')
                .count(),
            "every non-Latin letter, and only those, is outside the chrome atlas"
        );
        assert!(dropped > 0);

        // The survivor named in the report. If this ever flips, the picture in
        // the bug changes and so should this test.
        assert!(GlyphRepertoire::InterfaceSubset.covers('\u{2014}'));
        assert!(!GlyphRepertoire::InterfaceSubset.covers('Κ'));
        assert!(!GlyphRepertoire::InterfaceSubset.covers('м'));
        assert!(GlyphRepertoire::Curated.covers('Κ'));
        assert!(GlyphRepertoire::Curated.covers('м'));

        // raylib's bitmap fallback is worse than both and must not be mistaken
        // for either: it loses the accents too.
        assert_eq!(GlyphRepertoire::RaylibDefault.missing("Ångström"), 2);
        assert_eq!(GlyphRepertoire::InterfaceSubset.missing("Ångström"), 0);

        // Newlines are layout, not glyphs. Counting them would make every
        // multi-line lyric report a defect it does not have.
        assert_eq!(GlyphRepertoire::RaylibDefault.missing("a\nb\tc"), 0);

        // CJK is outside the *curated* set as well, which is a real limit of the
        // bundled faces rather than a bug in the seam, and is worth pinning so a
        // future reader does not read `missing=0` as "any script works".
        assert!(GlyphRepertoire::Curated.missing("東京") > 0);
    }

    #[test]
    fn the_truncation_marker_is_one_the_serving_face_can_draw() {
        // U+2026 is in the curated General Punctuation range and survives
        // `ui_codepoint`, so both real atlases carry it and only raylib's
        // fallback needs three periods. Asserted rather than assumed, because
        // "the ellipsis draws as a box" is precisely the bug the shell already
        // paid for once.
        assert_eq!(GlyphRepertoire::Curated.ellipsis(), "\u{2026}");
        assert_eq!(GlyphRepertoire::InterfaceSubset.ellipsis(), "\u{2026}");
        assert_eq!(GlyphRepertoire::RaylibDefault.ellipsis(), "...");
        for repertoire in [
            GlyphRepertoire::Curated,
            GlyphRepertoire::InterfaceSubset,
            GlyphRepertoire::RaylibDefault,
        ] {
            assert_eq!(
                repertoire.missing(repertoire.ellipsis()),
                0,
                "{repertoire:?} cannot draw its own truncation marker"
            );
        }
    }

    #[test]
    fn ellipsize_cuts_on_character_boundaries_and_measures_what_it_returns() {
        // One unit per character, which is the same stand-in `caption_layout`'s
        // tests use. The point is the boundary arithmetic, not the metrics.
        let measure = |text: &str| text.chars().count() as f32;

        let text = "мир продолжает звучать";
        assert_eq!(ellipsize_with(text, 100.0, "\u{2026}", measure), text);

        let fitted = ellipsize_with(text, 10.0, "\u{2026}", measure);
        assert!(measure(&fitted) <= 10.0, "{fitted:?} overflows its row");
        assert!(fitted.ends_with('\u{2026}'));
        assert!(text.starts_with(fitted.trim_end_matches('\u{2026}')));
        // A byte slice at 10 would land inside a two-byte Cyrillic scalar; a
        // character slice does not. This is the assertion that would have caught
        // a `&text[..n]` truncation, which panics rather than clipping.
        assert!(fitted.chars().count() < text.chars().count());

        // Degenerate widths answer rather than panic.
        assert_eq!(ellipsize_with(text, 0.0, "\u{2026}", measure), "");
        assert_eq!(ellipsize_with(text, 1.0, "\u{2026}", measure), "\u{2026}");
        assert_eq!(ellipsize_with("", 5.0, "\u{2026}", measure), "");
        // A single character that does not fit still leaves the marker.
        assert_eq!(ellipsize_with("Ω", 0.5, "\u{2026}", measure), "");
    }

    #[test]
    fn the_authored_seam_names_its_face_and_degrades_loudly() {
        // The only configuration a headless test can build is the worst rung, so
        // that is the one pinned: nothing rasterized must report `default` and a
        // non-zero missing count for the reported string, never silently serve
        // it. The `caption` rung is what a windowed run reaches, and the gate in
        // `tools/headless_check.sh` asserts it from the panel's report line.
        let faces = Faces::fallback_only();
        let authored = faces.authored();
        assert_eq!(authored.name(), "default");
        assert_eq!(authored.repertoire(), GlyphRepertoire::RaylibDefault);
        assert!(authored.missing("Καλημέρα") > 0);
        assert_eq!(authored.ellipsis(), "...");
        assert_eq!(authored.missing("plain ascii"), 0);

        // `describe` carries it, because a capture has to answer "which face drew
        // the user's own words" without anyone recognizing Alegreya by eye.
        assert!(faces.describe().contains("authored=default"));

        // `from_ui` is the seam a widget takes when it has only the chrome bank.
        // It must report the bank honestly and must not invent a different
        // pointer scale, because a field converts mouse coordinates with it.
        let chrome = AuthoredText::from_ui(faces.ui());
        assert_eq!(chrome.name(), authored.name());
        assert_eq!(chrome.repertoire(), authored.repertoire());
        assert!((chrome.scale() - faces.ui().scale()).abs() < f32::EPSILON);
    }

    /// The atlas is built from [`Icon::ALL`], so a variant missing from it is a
    /// glyph the face was never asked for — which draws as an empty box at a size
    /// nobody notices until a capture is reviewed.
    #[test]
    fn every_icon_is_in_the_atlas_request_exactly_once() {
        let mut seen = std::collections::BTreeSet::new();
        for icon in Icon::ALL {
            assert!(
                seen.insert(icon.codepoint()),
                "{icon:?} shares a codepoint with an earlier variant"
            );
            // Private Use Area. Not a range check for its own sake: a codepoint
            // that strayed out of it would be a real Unicode character, which
            // Space Grotesk might well have — so the icon would draw as a letter
            // rather than as an obvious box, and look deliberate.
            assert!(
                (0xE000..=0xF8FF).contains(&icon.codepoint()),
                "{icon:?} is outside the Private Use Area"
            );
            assert_ne!(
                icon.glyph(),
                '\u{FFFD}',
                "{icon:?} is not a valid Unicode scalar"
            );
        }
        assert_eq!(seen.len(), Icon::ALL.len());
    }
}
