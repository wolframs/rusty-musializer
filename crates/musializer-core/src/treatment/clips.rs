//! A treatment's clip sections (post-legacy, 2026-11-14, VG): the `clip`
//! member a `clip` segment names its file and its timing with.
//!
//! ```json
//! { "at": 32, "scene": "clip", "clip": "takes/door.mp4" }
//! { "at": 40, "scene": "clip", "clip": { "path": "takes/door.mp4", "from": 2.5,
//!                                         "after": "bounce", "fit": "contain" } }
//! ```
//!
//! The file is the application's to read — this crate opens nothing — so a
//! parsed treatment lists the files it names ([`super::Treatment::clip_files`])
//! and compiles only once each has been read, hashed and probed into a
//! [`ClipAsset`] and attached ([`super::Treatment::attach_clip`]).
//!
//! A clip can instead say how a model is to make it (VG H4):
//!
//! ```json
//! { "at": 48, "scene": "clip", "clip": { "generate": {
//!     "prompt": "the door opens onto fog", "model": "google/veo-3.1-lite",
//!     "seconds": 4, "first": "previous", "last": "next" }, "after": "hold" } }
//! ```
//!
//! Such a section is listed by [`super::Treatment::clip_generations`]; the
//! application renders its frames, finds the clip it already has for the
//! request or — only when the run carries `--generate-clips --max-spend` —
//! asks for it, and attaches it under [`SegmentClip::path`], which for a
//! generated clip is a key (`generate:segments[N]`), not a file.

use serde::Deserialize;
use serde_json::Value;

use super::{invalid, TreatmentError, MAX_PATH_BYTES};
use crate::project::clip::{ClipAsset, ClipFit, ClipOverrun, ClipPlacement, MAX_CLIP_SECONDS};
use crate::project::sha256;

/// Where a generated clip's first or last frame comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameSource {
    /// The last frame of the section before, as it is drawn (`"previous"`).
    Previous,
    /// The first frame of the section after (`"next"`).
    Next,
    /// An image file, PNG or JPEG, relative to the treatment.
    File(String),
}

impl FrameSource {
    /// The treatment's spelling.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Previous => "the previous section's last frame".into(),
            Self::Next => "the next section's first frame".into(),
            Self::File(path) => path.clone(),
        }
    }
}

/// A clip a model is to make, as a treatment asks for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateSpec {
    pub prompt: String,
    pub model: String,
    pub seconds: u32,
    pub first: Option<FrameSource>,
    pub last: Option<FrameSource>,
}

/// A segment's `clip`, as written.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SegmentClip {
    /// As written: relative to the treatment file. For a generated clip, the
    /// key it is attached under (`generate:segments[N]`).
    pub path: String,
    pub generate: Option<GenerateSpec>,
    pub sha256: Option<String>,
    pub from: Option<f64>,
    pub after: ClipOverrun,
    pub fit: ClipFit,
}

impl SegmentClip {
    pub fn placement(&self, asset: &ClipAsset) -> Option<ClipPlacement> {
        Some(ClipPlacement {
            clip: asset.clip_ref()?,
            from_seconds: self.from,
            after: self.after,
            fit: self.fit,
        })
    }
}

/// A clip file a treatment names, for the application to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipFile {
    /// As written in the treatment: relative to its own directory.
    pub path: String,
    /// The digest the treatment pins, when it pins one.
    pub sha256: Option<String>,
    /// Where it was first named, for a refusal about the file.
    pub place: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateWire {
    prompt: String,
    model: String,
    seconds: u32,
    #[serde(default)]
    first: Option<String>,
    #[serde(default)]
    last: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipWire {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    generate: Option<GenerateWire>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    from: Option<Value>,
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    fit: Option<String>,
}

fn names<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|value| name(*value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The key a generated clip is attached under, from its section's place.
#[must_use]
pub(super) fn generate_key(index: usize) -> String {
    format!("generate:segments[{index}]")
}

fn frame_source(
    place: &str,
    text: Option<String>,
    word: &'static str,
) -> Result<Option<FrameSource>, TreatmentError> {
    let Some(text) = text else {
        return Ok(None);
    };
    if text == word {
        return Ok(Some(if word == "previous" {
            FrameSource::Previous
        } else {
            FrameSource::Next
        }));
    }
    if matches!(text.as_str(), "previous" | "next" | "this") {
        return Err(invalid(
            place,
            format!(
                "{text:?} is not a frame here: a first frame is \"previous\" or an image file, \
                 a last frame \"next\" or an image file (a clip section's own picture is the clip)"
            ),
        ));
    }
    if text.is_empty() || text.len() > MAX_PATH_BYTES || text.contains('\0') {
        return Err(invalid(
            place,
            format!("an image path is 1..={MAX_PATH_BYTES} bytes"),
        ));
    }
    Ok(Some(FrameSource::File(text)))
}

fn parse_generate(place: &str, wire: GenerateWire) -> Result<GenerateSpec, TreatmentError> {
    crate::generation::request::check_prompt(&wire.prompt)
        .map_err(|error| invalid(format!("{place}.prompt"), error.to_string()))?;
    let slug = |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | ':');
    if wire.model.is_empty() || wire.model.len() > 128 || !wire.model.chars().all(slug) {
        return Err(invalid(
            format!("{place}.model"),
            format!(
                "a model is an OpenRouter video model id, such as {}",
                crate::generation::PICKS[0].id
            ),
        ));
    }
    if !(1..=crate::generation::MAX_SECONDS).contains(&wire.seconds) {
        return Err(invalid(
            format!("{place}.seconds"),
            format!(
                "a generated clip lasts 1 to {} seconds; the model's own lengths are in --vocabulary",
                crate::generation::MAX_SECONDS
            ),
        ));
    }
    Ok(GenerateSpec {
        prompt: wire.prompt,
        model: wire.model,
        seconds: wire.seconds,
        first: frame_source(&format!("{place}.first"), wire.first, "previous")?,
        last: frame_source(&format!("{place}.last"), wire.last, "next")?,
    })
}

/// Parses a segment's `clip`: a path, or an object.
pub(super) fn parse(place: &str, value: &Value) -> Result<SegmentClip, TreatmentError> {
    let wire = match value {
        Value::String(path) => ClipWire {
            path: Some(path.clone()),
            generate: None,
            sha256: None,
            from: None,
            after: None,
            fit: None,
        },
        Value::Object(_) => serde_json::from_value(value.clone())
            .map_err(|error| invalid(place, error.to_string()))?,
        _ => {
            return Err(invalid(
                place,
                "a clip is the path of a video file, or { \"path\", \"sha256\", \"from\", \
                 \"after\", \"fit\" }, or { \"generate\": { \"prompt\", \"model\", \
                 \"seconds\", \"first\", \"last\" } }",
            ));
        }
    };
    let (path, generate) = match (wire.path, wire.generate) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                place,
                "a clip is a file (`path`) or generated (`generate`), not both",
            ));
        }
        (None, None) => {
            return Err(invalid(
                place,
                "a clip names its file (`path`) or says how to generate it (`generate`)",
            ));
        }
        (Some(path), None) => {
            if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
                return Err(invalid(
                    format!("{place}.path"),
                    format!("a clip's path is 1..={MAX_PATH_BYTES} bytes"),
                ));
            }
            if path.starts_with("generate:") {
                return Err(invalid(
                    format!("{place}.path"),
                    "a path beginning `generate:` is reserved; name the file `./generate:…`",
                ));
            }
            (path, None)
        }
        (None, Some(generate)) => {
            if wire.sha256.is_some() {
                return Err(invalid(
                    format!("{place}.sha256"),
                    "a generated clip has no file to pin; its request is its identity",
                ));
            }
            // The key is set by the caller, which knows the section's index.
            (
                String::new(),
                Some(parse_generate(&format!("{place}.generate"), generate)?),
            )
        }
    };
    let sha256 = match wire.sha256 {
        Some(digest) if sha256::is_hex_digest(&digest.to_ascii_lowercase()) => {
            Some(digest.to_ascii_lowercase())
        }
        Some(_) => {
            return Err(invalid(
                format!("{place}.sha256"),
                "a SHA-256 is 64 hex digits",
            ));
        }
        None => None,
    };
    let from = match &wire.from {
        None => None,
        Some(value) => {
            let seconds = super::parse_time(&format!("{place}.from"), value)?;
            if !(0.0..MAX_CLIP_SECONDS).contains(&seconds) {
                return Err(invalid(
                    format!("{place}.from"),
                    format!("seconds into the clip, 0 up to {MAX_CLIP_SECONDS}"),
                ));
            }
            Some(seconds)
        }
    };
    let after = match wire.after.as_deref() {
        None => ClipOverrun::Hold,
        Some(name) => ClipOverrun::from_name(name).ok_or_else(|| {
            invalid(
                format!("{place}.after"),
                format!(
                    "unknown {name:?}; expected {}",
                    names(&ClipOverrun::ALL, ClipOverrun::name)
                ),
            )
        })?,
    };
    let fit = match wire.fit.as_deref() {
        None => ClipFit::Cover,
        Some(name) => ClipFit::from_name(name).ok_or_else(|| {
            invalid(
                format!("{place}.fit"),
                format!(
                    "unknown {name:?}; expected {}",
                    names(&ClipFit::ALL, ClipFit::name)
                ),
            )
        })?,
    };
    Ok(SegmentClip {
        path,
        generate,
        sha256,
        from,
        after,
        fit,
    })
}

/// The `--vocabulary` entry for a clip section.
pub(super) fn vocabulary() -> Value {
    serde_json::json!({
        "scene": "clip",
        "member": "clip",
        "forms": [
            "\"path/to/clip.mp4\"",
            "{ \"path\", \"sha256\", \"from\", \"after\", \"fit\" }",
            "{ \"generate\": { \"prompt\", \"model\", \"seconds\", \"first\", \"last\" }, \"from\", \"after\", \"fit\" }"
        ],
        "generate": {
            "prompt": format!("what the clip shows, 1..={} bytes; no names or handles needed", crate::generation::MAX_PROMPT_BYTES),
            "model": "an OpenRouter video model id; the picks, with what each accepts and costs, are under video_models",
            "seconds": "one of the model's lengths",
            "first": "\"previous\" (the section before's last frame, as drawn) or a PNG/JPEG file",
            "last": "\"next\" (the section after's first frame, as drawn) or a PNG/JPEG file",
            "spending": "a run never spends by itself: without --generate-clips --max-spend USD it prints the estimate of each missing clip and exits 1; with them it asks for what is missing up to the ceiling. A clip made once is found again by its request (model, prompt, seconds, shape and the frames' bytes) and never paid twice",
            "frames_leave_the_machine": "the prompt and the frame images go to OpenRouter and the model's provider; nothing else",
        },
        "path": "relative to the treatment file; any video FFmpeg reads; its sound is ignored",
        "from": "seconds into the clip the section starts at; absent continues the section before when it plays the same clip, else 0",
        "after": names(&ClipOverrun::ALL, ClipOverrun::name),
        "after_default": ClipOverrun::Hold.name(),
        "fit": names(&ClipFit::ALL, ClipFit::name),
        "fit_default": ClipFit::Cover.name(),
        "limits": {
            "seconds": MAX_CLIP_SECONDS,
            "frames": crate::project::clip::MAX_CLIP_FRAMES,
            "largest_side": crate::project::clip::MAX_CLIP_SIDE,
            "bytes": crate::project::clip::MAX_CLIP_BYTES,
        },
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::{Treatment, TreatmentError};
    use crate::project::clip::{ClipAsset, ClipFit, ClipOverrun};
    use crate::project::sha256;

    /// A clip as the application would hand one over: ten seconds of 1080p.
    pub fn asset() -> ClipAsset {
        let digest = sha256::digest_hex(b"take");
        ClipAsset {
            path: "/clips/take.mp4".into(),
            sha256: digest,
            name: "take.mp4".into(),
            width: 1920,
            height: 1080,
            frames: 300,
            duration_seconds: 10.0,
            generated: None,
        }
    }

    fn treatment(segments: &str) -> Result<Treatment, TreatmentError> {
        Treatment::parse(
            format!(r#"{{ "schema": "musializer.treatment/v1", "segments": [ {segments} ] }}"#)
                .as_bytes(),
        )
    }

    fn reason(result: Result<Treatment, TreatmentError>) -> String {
        match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("accepted"),
        }
    }

    #[test]
    fn a_clip_section_names_its_file_and_compiles_once_it_is_read() {
        let mut parsed = treatment(
            r#"{ "at": 0, "scene": "spectrum" },
               { "at": 4, "scene": "clip", "clip": "take.mp4", "transition": "fade" },
               { "at": 9, "scene": "clip", "clip": { "path": "take.mp4", "from": "0:02.5",
                                                     "after": "bounce", "fit": "contain" } },
               { "at": 12, "scene": "clip", "clip": "take.mp4" }"#,
        )
        .unwrap();
        let files = parsed.clip_files();
        assert_eq!(files.len(), 1, "one file named three times is read once");
        assert_eq!(files[0].path, "take.mp4");
        assert!(files[0].place.starts_with("segments[1]"));
        // Unread, it does not compile, and says which.
        let refused = parsed.compile(20.0).unwrap_err().to_string();
        assert!(refused.contains("which has not been read"), "{refused}");
        parsed.attach_clip("take.mp4", asset()).unwrap();
        let compiled = parsed.compile(20.0).unwrap();
        assert_eq!(compiled.summary.clip_sections, 3);
        assert_eq!(compiled.summary.clips, 1);
        assert_eq!(compiled.clips.len(), 1);
        let cues = compiled.scene_switches.cues();
        assert_eq!(cues[0].clip, None);
        let second = cues[2].clip.unwrap();
        assert_eq!(second.from_seconds, Some(2.5));
        assert_eq!(second.after, ClipOverrun::Bounce);
        assert_eq!(second.fit, ClipFit::Contain);
        assert_eq!(cues[1].clip.unwrap().from_seconds, None);
        assert_eq!(compiled.base_clip, cues[1].clip);
        // The third continues the second's clock: no `from` of its own.
        let at_13 = compiled.scene_switches.clip_at(13.0, None).unwrap();
        assert_eq!(at_13.origin_seconds, 9.0);
        assert_eq!(at_13.from_seconds, 2.5);
        assert_eq!(at_13.local_seconds(13.0), 6.5);
        // The first clip section starts its clip at its own start.
        let at_5 = compiled.scene_switches.clip_at(5.0, None).unwrap();
        assert_eq!((at_5.origin_seconds, at_5.from_seconds), (4.0, 0.0));
    }

    #[test]
    fn a_clip_where_none_belongs_or_none_where_one_does_is_refused() {
        let message = reason(treatment(
            r#"{ "at": 0, "scene": "spectrum", "clip": "a.mp4" }"#,
        ));
        assert!(
            message.contains("only a `clip` section plays a clip"),
            "{message}"
        );
        let message = reason(treatment(r#"{ "at": 0, "scene": "clip" }"#));
        assert!(message.contains("names its video file"), "{message}");
        for (clip, expect) in [
            (
                r#"{ "path": "a.mp4", "after": "pingpong" }"#,
                "expected hold, loop, bounce",
            ),
            (
                r#"{ "path": "a.mp4", "fit": "stretch" }"#,
                "expected cover, contain",
            ),
            (r#"{ "path": "a.mp4", "from": -1 }"#, "is not a time"),
            (
                r#"{ "path": "a.mp4", "from": 600 }"#,
                "seconds into the clip",
            ),
            (r#"{ "path": "a.mp4", "sha256": "abc" }"#, "64 hex digits"),
            (r#"{ "path": "" }"#, "a clip's path"),
            (r#"{ "path": "a.mp4", "speed": 2 }"#, "unknown field"),
            ("12", "the path of a video file"),
        ] {
            let message = reason(treatment(&format!(
                r#"{{ "at": 0, "scene": "clip", "clip": {clip} }}"#
            )));
            assert!(message.contains(expect), "{clip}: {message}");
            assert!(message.contains("segments[0]"), "{clip}: {message}");
        }
    }

    #[test]
    fn a_pin_or_a_start_the_file_disagrees_with_is_refused_by_section() {
        let mut parsed = treatment(&format!(
            r#"{{ "at": 0, "scene": "clip", "clip": {{ "path": "a.mp4", "sha256": "{}" }} }}"#,
            "0".repeat(64)
        ))
        .unwrap();
        let message = parsed
            .attach_clip("a.mp4", asset())
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("segments[0] (0:00.000).clip.sha256"),
            "{message}"
        );
        let mut parsed =
            treatment(r#"{ "at": 0, "scene": "clip", "clip": { "path": "a.mp4", "from": 10 } }"#)
                .unwrap();
        let message = parsed
            .attach_clip("a.mp4", asset())
            .unwrap_err()
            .to_string();
        assert!(message.contains("10.000 s long"), "{message}");
    }

    const GENERATE: &str = r#"{ "generate": { "prompt": "fog rolls in", "model": "google/veo-3.1-lite",
                                 "seconds": 4, "first": "previous", "last": "next" }, "after": "loop" }"#;

    #[test]
    fn a_generated_clip_is_listed_keyed_and_compiles_once_attached() {
        let mut parsed = treatment(&format!(
            r#"{{ "at": 0, "scene": "spectrum" }},
               {{ "at": 4, "scene": "clip", "clip": {GENERATE} }},
               {{ "at": 8, "scene": "clip", "clip": "take.mp4" }}"#
        ))
        .unwrap();
        assert_eq!(
            parsed.clip_files().len(),
            1,
            "a generated clip is no file to read"
        );
        let generations = parsed.clip_generations();
        assert_eq!(generations.len(), 1);
        let generation = &generations[0];
        assert_eq!(generation.key, "generate:segments[1]");
        assert_eq!(
            (generation.at, generation.next_at, generation.previous_at),
            (4.0, Some(8.0), Some(0.0))
        );
        assert_eq!(generation.spec.first, Some(super::FrameSource::Previous));
        assert_eq!(generation.spec.last, Some(super::FrameSource::Next));
        assert_eq!(generation.spec.seconds, 4);
        assert!(!generation.attached);
        parsed.attach_clip("take.mp4", asset()).unwrap();
        let refused = parsed.compile(20.0).unwrap_err().to_string();
        assert!(
            refused.contains("generate:segments[1]") && refused.contains("not been read"),
            "{refused}"
        );
        let mut generated = asset();
        generated.sha256 = sha256::digest_hex(b"generated");
        parsed
            .attach_clip("generate:segments[1]", generated.clone())
            .unwrap();
        assert!(parsed.clip_generations()[0].attached);
        let compiled = parsed.compile(20.0).unwrap();
        assert_eq!(compiled.summary.clip_sections, 2);
        let cue = compiled.scene_switches.cues()[1];
        assert_eq!(cue.clip.unwrap().clip, generated.clip_ref().unwrap());
        assert_eq!(cue.clip.unwrap().after, ClipOverrun::Loop);
    }

    #[test]
    fn a_stand_in_draws_the_neighbours_and_a_cut_into_a_section_is_plain() {
        let mut parsed = treatment(&format!(
            r#"{{ "at": 0, "scene": "spectrum" }},
               {{ "at": 4, "scene": "clip", "clip": {GENERATE}, "transition": "fade" }},
               {{ "at": 8, "scene": "pulse", "transition": {{ "kind": "fade", "seconds": 1 }} }}"#
        ))
        .unwrap();
        parsed.stand_in(1);
        parsed.cut_into(2);
        let compiled = parsed.compile(20.0).unwrap();
        let cues = compiled.scene_switches.cues();
        let stand_in = crate::scene::SceneId::from_index(cues[1].scene_index as usize).unwrap();
        assert!(
            stand_in != crate::scene::SceneId::Spectrum
                && stand_in.index() != cues[2].scene_index as usize,
            "{stand_in:?}"
        );
        assert_eq!(cues[1].clip, None);
        assert_eq!(
            cues[2].transition,
            crate::project::transition::Transition::CUT
        );
    }

    #[test]
    fn a_generated_clip_that_cannot_have_its_frames_is_refused_by_name() {
        let generate = |first: &str, last: &str| {
            format!(
                r#"{{ "generate": {{ "prompt": "p", "model": "google/veo-3.1-lite", "seconds": 4{first}{last} }} }}"#
            )
        };
        for (segments, expect) in [
            (
                format!(r#"{{ "at": 0, "scene": "clip", "clip": {} }}"#, generate(r#", "first": "previous""#, "")),
                "the first section has no section before it",
            ),
            (
                format!(
                    r#"{{ "at": 0, "scene": "spectrum" }}, {{ "at": 4, "scene": "clip", "clip": {} }}"#,
                    generate("", r#", "last": "next""#)
                ),
                "the last section has no section after it",
            ),
            (
                format!(
                    r#"{{ "at": 0, "scene": "clip", "clip": {} }}, {{ "at": 4, "scene": "clip", "clip": {} }}"#,
                    generate("", r#", "last": "next""#),
                    generate("", "")
                ),
                "the next section is generated too",
            ),
            (
                format!(r#"{{ "at": 0, "scene": "clip", "clip": {} }}"#, generate(r#", "first": "this""#, "")),
                "a clip section's own picture is the clip",
            ),
            (
                format!(r#"{{ "at": 0, "scene": "clip", "clip": {} }}"#, generate(r#", "first": "next""#, "")),
                "is not a frame here",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "path": "a.mp4", "generate": { "prompt": "p", "model": "m/x", "seconds": 4 } } }"#.to_owned(),
                "not both",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "from": 1 } }"#.to_owned(),
                "names its file",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "generate": { "prompt": " ", "model": "m/x", "seconds": 4 } } }"#.to_owned(),
                "the prompt is empty",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "generate": { "prompt": "p", "model": "m x", "seconds": 4 } } }"#.to_owned(),
                "an OpenRouter video model id",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "generate": { "prompt": "p", "model": "m/x", "seconds": 31 } } }"#.to_owned(),
                "1 to 30 seconds",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "generate": { "prompt": "p", "model": "m/x", "seconds": 4, "audio": true } } }"#.to_owned(),
                "unknown field",
            ),
            (
                r#"{ "at": 0, "scene": "clip", "clip": { "path": "generate:x" } }"#.to_owned(),
                "reserved",
            ),
        ] {
            let message = reason(treatment(&segments));
            assert!(message.contains(expect), "{segments}: {message}");
        }
    }
}
