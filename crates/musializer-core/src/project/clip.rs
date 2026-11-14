//! Video clips as scene material (post-legacy, VG's first half, 2026-11-14).
//!
//! A clip is a video file a section of the video plays: generated elsewhere,
//! shot, or exported from this application. This module is the pure half — the
//! asset a `.musi` records, where a section places it, and the arithmetic that
//! says which of the clip's frames a video frame shows. Decoding is FFmpeg's,
//! in `musializer-runtime::clip`; drawing is the `clip` scene's, in the app.
//!
//! **Which frame is exact arithmetic on time.** A section says where in the
//! clip it starts (`from`) and what happens when it outlasts the clip (`after`:
//! hold the last frame, loop, or bounce). The clip time of a video frame at
//! track time `t` is `from + (t - origin)` folded by `after`, and the frame
//! shown is the last one the clip presents at or before that time, from the
//! frame times its probe read ([`ClipTiming`]). No wall clock enters, so the
//! same project exports the same frames every time, and a still at `t` is the
//! export's frame at `t`, because both evaluate this function at the same `t`.
//!
//! **A section continues the one before it** when both play the same clip and
//! the later one does not say `from`: two sections of the same scene keep the
//! scene's running state and only change its tuning, and a clip's running state
//! is its clock. A `from` restarts it.

use crate::project::sha256;

/// The most clips one project holds.
pub const MAX_CLIPS: usize = 64;
/// The longest clip, in seconds. Ten minutes is longer than any generated
/// clip (30 s, `docs/VIDEO_GENERATION.md`) by far and short of a whole film.
pub const MAX_CLIP_SECONDS: f64 = 600.0;
/// The most frames a clip may have: ten minutes at 60 fps. Bounds the frame
/// table a probe builds before anything allocates for it.
pub const MAX_CLIP_FRAMES: u32 = 36_000;
/// The largest side and the most pixels of a clip's frame: an 8K UHD frame, in
/// either orientation. One decoded RGBA frame is then at most 132 MB.
pub const MAX_CLIP_SIDE: u32 = 7680;
pub const MAX_CLIP_PIXELS: u64 = 7680 * 4320;
pub const MIN_CLIP_SIDE: u32 = 2;
/// The largest clip file.
pub const MAX_CLIP_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// A clip's display name: its file name when it was imported.
pub const CLIP_NAME_MAX_BYTES: usize = 128;
/// The resolution a frame time is compared at. Frame times come from integer
/// timestamps over a rational time base and the query from `k / fps`; both are
/// rounded through `f64`, so an exact tie can land either side by a few ulps.
/// A nanosecond is below any frame period by six orders of magnitude.
pub const TIME_EPSILON: f64 = 1.0e-9;

/// A clip, by the SHA-256 of its bytes. Copy, so a section can hold one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClipRef([u8; 32]);

impl ClipRef {
    /// From a lowercase 64-hex digest; `None` for anything else.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if !sha256::is_hex_digest(text) {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The first eight hex digits, for a report line.
    #[must_use]
    pub fn short(self) -> String {
        self.to_hex()[..8].to_owned()
    }
}

/// What a section does when it outlasts its clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipOverrun {
    /// The last frame stays. The default, and never written.
    #[default]
    Hold,
    /// From the start again.
    Loop,
    /// Backwards to the start, then forwards again.
    Bounce,
}

impl ClipOverrun {
    pub const ALL: [Self; 3] = [Self::Hold, Self::Loop, Self::Bounce];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Loop => "loop",
            Self::Bounce => "bounce",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// How a clip fits the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipFit {
    /// Fills the frame, cropping the clip's long side. The default, and never
    /// written: a music video has no letterbox unless someone asks for one.
    #[default]
    Cover,
    /// The whole clip, with black where its shape and the frame's differ.
    Contain,
}

impl ClipFit {
    pub const ALL: [Self; 2] = [Self::Cover, Self::Contain];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// Where a section places a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlacement {
    pub clip: ClipRef,
    /// Seconds into the clip the section starts at. `None` continues the
    /// section before, when it played the same clip, and is 0 otherwise.
    pub from_seconds: Option<f64>,
    pub after: ClipOverrun,
    pub fit: ClipFit,
}

impl ClipPlacement {
    #[must_use]
    pub fn new(clip: ClipRef) -> Self {
        Self {
            clip,
            from_seconds: None,
            after: ClipOverrun::Hold,
            fit: ClipFit::Cover,
        }
    }

    /// Whether the placement's own numbers are ones a project may hold. That
    /// `from` lies inside the clip is checked against the clip's asset.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.from_seconds
            .is_none_or(|from| from.is_finite() && (0.0..MAX_CLIP_SECONDS).contains(&from))
    }
}

/// A clip the project holds (`clips[]` in the `.musi`): where its bytes are,
/// their digest, and what its probe said, so a reopen on a machine whose
/// FFmpeg reads the file differently says so instead of drawing other frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipAsset {
    /// Project-relative inside the bundle once saved.
    pub path: String,
    pub sha256: String,
    /// The file name it was imported under, for people: placeholders, the
    /// lane, the report.
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub duration_seconds: f64,
}

impl ClipAsset {
    #[must_use]
    pub fn clip_ref(&self) -> Option<ClipRef> {
        ClipRef::from_hex(&self.sha256)
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.path.is_empty()
            && self.path.len() <= crate::project::model::capacity::PATH
            && self.clip_ref().is_some()
            && !self.name.is_empty()
            && self.name.len() <= CLIP_NAME_MAX_BYTES
            && !self.name.contains('\0')
            && size_is_valid(self.width, self.height)
            && (1..=MAX_CLIP_FRAMES).contains(&self.frames)
            && self.duration_seconds.is_finite()
            && self.duration_seconds > 0.0
            && self.duration_seconds <= MAX_CLIP_SECONDS
    }
}

/// Whether a frame size is one a clip may have.
#[must_use]
pub fn size_is_valid(width: u32, height: u32) -> bool {
    (MIN_CLIP_SIDE..=MAX_CLIP_SIDE).contains(&width)
        && (MIN_CLIP_SIDE..=MAX_CLIP_SIDE).contains(&height)
        && u64::from(width) * u64::from(height) <= MAX_CLIP_PIXELS
}

/// When each of a clip's frames is presented, from its probe: seconds from the
/// first frame, strictly increasing, and where the last one ends.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipTiming {
    times: Vec<f64>,
    duration: f64,
}

/// Why a probe's frame times are not a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TimingError {
    #[error("it has no video frames")]
    Empty,
    #[error("it has more than {MAX_CLIP_FRAMES} frames")]
    TooManyFrames,
    #[error("two of its frames share a timestamp, or one is not a number")]
    Timestamps,
    #[error("it is longer than {MAX_CLIP_SECONDS} seconds")]
    TooLong,
}

impl ClipTiming {
    /// From presentation timestamps in a stream's integer time base, in any
    /// order (a probe lists packets in decode order). `fallback_frame` is the
    /// last frame's duration when the clip has only one frame.
    ///
    /// # Errors
    ///
    /// No frames, too many, a repeated or non-monotone timestamp after sorting,
    /// or longer than [`MAX_CLIP_SECONDS`].
    pub fn from_pts(
        pts: &[i64],
        time_base: (i64, i64),
        fallback_frame: f64,
    ) -> Result<Self, TimingError> {
        if pts.is_empty() {
            return Err(TimingError::Empty);
        }
        if pts.len() > MAX_CLIP_FRAMES as usize {
            return Err(TimingError::TooManyFrames);
        }
        let (num, den) = time_base;
        if num <= 0 || den <= 0 {
            return Err(TimingError::Timestamps);
        }
        let mut sorted = pts.to_vec();
        sorted.sort_unstable();
        let first = sorted[0];
        let mut times = Vec::with_capacity(sorted.len());
        for (index, &value) in sorted.iter().enumerate() {
            if index > 0 && value == sorted[index - 1] {
                return Err(TimingError::Timestamps);
            }
            // Exact in i128, then one rounding to f64.
            let ticks = i128::from(value) - i128::from(first);
            let seconds = (ticks * i128::from(num)) as f64 / den as f64;
            times.push(seconds);
        }
        let last_gap = if times.len() >= 2 {
            times[times.len() - 1] - times[times.len() - 2]
        } else {
            fallback_frame
        };
        if !last_gap.is_finite() || last_gap <= 0.0 {
            return Err(TimingError::Timestamps);
        }
        let duration = times[times.len() - 1] + last_gap;
        if duration > MAX_CLIP_SECONDS {
            return Err(TimingError::TooLong);
        }
        Ok(Self { times, duration })
    }

    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.times.len()
    }

    /// Where the last frame ends: the clip's length.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// When frame `index` is presented.
    #[must_use]
    pub fn time_of(&self, index: usize) -> f64 {
        self.times[index.min(self.times.len() - 1)]
    }

    /// The frame on screen at clip time `seconds`: the last one presented at
    /// or before it. Before the first frame is the first; past the end is the
    /// last.
    #[must_use]
    pub fn frame_at(&self, seconds: f64) -> usize {
        if !seconds.is_finite() || seconds <= 0.0 {
            return 0;
        }
        let probe = seconds + TIME_EPSILON;
        self.times
            .partition_point(|&time| time <= probe)
            .saturating_sub(1)
    }

    /// Where a decoder told to start at a time must be told to start so that
    /// frame `index` is the first it returns: halfway back to the frame
    /// before. FFmpeg's accurate input seek drops every frame presented before
    /// the position it is given (measured on FFmpeg 8.0: a position anywhere in
    /// `(t[k-1], t[k]]` returns `k` first), so the midpoint is the position
    /// furthest from both edges.
    #[must_use]
    pub fn seek_seconds(&self, index: usize) -> f64 {
        let index = index.min(self.times.len() - 1);
        if index == 0 {
            return 0.0;
        }
        (self.times[index - 1] + self.times[index]) * 0.5
    }

    /// The clip time a section's local time folds to, by `after`.
    #[must_use]
    pub fn fold(&self, seconds: f64, after: ClipOverrun) -> f64 {
        let length = self.duration;
        if !seconds.is_finite() || seconds <= 0.0 {
            return 0.0;
        }
        match after {
            ClipOverrun::Hold => seconds.min(length),
            ClipOverrun::Loop => seconds.rem_euclid(length),
            ClipOverrun::Bounce => {
                let phase = seconds.rem_euclid(2.0 * length);
                if phase >= length {
                    2.0 * length - phase
                } else {
                    phase
                }
            }
        }
    }
}

/// The clip a frame shows, resolved from the plan or the track: which clip,
/// how it fits, and where its clock started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipMoment {
    pub clip: ClipRef,
    pub after: ClipOverrun,
    pub fit: ClipFit,
    /// The clip time at `origin_seconds`.
    pub from_seconds: f64,
    /// The track time the clip's clock is counted from.
    pub origin_seconds: f64,
}

impl ClipMoment {
    /// The clip from track time zero: the base scene's, with no plan.
    #[must_use]
    pub fn from_start(placement: ClipPlacement) -> Self {
        Self {
            clip: placement.clip,
            after: placement.after,
            fit: placement.fit,
            from_seconds: placement.from_seconds.unwrap_or(0.0),
            origin_seconds: 0.0,
        }
    }

    /// Clip time at track time `time_seconds`, before folding.
    #[must_use]
    pub fn local_seconds(&self, time_seconds: f64) -> f64 {
        self.from_seconds + (time_seconds - self.origin_seconds).max(0.0)
    }

    /// The frame this moment shows at `time_seconds`.
    #[must_use]
    pub fn frame(&self, timing: &ClipTiming, time_seconds: f64) -> usize {
        timing.frame_at(timing.fold(self.local_seconds(time_seconds), self.after))
    }
}

/// Where a clip of `clip` size lands in a frame of `frame` size: the part of
/// the clip drawn (`[x, y, w, h]` in clip pixels) and where it goes (in frame
/// pixels, from the frame's corner).
#[must_use]
pub fn fit_rects(fit: ClipFit, clip: [f32; 2], frame: [f32; 2]) -> ([f32; 4], [f32; 4]) {
    let (cw, ch) = (clip[0].max(1.0), clip[1].max(1.0));
    let (fw, fh) = (frame[0].max(1.0), frame[1].max(1.0));
    let clip_aspect = cw / ch;
    let frame_aspect = fw / fh;
    match fit {
        ClipFit::Cover => {
            // Crop the clip to the frame's shape, centred.
            let source = if clip_aspect > frame_aspect {
                let width = ch * frame_aspect;
                [(cw - width) * 0.5, 0.0, width, ch]
            } else {
                let height = cw / frame_aspect;
                [0.0, (ch - height) * 0.5, cw, height]
            };
            (source, [0.0, 0.0, fw, fh])
        }
        ClipFit::Contain => {
            let dest = if clip_aspect > frame_aspect {
                let height = fw / clip_aspect;
                [0.0, (fh - height) * 0.5, fw, height]
            } else {
                let width = fh * clip_aspect;
                [(fw - width) * 0.5, 0.0, width, fh]
            };
            ([0.0, 0.0, cw, ch], dest)
        }
    }
}

/// Whether a clip library is one a project may hold: each asset valid, no
/// digest twice.
#[must_use]
pub fn library_is_valid(clips: &[ClipAsset]) -> bool {
    if clips.len() > MAX_CLIPS || !clips.iter().all(ClipAsset::is_valid) {
        return false;
    }
    let mut seen: Vec<ClipRef> = clips.iter().filter_map(ClipAsset::clip_ref).collect();
    seen.sort_unstable();
    seen.windows(2).all(|pair| pair[0] != pair[1])
}

/// The asset a placement names, in a library.
#[must_use]
pub fn find(clips: &[ClipAsset], clip: ClipRef) -> Option<&ClipAsset> {
    clips.iter().find(|asset| asset.clip_ref() == Some(clip))
}

/// Whether a placement fits the library: its clip is there and `from` lies
/// inside it.
#[must_use]
pub fn placement_fits(clips: &[ClipAsset], placement: &ClipPlacement) -> bool {
    placement.is_valid()
        && find(clips, placement.clip).is_some_and(|asset| {
            placement
                .from_seconds
                .is_none_or(|from| from < asset.duration_seconds)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: &str) -> ClipRef {
        ClipRef::from_hex(&sha256::digest_hex(seed.as_bytes())).unwrap()
    }

    /// 23.976 fps: ticks of 1001 at 1/24000, the rate a check cannot get right
    /// by multiplying by 24.
    fn ntsc(frames: usize) -> ClipTiming {
        let pts: Vec<i64> = (0..frames as i64).map(|k| k * 1001).collect();
        ClipTiming::from_pts(&pts, (1, 24000), 1.0 / 24.0).unwrap()
    }

    #[test]
    fn a_digest_round_trips_and_nothing_else_is_one() {
        let clip = digest("x");
        assert_eq!(ClipRef::from_hex(&clip.to_hex()), Some(clip));
        assert_eq!(clip.short().len(), 8);
        assert_eq!(ClipRef::from_hex("abc"), None);
        assert_eq!(ClipRef::from_hex(&"A".repeat(64)), None);
    }

    #[test]
    fn frame_times_come_from_the_time_base_exactly() {
        let timing = ntsc(288);
        assert_eq!(timing.frame_count(), 288);
        // Frame k is presented at k * 1001 / 24000 seconds.
        assert_eq!(timing.time_of(100), 100.0 * 1001.0 / 24000.0);
        assert!((timing.duration() - 288.0 * 1001.0 / 24000.0).abs() < 1.0e-12);
        // At a 30 fps export, video frame n is at n / 30: n = 30 (1.0 s) is
        // past frame 23 (0.959 s) and before 24 (1.001 s).
        assert_eq!(timing.frame_at(30.0 / 30.0), 23);
        // Exactly on a frame's time is that frame, though the two sides were
        // rounded differently.
        let on = 24.0 * 1001.0 / 24000.0;
        assert_eq!(timing.frame_at(on), 24);
        assert_eq!(timing.frame_at(on - 1.0e-6), 23);
        assert_eq!(timing.frame_at(-1.0), 0);
        assert_eq!(timing.frame_at(1.0e6), 287);
    }

    #[test]
    fn decode_order_and_a_late_start_are_normalised() {
        // B-frame order and a stream that starts at tick 2002.
        let timing =
            ClipTiming::from_pts(&[2002, 5005, 3003, 4004], (1, 24000), 1.0 / 24.0).unwrap();
        assert_eq!(timing.time_of(0), 0.0);
        assert_eq!(timing.time_of(3), 3003.0 / 24000.0);
        assert_eq!(
            ClipTiming::from_pts(&[0, 0], (1, 30), 1.0),
            Err(TimingError::Timestamps)
        );
        assert_eq!(
            ClipTiming::from_pts(&[], (1, 30), 1.0),
            Err(TimingError::Empty)
        );
        let long: Vec<i64> = (0..=(MAX_CLIP_FRAMES as i64)).collect();
        assert_eq!(
            ClipTiming::from_pts(&long, (1, 60), 1.0),
            Err(TimingError::TooManyFrames)
        );
        assert_eq!(
            ClipTiming::from_pts(&[0, 601], (1, 1), 1.0),
            Err(TimingError::TooLong)
        );
    }

    #[test]
    fn a_variable_rate_is_read_frame_by_frame() {
        // Sixty frames at 30 fps, then thirty at 15.
        let mut pts: Vec<i64> = (0..60).map(|k| k * 2).collect();
        pts.extend((0..30).map(|k| 120 + k * 4));
        let timing = ClipTiming::from_pts(&pts, (1, 60), 1.0 / 30.0).unwrap();
        assert_eq!(timing.frame_at(1.99), 59);
        assert_eq!(timing.frame_at(2.0), 60);
        assert_eq!(timing.frame_at(2.0 + 1.0 / 15.0), 61);
        assert_eq!(timing.frame_at(2.0 + 1.5 / 15.0), 61);
        assert!((timing.duration() - 4.0).abs() < 1.0e-12);
    }

    #[test]
    fn a_seek_lands_between_a_frame_and_the_one_before() {
        let timing = ntsc(10);
        assert_eq!(timing.seek_seconds(0), 0.0);
        let k = 5;
        let seek = timing.seek_seconds(k);
        assert!(seek > timing.time_of(k - 1) && seek < timing.time_of(k));
        assert_eq!(
            timing.frame_at(seek),
            k - 1,
            "the seek is not itself frame k"
        );
    }

    #[test]
    fn hold_loop_and_bounce_fold_at_the_boundary() {
        // Ten frames at 10 fps: a one-second clip.
        let pts: Vec<i64> = (0..10).collect();
        let timing = ClipTiming::from_pts(&pts, (1, 10), 0.1).unwrap();
        let at = |seconds: f64, after| timing.frame_at(timing.fold(seconds, after));
        // Hold: the last frame stays.
        assert_eq!(at(0.95, ClipOverrun::Hold), 9);
        assert_eq!(at(1.0, ClipOverrun::Hold), 9);
        assert_eq!(at(7.3, ClipOverrun::Hold), 9);
        // Loop: frame 0 again exactly at the length.
        assert_eq!(at(0.999, ClipOverrun::Loop), 9);
        assert_eq!(at(1.0, ClipOverrun::Loop), 0);
        assert_eq!(at(1.25, ClipOverrun::Loop), 2);
        // Bounce: the last frame across the turn, then backwards.
        assert_eq!(at(0.95, ClipOverrun::Bounce), 9);
        assert_eq!(at(1.05, ClipOverrun::Bounce), 9);
        assert_eq!(at(1.15, ClipOverrun::Bounce), 8);
        assert_eq!(at(1.95, ClipOverrun::Bounce), 0);
        assert_eq!(at(2.05, ClipOverrun::Bounce), 0);
        assert_eq!(at(2.15, ClipOverrun::Bounce), 1);
    }

    #[test]
    fn a_moment_counts_from_its_origin_and_from() {
        let pts: Vec<i64> = (0..10).collect();
        let timing = ClipTiming::from_pts(&pts, (1, 10), 0.1).unwrap();
        let moment = ClipMoment {
            clip: digest("c"),
            after: ClipOverrun::Hold,
            fit: ClipFit::Cover,
            from_seconds: 0.3,
            origin_seconds: 12.0,
        };
        assert_eq!(moment.frame(&timing, 12.0), 3);
        assert_eq!(moment.frame(&timing, 12.45), 7);
        // Before the origin (a transition's lead-in never asks, but a seek may).
        assert_eq!(moment.frame(&timing, 11.0), 3);
    }

    #[test]
    fn cover_crops_and_contain_letterboxes() {
        let near = |got: [f32; 4], want: [f32; 4]| {
            assert!(
                got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0e-3),
                "{got:?} != {want:?}"
            );
        };
        // A square clip in a 16:9 frame.
        let (source, dest) = fit_rects(ClipFit::Cover, [1000.0, 1000.0], [1920.0, 1080.0]);
        near(dest, [0.0, 0.0, 1920.0, 1080.0]);
        near(source, [0.0, 218.75, 1000.0, 562.5]);
        let (source, dest) = fit_rects(ClipFit::Contain, [1000.0, 1000.0], [1920.0, 1080.0]);
        near(source, [0.0, 0.0, 1000.0, 1000.0]);
        near(dest, [420.0, 0.0, 1080.0, 1080.0]);
        // A 16:9 clip in a 9:16 frame.
        let (source, _) = fit_rects(ClipFit::Cover, [1920.0, 1080.0], [1080.0, 1920.0]);
        near(source, [656.25, 0.0, 607.5, 1080.0]);
        let (_, dest) = fit_rects(ClipFit::Contain, [1920.0, 1080.0], [1080.0, 1920.0]);
        near(dest, [0.0, 656.25, 1080.0, 607.5]);
    }

    fn asset(seed: &str) -> ClipAsset {
        ClipAsset {
            path: format!("show.assets/clips/{}.mp4", digest(seed).to_hex()),
            sha256: digest(seed).to_hex(),
            name: "take.mp4".into(),
            width: 1920,
            height: 1080,
            frames: 300,
            duration_seconds: 10.0,
        }
    }

    #[test]
    fn an_asset_is_bounded_field_by_field() {
        assert!(asset("a").is_valid());
        type Breaker = fn(&mut ClipAsset);
        let breakers: [(&str, Breaker); 9] = [
            ("no path", |a| a.path.clear()),
            ("bad digest", |a| a.sha256 = "nope".into()),
            ("no name", |a| a.name.clear()),
            ("long name", |a| a.name = "n".repeat(129)),
            ("too wide", |a| a.width = 7681),
            ("too many pixels", |a| (a.width, a.height) = (7680, 7680)),
            ("one pixel", |a| a.height = 1),
            ("no frames", |a| a.frames = 0),
            ("too long", |a| a.duration_seconds = 601.0),
        ];
        for (name, break_it) in breakers {
            let mut candidate = asset("a");
            break_it(&mut candidate);
            assert!(!candidate.is_valid(), "accepted {name}");
        }
        assert!(size_is_valid(4320, 7680), "portrait 8K");
    }

    #[test]
    fn a_library_holds_each_clip_once_and_a_placement_must_fit_it() {
        let library = vec![asset("a"), asset("b")];
        assert!(library_is_valid(&library));
        assert!(!library_is_valid(&[asset("a"), asset("a")]));
        let mut placement = ClipPlacement::new(digest("a"));
        assert!(placement_fits(&library, &placement));
        placement.from_seconds = Some(9.99);
        assert!(placement_fits(&library, &placement));
        placement.from_seconds = Some(10.0);
        assert!(!placement_fits(&library, &placement), "from past the end");
        placement.from_seconds = Some(f64::NAN);
        assert!(!placement_fits(&library, &placement));
        assert!(!placement_fits(&library, &ClipPlacement::new(digest("z"))));
    }

    #[test]
    fn names_round_trip() {
        for value in ClipOverrun::ALL {
            assert_eq!(ClipOverrun::from_name(value.name()), Some(value));
        }
        for value in ClipFit::ALL {
            assert_eq!(ClipFit::from_name(value.name()), Some(value));
        }
        assert_eq!(ClipOverrun::from_name("pingpong"), None);
    }
}
