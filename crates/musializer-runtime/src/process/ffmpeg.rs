//! The FFmpeg child process and raw-frame pipe.
//!
//! **Owner: Agent E.** Ported from `src/ffmpeg.h` and `src/ffmpeg_posix.c` in
//! the frozen C oracle at `../musializer` (commit `9300af9`, read-only).
//! `src/ffmpeg_windows.c` is a stated non-goal and is not ported.
//!
//! FFmpeg stays an external executable. The application writes raw `rgba`
//! frames into its stdin and lets it mux, which means the whole transport is
//! one pipe and one exit status — and that the two things that can go wrong
//! are a hung encoder and a half-written destination. Both are handled here.
//!
//! The shape:
//!
//! - [`ffmpeg_available`] is a cheap preflight for the destination picker.
//!   Startup is validated independently, because between the check and the
//!   spawn the binary can vanish (`src/ffmpeg.h:12-13`).
//! - [`Encoder::start`] spawns the child writing to a **temporary sibling** of
//!   the destination, never to the destination itself.
//! - [`Encoder::send_frame_flipped`] writes one frame bottom-row-first, because
//!   an OpenGL framebuffer read is upside down relative to a video frame.
//! - [`Encoder::finish`] is the only way an export becomes visible. It always
//!   consumes the encoder, reaps the child within a bounded grace period, and
//!   publishes with `rename` **only** after a clean exit.
//!
//! [`Drop`] is a best-effort net against abandonment, not a substitute for
//! [`Encoder::finish`]: it cannot report anything, so it kills, reaps and warns
//! on stderr.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use super::process_group::{
    describe_exit, kill_group_and_reap, signal_group_or_process, ShutdownPolicy, Signal,
    WaitOutcome,
};
use super::publish::export_temporary_path;

/// The three encode qualities, `Render_Quality`
/// (`src/render_export.h:23-28`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportQuality {
    /// x264 `-preset fast -crf 20`, 256 kbit/s audio.
    Balanced,
    /// The default. x264 `-preset slow -crf 16`, 256 kbit/s audio.
    #[default]
    High,
    /// x264 `-preset slow -crf 12`, 320 kbit/s audio.
    Master,
}

impl ExportQuality {
    /// `src/ffmpeg_posix.c:151`. Everything except Balanced is `slow`.
    const fn preset(self) -> &'static str {
        match self {
            ExportQuality::Balanced => "fast",
            ExportQuality::High | ExportQuality::Master => "slow",
        }
    }

    /// `src/ffmpeg_posix.c:152-153`.
    const fn crf(self) -> &'static str {
        match self {
            ExportQuality::Balanced => "20",
            ExportQuality::High => "16",
            ExportQuality::Master => "12",
        }
    }

    /// `src/ffmpeg_posix.c:154`. Only Master gets 320k.
    const fn audio_bitrate(self) -> &'static str {
        match self {
            ExportQuality::Balanced | ExportQuality::High => "256k",
            ExportQuality::Master => "320k",
        }
    }
}

/// The subset of `Render_Export_Config` (`src/render_export.h:30-36`) the
/// encoder needs.
///
/// **Boundary note.** `render_export.c` is Agent A's, and the authoritative
/// config type with its resolution/frame-rate/quality tables belongs in
/// `musializer-core::timing::render_export`. This struct exists so the encoder
/// could be written and tested before that landed; when it lands, this becomes
/// a `From` conversion and [`ExportConfig::validate`] is deleted in favour of
/// `render_export_config_validate`.
#[derive(Debug, Clone, Copy)]
pub struct ExportConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: ExportQuality,
    /// Offline supersampling factor. Only 1 and 2 are valid. The encoder never
    /// sees a supersampled frame — the render target is downsampled to
    /// `width`×`height` first — but the factor is validated here because the C
    /// validates the whole config before spawning.
    pub supersample_factor: u32,
}

impl Default for ExportConfig {
    /// `render_export_config_init` (`src/render_export.c:16-25`).
    fn default() -> Self {
        ExportConfig {
            width: 1920,
            height: 1080,
            fps: 30,
            quality: ExportQuality::High,
            supersample_factor: 2,
        }
    }
}

impl ExportConfig {
    /// `render_export_config_validate` (`src/render_export.c:67-85`).
    ///
    /// Odd dimensions are rejected because `yuv420p` chroma subsampling has no
    /// answer for them; the bounds are 16..=7680 by 16..=4320.
    pub fn validate(&self) -> Result<(), StartError> {
        if self.width < 16
            || self.height < 16
            || self.width > 7680
            || self.height > 4320
            || self.width % 2 != 0
            || self.height % 2 != 0
        {
            return Err(StartError::InvalidResolution);
        }
        if self.fps < 1 || self.fps > 240 {
            return Err(StartError::InvalidFrameRate);
        }
        if self.supersample_factor < 1 || self.supersample_factor > 2 {
            return Err(StartError::InvalidSupersample);
        }
        Ok(())
    }
}

/// Why an encoder could not be started.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("invalid rendering parameters")]
    InvalidParameters,
    #[error("invalid output resolution")]
    InvalidResolution,
    #[error("invalid frame rate")]
    InvalidFrameRate,
    #[error("invalid supersampling")]
    InvalidSupersample,
    #[error("could not format rendering parameters")]
    Unformattable,
    #[error("could not allocate transactional output paths")]
    TransactionalPath,
    #[error("could not start ffmpeg: {0}")]
    Spawn(#[source] std::io::Error),
}

/// Why a frame could not be handed to the encoder.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("invalid frame parameters")]
    InvalidParameters,
    #[error("the frame buffer is shorter than {expected} bytes")]
    ShortBuffer { expected: usize },
    #[error("failed to write into the ffmpeg pipe: {0}")]
    Pipe(#[source] std::io::Error),
    #[error("the encoder has already been finalized")]
    Finished,
}

/// How an export ended, when it ended well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finished {
    /// The encode completed and the destination now holds it.
    Published,
    /// The encode was cancelled, the child is reaped, and the temporary is
    /// gone. **The pre-existing destination, if any, is untouched.**
    Cancelled,
}

/// Why an export did not end well.
///
/// Every variant that can leave bytes on disk says where they are, because the
/// C's messages do (`src/ffmpeg_posix.c:266-268`, `:303-306`) and a user told
/// "the encode survived at …" can recover the render by hand.
#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    /// `ffmpeg` exited non-zero or died from a signal. The temporary has been
    /// removed.
    #[error("ffmpeg {description}")]
    Encoder { description: String },
    /// The grace period expired, so the child was force-killed. The temporary
    /// has been removed.
    #[error(
        "the encoder did not {phase} within the {grace_ms} ms grace period; it was terminated"
    )]
    Timeout { phase: &'static str, grace_ms: u128 },
    /// The worst case: the child could not be reaped even after `SIGKILL`.
    /// Nothing was deleted, because a process that may still be writing must
    /// not have its output pulled out from under it.
    #[error("the encoder could not be reaped; prior output is preserved and the in-progress file remains at {temporary}")]
    Unreaped { temporary: PathBuf },
    /// The encode succeeded but could not be moved into place. The bytes are
    /// still at `temporary` and the destination is untouched.
    #[error("could not publish the completed output to {destination}: {source} (the encoded file remains at {temporary})")]
    Publish {
        destination: PathBuf,
        temporary: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A frame write had already failed, so the file cannot be complete. The
    /// temporary has been removed.
    #[error("the frame transport had already failed, so the output was discarded")]
    TransportBroken,
    /// Everything else worked but the temporary could not be deleted.
    #[error("could not remove the temporary output {temporary}: {source}")]
    Cleanup {
        temporary: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not wait for the encoder: {0}")]
    Wait(#[source] std::io::Error),
}

/// Whether an `ffmpeg` executable is reachable through `PATH`.
///
/// `ffmpeg_available` (`src/ffmpeg_posix.c:30-55`), reproduced including its two
/// quirks: an unset or empty `PATH` is `false`, and an **empty element** is
/// treated as `"."` — so `PATH=":/usr/bin"` will find `./ffmpeg`.
///
/// One deliberate divergence: the C uses `access(path, X_OK)`, which consults
/// the real uid and gid. Without `libc` this checks "is a file with some
/// execute bit set", which accepts a file the current user could not actually
/// execute. The failure mode is a preflight that says yes and a spawn that then
/// fails — which the C is already designed to tolerate, since it re-validates
/// startup independently (`src/ffmpeg.h:12-13`).
pub fn ffmpeg_available() -> bool {
    executable_on_path("ffmpeg")
}

fn executable_on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let path = path.to_string_lossy();
    if path.is_empty() {
        return false;
    }
    path.split(':').any(|element| {
        let directory = if element.is_empty() { "." } else { element };
        let candidate = Path::new(directory).join(program);
        std::fs::metadata(&candidate)
            .map(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false)
    })
}

/// Formats the exact video transport duration for FFmpeg's `-t` cap.
///
/// `render_export_transport_duration_text` (`src/render_export.c:183-207`).
/// Nine fractional digits are exact for every supported integer FPS, and the
/// half-up rounding plus the carry at exactly one second are reproduced.
///
/// **Boundary note:** this is Agent A's function. It is duplicated here only
/// until `musializer-core::timing::render_export` lands; delete it then.
fn transport_duration_text(total_frames: u64, fps: u32) -> Option<String> {
    if total_frames == 0 || fps == 0 || fps > 240 {
        return None;
    }
    let fps = u64::from(fps);
    let mut whole_seconds = total_frames / fps;
    let remainder = total_frames % fps;
    let mut nanoseconds = (remainder * 1_000_000_000 + fps / 2) / fps;
    if nanoseconds == 1_000_000_000 {
        whole_seconds = whole_seconds.checked_add(1)?;
        nanoseconds = 0;
    }
    let mut text = String::new();
    write!(text, "{whole_seconds}.{nanoseconds:09}").ok()?;
    Some(text)
}

/// A running `ffmpeg` child and its frame pipe.
///
/// Owning this value means owning a child process. It is never `Copy` or
/// `Clone`, [`finish`](Encoder::finish) consumes it, and `Drop` cleans up after
/// a caller who forgot. That is the ownership policy the plan asks for; safe
/// Rust does not supply one.
#[derive(Debug)]
pub struct Encoder {
    child: Option<Child>,
    pipe: Option<ChildStdin>,
    destination: PathBuf,
    temporary: PathBuf,
    /// False once a frame write has failed. The C tracks the same idea as
    /// `transport_ok` (`src/ffmpeg_posix.c:231`) and refuses to publish when it
    /// is false, so a truncated encode cannot replace a good file.
    transport_ok: bool,
}

impl Encoder {
    /// Starts `ffmpeg` from `PATH`, writing to a temporary sibling of
    /// `destination`.
    ///
    /// `ffmpeg_start_rendering` (`src/ffmpeg_posix.c:98-221`). `total_frames`
    /// and `job_nonce` must both be non-zero; the nonce is what keeps two
    /// concurrent exports of the same destination from sharing a temporary.
    pub fn start(
        destination: &Path,
        config: &ExportConfig,
        sound_file: &Path,
        total_frames: u64,
        job_nonce: u64,
    ) -> Result<Self, StartError> {
        Self::start_with_program(
            OsStr::new("ffmpeg"),
            destination,
            config,
            sound_file,
            total_frames,
            job_nonce,
        )
    }

    /// [`start`](Encoder::start) with an explicit encoder program.
    ///
    /// The seam exists so the process lifecycle can be tested without depending
    /// on a real `ffmpeg` in `cargo test`. The oracle hardcodes `execlp("ffmpeg",
    /// …)` and has no override, so **this must not grow an environment
    /// variable** — that would be inventing behaviour the frozen binary does
    /// not have.
    pub fn start_with_program(
        program: &OsStr,
        destination: &Path,
        config: &ExportConfig,
        sound_file: &Path,
        total_frames: u64,
        job_nonce: u64,
    ) -> Result<Self, StartError> {
        if destination.as_os_str().is_empty()
            || sound_file.as_os_str().is_empty()
            || total_frames == 0
            || job_nonce == 0
        {
            return Err(StartError::InvalidParameters);
        }
        config.validate()?;

        let resolution = format!("{}x{}", config.width, config.height);
        let framerate = config.fps.to_string();
        let duration =
            transport_duration_text(total_frames, config.fps).ok_or(StartError::Unformattable)?;

        let destination_text = destination.to_str().ok_or(StartError::TransactionalPath)?;
        let temporary =
            export_temporary_path(destination_text, u64::from(std::process::id()), job_nonce)
                .ok_or(StartError::TransactionalPath)?;

        let mut command = Command::new(program);
        command
            .arg("-loglevel")
            .arg("warning")
            .arg("-y")
            // The raw video input: our pipe.
            .arg("-f")
            .arg("rawvideo")
            .arg("-pix_fmt")
            .arg("rgba")
            .arg("-s")
            .arg(&resolution)
            .arg("-framerate")
            .arg(&framerate)
            .arg("-i")
            .arg("-")
            // The audio input: the decoded track on disk.
            .arg("-i")
            .arg(sound_file)
            .arg("-map")
            .arg("0:v:0")
            .arg("-map")
            .arg("1:a:0")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg(config.quality.preset())
            .arg("-crf")
            .arg(config.quality.crf())
            .arg("-profile:v")
            .arg("high")
            .arg("-x264-params")
            .arg("colorprim=bt709:transfer=bt709:colormatrix=bt709")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg(config.quality.audio_bitrate())
            // Keep every deterministic video frame. The decoded PCM may end
            // within the last frame, so pad only that sub-frame tail and let
            // the video stream define the container duration.
            .arg("-af")
            .arg("apad")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-color_primaries")
            .arg("bt709")
            .arg("-color_trc")
            .arg("bt709")
            .arg("-colorspace")
            .arg("bt709")
            .arg("-movflags")
            .arg("+faststart")
            .arg("-t")
            .arg(&duration)
            .arg(&temporary)
            .stdin(Stdio::piped())
            // stdout and stderr stay inherited, as they are in the C: ffmpeg's
            // warnings belong in the application's log.
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().map_err(StartError::Spawn)?;
        let pipe = child.stdin.take();
        Ok(Encoder {
            child: Some(child),
            pipe,
            destination: destination.to_path_buf(),
            temporary,
            transport_ok: true,
        })
    }

    /// Where the in-progress encode is being written.
    pub fn temporary_path(&self) -> &Path {
        &self.temporary
    }

    /// Where a successful encode will be published.
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Writes one `rgba` frame, bottom row first.
    ///
    /// `ffmpeg_send_frame_flipped` (`src/ffmpeg_posix.c:338-370`). The flip is
    /// not cosmetic: `LoadImageFromTexture`/`rlReadScreenPixels` hand back an
    /// OpenGL framebuffer whose first row is the bottom of the image, and
    /// `rawvideo` expects the top row first.
    ///
    /// `height == 0` writes nothing and succeeds, as in the C.
    pub fn send_frame_flipped(
        &mut self,
        data: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), FrameError> {
        if width == 0 || width > usize::MAX / 4 || (height != 0 && height > usize::MAX / width) {
            return Err(FrameError::InvalidParameters);
        }
        let row_size = width * 4;
        let expected = row_size * height;
        if data.len() < expected {
            return Err(FrameError::ShortBuffer { expected });
        }
        let pipe = self.pipe.as_mut().ok_or(FrameError::Finished)?;
        for y in (0..height).rev() {
            let row = &data[y * row_size..(y + 1) * row_size];
            // `write_all` retries on `Interrupted` and reports a zero-length
            // write as `WriteZero`, which is the C's EINTR and
            // "made no progress" handling.
            if let Err(error) = pipe.write_all(row) {
                self.transport_ok = false;
                return Err(FrameError::Pipe(error));
            }
        }
        Ok(())
    }

    /// Finalizes the export. Always consumes the encoder and always reaps the
    /// child.
    ///
    /// `ffmpeg_end_rendering` (`src/ffmpeg_posix.c:223-336`), whose truth table
    /// is reproduced arm for arm:
    ///
    /// | state | result |
    /// | --- | --- |
    /// | `cancel`, child killed | temporary removed, [`Finished::Cancelled`] |
    /// | `cancel`, child had already exited 0 | temporary removed, `Cancelled` |
    /// | normal, exit 0, transport intact | `rename` to the destination |
    /// | normal, non-zero exit or signal | temporary removed, error |
    /// | grace expired | `SIGKILL`, reap, temporary removed, error |
    /// | still unreaped after `SIGKILL` | **nothing deleted**, error naming the file |
    ///
    /// Cancellation is not a failure: a user who cancels gets `Ok(Cancelled)`
    /// and an untouched destination.
    pub fn finish(mut self, cancel: bool) -> Result<Finished, FinishError> {
        // Closing the write end lets a non-cancelled ffmpeg see EOF and
        // finalize. The C also treats a failing close(2) as a broken transport;
        // safe Rust has no fallible close for a `ChildStdin`, so that arm is
        // folded into `transport_ok`, which a failed frame write already
        // clears. The net effect is the same: a broken transport never
        // publishes.
        self.pipe = None;

        let mut child = self.child.take().expect("finish consumes the encoder once");
        let mut cancel_sent = false;
        if cancel {
            // SIGKILL rather than SIGTERM: the C does not want a partially
            // muxed file finalized on the way out (src/ffmpeg_posix.c:242).
            match signal_group_or_process(child.id(), Signal::Kill) {
                Ok(_) => cancel_sent = true,
                Err(error) => {
                    self.transport_ok = false;
                    // Fall through and still try to reap: an unsignalled child
                    // is worse than a badly signalled one.
                    let _ = error;
                }
            }
        }

        let policy = if cancel {
            ShutdownPolicy::cancel()
        } else {
            ShutdownPolicy::finalize()
        };
        let waited = super::process_group::wait_bounded(&mut child, &policy);

        let status = match waited {
            Ok(WaitOutcome::Reaped(status)) => status,
            Ok(WaitOutcome::TimedOut) | Err(_) => {
                let phase = if cancel { "cancel" } else { "finalize" };
                let grace_ms = policy.grace.as_millis();
                match kill_group_and_reap(&mut child) {
                    Ok(WaitOutcome::Reaped(_)) => {
                        self.remove_temporary_or_report()?;
                        return Err(FinishError::Timeout { phase, grace_ms });
                    }
                    // Unreapable. Delete nothing: the child may still be
                    // writing, and the prior destination is already safe.
                    _ => {
                        return Err(FinishError::Unreaped {
                            temporary: std::mem::take(&mut self.temporary),
                        })
                    }
                }
            }
        };

        if cancel {
            // A cancelled export is a success as long as the child is gone and
            // the temporary is cleaned. The C additionally accepts a child that
            // exited 0 on its own before the kill landed (`cancel_sent ||
            // exit_status == 0`), which is the same thing said twice given that
            // kill(2) succeeds against a zombie.
            let _ = cancel_sent;
            self.remove_temporary_or_report()?;
            return Ok(Finished::Cancelled);
        }

        if !status.success() {
            self.remove_temporary_or_report()?;
            return Err(FinishError::Encoder {
                description: describe_exit(status),
            });
        }
        if !self.transport_ok {
            self.remove_temporary_or_report()?;
            return Err(FinishError::TransportBroken);
        }

        let temporary = std::mem::take(&mut self.temporary);
        let destination = std::mem::take(&mut self.destination);
        match std::fs::rename(&temporary, &destination) {
            Ok(()) => Ok(Finished::Published),
            Err(source) => Err(FinishError::Publish {
                destination,
                temporary,
                source,
            }),
        }
    }

    /// Removes the in-progress file, reporting a failure rather than swallowing
    /// it. `remove_temporary_output` (`src/ffmpeg_posix.c:63-69`): a missing
    /// file is a success.
    fn remove_temporary_or_report(&mut self) -> Result<(), FinishError> {
        let temporary = std::mem::take(&mut self.temporary);
        if temporary.as_os_str().is_empty() {
            return Ok(());
        }
        match std::fs::remove_file(&temporary) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(FinishError::Cleanup { temporary, source }),
        }
    }
}

impl Drop for Encoder {
    /// Best effort against abandonment: kill, reap, delete the temporary, and
    /// say so on stderr.
    ///
    /// This runs only when a caller dropped an encoder without calling
    /// [`Encoder::finish`], which is a bug. It is loud on purpose — a silent
    /// `Drop` here would hide an abandoned child process, which is exactly the
    /// invariant this workstream exists to protect.
    fn drop(&mut self) {
        self.pipe = None;
        if let Some(mut child) = self.child.take() {
            eprintln!(
                "FFMPEG: encoder dropped without finish(); killing and reaping pid {}",
                child.id()
            );
            match kill_group_and_reap(&mut child) {
                Ok(WaitOutcome::Reaped(_)) => {}
                Ok(WaitOutcome::TimedOut) => eprintln!(
                    "FFMPEG: abandoned encoder could not be reaped; {} may still be written",
                    self.temporary.display()
                ),
                Err(error) => eprintln!("FFMPEG: could not reap abandoned encoder: {error}"),
            }
        }
        if !self.temporary.as_os_str().is_empty() {
            if let Err(error) = std::fs::remove_file(&self.temporary) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "FFMPEG: could not remove temporary output {}: {error}",
                        self.temporary.display()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("musializer-ffmpeg-{}-{label}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// A stand-in encoder. It receives exactly the argv the real `ffmpeg`
        /// would, and the output path is always the last argument, so a POSIX
        /// shell can find it without knowing the flag layout.
        fn fake_encoder(&self, label: &str, body: &str) -> PathBuf {
            let path = self.join(label);
            let script =
                format!("#!/bin/sh\nout=\"\"\nfor a in \"$@\"; do out=\"$a\"; done\n{body}\n");
            fs::write(&path, script).expect("script");
            fs::set_permissions(&path, {
                use std::os::unix::fs::PermissionsExt;
                fs::Permissions::from_mode(0o755)
            })
            .expect("chmod");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn frame(width: usize, height: usize) -> Vec<u8> {
        // Row y is filled with the byte y, so a flipped write is detectable.
        let mut data = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            data.extend(std::iter::repeat(y as u8).take(width * 4));
        }
        data
    }

    fn small() -> ExportConfig {
        ExportConfig {
            width: 16,
            height: 16,
            fps: 30,
            quality: ExportQuality::Balanced,
            supersample_factor: 1,
        }
    }

    /// One real encode through the real `ffmpeg`, gated out of `cargo test`.
    ///
    /// `cargo test -p musializer-runtime -- --ignored real_ffmpeg` runs it. It
    /// stays ignored because the suite must not depend on external tools, but
    /// the argv above is exactly the oracle's and only a real encoder proves
    /// `ffmpeg` accepts it — a fake encoder proves the process lifecycle and
    /// nothing about the flags.
    #[test]
    #[ignore = "requires a real ffmpeg and ffprobe"]
    fn real_ffmpeg_produces_a_playable_mp4() {
        let scratch = Scratch::new("real");
        let audio = scratch.join("tone.wav");
        let synthesized = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2:sample_rate=44100",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&audio)
            .status()
            .expect("ffmpeg must be installed for an --ignored run");
        assert!(synthesized.success());

        let config = ExportConfig {
            width: 320,
            height: 240,
            fps: 30,
            quality: ExportQuality::Balanced,
            supersample_factor: 1,
        };
        let total_frames = 60; // exactly two seconds at 30 fps
        let destination = scratch.join("render.mp4");
        let mut encoder =
            Encoder::start(&destination, &config, &audio, total_frames, 1).expect("start");
        let pixels = frame(320, 240);
        for _ in 0..total_frames {
            encoder
                .send_frame_flipped(&pixels, 320, 240)
                .expect("frame");
        }
        assert_eq!(encoder.finish(false).expect("finish"), Finished::Published);

        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_frames",
                "-show_entries",
                "stream=nb_read_frames,width,height,codec_name",
                "-of",
                "csv=p=0",
            ])
            .arg(&destination)
            .output()
            .expect("ffprobe");
        let summary = String::from_utf8_lossy(&probe.stdout).trim().to_string();
        assert_eq!(
            summary, "h264,320,240,60",
            "ffprobe should see exactly the frames that were written"
        );
    }

    #[test]
    fn quality_maps_to_the_oracles_x264_settings() {
        assert_eq!(ExportQuality::Balanced.preset(), "fast");
        assert_eq!(ExportQuality::High.preset(), "slow");
        assert_eq!(ExportQuality::Master.preset(), "slow");
        assert_eq!(ExportQuality::Balanced.crf(), "20");
        assert_eq!(ExportQuality::High.crf(), "16");
        assert_eq!(ExportQuality::Master.crf(), "12");
        assert_eq!(ExportQuality::Balanced.audio_bitrate(), "256k");
        assert_eq!(ExportQuality::High.audio_bitrate(), "256k");
        assert_eq!(ExportQuality::Master.audio_bitrate(), "320k");
    }

    #[test]
    fn the_transport_duration_is_exact_to_nine_places() {
        // 30 fps: 90 frames is exactly three seconds.
        assert_eq!(transport_duration_text(90, 30).unwrap(), "3.000000000");
        // A non-dividing frame count rounds half-up in nanoseconds.
        assert_eq!(transport_duration_text(91, 30).unwrap(), "3.033333333");
        assert_eq!(transport_duration_text(1, 24).unwrap(), "0.041666667");
        // 7 frames at 60 fps is 0.11666..., which rounds up at the ninth digit.
        assert_eq!(transport_duration_text(7, 60).unwrap(), "0.116666667");
        assert!(transport_duration_text(0, 30).is_none());
        assert!(transport_duration_text(30, 0).is_none());
        assert!(transport_duration_text(30, 241).is_none());
    }

    #[test]
    fn invalid_configurations_are_refused_before_anything_is_spawned() {
        let odd = ExportConfig {
            width: 1921,
            ..ExportConfig::default()
        };
        assert!(matches!(odd.validate(), Err(StartError::InvalidResolution)));
        let tiny = ExportConfig {
            width: 8,
            height: 8,
            ..ExportConfig::default()
        };
        assert!(matches!(
            tiny.validate(),
            Err(StartError::InvalidResolution)
        ));
        let fast = ExportConfig {
            fps: 241,
            ..ExportConfig::default()
        };
        assert!(matches!(fast.validate(), Err(StartError::InvalidFrameRate)));
        let oversampled = ExportConfig {
            supersample_factor: 3,
            ..ExportConfig::default()
        };
        assert!(matches!(
            oversampled.validate(),
            Err(StartError::InvalidSupersample)
        ));
        assert!(ExportConfig::default().validate().is_ok());
    }

    #[test]
    fn a_zero_frame_count_or_nonce_is_refused() {
        let scratch = Scratch::new("params");
        let program = scratch.fake_encoder("enc", "cat > \"$out\"");
        let destination = scratch.join("out.mp4");
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();
        for (frames, nonce) in [(0u64, 1u64), (1, 0)] {
            let error = Encoder::start_with_program(
                program.as_os_str(),
                &destination,
                &small(),
                &audio,
                frames,
                nonce,
            )
            .expect_err("the C refuses both");
            assert!(matches!(error, StartError::InvalidParameters), "{error:?}");
        }
    }

    #[test]
    fn a_completed_encode_is_published_by_rename() {
        let scratch = Scratch::new("publish");
        let program = scratch.fake_encoder("enc", "cat > \"$out\"");
        let destination = scratch.join("out.mp4");
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();

        let mut encoder =
            Encoder::start_with_program(program.as_os_str(), &destination, &small(), &audio, 30, 7)
                .expect("start");
        let temporary = encoder.temporary_path().to_path_buf();
        assert!(
            temporary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".part.mp4"),
            "the in-progress name must let ffmpeg infer the muxer"
        );
        encoder
            .send_frame_flipped(&frame(16, 16), 16, 16)
            .expect("frame");
        assert_eq!(encoder.finish(false).expect("finish"), Finished::Published);

        assert!(!temporary.exists(), "the temporary must be gone");
        let written = fs::read(&destination).expect("destination");
        assert_eq!(written.len(), 16 * 16 * 4);
        // Row-flipped: the first row on the wire is the last row in memory.
        assert_eq!(written[0], 15);
        assert_eq!(written[written.len() - 1], 0);
    }

    #[test]
    fn a_cancelled_export_leaves_an_existing_destination_intact() {
        // The Phase 3 acceptance test, at the unit level.
        let scratch = Scratch::new("cancel");
        let program = scratch.fake_encoder("enc", "cat > \"$out\"; sleep 30");
        let destination = scratch.join("out.mp4");
        fs::write(&destination, b"a render from yesterday").unwrap();
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();

        let mut encoder =
            Encoder::start_with_program(program.as_os_str(), &destination, &small(), &audio, 30, 9)
                .expect("start");
        let temporary = encoder.temporary_path().to_path_buf();
        encoder
            .send_frame_flipped(&frame(16, 16), 16, 16)
            .expect("frame");

        assert_eq!(encoder.finish(true).expect("cancel"), Finished::Cancelled);
        assert!(
            !temporary.exists(),
            "a cancelled export cleans up after itself"
        );
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"a render from yesterday",
            "cancellation must not destroy the previous render"
        );
    }

    #[test]
    fn a_failing_encoder_is_reported_and_destroys_nothing() {
        let scratch = Scratch::new("fail");
        // Exits non-zero without writing anything, like an ffmpeg that rejects
        // its own arguments.
        let program = scratch.fake_encoder("enc", "exit 3");
        let destination = scratch.join("out.mp4");
        fs::write(&destination, b"previous render").unwrap();
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();

        let encoder = Encoder::start_with_program(
            program.as_os_str(),
            &destination,
            &small(),
            &audio,
            30,
            11,
        )
        .expect("start");
        let temporary = encoder.temporary_path().to_path_buf();
        let error = encoder
            .finish(false)
            .expect_err("a non-zero exit is a failure");
        match error {
            FinishError::Encoder { description } => {
                assert_eq!(description, "exited with code 3")
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!temporary.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"previous render");
    }

    #[test]
    fn an_encoder_that_ignores_the_pipe_closing_is_killed_after_the_grace_period() {
        let scratch = Scratch::new("hang");
        // Reads nothing and never exits: the "encoder wedged during
        // finalization" case the bounded wait exists for.
        let program = scratch.fake_encoder("enc", "trap '' TERM; sleep 30");
        let destination = scratch.join("out.mp4");
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();

        let mut encoder = Encoder::start_with_program(
            program.as_os_str(),
            &destination,
            &small(),
            &audio,
            30,
            13,
        )
        .expect("start");
        // The real grace period is five minutes, which no test should wait for.
        // Cancelling exercises the same ladder with the 5 s policy, and the
        // fake encoder ignores SIGTERM so only the SIGKILL escalation can
        // finish it — which is the behaviour under test.
        let temporary = encoder.temporary_path().to_path_buf();
        drop(encoder.send_frame_flipped(&frame(16, 16), 16, 16));
        assert_eq!(encoder.finish(true).expect("cancel"), Finished::Cancelled);
        assert!(!temporary.exists());
        assert!(!destination.exists());
    }

    #[test]
    fn a_frame_shorter_than_its_geometry_is_refused_before_the_pipe_sees_it() {
        let scratch = Scratch::new("short");
        let program = scratch.fake_encoder("enc", "cat > \"$out\"");
        let destination = scratch.join("out.mp4");
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();
        let mut encoder = Encoder::start_with_program(
            program.as_os_str(),
            &destination,
            &small(),
            &audio,
            30,
            17,
        )
        .expect("start");

        let error = encoder
            .send_frame_flipped(&frame(16, 15), 16, 16)
            .expect_err("a short buffer must not be read past its end");
        assert!(matches!(error, FrameError::ShortBuffer { .. }), "{error:?}");
        assert!(matches!(
            encoder.send_frame_flipped(&[], 0, 0),
            Err(FrameError::InvalidParameters)
        ));
        // height == 0 writes nothing and succeeds, as the C loop does.
        encoder.send_frame_flipped(&[], 16, 0).expect("empty frame");
        let _ = encoder.finish(true);
    }

    #[test]
    fn dropping_an_encoder_without_finishing_it_still_reaps_and_cleans_up() {
        let scratch = Scratch::new("drop");
        let program = scratch.fake_encoder("enc", "cat > \"$out\"; sleep 30");
        let destination = scratch.join("out.mp4");
        let audio = scratch.join("in.wav");
        fs::write(&audio, b"pcm").unwrap();

        let temporary;
        {
            let mut encoder = Encoder::start_with_program(
                program.as_os_str(),
                &destination,
                &small(),
                &audio,
                30,
                19,
            )
            .expect("start");
            temporary = encoder.temporary_path().to_path_buf();
            encoder
                .send_frame_flipped(&frame(16, 16), 16, 16)
                .expect("frame");
            // No finish() call. Drop is the net.
        }
        assert!(!temporary.exists(), "Drop must remove the in-progress file");
        assert!(!destination.exists(), "Drop must never publish");
    }

    #[test]
    fn ffmpeg_available_reads_path_the_way_the_oracle_does() {
        // An unset or empty PATH is false. Setting a process-wide env var races
        // with other tests, so the PATH-walking logic is exercised through the
        // same helper with a program name nothing can provide.
        assert!(!executable_on_path("musializer-no-such-program-9d3f"));
        // /bin/sh exists and is executable on any machine that can run these
        // tests at all, which makes this a real positive rather than a tautology.
        let path = std::env::var_os("PATH").unwrap_or_default();
        if !path.is_empty() {
            assert!(
                executable_on_path("sh"),
                "PATH is {path:?} and /bin/sh should be on it"
            );
        }
    }
}
