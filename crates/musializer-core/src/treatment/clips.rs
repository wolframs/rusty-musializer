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

use serde::Deserialize;
use serde_json::Value;

use super::{invalid, TreatmentError, MAX_PATH_BYTES};
use crate::project::clip::{ClipAsset, ClipFit, ClipOverrun, ClipPlacement, MAX_CLIP_SECONDS};
use crate::project::sha256;

/// A segment's `clip`, as written.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SegmentClip {
    /// As written: relative to the treatment file.
    pub path: String,
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
struct ClipWire {
    path: String,
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

/// Parses a segment's `clip`: a path, or an object.
pub(super) fn parse(place: &str, value: &Value) -> Result<SegmentClip, TreatmentError> {
    let wire = match value {
        Value::String(path) => ClipWire {
            path: path.clone(),
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
                 \"after\", \"fit\" }",
            ));
        }
    };
    if wire.path.is_empty() || wire.path.len() > MAX_PATH_BYTES || wire.path.contains('\0') {
        return Err(invalid(
            format!("{place}.path"),
            format!("a clip's path is 1..={MAX_PATH_BYTES} bytes"),
        ));
    }
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
        path: wire.path,
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
        "forms": ["\"path/to/clip.mp4\"", "{ \"path\", \"sha256\", \"from\", \"after\", \"fit\" }"],
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
}
