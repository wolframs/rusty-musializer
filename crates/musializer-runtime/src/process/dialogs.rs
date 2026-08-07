//! Native file dialogs, as supervised child processes.
//!
//! **Owner: Agent E.** The C uses `../musializer/thirdparty/tinyfiledialogs.c`,
//! which is upstream third-party code, not first-party source. The plan lists
//! keeping it through FFI as *"genuinely provisional — revisit when convenient"*
//! and Phase 5 as the point at which the choice is made
//! (`REWRITE_PLAN.md`, "Keep, but genuinely provisional" and "Phase 5").
//!
//! This module's own stub laid out three candidate backends and named the second
//! as the one that "fits this module's existing machinery exactly": **`kdialog`
//! or `zenity` as a child process** — spawn, read one line of stdout, reap. That
//! is what is implemented here, and the reasoning stands: it needs no new
//! dependency, it is exactly what tinyfiledialogs itself does on Linux, and it is
//! the only one of the three whose failure mode is a machine that has neither
//! helper installed, which [`DialogError::Unavailable`] already describes.
//!
//! Two behaviours the call sites depend on, preserved:
//!
//! - **Cancellation is `Ok(None)`, not an error.** Every C call site treats
//!   tinyfiledialogs' `NULL` as "the user changed their mind" and does nothing.
//! - **A returned path is untrusted input**, validated by whoever consumes it
//!   (`src/plug.c:2147` bounds the chosen lyrics file at 256 KiB before reading
//!   it).
//!
//! ## The display guard is not optional
//!
//! [`backend`] refuses to spawn anything unless `DISPLAY` or `WAYLAND_DISPLAY` is
//! set, and that check is the most important line in the file. `kdialog` with no
//! reachable display does not print an error and exit — it **aborts with
//! `SIGABRT`**, which on Ubuntu summons an Apport "internal error" report for a
//! crash the caller caused. This project has already done that to its operator
//! once, from a two-line shell test; `tools/rusty-musializer-launcher` carries the
//! same guard and `AGENTS.md` carries the trap.
//!
//! Nothing in this module is exercised by `cargo test`. A dialog opened by a test
//! run would appear on the operator's real desktop, so the policy is factored into
//! [`choose_backend`], which is pure, and that is what the tests drive.
//!
//! ## The call sites this satisfies
//!
//! | Site | Kind | Title | Filter label |
//! | --- | --- | --- | --- |
//! | `src/plug.c:2143` | open | Choose authored lyrics | lyric text |
//! | `src/plug.c:4634` | save | Save Musializer project | Musializer project |
//! | `src/plug.c:5049` | open | Open Musializer project | Musializer project |
//! | `src/plug.c:5199` | open | Add audio | audio files |
//! | `src/plug.c:6373` | open | Image for ASCII Field | image files |
//! | `src/plug.c:7132` | save | Export video | MP4 video |
//! | `src/plug.c:7248` | yes/no | Unresolved Musializer work | — |
//! | `src/plug.c:7793` | open | Open audio | audio files |
//!
//! Note that `:7793` passes `allow_multiple_selects` and then uses the result as
//! a single path, so multiple selection is not actually supported by the caller.
//! [`FileDialog`] does not offer it.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

/// Why a dialog produced nothing.
#[derive(Debug, thiserror::Error)]
pub enum DialogError {
    /// There is no reachable display, or neither helper is installed.
    ///
    /// Callers must treat it as "the user cannot be asked here" and fall back to
    /// something typed — the CLI accepts every path the dialogs pick, so no
    /// feature is unreachable without them, only less convenient.
    #[error("no native file dialog is available: {0}")]
    Unavailable(&'static str),

    /// The helper could not be started.
    #[error("could not run {program}: {source}")]
    Spawn {
        program: &'static str,
        #[source]
        source: std::io::Error,
    },

    /// The helper ran and neither chose nor cancelled.
    ///
    /// Distinct from cancellation on purpose. A caller that cannot tell "the user
    /// said no" from "the picker broke" will silently discard work in the second
    /// case, which is the mistake [`confirm_warning`]'s doc comment warns about.
    #[error("{program} exited with {status}")]
    Failed {
        program: &'static str,
        status: String,
    },
}

/// One filter row: a human label and the glob patterns it covers.
///
/// tinyfiledialogs takes patterns and a single description
/// (`tinyfd_openFileDialog(title, default, count, patterns, description, multi)`),
/// which is why `label` is per-dialog rather than per-pattern in the C. Kept as a
/// pair here so a richer backend can present it properly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    pub label: &'static str,
    pub patterns: &'static [&'static str],
}

/// The filters the oracle's call sites use, so a backend does not have to
/// rediscover them and two panels cannot disagree about what an audio file is.
pub mod filters {
    use super::FileFilter;

    pub const MUSIALIZER_PROJECT: FileFilter = FileFilter {
        label: "Musializer project",
        patterns: &["*.musi"],
    };
    /// The seven extensions the welcome screen and the "Add audio" site offer, in
    /// the oracle's order (`plug.c:7791-7792`, `:5197`).
    ///
    /// The same seven the welcome screen prints along its bottom edge and the same
    /// seven the desktop entry claims a MIME type for, because a picker that
    /// filters out a format the application can open is worse than no filter.
    pub const AUDIO: FileFilter = FileFilter {
        label: "audio files",
        patterns: &[
            "*.wav", "*.ogg", "*.mp3", "*.qoa", "*.xm", "*.mod", "*.flac",
        ],
    };
    pub const LYRIC_TEXT: FileFilter = FileFilter {
        label: "lyric text",
        patterns: &["*.txt", "*.lyrics.txt"],
    };
    pub const MP4_VIDEO: FileFilter = FileFilter {
        label: "MP4 video",
        patterns: &["*.mp4"],
    };
    /// The formats ASCII Field's image import accepts (`plug.c:6362-6366`).
    ///
    /// Exactly the four the drop path classifies as images
    /// (`ui::shell::classify_drop`), and that is the contract rather than a
    /// coincidence: a picker that offers a fifth would produce a file the drop
    /// path sends to the audio decoder.
    pub const ASCII_IMAGE: FileFilter = FileFilter {
        label: "images",
        patterns: &["*.png", "*.jpg", "*.jpeg", "*.bmp"],
    };
}

/// A dialog request. Built by the caller, run by whatever backend exists.
#[derive(Debug, Clone, Default)]
pub struct FileDialog {
    pub title: String,
    /// The suggested path or starting directory. `"./"` at every C open site.
    pub default_path: PathBuf,
    pub filters: Vec<FileFilter>,
}

impl FileDialog {
    pub fn new(title: impl Into<String>) -> Self {
        FileDialog {
            title: title.into(),
            default_path: PathBuf::from("./"),
            filters: Vec::new(),
        }
    }

    pub fn with_default_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.default_path = path.into();
        self
    }

    pub fn with_filter(mut self, filter: FileFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Asks for an existing file. `Ok(None)` is cancellation.
    pub fn pick_file(&self) -> Result<Option<PathBuf>, DialogError> {
        match backend()? {
            Backend::KDialog => run_for_path(
                "kdialog",
                &[
                    OsStr::new("--title"),
                    OsStr::new(&self.title),
                    OsStr::new("--getopenfilename"),
                    self.default_path.as_os_str(),
                    OsStr::new(&self.kdialog_filter()),
                ],
            ),
            Backend::Zenity => {
                let mut arguments = vec![
                    format!("--title={}", self.title),
                    "--file-selection".to_string(),
                    format!("--filename={}", self.zenity_filename()),
                ];
                arguments.extend(self.zenity_filters());
                run_for_path("zenity", &os_args(&arguments))
            }
        }
    }

    /// Asks where to write. `Ok(None)` is cancellation.
    pub fn save_file(&self) -> Result<Option<PathBuf>, DialogError> {
        match backend()? {
            Backend::KDialog => run_for_path(
                "kdialog",
                &[
                    OsStr::new("--title"),
                    OsStr::new(&self.title),
                    OsStr::new("--getsavefilename"),
                    self.default_path.as_os_str(),
                    OsStr::new(&self.kdialog_filter()),
                ],
            ),
            Backend::Zenity => {
                let mut arguments = vec![
                    format!("--title={}", self.title),
                    "--file-selection".to_string(),
                    "--save".to_string(),
                    // Without this, zenity silently overwrites. kdialog asks by
                    // default, so the two backends only agree if this is passed.
                    "--confirm-overwrite".to_string(),
                    format!("--filename={}", self.zenity_filename()),
                ];
                arguments.extend(self.zenity_filters());
                run_for_path("zenity", &os_args(&arguments))
            }
        }
    }

    /// kdialog takes one filter string, `"pat pat|label"` per row, rows separated
    /// by newlines. Empty when there are no filters, which kdialog reads as "any
    /// file".
    fn kdialog_filter(&self) -> String {
        self.filters
            .iter()
            .map(|filter| format!("{}|{}", filter.patterns.join(" "), filter.label))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// zenity takes one `--file-filter` per row, `"label | pat pat"`.
    fn zenity_filters(&self) -> Vec<String> {
        self.filters
            .iter()
            .map(|filter| {
                format!(
                    "--file-filter={} | {}",
                    filter.label,
                    filter.patterns.join(" ")
                )
            })
            .collect()
    }

    /// zenity treats a `--filename` with a trailing separator as a starting
    /// directory and one without as a suggested name. The C's `"./"` is the
    /// former, and dropping the slash would make every open dialog suggest a file
    /// literally named `.`.
    fn zenity_filename(&self) -> String {
        let path = self.default_path.to_string_lossy().into_owned();
        if self.default_path.is_dir() && !path.ends_with('/') {
            format!("{path}/")
        } else {
            path
        }
    }
}

/// A yes/no confirmation, `tinyfd_messageBox(…, "yesno", "warning", 0)`.
///
/// The one C call site guards quitting with unsaved work (`src/plug.c:7248`),
/// which is why a missing backend is an `Err` and **not** `false` and **not**
/// `true`: the caller has to decide, and the safe decision there is to keep the
/// work. Returning a bool would make "nobody could be asked" indistinguishable
/// from "the user said no".
pub fn confirm_warning(title: &str, message: &str) -> Result<bool, DialogError> {
    let (program, arguments) = match backend()? {
        Backend::KDialog => (
            "kdialog",
            vec![
                "--title".to_string(),
                title.to_string(),
                "--warningyesno".to_string(),
                message.to_string(),
            ],
        ),
        Backend::Zenity => (
            "zenity",
            vec![
                format!("--title={title}"),
                "--question".to_string(),
                format!("--text={message}"),
            ],
        ),
    };
    let output = Command::new(program)
        .args(&arguments)
        .output()
        .map_err(|source| DialogError::Spawn { program, source })?;
    // Both helpers answer through the exit status: 0 is yes, 1 is no, anything
    // else did not ask.
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(DialogError::Failed {
            program,
            status: output.status.to_string(),
        }),
    }
}

/// Whether a dialog can be opened right now.
///
/// A panel should use this to decide whether to draw a "Browse…" button at all,
/// rather than drawing one that always fails. A feature nobody can find is a
/// feature nobody has, and a button that never works is worse than no button.
#[must_use]
pub fn dialogs_available() -> bool {
    backend().is_ok()
}

/// The helper this machine can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    /// Preferred where present: on a Plasma session it is the picker the rest of
    /// the desktop uses, and it is the one this project's launcher already
    /// depends on.
    KDialog,
    Zenity,
}

fn backend() -> Result<Backend, DialogError> {
    let display = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    choose_backend(display, on_path("kdialog"), on_path("zenity"))
}

/// The backend policy, with the environment passed in.
///
/// Pure so it can be tested. Calling the real [`backend`] in a test would be
/// harmless, but calling anything downstream of it would open a dialog on the
/// operator's desktop, so the split is worth keeping even though only this half
/// is asserted.
fn choose_backend(display: bool, kdialog: bool, zenity: bool) -> Result<Backend, DialogError> {
    // First, and never reordered. See the module comment: kdialog with no display
    // aborts under SIGABRT rather than failing, and Ubuntu files a crash report
    // for it.
    if !display {
        return Err(DialogError::Unavailable(
            "no display: DISPLAY and WAYLAND_DISPLAY are both unset",
        ));
    }
    if kdialog {
        return Ok(Backend::KDialog);
    }
    if zenity {
        return Ok(Backend::Zenity);
    }
    Err(DialogError::Unavailable(
        "neither kdialog nor zenity is installed",
    ))
}

/// Whether a helper is executable somewhere on `PATH`.
///
/// Hand-rolled rather than shelling out to `which`, which would be a second
/// process to answer a question about the first.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(program);
        // `is_file` rather than `exists`: a directory named `kdialog` on PATH is
        // not a program.
        candidate.is_file()
    })
}

/// Runs a picker and interprets its answer.
///
/// This blocks the calling thread, and therefore the render loop, for as long as
/// the dialog is open. tinyfiledialogs blocks the C the same way, and a modal
/// picker over a frozen window is what every other application on the desktop
/// does too — but it does mean the window stops repainting, which is worth knowing
/// before someone reports it as a hang.
fn run_for_path(
    program: &'static str,
    arguments: &[&OsStr],
) -> Result<Option<PathBuf>, DialogError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|source| DialogError::Spawn { program, source })?;
    match output.status.code() {
        Some(0) => {
            // Both helpers print the path followed by a newline. A zero exit with
            // nothing on stdout is cancellation as far as a caller is concerned:
            // there is no path, so there is nothing to act on.
            let chosen = String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string();
            if chosen.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(chosen)))
            }
        }
        // Cancellation, in both. Deliberately not an error.
        Some(1) => Ok(None),
        _ => Err(DialogError::Failed {
            program,
            status: output.status.to_string(),
        }),
    }
}

fn os_args(arguments: &[String]) -> Vec<&OsStr> {
    arguments.iter().map(OsStr::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_display_means_no_dialog_whatever_is_installed() {
        // The single most important assertion in this file. Spawning kdialog with
        // no display does not fail politely — it aborts, and Ubuntu's Apport
        // reports the crash to the user as an application bug. This has happened
        // once already, so it is pinned rather than remembered.
        for (kdialog, zenity) in [(true, true), (true, false), (false, true), (false, false)] {
            assert!(
                matches!(
                    choose_backend(false, kdialog, zenity),
                    Err(DialogError::Unavailable(_))
                ),
                "a dialog was offered with no display (kdialog={kdialog}, zenity={zenity})"
            );
        }
    }

    #[test]
    fn kdialog_wins_where_both_are_installed() {
        assert_eq!(choose_backend(true, true, true).unwrap(), Backend::KDialog);
        assert_eq!(choose_backend(true, false, true).unwrap(), Backend::Zenity);
        assert_eq!(choose_backend(true, true, false).unwrap(), Backend::KDialog);
        assert!(choose_backend(true, false, false).is_err());
    }

    #[test]
    fn the_builder_defaults_to_the_oracles_starting_directory() {
        let dialog = FileDialog::new("Add audio");
        assert_eq!(dialog.default_path, PathBuf::from("./"));
        let dialog = dialog.with_default_path("/tmp/song.mp3");
        assert_eq!(dialog.default_path, PathBuf::from("/tmp/song.mp3"));
    }

    #[test]
    fn each_backend_gets_its_own_filter_syntax() {
        // The two helpers disagree about which side of the bar the label goes on,
        // and getting it backwards does not fail — it produces a picker that
        // offers a filter named `*.wav *.mp3`, which looks like a typo rather
        // than a bug.
        let dialog = FileDialog::new("Open audio")
            .with_filter(super::filters::AUDIO)
            .with_filter(super::filters::MUSIALIZER_PROJECT);
        let kdialog = dialog.kdialog_filter();
        assert!(kdialog.starts_with("*.wav "), "{kdialog}");
        assert!(kdialog.contains("|audio files"), "{kdialog}");
        assert_eq!(kdialog.lines().count(), 2);

        let zenity = dialog.zenity_filters();
        assert_eq!(zenity.len(), 2);
        assert!(
            zenity[0].starts_with("--file-filter=audio files | *.wav"),
            "{:?}",
            zenity[0]
        );
    }

    #[test]
    fn a_starting_directory_keeps_its_trailing_separator_for_zenity() {
        // Without it zenity suggests a file named after the directory instead of
        // opening in it, and the C's default path is exactly `"./"`.
        let dialog = FileDialog::new("Add audio");
        assert!(dialog.zenity_filename().ends_with('/'));
        let dialog = FileDialog::new("Export video").with_default_path("/tmp/out.mp4");
        assert_eq!(dialog.zenity_filename(), "/tmp/out.mp4");
    }
}
