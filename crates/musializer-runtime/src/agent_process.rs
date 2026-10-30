//! An agent CLI as a child process speaking lines over stdio (WX-4).
//!
//! The protocol is `musializer_core::agent`'s; this module only moves lines.
//! Two reader threads turn stdout and stderr into a channel, so the UI thread
//! never blocks on the agent. Lines are written to stdin from the UI thread,
//! one `write_all` per line. They are short, and a pipe that cannot take a
//! line means the agent has died, which `poll` reports anyway.
//!
//! **The process gets its own process group.** An agent starts tools, MCP
//! servers and shells of its own. Ending the group is the only way to end all
//! of them. [`AgentProcess::close`] sends `SIGTERM` to the group and hands
//! the child to a thread that sends `SIGKILL` two seconds later if anything is
//! left, then reaps it. The UI thread never waits.
//!
//! **Which binary, and with which PATH.** A desktop launch does not read the
//! interactive shell's rc files, and that is where `~/.local/npm-global/bin`
//! (Codex) or an nvm `node` usually come from. [`warm_shell_path`] asks
//! `$SHELL -ic` once, on a background thread, for its `PATH`. Agents are then
//! started with that `PATH`. `MUSIALIZER_AGENT_<NAME>` names an exact program
//! instead, which is how the gate starts a fake agent and never a real one.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::process::process_group::{signal_group_or_process, Signal};

/// What the agent did since the last poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Line(String),
    /// A line on stderr; kept for the error message if the process dies.
    Diagnostic(String),
    /// The process ended; the text says how.
    Exited(String),
}

/// How to start one agent.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    /// Look the program up on the login shell's PATH (see the module
    /// comment). Off for an absolute override.
    pub use_shell_path: bool,
}

pub struct AgentProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    output: Receiver<Output>,
    pid: u32,
    exited: bool,
    stderr_tail: Vec<String>,
}

static SHELL_PATH: OnceLock<Mutex<ShellPath>> = OnceLock::new();

struct ShellPath {
    receiver: Receiver<Option<String>>,
    // None means pending; Some(None) means the shell answered without a PATH.
    resolved: Option<Option<String>>,
    locations: HashMap<String, Location>,
}

impl ShellPath {
    fn poll(&mut self, wait: Duration) {
        if self.resolved.is_some() {
            return;
        }
        match self.receiver.recv_timeout(wait) {
            Ok(path) => self.resolved = Some(path),
            Err(mpsc::RecvTimeoutError::Disconnected) => self.resolved = Some(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    fn locate(&mut self, program: &str, fallback: &str) -> Location {
        self.poll(Duration::ZERO);
        if let Some(location) = self.locations.get(program) {
            return location.clone();
        }
        let path = self.resolved.as_ref().and_then(Option::as_deref);
        let location = if let Some(found) = resolve(program, path.unwrap_or(fallback)) {
            Location::Found(found)
        } else if self.resolved.is_none() {
            Location::Checking
        } else {
            Location::NotFound
        };
        if self.resolved.is_some() {
            self.locations.insert(program.to_owned(), location.clone());
        }
        location
    }
}

/// Starts asking the login shell for its PATH, once per process.
pub fn warm_shell_path() {
    SHELL_PATH.get_or_init(|| {
        let (tx, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(ask_shell_for_path());
        });
        Mutex::new(ShellPath {
            receiver,
            resolved: None,
            locations: HashMap::new(),
        })
    });
}

fn ask_shell_for_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    detached_shell_path(&shell, Duration::from_secs(4))
}

fn detached_shell_path(shell: &str, timeout: Duration) -> Option<String> {
    // setsid detaches the interactive shell from the launcher's controlling
    // terminal: bash job control must never SIGTTIN the backgrounded app.
    // This is only agent discovery, never the Python Assist helpers (which
    // establish their own session).
    let mut child = Command::new("/usr/bin/setsid")
        .args([
            "--wait",
            shell,
            "-ic",
            "printf '\\n__MUSIALIZER_PATH__%s\\n' \"$PATH\"",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(path) = line.strip_prefix("__MUSIALIZER_PATH__") {
                let _ = tx.send(path.to_owned());
            }
        }
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return rx
                    .recv_timeout(Duration::from_millis(100))
                    .ok()
                    .filter(|p| !p.is_empty())
            }
            Err(_) => break,
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let _ = signal_group_or_process(child.id(), Signal::Kill);
    let _ = child.wait();
    None
}

/// Collect the shell's answer without waiting; spawning never stalls a frame.
fn shell_path() -> Option<String> {
    warm_shell_path();
    let mut state = SHELL_PATH.get()?.lock().ok()?;
    state.poll(Duration::ZERO);
    state.resolved.clone().flatten()
}

/// An executable lookup must distinguish a pending shell PATH from absence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Checking,
    Found(PathBuf),
    NotFound,
}

/// Starts discovery and collects its answer without waiting on the UI thread.
/// Missing on the inherited PATH is inconclusive until the shell has answered.
#[must_use]
pub fn locate(program: &str) -> Location {
    warm_shell_path();
    let fallback = std::env::var("PATH").unwrap_or_default();
    SHELL_PATH
        .get()
        .and_then(|state| state.lock().ok())
        .map_or(Location::Checking, |mut state| {
            state.locate(program, &fallback)
        })
}

/// Resolves `program` against `path` the way `execvp` would.
fn resolve(program: &str, path: &str) -> Option<PathBuf> {
    if program.contains('/') {
        return Some(PathBuf::from(program));
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(program))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|meta| {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && meta.permissions().mode() & 0o111 != 0
            })
        })
}

impl AgentProcess {
    pub fn spawn(launch: &Launch) -> std::io::Result<Self> {
        let mut env = launch.env.clone();
        let mut program = PathBuf::from(&launch.program);
        if launch.use_shell_path {
            if let Some(path) = shell_path() {
                if let Some(found) = resolve(&launch.program, &path) {
                    program = found;
                }
                env.insert("PATH".to_string(), path);
            }
        }
        let mut command = Command::new(&program);
        command
            .args(&launch.args)
            .envs(&env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(dir) = &launch.working_directory {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(|error| {
            std::io::Error::new(error.kind(), format!("{}: {error}", program.display()))
        })?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel();
        if let Some(stdout) = stdout {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => {
                            if tx.send(Output::Line(line)).is_err() {
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = tx.send(Output::Exited(String::new()));
            });
        }
        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if tx.send(Output::Diagnostic(line)).is_err() {
                        return;
                    }
                }
            });
        }
        Ok(Self {
            child: Some(child),
            stdin,
            output: rx,
            pid,
            exited: false,
            stderr_tail: Vec::new(),
        })
    }

    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Writes one line (a newline is added).
    pub fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the agent's input is closed"))?;
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        stdin.write_all(&bytes)?;
        stdin.flush()
    }

    /// Everything that arrived since the last call. An `Exited` carries the
    /// exit status and the last lines of stderr.
    pub fn poll(&mut self) -> Vec<Output> {
        let mut out = Vec::new();
        while let Ok(item) = self.output.try_recv() {
            match item {
                Output::Diagnostic(line) => {
                    self.stderr_tail.push(line.clone());
                    if self.stderr_tail.len() > 8 {
                        self.stderr_tail.remove(0);
                    }
                    out.push(Output::Diagnostic(line));
                }
                Output::Exited(_) => {
                    if self.exited {
                        continue;
                    }
                    self.exited = true;
                    let status = self
                        .child
                        .as_mut()
                        .and_then(|child| {
                            // stdout closing and the process ending race by a
                            // moment; give it that moment.
                            for _ in 0..20 {
                                if let Ok(Some(status)) = child.try_wait() {
                                    return Some(status);
                                }
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            None
                        })
                        .map_or_else(|| "still running".to_string(), |status| status.to_string());
                    let tail = self.stderr_tail.join(" | ");
                    out.push(Output::Exited(if tail.is_empty() {
                        status
                    } else {
                        format!("{status}: {tail}")
                    }));
                }
                line => out.push(line),
            }
        }
        out
    }

    #[must_use]
    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// Ends the agent and everything it started. Returns at once.
    pub fn close(&mut self) {
        self.stdin = None;
        let Some(mut child) = self.child.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        let pid = self.pid;
        let _ = signal_group_or_process(pid, Signal::Term);
        std::thread::spawn(move || {
            for _ in 0..40 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = signal_group_or_process(pid, Signal::Kill);
            let _ = child.wait();
        });
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn collect_until(process: &mut AgentProcess, needle: &str) -> Vec<Output> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            let batch = process.poll();
            let done = batch.iter().any(|item| match item {
                Output::Line(line) | Output::Exited(line) | Output::Diagnostic(line) => {
                    line.contains(needle)
                }
            });
            all.extend(batch);
            if done {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    #[test]
    fn lines_go_in_and_come_out_in_order() {
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "while read line; do echo \"got:$line\"; echo oops >&2; done".into(),
            ],
            ..Launch::default()
        };
        let mut process = AgentProcess::spawn(&launch).expect("spawn");
        process.write_line("one").unwrap();
        process.write_line("two").unwrap();
        let out = collect_until(&mut process, "got:two");
        let lines: Vec<&str> = out
            .iter()
            .filter_map(|item| match item {
                Output::Line(line) => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, ["got:one", "got:two"]);
    }

    #[test]
    fn an_exit_is_reported_with_its_status_and_stderr() {
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo 'not logged in' >&2; exit 3".into()],
            ..Launch::default()
        };
        let mut process = AgentProcess::spawn(&launch).expect("spawn");
        let out = collect_until(&mut process, "exit status");
        let exited = out
            .iter()
            .find_map(|item| match item {
                Output::Exited(text) => Some(text.clone()),
                _ => None,
            })
            .expect("an exit");
        assert!(exited.contains("exit status: 3"), "{exited}");
        assert!(process.has_exited());
    }

    #[test]
    fn closing_ends_what_the_agent_started() {
        let marker = std::env::temp_dir().join(format!("mz-agent-close-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "sleep 30 & echo $! > {}; echo ready; wait",
                    marker.display()
                ),
            ],
            ..Launch::default()
        };
        let mut process = AgentProcess::spawn(&launch).expect("spawn");
        collect_until(&mut process, "ready");
        let child: i32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        process.close();
        let alive = |pid: i32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| !stat.contains(") Z "))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(child) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&marker);
        assert!(!alive(child), "the agent's own child survived close()");
    }

    #[test]
    fn discovery_collects_the_shell_answer_without_starting_an_agent() {
        let (tx, receiver) = mpsc::channel();
        let mut state = ShellPath {
            receiver,
            resolved: None,
            locations: HashMap::new(),
        };
        assert_eq!(state.locate("sh", "/nonexistent"), Location::Checking);
        tx.send(Some("/bin".into())).unwrap();
        assert_eq!(
            state.locate("sh", "/nonexistent"),
            Location::Found(PathBuf::from("/bin/sh"))
        );
        assert_eq!(
            state.locate("no-such-program-mz", "/bin"),
            Location::NotFound
        );
    }

    #[test]
    fn a_timeout_keeps_discovery_pending_and_failure_settles_on_inherited_path() {
        let (tx, receiver) = mpsc::channel();
        let mut state = ShellPath {
            receiver,
            resolved: None,
            locations: HashMap::new(),
        };
        state.poll(Duration::ZERO);
        assert_eq!(
            state.locate("no-such-program-mz", "/bin"),
            Location::Checking
        );
        assert_eq!(
            state.locate("sh", "/bin"),
            Location::Found(PathBuf::from("/bin/sh"))
        );
        drop(tx);
        assert_eq!(
            state.locate("no-such-program-mz", "/bin"),
            Location::NotFound
        );
        assert_eq!(
            state.locate("sh", "/bin"),
            Location::Found(PathBuf::from("/bin/sh"))
        );
    }

    #[test]
    fn resolve_finds_an_executable_on_a_given_path() {
        assert_eq!(
            resolve("sh", "/nonexistent:/bin"),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(resolve("/x/y", ""), Some(PathBuf::from("/x/y")));
        assert_eq!(resolve("no-such-program-mz", "/bin"), None);
    }
}
