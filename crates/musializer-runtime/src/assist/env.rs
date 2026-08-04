//! The startup credential import, and taking the key back out of the process.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §4 E1 and §7 decision 4. The helper
//! already defends itself — `external_analysis.py::_safe_local_env` strips every
//! variable whose name carries a credential marker, and only `_openrouter_env`
//! re-adds the one authorized key — but that only protects the children *it*
//! launches. This application also spawns `ffmpeg`, `kdialog`/`zenity`, `codex`
//! and the analysis helpers directly, and every one of them inherits this
//! process's environment.
//!
//! So the key is read **once**, at the top of `main`, into a
//! [`SessionCredentials`] value, and then removed from the environment. After
//! that there is no environment variable for a child to inherit by accident, and
//! the only remaining copy is the [`Secret`], which has one owner and is
//! zeroized on drop.
//!
//! ## Why `/proc/self/environ` still shows the key, and why that is not a hole
//!
//! On Linux `/proc/<pid>/environ` is read from the region the kernel populated
//! at `execve`, not from the live `environ` array. `unsetenv` cannot erase it, so
//! *this* process's `/proc/self/environ` keeps whatever it was started with. What
//! changes is what a **child** gets: a child's environment is built from the live
//! array at `posix_spawn` time, so it never sees the variable at all, and its own
//! `/proc/self/environ` does not contain it. That child-side check is the
//! evidence `tools/secret_canary_check.sh` measures, because it is the one that
//! corresponds to the exposure being prevented.

use musializer_core::assist::secret::Secret;

/// The one credential variable this application imports.
pub const OPENROUTER_KEY_VARIABLE: &str = "OPENROUTER_API_KEY";

/// The name markers `external_analysis.py::_safe_local_env` strips, duplicated
/// here rather than shared.
///
/// Duplicated on purpose, for the reason the differential harnesses duplicate
/// their fixture generators: a shared list would hide a drift between the two
/// sides, and this list is small enough that a drift is caught by reading it.
const CREDENTIAL_MARKERS: [&str; 6] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"];

/// Whether a variable name looks like it carries a credential.
///
/// Used to *report* what a child can still see, not to decide what to strip —
/// blanket-stripping the parent environment would break `SSH_AUTH_SOCK`,
/// `XAUTHORITY` and anything else a desktop session needs.
#[must_use]
pub fn is_credential_named(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    CREDENTIAL_MARKERS
        .iter()
        .any(|marker| upper.contains(marker))
}

/// Every credential-named variable currently visible in an environment snapshot,
/// sorted. The canary probe prints this for the child it spawns.
#[must_use]
pub fn credential_named_variables<'a, I>(environment: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut names: Vec<String> = environment
        .into_iter()
        .filter(|name| is_credential_named(name))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Credentials held for this run only. Nothing here is written anywhere.
#[derive(Debug, Default)]
pub struct SessionCredentials {
    openrouter: Option<Secret>,
}

impl SessionCredentials {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Replaces the OpenRouter credential. Takes ownership; there is no way to
    /// copy one back out.
    pub fn set_openrouter(&mut self, secret: Option<Secret>) {
        self.openrouter = secret;
    }

    #[must_use]
    pub fn openrouter(&self) -> Option<&Secret> {
        self.openrouter.as_ref()
    }

    #[must_use]
    pub fn has_openrouter(&self) -> bool {
        self.openrouter.is_some()
    }

    /// `sha256(secret)[0..8]` — the only identity safe to display or log (§3).
    #[must_use]
    pub fn openrouter_fingerprint(&self) -> Option<String> {
        self.openrouter.as_ref().map(Secret::fingerprint)
    }

    /// A line safe to print anywhere, including a diagnostic dump the user may
    /// paste into a bug report (E4, E9).
    #[must_use]
    pub fn describe(&self) -> String {
        match self.openrouter_fingerprint() {
            Some(fingerprint) => {
                format!("openrouter credential imported for this session ({fingerprint})")
            }
            None => "no openrouter credential in this session".to_string(),
        }
    }
}

/// Reads `OPENROUTER_API_KEY` once and removes it from this process's
/// environment.
///
/// An empty or whitespace-only value imports nothing but is still removed, so
/// `OPENROUTER_API_KEY=` in a desktop file cannot leave a blank credential for a
/// child to find and misreport as configured.
///
/// # Safety
///
/// `std::env::remove_var` is `unsafe` in edition 2024 because the C environment
/// is process-global and `getenv` may hand out a pointer another thread is
/// reading. The caller must guarantee that **no other thread exists yet** — in
/// practice this means the first statements of `main`, before the window, the
/// audio device, any `std::thread::spawn`, and before any library that starts a
/// thread of its own. `musializer-app`'s `main` calls it there and nowhere else.
#[must_use]
pub unsafe fn import_session_credentials() -> SessionCredentials {
    let imported = std::env::var(OPENROUTER_KEY_VARIABLE).ok();
    // SAFETY: the caller of this `unsafe fn` has promised no other thread is
    // running, which is the whole of `remove_var`'s contract. Nothing in this
    // function reads the environment after the removal, and the value has
    // already been copied into an owned `String`.
    unsafe {
        std::env::remove_var(OPENROUTER_KEY_VARIABLE);
    }
    let mut session = SessionCredentials::empty();
    session.set_openrouter(
        imported
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(Secret::new),
    );
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_list_matches_the_helpers() {
        // The names `_safe_local_env` strips, spelled the way a real environment
        // spells them.
        for name in [
            "OPENROUTER_API_KEY",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "PGPASSWORD",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "SSH_AUTH_SOCK",
            "openrouter_api_key",
        ] {
            assert!(is_credential_named(name), "{name} was not recognised");
        }
        for name in ["HOME", "PATH", "XDG_CONFIG_HOME", "DISPLAY", "LANG"] {
            assert!(!is_credential_named(name), "{name} was recognised");
        }
    }

    #[test]
    fn credential_named_variables_are_sorted_and_deduplicated() {
        let names = credential_named_variables(["PATH", "GITHUB_TOKEN", "HOME", "API_KEY"]);
        assert_eq!(
            names,
            vec!["API_KEY".to_string(), "GITHUB_TOKEN".to_string()]
        );
    }

    #[test]
    fn a_session_without_a_credential_says_so_without_inventing_one() {
        let session = SessionCredentials::empty();
        assert!(!session.has_openrouter());
        assert_eq!(session.openrouter_fingerprint(), None);
        assert_eq!(
            session.describe(),
            "no openrouter credential in this session"
        );
    }

    #[test]
    fn a_session_describes_a_credential_by_fingerprint_only() {
        let planted = "sk-or-v1-MUSICANARY7Q4X2ZK9aaaabbbbcccc";
        let mut session = SessionCredentials::empty();
        session.set_openrouter(Some(Secret::new(planted.to_string())));
        let described = session.describe();
        assert!(described.contains(&session.openrouter_fingerprint().unwrap()));
        assert!(!described.contains("MUSICANARY"));
        assert!(!described.contains("sk-or"));
        assert!(!format!("{session:?}").contains("MUSICANARY"));
    }
}
