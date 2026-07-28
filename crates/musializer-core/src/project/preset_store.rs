//! The per-user scene tuning preset library.
//!
//! **Owner: Agent B.** Port of `../musializer/src/preset_store.c/.h`.
//!
//! One strict-JSON file in the per-user data directory, edited by the Tune
//! inspector and durable across every track and project. Track-local presets inside
//! `.musi` files remain valid project data and are **copied, never moved**, into
//! this store on open — which is why [`merge`] exists and why nothing here deletes
//! from a project.
//!
//! Two boundaries worth stating, because both were choices:
//!
//! - The filesystem half of `preset_store_load`/`_save` is not here. It opens files
//!   and creates directories, so it belongs to `musializer-runtime`. What is here is
//!   the pure half — [`load_from_bytes`], [`save_to_string`], and
//!   [`default_path`], which takes its environment as an argument instead of
//!   reading it, so the path policy is testable and this crate stays free of global
//!   state.
//! - [`PresetLibrary`] is the runtime shape of `Scene_Settings_Preset_Library`
//!   (`scene_settings.h:75-80`). It belongs in [`crate::scene::settings`] with the
//!   rest of that contract, and it lives here only because that module is owned by
//!   the integration owner and had no preset library yet. **Move it when
//!   convenient**; this module should then just convert between it and the file.

use crate::project::io::{
    preset_store_deserialize, preset_store_serialize, PresetStoreDocument, ProjectIoError,
};
use crate::project::model::{capacity, ScenePreset};
use crate::scene::settings::{self, SettingsSnapshot, PRESETS_PER_SCENE, PRESET_NAME_CAPACITY};
use crate::scene::{SceneId, SCENE_COUNT};

/// Largest store file the reader will look at (`PRESET_STORE_MAX_FILE_SIZE`,
/// `preset_store.c:18`): 1 MiB. Smaller than the `.musi` ceiling because a preset
/// store is a list of short records and nothing else.
pub const MAX_FILE_SIZE: usize = 1024 * 1024;

/// Maximum preset name length in **bytes** (`scene_settings.h:11`, capacity minus
/// its NUL).
pub const NAME_MAX_BYTES: usize = PRESET_NAME_CAPACITY - 1;

/// What happened to a preset store operation (`Preset_Store_Result`,
/// `preset_store.h:14-23`).
///
/// `Missing` is not a failure: a store that does not exist yet is the normal
/// first-run state. Every *other* failure leaves the library empty **and the file
/// untouched** — callers must keep the store read-only until the user resolves it
/// rather than overwriting recoverable data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PresetStoreError {
    #[error("invalid preset store request")]
    Argument,
    #[error("preset store path unavailable")]
    Path,
    #[error("preset store could not be read")]
    Read,
    #[error("preset store contents were rejected")]
    Format,
    #[error("preset store directory could not be created")]
    Directory,
    #[error("preset store could not be written")]
    Write,
}

/// One saved tuning preset (`Scene_Settings_Preset`, `scene_settings.h:69-73`).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub snapshot: SettingsSnapshot,
}

/// Every scene's presets, with one shared id allocator
/// (`Scene_Settings_Preset_Library`, `scene_settings.h:75-80`).
///
/// The ids are unique across the **whole** library, not per scene, so a preset can
/// be referred to without also naming its scene.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetLibrary {
    next_id: u64,
    scenes: [Vec<Preset>; SCENE_COUNT],
}

impl Default for PresetLibrary {
    /// `scene_settings_preset_library_init` (`scene_settings.c:284-289`).
    fn default() -> Self {
        Self {
            next_id: 1,
            scenes: core::array::from_fn(|_| Vec::new()),
        }
    }
}

impl PresetLibrary {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    #[must_use]
    pub fn presets(&self, scene: SceneId) -> &[Preset] {
        &self.scenes[scene.index()]
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.scenes.iter().map(Vec::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// `scene_settings_preset_library_valid` (`scene_settings.c:291-315`).
    ///
    /// Note `id < next_id` again: an id at or above the allocator's cursor could be
    /// handed out a second time, and two presets sharing an id is how a rename
    /// silently overwrites the wrong one.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.next_id == 0 {
            return false;
        }
        let mut seen: Vec<u64> = Vec::with_capacity(self.total());
        for scene in SceneId::ALL {
            let presets = &self.scenes[scene.index()];
            if presets.len() > PRESETS_PER_SCENE {
                return false;
            }
            for preset in presets {
                if preset.id == 0
                    || preset.id >= self.next_id
                    || preset.name.is_empty()
                    || preset.name.len() > NAME_MAX_BYTES
                    || !preset.snapshot.is_valid_for(scene)
                    || !preset.snapshot.captured
                    || seen.contains(&preset.id)
                {
                    return false;
                }
                seen.push(preset.id);
            }
        }
        true
    }

    /// Adds a preset with a freshly allocated id
    /// (`scene_settings_preset_save`, `scene_settings.c:317-339`).
    ///
    /// `None` when the scene is full, the name is out of bounds, or the snapshot is
    /// not valid for the scene. Returns the new preset's index within its scene.
    pub fn push(
        &mut self,
        scene: SceneId,
        name: &str,
        snapshot: &SettingsSnapshot,
    ) -> Option<usize> {
        if name.is_empty() || name.len() > NAME_MAX_BYTES {
            return None;
        }
        if !snapshot.captured || !snapshot.is_valid_for(scene) {
            return None;
        }
        if self.scenes[scene.index()].len() >= PRESETS_PER_SCENE {
            return None;
        }
        if !self.is_valid() {
            return None;
        }
        let index = self.scenes[scene.index()].len();
        self.scenes[scene.index()].push(Preset {
            id: self.next_id,
            name: name.to_owned(),
            snapshot: *snapshot,
        });
        self.next_id += 1;
        Some(index)
    }

    /// Adds a preset with an id that already exists, from a file
    /// (`plug.c:4776-4801` for `.musi`, `library_from_document` for the store).
    ///
    /// Distinct from [`Self::push`], which *allocates* an id: hydration must
    /// preserve the identifiers the file recorded, or a project's cue references
    /// would point at different presets after a round trip. The allocator is
    /// advanced past every restored id so a later `push` cannot collide.
    ///
    /// `u64::MAX` is refused because advancing past it would wrap to zero.
    pub fn restore(
        &mut self,
        scene: SceneId,
        id: u64,
        name: &str,
        snapshot: &SettingsSnapshot,
    ) -> bool {
        if id == 0 || id == u64::MAX || name.is_empty() || name.len() > NAME_MAX_BYTES {
            return false;
        }
        if !snapshot.captured || !snapshot.is_valid_for(scene) {
            return false;
        }
        if self.scenes[scene.index()].len() >= PRESETS_PER_SCENE {
            return false;
        }
        if self.scenes.iter().flatten().any(|preset| preset.id == id) {
            return false;
        }
        self.scenes[scene.index()].push(Preset {
            id,
            name: name.to_owned(),
            snapshot: *snapshot,
        });
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        true
    }

    /// Replaces one preset's captured values, keeping its id and name
    /// (`scene_settings_preset_replace`, `scene_settings.c:341-353`).
    pub fn replace_snapshot(
        &mut self,
        scene: SceneId,
        index: usize,
        snapshot: &SettingsSnapshot,
    ) -> bool {
        if index >= self.scenes[scene.index()].len()
            || !snapshot.captured
            || !snapshot.is_valid_for(scene)
        {
            return false;
        }
        self.scenes[scene.index()][index].snapshot = *snapshot;
        true
    }

    /// `scene_settings_preset_remove` (`scene_settings.c:355+`).
    ///
    /// Does **not** lower `next_id`: an id is never reused, so a removed preset's id
    /// cannot come back attached to different values.
    pub fn remove(&mut self, scene: SceneId, index: usize) -> bool {
        if index >= self.scenes[scene.index()].len() {
            return false;
        }
        self.scenes[scene.index()].remove(index);
        true
    }
}

/// The stable scene token used in the store file
/// (`preset_store_scene_token`, `preset_store.c:53-71`).
///
/// Taken from the scene's persisted setting keys — `settings.loom.weight` yields
/// `loom` — so the mapping **cannot drift** from the `.musi` contract. Deriving it
/// rather than writing a second table is the whole trick: one of them would
/// eventually be wrong.
#[must_use]
pub fn scene_token(scene: SceneId) -> Option<&'static str> {
    let key = settings::descriptor(scene, 0)?.key;
    let after_first = key.find('.')? + 1;
    let rest = &key[after_first..];
    let end = rest.find('.')?;
    (end > 0).then(|| &rest[..end])
}

/// `preset_store_scene_from_token` (`preset_store.c:73-87`).
#[must_use]
pub fn scene_from_token(token: &str) -> Option<SceneId> {
    SceneId::ALL
        .into_iter()
        .find(|scene| scene_token(*scene) == Some(token))
}

/// The environment [`default_path`] consults, passed in rather than read.
///
/// `musializer-core` has no access to `getenv` by design, and that turns out to be
/// an improvement: the path policy is now a pure function with tests, where in C it
/// could only be exercised by mutating the process environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathEnvironment<'a> {
    /// `MUSIALIZER_PRESET_STORE` — overrides everything, for tests and portable
    /// setups.
    pub preset_store_override: Option<&'a str>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<&'a str>,
    /// `HOME`.
    pub home: Option<&'a str>,
}

/// `preset_store_default_path` (`preset_store.c:28-51`), the Linux branch.
///
/// `MUSIALIZER_PRESET_STORE` wins outright; otherwise
/// `$XDG_DATA_HOME/musializer/presets.json`, falling back to
/// `$HOME/.local/share/musializer/presets.json`.
///
/// The C also has Windows (`%APPDATA%\Musializer\presets.json`) and macOS
/// (`$HOME/Library/Application Support/...`) branches. Those are stated non-goals
/// for the first pass and are deliberately absent rather than half-written.
#[must_use]
pub fn default_path(environment: &PathEnvironment<'_>) -> Option<String> {
    if let Some(override_path) = environment.preset_store_override {
        if !override_path.is_empty() {
            return Some(override_path.to_owned());
        }
    }
    if let Some(data_home) = environment.xdg_data_home {
        if !data_home.is_empty() {
            return Some(format!("{data_home}/musializer/presets.json"));
        }
    }
    let home = environment.home.filter(|home| !home.is_empty())?;
    Some(format!("{home}/.local/share/musializer/presets.json"))
}

/// `library_from_document` (`preset_store.c:89-118`).
///
/// Every record is checked against the *live* settings tables, so a store written by
/// a build whose scene had fewer controls loads through the snapshot legacy-count
/// rules, and a store naming a scene this build does not have is rejected outright
/// rather than dropped.
pub fn library_from_document(
    store: &PresetStoreDocument,
) -> Result<PresetLibrary, PresetStoreError> {
    let mut library = PresetLibrary::new();
    for source in &store.presets {
        let scene = scene_from_token(&source.scene_name).ok_or(PresetStoreError::Format)?;
        if source.id == u64::MAX {
            return Err(PresetStoreError::Format);
        }
        if library.scenes[scene.index()].len() >= PRESETS_PER_SCENE {
            return Err(PresetStoreError::Format);
        }
        let mut snapshot = SettingsSnapshot {
            captured: true,
            count: source.settings.len(),
            values: [0.0; settings::MAX_CONTROLS],
        };
        if source.settings.len() > settings::MAX_CONTROLS {
            return Err(PresetStoreError::Format);
        }
        snapshot.values[..source.settings.len()].copy_from_slice(&source.settings);
        if !snapshot.is_valid_for(scene) {
            return Err(PresetStoreError::Format);
        }
        library.scenes[scene.index()].push(Preset {
            id: source.id,
            name: source.name.clone(),
            snapshot,
        });
        if source.id >= library.next_id {
            library.next_id = source.id + 1;
        }
    }
    if !library.is_valid() {
        return Err(PresetStoreError::Format);
    }
    Ok(library)
}

/// `document_from_library` (`preset_store.c:120-149`).
///
/// Deterministic order — scene by scene, in registry order — so an unchanged library
/// serializes to identical bytes and a store file does not churn.
pub fn document_from_library(
    library: &PresetLibrary,
) -> Result<PresetStoreDocument, PresetStoreError> {
    if !library.is_valid() {
        return Err(PresetStoreError::Argument);
    }
    let mut store = PresetStoreDocument {
        next_id: if library.next_id > 0 {
            library.next_id
        } else {
            1
        },
        presets: Vec::with_capacity(library.total()),
    };
    for scene in SceneId::ALL {
        let token = scene_token(scene).ok_or(PresetStoreError::Argument)?;
        if token.len() > capacity::TYPE {
            return Err(PresetStoreError::Argument);
        }
        for preset in library.presets(scene) {
            store.presets.push(ScenePreset {
                id: preset.id,
                scene_name: token.to_owned(),
                name: preset.name.clone(),
                settings: preset.snapshot.values[..preset.snapshot.count].to_vec(),
            });
        }
    }
    Ok(store)
}

/// The pure half of `preset_store_load` (`preset_store.c:151-195`).
///
/// The runtime reads the file — a missing one is [`PresetStoreError::Read`]'s
/// caller's business, reported as "no store yet" — and hands the bytes here. An
/// empty or oversized file is a format error, not an empty library, because
/// truncation is indistinguishable from emptiness and overwriting it would destroy
/// recoverable data.
pub fn load_from_bytes(input: &[u8]) -> Result<PresetLibrary, PresetStoreError> {
    if input.is_empty() || input.len() > MAX_FILE_SIZE {
        return Err(PresetStoreError::Format);
    }
    let store = preset_store_deserialize(input).map_err(|error| match error {
        ProjectIoError::InputSize => PresetStoreError::Read,
        _ => PresetStoreError::Format,
    })?;
    library_from_document(&store)
}

/// The pure half of `preset_store_save` (`preset_store.c:218-259`).
///
/// The runtime creates the parent directories and publishes these bytes atomically.
/// Serializing first means a library that cannot be represented fails **before**
/// anything touches the destination.
pub fn save_to_string(library: &PresetLibrary) -> Result<String, PresetStoreError> {
    let store = document_from_library(library)?;
    preset_store_serialize(&store).map_err(|_| PresetStoreError::Argument)
}

/// `preset_store_merge` (`preset_store.c:269-305`).
///
/// Copies presets the destination does not already hold. Identity is
/// `(scene, exact setting values)`: an identical snapshot is skipped no matter its
/// name, while a same-named preset with *different* values is imported. Imports
/// receive fresh destination ids. Presets that do not fit a full scene are counted in
/// `skipped` and left out.
///
/// Returns `(imported, skipped)`.
pub fn merge(
    destination: &mut PresetLibrary,
    source: &PresetLibrary,
) -> Result<(usize, usize), PresetStoreError> {
    if !destination.is_valid() || !source.is_valid() {
        return Err(PresetStoreError::Argument);
    }
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for scene in SceneId::ALL {
        for candidate in source.presets(scene) {
            let present = destination.presets(scene).iter().any(|existing| {
                existing.snapshot.count == candidate.snapshot.count
                    && existing.snapshot.values[..existing.snapshot.count]
                        == candidate.snapshot.values[..candidate.snapshot.count]
            });
            if present {
                continue;
            }
            if destination.scenes[scene.index()].len() >= PRESETS_PER_SCENE {
                skipped += 1;
                continue;
            }
            let id = destination.next_id;
            destination.scenes[scene.index()].push(Preset {
                id,
                name: candidate.name.clone(),
                snapshot: candidate.snapshot,
            });
            destination.next_id += 1;
            imported += 1;
        }
    }
    Ok((imported, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::SceneSettings;

    fn snapshot_of(scene: SceneId, first_value: f32) -> SettingsSnapshot {
        let mut values = SceneSettings::new();
        let index = 0usize;
        let descriptor = settings::descriptor(scene, index).unwrap();
        let clamped = first_value.clamp(descriptor.minimum, descriptor.maximum);
        assert!(values.set(scene, index, clamped));
        values.capture(scene).unwrap()
    }

    fn library() -> PresetLibrary {
        let mut library = PresetLibrary::new();
        library
            .push(
                SceneId::Spectrum,
                "Bright",
                &snapshot_of(SceneId::Spectrum, 1.0),
            )
            .unwrap();
        library
            .push(SceneId::Loom, "Dense", &snapshot_of(SceneId::Loom, 0.9))
            .unwrap();
        library
    }

    #[test]
    fn scene_tokens_come_from_the_persisted_setting_keys() {
        assert_eq!(scene_token(SceneId::Loom), Some("loom"));
        assert_eq!(scene_token(SceneId::Spectrum), Some("spectrum"));
        for scene in SceneId::ALL {
            let token = scene_token(scene).expect("every scene has a token");
            assert!(!token.is_empty());
            assert_eq!(
                scene_from_token(token),
                Some(scene),
                "{token} must round trip"
            );
            // The token is a prefix of the scene's own keys, which is what stops it
            // drifting from the `.musi` contract.
            assert!(settings::descriptor(scene, 0)
                .unwrap()
                .key
                .starts_with(&format!("settings.{token}.")));
        }
        assert_eq!(scene_from_token("nonexistent"), None);
        assert_eq!(scene_from_token(""), None);
    }

    #[test]
    fn every_scene_token_is_distinct() {
        let mut tokens: Vec<&str> = SceneId::ALL
            .into_iter()
            .map(|scene| scene_token(scene).unwrap())
            .collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), SCENE_COUNT);
    }

    #[test]
    fn the_override_wins_then_xdg_then_home() {
        assert_eq!(
            default_path(&PathEnvironment {
                preset_store_override: Some("/tmp/store.json"),
                xdg_data_home: Some("/xdg"),
                home: Some("/home/user"),
            }),
            Some("/tmp/store.json".into())
        );
        assert_eq!(
            default_path(&PathEnvironment {
                xdg_data_home: Some("/xdg"),
                home: Some("/home/user"),
                ..PathEnvironment::default()
            }),
            Some("/xdg/musializer/presets.json".into())
        );
        assert_eq!(
            default_path(&PathEnvironment {
                home: Some("/home/user"),
                ..PathEnvironment::default()
            }),
            Some("/home/user/.local/share/musializer/presets.json".into())
        );
        // An empty variable counts as unset, exactly as the C's `[0] != '\0'` does.
        assert_eq!(
            default_path(&PathEnvironment {
                preset_store_override: Some(""),
                xdg_data_home: Some(""),
                home: Some("/home/user"),
            }),
            Some("/home/user/.local/share/musializer/presets.json".into())
        );
        assert_eq!(default_path(&PathEnvironment::default()), None);
    }

    #[test]
    fn a_library_round_trips_through_the_store_file() {
        let library = library();
        let text = save_to_string(&library).unwrap();
        let reloaded = load_from_bytes(text.as_bytes()).unwrap();
        assert_eq!(reloaded, library);
    }

    #[test]
    fn serialization_is_stable_so_the_store_file_does_not_churn() {
        let library = library();
        let first = save_to_string(&library).unwrap();
        let second = save_to_string(&load_from_bytes(first.as_bytes()).unwrap()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn an_empty_or_truncated_store_is_a_format_error_not_an_empty_library() {
        // Overwriting either would destroy recoverable data.
        assert_eq!(load_from_bytes(b"").unwrap_err(), PresetStoreError::Format);
        let text = save_to_string(&library()).unwrap();
        let truncated = &text.as_bytes()[..text.len() / 2];
        assert_eq!(
            load_from_bytes(truncated).unwrap_err(),
            PresetStoreError::Format
        );
        let oversized = vec![b' '; MAX_FILE_SIZE + 1];
        assert_eq!(
            load_from_bytes(&oversized).unwrap_err(),
            PresetStoreError::Format
        );
    }

    #[test]
    fn a_store_naming_an_unknown_scene_is_rejected_not_dropped() {
        let text = save_to_string(&library()).unwrap();
        let broken = text.replace("\"scene_name\":\"loom\"", "\"scene_name\":\"tapestry\"");
        assert!(broken != text);
        assert_eq!(
            load_from_bytes(broken.as_bytes()).unwrap_err(),
            PresetStoreError::Format
        );
    }

    #[test]
    fn a_snapshot_out_of_range_for_its_scene_is_rejected() {
        let text = save_to_string(&library()).unwrap();
        // Spectrum's first control is amplitude; a wildly out-of-range value must
        // not load, because applying it would produce a scene nobody authored.
        let store = preset_store_deserialize(text.as_bytes()).unwrap();
        let mut broken = store.clone();
        broken.presets[0].settings[0] = 1e9;
        assert_eq!(
            library_from_document(&broken).unwrap_err(),
            PresetStoreError::Format
        );
    }

    #[test]
    fn a_legacy_snapshot_with_fewer_values_still_loads() {
        // Spectrum had 3 and 7 controls in earlier builds
        // (`settings::count_is_legacy`), and those stores must keep working.
        let library = library();
        let store = document_from_library(&library).unwrap();
        let mut legacy = store.clone();
        legacy.presets[0].settings.truncate(3);
        assert!(settings::count_is_legacy(SceneId::Spectrum, 3));
        let loaded = library_from_document(&legacy).expect("a legacy count must load");
        assert_eq!(loaded.presets(SceneId::Spectrum)[0].snapshot.count, 3);

        // A count that was never real is refused rather than back-filled.
        let mut nonsense = store;
        nonsense.presets[0].settings.truncate(5);
        assert!(!settings::count_is_legacy(SceneId::Spectrum, 5));
        assert_eq!(
            library_from_document(&nonsense).unwrap_err(),
            PresetStoreError::Format
        );
    }

    #[test]
    fn ids_must_be_nonzero_unique_and_below_next_id() {
        let library = library();
        let store = document_from_library(&library).unwrap();

        let mut zero = store.clone();
        zero.presets[0].id = 0;
        assert_eq!(
            library_from_document(&zero).unwrap_err(),
            PresetStoreError::Format
        );

        let mut duplicate = store.clone();
        duplicate.presets[1].id = duplicate.presets[0].id;
        assert_eq!(
            library_from_document(&duplicate).unwrap_err(),
            PresetStoreError::Format
        );

        // u64::MAX is refused because next_id could not advance past it.
        let mut saturated = store;
        saturated.presets[0].id = u64::MAX;
        assert_eq!(
            library_from_document(&saturated).unwrap_err(),
            PresetStoreError::Format
        );
    }

    #[test]
    fn pushing_allocates_a_fresh_id_every_time() {
        let mut library = PresetLibrary::new();
        assert_eq!(
            library.push(SceneId::Loom, "A", &snapshot_of(SceneId::Loom, 0.5)),
            Some(0)
        );
        assert_eq!(
            library.push(SceneId::Loom, "B", &snapshot_of(SceneId::Loom, 0.6)),
            Some(1)
        );
        assert_eq!(library.presets(SceneId::Loom)[0].id, 1);
        assert_eq!(library.presets(SceneId::Loom)[1].id, 2);
        assert_eq!(library.next_id(), 3);

        // Removing does not lower next_id: an id is never reused.
        assert!(library.remove(SceneId::Loom, 0));
        assert_eq!(library.next_id(), 3);
        assert!(library.is_valid());
        assert!(!library.remove(SceneId::Loom, 9));
    }

    #[test]
    fn a_full_scene_refuses_another_preset() {
        let mut library = PresetLibrary::new();
        for index in 0..PRESETS_PER_SCENE {
            assert!(library
                .push(
                    SceneId::Loom,
                    &format!("P{index}"),
                    &snapshot_of(SceneId::Loom, 0.5)
                )
                .is_some());
        }
        assert_eq!(
            library.push(SceneId::Loom, "one more", &snapshot_of(SceneId::Loom, 0.5)),
            None
        );
        // But a different scene still has room; the capacity is per scene.
        assert!(library
            .push(SceneId::Cadence, "ok", &snapshot_of(SceneId::Cadence, 0.5))
            .is_some());
    }

    #[test]
    fn pushing_refuses_a_bad_name_or_a_foreign_snapshot() {
        let mut library = PresetLibrary::new();
        let good = snapshot_of(SceneId::Loom, 0.5);
        assert_eq!(library.push(SceneId::Loom, "", &good), None);
        assert_eq!(
            library.push(SceneId::Loom, &"x".repeat(NAME_MAX_BYTES + 1), &good),
            None
        );
        assert_eq!(
            library.push(SceneId::Loom, "ok", &SettingsSnapshot::default()),
            None,
            "an uncaptured snapshot is not a preset"
        );
        // Song Atlas has 12 controls, Loom has 7, so the shapes cannot be confused.
        assert_eq!(
            library.push(SceneId::Loom, "ok", &snapshot_of(SceneId::SongAtlas, 0.5)),
            None
        );
    }

    #[test]
    fn merge_skips_identical_snapshots_whatever_they_are_called() {
        let mut destination = library();
        let mut source = PresetLibrary::new();
        // Same values as the destination's Spectrum preset, different name.
        source
            .push(
                SceneId::Spectrum,
                "A Different Name",
                &destination.presets(SceneId::Spectrum)[0].snapshot.clone(),
            )
            .unwrap();
        let (imported, skipped) = merge(&mut destination, &source).unwrap();
        assert_eq!((imported, skipped), (0, 0));
        assert_eq!(destination.presets(SceneId::Spectrum).len(), 1);
    }

    #[test]
    fn merge_imports_same_named_presets_with_different_values() {
        let mut destination = library();
        let mut source = PresetLibrary::new();
        source
            .push(
                SceneId::Spectrum,
                "Bright",
                &snapshot_of(SceneId::Spectrum, 0.25),
            )
            .unwrap();
        let before = destination.next_id();
        let (imported, skipped) = merge(&mut destination, &source).unwrap();
        assert_eq!((imported, skipped), (1, 0));
        assert_eq!(destination.presets(SceneId::Spectrum).len(), 2);
        let added = &destination.presets(SceneId::Spectrum)[1];
        assert_eq!(added.id, before, "imports get fresh destination ids");
        assert_eq!(added.name, "Bright");
        assert!(destination.is_valid());
    }

    #[test]
    fn merge_counts_what_does_not_fit_instead_of_dropping_it_silently() {
        let mut destination = PresetLibrary::new();
        for index in 0..PRESETS_PER_SCENE {
            destination
                .push(
                    SceneId::Loom,
                    &format!("D{index}"),
                    &snapshot_of(SceneId::Loom, 0.1 + index as f32 * 0.05),
                )
                .unwrap();
        }
        let mut source = PresetLibrary::new();
        source
            .push(SceneId::Loom, "extra", &snapshot_of(SceneId::Loom, 0.99))
            .unwrap();
        let (imported, skipped) = merge(&mut destination, &source).unwrap();
        assert_eq!((imported, skipped), (0, 1));
        assert_eq!(destination.presets(SceneId::Loom).len(), PRESETS_PER_SCENE);
    }

    #[test]
    fn merge_refuses_an_invalid_library_on_either_side() {
        let mut broken = library();
        broken.next_id = 1;
        assert!(!broken.is_valid());
        assert_eq!(
            merge(&mut broken, &library()).unwrap_err(),
            PresetStoreError::Argument
        );
        let mut destination = library();
        let mut source = library();
        source.next_id = 0;
        assert_eq!(
            merge(&mut destination, &source).unwrap_err(),
            PresetStoreError::Argument
        );
    }

    #[test]
    fn replacing_a_snapshot_keeps_the_id_and_name() {
        let mut library = library();
        let before = library.presets(SceneId::Loom)[0].clone();
        let replacement = snapshot_of(SceneId::Loom, 0.2);
        assert!(library.replace_snapshot(SceneId::Loom, 0, &replacement));
        let after = &library.presets(SceneId::Loom)[0];
        assert_eq!(after.id, before.id);
        assert_eq!(after.name, before.name);
        assert_eq!(after.snapshot, replacement);
        assert!(!library.replace_snapshot(SceneId::Loom, 9, &replacement));
        assert!(!library.replace_snapshot(SceneId::Loom, 0, &SettingsSnapshot::default()));
    }
}
