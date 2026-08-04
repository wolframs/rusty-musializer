//! Where assist settings and credentials live on disk, and how they are written.
//!
//! Path resolution follows `musializer-app`'s `ui/preferences.rs` exactly —
//! an explicit environment override, then `$XDG_CONFIG_HOME`, then
//! `$HOME/.config` — so a user who moved one file has moved all of them.
//!
//! Two rules here are not conveniences and each has a test:
//!
//! - **The permissions exist before the secret does.** The containing directory
//!   is `0700` and the temporary file is created `0600` by `open(2)` itself, so
//!   there is no window in which the bytes are on disk and readable by anyone
//!   else. A `umask` can only take bits away from a `create_new` mode, never add
//!   them, and the mode is re-applied explicitly anyway.
//! - **A loose-permission credentials file is refused, not repaired.** The same
//!   stance `ssh` takes on a group-readable private key. Silently `chmod`ding it
//!   would hide the fact that the key has already been exposed, which is the one
//!   piece of information the user needs.
//!
//! Settings are written through [`crate::process::publish::atomic_write`], the
//! same transaction `.musi` saves use. Credentials cannot: that path preserves
//! an existing file's mode and creates a new one `0666 & ~umask`, which is
//! exactly wrong for a key, so the private write is spelled out below.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use musializer_core::assist::credentials::{CredentialStore, CredentialsError};
use musializer_core::assist::settings::{AssistSettings, SettingsError};

/// `O_CLOEXEC`, so a staged credential fd cannot leak into `ffmpeg`, `codex`, a
/// Python helper or a file dialog. Hardcoded rather than pulled from `libc`,
/// which is not a dependency; the value is fixed ABI on Linux. Same constant and
/// same reason as `process::publish`.
const O_CLOEXEC: i32 = 0o2_000_000;

/// Owner-only file. Anything else is a refusal, not a repair.
pub const PRIVATE_FILE_MODE: u32 = 0o600;

/// Owner-only directory.
pub const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

/// How many temporary names to try before giving up.
const NAME_ATTEMPTS: u32 = 64;

static NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum AssistFileError {
    #[error("could not read {0}")]
    Read(PathBuf),
    #[error("could not create the configuration directory {0}")]
    Directory(PathBuf),
    #[error("could not write {0}")]
    Write(PathBuf),
    #[error(
        "{0} is readable by other users (mode {1:04o}); \
         treat the credential as exposed, then run `chmod 600` on it"
    )]
    Permissions(PathBuf, u32),
    #[error("no per-user configuration directory could be resolved")]
    NoConfigDirectory,
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
}

/// `$MUSIALIZER_ASSIST_SETTINGS`, else `$XDG_CONFIG_HOME/musializer/assist.json`,
/// else `$HOME/.config/musializer/assist.json` (§2).
#[must_use]
pub fn settings_path() -> Option<PathBuf> {
    resolve_path("MUSIALIZER_ASSIST_SETTINGS", "assist.json")
}

/// `$MUSIALIZER_ASSIST_CREDENTIALS`, else
/// `$XDG_CONFIG_HOME/musializer/credentials.json`, else the `$HOME/.config`
/// spelling (§3).
#[must_use]
pub fn credentials_path() -> Option<PathBuf> {
    resolve_path("MUSIALIZER_ASSIST_CREDENTIALS", "credentials.json")
}

fn resolve_path(override_variable: &str, file_name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(override_variable) {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer").join(file_name));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| {
            PathBuf::from(path)
                .join(".config/musializer")
                .join(file_name)
        })
}

fn read_bounded(path: &Path, cap: u64) -> Result<Option<Vec<u8>>, AssistFileError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AssistFileError::Read(path.to_path_buf())),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AssistFileError::Read(path.to_path_buf()))?;
    Ok(Some(bytes))
}

/// Reads the settings record. `Ok(None)` means "no file yet", which is the only
/// case that legitimately falls back to defaults; every other problem is an
/// error, so a corrupt file is never silently replaced.
pub fn load_settings(path: &Path) -> Result<Option<AssistSettings>, AssistFileError> {
    let Some(bytes) = read_bounded(path, musializer_core::assist::settings::MAX_FILE_SIZE)? else {
        return Ok(None);
    };
    Ok(Some(AssistSettings::parse(&bytes)?))
}

/// Writes the settings record atomically.
pub fn save_settings(path: &Path, settings: &AssistSettings) -> Result<(), AssistFileError> {
    let bytes = settings.to_bytes()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| AssistFileError::Directory(parent.to_path_buf()))?;
    }
    crate::process::publish::atomic_write(path, &bytes)
        .map_err(|_| AssistFileError::Write(path.to_path_buf()))
}

/// Reads the credentials file, refusing a loose-permission one.
///
/// The permission check happens **before** the bytes are read, so a file this
/// function refuses is a file it never opened.
pub fn load_credentials(path: &Path) -> Result<Option<CredentialStore>, AssistFileError> {
    match fs::metadata(path) {
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o7777;
            if mode & 0o077 != 0 {
                return Err(AssistFileError::Permissions(path.to_path_buf(), mode));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AssistFileError::Read(path.to_path_buf())),
    }
    let Some(bytes) = read_bounded(path, musializer_core::assist::credentials::MAX_FILE_SIZE)?
    else {
        return Ok(None);
    };
    Ok(Some(CredentialStore::parse(&bytes)?))
}

/// Writes the credentials file: directory `0700`, file `0600`, temp file created
/// `0600`, `fsync`, `rename`.
pub fn save_credentials(path: &Path, store: &CredentialStore) -> Result<(), AssistFileError> {
    let bytes = store.to_bytes()?;
    atomic_write_private(path, &bytes)
}

/// `Forget` (§3): removes one provider's entry, rewrites the file atomically,
/// and removes the file entirely when nothing is left. Reports whether an entry
/// was actually there.
pub fn forget_credential(
    path: &Path,
    provider: &str,
    lookup_id: &str,
) -> Result<bool, AssistFileError> {
    let Some(mut store) = load_credentials(path)? else {
        return Ok(false);
    };
    if !store.forget(provider, lookup_id) {
        return Ok(false);
    }
    if store.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(AssistFileError::Write(path.to_path_buf())),
        }
    } else {
        save_credentials(path, &store)?;
    }
    Ok(true)
}

/// Creates the containing directory as `0700` and re-applies the mode if it
/// already existed with something looser.
///
/// Re-applying is the deliberate asymmetry with [`load_credentials`]: tightening
/// a *directory* we are about to write a secret into is not hiding a past
/// exposure, because the secret is not there yet.
pub fn ensure_private_directory(directory: &Path) -> Result<(), AssistFileError> {
    let mut builder = DirBuilder::new();
    builder.recursive(true).mode(PRIVATE_DIRECTORY_MODE);
    builder
        .create(directory)
        .map_err(|_| AssistFileError::Directory(directory.to_path_buf()))?;
    fs::set_permissions(
        directory,
        fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
    )
    .map_err(|_| AssistFileError::Directory(directory.to_path_buf()))
}

fn atomic_write_private(destination: &Path, data: &[u8]) -> Result<(), AssistFileError> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| AssistFileError::Directory(destination.to_path_buf()))?;
    ensure_private_directory(parent)?;

    let stem = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AssistFileError::Write(destination.to_path_buf()))?;
    let process_id = std::process::id();

    for _ in 0..NAME_ATTEMPTS {
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{stem}.{process_id}.{nonce}.tmp"));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(O_CLOEXEC)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(AssistFileError::Write(temporary)),
        };
        // `create_new` honours `mode` only for a fresh file and a `umask` can
        // trim it further, so the mode is stated again while the file is still
        // empty. Nothing secret has been written at this point.
        let staged = (|| -> std::io::Result<()> {
            file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
            file.write_all(data)?;
            file.sync_all()
        })();
        drop(file);
        if staged.is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(AssistFileError::Write(temporary));
        }
        if fs::rename(&temporary, destination).is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(AssistFileError::Write(destination.to_path_buf()));
        }
        // The rename is only durable once the directory entry is on disk.
        if let Ok(handle) = fs::File::open(parent) {
            let _ = handle.sync_all();
        }
        return Ok(());
    }
    Err(AssistFileError::Write(destination.to_path_buf()))
}

/// The mode bits of a path, for tests and for the canary probe's report.
pub fn mode_of(path: &Path) -> std::io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o7777)
}

#[cfg(test)]
mod tests {
    use musializer_core::assist::secret::Secret;

    use super::*;

    const CANARY: &str = "sk-or-v1-MUSICANARY7Q4X2ZK9aaaabbbbcccc";

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-assist-files")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn two_providers() -> CredentialStore {
        let mut store = CredentialStore::new();
        store.insert(
            "openrouter",
            "default",
            Secret::new(CANARY.to_string()),
            Some("Personal".to_string()),
        );
        store.insert(
            "together",
            "work",
            Secret::new("sk-together-second-key".to_string()),
            None,
        );
        store
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let path = scratch("settings").join("musializer/assist.json");
        assert!(load_settings(&path).unwrap().is_none());
        let settings = AssistSettings::default();
        save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path).unwrap(), Some(settings));
    }

    #[test]
    fn a_corrupt_settings_file_is_not_treated_as_an_empty_record() {
        let path = scratch("settings-corrupt").join("assist.json");
        fs::write(&path, b"{ broken").unwrap();
        assert!(matches!(
            load_settings(&path),
            Err(AssistFileError::Settings(SettingsError::Format))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{ broken");
    }

    #[test]
    fn an_oversized_settings_file_is_refused() {
        let path = scratch("settings-oversize").join("assist.json");
        let cap = musializer_core::assist::settings::MAX_FILE_SIZE as usize;
        fs::write(&path, vec![b' '; cap + 1]).unwrap();
        assert!(matches!(
            load_settings(&path),
            Err(AssistFileError::Settings(SettingsError::TooLarge))
        ));
    }

    #[test]
    fn credentials_are_written_owner_only_and_so_is_their_directory() {
        let root = scratch("credentials-modes");
        let path = root.join("musializer/credentials.json");
        save_credentials(&path, &two_providers()).unwrap();

        let file_mode = mode_of(&path).unwrap();
        let directory_mode = mode_of(path.parent().unwrap()).unwrap();
        assert_eq!(file_mode & 0o077, 0, "file mode {file_mode:04o}");
        assert_eq!(directory_mode & 0o077, 0, "dir mode {directory_mode:04o}");
        assert_eq!(file_mode, PRIVATE_FILE_MODE);
        assert_eq!(directory_mode, PRIVATE_DIRECTORY_MODE);

        let store = load_credentials(&path).unwrap().unwrap();
        assert_eq!(
            store.get("openrouter", "default").unwrap().secret.expose(),
            CANARY
        );
    }

    #[test]
    fn no_temporary_file_survives_a_write() {
        let root = scratch("credentials-temp");
        let path = root.join("musializer/credentials.json");
        save_credentials(&path, &two_providers()).unwrap();
        save_credentials(&path, &two_providers()).unwrap();
        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["credentials.json".to_string()]);
    }

    #[test]
    fn a_group_readable_credentials_file_is_refused_and_left_alone() {
        let root = scratch("credentials-loose");
        let path = root.join("credentials.json");
        fs::write(&path, CredentialStore::new().to_bytes().unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = load_credentials(&path).unwrap_err();
        assert!(
            matches!(error, AssistFileError::Permissions(_, 0o644)),
            "{error:?}"
        );
        // Refused, not repaired: the mode is exactly what it was.
        assert_eq!(mode_of(&path).unwrap(), 0o644);
        assert!(error.to_string().contains("exposed"));
    }

    #[test]
    fn every_loose_mode_bit_is_refused() {
        let root = scratch("credentials-loose-sweep");
        for mode in [0o604, 0o640, 0o660, 0o606, 0o666, 0o601] {
            let path = root.join(format!("credentials-{mode:o}.json"));
            fs::write(&path, CredentialStore::new().to_bytes().unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(
                matches!(
                    load_credentials(&path),
                    Err(AssistFileError::Permissions(_, _))
                ),
                "mode {mode:04o} was accepted"
            );
            assert_eq!(mode_of(&path).unwrap(), mode);
        }
        // And the two acceptable ones still load.
        for mode in [0o600, 0o400] {
            let path = root.join(format!("credentials-ok-{mode:o}.json"));
            fs::write(&path, CredentialStore::new().to_bytes().unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(
                load_credentials(&path).is_ok(),
                "mode {mode:04o} was refused"
            );
        }
    }

    #[test]
    fn forget_leaves_the_other_providers_bytes_identical_on_disk() {
        let root = scratch("credentials-forget");
        let path = root.join("musializer/credentials.json");
        save_credentials(&path, &two_providers()).unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let kept_before = entry_block(&before, "together/work");

        assert!(forget_credential(&path, "openrouter", "default").unwrap());

        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(entry_block(&after, "together/work"), kept_before);
        assert!(!after.contains("MUSICANARY"));
        assert_eq!(mode_of(&path).unwrap(), PRIVATE_FILE_MODE);
    }

    #[test]
    fn forgetting_the_last_entry_removes_the_file() {
        let root = scratch("credentials-forget-last");
        let path = root.join("musializer/credentials.json");
        let mut store = CredentialStore::new();
        store.insert(
            "openrouter",
            "default",
            Secret::new(CANARY.to_string()),
            None,
        );
        save_credentials(&path, &store).unwrap();

        assert!(forget_credential(&path, "openrouter", "default").unwrap());
        assert!(!path.exists());
        assert!(!forget_credential(&path, "openrouter", "default").unwrap());
    }

    fn entry_block(text: &str, key: &str) -> String {
        let opening = format!("\"{key}\": {{");
        let start = text.find(&opening).expect("entry is present");
        let end = text[start..].find("\n    }").expect("entry closes") + start + "\n    }".len();
        text[start..end].to_string()
    }
}
