//! The Rust half of `tools/differential_project_io.sh`.
//!
//! Drives `project::io`'s `.musi` codec in both directions and prints the exact
//! line set `tests/differential/project_io_oracle.c` prints, so the driver can
//! hold any two dumps against each other field by field.
//!
//! Three modes, matching the C harness so either program can sit at either end of
//! a chain:
//!
//! ```text
//! build   <out.musi>            construct the fixture, serialize it, dump it
//! read    <in.musi>             parse, dump what the parser produced
//! rewrite <in.musi> <out.musi>  parse, dump, and serialize the parsed value
//! ```
//!
//! The composition is the test that matters — `C write -> Rust read -> Rust
//! re-write -> C read` — because a field that is *parsed and then dropped on
//! re-write* survives either single direction and dies in the round trip.
//!
//! # Why the numbers are printed at 17 digits and compared as values
//!
//! C writes JSON numbers with `%.17g`; Rust writes its shortest round-tripping
//! form, so `0.2345` here is `0.23449999999999999` there. That difference is
//! settled as *not* a parity bug (`io::write_f64`'s doc comment carries the
//! reasoning: nothing in the oracle ever hashes or byte-compares a `.musi`), and
//! this harness therefore compares values and never bytes.
//!
//! The *dump* nonetheless emulates `%.17g` rather than using Rust's `Display`,
//! which is not normalization in either direction: 17 significant digits is
//! lossless for a `f64`, so both sides spell the same double the same way and a
//! plain `diff` becomes a usable first look. The Python comparator in the driver
//! is still the authority, because glibc and Rust need not break an exact
//! rounding tie identically.
//!
//! # The fixture
//!
//! Built here and again, independently, in the C harness. That duplication is
//! `AGENTS.md`'s rule, not an oversight: a shared generator can hide the very
//! difference the harness exists to find. Every field carries a distinctive
//! non-default value, strings carry non-ASCII UTF-8 wherever the schema allows
//! one, and the `f32`-typed fields carry values whose `f64` widenings are
//! deliberately ugly — `f32 -> f64 -> text -> f64 -> f32` is where a genuine
//! round-trip loss would hide.

use std::fmt::Write as _;
use std::process::ExitCode;

use musializer_core::project::event_timeline::EventTimeline;
use musializer_core::project::frame_lanes::{ProjectFrameLanes, SceneFrameTiming};
use musializer_core::project::io;
use musializer_core::project::lyrics::{LyricCue, LyricsDocument};
use musializer_core::project::model::{
    AnalysisLaneKind, AnalysisLaneReference, AsciiImageAsset, AssetMode, AudioAsset, BlendMode,
    CaptionAnchor, CaptionBox, CaptionFace, CaptionStyle, FontAsset, Metadata, OutputFormat,
    OutputQuality, OutputSettings, ParameterCue, Project, Provenance, SceneEntry, ScenePreset,
    SceneSwitchSuggestion, SceneSwitchSuggestions,
};
use musializer_core::scene::events::{EventRecord, EventType, VALUE_CAPACITY};
use musializer_core::scene::routes::{AnalysisSource, Interpolation, ParameterMapping};
use musializer_core::scene::settings::MAX_CONTROLS;
use musializer_core::scene::{SceneAudioFrame, SceneSettings};

// ---------------------------------------------------------------------------
// The dump
// ---------------------------------------------------------------------------

/// C's `%.17g`, reproduced.
///
/// Seventeen significant digits round-trip every `f64`, so this is a lossless
/// spelling and not a rounding of the C's output. `%g` drops trailing zeros from
/// the fraction and switches to `%e` when the decimal exponent is below -4 or at
/// least the precision; the exponent then carries at least two digits.
fn format_g17(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    // `{:e}` would spell zero as `0e0`, and `%.17g` distinguishes the two zeroes.
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }

    // Precision 16 after the point is 17 significant digits, correctly rounded.
    let scientific = format!("{value:.16e}");
    let (mantissa, exponent_text) = scientific
        .split_once('e')
        .expect("Rust's `{:e}` always emits an exponent");
    let exponent: i32 = exponent_text
        .parse()
        .expect("Rust's `{:e}` always emits a decimal exponent");
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    debug_assert_eq!(digits.len(), 17, "precision 16 is 17 significant digits");

    if !(-4..17).contains(&exponent) {
        let fraction = digits[1..].trim_end_matches('0');
        let point = if fraction.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        let magnitude = exponent.abs();
        return format!(
            "{sign}{}{point}{fraction}e{exponent_sign}{magnitude:02}",
            &digits[..1]
        );
    }
    if exponent >= 0 {
        let split = (exponent + 1) as usize;
        let fraction = digits[split..].trim_end_matches('0');
        let point = if fraction.is_empty() { "" } else { "." };
        return format!("{sign}{}{point}{fraction}", &digits[..split]);
    }
    let leading_zeros = "0".repeat((-exponent - 1) as usize);
    let fraction = digits.trim_end_matches('0');
    format!("{sign}0.{leading_zeros}{fraction}")
}

/// Printable ASCII passes through; everything else -- spaces, tabs, newlines and
/// every byte of a multi-byte character -- becomes `%xx`, so a title containing a
/// quote or a newline cannot break its own dump line. `%`, `[` and `]` are escaped
/// too, which makes the encoding unambiguous.
fn dump_string(name: &str, value: &str) {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if (0x21..=0x7E).contains(&byte) && byte != b'%' && byte != b'[' && byte != b']' {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02x}");
        }
    }
    println!("{name} [{encoded}]");
}

fn dump_real(name: &str, value: f64) {
    println!("{name} {}", format_g17(value));
}

fn dump_uint(name: &str, value: u64) {
    println!("{name} {value}");
}

fn dump_bool(name: &str, value: bool) {
    println!("{name} {value}");
}

/// The Rust enums are total, so `OUT-OF-RANGE` can only ever come from the C side
/// or from `EventRecord::event_type`, which is a raw `u32` because the C field is.
fn dump_enum(name: &str, value: &str) {
    println!("{name} {value}");
}

fn event_type_name(raw: u32) -> &'static str {
    match EventType::from_raw(raw) {
        Some(EventType::Lyric) => "lyric",
        Some(EventType::Semantic) => "semantic",
        Some(EventType::Cue) => "cue",
        Some(EventType::Custom) => "custom",
        None => "OUT-OF-RANGE",
    }
}

fn dump_mapping(prefix: &str, scene: usize, index: usize, mapping: &ParameterMapping) {
    let at = format!("{prefix}.{scene}.mapping.{index}");
    dump_string(&format!("{at}.parameter"), &mapping.parameter);
    dump_enum(&format!("{at}.source"), mapping.source.canonical_name());
    dump_uint(&format!("{at}.band_index"), u64::from(mapping.band_index));
    dump_real(&format!("{at}.input_min"), mapping.input_min);
    dump_real(&format!("{at}.input_max"), mapping.input_max);
    dump_real(&format!("{at}.output_min"), mapping.output_min);
    dump_real(&format!("{at}.output_max"), mapping.output_max);
    dump_enum(
        &format!("{at}.interpolation"),
        mapping.interpolation.canonical_name(),
    );
    dump_bool(&format!("{at}.clamp"), mapping.clamp);
}

fn dump_events(prefix: &str, timeline: &EventTimeline) {
    dump_uint(&format!("{prefix}.count"), timeline.len() as u64);
    for (index, event) in timeline.events().iter().enumerate() {
        dump_real(
            &format!("{prefix}.{index}.timestamp_seconds"),
            event.timestamp_seconds,
        );
        dump_uint(&format!("{prefix}.{index}.id"), event.id);
        dump_enum(
            &format!("{prefix}.{index}.type"),
            event_type_name(event.event_type),
        );
        dump_uint(
            &format!("{prefix}.{index}.value_count"),
            u64::from(event.value_count),
        );
        for (position, value) in event.values().iter().enumerate() {
            dump_real(
                &format!("{prefix}.{index}.value.{position}"),
                f64::from(*value),
            );
        }
    }
}

/// The application-boundary selection made from the project that was just read.
/// These times pin every lyric start/end and the semantic cue boundary at 12.5.
fn dump_frame_lanes(project: &Project) {
    let settings = SceneSettings::new();
    for (index, time_seconds) in [0.0, 1.5, 4.25, 8.0, 12.5, FIXTURE_DURATION]
        .into_iter()
        .enumerate()
    {
        let prefix = format!("frame_lane.{index}");
        let lanes = ProjectFrameLanes::build(
            time_seconds,
            &project.lyrics,
            &project.semantic_events,
            &project.manual_events,
        );
        let status = lanes.status();
        let frame = lanes.scene_frame(
            SceneFrameTiming {
                time_seconds,
                duration_seconds: project.audio.duration_seconds,
                ..SceneFrameTiming::default()
            },
            SceneAudioFrame::default(),
            &settings,
            None,
        );
        dump_real(&format!("{prefix}.time"), time_seconds);
        dump_uint(&format!("{prefix}.lyric_id"), status.lyric_id.unwrap_or(0));
        dump_bool(
            &format!("{prefix}.semantic.available"),
            status.semantic_available,
        );
        dump_uint(
            &format!("{prefix}.semantic.source_id"),
            status.semantic_source_id,
        );
        dump_real(
            &format!("{prefix}.semantic.energy"),
            f64::from(frame.semantic.energy),
        );
        dump_real(
            &format!("{prefix}.semantic.tension"),
            f64::from(frame.semantic.tension),
        );
        dump_real(
            &format!("{prefix}.semantic.valence"),
            f64::from(frame.semantic.valence),
        );
        dump_real(
            &format!("{prefix}.semantic.confidence"),
            f64::from(frame.semantic.confidence),
        );
        dump_uint(&format!("{prefix}.events.count"), frame.events.len() as u64);
        for (event_index, event) in frame.events.events.iter().enumerate() {
            dump_real(
                &format!("{prefix}.events.{event_index}.time"),
                event.timestamp_seconds,
            );
            dump_uint(&format!("{prefix}.events.{event_index}.id"), event.id);
            dump_enum(
                &format!("{prefix}.events.{event_index}.type"),
                event_type_name(event.event_type),
            );
        }
    }
}

fn dump_project(project: &Project) {
    dump_uint("schema_version", u64::from(project.schema_version));

    let metadata = &project.metadata;
    dump_string("metadata.project_id", &metadata.project_id);
    dump_string("metadata.title", &metadata.title);
    dump_string("metadata.author", &metadata.author);
    dump_string("metadata.created_utc", &metadata.created_utc);
    dump_string("metadata.modified_utc", &metadata.modified_utc);
    dump_string(
        "metadata.application_version",
        &metadata.application_version,
    );

    let audio = &project.audio;
    dump_enum("audio.mode", audio.mode.canonical_name());
    dump_string("audio.path", &audio.path);
    dump_string("audio.sha256", &audio.sha256);
    dump_real("audio.duration_seconds", audio.duration_seconds);
    dump_uint("audio.sample_rate", u64::from(audio.sample_rate));
    dump_uint("audio.channels", u64::from(audio.channels));

    // The C carries `present` as a flag beside a zeroed struct where Rust uses an
    // `Option`, so the absent case has to be spelled out to keep the line set
    // identical -- and an absent asset with a non-empty path would be a real
    // difference worth seeing.
    let absent_ascii = AsciiImageAsset::default();
    let ascii = project.ascii_image.as_ref().unwrap_or(&absent_ascii);
    dump_bool("ascii_image.present", project.ascii_image.is_some());
    dump_string("ascii_image.path", &ascii.path);
    dump_string("ascii_image.sha256", &ascii.sha256);
    dump_uint("ascii_image.columns", u64::from(ascii.columns));
    dump_uint("ascii_image.rows", u64::from(ascii.rows));

    let caption = &project.caption_style;
    dump_enum("caption.face", caption.face.canonical_name());
    dump_enum("caption.box", caption.box_style.canonical_name());
    dump_enum("caption.anchor", caption.anchor.canonical_name());
    dump_real("caption.size_scale", caption.size_scale);
    dump_real("caption.margin_scale", caption.margin_scale);
    dump_real("caption.width_scale", caption.width_scale);
    dump_uint("caption.text_rgba", u64::from(caption.text_rgba));
    dump_uint("caption.box_rgba", u64::from(caption.box_rgba));
    let absent_font = FontAsset::default();
    let font = caption.font.as_ref().unwrap_or(&absent_font);
    dump_bool("caption.font.present", caption.font.is_some());
    dump_string("caption.font.path", &font.path);
    dump_string("caption.font.sha256", &font.sha256);
    dump_string("caption.font.family", &font.family);
    dump_string("caption.font.licence_path", &font.licence_path);
    dump_string("caption.font.licence_sha256", &font.licence_sha256);
    dump_string("caption.font.licence_name", &font.licence_name);

    let output = &project.output;
    dump_uint("output.width", u64::from(output.width));
    dump_uint("output.height", u64::from(output.height));
    dump_uint("output.fps_numerator", u64::from(output.fps_numerator));
    dump_uint("output.fps_denominator", u64::from(output.fps_denominator));
    dump_real("output.start_seconds", output.start_seconds);
    dump_real("output.end_seconds", output.end_seconds);
    dump_enum("output.format", output.format.canonical_name());
    dump_enum("output.quality", output.quality.canonical_name());

    dump_uint("deterministic_seed", project.deterministic_seed);

    dump_uint("scene.count", project.scenes.len() as u64);
    for (index, scene) in project.scenes.iter().enumerate() {
        dump_uint(&format!("scene.{index}.instance_id"), scene.instance_id);
        dump_string(&format!("scene.{index}.scene_type"), &scene.scene_type);
        dump_bool(&format!("scene.{index}.enabled"), scene.enabled);
        dump_real(&format!("scene.{index}.start_seconds"), scene.start_seconds);
        dump_real(&format!("scene.{index}.end_seconds"), scene.end_seconds);
        dump_real(&format!("scene.{index}.opacity"), scene.opacity);
        dump_enum(
            &format!("scene.{index}.blend_mode"),
            scene.blend_mode.canonical_name(),
        );
        dump_uint(
            &format!("scene.{index}.mapping.count"),
            scene.mappings.len() as u64,
        );
        for (position, mapping) in scene.mappings.iter().enumerate() {
            dump_mapping("scene", index, position, mapping);
        }
    }

    dump_uint("cue.count", project.cues.len() as u64);
    for (index, cue) in project.cues.iter().enumerate() {
        dump_uint(&format!("cue.{index}.cue_id"), cue.cue_id);
        dump_uint(&format!("cue.{index}.target_scene_id"), cue.target_scene_id);
        dump_string(&format!("cue.{index}.parameter"), &cue.parameter);
        dump_real(&format!("cue.{index}.start_seconds"), cue.start_seconds);
        dump_real(&format!("cue.{index}.end_seconds"), cue.end_seconds);
        dump_real(&format!("cue.{index}.from_value"), cue.from_value);
        dump_real(&format!("cue.{index}.to_value"), cue.to_value);
        dump_enum(
            &format!("cue.{index}.interpolation"),
            cue.interpolation.canonical_name(),
        );
    }

    dump_uint("lane.count", project.analysis_lanes.len() as u64);
    for (index, lane) in project.analysis_lanes.iter().enumerate() {
        dump_enum(&format!("lane.{index}.kind"), lane.kind.canonical_name());
        dump_string(&format!("lane.{index}.path"), &lane.path);
        dump_string(&format!("lane.{index}.sha256"), &lane.sha256);
        dump_string(&format!("lane.{index}.audio_sha256"), &lane.audio_sha256);
        let provenance = &lane.provenance;
        dump_string(
            &format!("lane.{index}.provenance.adapter"),
            &provenance.adapter,
        );
        dump_string(
            &format!("lane.{index}.provenance.adapter_version"),
            &provenance.adapter_version,
        );
        dump_string(
            &format!("lane.{index}.provenance.schema_version"),
            &provenance.schema_version,
        );
        dump_string(&format!("lane.{index}.provenance.model"), &provenance.model);
        dump_string(
            &format!("lane.{index}.provenance.provider"),
            &provenance.provider,
        );
        dump_string(
            &format!("lane.{index}.provenance.prompt_version"),
            &provenance.prompt_version,
        );
    }

    // `duration_seconds` is not a field in the file: the parser copies it from the
    // audio asset (`project_io.c:306`) and validation then insists the two agree.
    // It is dumped because a port that forgot the copy would still pass every
    // other line.
    dump_real("lyrics.duration_seconds", project.lyrics.duration_seconds());
    dump_uint("lyrics.next_id", project.lyrics.next_id());
    dump_uint("lyrics.count", project.lyrics.len() as u64);
    for (index, cue) in project.lyrics.cues().iter().enumerate() {
        dump_uint(&format!("lyrics.{index}.id"), cue.id);
        dump_real(&format!("lyrics.{index}.start_seconds"), cue.start_seconds);
        dump_real(&format!("lyrics.{index}.end_seconds"), cue.end_seconds);
        dump_string(&format!("lyrics.{index}.text"), &cue.text);
    }

    let switches = &project.scene_switches;
    dump_bool("switches.enabled", switches.enabled);
    dump_uint("switches.count", switches.cues.len() as u64);
    for (index, cue) in switches.cues.iter().enumerate() {
        dump_uint(&format!("switches.{index}.id"), cue.id);
        dump_real(
            &format!("switches.{index}.start_seconds"),
            cue.start_seconds,
        );
        dump_real(&format!("switches.{index}.end_seconds"), cue.end_seconds);
        dump_string(&format!("switches.{index}.scene_name"), &cue.scene_name);
        dump_real(
            &format!("switches.{index}.strength"),
            f64::from(cue.strength),
        );
        dump_uint(
            &format!("switches.{index}.setting_count"),
            cue.settings.len() as u64,
        );
        for (position, value) in cue.settings.iter().enumerate() {
            dump_real(
                &format!("switches.{index}.setting.{position}"),
                f64::from(*value),
            );
        }
    }

    dump_uint("presets.count", project.scene_presets.len() as u64);
    for (index, preset) in project.scene_presets.iter().enumerate() {
        dump_uint(&format!("presets.{index}.id"), preset.id);
        dump_string(&format!("presets.{index}.scene_name"), &preset.scene_name);
        dump_string(&format!("presets.{index}.name"), &preset.name);
        dump_uint(
            &format!("presets.{index}.setting_count"),
            preset.settings.len() as u64,
        );
        for (position, value) in preset.settings.iter().enumerate() {
            dump_real(
                &format!("presets.{index}.setting.{position}"),
                f64::from(*value),
            );
        }
    }

    dump_events("semantic_events", &project.semantic_events);
    dump_events("manual_events", &project.manual_events);
    dump_frame_lanes(project);
}

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// Exactly representable, and exactly three times the switch cue length below, so
/// the scene-switch plan tiles `[0, duration]` with no rounding slack at all.
const FIXTURE_DURATION: f64 = 187.3125;
const FIXTURE_SWITCH_STEP: f64 = 62.4375;

// The digests are literal 64-hex constants rather than hashes of real files. The
// codec never verifies a digest against its bytes -- that is the bundle
// machinery's job, in `musializer-runtime` -- so what the format requires here is
// the schema's own rule, sixty-four lowercase hex digits, and nothing more.
const AUDIO_SHA: &str = "1f2e3d4c5b6a798807162534435261708f9e0d1c2b3a4958675645342312a0b9";
const ASCII_SHA: &str = "aabbccdd001122334455667788990011ffeeddccbbaa99887766554433221100";
const FONT_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const LICENCE_SHA: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const LANE0_SHA: &str = "1111111122222222333333334444444455555555666666667777777788888888";
const LANE1_SHA: &str = "99999999aaaaaaaabbbbbbbbccccccccddddddddeeeeeeeeffffffff00000000";
const LANE2_SHA: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

// One parameter per mapping field, in the file's order, so the C harness's
// `set_mapping` call sites and these transcribe the same table.
#[allow(clippy::too_many_arguments)]
fn mapping(
    parameter: &str,
    source: AnalysisSource,
    band_index: u16,
    input_min: f64,
    input_max: f64,
    output_min: f64,
    output_max: f64,
    interpolation: Interpolation,
    clamp: bool,
) -> ParameterMapping {
    ParameterMapping {
        parameter: parameter.to_owned(),
        source,
        band_index,
        input_min,
        input_max,
        output_min,
        output_max,
        interpolation,
        clamp,
    }
}

fn event(timestamp_seconds: f64, id: u64, event_type: EventType, values: &[f32]) -> EventRecord {
    let mut record = EventRecord {
        timestamp_seconds,
        id,
        event_type: event_type as u32,
        value_count: values.len() as u8,
        values: [0.0; VALUE_CAPACITY],
    };
    record.values[..values.len()].copy_from_slice(values);
    record
}

/// One row of the parameter-cue table: `(id, scene, parameter, start, end, from,
/// to, interpolation)`. Named so the table below can stay one line per cue —
/// `#[rustfmt::skip]` keeps it column-checkable against the C harness, and
/// spelling the tuple inline would trip `clippy::type_complexity`.
type CueRow = (u64, u64, &'static str, f64, f64, f64, f64, Interpolation);

fn provenance(
    adapter: &str,
    adapter_version: &str,
    schema_version: &str,
    model: &str,
    provider: &str,
    prompt_version: &str,
) -> Provenance {
    Provenance {
        adapter: adapter.to_owned(),
        adapter_version: adapter_version.to_owned(),
        schema_version: schema_version.to_owned(),
        model: model.to_owned(),
        provider: provider.to_owned(),
        prompt_version: prompt_version.to_owned(),
    }
}

fn build_fixture() -> Project {
    // -- metadata. `project_id` is the one string here that must be a
    // `stable_name`, so it stays ASCII; every other one carries multi-byte UTF-8,
    // and `title` also carries a quote, a backslash, a tab and a newline so both
    // writers' escape sets are exercised.
    let metadata = Metadata {
        project_id: "round-trip.fixture:01".to_owned(),
        title: "Nächtliche Röhren — 夜の曲 ✦ \"quoted\" \\ back\ttab\nnewline".to_owned(),
        author: "Ægir Þórsson & 佐藤".to_owned(),
        created_utc: "2026-07-29T12:34:56.789Z".to_owned(),
        modified_utc: "2026-07-29T13:00:00Z".to_owned(),
        application_version: "rusty-musializer 0.1.0-δ+round·trip".to_owned(),
    };

    // -- audio. `Referenced` rather than the `Imported` default.
    let audio = AudioAsset {
        mode: AssetMode::Referenced,
        path: "média/Nächtliche Röhren.flac".to_owned(),
        sha256: AUDIO_SHA.to_owned(),
        duration_seconds: FIXTURE_DURATION,
        sample_rate: 48000,
        channels: 6,
    };

    // -- ascii image, present rather than `None`.
    let ascii_image = Some(AsciiImageAsset {
        path: "images/naïve grid ✦.png".to_owned(),
        sha256: ASCII_SHA.to_owned(),
        columns: 77,
        rows: 41,
    });

    // -- caption style. Every enum off its default, and the imported face, which
    // is the only arrangement that makes the font asset legal.
    let caption_style = CaptionStyle {
        face: CaptionFace::Imported,
        box_style: CaptionBox::Shadow,
        anchor: CaptionAnchor::TopRight,
        size_scale: 0.1234,
        margin_scale: 0.2345,
        width_scale: 0.9876,
        text_rgba: 0x1A2B_3C4D,
        box_rgba: 0xFEDC_BA98,
        font: Some(FontAsset {
            path: "fixture.assets/fonts/Ünicode Face.ttf".to_owned(),
            sha256: FONT_SHA.to_owned(),
            family: "Ünicode Face ✦".to_owned(),
            licence_path: "fixture.assets/fonts/OFL-1.1.txt".to_owned(),
            licence_sha256: LICENCE_SHA.to_owned(),
            licence_name: "SIL OFL 1.1".to_owned(),
        }),
        // Deliberately default: this fixture is compared against the frozen C,
        // which predates the effects block. A default block is not serialized,
        // so the C parser never sees it. Effects round-tripping is covered by
        // the Rust-only tests in `project::io`.
        effects: Default::default(),
    };

    // -- output. A non-integer frame rate, a partial range, and the last format
    // and quality in each table rather than the first.
    let output = OutputSettings {
        width: 3840,
        height: 2160,
        fps_numerator: 24000,
        fps_denominator: 1001,
        start_seconds: 0.5,
        end_seconds: FIXTURE_DURATION,
        format: OutputFormat::MovProres,
        quality: OutputQuality::Master,
    };

    // -- scenes. Three: one with a mapping for every source kind and every
    // interpolation kind, one with a single mapping, and one with none, so the
    // empty array is covered too.
    let scenes = vec![
        SceneEntry {
            instance_id: 1001,
            scene_type: "spectrum".to_owned(),
            enabled: true,
            start_seconds: 0.0,
            end_seconds: FIXTURE_DURATION,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mappings: vec![
                mapping(
                    "settings.spectrum.amplitude",
                    AnalysisSource::Rms,
                    0,
                    0.0,
                    1.0,
                    0.4,
                    2.2,
                    Interpolation::Step,
                    true,
                ),
                mapping(
                    "settings.spectrum.trail",
                    AnalysisSource::Peak,
                    0,
                    0.125,
                    0.875,
                    1.5,
                    0.25,
                    Interpolation::Linear,
                    false,
                ),
                mapping(
                    "settings.spectrum.saturation",
                    AnalysisSource::SpectralFlux,
                    0,
                    -2.5,
                    3.5,
                    -40.0,
                    40.0,
                    Interpolation::Smoothstep,
                    true,
                ),
                mapping(
                    "settings.spectrum.hue_swing",
                    AnalysisSource::BeatPhase,
                    0,
                    0.0,
                    // The C harness spells this `6.283185307179586`, which is the
                    // shortest form of exactly this double; comparison 0 in the
                    // driver would fail if the two literals disagreed.
                    std::f64::consts::TAU,
                    0.0,
                    360.0,
                    Interpolation::EaseIn,
                    false,
                ),
                // Equal output endpoints: legal for the model, and the spelling a
                // slider constant uses.
                mapping(
                    "settings.spectrum.band7",
                    AnalysisSource::Band,
                    7,
                    0.0,
                    1.0,
                    1.0,
                    1.0,
                    Interpolation::EaseOut,
                    true,
                ),
                // The largest band index the field can hold, so the u16 bound
                // round-trips.
                mapping(
                    "settings.spectrum.band_max",
                    AnalysisSource::Band,
                    u16::MAX,
                    1e-9,
                    1.0,
                    -1.0e6,
                    1.0e6,
                    Interpolation::Step,
                    false,
                ),
            ],
        },
        SceneEntry {
            instance_id: 2002,
            scene_type: "loom".to_owned(),
            enabled: false,
            start_seconds: 10.25,
            end_seconds: 100.5,
            opacity: 0.5,
            blend_mode: BlendMode::Multiply,
            // `f64::from(0.4_f32)` is 0.400000005960464477539062500: the value the
            // task names as where an f32 -> f64 -> text -> f64 -> f32 loss would
            // hide. Written as the widening rather than as a decimal literal so
            // the intent survives a later reader.
            mappings: vec![mapping(
                "settings.loom.weight",
                AnalysisSource::Rms,
                0,
                0.0,
                1.0,
                f64::from(0.4_f32),
                2.5,
                Interpolation::Smoothstep,
                false,
            )],
        },
        SceneEntry {
            instance_id: 3003,
            scene_type: "atlas.terrain".to_owned(),
            enabled: true,
            start_seconds: 0.125,
            end_seconds: FIXTURE_DURATION,
            opacity: 0.0,
            blend_mode: BlendMode::Screen,
            mappings: Vec::new(),
        },
    ];

    // -- parameter cues. Sorted by (start, id) and non-overlapping per
    // (scene, parameter), which is what validation demands. The two cues on one
    // parameter abut exactly, which is the boundary the overlap rule has to admit.
    #[rustfmt::skip]
    let cue_rows: [CueRow; 5] = [
        (5, 1001, "settings.spectrum.amplitude",  0.0,  10.0,   0.0,   1.0,  Interpolation::Step),
        (6, 1001, "settings.spectrum.amplitude", 10.0,  20.0,   1.0,   0.25, Interpolation::Linear),
        (7, 2002, "settings.loom.weight",        20.0,  40.5,  -3.5,   7.25, Interpolation::Smoothstep),
        (8, 3003, "settings.atlas.wireframe",    40.5,  90.0,   0.1,   0.9,  Interpolation::EaseIn),
        (9, 1001, "settings.spectrum.hue_swing", 90.0,  FIXTURE_DURATION,
                                                        359.5,  0.5,  Interpolation::EaseOut),
    ];
    let cues = cue_rows
        .into_iter()
        .map(
            |(cue_id, target_scene_id, parameter, start, end, from, to, interpolation)| {
                ParameterCue {
                    cue_id,
                    target_scene_id,
                    parameter: parameter.to_owned(),
                    start_seconds: start,
                    end_seconds: end,
                    from_value: from,
                    to_value: to,
                    interpolation,
                }
            },
        )
        .collect();

    // -- analysis lanes. One of each kind, because a repeated kind is invalid, so
    // three is the whole space. The first leaves the three optional provenance
    // strings empty, which is the case a fixture that populates everything would
    // otherwise never cover.
    let analysis_lanes = vec![
        AnalysisLaneReference {
            kind: AnalysisLaneKind::MeasuredSignal,
            path: "analysis/signal.jsonl".to_owned(),
            sha256: LANE0_SHA.to_owned(),
            audio_sha256: AUDIO_SHA.to_owned(),
            provenance: provenance(
                "builtin.analyzer",
                "1.2.3",
                "musializer.analysis/v1",
                "",
                "",
                "",
            ),
        },
        AnalysisLaneReference {
            kind: AnalysisLaneKind::LyricTiming,
            path: "analysis/lyrics.jsonl".to_owned(),
            sha256: LANE1_SHA.to_owned(),
            audio_sha256: AUDIO_SHA.to_owned(),
            provenance: provenance(
                "whisper.adapter",
                "0.9.1-β",
                "musializer.lyrics/v1",
                "large-v3",
                "OpenAI Whisper",
                "p-7",
            ),
        },
        AnalysisLaneReference {
            kind: AnalysisLaneKind::SemanticScore,
            path: "analysis/sémantique.jsonl".to_owned(),
            sha256: LANE2_SHA.to_owned(),
            audio_sha256: AUDIO_SHA.to_owned(),
            provenance: provenance(
                "semantic.adapter:v2",
                "2.0.0",
                "musializer.semantic/v1",
                "模型-α",
                "Anbieter Ω",
                "prompt-Δ-3",
            ),
        },
    ];

    // -- lyrics. The document is created at the audio's duration, which is what
    // the parser derives, and the three ids are explicit so `next_id` lands on 4.
    let mut lyrics = LyricsDocument::new(FIXTURE_DURATION).expect("a positive duration");
    #[rustfmt::skip]
    let lyric_rows: [(u64, f64, f64, &str); 3] = [
        (1, 1.5,  4.25, "Erste Zeile — ✓"),
        // A tab is the one control character lyric text may contain.
        (2, 4.25, 8.0,  "Zweite\tZeile"),
        (3, 8.0,  12.5, "三行目 ✦"),
    ];
    for (id, start_seconds, end_seconds, text) in lyric_rows {
        lyrics
            .insert(LyricCue {
                id,
                start_seconds,
                end_seconds,
                text: text.to_owned(),
                // LX1 added `origin`, which is written only when it is not the
                // default. This fixture keeps the default deliberately: the C
                // oracle has no such field, so the only way the harness can
                // stay a comparison is for the bytes it compares to be the
                // ones both sides can produce. The field's own round trip is
                // pinned by unit tests in `project::io` instead.
                origin: Default::default(),
            })
            .expect("a valid lyric cue");
    }

    // -- scene switches. Enabled, and tiling the whole track, because a nonempty
    // plan that does not cover [0, duration] is invalid. The last cue's settings
    // array is empty, which is how an early v1 project spells it.
    let scene_switches = SceneSwitchSuggestions {
        enabled: true,
        cues: vec![
            SceneSwitchSuggestion {
                id: 11,
                start_seconds: 0.0,
                end_seconds: FIXTURE_SWITCH_STEP,
                scene_name: "spectrum".to_owned(),
                strength: 0.125,
                settings: vec![0.1, 0.2, 0.3, 0.4],
            },
            SceneSwitchSuggestion {
                id: 22,
                start_seconds: FIXTURE_SWITCH_STEP,
                end_seconds: 2.0 * FIXTURE_SWITCH_STEP,
                scene_name: "loom".to_owned(),
                strength: 0.664_062_5,
                // The last two are the exact endpoints of the f32 range: the
                // parser rejects anything beyond them, so equality is the case
                // worth checking.
                settings: vec![
                    1.0 / 3.0,
                    0.7,
                    -12.5,
                    0.0,
                    65504.0,
                    -0.0,
                    f32::MAX,
                    -f32::MAX,
                ],
            },
            SceneSwitchSuggestion {
                id: 33,
                start_seconds: 2.0 * FIXTURE_SWITCH_STEP,
                end_seconds: FIXTURE_DURATION,
                scene_name: "atlas.terrain".to_owned(),
                strength: 1.0,
                settings: Vec::new(),
            },
        ],
    };

    // -- scene presets. The first fills all twelve controls.
    #[rustfmt::skip]
    let loud: [f32; MAX_CONTROLS] = [
        0.1, 0.2, 0.3, 0.4, 0.5, 0.6,
        0.7, 0.8, 0.9, 1.0, -1.0, 1.0 / 3.0,
    ];
    let scene_presets = vec![
        ScenePreset {
            id: 101,
            scene_name: "spectrum".to_owned(),
            name: "Loud & Proud ✦".to_owned(),
            settings: loud.to_vec(),
        },
        ScenePreset {
            id: 102,
            scene_name: "loom".to_owned(),
            name: "Sanft — Ø".to_owned(),
            settings: vec![1.0 / 3.0, 0.7, -12.5],
        },
        ScenePreset {
            id: 103,
            scene_name: "atlas.terrain".to_owned(),
            name: "Ω".to_owned(),
            settings: vec![0.0],
        },
    ];

    // -- semantic events. Every one must be `Semantic` with four values in the
    // documented ranges, so this lane's variety is in its numbers.
    #[rustfmt::skip]
    let semantic_rows: [(f64, u64, [f32; 4]); 3] = [
        (0.0,              1, [0.0, 1.0, -1.0, 0.5]),
        (12.5,             2, [0.1, 0.2, -0.3, 0.4]),
        (FIXTURE_DURATION, 3, [1.0, 0.0,  1.0, 1.0 / 3.0]),
    ];
    let mut semantic_events = EventTimeline::new();
    for (timestamp, id, values) in semantic_rows {
        semantic_events
            .record(event(timestamp, id, EventType::Semantic, &values))
            .expect("a valid semantic event");
    }

    // -- manual events. All four types and all four value counts.
    let mut manual_events = EventTimeline::new();
    for record in [
        event(1.0, 10, EventType::Lyric, &[1.0]),
        event(2.0, 11, EventType::Semantic, &[0.1, 0.2]),
        event(3.0, 12, EventType::Cue, &[0.3, 0.4, 0.5]),
        event(4.0, 13, EventType::Custom, &[1.0 / 3.0, -2.5, 1.0e10, -0.0]),
    ] {
        manual_events.record(record).expect("a valid manual event");
    }

    let project = Project {
        metadata,
        audio,
        ascii_image,
        caption_style,
        output,
        deterministic_seed: 0xDEAD_BEEF_CAFE_F00D,
        scenes,
        cues,
        analysis_lanes,
        lyrics,
        scene_switches,
        scene_presets,
        semantic_events,
        manual_events,
        ..Project::default()
    };

    if let Err(validation) = project.validate() {
        eprintln!(
            "fixture is invalid: {} (index {}, subindex {})",
            validation.error, validation.index, validation.subindex
        );
        std::process::exit(2);
    }
    project
}

// ---------------------------------------------------------------------------
// File edges
// ---------------------------------------------------------------------------

fn serialize_to(project: &Project, path: &str) {
    match io::serialize(project) {
        Ok(text) => {
            if let Err(error) = std::fs::write(path, text) {
                eprintln!("cannot create {path}: {error}");
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("Rust serialize failed: {error}");
            std::process::exit(3);
        }
    }
}

fn deserialize_from(path: &str) -> Project {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot open {path}: {error}");
            std::process::exit(2);
        }
    };
    match io::deserialize(&bytes) {
        Ok(project) => project,
        Err(error) => {
            // The error type *is* the finding when one codec refuses the other's
            // output, so it is named on stderr and the exit status is distinct.
            eprintln!("Rust deserialize of {path} failed: {error:?} ({error})");
            std::process::exit(4);
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().collect();
    let usage = || {
        eprintln!(
            "usage: project_io_dump build <out.musi>\n\
             \x20      project_io_dump read <in.musi>\n\
             \x20      project_io_dump rewrite <in.musi> <out.musi>"
        );
    };
    if arguments.len() < 3 {
        usage();
        return ExitCode::from(1);
    }
    match arguments[1].as_str() {
        "build" => {
            let fixture = build_fixture();
            serialize_to(&fixture, &arguments[2]);
            dump_project(&fixture);
        }
        "read" => {
            let project = deserialize_from(&arguments[2]);
            dump_project(&project);
        }
        "rewrite" => {
            if arguments.len() < 4 {
                eprintln!("rewrite needs an output path");
                return ExitCode::from(1);
            }
            let project = deserialize_from(&arguments[2]);
            dump_project(&project);
            serialize_to(&project, &arguments[3]);
        }
        other => {
            eprintln!("unknown mode: {other}");
            usage();
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}
