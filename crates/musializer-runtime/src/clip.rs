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
//! from also reading keystrokes off that descriptor. A project's bundled clip
//! is not opened by a name at all: [`ClipSource::open_beneath`] walks its
//! record from the project's folder, so the open that is read is the open
//! the containment check was about.
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
//!
//! # One budget for every clip
//!
//! A running decoder and its kept frames are a lease from one process-wide
//! [`ClipBudget`]: at most [`BUDGET_DECODERS`] FFmpeg decoders and
//! [`BUDGET_BYTES`] of kept frames, whichever renderer asks. A worker takes a
//! lease when it starts a decoder; when the budget is full, the worker asked
//! least recently gives its lease back (its decoder stops, its kept frames
//! go) before the new one starts. A worker nobody has asked anything for
//! [`BUDGET_IDLE`] gives its lease back on its own. Either way it keeps the
//! last frame it served, so a paused picture stays. A clip whose file was
//! hashed and probed once in this process is not hashed again while the
//! file's inode, size, modification and change times are the same
//! ([`verified`]), so a worker started again for a clip coming back on screen
//! costs a decoder start, not a hash.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use musializer_core::project::clip::{self, ClipTiming, TimingError};
use musializer_core::project::sha256;

use crate::process::process_group::{kill_group_and_reap, signal_group_or_process, Signal};

/// `open(2)`'s `O_NONBLOCK` on Linux x86-64 and AArch64, as in `decode`.
const O_NONBLOCK: i32 = 0o4000;
/// `open(2)`'s `O_NOFOLLOW` and `O_DIRECTORY`, which differ between the two.
#[cfg(target_arch = "x86_64")]
const O_NOFOLLOW: i32 = 0o400_000;
#[cfg(target_arch = "x86_64")]
const O_DIRECTORY: i32 = 0o200_000;
#[cfg(target_arch = "aarch64")]
const O_NOFOLLOW: i32 = 0o100_000;
#[cfg(target_arch = "aarch64")]
const O_DIRECTORY: i32 = 0o40_000;
/// `ELOOP`, what `O_NOFOLLOW` answers for a symlink: 40 on both.
const ELOOP: i32 = 40;
/// How long a probe may take. A healthy one takes tens of milliseconds.
const PROBE_DEADLINE: Duration = Duration::from_secs(20);
/// The most a probe's frame list may print: one
/// `pts=…|duration=…|flags=…` line a frame.
const PACKET_LINE_BYTES: usize = 64;
const STREAM_REPORT_BYTES: usize = 4096;
/// Forward by reading rather than restarting, up to this many frames. A
/// restart costs 95–340 ms on a 1080p H.264 clip and a frame read about 4 ms
/// (measured 2026-11-14; `docs/VIDEO_GENERATION.md`).
const SKIP_LIMIT: usize = 30;
/// The most FFmpeg decoders all clips together run at once.
pub const BUDGET_DECODERS: usize = 4;
/// What the frames all clips keep may hold together. One lease is its share,
/// `BUDGET_BYTES / BUDGET_DECODERS`, at most: 15 frames of 1080p.
pub const BUDGET_BYTES: usize = 512 * 1024 * 1024;
/// How long a worker nobody asks anything keeps its decoder and its frames.
pub const BUDGET_IDLE: Duration = Duration::from_secs(3);
/// How long a new decoder waits for an evicted one to give its lease back
/// before it starts anyway (and the overrun is counted).
const LEASE_WAIT: Duration = Duration::from_secs(2);
/// How many verified files the identity cache remembers.
const VERIFIED_FILES: usize = 128;
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
    /// A project's record names a file outside the project's folder (`../`,
    /// an absolute path, a symlink leading out). Never opened.
    #[error("its path leaves the project's folder, so it is not read")]
    OutsideProject,
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

/// Where a clip's bytes are, and whether they must stay in a project's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipLocation {
    /// A file the person chose: an import not yet saved, a recovery's
    /// absolute runtime path, a treatment's clip.
    File(PathBuf),
    /// A project's bundled copy: `relative` is the record, walked from the
    /// project's folder one name at a time, none of them a symlink
    /// ([`ClipSource::open_beneath`]).
    Bundled { folder: PathBuf, relative: String },
}

impl ClipLocation {
    /// The path a person is shown.
    #[must_use]
    pub fn display_path(&self) -> PathBuf {
        match self {
            Self::File(path) => path.clone(),
            Self::Bundled { folder, relative } => folder.join(relative),
        }
    }
}

impl From<PathBuf> for ClipLocation {
    fn from(path: PathBuf) -> Self {
        Self::File(path)
    }
}

/// A clip file, opened once and checked by its handle.
#[derive(Debug)]
pub struct ClipSource {
    file: File,
    path: PathBuf,
    bytes: u64,
}

impl ClipSource {
    /// Opens `location`: a plain file by [`ClipSource::open`], a bundled one
    /// by [`ClipSource::open_beneath`].
    ///
    /// # Errors
    ///
    /// As those two.
    pub fn open_at(location: &ClipLocation) -> Result<Self, ClipError> {
        match location {
            ClipLocation::File(path) => Self::open(path),
            ClipLocation::Bundled { folder, relative } => Self::open_beneath(folder, relative),
        }
    }

    /// Opens the project-relative `relative` inside `folder`, so that what is
    /// opened is what the containment check was about: the folder is opened
    /// once, and each recorded name is opened in the directory the one before
    /// it opened (through `/proc/self/fd/N/name`, which the kernel resolves
    /// from that directory's handle, never from its path again), with
    /// `O_NOFOLLOW`. A name swapped for a symlink — the file or any directory
    /// on the way — answers `ELOOP` and is refused unopened; nothing outside
    /// the folder is ever opened, let alone read. The folder itself is the
    /// project's, and is followed like any path the person opened.
    ///
    /// # Errors
    ///
    /// [`ClipError::OutsideProject`] for a record that is not plainly
    /// relative or a symlink on the way; otherwise as [`ClipSource::open`].
    pub fn open_beneath(folder: &Path, relative: &str) -> Result<Self, ClipError> {
        let names: Vec<&str> = relative.split('/').collect();
        let plain = |name: &&str| {
            !name.is_empty() && *name != "." && *name != ".." && !name.contains(['\\', '\0'])
        };
        if relative.starts_with('/') || !names.iter().all(plain) {
            return Err(ClipError::OutsideProject);
        }
        let refused = |error: std::io::Error| match error.raw_os_error() {
            Some(ELOOP) => ClipError::OutsideProject,
            _ => match error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                    ClipError::Missing
                }
                _ => ClipError::Read(error.to_string()),
            },
        };
        let mut directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECTORY)
            .open(folder)
            .map_err(refused)?;
        let (last, through) = names.split_last().ok_or(ClipError::OutsideProject)?;
        for name in through {
            let entry = in_directory(&directory, name);
            directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(O_DIRECTORY | O_NOFOLLOW)
                .open(&entry)
                .map_err(|error| {
                    // `O_DIRECTORY` answers a symlink with `ENOTDIR`, not
                    // `ELOOP`; it is still a way out, not a missing file.
                    if entry.symlink_metadata().is_ok_and(|meta| meta.is_symlink()) {
                        ClipError::OutsideProject
                    } else {
                        refused(error)
                    }
                })?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK | O_NOFOLLOW)
            .open(in_directory(&directory, last))
            .map_err(refused)?;
        Self::checked(file, folder.join(relative))
    }

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
        Self::checked(file, path.to_path_buf())
    }

    /// The handle's own answer: a regular file within the cap.
    fn checked(file: File, path: PathBuf) -> Result<Self, ClipError> {
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
            path,
            bytes: metadata.len(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A name for this handle's own file while it is held: another reader
    /// (the save's bundling) opens this and reaches the inode checked here,
    /// whatever has been put under [`ClipSource::path`] since.
    #[must_use]
    pub fn held_path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd()))
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
        self.sha256_hex_until(&|| false)
    }

    /// [`ClipSource::sha256_hex`], looking at `stopped` between reads: a
    /// worker being dropped must not wait for a multi-gigabyte hash.
    fn sha256_hex_until(&self, stopped: &dyn Fn() -> bool) -> Result<String, ClipError> {
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
            if stopped() {
                return Err(ClipError::Stopped);
            }
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

/// `name` inside the directory `directory` holds open: the magic link
/// `/proc/self/fd/N` is the directory's inode itself, however its path has
/// changed since, so only `name` is looked up.
fn in_directory(directory: &File, name: &str) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name)
}

/// What a probe found: enough to decode exactly and to say what the file is.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub codec: String,
    /// The size a player shows: the coded size turned by the stream's display
    /// matrix. FFmpeg turns the picture itself (its autorotate), so this is
    /// also the size its decoder's frames come out at.
    pub width: u32,
    pub height: u32,
    /// The display matrix turns the picture a quarter or three quarters, so
    /// the coded size is `height x width`.
    pub turned: bool,
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

/// The FFmpeg tools a clip runs, by name on `PATH`. A test swaps in a
/// stand-in to hold a start-up still.
#[derive(Clone, Debug)]
struct Programs {
    ffmpeg: OsString,
    ffprobe: OsString,
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            ffmpeg: OsString::from("ffmpeg"),
            ffprobe: OsString::from("ffprobe"),
        }
    }
}

/// What lets another thread stop a worker's FFmpeg tools: whether it has
/// been asked to, and the running child's process group, which a drop kills
/// without waiting. The thread clears the group under its lock before it
/// reaps the child, and the drop signals under the same lock, so a reaped id
/// (free for reuse) is never signalled.
struct Interrupt<'a> {
    stopped: &'a dyn Fn() -> bool,
    group: &'a Mutex<Option<u32>>,
}

/// Kills and reaps a child whose group `interrupt` holds, clearing it first.
fn end_child(child: &mut Child, interrupt: &Interrupt<'_>) {
    let mut group = lock(interrupt.group);
    *group = None;
    let _ = kill_group_and_reap(child);
}

/// Runs a short-lived FFmpeg tool over the source, reading at most `cap`
/// bytes of its output before `deadline`. Its own group, registered with
/// `interrupt` so a drop can kill it, and killed and reaped on every way out.
fn run_bounded(
    program: &OsStr,
    args: &[&str],
    source: &ClipSource,
    cap: usize,
    deadline: Duration,
    interrupt: &Interrupt<'_>,
) -> Result<(Vec<u8>, bool), ClipError> {
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(source.stdin()?)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ClipError::NoFfmpeg(error.to_string()))?;
    *lock(interrupt.group) = Some(child.id());
    let Some(stdout) = child.stdout.take() else {
        end_child(&mut child, interrupt);
        return Err(ClipError::Unreadable);
    };
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(cap as u64 + 1).read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    while !reader.is_finished() {
        // A stop raised before the group was registered is seen here.
        if (interrupt.stopped)() {
            end_child(&mut child, interrupt);
            let _ = reader.join();
            return Err(ClipError::Stopped);
        }
        if started.elapsed() >= deadline {
            end_child(&mut child, interrupt);
            let _ = reader.join();
            return Err(ClipError::Timeout(deadline.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let out = reader.join().map_err(|_| ClipError::Unreadable)?;
    if out.len() > cap {
        end_child(&mut child, interrupt);
        return Ok((out, false));
    }
    let status = loop {
        let polled = {
            // Reaped under the lock, so the group is cleared before a drop
            // can read it again.
            let mut group = lock(interrupt.group);
            let polled = child.try_wait();
            if matches!(polled, Ok(Some(_))) {
                *group = None;
            }
            polled
        };
        match polled {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < deadline && !(interrupt.stopped)() => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => {
                end_child(&mut child, interrupt);
                break None;
            }
        }
    };
    Ok((out, status.is_some_and(|status| status.success())))
}

/// Whether a display matrix's rotation, in degrees, turns the picture by an
/// odd number of quarters: what FFmpeg's autorotate transposes for. Other
/// angles it turns inside the coded size (its `rotate` filter keeps it).
fn turns_a_quarter(degrees: f64) -> bool {
    if !degrees.is_finite() {
        return false;
    }
    let quarters = (degrees / 90.0).round();
    (degrees - quarters * 90.0).abs() < 1.0 && (quarters as i64).rem_euclid(2) == 1
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
    let group = Mutex::new(None);
    let interrupt = Interrupt {
        stopped: &|| false,
        group: &group,
    };
    probe_with(source, &Programs::default(), &interrupt)
}

/// [`probe`] with the tools and the interrupt a worker brings.
fn probe_with(
    source: &ClipSource,
    programs: &Programs,
    interrupt: &Interrupt<'_>,
) -> Result<Probe, ClipError> {
    let ffprobe = programs.ffprobe.as_os_str();
    let input = "file:/proc/self/fd/0";
    let (stream, ok) = run_bounded(
        ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,time_base,avg_frame_rate,start_pts:stream_side_data=rotation:format=start_time",
            "-of",
            "default=nw=1",
            input,
        ],
        source,
        STREAM_REPORT_BYTES,
        PROBE_DEADLINE,
        interrupt,
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
    let coded_width: u32 = field("width")
        .and_then(|value| value.parse().ok())
        .ok_or(ClipError::NoVideo)?;
    let coded_height: u32 = field("height")
        .and_then(|value| value.parse().ok())
        .ok_or(ClipError::NoVideo)?;
    // A phone records portrait as landscape pixels and a display matrix that
    // turns them. FFmpeg's decoder turns the frames by it (autorotate), so a
    // decoder asked for the coded size would squeeze the turned picture back
    // into it.
    let turned = field("rotation")
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(turns_a_quarter);
    let (width, height) = if turned {
        (coded_height, coded_width)
    } else {
        (coded_width, coded_height)
    };
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
            "packet=pts,duration,flags",
            "-of",
            "compact=p=0",
            input,
        ],
        source,
        cap,
        PROBE_DEADLINE,
        interrupt,
    )?;
    if packets.len() > cap {
        return Err(ClipError::Timing(TimingError::TooManyFrames));
    }
    if !ok {
        return Err(ClipError::Unreadable);
    }
    // Keyed, not positional: ffprobe prints a section's fields in its own
    // order whatever order they are asked for.
    let mut frames: Vec<(i64, Option<i64>)> = Vec::new();
    for line in String::from_utf8_lossy(&packets).lines() {
        let value = |key: &str| {
            line.split('|')
                .find_map(|field| field.strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
        };
        // A packet the container's edit list discards is never shown.
        if value("flags").is_some_and(|flags| flags.contains('D')) {
            continue;
        }
        let pts: i64 = value("pts")
            .and_then(|pts| pts.parse().ok())
            .ok_or(ClipError::NoTimestamps)?;
        // `N/A` when the container records none.
        let duration = value("duration").and_then(|duration| duration.parse().ok());
        frames.push((pts, duration));
        if frames.len() > clip::MAX_CLIP_FRAMES as usize {
            return Err(ClipError::Timing(TimingError::TooManyFrames));
        }
    }
    let fallback = if rate.0 > 0 {
        rate.1 as f64 / rate.0 as f64
    } else {
        1.0 / 30.0
    };
    let first = frames.iter().map(|&(pts, _)| pts).min();
    let timing =
        ClipTiming::from_packets(&frames, time_base, fallback).map_err(ClipError::Timing)?;
    let video_start = first.map_or(0.0, |first| {
        (i128::from(first) * i128::from(time_base.0)) as f64 / time_base.1 as f64
    });
    Ok(Probe {
        codec,
        width,
        height,
        turned,
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
    #[cfg(test)]
    fn start(
        source: &ClipSource,
        probe: &Probe,
        index: usize,
        group: &Mutex<Option<u32>>,
    ) -> Result<Self, ClipError> {
        Self::start_with(source, probe, index, &Programs::default(), group)
    }

    fn start_with(
        source: &ClipSource,
        probe: &Probe,
        index: usize,
        programs: &Programs,
        group: &Mutex<Option<u32>>,
    ) -> Result<Self, ClipError> {
        let mut command = Command::new(&programs.ffmpeg);
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
        // Cleared and reaped under the lock: a drop signalling the group
        // holds it too, so it never signals a reaped id.
        let mut held = lock(group);
        *held = None;
        let _ = kill_group_and_reap(&mut self.child);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Orders requests across every worker, so the budget can tell which was
/// asked least recently.
static ASKED: AtomicU64 = AtomicU64::new(1);
/// Workers alive in this process, for the report.
static WORKERS: AtomicUsize = AtomicUsize::new(0);

/// The decoders and kept frames all clip workers share (see the module
/// comment). Locks are taken budget first, then a worker's state, never the
/// other way round.
pub struct ClipBudget {
    decoders: usize,
    bytes: usize,
    idle: Duration,
    state: Mutex<BudgetState>,
    released: Condvar,
}

#[derive(Default)]
struct BudgetState {
    leases: Vec<Lease>,
    evictions: u64,
    overruns: u64,
}

struct Lease {
    worker: u64,
    bytes: usize,
    /// Asked to give it back, and not yet done.
    recalled: bool,
    shared: Weak<Shared>,
}

/// What the budget holds now, for a report line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BudgetUse {
    pub decoders: usize,
    pub decoder_limit: usize,
    pub bytes: usize,
    pub byte_limit: usize,
    pub workers: usize,
    /// Leases taken back from a worker to make room for another.
    pub evictions: u64,
    /// Decoders started past the budget because an eviction took too long.
    pub overruns: u64,
}

impl BudgetUse {
    /// `decoders 1/4, frames 120/512 MiB, workers 3, evictions 5, overruns 0`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "decoders {}/{}, frames {}/{} MiB, workers {}, evictions {}, overruns {}",
            self.decoders,
            self.decoder_limit,
            self.bytes >> 20,
            self.byte_limit >> 20,
            self.workers,
            self.evictions,
            self.overruns
        )
    }
}

impl ClipBudget {
    #[must_use]
    pub fn new(decoders: usize, bytes: usize, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            decoders: decoders.max(1),
            bytes,
            idle,
            state: Mutex::new(BudgetState::default()),
            released: Condvar::new(),
        })
    }

    /// The process's budget, which every [`ClipWorker::spawn`] draws on.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<ClipBudget>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Self::new(BUDGET_DECODERS, BUDGET_BYTES, BUDGET_IDLE)))
    }

    #[must_use]
    pub fn usage(&self) -> BudgetUse {
        let state = lock(&self.state);
        BudgetUse {
            decoders: state.leases.len(),
            decoder_limit: self.decoders,
            bytes: state.leases.iter().map(|lease| lease.bytes).sum(),
            byte_limit: self.bytes,
            workers: WORKERS.load(Ordering::Relaxed),
            evictions: state.evictions,
            overruns: state.overruns,
        }
    }

    /// The frames one lease may keep, for frames of `frame_bytes`.
    fn capacity(&self, frame_bytes: usize) -> usize {
        (self.bytes / self.decoders / frame_bytes.max(1)).clamp(2, BACK_RUN * 2)
    }

    /// Takes a lease of `bytes` for `worker`, recalling the least recently
    /// asked worker's lease while the budget is full, and waiting for it to
    /// be given back. One lease is always granted, whatever its size.
    fn acquire(&self, worker: u64, bytes: usize, shared: &Arc<Shared>) {
        let started = Instant::now();
        let mut state = lock(&self.state);
        loop {
            state.leases.retain(|lease| lease.shared.strong_count() > 0);
            let used: usize = state.leases.iter().map(|lease| lease.bytes).sum();
            let fits = state.leases.len() < self.decoders && used + bytes <= self.bytes;
            if fits || state.leases.is_empty() {
                break;
            }
            if started.elapsed() >= LEASE_WAIT {
                state.overruns += 1;
                break;
            }
            // Recall the least recently asked lease not already recalled.
            let victim = state
                .leases
                .iter_mut()
                .filter(|lease| !lease.recalled && lease.worker != worker)
                .filter_map(|lease| {
                    let other = lease.shared.upgrade()?;
                    let asked = lock(&other.state).asked;
                    Some((asked, lease, other))
                })
                .min_by_key(|(asked, _, _)| *asked);
            if let Some((_, lease, other)) = victim {
                lease.recalled = true;
                lock(&other.state).recall = true;
                other.changed.notify_all();
                state.evictions += 1;
            }
            let left = LEASE_WAIT.saturating_sub(started.elapsed());
            state = self
                .released
                .wait_timeout(state, left.min(Duration::from_millis(50)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.leases.push(Lease {
            worker,
            bytes,
            recalled: false,
            shared: Arc::downgrade(shared),
        });
    }

    fn release(&self, worker: u64) {
        lock(&self.state)
            .leases
            .retain(|lease| lease.worker != worker);
        self.released.notify_all();
    }
}

/// A worker's hold on its lease: taken before a decoder starts, given back
/// when the decoder and the kept frames go, and on every way out of the
/// thread.
struct Holder<'a> {
    budget: &'a ClipBudget,
    worker: u64,
    bytes: usize,
    shared: &'a Arc<Shared>,
    held: bool,
}

impl Holder<'_> {
    fn take(&mut self) {
        if !self.held {
            self.budget.acquire(self.worker, self.bytes, self.shared);
            self.held = true;
        }
    }

    fn give_back(&mut self) {
        if self.held {
            self.budget.release(self.worker);
            self.held = false;
        }
    }
}

impl Drop for Holder<'_> {
    fn drop(&mut self) {
        self.give_back();
    }
}

/// What makes a file the same file without reading it again: its inode, its
/// size, and its modification and change times to the nanosecond. Any write
/// moves the change time, which no program can set back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileKey {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileKey {
    fn of(file: &File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

/// Files this process hashed and probed and found to be the clip they were
/// asked to be: the digest and the probe, by [`FileKey`].
fn verified() -> &'static Mutex<Vec<(FileKey, String, Probe)>> {
    static VERIFIED: OnceLock<Mutex<Vec<(FileKey, String, Probe)>>> = OnceLock::new();
    VERIFIED.get_or_init(|| Mutex::new(Vec::new()))
}

fn remember_verified(key: FileKey, sha256: &str, probe: &Probe) {
    let mut known = lock(verified());
    known.retain(|(other, _, _)| *other != key);
    if known.len() >= VERIFIED_FILES {
        known.remove(0);
    }
    known.push((key, sha256.to_owned(), probe.clone()));
}

fn recall_verified(key: FileKey, sha256: &str) -> Option<Probe> {
    lock(verified())
        .iter()
        .find(|(other, digest, _)| *other == key && digest == sha256)
        .map(|(_, _, probe)| probe.clone())
}

/// How long the clip with digest `sha256` lasts by its probe, once a worker
/// in this process has verified and probed a file with those bytes: the
/// length playback folds against. A length is a function of the bytes, so
/// any file with the digest answers for all.
#[must_use]
pub fn probed_duration(sha256: &str) -> Option<f64> {
    lock(verified())
        .iter()
        .rev()
        .find(|(_, digest, _)| digest == sha256)
        .map(|(_, _, probe)| probe.timing.duration())
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
    /// The budget wants this worker's lease back.
    recall: bool,
    /// When this worker was last asked for a frame, on [`ASKED`]'s clock.
    asked: u64,
    restarts: u64,
    decoded: u64,
    /// Whether the last start-up skipped the hash and the probe because the
    /// file was verified before.
    reused_identity: bool,
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
    pub reused_identity: bool,
}

/// One clip's decoder thread. Dropping it stops FFmpeg and joins the thread.
pub struct ClipWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    budget: Arc<ClipBudget>,
}

impl ClipWorker {
    /// Starts the worker: it opens `location`, checks the digest and the probe
    /// against `expected`, then waits to be asked for frames.
    #[must_use]
    pub fn spawn(location: impl Into<ClipLocation>, expected: Expected) -> Self {
        Self::spawn_with(
            location.into(),
            expected,
            Programs::default(),
            ClipBudget::shared(),
        )
    }

    /// [`ClipWorker::spawn`], drawing on `budget` instead of the process's.
    #[must_use]
    pub fn spawn_in(
        location: impl Into<ClipLocation>,
        expected: Expected,
        budget: Arc<ClipBudget>,
    ) -> Self {
        Self::spawn_with(location.into(), expected, Programs::default(), budget)
    }

    fn spawn_with(
        location: ClipLocation,
        expected: Expected,
        programs: Programs,
        budget: Arc<ClipBudget>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: ClipPhase::Starting,
                wanted: None,
                latest: None,
                stop: false,
                recall: false,
                asked: ASKED.fetch_add(1, Ordering::Relaxed),
                restarts: 0,
                decoded: 0,
                reused_identity: false,
            }),
            changed: Condvar::new(),
            group: Mutex::new(None),
        });
        WORKERS.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::clone(&shared);
        let thread_budget = Arc::clone(&budget);
        let thread = std::thread::Builder::new()
            .name("clip-decoder".into())
            .spawn(move || run(&inner, &location, &expected, &programs, &thread_budget))
            .ok();
        if thread.is_none() {
            lock(&shared.state).phase = ClipPhase::Failed(ClipError::NoFfmpeg("no thread".into()));
        }
        Self {
            shared,
            thread,
            budget,
        }
    }

    /// What the budget this worker draws on holds now.
    #[must_use]
    pub fn budget(&self) -> BudgetUse {
        self.budget.usage()
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
            reused_identity: state.reused_identity,
        }
    }

    /// Asks for frame `index`, without waiting.
    pub fn request(&self, index: usize) {
        let mut state = lock(&self.shared.state);
        state.asked = ASKED.fetch_add(1, Ordering::Relaxed);
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
        // Signalled under the lock the thread clears it under before a reap.
        // Whatever starts after this sees `stop` and ends itself: the hash
        // between reads, a probe in its poll, a decoder right after its spawn.
        if let Some(group) = *lock(&self.shared.group) {
            let _ = signal_group_or_process(group, Signal::Kill);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        WORKERS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The worker thread.
fn run(
    shared: &Arc<Shared>,
    location: &ClipLocation,
    expected: &Expected,
    programs: &Programs,
    budget: &ClipBudget,
) {
    let stopped = || lock(&shared.state).stop;
    let fail = |error: ClipError| {
        // A stop is nobody's failure: whoever asked is gone.
        if error == ClipError::Stopped || stopped() {
            return;
        }
        lock(&shared.state).phase = ClipPhase::Failed(error);
        shared.changed.notify_all();
    };
    let interrupt = Interrupt {
        stopped: &stopped,
        group: &shared.group,
    };
    let source = match ClipSource::open_at(location) {
        Ok(source) => source,
        Err(error) => return fail(error),
    };
    let key = FileKey::of(&source.file);
    let known = key.and_then(|key| recall_verified(key, &expected.sha256));
    lock(&shared.state).reused_identity = known.is_some();
    let probe = if let Some(probe) = known {
        probe
    } else {
        match source.sha256_hex_until(&stopped) {
            Ok(found) if found == expected.sha256 => {}
            Ok(found) => {
                return fail(ClipError::Changed {
                    expected: expected.sha256.clone(),
                    found,
                });
            }
            Err(error) => return fail(error),
        }
        match probe_with(&source, programs, &interrupt) {
            Ok(probe) => probe,
            Err(error) => return fail(error),
        }
    };
    let found = (probe.width, probe.height, probe.timing.frame_count());
    // A turned clip recorded before its turn was read carries its coded
    // size; it is the same clip, now shown upright.
    let coded = (probe.height, probe.width, probe.timing.frame_count());
    let recorded = (expected.width, expected.height, expected.frames as usize);
    if found != recorded && !(probe.turned && coded == recorded) {
        return fail(ClipError::Different {
            expected: format!(
                "{}x{} in {} frames",
                expected.width, expected.height, expected.frames
            ),
            found: format!("{}x{} in {} frames", found.0, found.1, found.2),
        });
    }
    if let Some(key) = key {
        remember_verified(key, &expected.sha256, &probe);
    }
    lock(&shared.state).phase = ClipPhase::Ready(probe.clone());
    shared.changed.notify_all();

    let mut decoder: Option<Decoder> = None;
    let mut cache: BTreeMap<usize, Frame> = BTreeMap::new();
    let capacity = budget.capacity(probe.frame_bytes());
    let mut holder = Holder {
        budget,
        worker: Arc::as_ptr(shared) as u64,
        bytes: capacity * probe.frame_bytes(),
        shared,
        held: false,
    };
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
                // Recalled by the budget, or nobody asked anything for a
                // while: the decoder and the kept frames go, the lease goes
                // back. The last frame served stays, so a paused picture
                // does not turn into a card.
                if state.recall {
                    state.recall = false;
                    drop(state);
                    if let Some(decoder) = decoder.take() {
                        decoder.stop(&shared.group);
                    }
                    cache.clear();
                    holder.give_back();
                    state = lock(&shared.state);
                    continue;
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
                if holder.held {
                    let (next, timeout) = shared
                        .changed
                        .wait_timeout(state, budget.idle)
                        .unwrap_or_else(PoisonError::into_inner);
                    state = next;
                    if timeout.timed_out() {
                        // Nothing changed for the whole wait: idle.
                        state.recall = true;
                    }
                } else {
                    state = shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
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
                    programs,
                    &mut holder,
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
    programs: &Programs,
    holder: &mut Holder<'_>,
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
        // The budget first: this may wait for another worker to stop its
        // decoder and drop its frames.
        holder.take();
        let started = Decoder::start_with(source, probe, start, programs, &shared.group)?;
        // A drop that read the group before this spawn registered it.
        if lock(&shared.state).stop {
            started.stop(&shared.group);
            return Err(ClipError::Stopped);
        }
        *decoder = Some(started);
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

    /// A bundled clip is opened through its record, never by a name a swap
    /// can redirect: the file replaced by a symlink out, then the directory
    /// above it replaced by one, each with an outside copy of the same bytes
    /// waiting at the other end. Both refuse without opening it; by name
    /// ([`ClipSource::open`]) both would have read the outside file.
    #[test]
    fn a_bundled_clip_is_opened_beneath_its_folder() {
        let scratch = Scratch::new("beneath");
        let project = scratch.0.join("project");
        let clips = project.join("song.assets/clips");
        std::fs::create_dir_all(&clips).unwrap();
        let outside = scratch.0.join("outside/clips");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(clips.join("take.mp4"), b"bundled bytes").unwrap();
        std::fs::write(outside.join("take.mp4"), b"bundled bytes").unwrap();
        let relative = "song.assets/clips/take.mp4";

        let opened = ClipSource::open_beneath(&project, relative).expect("inside");
        assert_eq!(opened.bytes(), 13);
        assert_eq!(opened.path(), project.join(relative));

        // The file itself.
        std::fs::remove_file(clips.join("take.mp4")).unwrap();
        std::os::unix::fs::symlink(outside.join("take.mp4"), clips.join("take.mp4")).unwrap();
        assert!(ClipSource::open(&clips.join("take.mp4")).is_ok(), "by name");
        assert_eq!(
            ClipSource::open_beneath(&project, relative).unwrap_err(),
            ClipError::OutsideProject,
            "file swapped for a symlink"
        );

        // A directory above it.
        std::fs::remove_file(clips.join("take.mp4")).unwrap();
        std::fs::remove_dir(&clips).unwrap();
        std::os::unix::fs::symlink(&outside, &clips).unwrap();
        assert!(ClipSource::open(&clips.join("take.mp4")).is_ok(), "by name");
        assert_eq!(
            ClipSource::open_beneath(&project, relative).unwrap_err(),
            ClipError::OutsideProject,
            "directory swapped for a symlink"
        );

        // Missing stays missing, and a record that is not plainly relative is
        // refused before anything is opened.
        std::fs::remove_file(&clips).unwrap();
        assert_eq!(
            ClipSource::open_beneath(&project, relative).unwrap_err(),
            ClipError::Missing
        );
        for bad in [
            "../outside/clips/take.mp4",
            "/etc/hostname",
            "a//b",
            "./a",
            "",
        ] {
            assert_eq!(
                ClipSource::open_beneath(&project, bad).unwrap_err(),
                ClipError::OutsideProject,
                "{bad}"
            );
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

    /// F1b defect 4, against FFmpeg: frames at 0, 0.04 and 0.08 s, the last
    /// one's packet lasting a second (rewritten with the `setts` filter, as a
    /// container that records a held frame does). The probe reads 1.08 s.
    #[test]
    fn a_probe_reads_how_long_the_last_frame_lasts() {
        let scratch = Scratch::new("held");
        let short = make_clip(
            &scratch.0,
            "short.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x54:rate=25",
                "-frames:v",
                "3",
                "-c:v",
                "libx264",
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-video_track_timescale",
                "12800",
            ],
        );
        let held = make_clip(
            &scratch.0,
            "held.mp4",
            &[
                "-i",
                short.to_str().unwrap(),
                "-c",
                "copy",
                "-bsf:v",
                "setts=duration='if(eq(N,2),12800,DURATION)'",
            ],
        );
        let source = ClipSource::open(&held).unwrap();
        let probe = probe(&source).unwrap();
        assert_eq!(probe.timing.frame_count(), 3);
        assert!((probe.timing.time_of(2) - 0.08).abs() < 1.0e-9);
        assert!(
            (probe.timing.duration() - 1.08).abs() < 1.0e-9,
            "{}",
            probe.timing.duration()
        );
    }

    /// F1b defect 5: a clip whose display matrix turns it a quarter (a phone
    /// video) is the size a player shows, upright, and every frame the
    /// decoder returns is byte for byte the frame FFmpeg itself shows — not
    /// the turned picture squeezed back into the coded size.
    #[test]
    fn a_turned_clip_decodes_as_ffmpeg_shows_it() {
        let scratch = Scratch::new("turned");
        let flat = make_clip(
            &scratch.0,
            "flat.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x54:rate=25",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-g",
                "10",
                "-pix_fmt",
                "yuv420p",
            ],
        );
        for (degrees, turned) in [("90", true), ("-90", true), ("180", false)] {
            let name = format!("turned{degrees}.mp4");
            let path = make_clip(
                &scratch.0,
                &name,
                &[
                    "-display_rotation:v:0",
                    degrees,
                    "-i",
                    flat.to_str().unwrap(),
                    "-c",
                    "copy",
                ],
            );
            let source = ClipSource::open(&path).unwrap();
            let probe = probe(&source).unwrap();
            let size = if turned { (54, 96) } else { (96, 54) };
            assert_eq!((probe.width, probe.height), size, "{degrees}");
            assert_eq!(probe.turned, turned, "{degrees}");
            // FFmpeg's own frames, turned by its default autorotate.
            let shown = Command::new("ffmpeg")
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&path)
                .args([
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "-",
                ])
                .stdin(Stdio::null())
                .output()
                .unwrap()
                .stdout;
            let ours: Vec<u8> = all_frames(&source, &probe).concat();
            assert_eq!(ours.len(), shown.len(), "{degrees}: frame count and size");
            assert!(ours == shown, "{degrees}: the frames differ from FFmpeg's");
            // A seek lands on the same turned frame.
            let group = Mutex::new(None);
            let mut decoder = Decoder::start(&source, &probe, 13, &group).unwrap();
            let mut frame = vec![0u8; probe.frame_bytes()];
            assert!(decoder.read(&mut frame).unwrap());
            decoder.stop(&group);
            let at = 13 * probe.frame_bytes();
            assert!(
                frame == shown[at..at + probe.frame_bytes()],
                "{degrees}: seek"
            );
        }
        assert!(turns_a_quarter(270.0) && turns_a_quarter(-270.0) && turns_a_quarter(90.4));
        assert!(!turns_a_quarter(0.0) && !turns_a_quarter(45.0) && !turns_a_quarter(f64::NAN));
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

    /// F1b defect 7: dropping a worker while it hashes a large clip returns
    /// at once. The hash looks at `stop` between reads; before, the drop
    /// joined a thread that read all 3 GiB first.
    #[test]
    fn dropping_a_worker_cancels_its_hash() {
        let scratch = Scratch::new("drop-hash");
        let big = scratch.0.join("big.mp4");
        // Sparse: three gibibytes of zeros that take no disk.
        File::create(&big).unwrap().set_len(3 << 30).unwrap();
        let worker = ClipWorker::spawn(
            big,
            Expected {
                sha256: "0".repeat(64),
                width: 96,
                height: 54,
                frames: 48,
            },
        );
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(worker.phase(), ClipPhase::Starting, "still hashing");
        let started = Instant::now();
        drop(worker);
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(500),
            "the drop waited {took:?}"
        );
    }

    /// F1b defect 7: dropping a worker while its probe runs kills the probe,
    /// which is in a process group the drop can reach, and returns at once
    /// rather than at the probe's 20 s deadline. A stand-in ffprobe that only
    /// sleeps holds the start-up still.
    #[test]
    fn dropping_a_worker_kills_its_probe() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("drop-probe");
        let path = ntsc_clip(&scratch.0);
        let source = ClipSource::open(&path).unwrap();
        let real = probe(&source).unwrap();
        let marker = format!("37.{}", std::process::id());
        let stand_in = scratch.0.join("ffprobe-that-sleeps");
        std::fs::write(&stand_in, format!("#!/bin/sh\nexec sleep {marker}\n")).unwrap();
        std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sleeping = || {
            Command::new("pgrep")
                .args(["-f", &format!("sleep {marker}")])
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        let worker = ClipWorker::spawn_with(
            path.into(),
            expected(&source, &real),
            Programs {
                ffprobe: stand_in.into_os_string(),
                ..Programs::default()
            },
            ClipBudget::new(2, usize::MAX, Duration::from_secs(60)),
        );
        let waited = Instant::now();
        while !sleeping() {
            assert!(
                waited.elapsed() < Duration::from_secs(5),
                "the probe never started"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let started = Instant::now();
        drop(worker);
        let took = started.elapsed();
        assert!(took < Duration::from_secs(1), "the drop waited {took:?}");
        assert!(!sleeping(), "the probe outlived its worker");
    }

    /// `count` copies of the 23.976 clip under distinct names: distinct
    /// inodes, so each is its own file to the identity cache.
    fn copies(dir: &Path, count: usize) -> (Vec<PathBuf>, Expected, Vec<Vec<u8>>) {
        let path = ntsc_clip(dir);
        let source = ClipSource::open(&path).unwrap();
        let probe = probe(&source).unwrap();
        let frames = all_frames(&source, &probe);
        let paths = (0..count)
            .map(|index| {
                let copy = dir.join(format!("copy{index}.mp4"));
                std::fs::copy(&path, &copy).unwrap();
                copy
            })
            .collect();
        (paths, expected(&source, &probe), frames)
    }

    /// F1b defect 2: one budget across every clip. Three clips asked in turn
    /// on a budget of two decoders: the third recalls the least recently
    /// asked clip's lease, so two decoders run, never three, and the recalled
    /// clip still serves its last frame and decodes again when asked
    /// something new.
    #[test]
    fn a_budget_of_two_decoders_runs_two_whatever_the_clips() {
        let scratch = Scratch::new("budget");
        let (paths, expected, frames) = copies(&scratch.0, 3);
        let budget = ClipBudget::new(2, usize::MAX, Duration::from_secs(60));
        let workers: Vec<ClipWorker> = paths
            .iter()
            .map(|path| ClipWorker::spawn_in(path.clone(), expected.clone(), Arc::clone(&budget)))
            .collect();
        let deadline = Duration::from_secs(20);
        for (index, worker) in workers.iter().enumerate() {
            let frame = worker.wait_for(10 + index, deadline).unwrap();
            assert!(*frame == frames[10 + index]);
            assert!(budget.usage().decoders <= 2, "{:?}", budget.usage());
        }
        assert_eq!(budget.usage().evictions, 1, "{:?}", budget.usage());
        // The first, recalled: its frame is still there without a decoder.
        assert!(*workers[0].wait_for(10, deadline).unwrap() == frames[10]);
        // Asked something new it takes a lease again, from the second.
        assert!(*workers[0].wait_for(20, deadline).unwrap() == frames[20]);
        let usage = budget.usage();
        assert_eq!((usage.decoders, usage.evictions, usage.overruns), (2, 2, 0));
        drop(workers);
        assert_eq!(budget.usage().decoders, 0, "every lease went back");
    }

    /// F1b defect 2: the budget's bytes bound the kept frames the same way,
    /// and a worker nobody asks gives its lease back on its own.
    #[test]
    fn the_budget_bounds_kept_frames_and_idle_workers_give_back() {
        let scratch = Scratch::new("budget-bytes");
        let (paths, expected, frames) = copies(&scratch.0, 2);
        let frame = 96 * 54 * 4;
        // Room for one lease of the smallest cache (two frames) and no more.
        let budget = ClipBudget::new(4, 3 * frame, Duration::from_millis(200));
        let first = ClipWorker::spawn_in(paths[0].clone(), expected.clone(), Arc::clone(&budget));
        let second = ClipWorker::spawn_in(paths[1].clone(), expected.clone(), Arc::clone(&budget));
        let deadline = Duration::from_secs(20);
        assert!(*first.wait_for(5, deadline).unwrap() == frames[5]);
        assert_eq!(budget.usage().bytes, 2 * frame);
        assert!(*second.wait_for(6, deadline).unwrap() == frames[6]);
        let usage = budget.usage();
        assert_eq!(
            (usage.decoders, usage.bytes, usage.evictions),
            (1, 2 * frame, 1)
        );
        // Idle: nobody asks the second anything new.
        let waited = Instant::now();
        while budget.usage().decoders > 0 {
            assert!(
                waited.elapsed() < Duration::from_secs(5),
                "{:?}",
                budget.usage()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let restarts = second.stats().restarts;
        assert!(
            *second.wait_for(6, deadline).unwrap() == frames[6],
            "the paused frame stays"
        );
        assert_eq!(
            second.stats().restarts,
            restarts,
            "a paused frame needs no decoder"
        );
    }

    /// F1b defect 2: a clip verified once in this process is not hashed and
    /// probed again by the next worker for the same unchanged file, and is
    /// the moment the file is touched.
    #[test]
    fn a_verified_file_is_not_hashed_again_until_it_changes() {
        let scratch = Scratch::new("verified");
        let (paths, expected, _) = copies(&scratch.0, 1);
        let budget = ClipBudget::new(2, usize::MAX, Duration::from_secs(60));
        let deadline = Duration::from_secs(20);
        let first = ClipWorker::spawn_in(paths[0].clone(), expected.clone(), Arc::clone(&budget));
        first.wait_for(0, deadline).unwrap();
        assert!(!first.stats().reused_identity);
        drop(first);
        let again = ClipWorker::spawn_in(paths[0].clone(), expected.clone(), Arc::clone(&budget));
        again.wait_for(0, deadline).unwrap();
        assert!(
            again.stats().reused_identity,
            "the same file, verified before"
        );
        drop(again);
        // Any write moves the change time: verified again, from the bytes.
        let file = std::fs::OpenOptions::new()
            .append(true)
            .open(&paths[0])
            .unwrap();
        file.set_len(std::fs::metadata(&paths[0]).unwrap().len())
            .unwrap();
        drop(file);
        std::thread::sleep(Duration::from_millis(20));
        let bytes = std::fs::read(&paths[0]).unwrap();
        std::fs::write(&paths[0], &bytes).unwrap();
        let touched = ClipWorker::spawn_in(paths[0].clone(), expected, budget);
        touched.wait_for(0, deadline).unwrap();
        assert!(
            !touched.stats().reused_identity,
            "a written file is hashed again"
        );
    }
}
