//! The deterministic export transport: frame counts over decoded audio frames.
//!
//! **Owner: Agent A.** Port of `../musializer/src/render_export.c` and `.h`.
//!
//! This module is the reason "preview and export use the same scene semantics"
//! is true rather than aspirational. Every function here is exact integer or
//! bounded floating-point arithmetic over the decoded audio length, with no
//! clock, no randomness, and no I/O — so the frame at index *n* is the same frame
//! whether it was drawn to a window or piped to an encoder.
//!
//! **The FFmpeg process is not here.** It is Agent E's, in
//! `musializer-runtime`. What lives here is the maths that process is driven by:
//! how many frames a track is, which sample each frame starts at, which frames a
//! render window covers, the `-t` duration cap, the supersample scale, and the
//! finalization deadlines both the export backend and its tests share.
//!
//! ## Deliberate divergences from C, all of them error classes C can reach and
//! Rust cannot
//!
//! - `set_resolution` / `set_frame_rate` / `set_quality` are infallible. C
//!   returns `RENDER_EXPORT_ERROR_*` for an out-of-range enum
//!   (`render_export.c:34-36`); a Rust `enum` cannot hold one.
//! - The path and duration helpers return `String`, so
//!   [`RenderExportError::OutputBufferTooSmall`] is unreachable. The variant is
//!   kept because C's `render_export_result_string` table
//!   (`render_export.c:105-115`) is user-visible text that Agent F's notices will
//!   want verbatim, and dropping a row would silently renumber that contract.

use std::ops::Range;

/// Largest frame rate the transport accepts (`render_export.c:124`).
pub const MAX_FPS: u32 = 240;

/// Output resolution presets (`render_export.h:8-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    P720,
    P1080,
    P1440,
    P2160,
}

impl Resolution {
    /// Every preset, in C's enum order.
    pub const ALL: [Self; 4] = [Self::P720, Self::P1080, Self::P1440, Self::P2160];

    /// Pixel dimensions (`render_export.c:31-32`).
    #[must_use]
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::P720 => (1280, 720),
            Self::P1080 => (1920, 1080),
            Self::P1440 => (2560, 1440),
            Self::P2160 => (3840, 2160),
        }
    }

    /// The UI label (`render_export.c:89`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::P1440 => "1440p",
            Self::P2160 => "2160p",
        }
    }

    /// The number the rung is named after: the **short** edge (EX2).
    ///
    /// At 16:9 that is the height, which is what "1080p" has always meant here
    /// and in the C. Naming it explicitly is what lets [`Aspect`] turn one rung
    /// into a vertical or square geometry without a second table.
    #[must_use]
    pub fn short_edge(self) -> u32 {
        self.dimensions().1
    }

    /// Which rung a geometry sits on, by its short edge, if any.
    #[must_use]
    pub fn of_short_edge(edge: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|rung| rung.short_edge() == edge)
    }
}

/// Output aspect-ratio presets (EX2, operator request 2026-08-06).
///
/// **Not the oracle's.** The frozen C has four resolution buttons and all four
/// are 16:9 (`render_export.c:31-32`); a vertical or square export is simply not
/// expressible in its interface. Nothing else in the pipeline needed changing —
/// [`RenderExportConfig::validate`] already accepts any even geometry from 16x16
/// to 7680x4320, and `--resolution 1080x1920` rendered correctly before this
/// enum existed. What was missing was a way to *ask* for it.
///
/// The four are the ones a person actually delivers to: 16:9 for everything with
/// a landscape player, 9:16 for Reels/Shorts/TikTok, 1:1 for a feed post, and
/// 4:5 for the taller Instagram portrait crop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    Wide16x9,
    Tall9x16,
    Square1x1,
    Portrait4x5,
}

impl Aspect {
    pub const ALL: [Self; 4] = [
        Self::Wide16x9,
        Self::Tall9x16,
        Self::Square1x1,
        Self::Portrait4x5,
    ];

    /// Width and height as a ratio, in that order.
    #[must_use]
    pub fn ratio(self) -> (u32, u32) {
        match self {
            Self::Wide16x9 => (16, 9),
            Self::Tall9x16 => (9, 16),
            Self::Square1x1 => (1, 1),
            Self::Portrait4x5 => (4, 5),
        }
    }

    /// The UI label.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Wide16x9 => "16:9",
            Self::Tall9x16 => "9:16",
            Self::Square1x1 => "1:1",
            Self::Portrait4x5 => "4:5",
        }
    }

    /// The geometry this aspect gives a [`Resolution`] rung.
    ///
    /// The rung names the **short** edge, which is the only reading that makes
    /// "1080p" mean the same amount of picture in every shape: at 16:9 the short
    /// edge is the height and the answer is the C's own 1920x1080, so every
    /// existing preset is unchanged; at 9:16 it is the width and the answer is
    /// 1080x1920, which is what every vertical platform calls 1080p.
    ///
    /// Every result is even on both axes, which
    /// [`RenderExportConfig::validate`] requires for 4:2:0 chroma — check the
    /// table rather than assuming, because a rung and a ratio that produced an
    /// odd edge would be a preset button that refuses to export.
    #[must_use]
    pub fn dimensions(self, resolution: Resolution) -> (u32, u32) {
        let short = resolution.short_edge();
        let (width_ratio, height_ratio) = self.ratio();
        if width_ratio >= height_ratio {
            (short * width_ratio / height_ratio, short)
        } else {
            (short, short * height_ratio / width_ratio)
        }
    }

    /// Which preset a geometry is, if any.
    ///
    /// Exact, like [`selected_resolution`](RenderExportConfig) in the panel:
    /// a hand-written `--resolution 1234x568` belongs to no preset, and
    /// highlighting the nearest one would tell the user their export is 16:9
    /// when it is not.
    #[must_use]
    pub fn of(width: u32, height: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|aspect| {
            Resolution::ALL
                .into_iter()
                .any(|rung| aspect.dimensions(rung) == (width, height))
        })
    }
}

/// Frame-rate presets (`render_export.h:16-21`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameRate {
    Fps24,
    Fps30,
    Fps60,
}

impl FrameRate {
    pub const ALL: [Self; 3] = [Self::Fps24, Self::Fps30, Self::Fps60];

    /// Frames per second (`render_export.c:45`).
    #[must_use]
    pub fn fps(self) -> u32 {
        match self {
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }

    /// The UI label (`render_export.c:95`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fps24 => "24 fps",
            Self::Fps30 => "30 fps",
            Self::Fps60 => "60 fps",
        }
    }
}

/// Quality presets (`render_export.h:23-28`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Balanced,
    High,
    Master,
}

impl Quality {
    pub const ALL: [Self; 3] = [Self::Balanced, Self::High, Self::Master];

    /// Offline supersampling factor (`render_export.c:57`).
    ///
    /// Note that `High` and `Master` both supersample 2x: quality is *not* only
    /// a supersample selector, and the difference between them lives in the
    /// encoder settings Agent E owns rather than here.
    #[must_use]
    pub fn supersample_factor(self) -> u32 {
        match self {
            Self::Balanced => 1,
            Self::High | Self::Master => 2,
        }
    }

    /// The UI label (`render_export.c:101`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::High => "High",
            Self::Master => "Master",
        }
    }
}

/// Export geometry and quality (`render_export.h:30-36`).
///
/// [`Default`] is C's `render_export_config_init` (`render_export.c:16-26`):
/// 1920x1080 at 30 fps, `High` quality, 2x supersampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderExportConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: Quality,
    pub supersample_factor: u32,
}

impl Default for RenderExportConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            quality: Quality::High,
            supersample_factor: 2,
        }
    }
}

impl RenderExportConfig {
    /// Applies a resolution preset (`render_export.c:28-40`).
    ///
    /// Keeps the current **aspect** if the geometry is on one of [`Aspect`]'s
    /// presets, so changing 1080p to 2160p on a vertical export stays vertical
    /// (EX2). A geometry that is on no preset — a hand-written `--resolution` —
    /// falls back to the C's behaviour and becomes 16:9, because there is no
    /// aspect to preserve and quietly inventing one from the ratio would land on
    /// a size the user never chose.
    pub fn set_resolution(&mut self, resolution: Resolution) {
        let aspect = Aspect::of(self.width, self.height).unwrap_or(Aspect::Wide16x9);
        let (width, height) = aspect.dimensions(resolution);
        self.width = width;
        self.height = height;
    }

    /// Applies an aspect preset, keeping the current rung (EX2).
    ///
    /// The rung is read from the short edge, so a geometry that is on no preset
    /// at all still lands on the nearest rung's short edge rather than being
    /// refused: the user pressed a shape button and a shape has to happen.
    pub fn set_aspect(&mut self, aspect: Aspect) {
        let short = self.width.min(self.height);
        let rung = Resolution::of_short_edge(short).unwrap_or(Resolution::P1080);
        let (width, height) = aspect.dimensions(rung);
        self.width = width;
        self.height = height;
    }

    /// Applies a frame-rate preset (`render_export.c:42-52`).
    pub fn set_frame_rate(&mut self, frame_rate: FrameRate) {
        self.fps = frame_rate.fps();
    }

    /// Applies a quality preset, which also sets the supersample factor
    /// (`render_export.c:54-65`).
    ///
    /// The coupling is the oracle's and matters: `supersample_factor` is a public
    /// field, so nothing stops a caller writing it directly, but selecting a
    /// quality always overwrites it.
    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
        self.supersample_factor = quality.supersample_factor();
    }

    /// Validates the geometry (`render_export.c:67-85`).
    ///
    /// # Errors
    ///
    /// - [`RenderExportError::Resolution`] outside `16..=7680` x `16..=4320`, or
    ///   for an odd width or height. Odd dimensions are rejected because
    ///   4:2:0 chroma subsampling — which every preset FFmpeg pipeline here uses
    ///   — halves both axes.
    /// - [`RenderExportError::FrameRate`] outside `1..=240`.
    /// - [`RenderExportError::Supersample`] for a factor outside `1..=2`, or one
    ///   whose product with the dimensions would overflow `u32`.
    pub fn validate(&self) -> Result<(), RenderExportError> {
        if self.width < 16
            || self.height < 16
            || self.width > 7680
            || self.height > 4320
            || self.width % 2 != 0
            || self.height % 2 != 0
        {
            return Err(RenderExportError::Resolution);
        }
        if self.fps < 1 || self.fps > MAX_FPS {
            return Err(RenderExportError::FrameRate);
        }
        if self.supersample_factor < 1
            || self.supersample_factor > 2
            || self.width > u32::MAX / self.supersample_factor
            || self.height > u32::MAX / self.supersample_factor
        {
            return Err(RenderExportError::Supersample);
        }
        Ok(())
    }

    /// The uniform scale from the logical composition to an offline target
    /// (`render_export.c:209-229`).
    ///
    /// The target must be an *exact* integer multiple of the configured size on
    /// both axes, by the same factor, and no larger than the supersample factor
    /// allows. Anything else would change the composition rather than
    /// supersample it — a scene that lays out captions as fractions of the frame
    /// would still be correct, but one that reasons about aspect ratio would not.
    ///
    /// # Errors
    /// Propagates [`RenderExportConfig::validate`], then
    /// [`RenderExportError::Resolution`] for a zero, non-multiple, non-uniform,
    /// or too-large target.
    pub fn target_scale(
        &self,
        target_width: u32,
        target_height: u32,
    ) -> Result<f32, RenderExportError> {
        self.validate()?;
        if target_width == 0
            || target_height == 0
            || target_width % self.width != 0
            || target_height % self.height != 0
        {
            return Err(RenderExportError::Resolution);
        }
        let width_scale = target_width / self.width;
        let height_scale = target_height / self.height;
        if width_scale == 0 || width_scale != height_scale || width_scale > self.supersample_factor
        {
            return Err(RenderExportError::Resolution);
        }
        Ok(width_scale as f32)
    }

    /// Suggests a sibling output path such as
    /// `song-musializer-constellation-1080p30.mp4` (`render_export.c:242-272`).
    ///
    /// `scene_name` must be non-empty ASCII alphanumerics, `-`, or `_`
    /// (`render_export.c:231-240`), because it goes straight into a file name.
    ///
    /// The prefix is `audio_path` up to the last `.` **in its file-name
    /// component**, so a dot in a directory name cannot truncate the path. A
    /// name with no dot, or a dotfile whose only dot is leading, keeps the whole
    /// path (`render_export.c:261-262`).
    ///
    /// # Errors
    /// [`RenderExportError::Path`] for an empty `audio_path` or an unsafe
    /// `scene_name`; otherwise whatever [`RenderExportConfig::validate`] returns.
    pub fn suggest_path(
        &self,
        audio_path: &str,
        scene_name: &str,
    ) -> Result<String, RenderExportError> {
        if audio_path.is_empty() || !scene_name_is_safe(scene_name) {
            return Err(RenderExportError::Path);
        }
        self.validate()?;

        let name_start = audio_path.rfind(['/', '\\']).map_or(0, |index| index + 1);
        let name = &audio_path[name_start..];
        // `dot != name` in C: a leading dot is a hidden file, not an extension.
        let prefix_length = match name.rfind('.') {
            Some(0) | None => audio_path.len(),
            Some(index) => name_start + index,
        };
        Ok(format!(
            "{}-musializer-{}-{}p{}.mp4",
            &audio_path[..prefix_length],
            scene_name,
            self.height,
            self.fps
        ))
    }
}

/// Every failure the transport reports (`render_export.h:38-49`).
///
/// The `Display` strings are C's `render_export_result_string` table verbatim
/// (`render_export.c:105-115`) so a UI notice reads identically. `RENDER_EXPORT_OK`
/// has no variant; it is `Ok(())`.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderExportError {
    #[error("invalid output resolution")]
    Resolution,
    #[error("invalid frame rate")]
    FrameRate,
    #[error("invalid quality")]
    Quality,
    #[error("invalid supersampling")]
    Supersample,
    #[error("integer overflow")]
    Overflow,
    #[error("invalid output path")]
    Path,
    /// Unreachable in Rust; see the module docs. Kept for message parity.
    #[error("output buffer is too small")]
    OutputBufferTooSmall,
    #[error("render window is outside the track or not a positive finite range")]
    Window,
}

/// What a finalization poll should do next (`render_export.h:51-55`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitAction {
    Continue,
    Complete,
    Timeout,
}

/// Total video frames for a decoded track (`render_export.c:117-132`).
///
/// `frame_count` is **decoded audio frames**, not interleaved samples. The count
/// rounds *up*, so the last partial video frame is rendered rather than dropped,
/// and a track shorter than one frame still produces one.
///
/// Note the error classes, which look wrong and are reproduced anyway: a zero
/// `frame_count` or a zero `sample_rate` both report
/// [`RenderExportError::FrameRate`] (`render_export.c:123`), not a dedicated
/// class. Do not "fix" that here without the human's call — Agent F may be
/// matching notice text against it.
///
/// # Errors
/// [`RenderExportError::FrameRate`] for a zero `frame_count`, a zero
/// `sample_rate`, or an `fps` outside `1..=240`;
/// [`RenderExportError::Overflow`] when `frame_count * fps` would not fit `u64`.
pub fn total_frames(
    frame_count: u64,
    sample_rate: u32,
    fps: u32,
) -> Result<u64, RenderExportError> {
    if frame_count == 0 || sample_rate == 0 {
        return Err(RenderExportError::FrameRate);
    }
    if fps == 0 || fps > MAX_FPS {
        return Err(RenderExportError::FrameRate);
    }
    let fps = u64::from(fps);
    if frame_count > u64::MAX / fps {
        return Err(RenderExportError::Overflow);
    }
    let numerator = frame_count * fps;
    let sample_rate = u64::from(sample_rate);
    let mut result = numerator / sample_rate;
    if numerator % sample_rate != 0 {
        result += 1;
    }
    if result == 0 {
        result = 1;
    }
    Ok(result)
}

/// The decoded audio frame a given video frame starts at
/// (`render_export.c:134-147`).
///
/// Truncating rather than rounding is what makes the mapping exact and
/// monotonic: video frame *n* always begins at `n * sample_rate / fps`, so
/// consecutive frames never revisit a sample. The result clamps to `frame_count`,
/// which is how the transport ends at the audio's duration instead of running on
/// through a reverb tail that is not there.
///
/// # Errors
/// [`RenderExportError::FrameRate`] for a zero `sample_rate` or an `fps` outside
/// `1..=240`; [`RenderExportError::Overflow`] when `frame_index * sample_rate`
/// would not fit `u64`.
pub fn sample_cursor(
    frame_index: u64,
    sample_rate: u32,
    fps: u32,
    frame_count: u64,
) -> Result<u64, RenderExportError> {
    if sample_rate == 0 || fps == 0 || fps > MAX_FPS {
        return Err(RenderExportError::FrameRate);
    }
    let sample_rate = u64::from(sample_rate);
    if frame_index > u64::MAX / sample_rate {
        return Err(RenderExportError::Overflow);
    }
    Ok((frame_index * sample_rate / u64::from(fps)).min(frame_count))
}

/// Resolves a render window in seconds onto exact frame indices
/// (`render_export.c:149-181`).
///
/// The returned [`Range`] is half-open — `start..end` — with **absolute** indices
/// on the full timeline, not indices relative to the window. Its three rounding
/// rules are asymmetric on purpose (`render_export.h:83-88`):
///
/// - the **start floors**, to the frame whose interval contains `start_seconds`;
/// - the **end ceils**, so the requested interval is fully enclosed rather than
///   losing its tail;
/// - a **sub-frame duration still yields one frame**, via `end = start + 1`.
///
/// A duration that runs past the track, including a huge finite one like
/// `1e300`, clamps to `total_frames` without overflowing.
///
/// # Errors
/// [`RenderExportError::FrameRate`] for an `fps` outside `1..=240`;
/// [`RenderExportError::Window`] for a zero `total_frames`, a non-finite bound,
/// a negative start, a non-positive duration, or a start at or after the end of
/// the track.
pub fn window_frames(
    total_frames: u64,
    fps: u32,
    start_seconds: f64,
    duration_seconds: f64,
) -> Result<Range<u64>, RenderExportError> {
    if fps == 0 || fps > MAX_FPS {
        return Err(RenderExportError::FrameRate);
    }
    if total_frames == 0
        || !start_seconds.is_finite()
        || !duration_seconds.is_finite()
        || start_seconds < 0.0
        || duration_seconds <= 0.0
    {
        return Err(RenderExportError::Window);
    }
    // The start position stays comfortably inside f64's exact-integer range
    // because it is bounded by total_frames, itself derived from a decodable
    // audio length (`render_export.c:163-165`).
    let total = total_frames as f64;
    let start_position = start_seconds * f64::from(fps);
    // C writes this as `!(start_position < total_frames)` to catch NaN
    // (`render_export.c:167`). Here NaN is already excluded by the `is_finite`
    // check above — the product can only be finite or `+inf` — so the plain
    // comparison is equivalent and clippy-clean.
    if start_position >= total {
        return Err(RenderExportError::Window);
    }
    let start = start_position as u64;

    let requested_end_seconds = start_seconds + duration_seconds;
    let requested_end_position = requested_end_seconds * f64::from(fps);
    let mut end = total_frames;
    if requested_end_seconds.is_finite()
        && requested_end_position.is_finite()
        && requested_end_position < total
    {
        end = requested_end_position.ceil() as u64;
    }
    if end <= start {
        end = start + 1;
    }
    Ok(start..end)
}

/// The exact transport duration for FFmpeg's output `-t` cap
/// (`render_export.c:183-207`).
///
/// Nine fractional digits are enough to be exact for every supported integer
/// FPS, and the nanosecond field rounds half-up with an explicit carry into the
/// whole-seconds field, so the text never reads `N.1000000000`.
///
/// # Errors
/// [`RenderExportError::FrameRate`] for a zero `total_frames` or an `fps`
/// outside `1..=240`; [`RenderExportError::Overflow`] only if the carry would
/// overflow `u64` seconds.
pub fn transport_duration_text(total_frames: u64, fps: u32) -> Result<String, RenderExportError> {
    if total_frames == 0 || fps == 0 || fps > MAX_FPS {
        return Err(RenderExportError::FrameRate);
    }
    let fps = u64::from(fps);
    let mut whole_seconds = total_frames / fps;
    let remainder = total_frames % fps;
    let mut nanoseconds = (remainder * 1_000_000_000 + fps / 2) / fps;
    if nanoseconds == 1_000_000_000 {
        if whole_seconds == u64::MAX {
            return Err(RenderExportError::Overflow);
        }
        whole_seconds += 1;
        nanoseconds = 0;
    }
    Ok(format!("{whole_seconds}.{nanoseconds:09}"))
}

/// Whether a name is safe to interpolate into a file name
/// (`render_export.c:231-240`).
fn scene_name_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// The directory prefix of a path, including its trailing separator
/// (`render_export.c:293-299`).
fn directory_prefix(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(index) => &path[..=index],
        None => "",
    }
}

/// A hidden sibling MP4 that FFmpeg can infer a muxer for
/// (`render_export.c:274-316`).
///
/// Two properties make this the destination an export actually writes to:
/// it is a **sibling** of the final path, so publication is a rename within one
/// directory rather than a cross-device copy; and it keeps the `.mp4` extension,
/// so FFmpeg does not need `-f mp4`. `process_id` and `nonce` keep concurrent
/// exports from colliding, and both must be non-zero.
///
/// Returns `None` for an empty path or a zero `process_id`/`nonce`. C's
/// 65-byte identity-buffer failure (`render_export.c:289`) is unreachable: two
/// `u64` and a separator is at most 41 characters.
#[must_use]
pub fn temporary_path(output_path: &str, process_id: u64, nonce: u64) -> Option<String> {
    if output_path.is_empty() || process_id == 0 || nonce == 0 {
        return None;
    }
    Some(format!(
        "{}.musializer-{process_id}-{nonce}.part.mp4",
        directory_prefix(output_path)
    ))
}

/// A hidden sibling WAV for the decoded PCM the analyzer and FFmpeg share
/// (`render_export.c:318-344`).
///
/// It lands beside the *output*, not beside the source audio, because the output
/// directory is the one the user just proved writable.
///
/// Returns `None` for an empty path or a zero `process_id`/`nonce`.
#[must_use]
pub fn decoded_audio_path(output_path: &str, process_id: u64, nonce: u64) -> Option<String> {
    if output_path.is_empty() || process_id == 0 || nonce == 0 {
        return None;
    }
    Some(format!(
        "{}.musializer-audio-{process_id}-{nonce}.wav",
        directory_prefix(output_path)
    ))
}

/// How long to wait for the encoder to finish (`render_export.c:346-349`).
///
/// Normal completion gets five minutes because finalizing a 4K MP4 may relocate
/// the whole file to move the `moov` atom to the front for faststart.
/// Cancellation gets five seconds: the user asked for it to stop, and the output
/// is being discarded anyway.
#[must_use]
pub fn finalize_grace_ms(cancellation: bool) -> u32 {
    if cancellation {
        5_000
    } else {
        300_000
    }
}

/// The shared deadline decision for every export backend
/// (`render_export.c:351-358`).
///
/// The boundary is `>=`: at exactly the grace period the action is
/// [`WaitAction::Timeout`]. An exited process reports [`WaitAction::Complete`]
/// however long it took, so a slow but successful finalize is never killed after
/// the fact.
#[must_use]
pub fn wait_action(process_exited: bool, elapsed_ms: u64, cancellation: bool) -> WaitAction {
    if process_exited {
        return WaitAction::Complete;
    }
    if elapsed_ms >= u64::from(finalize_grace_ms(cancellation)) {
        WaitAction::Timeout
    } else {
        WaitAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_rejects_degenerate_rates_and_counts() {
        assert_eq!(
            total_frames(0, 48_000, 30),
            Err(RenderExportError::FrameRate),
            "a zero frame count reports FrameRate, oddly but faithfully"
        );
        assert_eq!(
            total_frames(1, 0, 30),
            Err(RenderExportError::FrameRate),
            "so does a zero sample rate"
        );
        assert_eq!(
            total_frames(1, 48_000, 0),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            total_frames(1, 48_000, MAX_FPS + 1),
            Err(RenderExportError::FrameRate)
        );
        assert!(total_frames(1, 48_000, MAX_FPS).is_ok(), "240 is in range");

        assert_eq!(
            total_frames(u64::MAX, 48_000, 30),
            Err(RenderExportError::Overflow)
        );

        assert_eq!(
            sample_cursor(0, 0, 30, 10),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            sample_cursor(0, 48_000, MAX_FPS + 1, 10),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            sample_cursor(u64::MAX, 48_000, 30, 10),
            Err(RenderExportError::Overflow)
        );
    }

    #[test]
    fn transport_ends_at_the_audio_duration_without_a_decay_tail() {
        // 12000 frames at 48 kHz is 0.25 s: 7.5 video frames, rounded up to 8.
        assert_eq!(total_frames(12_000, 48_000, 30), Ok(8));
        assert_eq!(sample_cursor(7, 48_000, 30, 12_000), Ok(11_200));
        // The final frame's cursor clamps to the end of the audio.
        assert_eq!(sample_cursor(8, 48_000, 30, 12_000), Ok(12_000));

        assert_eq!(total_frames(44_100, 44_100, 60), Ok(60));
        // A single sample is still one whole video frame.
        assert_eq!(total_frames(1, 48_000, 24), Ok(1));
    }

    /// The cursor mapping has to be monotonic and never revisit a sample, or two
    /// consecutive export frames would analyze overlapping audio.
    #[test]
    fn sample_cursors_advance_monotonically_and_cover_the_track() {
        let (frame_count, sample_rate, fps) = (44_100u64, 44_100u32, 30u32);
        let total = total_frames(frame_count, sample_rate, fps).unwrap();
        let mut previous = 0u64;
        for index in 0..=total {
            let cursor = sample_cursor(index, sample_rate, fps, frame_count).unwrap();
            assert!(cursor >= previous, "cursor went backwards at frame {index}");
            assert!(cursor <= frame_count, "cursor ran past the decoded audio");
            previous = cursor;
        }
        assert_eq!(previous, frame_count, "the last frame reaches the end");
        assert_eq!(sample_cursor(0, sample_rate, fps, frame_count), Ok(0));
    }

    #[test]
    fn window_rejects_degenerate_ranges() {
        // Starting at or after the end of the track cannot render anything.
        assert_eq!(
            window_frames(240, 24, 10.0, 1.0),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            window_frames(240, 24, 11.0, 1.0),
            Err(RenderExportError::Window)
        );
        // Non-positive, non-finite, and negative ranges.
        for (start, duration) in [
            (-0.5, 1.0),
            (0.0, 0.0),
            (0.0, -2.0),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (0.0, f64::NAN),
            (f64::INFINITY, 1.0),
        ] {
            assert_eq!(
                window_frames(240, 24, start, duration),
                Err(RenderExportError::Window),
                "start={start} duration={duration}"
            );
        }
        assert_eq!(
            window_frames(0, 24, 0.0, 1.0),
            Err(RenderExportError::Window)
        );
        // An invalid transport keeps its own error class.
        assert_eq!(
            window_frames(240, 0, 0.0, 1.0),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            window_frames(240, MAX_FPS + 1, 0.0, 1.0),
            Err(RenderExportError::FrameRate)
        );
    }

    #[test]
    fn window_maps_to_exact_frames_and_clamps() {
        // The full track expressed as a window is the identity transport.
        assert_eq!(window_frames(240, 24, 0.0, 10.0), Ok(0..240));
        // Interior windows use absolute indices on the full timeline.
        assert_eq!(window_frames(4661, 24, 35.0, 20.0), Ok(840..1320));
        // A start inside a frame floors; the end encloses the whole interval.
        assert_eq!(window_frames(240, 24, 0.1, 1.0), Ok(2..27));
        // Crossing both frame boundaries by a fraction includes both frames.
        assert_eq!(
            window_frames(240, 24, 1.0 / 24.0 + 1.0e-9, 1.0 / 24.0),
            Ok(1..3)
        );
        // A sub-frame duration still renders one whole frame.
        assert_eq!(window_frames(240, 24, 1.0, 0.001), Ok(24..25));
        // Durations past the end of the track clamp without overflow.
        assert_eq!(window_frames(240, 24, 9.0, 100.0), Ok(216..240));
        assert_eq!(window_frames(240, 24, 0.0, 1.0e300), Ok(0..240));
    }

    /// Not in the C suite. Every accepted window must be a non-empty subrange of
    /// the timeline, which is what lets the export loop treat `start..end` as a
    /// plain iteration bound without re-checking anything.
    #[test]
    fn every_accepted_window_is_a_non_empty_subrange_of_the_track() {
        let total = 240u64;
        for start_ms in (0..10_000).step_by(137) {
            for duration_ms in [1u64, 40, 41, 999, 1_000, 9_999, 20_000] {
                let start = start_ms as f64 / 1000.0;
                let duration = duration_ms as f64 / 1000.0;
                if let Ok(window) = window_frames(total, 24, start, duration) {
                    assert!(window.start < window.end, "empty window for {start}s");
                    assert!(window.start < total);
                    assert!(window.end <= total, "{:?} escaped the track", window);
                }
            }
        }
    }

    #[test]
    fn duration_text_is_exact_and_rejects_a_degenerate_transport() {
        assert_eq!(transport_duration_text(24, 30).unwrap(), "0.800000000");
        assert_eq!(transport_duration_text(5, 24).unwrap(), "0.208333333");
        assert_eq!(
            transport_duration_text(0, 30),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            transport_duration_text(1, 0),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            transport_duration_text(1, MAX_FPS + 1),
            Err(RenderExportError::FrameRate)
        );
    }

    /// The nanosecond field is always nine digits and never carries into a tenth,
    /// which is the property FFmpeg's `-t` parser needs.
    #[test]
    fn duration_text_is_always_nine_fractional_digits() {
        for fps in [1u32, 24, 25, 30, 50, 60, 120, MAX_FPS] {
            for frames in [1u64, 2, 3, 7, 23, 24, 25, 59, 60, 61, 1_000, 123_456] {
                let text = transport_duration_text(frames, fps).unwrap();
                let (whole, fraction) = text.split_once('.').expect("always has a point");
                assert_eq!(fraction.len(), 9, "{text} for {frames}@{fps}");
                assert!(fraction.parse::<u64>().unwrap() < 1_000_000_000, "{text}");
                assert!(!whole.is_empty());
                // Within half a nanosecond of the exact rational duration.
                let exact = frames as f64 / f64::from(fps);
                let parsed: f64 = text.parse().unwrap();
                assert!((parsed - exact).abs() < 1.0e-9, "{text} vs {exact}");
            }
        }
    }

    /// The exact-second case is where a naive `format!("{:.9}")` would drift:
    /// integer seconds must print a zero fraction, not `999999999`.
    #[test]
    fn duration_text_prints_whole_seconds_cleanly() {
        assert_eq!(transport_duration_text(30, 30).unwrap(), "1.000000000");
        assert_eq!(transport_duration_text(240, 24).unwrap(), "10.000000000");
        assert_eq!(transport_duration_text(1, 1).unwrap(), "1.000000000");
    }

    #[test]
    fn defaults_and_presets_are_valid() {
        let mut config = RenderExportConfig::default();
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.fps, 30);
        assert_eq!(config.quality, Quality::High);
        assert_eq!(config.supersample_factor, 2);
        assert_eq!(config.validate(), Ok(()));

        for resolution in Resolution::ALL {
            config.set_resolution(resolution);
            assert_eq!(config.validate(), Ok(()), "{}", resolution.name());
            assert!(!resolution.name().is_empty());
        }
        for frame_rate in FrameRate::ALL {
            config.set_frame_rate(frame_rate);
            assert_eq!(config.validate(), Ok(()), "{}", frame_rate.name());
        }
        for quality in Quality::ALL {
            config.set_quality(quality);
            assert_eq!(config.validate(), Ok(()), "{}", quality.name());
            assert_eq!(config.supersample_factor, quality.supersample_factor());
        }
        // The preset labels are user-visible strings; pin them.
        assert_eq!(Resolution::P2160.name(), "2160p");
        assert_eq!(FrameRate::Fps60.name(), "60 fps");
        assert_eq!(Quality::Balanced.name(), "Balanced");
        assert_eq!(Quality::Balanced.supersample_factor(), 1);
        assert_eq!(Quality::Master.supersample_factor(), 2);
    }

    /// Every rung x every aspect is a legal export, and 16:9 is byte-identical
    /// to the C's own table.
    ///
    /// The evenness check is the load-bearing one: `validate` refuses an odd
    /// edge because 4:2:0 halves both axes, so a rung-and-ratio pair that
    /// produced one would be a preset button that cannot export. It is asserted
    /// rather than eyeballed because 4:5 of 1440 is 1800 and 4:5 of 720 is 900 —
    /// both fine, but nothing about the formula guarantees that in general.
    #[test]
    fn every_aspect_and_rung_is_a_legal_even_geometry() {
        for rung in Resolution::ALL {
            // 16:9 must reproduce the oracle's table exactly, or every existing
            // preset silently moves.
            assert_eq!(
                Aspect::Wide16x9.dimensions(rung),
                rung.dimensions(),
                "{} at 16:9 drifted from the C's table",
                rung.name()
            );
            for aspect in Aspect::ALL {
                let (width, height) = aspect.dimensions(rung);
                assert_eq!(width % 2, 0, "{} at {} is odd", rung.name(), aspect.name());
                assert_eq!(height % 2, 0, "{} at {} is odd", rung.name(), aspect.name());
                let config = RenderExportConfig {
                    width,
                    height,
                    ..RenderExportConfig::default()
                };
                assert_eq!(
                    config.validate(),
                    Ok(()),
                    "{} at {} does not validate",
                    rung.name(),
                    aspect.name()
                );
                assert_eq!(
                    width.min(height),
                    rung.short_edge(),
                    "the rung names the short edge"
                );
            }
        }
    }

    /// The geometries a person would name, pinned by hand.
    #[test]
    fn the_named_geometries_are_what_the_platforms_call_them() {
        assert_eq!(Aspect::Wide16x9.dimensions(Resolution::P1080), (1920, 1080));
        assert_eq!(Aspect::Tall9x16.dimensions(Resolution::P1080), (1080, 1920));
        assert_eq!(
            Aspect::Square1x1.dimensions(Resolution::P1080),
            (1080, 1080)
        );
        assert_eq!(
            Aspect::Portrait4x5.dimensions(Resolution::P1080),
            (1080, 1350)
        );
        assert_eq!(Aspect::Tall9x16.dimensions(Resolution::P2160), (2160, 3840));
        // 2160 at 4:5 is 2700 tall, comfortably inside the 4320 ceiling.
        assert_eq!(
            Aspect::Portrait4x5.dimensions(Resolution::P2160),
            (2160, 2700)
        );
    }

    /// Changing the rung keeps the shape, and changing the shape keeps the rung.
    ///
    /// This is the whole reason the two live on one config rather than as eight
    /// independent buttons: pressing 2160p on a vertical export must not silently
    /// turn it landscape.
    #[test]
    fn a_rung_and_an_aspect_are_independent_choices() {
        let mut config = RenderExportConfig::default();
        config.set_aspect(Aspect::Tall9x16);
        assert_eq!((config.width, config.height), (1080, 1920));
        config.set_resolution(Resolution::P2160);
        assert_eq!(
            (config.width, config.height),
            (2160, 3840),
            "the rung changed and the shape did not"
        );
        config.set_aspect(Aspect::Square1x1);
        assert_eq!(
            (config.width, config.height),
            (2160, 2160),
            "the shape changed and the rung did not"
        );
        config.set_aspect(Aspect::Wide16x9);
        assert_eq!((config.width, config.height), (3840, 2160));
    }

    /// A geometry no preset produces belongs to no preset, both ways.
    #[test]
    fn a_hand_written_geometry_matches_no_aspect_and_falls_back_to_wide() {
        assert_eq!(Aspect::of(1920, 1080), Some(Aspect::Wide16x9));
        assert_eq!(Aspect::of(1080, 1920), Some(Aspect::Tall9x16));
        assert_eq!(Aspect::of(1234, 568), None);
        assert_eq!(Resolution::of_short_edge(1080), Some(Resolution::P1080));
        assert_eq!(Resolution::of_short_edge(568), None);

        // `--resolution 1234x568` then 1440p: no aspect to preserve, so the C's
        // own behaviour — the 16:9 rung — rather than a ratio invented from a
        // number the user typed for some other reason.
        let mut odd = RenderExportConfig {
            width: 1234,
            height: 568,
            ..RenderExportConfig::default()
        };
        odd.set_resolution(Resolution::P1440);
        assert_eq!((odd.width, odd.height), (2560, 1440));
    }

    #[test]
    fn validate_rejects_out_of_bounds_and_odd_geometry() {
        let base = RenderExportConfig::default();
        for (width, height) in [(14u32, 1080u32), (1920, 14), (7682, 1080), (1920, 4322)] {
            let config = RenderExportConfig {
                width,
                height,
                ..base
            };
            assert_eq!(config.validate(), Err(RenderExportError::Resolution));
        }
        // Odd dimensions break 4:2:0 chroma subsampling.
        for (width, height) in [(1921u32, 1080u32), (1920, 1081)] {
            let config = RenderExportConfig {
                width,
                height,
                ..base
            };
            assert_eq!(config.validate(), Err(RenderExportError::Resolution));
        }
        assert_eq!(
            RenderExportConfig { fps: 0, ..base }.validate(),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            RenderExportConfig {
                fps: MAX_FPS + 1,
                ..base
            }
            .validate(),
            Err(RenderExportError::FrameRate)
        );
        for factor in [0u32, 3, u32::MAX] {
            assert_eq!(
                RenderExportConfig {
                    supersample_factor: factor,
                    ..base
                }
                .validate(),
                Err(RenderExportError::Supersample),
                "factor {factor}"
            );
        }
    }

    #[test]
    fn target_scale_preserves_the_logical_composition() {
        let config = RenderExportConfig::default();
        assert_eq!(config.target_scale(3840, 2160), Ok(2.0));
        assert_eq!(config.target_scale(1920, 1080), Ok(1.0));
        // Non-uniform, non-multiple, zero, and beyond the supersample factor.
        assert_eq!(
            config.target_scale(3840, 1080),
            Err(RenderExportError::Resolution)
        );
        assert_eq!(
            config.target_scale(1921, 1080),
            Err(RenderExportError::Resolution)
        );
        assert_eq!(
            config.target_scale(0, 1080),
            Err(RenderExportError::Resolution)
        );
        assert_eq!(
            config.target_scale(5760, 3240),
            Err(RenderExportError::Resolution),
            "3x exceeds the 2x supersample factor"
        );

        // Balanced supersamples 1x, so even 2x is refused.
        let mut balanced = RenderExportConfig::default();
        balanced.set_quality(Quality::Balanced);
        assert_eq!(balanced.target_scale(1920, 1080), Ok(1.0));
        assert_eq!(
            balanced.target_scale(3840, 2160),
            Err(RenderExportError::Resolution)
        );

        // An invalid config fails before the target is even considered.
        let broken = RenderExportConfig {
            fps: 0,
            ..RenderExportConfig::default()
        };
        assert_eq!(
            broken.target_scale(1920, 1080),
            Err(RenderExportError::FrameRate)
        );
    }

    #[test]
    fn suggested_paths_reject_unsafe_names_and_empty_sources() {
        let config = RenderExportConfig::default();
        assert_eq!(
            config.suggest_path("", "constellation"),
            Err(RenderExportError::Path)
        );
        for name in ["", "bad/name", "bad name", "bad.name", "bad\\name", "b*d"] {
            assert_eq!(
                config.suggest_path("song.wav", name),
                Err(RenderExportError::Path),
                "{name:?} must not reach a file name"
            );
        }
        for name in ["ok", "ok-name", "ok_name", "OK123"] {
            assert!(config.suggest_path("song.wav", name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn suggests_a_descriptive_sibling_path() {
        let config = RenderExportConfig::default();
        assert_eq!(
            config
                .suggest_path("/music/Autoregressive Kitty.mp3", "constellation")
                .unwrap(),
            "/music/Autoregressive Kitty-musializer-constellation-1080p30.mp4"
        );
    }

    /// The extension is stripped from the *file name*, so a dot in a directory
    /// cannot truncate the path, and a dotfile keeps its whole name.
    #[test]
    fn only_the_file_names_own_extension_is_stripped() {
        let config = RenderExportConfig::default();
        let cases = [
            ("/a.b/song", "/a.b/song-musializer-x-1080p30.mp4"),
            ("/a.b/song.wav", "/a.b/song-musializer-x-1080p30.mp4"),
            ("song", "song-musializer-x-1080p30.mp4"),
            // A leading dot is a hidden file, not an extension.
            ("/a/.hidden", "/a/.hidden-musializer-x-1080p30.mp4"),
            (".hidden", ".hidden-musializer-x-1080p30.mp4"),
            ("/a/.hidden.wav", "/a/.hidden-musializer-x-1080p30.mp4"),
            ("C:\\m\\song.flac", "C:\\m\\song-musializer-x-1080p30.mp4"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                config.suggest_path(input, "x").unwrap(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn temporary_and_decoded_paths_reject_degenerate_input() {
        assert_eq!(temporary_path("", 1, 1), None);
        assert_eq!(temporary_path("movie.mp4", 0, 1), None);
        assert_eq!(temporary_path("movie.mp4", 1, 0), None);
        assert_eq!(decoded_audio_path("", 1, 1), None);
        assert_eq!(decoded_audio_path("movie.mp4", 0, 1), None);
        assert_eq!(decoded_audio_path("movie.mp4", 1, 0), None);
    }

    #[test]
    fn temporary_paths_are_unique_hidden_siblings_that_keep_the_mp4_extension() {
        let first = temporary_path("/tmp/movie final.mp4", 42, 1).unwrap();
        let second = temporary_path("/tmp/movie final.mp4", 42, 2).unwrap();
        assert_eq!(first, "/tmp/.musializer-42-1.part.mp4");
        assert_eq!(second, "/tmp/.musializer-42-2.part.mp4");
        assert_ne!(first, second);
        // A sibling, so publication is a rename rather than a copy.
        assert!(first.starts_with("/tmp/"));
        assert!(first.ends_with(".mp4"), "FFmpeg infers the muxer from this");
        // No directory at all means the current directory.
        assert_eq!(
            temporary_path("movie.mp4", 7, 8).unwrap(),
            ".musializer-7-8.part.mp4"
        );
    }

    #[test]
    fn windows_paths_stay_in_the_destination_directory() {
        assert_eq!(
            temporary_path("C:\\Videos\\movie.mp4", 7, 3).unwrap(),
            "C:\\Videos\\.musializer-7-3.part.mp4"
        );
        assert_eq!(
            decoded_audio_path("C:\\Videos\\movie.mp4", 99, 3).unwrap(),
            "C:\\Videos\\.musializer-audio-99-3.wav"
        );
        // A mixed path takes the rightmost separator, whichever it is.
        assert_eq!(
            temporary_path("a/b\\c/d.mp4", 1, 1).unwrap(),
            "a/b\\c/.musializer-1-1.part.mp4"
        );
    }

    #[test]
    fn decoded_audio_paths_are_unique_siblings_of_the_output() {
        let first = decoded_audio_path("/tmp/final movie.mp4", 42, 1).unwrap();
        let second = decoded_audio_path("/tmp/final movie.mp4", 42, 2).unwrap();
        assert_eq!(first, "/tmp/.musializer-audio-42-1.wav");
        assert_ne!(first, second);
        // Beside the output, not beside the source audio.
        assert!(first.starts_with("/tmp/"));
        assert!(first.ends_with(".wav"));
    }

    /// A `u64` pair is 41 characters at most, which is why C's 65-byte identity
    /// buffer can never overflow and Rust does not need the check at all.
    #[test]
    fn extreme_identities_still_produce_a_path() {
        let path = temporary_path("/tmp/x.mp4", u64::MAX, u64::MAX).unwrap();
        assert_eq!(
            path,
            "/tmp/.musializer-18446744073709551615-18446744073709551615.part.mp4"
        );
        let identity = format!("{}-{}", u64::MAX, u64::MAX);
        assert_eq!(
            identity.len(),
            41,
            "the widest identity C's 65-byte buffer sees"
        );
    }

    #[test]
    fn finalize_deadlines_are_bounded_and_boundary_exact() {
        assert_eq!(finalize_grace_ms(true), 5_000);
        assert_eq!(finalize_grace_ms(false), 300_000);
        assert_eq!(wait_action(false, 4_999, true), WaitAction::Continue);
        assert_eq!(wait_action(false, 5_000, true), WaitAction::Timeout);
        assert_eq!(wait_action(false, 299_999, false), WaitAction::Continue);
        assert_eq!(wait_action(false, 300_000, false), WaitAction::Timeout);
        // An exited process is complete however long it took.
        assert_eq!(wait_action(true, u64::MAX, false), WaitAction::Complete);
        assert_eq!(wait_action(true, u64::MAX, true), WaitAction::Complete);
        assert_eq!(wait_action(true, 0, false), WaitAction::Complete);
    }

    /// **Differential against the frozen C, not against this implementation.**
    ///
    /// Produced by compiling `../musializer/src/render_export.c` unmodified into a
    /// scratch harness *outside both repositories*. A 60 s track at 44.1 kHz and
    /// 30 fps, which is the case where every rounding decision in the module is
    /// simultaneously non-trivial: the frame count is exact, the last cursor is
    /// not on a sample boundary, the window straddles two frames at both ends, and
    /// the duration text needs all nine digits.
    #[test]
    fn matches_the_c_oracle_on_a_real_transport() {
        assert_eq!(total_frames(2_646_000, 44_100, 30), Ok(1_800));
        assert_eq!(sample_cursor(1_799, 44_100, 30, 2_646_000), Ok(2_644_530));
        assert_eq!(window_frames(1_800, 30, 12.345, 3.21), Ok(370..467));
        assert_eq!(
            transport_duration_text(1_799, 30).unwrap(),
            "59.966666667",
            "the frame *before* the end of a 60 s track"
        );
        assert_eq!(transport_duration_text(1, 240).unwrap(), "0.004166667");
        assert_eq!(transport_duration_text(7, 60).unwrap(), "0.116666667");
    }

    /// The user-visible failure text, pinned so a refactor of the enum cannot
    /// silently reword a notice. These are C's strings verbatim
    /// (`render_export.c:105-115`).
    #[test]
    fn error_messages_match_the_oracles_string_table() {
        let cases = [
            (RenderExportError::Resolution, "invalid output resolution"),
            (RenderExportError::FrameRate, "invalid frame rate"),
            (RenderExportError::Quality, "invalid quality"),
            (RenderExportError::Supersample, "invalid supersampling"),
            (RenderExportError::Overflow, "integer overflow"),
            (RenderExportError::Path, "invalid output path"),
            (
                RenderExportError::OutputBufferTooSmall,
                "output buffer is too small",
            ),
            (
                RenderExportError::Window,
                "render window is outside the track or not a positive finite range",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
