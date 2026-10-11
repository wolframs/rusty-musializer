//! Treatments: a whole music video described as one file a script or an agent
//! can write (`*.treatment.json`, `--treatment`).
//!
//! **Post-legacy product extension (operator request, 2026-10-11).** The frozen C
//! has nothing like this, and until now neither did this application: a scene
//! plan, its per-section tuning, the routes, the lyric timing and the caption
//! style could all be *rendered* from a command line, but every one of them could
//! only be *authored* by dragging something in the window. `--scene` picks one
//! scene for a whole track, `--route` adds one mapping, and the plan and the
//! lyrics had no flag at all — so a caller with no pointer could export a
//! visualizer and could not direct a video.
//!
//! A treatment is the director's document: which scene plays when, how each
//! section is tuned, what the audio drives, which line is on screen from when to
//! when, and how the type is set. The application compiles it into exactly the
//! state the window's own editors produce, so `--save-project` after
//! `--treatment` writes an ordinary `.musi` that opens in the interface.
//!
//! # Why not write the `.musi` directly
//!
//! The `.musi` is a storage format and it is shaped like one. A segment's tuning
//! is a bare array whose meaning comes from position (`[0.4, 1.0, 0.2, …]`), the
//! plan must already be contiguous to the millisecond, the audio is addressed by
//! digest, and a caption style is all-or-nothing. Every one of those is right for
//! a file the application round-trips and wrong for a file somebody *writes*: a
//! transposed pair of values in a snapshot is a valid project that looks subtly
//! different, and nothing can say which control was meant. A treatment names
//! every setting by key, states only where a section *starts*, and omits whatever
//! it does not care about.
//!
//! # Boundaries
//!
//! Pure, like the rest of this crate: [`Treatment::parse`] takes bytes and
//! [`Treatment::compile`] takes a duration. The application owns the file read,
//! the audio path resolution and the digest. Parsing is strict in the `.musi`
//! codec's sense — an unknown field, scene, setting or source is refused by name
//! rather than ignored, because a typo that silently does nothing is the one
//! failure a caller with no eyes on the window cannot notice.
//!
//! Out-of-range values are **refused, not clamped**, with the bounds in the
//! message. `SceneSettings::set` already refuses, and a file that asked for
//! `glow: 9` and quietly got `1.0` has stopped describing the video it makes.
//! In-range values are snapped to the control's own precision, which is what the
//! Tune panel does (PX6): the number in the file is the number the readout shows.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::project::event_timeline::EventTimeline;
use crate::project::lyrics::{self, CueOrigin, LyricCue, LyricsDocument};
use crate::project::model::{
    caption, caption_fx, CaptionAnchor, CaptionBox, CaptionFace, CaptionStyle, EffectDrive,
};
use crate::project::scene_switch::{self, SceneSwitchCue, SceneSwitchTimeline};
use crate::project::sha256;
use crate::scene::events::{EventRecord, EventType, VALUE_CAPACITY};
use crate::scene::routes::{
    self, AnalysisSource, Interpolation, ParameterMapping, RouteError, RouteTable,
};
use crate::scene::settings::{self, SceneSettings, SettingDescriptor, SettingKind};
use crate::scene::{SceneId, SCENE_COUNT};
use crate::timing::render_export::Quality;
use crate::ui::lyric_lane_edit::parse_cue_timestamp;
use crate::ui::tune_explore;

/// The treatment file's `schema` value. Versioned so a later shape can add
/// fields without this parser guessing at them.
pub const TREATMENT_SCHEMA: &str = "musializer.treatment/v1";

/// The `schema` value of the `--vocabulary` document.
pub const VOCABULARY_SCHEMA: &str = "musializer.vocabulary/v1";

/// The largest input [`Treatment::parse`] will look at: 1 MiB, far above any
/// real treatment (a four-minute song with every line timed is ~8 KiB).
pub const MAX_INPUT: usize = 1024 * 1024;

/// How long a lyric line stays up when it does not say (`until` omitted) and no
/// later line takes over first.
///
/// Six seconds because a caption left standing through an instrumental break
/// reads as a stuck frame, and a sung line that genuinely lasts longer is the
/// case worth writing an `until` for. The next line's start always wins when it
/// comes sooner, so a verse written as bare `at`s is contiguous.
pub const DEFAULT_LYRIC_HOLD_SECONDS: f64 = 6.0;

/// The shortest lyric line a treatment may ask for. Below a frame at 24 fps a
/// cue can fall between two frames and never be drawn at all.
pub const MIN_LYRIC_SECONDS: f64 = 0.05;

/// Byte bounds on free text, applied before anything else looks at it.
pub const MAX_TITLE_BYTES: usize = crate::project::model::capacity::NAME;
pub const MAX_PATH_BYTES: usize = 4096;

/// Why a treatment was refused. Every [`TreatmentError::Invalid`] names the
/// place — `segments[3] (1:13.000)`, `scenes.atlas.settings.camera` — because
/// "segment 3 names scene `cadance`" is a one-line fix and "invalid file" is a
/// search.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TreatmentError {
    #[error("treatment file exceeds {MAX_INPUT} bytes")]
    TooLarge,
    #[error("not a treatment file: {0}")]
    Json(String),
    #[error("schema is {found:?}, this build reads {TREATMENT_SCHEMA:?}")]
    Schema { found: String },
    #[error("{place}: {reason}")]
    Invalid { place: String, reason: String },
}

fn invalid(place: impl Into<String>, reason: impl Into<String>) -> TreatmentError {
    TreatmentError::Invalid {
        place: place.into(),
        reason: reason.into(),
    }
}

// -- wire shape ----------------------------------------------------------------
//
// The serde structs are the file format. Times, setting values and routes are
// taken as raw `Value`s and resolved by hand, because an untagged enum's only
// error is "data did not match any variant" — which names neither the place nor
// what would have been accepted.

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreatmentWire {
    schema: String,
    #[serde(default)]
    title: Option<String>,
    /// The author's own remarks. Parsed so a strict file can carry them, and
    /// otherwise ignored — JSON has no comments and a treatment is exactly the
    /// kind of document that wants one.
    #[serde(default)]
    #[allow(dead_code)]
    notes: Option<Value>,
    #[serde(default)]
    audio: Option<AudioWire>,
    #[serde(default)]
    output: Option<OutputWire>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    caption_style: Option<CaptionStyleWire>,
    #[serde(default)]
    scenes: BTreeMap<String, SceneWire>,
    segments: Vec<SegmentWire>,
    #[serde(default)]
    lyrics: Option<Vec<LyricWire>>,
    #[serde(default)]
    events: Option<Vec<EventWire>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioWire {
    path: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputWire {
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    fps: Option<u32>,
    #[serde(default)]
    quality: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneWire {
    #[serde(default)]
    settings: BTreeMap<String, Value>,
    #[serde(default)]
    routes: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SegmentWire {
    at: Value,
    scene: String,
    #[serde(default)]
    settings: BTreeMap<String, Value>,
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LyricWire {
    at: Value,
    #[serde(default)]
    until: Option<Value>,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventWire {
    at: Value,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    value: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedRouteWire {
    setting: String,
    source: String,
    #[serde(default)]
    band: u16,
    input: [f64; 2],
    output: [f64; 2],
    #[serde(default)]
    curve: Option<String>,
    #[serde(default)]
    clamp: Option<bool>,
}

/// The `.musi` caption member names, each optional here.
///
/// A `.musi` requires all nine once the block is present, so a half-specified
/// style cannot silently mix intent with defaults *in a file the application
/// round-trips*. A treatment is the other kind of file: it is authored, the
/// defaults are documented by `--vocabulary`, and requiring nine members to
/// move one anchor would teach people to paste blocks they have not read.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptionStyleWire {
    #[serde(default)]
    face: Option<String>,
    #[serde(default)]
    r#box: Option<String>,
    #[serde(default)]
    anchor: Option<String>,
    #[serde(default)]
    size_scale: Option<f64>,
    #[serde(default)]
    margin_scale: Option<f64>,
    #[serde(default)]
    width_scale: Option<f64>,
    #[serde(default)]
    text_rgba: Option<String>,
    #[serde(default)]
    box_rgba: Option<String>,
    #[serde(default)]
    effects: Option<CaptionEffectsWire>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptionEffectsWire {
    #[serde(default)]
    glow_strength: Option<f64>,
    #[serde(default)]
    glow_radius: Option<f64>,
    #[serde(default)]
    glow_rgba: Option<String>,
    #[serde(default)]
    glow_pulse: Option<String>,
    #[serde(default)]
    glow_pulse_depth: Option<f64>,
    #[serde(default)]
    glow_hue_drive: Option<String>,
    #[serde(default)]
    glow_hue_range: Option<f64>,
    #[serde(default)]
    shadow_blur: Option<f64>,
    #[serde(default)]
    shadow_opacity: Option<f64>,
    #[serde(default)]
    plate_roundness: Option<f64>,
}

// -- domain --------------------------------------------------------------------

/// The audio a treatment is about, as its author wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreatmentAudio {
    /// May be relative; the application resolves it against the treatment
    /// file's own directory, the rule project assets and protocols use.
    pub path: String,
    /// Lowercase 64-hex digest, when the author pinned one. Optional — unlike a
    /// protocol's, where a wrong track mis-asks a human — because a treatment
    /// is usually written minutes before it is rendered, beside its audio. Pin
    /// it when the file is meant to outlive the session.
    pub sha256: Option<String>,
}

impl TreatmentAudio {
    /// Compare a computed digest against the pinned one, case-insensitively.
    /// `Ok` when nothing was pinned.
    ///
    /// # Errors
    ///
    /// Names both digests when they differ.
    pub fn verify_digest(&self, actual: &str) -> Result<(), TreatmentError> {
        match &self.sha256 {
            Some(expected) if !expected.eq_ignore_ascii_case(actual) => Err(invalid(
                "audio.sha256",
                format!("the treatment expects {expected}, the file is {actual}"),
            )),
            _ => Ok(()),
        }
    }
}

/// The treatment's `output` block. Each member is independent: a treatment may
/// fix the shape and leave the frame rate to the project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputRequest {
    pub resolution: Option<(u32, u32)>,
    pub fps: Option<u32>,
    pub quality: Option<Quality>,
}

#[derive(Clone, Debug, PartialEq)]
struct Segment {
    at: f64,
    scene: SceneId,
    /// The scene's whole table for this section: defaults, then the treatment's
    /// per-scene base, then this segment's own overrides.
    settings: SceneSettings,
}

#[derive(Clone, Debug, PartialEq)]
struct LyricLine {
    at: f64,
    until: Option<f64>,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EventMark {
    at: f64,
    kind: EventType,
    value: f32,
}

/// A parsed treatment: everything that can be checked without the audio.
#[derive(Clone, Debug)]
pub struct Treatment {
    pub title: Option<String>,
    pub audio: Option<TreatmentAudio>,
    pub output: Option<OutputRequest>,
    pub seed: Option<u64>,
    /// `None` leaves the track's own style alone.
    pub caption_style: Option<CaptionStyle>,
    scene_settings: SceneSettings,
    scene_routes: RouteTable,
    route_count: usize,
    segments: Vec<Segment>,
    /// `None` when the file has no `lyrics` key at all, which is different
    /// from an empty list: see [`CompiledTreatment::lyrics`].
    lyrics: Option<Vec<LyricLine>>,
    events: Option<Vec<EventMark>>,
}

/// What [`Treatment::compile`] hands the application: the same pieces a `.musi`
/// hydrates into, already validated against one track's duration.
#[derive(Clone, Debug)]
pub struct CompiledTreatment {
    /// The first section's scene, which is what the track's base scene becomes
    /// so a plan switched off in the interface still shows something chosen.
    pub base_scene: SceneId,
    /// The per-scene base tuning (defaults plus the treatment's `scenes` block).
    pub scene_settings: SceneSettings,
    pub scene_routes: RouteTable,
    /// Enabled, contiguous over the whole duration, one cue per section.
    pub scene_switches: SceneSwitchTimeline,
    /// `None` leaves the track's lyrics alone; `Some`, even of nothing,
    /// replaces them.
    ///
    /// The distinction is a workflow, not a nicety. Lines timed by ear in the
    /// lyric editor, or placed by an Assist run, are the slowest thing in a
    /// project to get right — and the natural second step is to direct the
    /// picture over them (`--project timed.musi --treatment look.json`). A
    /// treatment that says nothing about lyrics must not be read as "there are
    /// none". `"lyrics": []` is how a treatment says that.
    pub lyrics: Option<LyricsDocument>,
    /// The authored event lane (`events`), ids allocated from 1. `None` leaves
    /// the track's lane alone, on the same rule as [`Self::lyrics`].
    pub manual_events: Option<EventTimeline>,
    pub summary: Summary,
}

/// The counts a report line states. A treatment that applied and a treatment
/// that silently applied nothing produce the same exit status, so the line has
/// to carry numbers a caller can check against the file it wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub segments: usize,
    pub distinct_scenes: usize,
    pub routes: usize,
    /// `None` when the treatment does not speak for the lyrics at all.
    pub lyrics: Option<usize>,
    /// Lines that never reach the screen because a later line covers their whole
    /// span. Not an error — the lyric model allows overlap — but never intended.
    pub lyrics_fully_shadowed: usize,
    /// `None` when the treatment does not speak for the event lane.
    pub events: Option<usize>,
}

impl Treatment {
    /// Parse and validate a treatment file's bytes.
    ///
    /// # Errors
    ///
    /// Refuses — naming the first offending place — rather than repairs: junk
    /// JSON, an unknown schema, field, scene, setting key, source, curve or
    /// enum value; a setting outside its bounds or a toggle that is not 0/1;
    /// a malformed or duplicate route; a time that is not a number of seconds
    /// or `m:ss[.fff]`; sections that do not start at zero or are not in
    /// ascending order; a lyric line that is empty, too long, or out of order.
    pub fn parse(bytes: &[u8]) -> Result<Self, TreatmentError> {
        if bytes.len() > MAX_INPUT {
            return Err(TreatmentError::TooLarge);
        }
        let wire: TreatmentWire = serde_json::from_slice(bytes)
            .map_err(|error| TreatmentError::Json(error.to_string()))?;
        if wire.schema != TREATMENT_SCHEMA {
            return Err(TreatmentError::Schema { found: wire.schema });
        }

        if let Some(title) = &wire.title {
            if title.is_empty() || title.len() > MAX_TITLE_BYTES || title.contains('\0') {
                return Err(invalid(
                    "title",
                    format!("must be 1..={MAX_TITLE_BYTES} bytes"),
                ));
            }
        }

        let audio = wire.audio.map(parse_audio).transpose()?;
        let output = wire.output.map(parse_output).transpose()?;
        let caption_style = wire.caption_style.map(parse_caption_style).transpose()?;

        // The per-scene base: defaults, then whatever `scenes` says.
        let mut scene_settings = SceneSettings::new();
        let mut scene_routes = RouteTable::new();
        let mut route_count = 0usize;
        for (name, block) in &wire.scenes {
            let place = format!("scenes.{name}");
            let scene = resolve_scene(&place, name)?;
            apply_settings(
                &format!("{place}.settings"),
                scene,
                &block.settings,
                &mut scene_settings,
            )?;
            for (index, route) in block.routes.iter().enumerate() {
                let place = format!("{place}.routes[{index}]");
                let mapping = parse_route(&place, scene, route)?;
                scene_routes
                    .add(scene, mapping)
                    .map_err(|error| match error {
                        RouteError::Duplicate => invalid(
                            &place,
                            "a second route for a setting this scene already routes; \
                             one setting takes one source",
                        ),
                        other => invalid(&place, other.to_string()),
                    })?;
                route_count += 1;
            }
        }

        if wire.segments.is_empty() || wire.segments.len() > scene_switch::CAPACITY {
            return Err(invalid(
                "segments",
                format!(
                    "a treatment needs 1..={} sections, this one has {}",
                    scene_switch::CAPACITY,
                    wire.segments.len()
                ),
            ));
        }
        let mut segments: Vec<Segment> = Vec::with_capacity(wire.segments.len());
        for (index, segment) in wire.segments.iter().enumerate() {
            let at = parse_time(&format!("segments[{index}].at"), &segment.at)?;
            let place = format!("segments[{index}] ({})", format_time(at));
            match segments.last() {
                None if at != 0.0 => {
                    return Err(invalid(
                        &place,
                        "the first section must start at 0 — every instant of the \
                         track belongs to exactly one section",
                    ));
                }
                Some(previous) if at < previous.at + scene_switch::MIN_CUE_SECONDS => {
                    return Err(invalid(
                        &place,
                        format!(
                            "starts at or before the section ahead of it ({}); \
                             sections are listed in playing order",
                            format_time(previous.at)
                        ),
                    ));
                }
                _ => {}
            }
            let scene = resolve_scene(&place, &segment.scene)?;
            let mut table = scene_settings;
            apply_settings(
                &format!("{place}.settings"),
                scene,
                &segment.settings,
                &mut table,
            )?;
            segments.push(Segment {
                at,
                scene,
                settings: table,
            });
        }

        let wire_lyrics = wire.lyrics.as_deref().unwrap_or(&[]);
        if wire_lyrics.len() > lyrics::CUE_CAPACITY {
            return Err(invalid(
                "lyrics",
                format!(
                    "at most {} lines, this treatment has {}",
                    lyrics::CUE_CAPACITY,
                    wire_lyrics.len()
                ),
            ));
        }
        let mut lines: Vec<LyricLine> = Vec::with_capacity(wire_lyrics.len());
        for (index, line) in wire_lyrics.iter().enumerate() {
            let at = parse_time(&format!("lyrics[{index}].at"), &line.at)?;
            let place = format!("lyrics[{index}] ({})", format_time(at));
            let until = line
                .until
                .as_ref()
                .map(|value| parse_time(&format!("{place}.until"), value))
                .transpose()?;
            if let Some(until) = until {
                if until < at + MIN_LYRIC_SECONDS {
                    return Err(invalid(
                        &place,
                        format!(
                            "`until` ({}) must be at least {MIN_LYRIC_SECONDS} s after `at`",
                            format_time(until)
                        ),
                    ));
                }
            }
            if let Some(previous) = lines.last() {
                // Ascending rather than sorted for the author. A line listed
                // out of order is almost always a mistyped minute, and sorting
                // it into place would time the wrong words confidently.
                if at < previous.at {
                    return Err(invalid(
                        &place,
                        format!(
                            "starts before the line ahead of it ({}); lines are \
                             listed in singing order",
                            format_time(previous.at)
                        ),
                    ));
                }
            }
            lyrics::validate_text(&line.text).map_err(|error| {
                invalid(
                    &place,
                    format!(
                        "text is not usable ({error}); a line is 1..={} bytes",
                        lyrics::TEXT_MAX_BYTES
                    ),
                )
            })?;
            lines.push(LyricLine {
                at,
                until,
                text: line.text.clone(),
            });
        }

        let wire_events = wire.events.as_deref().unwrap_or(&[]);
        if wire_events.len() > crate::scene::events::TIMELINE_CAPACITY {
            return Err(invalid(
                "events",
                format!(
                    "at most {} events, this treatment has {}",
                    crate::scene::events::TIMELINE_CAPACITY,
                    wire_events.len()
                ),
            ));
        }
        let mut events = Vec::with_capacity(wire_events.len());
        for (index, event) in wire_events.iter().enumerate() {
            let at = parse_time(&format!("events[{index}].at"), &event.at)?;
            let place = format!("events[{index}] ({})", format_time(at));
            let kind = match event.r#type.as_deref() {
                None | Some("cue") => EventType::Cue,
                Some("lyric") => EventType::Lyric,
                Some("semantic") => EventType::Semantic,
                Some("custom") => EventType::Custom,
                Some(other) => {
                    return Err(invalid(
                        &place,
                        format!("unknown type {other:?}; expected cue, lyric, semantic or custom"),
                    ));
                }
            };
            let value = event.value.unwrap_or(1.0);
            if !value.is_finite() {
                return Err(invalid(&place, "value must be a finite number"));
            }
            events.push(EventMark {
                at,
                kind,
                value: value as f32,
            });
        }

        Ok(Self {
            title: wire.title,
            audio,
            output,
            seed: wire.seed,
            caption_style,
            scene_settings,
            scene_routes,
            route_count,
            segments,
            lyrics: wire.lyrics.is_some().then_some(lines),
            events: wire.events.is_some().then_some(events),
        })
    }

    /// How many sections the treatment has. Never zero.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Resolve the treatment against one track.
    ///
    /// The duration is the only thing a treatment cannot know about itself, and
    /// it closes every open end: each section runs to the next one's start and
    /// the last to the end of the track; a lyric line without an `until` runs to
    /// the next line, to [`DEFAULT_LYRIC_HOLD_SECONDS`], or to the end,
    /// whichever comes first.
    ///
    /// # Errors
    ///
    /// A section, line or event that starts at or past the end of the track, an
    /// explicit `until` past it, or (unreachable from [`Treatment::parse`], but
    /// reported rather than unwrapped) a plan the model itself refuses.
    pub fn compile(&self, duration_seconds: f64) -> Result<CompiledTreatment, TreatmentError> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(invalid("audio", "the track has no usable duration"));
        }
        let end = format_time(duration_seconds);

        let mut cues = Vec::with_capacity(self.segments.len());
        for (index, segment) in self.segments.iter().enumerate() {
            let place = format!("segments[{index}] ({})", format_time(segment.at));
            // Each section is judged on its own start. A next section past the
            // end is the next iteration's refusal, so the message names the
            // section that is actually misplaced rather than the one before it.
            if segment.at >= duration_seconds - scene_switch::MIN_CUE_SECONDS {
                return Err(invalid(
                    &place,
                    format!("starts at or after the end of the track ({end})"),
                ));
            }
            let until = self
                .segments
                .get(index + 1)
                .map_or(duration_seconds, |next| next.at.min(duration_seconds));
            let snapshot = segment
                .settings
                .capture(segment.scene)
                .ok_or_else(|| invalid(&place, "its tuning is outside the scene's bounds"))?;
            cues.push(SceneSwitchCue {
                id: index as u64 + 1,
                start_seconds: segment.at,
                end_seconds: until,
                scene_index: segment.scene.index() as u32,
                strength: 1.0,
                settings: snapshot,
            });
        }
        let mut scene_switches = SceneSwitchTimeline::new();
        scene_switches
            .replace(&cues, duration_seconds, SCENE_COUNT as u32)
            .map_err(|error| invalid("segments", format!("the plan was refused: {error}")))?;
        scene_switches.enabled = true;

        let mut document = LyricsDocument::new(duration_seconds)
            .map_err(|error| invalid("lyrics", error.to_string()))?;
        let lines = self.lyrics.as_deref().unwrap_or(&[]);
        for (index, line) in lines.iter().enumerate() {
            let place = format!("lyrics[{index}] ({})", format_time(line.at));
            if line.at > duration_seconds - MIN_LYRIC_SECONDS {
                return Err(invalid(
                    &place,
                    format!("starts at or after the end of the track ({end})"),
                ));
            }
            let until = match line.until {
                Some(until) if until > duration_seconds + 1.0e-6 => {
                    return Err(invalid(
                        &place,
                        format!(
                            "`until` ({}) is past the end of the track ({end})",
                            format_time(until)
                        ),
                    ));
                }
                Some(until) => until.min(duration_seconds),
                None => {
                    let hold = line.at + DEFAULT_LYRIC_HOLD_SECONDS;
                    // The next line that actually starts later. Two lines on
                    // one instant are legal (the later one in the file wins the
                    // frame), and a zero-length cue is not.
                    let next = lines[index + 1..]
                        .iter()
                        .map(|later| later.at)
                        .find(|&at| at >= line.at + MIN_LYRIC_SECONDS)
                        .unwrap_or(f64::INFINITY);
                    hold.min(next).min(duration_seconds)
                }
            };
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: line.at,
                    end_seconds: until,
                    text: line.text.clone(),
                    origin: CueOrigin::UserApplied,
                })
                .map_err(|error| invalid(&place, error.to_string()))?;
        }
        let lyrics_fully_shadowed = document
            .shadowed_cues()
            .iter()
            .filter(|shadow| shadow.fully)
            .count();

        let mut manual_events = EventTimeline::new();
        for (index, event) in self.events.as_deref().unwrap_or(&[]).iter().enumerate() {
            let place = format!("events[{index}] ({})", format_time(event.at));
            if event.at > duration_seconds {
                return Err(invalid(
                    &place,
                    format!("is past the end of the track ({end})"),
                ));
            }
            let mut values = [0.0f32; VALUE_CAPACITY];
            values[0] = event.value;
            manual_events
                .record(EventRecord {
                    timestamp_seconds: event.at,
                    id: index as u64 + 1,
                    event_type: event.kind as u32,
                    value_count: 1,
                    values,
                })
                .map_err(|error| invalid(&place, error.to_string()))?;
        }

        let mut seen = [false; SCENE_COUNT];
        for segment in &self.segments {
            seen[segment.scene.index()] = true;
        }
        Ok(CompiledTreatment {
            base_scene: self.segments[0].scene,
            scene_settings: self.scene_settings,
            scene_routes: self.scene_routes.clone(),
            scene_switches,
            summary: Summary {
                segments: self.segments.len(),
                distinct_scenes: seen.iter().filter(|used| **used).count(),
                routes: self.route_count,
                lyrics: self.lyrics.is_some().then_some(document.len()),
                lyrics_fully_shadowed,
                events: self.events.is_some().then_some(manual_events.len()),
            },
            lyrics: self.lyrics.is_some().then_some(document),
            manual_events: self.events.is_some().then_some(manual_events),
        })
    }
}

// -- field parsers ---------------------------------------------------------------

fn parse_audio(wire: AudioWire) -> Result<TreatmentAudio, TreatmentError> {
    if wire.path.is_empty() || wire.path.len() > MAX_PATH_BYTES || wire.path.contains('\0') {
        return Err(invalid(
            "audio.path",
            format!("must be 1..={MAX_PATH_BYTES} bytes"),
        ));
    }
    let sha256 = match wire.sha256 {
        None => None,
        Some(digest) => {
            let lowered = digest.to_ascii_lowercase();
            if !sha256::is_hex_digest(&lowered) {
                return Err(invalid(
                    "audio.sha256",
                    format!("{digest:?} is not a 64-hex sha256"),
                ));
            }
            Some(lowered)
        }
    };
    Ok(TreatmentAudio {
        path: wire.path,
        sha256,
    })
}

fn parse_output(wire: OutputWire) -> Result<OutputRequest, TreatmentError> {
    let resolution = match wire.resolution.as_deref() {
        None => None,
        Some(text) => Some(parse_resolution(text).ok_or_else(|| {
            invalid(
                "output.resolution",
                format!("{text:?} is not WIDTHxHEIGHT, e.g. \"1920x1080\""),
            )
        })?),
    };
    if wire.fps == Some(0) {
        return Err(invalid("output.fps", "must be a positive whole number"));
    }
    let quality = match wire.quality.as_deref() {
        None => None,
        Some("balanced") => Some(Quality::Balanced),
        Some("high") => Some(Quality::High),
        Some("master") => Some(Quality::Master),
        Some(other) => {
            return Err(invalid(
                "output.quality",
                format!("{other:?} is not balanced, high or master"),
            ));
        }
    };
    Ok(OutputRequest {
        resolution,
        fps: wire.fps,
        quality,
    })
}

/// `WIDTHxHEIGHT`, the `--resolution` grammar: one lowercase `x`, two positive
/// integers. Geometry limits are the render configuration's to enforce, where
/// the frame rate and quality are known too.
fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let width: u32 = width.parse().ok()?;
    let height: u32 = height.parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn resolve_scene(place: &str, name: &str) -> Result<SceneId, TreatmentError> {
    SceneId::from_stable_name(name).ok_or_else(|| {
        invalid(
            place,
            format!(
                "unknown scene {name:?}; the scenes are {}",
                SceneId::ALL.map(SceneId::stable_name).join(", ")
            ),
        )
    })
}

/// Resolve a setting name written inside a block that already names its scene.
///
/// `camera`, `atlas.camera` and `settings.atlas.camera` are one control. The
/// short form is the one worth typing — the scene is stated one line up — and
/// the long forms are accepted so a key copied out of a `.musi` or a `--route`
/// spec works unchanged.
fn resolve_setting(
    place: &str,
    scene: SceneId,
    name: &str,
) -> Result<(usize, &'static SettingDescriptor), TreatmentError> {
    let stable = scene.stable_name();
    let key = if name.starts_with("settings.") {
        name.to_owned()
    } else if name.contains('.') {
        format!("settings.{name}")
    } else {
        format!("settings.{stable}.{name}")
    };
    match settings::descriptor_by_key(settings::canonical_key(&key)) {
        Some((owner, index, descriptor)) if owner == scene => Ok((index, descriptor)),
        Some((owner, _, _)) => Err(invalid(
            place,
            format!(
                "{name:?} is a {} setting, and this block is for {stable}",
                owner.stable_name()
            ),
        )),
        None => Err(invalid(
            place,
            format!(
                "{stable} has no setting {name:?}; it has {}",
                settings::descriptors(scene)
                    .iter()
                    .map(|descriptor| short_name(descriptor))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// The last component of a descriptor key: `settings.atlas.camera` → `camera`.
fn short_name(descriptor: &SettingDescriptor) -> &'static str {
    descriptor
        .key
        .rsplit_once('.')
        .map_or(descriptor.key, |(_, name)| name)
}

fn apply_settings(
    place: &str,
    scene: SceneId,
    values: &BTreeMap<String, Value>,
    table: &mut SceneSettings,
) -> Result<(), TreatmentError> {
    for (name, value) in values {
        let place = format!("{place}.{name}");
        let (index, descriptor) = resolve_setting(&place, scene, name)?;
        let number = match value {
            Value::Bool(flag) if descriptor.kind == SettingKind::Toggle => {
                f64::from(u8::from(*flag))
            }
            Value::Bool(_) => {
                return Err(invalid(
                    &place,
                    format!(
                        "is a slider ({}..{}), not a switch; give it a number",
                        descriptor.minimum, descriptor.maximum
                    ),
                ));
            }
            Value::Number(number) => number
                .as_f64()
                .ok_or_else(|| invalid(&place, "must be a finite number"))?,
            _ => return Err(invalid(&place, "must be a number")),
        };
        if descriptor.kind == SettingKind::Toggle {
            if number != 0.0 && number != 1.0 {
                return Err(invalid(
                    &place,
                    format!("is a switch: true/false or 1/0, not {number}"),
                ));
            }
        } else if !number.is_finite()
            || number < f64::from(descriptor.minimum)
            || number > f64::from(descriptor.maximum)
        {
            return Err(invalid(
                &place,
                format!(
                    "{number} is outside {}..{} (default {})",
                    descriptor.minimum, descriptor.maximum, descriptor.default_value
                ),
            ));
        }
        // In range, so `conform` only snaps to the readout's grid (PX6).
        let conformed = tune_explore::conform(descriptor, number as f32);
        if !table.set(scene, index, conformed) {
            return Err(invalid(&place, "was refused by the settings table"));
        }
    }
    Ok(())
}

fn parse_route(
    place: &str,
    scene: SceneId,
    value: &Value,
) -> Result<ParameterMapping, TreatmentError> {
    match value {
        // The `--route` grammar, verbatim, so a spec proven on the command line
        // can be pasted in.
        Value::String(spec) => {
            let (owner, mapping) = routes::parse_route_spec(spec).ok_or_else(|| {
                invalid(
                    place,
                    format!(
                        "{spec:?} is not a route; expected \
                         setting:source:band:in_min:in_max:out_min:out_max[:curve][:noclamp]"
                    ),
                )
            })?;
            if owner != scene {
                return Err(invalid(
                    place,
                    format!(
                        "routes a {} setting inside the {} block",
                        owner.stable_name(),
                        scene.stable_name()
                    ),
                ));
            }
            Ok(mapping)
        }
        Value::Object(_) => {
            let wire: NamedRouteWire = serde_json::from_value(value.clone())
                .map_err(|error| invalid(place, error.to_string()))?;
            let (_, descriptor) = resolve_setting(place, scene, &wire.setting)?;
            let source = AnalysisSource::from_canonical_name(&wire.source).ok_or_else(|| {
                invalid(
                    place,
                    format!(
                        "unknown source {:?}; the sources are {}",
                        wire.source,
                        AnalysisSource::ALL
                            .map(AnalysisSource::canonical_name)
                            .join(", ")
                    ),
                )
            })?;
            let interpolation = match wire.curve.as_deref() {
                None => Interpolation::Linear,
                Some(name) => Interpolation::from_canonical_name(name).ok_or_else(|| {
                    invalid(
                        place,
                        format!(
                            "unknown curve {name:?}; the curves are {}",
                            Interpolation::ALL
                                .map(Interpolation::canonical_name)
                                .join(", ")
                        ),
                    )
                })?,
            };
            let mapping = ParameterMapping {
                parameter: descriptor.key.to_owned(),
                source,
                band_index: wire.band,
                input_min: wire.input[0],
                input_max: wire.input[1],
                output_min: wire.output[0],
                output_max: wire.output[1],
                interpolation,
                clamp: wire.clamp.unwrap_or(true),
            };
            if !mapping.is_valid_for(scene) {
                return Err(invalid(
                    place,
                    "is not a usable route: `input` must rise, `output` must name two \
                     different values (a constant belongs in `settings`), and `band` is \
                     only for source \"band\"",
                ));
            }
            Ok(mapping)
        }
        _ => Err(invalid(
            place,
            "a route is a spec string or an object with setting, source, input and output",
        )),
    }
}

/// A time: a number of seconds, or a string in the lyric editor's own grammar
/// (`1:23.4`, `01:23.456`, `83.4`).
///
/// Both, because the two halves of the job think in different units — a
/// listening report says "the drop is at 1:13" and an analysis sidecar says
/// `73.0` — and converting by hand between them is where a minute goes missing.
fn parse_time(place: &str, value: &Value) -> Result<f64, TreatmentError> {
    let seconds = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_cue_timestamp(text),
        _ => None,
    };
    match seconds {
        Some(seconds) if seconds.is_finite() && seconds >= 0.0 => Ok(seconds),
        _ => Err(invalid(
            place,
            format!("{value} is not a time; write seconds (73.5) or \"m:ss.fff\" (\"1:13.5\")"),
        )),
    }
}

/// `m:ss.fff`, for messages and report lines. The grammar [`parse_time`] reads.
#[must_use]
pub fn format_time(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{}:{:02}.{:03}",
        total_ms / 60_000,
        (total_ms / 1000) % 60,
        total_ms % 1000
    )
}

/// A caption colour: `#rrggbb`, `#rrggbbaa` or the `.musi`'s bare `rrggbbaa`,
/// in either case. The `.musi` keeps one spelling per colour because it is
/// compared and diffed; a treatment is typed, usually from a colour picker.
fn parse_rgba(place: &str, text: &str) -> Result<u32, TreatmentError> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    let parsed = match digits.len() {
        6 => u32::from_str_radix(digits, 16)
            .ok()
            .map(|rgb| (rgb << 8) | 0xFF),
        8 => u32::from_str_radix(digits, 16).ok(),
        _ => None,
    };
    parsed
        .filter(|_| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            invalid(
                place,
                format!("{text:?} is not a colour; write \"#rrggbb\" or \"#rrggbbaa\""),
            )
        })
}

fn bounded(place: &str, value: f64, minimum: f64, maximum: f64) -> Result<f64, TreatmentError> {
    if value.is_finite() && value >= minimum && value <= maximum {
        Ok(value)
    } else {
        Err(invalid(
            place,
            format!("{value} is outside {minimum}..{maximum}"),
        ))
    }
}

fn parse_drive(place: &str, name: &str) -> Result<EffectDrive, TreatmentError> {
    EffectDrive::from_canonical_name(name).ok_or_else(|| {
        invalid(
            place,
            format!(
                "unknown drive {name:?}; the drives are {}",
                names(EffectDrive::ALL, EffectDrive::canonical_name)
            ),
        )
    })
}

fn names<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|value| name(*value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_caption_style(wire: CaptionStyleWire) -> Result<CaptionStyle, TreatmentError> {
    let mut style = CaptionStyle::default();
    if let Some(name) = wire.face.as_deref() {
        style.face = match CaptionFace::from_canonical_name(name) {
            // An imported face is a bundled asset with a digest and a licence
            // record, and a treatment has nowhere to carry either. Said here
            // rather than accepted and rendered in the wrong face.
            Some(CaptionFace::Imported) => {
                return Err(invalid(
                    "caption_style.face",
                    "a treatment cannot name an imported face yet; use alegreya or \
                     space_grotesk, or import the face in the Lyrics panel after \
                     --save-project",
                ));
            }
            Some(face) => face,
            None => {
                return Err(invalid(
                    "caption_style.face",
                    format!("unknown face {name:?}; the faces are alegreya, space_grotesk"),
                ));
            }
        };
    }
    if let Some(name) = wire.r#box.as_deref() {
        style.box_style = CaptionBox::from_canonical_name(name).ok_or_else(|| {
            invalid(
                "caption_style.box",
                format!(
                    "unknown backing {name:?}; the backings are {}",
                    names(CaptionBox::ALL, CaptionBox::canonical_name)
                ),
            )
        })?;
    }
    if let Some(name) = wire.anchor.as_deref() {
        style.anchor = CaptionAnchor::from_canonical_name(name).ok_or_else(|| {
            invalid(
                "caption_style.anchor",
                format!(
                    "unknown anchor {name:?}; the anchors are {}",
                    names(CaptionAnchor::ALL, CaptionAnchor::canonical_name)
                ),
            )
        })?;
    }
    if let Some(value) = wire.size_scale {
        style.size_scale = bounded(
            "caption_style.size_scale",
            value,
            caption::SIZE_MINIMUM,
            caption::SIZE_MAXIMUM,
        )?;
    }
    if let Some(value) = wire.margin_scale {
        style.margin_scale = bounded(
            "caption_style.margin_scale",
            value,
            caption::MARGIN_MINIMUM,
            caption::MARGIN_MAXIMUM,
        )?;
    }
    if let Some(value) = wire.width_scale {
        style.width_scale = bounded(
            "caption_style.width_scale",
            value,
            caption::WIDTH_MINIMUM,
            caption::WIDTH_MAXIMUM,
        )?;
    }
    if let Some(text) = wire.text_rgba.as_deref() {
        style.text_rgba = parse_rgba("caption_style.text_rgba", text)?;
    }
    if let Some(text) = wire.box_rgba.as_deref() {
        style.box_rgba = parse_rgba("caption_style.box_rgba", text)?;
    }
    if let Some(effects) = wire.effects {
        let fx = &mut style.effects;
        if let Some(value) = effects.glow_strength {
            fx.glow_strength = bounded(
                "caption_style.effects.glow_strength",
                value,
                caption_fx::GLOW_STRENGTH_MINIMUM,
                caption_fx::GLOW_STRENGTH_MAXIMUM,
            )?;
        }
        if let Some(value) = effects.glow_radius {
            fx.glow_radius = bounded(
                "caption_style.effects.glow_radius",
                value,
                caption_fx::GLOW_RADIUS_MINIMUM,
                caption_fx::GLOW_RADIUS_MAXIMUM,
            )?;
        }
        if let Some(text) = effects.glow_rgba.as_deref() {
            fx.glow_rgba = parse_rgba("caption_style.effects.glow_rgba", text)?;
        }
        if let Some(name) = effects.glow_pulse.as_deref() {
            fx.glow_pulse = parse_drive("caption_style.effects.glow_pulse", name)?;
        }
        if let Some(value) = effects.glow_pulse_depth {
            fx.glow_pulse_depth = bounded(
                "caption_style.effects.glow_pulse_depth",
                value,
                caption_fx::PULSE_DEPTH_MINIMUM,
                caption_fx::PULSE_DEPTH_MAXIMUM,
            )?;
        }
        if let Some(name) = effects.glow_hue_drive.as_deref() {
            fx.glow_hue_drive = parse_drive("caption_style.effects.glow_hue_drive", name)?;
        }
        if let Some(value) = effects.glow_hue_range {
            fx.glow_hue_range = bounded(
                "caption_style.effects.glow_hue_range",
                value,
                caption_fx::HUE_RANGE_MINIMUM,
                caption_fx::HUE_RANGE_MAXIMUM,
            )?;
        }
        if let Some(value) = effects.shadow_blur {
            fx.shadow_blur = bounded(
                "caption_style.effects.shadow_blur",
                value,
                caption_fx::SHADOW_BLUR_MINIMUM,
                caption_fx::SHADOW_BLUR_MAXIMUM,
            )?;
        }
        if let Some(value) = effects.shadow_opacity {
            fx.shadow_opacity = bounded(
                "caption_style.effects.shadow_opacity",
                value,
                caption_fx::SHADOW_OPACITY_MINIMUM,
                caption_fx::SHADOW_OPACITY_MAXIMUM,
            )?;
        }
        if let Some(value) = effects.plate_roundness {
            fx.plate_roundness = bounded(
                "caption_style.effects.plate_roundness",
                value,
                caption_fx::PLATE_ROUNDNESS_MINIMUM,
                caption_fx::PLATE_ROUNDNESS_MAXIMUM,
            )?;
        }
    }
    Ok(style)
}

// -- vocabulary ------------------------------------------------------------------

/// What a scene looks like and what it answers to, in one line each.
///
/// The descriptor tables can say a control is called `weight` and runs 0.4 to
/// 2.2; they cannot say what the scene *is*, and a caller choosing between
/// thirteen of them from a command line has never seen any. Exhaustive on
/// purpose: adding a scene without describing it here is a compile error.
#[must_use]
pub fn scene_look(scene: SceneId) -> &'static str {
    match scene {
        SceneId::Spectrum => {
            "Log-frequency bars with glowing caps standing on a reflective floor. The \
             plainest reading of the mix: silent bands are simply absent."
        }
        SceneId::PulseField => {
            "Concentric rounded rings around a bright core, swelling and wobbling with \
             level. A single centred emblem; reads well small."
        }
        SceneId::OrbitalLattice => {
            "A tilted lattice of glowing nodes on orbiting rings, linked by fine arcs. \
             Sparse and dark when quiet, a dense turning halo when loud."
        }
        SceneId::AsciiField => {
            "A wall of ASCII glyphs: a scrolling spectrogram waterfall, or an imported \
             image (--ascii-image) rendered as characters that the audio disturbs."
        }
        SceneId::SongAtlas => {
            "Tideline: the whole track as a spiral relief with travelling folds and a \
             warm light moving along it. Shows the song's shape, not just this moment."
        }
        SceneId::SpectralTerrarium => {
            "A small ecosystem under glass: plants that grow with the spectrum, a \
             drifting flock, bioluminescent particles. Slow, organic, intimate."
        }
        SceneId::Constellation => {
            "Drifting star nodes joined by a faint web. The one scene that flares on \
             authored `events`, coloured by event type."
        }
        SceneId::Cadence => {
            "Kinetic typography: the current lyric line is the picture, word by word, \
             over soft strata. With no line on screen only the strata remain. The \
             caption overlay is not drawn on top of it."
        }
        SceneId::Loom => {
            "A cloth woven left to right across the whole track, one column per moment; \
             by the end the tapestry is a record of the song's arc."
        }
        SceneId::Pentagram => {
            "Nested five-point orbits traced as fine rotating polygons with bright \
             stations. Geometric, cool, hypnotic."
        }
        SceneId::PhosphorDream => {
            "A generative ASCII CRT: ten procedural fields (plasma, tunnel, rain, \
             vortex…) cycling on a dwell clock under bloom and scanlines. `titles` \
             surfaces the current lyric line through the glyph field."
        }
        SceneId::Clawd => {
            "A twelve-petalled flower character on a cream ground: petals plump with \
             their bands, the face reacts, ASCII cats bounce, and a hue-wheel petal \
             show ignites on the track's biggest moments. `terminal` types the lyric."
        }
        SceneId::OpenAiKnot => {
            "Six glossy interlocked ribbons forming one knot, lit by the audio. A bold \
             single object on a deep ground."
        }
    }
}

/// An `f32` as the decimal a person would write, so `0.1` prints as `0.1` and
/// not as the `f64` nearest the `f32` nearest it.
fn tidy(value: f32) -> Value {
    value
        .to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn range(minimum: f64, maximum: f64, default: f64) -> Value {
    json!({ "minimum": minimum, "maximum": maximum, "default": default })
}

/// Every name a treatment or a `--route` may use, as one JSON document.
///
/// Generated from the descriptor tables and the enums themselves rather than
/// written down beside them, for the reason `AGENTS.md` gives for refusing to
/// list "currently supported X" anywhere: the list goes stale within weeks and
/// the code cannot.
#[must_use]
pub fn vocabulary() -> Value {
    let scenes: Vec<Value> = SceneId::ALL
        .iter()
        .map(|&scene| {
            let controls: Vec<Value> = settings::descriptors(scene)
                .iter()
                .map(|descriptor| {
                    json!({
                        "name": short_name(descriptor),
                        "key": descriptor.key,
                        "label": descriptor.label,
                        "kind": match descriptor.kind {
                            SettingKind::Slider => "slider",
                            SettingKind::Toggle => "toggle",
                        },
                        "minimum": tidy(descriptor.minimum),
                        "maximum": tidy(descriptor.maximum),
                        "default": tidy(descriptor.default_value),
                        "precision": descriptor.precision,
                    })
                })
                .collect();
            json!({
                "name": scene.stable_name(),
                "display_name": scene.display_name(),
                "look": scene_look(scene),
                "settings": controls,
            })
        })
        .collect();

    json!({
        "schema": VOCABULARY_SCHEMA,
        "treatment_schema": TREATMENT_SCHEMA,
        "time": "seconds as a number (73.5), or a string \"m:ss.fff\" (\"1:13.5\")",
        "scenes": scenes,
        "routes": {
            "sources": AnalysisSource::ALL.map(AnalysisSource::canonical_name),
            "curves": Interpolation::ALL.map(Interpolation::canonical_name),
            "bands": crate::audio::analyzer::MAX_BANDS,
            "per_scene": routes::ROUTES_PER_SCENE,
            "spec": "setting:source:band:in_min:in_max:out_min:out_max[:curve][:noclamp]",
            "object": {
                "setting": "a setting name of the block's scene",
                "source": "one of sources",
                "band": "0 unless source is \"band\"",
                "input": "[low, high] of the source, usually [0, 1]",
                "output": "[value at low, value at high], two different values",
                "curve": "optional, default linear",
                "clamp": "optional, default true",
            },
        },
        "caption_style": {
            "face": ["alegreya", "space_grotesk"],
            "box": CaptionBox::ALL.iter().map(|value| value.canonical_name()).collect::<Vec<_>>(),
            "anchor": CaptionAnchor::ALL.iter().map(|value| value.canonical_name()).collect::<Vec<_>>(),
            "size_scale": range(caption::SIZE_MINIMUM, caption::SIZE_MAXIMUM, caption::SIZE_DEFAULT),
            "margin_scale": range(caption::MARGIN_MINIMUM, caption::MARGIN_MAXIMUM, caption::MARGIN_DEFAULT),
            "width_scale": range(caption::WIDTH_MINIMUM, caption::WIDTH_MAXIMUM, caption::WIDTH_DEFAULT),
            "text_rgba": "\"#rrggbb\" or \"#rrggbbaa\"; default #ffffffff",
            "box_rgba": "plate fill or shadow colour; default #000000b7",
            "effects": {
                "drives": EffectDrive::ALL.iter().map(|value| value.canonical_name()).collect::<Vec<_>>(),
                "glow_strength": range(caption_fx::GLOW_STRENGTH_MINIMUM, caption_fx::GLOW_STRENGTH_MAXIMUM, caption_fx::GLOW_STRENGTH_DEFAULT),
                "glow_radius": range(caption_fx::GLOW_RADIUS_MINIMUM, caption_fx::GLOW_RADIUS_MAXIMUM, caption_fx::GLOW_RADIUS_DEFAULT),
                "glow_rgba": "default #ffc864ff",
                "glow_pulse": "a drive; default none",
                "glow_pulse_depth": range(caption_fx::PULSE_DEPTH_MINIMUM, caption_fx::PULSE_DEPTH_MAXIMUM, caption_fx::PULSE_DEPTH_DEFAULT),
                "glow_hue_drive": "a drive; default none",
                "glow_hue_range": range(caption_fx::HUE_RANGE_MINIMUM, caption_fx::HUE_RANGE_MAXIMUM, caption_fx::HUE_RANGE_DEFAULT),
                "shadow_blur": range(caption_fx::SHADOW_BLUR_MINIMUM, caption_fx::SHADOW_BLUR_MAXIMUM, caption_fx::SHADOW_BLUR_DEFAULT),
                "shadow_opacity": range(caption_fx::SHADOW_OPACITY_MINIMUM, caption_fx::SHADOW_OPACITY_MAXIMUM, caption_fx::SHADOW_OPACITY_DEFAULT),
                "plate_roundness": range(caption_fx::PLATE_ROUNDNESS_MINIMUM, caption_fx::PLATE_ROUNDNESS_MAXIMUM, caption_fx::PLATE_ROUNDNESS_DEFAULT),
            },
        },
        "output": {
            "resolution": "\"WIDTHxHEIGHT\", even numbers, e.g. \"1920x1080\" or \"1080x1920\"",
            "fps": "a positive whole number",
            "quality": ["balanced", "high", "master"],
        },
        "events": { "type": ["cue", "lyric", "semantic", "custom"], "value": "default 1" },
        "limits": {
            "segments": scene_switch::CAPACITY,
            "lyrics": lyrics::CUE_CAPACITY,
            "lyric_text_bytes": lyrics::TEXT_MAX_BYTES,
            "lyric_hold_seconds": DEFAULT_LYRIC_HOLD_SECONDS,
            "events": crate::scene::events::TIMELINE_CAPACITY,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::index;

    fn parse(text: &str) -> Result<Treatment, TreatmentError> {
        Treatment::parse(text.as_bytes())
    }

    fn refusal(text: &str) -> String {
        parse(text)
            .expect_err("the treatment should be refused")
            .to_string()
    }

    const MINIMAL: &str = r#"{
        "schema": "musializer.treatment/v1",
        "segments": [ { "at": 0, "scene": "spectrum" } ]
    }"#;

    #[test]
    fn the_smallest_treatment_is_one_section_from_zero() {
        let treatment = parse(MINIMAL).unwrap();
        let compiled = treatment.compile(120.0).unwrap();
        assert_eq!(compiled.base_scene, SceneId::Spectrum);
        let cues = compiled.scene_switches.cues();
        assert_eq!(cues.len(), 1);
        assert_eq!((cues[0].start_seconds, cues[0].end_seconds), (0.0, 120.0));
        assert!(compiled.scene_switches.enabled);
        assert!(cues[0].settings.captured);
        assert_eq!(
            compiled.summary,
            Summary {
                segments: 1,
                distinct_scenes: 1,
                routes: 0,
                lyrics: None,
                lyrics_fully_shadowed: 0,
                events: None,
            }
        );
        // Silent about lyrics and events, so it does not speak for them.
        assert!(compiled.lyrics.is_none());
        assert!(compiled.manual_events.is_none());
    }

    #[test]
    fn an_absent_lyrics_key_and_an_empty_list_are_different_statements() {
        let empty = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "spectrum" } ],
                 "lyrics": [], "events": [] }"#,
        )
        .unwrap()
        .compile(120.0)
        .unwrap();
        // "There are none", which replaces whatever the track had.
        assert!(empty.lyrics.as_ref().is_some_and(LyricsDocument::is_empty));
        assert!(empty
            .manual_events
            .as_ref()
            .is_some_and(EventTimeline::is_empty));
        assert_eq!(empty.summary.lyrics, Some(0));
        assert_eq!(empty.summary.events, Some(0));
    }

    #[test]
    fn sections_run_to_the_next_start_and_the_last_to_the_end() {
        let treatment = parse(
            r#"{
            "schema": "musializer.treatment/v1",
            "segments": [
                { "at": 0, "scene": "atlas" },
                { "at": "0:23", "scene": "cadence" },
                { "at": 73.5, "scene": "knot", "note": "the drop" },
                { "at": "1:36.25", "scene": "atlas" }
            ]
        }"#,
        )
        .unwrap();
        let compiled = treatment.compile(240.0).unwrap();
        let spans: Vec<(f64, f64, u32)> = compiled
            .scene_switches
            .cues()
            .iter()
            .map(|cue| (cue.start_seconds, cue.end_seconds, cue.scene_index))
            .collect();
        assert_eq!(
            spans,
            vec![
                (0.0, 23.0, SceneId::SongAtlas.index() as u32),
                (23.0, 73.5, SceneId::Cadence.index() as u32),
                (73.5, 96.25, SceneId::OpenAiKnot.index() as u32),
                (96.25, 240.0, SceneId::SongAtlas.index() as u32),
            ]
        );
        assert_eq!(compiled.summary.distinct_scenes, 3);
    }

    #[test]
    fn a_section_setting_overrides_the_scene_base_which_overrides_the_default() {
        let treatment = parse(
            r#"{
            "schema": "musializer.treatment/v1",
            "scenes": { "loom": { "settings": { "weight": 2.0, "density": 1.5 } } },
            "segments": [
                { "at": 0, "scene": "loom" },
                { "at": 10, "scene": "loom", "settings": { "weight": 0.5 } }
            ]
        }"#,
        )
        .unwrap();
        let compiled = treatment.compile(20.0).unwrap();
        let cues = compiled.scene_switches.cues();
        // The base reaches both sections; the override reaches only its own.
        assert_eq!(cues[0].settings.values[index::loom::WEIGHT], 2.0);
        assert_eq!(cues[1].settings.values[index::loom::WEIGHT], 0.5);
        assert_eq!(cues[0].settings.values[index::loom::DENSITY], 1.5);
        assert_eq!(cues[1].settings.values[index::loom::DENSITY], 1.5);
        // An untouched control is the descriptor default, in both.
        let default = settings::descriptors(SceneId::Loom)[index::loom::MOTION].default_value;
        assert_eq!(cues[0].settings.values[index::loom::MOTION], default);
        // And the track-wide table carries the base, not the section override.
        assert_eq!(
            compiled
                .scene_settings
                .get(SceneId::Loom, index::loom::WEIGHT),
            2.0
        );
    }

    #[test]
    fn a_setting_is_one_control_under_three_spellings() {
        for name in ["weight", "loom.weight", "settings.loom.weight"] {
            let text = format!(
                r#"{{ "schema": "musializer.treatment/v1",
                     "segments": [ {{ "at": 0, "scene": "loom", "settings": {{ "{name}": 1.25 }} }} ] }}"#
            );
            let compiled = parse(&text).unwrap().compile(5.0).unwrap();
            assert_eq!(
                compiled.scene_switches.cues()[0].settings.values[index::loom::WEIGHT],
                1.25,
                "{name}"
            );
        }
    }

    #[test]
    fn an_out_of_range_setting_is_refused_with_its_bounds_not_clamped() {
        let descriptor = &settings::descriptors(SceneId::Loom)[index::loom::WEIGHT];
        let message = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom", "settings": { "weight": 99 } } ] }"#,
        );
        assert!(message.contains("segments[0]"), "{message}");
        assert!(message.contains("weight"), "{message}");
        assert!(
            message.contains(&format!("{}..{}", descriptor.minimum, descriptor.maximum)),
            "{message}"
        );
    }

    #[test]
    fn an_in_range_setting_is_snapped_to_the_grid_the_readout_prints() {
        // Loom's weight prints two places (PX6): 1.23456 must not reach a file.
        let compiled = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom", "settings": { "weight": 1.23456 } } ] }"#,
        )
        .unwrap()
        .compile(5.0)
        .unwrap();
        assert_eq!(
            compiled.scene_switches.cues()[0].settings.values[index::loom::WEIGHT],
            1.23
        );
    }

    #[test]
    fn a_toggle_takes_a_boolean_or_exactly_zero_or_one() {
        let titles = |value: &str| {
            format!(
                r#"{{ "schema": "musializer.treatment/v1",
                     "segments": [ {{ "at": 0, "scene": "phosphor", "settings": {{ "titles": {value} }} }} ] }}"#
            )
        };
        for (value, expected) in [("true", 1.0), ("false", 0.0), ("1", 1.0), ("0", 0.0)] {
            let compiled = parse(&titles(value)).unwrap().compile(5.0).unwrap();
            assert_eq!(
                compiled.scene_switches.cues()[0].settings.values[index::phosphor::TITLES],
                expected,
                "{value}"
            );
        }
        assert!(refusal(&titles("0.5")).contains("switch"));
        // And a slider is not a switch.
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom", "settings": { "weight": true } } ] }"#
        )
        .contains("slider"));
    }

    #[test]
    fn a_wrong_name_is_refused_with_the_names_that_exist() {
        let scene = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "cadance" } ] }"#,
        );
        assert!(scene.contains("unknown scene \"cadance\""), "{scene}");
        assert!(scene.contains("cadence"), "{scene}");

        let setting = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom", "settings": { "wieght": 1 } } ] }"#,
        );
        assert!(
            setting.contains("loom has no setting \"wieght\""),
            "{setting}"
        );
        assert!(setting.contains("weight"), "{setting}");

        // A real key, of another scene: the two most plausible mistakes are told
        // apart, because the fixes are different.
        let foreign = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom", "settings": { "spectrum.trail": 1 } } ] }"#,
        );
        assert!(foreign.contains("is a spectrum setting"), "{foreign}");
    }

    #[test]
    fn an_unknown_field_is_refused_at_every_level() {
        for text in [
            r#"{ "schema": "musializer.treatment/v1", "segmets": [], "segments": [ { "at": 0, "scene": "loom" } ] }"#,
            r#"{ "schema": "musializer.treatment/v1", "segments": [ { "at": 0, "scene": "loom", "setings": {} } ] }"#,
            r#"{ "schema": "musializer.treatment/v1", "segments": [ { "at": 0, "scene": "loom" } ], "lyrics": [ { "at": 1, "txt": "x", "text": "x" } ] }"#,
            r##"{ "schema": "musializer.treatment/v1", "segments": [ { "at": 0, "scene": "loom" } ], "caption_style": { "colour": "#fff" } }"##,
        ] {
            assert!(
                matches!(parse(text), Err(TreatmentError::Json(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn the_plan_must_start_at_zero_and_rise() {
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 2, "scene": "loom" } ] }"#
        )
        .contains("must start at 0"));
        let order = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" }, { "at": 30, "scene": "atlas" },
                               { "at": "0:20", "scene": "knot" } ] }"#,
        );
        assert!(order.contains("segments[2] (0:20.000)"), "{order}");
        assert!(order.contains("0:30.000"), "{order}");
        // Two sections on one instant would leave the first with no span.
        assert!(parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" }, { "at": 0, "scene": "atlas" } ] }"#
        )
        .is_err());
    }

    #[test]
    fn a_section_past_the_end_of_the_track_is_refused_at_compile() {
        let treatment = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" }, { "at": "4:10", "scene": "atlas" } ] }"#,
        )
        .unwrap();
        let message = treatment.compile(239.96).unwrap_err().to_string();
        assert!(message.contains("segments[1] (4:10.000)"), "{message}");
        assert!(message.contains("3:59.960"), "{message}");
        // The same file is fine against a longer track: the refusal is about
        // this audio, which is why it is `compile`'s and not `parse`'s.
        assert!(treatment.compile(300.0).is_ok());
    }

    #[test]
    fn times_are_seconds_or_the_lyric_editors_own_grammar() {
        let at = |value: &str| {
            format!(
                r#"{{ "schema": "musializer.treatment/v1",
                     "segments": [ {{ "at": 0, "scene": "loom" }} ],
                     "lyrics": [ {{ "at": {value}, "text": "line" }} ] }}"#
            )
        };
        for (value, expected) in [
            ("73.5", 73.5),
            ("\"1:13.5\"", 73.5),
            ("\"01:13.500\"", 73.5),
            ("\"73.5\"", 73.5),
            ("0", 0.0),
        ] {
            let compiled = parse(&at(value)).unwrap().compile(200.0).unwrap();
            let document = compiled.lyrics.as_ref().unwrap();
            assert_eq!(document.cues()[0].start_seconds, expected, "{value}");
        }
        for value in [
            "-1",
            "\"1:60\"",
            "\"1:2:3\"",
            "\"soon\"",
            "true",
            "null",
            "[1]",
        ] {
            assert!(refusal(&at(value)).contains("is not a time"), "{value}");
        }
    }

    #[test]
    fn a_line_without_an_until_runs_to_the_next_line_the_hold_or_the_end() {
        let compiled = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [
                    { "at": 10, "text": "next line takes over" },
                    { "at": 13, "text": "nothing follows for a while" },
                    { "at": 40, "until": 41.5, "text": "said so" },
                    { "at": 58, "text": "the track ends first" }
                 ] }"#,
        )
        .unwrap()
        .compile(60.0)
        .unwrap();
        let spans: Vec<(f64, f64)> = compiled
            .lyrics
            .as_ref()
            .unwrap()
            .cues()
            .iter()
            .map(|cue| (cue.start_seconds, cue.end_seconds))
            .collect();
        assert_eq!(
            spans,
            vec![
                (10.0, 13.0),
                (13.0, 13.0 + DEFAULT_LYRIC_HOLD_SECONDS),
                (40.0, 41.5),
                (58.0, 60.0),
            ]
        );
        assert_eq!(compiled.summary.lyrics, Some(4));
        assert_eq!(compiled.summary.lyrics_fully_shadowed, 0);
        assert!(compiled
            .lyrics
            .as_ref()
            .unwrap()
            .cues()
            .iter()
            .all(|cue| cue.origin == CueOrigin::UserApplied));
    }

    #[test]
    fn lyric_lines_must_be_in_singing_order_and_inside_the_track() {
        let order = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [ { "at": "1:20", "text": "b" }, { "at": "1:02", "text": "a" } ] }"#,
        );
        assert!(order.contains("lyrics[1] (1:02.000)"), "{order}");
        assert!(order.contains("singing order"), "{order}");

        let treatment = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [ { "at": 10, "until": 70, "text": "too long" } ] }"#,
        )
        .unwrap();
        assert!(treatment
            .compile(60.0)
            .unwrap_err()
            .to_string()
            .contains("past the end of the track"));

        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [ { "at": 10, "until": 10, "text": "no span" } ] }"#
        )
        .contains("`until`"));
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [ { "at": 10, "text": "" } ] }"#
        )
        .contains("text is not usable"));
    }

    #[test]
    fn a_line_hidden_behind_a_later_one_is_counted_not_refused() {
        // The model allows overlap, and the later line wins the frame. A line
        // nobody will ever see is worth a number in the report.
        let compiled = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "lyrics": [ { "at": 10, "until": 12, "text": "hidden" },
                             { "at": 10, "until": 14, "text": "shown" } ] }"#,
        )
        .unwrap()
        .compile(60.0)
        .unwrap();
        assert_eq!(compiled.summary.lyrics, Some(2));
        assert_eq!(compiled.summary.lyrics_fully_shadowed, 1);
    }

    #[test]
    fn a_route_is_a_spec_string_or_a_named_object_and_both_mean_the_same() {
        let with = |route: &str| {
            format!(
                r#"{{ "schema": "musializer.treatment/v1",
                     "scenes": {{ "loom": {{ "routes": [ {route} ] }} }},
                     "segments": [ {{ "at": 0, "scene": "loom" }} ] }}"#
            )
        };
        let spec = parse(&with(r#""loom.weight:band:2:0:1:0.4:2.2:smoothstep""#))
            .unwrap()
            .compile(5.0)
            .unwrap();
        let named = parse(&with(
            r#"{ "setting": "weight", "source": "band", "band": 2,
                 "input": [0, 1], "output": [0.4, 2.2], "curve": "smoothstep" }"#,
        ))
        .unwrap()
        .compile(5.0)
        .unwrap();
        let expected = routes::parse_route_spec("loom.weight:band:2:0:1:0.4:2.2:smoothstep")
            .unwrap()
            .1;
        assert_eq!(
            spec.scene_routes.scene(SceneId::Loom).items(),
            std::slice::from_ref(&expected)
        );
        assert_eq!(named.scene_routes.scene(SceneId::Loom).items(), &[expected]);
        assert_eq!(named.summary.routes, 1);
    }

    #[test]
    fn a_bad_route_says_which_and_why() {
        let with = |route: &str| {
            format!(
                r#"{{ "schema": "musializer.treatment/v1",
                     "scenes": {{ "loom": {{ "routes": [ {route} ] }} }},
                     "segments": [ {{ "at": 0, "scene": "loom" }} ] }}"#
            )
        };
        let source = refusal(&with(
            r#"{ "setting": "weight", "source": "loudness", "input": [0, 1], "output": [0.4, 2.2] }"#,
        ));
        assert!(source.contains("scenes.loom.routes[0]"), "{source}");
        assert!(source.contains("unknown source \"loudness\""), "{source}");
        assert!(source.contains("rms"), "{source}");

        // A flat output is a constant, which is what `settings` is for.
        assert!(refusal(&with(
            r#"{ "setting": "weight", "source": "rms", "input": [0, 1], "output": [1, 1] }"#
        ))
        .contains("constant"));
        // A route for another scene's setting, in either spelling.
        assert!(refusal(&with(r#""spectrum.trail:rms:0:0:1:0:1""#))
            .contains("routes a spectrum setting inside the loom block"));
        // One setting, one source.
        let message = refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "scenes": { "loom": { "routes": [
                    "loom.weight:rms:0:0:1:0.4:2.2", "loom.weight:bass:0:0:1:0.4:2.2" ] } },
                 "segments": [ { "at": 0, "scene": "loom" } ] }"#,
        );
        assert!(message.contains("scenes.loom.routes[1]"), "{message}");
        assert!(message.contains("already routes"), "{message}");
    }

    #[test]
    fn a_caption_style_states_only_what_it_changes() {
        let treatment = parse(
            r##"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "caption_style": {
                    "face": "space_grotesk", "box": "shadow", "anchor": "middle_center",
                    "size_scale": 0.08, "text_rgba": "#FFE9C2",
                    "effects": { "glow_strength": 0.6, "glow_pulse": "beat", "glow_rgba": "ffc86480" }
                 } }"##,
        )
        .unwrap();
        let style = treatment.caption_style.unwrap();
        assert_eq!(style.face, CaptionFace::SpaceGrotesk);
        assert_eq!(style.box_style, CaptionBox::Shadow);
        assert_eq!(style.anchor, CaptionAnchor::MiddleCenter);
        assert_eq!(style.size_scale, 0.08);
        assert_eq!(style.text_rgba, 0xFFE9_C2FF);
        assert_eq!(style.effects.glow_strength, 0.6);
        assert_eq!(style.effects.glow_pulse, EffectDrive::Beat);
        assert_eq!(style.effects.glow_rgba, 0xFFC8_6480);
        // Everything unstated is the shipped default, not zero.
        let default = CaptionStyle::default();
        assert_eq!(style.margin_scale, default.margin_scale);
        assert_eq!(style.width_scale, default.width_scale);
        assert_eq!(style.box_rgba, default.box_rgba);
        assert_eq!(style.effects.glow_radius, default.effects.glow_radius);
        assert!(style.font.is_none());
        // A treatment with no block leaves the track's own style alone.
        assert!(parse(MINIMAL).unwrap().caption_style.is_none());
    }

    #[test]
    fn caption_values_are_bounded_and_named_like_everything_else() {
        let with = |body: &str| {
            format!(
                r##"{{ "schema": "musializer.treatment/v1",
                     "segments": [ {{ "at": 0, "scene": "loom" }} ],
                     "caption_style": {{ {body} }} }}"##
            )
        };
        assert!(refusal(&with(r#""size_scale": 0.9"#)).contains("caption_style.size_scale"));
        assert!(refusal(&with(r#""anchor": "centre""#)).contains("middle_center"));
        assert!(refusal(&with(r##""text_rgba": "#ffff""##)).contains("not a colour"));
        assert!(refusal(&with(r##""text_rgba": "#gggggg""##)).contains("not a colour"));
        assert!(refusal(&with(r#""effects": { "glow_pulse": "kick" }"#)).contains("beat"));
        assert!(refusal(&with(r#""face": "imported""#)).contains("cannot name an imported face"));
    }

    #[test]
    fn the_audio_block_carries_a_path_and_an_optional_pinned_digest() {
        let digest = "0d50a3cc8c1c29b56719fd188d9a591de8157a544eee2e662c427baec65f0000";
        let treatment = parse(&format!(
            r#"{{ "schema": "musializer.treatment/v1",
                 "audio": {{ "path": "song.mp3", "sha256": "{}" }},
                 "segments": [ {{ "at": 0, "scene": "loom" }} ] }}"#,
            digest.to_ascii_uppercase()
        ))
        .unwrap();
        let audio = treatment.audio.unwrap();
        assert_eq!(audio.path, "song.mp3");
        assert_eq!(audio.sha256.as_deref(), Some(digest));
        assert!(audio.verify_digest(digest).is_ok());
        let wrong = audio
            .verify_digest(&"1".repeat(64))
            .unwrap_err()
            .to_string();
        assert!(
            wrong.contains(digest) && wrong.contains(&"1".repeat(64)),
            "{wrong}"
        );

        // Unpinned is allowed, and then any file passes.
        let loose = TreatmentAudio {
            path: "song.mp3".into(),
            sha256: None,
        };
        assert!(loose.verify_digest(&"1".repeat(64)).is_ok());
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "audio": { "path": "song.mp3", "sha256": "abc" },
                 "segments": [ { "at": 0, "scene": "loom" } ] }"#
        )
        .contains("not a 64-hex sha256"));
    }

    #[test]
    fn the_output_block_is_three_independent_requests() {
        let treatment = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "output": { "resolution": "1080x1920", "quality": "master" },
                 "seed": 2012, "title": "Haul Away",
                 "segments": [ { "at": 0, "scene": "loom" } ] }"#,
        )
        .unwrap();
        assert_eq!(
            treatment.output,
            Some(OutputRequest {
                resolution: Some((1080, 1920)),
                fps: None,
                quality: Some(Quality::Master),
            })
        );
        assert_eq!(treatment.seed, Some(2012));
        assert_eq!(treatment.title.as_deref(), Some("Haul Away"));
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1", "output": { "resolution": "1080p" },
                 "segments": [ { "at": 0, "scene": "loom" } ] }"#
        )
        .contains("WIDTHxHEIGHT"));
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1", "output": { "quality": "ultra" },
                 "segments": [ { "at": 0, "scene": "loom" } ] }"#
        )
        .contains("balanced, high or master"));
    }

    #[test]
    fn events_land_in_the_authored_lane_with_allocated_ids() {
        let compiled = parse(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "constellation" } ],
                 "events": [ { "at": "1:13" }, { "at": 80, "type": "custom", "value": 0.5 } ] }"#,
        )
        .unwrap()
        .compile(200.0)
        .unwrap();
        let lane = compiled.manual_events.as_ref().unwrap();
        let events = lane.events();
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].id, events[0].timestamp_seconds), (1, 73.0));
        assert_eq!(events[0].kind(), Some(EventType::Cue));
        assert_eq!(events[0].values(), &[1.0]);
        assert_eq!(events[1].kind(), Some(EventType::Custom));
        assert_eq!(events[1].values(), &[0.5]);
        assert!(refusal(
            r#"{ "schema": "musializer.treatment/v1",
                 "segments": [ { "at": 0, "scene": "loom" } ],
                 "events": [ { "at": 1, "type": "flash" } ] }"#
        )
        .contains("unknown type \"flash\""));
    }

    #[test]
    fn the_wrong_schema_and_junk_are_refused_before_anything_is_read() {
        assert!(matches!(
            parse(r#"{ "schema": "musializer.treatment/v2", "segments": [] }"#),
            Err(TreatmentError::Schema { .. })
        ));
        assert!(matches!(parse("not json"), Err(TreatmentError::Json(_))));
        assert!(matches!(
            Treatment::parse(&vec![b' '; MAX_INPUT + 1]),
            Err(TreatmentError::TooLarge)
        ));
        assert!(
            refusal(r#"{ "schema": "musializer.treatment/v1", "segments": [] }"#)
                .contains("needs 1..=")
        );
    }

    #[test]
    fn the_vocabulary_names_every_scene_and_every_setting_it_would_accept() {
        let vocabulary = vocabulary();
        assert_eq!(vocabulary["schema"], VOCABULARY_SCHEMA);
        let scenes = vocabulary["scenes"].as_array().unwrap();
        assert_eq!(scenes.len(), SCENE_COUNT);
        for (scene, entry) in SceneId::ALL.iter().zip(scenes) {
            assert_eq!(entry["name"], scene.stable_name());
            assert!(!entry["look"].as_str().unwrap().is_empty());
            let controls = entry["settings"].as_array().unwrap();
            assert_eq!(controls.len(), settings::count(*scene));
            // The negative control on the whole document: every name it prints
            // must resolve through the parser it documents. A vocabulary that
            // drifted from `resolve_setting` would teach names that are refused.
            for control in controls {
                let name = control["name"].as_str().unwrap();
                let (_, descriptor) = resolve_setting("test", *scene, name).unwrap();
                assert_eq!(control["key"], descriptor.key);
            }
        }
        // A value prints as the decimal a person would write.
        let spectrum = &scenes[SceneId::Spectrum.index()]["settings"];
        assert!(spectrum
            .as_array()
            .unwrap()
            .iter()
            .all(|control| control["default"].to_string().len() <= 6));
        assert_eq!(
            vocabulary["routes"]["sources"].as_array().unwrap().len(),
            AnalysisSource::ALL.len()
        );
    }

    #[test]
    fn every_setting_key_is_its_scenes_name_plus_one_word() {
        // `resolve_setting` builds `settings.<scene>.<name>` from a short name.
        // That is only sound while every key has exactly that shape.
        for scene in SceneId::ALL {
            for descriptor in settings::descriptors(scene) {
                let prefix = format!("settings.{}.", scene.stable_name());
                let name = descriptor
                    .key
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|| panic!("{} is not under {prefix}", descriptor.key));
                assert!(
                    !name.is_empty() && !name.contains('.'),
                    "{}",
                    descriptor.key
                );
            }
        }
    }

    #[test]
    fn the_documented_example_is_a_treatment_this_build_accepts() {
        // `docs/MUSIC_VIDEO_CLI.md` points at this file as the thing to copy.
        // An example that the parser refuses is worse than no example, and
        // nothing else would notice: prose does not compile.
        let example = include_str!("../../../docs/examples/example.treatment.json");
        let treatment = parse(example).unwrap();
        let compiled = treatment.compile(60.0).unwrap();
        assert_eq!(compiled.summary.segments, 5);
        assert_eq!(compiled.summary.distinct_scenes, 4);
        assert_eq!(compiled.summary.routes, 2);
        assert_eq!(compiled.summary.lyrics, Some(4));
        assert_eq!(compiled.summary.events, Some(2));
        assert!(treatment.caption_style.is_some());
        // The two open-ended lines close the way the file says they do.
        let document = compiled.lyrics.as_ref().unwrap();
        let spans: Vec<(f64, f64)> = document
            .cues()
            .iter()
            .map(|cue| (cue.start_seconds, cue.end_seconds))
            .collect();
        assert_eq!(
            spans,
            vec![(8.2, 13.0), (16.0, 20.0), (20.0, 24.0), (53.0, 59.0)]
        );
    }

    #[test]
    fn format_time_is_the_grammar_parse_time_reads() {
        for seconds in [0.0, 0.5, 23.0, 73.5, 239.96, 3599.999] {
            let text = format_time(seconds);
            let back = parse_time("test", &Value::String(text.clone())).unwrap();
            assert!(
                (back - seconds).abs() < 0.0005,
                "{seconds} -> {text} -> {back}"
            );
        }
        assert_eq!(format_time(73.5), "1:13.500");
    }
}
