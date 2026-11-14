//! The video models OpenRouter lists, what each accepts, and what it costs.
//!
//! Read from `GET https://openrouter.ai/api/v1/videos/models`, public and
//! keyless (the ordinary `/models` list prices video models at zero; the real
//! prices are only here). The response is untrusted display data (E6 in
//! `docs/ASSIST_PROVIDER_CONTRACTS.md`): it is normalized to a bounded
//! allowlist of fields before it is cached, no string from it becomes a path,
//! and a model id is only ever compared against [`super::PICKS`] or sent back
//! as the `model` of a request.
//!
//! The cache is [`Catalog`], written by the application under
//! `$XDG_CACHE_HOME/musializer/video-models.json` with the time it was read;
//! [`Catalog::age_days`] against [`STALE_AFTER_DAYS`] is what makes a price
//! "unknown" rather than old.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The cache's schema token.
pub const SCHEMA: &str = "musializer.video-models/v1";

/// A price list older than this is not a price: video prices moved monthly
/// through 2026 (`docs/VIDEO_GENERATION.md`), and a week is the longest a
/// number shown on a button should be trusted without a refresh.
pub const STALE_AFTER_DAYS: i64 = 7;

/// The largest response or cache accepted, before parsing. The live list was
/// 30 KB for 32 models on 2026-11-14.
pub const MAX_BYTES: usize = 1024 * 1024;

/// The most models kept.
pub const MAX_MODELS: usize = 256;

const MAX_ID: usize = 128;
const MAX_NAME: usize = 128;
const MAX_LIST: usize = 64;
const MAX_TOKEN: usize = 32;
const MAX_SKUS: usize = 64;

/// What one model accepts and costs, as the list said.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub id: String,
    pub name: String,
    /// Seconds a clip may last, ascending.
    pub durations: Vec<u32>,
    /// Resolution tiers: `480p`, `720p`, `1080p`, `2K`, `4K`.
    pub resolutions: Vec<String>,
    /// `16:9`, `9:16`, `1:1`, …
    pub aspect_ratios: Vec<String>,
    /// Exact sizes, when the model lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<(u32, u32)>,
    pub first_frame: bool,
    pub last_frame: bool,
    /// The model can make sound; a request then says `generate_audio: false`
    /// (the track is the music, and silence is cheaper where it is priced).
    pub audio: bool,
    /// The pricing SKUs as numbers, keyed as the list keys them. Units mix:
    /// `cents_per_*` is cents per second, `duration_seconds*` dollars per
    /// second, `video_tokens*` dollars per token (`super::price`).
    pub skus: BTreeMap<String, f64>,
}

/// The cached list, with when it was read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: String,
    /// Seconds since the Unix epoch, UTC, when the list was fetched.
    pub fetched_unix: i64,
    pub models: Vec<Offer>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("the price list is larger than {MAX_BYTES} bytes")]
    TooLarge,
    #[error("the price list is not the JSON OpenRouter documents ({0})")]
    Format(String),
    #[error("the cached price list declares schema {0:?}, not {SCHEMA}")]
    Schema(String),
}

fn bounded_string(value: Option<&Value>, max: usize) -> Option<String> {
    let text = value?.as_str()?;
    (!text.is_empty() && text.len() <= max && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_LIST)
                .filter_map(|item| bounded_string(Some(item), MAX_TOKEN))
                .collect()
        })
        .unwrap_or_default()
}

/// `"1920x1080"` → `(1920, 1080)`, bounded like a clip's own size.
#[must_use]
pub fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let size = (width.parse().ok()?, height.parse().ok()?);
    crate::project::clip::size_is_valid(size.0, size.1).then_some(size)
}

/// A SKU's value: the list writes numbers as strings (`"0.05"`), and either
/// form is read. Negative, non-finite or absurd values are dropped, never
/// clamped: a price that is wrong must read as no price.
fn sku_value(value: &Value) -> Option<f64> {
    let number = match value {
        Value::String(text) if text.len() <= MAX_TOKEN => text.trim().parse::<f64>().ok()?,
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    (number.is_finite() && (0.0..1.0e4).contains(&number)).then_some(number)
}

impl Offer {
    /// One entry of the live list, or `None` when it is not one this
    /// application can ask for (no id, no duration, nothing it can be sent).
    fn from_live(model: &Value) -> Option<Self> {
        let id = bounded_string(model.get("id"), MAX_ID)?;
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | ':'))
        {
            return None;
        }
        let name = bounded_string(model.get("name"), MAX_NAME).unwrap_or_else(|| id.clone());
        let mut durations: Vec<u32> = model
            .get("supported_durations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(MAX_LIST)
                    .filter_map(Value::as_u64)
                    .filter(|seconds| (1..=u64::from(super::MAX_SECONDS) * 4).contains(seconds))
                    .map(|seconds| seconds as u32)
                    .collect()
            })
            .unwrap_or_default();
        durations.sort_unstable();
        durations.dedup();
        if durations.is_empty() {
            return None;
        }
        let frames = string_list(model.get("supported_frame_images"));
        let skus = model
            .get("pricing_skus")
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .filter(|(key, _)| {
                        key.len() <= MAX_TOKEN * 2
                            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
                    .take(MAX_SKUS)
                    .filter_map(|(key, value)| Some((key.clone(), sku_value(value)?)))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id,
            name,
            durations,
            resolutions: string_list(model.get("supported_resolutions")),
            aspect_ratios: string_list(model.get("supported_aspect_ratios")),
            sizes: string_list(model.get("supported_sizes"))
                .iter()
                .filter_map(|text| parse_size(text))
                .collect(),
            first_frame: frames.iter().any(|frame| frame == "first_frame"),
            last_frame: frames.iter().any(|frame| frame == "last_frame"),
            audio: model.get("generate_audio").and_then(Value::as_bool) == Some(true),
            skus,
        })
    }

    /// The durations, as a reader would say them: `4, 6, 8 s` or `2–30 s`.
    #[must_use]
    pub fn durations_text(&self) -> String {
        let contiguous = self.durations.windows(2).all(|pair| pair[1] == pair[0] + 1);
        match (self.durations.first(), self.durations.last()) {
            (Some(first), Some(last)) if contiguous && self.durations.len() > 2 => {
                format!("{first}\u{2013}{last} s")
            }
            _ => format!(
                "{} s",
                self.durations
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// What images it takes, in words.
    #[must_use]
    pub fn frames_text(&self) -> &'static str {
        match (self.first_frame, self.last_frame) {
            (true, true) => "first and last frame",
            (true, false) => "first frame only",
            (false, true) => "last frame only",
            (false, false) => "text only",
        }
    }
}

impl Catalog {
    /// Normalizes the live response.
    ///
    /// # Errors
    ///
    /// A response past [`MAX_BYTES`], or one that is not `{ "data": [ … ] }`.
    pub fn from_live(bytes: &[u8], fetched_unix: i64) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let document: Value = serde_json::from_slice(bytes)
            .map_err(|error| CatalogError::Format(error.to_string()))?;
        let data = document
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| CatalogError::Format("no data array".into()))?;
        let mut models: Vec<Offer> = data
            .iter()
            .take(MAX_MODELS)
            .filter_map(Offer::from_live)
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|a, b| a.id == b.id);
        Ok(Self {
            schema: SCHEMA.to_owned(),
            fetched_unix,
            models,
        })
    }

    /// Reads the cache.
    ///
    /// # Errors
    ///
    /// Too large, not this schema, or a field this schema does not have.
    pub fn from_cache(bytes: &[u8]) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let catalog: Self = serde_json::from_slice(bytes)
            .map_err(|error| CatalogError::Format(error.to_string()))?;
        if catalog.schema != SCHEMA {
            return Err(CatalogError::Schema(catalog.schema));
        }
        if catalog.models.len() > MAX_MODELS {
            return Err(CatalogError::TooLarge);
        }
        Ok(catalog)
    }

    /// The cache's bytes.
    #[must_use]
    pub fn to_cache(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    #[must_use]
    pub fn offer(&self, id: &str) -> Option<&Offer> {
        self.models.iter().find(|offer| offer.id == id)
    }

    /// Whole days since the list was read; a list from the future counts as
    /// read now.
    #[must_use]
    pub fn age_days(&self, now_unix: i64) -> i64 {
        (now_unix - self.fetched_unix).max(0) / 86_400
    }

    #[must_use]
    pub fn is_stale(&self, now_unix: i64) -> bool {
        self.age_days(now_unix) >= STALE_AFTER_DAYS
    }

    /// The date it was read, `YYYY-MM-DD` UTC, for a reader.
    #[must_use]
    pub fn fetched_date(&self) -> String {
        utc_date(self.fetched_unix)
    }
}

/// `YYYY-MM-DD` of a Unix time, UTC (days-from-civil inverted; no clock read).
#[must_use]
pub fn utc_date(unix: i64) -> String {
    let (year, month, day) = civil_from_days(unix.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// RFC 3339 of a Unix time, UTC, to the second.
#[must_use]
pub fn utc_timestamp(unix: i64) -> String {
    let seconds = unix.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        utc_date(unix),
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The two-model list the tests read, for the application's tests too.
#[doc(hidden)]
#[must_use]
pub fn tests_live() -> &'static str {
    tests_fixture::LIVE
}

#[doc(hidden)]
mod tests_fixture {
    /// Two entries of the live list as it was on 2026-11-14, verbatim in the
    /// fields this module reads, plus two it must drop.
    pub const LIVE: &str = r#"{ "data": [
      { "id": "google/veo-3.1-lite", "name": "Google: Veo 3.1 Lite",
        "supported_resolutions": ["720p","1080p"], "supported_aspect_ratios": ["16:9","9:16"],
        "supported_sizes": ["1280x720","720x1280","1920x1080","1080x1920"],
        "supported_durations": [8,4,6], "supported_frame_images": ["first_frame","last_frame"],
        "generate_audio": true, "seed": true,
        "pricing_skus": { "duration_seconds_with_audio": "0.08", "duration_seconds_without_audio": "0.05",
                          "duration_seconds_with_audio_720p": "0.05", "duration_seconds_without_audio_720p": "0.03" },
        "allowed_passthrough_parameters": ["personGeneration"] },
      { "id": "bytedance/seedance-2.0", "name": "ByteDance: Seedance 2.0",
        "supported_resolutions": ["480p","720p","1080p","4K"], "supported_aspect_ratios": ["16:9","1:1"],
        "supported_sizes": null, "supported_durations": [4,5,6,7,8,9,10,11,12,13,14,15],
        "supported_frame_images": ["first_frame","last_frame"], "generate_audio": true,
        "pricing_skus": { "video_tokens": "0.000007", "video_tokens_1080p": "0.0000077",
                          "video_tokens_without_audio": "0.000007" } },
      { "id": "", "supported_durations": [5] },
      { "id": "example/upscaler", "supported_durations": [] }
    ] }"#;
}

#[cfg(test)]
pub(crate) mod tests {
    pub use super::tests_fixture::LIVE;
    use super::*;

    #[test]
    fn the_live_list_is_normalized_to_what_a_request_needs() {
        let catalog = Catalog::from_live(LIVE.as_bytes(), 1_794_700_000).unwrap();
        assert_eq!(
            catalog.models.len(),
            2,
            "an entry with no id or no duration is dropped"
        );
        let veo = catalog.offer("google/veo-3.1-lite").unwrap();
        assert_eq!(veo.durations, [4, 6, 8], "sorted");
        assert_eq!(veo.sizes[2], (1920, 1080));
        assert!(veo.first_frame && veo.last_frame && veo.audio);
        assert_eq!(veo.skus["duration_seconds_without_audio_720p"], 0.03);
        assert_eq!(veo.durations_text(), "4, 6, 8 s");
        let seedance = catalog.offer("bytedance/seedance-2.0").unwrap();
        assert_eq!(seedance.durations_text(), "4\u{2013}15 s");
        assert!(seedance.sizes.is_empty());
        // The cache reads back to the same thing, and refuses another schema.
        let again = Catalog::from_cache(catalog.to_cache().as_bytes()).unwrap();
        assert_eq!(again, catalog);
        let other = catalog
            .to_cache()
            .replace(SCHEMA, "musializer.video-models/v0");
        assert!(matches!(
            Catalog::from_cache(other.as_bytes()),
            Err(CatalogError::Schema(_))
        ));
    }

    #[test]
    fn a_hostile_list_is_refused_or_dropped_not_believed() {
        assert_eq!(
            Catalog::from_live(&vec![b' '; MAX_BYTES + 1], 0),
            Err(CatalogError::TooLarge)
        );
        assert!(Catalog::from_live(b"{\"models\":[]}", 0).is_err());
        let catalog = Catalog::from_live(
            br#"{"data":[{"id":"a/../b; rm","supported_durations":[4]},
                         {"id":"ok/m","supported_durations":[4],
                          "pricing_skus":{"duration_seconds":"-1","cents_per_second_output":"NaN",
                                          "duration_seconds_1080p":"0.2"}}]}"#,
            0,
        )
        .unwrap();
        assert_eq!(
            catalog.models.len(),
            1,
            "an id that is not a slug is dropped"
        );
        let skus = &catalog.models[0].skus;
        assert_eq!(
            skus.len(),
            1,
            "a negative or non-number price is no price: {skus:?}"
        );
    }

    #[test]
    fn age_and_dates_are_arithmetic_on_the_recorded_time() {
        let catalog = Catalog::from_live(LIVE.as_bytes(), 1_794_700_000).unwrap();
        assert_eq!(catalog.fetched_date(), "2026-11-14");
        assert_eq!(utc_timestamp(1_794_700_000), "2026-11-14T23:46:40Z");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert!(!catalog.is_stale(1_794_700_000 + 6 * 86_400 + 86_399));
        assert!(catalog.is_stale(1_794_700_000 + 7 * 86_400));
        assert_eq!(catalog.age_days(1_794_700_000 - 99), 0);
    }
}
