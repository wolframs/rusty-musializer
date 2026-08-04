//! The real writability probe behind the models-directory rule.
//!
//! The decision is [`musializer_core::assist::models_dir`], which opens
//! nothing. What is here is the part that has to touch the disk: reading the
//! executable's own location, the home directory, and answering "could this
//! process create files there?" for a path that may not exist yet.
//!
//! The probe **creates and removes one empty file** in the nearest existing
//! ancestor of the candidate. That is deliberate: `access(2)`-style permission
//! arithmetic gets read-only mounts, ACLs, full filesystems and immutable bits
//! wrong, and this question is only worth asking if the answer is the same one
//! the download will get. It is the same stance `musializer_doctor.py`'s
//! `_probe_directory` takes, and the two must agree, because the doctor's job
//! is to report what the application will do.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use musializer_core::assist::models_dir::{
    self, ModelsDirError, ModelsDirRequest, ModelsDirResolution,
};
use musializer_core::assist::settings::AssistSettings;

/// `O_CLOEXEC`, for the same reason `assist::files` hardcodes it: a probe file
/// descriptor must not survive into `ffmpeg`, a dialog or a Python helper.
const O_CLOEXEC: i32 = 0o2_000_000;

/// How many probe names to try before deciding the directory is unusable.
const NAME_ATTEMPTS: u32 = 8;

static NONCE: AtomicU64 = AtomicU64::new(0);

/// The directory holding the running executable, when it can be determined.
#[must_use]
pub fn executable_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

/// `$HOME`, when it is set and non-empty.
#[must_use]
pub fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Whether this process could create files in `candidate`.
///
/// A candidate that does not exist yet is answered by probing its nearest
/// existing ancestor, because the question the caller is really asking is "can
/// the download create this directory and write into it?". A path whose
/// existing prefix is a *file* rather than a directory answers false rather
/// than erroring: it is unusable either way, and a probe is not the place to
/// explain filesystem layout.
#[must_use]
pub fn is_writable(candidate: &Path) -> bool {
    let mut probe = candidate;
    loop {
        match fs::metadata(probe) {
            Ok(metadata) if metadata.is_dir() => break,
            // An existing non-directory in the chain cannot become one.
            Ok(_) => return false,
            Err(error) if error.kind() == ErrorKind::NotFound => match probe.parent() {
                Some(parent) if parent != probe && !parent.as_os_str().is_empty() => {
                    probe = parent;
                }
                _ => return false,
            },
            Err(_) => return false,
        }
    }

    let process_id = std::process::id();
    for _ in 0..NAME_ATTEMPTS {
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let temporary = probe.join(format!(".musializer-models-probe.{process_id}.{nonce}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(O_CLOEXEC)
            .open(&temporary)
        {
            Ok(file) => {
                drop(file);
                let _ = fs::remove_file(&temporary);
                return true;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

/// Applies the operator's models-directory rule against this machine.
///
/// `settings` is the loaded record, or `None` when there is no settings file
/// yet — which is the same thing as an unset override.
pub fn resolve(settings: Option<&AssistSettings>) -> Result<ModelsDirResolution, ModelsDirError> {
    let executable_dir = executable_directory();
    let home_dir = home_directory();
    resolve_from(settings, executable_dir.as_deref(), home_dir.as_deref())
}

/// [`resolve`] with the two locations supplied, so a test can state them
/// instead of depending on where the test binary happens to live.
pub fn resolve_from(
    settings: Option<&AssistSettings>,
    executable_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> Result<ModelsDirResolution, ModelsDirError> {
    let request = match settings {
        Some(settings) => ModelsDirRequest::from_settings(settings, executable_dir, home_dir),
        None => ModelsDirRequest {
            configured: "",
            executable_dir,
            home_dir,
        },
    };
    models_dir::resolve(request, is_writable)
}

#[cfg(test)]
mod tests {
    use musializer_core::assist::models_dir::ModelsDirSource;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-assist-models")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_writable_directory_probes_true_and_leaves_nothing_behind() {
        let root = scratch("writable");
        assert!(is_writable(&root));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn a_directory_that_does_not_exist_yet_probes_its_nearest_parent() {
        let root = scratch("creatable");
        let candidate = root.join("models/mms-ctc/weights");
        assert!(is_writable(&candidate));
        assert!(
            !candidate.exists(),
            "the probe must not create the candidate"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn a_path_whose_prefix_is_a_file_probes_false() {
        let root = scratch("file-prefix");
        let file = root.join("models");
        fs::write(&file, b"not a directory").unwrap();
        assert!(!is_writable(&file.join("mms-ctc")));
    }

    #[test]
    fn a_read_only_directory_probes_false() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("read-only");
        let locked = root.join("install");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();

        let probed = is_writable(&locked);
        // Restore before asserting, so a failure does not leave an
        // undeletable directory in build/.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        if probed {
            // Running as root, where the mode bits do not apply. The assertion
            // below would be testing the test environment, not the probe.
            eprintln!("skipped: this process can write to a 0500 directory");
            return;
        }
        assert!(!probed);
    }

    #[test]
    fn a_read_only_install_directory_resolves_to_the_home_fallback() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("resolve-fallback");
        let install = root.join("install");
        let home = root.join("home");
        fs::create_dir(&install).unwrap();
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&install, fs::Permissions::from_mode(0o500)).unwrap();

        let resolved = resolve_from(None, Some(&install), Some(&home));
        fs::set_permissions(&install, fs::Permissions::from_mode(0o700)).unwrap();
        let resolved = resolved.unwrap();
        if resolved.install_default_writable {
            eprintln!("skipped: this process can write to a 0500 directory");
            return;
        }
        assert_eq!(resolved.source, ModelsDirSource::HomeFallback);
        assert_eq!(resolved.path, home.join("musializer/models"));
        assert!(resolved.writable);
    }

    #[test]
    fn a_writable_install_directory_resolves_to_the_install_default() {
        let root = scratch("resolve-default");
        let install = root.join("install");
        fs::create_dir(&install).unwrap();
        let resolved = resolve_from(None, Some(&install), Some(&root.join("home"))).unwrap();
        assert_eq!(resolved.source, ModelsDirSource::InstallDefault);
        assert_eq!(resolved.path, install.join("models"));
    }

    #[test]
    fn a_settings_override_wins_against_this_machine() {
        let root = scratch("resolve-override");
        let install = root.join("install");
        fs::create_dir(&install).unwrap();
        let mut settings = AssistSettings::default();
        let chosen = root.join("elsewhere/models");
        settings.local_runtimes.models_dir = chosen.to_string_lossy().into_owned();

        let resolved =
            resolve_from(Some(&settings), Some(&install), Some(&root.join("home"))).unwrap();
        assert_eq!(resolved.source, ModelsDirSource::SettingsOverride);
        assert_eq!(resolved.path, chosen);
        // The default it beat is still reported, so a surface can show it.
        assert_eq!(resolved.install_default, Some(install.join("models")));
        assert!(resolved.install_default_writable);
    }

    #[test]
    fn the_executable_directory_is_the_one_this_test_binary_runs_from() {
        let directory = executable_directory().expect("a test binary has a path");
        assert!(directory.is_dir());
        assert!(std::env::current_exe().unwrap().starts_with(&directory));
    }
}
