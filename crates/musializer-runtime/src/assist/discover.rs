//! Where an external executable actually is, and how it was found.
//!
//! `PATH` alone is wrong, and it is wrong in the one case that matters most: a
//! GUI process started from a desktop entry inherits the session manager's
//! minimal `PATH`, not the login shell's. On this operator's machine `codex`
//! lives in `~/.local/npm-global/bin` — on the login shell's `PATH` and on no
//! desktop entry's — so the dialog said "codex not on PATH" about a binary the
//! user can run by typing its name. A discovery answer that is right in a
//! terminal and wrong under the launcher is worse than no answer, because it
//! sends the user to fix something that is not broken.
//!
//! So the ladder, in order, with the first hit winning:
//!
//! 1. **An explicit user override.** Always wins, and a set-but-missing
//!    override is a *loud failure* rather than a fallback: the precedent is
//!    `local_runtimes.whisper_bin`, and silently ignoring a path the user typed
//!    is how a setting comes to look like it does nothing.
//! 2. **`PATH` as inherited.** Correct whenever the process was started from a
//!    shell, which is every developer run.
//! 3. **A documented list of common install locations**, expanded from `$HOME`.
//!    Written down in [`well_known_dirs`] rather than guessed per caller, so
//!    "where did you look" is answerable.
//! 4. **The login shell's own `PATH`** (`$SHELL -lc 'command -v <name>'`). This
//!    is the step that fixes the desktop-entry case *generally* — it finds a
//!    binary in a directory nobody here has heard of, because the user's own
//!    rc files put it there.
//!
//! Steps 3 and 4 can spawn a child (`npm config get prefix`, and the shell
//! itself), so they are gated behind [`Options::allow_subprocess`] and every
//! child runs under a hard timeout. **Never call this with
//! `allow_subprocess` on from a frame callback**: use [`resolve_cached`] from a
//! background thread and read the cache from the UI, which is what
//! `ui::assist_settings` does. The cache is per process, so the shell runs once
//! per session at most.
//!
//! Nothing here reads a provider's authentication files, and no child inherits
//! a credential-named variable (`docs/ASSIST_PROVIDER_CONTRACTS.md` §4 E1).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How the answer was reached. Shown to the user, because "we found it in your
/// login shell's PATH" and "it was on this process's PATH" call for different
/// fixes when the answer is later wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// `local_runtimes` named it explicitly.
    Override,
    /// This process's inherited `PATH`.
    Path,
    /// One of [`well_known_dirs`].
    WellKnown,
    /// `$SHELL -lc 'command -v <name>'`.
    LoginShell,
}

impl Method {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::Path => "PATH",
            Self::WellKnown => "well-known",
            Self::LoginShell => "login-shell",
        }
    }

    /// The sentence the Codex section prints beside the path.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::Override => "set explicitly in AI settings",
            Self::Path => "on this process's PATH",
            Self::WellKnown => "in a common install location",
            Self::LoginShell => "on your login shell's PATH, not this process's",
        }
    }
}

/// What the ladder concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Found {
        path: PathBuf,
        method: Method,
    },
    /// An override was set and is not a runnable file. **Never** followed by a
    /// fallback: the user named a path and is owed the failure.
    OverrideMissing {
        path: PathBuf,
        reason: String,
    },
    NotFound,
}

/// One resolution, with the evidence behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    pub name: String,
    pub outcome: Outcome,
    /// Every directory and mechanism consulted, in the order they were tried.
    /// This is what a "not found" answer shows the user, so it is collected even
    /// on the success path — a wrong hit is diagnosed by seeing what came first.
    pub searched: Vec<String>,
    /// Whether a child process was spawned. The UI reports it, because a run
    /// with `allow_subprocess` off that found nothing has not finished looking.
    pub consulted_subprocess: bool,
}

impl Discovery {
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.outcome {
            Outcome::Found { path, .. } => Some(path.as_path()),
            _ => None,
        }
    }

    #[must_use]
    pub fn method(&self) -> Option<Method> {
        match &self.outcome {
            Outcome::Found { method, .. } => Some(*method),
            _ => None,
        }
    }

    /// A one-line report token: `codex=/x/y via=PATH` or
    /// `codex=absent searched=11`.
    #[must_use]
    pub fn summary(&self) -> String {
        match &self.outcome {
            Outcome::Found { path, method } => {
                format!("{}={} via={}", self.name, path.display(), method.token())
            }
            Outcome::OverrideMissing { path, reason } => format!(
                "{}=override-missing({}) reason=\"{reason}\"",
                self.name,
                path.display()
            ),
            // Deliberately bare. How many places were looked in, and whether a
            // subprocess rung ran, are the caller's fields — a summary that
            // repeated them printed each twice in the dialog's report line.
            Outcome::NotFound => format!("{}=absent", self.name),
        }
    }
}

/// Knobs the caller has to choose deliberately.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Whether steps that spawn a child may run. Off on the UI thread.
    pub allow_subprocess: bool,
    /// The wall clock each child gets before it is killed.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_subprocess: false,
            // Long enough for a login shell to source a normal rc file, short
            // enough that a pathological one does not become a hang the user
            // has to explain.
            timeout: Duration::from_millis(2500),
        }
    }
}

impl Options {
    /// The background-thread configuration: the whole ladder, children allowed.
    #[must_use]
    pub fn thorough() -> Self {
        Self {
            allow_subprocess: true,
            ..Self::default()
        }
    }
}

/// Whether a path is a file this process could execute.
///
/// `is_file` alone is not enough — `~/.local/bin/codex` may be a stale symlink
/// or a non-executable leftover, and offering it would turn a discovery bug into
/// a spawn failure further downstream. Symlinks are followed on purpose: the npm
/// global prefix installs its entry points as symlinks into `node_modules`.
#[must_use]
pub fn is_executable_file(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// The documented common install locations, in search order.
///
/// A pure function of the home directory and whatever npm reported, so the list
/// is testable without a home directory and without npm. Duplicates are the
/// caller's problem — [`resolve`] deduplicates against `PATH`.
#[must_use]
pub fn well_known_dirs(home: Option<&Path>, npm_prefix: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".local/npm-global/bin"));
        dirs.push(home.join(".npm-global/bin"));
    }
    if let Some(prefix) = npm_prefix {
        dirs.push(prefix.join("bin"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".asdf/shims"));
        dirs.extend(nvm_bin_dirs(home));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    if let Some(home) = home {
        dirs.push(home.join(".cargo/bin"));
    }
    dirs
}

/// nvm's installed node versions, newest directory name first.
///
/// nvm has no "current version" on disk that a non-shell process can read — it
/// is a shell function that rewrites `PATH` — so the honest answer is every
/// version it has, ordered so the newest is tried first. The login-shell step is
/// what gets the *actually* selected one.
fn nvm_bin_dirs(home: &Path) -> Vec<PathBuf> {
    let root = home.join(".nvm/versions/node");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    names.sort();
    names.reverse();
    names.into_iter().map(|path| path.join("bin")).collect()
}

/// `npm config get prefix`, when npm is reachable and children are allowed.
fn npm_prefix(options: Options) -> Option<PathBuf> {
    let mut command = Command::new("npm");
    command.args(["config", "get", "prefix"]);
    let output = run_briefly(command, options.timeout).ok()?;
    let text = String::from_utf8_lossy(&output);
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('/'))?;
    Some(PathBuf::from(line))
}

/// `$SHELL -lc 'command -v <name>'`, bounded.
///
/// A login shell sources the user's own rc files, which is the entire point:
/// this finds a binary in a directory this module has never heard of. It is also
/// the only step that runs arbitrary user configuration, so it is last, it is
/// bounded, and it accepts only an absolute path to an executable back.
fn login_shell_lookup(name: &str, options: Options) -> Option<PathBuf> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        // Nothing here builds a name from user input, and it will stay that way
        // because a name that could carry a quote never reaches a shell.
        return None;
    }
    let shell = std::env::var_os("SHELL")?;
    if shell.is_empty() {
        return None;
    }
    let mut command = Command::new(&shell);
    command.arg("-lc").arg(format!("command -v {name}"));
    let output = run_briefly(command, options.timeout).ok()?;
    // An rc file may print a banner, so the answer is "the first line that is an
    // absolute path to something runnable" rather than "the last line".
    String::from_utf8_lossy(&output)
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .find(|path| is_executable_file(path))
}

/// Runs a child, kills it at the deadline, and returns its stdout.
///
/// `Child::wait_with_output` has no timeout and `std` has no bounded wait, so
/// this polls `try_wait`. stdout is a pipe read *after* exit: every command here
/// prints one short line, so it cannot fill the pipe buffer and deadlock, and a
/// command that tried would be killed at the deadline anyway.
fn run_briefly(mut command: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    strip_credential_variables(&mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("exited with {status}"));
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut bytes = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut bytes);
    }
    Ok(bytes)
}

/// The same rule `external_analysis.py::_safe_local_env` applies to its own
/// children (E1). None of these commands needs a credential, and `npm` and a
/// login shell are exactly the kind of child that would log its environment.
fn strip_credential_variables(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if super::env::is_credential_named(&name) {
            command.env_remove(name.as_ref());
        }
    }
}

/// The first `PATH` entry holding an executable called `name`.
#[must_use]
pub fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable_file(candidate))
}

/// Runs the whole ladder. See the module docs for the order and the reasons.
///
/// `override_path` is `local_runtimes`' explicit setting, when the user set one.
#[must_use]
pub fn resolve(name: &str, override_path: Option<&str>, options: Options) -> Discovery {
    let mut searched: Vec<String> = Vec::new();
    let mut consulted_subprocess = false;

    // 1. The override, and its loud failure.
    if let Some(raw) = override_path.map(str::trim).filter(|text| !text.is_empty()) {
        let path = PathBuf::from(raw);
        searched.push(format!("override: {raw}"));
        if is_executable_file(&path) {
            return Discovery {
                name: name.to_string(),
                outcome: Outcome::Found {
                    path,
                    method: Method::Override,
                },
                searched,
                consulted_subprocess,
            };
        }
        let reason = if path.exists() {
            "not an executable file"
        } else {
            "no such file"
        };
        return Discovery {
            name: name.to_string(),
            outcome: Outcome::OverrideMissing {
                path,
                reason: reason.to_string(),
            },
            searched,
            consulted_subprocess,
        };
    }

    // 2. PATH.
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    for directory in &path_dirs {
        searched.push(format!("PATH: {}", directory.display()));
        let candidate = directory.join(name);
        if is_executable_file(&candidate) {
            return Discovery {
                name: name.to_string(),
                outcome: Outcome::Found {
                    path: candidate,
                    method: Method::Path,
                },
                searched,
                consulted_subprocess,
            };
        }
    }

    // 3. The documented locations. `npm config get prefix` is the one entry that
    //    needs a child, so without one the list is the static part only — and
    //    the report says a subprocess was not consulted, rather than implying
    //    the search was complete.
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let prefix = if options.allow_subprocess {
        consulted_subprocess = true;
        npm_prefix(options)
    } else {
        None
    };
    for directory in well_known_dirs(home.as_deref(), prefix.as_deref()) {
        if path_dirs.contains(&directory) {
            continue;
        }
        searched.push(directory.display().to_string());
        let candidate = directory.join(name);
        if is_executable_file(&candidate) {
            return Discovery {
                name: name.to_string(),
                outcome: Outcome::Found {
                    path: candidate,
                    method: Method::WellKnown,
                },
                searched,
                consulted_subprocess,
            };
        }
    }

    // 4. The login shell. Last, because it runs the user's own configuration.
    if options.allow_subprocess {
        consulted_subprocess = true;
        searched.push("login shell: $SHELL -lc 'command -v'".to_string());
        if let Some(path) = login_shell_lookup(name, options) {
            return Discovery {
                name: name.to_string(),
                outcome: Outcome::Found {
                    path,
                    method: Method::LoginShell,
                },
                searched,
                consulted_subprocess,
            };
        }
    }

    Discovery {
        name: name.to_string(),
        outcome: Outcome::NotFound,
        searched,
        consulted_subprocess,
    }
}

/// [`resolve`], memoized for the life of the process.
///
/// The cache is what makes the expensive steps safe to have at all: the login
/// shell runs at most once per session per (name, override) pair, however many
/// times the dialog is opened or a frame asks. A resolution that found nothing
/// with `allow_subprocess` off is **not** cached, so the background pass can
/// still improve on it.
#[must_use]
pub fn resolve_cached(name: &str, override_path: Option<&str>, options: Options) -> Discovery {
    static CACHE: OnceLock<Mutex<HashMap<String, Discovery>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{name}\u{0}{}", override_path.unwrap_or(""));
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map.get(&key) {
            return hit.clone();
        }
    }
    let discovery = resolve(name, override_path, options);
    let worth_caching = !matches!(discovery.outcome, Outcome::NotFound) || options.allow_subprocess;
    if worth_caching {
        if let Ok(mut map) = cache.lock() {
            map.insert(key, discovery.clone());
        }
    }
    discovery
}

/// The environment variable name a caller may use to switch the subprocess steps
/// off wholesale.
///
/// The headless gate sets it: a check that shells out to the operator's real
/// login shell would depend on their rc files, and `tools/headless_check.sh`
/// exists to be independent of the session it runs in.
pub const DISCOVERY_VARIABLE: &str = "MUSIALIZER_ASSIST_DISCOVERY";

/// Whether the subprocess steps are permitted at all in this process.
#[must_use]
pub fn subprocess_permitted() -> bool {
    !matches!(
        std::env::var(DISCOVERY_VARIABLE)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "off" | "0" | "path-only"
    )
}

/// The login shell's basename, for reports. Never its path — a report line is
/// read out of context and a home directory in one is noise.
#[must_use]
pub fn shell_label() -> String {
    let Some(value) = std::env::var_os("SHELL") else {
        return "<unset>".to_string();
    };
    Path::new(&value)
        .file_name()
        .unwrap_or(OsStr::new("?"))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented list, in the documented order. Pinned because the Codex
    /// section prints it to the user as "where I looked" — a silent reorder
    /// would change a diagnosis a user acted on.
    #[test]
    fn the_well_known_list_is_the_documented_one() {
        let home = PathBuf::from("/home/example");
        let dirs = well_known_dirs(Some(&home), Some(Path::new("/opt/npm")));
        let names: Vec<String> = dirs.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            names,
            vec![
                "/home/example/.local/bin",
                "/home/example/.local/npm-global/bin",
                "/home/example/.npm-global/bin",
                "/opt/npm/bin",
                "/home/example/.volta/bin",
                "/home/example/.bun/bin",
                "/home/example/.asdf/shims",
                "/usr/local/bin",
                "/opt/homebrew/bin",
                "/home/example/.cargo/bin",
            ]
        );
    }

    /// Without a home directory the list is still valid, and without npm it
    /// simply loses one entry rather than failing.
    #[test]
    fn the_list_degrades_rather_than_failing() {
        assert_eq!(
            well_known_dirs(None, None),
            vec![
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ]
        );
        let home = PathBuf::from("/home/example");
        assert!(!well_known_dirs(Some(&home), None)
            .iter()
            .any(|path| path.starts_with("/opt/npm")));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "musializer-discover-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stub(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A set-but-missing override fails loudly and **does not** fall through to
    /// a binary that happens to be on PATH. Silently ignoring a path the user
    /// typed is how a setting comes to look like it does nothing.
    #[test]
    fn a_set_but_missing_override_refuses_rather_than_falling_back() {
        let dir = scratch("override");
        let real = stub(&dir, "sh");
        let discovery = resolve(
            "sh",
            Some(&dir.join("nowhere/sh").display().to_string()),
            Options::default(),
        );
        assert!(matches!(discovery.outcome, Outcome::OverrideMissing { .. }));
        assert_ne!(discovery.path(), Some(real.as_path()));
        assert_eq!(discovery.searched.len(), 1, "it stopped at the override");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A present override wins over PATH, and says so.
    #[test]
    fn an_override_wins_over_path() {
        let dir = scratch("wins");
        let stubbed = stub(&dir, "musializer-fake-tool");
        let discovery = resolve(
            "musializer-fake-tool",
            Some(&stubbed.display().to_string()),
            Options::default(),
        );
        assert_eq!(discovery.method(), Some(Method::Override));
        assert_eq!(discovery.path(), Some(stubbed.as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A non-executable file is not a hit. It would otherwise be found here and
    /// fail at spawn, which is a much worse error message.
    #[test]
    fn a_non_executable_file_is_not_a_binary() {
        let dir = scratch("mode");
        let path = dir.join("codex");
        std::fs::write(&path, "not a program").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable_file(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable_file(&path));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Not found still answers *where it looked*, which is the whole difference
    /// between a diagnosis and a shrug.
    #[test]
    fn a_missing_binary_reports_every_place_it_looked() {
        let discovery = resolve(
            "musializer-definitely-not-installed",
            None,
            Options::default(),
        );
        assert_eq!(discovery.outcome, Outcome::NotFound);
        assert!(
            discovery.searched.len() > 3,
            "searched only {:?}",
            discovery.searched
        );
        assert!(!discovery.consulted_subprocess);
        assert!(discovery.summary().contains("absent"));
    }

    /// The names that reach a shell are constrained at the boundary, not by
    /// hoping every caller behaves.
    #[test]
    fn a_shell_hostile_name_never_reaches_a_shell() {
        assert_eq!(
            login_shell_lookup("codex; rm -rf /", Options::thorough()),
            None
        );
        assert_eq!(login_shell_lookup("$(id)", Options::thorough()), None);
    }

    /// The bounded runner kills rather than hanging.
    #[test]
    fn a_slow_child_is_killed_at_the_deadline() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let started = Instant::now();
        let outcome = run_briefly(command, Duration::from_millis(250));
        assert_eq!(outcome, Err("timed out".to_string()));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout did not bound the wait"
        );
    }
}
