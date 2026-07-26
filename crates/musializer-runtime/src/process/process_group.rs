//! Linux process-group ownership and bounded shutdown.
//!
//! **Owner: Agent E.** Ported from the frozen C oracle at `../musializer`
//! (commit `9300af9`, read-only): the bounded reap loop is
//! `waitpid_bounded` (`src/ffmpeg_posix.c:71-96`) with its grace periods from
//! `render_export_finalize_grace_ms` (`src/render_export.c:346-358`), and the
//! group signalling is the `kill(-pid, …)`-then-`kill(pid, …)` pattern the
//! Assist supervisor uses (`src/plug.c:4111-4121`, `:3999-4001`,
//! `:4152-4168`).
//!
//! Three things live here because every supervised child family needs them and
//! none of them should be reinvented per family:
//!
//! 1. **Signalling.** `std`'s [`Child::kill`] sends `SIGKILL` to one process
//!    and nothing else, so `SIGTERM` and process-group delivery need `kill(2)`.
//!    That is the one unsafe island in this module.
//! 2. **Bounded reaping.** Safe Rust does not reap children automatically. A
//!    dropped [`Child`] leaks a zombie until the process exits. Every owner in
//!    this crate finalizes through [`wait_bounded`] so a hung child becomes a
//!    reported timeout instead of an unbounded stall.
//! 3. **The group-then-process fallback**, which is subtler than it looks —
//!    see [`signal_group_or_process`].
//!
//! ## Why there is no `setsid`/`setpgid` here
//!
//! It would be natural to give every child its own group from the parent with
//! `std::os::unix::process::CommandExt::process_group(0)`, which would close a
//! race the oracle has (below). **Do not do it for the Python helpers.**
//! `tools/external_analysis.py` calls `os.setsid()` on itself when it is passed
//! `--new-process-group` (`tools/external_analysis.py:307-310`, `:1519-1520`),
//! and `setsid(2)` fails with `EPERM` when the caller is already a process-group
//! leader. Making the child a group leader from the parent would therefore make
//! the helper raise `PermissionError` and die on startup. The child creates its
//! own group; the parent only signals it.
//!
//! The race that leaves: between `fork`/`exec` and the helper's `setsid`, the
//! child's group is still the application's. `kill(-child_pid, …)` in that
//! window addresses a group whose id equals a non-leader pid, which does not
//! exist, so it fails with `ESRCH` and the fallback signals the single process.
//! That is why the fallback exists and why it must not be "simplified" away.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// The signals this application sends. Numbers are the Linux/glibc values;
/// `libc` is not a dependency of this crate yet (see the note in
/// `REWRITE_PLAN.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Signal {
    /// Ask politely. The Python helpers install handlers for this.
    Term = 15,
    /// Do not ask. `ffmpeg` cancellation and every escalation path use it.
    Kill = 9,
}

impl Signal {
    /// The raw signal number, for the one `kill(2)` call site.
    pub const fn number(self) -> i32 {
        self as i32
    }

    /// The name `strsignal` would print, for messages that name a signal the
    /// way the C `TraceLog` output did (`src/ffmpeg_posix.c:322`).
    pub const fn name(self) -> &'static str {
        match self {
            Signal::Term => "Terminated",
            Signal::Kill => "Killed",
        }
    }
}

/// What a signal delivery attempt achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The signal was accepted by at least one process.
    Delivered,
    /// Nothing by that name exists (`ESRCH`). Not an error: a child that
    /// already exited is exactly the state cancellation was trying to reach.
    NotFound,
}

/// `ESRCH`, the only `kill(2)` failure this code treats as success.
const ESRCH: i32 = 3;

/// `kill(2)`, the only libc symbol this crate declares by hand.
///
/// Declaring it rather than adding `libc` keeps a leaf agent from editing the
/// workspace manifest. The signature is stable ABI: `pid_t` is `int` on every
/// Linux target Rust supports.
mod sys {
    use core::ffi::c_int;

    extern "C" {
        pub fn kill(pid: c_int, sig: c_int) -> c_int;
    }
}

/// Sends `signal` to one process, or to a group when `pid` is negative.
fn raw_kill(pid: i32, signal: Signal) -> io::Result<Delivery> {
    // SAFETY: `kill(2)` takes two integers by value, writes nothing through a
    // pointer, and is async-signal-safe. It cannot violate a Rust invariant:
    // the worst a wrong `pid` can do is signal an unrelated process, which is a
    // logic bug rather than unsoundness, and every caller here passes a pid it
    // owns as a live `std::process::Child` (or the negation of one).
    let result = unsafe { sys::kill(pid, signal.number()) };
    if result == 0 {
        return Ok(Delivery::Delivered);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ESRCH) {
        return Ok(Delivery::NotFound);
    }
    Err(error)
}

/// Signals a single process by pid.
pub fn signal_process(pid: u32, signal: Signal) -> io::Result<Delivery> {
    raw_kill(pid_as_i32(pid)?, signal)
}

/// Signals every process in the group whose id is `pgid`.
pub fn signal_group(pgid: u32, signal: Signal) -> io::Result<Delivery> {
    let pid = pid_as_i32(pgid)?;
    raw_kill(-pid, signal)
}

/// Signals the child's process group, falling back to the child alone when no
/// such group exists.
///
/// This is `src/plug.c:4112-4113` exactly:
///
/// ```c
/// int result = kill(-process, SIGTERM);
/// if (result < 0 && errno == ESRCH) result = kill(process, SIGTERM);
/// ```
///
/// The fallback is not defensive padding. The helper puts itself in a new
/// session after `exec`, so there is a window in which no group with that id
/// exists yet; signalling the single process is the only thing that reaches it
/// during that window. It is also what happens after the whole tree has already
/// exited, which is why `NotFound` from both attempts is reported as
/// [`Delivery::NotFound`] rather than as an error.
pub fn signal_group_or_process(pid: u32, signal: Signal) -> io::Result<Delivery> {
    match signal_group(pid, signal)? {
        Delivery::Delivered => Ok(Delivery::Delivered),
        Delivery::NotFound => signal_process(pid, signal),
    }
}

fn pid_as_i32(pid: u32) -> io::Result<i32> {
    i32::try_from(pid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "process id does not fit in pid_t",
        )
    })
}

/// How long a bounded wait waits, and how often it looks.
///
/// The two grace periods are `render_export_finalize_grace_ms`
/// (`src/render_export.c:346-349`): finalization gets five minutes because
/// `-movflags +faststart` makes `ffmpeg` relocate the moov atom of a possibly
/// multi-gigabyte 4K file, and cancellation gets five seconds because a
/// `SIGKILL`ed process still alive after five seconds is stuck in the kernel,
/// not busy.
#[derive(Debug, Clone, Copy)]
pub struct ShutdownPolicy {
    /// Upper bound on the whole wait.
    pub grace: Duration,
    /// Sleep between `waitpid(WNOHANG)` attempts. 20 ms in the C
    /// (`src/ffmpeg_posix.c:93`).
    pub poll_interval: Duration,
}

impl ShutdownPolicy {
    /// Normal completion: `render_export_finalize_grace_ms(false)` = 300000 ms.
    pub const fn finalize() -> Self {
        ShutdownPolicy {
            grace: Duration::from_millis(300_000),
            poll_interval: Duration::from_millis(20),
        }
    }

    /// Cancellation or forced termination:
    /// `render_export_finalize_grace_ms(true)` = 5000 ms.
    pub const fn cancel() -> Self {
        ShutdownPolicy {
            grace: Duration::from_millis(5_000),
            poll_interval: Duration::from_millis(20),
        }
    }

    /// The same shape with an explicit grace, for tests and for the Assist
    /// supervisor's own 10 ms × 200 loops (`src/plug.c:4157-4166`).
    pub const fn with_grace(grace: Duration, poll_interval: Duration) -> Self {
        ShutdownPolicy {
            grace,
            poll_interval,
        }
    }
}

/// The result of a bounded wait. `1`, `0` and `-1` in the C
/// (`src/ffmpeg_posix.c:71`); an `Err` is the `-1` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The child was reaped. No zombie remains.
    Reaped(ExitStatus),
    /// The grace period expired with the child still alive. The caller must
    /// escalate; the child is still this process's responsibility.
    TimedOut,
}

impl WaitOutcome {
    /// Whether the child is gone and reaped.
    pub fn is_reaped(self) -> bool {
        matches!(self, WaitOutcome::Reaped(_))
    }
}

/// Polls for the child's exit until `policy.grace` elapses.
///
/// Uses a monotonic [`Instant`] rather than `clock_gettime(CLOCK_MONOTONIC)`
/// arithmetic, which removes the C's borrow-from-seconds fixup
/// (`src/ffmpeg_posix.c:83-89`) without changing the boundary: the C compares
/// `elapsed_ms >= grace`, and so does this.
pub fn wait_bounded(child: &mut Child, policy: &ShutdownPolicy) -> io::Result<WaitOutcome> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(WaitOutcome::Reaped(status));
        }
        if started.elapsed() >= policy.grace {
            return Ok(WaitOutcome::TimedOut);
        }
        std::thread::sleep(policy.poll_interval);
    }
}

/// Sends `SIGKILL` to the child's group (falling back to the child) and reaps
/// it within the cancellation grace period.
///
/// This is the escalation half of `ffmpeg_end_rendering`
/// (`src/ffmpeg_posix.c:261-272`) and of `cancel_assist_job_blocking`
/// (`src/plug.c:4168-4181`), which both kill and then wait again rather than
/// assuming a `SIGKILL` is instantaneous.
pub fn kill_group_and_reap(child: &mut Child) -> io::Result<WaitOutcome> {
    let pid = child.id();
    signal_group_or_process(pid, Signal::Kill)?;
    wait_bounded(child, &ShutdownPolicy::cancel())
}

/// Describes an exit status the way the C's `TraceLog` messages did, so error
/// text can be matched against the oracle's output.
///
/// `WIFEXITED`/`WEXITSTATUS` is [`ExitStatus::code`]; `WIFSIGNALED`/`WTERMSIG`
/// is [`ExitStatusExt::signal`]. The "unexpected process state" arm
/// (`src/ffmpeg_posix.c:328`) is unreachable through `waitpid` without
/// `WUNTRACED`, and is kept only so the message exists if it ever is.
pub fn describe_exit(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exited with code {code}");
    }
    if let Some(signal) = status.signal() {
        return format!("was terminated by signal {signal}");
    }
    "ended in an unexpected process state".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn spawn_sh(script: &str) -> Child {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("/bin/sh should be spawnable")
    }

    #[test]
    fn wait_bounded_reaps_a_child_that_exits_immediately() {
        let mut child = spawn_sh("exit 0");
        let outcome =
            wait_bounded(&mut child, &ShutdownPolicy::cancel()).expect("wait must not fail");
        match outcome {
            WaitOutcome::Reaped(status) => assert_eq!(status.code(), Some(0)),
            WaitOutcome::TimedOut => panic!("a child that exits at once must be reaped"),
        }
    }

    #[test]
    fn wait_bounded_reports_a_nonzero_exit_rather_than_hiding_it() {
        let mut child = spawn_sh("exit 3");
        let outcome = wait_bounded(&mut child, &ShutdownPolicy::cancel()).expect("wait");
        assert_eq!(
            match outcome {
                WaitOutcome::Reaped(status) => status.code(),
                WaitOutcome::TimedOut => None,
            },
            Some(3)
        );
    }

    #[test]
    fn wait_bounded_times_out_instead_of_stalling_forever() {
        let mut child = spawn_sh("sleep 30");
        let policy =
            ShutdownPolicy::with_grace(Duration::from_millis(60), Duration::from_millis(5));
        let outcome = wait_bounded(&mut child, &policy).expect("wait");
        assert_eq!(outcome, WaitOutcome::TimedOut);
        // A timeout leaves the child ours, so the test must not leak it either.
        assert!(kill_group_and_reap(&mut child).expect("kill").is_reaped());
    }

    #[test]
    fn sigterm_stops_an_ordinary_child() {
        let mut child = spawn_sh("sleep 30");
        assert_eq!(
            signal_process(child.id(), Signal::Term).expect("kill"),
            Delivery::Delivered
        );
        let policy =
            ShutdownPolicy::with_grace(Duration::from_millis(2_000), Duration::from_millis(5));
        let outcome = wait_bounded(&mut child, &policy).expect("wait");
        match outcome {
            WaitOutcome::Reaped(status) => {
                assert_eq!(status.signal(), Some(Signal::Term.number()));
            }
            WaitOutcome::TimedOut => panic!("an unhandled SIGTERM must stop /bin/sh"),
        }
    }

    #[test]
    fn a_child_that_ignores_sigterm_is_escalated_to_sigkill() {
        // This is the case the whole escalation ladder exists for: the C sends
        // SIGTERM, waits, and only then sends SIGKILL (src/plug.c:4157-4181).
        let mut child = spawn_sh("trap '' TERM; sleep 30");
        // Give the trap a moment to install; before it does, SIGTERM would
        // stop the shell and the test would prove nothing.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            signal_process(child.id(), Signal::Term).expect("kill"),
            Delivery::Delivered
        );
        let ignored = wait_bounded(
            &mut child,
            &ShutdownPolicy::with_grace(Duration::from_millis(200), Duration::from_millis(10)),
        )
        .expect("wait");
        assert_eq!(
            ignored,
            WaitOutcome::TimedOut,
            "the child trapped SIGTERM, so the polite request must have been ignored"
        );

        let forced = kill_group_and_reap(&mut child).expect("kill");
        match forced {
            WaitOutcome::Reaped(status) => {
                assert_eq!(status.signal(), Some(Signal::Kill.number()))
            }
            WaitOutcome::TimedOut => panic!("SIGKILL is not maskable"),
        }
    }

    #[test]
    fn signalling_something_that_does_not_exist_is_not_an_error() {
        // i32::MAX is above every /proc/sys/kernel/pid_max, so no live process
        // can own it. Signalling a pid we just reaped would be the more
        // faithful test and is exactly the pid-reuse hazard not to write.
        assert_eq!(
            signal_process(i32::MAX as u32, Signal::Kill).expect("ESRCH is not a failure"),
            Delivery::NotFound
        );
        assert_eq!(
            signal_group_or_process(i32::MAX as u32, Signal::Term).expect("ESRCH"),
            Delivery::NotFound
        );
    }

    #[test]
    fn a_group_signal_reaches_a_grandchild_a_process_signal_would_miss() {
        // The reason the Assist supervisor signals the group at all: the helper
        // spawns whisper.cpp and codex, and a SIGTERM to python alone can leave
        // them running (src/plug.c:4111-4113).
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            // setsid puts the shell in its own session, as the Python helper
            // does for itself; then it spawns a grandchild and waits.
            .arg("exec setsid /bin/sh -c 'sleep 30 & echo $! ; wait'")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");

        // setsid(1) may be absent; the invariant is worth testing, not worth
        // failing a suite over a missing util-linux binary.
        let mut grandchild_pid = String::new();
        {
            use std::io::Read;
            let mut stdout = child.stdout.take().expect("piped");
            let mut buffer = [0u8; 32];
            if let Ok(read) = stdout.read(&mut buffer) {
                grandchild_pid = String::from_utf8_lossy(&buffer[..read]).trim().to_string();
            }
        }
        let Ok(grandchild) = grandchild_pid.parse::<u32>() else {
            let _ = kill_group_and_reap(&mut child);
            return;
        };

        assert_eq!(
            signal_group(child.id(), Signal::Kill).expect("killpg"),
            Delivery::Delivered
        );
        assert!(kill_group_and_reap(&mut child).expect("reap").is_reaped());

        // The grandchild is not ours to reap, so it may linger as a zombie
        // owned by init; either way it must not still be running.
        std::thread::sleep(Duration::from_millis(150));
        let running = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
            .map(|stat| !stat.contains(") Z "))
            .unwrap_or(false);
        assert!(
            !running,
            "the group signal must have reached the grandchild"
        );
    }

    #[test]
    fn describe_exit_names_codes_and_signals() {
        let mut exited = spawn_sh("exit 7");
        let status = exited.wait().expect("wait");
        assert_eq!(describe_exit(status), "exited with code 7");

        let mut signalled = spawn_sh("sleep 30");
        signalled.kill().expect("kill");
        let status = signalled.wait().expect("wait");
        assert_eq!(describe_exit(status), "was terminated by signal 9");
    }
}
