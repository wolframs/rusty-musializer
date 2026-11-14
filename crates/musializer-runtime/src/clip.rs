//! Video clips through FFmpeg (post-legacy, 2026-11-14, VG): opening one,
//! probing it, and decoding its frames exactly, for the `clip` scene.
//!
//! # The file is opened once
//!
//! A check on a name says nothing about the file then opened (`AGENTS.md`), so
//! [`ClipSource::open`] opens the path once without blocking, asks the handle
//! whether it is a regular file of a bounded size, and from then on nothing
//! opens the path again. The digest is read from that handle, and FFmpeg gets
//! the handle as its standard input and reads `/proc/self/fd/0` — on Linux a
//! fresh open of the same inode, which a name swapped in afterwards cannot
//! reach, and which, unlike a pipe, FFmpeg can seek in. `-nostdin` keeps it
//! from also reading keystrokes off that descriptor.
//!
//! # Everything a file claims is bounded first
//!
//! The probe reads the stream's size, time base and start before it lists a
//! single frame, and refuses a size past [`clip::MAX_CLIP_SIDE`] then; the
//! frame list is read through a cap of [`clip::MAX_CLIP_FRAMES`] lines, and a
//! decoder's output is cut at exactly one frame of the probed size, with
//! `-max_pixels` stopping FFmpeg allocating for a bitstream that lied.
//!
//! # Which frame
//!
//! Frame `k` is the `k`-th frame the decoder returns from the start. A decoder
//! started part-way is told [`ClipTiming::seek_seconds`]`(k)`, which FFmpeg's
//! accurate seek turns into exactly `k` first (measured on FFmpeg 8.0, and
//! pinned by the tests below against the frames read from the start).
//!
//! # The worker
//!
//! [`ClipWorker`] owns one FFmpeg and one thread. The preview asks it for a
//! frame and draws whatever it has ([`ClipWorker::latest`]), saying when that
//! is not the one asked for; an export waits for the exact frame
//! ([`ClipWorker::wait_for`]). Moving forward it reads on; far ahead or
//! backwards it restarts FFmpeg at the frame, and backwards it decodes a
//! run of frames ending at the one asked for and keeps them, so a bounce
//! plays backwards at a restart per run rather than per frame. The choice and
//! its numbers: `docs/VIDEO_GENERATION.md`, *Decoding*.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use musializer_core::project::clip::{self, ClipTiming, TimingError};
use musializer_core::project::sha256;

use crate::process::process_group::{kill_group_and_reap, signal_group_or_process, Signal};

/// `open(2)`'s `O_NONBLOCK` on Linux x86-64 and AArch64, as in `decode`.
const O_NONBLOCK: i32 = 0o4000;
/// How long a probe may take. A healthy one takes tens of milliseconds.
const PROBE_DEADLINE: Duration = Duration::from_secs(20);
/// The most a probe's frame list may print: one `pts,flags` line a frame.
const PACKET_LINE_BYTES: usize = 48;
const STREAM_REPORT_BYTES: usize = 4096;
/// Forward by reading rather than restarting, up to this many frames. A
/// restart costs 95–340 ms on a 1080p H.264 clip and a frame read about 4 ms
/// (measured 2026-11-14; `docs/VIDEO_GENERATION.md`).
const SKIP_LIMIT: usize = 30;
/// What the frames kept for backwards playback may hold.
const CACHE_BYTES: usize = 192 * 1024 * 1024;
/// The longest run decoded and kept for one backwards step.
const BACK_RUN: usize = 24;
/// Frames read ahead of the last one asked for while nothing new is asked:
/// a played clip's next frame is then waiting when the preview asks, rather
/// than decoded after it asked and drawn a frame late.
const LOOKAHEAD: usize = 6;
/// What FFmpeg rounds a frame up to before it checks `-max_pixels`.
const FFMPEG_ALIGNMENT: u64 = 128;

/// Why a clip cannot be drawn. Each says what is wrong in words a person can
/// act on; the placeholder and the report line print them as they are.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    #[error("the file is missing")]
    Missing,
    #[error("it is not a regular file")]
    NotAFile,
    #[error("the file could not be read: {0}")]
    Read(String),
    #[error("the file is larger than {} GiB", clip::MAX_CLIP_BYTES >> 30)]
    TooLarge,
    #[error("the file changed: its SHA-256 is {found}, the project's is {expected}")]
    Changed { expected: String, found: String },
    #[error("FFmpeg could not be started: {0}")]
    NoFfmpeg(String),
    #[error("FFmpeg cannot read it as a video")]
    Unreadable,
    #[error("it has no video stream")]
    NoVideo,
    #[error("its frames are {0}x{1}, outside 2x2..{side}x{side} or past 8K's pixels", side = clip::MAX_CLIP_SIDE)]
    Size(u32, u32),
    #[error("its frames carry no timestamps")]
    NoTimestamps,
    #[error("{0}")]
    Timing(TimingError),
    #[error("it decodes as {found}, the project recorded {expected}")]
    Different { expected: String, found: String },
    #[error("it ends at frame {decoded}, though its index lists {listed} (truncated?)")]
    Truncated { decoded: usize, listed: usize },
    #[error("FFmpeg took longer than {0} s")]
    Timeout(u64),
    #[error("the decoder stopped")]
    Stopped,
}

/// A clip file, opened once and checked by its handle.
#[derive(Debug)]
pub struct ClipSource {
    file: File,
    path: PathBuf,
    bytes: u64,
}

impl ClipSource {
    /// Opens `path` without blocking (a FIFO under the name is refused, not
    /// waited on), and checks the handle: a regular file within
    /// [`clip::MAX_CLIP_BYTES`].
    ///
    /// # Errors
    ///
    /// [`ClipError::Missing`], [`ClipError::NotAFile`], [`ClipError::TooLarge`]
    /// or [`ClipError::Read`].
    pub fn open(path: &Path) -> Result<Self, ClipError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                    ClipError::Missing
                }
                _ => ClipError::Read(error.to_string()),
            })?;
        let metadata = file
            .metadata()
            .map_err(|error| ClipError::Read(error.to_string()))?;
        if !metadata.is_file() {
            return Err(ClipError::NotAFile);
        }
        if metadata.len() > clip::MAX_CLIP_BYTES {
            return Err(ClipError::TooLarge);
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            bytes: metadata.len(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The SHA-256 of the bytes behind the handle, read through the cap.
    ///
    /// # Errors
    ///
    /// A read failure, or a file that grew past the cap while being read.
    pub fn sha256_hex(&self) -> Result<String, ClipError> {
        let mut file = self
            .file
            .try_clone()
            .map_err(|error| ClipError::Read(error.to_string()))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| ClipError::Read(error.to_string()))?;
        let mut reader = file.take(clip::MAX_CLIP_BYTES + 1);
        let mut hasher = sha256::Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total = 0u64;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    hasher.update(&buffer[..count]);
                    total += count as u64;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(ClipError::Read(error.to_string())),
            }
        }
        if total > clip::MAX_CLIP_BYTES {
            return Err(ClipError::TooLarge);
        }
        Ok(sha256::hex(&hasher.finalize()))
    }

    /// The handle again, as a child's standard input.
    fn stdin(&self) -> Result<Stdio, ClipError> {
        self.file
            .try_clone()
            .map(Stdio::from)
            .map_err(|error| ClipError::Read(error.to_string()))
    }
}

/// What a probe found: enough to decode exactly and to say what the file is.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// The stream's average rate, for a report line; frame times come from
    /// [`Probe::timing`], not from this.
    pub rate: (i64, i64),
    pub timing: ClipTiming,
    /// Seconds from where FFmpeg counts a seek from (the container's start)
    /// to the video's first frame. A container whose sound starts first has
    /// one; FFmpeg's `-ss` is counted from the container's start.
    pub seek_offset: f64,
}

impl Probe {
    #[must_use]
    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// `1920x1080 h264 30/1 300 frames 10.000 s`, for the report.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{}x{} {} {}/{} {} frames {:.3} s",
            self.width,
            self.height,
            self.codec,
            self.rate.0,
            self.rate.1,
            self.timing.frame_count(),
            self.timing.duration()
        )
    }
}

fn ffmpeg_program(name: &str) -> &OsStr {
    OsStr::new(name)
}

/// Runs a short-lived FFmpeg tool over the source, reading at most `cap`
/// bytes of its output before `deadline`. Its own group, killed and reaped on
/// every way out.
fn run_bounded(
    program: &OsStr,
    args: &[&str],
    source: &ClipSource,
    cap: usize,
    deadline: Duration,
) -> Result<(Vec<u8>, bool), ClipError> {
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(source.stdin()?)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ClipError::NoFfmpeg(error.to_string()))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = kill_group_and_reap(&mut child);
        return Err(ClipError::Unreadable);
    };
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(cap as u64 + 1).read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    while !reader.is_finished() {
        if started.elapsed() >= deadline {
            let _ = kill_group_and_reap(&mut child);
            let _ = reader.join();
            return Err(ClipError::Timeout(deadline.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let out = reader.join().map_err(|_| ClipError::Unreadable)?;
    if out.len() > cap {
        let _ = kill_group_and_reap(&mut child);
        return Ok((out, false));
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => {
                let _ = kill_group_and_reap(&mut child);
                break None;
            }
        }
    };
    Ok((out, status.is_some_and(|status| status.success())))
}

fn rational(text: &str) -> Option<(i64, i64)> {
    let (num, den) = text.split_once('/')?;
    Some((num.trim().parse().ok()?, den.trim().parse().ok()?))
}

/// Probes a clip: first its stream, bounded, then its frame list.
///
/// # Errors
///
/// No FFmpeg, not a video, no video stream, a size past the bounds, no
/// timestamps, too many frames, too long.
pub fn probe(source: &ClipSource) -> Result<Probe, ClipError> {
    let ffprobe = ffmpeg_program("ffprobe");
    let input = "file:/proc/self/fd/0";
    let (stream, ok) = run_bounded(
        ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,time_base,avg_frame_rate,start_pts:format=start_time",
            "-of",
            "default=nw=1",
            input,
        ],
        source,
        STREAM_REPORT_BYTES,
        PROBE_DEADLINE,
    )?;
    if !ok {
        return Err(ClipError::Unreadable);
    }
    let text = String::from_utf8_lossy(&stream);
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "N/A")
    };
    let codec = field("codec_name").ok_or(ClipError::NoVideo)?.to_owned();
    let width: u32 = field("width")
        .and_then(|value| value.parse().ok())
        .ok_or(ClipError::NoVideo)?;
    let height: u32 = field("height")
        .and_then(|value| value.parse().ok())
        .ok_or(ClipError::NoVideo)?;
    // Refused here, before any frame is listed or decoded.
    if !clip::size_is_valid(width, height) {
        return Err(ClipError::Size(width, height));
    }
    let time_base = field("time_base")
        .and_then(rational)
        .filter(|(num, den)| *num > 0 && *den > 0)
        .ok_or(ClipError::NoTimestamps)?;
    let rate = field("avg_frame_rate")
        .and_then(rational)
        .filter(|(num, den)| *num > 0 && *den > 0)
        .unwrap_or((0, 1));
    let container_start: f64 = field("start_time")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.0);

    let cap = clip::MAX_CLIP_FRAMES as usize * PACKET_LINE_BYTES;
    let (packets, ok) = run_bounded(
        ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts,flags",
            "-of",
            "csv=p=0",
            input,
        ],
        source,
        cap,
        PROBE_DEADLINE,
    )?;
    if packets.len() > cap {
        return Err(ClipError::Timing(TimingError::TooManyFrames));
    }
    if !ok {
        return Err(ClipError::Unreadable);
    }
    let mut pts = Vec::new();
    for line in String::from_utf8_lossy(&packets).lines() {
        let (value, flags) = line.split_once(',').unwrap_or((line, ""));
        // A packet the container's edit list discards is never shown.
        if flags.contains('D') {
            continue;
        }
        let value: i64 = value.trim().parse().map_err(|_| ClipError::NoTimestamps)?;
        pts.push(value);
        if pts.len() > clip::MAX_CLIP_FRAMES as usize {
            return Err(ClipError::Timing(TimingError::TooManyFrames));
        }
    }
    let fallback = if rate.0 > 0 {
        rate.1 as f64 / rate.0 as f64
    } else {
        1.0 / 30.0
    };
    let first = pts.iter().copied().min();
    let timing = ClipTiming::from_pts(&pts, time_base, fallback).map_err(ClipError::Timing)?;
    let video_start = first.map_or(0.0, |first| {
        (i128::from(first) * i128::from(time_base.0)) as f64 / time_base.1 as f64
    });
    Ok(Probe {
        codec,
        width,
        height,
        rate,
        timing,
        seek_offset: (video_start - container_start).max(0.0),
    })
}

/// One FFmpeg decoding frames in order from some frame on. Synchronous: the
/// worker drives it.
struct Decoder {
    child: Child,
    stdout: ChildStdout,
    /// The index the next frame read is.
    next: usize,
}

impl Decoder {
    fn start(
        source: &ClipSource,
        probe: &Probe,
        index: usize,
        group: &Mutex<Option<u32>>,
    ) -> Result<Self, ClipError> {
        let mut command = Command::new(ffmpeg_program("ffmpeg"));
        command.args(["-nostdin", "-v", "error", "-max_pixels"]);
        let aligned = (u64::from(probe.width) + FFMPEG_ALIGNMENT)
            * (u64::from(probe.height) + FFMPEG_ALIGNMENT);
        command.arg(aligned.to_string());
        if index > 0 {
            let seek = probe.seek_offset + probe.timing.seek_seconds(index);
            command.args(["-ss", &format!("{seek:.9}")]);
        }
        command
            .args([
                "-i",
                "file:/proc/self/fd/0",
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
            ])
            .args(["-fps_mode", "passthrough", "-s"])
            .arg(format!("{}x{}", probe.width, probe.height))
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .process_group(0)
            .stdin(source.stdin()?)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| ClipError::NoFfmpeg(error.to_string()))?;
        *lock(group) = Some(child.id());
        let Some(stdout) = child.stdout.take() else {
            *lock(group) = None;
            let _ = kill_group_and_reap(&mut child);
            return Err(ClipError::Unreadable);
        };
        Ok(Self {
            child,
            stdout,
            next: index,
        })
    }

    /// Reads the next frame into `frame`, exactly one probed frame's bytes.
    fn read(&mut self, frame: &mut [u8]) -> Result<bool, ClipError> {
        let mut filled = 0;
        while filled < frame.len() {
            match self.stdout.read(&mut frame[filled..]) {
                Ok(0) => return Ok(false),
                Ok(count) => filled += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Ok(false),
            }
        }
        self.next += 1;
        Ok(true)
    }

    fn stop(mut self, group: &Mutex<Option<u32>>) {
        *lock(group) = None;
        let _ = kill_group_and_reap(&mut self.child);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a clip worker is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipPhase {
    /// Opening, hashing, probing.
    Starting,
    Ready(Probe),
    Failed(ClipError),
}

/// A decoded frame.
pub type Frame = Arc<Vec<u8>>;

/// What the worker shares with whoever asks.
struct State {
    phase: ClipPhase,
    wanted: Option<usize>,
    latest: Option<(usize, Frame)>,
    stop: bool,
    restarts: u64,
    decoded: u64,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// The running FFmpeg's group, so a drop can kill it without waiting for
    /// the thread, which may be inside a read.
    group: Mutex<Option<u32>>,
}

/// What a clip was asked to be: its digest, and what its project recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
}

/// Counters for a report line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStats {
    pub restarts: u64,
    pub decoded: u64,
}

/// One clip's decoder thread. Dropping it stops FFmpeg and joins the thread.
pub struct ClipWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl ClipWorker {
    /// Starts the worker: it opens `path`, checks the digest and the probe
    /// against `expected`, then waits to be asked for frames.
    #[must_use]
    pub fn spawn(path: PathBuf, expected: Expected) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: ClipPhase::Starting,
                wanted: None,
                latest: None,
                stop: false,
                restarts: 0,
                decoded: 0,
            }),
            changed: Condvar::new(),
            group: Mutex::new(None),
        });
        let inner = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("clip-decoder".into())
            .spawn(move || run(&inner, &path, &expected))
            .ok();
        if thread.is_none() {
            lock(&shared.state).phase = ClipPhase::Failed(ClipError::NoFfmpeg("no thread".into()));
        }
        Self { shared, thread }
    }

    #[must_use]
    pub fn phase(&self) -> ClipPhase {
        lock(&self.shared.state).phase.clone()
    }

    #[must_use]
    pub fn stats(&self) -> WorkerStats {
        let state = lock(&self.shared.state);
        WorkerStats {
            restarts: state.restarts,
            decoded: state.decoded,
        }
    }

    /// Asks for frame `index`, without waiting.
    pub fn request(&self, index: usize) {
        let mut state = lock(&self.shared.state);
        if state.wanted != Some(index) {
            state.wanted = Some(index);
            self.shared.changed.notify_all();
        }
    }

    /// The frame most recently decoded for a request, and its index.
    #[must_use]
    pub fn latest(&self) -> Option<(usize, Frame)> {
        lock(&self.shared.state).latest.clone()
    }

    /// Asks for frame `index` and waits for exactly that frame.
    ///
    /// # Errors
    ///
    /// The clip failed (missing, changed, unreadable, truncated), or nothing
    /// arrived within `deadline`.
    pub fn wait_for(&self, index: usize, deadline: Duration) -> Result<Frame, ClipError> {
        self.request(index);
        let started = Instant::now();
        let mut state = lock(&self.shared.state);
        loop {
            if let ClipPhase::Failed(error) = &state.phase {
                return Err(error.clone());
            }
            if let Some((at, frame)) = &state.latest {
                if *at == index {
                    return Ok(Arc::clone(frame));
                }
            }
            let left = deadline.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(ClipError::Timeout(deadline.as_secs()));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for ClipWorker {
    fn drop(&mut self) {
        lock(&self.shared.state).stop = true;
        self.shared.changed.notify_all();
        if let Some(group) = *lock(&self.shared.group) {
            let _ = signal_group_or_process(group, Signal::Kill);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The worker thread.
fn run(shared: &Shared, path: &Path, expected: &Expected) {
    let fail = |error: ClipError| {
        lock(&shared.state).phase = ClipPhase::Failed(error);
        shared.changed.notify_all();
    };
    let source = match ClipSource::open(path) {
        Ok(source) => source,
        Err(error) => return fail(error),
    };
    match source.sha256_hex() {
        Ok(found) if found == expected.sha256 => {}
        Ok(found) => {
            return fail(ClipError::Changed {
                expected: expected.sha256.clone(),
                found,
            });
        }
        Err(error) => return fail(error),
    }
    let probe = match probe(&source) {
        Ok(probe) => probe,
        Err(error) => return fail(error),
    };
    let found = (probe.width, probe.height, probe.timing.frame_count());
    if found != (expected.width, expected.height, expected.frames as usize) {
        return fail(ClipError::Different {
            expected: format!(
                "{}x{} in {} frames",
                expected.width, expected.height, expected.frames
            ),
            found: format!("{}x{} in {} frames", found.0, found.1, found.2),
        });
    }
    lock(&shared.state).phase = ClipPhase::Ready(probe.clone());
    shared.changed.notify_all();

    let mut decoder: Option<Decoder> = None;
    let mut cache: BTreeMap<usize, Frame> = BTreeMap::new();
    let capacity = (CACHE_BYTES / probe.frame_bytes().max(1)).clamp(2, BACK_RUN * 2);
    let mut served: Option<usize> = None;
    let last = probe.timing.frame_count() - 1;
    loop {
        let wanted = {
            let mut state = lock(&shared.state);
            loop {
                if state.stop {
                    if let Some(decoder) = decoder.take() {
                        drop(state);
                        decoder.stop(&shared.group);
                    }
                    return;
                }
                match state.wanted {
                    Some(wanted) if Some(wanted) != served => break wanted.min(last),
                    _ => {}
                }
                // Nothing new asked: read ahead while the decoder is just
                // past the last frame served.
                let ahead = match (&decoder, served) {
                    (Some(running), Some(served)) => {
                        running.next <= last && running.next <= served + LOOKAHEAD
                    }
                    _ => false,
                };
                if ahead {
                    break usize::MAX;
                }
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        if wanted == usize::MAX {
            let Some(running) = decoder.as_mut() else {
                continue;
            };
            let index = running.next;
            let mut frame = vec![0u8; probe.frame_bytes()];
            match running.read(&mut frame) {
                Ok(true) => {
                    lock(&shared.state).decoded += 1;
                    cache.insert(index, Arc::new(frame));
                    trim(&mut cache, capacity, served.unwrap_or(index));
                }
                // The end, or a fault the next real request will meet and
                // name; reading ahead stops here either way.
                _ => {
                    if let Some(old) = decoder.take() {
                        old.stop(&shared.group);
                    }
                }
            }
            continue;
        }
        // A bounce's way back steps a frame or two at a time and is worth a
        // decoded run; a scrub's jump back wants only its one frame.
        let backwards = served.is_some_and(|served| wanted < served && served - wanted <= 4);
        let frame = match cache.get(&wanted) {
            Some(frame) => Arc::clone(frame),
            None => {
                match produce(
                    shared,
                    &source,
                    &probe,
                    &mut decoder,
                    &mut cache,
                    capacity,
                    wanted,
                    backwards,
                ) {
                    Ok(frame) => frame,
                    Err(error) => {
                        if let Some(decoder) = decoder.take() {
                            decoder.stop(&shared.group);
                        }
                        // A stop raised mid-read is not the clip's fault.
                        if lock(&shared.state).stop {
                            return;
                        }
                        return fail(error);
                    }
                }
            }
        };
        served = Some(wanted);
        let mut state = lock(&shared.state);
        state.latest = Some((wanted, frame));
        shared.changed.notify_all();
    }
}

/// Gets frame `wanted` from the decoder: reading on when it is a little
/// ahead, restarting otherwise, and backwards decoding a run ending at it.
#[allow(
    clippy::too_many_arguments,
    reason = "the worker's own state, passed down"
)]
fn produce(
    shared: &Shared,
    source: &ClipSource,
    probe: &Probe,
    decoder: &mut Option<Decoder>,
    cache: &mut BTreeMap<usize, Frame>,
    capacity: usize,
    wanted: usize,
    backwards: bool,
) -> Result<Frame, ClipError> {
    let reachable = decoder
        .as_ref()
        .is_some_and(|decoder| decoder.next <= wanted && wanted <= decoder.next + SKIP_LIMIT);
    let keep_from = if reachable {
        wanted
    } else {
        if let Some(old) = decoder.take() {
            old.stop(&shared.group);
        }
        let start = if backwards {
            wanted.saturating_sub(BACK_RUN.min(capacity) - 1)
        } else {
            wanted
        };
        *decoder = Some(Decoder::start(source, probe, start, &shared.group)?);
        lock(&shared.state).restarts += 1;
        if backwards {
            start
        } else {
            wanted
        }
    };
    let Some(running) = decoder.as_mut() else {
        return Err(ClipError::Stopped);
    };
    loop {
        let index = running.next;
        let mut frame = vec![0u8; probe.frame_bytes()];
        if !running.read(&mut frame)? {
            if lock(&shared.state).stop {
                return Err(ClipError::Stopped);
            }
            return Err(ClipError::Truncated {
                decoded: index,
                listed: probe.timing.frame_count(),
            });
        }
        lock(&shared.state).decoded += 1;
        let frame = Arc::new(frame);
        if index >= keep_from {
            cache.insert(index, Arc::clone(&frame));
            trim(cache, capacity, wanted);
        }
        if index == wanted {
            return Ok(frame);
        }
    }
}

/// Keeps the cache to `capacity`, dropping whichever end is further from
/// `centre`.
fn trim(cache: &mut BTreeMap<usize, Frame>, capacity: usize, centre: usize) {
    while cache.len() > capacity {
        let (low, high) = (
            *cache.keys().next().unwrap_or(&0),
            *cache.keys().next_back().unwrap_or(&0),
        );
        let evict = if centre.abs_diff(low) > centre.abs_diff(high) {
            low
        } else {
            high
        };
        cache.remove(&evict);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that takes itself with it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("musializer-clip-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A small synthetic clip: testsrc2 draws a frame counter, so every frame
    /// differs from its neighbours.
    fn make_clip(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
        let path = dir.join(name);
        let status = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y"])
            .args(args)
            .arg(&path)
            .stdin(Stdio::null())
            .status()
            .expect("ffmpeg runs");
        assert!(status.success(), "{name}");
        path
    }

    fn ntsc_clip(dir: &Path) -> PathBuf {
        make_clip(
            dir,
            "ntsc.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x54:rate=24000/1001",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-g",
                "12",
                "-pix_fmt",
                "yuv420p",
            ],
        )
    }

    fn expected(source: &ClipSource, probe: &Probe) -> Expected {
        Expected {
            sha256: source.sha256_hex().unwrap(),
            width: probe.width,
            height: probe.height,
            frames: probe.timing.frame_count() as u32,
        }
    }

    fn all_frames(source: &ClipSource, probe: &Probe) -> Vec<Vec<u8>> {
        let group = Mutex::new(None);
        let mut decoder = Decoder::start(source, probe, 0, &group).unwrap();
        let mut frames = Vec::new();
        loop {
            let mut frame = vec![0u8; probe.frame_bytes()];
            if !decoder.read(&mut frame).unwrap() {
                break;
            }
            frames.push(frame);
        }
        decoder.stop(&group);
        frames
    }

    #[test]
    fn a_probe_reads_the_size_the_rate_and_every_frame_time() {
        let scratch = Scratch::new("probe");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        assert_eq!((probe.width, probe.height), (96, 54));
        assert_eq!(probe.rate, (24000, 1001));
        assert_eq!(probe.timing.frame_count(), 48);
        assert!((probe.timing.time_of(47) - 47.0 * 1001.0 / 24000.0).abs() < 1.0e-9);
        assert_eq!(all_frames(&source, &probe).len(), 48, "the decoder agrees");
    }

    #[test]
    fn a_seek_returns_exactly_the_frame_read_from_the_start() {
        let scratch = Scratch::new("seek");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        let frames = all_frames(&source, &probe);
        for pair in frames.windows(2) {
            assert_ne!(pair[0], pair[1], "testsrc2 frames all differ");
        }
        let group = Mutex::new(None);
        for index in [1, 11, 12, 13, 30, 47] {
            let mut decoder = Decoder::start(&source, &probe, index, &group).unwrap();
            let mut frame = vec![0u8; probe.frame_bytes()];
            assert!(decoder.read(&mut frame).unwrap());
            decoder.stop(&group);
            assert!(frame == frames[index], "seek to {index}");
        }
    }

    #[test]
    fn the_worker_serves_forward_far_and_backward_requests_exactly() {
        let scratch = Scratch::new("worker");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        let frames = all_frames(&source, &probe);
        let worker = ClipWorker::spawn(path.clone(), expected(&source, &probe));
        let deadline = Duration::from_secs(20);
        // Forward, a jump, then a bounce's way back down.
        for index in [0, 1, 2, 3, 40, 41, 47, 46, 45, 30, 29, 5, 0] {
            let frame = worker.wait_for(index, deadline).unwrap();
            assert!(*frame == frames[index], "frame {index}");
        }
        let stats = worker.stats();
        assert!(stats.restarts <= 5, "{stats:?}");
        assert!(matches!(worker.phase(), ClipPhase::Ready(_)));
    }

    #[test]
    fn a_changed_or_missing_file_is_named() {
        let scratch = Scratch::new("named");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        let mut wrong = expected(&source, &probe);
        wrong.sha256 = "0".repeat(64);
        let worker = ClipWorker::spawn(path.clone(), wrong);
        let error = worker.wait_for(0, Duration::from_secs(20)).unwrap_err();
        assert!(matches!(error, ClipError::Changed { .. }), "{error}");
        let mut taller = expected(&source, &probe);
        taller.frames = 49;
        let worker = ClipWorker::spawn(path, taller);
        let error = worker.wait_for(0, Duration::from_secs(20)).unwrap_err();
        assert!(error.to_string().contains("96x54 in 48 frames"), "{error}");
        let gone = scratch.0.join("gone.mp4");
        let worker = ClipWorker::spawn(gone.clone(), expected(&source, &probe));
        assert_eq!(
            worker.wait_for(0, Duration::from_secs(5)).unwrap_err(),
            ClipError::Missing
        );
        assert_eq!(ClipSource::open(&gone).unwrap_err(), ClipError::Missing);
    }

    #[test]
    fn hostile_files_are_refused_before_anything_decodes() {
        let scratch = Scratch::new("hostile");
        // A 53-byte file whose header claims 16000 x 9000: 144 megapixels,
        // a 216 MB frame. FFmpeg itself takes it (it refuses 30000 x 30000);
        // this is refused from the stream report, before a frame is listed or
        // decoded. ffprobe peaks at 53 MB reading the claim (measured).
        let huge = scratch.0.join("huge.y4m");
        std::fs::write(
            &huge,
            b"YUV4MPEG2 W16000 H9000 F30:1 Ip A1:1 C420jpeg\nFRAME\n\0\0\0\0",
        )
        .unwrap();
        let source = ClipSource::open(&huge).unwrap();
        assert_eq!(probe(&source).unwrap_err(), ClipError::Size(16000, 9000));
        // Not a video at all.
        let text = scratch.0.join("notes.mp4");
        std::fs::write(&text, b"this is not a video").unwrap();
        let source = ClipSource::open(&text).unwrap();
        assert_eq!(probe(&source).unwrap_err(), ClipError::Unreadable);
        // Truncated: the first half of a clip, index at the end and lost.
        let path = ntsc_clip(&scratch.0);
        let bytes = std::fs::read(&path).unwrap();
        let cut = scratch.0.join("cut.mp4");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let source = ClipSource::open(&cut).unwrap();
        assert!(probe(&source).is_err());
        // A FIFO under the name is refused at once, not waited on.
        let fifo = scratch.0.join("fifo.mp4");
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let (done, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(ClipSource::open(&fifo).map(|_| ()));
        });
        assert_eq!(
            answer.recv_timeout(Duration::from_secs(5)),
            Ok(Err(ClipError::NotAFile))
        );
    }

    #[test]
    fn the_handle_not_the_name_is_what_gets_read() {
        let scratch = Scratch::new("swap");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let digest = source.sha256_hex().unwrap();
        // Another file moved in under the name after the open.
        std::fs::rename(&path, scratch.0.join("moved.mp4")).unwrap();
        std::fs::write(&path, b"an impostor").unwrap();
        assert_eq!(source.sha256_hex().unwrap(), digest);
        let probe = probe(&source).unwrap();
        assert_eq!(probe.timing.frame_count(), 48, "FFmpeg read the handle");
    }

    #[test]
    fn a_stream_that_starts_after_its_container_seeks_by_its_own_frames() {
        let scratch = Scratch::new("offset");
        // Sound from 0, picture from 0.5 s.
        let path = make_clip(
            &scratch.0,
            "late.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-itsoffset",
                "0.5",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x54:rate=25",
                "-t",
                "2",
                "-map",
                "1:v",
                "-map",
                "0:a",
                "-c:v",
                "libx264",
                "-g",
                "10",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ],
        );
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        assert!(probe.seek_offset > 0.4, "{}", probe.seek_offset);
        let frames = all_frames(&source, &probe);
        let group = Mutex::new(None);
        for index in [1, 9, 10, 23] {
            let mut decoder = Decoder::start(&source, &probe, index, &group).unwrap();
            let mut frame = vec![0u8; probe.frame_bytes()];
            assert!(decoder.read(&mut frame).unwrap());
            decoder.stop(&group);
            assert!(frame == frames[index], "seek to {index}");
        }
    }
}
