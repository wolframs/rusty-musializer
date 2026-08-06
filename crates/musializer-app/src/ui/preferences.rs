//! Per-user shell scale and split preferences.
//!
//! These values describe the operator's workstation, not a music project, so
//! they deliberately live outside `.musi`. A corrupt file is an error rather
//! than an empty preference set; the application can keep running with defaults
//! but must refuse to overwrite evidence the user may be able to repair.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::scale::UiScalePreference;

const SCHEMA: &str = "musializer.ui-preferences/v1";
const MAX_FILE_SIZE: u64 = 64 * 1024;

/// What a persisted lyric cue lane height may be (LX1-d).
///
/// Deliberately wider than the panel's own 33..66 clamp and checked separately
/// from the split widths: a lane is tens of pixels, not hundreds, so the shared
/// `80.0..=4096.0` bound would reject every value this field can legitimately
/// hold. The panel clamps again against the window it is drawn in, so this range
/// only has to exclude values that are not a lane at all.
const LANE_HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 8.0..=256.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiPreferences {
    pub scale: UiScalePreference,
    pub sidebar_width: Option<f32>,
    pub inspector_width: Option<f32>,
    pub timeline_height: Option<f32>,
    /// The lyric editor's cue lane height, dragged by its bottom edge (LX1-d).
    ///
    /// Here rather than in the `.musi` file for this module's stated reason: it
    /// describes the operator's screen, not the music. A resize the user has to
    /// redo every launch is the friction the resize was asked for to remove.
    pub lyric_lane_height: Option<f32>,
}

impl UiPreferences {
    #[must_use]
    pub fn sane(self) -> bool {
        [
            self.sidebar_width,
            self.inspector_width,
            self.timeline_height,
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && (80.0..=4096.0).contains(&value))
            // `into_iter().all` rather than `is_none_or`, which is stable only
            // since 1.82 and this workspace's MSRV is 1.80.
            && self
                .lyric_lane_height
                .into_iter()
                .all(|value| value.is_finite() && LANE_HEIGHT_RANGE.contains(&value))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UiPreferencesError {
    #[error("could not read the preference file")]
    Read,
    #[error("the preference file has an invalid format")]
    Format,
    #[error("could not create the preference directory")]
    Directory,
    #[error("could not write the preference file")]
    Write,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    scale: String,
    sidebar_width: Option<f32>,
    inspector_width: Option<f32>,
    timeline_height: Option<f32>,
    /// `default` rather than required, and that is the compatibility contract
    /// rather than a convenience: every `ui.json` this application has already
    /// written carries exactly the four fields above, and a fifth required one
    /// would make all of them fail to parse — which this module treats as
    /// corruption and refuses to overwrite, so the user would lose their splits
    /// *and* be told the file is broken.
    #[serde(default)]
    lyric_lane_height: Option<f32>,
}

#[must_use]
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSIALIZER_UI_PREFERENCES") {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer/ui.json"));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".config/musializer/ui.json"))
}

pub fn load(path: &Path) -> Result<Option<UiPreferences>, UiPreferencesError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UiPreferencesError::Read),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UiPreferencesError::Read)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(UiPreferencesError::Format);
    }
    let document: Document =
        serde_json::from_slice(&bytes).map_err(|_| UiPreferencesError::Format)?;
    if document.schema != SCHEMA {
        return Err(UiPreferencesError::Format);
    }
    let scale = UiScalePreference::parse(&document.scale).ok_or(UiPreferencesError::Format)?;
    let preferences = UiPreferences {
        scale,
        sidebar_width: document.sidebar_width,
        inspector_width: document.inspector_width,
        timeline_height: document.timeline_height,
        lyric_lane_height: document.lyric_lane_height,
    };
    preferences
        .sane()
        .then_some(Some(preferences))
        .ok_or(UiPreferencesError::Format)
}

pub fn save(path: &Path, preferences: UiPreferences) -> Result<(), UiPreferencesError> {
    if !preferences.sane() {
        return Err(UiPreferencesError::Format);
    }
    let document = Document {
        schema: SCHEMA.to_string(),
        scale: match preferences.scale {
            UiScalePreference::Auto => "auto".to_string(),
            UiScalePreference::Fixed(scale) => scale.percent().to_string(),
        },
        sidebar_width: preferences.sidebar_width,
        inspector_width: preferences.inspector_width,
        timeline_height: preferences.timeline_height,
        lyric_lane_height: preferences.lyric_lane_height,
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|_| UiPreferencesError::Format)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| UiPreferencesError::Directory)?;
    }
    musializer_runtime::process::publish::atomic_write(path, &bytes)
        .map_err(|_| UiPreferencesError::Write)
}

#[cfg(test)]
mod tests {
    use super::super::scale::UI_SCALE_STEPS;
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-ui-preferences")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn preferences_round_trip_without_entering_a_project() {
        let path = scratch("round-trip").join("nested/ui.json");
        let preferences = UiPreferences {
            scale: UiScalePreference::parse("150").unwrap(),
            sidebar_width: Some(384.0),
            inspector_width: Some(420.0),
            timeline_height: Some(460.0),
            lyric_lane_height: Some(48.0),
        };
        save(&path, preferences).unwrap();
        assert_eq!(load(&path).unwrap(), Some(preferences));
    }

    #[test]
    fn a_file_written_before_the_lane_was_resizable_still_opens() {
        // LX1-d's compatibility contract, stated as a test rather than as a
        // comment: this is the exact document a build before the resize wrote.
        let path = scratch("pre-lane").join("ui.json");
        std::fs::write(
            &path,
            br#"{"schema":"musializer.ui-preferences/v1","scale":"auto",
                 "sidebar_width":300.0,"inspector_width":null,"timeline_height":null}"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap().expect("an older file still parses");
        assert_eq!(loaded.sidebar_width, Some(300.0));
        assert_eq!(loaded.lyric_lane_height, None);
    }

    #[test]
    fn a_lane_height_outside_its_own_range_is_refused_rather_than_clamped() {
        // The split widths' 80 px floor must not be applied to a lane: 33 is the
        // lane's *default*, and a shared bound would reject every honest value.
        let path = scratch("lane-range").join("ui.json");
        for accepted in [8.0f32, 33.0, 66.0, 256.0] {
            let preferences = UiPreferences {
                lyric_lane_height: Some(accepted),
                ..UiPreferences::default()
            };
            assert!(preferences.sane(), "{accepted} is a lane height");
            save(&path, preferences).unwrap();
            assert_eq!(load(&path).unwrap(), Some(preferences));
        }
        for refused in [0.0f32, -33.0, 257.0, f32::NAN] {
            let preferences = UiPreferences {
                lyric_lane_height: Some(refused),
                ..UiPreferences::default()
            };
            assert!(!preferences.sane(), "{refused} is not a lane height");
            assert_eq!(save(&path, preferences), Err(UiPreferencesError::Format));
        }
    }

    #[test]
    fn corrupt_preferences_are_not_treated_as_an_empty_store() {
        let path = scratch("corrupt").join("ui.json");
        std::fs::write(&path, b"{ broken").unwrap();
        assert_eq!(load(&path), Err(UiPreferencesError::Format));
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
    }

    #[test]
    fn every_supported_scale_serializes() {
        for scale in UI_SCALE_STEPS {
            let preferences = UiPreferences {
                scale: UiScalePreference::Fixed(super::super::scale::UiScale::new(scale).unwrap()),
                ..UiPreferences::default()
            };
            let path = scratch(&format!("scale-{scale}")).join("ui.json");
            save(&path, preferences).unwrap();
            assert_eq!(load(&path).unwrap(), Some(preferences));
        }
    }
}
