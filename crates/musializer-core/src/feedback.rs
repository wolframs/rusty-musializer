//! Human-feedback protocols: the `*.protocol.json` schema and the append-only
//! answers log (HX-1, HX-3).
//!
//! **Post-legacy product extension (HX, operator proposal 2026-08-08).** The
//! frozen C has nothing like this. The loop it replaces is structural: an agent
//! writes a prose listening protocol into the plan, the operator holds it in
//! their head, runs the app, and reports back in chat — three lossy hops, and
//! one thing prose can never do: *blind* the operator. A plan section that says
//! which tuning is "current" and which is "proposed" has already unblinded the
//! comparison. The application is the only party that can apply variant `a` or
//! `b` without saying which, so the questions have to be a file the application
//! can execute.
//!
//! # Boundaries
//!
//! This module is pure: it parses bytes, validates, and formats strings. The
//! application owns every file edge — reading the protocol, hashing the audio,
//! appending answer lines. A protocol refers to its audio by path **and**
//! sha256 digest (the ASCII-import pattern) so the wrong track is refused
//! rather than mis-asked; the digest comparison lives here
//! ([`Protocol::verify_audio_digest`]) and the hashing lives with the caller.
//!
//! # Why the answers file is JSONL, not JSON
//!
//! Append-only, one record per line, because a crash mid-session must lose at
//! most one line and an agent reading the log needs no lock. An atomically
//! replaced JSON array would re-serialize the whole history on every keypress
//! and turn a crash into a zero-answer file. [`read_answer_log`] therefore
//! tolerates exactly one torn line, and only at the end.

use crate::project::sha256;
use crate::scene::settings::{SettingsSnapshot, MAX_CONTROLS};
use crate::scene::SceneId;
use serde::{Deserialize, Serialize};

/// The protocol file's `schema` value. Versioned so a later shape can add
/// fields without this parser guessing at them — the parser is strict
/// (`deny_unknown_fields`), matching the `.musi` codec's stance.
pub const PROTOCOL_SCHEMA: &str = "musializer.protocol/v1";

/// The answers file's per-line `schema` value.
pub const ANSWER_SCHEMA: &str = "musializer.protocol-answer/v1";

/// The most items one protocol may carry. A listening session past this is two
/// sessions; the bound exists so a malformed file cannot allocate unbounded.
pub const MAX_ITEMS: usize = 64;

/// Options per item: two to four, because answers land on keys `1`–`4`.
pub const MAX_OPTIONS: usize = 4;
/// A single option is not a question.
pub const MIN_OPTIONS: usize = 2;

/// Byte bounds on the free-text fields, applied before anything else looks at
/// them. Generous for a question, hostile to a file that is not a protocol.
pub const MAX_QUESTION_BYTES: usize = 500;
pub const MAX_OPTION_BYTES: usize = 64;
pub const MAX_TITLE_BYTES: usize = 200;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_ID_BYTES: usize = 64;

/// The largest input [`Protocol::parse`] will look at: 1 MiB, far above any
/// real protocol and far below anything that hurts.
pub const MAX_INPUT: usize = 1024 * 1024;

// -- errors --------------------------------------------------------------------

/// Why a protocol file was refused. Every variant names the item where it can,
/// because "item cadence-p3 names scene `cadance`" is actionable and "invalid
/// file" is not.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("protocol file exceeds {MAX_INPUT} bytes")]
    TooLarge,
    #[error("not a protocol file: {0}")]
    Json(String),
    #[error("schema is {found:?}, this build reads {PROTOCOL_SCHEMA:?}")]
    Schema { found: String },
    #[error("audio digest is not a 64-hex sha256: {found:?}")]
    BadDigest { found: String },
    #[error("audio digest mismatch: protocol expects {expected}, file is {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("field {field} is out of bounds: {reason}")]
    Field {
        field: &'static str,
        reason: &'static str,
    },
    #[error("a protocol needs 1..={MAX_ITEMS} items, this one has {count}")]
    ItemCount { count: usize },
    #[error("item id {id:?} is not 1..={MAX_ID_BYTES} of [A-Za-z0-9._-]")]
    BadItemId { id: String },
    #[error("item id {id:?} appears twice")]
    DuplicateItemId { id: String },
    #[error("item {id}: {reason}")]
    Item { id: String, reason: &'static str },
    #[error("item {id} names unknown scene {scene:?}")]
    UnknownScene { id: String, scene: String },
    #[error("item {id}: snapshot {variant} is not valid for scene {scene:?}")]
    InvalidSnapshot {
        id: String,
        variant: Variant,
        scene: &'static str,
    },
}

// -- wire shape ----------------------------------------------------------------
//
// The serde structs are the file format; the domain types below them are what
// the application holds. Kept separate so the wire stays strict and the domain
// stays resolved (a `SceneId`, not a token; a `SettingsSnapshot`, not a Vec).

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolWire {
    schema: String,
    title: String,
    audio: AudioWire,
    items: Vec<ItemWire>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioWire {
    path: String,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemWire {
    id: String,
    at_seconds: f64,
    window: WindowWire,
    question: String,
    kind: AnswerKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    apply: Option<ApplyWire>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowWire {
    pre: f64,
    post: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyWire {
    scene: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    snapshots: SnapshotsWire,
}

/// Labelled only `a`/`b` — `deny_unknown_fields` is what enforces "only".
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotsWire {
    a: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    b: Option<Vec<f32>>,
}

// -- domain --------------------------------------------------------------------

/// How an item is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnswerKind {
    /// One of 2–4 named options, keys `1`–`4`.
    Choice,
    /// An ordered rating whose points are the options, keys `1`–`4`. Same keys
    /// as `choice`; the distinction is for the reader of the answers file,
    /// where a scale's options are ordered and a choice's are nominal.
    Scale,
    /// Free text, typed in the app.
    Text,
}

impl AnswerKind {
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Scale => "scale",
            Self::Text => "text",
        }
    }
}

/// Which of an item's two snapshots is meant. The label deliberately carries no
/// other information — `a` is not "current" and `b` is not "proposed"; whichever
/// party wrote the file knows, and the screen never says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    A,
    B,
}

impl Variant {
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }

    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.token())
    }
}

/// The audition window around an item's anchor time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Seconds of run-up played before `at_seconds`.
    pub pre: f64,
    /// Seconds played after it.
    pub post: f64,
}

/// A resolved apply block: what the runner puts on screen before asking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Apply {
    pub scene: SceneId,
    /// Provenance of the draw that produced the snapshots, for the agent
    /// digesting the answers. The runner never shows it.
    pub seed: Option<u64>,
    pub a: SettingsSnapshot,
    pub b: Option<SettingsSnapshot>,
}

impl Apply {
    #[must_use]
    pub fn snapshot(&self, variant: Variant) -> Option<SettingsSnapshot> {
        match variant {
            Variant::A => Some(self.a),
            Variant::B => self.b,
        }
    }

    /// True when there are two snapshots to compare blind.
    #[must_use]
    pub fn is_ab(&self) -> bool {
        self.b.is_some()
    }
}

/// One question at one moment of one track.
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolItem {
    pub id: String,
    pub at_seconds: f64,
    pub window: Window,
    pub question: String,
    pub kind: AnswerKind,
    /// Empty exactly when `kind` is [`AnswerKind::Text`].
    pub options: Vec<String>,
    pub apply: Option<Apply>,
}

/// A parsed, fully validated protocol.
#[derive(Clone, Debug, PartialEq)]
pub struct Protocol {
    pub title: String,
    /// The audio the questions are about, as the author wrote it. May be
    /// relative; the application resolves it against the protocol file's own
    /// directory, the same rule project assets use.
    pub audio_path: String,
    /// Lowercase 64-hex sha256 of the audio file's bytes.
    pub audio_sha256: String,
    pub items: Vec<ProtocolItem>,
}

impl Protocol {
    /// Parse and validate a protocol file's bytes.
    ///
    /// # Errors
    ///
    /// Refuses — with the first offending item named — rather than repairs:
    /// junk JSON, an unknown schema or answer kind, an out-of-charset or
    /// duplicated id, a non-hex digest, option counts outside `2..=4` (or any
    /// options at all on a `text` item), a scene token the registry does not
    /// know, and a snapshot that is not valid for the scene its own item
    /// names.
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_INPUT {
            return Err(ProtocolError::TooLarge);
        }
        let wire: ProtocolWire = serde_json::from_slice(bytes)
            .map_err(|error| ProtocolError::Json(error.to_string()))?;
        if wire.schema != PROTOCOL_SCHEMA {
            return Err(ProtocolError::Schema { found: wire.schema });
        }
        if wire.title.is_empty() || wire.title.len() > MAX_TITLE_BYTES {
            return Err(ProtocolError::Field {
                field: "title",
                reason: "must be 1..=200 bytes",
            });
        }
        if wire.audio.path.is_empty() || wire.audio.path.len() > MAX_PATH_BYTES {
            return Err(ProtocolError::Field {
                field: "audio.path",
                reason: "must be 1..=4096 bytes",
            });
        }
        let digest = wire.audio.sha256.to_ascii_lowercase();
        if !sha256::is_hex_digest(&digest) {
            return Err(ProtocolError::BadDigest {
                found: wire.audio.sha256,
            });
        }
        if wire.items.is_empty() || wire.items.len() > MAX_ITEMS {
            return Err(ProtocolError::ItemCount {
                count: wire.items.len(),
            });
        }

        let mut items = Vec::with_capacity(wire.items.len());
        for item in wire.items {
            let parsed = parse_item(item)?;
            if items
                .iter()
                .any(|existing: &ProtocolItem| existing.id == parsed.id)
            {
                return Err(ProtocolError::DuplicateItemId { id: parsed.id });
            }
            items.push(parsed);
        }

        Ok(Self {
            title: wire.title,
            audio_path: wire.audio.path,
            audio_sha256: digest,
            items,
        })
    }

    /// Compare the protocol's declared digest against the digest of the audio
    /// the application actually opened.
    ///
    /// # Errors
    ///
    /// [`ProtocolError::DigestMismatch`] naming both digests — the wrong track
    /// is refused rather than mis-asked, exactly as an ASCII image asset is.
    pub fn verify_audio_digest(&self, actual_hex: &str) -> Result<(), ProtocolError> {
        let actual = actual_hex.to_ascii_lowercase();
        if actual != self.audio_sha256 {
            return Err(ProtocolError::DigestMismatch {
                expected: self.audio_sha256.clone(),
                actual,
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn item(&self, id: &str) -> Option<&ProtocolItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Serialize back to the wire format, pretty-printed. What an agent (or the
    /// HX-4 generate button, later) writes to disk.
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        let wire = ProtocolWire {
            schema: PROTOCOL_SCHEMA.to_string(),
            title: self.title.clone(),
            audio: AudioWire {
                path: self.audio_path.clone(),
                sha256: self.audio_sha256.clone(),
            },
            items: self.items.iter().map(item_to_wire).collect(),
        };
        let mut text =
            serde_json::to_string_pretty(&wire).expect("a validated protocol always serializes");
        text.push('\n');
        text
    }
}

fn item_to_wire(item: &ProtocolItem) -> ItemWire {
    ItemWire {
        id: item.id.clone(),
        at_seconds: item.at_seconds,
        window: WindowWire {
            pre: item.window.pre,
            post: item.window.post,
        },
        question: item.question.clone(),
        kind: item.kind,
        options: item.options.clone(),
        apply: item.apply.as_ref().map(|apply| ApplyWire {
            scene: apply.scene.stable_name().to_string(),
            seed: apply.seed,
            snapshots: SnapshotsWire {
                a: snapshot_values(&apply.a),
                b: apply.b.as_ref().map(snapshot_values),
            },
        }),
    }
}

fn snapshot_values(snapshot: &SettingsSnapshot) -> Vec<f32> {
    snapshot.values[..snapshot.count].to_vec()
}

fn valid_item_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

fn parse_item(wire: ItemWire) -> Result<ProtocolItem, ProtocolError> {
    if !valid_item_id(&wire.id) {
        return Err(ProtocolError::BadItemId { id: wire.id });
    }
    let id = wire.id;
    let fail = |reason: &'static str| ProtocolError::Item {
        id: id.clone(),
        reason,
    };

    if !wire.at_seconds.is_finite() || wire.at_seconds < 0.0 || wire.at_seconds > 36_000.0 {
        return Err(fail("at_seconds must be finite, 0..=36000"));
    }
    for (value, name) in [
        (wire.window.pre, "window.pre must be finite, 0..=60"),
        (wire.window.post, "window.post must be finite, 0..=600"),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(fail(name));
        }
    }
    if wire.window.pre > 60.0 {
        return Err(fail("window.pre must be finite, 0..=60"));
    }
    if wire.window.post > 600.0 {
        return Err(fail("window.post must be finite, 0..=600"));
    }
    if wire.window.pre + wire.window.post <= 0.0 {
        return Err(fail("the audition window must have positive length"));
    }
    if wire.question.is_empty() || wire.question.len() > MAX_QUESTION_BYTES {
        return Err(fail("question must be 1..=500 bytes"));
    }

    match wire.kind {
        AnswerKind::Choice | AnswerKind::Scale => {
            if wire.options.len() < MIN_OPTIONS || wire.options.len() > MAX_OPTIONS {
                return Err(fail("choice/scale items need 2..=4 options"));
            }
            if wire
                .options
                .iter()
                .any(|option| option.is_empty() || option.len() > MAX_OPTION_BYTES)
            {
                return Err(fail("each option must be 1..=64 bytes"));
            }
        }
        AnswerKind::Text => {
            if !wire.options.is_empty() {
                return Err(fail("a text item takes no options"));
            }
        }
    }

    let apply = match wire.apply {
        None => None,
        Some(apply_wire) => {
            let Some(scene) = SceneId::from_stable_name(&apply_wire.scene) else {
                return Err(ProtocolError::UnknownScene {
                    id,
                    scene: apply_wire.scene,
                });
            };
            let a = parse_snapshot(&id, scene, Variant::A, &apply_wire.snapshots.a)?;
            let b = match &apply_wire.snapshots.b {
                None => None,
                Some(values) => Some(parse_snapshot(&id, scene, Variant::B, values)?),
            };
            Some(Apply {
                scene,
                seed: apply_wire.seed,
                a,
                b,
            })
        }
    };

    Ok(ProtocolItem {
        id,
        at_seconds: wire.at_seconds,
        window: Window {
            pre: wire.window.pre,
            post: wire.window.post,
        },
        question: wire.question,
        kind: wire.kind,
        options: wire.options,
        apply,
    })
}

/// Build and validate one snapshot against the scene its item names.
///
/// The count comes from the array's own length, so a snapshot written for a
/// twelve-control scene is refused by an eight-control one — this is the
/// "snapshot for a scene the file doesn't name" refusal, and it reuses
/// [`SettingsSnapshot::is_valid_for`], the same validation a `.musi` cue
/// snapshot passes through.
fn parse_snapshot(
    id: &str,
    scene: SceneId,
    variant: Variant,
    values: &[f32],
) -> Result<SettingsSnapshot, ProtocolError> {
    let refuse = || ProtocolError::InvalidSnapshot {
        id: id.to_string(),
        variant,
        scene: scene.stable_name(),
    };
    if values.is_empty() || values.len() > MAX_CONTROLS {
        return Err(refuse());
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(refuse());
    }
    let mut snapshot = SettingsSnapshot {
        captured: true,
        count: values.len(),
        values: [0.0; MAX_CONTROLS],
    };
    snapshot.values[..values.len()].copy_from_slice(values);
    if !snapshot.is_valid_for(scene) {
        return Err(refuse());
    }
    Ok(snapshot)
}

// -- the answers log -----------------------------------------------------------

/// Why an answers line was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnswerError {
    #[error("not an answer line: {0}")]
    Json(String),
    #[error("answer schema is {found:?}, this build reads {ANSWER_SCHEMA:?}")]
    Schema { found: String },
    #[error("answer field {field} is out of bounds")]
    Field { field: &'static str },
}

/// One appended answer. Everything the digesting agent needs and nothing the
/// screen ever showed: `variant_order` is the order the application *actually
/// played* the snapshots, recorded at answer time, which is how the unblinding
/// survives for the agent while never reaching the operator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRecord {
    pub schema: String,
    pub item_id: String,
    /// The chosen key, 1-based, for `choice`/`scale` items. Absent for `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<u8>,
    /// The chosen option's label, or the typed text.
    pub answer: String,
    /// Every variant the app put on screen for this item, in play order.
    /// Empty for an item with no apply block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variant_order: Vec<Variant>,
    /// How many times the window was auditioned before answering. Itself
    /// feedback: a question auditioned five times was a hard question.
    pub auditions: u32,
    /// Where the playhead sat when the answer landed.
    pub playhead_seconds: f64,
    /// Wall clock, seconds since the epoch, supplied by the caller — this
    /// module reads no clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at_unix: Option<u64>,
}

impl AnswerRecord {
    /// A record ready to fill in.
    #[must_use]
    pub fn new(item_id: &str, answer: &str) -> Self {
        Self {
            schema: ANSWER_SCHEMA.to_string(),
            item_id: item_id.to_string(),
            choice: None,
            answer: answer.to_string(),
            variant_order: Vec::new(),
            auditions: 0,
            playhead_seconds: 0.0,
            answered_at_unix: None,
        }
    }

    /// One JSONL line, newline **included** so the caller can hand it straight
    /// to an append.
    #[must_use]
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("an answer record always serializes");
        debug_assert!(!line.contains('\n'), "serde_json emits no raw newlines");
        line.push('\n');
        line
    }

    /// Parse one line back.
    ///
    /// # Errors
    ///
    /// Junk JSON, a foreign schema, or an empty `item_id`/`answer` shape that
    /// could not have come from [`Self::to_line`].
    pub fn parse_line(line: &str) -> Result<Self, AnswerError> {
        let record: Self = serde_json::from_str(line.trim_end_matches(['\r', '\n']))
            .map_err(|error| AnswerError::Json(error.to_string()))?;
        if record.schema != ANSWER_SCHEMA {
            return Err(AnswerError::Schema {
                found: record.schema,
            });
        }
        if record.item_id.is_empty() {
            return Err(AnswerError::Field { field: "item_id" });
        }
        if !record.playhead_seconds.is_finite() {
            return Err(AnswerError::Field {
                field: "playhead_seconds",
            });
        }
        Ok(record)
    }
}

/// A read of a whole answers file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnswerLog {
    pub records: Vec<AnswerRecord>,
    /// A final line that did not parse — the crash-mid-append case the format
    /// exists to bound. `Some` is worth a notice; it is not worth refusing the
    /// lines above it.
    pub torn_tail: Option<String>,
}

impl AnswerLog {
    /// The most recent answer for an item, which is the one that counts —
    /// re-answering an item appends rather than rewrites.
    #[must_use]
    pub fn latest_for(&self, item_id: &str) -> Option<&AnswerRecord> {
        self.records
            .iter()
            .rev()
            .find(|record| record.item_id == item_id)
    }

    /// How many distinct items have at least one answer.
    #[must_use]
    pub fn answered_count(&self) -> usize {
        let mut seen: Vec<&str> = Vec::new();
        for record in &self.records {
            if !seen.contains(&record.item_id.as_str()) {
                seen.push(&record.item_id);
            }
        }
        seen.len()
    }
}

/// Read an answers file's text.
///
/// # Errors
///
/// A malformed line **before** the last one is a corrupt file and is refused
/// with its line number — only the final line may be torn, because only the
/// final line can be the one a crash interrupted.
pub fn read_answer_log(text: &str) -> Result<AnswerLog, (usize, AnswerError)> {
    let mut log = AnswerLog::default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    for (index, line) in lines.iter().enumerate() {
        match AnswerRecord::parse_line(line) {
            Ok(record) => log.records.push(record),
            Err(error) if index + 1 == lines.len() => {
                log.torn_tail = Some((*line).to_string());
                let _ = error;
            }
            Err(error) => return Err((index + 1, error)),
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::{self, SceneSettings};

    fn snapshot_for(scene: SceneId) -> SettingsSnapshot {
        SceneSettings::default().capture(scene).unwrap()
    }

    fn sample() -> Protocol {
        let atlas = snapshot_for(SceneId::SongAtlas);
        let mut atlas_b = atlas;
        atlas_b.values[0] = 2.0;
        Protocol {
            title: "CX-4 Surprise keepability".to_string(),
            audio_path: "fixtures/sparse.wav".to_string(),
            audio_sha256: sha256::digest_hex(b"the track"),
            items: vec![
                ProtocolItem {
                    id: "atlas-p1".to_string(),
                    at_seconds: 30.0,
                    window: Window {
                        pre: 2.0,
                        post: 8.0,
                    },
                    question: "Blind A/B: press 1-3.".to_string(),
                    kind: AnswerKind::Choice,
                    options: vec![
                        "keep".to_string(),
                        "fixable".to_string(),
                        "reject".to_string(),
                    ],
                    apply: Some(Apply {
                        scene: SceneId::SongAtlas,
                        seed: Some(7),
                        a: atlas,
                        b: Some(atlas_b),
                    }),
                },
                ProtocolItem {
                    id: "free-1".to_string(),
                    at_seconds: 45.0,
                    window: Window {
                        pre: 1.0,
                        post: 5.0,
                    },
                    question: "Anything else about this moment?".to_string(),
                    kind: AnswerKind::Text,
                    options: Vec::new(),
                    apply: None,
                },
            ],
        }
    }

    #[test]
    fn a_protocol_round_trips_exactly() {
        let protocol = sample();
        let json = protocol.to_json_pretty();
        let back = Protocol::parse(json.as_bytes()).unwrap();
        assert_eq!(back, protocol);
    }

    #[test]
    fn junk_json_is_refused_not_repaired() {
        for junk in [
            &b"not json at all"[..],
            b"{\"schema\": \"musializer.protocol/v1\"",
            b"[]",
            b"{}",
        ] {
            assert!(
                matches!(Protocol::parse(junk), Err(ProtocolError::Json(_))),
                "{junk:?} should be refused as junk"
            );
        }
    }

    #[test]
    fn a_foreign_schema_is_refused_by_name() {
        let text = sample()
            .to_json_pretty()
            .replace("musializer.protocol/v1", "musializer.protocol/v2");
        match Protocol::parse(text.as_bytes()) {
            Err(ProtocolError::Schema { found }) => {
                assert_eq!(found, "musializer.protocol/v2");
            }
            other => panic!("expected a schema refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_field_is_refused() {
        // The strictness stance of the .musi codec: a file from a future build
        // that added a field is refused loudly, never half-read.
        let text = sample()
            .to_json_pretty()
            .replace("\"title\"", "\"speed\": 3, \"title\"");
        assert!(matches!(
            Protocol::parse(text.as_bytes()),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn an_unknown_answer_kind_is_refused() {
        let text = sample().to_json_pretty().replace("\"choice\"", "\"rank\"");
        assert!(matches!(
            Protocol::parse(text.as_bytes()),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn an_unknown_scene_token_is_refused_by_name() {
        let text = sample()
            .to_json_pretty()
            .replace("\"atlas\"", "\"atlantis\"");
        match Protocol::parse(text.as_bytes()) {
            Err(ProtocolError::UnknownScene { id, scene }) => {
                assert_eq!(id, "atlas-p1");
                assert_eq!(scene, "atlantis");
            }
            other => panic!("expected an unknown-scene refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_for_a_scene_the_item_does_not_name_is_refused() {
        // Song Atlas has 12 controls, Cadence 7. Renaming the scene under an
        // atlas-sized snapshot must fail snapshot validation, not squeeze in.
        let text = sample()
            .to_json_pretty()
            .replace("\"atlas\"", "\"cadence\"");
        match Protocol::parse(text.as_bytes()) {
            Err(ProtocolError::InvalidSnapshot { id, variant, scene }) => {
                assert_eq!(id, "atlas-p1");
                assert_eq!(variant, Variant::A);
                assert_eq!(scene, "cadence");
            }
            other => panic!("expected a snapshot refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_with_an_out_of_range_value_is_refused() {
        let mut protocol = sample();
        let apply = protocol.items[0].apply.as_mut().unwrap();
        // settings.atlas.height is 0.35..2.75.
        apply.a.values[0] = 99.0;
        let text = protocol.to_json_pretty();
        assert!(matches!(
            Protocol::parse(text.as_bytes()),
            Err(ProtocolError::InvalidSnapshot { .. })
        ));
    }

    #[test]
    fn a_third_snapshot_label_is_refused() {
        let text = sample()
            .to_json_pretty()
            .replace("\"a\": [", "\"c\": [1.0], \"a\": [");
        assert!(matches!(
            Protocol::parse(text.as_bytes()),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn the_wrong_audio_digest_is_refused_with_both_digests() {
        let protocol = sample();
        let actual = sha256::digest_hex(b"a different track");
        match protocol.verify_audio_digest(&actual) {
            Err(ProtocolError::DigestMismatch { expected, actual }) => {
                assert_eq!(expected, protocol.audio_sha256);
                assert_eq!(actual, sha256::digest_hex(b"a different track"));
            }
            other => panic!("expected a digest refusal, got {other:?}"),
        }
        // And the right digest passes, case-insensitively.
        protocol
            .verify_audio_digest(&protocol.audio_sha256.to_ascii_uppercase())
            .unwrap();
    }

    #[test]
    fn a_malformed_declared_digest_is_refused_at_parse() {
        let protocol = sample();
        let text = protocol
            .to_json_pretty()
            .replace(&protocol.audio_sha256, "not-a-digest");
        assert!(matches!(
            Protocol::parse(text.as_bytes()),
            Err(ProtocolError::BadDigest { .. })
        ));
    }

    #[test]
    fn item_ids_are_bounded_unique_and_probe_safe() {
        // ':' is the probe spec's own separator (protocol-answer=ID:CHOICE), so
        // an id carrying one could never be addressed by the gate.
        let mut protocol = sample();
        protocol.items[1].id = "free:1".to_string();
        assert!(matches!(
            Protocol::parse(protocol.to_json_pretty().as_bytes()),
            Err(ProtocolError::BadItemId { .. })
        ));

        let mut duplicate = sample();
        duplicate.items[1].id = "atlas-p1".to_string();
        assert!(matches!(
            Protocol::parse(duplicate.to_json_pretty().as_bytes()),
            Err(ProtocolError::DuplicateItemId { .. })
        ));
    }

    #[test]
    fn option_counts_follow_the_keys_that_answer_them() {
        let mut one = sample();
        one.items[0].options.truncate(1);
        assert!(matches!(
            Protocol::parse(one.to_json_pretty().as_bytes()),
            Err(ProtocolError::Item { .. })
        ));

        let mut five = sample();
        five.items[0]
            .options
            .extend(["d".to_string(), "e".to_string(), "f".to_string()]);
        assert!(matches!(
            Protocol::parse(five.to_json_pretty().as_bytes()),
            Err(ProtocolError::Item { .. })
        ));

        let mut text_with_options = sample();
        text_with_options.items[1].options = vec!["yes".to_string(), "no".to_string()];
        assert!(matches!(
            Protocol::parse(text_with_options.to_json_pretty().as_bytes()),
            Err(ProtocolError::Item { .. })
        ));
    }

    #[test]
    fn windows_and_anchors_must_be_finite_and_sane() {
        let mut backwards = sample();
        backwards.items[0].at_seconds = -1.0;
        assert!(Protocol::parse(backwards.to_json_pretty().as_bytes()).is_err());

        let mut zero_window = sample();
        zero_window.items[0].window = Window {
            pre: 0.0,
            post: 0.0,
        };
        assert!(Protocol::parse(zero_window.to_json_pretty().as_bytes()).is_err());

        // NaN cannot survive to_json_pretty (serde_json refuses it), so build
        // the text by hand for the parse-side check.
        let text = sample()
            .to_json_pretty()
            .replace("\"at_seconds\": 45.0", "\"at_seconds\": 1e400");
        assert!(Protocol::parse(text.as_bytes()).is_err());
    }

    // -- answers ----------------------------------------------------------------

    fn answer() -> AnswerRecord {
        AnswerRecord {
            schema: ANSWER_SCHEMA.to_string(),
            item_id: "atlas-p1".to_string(),
            choice: Some(2),
            answer: "fixable".to_string(),
            variant_order: vec![Variant::B, Variant::A, Variant::B],
            auditions: 3,
            playhead_seconds: 31.25,
            answered_at_unix: Some(1_754_000_000),
        }
    }

    #[test]
    fn an_answer_line_round_trips_and_stays_one_line() {
        let record = answer();
        let line = record.to_line();
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        assert_eq!(AnswerRecord::parse_line(&line).unwrap(), record);
    }

    #[test]
    fn the_variant_order_survives_verbatim() {
        // The whole point of HX-3: the unblinding is in the file, not on the
        // screen. b-a-b in must be b-a-b out.
        let line = answer().to_line();
        let back = AnswerRecord::parse_line(&line).unwrap();
        assert_eq!(back.variant_order, vec![Variant::B, Variant::A, Variant::B]);
    }

    #[test]
    fn a_torn_final_line_is_reported_not_fatal() {
        let mut text = String::new();
        text.push_str(&answer().to_line());
        let mut second = answer();
        second.item_id = "free-1".to_string();
        text.push_str(&second.to_line());
        // The crash: a third answer, half flushed.
        text.push_str("{\"schema\":\"musializer.protocol-answer/v1\",\"item_id\":\"at");

        let log = read_answer_log(&text).unwrap();
        assert_eq!(log.records.len(), 2);
        assert!(log.torn_tail.is_some());
        assert_eq!(log.answered_count(), 2);
    }

    #[test]
    fn a_torn_middle_line_is_a_corrupt_file() {
        let mut text = String::new();
        text.push_str("{\"half\":");
        text.push('\n');
        text.push_str(&answer().to_line());
        let error = read_answer_log(&text).unwrap_err();
        assert_eq!(error.0, 1, "the refusal names the line");
    }

    #[test]
    fn re_answering_appends_and_the_latest_wins() {
        let mut first = answer();
        first.choice = Some(1);
        first.answer = "keep".to_string();
        let second = answer();
        let text = format!("{}{}", first.to_line(), second.to_line());
        let log = read_answer_log(&text).unwrap();
        assert_eq!(log.records.len(), 2);
        assert_eq!(log.answered_count(), 1);
        assert_eq!(log.latest_for("atlas-p1").unwrap().answer, "fixable");
    }

    #[test]
    fn a_foreign_answer_schema_is_refused() {
        let line = answer()
            .to_line()
            .replace("protocol-answer/v1", "answer/v9");
        assert!(matches!(
            AnswerRecord::parse_line(&line),
            Err(AnswerError::Schema { .. })
        ));
    }

    #[test]
    fn every_scene_token_in_an_apply_block_resolves_and_validates() {
        // The schema must be able to carry any scene the registry knows,
        // including the post-legacy eleventh.
        for scene in SceneId::ALL {
            let protocol = Protocol {
                title: "sweep".to_string(),
                audio_path: "x.wav".to_string(),
                audio_sha256: sha256::digest_hex(b"x"),
                items: vec![ProtocolItem {
                    id: format!("{}-1", scene.stable_name()),
                    at_seconds: 1.0,
                    window: Window {
                        pre: 1.0,
                        post: 2.0,
                    },
                    question: "?".to_string(),
                    kind: AnswerKind::Choice,
                    options: vec!["keep".to_string(), "reject".to_string()],
                    apply: Some(Apply {
                        scene,
                        seed: None,
                        a: snapshot_for(scene),
                        b: None,
                    }),
                }],
            };
            let back = Protocol::parse(protocol.to_json_pretty().as_bytes()).unwrap();
            let apply = back.items[0].apply.unwrap();
            assert_eq!(apply.scene, scene);
            assert_eq!(apply.a.count, settings::count(scene));
            assert!(!apply.is_ab());
        }
    }
}
