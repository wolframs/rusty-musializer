//! One request to a video model: what it asks, what it sends, what it is.

use serde_json::{json, Value};

use super::catalog::Offer;
use super::price::Shape;
use super::{MAX_PROMPT_BYTES, PROVIDER};
use crate::project::clip::ClipProvenance;
use crate::project::sha256;

/// The version of the request digest's text. A later version names itself,
/// so a project written under this one still knows what its digests meant.
pub const DIGEST_VERSION: &str = "musializer.video-request/v1";

/// A frame image a request carries: the exact PNG bytes, by digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameImage {
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

/// Everything that determines a clip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationRequest {
    pub model: String,
    pub prompt: String,
    pub seconds: u32,
    pub resolution: String,
    pub aspect_ratio: String,
    pub first: Option<FrameImage>,
    pub last: Option<FrameImage>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestError {
    #[error("the prompt is empty")]
    EmptyPrompt,
    #[error("the prompt is longer than {MAX_PROMPT_BYTES} bytes")]
    LongPrompt,
    #[error("the prompt contains a control character other than a line break")]
    ControlInPrompt,
    #[error("{model} makes clips of {lengths}, not {seconds} s")]
    Length {
        model: String,
        lengths: String,
        seconds: u32,
    },
    #[error("{model} does not take a {which} frame")]
    Frame { model: String, which: &'static str },
    #[error("{model} does not list {what}")]
    Shape { model: String, what: String },
}

/// Whether a prompt is one a request may carry.
///
/// # Errors
///
/// Empty, too long, or a control character other than a line break or tab.
pub fn check_prompt(prompt: &str) -> Result<(), RequestError> {
    if prompt.trim().is_empty() {
        return Err(RequestError::EmptyPrompt);
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(RequestError::LongPrompt);
    }
    if prompt
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(RequestError::ControlInPrompt);
    }
    Ok(())
}

impl GenerationRequest {
    /// A request at `shape`, unchecked: [`Self::check`] says whether the
    /// model takes it.
    #[must_use]
    pub fn new(model: &str, prompt: &str, seconds: u32, shape: &Shape) -> Self {
        Self {
            model: model.to_owned(),
            prompt: prompt.to_owned(),
            seconds,
            resolution: shape.resolution.clone(),
            aspect_ratio: shape.aspect_ratio.clone(),
            first: None,
            last: None,
        }
    }

    /// Whether the model, as listed, takes this request. The provider checks
    /// again (a 400 naming the supported values); this is so a refusal costs
    /// nothing and says why before anything is sent.
    ///
    /// # Errors
    ///
    /// The prompt, a length, a frame or a shape the model does not take.
    pub fn check(&self, offer: &Offer) -> Result<(), RequestError> {
        check_prompt(&self.prompt)?;
        if !offer.durations.contains(&self.seconds) {
            return Err(RequestError::Length {
                model: offer.name.clone(),
                lengths: offer.durations_text(),
                seconds: self.seconds,
            });
        }
        if self.first.is_some() && !offer.first_frame {
            return Err(RequestError::Frame {
                model: offer.name.clone(),
                which: "first",
            });
        }
        if self.last.is_some() && !offer.last_frame {
            return Err(RequestError::Frame {
                model: offer.name.clone(),
                which: "last",
            });
        }
        for (listed, value) in [
            (&offer.resolutions, &self.resolution),
            (&offer.aspect_ratios, &self.aspect_ratio),
        ] {
            if !listed.is_empty() && !listed.contains(value) {
                return Err(RequestError::Shape {
                    model: offer.name.clone(),
                    what: value.clone(),
                });
            }
        }
        Ok(())
    }

    /// How many frame images it carries.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        u32::from(self.first.is_some()) + u32::from(self.last.is_some())
    }

    /// The request's identity: SHA-256 of a versioned text naming every field
    /// that determines the clip, the prompt length-prefixed so no prompt can
    /// forge a field. The frames are their bytes' digests: a frame rendered
    /// differently is a different request, a frame rendered the same is not.
    #[must_use]
    pub fn digest(&self) -> String {
        let frame = |image: &Option<FrameImage>| {
            image
                .as_ref()
                .map_or_else(|| "none".to_owned(), |image| image.sha256.clone())
        };
        let text = format!(
            "{DIGEST_VERSION}\nprovider={PROVIDER}\nmodel={}\nprompt={}:{}\nseconds={}\n\
             resolution={}\naspect_ratio={}\naudio=false\nfirst={}\nlast={}\n",
            self.model,
            self.prompt.len(),
            self.prompt,
            self.seconds,
            self.resolution,
            self.aspect_ratio,
            frame(&self.first),
            frame(&self.last),
        );
        sha256::digest_hex(text.as_bytes())
    }

    /// The `POST /api/v1/videos` body, with each frame as the data URL the
    /// caller made of its bytes. No `user`, no `session_id`, no callback:
    /// nothing in it names the person or the project.
    #[must_use]
    pub fn body(&self, first_url: Option<&str>, last_url: Option<&str>) -> Value {
        let mut body = json!({
            "model": self.model,
            "prompt": self.prompt,
            "duration": self.seconds,
            "resolution": self.resolution,
            "aspect_ratio": self.aspect_ratio,
            "generate_audio": false,
        });
        let frames: Vec<Value> = [(first_url, "first_frame"), (last_url, "last_frame")]
            .into_iter()
            .filter_map(|(url, kind)| {
                Some(json!({
                    "type": "image_url",
                    "image_url": { "url": url? },
                    "frame_type": kind,
                }))
            })
            .collect();
        if !frames.is_empty() {
            body["frame_images"] = Value::Array(frames);
        }
        body
    }

    /// What leaves the machine, in words, for the card and the run's report.
    #[must_use]
    pub fn leaves(&self) -> String {
        let frames = match (&self.first, &self.last) {
            (None, None) => String::new(),
            (Some(image), None) | (None, Some(image)) => format!(
                " and one still frame this application rendered ({}x{} PNG)",
                image.width, image.height
            ),
            (Some(image), Some(_)) => format!(
                " and two still frames this application rendered ({}x{} PNG)",
                image.width, image.height
            ),
        };
        format!(
            "The prompt ({} characters){frames} go to OpenRouter, which hands them to the \
             provider of {}. No audio, no file name, nothing about you or this project. \
             OpenRouter keeps the clip for download; video is not eligible for zero data \
             retention.",
            self.prompt.chars().count(),
            self.model
        )
    }

    /// The provenance a clip made from this request carries.
    #[must_use]
    pub fn provenance(
        &self,
        generated_utc: &str,
        estimate_usd: Option<f64>,
        cost_usd: Option<f64>,
    ) -> ClipProvenance {
        ClipProvenance {
            provider: PROVIDER.to_owned(),
            model: self.model.clone(),
            prompt: self.prompt.clone(),
            seconds: self.seconds,
            resolution: self.resolution.clone(),
            aspect_ratio: self.aspect_ratio.clone(),
            first_sha256: self.first.as_ref().map(|image| image.sha256.clone()),
            last_sha256: self.last.as_ref().map(|image| image.sha256.clone()),
            request_sha256: self.digest(),
            generated_utc: generated_utc.to_owned(),
            estimate_usd,
            cost_usd,
        }
    }
}

/// The request a clip's provenance records, back again — for checking that a
/// project's clip still answers a request (the digest is recomputed, never
/// trusted from the file). Frame sizes are not recorded and come back as 0.
#[must_use]
pub fn from_provenance(record: &ClipProvenance) -> GenerationRequest {
    let frame = |digest: &Option<String>| {
        digest.as_ref().map(|digest| FrameImage {
            sha256: digest.clone(),
            width: 0,
            height: 0,
        })
    };
    GenerationRequest {
        model: record.model.clone(),
        prompt: record.prompt.clone(),
        seconds: record.seconds,
        resolution: record.resolution.clone(),
        aspect_ratio: record.aspect_ratio.clone(),
        first: frame(&record.first_sha256),
        last: frame(&record.last_sha256),
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{tests::LIVE, Catalog};
    use super::super::price::shape;
    use super::*;

    fn request() -> GenerationRequest {
        let catalog = Catalog::from_live(LIVE.as_bytes(), 0).unwrap();
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        GenerationRequest::new(
            &veo.id,
            "A slow push through fog",
            4,
            &shape(veo, (1920, 1080)).unwrap(),
        )
    }

    fn image(seed: &[u8]) -> FrameImage {
        FrameImage {
            sha256: sha256::digest_hex(seed),
            width: 1920,
            height: 1080,
        }
    }

    /// The digest is pinned, so a change to what identifies a request is a
    /// deliberate one: every project's generated clips are found by it.
    #[test]
    fn the_digest_names_everything_that_determines_the_clip() {
        let base = request();
        assert_eq!(
            base.digest(),
            sha256::digest_hex(
                b"musializer.video-request/v1\nprovider=openrouter\nmodel=google/veo-3.1-lite\n\
                  prompt=23:A slow push through fog\nseconds=4\nresolution=1080p\n\
                  aspect_ratio=16:9\naudio=false\nfirst=none\nlast=none\n"
            )
        );
        let mut changes: Vec<GenerationRequest> = Vec::new();
        let mut changed = base.clone();
        changed.prompt.push('.');
        changes.push(changed);
        let mut changed = base.clone();
        changed.seconds = 6;
        changes.push(changed);
        let mut changed = base.clone();
        changed.model = "alibaba/wan-3.0".into();
        changes.push(changed);
        let mut changed = base.clone();
        changed.resolution = "720p".into();
        changes.push(changed);
        let mut changed = base.clone();
        changed.first = Some(image(b"a"));
        changes.push(changed.clone());
        changed.first = Some(image(b"b"));
        changes.push(changed.clone());
        changed.last = changed.first.take();
        changes.push(changed);
        let mut digests: Vec<String> = changes.iter().map(GenerationRequest::digest).collect();
        digests.push(base.digest());
        let count = digests.len();
        digests.sort();
        digests.dedup();
        assert_eq!(digests.len(), count, "every change is a new request");
        // A frame's size is not identity; its bytes are.
        let mut small = base.clone();
        small.first = Some(FrameImage {
            width: 2,
            height: 2,
            ..image(b"a")
        });
        let mut large = base.clone();
        large.first = Some(image(b"a"));
        assert_eq!(small.digest(), large.digest());
    }

    #[test]
    fn the_body_asks_for_silence_and_carries_nothing_else() {
        let mut request = request();
        let body = request.body(None, None);
        assert_eq!(body["generate_audio"], false);
        assert_eq!(body["duration"], 4);
        assert!(body.get("frame_images").is_none());
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "aspect_ratio",
                "duration",
                "generate_audio",
                "model",
                "prompt",
                "resolution"
            ]
        );
        request.first = Some(image(b"a"));
        request.last = Some(image(b"b"));
        let body = request.body(
            Some("data:image/png;base64,AA=="),
            Some("data:image/png;base64,AQ=="),
        );
        let frames = body["frame_images"].as_array().unwrap();
        assert_eq!(frames[0]["frame_type"], "first_frame");
        assert_eq!(frames[1]["image_url"]["url"], "data:image/png;base64,AQ==");
        assert!(request.leaves().contains("two still frames"));
        assert!(request.leaves().contains("23 characters"));
    }

    #[test]
    fn a_request_the_model_does_not_take_is_refused_by_name() {
        let catalog = Catalog::from_live(LIVE.as_bytes(), 0).unwrap();
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        let mut request = request();
        assert_eq!(request.check(veo), Ok(()));
        request.seconds = 5;
        assert_eq!(
            request.check(veo).unwrap_err().to_string(),
            "Google: Veo 3.1 Lite makes clips of 4, 6, 8 s, not 5 s"
        );
        request.seconds = 4;
        request.prompt = " \n ".into();
        assert_eq!(request.check(veo), Err(RequestError::EmptyPrompt));
        request.prompt = "a\u{0}b".into();
        assert_eq!(request.check(veo), Err(RequestError::ControlInPrompt));
        request.prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
        assert_eq!(request.check(veo), Err(RequestError::LongPrompt));
        request.prompt = "line one\nline two".into();
        let mut text_only = veo.clone();
        text_only.last_frame = false;
        request.last = Some(image(b"b"));
        assert!(request
            .check(&text_only)
            .unwrap_err()
            .to_string()
            .contains("last frame"));
    }

    #[test]
    fn provenance_gives_back_the_request_it_came_from() {
        let mut request = request();
        request.first = Some(image(b"a"));
        let record = request.provenance("2026-11-14T12:00:00Z", Some(0.2), None);
        assert_eq!(record.request_sha256, request.digest());
        assert_eq!(from_provenance(&record).digest(), request.digest());
        assert!(record.is_valid(), "{record:?}");
    }
}
