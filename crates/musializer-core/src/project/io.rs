//! The `.musi` codec, its compatibility defaults, and transactional-save policy.
//!
//! **Owner: Agent B.** Port of `../musializer/src/project_io.c/.h`.
//!
//! # Why this is hand-rolled JSON and not serde
//!
//! `musializer-core` does not yet depend on `serde`, so nothing here could use a
//! derive today. That turned out to matter less than expected, because **serde
//! alone does not reproduce this format's semantics** — the plan says so and the C
//! proves it. The codec is *strict, not forward-compatible*:
//!
//! - an unknown field is a hard error ([`ProjectIoError::UnknownField`]), not
//!   something to ignore;
//! - a repeated field is a hard error ([`ProjectIoError::DuplicateField`]);
//! - every string is bounded in **UTF-8 bytes at parse time**, so an over-long
//!   title fails as [`ProjectIoError::String`] rather than surviving into
//!   validation;
//! - every number must be finite, and integers are parsed as integers — `1.0` is
//!   not an acceptable `width`;
//! - arrays are bounded *before* the element is parsed, so an oversized array
//!   fails as [`ProjectIoError::Capacity`] rather than allocating;
//! - compatibility comes from a short list of optional fields with documented
//!   defaults, never from tolerating unknown input.
//!
//! When `serde` is added, the sensible split is: derive the *shape*, keep every
//! rule above as explicit code, and keep this module's tests. A
//! `#[serde(deny_unknown_fields)]` derive would cover two of the six bullets.
//!
//! # What is deliberately not here
//!
//! `project_io.c` also contains the filesystem half of saving: `atomic_write`,
//! `canonicalize_existing_file`, `existing_files_alias`, `bundle_asset`, and the
//! directory `fsync` dance. Those open files, and this crate has none. They belong
//! to `musializer-runtime` (Agent E owns atomic publication). What lives here is
//! the part that is pure and therefore testable: the result enums that carry the
//! guarantees, the transaction filename policy, and the path-shape rules the
//! resolver applies before it ever touches a disk.

use crate::project::assets;
use crate::project::lyrics::{LyricCue, LyricsDocument, CUE_CAPACITY as LYRIC_CUE_CAPACITY};
use crate::project::model::{
    capacity, AnalysisLaneKind, AnalysisLaneReference, AsciiImageAsset, AssetMode, AudioAsset,
    BlendMode, CaptionAnchor, CaptionBox, CaptionEffects, CaptionFace, CaptionStyle, DriveTuning,
    EffectDrive, FontAsset, Metadata, OutputFormat, OutputQuality, OutputSettings, ParameterCue,
    Project, Provenance, SceneEntry, ScenePreset, SceneSwitchSuggestion, SceneSwitchSuggestions,
    MAX_ANALYSIS_LANES, MAX_CUES, MAX_MAPPINGS_PER_SCENE, MAX_SCENES, MAX_SCENE_PRESETS,
    SCENE_SWITCH_CAPACITY,
};
use crate::scene::events::{EventRecord, EventType, TIMELINE_CAPACITY, VALUE_CAPACITY};
use crate::scene::routes::{AnalysisSource, Interpolation, ParameterMapping};
use crate::scene::settings::MAX_CONTROLS;

use super::event_timeline::EventTimeline;

/// `MUSI_PROJECT_JSON_MAX_INPUT` (`project_io.h:6`): 4 MiB.
///
/// The first bound any `.musi` meets, before a single byte is interpreted.
pub const MAX_INPUT: usize = 4 * 1024 * 1024;

/// The file's `schema_version` value (`project_io.c:103`).
pub const SCHEMA_VERSION_STRING: &str = "musializer.project/v1";
/// The preset store's `schema_version` value (`project_io.c:330`).
pub const PRESET_STORE_SCHEMA_VERSION_STRING: &str = "musializer.presets/v1";

/// Longest object key or enum name the C parser can hold (`char k[80]`,
/// `project_io.c:198` and friends).
///
/// A longer key is a [`ProjectIoError::String`], **not** an unknown field, because
/// the C buffer overflows before the name is ever compared. Reproduced so the two
/// codecs disagree about nothing.
const KEY_MAX_BYTES: usize = 79;

/// Why a `.musi` document could not be read or written (`Musi_Project_Io_Result`,
/// `project_io.h:7-15`).
///
/// C's `ERROR_NULL`, `ERROR_OUTPUT_TOO_SMALL` and `ERROR_ALLOCATION` have no
/// counterpart: there are no null pointers here, serialization returns an owned
/// `String` rather than filling a caller's buffer, and allocation failure aborts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectIoError {
    #[error("invalid input size")]
    InputSize,
    #[error("malformed JSON")]
    Syntax,
    #[error("unknown field")]
    UnknownField,
    #[error("duplicate field")]
    DuplicateField,
    #[error("missing field")]
    MissingField,
    #[error("invalid or oversized string")]
    String,
    #[error("invalid number")]
    Number,
    #[error("array capacity exceeded")]
    Capacity,
    #[error("schema mismatch")]
    Schema,
    #[error("project validation failed")]
    Validation,
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Writes a JSON string with C's escape set (`project_io.c:56-73`).
///
/// Forward slashes are left alone, control characters below 0x20 become `\uXXXX`,
/// and the seven short escapes are used where they exist. `&str` is UTF-8 by
/// construction, so C's `valid_utf8` guard has nothing to check.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

/// Writes a finite number.
///
/// C uses `%.17g` and then rewrites a locale's decimal comma back to a point
/// (`project_io.c:46-55`). Rust's `Display` for `f64` is locale-independent and
/// emits the shortest representation that round-trips, so both the locale fix-up
/// and the 17-digit padding are unnecessary. The bytes differ from C's in places
/// (`0.1` rather than `0.10000000000000001`); the *values* do not.
///
/// # Why this is not a parity bug
///
/// It was carried as an open question to the parity gate, because `AGENTS.md`'s test
/// is "visible in a `.musi` file" and a spelling difference is visible in the bytes.
/// What settles it is that **nothing in the oracle ever hashes or byte-compares a
/// `.musi`.** Every `sha256` in the C project is over an *asset* — the audio, an
/// imported font, its licence, the ASCII source image — and the project file itself
/// is only ever parsed. So the C reads `0.1`, gets the same double it wrote, and has
/// no way to notice.
///
/// The requirement the gate actually states is a **bidirectional round trip with no
/// field lost**, which is about values, and `tools/differential_project_io.sh` is
/// what holds it. Export determinism is the place bit-identity is required, and an
/// MP4 is not a `.musi`.
///
/// Not merely harmless, either: a `.musi` is meant to be readable by the person who
/// authored it — the same reason colours are written as `"ffffffff"` rather than
/// `4294967295` two functions down.
fn write_f64(out: &mut String, value: f64) {
    out.push_str(&format!("{value}"));
}

/// Eight lowercase hex digits (`rgba`, `project_io.c:93-99`).
///
/// Colours are written as hex rather than an integer because a `.musi` is meant to
/// be readable by the person who authored it, and `"ffffffff"` says "opaque white"
/// where `4294967295` says nothing.
fn write_rgba(out: &mut String, value: u32) {
    out.push_str(&format!("\"{value:08x}\""));
}

fn write_mapping(out: &mut String, mapping: &ParameterMapping) {
    out.push_str("{\"parameter\":");
    write_string(out, &mapping.parameter);
    out.push_str(",\"source\":");
    write_string(out, mapping.source.canonical_name());
    out.push_str(&format!(",\"band_index\":{}", mapping.band_index));
    out.push_str(",\"input_min\":");
    write_f64(out, mapping.input_min);
    out.push_str(",\"input_max\":");
    write_f64(out, mapping.input_max);
    out.push_str(",\"output_min\":");
    write_f64(out, mapping.output_min);
    out.push_str(",\"output_max\":");
    write_f64(out, mapping.output_max);
    out.push_str(",\"interpolation\":");
    write_string(out, mapping.interpolation.canonical_name());
    out.push_str(if mapping.clamp {
        ",\"clamp\":true}"
    } else {
        ",\"clamp\":false}"
    });
}

/// `events_write` (`project_io.c:83-89`).
fn write_events(out: &mut String, timeline: &EventTimeline) {
    out.push('[');
    for (index, event) in timeline.events().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"timestamp_seconds\":");
        write_f64(out, event.timestamp_seconds);
        out.push_str(&format!(",\"id\":{},\"type\":", event.id));
        // Only well-formed timelines reach here: `validate` has already refused
        // any type outside the four named kinds.
        let name =
            EventType::from_raw(event.event_type).map_or("custom", event_type_canonical_name);
        write_string(out, name);
        out.push_str(",\"values\":[");
        for (at, value) in event.values().iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            write_f64(out, f64::from(*value));
        }
        out.push_str("]}");
    }
    out.push(']');
}

/// The four persisted event-type names (`project_io.c:85`).
fn event_type_canonical_name(kind: EventType) -> &'static str {
    match kind {
        EventType::Lyric => "lyric",
        EventType::Semantic => "semantic",
        EventType::Cue => "cue",
        EventType::Custom => "custom",
    }
}

fn write_f32_array(out: &mut String, values: &[f32]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_f64(out, f64::from(*value));
    }
    out.push(']');
}

/// `project_write` (`project_io.c:101-151`).
///
/// The field order is the C's, so a Rust-written project and a C-written project
/// differ only in number spelling. Writers always emit the *complete* current v1
/// workspace, including `caption_style` and the embedded semantic lane, even when
/// every value is a default: a canonical writer that omitted defaults would make
/// "absent" ambiguous between "old file" and "deliberately default".
fn write_project(out: &mut String, project: &Project) {
    out.push_str("{\"schema_version\":\"");
    out.push_str(SCHEMA_VERSION_STRING);
    out.push_str("\",\"metadata\":{\"project_id\":");
    let metadata = &project.metadata;
    write_string(out, &metadata.project_id);
    out.push_str(",\"title\":");
    write_string(out, &metadata.title);
    out.push_str(",\"author\":");
    write_string(out, &metadata.author);
    out.push_str(",\"created_utc\":");
    write_string(out, &metadata.created_utc);
    out.push_str(",\"modified_utc\":");
    write_string(out, &metadata.modified_utc);
    out.push_str(",\"application_version\":");
    write_string(out, &metadata.application_version);

    let audio = &project.audio;
    out.push_str("},\"audio\":{\"mode\":");
    write_string(out, audio.mode.canonical_name());
    out.push_str(",\"path\":");
    write_string(out, &audio.path);
    out.push_str(",\"sha256\":");
    write_string(out, &audio.sha256);
    out.push_str(",\"duration_seconds\":");
    write_f64(out, audio.duration_seconds);
    out.push_str(&format!(
        ",\"sample_rate\":{},\"channels\":{}",
        audio.sample_rate, audio.channels
    ));

    out.push_str("},\"ascii_image\":");
    match &project.ascii_image {
        Some(ascii) => {
            out.push_str("{\"path\":");
            write_string(out, &ascii.path);
            out.push_str(",\"sha256\":");
            write_string(out, &ascii.sha256);
            out.push_str(&format!(
                ",\"columns\":{},\"rows\":{}}}",
                ascii.columns, ascii.rows
            ));
        }
        None => out.push_str("null"),
    }

    let caption = &project.caption_style;
    out.push_str(",\"caption_style\":{\"face\":");
    write_string(out, caption.face.canonical_name());
    out.push_str(",\"box\":");
    write_string(out, caption.box_style.canonical_name());
    out.push_str(",\"anchor\":");
    write_string(out, caption.anchor.canonical_name());
    out.push_str(",\"size_scale\":");
    write_f64(out, caption.size_scale);
    out.push_str(",\"margin_scale\":");
    write_f64(out, caption.margin_scale);
    out.push_str(",\"width_scale\":");
    write_f64(out, caption.width_scale);
    out.push_str(",\"text_rgba\":");
    write_rgba(out, caption.text_rgba);
    out.push_str(",\"box_rgba\":");
    write_rgba(out, caption.box_rgba);
    out.push_str(",\"font\":");
    match &caption.font {
        Some(font) => {
            out.push_str("{\"path\":");
            write_string(out, &font.path);
            out.push_str(",\"sha256\":");
            write_string(out, &font.sha256);
            out.push_str(",\"family\":");
            write_string(out, &font.family);
            out.push_str(",\"licence_path\":");
            write_string(out, &font.licence_path);
            out.push_str(",\"licence_sha256\":");
            write_string(out, &font.licence_sha256);
            out.push_str(",\"licence_name\":");
            write_string(out, &font.licence_name);
            out.push('}');
        }
        None => out.push_str("null"),
    }
    // Post-legacy extension (2026-08-03): written only when authored, so every
    // pre-effects project — including the differential fixtures — serializes
    // byte-identically to the C. The frozen C cannot read a file carrying this
    // block; that is accepted and recorded in `AGENTS.md`.
    let effects = &caption.effects;
    if !effects.is_default() {
        out.push_str(",\"effects\":{\"glow_strength\":");
        write_f64(out, effects.glow_strength);
        out.push_str(",\"glow_radius\":");
        write_f64(out, effects.glow_radius);
        out.push_str(",\"glow_rgba\":");
        write_rgba(out, effects.glow_rgba);
        out.push_str(",\"glow_pulse\":");
        write_string(out, effects.glow_pulse.canonical_name());
        out.push_str(",\"glow_pulse_depth\":");
        write_f64(out, effects.glow_pulse_depth);
        // The tuning blocks follow their drives and are themselves written only
        // when authored, so an effects file from before UX0-C14 stays
        // byte-identical on a round trip.
        if !effects.pulse_tuning.is_default() {
            out.push_str(",\"pulse_tuning\":");
            write_drive_tuning(out, &effects.pulse_tuning);
        }
        out.push_str(",\"glow_hue_drive\":");
        write_string(out, effects.glow_hue_drive.canonical_name());
        out.push_str(",\"glow_hue_range\":");
        write_f64(out, effects.glow_hue_range);
        if !effects.hue_tuning.is_default() {
            out.push_str(",\"hue_tuning\":");
            write_drive_tuning(out, &effects.hue_tuning);
        }
        out.push_str(",\"shadow_blur\":");
        write_f64(out, effects.shadow_blur);
        out.push_str(",\"shadow_opacity\":");
        write_f64(out, effects.shadow_opacity);
        out.push_str(",\"plate_roundness\":");
        write_f64(out, effects.plate_roundness);
        out.push('}');
    }
    out.push('}');

    let output = &project.output;
    out.push_str(&format!(
        ",\"output\":{{\"width\":{},\"height\":{},\"fps_numerator\":{},\"fps_denominator\":{}",
        output.width, output.height, output.fps_numerator, output.fps_denominator
    ));
    out.push_str(",\"start_seconds\":");
    write_f64(out, output.start_seconds);
    out.push_str(",\"end_seconds\":");
    write_f64(out, output.end_seconds);
    out.push_str(",\"format\":");
    write_string(out, output.format.canonical_name());
    out.push_str(",\"quality\":");
    write_string(out, output.quality.canonical_name());
    out.push_str(&format!(
        "}},\"deterministic_seed\":{}",
        project.deterministic_seed
    ));

    out.push_str(",\"scenes\":[");
    for (index, scene) in project.scenes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"instance_id\":{},\"scene_type\":",
            scene.instance_id
        ));
        write_string(out, &scene.scene_type);
        out.push_str(if scene.enabled {
            ",\"enabled\":true,\"start_seconds\":"
        } else {
            ",\"enabled\":false,\"start_seconds\":"
        });
        write_f64(out, scene.start_seconds);
        out.push_str(",\"end_seconds\":");
        write_f64(out, scene.end_seconds);
        out.push_str(",\"opacity\":");
        write_f64(out, scene.opacity);
        out.push_str(",\"blend_mode\":");
        write_string(out, scene.blend_mode.canonical_name());
        out.push_str(",\"mappings\":[");
        for (at, mapping) in scene.mappings.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            write_mapping(out, mapping);
        }
        out.push_str("]}");
    }

    out.push_str("],\"cues\":[");
    for (index, cue) in project.cues.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"cue_id\":{},\"target_scene_id\":{},\"parameter\":",
            cue.cue_id, cue.target_scene_id
        ));
        write_string(out, &cue.parameter);
        out.push_str(",\"start_seconds\":");
        write_f64(out, cue.start_seconds);
        out.push_str(",\"end_seconds\":");
        write_f64(out, cue.end_seconds);
        out.push_str(",\"from_value\":");
        write_f64(out, cue.from_value);
        out.push_str(",\"to_value\":");
        write_f64(out, cue.to_value);
        out.push_str(",\"interpolation\":");
        write_string(out, cue.interpolation.canonical_name());
        out.push('}');
    }

    out.push_str("],\"analysis_lanes\":[");
    for (index, lane) in project.analysis_lanes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"kind\":");
        write_string(out, lane.kind.canonical_name());
        out.push_str(",\"path\":");
        write_string(out, &lane.path);
        out.push_str(",\"sha256\":");
        write_string(out, &lane.sha256);
        out.push_str(",\"audio_sha256\":");
        write_string(out, &lane.audio_sha256);
        out.push_str(",\"provenance\":{\"adapter\":");
        write_string(out, &lane.provenance.adapter);
        out.push_str(",\"adapter_version\":");
        write_string(out, &lane.provenance.adapter_version);
        out.push_str(",\"schema_version\":");
        write_string(out, &lane.provenance.schema_version);
        out.push_str(",\"model\":");
        write_string(out, &lane.provenance.model);
        out.push_str(",\"provider\":");
        write_string(out, &lane.provenance.provider);
        out.push_str(",\"prompt_version\":");
        write_string(out, &lane.provenance.prompt_version);
        out.push_str("}}");
    }

    out.push_str(&format!(
        "],\"lyrics\":{{\"next_id\":{},\"cues\":[",
        project.lyrics.next_id()
    ));
    for (index, cue) in project.lyrics.cues().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"id\":{},\"start_seconds\":", cue.id));
        write_f64(out, cue.start_seconds);
        out.push_str(",\"end_seconds\":");
        write_f64(out, cue.end_seconds);
        out.push_str(",\"text\":");
        write_string(out, &cue.text);
        // LX1's origin is written **only when it is not the default**, which is
        // the same rule `caption_style.effects` follows and it is load-bearing
        // twice over: a project whose cues were all placed by hand stays
        // byte-identical to what every earlier build wrote (so
        // `differential_project_io.sh` keeps pinning the format it pins), and a
        // reader that has never heard of the field sees a file it can still
        // parse. The schema version therefore does not move — `model.rs` accepts
        // exactly one, so bumping it would make this application refuse to open
        // its own older files, which is the compatibility contract we do have.
        if cue.origin != crate::project::lyrics::CueOrigin::default() {
            out.push_str(",\"origin\":");
            write_string(out, cue.origin.token());
        }
        out.push('}');
    }

    out.push_str(if project.scene_switches.enabled {
        "]},\"scene_switches\":{\"enabled\":true,\"cues\":["
    } else {
        "]},\"scene_switches\":{\"enabled\":false,\"cues\":["
    });
    for (index, cue) in project.scene_switches.cues.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"id\":{},\"start_seconds\":", cue.id));
        write_f64(out, cue.start_seconds);
        out.push_str(",\"end_seconds\":");
        write_f64(out, cue.end_seconds);
        out.push_str(",\"scene_name\":");
        write_string(out, &cue.scene_name);
        out.push_str(",\"strength\":");
        write_f64(out, f64::from(cue.strength));
        out.push_str(",\"settings\":");
        write_f32_array(out, &cue.settings);
        out.push('}');
    }

    out.push_str("]},\"scene_presets\":[");
    for (index, preset) in project.scene_presets.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"id\":{},\"scene_name\":", preset.id));
        write_string(out, &preset.scene_name);
        out.push_str(",\"name\":");
        write_string(out, &preset.name);
        out.push_str(",\"settings\":");
        write_f32_array(out, &preset.settings);
        out.push('}');
    }

    out.push_str("],\"semantic_events\":");
    write_events(out, &project.semantic_events);
    out.push_str(",\"manual_events\":");
    write_events(out, &project.manual_events);
    out.push('}');
}

/// `musi_project_json_serialize` (`project_io.c:308-316`).
///
/// A project is validated **before** it is written, so an invalid model can never
/// reach a file. That is deliberate: the destination-replacing save below assumes
/// its bytes are already good.
pub fn serialize(project: &Project) -> Result<String, ProjectIoError> {
    project.validate().map_err(|_| ProjectIoError::Validation)?;
    let mut out = String::new();
    write_project(&mut out, project);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
}

/// Tracks which members an object has seen, for the duplicate and missing checks
/// (`SEEN`/mask, `project_io.c:193`).
#[derive(Default)]
struct Seen(u64);

impl Seen {
    fn mark(&mut self, field: usize) -> Result<(), ProjectIoError> {
        let bit = 1u64 << field;
        if self.0 & bit != 0 {
            return Err(ProjectIoError::DuplicateField);
        }
        self.0 |= bit;
        Ok(())
    }

    fn has_all(&self, required: u64) -> bool {
        self.0 & required == required
    }
}

type Parsed<T> = Result<T, ProjectIoError>;

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, at: 0 }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.input.get(self.at), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_whitespace();
        self.input.get(self.at).copied()
    }

    fn take(&mut self, byte: u8) -> Parsed<()> {
        if self.peek() == Some(byte) {
            self.at += 1;
            return Ok(());
        }
        Err(ProjectIoError::Syntax)
    }

    fn take_literal(&mut self, literal: &[u8]) -> bool {
        self.skip_whitespace();
        if self.input[self.at..].starts_with(literal) {
            self.at += literal.len();
            return true;
        }
        false
    }

    /// `jstring` (`project_io.c:165-175`), bounded to `max_bytes`.
    ///
    /// The bound is enforced *while decoding*, exactly as C's fixed output buffer
    /// does, so an over-long value is [`ProjectIoError::String`] and never reaches
    /// model validation. `U+0000` is rejected however it is spelled — literally,
    /// as ` `, or as a surrogate pair — because no field may contain one.
    fn string(&mut self, max_bytes: usize) -> Parsed<String> {
        self.skip_whitespace();
        if self.input.get(self.at) != Some(&b'"') {
            return Err(ProjectIoError::Syntax);
        }
        self.at += 1;
        let mut out = String::new();
        loop {
            let byte = *self.input.get(self.at).ok_or(ProjectIoError::String)?;
            if byte == b'"' {
                self.at += 1;
                return Ok(out);
            }
            self.at += 1;
            let codepoint = if byte == b'\\' {
                let escape = *self.input.get(self.at).ok_or(ProjectIoError::String)?;
                self.at += 1;
                match escape {
                    b'"' | b'\\' | b'/' => u32::from(escape),
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    b'u' => {
                        let high = self.hex4()?;
                        if (0xD800..=0xDBFF).contains(&high) {
                            if self.input[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                            } else {
                                return Err(ProjectIoError::String);
                            }
                            let low = self.hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(ProjectIoError::String);
                            }
                            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                        } else if (0xDC00..=0xDFFF).contains(&high) {
                            return Err(ProjectIoError::String);
                        } else {
                            high
                        }
                    }
                    _ => return Err(ProjectIoError::String),
                }
            } else if byte < 0x20 {
                // A raw control character in a JSON string is malformed.
                return Err(ProjectIoError::String);
            } else if byte < 0x80 {
                u32::from(byte)
            } else {
                // Multi-byte: decode and reject overlongs, surrogates and
                // out-of-range scalars, which is what C's inline check does.
                let continuation = match byte {
                    0xC2..=0xDF => 1,
                    0xE0..=0xEF => 2,
                    0xF0..=0xF4 => 3,
                    _ => return Err(ProjectIoError::String),
                };
                let start = self.at - 1;
                if start + continuation + 1 > self.input.len() {
                    return Err(ProjectIoError::String);
                }
                let slice = &self.input[start..start + continuation + 1];
                let text = core::str::from_utf8(slice).map_err(|_| ProjectIoError::String)?;
                self.at += continuation;
                text.chars().next().map_or(0, u32::from)
            };
            let character = char::from_u32(codepoint)
                .filter(|character| *character != '\0')
                .ok_or(ProjectIoError::String)?;
            if out.len() + character.len_utf8() > max_bytes {
                return Err(ProjectIoError::String);
            }
            out.push(character);
        }
    }

    fn hex4(&mut self) -> Parsed<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            let byte = *self.input.get(self.at).ok_or(ProjectIoError::String)?;
            let digit = char::from(byte)
                .to_digit(16)
                .ok_or(ProjectIoError::String)?;
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }

    /// `number_token` (`project_io.c:176-183`): strict JSON number grammar. With
    /// `integer` set, a fraction or exponent is not merely ignored — it is a
    /// syntax error, so `"width": 1920.0` is refused.
    fn number_token(&mut self, integer: bool) -> Option<&'a [u8]> {
        self.skip_whitespace();
        let start = self.at;
        if self.input.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        match self.input.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(byte) if byte.is_ascii_digit() => {
                while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
            }
            _ => return None,
        }
        if !integer && self.input.get(self.at) == Some(&b'.') {
            self.at += 1;
            if !self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                return None;
            }
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        if !integer && matches!(self.input.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.input.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                return None;
            }
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        Some(&self.input[start..self.at])
    }

    /// `ju64` (`project_io.c:184-185`): unsigned only, and overflow is refused
    /// rather than wrapped.
    fn u64(&mut self) -> Parsed<u64> {
        let token = self.number_token(true).ok_or(ProjectIoError::Number)?;
        if token.first() == Some(&b'-') {
            return Err(ProjectIoError::Number);
        }
        let mut value = 0u64;
        for byte in token {
            let digit = u64::from(byte - b'0');
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or(ProjectIoError::Number)?;
        }
        Ok(value)
    }

    fn bounded_u32(&mut self) -> Parsed<u32> {
        u32::try_from(self.u64()?).map_err(|_| ProjectIoError::Number)
    }

    fn bounded_u16(&mut self) -> Parsed<u16> {
        u16::try_from(self.u64()?).map_err(|_| ProjectIoError::Number)
    }

    /// `jdouble` (`project_io.c:186-187`): finite only. C additionally caps the
    /// token at 64 bytes because it copies into a stack buffer; that limit is
    /// reproduced so a pathological literal fails the same way.
    fn f64(&mut self) -> Parsed<f64> {
        let token = self.number_token(false).ok_or(ProjectIoError::Number)?;
        if token.len() >= 64 {
            return Err(ProjectIoError::Number);
        }
        let text = core::str::from_utf8(token).map_err(|_| ProjectIoError::Number)?;
        let value: f64 = text.parse().map_err(|_| ProjectIoError::Number)?;
        value
            .is_finite()
            .then_some(value)
            .ok_or(ProjectIoError::Number)
    }

    /// A number narrowed to `f32`, refusing anything outside its range
    /// (`parse_float_array`, `project_io.c:222-223`).
    fn f32(&mut self) -> Parsed<f32> {
        let value = self.f64()?;
        if value > f64::from(f32::MAX) || value < f64::from(f32::MIN) {
            return Err(ProjectIoError::Number);
        }
        Ok(value as f32)
    }

    fn bool(&mut self) -> Parsed<bool> {
        if self.take_literal(b"true") {
            return Ok(true);
        }
        if self.take_literal(b"false") {
            return Ok(false);
        }
        Err(ProjectIoError::Syntax)
    }

    /// `enum_string` (`project_io.c:190`): an unrecognized name is a schema
    /// mismatch, never a default.
    fn enum_value<T>(&mut self, from_name: impl Fn(&str) -> Option<T>) -> Parsed<T> {
        let text = self.string(KEY_MAX_BYTES)?;
        from_name(&text).ok_or(ProjectIoError::Schema)
    }

    /// `parse_rgba` (`project_io.c:236-252`): exactly eight lowercase hex digits.
    ///
    /// Accepting `"#fff"` or uppercase would make two spellings of one colour, and
    /// the format promises exact identity.
    fn rgba(&mut self) -> Parsed<u32> {
        let text = self.string(15)?;
        if text.len() != 8 {
            return Err(ProjectIoError::String);
        }
        let mut value = 0u32;
        for byte in text.bytes() {
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                _ => return Err(ProjectIoError::String),
            };
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    /// Iterates an object's members, handing each key to `body`
    /// (`member`, `project_io.c:191-192`).
    fn object(&mut self, mut body: impl FnMut(&mut Self, &str) -> Parsed<()>) -> Parsed<()> {
        self.take(b'{')?;
        let mut first = true;
        loop {
            if self.peek() == Some(b'}') {
                self.at += 1;
                return Ok(());
            }
            if first {
                first = false;
            } else {
                self.take(b',')?;
            }
            let key = self.string(KEY_MAX_BYTES)?;
            self.take(b':')?;
            body(self, &key)?;
        }
    }

    /// Iterates an array, checking the capacity **before** parsing each element,
    /// exactly as C does (`project_io.c:201`).
    fn array(
        &mut self,
        capacity: usize,
        mut body: impl FnMut(&mut Self, usize) -> Parsed<()>,
    ) -> Parsed<()> {
        self.take(b'[')?;
        let mut count = 0usize;
        loop {
            if self.peek() == Some(b']') {
                self.at += 1;
                return Ok(());
            }
            if count > 0 {
                self.take(b',')?;
            }
            if count >= capacity {
                return Err(ProjectIoError::Capacity);
            }
            body(self, count)?;
            count += 1;
        }
    }
}

/// Looks a key up in an ordered name table (`field_index`, `project_io.c:189`).
fn field_index(key: &str, names: &[&str]) -> Parsed<usize> {
    names
        .iter()
        .position(|name| *name == key)
        .ok_or(ProjectIoError::UnknownField)
}

fn parse_mapping(parser: &mut Parser<'_>) -> Parsed<ParameterMapping> {
    const NAMES: [&str; 9] = [
        "parameter",
        "source",
        "band_index",
        "input_min",
        "input_max",
        "output_min",
        "output_max",
        "interpolation",
        "clamp",
    ];
    let mut mapping = ParameterMapping {
        parameter: String::new(),
        source: AnalysisSource::Rms,
        band_index: 0,
        input_min: 0.0,
        input_max: 0.0,
        output_min: 0.0,
        output_max: 0.0,
        interpolation: Interpolation::Step,
        clamp: false,
    };
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => mapping.parameter = parser.string(capacity::PARAMETER)?,
            1 => mapping.source = parser.enum_value(AnalysisSource::from_canonical_name)?,
            2 => mapping.band_index = parser.bounded_u16()?,
            3 => mapping.input_min = parser.f64()?,
            4 => mapping.input_max = parser.f64()?,
            5 => mapping.output_min = parser.f64()?,
            6 => mapping.output_max = parser.f64()?,
            7 => {
                mapping.interpolation = parser.enum_value(Interpolation::from_canonical_name)?;
            }
            _ => mapping.clamp = parser.bool()?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x1ff) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(mapping)
}

fn parse_scene(parser: &mut Parser<'_>) -> Parsed<SceneEntry> {
    const NAMES: [&str; 8] = [
        "instance_id",
        "scene_type",
        "enabled",
        "start_seconds",
        "end_seconds",
        "opacity",
        "blend_mode",
        "mappings",
    ];
    let mut scene = SceneEntry::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => scene.instance_id = parser.u64()?,
            1 => scene.scene_type = parser.string(capacity::TYPE)?,
            2 => scene.enabled = parser.bool()?,
            3 => scene.start_seconds = parser.f64()?,
            4 => scene.end_seconds = parser.f64()?,
            5 => scene.opacity = parser.f64()?,
            6 => scene.blend_mode = parser.enum_value(BlendMode::from_canonical_name)?,
            _ => {
                let mappings = &mut scene.mappings;
                parser.array(MAX_MAPPINGS_PER_SCENE, |parser, _| {
                    mappings.push(parse_mapping(parser)?);
                    Ok(())
                })?;
            }
        }
        Ok(())
    })?;
    if !seen.has_all(0xff) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(scene)
}

fn parse_cue(parser: &mut Parser<'_>) -> Parsed<ParameterCue> {
    const NAMES: [&str; 8] = [
        "cue_id",
        "target_scene_id",
        "parameter",
        "start_seconds",
        "end_seconds",
        "from_value",
        "to_value",
        "interpolation",
    ];
    let mut cue = ParameterCue::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => cue.cue_id = parser.u64()?,
            1 => cue.target_scene_id = parser.u64()?,
            2 => cue.parameter = parser.string(capacity::PARAMETER)?,
            3 => cue.start_seconds = parser.f64()?,
            4 => cue.end_seconds = parser.f64()?,
            5 => cue.from_value = parser.f64()?,
            6 => cue.to_value = parser.f64()?,
            _ => cue.interpolation = parser.enum_value(Interpolation::from_canonical_name)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0xff) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(cue)
}

/// `parse_metadata` (`project_io.c:205-206`).
///
/// Only `project_id`, `title` and `application_version` are required — mask
/// `0x23`. The three optional fields default to the empty string, which is a
/// documented compatibility default.
fn parse_metadata(parser: &mut Parser<'_>) -> Parsed<Metadata> {
    const NAMES: [&str; 6] = [
        "project_id",
        "title",
        "author",
        "created_utc",
        "modified_utc",
        "application_version",
    ];
    let mut metadata = Metadata::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => metadata.project_id = parser.string(capacity::ID)?,
            1 => metadata.title = parser.string(capacity::NAME)?,
            2 => metadata.author = parser.string(capacity::NAME)?,
            3 => metadata.created_utc = parser.string(capacity::TIMESTAMP)?,
            4 => metadata.modified_utc = parser.string(capacity::TIMESTAMP)?,
            _ => metadata.application_version = parser.string(capacity::VERSION)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x23) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(metadata)
}

fn parse_audio(parser: &mut Parser<'_>) -> Parsed<AudioAsset> {
    const NAMES: [&str; 6] = [
        "mode",
        "path",
        "sha256",
        "duration_seconds",
        "sample_rate",
        "channels",
    ];
    let mut audio = AudioAsset::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => audio.mode = parser.enum_value(AssetMode::from_canonical_name)?,
            1 => audio.path = parser.string(capacity::PATH)?,
            2 => audio.sha256 = parser.string(capacity::ID)?,
            3 => audio.duration_seconds = parser.f64()?,
            4 => audio.sample_rate = parser.bounded_u32()?,
            _ => audio.channels = parser.bounded_u16()?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x3f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(audio)
}

/// `parse_ascii_image` (`project_io.c:209-210`): `null` or a complete object.
fn parse_ascii_image(parser: &mut Parser<'_>) -> Parsed<Option<AsciiImageAsset>> {
    if parser.take_literal(b"null") {
        return Ok(None);
    }
    const NAMES: [&str; 4] = ["path", "sha256", "columns", "rows"];
    let mut ascii = AsciiImageAsset::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => ascii.path = parser.string(capacity::PATH)?,
            1 => ascii.sha256 = parser.string(capacity::ID)?,
            2 => ascii.columns = parser.bounded_u32()?,
            _ => ascii.rows = parser.bounded_u32()?,
        }
        Ok(())
    })?;
    if !seen.has_all(0xf) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(Some(ascii))
}

/// `parse_output` (`project_io.c:211-212`).
///
/// `quality` is the one optional member (mask `0x7f` covers the first seven), and
/// its absence means [`OutputQuality::High`] — the value the original v1 contract
/// implied before quality was authorable. That is the "readers accept the original
/// v1 contract without quality" compatibility rule from `project_io.h:104-106`.
fn parse_output(parser: &mut Parser<'_>) -> Parsed<OutputSettings> {
    const NAMES: [&str; 8] = [
        "width",
        "height",
        "fps_numerator",
        "fps_denominator",
        "start_seconds",
        "end_seconds",
        "format",
        "quality",
    ];
    let mut output = OutputSettings {
        quality: OutputQuality::High,
        ..OutputSettings::default()
    };
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => output.width = parser.bounded_u32()?,
            1 => output.height = parser.bounded_u32()?,
            2 => output.fps_numerator = parser.bounded_u32()?,
            3 => output.fps_denominator = parser.bounded_u32()?,
            4 => output.start_seconds = parser.f64()?,
            5 => output.end_seconds = parser.f64()?,
            6 => output.format = parser.enum_value(OutputFormat::from_canonical_name)?,
            _ => output.quality = parser.enum_value(OutputQuality::from_canonical_name)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x7f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(output)
}

/// `parse_provenance` (`project_io.c:213-214`): `model`, `provider` and
/// `prompt_version` are optional and default to empty.
fn parse_provenance(parser: &mut Parser<'_>) -> Parsed<Provenance> {
    const NAMES: [&str; 6] = [
        "adapter",
        "adapter_version",
        "schema_version",
        "model",
        "provider",
        "prompt_version",
    ];
    let mut provenance = Provenance::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => provenance.adapter = parser.string(capacity::TYPE)?,
            1 => provenance.adapter_version = parser.string(capacity::VERSION)?,
            2 => provenance.schema_version = parser.string(capacity::VERSION)?,
            3 => provenance.model = parser.string(capacity::PROVIDER)?,
            4 => provenance.provider = parser.string(capacity::PROVIDER)?,
            _ => provenance.prompt_version = parser.string(capacity::VERSION)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x7) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(provenance)
}

fn parse_lane(parser: &mut Parser<'_>) -> Parsed<AnalysisLaneReference> {
    const NAMES: [&str; 5] = ["kind", "path", "sha256", "audio_sha256", "provenance"];
    let mut lane = AnalysisLaneReference::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => lane.kind = parser.enum_value(AnalysisLaneKind::from_canonical_name)?,
            1 => lane.path = parser.string(capacity::PATH)?,
            2 => lane.sha256 = parser.string(capacity::ID)?,
            3 => lane.audio_sha256 = parser.string(capacity::ID)?,
            _ => lane.provenance = parse_provenance(parser)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x1f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(lane)
}

fn parse_lyric_cue(parser: &mut Parser<'_>) -> Parsed<LyricCue> {
    // `origin` is the one **optional** name here (LX1): absent means
    // [`CueOrigin::UserApplied`], which is what makes every pre-LX1 file open
    // unchanged. `has_all` below still demands the original four, so optional
    // does not mean forgiving.
    const NAMES: [&str; 5] = ["id", "start_seconds", "end_seconds", "text", "origin"];
    /// Longest [`CueOrigin::token`]. Bounding the read before the lookup keeps
    /// the "strings are bounded at parse time" rule the module comment states.
    const ORIGIN_MAX_BYTES: usize = 16;
    let mut cue = LyricCue::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => cue.id = parser.u64()?,
            1 => cue.start_seconds = parser.f64()?,
            2 => cue.end_seconds = parser.f64()?,
            3 => cue.text = parser.string(crate::project::lyrics::TEXT_MAX_BYTES)?,
            // An unrecognised token is a hard error rather than a fallback to
            // the default. Silently reading a future "verified" origin as
            // "user applied" would be this codec lying about provenance, and
            // provenance is the whole point of the field.
            _ => {
                let token = parser.string(ORIGIN_MAX_BYTES)?;
                cue.origin = crate::project::lyrics::CueOrigin::from_token(&token)
                    .ok_or(ProjectIoError::String)?;
            }
        }
        Ok(())
    })?;
    if !seen.has_all(0xf) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(cue)
}

/// `parse_lyrics` (`project_io.c:220-221`).
///
/// Cues are pushed in file order and validated as a whole afterwards, rather than
/// going through the editing insert path: a file that lists them out of order must
/// be *rejected*, not quietly sorted.
fn parse_lyrics(parser: &mut Parser<'_>, document: &mut LyricsDocument) -> Parsed<()> {
    const NAMES: [&str; 2] = ["next_id", "cues"];
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        if field == 0 {
            document.set_next_id(parser.u64()?);
        } else {
            parser.array(LYRIC_CUE_CAPACITY, |parser, _| {
                let cue = parse_lyric_cue(parser)?;
                document.push_unvalidated(cue);
                Ok(())
            })?;
        }
        Ok(())
    })?;
    if !seen.has_all(0x3) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(())
}

fn parse_f32_array(parser: &mut Parser<'_>, capacity: usize) -> Parsed<Vec<f32>> {
    let mut values = Vec::new();
    parser.array(capacity, |parser, _| {
        values.push(parser.f32()?);
        Ok(())
    })?;
    Ok(values)
}

/// `parse_switch` (`project_io.c:224-225`): `settings` is optional and defaults to
/// empty, which is how early v1 projects are spelled.
fn parse_switch(parser: &mut Parser<'_>) -> Parsed<SceneSwitchSuggestion> {
    const NAMES: [&str; 6] = [
        "id",
        "start_seconds",
        "end_seconds",
        "scene_name",
        "strength",
        "settings",
    ];
    let mut cue = SceneSwitchSuggestion::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => cue.id = parser.u64()?,
            1 => cue.start_seconds = parser.f64()?,
            2 => cue.end_seconds = parser.f64()?,
            3 => cue.scene_name = parser.string(capacity::TYPE)?,
            4 => cue.strength = parser.f32()?,
            _ => cue.settings = parse_f32_array(parser, MAX_CONTROLS)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x1f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(cue)
}

fn parse_switches(parser: &mut Parser<'_>) -> Parsed<SceneSwitchSuggestions> {
    const NAMES: [&str; 2] = ["enabled", "cues"];
    let mut switches = SceneSwitchSuggestions::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        if field == 0 {
            switches.enabled = parser.bool()?;
        } else {
            let cues = &mut switches.cues;
            parser.array(SCENE_SWITCH_CAPACITY, |parser, _| {
                cues.push(parse_switch(parser)?);
                Ok(())
            })?;
        }
        Ok(())
    })?;
    if !seen.has_all(0x3) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(switches)
}

fn parse_event(parser: &mut Parser<'_>) -> Parsed<EventRecord> {
    const NAMES: [&str; 4] = ["timestamp_seconds", "id", "type", "values"];
    const TYPES: [&str; 4] = ["lyric", "semantic", "cue", "custom"];
    let mut event = EventRecord::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => event.timestamp_seconds = parser.f64()?,
            1 => event.id = parser.u64()?,
            2 => {
                let name = parser.string(KEY_MAX_BYTES)?;
                let index = TYPES
                    .iter()
                    .position(|candidate| *candidate == name)
                    .ok_or(ProjectIoError::Schema)?;
                event.event_type = index as u32 + EventType::Lyric as u32;
            }
            _ => {
                let values = parse_f32_array(parser, VALUE_CAPACITY)?;
                event.value_count = values.len() as u8;
                for (at, value) in values.into_iter().enumerate() {
                    event.values[at] = value;
                }
            }
        }
        Ok(())
    })?;
    if !seen.has_all(0xf) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(event)
}

/// `parse_events` (`project_io.c:230-231`).
///
/// Events are appended in file order and the lane is validated afterwards, so an
/// unsorted or duplicate-id lane is rejected rather than reordered. That is why
/// this cannot use [`EventTimeline::record`], which would sort as it goes.
fn parse_events(parser: &mut Parser<'_>) -> Parsed<Vec<EventRecord>> {
    let mut events = Vec::new();
    parser.array(TIMELINE_CAPACITY, |parser, _| {
        events.push(parse_event(parser)?);
        Ok(())
    })?;
    Ok(events)
}

fn parse_scene_preset(parser: &mut Parser<'_>) -> Parsed<ScenePreset> {
    const NAMES: [&str; 4] = ["id", "scene_name", "name", "settings"];
    let mut preset = ScenePreset::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => preset.id = parser.u64()?,
            1 => preset.scene_name = parser.string(capacity::TYPE)?,
            2 => preset.name = parser.string(capacity::NAME)?,
            _ => preset.settings = parse_f32_array(parser, MAX_CONTROLS)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0xf) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(preset)
}

/// `parse_font_asset` (`project_io.c:254-274`): `null`, or all six fields.
fn parse_font_asset(parser: &mut Parser<'_>) -> Parsed<Option<FontAsset>> {
    if parser.take_literal(b"null") {
        return Ok(None);
    }
    const NAMES: [&str; 6] = [
        "path",
        "sha256",
        "family",
        "licence_path",
        "licence_sha256",
        "licence_name",
    ];
    let mut font = FontAsset::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => font.path = parser.string(capacity::PATH)?,
            1 => font.sha256 = parser.string(capacity::ID)?,
            2 => font.family = parser.string(capacity::NAME)?,
            3 => font.licence_path = parser.string(capacity::PATH)?,
            4 => font.licence_sha256 = parser.string(capacity::ID)?,
            _ => font.licence_name = parser.string(capacity::NAME)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x3f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(Some(font))
}

/// `parse_caption_style` (`project_io.c:276-302`).
///
/// The block is optional at the top level, but **once present every one of its
/// nine legacy members is required**: a half-specified style would silently mix
/// the author's intent with the shipped defaults, and the file could then no
/// longer say which was which. `effects` is the one optional member — it
/// post-dates the frozen C (2026-08-03) and its absence *is* the default, the
/// same contract the whole block has at the project level.
fn parse_caption_style(parser: &mut Parser<'_>) -> Parsed<CaptionStyle> {
    const NAMES: [&str; 10] = [
        "face",
        "box",
        "anchor",
        "size_scale",
        "margin_scale",
        "width_scale",
        "text_rgba",
        "box_rgba",
        "font",
        "effects",
    ];
    let mut style = CaptionStyle::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => style.face = parser.enum_value(CaptionFace::from_canonical_name)?,
            1 => style.box_style = parser.enum_value(CaptionBox::from_canonical_name)?,
            2 => style.anchor = parser.enum_value(CaptionAnchor::from_canonical_name)?,
            3 => style.size_scale = parser.f64()?,
            4 => style.margin_scale = parser.f64()?,
            5 => style.width_scale = parser.f64()?,
            6 => style.text_rgba = parser.rgba()?,
            7 => style.box_rgba = parser.rgba()?,
            8 => style.font = parse_font_asset(parser)?,
            _ => style.effects = parse_caption_effects(parser)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x1ff) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(style)
}

/// The `effects` block: once present, all ten 2026-08-03 members are required,
/// matching the parent style's all-or-nothing contract. The two tuning blocks
/// post-date them (UX0-C14) and are optional with the identity as default —
/// the same forward-compatibility shape `effects` itself has inside the style.
fn parse_caption_effects(parser: &mut Parser<'_>) -> Parsed<CaptionEffects> {
    const NAMES: [&str; 12] = [
        "glow_strength",
        "glow_radius",
        "glow_rgba",
        "glow_pulse",
        "glow_pulse_depth",
        "glow_hue_drive",
        "glow_hue_range",
        "shadow_blur",
        "shadow_opacity",
        "plate_roundness",
        "pulse_tuning",
        "hue_tuning",
    ];
    let mut effects = CaptionEffects::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => effects.glow_strength = parser.f64()?,
            1 => effects.glow_radius = parser.f64()?,
            2 => effects.glow_rgba = parser.rgba()?,
            3 => effects.glow_pulse = parser.enum_value(EffectDrive::from_canonical_name)?,
            4 => effects.glow_pulse_depth = parser.f64()?,
            5 => effects.glow_hue_drive = parser.enum_value(EffectDrive::from_canonical_name)?,
            6 => effects.glow_hue_range = parser.f64()?,
            7 => effects.shadow_blur = parser.f64()?,
            8 => effects.shadow_opacity = parser.f64()?,
            9 => effects.plate_roundness = parser.f64()?,
            10 => effects.pulse_tuning = parse_drive_tuning(parser)?,
            _ => effects.hue_tuning = parse_drive_tuning(parser)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x3ff) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(effects)
}

fn write_drive_tuning(out: &mut String, tuning: &DriveTuning) {
    out.push_str("{\"input_min\":");
    write_f64(out, tuning.input_min);
    out.push_str(",\"input_max\":");
    write_f64(out, tuning.input_max);
    out.push_str(",\"output_min\":");
    write_f64(out, tuning.output_min);
    out.push_str(",\"output_max\":");
    write_f64(out, tuning.output_max);
    out.push_str(",\"curve\":");
    write_string(out, tuning.curve.canonical_name());
    out.push_str(",\"clamp\":");
    out.push_str(if tuning.clamp { "true" } else { "false" });
    out.push('}');
}

/// A drive tuning block: once present, all six members are required — the
/// all-or-nothing contract every optional block in this format carries.
fn parse_drive_tuning(parser: &mut Parser<'_>) -> Parsed<DriveTuning> {
    const NAMES: [&str; 6] = [
        "input_min",
        "input_max",
        "output_min",
        "output_max",
        "curve",
        "clamp",
    ];
    let mut tuning = DriveTuning::default();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => tuning.input_min = parser.f64()?,
            1 => tuning.input_max = parser.f64()?,
            2 => tuning.output_min = parser.f64()?,
            3 => tuning.output_max = parser.f64()?,
            4 => tuning.curve = parser.enum_value(Interpolation::from_canonical_name)?,
            _ => tuning.clamp = parser.bool()?,
        }
        Ok(())
    })?;
    if !seen.has_all(0x3f) {
        return Err(ProjectIoError::MissingField);
    }
    Ok(tuning)
}

/// `parse_project` (`project_io.c:304-306`).
///
/// The required mask is `0xff`: the first eight fields in the C name table. The
/// remaining seven are optional, and each has a documented default —
/// `ascii_image` null, `caption_style` the shipped style, the workspace lanes
/// empty. That is the whole of this format's forward compatibility.
fn parse_project(parser: &mut Parser<'_>) -> Parsed<Project> {
    const NAMES: [&str; 15] = [
        "schema_version",
        "metadata",
        "audio",
        "output",
        "deterministic_seed",
        "scenes",
        "cues",
        "analysis_lanes",
        "lyrics",
        "scene_switches",
        "manual_events",
        "semantic_events",
        "scene_presets",
        "ascii_image",
        "caption_style",
    ];
    let mut project = Project::default();
    let mut manual_events = Vec::new();
    let mut semantic_events = Vec::new();
    let mut seen = Seen::default();
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => {
                let version = parser.string(63)?;
                if version != SCHEMA_VERSION_STRING {
                    return Err(ProjectIoError::Schema);
                }
            }
            1 => project.metadata = parse_metadata(parser)?,
            2 => project.audio = parse_audio(parser)?,
            3 => project.output = parse_output(parser)?,
            4 => project.deterministic_seed = parser.u64()?,
            5 => {
                let scenes = &mut project.scenes;
                parser.array(MAX_SCENES, |parser, _| {
                    scenes.push(parse_scene(parser)?);
                    Ok(())
                })?;
            }
            6 => {
                let cues = &mut project.cues;
                parser.array(MAX_CUES, |parser, _| {
                    cues.push(parse_cue(parser)?);
                    Ok(())
                })?;
            }
            7 => {
                let lanes = &mut project.analysis_lanes;
                parser.array(MAX_ANALYSIS_LANES, |parser, _| {
                    lanes.push(parse_lane(parser)?);
                    Ok(())
                })?;
            }
            8 => parse_lyrics(parser, &mut project.lyrics)?,
            9 => project.scene_switches = parse_switches(parser)?,
            10 => manual_events = parse_events(parser)?,
            11 => semantic_events = parse_events(parser)?,
            12 => {
                let presets = &mut project.scene_presets;
                parser.array(MAX_SCENE_PRESETS, |parser, _| {
                    presets.push(parse_scene_preset(parser)?);
                    Ok(())
                })?;
            }
            13 => project.ascii_image = parse_ascii_image(parser)?,
            _ => project.caption_style = parse_caption_style(parser)?,
        }
        Ok(())
    })?;
    if !seen.has_all(0xff) {
        return Err(ProjectIoError::MissingField);
    }
    // `lyrics.duration_seconds` is not a field in the file; it is copied from the
    // audio asset (`project_io.c:306`), and validation then insists the two agree.
    project
        .lyrics
        .set_duration_unvalidated(project.audio.duration_seconds);
    project.manual_events = EventTimeline::from_file_order(manual_events);
    project.semantic_events = EventTimeline::from_file_order(semantic_events);
    Ok(project)
}

/// `musi_project_json_deserialize` (`project_io.c:317-327`).
///
/// The destination is replaced only after complete parsing **and** model
/// validation, which is the "`.musi` input is bounded and validated before
/// mutating application state" invariant in one function: this returns a whole new
/// [`Project`] or an error, and never a half-updated one.
pub fn deserialize(input: &[u8]) -> Result<Project, ProjectIoError> {
    if input.is_empty() || input.len() > MAX_INPUT || input.contains(&0) {
        return Err(ProjectIoError::InputSize);
    }
    let mut parser = Parser::new(input);
    let project = parse_project(&mut parser)?;
    parser.skip_whitespace();
    if parser.at != input.len() {
        return Err(ProjectIoError::Syntax);
    }
    project.validate().map_err(|_| ProjectIoError::Validation)?;
    Ok(project)
}

// ---------------------------------------------------------------------------
// The shared preset store document
// ---------------------------------------------------------------------------

/// The per-user shared preset store file (`Musi_Preset_Store_Document`,
/// `project_io.h:85-89`).
///
/// The same strict JSON discipline and record shape as the project codec, reused
/// for tuning presets that live with the user instead of with one track.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetStoreDocument {
    pub next_id: u64,
    pub presets: Vec<ScenePreset>,
}

impl Default for PresetStoreDocument {
    /// `musi_preset_store_document_init` (`project_io.c:371-376`).
    fn default() -> Self {
        Self {
            next_id: 1,
            presets: Vec::new(),
        }
    }
}

impl PresetStoreDocument {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `preset_store_document_valid` (`project_io.c:350-369`).
    ///
    /// Note `id < next_id`: an id at or above the allocator's cursor could be
    /// handed out again, and two presets sharing an id is how a rename loses one.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.presets.len() > MAX_SCENE_PRESETS {
            return false;
        }
        for (index, preset) in self.presets.iter().enumerate() {
            if preset.id == 0
                || preset.id >= self.next_id
                || preset.scene_name.is_empty()
                || preset.scene_name.len() > capacity::TYPE
                || preset.name.is_empty()
                || preset.name.len() > capacity::NAME
                || preset.settings.is_empty()
                || preset.settings.len() > MAX_CONTROLS
                || preset.settings.iter().any(|value| !value.is_finite())
            {
                return false;
            }
            if self.presets[..index]
                .iter()
                .any(|other| other.id == preset.id)
            {
                return false;
            }
        }
        true
    }
}

/// `musi_preset_store_serialize` (`project_io.c:377-385`).
pub fn preset_store_serialize(store: &PresetStoreDocument) -> Result<String, ProjectIoError> {
    if !store.is_valid() {
        return Err(ProjectIoError::Validation);
    }
    let mut out = format!(
        "{{\"schema_version\":\"{PRESET_STORE_SCHEMA_VERSION_STRING}\",\"next_id\":{},\"presets\":[",
        store.next_id
    );
    for (index, preset) in store.presets.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"id\":{},\"scene_name\":", preset.id));
        write_string(&mut out, &preset.scene_name);
        out.push_str(",\"name\":");
        write_string(&mut out, &preset.name);
        out.push_str(",\"settings\":");
        write_f32_array(&mut out, &preset.settings);
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

/// `musi_preset_store_deserialize` (`project_io.c:386-396`).
pub fn preset_store_deserialize(input: &[u8]) -> Result<PresetStoreDocument, ProjectIoError> {
    if input.is_empty() || input.len() > MAX_INPUT || input.contains(&0) {
        return Err(ProjectIoError::InputSize);
    }
    const NAMES: [&str; 3] = ["schema_version", "next_id", "presets"];
    let mut store = PresetStoreDocument::new();
    let mut seen = Seen::default();
    let mut parser = Parser::new(input);
    parser.object(|parser, key| {
        let field = field_index(key, &NAMES)?;
        seen.mark(field)?;
        match field {
            0 => {
                let version = parser.string(63)?;
                if version != PRESET_STORE_SCHEMA_VERSION_STRING {
                    return Err(ProjectIoError::Schema);
                }
            }
            1 => store.next_id = parser.u64()?,
            _ => {
                let presets = &mut store.presets;
                parser.array(MAX_SCENE_PRESETS, |parser, _| {
                    presets.push(parse_scene_preset(parser)?);
                    Ok(())
                })?;
            }
        }
        Ok(())
    })?;
    if !seen.has_all(0x7) {
        return Err(ProjectIoError::MissingField);
    }
    parser.skip_whitespace();
    if parser.at != input.len() {
        return Err(ProjectIoError::Syntax);
    }
    if !store.is_valid() {
        return Err(ProjectIoError::Validation);
    }
    Ok(store)
}

// ---------------------------------------------------------------------------
// Transactional-save policy (the pure half)
// ---------------------------------------------------------------------------

/// How a durable write failed (`Musi_Project_File_Result`, `project_io.h:41-52`).
///
/// The granularity is load-bearing and the rewrite must keep it: `Sync`,
/// `Publish` and `Durability` are three different stories about what happened to
/// the destination, and the atomic-save guarantees the C tests assert depend on
/// telling them apart. `Durability` in particular means the project **was**
/// published and only the parent-directory `fsync` went unconfirmed — which is why
/// it is the one failure that does *not* delete the transaction file
/// (`project_io.c:1202-1205`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectFileError {
    #[error("invalid or oversized destination path")]
    Path,
    #[error("could not create a transaction file")]
    Open,
    #[error("could not write the complete project")]
    Write,
    #[error("could not preserve project permissions")]
    Permissions,
    #[error("could not flush the project to storage")]
    Sync,
    #[error("could not close the project transaction")]
    Close,
    #[error("could not atomically publish the project")]
    Publish,
    #[error("project was published but parent-directory durability was not confirmed")]
    Durability,
}

/// How an asset reference resolved (`Musi_Project_Path_Result`,
/// `project_io.h:21-28`).
///
/// The three success values exist so a caller can *surface* the legacy
/// working-directory fallback instead of silently making a project depend on the
/// process working directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathResolution {
    Absolute,
    ProjectRelative,
    LegacyWorkingDirectory,
}

/// Why an asset reference did not resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("null or empty path argument")]
    Empty,
    #[error("asset not found")]
    NotFound,
    #[error("resolved path is too long")]
    TooLong,
}

/// How a runtime asset path was converted for storage
/// (`Musi_Project_Stored_Path_Result`, `project_io.h:33-39`).
///
/// A relative path is emitted only when it resolves, beside the destination
/// project, back to the same existing file. The absolute path is the portable
/// fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredPath {
    Relative,
    Absolute,
}

/// `musi_project_temporary_path` (`project_io.c:984-1004`).
///
/// A same-directory transaction name. `process_id` and `nonce` are explicit
/// arguments rather than read from the environment so the collision policy can be
/// tested deterministically — which is also what lets this live in a crate with no
/// access to `getpid`.
///
/// Returns `None` for an empty destination or one ending in a separator, because
/// neither names a file this could replace.
#[must_use]
pub fn temporary_path(destination: &str, process_id: u64, nonce: u64) -> Option<String> {
    if destination.is_empty() || destination.ends_with('/') || destination.ends_with('\\') {
        return None;
    }
    let directory_length = last_separator(destination).map_or(0, |at| at + 1);
    Some(format!(
        "{}.musializer-project-{process_id}-{nonce:016x}.tmp",
        &destination[..directory_length]
    ))
}

/// Index of the last path separator (`project_last_separator`,
/// `project_io.c:414-424`).
///
/// Backslash counts on every platform because stored relative paths use it as a
/// portable separator; see [`is_unambiguous_relative_path`] for the cost of that.
#[must_use]
pub fn last_separator(path: &str) -> Option<usize> {
    path.rfind(['/', '\\'])
}

/// `project_path_is_absolute` (`project_io.c:400-412`), Linux-first.
#[must_use]
pub fn path_is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// The directory a project path lives in (`project_directory_copy`,
/// `project_io.c:633-657`).
///
/// `None` for an empty path or one ending in a separator — a project path must
/// name a file. A path with no separator resolves to `"."`, and a filesystem root
/// is preserved as `"/"` rather than collapsing to the empty string.
#[must_use]
pub fn directory_of(project_path: &str) -> Option<&str> {
    if project_path.is_empty() || project_path.ends_with('/') || project_path.ends_with('\\') {
        return None;
    }
    match last_separator(project_path) {
        None => Some("."),
        Some(0) => Some(&project_path[..1]),
        Some(at) => Some(&project_path[..at]),
    }
}

/// `project_relative_path_is_unambiguous` (`project_io.c:725-747`).
///
/// Rejects absolute paths, empty components, `.` and `..`, and — on Linux — any
/// literal backslash. The last one is deliberate and worth keeping: the resolver
/// treats a backslash in a *stored* relative path as a portable separator, so a
/// POSIX filename that really contains one must be stored absolutely or it would
/// resolve to a different file on another platform.
#[must_use]
pub fn is_unambiguous_relative_path(path: &str) -> bool {
    if path.is_empty() || path_is_absolute(path) || path.contains('\\') {
        return false;
    }
    path.split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

/// `project_relative_descendant_path` (`project_io.c:749-787`).
///
/// The relative form of `asset` under `directory`, or `None` when `asset` is not a
/// descendant. Both arguments are expected to be canonical absolute paths — this
/// is string arithmetic on an answer the filesystem already gave.
#[must_use]
pub fn relative_descendant_path<'a>(directory: &str, asset: &'a str) -> Option<&'a str> {
    let mut directory_length = directory.len();
    while directory_length > 1 && directory.as_bytes()[directory_length - 1] == b'/' {
        directory_length -= 1;
    }
    let directory = &directory[..directory_length];
    if asset.len() <= directory_length || !asset.starts_with(directory) {
        return None;
    }
    let mut offset = directory_length;
    if !directory.ends_with('/') {
        if asset.as_bytes()[offset] != b'/' {
            return None;
        }
        offset += 1;
    }
    let relative = asset.get(offset..)?;
    is_unambiguous_relative_path(relative).then_some(relative)
}

/// Re-exported for callers that were reaching for it here: the asset bundle's
/// content-addressed layout lives in [`crate::project::assets`].
pub use assets::{bundle_paths, AssetCategory, BundleError, BundlePaths};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::model::tests_support::valid_project;
    use crate::project::sha256;
    use crate::scene::routes::{constant_mapping, AnalysisSource, Interpolation, ParameterMapping};

    fn round_trip(project: &Project) -> Project {
        let text = serialize(project).expect("the fixture project serializes");
        deserialize(text.as_bytes()).expect("its own output parses")
    }

    /// The C suite has no `.musi` fixture files either: it serializes a project and
    /// then edits the text (`tests/test_project_io.c`). Same approach, so nothing
    /// synthetic has to be committed and every fixture stays in step with the model.
    fn without_block(text: &str, key: &str) -> String {
        let needle = format!("\"{key}\":");
        let start = text.find(&needle).expect("the block is present");
        let mut at = start + needle.len();
        let bytes = text.as_bytes();
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        loop {
            let byte = bytes[at];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else {
                match byte {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' if depth > 0 => depth -= 1,
                    b',' | b'}' if depth == 0 => break,
                    _ => {}
                }
            }
            at += 1;
        }
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        // Drop the comma this block owned, whichever side it is on.
        if bytes[at] == b',' {
            out.push_str(&text[at + 1..]);
        } else if out.ends_with(',') {
            out.pop();
            out.push_str(&text[at..]);
        } else {
            out.push_str(&text[at..]);
        }
        out
    }

    #[test]
    fn a_project_survives_a_round_trip() {
        let project = valid_project();
        let reparsed = round_trip(&project);
        assert_eq!(reparsed.metadata, project.metadata);
        assert_eq!(reparsed.audio, project.audio);
        assert_eq!(reparsed.output, project.output);
        assert_eq!(reparsed.caption_style, project.caption_style);
        assert_eq!(reparsed.scenes, project.scenes);
        assert_eq!(reparsed.lyrics, project.lyrics);
    }

    #[test]
    fn serialization_is_stable_across_a_round_trip() {
        let project = valid_project();
        let first = serialize(&project).unwrap();
        let second = serialize(&deserialize(first.as_bytes()).unwrap()).unwrap();
        assert_eq!(first, second, "reading and rewriting must not drift");
    }

    #[test]
    fn post_legacy_balance_routes_round_trip_by_canonical_name() {
        let mut project = valid_project();
        project.scenes[0].mappings.clear();
        project.scenes[0].mappings.push(ParameterMapping {
            parameter: "settings.spectrum.amplitude".to_string(),
            source: AnalysisSource::Balance,
            band_index: 0,
            input_min: 0.2,
            input_max: 0.8,
            output_min: 0.5,
            output_max: 2.0,
            interpolation: Interpolation::Smoothstep,
            clamp: true,
        });
        let text = serialize(&project).unwrap();
        assert!(text.contains("\"source\":\"balance\""));
        let reparsed = deserialize(text.as_bytes()).unwrap();
        assert_eq!(reparsed.scenes[0].mappings, project.scenes[0].mappings);
    }

    #[test]
    fn every_optional_block_may_be_absent() {
        // This is the compatibility surface: seven fields a pre-workspace file
        // simply does not have, each with a documented default.
        let project = valid_project();
        let text = serialize(&project).unwrap();
        for key in [
            "lyrics",
            "scene_switches",
            "manual_events",
            "semantic_events",
            "scene_presets",
            "ascii_image",
            "caption_style",
        ] {
            let reduced = without_block(&text, key);
            let parsed = deserialize(reduced.as_bytes())
                .unwrap_or_else(|error| panic!("{key} absent should parse, got {error}"));
            assert_eq!(parsed.validate(), Ok(()));
        }
    }

    #[test]
    fn an_absent_caption_style_takes_the_shipped_defaults() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        let parsed = deserialize(without_block(&text, "caption_style").as_bytes()).unwrap();
        assert!(
            parsed.caption_style.is_default(),
            "a pre-caption file must reproduce the appearance it was authored against"
        );
        assert!(parsed.caption_style.font.is_none());
    }

    #[test]
    fn an_absent_quality_defaults_to_high() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        // Remove only the quality member, which lives inside `output`.
        let reduced = text.replace(",\"quality\":\"high\"", "");
        assert!(reduced.len() < text.len());
        let parsed = deserialize(reduced.as_bytes()).unwrap();
        assert_eq!(parsed.output.quality, OutputQuality::High);
    }

    #[test]
    fn every_required_block_is_required() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        for key in [
            "schema_version",
            "metadata",
            "audio",
            "output",
            "deterministic_seed",
            "scenes",
            "cues",
            "analysis_lanes",
        ] {
            let reduced = without_block(&text, key);
            assert_eq!(
                deserialize(reduced.as_bytes()).unwrap_err(),
                ProjectIoError::MissingField,
                "{key} should be required"
            );
        }
    }

    #[test]
    fn an_unknown_field_is_a_hard_error() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        let with_extra = text.replacen('{', "{\"surprise\":1,", 1);
        assert_eq!(
            deserialize(with_extra.as_bytes()).unwrap_err(),
            ProjectIoError::UnknownField
        );
        // And nested, so the strictness is not only top level.
        let nested = text.replacen("\"metadata\":{", "\"metadata\":{\"surprise\":1,", 1);
        assert_eq!(
            deserialize(nested.as_bytes()).unwrap_err(),
            ProjectIoError::UnknownField
        );
    }

    #[test]
    fn a_duplicate_field_is_a_hard_error() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        let duplicated = text.replacen(
            "\"deterministic_seed\":",
            "\"deterministic_seed\":0,\"deterministic_seed\":",
            1,
        );
        assert_eq!(
            deserialize(duplicated.as_bytes()).unwrap_err(),
            ProjectIoError::DuplicateField
        );
    }

    #[test]
    fn input_size_is_bounded_before_anything_is_interpreted() {
        assert_eq!(deserialize(b"").unwrap_err(), ProjectIoError::InputSize);
        let mut with_nul = serialize(&valid_project()).unwrap().into_bytes();
        with_nul.push(0);
        assert_eq!(
            deserialize(&with_nul).unwrap_err(),
            ProjectIoError::InputSize
        );
        let oversized = vec![b' '; MAX_INPUT + 1];
        assert_eq!(
            deserialize(&oversized).unwrap_err(),
            ProjectIoError::InputSize
        );
    }

    #[test]
    fn trailing_content_after_the_object_is_refused() {
        let text = serialize(&valid_project()).unwrap();
        assert!(deserialize(format!("{text}  \n").as_bytes()).is_ok());
        assert_eq!(
            deserialize(format!("{text}{{}}").as_bytes()).unwrap_err(),
            ProjectIoError::Syntax
        );
        assert_eq!(
            deserialize(format!("{text}garbage").as_bytes()).unwrap_err(),
            ProjectIoError::Syntax
        );
    }

    #[test]
    fn a_wrong_schema_version_is_a_schema_mismatch() {
        let text = serialize(&valid_project()).unwrap();
        let bumped = text.replace("musializer.project/v1", "musializer.project/v2");
        assert_eq!(
            deserialize(bumped.as_bytes()).unwrap_err(),
            ProjectIoError::Schema
        );
    }

    #[test]
    fn an_unknown_enum_name_is_a_schema_mismatch_not_a_default() {
        let text = serialize(&valid_project()).unwrap();
        for (from, to) in [
            ("\"format\":\"mp4_h264\"", "\"format\":\"av1\""),
            ("\"mode\":\"imported\"", "\"mode\":\"linked\""),
            ("\"blend_mode\":\"normal\"", "\"blend_mode\":\"overlay\""),
            ("\"face\":\"alegreya\"", "\"face\":\"comic\""),
            ("\"box\":\"plate\"", "\"box\":\"bubble\""),
            ("\"anchor\":\"bottom_center\"", "\"anchor\":\"middle\""),
            // Case matters: one spelling per value.
            ("\"format\":\"mp4_h264\"", "\"format\":\"MP4_H264\""),
        ] {
            let broken = text.replace(from, to);
            assert!(broken != text, "{from} not found");
            assert_eq!(
                deserialize(broken.as_bytes()).unwrap_err(),
                ProjectIoError::Schema,
                "{to}"
            );
        }
    }

    #[test]
    fn integers_must_be_integers_and_numbers_must_be_finite() {
        let text = serialize(&valid_project()).unwrap();
        for (from, to, expected) in [
            // An integer field stops at the digits, so the fraction is left for the
            // object loop to choke on — the C reports this as a syntax error, not a
            // number error, and reproducing that is the point.
            ("\"width\":1920", "\"width\":1920.0", ProjectIoError::Syntax),
            ("\"width\":1920", "\"width\":1e3", ProjectIoError::Syntax),
            // A negative value for an unsigned field is a number error, though.
            ("\"width\":1920", "\"width\":-1920", ProjectIoError::Number),
            (
                "\"deterministic_seed\":0",
                "\"deterministic_seed\":99999999999999999999999",
                ProjectIoError::Number,
            ),
            (
                "\"duration_seconds\":60",
                "\"duration_seconds\":nan",
                ProjectIoError::Number,
            ),
            (
                "\"duration_seconds\":60",
                "\"duration_seconds\":1e999",
                ProjectIoError::Number,
            ),
            (
                "\"duration_seconds\":60",
                "\"duration_seconds\":\"60\"",
                ProjectIoError::Number,
            ),
            (
                "\"duration_seconds\":60",
                "\"duration_seconds\":01",
                ProjectIoError::Syntax,
            ),
        ] {
            let broken = text.replace(from, to);
            assert!(broken != text, "{from} not found");
            assert_eq!(
                deserialize(broken.as_bytes()).unwrap_err(),
                expected,
                "{to}"
            );
        }
    }

    #[test]
    fn an_oversized_string_fails_at_parse_time_not_validation() {
        // The distinction matters: C's fixed buffer overflows while decoding, so
        // the error names the string, not the model.
        let mut project = valid_project();
        project.metadata.title = "t".into();
        let text = serialize(&project).unwrap();
        let long = "x".repeat(capacity::NAME + 1);
        let broken = text.replace("\"title\":\"t\"", &format!("\"title\":\"{long}\""));
        assert_eq!(
            deserialize(broken.as_bytes()).unwrap_err(),
            ProjectIoError::String
        );
    }

    #[test]
    fn string_bounds_count_utf8_bytes() {
        let mut project = valid_project();
        project.metadata.title = "t".into();
        let text = serialize(&project).unwrap();
        // 43 three-byte characters is 129 bytes: rejected. 42 is 126: accepted.
        let over = "→".repeat(43);
        let under = "→".repeat(42);
        assert_eq!(
            deserialize(
                text.replace("\"title\":\"t\"", &format!("\"title\":\"{over}\""))
                    .as_bytes()
            )
            .unwrap_err(),
            ProjectIoError::String
        );
        assert!(deserialize(
            text.replace("\"title\":\"t\"", &format!("\"title\":\"{under}\""))
                .as_bytes()
        )
        .is_ok());
    }

    #[test]
    fn escapes_and_surrogate_pairs_decode_and_nul_is_refused() {
        let mut project = valid_project();
        project.metadata.title = "t".into();
        let text = serialize(&project).unwrap();
        let parsed = deserialize(
            text.replace(
                "\"title\":\"t\"",
                "\"title\":\"a\\u0041\\ud83c\\udfb5b\\t\\\\\"",
            )
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.metadata.title, "aA\u{1f3b5}b\t\\");

        for spelling in ["\\u0000", "\\ud83c", "\\udfb5", "\\udfb5\\ud83c", "\\q"] {
            let broken = text.replace("\"title\":\"t\"", &format!("\"title\":\"{spelling}\""));
            assert_eq!(
                deserialize(broken.as_bytes()).unwrap_err(),
                ProjectIoError::String,
                "{spelling}"
            );
        }
    }

    #[test]
    fn invalid_utf8_in_a_string_is_refused() {
        let mut project = valid_project();
        project.metadata.title = "t".into();
        let text = serialize(&project).unwrap();
        let placeholder = "\"title\":\"t\"";
        let at = text.find(placeholder).unwrap();
        let mut bytes = text.as_bytes().to_vec();
        // Overwrite the `t` with a lone continuation byte.
        let title_at = at + placeholder.len() - 2;
        bytes[title_at] = 0x80;
        assert_eq!(deserialize(&bytes).unwrap_err(), ProjectIoError::String);
        // And with an overlong two-byte encoding of '/'.
        let mut bytes = text.as_bytes().to_vec();
        bytes[title_at] = 0xC0;
        assert_eq!(deserialize(&bytes).unwrap_err(), ProjectIoError::String);
    }

    #[test]
    fn caption_colours_accept_exactly_eight_lowercase_hex_digits() {
        let mut project = valid_project();
        project.caption_style.text_rgba = 0xAABB_CCDD;
        let text = serialize(&project).unwrap();
        assert!(text.contains("\"text_rgba\":\"aabbccdd\""));
        assert_eq!(
            deserialize(text.as_bytes())
                .unwrap()
                .caption_style
                .text_rgba,
            0xAABB_CCDD
        );

        for spelling in [
            "\"AABBCCDD\"",
            "\"#aabbccdd\"",
            "\"abc\"",
            "\"aabbccddee\"",
            "0",
        ] {
            let broken = text.replace(
                "\"text_rgba\":\"aabbccdd\"",
                &format!("\"text_rgba\":{spelling}"),
            );
            assert!(
                deserialize(broken.as_bytes()).is_err(),
                "accepted {spelling}"
            );
        }
    }

    #[test]
    fn a_half_specified_caption_style_is_refused() {
        // `tests/test_project_io.c:834`,
        // `project_io_rejects_a_half_specified_or_misspelled_caption_style`.
        let project = valid_project();
        let text = serialize(&project).unwrap();
        for member in [
            "\"face\":\"alegreya\",",
            "\"box\":\"plate\",",
            "\"anchor\":\"bottom_center\",",
            "\"size_scale\":0.047,",
            "\"margin_scale\":0.065,",
            "\"width_scale\":0.82,",
            "\"text_rgba\":\"ffffffff\",",
            "\"box_rgba\":\"000000b7\",",
            ",\"font\":null",
        ] {
            let reduced = text.replace(member, "");
            assert!(reduced != text, "{member} not found");
            assert_eq!(
                deserialize(reduced.as_bytes()).unwrap_err(),
                ProjectIoError::MissingField,
                "{member} should be required once the style is present"
            );
        }
        // A misspelled member is unknown, not ignored.
        let misspelled = text.replace("\"margin_scale\":", "\"margin-scale\":");
        assert_eq!(
            deserialize(misspelled.as_bytes()).unwrap_err(),
            ProjectIoError::UnknownField
        );
    }

    #[test]
    fn default_caption_effects_are_not_written_and_absence_is_the_default() {
        let project = valid_project();
        assert!(project.caption_style.effects.is_default());
        let text = serialize(&project).unwrap();
        assert!(
            !text.contains("\"effects\""),
            "a default effects block must not widen pre-effects files"
        );
        assert!(deserialize(text.as_bytes())
            .unwrap()
            .caption_style
            .effects
            .is_default());
    }

    #[test]
    fn authored_caption_effects_round_trip_exactly() {
        let mut project = valid_project();
        project.caption_style.effects = CaptionEffects {
            glow_strength: 0.8,
            glow_radius: 0.25,
            glow_rgba: 0x39FF_88E0,
            glow_pulse: EffectDrive::Bass,
            glow_pulse_depth: 0.75,
            pulse_tuning: DriveTuning {
                input_min: 0.2,
                input_max: 0.9,
                output_min: 0.1,
                output_max: 1.0,
                curve: Interpolation::Smoothstep,
                clamp: true,
            },
            glow_hue_drive: EffectDrive::Time,
            glow_hue_range: 200.0,
            hue_tuning: DriveTuning::default(),
            shadow_blur: 0.12,
            shadow_opacity: 0.9,
            plate_roundness: 0.3,
        };
        let text = serialize(&project).unwrap();
        assert!(text.contains("\"effects\":{\"glow_strength\":"));
        assert!(text.contains("\"glow_rgba\":\"39ff88e0\""));
        assert!(text.contains("\"glow_pulse\":\"bass\""));
        assert!(
            text.contains("\"pulse_tuning\":{\"input_min\":0.2,")
                && text.contains("\"curve\":\"smoothstep\""),
            "an authored tuning block is written: {text}"
        );
        assert!(
            !text.contains("\"hue_tuning\""),
            "a default tuning block must not widen pre-tuning effects files"
        );
        let reparsed = round_trip(&project);
        assert_eq!(
            reparsed.caption_style.effects,
            project.caption_style.effects
        );
    }

    #[test]
    fn a_half_specified_drive_tuning_is_refused() {
        let mut project = valid_project();
        project.caption_style.effects.glow_strength = 0.5;
        project.caption_style.effects.hue_tuning.output_max = 0.7;
        let text = serialize(&project).unwrap();
        assert!(text.contains("\"hue_tuning\""));
        let reduced = text.replace("\"input_max\":1,", "");
        assert!(reduced != text);
        assert_eq!(
            deserialize(reduced.as_bytes()).unwrap_err(),
            ProjectIoError::MissingField
        );
        let misspelled = text.replace("\"output_max\":0.7", "\"output_maxx\":0.7");
        assert_eq!(
            deserialize(misspelled.as_bytes()).unwrap_err(),
            ProjectIoError::UnknownField
        );
    }

    #[test]
    fn a_half_specified_effects_block_is_refused() {
        let mut project = valid_project();
        project.caption_style.effects.glow_strength = 0.5;
        let text = serialize(&project).unwrap();
        for member in [
            "\"glow_radius\":0.18,",
            "\"glow_pulse\":\"none\",",
            "\"shadow_opacity\":1,",
        ] {
            let reduced = text.replace(member, "");
            assert!(reduced != text, "{member} not found in {text}");
            assert_eq!(
                deserialize(reduced.as_bytes()).unwrap_err(),
                ProjectIoError::MissingField,
                "{member} should be required once effects are present"
            );
        }
        let misspelled = text.replace("\"glow_hue_drive\":", "\"glow_hue_driver\":");
        assert_eq!(
            deserialize(misspelled.as_bytes()).unwrap_err(),
            ProjectIoError::UnknownField
        );
        let bad_drive = text.replace("\"glow_pulse\":\"none\"", "\"glow_pulse\":\"midi\"");
        assert!(deserialize(bad_drive.as_bytes()).is_err());
    }

    #[test]
    fn an_imported_caption_face_round_trips_with_its_licence() {
        let mut project = valid_project();
        project.caption_style.face = CaptionFace::Imported;
        project.caption_style.font = Some(FontAsset {
            path: "fixture.assets/fonts/face.ttf".into(),
            sha256: sha256::digest_hex(b"face"),
            family: "Some Family".into(),
            licence_path: "fixture.assets/fonts/OFL.txt".into(),
            licence_sha256: sha256::digest_hex(b"licence"),
            licence_name: "OFL-1.1".into(),
        });
        let reparsed = round_trip(&project);
        assert_eq!(reparsed.caption_style, project.caption_style);

        // The face without its asset does not survive, in either direction: a
        // project whose captions cannot be reproduced from the file is invalid.
        let text = serialize(&project).unwrap();
        let orphaned = text.replace(&serialize_font_block(&project), "\"font\":null");
        assert_eq!(
            deserialize(orphaned.as_bytes()).unwrap_err(),
            ProjectIoError::Validation
        );
        let orphaned = text.replace("\"face\":\"imported\"", "\"face\":\"alegreya\"");
        assert_eq!(
            deserialize(orphaned.as_bytes()).unwrap_err(),
            ProjectIoError::Validation
        );
    }

    fn serialize_font_block(project: &Project) -> String {
        let font = project.caption_style.font.as_ref().unwrap();
        format!(
            "\"font\":{{\"path\":\"{}\",\"sha256\":\"{}\",\"family\":\"{}\",\
             \"licence_path\":\"{}\",\"licence_sha256\":\"{}\",\"licence_name\":\"{}\"}}",
            font.path,
            font.sha256,
            font.family,
            font.licence_path,
            font.licence_sha256,
            font.licence_name
        )
    }

    #[test]
    fn arrays_are_bounded_before_their_elements_are_parsed() {
        let mut project = valid_project();
        project.scenes[0].mappings = vec![constant_mapping("settings.spectrum.amplitude", 0.5)];
        let text = serialize(&project).unwrap();
        let one = text
            .split_once("\"mappings\":[")
            .map(|(_, tail)| tail.split_once("]}").unwrap().0.to_owned())
            .unwrap();

        // MAX_MAPPINGS_PER_SCENE + 1 entries: capacity, not validation.
        let mut many = String::new();
        for index in 0..=MAX_MAPPINGS_PER_SCENE {
            if index > 0 {
                many.push(',');
            }
            many.push_str(&one.replace("amplitude", &format!("p{index}")));
        }
        let broken = text.replace(&one, &many);
        assert_eq!(
            deserialize(broken.as_bytes()).unwrap_err(),
            ProjectIoError::Capacity
        );
    }

    #[test]
    fn too_many_scenes_is_a_capacity_error() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        let one = text
            .split_once("\"scenes\":[")
            .map(|(_, tail)| tail.split_once("]}],").unwrap().0.to_owned())
            .unwrap();
        let mut many = String::new();
        for index in 0..=MAX_SCENES {
            if index > 0 {
                many.push(',');
            }
            many.push_str(&one.replace(
                "\"instance_id\":1",
                &format!("\"instance_id\":{}", index + 1),
            ));
            many.push_str("]}");
        }
        // Trim the trailing marker the last element already carries.
        let many = many.trim_end_matches("]}").to_owned();
        let broken = text.replace(&one, &many);
        assert_eq!(
            deserialize(broken.as_bytes()).unwrap_err(),
            ProjectIoError::Capacity
        );
    }

    #[test]
    fn malformed_json_is_a_syntax_error() {
        let text = serialize(&valid_project()).unwrap();
        for broken in [
            text.trim_end_matches('}').to_owned(),
            text.replacen('{', "", 1),
            text.replace("\"metadata\":", "\"metadata\""),
            text.replace(",\"audio\":", "\"audio\":"),
            format!("[{text}]"),
            "{".to_owned(),
            "null".to_owned(),
        ] {
            assert!(
                deserialize(broken.as_bytes()).is_err(),
                "accepted {}",
                &broken[..broken.len().min(60)]
            );
        }
    }

    #[test]
    fn a_valid_document_that_fails_the_model_is_a_validation_error() {
        let project = valid_project();
        let text = serialize(&project).unwrap();
        // Each of these is syntactically perfect and semantically impossible, so the
        // parser has to hand off to the model rather than deciding for itself.
        for (from, to) in [
            ("\"channels\":2", "\"channels\":0"),
            ("\"sample_rate\":44100", "\"sample_rate\":768001"),
            ("\"opacity\":1", "\"opacity\":2"),
            ("\"project_id\":\"fixture\"", "\"project_id\":\"has space\""),
            ("\"instance_id\":1", "\"instance_id\":0"),
        ] {
            let broken = text.replace(from, to);
            assert!(broken != text, "{from} not found");
            assert_eq!(
                deserialize(broken.as_bytes()).unwrap_err(),
                ProjectIoError::Validation,
                "{to}"
            );
        }
    }

    #[test]
    fn an_unsorted_lyric_lane_is_rejected_not_quietly_sorted() {
        let mut project = valid_project();
        project
            .lyrics
            .insert(LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "first".into(),
                origin: Default::default(),
            })
            .unwrap();
        project
            .lyrics
            .insert(LyricCue {
                id: 0,
                start_seconds: 5.0,
                end_seconds: 6.0,
                text: "second".into(),
                origin: Default::default(),
            })
            .unwrap();
        let text = serialize(&project).unwrap();
        let first = "{\"id\":1,\"start_seconds\":1,\"end_seconds\":2,\"text\":\"first\"}";
        let second = "{\"id\":2,\"start_seconds\":5,\"end_seconds\":6,\"text\":\"second\"}";
        assert!(text.contains(first) && text.contains(second));
        let swapped = text.replace(&format!("{first},{second}"), &format!("{second},{first}"));
        assert_eq!(
            deserialize(swapped.as_bytes()).unwrap_err(),
            ProjectIoError::Validation,
            "a file listing cues out of order must be refused, not sorted"
        );
    }

    /// LX1. The origin field has no C counterpart, so `differential_project_io.sh`
    /// cannot pin it — these do, and the first assertion is the one that keeps
    /// that harness meaningful: a document of hand-placed cues must still
    /// serialize to the exact bytes it did before the field existed.
    #[test]
    fn a_cue_origin_survives_the_round_trip_and_costs_a_default_cue_nothing() {
        use crate::project::lyrics::CueOrigin;

        let mut project = valid_project();
        for (start, origin) in [
            (1.0, CueOrigin::UserApplied),
            (2.0, CueOrigin::InferredCertain),
            (3.0, CueOrigin::InferredAmbiguous),
            (4.0, CueOrigin::Potential),
        ] {
            project
                .lyrics
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 0.5,
                    text: format!("line at {start}"),
                    origin,
                })
                .unwrap();
        }
        let text = serialize(&project).unwrap();

        // The default is absent from the file, and the other three are present
        // by name. Absence is the compatibility guarantee; presence is the
        // provenance guarantee.
        assert!(
            text.contains("\"text\":\"line at 1\"}"),
            "a user-applied cue must not gain a field: {text}"
        );
        for token in ["certain", "ambiguous", "potential"] {
            assert!(
                text.contains(&format!("\"origin\":\"{token}\"")),
                "{token} must survive into the file: {text}"
            );
        }

        let reloaded = deserialize(text.as_bytes()).unwrap();
        let origins: Vec<CueOrigin> = reloaded
            .lyrics
            .cues()
            .iter()
            .map(|cue| cue.origin)
            .collect();
        assert_eq!(
            origins,
            vec![
                CueOrigin::UserApplied,
                CueOrigin::InferredCertain,
                CueOrigin::InferredAmbiguous,
                CueOrigin::Potential,
            ]
        );

        // A file written before LX1 has no `origin` anywhere and still opens,
        // with every cue reading as the user's. This is the case that would
        // otherwise be found by a user losing a project, not by a test.
        let stripped = text
            .replace(",\"origin\":\"certain\"", "")
            .replace(",\"origin\":\"ambiguous\"", "")
            .replace(",\"origin\":\"potential\"", "");
        assert!(!stripped.contains("origin"));
        let legacy = deserialize(stripped.as_bytes()).unwrap();
        assert!(legacy
            .lyrics
            .cues()
            .iter()
            .all(|cue| cue.origin == CueOrigin::UserApplied));

        // And an origin this build has never heard of is refused rather than
        // read as the default, because "user applied" is a claim about a human.
        let forged = text.replace("\"origin\":\"certain\"", "\"origin\":\"verified\"");
        assert_eq!(
            deserialize(forged.as_bytes()).unwrap_err(),
            ProjectIoError::String
        );
    }

    #[test]
    fn an_unsorted_event_lane_is_rejected_not_quietly_sorted() {
        let mut project = valid_project();
        for (timestamp, id) in [(1.0, 1u64), (5.0, 2)] {
            project
                .manual_events
                .record(EventRecord {
                    timestamp_seconds: timestamp,
                    id,
                    event_type: EventType::Cue as u32,
                    value_count: 1,
                    values: [1.0, 0.0, 0.0, 0.0],
                })
                .unwrap();
        }
        let text = serialize(&project).unwrap();
        let first = "{\"timestamp_seconds\":1,\"id\":1,\"type\":\"cue\",\"values\":[1]}";
        let second = "{\"timestamp_seconds\":5,\"id\":2,\"type\":\"cue\",\"values\":[1]}";
        assert!(text.contains(first) && text.contains(second), "{text}");
        let swapped = text.replace(&format!("{first},{second}"), &format!("{second},{first}"));
        assert_eq!(
            deserialize(swapped.as_bytes()).unwrap_err(),
            ProjectIoError::Validation
        );
    }

    #[test]
    fn opening_a_project_never_rewrites_it() {
        // The invariant, as a test: parse, do nothing, re-serialize, compare. Any
        // "helpful" normalization in the reader shows up here.
        let mut project = valid_project();
        project.scene_switches = SceneSwitchSuggestions {
            enabled: true,
            cues: vec![SceneSwitchSuggestion {
                id: 1,
                start_seconds: 0.0,
                end_seconds: 60.0,
                scene_name: "spectrum".into(),
                strength: 0.75,
                settings: vec![0.5, 0.25],
            }],
        };
        project.scene_presets = vec![ScenePreset {
            id: 1,
            scene_name: "loom".into(),
            name: "Dense".into(),
            settings: vec![0.5, 1.5],
        }];
        let original = serialize(&project).unwrap();
        let reopened = serialize(&deserialize(original.as_bytes()).unwrap()).unwrap();
        assert_eq!(original, reopened);
    }

    // -- the preset store ----------------------------------------------------

    fn preset_store() -> PresetStoreDocument {
        PresetStoreDocument {
            next_id: 3,
            presets: vec![
                ScenePreset {
                    id: 1,
                    scene_name: "spectrum".into(),
                    name: "Bright".into(),
                    settings: vec![1.0, 0.5],
                },
                ScenePreset {
                    id: 2,
                    scene_name: "loom".into(),
                    name: "Dense".into(),
                    settings: vec![0.25],
                },
            ],
        }
    }

    #[test]
    fn the_preset_store_round_trips() {
        let store = preset_store();
        let text = preset_store_serialize(&store).unwrap();
        assert_eq!(preset_store_deserialize(text.as_bytes()).unwrap(), store);
    }

    #[test]
    fn the_preset_store_refuses_ids_at_or_above_next_id() {
        let mut store = preset_store();
        store.next_id = 2;
        assert_eq!(
            preset_store_serialize(&store).unwrap_err(),
            ProjectIoError::Validation
        );
        // And on the way in, too.
        let text = preset_store_serialize(&preset_store()).unwrap();
        let broken = text.replace("\"next_id\":3", "\"next_id\":2");
        assert_eq!(
            preset_store_deserialize(broken.as_bytes()).unwrap_err(),
            ProjectIoError::Validation
        );
    }

    #[test]
    fn the_preset_store_refuses_duplicates_zero_ids_and_empty_names() {
        type Breaker = fn(&mut PresetStoreDocument);
        let breakers: Vec<(&str, Breaker)> = vec![
            ("duplicate id", |store| store.presets[1].id = 1),
            ("zero id", |store| store.presets[0].id = 0),
            ("empty name", |store| {
                store.presets[0].name = String::new();
            }),
            ("empty scene", |store| {
                store.presets[0].scene_name = String::new();
            }),
            ("no settings", |store| store.presets[0].settings.clear()),
            ("too many settings", |store| {
                store.presets[0].settings = vec![0.0; MAX_CONTROLS + 1];
            }),
            ("non-finite setting", |store| {
                store.presets[0].settings[0] = f32::NAN;
            }),
        ];
        for (name, break_it) in breakers {
            let mut store = preset_store();
            break_it(&mut store);
            assert!(!store.is_valid(), "{name}");
            assert_eq!(
                preset_store_serialize(&store).unwrap_err(),
                ProjectIoError::Validation,
                "{name}"
            );
        }
    }

    #[test]
    fn the_preset_store_is_as_strict_as_the_project_codec() {
        let text = preset_store_serialize(&preset_store()).unwrap();
        assert_eq!(
            preset_store_deserialize(text.replacen('{', "{\"surprise\":1,", 1).as_bytes())
                .unwrap_err(),
            ProjectIoError::UnknownField
        );
        assert_eq!(
            preset_store_deserialize(
                text.replace("musializer.presets/v1", "musializer.presets/v2")
                    .as_bytes()
            )
            .unwrap_err(),
            ProjectIoError::Schema
        );
        assert_eq!(
            preset_store_deserialize(without_block(&text, "next_id").as_bytes()).unwrap_err(),
            ProjectIoError::MissingField
        );
        assert_eq!(
            preset_store_deserialize(b"").unwrap_err(),
            ProjectIoError::InputSize
        );
    }

    // -- transaction and path policy -----------------------------------------

    #[test]
    fn a_transaction_file_is_a_sibling_of_its_destination() {
        assert_eq!(
            temporary_path("/home/user/show.musi", 4242, 7).unwrap(),
            "/home/user/.musializer-project-4242-0000000000000007.tmp"
        );
        assert_eq!(
            temporary_path("show.musi", 1, 1).unwrap(),
            ".musializer-project-1-0000000000000001.tmp"
        );
        // A directory is not a destination.
        assert_eq!(temporary_path("/home/user/", 1, 1), None);
        assert_eq!(temporary_path("", 1, 1), None);
    }

    #[test]
    fn a_transaction_name_changes_with_the_nonce() {
        let first = temporary_path("/tmp/show.musi", 9, 1).unwrap();
        let second = temporary_path("/tmp/show.musi", 9, 2).unwrap();
        assert_ne!(first, second, "the collision policy needs distinct names");
    }

    #[test]
    fn a_project_directory_is_the_part_before_the_last_separator() {
        assert_eq!(directory_of("/home/user/show.musi"), Some("/home/user"));
        assert_eq!(directory_of("show.musi"), Some("."));
        assert_eq!(directory_of("/show.musi"), Some("/"));
        assert_eq!(directory_of("/home/user/"), None);
        assert_eq!(directory_of(""), None);
    }

    #[test]
    fn unambiguous_relative_paths_refuse_traversal_and_backslashes() {
        assert!(is_unambiguous_relative_path("show.assets/fonts/face.ttf"));
        assert!(!is_unambiguous_relative_path(""));
        assert!(!is_unambiguous_relative_path("/absolute"));
        assert!(!is_unambiguous_relative_path("../escape"));
        assert!(!is_unambiguous_relative_path("./here"));
        assert!(!is_unambiguous_relative_path("double//slash"));
        assert!(
            !is_unambiguous_relative_path("back\\slash"),
            "a stored backslash is a portable separator, so a real one must stay absolute"
        );
    }

    #[test]
    fn relative_descendant_paths_are_only_produced_for_real_descendants() {
        assert_eq!(
            relative_descendant_path("/home/user", "/home/user/show.assets/audio/x.wav"),
            Some("show.assets/audio/x.wav")
        );
        assert_eq!(
            relative_descendant_path("/home/user/", "/home/user/x.wav"),
            Some("x.wav")
        );
        assert_eq!(relative_descendant_path("/home/user", "/home/user"), None);
        assert_eq!(
            relative_descendant_path("/home/user", "/home/userx/x"),
            None
        );
        assert_eq!(relative_descendant_path("/home/user", "/elsewhere/x"), None);
        assert_eq!(relative_descendant_path("/", "/x.wav"), Some("x.wav"));
    }
}
