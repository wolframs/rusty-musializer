//! The filesystem half of the shared tuning-preset store.
//!
//! **Owner: Agent L.** Port of `preset_store_load`, `ensure_parent_directories`
//! and `preset_store_save` (`../musializer/src/preset_store.c:151-256`), plus
//! the `getenv` half of `preset_store_default_path` (`:28-51`).
//!
//! [`musializer_core::project::preset_store`]'s module comment says exactly this
//! much is deliberately absent from the core crate: it opens files, creates
//! directories and reads the environment, so it lives here. Everything it
//! decides — what a store document contains, which scene a token names, where the
//! path is *given* an environment — stays pure over there and is called from
//! here, so there is one definition of each.
//!
//! # The rule that shapes both functions
//!
//! **A store that cannot be read is not an empty store.** A missing file is the
//! ordinary first-run state and yields `Ok(None)`; anything else is an error, and
//! a caller that treats an error as "start fresh" would overwrite a file the user
//! could still have fixed. The C says the same thing in `preset_store.h:41-46` and
//! enforces it with `p->preset_store_ready`, which the application here mirrors
//! by refusing every mutation while the store is unreadable.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use musializer_core::project::preset_store::{
    self, PathEnvironment, PresetLibrary, PresetStoreError, MAX_FILE_SIZE,
};

use crate::process::publish;

/// `preset_store_default_path` (`preset_store.c:28-51`), with the environment
/// read here instead of inside the policy.
///
/// `None` when neither `MUSIALIZER_PRESET_STORE`, `XDG_DATA_HOME` nor `HOME` says
/// anything usable — in which case the application runs with no shared library at
/// all rather than inventing a location.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let override_path = std::env::var("MUSIALIZER_PRESET_STORE").ok();
    let data_home = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    preset_store::default_path(&PathEnvironment {
        preset_store_override: override_path.as_deref(),
        xdg_data_home: data_home.as_deref(),
        home: home.as_deref(),
    })
    .map(PathBuf::from)
}

/// `preset_store_load` (`preset_store.c:151-194`).
///
/// `Ok(None)` is the C's `PRESET_STORE_MISSING`: the file does not exist, which
/// is the normal first run and not a failure. Every other outcome is an error and
/// **leaves the file untouched**.
///
/// A file at exactly the size ceiling with more bytes behind it is a format
/// error, not a truncated read, which is why one extra byte is asked for rather
/// than trusting the length: a store the reader silently truncated would then be
/// re-serialized short and the tail lost.
pub fn load(path: &Path) -> Result<Option<PresetLibrary>, PresetStoreError> {
    if path.as_os_str().is_empty() {
        return Err(PresetStoreError::Argument);
    }
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(PresetStoreError::Read),
    };
    let mut data = Vec::new();
    file.by_ref()
        .take(MAX_FILE_SIZE as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| PresetStoreError::Read)?;
    if data.len() > MAX_FILE_SIZE {
        return Err(PresetStoreError::Format);
    }
    preset_store::load_from_bytes(&data).map(Some)
}

/// `preset_store_save` (`preset_store.c:218-256`).
///
/// Serializes **before** touching the destination, so a library that cannot be
/// represented fails without having replaced anything, then publishes through the
/// same transactional rename `.musi` saves use.
pub fn save(path: &Path, library: &PresetLibrary) -> Result<(), PresetStoreError> {
    if path.as_os_str().is_empty() {
        return Err(PresetStoreError::Argument);
    }
    let encoded = preset_store::save_to_string(library)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|_| PresetStoreError::Directory)?;
        }
    }
    publish::atomic_write(path, encoded.as_bytes()).map_err(|_| PresetStoreError::Write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::scene::settings::{SceneSettings, SettingsSnapshot};
    use musializer_core::scene::SceneId;
    use std::fs;

    /// A private directory under `build/`, which is gitignored.
    ///
    /// Every test here points the store somewhere under it. The operator's real
    /// library lives under `$XDG_DATA_HOME`, and a test that wrote there would
    /// destroy work this repository has no business touching.
    fn scratch(name: &str) -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-preset-files")
            .join(name);
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("scratch directory");
        directory.canonicalize().expect("scratch is real")
    }

    fn snapshot(scene: SceneId, value: f32) -> SettingsSnapshot {
        let mut settings = SceneSettings::new();
        settings.set(scene, 0, value);
        settings.capture(scene).expect("a capturable scene")
    }

    fn library() -> PresetLibrary {
        let mut library = PresetLibrary::new();
        library
            .push(SceneId::Loom, "warm", &snapshot(SceneId::Loom, 0.5))
            .expect("room for one");
        library
            .push(
                SceneId::Spectrum,
                "bright",
                &snapshot(SceneId::Spectrum, 0.25),
            )
            .expect("room for one");
        library
    }

    #[test]
    fn a_missing_store_is_the_first_run_not_a_failure() {
        let directory = scratch("missing");
        assert_eq!(load(&directory.join("presets.json")), Ok(None));
    }

    #[test]
    fn a_saved_store_round_trips_through_the_file() {
        let directory = scratch("round-trip");
        let path = directory.join("nested/presets.json");
        let written = library();
        save(&path, &written).expect("save");
        assert!(path.exists(), "parent directories are created");
        let read = load(&path).expect("load").expect("present");
        assert_eq!(read, written);
    }

    #[test]
    fn an_unchanged_library_serializes_to_identical_bytes() {
        // Determinism is why `document_from_library` walks the scenes in registry
        // order: a store that churns would show up as a spurious change in a
        // backup or a sync.
        let directory = scratch("stable");
        let first = directory.join("a.json");
        let second = directory.join("b.json");
        save(&first, &library()).expect("save");
        save(&second, &library()).expect("save");
        assert_eq!(
            fs::read(&first).expect("read"),
            fs::read(&second).expect("read")
        );
    }

    #[test]
    fn a_corrupt_store_is_an_error_and_the_file_survives_it() {
        let directory = scratch("corrupt");
        let path = directory.join("presets.json");
        fs::write(&path, b"{ not json").expect("write");
        assert_eq!(load(&path), Err(PresetStoreError::Format));
        assert_eq!(
            fs::read(&path).expect("read"),
            b"{ not json",
            "a rejected store must be left exactly as it was"
        );
    }

    #[test]
    fn an_empty_store_file_is_a_format_error_not_an_empty_library() {
        // Truncation is indistinguishable from emptiness, and treating it as
        // "no presets" would let the next save destroy recoverable data.
        let directory = scratch("empty");
        let path = directory.join("presets.json");
        fs::write(&path, b"").expect("write");
        assert_eq!(load(&path), Err(PresetStoreError::Format));
    }

    #[test]
    fn a_store_past_the_size_ceiling_is_refused_rather_than_truncated() {
        let directory = scratch("oversize");
        let path = directory.join("presets.json");
        fs::write(&path, vec![b' '; MAX_FILE_SIZE + 1]).expect("write");
        assert_eq!(load(&path), Err(PresetStoreError::Format));
    }
}
