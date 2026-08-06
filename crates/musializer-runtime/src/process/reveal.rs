//! Opening a directory in the user's file manager.
//!
//! **Tranche LX1-f.** The Assist review list tells a user that the full list of
//! flagged lines is "in the job folder", and until now the only way to get there
//! was the Copy button, which puts the path on the clipboard and leaves the rest
//! to them. A path on a clipboard is not a route; it is homework.
//!
//! There is no oracle for this. The frozen C has no reveal of any kind, so the
//! mechanism is ours to choose, and it is chosen the same way
//! [`super::dialogs`] chose `kdialog`/`zenity`: a supervised child process, no
//! new dependency, and a policy function that is pure so it can be tested
//! without anything opening on the operator's desktop.
//!
//! # The display guard is not optional, and neither is the reaping
//!
//! `AGENTS.md` records what a GUI helper does with no reachable display: it does
//! not print an error and exit, it aborts, and Ubuntu files a crash report for a
//! crash the caller caused. This project has already done that to its operator
//! once. So [`reveal_policy`] refuses before anything is spawned, and — because
//! `DISPLAY` alone proves nothing on a Wayland session — it is told about both
//! variables.
//!
//! The child is `/bin/sh`, not `xdg-open`, and that is deliberate. `xdg-open`
//! delegates to a desktop helper that may not return until the file manager
//! window closes, and this call happens on the frame thread: waiting on it would
//! freeze the interface for as long as the user browses. So the shell backgrounds
//! `xdg-open` and exits immediately; the shell itself is waited for (it is
//! already gone), and `xdg-open` is reparented to init rather than left as a
//! zombie this process never reaps. The path is passed as `"$1"` so no quoting or
//! metacharacter in a job folder's name can reach the shell's parser.

use std::path::Path;
use std::process::{Command, Stdio};

/// Why a directory could not be revealed, or that it can be.
///
/// An enum rather than a `Result<(), E>` at the decision point, for
/// `BeatUpdate`'s reason: the interface has to draw the control *before* anyone
/// presses it, and "there is no folder yet", "this machine has no display" and
/// "this is a probe run" are three different sentences a tooltip has to be able
/// to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealState {
    /// The folder exists, a display is reachable and `xdg-open` is installed.
    Ready,
    /// No job folder: the run wrote nothing, or there is no run.
    NoFolder,
    /// Neither `DISPLAY` nor `WAYLAND_DISPLAY` is set.
    NoDisplay,
    /// `xdg-open` is not on `PATH`.
    NoHelper,
    /// A headless probe run. Refused whatever the environment says, because the
    /// gate's Xvfb *is* a reachable display and a file manager opening during a
    /// capture is a process this repository did not ask for.
    Probe,
}

impl RevealState {
    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, RevealState::Ready)
    }

    /// The sentence the control says about itself.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            RevealState::Ready => "Open the job folder in your file manager",
            RevealState::NoFolder => "This run left no job folder to open",
            RevealState::NoDisplay => {
                "No desktop session is reachable, so a file manager cannot be opened"
            }
            RevealState::NoHelper => "xdg-open is not installed, so the folder cannot be opened",
            RevealState::Probe => "Disabled during a headless probe run",
        }
    }
}

/// The policy, with the environment passed in.
///
/// Pure for [`super::dialogs::choose_backend`]'s reason, and it is the stronger
/// half of it here: calling the real [`reveal_state`] in a test is harmless, but
/// calling anything downstream of it would open a file manager on the operator's
/// desktop mid-`cargo test`.
///
/// The order is the order of severity, and `probe` is checked **first**: a gate
/// run has a display and has `xdg-open`, so every other branch would pass.
#[must_use]
pub fn reveal_policy(probe: bool, folder: bool, display: bool, helper: bool) -> RevealState {
    if probe {
        return RevealState::Probe;
    }
    if !folder {
        return RevealState::NoFolder;
    }
    if !display {
        return RevealState::NoDisplay;
    }
    if !helper {
        return RevealState::NoHelper;
    }
    RevealState::Ready
}

/// Whether `path` can be revealed right now.
///
/// `probe` is the caller's, because what counts as a probe run is the
/// application's question and not this module's.
#[must_use]
pub fn reveal_state(path: &Path, probe: bool) -> RevealState {
    reveal_policy(
        probe,
        path.is_dir(),
        display_reachable(),
        on_path("xdg-open"),
    )
}

/// Whether a desktop session is reachable.
///
/// Both variables, not just `DISPLAY`. This operator's session is Wayland, and a
/// check that looks only at `DISPLAY` reports "headless" on the machine where
/// the dialog would in fact appear — and, worse, the reverse under `DISPLAY= `,
/// which is the trap `AGENTS.md` records.
#[must_use]
pub fn display_reachable() -> bool {
    std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
}

/// Whether a helper is executable somewhere on `PATH`.
///
/// A copy of [`super::dialogs`]'s, kept rather than shared: that one is private
/// to a module whose owner is another agent, and three lines of `PATH` walking
/// is a smaller cost than widening someone else's interface.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
}

/// Why a reveal that was attempted did not happen.
#[derive(Debug, thiserror::Error)]
pub enum RevealError {
    /// The policy refused before anything was spawned.
    #[error("{0}")]
    Refused(&'static str),
    /// `/bin/sh` could not be started.
    #[error("could not start the file manager: {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
    /// The shell ran and failed, which means `xdg-open` was never backgrounded.
    #[error("the file manager could not be started: sh exited with {status}")]
    Failed { status: String },
}

/// Opens `path` in the user's file manager, if the policy allows it.
///
/// Returns as soon as the shell has forked `xdg-open`, which is what keeps this
/// callable from the frame thread. A successful return means the request was
/// handed over, not that a window appeared — nothing on this side can observe
/// the second thing, and pretending otherwise would be the "exit 0 proves the
/// renderer works" mistake in a new place.
pub fn reveal_directory(path: &Path, probe: bool) -> Result<(), RevealError> {
    let state = reveal_state(path, probe);
    if !state.is_ready() {
        return Err(RevealError::Refused(state.reason()));
    }
    let status = Command::new("/bin/sh")
        .arg("-c")
        // `exec`-less on purpose: the shell has to survive long enough to
        // background the child and then exit on its own.
        .arg("xdg-open \"$1\" >/dev/null 2>&1 &")
        .arg("sh")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // `status` rather than `spawn`: it waits, and the wait is what reaps the
        // shell. The shell exits the instant it has forked, so this does not
        // block the frame.
        .status()
        .map_err(|source| RevealError::Spawn { source })?;
    if status.success() {
        Ok(())
    } else {
        Err(RevealError::Failed {
            status: status.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_run_is_refused_before_every_other_question_is_asked() {
        // The gate's Xvfb is a reachable display and its PATH has xdg-open, so
        // every other branch passes and only the ordering saves the capture.
        assert_eq!(
            reveal_policy(true, true, true, true),
            RevealState::Probe,
            "a headless probe run must never spawn a file manager"
        );
    }

    #[test]
    fn each_refusal_is_a_different_sentence() {
        assert_eq!(
            reveal_policy(false, false, true, true),
            RevealState::NoFolder
        );
        assert_eq!(
            reveal_policy(false, true, false, true),
            RevealState::NoDisplay
        );
        assert_eq!(
            reveal_policy(false, true, true, false),
            RevealState::NoHelper
        );
        assert_eq!(reveal_policy(false, true, true, true), RevealState::Ready);

        // Four states, four sentences. A control that says the same thing for
        // two different refusals is a control that explains nothing.
        let reasons = [
            RevealState::Ready.reason(),
            RevealState::NoFolder.reason(),
            RevealState::NoDisplay.reason(),
            RevealState::NoHelper.reason(),
            RevealState::Probe.reason(),
        ];
        for (index, reason) in reasons.iter().enumerate() {
            assert!(!reason.is_empty());
            assert!(
                !reasons[..index].contains(reason),
                "two states share a sentence: {reason}"
            );
        }
    }

    #[test]
    fn a_refused_reveal_spawns_nothing_and_says_why() {
        // `/nonexistent` is not a directory, so this exercises the whole guard
        // without a display, a helper or a child process being involved.
        let error = reveal_directory(Path::new("/nonexistent/musializer-reveal"), false)
            .expect_err("a missing folder cannot be revealed");
        assert!(
            matches!(error, RevealError::Refused(_)),
            "got {error:?}, which means something was spawned"
        );
    }
}
