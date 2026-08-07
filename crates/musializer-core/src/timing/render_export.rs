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
        let prefix = self.suggestion_prefix(audio_path, scene_name)?;
        Ok(format!(
            "{prefix}-musializer-{scene_name}-{}p{}.mp4",
            self.height, self.fps
        ))
    }

    /// The same suggestion for a **clip** export (UX0-C01).
    ///
    /// A separate name rather than a suffix on [`suggest_path`](Self::suggest_path),
    /// for one reason a user would notice: a clip and a full render of the same
    /// track and scene would otherwise propose the identical file name, and the
    /// second one silently replaces the first. The bounds are written as whole
    /// seconds, which is enough to tell two clips apart and short enough to read.
    ///
    /// # Errors
    /// As [`suggest_path`](Self::suggest_path), plus
    /// [`RenderExportError::Window`] for a non-finite or non-positive range.
    pub fn suggest_clip_path(
        &self,
        audio_path: &str,
        scene_name: &str,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<String, RenderExportError> {
        if !start_seconds.is_finite()
            || !end_seconds.is_finite()
            || start_seconds < 0.0
            || end_seconds <= start_seconds
        {
            return Err(RenderExportError::Window);
        }
        let prefix = self.suggestion_prefix(audio_path, scene_name)?;
        Ok(format!(
            "{prefix}-musializer-{scene_name}-{}p{}-clip-{}-{}.mp4",
            self.height,
            self.fps,
            timestamp_token(start_seconds),
            timestamp_token(end_seconds),
        ))
    }

    /// The suggestion for a **still frame** (UX0-C10).
    ///
    /// Same shape as the video's, so the two land beside each other and sort
    /// together, with the frame's own time in the name — a cover taken at the
    /// drop and one taken at the first chorus are different pictures and must
    /// not be the same file. The frame rate is deliberately absent: a still has
    /// no frame rate a viewer can see, and including it would suggest the file
    /// depends on a setting it does not.
    ///
    /// # Errors
    /// As [`suggest_path`](Self::suggest_path), plus
    /// [`RenderExportError::Window`] for a non-finite or negative time.
    pub fn suggest_still_path(
        &self,
        audio_path: &str,
        scene_name: &str,
        time_seconds: f64,
    ) -> Result<String, RenderExportError> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            return Err(RenderExportError::Window);
        }
        let prefix = self.suggestion_prefix(audio_path, scene_name)?;
        Ok(format!(
            "{prefix}-musializer-{scene_name}-{}p-still-{}.png",
            self.height,
            timestamp_token(time_seconds),
        ))
    }

    /// The shared half of the three suggestions: the source path with its own
    /// extension removed, once every name has been proven safe.
    fn suggestion_prefix<'a>(
        &self,
        audio_path: &'a str,
        scene_name: &str,
    ) -> Result<&'a str, RenderExportError> {
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
        Ok(&audio_path[..prefix_length])
    }
}

/// `MMmSSsMMM`, for a file name: no colon, no dot, and it sorts.
///
/// Milliseconds are kept because a still is a single frame and two frames 40 ms
/// apart are different pictures; truncating to the second would collide.
fn timestamp_token(seconds: f64) -> String {
    let total_milliseconds = (seconds.max(0.0) * 1000.0).round() as u64;
    let minutes = total_milliseconds / 60_000;
    let remainder = total_milliseconds % 60_000;
    format!(
        "{minutes:02}m{:02}s{:03}",
        remainder / 1000,
        remainder % 1000
    )
}

/// The seconds one video frame occupies at `fps`.
///
/// The smallest window the transport can express: [`window_frames`] gives a
/// sub-frame duration one whole frame anyway, so a clip shorter than this is a
/// clip of exactly one frame with a misleading readout.
#[must_use]
pub fn frame_seconds(fps: u32) -> f64 {
    if fps == 0 {
        return 0.0;
    }
    1.0 / f64::from(fps)
}

/// The scene clock a given export frame is drawn at
/// (`render_job.rs::scene_time`, `plug.c:8019`).
///
/// Here rather than only on the job because the still export (UX0-C10) draws a
/// frame without one, and a still drawn at a *different* time from the video
/// frame with the same index would be the whole feature quietly failing.
#[must_use]
pub fn frame_time_seconds(frame_index: u64, fps: u32) -> f64 {
    if fps == 0 {
        return 0.0;
    }
    frame_index as f64 / f64::from(fps)
}

/// The scene delta a given export frame advances by
/// (`render_job.rs::scene_delta`, `plug.c:8013-8014`).
///
/// Zero at frame zero and exactly `1/fps` after it — the property that makes an
/// export reproducible, and the reason a still cannot use a wall-clock delta
/// either.
#[must_use]
pub fn frame_delta_seconds(frame_index: u64, fps: u32) -> f32 {
    if frame_index == 0 || fps == 0 {
        return 0.0;
    }
    1.0 / fps as f32
}

/// The video frame a still publishes for a playhead time (UX0-C10).
///
/// **Not the oracle's**: the frozen C cannot export a frame at all. The rule is
/// the transport's own, so the still is a *video frame* rather than a picture
/// taken near one — it floors, exactly as [`window_frames`]'s start does, so the
/// still and a clip export starting at the same second publish the same frame
/// index, and it clamps to the last frame of the track rather than running past
/// the audio.
///
/// # Errors
/// [`RenderExportError::FrameRate`] for an `fps` outside `1..=240`;
/// [`RenderExportError::Window`] for a zero `total_frames` or a non-finite or
/// negative time.
pub fn still_frame_index(
    time_seconds: f64,
    fps: u32,
    total_frames: u64,
) -> Result<u64, RenderExportError> {
    if fps == 0 || fps > MAX_FPS {
        return Err(RenderExportError::FrameRate);
    }
    if total_frames == 0 || !time_seconds.is_finite() || time_seconds < 0.0 {
        return Err(RenderExportError::Window);
    }
    let position = time_seconds * f64::from(fps);
    // `total_frames` comes from a decodable audio length, so the comparison
    // stays inside f64's exact-integer range; the `+inf` case is excluded above.
    if position >= total_frames as f64 {
        return Ok(total_frames - 1);
    }
    Ok((position as u64).min(total_frames - 1))
}

/// A user-chosen render window, in seconds, on the current track (UX0-C01).
///
/// **Not the oracle's.** The frozen C renders whole tracks: `--render-window` is
/// this rewrite's own command-line flag over `render_export_window_frames`, and
/// nothing in the C's interface can ask for a clip. This type is the editable
/// state behind the export panel's CLIP row, kept here rather than in the panel
/// so the arithmetic that decides what an export covers is checkable without a
/// window.
///
/// Two invariants make it safe to hand straight to the render plan:
///
/// - it is only ever `enabled` with `start < end`, both finite and inside the
///   track, and at least one frame apart, because every mutator re-establishes
///   that; and
/// - [`window`](Self::window) re-clamps against the *current* duration anyway,
///   because a track can be replaced under a retained selection.
///
/// Setting one end past the other is **not** refused: it re-reads as an
/// open-ended selection — an IN past the OUT means "from here to the end", an
/// OUT before the IN means "from the start to here". Both are single gestures a
/// user makes on purpose, and a refusal would leave the control looking broken
/// at exactly the moment they are moving fastest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipSelection {
    enabled: bool,
    start_seconds: f64,
    end_seconds: f64,
}

impl Default for ClipSelection {
    fn default() -> Self {
        Self::full_track()
    }
}

impl ClipSelection {
    /// No clip: the export covers the whole track.
    #[must_use]
    pub fn full_track() -> Self {
        Self {
            enabled: false,
            start_seconds: 0.0,
            end_seconds: 0.0,
        }
    }

    /// The selection `--render-window START DURATION` describes.
    ///
    /// The command line and the panel are one state on purpose: an export
    /// started from the panel after `--render-window` was passed must cover the
    /// window the command line asked for, and a panel that showed "full track"
    /// while the flag was in force would be lying about what it is about to
    /// produce.
    #[must_use]
    pub fn from_window(window: Option<(f64, f64)>) -> Self {
        match window {
            Some((start, duration))
                if start.is_finite() && duration.is_finite() && start >= 0.0 && duration > 0.0 =>
            {
                Self {
                    enabled: true,
                    start_seconds: start,
                    end_seconds: start + duration,
                }
            }
            _ => Self::full_track(),
        }
    }

    #[must_use]
    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    /// The stored IN point. Meaningless while disabled, which is why every
    /// reader goes through [`window`](Self::window) instead.
    #[must_use]
    pub fn start_seconds(self) -> f64 {
        self.start_seconds
    }

    #[must_use]
    pub fn end_seconds(self) -> f64 {
        self.end_seconds
    }

    /// Back to the whole track.
    pub fn clear(&mut self) {
        *self = Self::full_track();
    }

    /// Moves the IN point to `seconds`, enabling the clip.
    ///
    /// A first touch selects from here **to the end of the track**, which is the
    /// meaning that needs no second click: "post from the drop onward" is one
    /// gesture. An IN at or past the current OUT re-opens the selection the same
    /// way rather than refusing.
    pub fn set_start(&mut self, seconds: f64, duration_seconds: f64, fps: u32) {
        let Some((floor, ceiling, quantum)) = self.bounds(duration_seconds, fps) else {
            return;
        };
        let start = seconds.clamp(floor, (ceiling - quantum).max(floor));
        let open_ended = !self.enabled || self.end_seconds < start + quantum;
        self.end_seconds = if open_ended {
            ceiling
        } else {
            self.end_seconds.min(ceiling)
        };
        self.start_seconds = start;
        self.enabled = true;
    }

    /// Moves the OUT point to `seconds`, enabling the clip.
    ///
    /// The mirror of [`set_start`](Self::set_start): a first touch, or an OUT at
    /// or before the current IN, selects from the **start of the track** to
    /// here.
    pub fn set_end(&mut self, seconds: f64, duration_seconds: f64, fps: u32) {
        let Some((floor, ceiling, quantum)) = self.bounds(duration_seconds, fps) else {
            return;
        };
        let end = seconds.clamp((floor + quantum).min(ceiling), ceiling);
        let open_ended = !self.enabled || self.start_seconds + quantum > end;
        self.start_seconds = if open_ended {
            floor
        } else {
            self.start_seconds
        };
        self.end_seconds = end;
        self.enabled = true;
    }

    /// `(start_seconds, duration_seconds)` for a [`RenderRequest`-style] window,
    /// or `None` for the whole track.
    ///
    /// Re-clamped against the live duration, because the track under a retained
    /// selection can change: a clip that outlived its track would otherwise be
    /// refused by `window_frames` at export start, which is a notice instead of
    /// a render.
    ///
    /// [`RenderRequest`-style]: crate::timing::render_export
    #[must_use]
    pub fn window(self, duration_seconds: f64, fps: u32) -> Option<(f64, f64)> {
        if !self.enabled {
            return None;
        }
        let (floor, ceiling, quantum) = self.bounds(duration_seconds, fps)?;
        let start = self
            .start_seconds
            .clamp(floor, (ceiling - quantum).max(floor));
        let end = self.end_seconds.clamp(start + quantum, ceiling);
        let length = end - start;
        (length > 0.0).then_some((start, length))
    }

    /// The frames this selection covers of a track of `duration_seconds`.
    ///
    /// The total is derived from the duration rather than from the decoded
    /// sample count, which is what a panel has: the two agree, because
    /// `total_frames` is `ceil(frame_count * fps / sample_rate)` and the
    /// duration *is* `frame_count / sample_rate`. It is a readout, and the
    /// render itself re-resolves against the decoded audio.
    #[must_use]
    pub fn frames(self, duration_seconds: f64, fps: u32) -> Option<Range<u64>> {
        let total = self.total_frames(duration_seconds, fps)?;
        match self.window(duration_seconds, fps) {
            None => Some(0..total),
            Some((start, length)) => window_frames(total, fps, start, length).ok(),
        }
    }

    /// The one-line state a report and a panel readout both need.
    #[must_use]
    pub fn describe(self, duration_seconds: f64, fps: u32) -> String {
        let Some((start, length)) = self.window(duration_seconds, fps) else {
            return match self.frames(duration_seconds, fps) {
                Some(frames) => format!("full track ({} frames)", frames.end - frames.start),
                None => "full track".to_owned(),
            };
        };
        let frames = self
            .frames(duration_seconds, fps)
            .map_or(0, |frames| frames.end - frames.start);
        format!(
            "clip in {start:.3} out {:.3} ({length:.3} s, {frames} frames)",
            start + length
        )
    }

    /// `(floor, ceiling, one frame)` for a usable track, or `None` when there is
    /// nothing to clip.
    fn bounds(self, duration_seconds: f64, fps: u32) -> Option<(f64, f64, f64)> {
        let quantum = frame_seconds(fps);
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || quantum <= 0.0 {
            return None;
        }
        Some((0.0, duration_seconds, quantum.min(duration_seconds)))
    }

    fn total_frames(self, duration_seconds: f64, fps: u32) -> Option<u64> {
        let (_, ceiling, _) = self.bounds(duration_seconds, fps)?;
        let total = (ceiling * f64::from(fps)).ceil();
        (total.is_finite() && total >= 1.0).then_some(total as u64)
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

    /// A still is a *video frame*, chosen by the same floor the window's start
    /// uses, so "export a still here" and "export a clip from here" publish the
    /// same picture (UX0-C10).
    #[test]
    fn a_still_picks_the_frame_a_clip_starting_there_would_draw() {
        let total = total_frames(2_646_000, 44_100, 30).unwrap(); // 60 s at 30 fps
        assert_eq!(total, 1_800);
        for time in [0.0, 0.033, 0.999, 12.345, 42.0, 59.999] {
            let still = still_frame_index(time, 30, total).unwrap();
            let clip = window_frames(total, 30, time, 1.0).unwrap();
            assert_eq!(still, clip.start, "at {time}s");
        }
        // Exactly on a frame boundary is that frame, not the one before it.
        assert_eq!(still_frame_index(1.0, 30, total), Ok(30));
        assert_eq!(still_frame_index(0.0, 30, total), Ok(0));
        // Past the end clamps to the last frame rather than running off the
        // audio: a playhead parked at the very end must still take a picture.
        assert_eq!(still_frame_index(60.0, 30, total), Ok(1_799));
        assert_eq!(still_frame_index(1.0e9, 30, total), Ok(1_799));
    }

    #[test]
    fn a_still_refuses_a_degenerate_transport() {
        assert_eq!(
            still_frame_index(1.0, 30, 0),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            still_frame_index(-0.001, 30, 100),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            still_frame_index(f64::NAN, 30, 100),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            still_frame_index(f64::INFINITY, 30, 100),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            still_frame_index(1.0, 0, 100),
            Err(RenderExportError::FrameRate)
        );
        assert_eq!(
            still_frame_index(1.0, MAX_FPS + 1, 100),
            Err(RenderExportError::FrameRate)
        );
    }

    /// The per-frame clock a still has to reproduce, since it draws one frame
    /// without a [`RenderJob`](crate) to ask.
    #[test]
    fn frame_clock_matches_the_jobs_own_definitions() {
        // `render_job.rs::scene_delta`: zero at frame zero, `1/fps` after it.
        assert_eq!(frame_delta_seconds(0, 30), 0.0);
        assert_eq!(frame_delta_seconds(1, 30), 1.0 / 30.0);
        assert_eq!(frame_delta_seconds(9_999, 60), 1.0 / 60.0);
        // `render_job.rs::scene_time`: index over fps, exactly.
        assert_eq!(frame_time_seconds(0, 30), 0.0);
        assert_eq!(frame_time_seconds(30, 30), 1.0);
        assert_eq!(frame_time_seconds(45, 30), 1.5);
        assert_eq!(frame_seconds(30), 1.0 / 30.0);
        // A degenerate rate cannot divide by zero anywhere.
        assert_eq!(frame_seconds(0), 0.0);
        assert_eq!(frame_time_seconds(10, 0), 0.0);
        assert_eq!(frame_delta_seconds(10, 0), 0.0);
    }

    /// The whole point of the CLIP row: two presses and the window is the one a
    /// user meant, in either order (UX0-C01).
    #[test]
    fn setting_either_end_from_the_playhead_selects_what_a_user_meant() {
        let (duration, fps) = (60.0, 30u32);

        // "From the drop onward": one press, and the tail is the rest of the track.
        let mut clip = ClipSelection::full_track();
        assert_eq!(clip.window(duration, fps), None);
        clip.set_start(12.0, duration, fps);
        assert_eq!(clip.window(duration, fps), Some((12.0, 48.0)));
        // Then the second press closes it.
        clip.set_end(42.0, duration, fps);
        assert_eq!(clip.window(duration, fps), Some((12.0, 30.0)));

        // The other order: OUT first selects from the start of the track.
        let mut reverse = ClipSelection::full_track();
        reverse.set_end(42.0, duration, fps);
        assert_eq!(reverse.window(duration, fps), Some((0.0, 42.0)));
        reverse.set_start(12.0, duration, fps);
        assert_eq!(reverse.window(duration, fps), Some((12.0, 30.0)));

        // Clearing is the whole track again, not a zero-length clip.
        reverse.clear();
        assert!(!reverse.is_enabled());
        assert_eq!(reverse.window(duration, fps), None);
    }

    /// Setting one end past the other re-opens the selection rather than
    /// refusing — the control must never look broken mid-gesture.
    #[test]
    fn crossing_the_other_end_re_reads_as_an_open_ended_selection() {
        let (duration, fps) = (60.0, 30u32);
        let mut clip = ClipSelection::full_track();
        clip.set_start(10.0, duration, fps);
        clip.set_end(20.0, duration, fps);
        assert_eq!(clip.window(duration, fps), Some((10.0, 10.0)));

        // An IN past the OUT: "from here to the end".
        clip.set_start(30.0, duration, fps);
        assert_eq!(clip.window(duration, fps), Some((30.0, 30.0)));

        // An OUT before the IN: "from the start to here".
        clip.set_end(5.0, duration, fps);
        assert_eq!(clip.window(duration, fps), Some((0.0, 5.0)));

        // Exactly on the other end is a crossing too: a zero-length window is
        // the one thing the transport cannot render.
        let mut touching = ClipSelection::full_track();
        touching.set_start(10.0, duration, fps);
        touching.set_end(20.0, duration, fps);
        touching.set_start(20.0, duration, fps);
        assert_eq!(touching.window(duration, fps), Some((20.0, 40.0)));
    }

    /// Every reachable selection is a window `window_frames` accepts. This is
    /// the property that keeps a clip export from failing at start with a
    /// notice, which is the failure a user cannot act on.
    #[test]
    fn every_edited_selection_is_a_window_the_transport_accepts() {
        let fps = 30u32;
        for duration in [0.04f64, 1.0, 8.0, 60.0, 3_600.0] {
            let total = (duration * f64::from(fps)).ceil() as u64;
            for start in [-5.0, 0.0, 0.001, duration / 3.0, duration, duration + 10.0] {
                for end in [-1.0, 0.0, duration / 2.0, duration, duration * 2.0] {
                    let mut clip = ClipSelection::full_track();
                    clip.set_start(start, duration, fps);
                    clip.set_end(end, duration, fps);
                    let window = clip
                        .window(duration, fps)
                        .expect("an enabled selection always yields a window");
                    assert!(window.0 >= 0.0 && window.1 > 0.0, "{window:?}");
                    let frames = window_frames(total, fps, window.0, window.1)
                        .unwrap_or_else(|error| panic!("{window:?} of {duration}s: {error}"));
                    assert!(frames.start < frames.end);
                    assert!(frames.end <= total);
                }
            }
        }
    }

    /// A track that changes under a retained selection must not produce a
    /// window outside it — the panel keeps the clip when a shorter track becomes
    /// current, and `window` is the only thing standing between that and an
    /// export that refuses to start.
    #[test]
    fn a_selection_is_re_clamped_against_the_live_duration() {
        let mut clip = ClipSelection::full_track();
        clip.set_start(40.0, 60.0, 30);
        clip.set_end(55.0, 60.0, 30);
        assert_eq!(clip.window(60.0, 30), Some((40.0, 15.0)));
        // The same selection against a 10 s track: one frame at the last
        // position it can still occupy, rather than a refusal.
        let (start, length) = clip.window(10.0, 30).expect("still renderable");
        assert!((start + length) <= 10.0 + f64::EPSILON, "{start} {length}");
        assert!(length > 0.0);
        // No track at all is no window, not a panic.
        assert_eq!(clip.window(0.0, 30), None);
        assert_eq!(clip.window(f64::NAN, 30), None);
        assert_eq!(clip.window(60.0, 0), None);
    }

    /// The command line and the panel are one state (UX0-C01).
    #[test]
    fn a_render_window_flag_becomes_the_panels_clip() {
        let clip = ClipSelection::from_window(Some((5.0, 2.5)));
        assert!(clip.is_enabled());
        assert_eq!(clip.window(60.0, 30), Some((5.0, 2.5)));
        assert_eq!(
            ClipSelection::from_window(None),
            ClipSelection::full_track()
        );
        // A flag the CLI would itself have refused cannot enable a clip here.
        for window in [(0.0, 0.0), (-1.0, 5.0), (0.0, -5.0), (f64::NAN, 1.0)] {
            assert!(
                !ClipSelection::from_window(Some(window)).is_enabled(),
                "{window:?}"
            );
        }
    }

    #[test]
    fn the_clip_readout_counts_the_frames_it_will_render() {
        let (duration, fps) = (8.0, 30u32);
        let full = ClipSelection::full_track();
        assert_eq!(full.frames(duration, fps), Some(0..240));
        assert_eq!(full.describe(duration, fps), "full track (240 frames)");

        let mut clip = ClipSelection::full_track();
        clip.set_start(2.0, duration, fps);
        clip.set_end(5.0, duration, fps);
        assert_eq!(clip.frames(duration, fps), Some(60..150));
        assert_eq!(
            clip.describe(duration, fps),
            "clip in 2.000 out 5.000 (3.000 s, 90 frames)"
        );
        // No track: a readout rather than a panic or a lie about frames.
        assert_eq!(full.describe(0.0, fps), "full track");
    }

    /// A clip and a still must not propose the file name a full render does,
    /// or the second export silently replaces the first.
    #[test]
    fn clip_and_still_suggestions_are_distinct_siblings() {
        let config = RenderExportConfig::default();
        let full = config.suggest_path("/music/song.mp3", "spectrum").unwrap();
        let clip = config
            .suggest_clip_path("/music/song.mp3", "spectrum", 72.5, 102.0)
            .unwrap();
        let still = config
            .suggest_still_path("/music/song.mp3", "spectrum", 72.5)
            .unwrap();
        assert_eq!(full, "/music/song-musializer-spectrum-1080p30.mp4");
        assert_eq!(
            clip,
            "/music/song-musializer-spectrum-1080p30-clip-01m12s500-01m42s000.mp4"
        );
        assert_eq!(
            still,
            "/music/song-musializer-spectrum-1080p-still-01m12s500.png"
        );
        assert_ne!(full, clip);
        assert!(still.ends_with(".png"), "a still is not a video");

        // The same refusals as the video suggestion, plus the range's own.
        assert_eq!(
            config.suggest_still_path("", "spectrum", 1.0),
            Err(RenderExportError::Path)
        );
        assert_eq!(
            config.suggest_clip_path("/a/b.wav", "bad name", 0.0, 1.0),
            Err(RenderExportError::Path)
        );
        assert_eq!(
            config.suggest_clip_path("/a/b.wav", "ok", 5.0, 5.0),
            Err(RenderExportError::Window)
        );
        assert_eq!(
            config.suggest_still_path("/a/b.wav", "ok", -1.0),
            Err(RenderExportError::Window)
        );
        // The extension rule is the video suggestion's, because it is the same
        // code: a dot in a directory cannot truncate the path.
        assert_eq!(
            config.suggest_still_path("/a.b/song", "x", 0.0).unwrap(),
            "/a.b/song-musializer-x-1080p-still-00m00s000.png"
        );
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
