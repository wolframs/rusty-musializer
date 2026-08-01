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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiPreferences {
    pub scale: UiScalePreference,
    pub sidebar_width: Option<f32>,
    pub inspector_width: Option<f32>,
    pub timeline_height: Option<f32>,
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
        };
        save(&path, preferences).unwrap();
        assert_eq!(load(&path).unwrap(), Some(preferences));
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
