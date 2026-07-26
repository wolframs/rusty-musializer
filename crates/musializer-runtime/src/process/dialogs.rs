//! Optional native file dialogs. **A deliberate stub.**
//!
//! **Owner: Agent E.** The C uses `../musializer/thirdparty/tinyfiledialogs.c`,
//! which is upstream third-party code, not first-party source. The plan lists
//! keeping it through FFI as *"genuinely provisional — revisit when convenient"*
//! and Phase 5 as the point at which the choice is made
//! (`REWRITE_PLAN.md`, "Keep, but genuinely provisional" and "Phase 5").
//!
//! So this module defines the **call surface** the application needs and returns
//! [`DialogError::Unavailable`] for all of it. That is the honest shape of the
//! decision: the interface is known from the eight call sites in the oracle, the
//! backend is not chosen, and writing FFI for a file picker before the workspace
//! UI exists would be work thrown away by whichever answer Phase 5 gives.
//!
//! ## Whoever implements this
//!
//! There are three plausible backends, in rough order of cost:
//!
//! 1. **A Rust crate** (`rfd` is the obvious one), which brings portals and an
//!    async story and a dependency tree.
//! 2. **`zenity`/`kdialog` as child processes**, which fits this module's
//!    existing machinery exactly — spawn, read one line of stdout, reap — and
//!    needs no new dependency. It fails on a machine with neither installed,
//!    which is what [`DialogError::Unavailable`] already means.
//! 3. **tinyfiledialogs through FFI**, which is what the C does and which is a
//!    `cc` build plus a `*const c_char` surface.
//!
//! Whichever lands, keep the two behaviours the call sites depend on:
//! **cancellation is `Ok(None)`, not an error** — every C call site treats
//! `NULL` as "the user changed their mind" and does nothing — and a returned path
//! is **untrusted input**, validated by whoever consumes it (`src/plug.c:2147`
//! bounds the chosen lyrics file at 256 KiB before reading it).
//!
//! ## The call sites this must eventually satisfy
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

use std::path::PathBuf;

/// Why a dialog produced nothing.
#[derive(Debug, thiserror::Error)]
pub enum DialogError {
    /// No dialog backend is compiled in. This is what every function in this
    /// module currently returns.
    ///
    /// Callers must treat it as "the user cannot be asked here" and fall back to
    /// something typed — the CLI already accepts every path the dialogs pick, so
    /// no feature is unreachable without them, only less convenient.
    #[error("no native file dialog backend is available in this build")]
    Unavailable,
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
    pub const LYRIC_TEXT: FileFilter = FileFilter {
        label: "lyric text",
        patterns: &["*.txt", "*.lyrics.txt"],
    };
    pub const MP4_VIDEO: FileFilter = FileFilter {
        label: "MP4 video",
        patterns: &["*.mp4"],
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
    ///
    /// Currently always [`DialogError::Unavailable`].
    pub fn pick_file(&self) -> Result<Option<PathBuf>, DialogError> {
        Err(DialogError::Unavailable)
    }

    /// Asks where to write. `Ok(None)` is cancellation.
    ///
    /// Currently always [`DialogError::Unavailable`].
    pub fn save_file(&self) -> Result<Option<PathBuf>, DialogError> {
        Err(DialogError::Unavailable)
    }
}

/// A yes/no confirmation, `tinyfd_messageBox(…, "yesno", "warning", 0)`.
///
/// The one C call site guards quitting with unsaved work (`src/plug.c:7248`),
/// which means a missing backend must **not** be read as "yes". Whoever
/// implements this: an `Err` here has to keep the user's work, not discard it.
///
/// Currently always [`DialogError::Unavailable`].
pub fn confirm_warning(_title: &str, _message: &str) -> Result<bool, DialogError> {
    Err(DialogError::Unavailable)
}

/// Whether a dialog backend exists. Always `false` for now.
///
/// A panel should use this to decide whether to draw a "Browse…" button at all,
/// rather than drawing one that always fails. A feature nobody can find is a
/// feature nobody has, and a button that never works is worse than no button.
pub fn dialogs_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_unavailable_rather_than_silently_cancelling() {
        // The distinction matters: `Ok(None)` means the user said no, and a
        // caller may act on that. `Err(Unavailable)` means the user was never
        // asked, and a caller must not act at all.
        let dialog =
            FileDialog::new("Open Musializer project").with_filter(filters::MUSIALIZER_PROJECT);
        assert!(matches!(dialog.pick_file(), Err(DialogError::Unavailable)));
        assert!(matches!(dialog.save_file(), Err(DialogError::Unavailable)));
        assert!(matches!(
            confirm_warning("Unresolved Musializer work", "Discard?"),
            Err(DialogError::Unavailable)
        ));
        assert!(!dialogs_available());
    }

    #[test]
    fn the_builder_defaults_to_the_oracles_starting_directory() {
        let dialog = FileDialog::new("Add audio");
        assert_eq!(dialog.default_path, PathBuf::from("./"));
        let dialog = dialog.with_default_path("/tmp/song.mp3");
        assert_eq!(dialog.default_path, PathBuf::from("/tmp/song.mp3"));
    }
}
