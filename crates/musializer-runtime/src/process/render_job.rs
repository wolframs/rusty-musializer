//! One offline video export, from decode to publication.
//!
//! **Owner: Agent H.** Distributed from `../musializer/src/plug.c`'s
//! `start_rendering_track_to` (`:6863-7118`), `finish_rendering_track`
//! (`:7252-7280`) and the transport half of `rendering_screen` (`:7873-8058`).
//! The encoder process itself is [`super::ffmpeg`]; the frame arithmetic is
//! [`musializer_core::timing::render_export`]. What is new here is the *job*:
//! the thing that owns both, plus the decoded audio, the staging WAV and the
//! frame cursor, and that can be stepped one frame at a time from a frame loop.
//!
//! # Why this is not in the application crate
//!
//! Everything below is decode, arithmetic and process supervision. Only the two
//! lines that put pixels on a GPU are the caller's, which is what keeps the
//! export loop in `main.rs` down to "draw the scene into the target, read it
//! back, hand it here". That split is also what lets this be tested against a
//! fake encoder with synthetic samples in `cargo test`, with no window, no
//! audio device and no FFmpeg.
//!
//! # The one thing that must not drift
//!
//! **A windowed export must be bit-identical to the same frames of a full
//! render.** That is not achieved by seeking: it is achieved by running every
//! frame from zero — sample cursor, analyzer, beat tracker, scene state, cue
//! switching — and only *skipping the draw and the encode* until the window
//! starts (`plug.c:7984`, `:8022-8026`). [`RenderJob::batch_size`] is what keeps
//! that fast-forward brief, and [`RenderJob::draws_this_frame`] is the only
//! thing that differs between a prepared frame and an encoded one.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use musializer_core::audio::{AudioAnalyzerConfig, ChannelMode};
use musializer_core::timing::render_export::{
    self, Quality, RenderExportConfig, RenderExportError,
};
use raylib::core::audio::{RaylibAudio, Wave};

use super::ffmpeg::{Encoder, ExportConfig, ExportQuality, FinishError, Finished, FrameError};
use crate::project_files::existing_files_alias;

/// How many frames one fast-forward tick prepares (`plug.c:7984`).
///
/// The oracle's number, and the reason it is a constant rather than "as many as
/// fit in the frame budget": the export must not depend on how fast the machine
/// running it is, and a batch that varied with wall-clock time would make the
/// *progress readout* non-deterministic even though the frames stayed exact.
const FAST_FORWARD_BATCH: u32 = 240;

/// Per-process staging nonce (`p->render_job_nonce`, `plug.c:7035-7036`).
///
/// A counter rather than a random number, because it only has to be unique
/// within this process: the pid is the other half of every staging name.
static JOB_NONCE: AtomicU64 = AtomicU64::new(0);

fn next_nonce() -> u64 {
    // Zero is not a valid nonce for either path helper, and the C skips it on
    // wrap for the same reason (`plug.c:7036`).
    let nonce = JOB_NONCE.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if nonce == 0 {
        1
    } else {
        nonce
    }
}

/// What to render, and where.
#[derive(Debug)]
pub struct RenderRequest<'a> {
    /// The `.mp4` the user chose. Never written directly — see
    /// [`super::ffmpeg::Encoder`].
    pub destination: &'a Path,
    /// The track's source audio, decoded here rather than streamed.
    pub source_audio: &'a Path,
    pub config: RenderExportConfig,
    /// `(start_seconds, duration_seconds)` from `--render-window`
    /// (`plug_configure_render_window`, `plug.c:7171-7184`).
    pub window: Option<(f64, f64)>,
    /// Paths the destination must not alias, beyond the source audio: the
    /// track's `.musi`, in the oracle (`plug.c:6903-6909`).
    pub protected: &'a [PathBuf],
}

/// Why an export could not start.
///
/// One variant per refusal in `start_rendering_track_to` (`plug.c:6863-7117`),
/// because each of those is a distinct notice with distinct wording and a user
/// told "the path aliases your source audio" can act where "export failed"
/// leaves them guessing.
#[derive(Debug, thiserror::Error)]
pub enum RenderStartError {
    #[error("export path must end in .mp4")]
    NotMp4,
    #[error("export settings are invalid: {0}")]
    Config(#[source] RenderExportError),
    #[error("export path aliases the source audio")]
    AliasesSource,
    #[error("export path aliases a file the project needs")]
    AliasesProject,
    #[error("the source audio decoder rejected the file")]
    Decode,
    #[error("decoded audio samples could not be allocated")]
    DecodeSamples,
    #[error("export timeline could not be created: {0}")]
    Timeline(#[source] RenderExportError),
    #[error("render window is invalid: {0}")]
    Window(#[source] RenderExportError),
    #[error("no free staging name was available beside {0}")]
    StagingName(PathBuf),
    #[error("decoded export audio could not be staged at {0}")]
    Staging(PathBuf),
    #[error("the FFmpeg encoder could not start: {0}")]
    Encoder(#[source] super::ffmpeg::StartError),
}

/// The exact frame and sample bounds one export covers.
///
/// Pure, and separated from the job so the arithmetic is testable without a
/// decoder: this is the composition of `render_export_total_frames`,
/// `render_export_window_frames` and `render_export_sample_cursor` that
/// `plug.c:6955-7003` performs inline.
// Not `Copy`: `Range` is not, deliberately, because it is an iterator. Cloning
// three integers is free and a plan is read once per frame at most.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderPlan {
    /// Frames in the *whole* track, which is what the timeline is measured on.
    pub total_frames: u64,
    /// Absolute frame indices this export encodes, half-open.
    pub frames: Range<u64>,
    /// Decoded audio frames the staged WAV covers, half-open.
    pub samples: Range<u64>,
}

impl RenderPlan {
    /// Resolves a window request against a decoded track.
    ///
    /// # Errors
    /// [`RenderStartError::Timeline`] when the decoded length and frame rate
    /// cannot form a transport, [`RenderStartError::Window`] when the requested
    /// window is not a positive finite range inside the track — including the
    /// case the C checks separately, an accepted frame window whose sample
    /// bounds collapse to nothing (`plug.c:6989-6990`).
    pub fn resolve(
        audio_frames: u64,
        sample_rate: u32,
        fps: u32,
        window: Option<(f64, f64)>,
    ) -> Result<Self, RenderStartError> {
        let total_frames = render_export::total_frames(audio_frames, sample_rate, fps)
            .map_err(RenderStartError::Timeline)?;
        let Some((start_seconds, duration_seconds)) = window else {
            return Ok(RenderPlan {
                total_frames,
                frames: 0..total_frames,
                samples: 0..audio_frames,
            });
        };
        let frames =
            render_export::window_frames(total_frames, fps, start_seconds, duration_seconds)
                .map_err(RenderStartError::Window)?;
        let start_sample =
            render_export::sample_cursor(frames.start, sample_rate, fps, audio_frames)
                .map_err(RenderStartError::Window)?;
        let end_sample = render_export::sample_cursor(frames.end, sample_rate, fps, audio_frames)
            .map_err(RenderStartError::Window)?;
        if end_sample <= start_sample {
            return Err(RenderStartError::Window(RenderExportError::Window));
        }
        Ok(RenderPlan {
            total_frames,
            frames,
            samples: start_sample..end_sample,
        })
    }

    /// Frames this export actually encodes.
    #[must_use]
    pub fn encoded_frames(&self) -> u64 {
        self.frames.end.saturating_sub(self.frames.start)
    }

    /// Whether the staged audio is a strict slice of the decoded track
    /// (`plug.c:7054-7055`).
    #[must_use]
    pub fn is_windowed(&self, audio_frames: u64) -> bool {
        self.samples.start > 0 || self.samples.end < audio_frames
    }
}

/// How an export ended.
///
/// Carries the destination and the staging file so the caller can raise the
/// oracle's notices without keeping its own copy of either
/// (`plug.c:7889-7916`, `:6856-6861`).
#[derive(Debug)]
pub struct RenderCompletion {
    pub destination: PathBuf,
    pub result: Result<Finished, FinishError>,
    /// Set when the decoded staging WAV survived, which is a warning rather
    /// than a failure: something may still be reading it
    /// (`warn_render_staging_file_retained`, `plug.c:6856-6861`).
    pub retained_staging: Option<PathBuf>,
}

/// The decoded track an export runs over.
///
/// A struct rather than four parameters because the four have to agree —
/// `samples.len() == audio_frames * channels` — and a signature that could take
/// them in the wrong order was worth deleting.
#[derive(Debug)]
struct DecodedTrack {
    samples: Vec<f32>,
    channels: u32,
    sample_rate: u32,
    audio_frames: u64,
}

/// A running export: the encoder, the decoded audio, and the frame cursor.
///
/// Stepping it is four calls per frame, in this order — the order is the
/// oracle's loop body (`plug.c:7985-8057`) and getting it wrong is a parity bug
/// rather than a crash:
///
/// 1. [`take_samples`](RenderJob::take_samples) → push into the analyzer,
/// 2. analyze with [`scene_delta`](RenderJob::scene_delta),
/// 3. update the scene at [`scene_time`](RenderJob::scene_time), and draw and
///    [`send_frame`](RenderJob::send_frame) **only** when
///    [`draws_this_frame`](RenderJob::draws_this_frame),
/// 4. [`advance`](RenderJob::advance).
#[derive(Debug)]
pub struct RenderJob {
    config: RenderExportConfig,
    destination: PathBuf,
    staged_audio: PathBuf,
    /// The whole decoded track, interleaved. Analysis always runs from the
    /// start of the track, even for a windowed export (`plug.c:7050-7052`).
    samples: Vec<f32>,
    channels: u32,
    sample_rate: u32,
    audio_frames: u64,
    plan: RenderPlan,
    frame_index: u64,
    sample_cursor: u64,
    encoder: Option<Encoder>,
    started: Instant,
    finishing: bool,
    cancel_requested: bool,
}

impl RenderJob {
    /// Decodes, stages and spawns (`start_rendering_track_to`,
    /// `plug.c:6863-7118`).
    ///
    /// Everything that can be refused is refused before anything is written,
    /// and the encoder is the last thing started, so a failure here leaves the
    /// destination, the source and the previous render untouched.
    ///
    /// # Errors
    /// See [`RenderStartError`]; every variant is one of the oracle's refusals.
    pub fn start(
        audio: &RaylibAudio,
        request: &RenderRequest<'_>,
    ) -> Result<Self, RenderStartError> {
        Self::start_with_program(std::ffi::OsStr::new("ffmpeg"), audio, request)
    }

    /// [`start`](RenderJob::start) with an explicit encoder program.
    ///
    /// The seam exists for the same reason [`Encoder::start_with_program`]'s
    /// does: the lifecycle has to be testable without a real FFmpeg. It must
    /// **not** grow an environment variable — the oracle hardcodes `ffmpeg` and
    /// an override would be invented behaviour.
    pub fn start_with_program(
        program: &std::ffi::OsStr,
        audio: &RaylibAudio,
        request: &RenderRequest<'_>,
    ) -> Result<Self, RenderStartError> {
        // `IsFileExtension` is case-insensitive (`plug.c:6892`), so `.MP4` is an
        // MP4 here too.
        let is_mp4 = request
            .destination
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
        if !is_mp4 {
            return Err(RenderStartError::NotMp4);
        }
        request
            .config
            .validate()
            .map_err(RenderStartError::Config)?;
        if existing_files_alias(request.source_audio, request.destination) {
            return Err(RenderStartError::AliasesSource);
        }
        if request
            .protected
            .iter()
            .any(|path| existing_files_alias(path, request.destination))
        {
            return Err(RenderStartError::AliasesProject);
        }

        let source = request
            .source_audio
            .to_str()
            .ok_or(RenderStartError::Decode)?;
        let mut wave = audio
            .new_wave(source)
            .map_err(|_| RenderStartError::Decode)?;
        if !wave.is_wave_valid() {
            return Err(RenderStartError::Decode);
        }
        let sample_rate = wave.sample_rate();
        let channels = wave.channels();
        let audio_frames = u64::from(wave.frame_count());
        let samples = decoded_samples(&wave).ok_or(RenderStartError::DecodeSamples)?;

        let plan = RenderPlan::resolve(
            audio_frames,
            sample_rate,
            request.config.fps,
            request.window,
        )?;

        let destination_text = request
            .destination
            .to_str()
            .ok_or(RenderStartError::NotMp4)?;
        let (nonce, staged_audio) = reserve_staging(destination_text)
            .ok_or_else(|| RenderStartError::StagingName(request.destination.to_path_buf()))?;

        // FFmpeg must hear exactly the audio the window frames span, so a
        // windowed export stages only that slice (`plug.c:7050-7060`). The C
        // aliases the decoded buffer instead of copying; `WaveCrop` reallocates,
        // which costs one copy of the slice and is the same bytes on disk.
        if plan.is_windowed(audio_frames) {
            let (start, end) = (plan.samples.start, plan.samples.end);
            let (Ok(start), Ok(end)) = (i32::try_from(start), i32::try_from(end)) else {
                return Err(RenderStartError::Window(RenderExportError::Overflow));
            };
            wave.crop(start, end);
        }
        if !wave.export(&staged_audio) {
            let _ = std::fs::remove_file(&staged_audio);
            return Err(RenderStartError::Staging(staged_audio));
        }
        drop(wave);

        Self::spawn(
            program,
            request.destination,
            request.config,
            plan,
            DecodedTrack {
                samples,
                channels,
                sample_rate,
                audio_frames,
            },
            staged_audio,
            nonce,
        )
    }

    /// The half of [`start_with_program`](RenderJob::start_with_program) that is
    /// left once the audio is decoded and staged.
    ///
    /// Split out so the stepping logic can be exercised against a fake encoder
    /// and synthetic samples, with no audio device and no FFmpeg.
    fn spawn(
        program: &std::ffi::OsStr,
        destination: &Path,
        config: RenderExportConfig,
        plan: RenderPlan,
        track: DecodedTrack,
        staged_audio: PathBuf,
        nonce: u64,
    ) -> Result<Self, RenderStartError> {
        let encoder = Encoder::start_with_program(
            program,
            destination,
            &encoder_config(&config),
            &staged_audio,
            plan.encoded_frames(),
            nonce,
        );
        let encoder = match encoder {
            Ok(encoder) => encoder,
            Err(error) => {
                // The staging WAV is ours and nothing else can be reading it
                // yet, so a failed spawn takes it with it.
                let _ = std::fs::remove_file(&staged_audio);
                return Err(RenderStartError::Encoder(error));
            }
        };

        Ok(RenderJob {
            config,
            destination: destination.to_path_buf(),
            staged_audio,
            samples: track.samples,
            channels: track.channels,
            sample_rate: track.sample_rate,
            audio_frames: track.audio_frames,
            plan,
            frame_index: 0,
            sample_cursor: 0,
            encoder: Some(encoder),
            started: Instant::now(),
            finishing: false,
            cancel_requested: false,
        })
    }

    /// The analyzer configuration an export runs under
    /// (`analyzer_configure(p->wave.sampleRate, p->wave.channels)`,
    /// `plug.c:7094`).
    ///
    /// Note that this is **not** [`AudioAnalyzerConfig::preview`]: preview hears
    /// raylib's stereo mix whatever the file contained, while an export reads
    /// the decoded wave's own channel count. For a mono file the two disagree,
    /// and taking channel 0 of a 2-channel view of mono data would read every
    /// other sample.
    #[must_use]
    pub fn analyzer_config(&self) -> AudioAnalyzerConfig {
        AudioAnalyzerConfig {
            sample_rate: self.sample_rate,
            channel_count: self.channels,
            channel_mode: ChannelMode::Select(0),
        }
    }

    #[must_use]
    pub fn config(&self) -> RenderExportConfig {
        self.config
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    pub fn plan(&self) -> &RenderPlan {
        &self.plan
    }

    /// Frames to prepare in this tick: 240 while fast-forwarding, otherwise one
    /// (`plug.c:7984`).
    #[must_use]
    pub fn batch_size(&self) -> u32 {
        if self.frame_index < self.plan.frames.start {
            FAST_FORWARD_BATCH
        } else {
            1
        }
    }

    /// The decoded samples between the previous frame's cursor and this one's
    /// (`plug.c:7987-8011`).
    ///
    /// Empty at frame zero, because the C only advances the cursor from frame 1
    /// onward — the first frame is drawn against an empty analyzer, and that is
    /// what makes frame 0 of an export identical to frame 0 of a preview that
    /// has just started.
    ///
    /// # Errors
    /// [`RenderExportError`] when the cursor cannot be computed or would move
    /// backwards, which is the transport failing rather than the encoder
    /// (`plug.c:7992`).
    pub fn take_samples(&mut self) -> Result<&[f32], RenderExportError> {
        if self.frame_index == 0 {
            return Ok(&[]);
        }
        let next = render_export::sample_cursor(
            self.frame_index,
            self.sample_rate,
            self.config.fps,
            self.audio_frames,
        )?;
        if next < self.sample_cursor {
            return Err(RenderExportError::Overflow);
        }
        let channels = self.channels as usize;
        let from = self.sample_cursor as usize * channels;
        let to = (next as usize * channels).min(self.samples.len());
        self.sample_cursor = next;
        Ok(self.samples.get(from..to).unwrap_or(&[]))
    }

    /// The scene delta for the current frame (`plug.c:8013-8014`).
    ///
    /// Zero at frame zero and exactly `1/fps` thereafter. A wall-clock delta
    /// here is the single easiest way to make an export non-reproducible.
    #[must_use]
    pub fn scene_delta(&self) -> f32 {
        if self.frame_index == 0 {
            0.0
        } else {
            1.0 / self.config.fps as f32
        }
    }

    /// The scene clock for the current frame (`plug.c:8019`).
    #[must_use]
    pub fn scene_time(&self) -> f64 {
        self.frame_index as f64 / f64::from(self.config.fps)
    }

    #[must_use]
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// Whether this frame is inside the window and must be drawn and encoded
    /// (`plug.c:8022`).
    #[must_use]
    pub fn draws_this_frame(&self) -> bool {
        self.frame_index >= self.plan.frames.start
    }

    #[must_use]
    pub fn is_fast_forwarding(&self) -> bool {
        self.frame_index < self.plan.frames.start
    }

    /// Whether every window frame has been encoded (`plug.c:7902`).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.frame_index >= self.plan.frames.end
    }

    /// Frames handed to the encoder so far (`plug.c:7923-7924`).
    #[must_use]
    pub fn encoded_frames(&self) -> u64 {
        if self.is_fast_forwarding() {
            0
        } else {
            self.frame_index - self.plan.frames.start
        }
    }

    /// `0.0..=1.0` (`plug.c:7925-7927`).
    #[must_use]
    pub fn progress(&self) -> f64 {
        let window = self.plan.encoded_frames();
        if window == 0 {
            return 0.0;
        }
        (self.encoded_frames() as f64 / window as f64).min(1.0)
    }

    #[must_use]
    pub fn elapsed_seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// The estimate the progress screen prints (`plug.c:7928`).
    ///
    /// Below 0.1% progress it reports zero rather than a number derived from
    /// almost no evidence.
    #[must_use]
    pub fn remaining_seconds(&self) -> f64 {
        let progress = self.progress();
        if progress > 0.001 {
            self.elapsed_seconds() * (1.0 - progress) / progress
        } else {
            0.0
        }
    }

    /// True once the last frame has been written and the encoder is being given
    /// one tick to finalize (`p->render_finishing`, `plug.c:7903-7904`).
    #[must_use]
    pub fn is_finishing(&self) -> bool {
        self.finishing
    }

    /// Marks the job as finishing. Returns `true` the second time it is called
    /// on a complete job, which is the tick that should call
    /// [`finish`](RenderJob::finish) — the oracle's one-frame delay, which
    /// exists so the progress screen can repaint saying "finalizing" before the
    /// process blocks on `waitpid`.
    pub fn begin_finishing(&mut self) -> bool {
        if self.finishing {
            return true;
        }
        self.finishing = true;
        false
    }

    /// Asks for cancellation; honoured on the next tick (`plug.c:7975`).
    pub fn request_cancel(&mut self) {
        self.cancel_requested = true;
    }

    #[must_use]
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    /// Hands one RGBA frame to the encoder (`plug.c:8040`).
    ///
    /// # Errors
    /// [`FrameError`] if the pipe closed or the buffer does not match the
    /// geometry. A caller that sees one must finish with `cancel = true`: a
    /// truncated stream must never replace a good destination.
    pub fn send_frame(
        &mut self,
        rgba: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), FrameError> {
        let encoder = self.encoder.as_mut().ok_or(FrameError::Finished)?;
        encoder.send_frame_flipped(rgba, width, height)
    }

    /// Moves to the next frame (`p->scene_frame_index++`, `plug.c:1173`).
    ///
    /// In the oracle this happens inside `make_scene_frame`, so both the
    /// fast-forward path and the drawn path advance exactly once per loop
    /// iteration. Here it is explicit, because a caller that forgets it would
    /// otherwise loop forever on frame zero.
    pub fn advance(&mut self) {
        self.frame_index += 1;
    }

    /// Finalizes: reaps the encoder, publishes or discards, and removes the
    /// staging WAV (`ffmpeg_end_rendering` + `finish_rendering_track`).
    ///
    /// Always consumes the job. Cancellation is not a failure — it reports
    /// [`Finished::Cancelled`] and leaves any previous render in place.
    #[must_use]
    pub fn finish(mut self, cancel: bool) -> RenderCompletion {
        let result = match self.encoder.take() {
            Some(encoder) => encoder.finish(cancel),
            None => Ok(Finished::Cancelled),
        };
        let retained_staging = self.remove_staging();
        RenderCompletion {
            destination: std::mem::take(&mut self.destination),
            result,
            retained_staging,
        }
    }

    /// `remove_render_staging_file` (`plug.c:6846-6854`). A missing file is a
    /// success; anything else is reported rather than swallowed, because a
    /// retained staging WAV is a file the user will otherwise find later with
    /// no idea what wrote it.
    fn remove_staging(&mut self) -> Option<PathBuf> {
        let path = std::mem::take(&mut self.staged_audio);
        if path.as_os_str().is_empty() {
            return None;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => Some(path),
        }
    }
}

impl Drop for RenderJob {
    /// The net against a job dropped without [`RenderJob::finish`].
    ///
    /// [`Encoder`]'s own `Drop` kills and reaps the child and removes the
    /// partial MP4; this only has to take the staging WAV with it, and say so
    /// if it cannot.
    fn drop(&mut self) {
        if let Some(retained) = self.remove_staging() {
            eprintln!(
                "RENDER: temporary decoded audio was retained at {}",
                retained.display()
            );
        }
    }
}

/// `Render_Export_Config` → the encoder's view of it.
///
/// The boundary note in [`super::ffmpeg`] asked for exactly this conversion once
/// `musializer_core::timing::render_export` landed. It lives here rather than
/// there so that module keeps no dependency on this one.
fn encoder_config(config: &RenderExportConfig) -> ExportConfig {
    ExportConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        quality: match config.quality {
            Quality::Balanced => ExportQuality::Balanced,
            Quality::High => ExportQuality::High,
            Quality::Master => ExportQuality::Master,
        },
        supersample_factor: config.supersample_factor,
    }
}

/// Finds a nonce whose staging names are both free (`plug.c:7034-7049`).
///
/// Both names, not just the WAV: a colliding `.part.mp4` would let two
/// concurrent exports of the same destination write the same in-progress file,
/// and the loser would publish the winner's half-written bytes.
fn reserve_staging(destination: &str) -> Option<(u64, PathBuf)> {
    let process_id = u64::from(std::process::id());
    for _ in 0..1024 {
        let nonce = next_nonce();
        let audio = render_export::decoded_audio_path(destination, process_id, nonce)?;
        let video = render_export::temporary_path(destination, process_id, nonce)?;
        if !Path::new(&audio).exists() && !Path::new(&video).exists() {
            return Some((nonce, PathBuf::from(audio)));
        }
    }
    None
}

/// Every decoded sample of a wave, interleaved.
///
/// # The raylib-rs bug this exists for
///
/// `Wave::load_samples` builds its slice as `(pointer, self.frameCount)`
/// (`raylib-5.5.1/src/core/audio.rs:261-268`), but `LoadWaveSamples` allocates
/// and fills `frameCount * channels` floats
/// (`vendor/raylib-5.5/src/raudio.c:1299-1310`). For any stereo track the safe
/// wrapper therefore hands back **exactly half the audio**, silently: an export
/// built on it would analyze the first half of the track spread across the whole
/// timeline. This is the same class of defect as `load_font_from_memory`'s
/// codepoint miscount, and it gets the same treatment — call the ffi directly
/// and take the length from the format, not from the wrapper.
fn decoded_samples(wave: &Wave<'_>) -> Option<Vec<f32>> {
    let count = (wave.frame_count() as usize).checked_mul(wave.channels() as usize)?;
    if count == 0 {
        return Some(Vec::new());
    }
    // SAFETY: `**wave` is a bitwise copy of the `ffi::Wave` this `Wave` owns and
    // is only read; the owner outlives the call, so its `data` pointer is live.
    // `LoadWaveSamples` returns a fresh allocation of exactly
    // `frameCount * channels` floats or null, which is checked before the slice
    // is formed, and the allocation is handed straight back to
    // `UnloadWaveSamples` after the copy — so nothing borrowed here escapes.
    unsafe {
        let pointer = raylib::ffi::LoadWaveSamples(**wave);
        if pointer.is_null() {
            return None;
        }
        let samples = std::slice::from_raw_parts(pointer, count).to_vec();
        raylib::ffi::UnloadWaveSamples(pointer);
        Some(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that takes its contents with it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "musializer-render-job-{}-{label}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tiny_config() -> RenderExportConfig {
        let mut config = RenderExportConfig {
            width: 16,
            height: 16,
            fps: 30,
            ..RenderExportConfig::default()
        };
        config.set_quality(Quality::Balanced);
        config
    }

    /// A job over synthetic stereo audio with **no encoder process**.
    ///
    /// Deliberately no `fork`: the encoder's own lifecycle is tested in
    /// [`super::super::ffmpeg`], behind the `EXEC_LOCK` that exists there
    /// because writing a script while another thread forks makes the writer's
    /// own `exec` fail with `ETXTBSY`. A second module forking on its own would
    /// reintroduce that race from outside that lock, and would be testing the
    /// encoder twice rather than testing this. What is new here is the plan and
    /// the cursor, and neither needs a child process. The real encoder is
    /// exercised end to end by `examples/export_probe.rs`.
    fn prepared_job(
        scratch: &Scratch,
        label: &str,
        audio_frames: u64,
        window: Option<(f64, f64)>,
    ) -> RenderJob {
        let destination = scratch.join(&format!("{label}.mp4"));
        let staged = scratch.join(&format!(".staged-{label}.wav"));
        std::fs::write(&staged, b"pcm").expect("staging");
        let config = tiny_config();
        let plan =
            RenderPlan::resolve(audio_frames, 44_100, config.fps, window).expect("a valid plan");
        // Sample n of channel 0 is n as a float, so a mis-sliced push is
        // detectable rather than merely wrong.
        let mut samples = Vec::with_capacity(audio_frames as usize * 2);
        for frame in 0..audio_frames {
            samples.push(frame as f32);
            samples.push(-(frame as f32));
        }
        RenderJob {
            config,
            destination,
            staged_audio: staged,
            samples,
            channels: 2,
            sample_rate: 44_100,
            audio_frames,
            plan,
            frame_index: 0,
            sample_cursor: 0,
            encoder: None,
            started: Instant::now(),
            finishing: false,
            cancel_requested: false,
        }
    }

    #[test]
    fn a_full_render_covers_the_whole_track() {
        // 44100 frames at 44.1 kHz is one second; 30 fps is 30 frames.
        let plan = RenderPlan::resolve(44_100, 44_100, 30, None).expect("a valid transport");
        assert_eq!(plan.total_frames, 30);
        assert_eq!(plan.frames, 0..30);
        assert_eq!(plan.samples, 0..44_100);
        assert_eq!(plan.encoded_frames(), 30);
        assert!(!plan.is_windowed(44_100));
    }

    #[test]
    fn a_window_keeps_absolute_frame_indices() {
        // The property the fast-forward depends on: the window's frames are
        // indices on the *full* timeline, so frame 60 of a windowed export is
        // frame 60 of a full one (`render_export.h:83-88`).
        let plan = RenderPlan::resolve(441_000, 44_100, 30, Some((2.0, 2.0))).expect("valid");
        assert_eq!(plan.total_frames, 300);
        assert_eq!(plan.frames, 60..120);
        assert_eq!(plan.samples, 88_200..176_400);
        assert!(plan.is_windowed(441_000));
        assert_eq!(plan.encoded_frames(), 60);
    }

    #[test]
    fn a_degenerate_transport_or_window_is_refused_before_anything_is_written() {
        assert!(matches!(
            RenderPlan::resolve(0, 44_100, 30, None),
            Err(RenderStartError::Timeline(_))
        ));
        assert!(matches!(
            RenderPlan::resolve(44_100, 44_100, 0, None),
            Err(RenderStartError::Timeline(_))
        ));
        // Starting at or past the end of the track.
        assert!(matches!(
            RenderPlan::resolve(44_100, 44_100, 30, Some((5.0, 1.0))),
            Err(RenderStartError::Window(_))
        ));
        // A non-positive duration.
        assert!(matches!(
            RenderPlan::resolve(44_100, 44_100, 30, Some((0.0, 0.0))),
            Err(RenderStartError::Window(_))
        ));
    }

    /// The C checks the sample bounds separately from the frame bounds
    /// (`plug.c:6989-6990`), and the check is *not* redundant: an accepted frame
    /// window can still span no audio at all when consecutive frames map to the
    /// same sample.
    ///
    /// That needs `fps > sample_rate`, so no real track reaches it — the guard
    /// is defensive, and this test says which arm it is defending rather than
    /// pretending the case is ordinary. It is reproduced because an export that
    /// staged a zero-length WAV would fail much later, inside FFmpeg.
    #[test]
    fn a_window_whose_samples_collapse_is_refused() {
        // 100 frames at 100 Hz is one second: 240 video frames, so frames 0 and
        // 1 both start at decoded sample 0.
        let plan = RenderPlan::resolve(100, 100, 240, Some((0.0, 0.001)));
        assert!(matches!(plan, Err(RenderStartError::Window(_))), "{plan:?}");
        // The same transport with a window long enough to cover a sample is
        // accepted, which is what makes the check a bound rather than a ban.
        assert!(RenderPlan::resolve(100, 100, 240, Some((0.0, 0.05))).is_ok());
    }

    #[test]
    fn staging_names_are_hidden_siblings_of_the_destination_and_never_repeat() {
        let (first_nonce, first) = reserve_staging("/tmp/never-written.mp4").expect("names");
        let (second_nonce, second) = reserve_staging("/tmp/never-written.mp4").expect("names");
        assert_ne!(first_nonce, second_nonce);
        assert_ne!(first, second);
        assert_eq!(first.parent().unwrap(), Path::new("/tmp"));
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".musializer-audio-"));
    }

    /// The cursor walk, which is what makes preview and export agree: every
    /// decoded sample is pushed exactly once, in order, and frame zero pushes
    /// nothing (`plug.c:7987-8011`).
    #[test]
    fn every_sample_is_pushed_exactly_once_and_frame_zero_pushes_none() {
        let scratch = Scratch::new("cursor");
        let mut job = prepared_job(&scratch, "cursor", 44_100, None);
        assert!(job.take_samples().expect("frame 0").is_empty());

        let mut seen = 0usize;
        let mut previous = -1.0f32;
        while !job.is_complete() {
            let samples = job.take_samples().expect("a monotonic cursor");
            for frame in samples.chunks_exact(2) {
                assert!(frame[0] > previous, "the cursor revisited a sample");
                previous = frame[0];
                seen += 1;
            }
            job.advance();
        }
        // The last frame's cursor clamps to the end of the audio, so the whole
        // track is covered but never overrun.
        assert_eq!(seen, 44_100 - 1_470, "frame 0 consumes nothing");
        assert_eq!(job.frame_index(), 30);
        let _ = job.finish(true);
    }

    /// The fast-forward, which is the whole reason a windowed export is
    /// bit-identical to the same frames of a full one.
    #[test]
    fn a_windowed_job_prepares_the_frames_before_its_window_without_encoding_them() {
        let scratch = Scratch::new("window");
        // Ten seconds at 44.1 kHz, window 2 s..4 s.
        let mut job = prepared_job(&scratch, "window", 441_000, Some((2.0, 2.0)));
        assert_eq!(job.plan().frames, 60..120);
        assert_eq!(job.batch_size(), FAST_FORWARD_BATCH);
        assert!(!job.draws_this_frame());
        assert_eq!(job.encoded_frames(), 0);
        assert_eq!(job.progress(), 0.0);

        let mut drawn = 0u64;
        while !job.is_complete() {
            // The samples are consumed on every frame, inside the window or not.
            let consumed = job.take_samples().expect("cursor").len();
            assert!(job.frame_index() == 0 || consumed > 0);
            if job.draws_this_frame() {
                drawn += 1;
                assert_eq!(job.batch_size(), 1, "only the skipped frames batch");
            }
            job.advance();
        }
        assert_eq!(drawn, 60, "exactly the window's frames are drawn");
        assert_eq!(job.encoded_frames(), 60);
        assert_eq!(job.progress(), 1.0);
        let _ = job.finish(true);
    }

    /// Frame 0 of a windowed export and frame 0 of a full one must see the same
    /// scene clock at the same absolute index, because the window is a range of
    /// absolute indices and not a re-based timeline.
    #[test]
    fn the_scene_clock_is_absolute_in_both_a_full_and_a_windowed_export() {
        let scratch = Scratch::new("clock");
        let mut full = prepared_job(&scratch, "clock-full", 441_000, None);
        let mut windowed = prepared_job(&scratch, "clock-window", 441_000, Some((2.0, 2.0)));
        let mut full_clocks = Vec::new();
        while full.frame_index() < 120 {
            if full.frame_index() >= 60 {
                full_clocks.push((full.scene_time(), full.scene_delta()));
            }
            full.advance();
        }
        let mut window_clocks = Vec::new();
        while !windowed.is_complete() {
            if windowed.draws_this_frame() {
                window_clocks.push((windowed.scene_time(), windowed.scene_delta()));
            }
            windowed.advance();
        }
        assert_eq!(full_clocks, window_clocks);
        assert_eq!(full_clocks[0].0, 2.0);
        let _ = full.finish(true);
        let _ = windowed.finish(true);
    }

    /// The staging WAV is this module's to clean up — the encoder knows nothing
    /// about it — and it must not survive either ending
    /// (`finish_rendering_track`, `plug.c:7258-7264`).
    #[test]
    fn finishing_takes_the_staging_wav_with_it_and_publishes_nothing_on_its_own() {
        let scratch = Scratch::new("staging");
        let job = prepared_job(&scratch, "staging", 44_100, None);
        let staged = job.staged_audio.clone();
        let destination = job.destination().to_path_buf();
        std::fs::write(&destination, b"a render from yesterday").expect("previous");
        assert!(staged.is_file());

        let completion = job.finish(true);
        assert!(matches!(completion.result, Ok(Finished::Cancelled)));
        assert_eq!(completion.retained_staging, None);
        assert!(!staged.exists(), "the staging WAV must not survive");
        assert_eq!(
            std::fs::read(&destination).expect("previous render"),
            b"a render from yesterday",
            "cancelling must not destroy the previous render"
        );
    }

    /// Dropping a job without finishing it is a bug, and the net has to hold.
    #[test]
    fn dropping_a_job_removes_its_staging_wav() {
        let scratch = Scratch::new("drop");
        let staged;
        let destination;
        {
            let job = prepared_job(&scratch, "drop", 44_100, None);
            staged = job.staged_audio.clone();
            destination = job.destination().to_path_buf();
        }
        assert!(!staged.exists());
        assert!(!destination.exists(), "Drop must never publish");
    }

    #[test]
    fn the_finishing_handshake_takes_two_ticks() {
        // The oracle repaints the progress screen saying "Finishing encoder"
        // before it blocks on waitpid (`plug.c:7903-7904`).
        let scratch = Scratch::new("finishing");
        let mut job = prepared_job(&scratch, "finishing", 44_100, None);
        assert!(!job.is_finishing());
        assert!(!job.begin_finishing());
        assert!(job.is_finishing());
        assert!(job.begin_finishing());
        let _ = job.finish(true);
    }

    #[test]
    fn the_export_analyzer_follows_the_decoded_wave_not_the_stereo_preview() {
        // A mono file exports with channel_count 1 (`plug.c:7094`); reading it
        // as the preview's fixed stereo would take every other sample.
        let scratch = Scratch::new("analyzer");
        let job = prepared_job(&scratch, "analyzer", 44_100, None);
        let config = job.analyzer_config();
        assert_eq!(config.sample_rate, 44_100);
        assert_eq!(config.channel_count, 2);
        assert_eq!(config.channel_mode, ChannelMode::Select(0));
        let _ = job.finish(true);
    }

    #[test]
    fn quality_carries_through_to_the_encoders_own_table() {
        for (quality, expected) in [
            (Quality::Balanced, ExportQuality::Balanced),
            (Quality::High, ExportQuality::High),
            (Quality::Master, ExportQuality::Master),
        ] {
            let mut config = RenderExportConfig::default();
            config.set_quality(quality);
            let encoder = encoder_config(&config);
            assert_eq!(encoder.quality, expected);
            assert_eq!(encoder.supersample_factor, config.supersample_factor);
            assert_eq!(encoder.validate().is_ok(), config.validate().is_ok());
        }
    }
}
