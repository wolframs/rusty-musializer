//! Assist child-process supervision.
//!
//! **Owner: Agent E.** Ported from the Assist spawn and reap paths of the frozen
//! C oracle at `../musializer` (commit `9300af9`, read-only):
//! `start_assist_job` (`src/plug.c:3253-3385`), `poll_assist_job`
//! (`:3933-4086`), `request_assist_job_stop` (`:4093-4127`) and
//! `cancel_assist_job_blocking` (`:4129-4195`). The Windows job-object arms are
//! not ported.
//!
//! ## What this module is and is not
//!
//! `tools/external_analysis.py` is an independent evidence-producing tool. It
//! runs whisper.cpp, it may contact a model, and none of that belongs in a
//! renderer. This module supervises the process and hands back the path to the
//! bridge file it wrote. Reading and validating that bridge is Agent B's
//! (`analysis_bridge.c`, `analysis_candidate.c`), and deciding what to do with
//! the result is Agent F's — **a job finishing must never mutate a project**,
//! which is why [`AssistJob`] reports and never applies.
//!
//! The `Assist_Job_State` machine (`src/assist_ui_state.h:18-28`) is Agent F's
//! too. What lives here is the process half of it: [`AssistPoll`] reports what
//! the child did, and the caller maps that onto its own state.
//!
//! ## The one rule that is easy to get wrong
//!
//! Only an **explicitly chosen** lyric sheet is passed as `--lyrics-file`
//! (`src/plug.c:3344-3351`). A sibling `<stem>.lyrics.txt` is left to the
//! helper's own discovery so the rule lives in one place. Two copies drift, and
//! the helper's copy is the one with an embedded-tag fallback this side cannot
//! see without `ffprobe`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::process_group::{
    signal_group_or_process, wait_bounded, Delivery, ShutdownPolicy, Signal, WaitOutcome,
};

/// `ASSIST_JOB_TIMEOUT_SECONDS` (`src/assist_ui_state.h:8`). Forty minutes; the
/// same number is passed to the helper as `--timeout 2400`, so both sides agree
/// on the deadline.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(2400);

/// How long a stopping job is given to honour `SIGTERM` before the supervisor
/// escalates to `SIGKILL` on the whole group (`src/plug.c:3998`).
pub const ESCALATION_DELAY: Duration = Duration::from_secs(2);

/// `Assist_Mode` (`src/assist_ui_state.h:10-16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistMode {
    Lyrics,
    Sections,
    Mimo,
    All,
}

impl AssistMode {
    /// `assist_mode_argument` (`src/assist_ui_state.c:84-94`). These strings are
    /// the helper's `--mode` values **and** the prefix of every artifact file
    /// name, so they are not free to rename.
    pub const fn argument(self) -> &'static str {
        match self {
            AssistMode::Lyrics => "lyrics",
            AssistMode::Sections => "sections",
            AssistMode::Mimo => "mimo",
            AssistMode::All => "all",
        }
    }

    /// Whether the run sends audio to a model, which is what `--zdr`
    /// (zero-data-retention) is for (`src/plug.c:3341-3343`).
    pub const fn uses_model(self) -> bool {
        matches!(self, AssistMode::Mimo | AssistMode::All)
    }
}

/// Why the supervisor stopped a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The user asked. `ASSIST_JOB_CANCELLING`.
    Cancelled,
    /// The forty-minute deadline expired. `ASSIST_JOB_TIMING_OUT`.
    TimedOut,
    /// Termination could not be verified, so the job is being torn down as a
    /// failure. `ASSIST_JOB_FAILING`.
    Failing,
}

/// What a poll found. The caller maps this onto its own `Assist_Job_State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistPoll {
    /// Still working.
    Running,
    /// The helper exited zero. The bridge is ready to be **read and validated**;
    /// a zero exit is not by itself a valid result (`src/plug.c:4051-4058`).
    Succeeded,
    /// The helper exited non-zero or died from a signal. The job log has the
    /// reason.
    Failed,
    /// A stop this supervisor requested has completed, and the child is reaped.
    Stopped(StopReason),
}

/// Why an Assist job could not be started.
#[derive(Debug, thiserror::Error)]
pub enum AssistError {
    /// `"The Assist helper script is missing from this installation."`
    /// (`src/plug.c:3316-3317`).
    #[error("the Assist helper script is missing from this installation")]
    HelperMissing,
    /// `"The local analysis workspace could not be created. …"` (`:3267`).
    #[error("the analysis workspace could not be created: {0}")]
    Workspace(#[source] io::Error),
    /// `"A unique analysis artifact name could not be reserved."` (`:3310`).
    #[error("a unique analysis artifact name could not be reserved")]
    NoUniqueArtifact,
    /// `"Python could not launch the Assist helper. …"` (`:3367`).
    #[error("python could not launch the Assist helper: {0}")]
    Spawn(#[source] io::Error),
}

/// The immutable artifact names one job owns.
///
/// The output directory is deliberately **stable** across jobs for the same
/// track so the Python layer can reuse its measured and model caches, while each
/// accepted bridge and its diagnostic log are per-job and immutable: a later
/// Lyrics run must not invalidate the provenance of an earlier MiMo lane
/// (`src/plug.c:3277-3280`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistArtifacts {
    /// The nonce that produced these names. Non-zero.
    pub nonce: u64,
    /// `<dir>/<mode>-<pid>-<nonce:016x>.bridge.tsv`.
    pub bridge: PathBuf,
    /// `<dir>/<mode>-<pid>-<nonce:016x>.log`, the helper's stderr.
    pub log: PathBuf,
}

impl AssistArtifacts {
    /// Reserves a bridge and log name that do not already exist.
    ///
    /// `src/plug.c:3281-3310`: up to 1024 attempts, incrementing the nonce and
    /// skipping zero, requiring **both** names to be free. The pair must be free
    /// together — a nonce whose log exists but whose bridge does not would let a
    /// new job append to an old job's diagnostics.
    pub fn reserve(
        output_dir: &Path,
        mode: AssistMode,
        process_id: u64,
        previous_nonce: u64,
    ) -> Result<Self, AssistError> {
        let mut nonce = previous_nonce;
        for _ in 0..1024 {
            nonce = nonce.wrapping_add(1);
            if nonce == 0 {
                nonce = 1;
            }
            let stem = format!("{}-{}-{:016x}", mode.argument(), process_id, nonce);
            let bridge = output_dir.join(format!("{stem}.bridge.tsv"));
            let log = output_dir.join(format!("{stem}.log"));
            if !bridge.exists() && !log.exists() {
                return Ok(AssistArtifacts { nonce, bridge, log });
            }
        }
        Err(AssistError::NoUniqueArtifact)
    }
}

/// Everything a job needs, with nothing the application owns.
///
/// `lyrics_file` is the **explicitly chosen** sheet or `None`. Two rules:
/// the caller passes `None` for a mode that does not align authored lyrics
/// (`assist_mode_uses_lyric_reference`, Agent F's), and it never fills this in
/// by looking for a sibling file — that discovery is the helper's alone.
#[derive(Debug, Clone)]
pub struct AssistSpec<'a> {
    /// `tools/external_analysis.py`, resolved through
    /// [`super::font_import::find_assist_helper`].
    pub helper: &'a Path,
    /// The audio to analyse.
    pub audio: &'a Path,
    /// The per-track workspace, stable across jobs (`./build/analysis/<hash>`).
    pub output_dir: &'a Path,
    /// The decoded track duration, formatted to nine decimals like the C
    /// (`src/plug.c:3312-3313`).
    pub duration_seconds: f64,
    pub mode: AssistMode,
    /// An explicitly chosen lyric sheet, or `None`.
    pub lyrics_file: Option<&'a Path>,
}

/// A running `external_analysis.py` child and its artifacts.
///
/// Owning this owns a process tree. [`finalize`](AssistJob::finalize) is the
/// reporting path; `Drop` is the net.
#[derive(Debug)]
pub struct AssistJob {
    child: Option<Child>,
    mode: AssistMode,
    artifacts: AssistArtifacts,
    started: Instant,
    /// Set when a stop has been requested, with the instant it was requested, so
    /// the 2-second escalation is measurable (`src/plug.c:3998`).
    stopping: Option<(StopReason, Instant)>,
}

impl AssistJob {
    /// Spawns the helper.
    ///
    /// `start_assist_job` (`src/plug.c:3253-3385`), minus the parts that belong
    /// to the application: the track index, the output-directory hash, and the
    /// notices.
    ///
    /// The argv is the C's exactly, including `--timeout 2400` and
    /// `--new-process-group`. **The parent must not create the process group
    /// itself** — see the module docs on `process_group`; the helper calls
    /// `os.setsid()` and would fail with `EPERM` if it were already a group
    /// leader.
    pub fn start(spec: &AssistSpec<'_>, previous_nonce: u64) -> Result<Self, AssistError> {
        if !spec.helper.is_file() {
            return Err(AssistError::HelperMissing);
        }
        std::fs::create_dir_all(spec.output_dir).map_err(AssistError::Workspace)?;
        let artifacts = AssistArtifacts::reserve(
            spec.output_dir,
            spec.mode,
            u64::from(std::process::id()),
            previous_nonce,
        )?;

        let mut command = Command::new("python3");
        command
            .arg(spec.helper)
            .arg("assist")
            .arg(spec.audio)
            .arg(spec.output_dir)
            .arg("--duration")
            .arg(format!("{:.9}", spec.duration_seconds))
            .arg("--mode")
            .arg(spec.mode.argument())
            .arg("--bridge")
            .arg(&artifacts.bridge)
            .arg("--timeout")
            .arg("2400")
            // The helper puts itself in a new session so cancellation reaches
            // whisper.cpp and codex, not just python.
            .arg("--new-process-group");
        if spec.mode.uses_model() {
            command.arg("--zdr");
        }
        // Only an explicitly chosen sheet, and only if it is still there
        // (src/plug.c:3347-3351). Sibling discovery stays in the helper.
        if let Some(lyrics) = spec.lyrics_file {
            if lyrics.is_file() {
                command.arg("--lyrics-file").arg(lyrics);
            }
        }

        let log_file = std::fs::File::create(&artifacts.log).map_err(AssistError::Workspace)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // stderr to the job log, as nob's `.stderr_path` does.
            .stderr(Stdio::from(log_file));

        let child = command.spawn().map_err(AssistError::Spawn)?;
        Ok(AssistJob {
            child: Some(child),
            mode: spec.mode,
            artifacts,
            started: Instant::now(),
            stopping: None,
        })
    }

    pub fn mode(&self) -> AssistMode {
        self.mode
    }

    /// The bridge the helper was told to write. Reading and validating it is the
    /// caller's job; the file may not exist even after a zero exit.
    pub fn bridge_path(&self) -> &Path {
        &self.artifacts.bridge
    }

    /// The helper's stderr, which every Assist notice points the user at.
    pub fn log_path(&self) -> &Path {
        &self.artifacts.log
    }

    pub fn artifacts(&self) -> &AssistArtifacts {
        &self.artifacts
    }

    /// How long the job has been running.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Whether the forty-minute deadline has passed.
    ///
    /// `assist_job_deadline_expired` (`src/assist_ui_state.c:12-19`), which only
    /// ever returns true for a *running* job. A monotonic [`Instant`] also makes
    /// the C's "a clock that went backwards is not evidence of an overrun" guard
    /// unnecessary rather than reimplemented.
    pub fn deadline_expired(&self) -> bool {
        self.stopping.is_none() && self.elapsed() >= JOB_TIMEOUT
    }

    /// Remaining time before the deadline, for the panel's countdown
    /// (`assist_job_deadline_remaining`, `src/assist_ui_state.c:21-48`).
    pub fn deadline_remaining(&self) -> Duration {
        JOB_TIMEOUT.saturating_sub(self.elapsed())
    }

    /// Whether a stop has been requested and not yet observed.
    pub fn stopping(&self) -> Option<StopReason> {
        self.stopping.map(|(reason, _)| reason)
    }

    /// Asks the whole process tree to stop.
    ///
    /// `request_assist_job_stop` (`src/plug.c:4093-4127`): `SIGTERM` to the
    /// group, falling back to the process. A failure that is not `ESRCH` moves
    /// the job to [`StopReason::Failing`] and **retains ownership** rather than
    /// forgetting the child — the C's comment at `:4188-4189` is explicit that
    /// the caller must not free the plug while the tokens are live.
    ///
    /// Returns whether the request was accepted.
    pub fn request_stop(&mut self, reason: StopReason) -> bool {
        let Some(child) = self.child.as_ref() else {
            return false;
        };
        if self.stopping.is_some() {
            return false;
        }
        match signal_group_or_process(child.id(), Signal::Term) {
            Ok(Delivery::Delivered) | Ok(Delivery::NotFound) => {
                self.stopping = Some((reason, Instant::now()));
                true
            }
            Err(_) => {
                self.stopping = Some((StopReason::Failing, Instant::now()));
                false
            }
        }
    }

    /// Non-blocking progress check.
    ///
    /// `poll_assist_job` (`src/plug.c:3933-4086`) in the C's order:
    ///
    /// 1. A job with no child handle is a failure, not a job that will finish
    ///    (`:3936-3955`).
    /// 2. A running job past its deadline is asked to stop (`:3957-3964`).
    /// 3. A stopping job is reaped, escalating to `SIGKILL` on the group after
    ///    two seconds (`:3990-4002`).
    /// 4. Otherwise the exit status decides success or failure (`:4008-4050`).
    pub fn poll(&mut self) -> io::Result<AssistPoll> {
        if self.child.is_none() {
            return Ok(match self.stopping {
                Some((reason, _)) => AssistPoll::Stopped(reason),
                None => AssistPoll::Failed,
            });
        }

        // Before borrowing the child, because requesting a stop needs `self`.
        if self.deadline_expired() {
            self.request_stop(StopReason::TimedOut);
            // Fall through: the child may already be gone.
        }

        let child = self.child.as_mut().expect("checked above");
        if let Some((reason, requested_at)) = self.stopping {
            if let Some(_status) = child.try_wait()? {
                self.child = None;
                return Ok(AssistPoll::Stopped(reason));
            }
            if requested_at.elapsed() >= ESCALATION_DELAY {
                // SIGKILL the group. A polite request the tree ignored for two
                // seconds is not going to be honoured later.
                let child_id = child.id();
                signal_group_or_process(child_id, Signal::Kill)?;
            }
            return Ok(AssistPoll::Running);
        }

        match child.try_wait()? {
            None => Ok(AssistPoll::Running),
            Some(status) => {
                self.child = None;
                Ok(if status.success() {
                    AssistPoll::Succeeded
                } else {
                    AssistPoll::Failed
                })
            }
        }
    }

    /// Stops the tree and reaps it, blocking up to about four seconds.
    ///
    /// `cancel_assist_job_blocking` (`src/plug.c:4129-4195`): `SIGTERM` the
    /// group, poll 200 × 10 ms, then `SIGKILL` the group and poll 200 × 10 ms
    /// again. Returns `Ok(false)` when the child still could not be reaped, in
    /// which case **this job must not be dropped and forgotten** — the C keeps
    /// the ownership tokens live for exactly that reason.
    ///
    /// This is the shutdown path: the application calls it before exiting, and
    /// the C also called it before a hot reload.
    pub fn cancel_blocking(&mut self) -> io::Result<bool> {
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        let reason = self.stopping.map(|(reason, _)| reason);
        if reason.is_none() {
            signal_group_or_process(child.id(), Signal::Term)?;
            self.stopping = Some((StopReason::Cancelled, Instant::now()));
        }
        let ladder = ShutdownPolicy::with_grace(Duration::from_secs(2), Duration::from_millis(10));
        if wait_bounded(child, &ladder)?.is_reaped() {
            self.child = None;
            return Ok(true);
        }
        signal_group_or_process(child.id(), Signal::Kill)?;
        match wait_bounded(child, &ladder)? {
            WaitOutcome::Reaped(_) => {
                self.child = None;
                Ok(true)
            }
            WaitOutcome::TimedOut => Ok(false),
        }
    }

    /// Whether the child has been reaped and this job owns no process.
    pub fn is_finished(&self) -> bool {
        self.child.is_none()
    }
}

impl Drop for AssistJob {
    /// Best effort against abandonment. It is loud, because a silently
    /// abandoned Assist tree can be whisper.cpp holding a GPU.
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        match self.cancel_blocking() {
            Ok(true) => {}
            Ok(false) => eprintln!(
                "ASSIST: abandoned worker could not be reaped promptly; see {}",
                self.artifacts.log.display()
            ),
            Err(error) => eprintln!("ASSIST: abandoned worker could not be stopped: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("musializer-assist-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// A stand-in for `tools/external_analysis.py`, spawned through
        /// `python3` exactly as the real helper is, so the argv is under test.
        fn fake_helper(&self, label: &str, body: &str) -> PathBuf {
            let path = self.join(label);
            std::fs::write(
                &path,
                format!("import sys, time, os, signal, pathlib\n{body}\n"),
            )
            .expect("helper");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn spec<'a>(helper: &'a Path, output: &'a Path, mode: AssistMode) -> AssistSpec<'a> {
        AssistSpec {
            helper,
            audio: Path::new("/tmp/track.wav"),
            output_dir: output,
            duration_seconds: 40.5,
            mode,
            lyrics_file: None,
        }
    }

    fn poll_until_finished(job: &mut AssistJob) -> AssistPoll {
        for _ in 0..800 {
            match job.poll().expect("poll") {
                AssistPoll::Running => std::thread::sleep(Duration::from_millis(10)),
                other => return other,
            }
        }
        panic!("the fake helper never finished");
    }

    #[test]
    fn mode_arguments_are_the_helpers_own_spellings() {
        assert_eq!(AssistMode::Lyrics.argument(), "lyrics");
        assert_eq!(AssistMode::Sections.argument(), "sections");
        assert_eq!(AssistMode::Mimo.argument(), "mimo");
        assert_eq!(AssistMode::All.argument(), "all");
        // Only the two model modes get --zdr.
        assert!(!AssistMode::Lyrics.uses_model());
        assert!(!AssistMode::Sections.uses_model());
        assert!(AssistMode::Mimo.uses_model());
        assert!(AssistMode::All.uses_model());
    }

    #[test]
    fn artifact_names_are_reserved_around_files_that_already_exist() {
        let scratch = Scratch::new("reserve");
        let first =
            AssistArtifacts::reserve(&scratch.0, AssistMode::Lyrics, 4321, 0).expect("reserve");
        assert_eq!(first.nonce, 1);
        assert!(first
            .bridge
            .ends_with("lyrics-4321-0000000000000001.bridge.tsv"));
        assert!(first.log.ends_with("lyrics-4321-0000000000000001.log"));

        // An existing log alone must push the reservation forward: a new job
        // appending to an old job's diagnostics would destroy its provenance.
        std::fs::write(&first.log, "").unwrap();
        let second =
            AssistArtifacts::reserve(&scratch.0, AssistMode::Lyrics, 4321, 0).expect("reserve");
        assert_eq!(second.nonce, 2);

        // A different mode gets a different name space entirely.
        let sections =
            AssistArtifacts::reserve(&scratch.0, AssistMode::Sections, 4321, 0).expect("reserve");
        assert_eq!(sections.nonce, 1);
        assert!(sections
            .bridge
            .ends_with("sections-4321-0000000000000001.bridge.tsv"));
    }

    #[test]
    fn a_missing_helper_is_refused_before_python_is_invoked() {
        let scratch = Scratch::new("missing");
        let absent = scratch.join("nothing.py");
        let error = AssistJob::start(&spec(&absent, &scratch.0, AssistMode::Lyrics), 0)
            .expect_err("a missing helper cannot run");
        assert!(matches!(error, AssistError::HelperMissing), "{error:?}");
    }

    #[test]
    fn the_helper_receives_the_oracles_argv() {
        let scratch = Scratch::new("argv");
        // The helper records its own argv so the spawn contract is checked
        // rather than assumed.
        let helper = scratch.fake_helper(
            "helper.py",
            "pathlib.Path(sys.argv[sys.argv.index('--bridge') + 1])\
             .write_text('\\n'.join(sys.argv[1:]))",
        );
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Mimo), 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), AssistPoll::Succeeded);

        let argv: Vec<String> = std::fs::read_to_string(job.bridge_path())
            .expect("bridge")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(argv[0], "assist");
        assert_eq!(argv[1], "/tmp/track.wav");
        assert_eq!(argv[2], scratch.0.to_string_lossy());
        assert_eq!(argv[3], "--duration");
        assert_eq!(
            argv[4], "40.500000000",
            "nine decimals, as the C formats it"
        );
        assert_eq!(argv[5], "--mode");
        assert_eq!(argv[6], "mimo");
        assert_eq!(argv[7], "--bridge");
        assert_eq!(argv[9], "--timeout");
        assert_eq!(argv[10], "2400");
        assert_eq!(argv[11], "--new-process-group");
        assert_eq!(argv[12], "--zdr", "a model run must be zero-data-retention");
        assert!(
            !argv.contains(&"--lyrics-file".to_string()),
            "no sheet was chosen, so none may be passed"
        );
    }

    #[test]
    fn only_an_explicitly_chosen_lyric_sheet_is_passed() {
        let scratch = Scratch::new("lyrics");
        let helper = scratch.fake_helper(
            "helper.py",
            "pathlib.Path(sys.argv[sys.argv.index('--bridge') + 1])\
             .write_text('\\n'.join(sys.argv[1:]))",
        );
        let sheet = scratch.join("chosen.lyrics.txt");
        std::fs::write(&sheet, "a line").unwrap();

        let mut chosen = spec(&helper, &scratch.0, AssistMode::Lyrics);
        chosen.lyrics_file = Some(&sheet);
        let mut job = AssistJob::start(&chosen, 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), AssistPoll::Succeeded);
        let argv = std::fs::read_to_string(job.bridge_path()).expect("bridge");
        assert!(argv.contains("--lyrics-file"));
        assert!(argv.contains(&sheet.to_string_lossy().to_string()));

        // A chosen sheet that has since been deleted is silently not passed,
        // as the C's FileExists guard does. The helper then falls back to its
        // own discovery, which is where that rule belongs.
        std::fs::remove_file(&sheet).unwrap();
        let mut job = AssistJob::start(&chosen, job.artifacts().nonce).expect("start");
        assert_eq!(poll_until_finished(&mut job), AssistPoll::Succeeded);
        let argv = std::fs::read_to_string(job.bridge_path()).expect("bridge");
        assert!(!argv.contains("--lyrics-file"));
    }

    #[test]
    fn a_zero_exit_is_reported_as_success_and_nothing_is_applied() {
        let scratch = Scratch::new("success");
        // Exits zero without writing a bridge at all. The supervisor must still
        // say "succeeded" and leave validation to the caller: a zero exit is not
        // by itself a valid result.
        let helper = scratch.fake_helper("helper.py", "raise SystemExit(0)");
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Sections), 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), AssistPoll::Succeeded);
        assert!(
            !job.bridge_path().exists(),
            "the supervisor must not invent a bridge"
        );
        assert!(job.is_finished());
    }

    #[test]
    fn a_nonzero_exit_is_a_failure_and_the_log_survives() {
        let scratch = Scratch::new("failure");
        let helper = scratch.fake_helper(
            "helper.py",
            "sys.stderr.write('whisper is missing\\n')\nraise SystemExit(2)",
        );
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Lyrics), 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), AssistPoll::Failed);
        let log = std::fs::read_to_string(job.log_path()).expect("log");
        assert!(log.contains("whisper is missing"), "log was {log:?}");
    }

    #[test]
    fn cancelling_reaches_the_whole_tree_not_just_python() {
        let scratch = Scratch::new("tree");
        // The helper does what the real one does: puts itself in a new session
        // and spawns a long-lived child, then waits. A SIGTERM to python alone
        // would leave the grandchild running.
        let helper = scratch.fake_helper(
            "helper.py",
            "os.setsid()\n\
             child = os.fork()\n\
             if child == 0:\n\
             \x20   time.sleep(120)\n\
             \x20   os._exit(0)\n\
             pathlib.Path(sys.argv[sys.argv.index('--bridge') + 1]).write_text(str(child))\n\
             time.sleep(120)",
        );
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::All), 0).expect("start");

        // Wait for the helper to record the grandchild's pid, which also proves
        // os.setsid() succeeded — it would raise if the parent had already made
        // the child a group leader.
        let mut grandchild = None;
        for _ in 0..300 {
            if let Ok(text) = std::fs::read_to_string(job.bridge_path()) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    grandchild = Some(pid);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let grandchild = grandchild.expect(
            "the helper should have recorded its child; if os.setsid() failed, \
             the parent is wrongly creating the process group",
        );

        assert!(job.request_stop(StopReason::Cancelled));
        assert_eq!(job.stopping(), Some(StopReason::Cancelled));
        assert_eq!(
            poll_until_finished(&mut job),
            AssistPoll::Stopped(StopReason::Cancelled)
        );

        std::thread::sleep(Duration::from_millis(200));
        let running = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
            .map(|stat| !stat.contains(") Z "))
            .unwrap_or(false);
        assert!(
            !running,
            "the group signal must have reached the helper's own child"
        );
    }

    #[test]
    fn a_helper_that_ignores_sigterm_is_escalated_after_two_seconds() {
        let scratch = Scratch::new("stubborn");
        let helper = scratch.fake_helper(
            "helper.py",
            "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n\
             pathlib.Path(sys.argv[sys.argv.index('--bridge') + 1]).write_text('ready')\n\
             time.sleep(120)",
        );
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Lyrics), 0).expect("start");
        for _ in 0..300 {
            if job.bridge_path().exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(job.request_stop(StopReason::Cancelled));
        // The polite request is ignored, so the job stays Running until the
        // escalation fires. ESCALATION_DELAY is two seconds in the oracle; this
        // waits a little longer than that rather than reaching into the clock.
        let stopped = poll_until_finished(&mut job);
        assert_eq!(stopped, AssistPoll::Stopped(StopReason::Cancelled));
        assert!(job.elapsed() >= ESCALATION_DELAY);
    }

    #[test]
    fn cancel_blocking_reaps_a_stubborn_helper_and_reports_success() {
        let scratch = Scratch::new("blocking");
        let helper = scratch.fake_helper(
            "helper.py",
            "signal.signal(signal.SIGTERM, signal.SIG_IGN)\ntime.sleep(120)",
        );
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Lyrics), 0).expect("start");
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            job.cancel_blocking().expect("cancel"),
            "the SIGKILL half of the ladder must finish the job"
        );
        assert!(job.is_finished());
        // Idempotent: a second call has nothing to do.
        assert!(job.cancel_blocking().expect("cancel"));
    }

    #[test]
    fn dropping_a_running_job_still_reaps_the_tree() {
        let scratch = Scratch::new("drop");
        let helper = scratch.fake_helper("helper.py", "time.sleep(120)");
        let pid;
        {
            let mut job =
                AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Lyrics), 0).expect("start");
            pid = job.child.as_ref().expect("child").id();
            assert_eq!(job.poll().expect("poll"), AssistPoll::Running);
        }
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "Drop must reap the worker, not leave a zombie"
        );
    }

    #[test]
    fn the_deadline_is_forty_minutes_and_only_applies_while_running() {
        let scratch = Scratch::new("deadline");
        let helper = scratch.fake_helper("helper.py", "time.sleep(120)");
        let mut job =
            AssistJob::start(&spec(&helper, &scratch.0, AssistMode::Lyrics), 0).expect("start");
        assert_eq!(JOB_TIMEOUT, Duration::from_secs(2400));
        assert!(!job.deadline_expired());
        assert!(job.deadline_remaining() > Duration::from_secs(2399));
        // A stopping job is not "overrunning"; the C's deadline check requires
        // ASSIST_JOB_RUNNING.
        job.request_stop(StopReason::Cancelled);
        assert!(!job.deadline_expired());
        let _ = job.cancel_blocking();
    }
}
