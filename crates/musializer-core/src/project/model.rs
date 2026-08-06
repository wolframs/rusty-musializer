//! The `.musi` data model and its validation.
//!
//! **Owner: Agent B.** Port of `../musializer/src/project.c/.h`.
//!
//! Validation is the point of this module. `.musi` input is bounded and checked
//! **before** it mutates application state, and every bound here is the C bound,
//! not a plausible-looking one. Three rules are easy to get subtly wrong and are
//! called out where they appear:
//!
//! - every `maxLength` counts **UTF-8 bytes**, not code points
//!   (`project-v1.schema.json:6`), so [`str::len`] is correct and
//!   `chars().count()` is not;
//! - `caption_style` is **optional** in v1 and absent means the shipped defaults,
//!   which reproduce the appearance a pre-caption project was authored against;
//! - the imported caption face and its bundled font asset are one fact stated
//!   twice, and a project where they disagree is invalid in *either* direction.
//!
//! Mapping evaluation, sources, and curve shapes are **not** duplicated here:
//! they live in [`crate::scene::routes`], which landed first as a shared
//! contract. This module reuses [`ParameterMapping`], [`AnalysisSource`] and
//! [`Interpolation`] from there.

use crate::project::lyrics::LyricsDocument;
use crate::project::sha256;
use crate::scene::events::EventType;
use crate::scene::routes::{mappings_supported, AnalysisSource, Interpolation, ParameterMapping};
use crate::scene::settings::{MAX_CONTROLS, PRESETS_PER_SCENE};
use crate::scene::SCENE_COUNT;

use super::event_timeline::EventTimeline;

/// `MUSI_PROJECT_SCHEMA_VERSION` (`project.h:12`). The file spells this
/// `"musializer.project/v1"`; the number is the in-memory form.
pub const SCHEMA_VERSION: u32 = 1;
/// `project.h:13`.
pub const MAX_SCENES: usize = 32;
/// `project.h:14-15`: every scene's every control, since one `.musi` scene entry
/// persists the whole settings table as constant mappings.
pub const MAX_MAPPINGS_PER_SCENE: usize = SCENE_COUNT * MAX_CONTROLS;
/// `project.h:16`.
pub const MAX_CUES: usize = 256;
/// `project.h:17`.
pub const MAX_ANALYSIS_LANES: usize = 8;
/// `project.h:18-19`.
pub const MAX_SCENE_PRESETS: usize = SCENE_COUNT * PRESETS_PER_SCENE;
/// `scene_switch.h:10`.
pub const SCENE_SWITCH_CAPACITY: usize = 256;
/// `ascii_art.h:9-10`.
pub const ASCII_GRID_MAX_COLUMNS: u32 = 96;
/// `ascii_art.h:9-10`.
pub const ASCII_GRID_MAX_ROWS: u32 = 54;

/// Maximum **bytes** in a bounded string field. Each is the C buffer capacity
/// minus its NUL terminator (`project.h:21-28`).
pub mod capacity {
    /// `project_id`, digests, scene types, parameter keys, version strings.
    pub const ID: usize = 64;
    /// `title`, `author`, `family`, `licence_name`, preset names.
    pub const NAME: usize = 128;
    /// Every path field.
    pub const PATH: usize = 1024;
    /// `scene_type`, `scene_name`, provenance `adapter`.
    pub const TYPE: usize = 64;
    /// Mapping and cue `parameter`.
    pub const PARAMETER: usize = 64;
    /// `application_version`, `adapter_version`, `schema_version`,
    /// `prompt_version`.
    pub const VERSION: usize = 64;
    /// Provenance `model` and `provider`.
    pub const PROVIDER: usize = 128;
    /// `created_utc`, `modified_utc`.
    pub const TIMESTAMP: usize = 32;
}

/// Caption measurement bounds (`project.h:127-141`).
///
/// Every one is a **fraction of the frame**, never a pixel count, so a project
/// typeset against a 1280x720 preview exports identically at 3840x2160. A pixel
/// size here would reintroduce exactly the resolution dependence the old fixed
/// 42 px caption ceiling caused.
pub mod caption {
    /// Fraction of frame **height**.
    pub const SIZE_MINIMUM: f64 = 0.012;
    pub const SIZE_MAXIMUM: f64 = 0.300;
    pub const SIZE_DEFAULT: f64 = 0.047;
    /// Inset from the anchored edges, fraction of frame height.
    pub const MARGIN_MINIMUM: f64 = 0.0;
    pub const MARGIN_MAXIMUM: f64 = 0.400;
    pub const MARGIN_DEFAULT: f64 = 0.065;
    /// Widest the caption box may become, fraction of frame **width**.
    pub const WIDTH_MINIMUM: f64 = 0.20;
    pub const WIDTH_MAXIMUM: f64 = 1.00;
    pub const WIDTH_DEFAULT: f64 = 0.82;
    pub const TEXT_RGBA_DEFAULT: u32 = 0xFFFF_FFFF;
    /// Exactly what `ColorAlpha(BLACK, 0.72f)` produced before the plate colour
    /// was authorable: raylib truncates `255*0.72` to 183, so `0xB8` would
    /// silently change every existing project's captions by one step of alpha
    /// (`project.h:138-141`).
    pub const BOX_RGBA_DEFAULT: u32 = 0x0000_00B7;
}

macro_rules! named_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($variant:ident => $text:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum $name {
            #[default]
            $($variant),+
        }

        impl $name {
            /// Every value, in the C enum's order. The order is persisted through
            /// snapshots and indices, so appending is safe and reordering is not.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The name written to `.musi`.
            #[must_use]
            pub fn canonical_name(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            /// Parses the name written to `.musi`. Unknown names are a hard error
            /// upstream, not a default.
            #[must_use]
            pub fn from_canonical_name(name: &str) -> Option<Self> {
                match name { $($text => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

named_enum! {
    /// `Musi_Asset_Mode` (`project.h:30-34`).
    AssetMode { Imported => "imported", Referenced => "referenced" }
}

named_enum! {
    /// `Musi_Output_Format` (`project.h:36-43`).
    OutputFormat {
        Mp4H264 => "mp4_h264",
        MkvH264 => "mkv_h264",
        WebmVp9 => "webm_vp9",
        MovProres => "mov_prores",
        PngSequence => "png_sequence",
    }
}

named_enum! {
    /// `Musi_Output_Quality` (`project.h:45-50`). Durable *intent*: supersampling
    /// and encoder details are derived from it, never serialized separately.
    OutputQuality { Balanced => "balanced", High => "high", Master => "master" }
}

named_enum! {
    /// `Musi_Blend_Mode` (`project.h:52-58`).
    BlendMode {
        Normal => "normal",
        Add => "add",
        Multiply => "multiply",
        Screen => "screen",
    }
}

named_enum! {
    /// `Musi_Analysis_Lane_Kind` (`project.h:78-83`). One lane of each kind at
    /// most, and every lane's `audio_sha256` must equal the project's audio digest.
    AnalysisLaneKind {
        MeasuredSignal => "measured_signal",
        LyricTiming => "lyric_timing",
        SemanticScore => "semantic_score",
    }
}

named_enum! {
    /// `Musi_Caption_Face` (`project.h:89-97`). `Alegreya` is both the historical
    /// face and the default.
    CaptionFace {
        Alegreya => "alegreya",
        SpaceGrotesk => "space_grotesk",
        Imported => "imported",
    }
}

named_enum! {
    /// `Musi_Caption_Box` (`project.h:99-108`). `None` is text alone, `Shadow` an
    /// offset copy behind it, `Plate` the rounded panel the product shipped with.
    CaptionBox { None => "none", Shadow => "shadow", Plate => "plate" }
}

named_enum! {
    /// What modulates a caption effect per frame. First post-legacy extension
    /// (2026-08-03): no C counterpart. Every source is a pure function of the
    /// [`crate::scene::SceneFrame`], so preview and export agree by construction.
    ///
    /// `Rms` is overall loudness; `Bass` the low-band energy of the smoothed
    /// trails; `Beat` a sharp pulse decaying over each beat interval; `Flux`
    /// the spectral movement; `Time` a steady clock cycle for drives that
    /// should not depend on the music. The mapping from each source to its
    /// 0..=1 value lives in [`crate::project::caption_effects::drive_value`].
    EffectDrive {
        None => "none",
        Rms => "rms",
        Bass => "bass",
        Beat => "beat",
        Flux => "flux",
        Time => "time",
    }
}

/// Caption effect bounds. Like [`caption`], every length is a fraction —
/// of the resolved font size, not of the frame — so effects scale with the
/// type they decorate at any export resolution.
pub mod caption_fx {
    pub const GLOW_STRENGTH_MINIMUM: f64 = 0.0;
    pub const GLOW_STRENGTH_MAXIMUM: f64 = 1.0;
    pub const GLOW_STRENGTH_DEFAULT: f64 = 0.0;
    /// Fraction of the font size.
    pub const GLOW_RADIUS_MINIMUM: f64 = 0.02;
    pub const GLOW_RADIUS_MAXIMUM: f64 = 0.60;
    pub const GLOW_RADIUS_DEFAULT: f64 = 0.18;
    /// Warm amber, full alpha: legible over dark material the moment the
    /// strength slider leaves zero, without first visiting a colour control.
    pub const GLOW_RGBA_DEFAULT: u32 = 0xFFC8_64FF;
    pub const PULSE_DEPTH_MINIMUM: f64 = 0.0;
    pub const PULSE_DEPTH_MAXIMUM: f64 = 1.0;
    pub const PULSE_DEPTH_DEFAULT: f64 = 0.6;
    /// Degrees of hue swept across the drive's 0..1 range.
    pub const HUE_RANGE_MINIMUM: f64 = 0.0;
    pub const HUE_RANGE_MAXIMUM: f64 = 360.0;
    pub const HUE_RANGE_DEFAULT: f64 = 120.0;
    /// Fraction of the font size. Zero is the legacy hard-offset shadow.
    pub const SHADOW_BLUR_MINIMUM: f64 = 0.0;
    pub const SHADOW_BLUR_MAXIMUM: f64 = 0.50;
    pub const SHADOW_BLUR_DEFAULT: f64 = 0.0;
    pub const SHADOW_OPACITY_MINIMUM: f64 = 0.0;
    pub const SHADOW_OPACITY_MAXIMUM: f64 = 1.0;
    pub const SHADOW_OPACITY_DEFAULT: f64 = 1.0;
    /// `DrawRectangleRounded` roundness. The default is the constant the C
    /// hard-coded (`plug.c:1282`), now authorable.
    pub const PLATE_ROUNDNESS_MINIMUM: f64 = 0.0;
    pub const PLATE_ROUNDNESS_MAXIMUM: f64 = 0.5;
    pub const PLATE_ROUNDNESS_DEFAULT: f64 = 0.12;
}

/// How one caption drive's raw 0..=1 value maps to the amount an effect uses —
/// the same quiet/loud in→out ranges, curve and clamp the Tune editor gives a
/// scene route (UX0-C14, 2026-08-04).
///
/// The input is the drive's *shaped* value ([`super::caption_effects::
/// drive_value`]), so "Beat" stays a pulse and "RMS" stays perceptual before
/// tuning refines the window. Evaluation delegates to
/// [`ParameterMapping::evaluate_mapping`], so the semantics — including the
/// clamp asymmetry the route differential harness pins — cannot drift from the
/// scene routes'. The default is the identity (0..1 → 0..1, linear, clamped)
/// and, like the rest of the effects block, a default is never serialized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveTuning {
    pub input_min: f64,
    pub input_max: f64,
    pub output_min: f64,
    pub output_max: f64,
    pub curve: Interpolation,
    pub clamp: bool,
}

impl Default for DriveTuning {
    fn default() -> Self {
        Self {
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
            curve: Interpolation::Linear,
            clamp: true,
        }
    }
}

impl DriveTuning {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Drive values and effect amounts are both 0..=1, so every endpoint lives
    /// there too; the input window must be non-degenerate. Equal *outputs* are
    /// fine — a flat amount is a legitimate authored choice here, unlike a
    /// route, where the flat spelling is reserved for slider constants.
    #[must_use]
    pub fn validate(&self) -> bool {
        let unit = |value: f64| value.is_finite() && (0.0..=1.0).contains(&value);
        unit(self.input_min)
            && unit(self.input_max)
            && unit(self.output_min)
            && unit(self.output_max)
            && self.input_max > self.input_min
    }

    /// The tuned amount for one drive sample, clamped to the 0..=1 the effects
    /// consume. `None` never survives to a caller: a mapping this type's
    /// `validate` admits cannot fail evaluation on a finite sample, but the
    /// fallback keeps a stale in-memory draft harmless — it falls back to the
    /// raw drive.
    #[must_use]
    pub fn apply(&self, drive: f64) -> f64 {
        ParameterMapping::evaluate_mapping(
            drive,
            self.input_min,
            self.input_max,
            self.output_min,
            self.output_max,
            self.curve,
            self.clamp,
        )
        .unwrap_or(drive)
        .clamp(0.0, 1.0)
    }
}

/// Caption text effects: glow, soft shadow and plate shape.
///
/// First `.musi` extension past the frozen C (operator decision, 2026-08-03).
/// The default value renders **exactly** the legacy composition — no glow, the
/// hard offset shadow, the 0.12 plate roundness — and a default block is not
/// serialized at all, so every pre-effects project round-trips byte-identically
/// (which is also what keeps `differential_project_io.sh` green).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionEffects {
    /// 0 disables the glow entirely.
    pub glow_strength: f64,
    /// Halo radius as a fraction of the font size.
    pub glow_radius: f64,
    /// Base glow colour; its alpha is the ceiling the strength scales.
    pub glow_rgba: u32,
    /// Modulates glow intensity per frame.
    pub glow_pulse: EffectDrive,
    /// How much of the intensity the pulse owns: 0 is steady, 1 swings to zero.
    pub glow_pulse_depth: f64,
    /// Maps the pulse drive's raw value to the amount the pulse uses.
    pub pulse_tuning: DriveTuning,
    /// Shifts the glow hue per frame.
    pub glow_hue_drive: EffectDrive,
    /// Degrees of hue swept across the drive's range.
    pub glow_hue_range: f64,
    /// Maps the hue drive's raw value to the amount the sweep uses.
    pub hue_tuning: DriveTuning,
    /// Softens the `Shadow` backing; 0 is the legacy hard copy.
    pub shadow_blur: f64,
    /// Scales the shadow colour's alpha; 1 is the legacy value.
    pub shadow_opacity: f64,
    /// Corner roundness of the `Plate` backing.
    pub plate_roundness: f64,
}

impl Default for CaptionEffects {
    fn default() -> Self {
        Self {
            glow_strength: caption_fx::GLOW_STRENGTH_DEFAULT,
            glow_radius: caption_fx::GLOW_RADIUS_DEFAULT,
            glow_rgba: caption_fx::GLOW_RGBA_DEFAULT,
            glow_pulse: EffectDrive::None,
            glow_pulse_depth: caption_fx::PULSE_DEPTH_DEFAULT,
            pulse_tuning: DriveTuning::default(),
            glow_hue_drive: EffectDrive::None,
            glow_hue_range: caption_fx::HUE_RANGE_DEFAULT,
            hue_tuning: DriveTuning::default(),
            shadow_blur: caption_fx::SHADOW_BLUR_DEFAULT,
            shadow_opacity: caption_fx::SHADOW_OPACITY_DEFAULT,
            plate_roundness: caption_fx::PLATE_ROUNDNESS_DEFAULT,
        }
    }
}

impl CaptionEffects {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every scalar finite and inside its published bound.
    #[must_use]
    pub fn validate(&self) -> bool {
        let bounded = |value: f64, minimum: f64, maximum: f64| {
            value.is_finite() && value >= minimum && value <= maximum
        };
        bounded(
            self.glow_strength,
            caption_fx::GLOW_STRENGTH_MINIMUM,
            caption_fx::GLOW_STRENGTH_MAXIMUM,
        ) && bounded(
            self.glow_radius,
            caption_fx::GLOW_RADIUS_MINIMUM,
            caption_fx::GLOW_RADIUS_MAXIMUM,
        ) && bounded(
            self.glow_pulse_depth,
            caption_fx::PULSE_DEPTH_MINIMUM,
            caption_fx::PULSE_DEPTH_MAXIMUM,
        ) && bounded(
            self.glow_hue_range,
            caption_fx::HUE_RANGE_MINIMUM,
            caption_fx::HUE_RANGE_MAXIMUM,
        ) && bounded(
            self.shadow_blur,
            caption_fx::SHADOW_BLUR_MINIMUM,
            caption_fx::SHADOW_BLUR_MAXIMUM,
        ) && bounded(
            self.shadow_opacity,
            caption_fx::SHADOW_OPACITY_MINIMUM,
            caption_fx::SHADOW_OPACITY_MAXIMUM,
        ) && bounded(
            self.plate_roundness,
            caption_fx::PLATE_ROUNDNESS_MINIMUM,
            caption_fx::PLATE_ROUNDNESS_MAXIMUM,
        ) && self.pulse_tuning.validate()
            && self.hue_tuning.validate()
    }
}

named_enum! {
    /// `Musi_Caption_Anchor` (`project.h:110-121`), in the C enum's 0..8 order.
    CaptionAnchor {
        BottomLeft => "bottom_left",
        BottomCenter => "bottom_center",
        BottomRight => "bottom_right",
        MiddleLeft => "middle_left",
        MiddleCenter => "middle_center",
        MiddleRight => "middle_right",
        TopLeft => "top_left",
        TopCenter => "top_center",
        TopRight => "top_right",
    }
}

/// True for the schema's `stable_name`: 1..64 bytes, `^[A-Za-z0-9][A-Za-z0-9._:-]*$`
/// (`project.c:21-37`, `project-v1.schema.json:917-922`).
#[must_use]
pub fn is_stable_name(value: &str) -> bool {
    if value.is_empty() || value.len() > capacity::ID {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

/// C's `bounded_string` (`project.c:15-19`): fits the field, and non-empty when
/// the field requires it.
///
/// The byte length is what matters. `U+0000` is rejected outright: C's buffer
/// would treat it as a terminator and silently accept a truncated value, and the
/// schema forbids it (`project-v1.schema.json:6`).
#[must_use]
pub fn is_bounded(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    value.len() <= max_bytes && (allow_empty || !value.is_empty()) && !value.as_bytes().contains(&0)
}

/// A bundled caption face and its licence (`Musi_Font_Asset`, `project.h:143-159`).
///
/// The licence is one fact in three fields: all empty, or all populated. A path
/// without a digest could not be verified before the text is shown, a digest
/// without a path describes nothing, and an unnamed licence file tells a
/// recipient that terms exist without telling them which terms they are.
///
/// Empty is legitimate: a face imported from the user's own disk carries terms
/// this application cannot assert, and inventing them would be worse than
/// recording none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontAsset {
    /// Project-relative path inside the sibling `<stem>.assets/fonts/` bundle.
    pub path: String,
    /// Digest of the face. Required and non-empty.
    pub sha256: String,
    /// Display name for the control. **Never** used to locate the file.
    pub family: String,
    pub licence_path: String,
    /// Digest of the bundled licence text. Empty exactly when `licence_path` is.
    ///
    /// The schema uses `^([0-9a-f]{64})?$` here rather than `$defs/sha256`
    /// (`project-v1.schema.json:251`) precisely so it may be empty. Reusing the
    /// shared definition is the easy mistake.
    pub licence_sha256: String,
    /// Which terms these are, e.g. `OFL-1.1`.
    pub licence_name: String,
}

impl FontAsset {
    /// True when a licence file travels with the face.
    #[must_use]
    pub fn has_licence(&self) -> bool {
        !self.licence_path.is_empty()
    }

    /// The three-field licence rule and the face's own bounds
    /// (`project.c:171-193`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if !is_bounded(&self.path, capacity::PATH, false)
            || !sha256::is_hex_digest(&self.sha256)
            || !is_bounded(&self.family, capacity::NAME, false)
        {
            return false;
        }
        let bundled = self.has_licence();
        if bundled == self.licence_sha256.is_empty() {
            return false;
        }
        if bundled
            && (!is_bounded(&self.licence_path, capacity::PATH, false)
                || !sha256::is_hex_digest(&self.licence_sha256)
                || self.licence_name.is_empty())
        {
            return false;
        }
        is_bounded(&self.licence_name, capacity::NAME, true)
    }
}

/// Caption typography (`Musi_Caption_Style`, `project.h:161-171`).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionStyle {
    pub face: CaptionFace,
    pub box_style: CaptionBox,
    pub anchor: CaptionAnchor,
    pub size_scale: f64,
    pub margin_scale: f64,
    pub width_scale: f64,
    pub text_rgba: u32,
    /// Plate fill, or the shadow colour when `box_style` is
    /// [`CaptionBox::Shadow`]. Ignored when it is [`CaptionBox::None`].
    pub box_rgba: u32,
    /// Present **exactly when** `face` is [`CaptionFace::Imported`].
    pub font: Option<FontAsset>,
    /// Glow, soft shadow and plate shape. Defaults render the legacy
    /// composition exactly; see [`CaptionEffects`].
    pub effects: CaptionEffects,
}

impl Default for CaptionStyle {
    /// `musi_caption_style_init` (`project.c:74-86`).
    ///
    /// The style a project has when its file predates caption typography. These
    /// are exactly the values the renderer used before the field existed, so an
    /// old project reopened under this build looks the same as it did. That is a
    /// documented compatibility default, not a missing field.
    fn default() -> Self {
        Self {
            face: CaptionFace::Alegreya,
            box_style: CaptionBox::Plate,
            anchor: CaptionAnchor::BottomCenter,
            size_scale: caption::SIZE_DEFAULT,
            margin_scale: caption::MARGIN_DEFAULT,
            width_scale: caption::WIDTH_DEFAULT,
            text_rgba: caption::TEXT_RGBA_DEFAULT,
            box_rgba: caption::BOX_RGBA_DEFAULT,
            font: None,
            effects: CaptionEffects::default(),
        }
    }
}

impl CaptionStyle {
    /// `musi_caption_style_is_default` (`project.c:88-100`): nothing to explain to
    /// the user and nothing that needs the asset bundle.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `project.c:148-193`.
    fn validate(&self) -> bool {
        if !self.size_scale.is_finite()
            || self.size_scale < caption::SIZE_MINIMUM
            || self.size_scale > caption::SIZE_MAXIMUM
            || !self.margin_scale.is_finite()
            || self.margin_scale < caption::MARGIN_MINIMUM
            || self.margin_scale > caption::MARGIN_MAXIMUM
            || !self.width_scale.is_finite()
            || self.width_scale < caption::WIDTH_MINIMUM
            || self.width_scale > caption::WIDTH_MAXIMUM
        {
            return false;
        }
        // The imported face and the font asset are one fact stated twice. Either
        // arrangement where they disagree would open a project whose captions are
        // typeset in a face the file does not carry (`project.c:164`).
        if (self.face == CaptionFace::Imported) != self.font.is_some() {
            return false;
        }
        if !self.effects.validate() {
            return false;
        }
        self.font.as_ref().map_or(true, FontAsset::is_valid)
    }
}

/// `Musi_Project_Metadata` (`project.h:173-180`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    /// A `stable_name`.
    pub project_id: String,
    pub title: String,
    pub author: String,
    pub created_utc: String,
    pub modified_utc: String,
    pub application_version: String,
}

/// `Musi_Audio_Asset` (`project.h:182-189`).
#[derive(Clone, Debug, PartialEq)]
pub struct AudioAsset {
    pub mode: AssetMode,
    pub path: String,
    pub sha256: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioAsset {
    fn default() -> Self {
        Self {
            mode: AssetMode::Imported,
            path: String::new(),
            sha256: String::new(),
            duration_seconds: 0.0,
            sample_rate: 0,
            channels: 0,
        }
    }
}

/// `Musi_Ascii_Image_Asset` (`project.h:191-197`). `None` is the file's `null`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AsciiImageAsset {
    pub path: String,
    pub sha256: String,
    pub columns: u32,
    pub rows: u32,
}

/// `Musi_Output_Settings` (`project.h:199-209`).
#[derive(Clone, Debug, PartialEq)]
pub struct OutputSettings {
    pub width: u32,
    pub height: u32,
    pub fps_numerator: u32,
    pub fps_denominator: u32,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub format: OutputFormat,
    pub quality: OutputQuality,
}

impl Default for OutputSettings {
    /// The defaults `musi_project_init` writes (`project.c:60-65`).
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps_numerator: 30,
            fps_denominator: 1,
            start_seconds: 0.0,
            end_seconds: 0.0,
            format: OutputFormat::Mp4H264,
            quality: OutputQuality::High,
        }
    }
}

/// One layer (`Musi_Scene_Entry`, `project.h:223-233`). Layer order is back to
/// front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneEntry {
    /// A `positive_uint64`: unique across the project and never zero.
    pub instance_id: u64,
    /// A `stable_name` — the scene's persisted name, not its display label.
    pub scene_type: String,
    pub enabled: bool,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub opacity: f64,
    pub blend_mode: BlendMode,
    pub mappings: Vec<ParameterMapping>,
}

/// One automation cue (`Musi_Parameter_Cue`, `project.h:235-244`).
///
/// Ranges are half-open `[start, end)`, and at a cue's end its `to_value`
/// *persists* until the next matching cue (`project-v1.schema.json:66`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParameterCue {
    pub cue_id: u64,
    pub target_scene_id: u64,
    pub parameter: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub from_value: f64,
    pub to_value: f64,
    pub interpolation: Interpolation,
}

/// Which adapter produced an analysis artifact (`Musi_Analysis_Provenance`,
/// `project.h:246-253`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    pub adapter: String,
    pub adapter_version: String,
    pub schema_version: String,
    pub model: String,
    pub provider: String,
    pub prompt_version: String,
}

/// A reference to an analysis artifact (`Musi_Analysis_Lane_Reference`,
/// `project.h:255-261`).
///
/// Provenance metadata, **not** a dependency: the referenced file is not required
/// in order to reopen evaluated project data
/// (`project-v1.schema.json:74`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalysisLaneReference {
    pub kind: AnalysisLaneKind,
    pub path: String,
    pub sha256: String,
    /// Must equal the project's `audio.sha256`. This is what stops analysis being
    /// attached to the wrong track.
    pub audio_sha256: String,
    pub provenance: Provenance,
}

/// One imported scene-switch suggestion (`Musi_Scene_Switch_Suggestion`,
/// `project.h:263-271`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSwitchSuggestion {
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// A `stable_name`.
    pub scene_name: String,
    pub strength: f32,
    /// Empty only for early v1 projects (`project-v1.schema.json:817`).
    pub settings: Vec<f32>,
}

/// The imported switch plan (`Musi_Scene_Switch_Suggestions`, `project.h:273-277`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSwitchSuggestions {
    /// The durable user opt-in. Nonempty cues must cover the whole track.
    pub enabled: bool,
    pub cues: Vec<SceneSwitchSuggestion>,
}

/// A saved tuning preset (`Musi_Scene_Preset`, `project.h:279-285`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenePreset {
    pub id: u64,
    /// A `stable_name` scene token, e.g. `"loom"`.
    pub scene_name: String,
    pub name: String,
    pub settings: Vec<f32>,
}

/// A whole `.musi` project (`Musi_Project`, `project.h:287-315`).
#[derive(Clone, Debug)]
pub struct Project {
    pub schema_version: u32,
    pub metadata: Metadata,
    pub audio: AudioAsset,
    pub ascii_image: Option<AsciiImageAsset>,
    /// Optional in the file format. Absent means [`CaptionStyle::default`].
    pub caption_style: CaptionStyle,
    pub output: OutputSettings,
    pub deterministic_seed: u64,
    pub scenes: Vec<SceneEntry>,
    pub cues: Vec<ParameterCue>,
    pub analysis_lanes: Vec<AnalysisLaneReference>,
    pub lyrics: LyricsDocument,
    pub scene_switches: SceneSwitchSuggestions,
    pub scene_presets: Vec<ScenePreset>,
    /// Validated model-derived semantic values are embedded project data. The
    /// `analysis_lanes` entries are provenance metadata, not dependencies required
    /// to reconstruct this evaluated lane.
    pub semantic_events: EventTimeline,
    /// **User-authored events only.** Semantic model output stays separate and
    /// must never be copied here as if manually authored — that is one of the
    /// evidence-lane separations the rewrite preserves.
    pub manual_events: EventTimeline,
}

impl Default for Project {
    /// `musi_project_init` (`project.c:54-72`).
    ///
    /// The lyrics document is created with a placeholder duration because C zeroes
    /// the struct and lets `duration_seconds` be filled in from the audio asset;
    /// [`Project::validate`] then insists the two agree.
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            metadata: Metadata::default(),
            audio: AudioAsset::default(),
            ascii_image: None,
            caption_style: CaptionStyle::default(),
            output: OutputSettings::default(),
            deterministic_seed: 0,
            scenes: Vec::new(),
            cues: Vec::new(),
            analysis_lanes: Vec::new(),
            lyrics: LyricsDocument::new(1.0).expect("1.0 is a valid duration"),
            scene_switches: SceneSwitchSuggestions::default(),
            scene_presets: Vec::new(),
            semantic_events: EventTimeline::new(),
            manual_events: EventTimeline::new(),
        }
    }
}

/// Why a project is invalid (`Musi_Project_Error`, `project.h:317-339`).
///
/// C's `MUSI_PROJECT_ERROR_NULL` has no counterpart. The messages are the C
/// strings from `musi_project_error_string` (`project.c:393-408`) so a Rust
/// diagnostic reads the same as the C one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    #[error("unsupported schema version")]
    SchemaVersion,
    #[error("invalid metadata")]
    Metadata,
    #[error("invalid audio asset")]
    Audio,
    #[error("invalid ASCII image asset")]
    AsciiImage,
    #[error("invalid caption style")]
    CaptionStyle,
    #[error("invalid output settings")]
    Output,
    #[error("capacity/count violation")]
    Count,
    #[error("invalid scene")]
    Scene,
    #[error("invalid parameter mapping")]
    Mapping,
    #[error("invalid cue")]
    Cue,
    #[error("cues are unsorted")]
    CueOrder,
    #[error("cues overlap")]
    CueOverlap,
    #[error("invalid analysis lane")]
    AnalysisLane,
    #[error("duplicate stable id")]
    DuplicateId,
    #[error("invalid lyrics")]
    Lyrics,
    #[error("invalid scene-switch suggestions")]
    SceneSwitch,
    #[error("invalid scene preset")]
    ScenePreset,
    #[error("invalid manual event")]
    ManualEvent,
    #[error("invalid semantic event")]
    SemanticEvent,
}

/// A validation failure and where it is (`Musi_Project_Validation`,
/// `project.h:341-345`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{error} (index {index}, subindex {subindex})")]
pub struct ProjectValidation {
    pub error: ProjectError,
    pub index: usize,
    pub subindex: usize,
}

/// Why this editor cannot round-trip an otherwise valid project
/// (`Musi_Project_Editor_Support`, `project.h:347-358`).
///
/// The schema is intentionally broader than today's single-scene MP4 editor. This
/// check is what stops open/edit/autosave from **silently normalizing** valid
/// fields the current UI cannot represent — which is the "existing projects are
/// never silently rewritten by merely opening them" invariant, enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditorSupport {
    #[error("the audio asset mode is not supported by this editor")]
    AudioMode,
    #[error("partial render ranges are not supported by this editor yet")]
    OutputRange,
    #[error("only integer-frame-rate H.264 MP4 output is supported by this editor")]
    OutputFormat,
    #[error("only one scene is supported by this editor yet")]
    SceneCount,
    #[error("parameter automation cues are not supported by this editor yet")]
    ParameterCues,
    #[error("the scene must cover the full track, be enabled, opaque, and Normal blend")]
    SceneLayout,
    #[error(
        "only built-in scene settings are supported, persisted as slider constants \
         or audio-driven routes"
    )]
    SceneMappings,
    #[error("an imported caption face is not supported by this editor yet")]
    CaptionFont,
}

fn fail(error: ProjectError, index: usize, subindex: usize) -> ProjectValidation {
    ProjectValidation {
        error,
        index,
        subindex,
    }
}

impl Project {
    /// `musi_project_validate` (`project.c:111-391`), in the same order, so the
    /// *first* complaint about a bad document matches the C's.
    pub fn validate(&self) -> Result<(), ProjectValidation> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(fail(ProjectError::SchemaVersion, 0, 0));
        }
        self.validate_metadata()?;
        self.validate_audio()?;
        self.validate_ascii_image()?;
        if !self.caption_style.validate() {
            return Err(fail(ProjectError::CaptionStyle, 0, 0));
        }
        self.validate_output()?;

        if self.scenes.is_empty()
            || self.scenes.len() > MAX_SCENES
            || self.cues.len() > MAX_CUES
            || self.analysis_lanes.len() > MAX_ANALYSIS_LANES
        {
            return Err(fail(ProjectError::Count, 0, 0));
        }
        self.validate_scenes()?;
        self.validate_cues()?;
        self.validate_analysis_lanes()?;

        if self.lyrics.duration_seconds() != self.audio.duration_seconds
            || self.lyrics.validate().is_err()
        {
            return Err(fail(ProjectError::Lyrics, 0, 0));
        }
        self.validate_scene_switches()?;
        self.validate_scene_presets()?;
        self.validate_event_lanes()
    }

    fn validate_metadata(&self) -> Result<(), ProjectValidation> {
        let metadata = &self.metadata;
        let ok = is_stable_name(&metadata.project_id)
            && is_bounded(&metadata.title, capacity::NAME, false)
            && is_bounded(&metadata.author, capacity::NAME, true)
            && is_bounded(&metadata.created_utc, capacity::TIMESTAMP, true)
            && is_bounded(&metadata.modified_utc, capacity::TIMESTAMP, true)
            && is_bounded(&metadata.application_version, capacity::VERSION, false);
        ok.then_some(())
            .ok_or_else(|| fail(ProjectError::Metadata, 0, 0))
    }

    fn validate_audio(&self) -> Result<(), ProjectValidation> {
        let audio = &self.audio;
        let ok = is_bounded(&audio.path, capacity::PATH, false)
            && sha256::is_hex_digest(&audio.sha256)
            && audio.duration_seconds.is_finite()
            && audio.duration_seconds > 0.0
            && audio.sample_rate > 0
            && audio.sample_rate <= 768_000
            && audio.channels > 0
            && audio.channels <= 64;
        ok.then_some(())
            .ok_or_else(|| fail(ProjectError::Audio, 0, 0))
    }

    fn validate_ascii_image(&self) -> Result<(), ProjectValidation> {
        let Some(ascii) = &self.ascii_image else {
            // C additionally checks that an absent asset left no stale fields
            // behind (`project.c:138-140`); `Option` makes that unrepresentable.
            return Ok(());
        };
        let ok = is_bounded(&ascii.path, capacity::PATH, false)
            && sha256::is_hex_digest(&ascii.sha256)
            && ascii.columns > 0
            && ascii.columns <= ASCII_GRID_MAX_COLUMNS
            && ascii.rows > 0
            && ascii.rows <= ASCII_GRID_MAX_ROWS;
        ok.then_some(())
            .ok_or_else(|| fail(ProjectError::AsciiImage, 0, 0))
    }

    fn validate_output(&self) -> Result<(), ProjectValidation> {
        let output = &self.output;
        // FPS is the exact rational `numerator/denominator` and must not exceed
        // 240 (`project.c:198`). The comparison is done in u64 so the product
        // cannot wrap.
        let fps_ok = output.fps_denominator > 0
            && output.fps_denominator <= 1001
            && output.fps_numerator > 0
            && u64::from(output.fps_numerator) <= 240 * u64::from(output.fps_denominator);
        let ok = output.width >= 16
            && output.width <= 16384
            && output.height >= 16
            && output.height <= 16384
            && fps_ok
            && output.start_seconds.is_finite()
            && output.start_seconds >= 0.0
            && output.end_seconds.is_finite()
            && output.end_seconds > output.start_seconds
            && output.end_seconds <= self.audio.duration_seconds;
        ok.then_some(())
            .ok_or_else(|| fail(ProjectError::Output, 0, 0))
    }

    fn validate_scenes(&self) -> Result<(), ProjectValidation> {
        for (index, scene) in self.scenes.iter().enumerate() {
            if scene.instance_id == 0
                || !is_stable_name(&scene.scene_type)
                || !scene.start_seconds.is_finite()
                || scene.start_seconds < 0.0
                || !scene.end_seconds.is_finite()
                || scene.end_seconds <= scene.start_seconds
                || scene.end_seconds > self.audio.duration_seconds
                || !scene.opacity.is_finite()
                || scene.opacity < 0.0
                || scene.opacity > 1.0
            {
                return Err(fail(ProjectError::Scene, index, 0));
            }
            if scene.mappings.len() > MAX_MAPPINGS_PER_SCENE {
                return Err(fail(ProjectError::Count, index, 0));
            }
            if let Some(previous) = self.scenes[..index]
                .iter()
                .position(|other| other.instance_id == scene.instance_id)
            {
                return Err(fail(ProjectError::DuplicateId, index, previous));
            }
            for (at, mapping) in scene.mappings.iter().enumerate() {
                // Deliberately *weaker* than `ParameterMapping::is_valid_for`:
                // the model accepts any `stable_name` parameter and equal output
                // endpoints, because a constant slider mapping is exactly that.
                // The stricter route rule belongs to editor support.
                if !is_stable_name(&mapping.parameter)
                    || (mapping.source != AnalysisSource::Band && mapping.band_index != 0)
                    || !mapping.input_min.is_finite()
                    || !mapping.input_max.is_finite()
                    || mapping.input_max <= mapping.input_min
                    || !mapping.output_min.is_finite()
                    || !mapping.output_max.is_finite()
                {
                    return Err(fail(ProjectError::Mapping, index, at));
                }
                if scene.mappings[..at]
                    .iter()
                    .any(|other| other.parameter == mapping.parameter)
                {
                    return Err(fail(ProjectError::Mapping, index, at));
                }
            }
        }
        Ok(())
    }

    fn validate_cues(&self) -> Result<(), ProjectValidation> {
        for (index, cue) in self.cues.iter().enumerate() {
            if cue.cue_id == 0
                || cue.target_scene_id == 0
                || !self
                    .scenes
                    .iter()
                    .any(|scene| scene.instance_id == cue.target_scene_id)
                || !is_stable_name(&cue.parameter)
                || !cue.start_seconds.is_finite()
                || cue.start_seconds < 0.0
                || !cue.end_seconds.is_finite()
                || cue.end_seconds <= cue.start_seconds
                || cue.end_seconds > self.audio.duration_seconds
                || !cue.from_value.is_finite()
                || !cue.to_value.is_finite()
            {
                return Err(fail(ProjectError::Cue, index, 0));
            }
            if index > 0 {
                let previous = &self.cues[index - 1];
                if cue.start_seconds < previous.start_seconds
                    || (cue.start_seconds == previous.start_seconds
                        && cue.cue_id <= previous.cue_id)
                {
                    return Err(fail(ProjectError::CueOrder, index, index - 1));
                }
            }
            for (at, previous) in self.cues[..index].iter().enumerate() {
                if previous.cue_id == cue.cue_id {
                    return Err(fail(ProjectError::DuplicateId, index, at));
                }
                // Half-open ranges, so touching at a boundary is not an overlap.
                if previous.target_scene_id == cue.target_scene_id
                    && previous.parameter == cue.parameter
                    && cue.start_seconds < previous.end_seconds
                {
                    return Err(fail(ProjectError::CueOverlap, index, at));
                }
            }
        }
        Ok(())
    }

    fn validate_analysis_lanes(&self) -> Result<(), ProjectValidation> {
        for (index, lane) in self.analysis_lanes.iter().enumerate() {
            let provenance = &lane.provenance;
            if !is_bounded(&lane.path, capacity::PATH, false)
                || !sha256::is_hex_digest(&lane.sha256)
                || !sha256::is_hex_digest(&lane.audio_sha256)
                || lane.audio_sha256 != self.audio.sha256
                || !is_stable_name(&provenance.adapter)
                || !is_bounded(&provenance.adapter_version, capacity::VERSION, false)
                || !is_bounded(&provenance.schema_version, capacity::VERSION, false)
                || !is_bounded(&provenance.model, capacity::PROVIDER, true)
                || !is_bounded(&provenance.provider, capacity::PROVIDER, true)
                || !is_bounded(&provenance.prompt_version, capacity::VERSION, true)
            {
                return Err(fail(ProjectError::AnalysisLane, index, 0));
            }
            if let Some(previous) = self.analysis_lanes[..index]
                .iter()
                .position(|other| other.kind == lane.kind)
            {
                return Err(fail(ProjectError::AnalysisLane, index, previous));
            }
        }
        Ok(())
    }

    fn validate_scene_switches(&self) -> Result<(), ProjectValidation> {
        let switches = &self.scene_switches;
        if switches.cues.len() > SCENE_SWITCH_CAPACITY
            || (switches.enabled && switches.cues.is_empty())
        {
            return Err(fail(ProjectError::SceneSwitch, 0, 0));
        }
        // Nonempty cues form contiguous full-duration coverage, within 1 ms of
        // drift (`project.c:315-343`).
        let mut cursor = 0.0f64;
        for (index, cue) in switches.cues.iter().enumerate() {
            if cue.id == 0
                || !is_stable_name(&cue.scene_name)
                || !cue.start_seconds.is_finite()
                || !cue.end_seconds.is_finite()
                || cue.start_seconds < 0.0
                || cue.end_seconds <= cue.start_seconds
                || cue.end_seconds > self.audio.duration_seconds
                || (cue.start_seconds - cursor).abs() > 0.001
                || !cue.strength.is_finite()
                || cue.strength < 0.0
                || cue.strength > 1.0
                || cue.settings.len() > MAX_CONTROLS
            {
                return Err(fail(ProjectError::SceneSwitch, index, 0));
            }
            if let Some(at) = cue.settings.iter().position(|value| !value.is_finite()) {
                return Err(fail(ProjectError::SceneSwitch, index, at));
            }
            if let Some(previous) = switches.cues[..index]
                .iter()
                .position(|other| other.id == cue.id)
            {
                return Err(fail(ProjectError::DuplicateId, index, previous));
            }
            cursor = cue.end_seconds;
        }
        if !switches.cues.is_empty() && (cursor - self.audio.duration_seconds).abs() > 0.001 {
            return Err(fail(ProjectError::SceneSwitch, switches.cues.len() - 1, 0));
        }
        Ok(())
    }

    fn validate_scene_presets(&self) -> Result<(), ProjectValidation> {
        if self.scene_presets.len() > MAX_SCENE_PRESETS {
            return Err(fail(ProjectError::ScenePreset, 0, 0));
        }
        for (index, preset) in self.scene_presets.iter().enumerate() {
            if preset.id == 0
                || !is_stable_name(&preset.scene_name)
                || !is_bounded(&preset.name, capacity::NAME, false)
                || preset.settings.is_empty()
                || preset.settings.len() > MAX_CONTROLS
            {
                return Err(fail(ProjectError::ScenePreset, index, 0));
            }
            if let Some(at) = preset.settings.iter().position(|value| !value.is_finite()) {
                return Err(fail(ProjectError::ScenePreset, index, at));
            }
            if let Some(previous) = self.scene_presets[..index]
                .iter()
                .position(|other| other.id == preset.id)
            {
                return Err(fail(ProjectError::DuplicateId, index, previous));
            }
        }
        Ok(())
    }

    fn validate_event_lanes(&self) -> Result<(), ProjectValidation> {
        if self.manual_events.validate().is_err() {
            return Err(fail(ProjectError::ManualEvent, 0, 0));
        }
        if let Some(index) = self
            .manual_events
            .events()
            .iter()
            .position(|event| event.timestamp_seconds > self.audio.duration_seconds)
        {
            return Err(fail(ProjectError::ManualEvent, index, 0));
        }
        if self.semantic_events.validate().is_err() {
            return Err(fail(ProjectError::SemanticEvent, 0, 0));
        }
        for (index, event) in self.semantic_events.events().iter().enumerate() {
            // The semantic lane's four values are bounded per position:
            // energy 0..1, tension 0..1, valence -1..1, confidence 0..1
            // (`project.c:378-388`, `project-v1.schema.json:859-912`).
            let bounded = event.value_count == 4
                && (0.0..=1.0).contains(&event.values[0])
                && (0.0..=1.0).contains(&event.values[1])
                && (-1.0..=1.0).contains(&event.values[2])
                && (0.0..=1.0).contains(&event.values[3]);
            if event.timestamp_seconds > self.audio.duration_seconds
                || event.event_type != EventType::Semantic as u32
                || !bounded
            {
                return Err(fail(ProjectError::SemanticEvent, index, 0));
            }
        }
        Ok(())
    }

    /// `musi_project_editor_support` (`project.c:435-480`).
    pub fn editor_support(&self) -> Result<(), EditorSupport> {
        // The editor only ever writes an imported face into the sibling asset
        // bundle, so it can only round-trip a project-relative descendant. An
        // absolute or traversing path is a face this build would fail to resolve,
        // typeset in the fallback, and then autosave over the author's choice —
        // exactly the silent normalization this check exists to prevent.
        if let Some(font) = &self.caption_style.font {
            if !is_bundled_relative_path(&font.path) {
                return Err(EditorSupport::CaptionFont);
            }
            if !font.licence_path.is_empty() && !is_bundled_relative_path(&font.licence_path) {
                return Err(EditorSupport::CaptionFont);
            }
        }
        if self.output.start_seconds.abs() > 0.000_001
            || (self.output.end_seconds - self.audio.duration_seconds).abs() > 0.000_001
        {
            return Err(EditorSupport::OutputRange);
        }
        if self.output.format != OutputFormat::Mp4H264 || self.output.fps_denominator != 1 {
            return Err(EditorSupport::OutputFormat);
        }
        if self.scenes.len() != 1 {
            return Err(EditorSupport::SceneCount);
        }
        if !self.cues.is_empty() {
            return Err(EditorSupport::ParameterCues);
        }
        let scene = &self.scenes[0];
        if !scene.enabled
            || scene.start_seconds.abs() > 0.000_001
            || (scene.end_seconds - self.audio.duration_seconds).abs() > 0.000_001
            || (scene.opacity - 1.0).abs() > 0.000_001
            || scene.blend_mode != BlendMode::Normal
        {
            return Err(EditorSupport::SceneLayout);
        }
        if !scene.mappings.is_empty() && !mappings_supported(&scene.mappings) {
            return Err(EditorSupport::SceneMappings);
        }
        Ok(())
    }

    /// `musi_project_audio_metadata_matches` (`project.c:500-514`).
    ///
    /// The decoded duration is compared within a tolerance because decoders round;
    /// the sample rate and channel count must match exactly, because a difference
    /// there means a different file.
    #[must_use]
    pub fn audio_metadata_matches(
        &self,
        decoded_duration_seconds: f64,
        decoded_sample_rate: u32,
        decoded_channels: u16,
        duration_tolerance_seconds: f64,
    ) -> bool {
        decoded_duration_seconds.is_finite()
            && decoded_duration_seconds > 0.0
            && duration_tolerance_seconds.is_finite()
            && duration_tolerance_seconds >= 0.0
            && (decoded_duration_seconds - self.audio.duration_seconds).abs()
                <= duration_tolerance_seconds
            && decoded_sample_rate == self.audio.sample_rate
            && decoded_channels == self.audio.channels
    }

    /// `musi_project_parameter_at` (`project.c:568-599`): the automated value of
    /// one parameter at one time, starting from `base_value`.
    ///
    /// The scan relies on cues being sorted and non-overlapping per
    /// `(scene, parameter)`, both of which [`Self::validate`] guarantees. Past a
    /// cue's end its `to_value` persists, which is why the loop keeps walking
    /// instead of stopping at the first non-match.
    #[must_use]
    pub fn parameter_at(
        &self,
        target_scene_id: u64,
        parameter: &str,
        base_value: f64,
        time_seconds: f64,
    ) -> Option<f64> {
        if target_scene_id == 0
            || !base_value.is_finite()
            || !time_seconds.is_finite()
            || time_seconds < 0.0
            || self.cues.len() > MAX_CUES
        {
            return None;
        }
        let mut value = base_value;
        for cue in &self.cues {
            if cue.target_scene_id != target_scene_id || cue.parameter != parameter {
                continue;
            }
            if time_seconds < cue.start_seconds {
                break;
            }
            if time_seconds >= cue.end_seconds {
                value = cue.to_value;
                continue;
            }
            let amount = (time_seconds - cue.start_seconds) / (cue.end_seconds - cue.start_seconds);
            value = interpolate(cue.from_value, cue.to_value, amount, cue.interpolation)?;
            return value.is_finite().then_some(value);
        }
        value.is_finite().then_some(value)
    }
}

/// `musi_interpolate` (`project.c:530-540`).
///
/// `None` where C returns NaN. The amount is clamped to `0..1` *before* shaping,
/// which is what makes `step` fire only at exactly 1.0.
#[must_use]
pub fn interpolate(
    from_value: f64,
    to_value: f64,
    amount: f64,
    interpolation: Interpolation,
) -> Option<f64> {
    if !from_value.is_finite() || !to_value.is_finite() || !amount.is_finite() {
        return None;
    }
    let shaped = interpolation.shape(amount.clamp(0.0, 1.0));
    Some(from_value + (to_value - from_value) * shaped)
}

/// A path the sibling asset bundle could actually contain: relative, no drive
/// letter, no traversal, no empty or `"."` component (`bundled_relative_path`,
/// `project.c:414-433`).
///
/// Deliberately stricter than the resolver, because it answers "can the editor
/// re-save this?" without a filesystem to consult.
#[must_use]
pub fn is_bundled_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }
    path.split(['/', '\\'])
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

// -- The persistence half of `core::scene::routes`, and where it went ---------
//
// `mapping_is_constant`, `constant_mapping`, `mappings_supported`,
// `export_mappings`, `import_mappings` and `parse_route_spec` used to live here,
// because they need this module's canonical names and bounds. They now live in
// [`crate::scene::routes`], next to the evaluation they persist.
//
// **They moved wholesale, and they must stay together.** The constant rule and
// the route rule are two halves of one decision — `.musi` v1 spells a slider
// value as a full-range RMS mapping with equal output endpoints, so a parameter
// is persisted as *either* a constant *or* a route and never both. Splitting them
// across two modules is what would let one parameter acquire both spellings, an
// ambiguity the format cannot represent.
//
// [`MAX_MAPPINGS_PER_SCENE`] and [`capacity::PARAMETER`] stay here: they are
// `.musi` schema bounds (`project.h:25`, `:36`), not route semantics, and the
// validator above reads them too.

/// Synthetic fixtures shared by this module's tests and the codec's.
///
/// There are **no `.musi` fixture files** in the frozen tree to copy — zero in
/// git, zero outside the gitignored `build/`. The C suite builds its
/// compatibility fixtures inline, by serializing a project and then editing the
/// text (`tests/test_project_io.c`), and so does this one. Nothing synthetic gets
/// committed and no fixture can drift out of step with the model.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn digest(seed: u8) -> String {
        sha256::digest_hex(&[seed])
    }

    /// The smallest project the validator accepts. Tests fill it in, then break
    /// exactly one field.
    pub(crate) fn valid_project() -> Project {
        let audio_sha = digest(1);
        let mut project = Project {
            metadata: Metadata {
                project_id: "fixture".into(),
                title: "Fixture".into(),
                application_version: "2026.07".into(),
                ..Metadata::default()
            },
            audio: AudioAsset {
                mode: AssetMode::Imported,
                path: "fixture.assets/audio/song.wav".into(),
                sha256: audio_sha,
                duration_seconds: 60.0,
                sample_rate: 44_100,
                channels: 2,
            },
            output: OutputSettings {
                end_seconds: 60.0,
                ..OutputSettings::default()
            },
            scenes: vec![SceneEntry {
                instance_id: 1,
                scene_type: "spectrum".into(),
                enabled: true,
                start_seconds: 0.0,
                end_seconds: 60.0,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mappings: Vec::new(),
            }],
            ..Project::default()
        };
        project.lyrics = LyricsDocument::new(60.0).unwrap();
        project
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{digest, valid_project};
    use super::*;
    use crate::project::lyrics::LyricCue;
    use crate::scene::events::EventRecord;

    /// One mutation that should break an otherwise valid fixture. Named because
    /// the table-driven tests below read better than nine near-identical tests,
    /// and because clippy is right that the bare `fn` type is unreadable inline.
    type Breaker = fn(&mut Project);
    type StyleBreaker = fn(&mut CaptionStyle);
    type FontBreaker = fn(&mut FontAsset);

    #[test]
    fn the_fixture_project_is_valid_and_editor_supported() {
        let project = valid_project();
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(project.editor_support(), Ok(()));
    }

    #[test]
    fn the_schema_version_is_checked_before_anything_else() {
        let mut project = valid_project();
        project.schema_version = 2;
        project.metadata.title = String::new();
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::SchemaVersion
        );
    }

    #[test]
    fn metadata_bounds_are_bytes_not_characters() {
        let mut project = valid_project();
        // 43 three-byte characters is 129 bytes: one over the 128-byte ceiling,
        // and far under 128 characters. A chars().count() check would accept it.
        project.metadata.title = "→".repeat(43);
        assert_eq!(project.metadata.title.len(), 129);
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::Metadata
        );
        project.metadata.title = "→".repeat(42);
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn metadata_requires_a_stable_project_id_and_a_title() {
        for project_id in ["", ".leading-dot", "has space", "-dash", &"a".repeat(65)] {
            let mut project = valid_project();
            project.project_id_for_test(project_id);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::Metadata,
                "accepted project_id {project_id:?}"
            );
        }
        let mut project = valid_project();
        project.metadata.title = String::new();
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::Metadata
        );
        // Optional fields may be empty but not oversized.
        let mut project = valid_project();
        project.metadata.author = String::new();
        assert_eq!(project.validate(), Ok(()));
        project.metadata.created_utc = "x".repeat(33);
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::Metadata
        );
    }

    #[test]
    fn audio_bounds_are_checked_field_by_field() {
        let breakers: Vec<(&str, Breaker)> = vec![
            ("empty path", |project| project.audio.path = String::new()),
            ("uppercase digest", |project| {
                project.audio.sha256 = project.audio.sha256.to_uppercase();
            }),
            ("short digest", |project| {
                project.audio.sha256.truncate(63);
            }),
            ("zero duration", |project| {
                project.audio.duration_seconds = 0.0;
            }),
            ("nan duration", |project| {
                project.audio.duration_seconds = f64::NAN;
            }),
            ("zero sample rate", |project| project.audio.sample_rate = 0),
            ("huge sample rate", |project| {
                project.audio.sample_rate = 768_001;
            }),
            ("zero channels", |project| project.audio.channels = 0),
            ("too many channels", |project| project.audio.channels = 65),
        ];
        for (name, break_it) in breakers {
            let mut project = valid_project();
            break_it(&mut project);
            let error = project.validate().unwrap_err().error;
            assert!(
                matches!(error, ProjectError::Audio | ProjectError::AnalysisLane),
                "{name} produced {error:?}"
            );
        }
    }

    #[test]
    fn output_settings_bound_resolution_frame_rate_and_range() {
        let breakers: Vec<(&str, Breaker)> = vec![
            ("width too small", |project| project.output.width = 15),
            ("width too large", |project| project.output.width = 16_385),
            ("height too small", |project| project.output.height = 15),
            ("zero fps numerator", |project| {
                project.output.fps_numerator = 0;
            }),
            ("zero fps denominator", |project| {
                project.output.fps_denominator = 0;
            }),
            ("fps denominator too large", |project| {
                project.output.fps_denominator = 1002;
            }),
            ("fps above 240", |project| {
                project.output.fps_numerator = 241;
                project.output.fps_denominator = 1;
            }),
            ("negative start", |project| {
                project.output.start_seconds = -0.001;
            }),
            ("end before start", |project| {
                project.output.start_seconds = 30.0;
                project.output.end_seconds = 30.0;
            }),
            ("end past the audio", |project| {
                project.output.end_seconds = 60.001;
            }),
        ];
        for (name, break_it) in breakers {
            let mut project = valid_project();
            break_it(&mut project);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::Output,
                "{name}"
            );
        }
        // Exactly 240 fps as a rational is legal.
        let mut project = valid_project();
        project.output.fps_numerator = 240_240;
        project.output.fps_denominator = 1001;
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn a_project_needs_at_least_one_scene_and_at_most_thirty_two() {
        let mut project = valid_project();
        project.scenes.clear();
        assert_eq!(project.validate().unwrap_err().error, ProjectError::Count);

        let mut project = valid_project();
        let template = project.scenes[0].clone();
        for index in 1..=MAX_SCENES {
            project.scenes.push(SceneEntry {
                instance_id: index as u64 + 1,
                ..template.clone()
            });
        }
        assert_eq!(project.scenes.len(), MAX_SCENES + 1);
        assert_eq!(project.validate().unwrap_err().error, ProjectError::Count);
    }

    #[test]
    fn duplicate_scene_instance_ids_are_refused() {
        let mut project = valid_project();
        let template = project.scenes[0].clone();
        project.scenes.push(template);
        let failure = project.validate().unwrap_err();
        assert_eq!(failure.error, ProjectError::DuplicateId);
        assert_eq!((failure.index, failure.subindex), (1, 0));
    }

    #[test]
    fn a_scene_must_fit_inside_the_audio() {
        let breakers: Vec<(&str, Breaker)> = vec![
            ("zero id", |project| project.scenes[0].instance_id = 0),
            ("bad type", |project| {
                project.scenes[0].scene_type = "not a name".into();
            }),
            ("negative start", |project| {
                project.scenes[0].start_seconds = -1.0;
            }),
            ("end before start", |project| {
                project.scenes[0].end_seconds = 0.0;
            }),
            ("end past the audio", |project| {
                project.scenes[0].end_seconds = 60.001;
            }),
            ("opacity above one", |project| {
                project.scenes[0].opacity = 1.001;
            }),
            ("opacity not finite", |project| {
                project.scenes[0].opacity = f64::NAN;
            }),
        ];
        for (name, break_it) in breakers {
            let mut project = valid_project();
            break_it(&mut project);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::Scene,
                "{name}"
            );
        }
    }

    fn mapping(parameter: &str) -> ParameterMapping {
        crate::scene::routes::constant_mapping(parameter, 0.5)
    }

    #[test]
    fn mapping_validation_is_weaker_than_the_route_rule_on_purpose() {
        let mut project = valid_project();
        // Equal output endpoints: a slider constant, valid in the model.
        project.scenes[0].mappings = vec![mapping("settings.spectrum.amplitude")];
        assert_eq!(project.validate(), Ok(()));

        // But an unresolvable parameter name is still a valid *model* mapping, as
        // long as it is a stable_name. The editor-support check is what rejects it.
        project.scenes[0].mappings = vec![mapping("settings.unknown.thing")];
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(
            project.editor_support().unwrap_err(),
            EditorSupport::SceneMappings
        );
    }

    #[test]
    fn mapping_bounds_and_duplicates_are_refused() {
        let breakers: Vec<(&str, Breaker)> = vec![
            ("bad parameter name", |project| {
                project.scenes[0].mappings = vec![mapping("not a name")];
            }),
            ("band index without band source", |project| {
                let mut broken = mapping("settings.spectrum.amplitude");
                broken.band_index = 3;
                project.scenes[0].mappings = vec![broken];
            }),
            ("degenerate input range", |project| {
                let mut broken = mapping("settings.spectrum.amplitude");
                broken.input_max = broken.input_min;
                project.scenes[0].mappings = vec![broken];
            }),
            ("non-finite output", |project| {
                let mut broken = mapping("settings.spectrum.amplitude");
                broken.output_max = f64::INFINITY;
                project.scenes[0].mappings = vec![broken];
            }),
            ("duplicate parameter", |project| {
                project.scenes[0].mappings = vec![
                    mapping("settings.spectrum.amplitude"),
                    mapping("settings.spectrum.amplitude"),
                ];
            }),
        ];
        for (name, break_it) in breakers {
            let mut project = valid_project();
            break_it(&mut project);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::Mapping,
                "{name}"
            );
        }
    }

    fn cue(cue_id: u64, parameter: &str, start: f64, end: f64) -> ParameterCue {
        ParameterCue {
            cue_id,
            target_scene_id: 1,
            parameter: parameter.into(),
            start_seconds: start,
            end_seconds: end,
            from_value: 0.0,
            to_value: 1.0,
            interpolation: Interpolation::Linear,
        }
    }

    #[test]
    fn cues_must_be_sorted_unique_and_non_overlapping() {
        let mut project = valid_project();
        project.cues = vec![cue(1, "settings.spectrum.amplitude", 0.0, 1.0)];
        assert_eq!(project.validate(), Ok(()));

        // Touching at a boundary is not an overlap: the ranges are half-open.
        project.cues = vec![
            cue(1, "settings.spectrum.amplitude", 0.0, 1.0),
            cue(2, "settings.spectrum.amplitude", 1.0, 2.0),
        ];
        assert_eq!(project.validate(), Ok(()));

        project.cues = vec![
            cue(1, "settings.spectrum.amplitude", 0.0, 2.0),
            cue(2, "settings.spectrum.amplitude", 1.0, 3.0),
        ];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::CueOverlap
        );

        // A different parameter may overlap freely.
        project.cues = vec![
            cue(1, "settings.spectrum.amplitude", 0.0, 2.0),
            cue(2, "settings.spectrum.trail", 1.0, 3.0),
        ];
        assert_eq!(project.validate(), Ok(()));

        // Unsorted.
        project.cues = vec![
            cue(1, "settings.spectrum.amplitude", 5.0, 6.0),
            cue(2, "settings.spectrum.trail", 1.0, 2.0),
        ];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::CueOrder
        );

        // Equal starts must be ordered by ascending cue id.
        project.cues = vec![
            cue(5, "settings.spectrum.amplitude", 1.0, 2.0),
            cue(3, "settings.spectrum.trail", 1.0, 2.0),
        ];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::CueOrder
        );

        // A cue must target a scene that exists.
        project.cues = vec![ParameterCue {
            target_scene_id: 99,
            ..cue(1, "settings.spectrum.amplitude", 0.0, 1.0)
        }];
        assert_eq!(project.validate().unwrap_err().error, ProjectError::Cue);
    }

    #[test]
    fn parameter_at_persists_the_last_to_value_past_a_cue_end() {
        let mut project = valid_project();
        project.cues = vec![cue(1, "settings.spectrum.amplitude", 10.0, 20.0)];
        let at = |time| {
            project
                .parameter_at(1, "settings.spectrum.amplitude", 0.25, time)
                .unwrap()
        };
        assert_eq!(at(0.0), 0.25, "before the cue, the base value holds");
        assert_eq!(at(10.0), 0.0, "at the start, from_value");
        assert_eq!(at(15.0), 0.5, "half way, linear");
        assert_eq!(at(20.0), 1.0, "at the end, to_value persists");
        assert_eq!(at(59.0), 1.0, "and keeps persisting");
        assert_eq!(
            project.parameter_at(1, "settings.spectrum.trail", 0.25, 15.0),
            Some(0.25),
            "an unrelated parameter is untouched"
        );
        assert_eq!(project.parameter_at(0, "x", 0.0, 0.0), None);
        assert_eq!(project.parameter_at(1, "x", f64::NAN, 0.0), None);
        assert_eq!(project.parameter_at(1, "x", 0.0, -1.0), None);
    }

    #[test]
    fn interpolation_clamps_the_amount_before_shaping() {
        assert_eq!(
            interpolate(0.0, 1.0, -1.0, Interpolation::Linear),
            Some(0.0)
        );
        assert_eq!(interpolate(0.0, 1.0, 2.0, Interpolation::Linear), Some(1.0));
        // Step only fires at exactly 1.0, which is why the clamp matters.
        assert_eq!(interpolate(0.0, 1.0, 0.999, Interpolation::Step), Some(0.0));
        assert_eq!(interpolate(0.0, 1.0, 1.0, Interpolation::Step), Some(1.0));
        assert_eq!(
            interpolate(0.0, 1.0, 0.5, Interpolation::Smoothstep),
            Some(0.5)
        );
        assert_eq!(
            interpolate(0.0, 1.0, 0.5, Interpolation::EaseIn),
            Some(0.25)
        );
        assert_eq!(
            interpolate(0.0, 1.0, 0.5, Interpolation::EaseOut),
            Some(0.75)
        );
        assert_eq!(interpolate(f64::NAN, 1.0, 0.5, Interpolation::Linear), None);
        assert_eq!(interpolate(0.0, 1.0, f64::NAN, Interpolation::Linear), None);
    }

    #[test]
    fn analysis_lanes_must_name_this_projects_audio_and_be_unique_per_kind() {
        let mut project = valid_project();
        let lane = AnalysisLaneReference {
            kind: AnalysisLaneKind::MeasuredSignal,
            path: "analysis/measured.json".into(),
            sha256: digest(2),
            audio_sha256: project.audio.sha256.clone(),
            provenance: Provenance {
                adapter: "measured".into(),
                adapter_version: "1".into(),
                schema_version: "measured-analysis-v1".into(),
                ..Provenance::default()
            },
        };
        project.analysis_lanes = vec![lane.clone()];
        assert_eq!(project.validate(), Ok(()));

        // A lane for a different track is refused. This is the check that stops
        // analysis being attached to the wrong audio.
        project.analysis_lanes = vec![AnalysisLaneReference {
            audio_sha256: digest(9),
            ..lane.clone()
        }];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::AnalysisLane
        );

        // Two lanes of the same kind are refused.
        project.analysis_lanes = vec![lane.clone(), lane.clone()];
        let failure = project.validate().unwrap_err();
        assert_eq!(failure.error, ProjectError::AnalysisLane);
        assert_eq!((failure.index, failure.subindex), (1, 0));

        // Two lanes of different kinds are fine.
        project.analysis_lanes = vec![
            lane.clone(),
            AnalysisLaneReference {
                kind: AnalysisLaneKind::SemanticScore,
                ..lane
            },
        ];
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn scene_switch_cues_must_cover_the_whole_track_contiguously() {
        let mut project = valid_project();
        let switch = |id, start, end| SceneSwitchSuggestion {
            id,
            start_seconds: start,
            end_seconds: end,
            scene_name: "spectrum".into(),
            strength: 0.5,
            settings: Vec::new(),
        };
        project.scene_switches.cues = vec![switch(1, 0.0, 30.0), switch(2, 30.0, 60.0)];
        assert_eq!(project.validate(), Ok(()));

        // A gap.
        project.scene_switches.cues = vec![switch(1, 0.0, 20.0), switch(2, 30.0, 60.0)];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::SceneSwitch
        );

        // Short of the end.
        project.scene_switches.cues = vec![switch(1, 0.0, 30.0)];
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::SceneSwitch
        );

        // Enabled with nothing to switch to.
        project.scene_switches = SceneSwitchSuggestions {
            enabled: true,
            cues: Vec::new(),
        };
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::SceneSwitch
        );
    }

    #[test]
    fn semantic_events_are_bounded_per_value_position() {
        let semantic = |values: [f32; 4]| EventRecord {
            timestamp_seconds: 1.0,
            id: 1,
            event_type: EventType::Semantic as u32,
            value_count: 4,
            values,
        };
        let mut project = valid_project();
        project
            .semantic_events
            .record(semantic([0.5, 0.5, -1.0, 1.0]))
            .unwrap();
        assert_eq!(project.validate(), Ok(()));

        for values in [
            [1.5, 0.5, 0.0, 0.5],
            [0.5, 1.5, 0.0, 0.5],
            [0.5, 0.5, -1.5, 0.5],
            [0.5, 0.5, 1.5, 0.5],
            [0.5, 0.5, 0.0, 1.5],
            [-0.5, 0.5, 0.0, 0.5],
        ] {
            let mut project = valid_project();
            project.semantic_events.record(semantic(values)).unwrap();
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::SemanticEvent,
                "{values:?}"
            );
        }

        // The lane accepts only semantic events, with exactly four values.
        let mut project = valid_project();
        project
            .semantic_events
            .record(EventRecord {
                event_type: EventType::Cue as u32,
                ..semantic([0.5; 4])
            })
            .unwrap();
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::SemanticEvent
        );
    }

    #[test]
    fn manual_events_may_not_start_past_the_audio() {
        let mut project = valid_project();
        project
            .manual_events
            .record(EventRecord {
                timestamp_seconds: 60.001,
                id: 1,
                event_type: EventType::Cue as u32,
                value_count: 1,
                values: [1.0, 0.0, 0.0, 0.0],
            })
            .unwrap();
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::ManualEvent
        );
    }

    #[test]
    fn lyrics_must_agree_with_the_audio_duration() {
        let mut project = valid_project();
        project.lyrics = LyricsDocument::new(59.0).unwrap();
        assert_eq!(project.validate().unwrap_err().error, ProjectError::Lyrics);

        let mut project = valid_project();
        project
            .lyrics
            .insert(LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "line".into(),
                origin: Default::default(),
            })
            .unwrap();
        assert_eq!(project.validate(), Ok(()));
    }

    // -- caption style ------------------------------------------------------

    fn font_asset() -> FontAsset {
        FontAsset {
            path: "fixture.assets/fonts/face.ttf".into(),
            sha256: digest(3),
            family: "Some Family".into(),
            licence_path: "fixture.assets/fonts/OFL.txt".into(),
            licence_sha256: digest(4),
            licence_name: "OFL-1.1".into(),
        }
    }

    #[test]
    fn the_shipped_caption_default_is_the_pre_caption_appearance() {
        let style = CaptionStyle::default();
        assert!(style.is_default());
        assert_eq!(style.face, CaptionFace::Alegreya);
        assert_eq!(style.box_style, CaptionBox::Plate);
        assert_eq!(style.anchor, CaptionAnchor::BottomCenter);
        assert_eq!(style.size_scale, 0.047);
        assert_eq!(style.margin_scale, 0.065);
        assert_eq!(style.width_scale, 0.82);
        assert_eq!(style.text_rgba, 0xFFFF_FFFF);
        // 0xB7, not 0xB8: raylib truncated 255*0.72 and every existing project
        // was authored against the truncated value.
        assert_eq!(style.box_rgba, 0x0000_00B7);
        assert!(style.font.is_none());
    }

    #[test]
    fn caption_measurements_are_bounded_fractions_of_the_frame() {
        let cases: Vec<(&str, StyleBreaker)> = vec![
            ("size below minimum", |style| style.size_scale = 0.0119),
            ("size above maximum", |style| style.size_scale = 0.3001),
            ("size not finite", |style| style.size_scale = f64::NAN),
            ("margin below minimum", |style| style.margin_scale = -0.001),
            ("margin above maximum", |style| style.margin_scale = 0.4001),
            ("width below minimum", |style| style.width_scale = 0.1999),
            ("width above maximum", |style| style.width_scale = 1.0001),
        ];
        for (name, break_it) in cases {
            let mut project = valid_project();
            break_it(&mut project.caption_style);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::CaptionStyle,
                "{name}"
            );
        }
        // The endpoints themselves are legal.
        for (size, margin, width) in [(0.012, 0.0, 0.20), (0.300, 0.400, 1.00)] {
            let mut project = valid_project();
            project.caption_style.size_scale = size;
            project.caption_style.margin_scale = margin;
            project.caption_style.width_scale = width;
            assert_eq!(project.validate(), Ok(()));
        }
    }

    #[test]
    fn an_imported_face_and_its_font_asset_are_one_fact_stated_twice() {
        // Imported without the asset: captions cannot be reproduced from the file.
        let mut project = valid_project();
        project.caption_style.face = CaptionFace::Imported;
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::CaptionStyle
        );

        // The asset without the imported face: equally invalid, the other way.
        let mut project = valid_project();
        project.caption_style.font = Some(font_asset());
        assert_eq!(
            project.validate().unwrap_err().error,
            ProjectError::CaptionStyle
        );

        // Both, agreeing.
        let mut project = valid_project();
        project.caption_style.face = CaptionFace::Imported;
        project.caption_style.font = Some(font_asset());
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn a_bundled_licence_is_all_three_fields_or_none() {
        let mut project = valid_project();
        project.caption_style.face = CaptionFace::Imported;

        // All three empty: a face from the user's own disk, whose terms this
        // application cannot assert. Legal.
        project.caption_style.font = Some(FontAsset {
            licence_path: String::new(),
            licence_sha256: String::new(),
            licence_name: String::new(),
            ..font_asset()
        });
        assert_eq!(project.validate(), Ok(()));

        let breakers: Vec<(&str, FontBreaker)> = vec![
            ("path without digest", |font| {
                font.licence_sha256 = String::new();
            }),
            ("digest without path", |font| {
                font.licence_path = String::new();
            }),
            ("licence without a name", |font| {
                font.licence_name = String::new();
            }),
            ("face digest missing", |font| font.sha256 = String::new()),
            ("face path missing", |font| font.path = String::new()),
            ("family missing", |font| font.family = String::new()),
            ("uppercase licence digest", |font| {
                font.licence_sha256 = font.licence_sha256.to_uppercase();
            }),
        ];
        for (name, break_it) in breakers {
            let mut project = valid_project();
            project.caption_style.face = CaptionFace::Imported;
            let mut font = font_asset();
            break_it(&mut font);
            project.caption_style.font = Some(font);
            assert_eq!(
                project.validate().unwrap_err().error,
                ProjectError::CaptionStyle,
                "{name}"
            );
        }
    }

    #[test]
    fn the_editor_refuses_a_caption_face_outside_the_bundle() {
        for path in [
            "/absolute/face.ttf",
            "../escape/face.ttf",
            "./face.ttf",
            "C:/face.ttf",
            "double//slash.ttf",
        ] {
            let mut project = valid_project();
            project.caption_style.face = CaptionFace::Imported;
            project.caption_style.font = Some(FontAsset {
                path: path.into(),
                ..font_asset()
            });
            assert_eq!(project.validate(), Ok(()), "{path} should still be valid");
            assert_eq!(
                project.editor_support().unwrap_err(),
                EditorSupport::CaptionFont,
                "{path}"
            );
        }
    }

    #[test]
    fn bundled_relative_paths_reject_traversal_and_roots() {
        assert!(is_bundled_relative_path("show.assets/fonts/face.ttf"));
        assert!(is_bundled_relative_path("face.ttf"));
        assert!(!is_bundled_relative_path(""));
        assert!(!is_bundled_relative_path("/face.ttf"));
        assert!(!is_bundled_relative_path("\\face.ttf"));
        assert!(!is_bundled_relative_path("C:/face.ttf"));
        assert!(!is_bundled_relative_path("../face.ttf"));
        assert!(!is_bundled_relative_path("a/../face.ttf"));
        assert!(!is_bundled_relative_path("a/./face.ttf"));
        assert!(!is_bundled_relative_path("a//face.ttf"));
        assert!(!is_bundled_relative_path("a/"));
    }

    // -- editor support ------------------------------------------------------

    #[test]
    fn editor_support_names_every_thing_it_cannot_round_trip() {
        let cases: Vec<(&str, EditorSupport, Breaker)> = vec![
            ("partial range", EditorSupport::OutputRange, |project| {
                project.output.start_seconds = 1.0;
            }),
            ("non-mp4", EditorSupport::OutputFormat, |project| {
                project.output.format = OutputFormat::WebmVp9;
            }),
            ("fractional fps", EditorSupport::OutputFormat, |project| {
                project.output.fps_numerator = 30_000;
                project.output.fps_denominator = 1001;
            }),
            ("two scenes", EditorSupport::SceneCount, |project| {
                let mut second = project.scenes[0].clone();
                second.instance_id = 2;
                project.scenes.push(second);
            }),
            ("automation", EditorSupport::ParameterCues, |project| {
                project.cues = vec![cue(1, "settings.spectrum.amplitude", 0.0, 1.0)];
            }),
            ("disabled scene", EditorSupport::SceneLayout, |project| {
                project.scenes[0].enabled = false;
            }),
            ("translucent scene", EditorSupport::SceneLayout, |project| {
                project.scenes[0].opacity = 0.5;
            }),
            ("additive blend", EditorSupport::SceneLayout, |project| {
                project.scenes[0].blend_mode = BlendMode::Add;
            }),
            (
                "partial scene span",
                EditorSupport::SceneLayout,
                |project| {
                    project.scenes[0].end_seconds = 30.0;
                },
            ),
        ];
        for (name, expected, break_it) in cases {
            let mut project = valid_project();
            break_it(&mut project);
            assert_eq!(project.validate(), Ok(()), "{name} must stay valid");
            assert_eq!(project.editor_support().unwrap_err(), expected, "{name}");
        }
    }

    #[test]
    fn audio_metadata_matching_tolerates_duration_but_not_format() {
        let project = valid_project();
        assert!(project.audio_metadata_matches(60.0, 44_100, 2, 0.0));
        assert!(project.audio_metadata_matches(60.05, 44_100, 2, 0.1));
        assert!(!project.audio_metadata_matches(60.2, 44_100, 2, 0.1));
        assert!(!project.audio_metadata_matches(60.0, 48_000, 2, 0.1));
        assert!(!project.audio_metadata_matches(60.0, 44_100, 1, 0.1));
        assert!(!project.audio_metadata_matches(0.0, 44_100, 2, 0.1));
        assert!(!project.audio_metadata_matches(f64::NAN, 44_100, 2, 0.1));
        assert!(!project.audio_metadata_matches(60.0, 44_100, 2, -1.0));
    }

    #[test]
    fn canonical_enum_names_round_trip() {
        for value in AssetMode::ALL {
            assert_eq!(
                AssetMode::from_canonical_name(value.canonical_name()),
                Some(*value)
            );
        }
        for value in OutputFormat::ALL {
            assert_eq!(
                OutputFormat::from_canonical_name(value.canonical_name()),
                Some(*value)
            );
        }
        for value in CaptionAnchor::ALL {
            assert_eq!(
                CaptionAnchor::from_canonical_name(value.canonical_name()),
                Some(*value)
            );
        }
        assert_eq!(CaptionAnchor::ALL.len(), 9);
        assert_eq!(CaptionAnchor::ALL[0], CaptionAnchor::BottomLeft);
        assert_eq!(CaptionAnchor::ALL[1], CaptionAnchor::BottomCenter);
        assert_eq!(CaptionAnchor::ALL[8], CaptionAnchor::TopRight);
        assert_eq!(
            AnalysisLaneKind::default(),
            AnalysisLaneKind::MeasuredSignal
        );
        assert_eq!(OutputFormat::from_canonical_name("MP4_H264"), None);
    }

    impl Project {
        fn project_id_for_test(&mut self, value: &str) {
            self.metadata.project_id = value.to_owned();
        }
    }
}
