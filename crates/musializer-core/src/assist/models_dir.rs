//! Where downloaded model weights live, resolved without touching the disk.
//!
//! The rule is an operator decision (`AGENTS.md`, "Persistent file storage",
//! 2026-08-04) and is written down again in `docs/ASSIST_PROVIDER_CONTRACTS.md`
//! §2 next to `local_runtimes.models_dir`:
//!
//! 1. an explicit setting wins, always;
//! 2. else `<directory of the running executable>/models/`, when it is writable;
//! 3. else `<home>/musializer/models`.
//!
//! and, underneath all three, *never a location the user was not shown* — which
//! is why [`ModelsDirResolution`] carries every candidate it considered and not
//! only the winner. A dialog, the doctor and a log line can then all say which
//! rung was taken and what the other rungs were, rather than presenting one
//! unexplained path.
//!
//! The writability probe is injected because deciding is pure and probing is
//! not: this crate opens no files (`AGENTS.md`, crate map). The real probe is
//! `musializer_runtime::assist::models`. `std::path` itself performs no I/O —
//! joining and comparing components is arithmetic on strings — so the types are
//! `Path`/`PathBuf` rather than `String`, and a component that would need a
//! separator escaped cannot be built by accident.

use std::path::{Path, PathBuf};

use crate::assist::settings::AssistSettings;

/// The leaf directory name under the install directory and under the home
/// fallback. One constant, because the two rungs must not drift apart.
pub const MODELS_DIRECTORY_NAME: &str = "models";

/// The home fallback's parent, i.e. `<home>/musializer/models`.
pub const HOME_FALLBACK_DIRECTORY: &str = "musializer";

/// Which rung of the ladder produced the resolved path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelsDirSource {
    /// `local_runtimes.models_dir` was set. It wins whether or not it is
    /// writable: an override the application quietly declined to use would be
    /// exactly the "location the user was not shown" the rule forbids.
    SettingsOverride,
    /// `<executable directory>/models/`.
    InstallDefault,
    /// `<home>/musializer/models`.
    HomeFallback,
}

impl ModelsDirSource {
    /// The stable token. It reaches the doctor's JSON and a settings dialog's
    /// explanation line, so it is spelled once here.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::SettingsOverride => "settings-override",
            Self::InstallDefault => "install-default",
            Self::HomeFallback => "home-fallback",
        }
    }
}

/// Why no models directory could be named at all.
///
/// Distinct variants because the remedies differ: the first is a broken
/// environment, the second is a read-only installation that a user with no home
/// directory cannot work around without choosing a path.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelsDirError {
    #[error(
        "no models directory could be resolved: neither the executable's \
         directory nor a home directory is known"
    )]
    NoCandidate,
    #[error(
        "the default models directory {0} is not writable and no home \
         directory is known to fall back to"
    )]
    NoWritableDefault(PathBuf),
}

/// The inputs the decision is made from. All three are supplied by the caller
/// so the decision itself stays testable and pure.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelsDirRequest<'a> {
    /// `local_runtimes.models_dir`. Empty (or whitespace) means unset.
    pub configured: &'a str,
    /// The directory holding the running executable.
    pub executable_dir: Option<&'a Path>,
    /// The user's home directory.
    pub home_dir: Option<&'a Path>,
}

impl<'a> ModelsDirRequest<'a> {
    /// Reads the override out of a settings record, so no caller has to know
    /// which field carries it.
    #[must_use]
    pub fn from_settings(
        settings: &'a AssistSettings,
        executable_dir: Option<&'a Path>,
        home_dir: Option<&'a Path>,
    ) -> Self {
        Self {
            configured: settings.local_runtimes.models_dir.as_str(),
            executable_dir,
            home_dir,
        }
    }
}

/// The resolved directory *and* everything that lost, so a surface can show the
/// whole ladder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelsDirResolution {
    /// The directory to use.
    pub path: PathBuf,
    /// Which rule produced it.
    pub source: ModelsDirSource,
    /// Whether the probe reports [`Self::path`] as writable. False on an
    /// override is a warning to display, not a reason to substitute one.
    pub writable: bool,
    /// `<executable directory>/models/`, when an executable directory is known.
    pub install_default: Option<PathBuf>,
    /// Whether the install default probed writable. False is what moves the
    /// ladder to the home fallback.
    pub install_default_writable: bool,
    /// `<home>/musializer/models`, when a home directory is known.
    pub home_fallback: Option<PathBuf>,
    /// The override as configured, when there was one.
    pub configured: Option<PathBuf>,
}

impl ModelsDirResolution {
    /// True when the resolved path came from the user rather than from a
    /// default. Kept as a method so a dialog does not re-derive it from
    /// [`ModelsDirSource`] and get the sense backwards.
    #[must_use]
    pub fn is_override(&self) -> bool {
        self.source == ModelsDirSource::SettingsOverride
    }
}

/// Applies the ladder.
///
/// `probe` answers "could this process create files here?". It is called at
/// most twice — once for the install default, once for whichever path is
/// resolved — and never for a candidate that has already lost, so an
/// unreachable network mount named in a setting cannot stall a resolution that
/// did not need it.
///
/// The install default is probed even when an override wins, because the caller
/// has to be able to *show* the default it did not take; that is one bounded
/// probe against a local directory, at startup.
pub fn resolve(
    request: ModelsDirRequest<'_>,
    mut probe: impl FnMut(&Path) -> bool,
) -> Result<ModelsDirResolution, ModelsDirError> {
    let install_default = request
        .executable_dir
        .map(|directory| directory.join(MODELS_DIRECTORY_NAME));
    let home_fallback = request.home_dir.map(|home| {
        home.join(HOME_FALLBACK_DIRECTORY)
            .join(MODELS_DIRECTORY_NAME)
    });
    let install_default_writable = install_default.as_deref().is_some_and(&mut probe);

    let configured = {
        let trimmed = request.configured.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    };

    let (path, source) = if let Some(configured) = configured.clone() {
        (configured, ModelsDirSource::SettingsOverride)
    } else if let Some(default) = install_default.clone().filter(|_| install_default_writable) {
        (default, ModelsDirSource::InstallDefault)
    } else if let Some(home_fallback) = home_fallback.clone() {
        (home_fallback, ModelsDirSource::HomeFallback)
    } else if let Some(install_default) = install_default.clone() {
        return Err(ModelsDirError::NoWritableDefault(install_default));
    } else {
        return Err(ModelsDirError::NoCandidate);
    };

    let writable = if source == ModelsDirSource::InstallDefault {
        true
    } else {
        probe(&path)
    };

    Ok(ModelsDirResolution {
        path,
        source,
        writable,
        install_default,
        install_default_writable,
        home_fallback,
        configured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A probe that answers from a list and records what it was asked.
    fn probe_of<'a>(
        writable: &'static [&'static str],
        seen: &'a RefCell<Vec<PathBuf>>,
    ) -> impl FnMut(&Path) -> bool + 'a {
        move |path: &Path| {
            seen.borrow_mut().push(path.to_path_buf());
            writable.iter().any(|allowed| Path::new(allowed) == path)
        }
    }

    #[test]
    fn a_writable_install_directory_is_the_default() {
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(
            ModelsDirRequest {
                configured: "",
                executable_dir: Some(Path::new("/opt/musializer")),
                home_dir: Some(Path::new("/home/example")),
            },
            probe_of(&["/opt/musializer/models"], &seen),
        )
        .unwrap();
        assert_eq!(resolved.path, Path::new("/opt/musializer/models"));
        assert_eq!(resolved.source, ModelsDirSource::InstallDefault);
        assert!(resolved.writable);
        assert!(resolved.install_default_writable);
        assert_eq!(
            resolved.home_fallback.as_deref(),
            Some(Path::new("/home/example/musializer/models"))
        );
        assert_eq!(resolved.configured, None);
        // The losing candidate is named but never probed: one probe, for the
        // rung that was actually taken.
        assert_eq!(
            *seen.borrow(),
            vec![PathBuf::from("/opt/musializer/models")]
        );
    }

    #[test]
    fn an_unwritable_install_directory_falls_back_to_the_home_directory() {
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(
            ModelsDirRequest {
                configured: "",
                executable_dir: Some(Path::new("/usr/lib/musializer")),
                home_dir: Some(Path::new("/home/example")),
            },
            probe_of(&["/home/example/musializer/models"], &seen),
        )
        .unwrap();
        assert_eq!(
            resolved.path,
            Path::new("/home/example/musializer/models"),
            "the fallback is <home>/musializer/models, per the operator rule"
        );
        assert_eq!(resolved.source, ModelsDirSource::HomeFallback);
        assert!(resolved.writable);
        assert!(!resolved.install_default_writable);
        assert_eq!(
            resolved.install_default.as_deref(),
            Some(Path::new("/usr/lib/musializer/models")),
            "the default that lost is still shown"
        );
    }

    #[test]
    fn an_explicit_override_wins_over_a_writable_default() {
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(
            ModelsDirRequest {
                configured: "/home/example/.local/share/musializer/models",
                executable_dir: Some(Path::new("/opt/musializer")),
                home_dir: Some(Path::new("/home/example")),
            },
            probe_of(
                &[
                    "/opt/musializer/models",
                    "/home/example/.local/share/musializer/models",
                ],
                &seen,
            ),
        )
        .unwrap();
        assert_eq!(
            resolved.path,
            Path::new("/home/example/.local/share/musializer/models")
        );
        assert_eq!(resolved.source, ModelsDirSource::SettingsOverride);
        assert!(resolved.is_override());
        assert!(resolved.writable);
        // And the default it beat is still reported, writable and all.
        assert!(resolved.install_default_writable);
        assert_eq!(
            resolved.install_default.as_deref(),
            Some(Path::new("/opt/musializer/models"))
        );
    }

    #[test]
    fn an_unwritable_override_still_wins_and_says_so() {
        // Substituting here would move a user's weights to a directory they
        // never chose, which is the failure the operator rule names.
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(
            ModelsDirRequest {
                configured: "/mnt/detached/models",
                executable_dir: Some(Path::new("/opt/musializer")),
                home_dir: Some(Path::new("/home/example")),
            },
            probe_of(&["/opt/musializer/models"], &seen),
        )
        .unwrap();
        assert_eq!(resolved.path, Path::new("/mnt/detached/models"));
        assert_eq!(resolved.source, ModelsDirSource::SettingsOverride);
        assert!(!resolved.writable);
    }

    #[test]
    fn whitespace_is_not_an_override() {
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(
            ModelsDirRequest {
                configured: "   ",
                executable_dir: Some(Path::new("/opt/musializer")),
                home_dir: None,
            },
            probe_of(&["/opt/musializer/models"], &seen),
        )
        .unwrap();
        assert_eq!(resolved.source, ModelsDirSource::InstallDefault);
        assert_eq!(resolved.configured, None);
    }

    #[test]
    fn a_read_only_install_with_no_home_is_an_error_naming_the_path() {
        let seen = RefCell::new(Vec::new());
        let error = resolve(
            ModelsDirRequest {
                configured: "",
                executable_dir: Some(Path::new("/usr/lib/musializer")),
                home_dir: None,
            },
            probe_of(&[], &seen),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ModelsDirError::NoWritableDefault(PathBuf::from("/usr/lib/musializer/models"))
        );
        assert!(error.to_string().contains("/usr/lib/musializer/models"));
    }

    #[test]
    fn nothing_known_at_all_is_the_other_error() {
        let seen = RefCell::new(Vec::new());
        let error = resolve(
            ModelsDirRequest {
                configured: "",
                executable_dir: None,
                home_dir: None,
            },
            probe_of(&[], &seen),
        )
        .unwrap_err();
        assert_eq!(error, ModelsDirError::NoCandidate);
    }

    #[test]
    fn the_request_reads_the_override_out_of_the_settings_record() {
        let mut settings = AssistSettings::default();
        settings.local_runtimes.models_dir = "/srv/weights".to_string();
        let executable_dir = PathBuf::from("/opt/musializer");
        let request = ModelsDirRequest::from_settings(&settings, Some(&executable_dir), None);
        assert_eq!(request.configured, "/srv/weights");
        let seen = RefCell::new(Vec::new());
        let resolved = resolve(request, probe_of(&[], &seen)).unwrap();
        assert_eq!(resolved.path, Path::new("/srv/weights"));

        // And an untouched record leaves the default in charge.
        let default = AssistSettings::default();
        let request = ModelsDirRequest::from_settings(&default, Some(&executable_dir), None);
        assert_eq!(request.configured, "");
    }

    #[test]
    fn source_tokens_are_the_spelling_the_doctor_prints() {
        assert_eq!(
            ModelsDirSource::SettingsOverride.token(),
            "settings-override"
        );
        assert_eq!(ModelsDirSource::InstallDefault.token(), "install-default");
        assert_eq!(ModelsDirSource::HomeFallback.token(), "home-fallback");
    }
}
