//! One frame of each library clip, for Look's clip choices (F1c).
//!
//! A clip choice shows the file it is, not a section of the video, so its
//! picture is decoded rather than replayed: the clip's middle frame from its
//! own decoder worker (`runtime::clip::ClipWorker`, inside the process's
//! decoder budget), cut down to a chip's thumbnail on the CPU and kept as a
//! small texture. One clip at a time, never blocking a frame except under
//! `posters=settle`, and the worker goes as soon as its frame is in. A clip
//! the worker refuses (missing, changed, unreadable) keeps the worker's
//! words, which are the placeholder's, so the card can say what is wrong.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use musializer_core::project::clip::ClipRef;
use musializer_runtime::clip::{ClipPhase, ClipWorker, Expected};
use raylib::prelude::{Color, Image, RaylibHandle, RaylibTexture2D, RaylibThread, Texture2D};

use crate::clips::TrackClip;
use crate::workspace::Workspace;

/// The box a thumbnail is fitted into, keeping the clip's shape: two chip
/// heights of 16:9 at the most.
pub const THUMB_BOX: (u32, u32) = (96, 54);
/// How many thumbnails stay, across tracks. 96x54 RGBA is 20 KB.
const CAPACITY: usize = 128;
/// A worker that has not delivered by then is given up on, and says so.
const DEADLINE: Duration = Duration::from_secs(20);

/// What a clip choice draws.
pub enum ThumbView<'a> {
    /// The clip's middle frame, `size` pixels.
    Ready {
        texture: &'a Texture2D,
        size: (u32, u32),
    },
    /// Not decoded yet.
    Pending,
    /// The clip cannot be read, in the placeholder's words.
    Refused(String),
}

struct Thumb {
    clip: ClipRef,
    source: PathBuf,
    /// The picture and its size, or why there is none.
    outcome: Result<(Texture2D, (u32, u32)), String>,
    seen: u64,
}

struct Job {
    clip: ClipRef,
    source: PathBuf,
    name: String,
    frame: usize,
    worker: ClipWorker,
    started: Instant,
}

/// The thumbnails, their one running decode, and this frame's wants.
#[derive(Default)]
pub struct ClipThumbs {
    entries: Vec<Thumb>,
    job: Option<Job>,
    /// (track slot, clip) the card drew this frame.
    wanted: Vec<(usize, ClipRef)>,
    clock: u64,
    made: u64,
    refused: u64,
}

/// The frame a thumbnail shows: the middle one, past any fade in.
fn thumb_frame(clip: &TrackClip) -> usize {
    (clip.asset.frames / 2) as usize
}

/// A clip's frame size fitted into [`THUMB_BOX`], both edges at least 2.
#[must_use]
pub fn thumb_size(width: u32, height: u32) -> (u32, u32) {
    let (box_width, box_height) = (f64::from(THUMB_BOX.0), f64::from(THUMB_BOX.1));
    let aspect = f64::from(width.max(1)) / f64::from(height.max(1));
    let (w, h) = if aspect > box_width / box_height {
        (box_width, box_width / aspect)
    } else {
        (box_height * aspect, box_height)
    };
    ((w.round() as u32).max(2), (h.round() as u32).max(2))
}

/// `pixels` (RGBA, `width` x `height`, top row first) averaged down to
/// `size`: each output pixel the mean of up to 4x4 samples spread over its
/// footprint, so a thin line in the clip does not flicker in or out.
#[must_use]
pub fn downscale(pixels: &[u8], width: u32, height: u32, size: (u32, u32)) -> Vec<u8> {
    let (out_w, out_h) = size;
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
        return out;
    }
    const TAPS: u32 = 4;
    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sum = [0u32; 4];
            for ty in 0..TAPS {
                let fy =
                    (f64::from(oy) + (f64::from(ty) + 0.5) / f64::from(TAPS)) / f64::from(out_h);
                let sy = ((fy * f64::from(height)) as u32).min(height - 1);
                for tx in 0..TAPS {
                    let fx = (f64::from(ox) + (f64::from(tx) + 0.5) / f64::from(TAPS))
                        / f64::from(out_w);
                    let sx = ((fx * f64::from(width)) as u32).min(width - 1);
                    let at = (sy as usize * width as usize + sx as usize) * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(pixels[at + channel]);
                    }
                }
            }
            let at = (oy as usize * out_w as usize + ox as usize) * 4;
            for (channel, total) in sum.iter().enumerate() {
                out[at + channel] = ((total + TAPS * TAPS / 2) / (TAPS * TAPS)) as u8;
            }
            // Opaque: a decoded frame has no alpha worth keeping.
            out[at + 3] = 255;
        }
    }
    out
}

impl ClipThumbs {
    fn entry(&self, clip: ClipRef, source: &Path) -> Option<&Thumb> {
        self.entries
            .iter()
            .find(|entry| entry.clip == clip && entry.source == source)
    }

    /// The clips the card drew this frame, in its order.
    pub fn want(&mut self, wanted: Vec<(usize, ClipRef)>) {
        self.wanted = wanted;
    }

    /// What a choice for `clip` draws. A record the project could not read
    /// when it opened says so without ever being opened.
    pub fn view(&self, clip: &TrackClip) -> ThumbView<'_> {
        if let Some(error) = &clip.unusable {
            return ThumbView::Refused(format!("{}: {error}", clip.asset.name));
        }
        let Some(key) = clip.clip_ref() else {
            return ThumbView::Refused(format!("{}: no usable digest", clip.asset.name));
        };
        match self.entry(key, &clip.source).map(|entry| &entry.outcome) {
            Some(Ok((texture, size))) => ThumbView::Ready {
                texture,
                size: *size,
            },
            Some(Err(reason)) => ThumbView::Refused(reason.clone()),
            None => ThumbView::Pending,
        }
    }

    /// One step: finish the running decode if its frame is in, else start
    /// the first wanted clip that has none. `settle` waits for each one.
    pub fn step(
        &mut self,
        d: &mut RaylibHandle,
        thread: &RaylibThread,
        workspace: &Workspace,
        settle: bool,
    ) {
        self.clock += 1;
        for (slot, clip) in &self.wanted {
            let Some(entry) = workspace
                .get(*slot)
                .and_then(|track| crate::clips::find(&track.clips, *clip))
            else {
                continue;
            };
            let clock = self.clock;
            if let Some(thumb) = self
                .entries
                .iter_mut()
                .find(|thumb| thumb.clip == *clip && thumb.source == entry.source)
            {
                thumb.seen = clock;
            }
        }
        loop {
            if self.job.is_none() && !self.start(workspace) {
                return;
            }
            if !self.advance(d, thread, settle) || !settle {
                return;
            }
        }
    }

    /// Starts the first wanted clip with no thumbnail. False with none.
    fn start(&mut self, workspace: &Workspace) -> bool {
        let next = self.wanted.iter().find_map(|(slot, clip)| {
            let entry = workspace
                .get(*slot)
                .and_then(|track| crate::clips::find(&track.clips, *clip))?;
            (entry.unusable.is_none() && self.entry(*clip, &entry.source).is_none())
                .then(|| entry.clone())
        });
        let Some(entry) = next else {
            return false;
        };
        let Some(clip) = entry.clip_ref() else {
            return false;
        };
        self.job = Some(Job {
            clip,
            source: entry.source.clone(),
            name: entry.asset.name.clone(),
            frame: thumb_frame(&entry),
            worker: ClipWorker::spawn(
                entry.source.clone(),
                Expected {
                    sha256: entry.asset.sha256.clone(),
                    width: entry.asset.width,
                    height: entry.asset.height,
                    frames: entry.asset.frames,
                },
            ),
            started: Instant::now(),
        });
        true
    }

    /// Advances the running decode. True when it finished (either way).
    fn advance(&mut self, d: &mut RaylibHandle, thread: &RaylibThread, settle: bool) -> bool {
        let Some(job) = self.job.as_ref() else {
            return false;
        };
        let outcome = match job.worker.phase() {
            ClipPhase::Failed(error) => Some(Err(format!("{}: {error}", job.name))),
            ClipPhase::Starting if !settle => None,
            ClipPhase::Starting | ClipPhase::Ready(_) => {
                let frame = if settle {
                    job.worker.wait_for(job.frame, DEADLINE).ok()
                } else {
                    job.worker.request(job.frame);
                    job.worker
                        .latest()
                        .filter(|(index, _)| *index == job.frame)
                        .map(|(_, frame)| frame)
                };
                match (frame, job.worker.phase()) {
                    (_, ClipPhase::Failed(error)) => Some(Err(format!("{}: {error}", job.name))),
                    (Some(pixels), ClipPhase::Ready(probe)) => {
                        let size = thumb_size(probe.width, probe.height);
                        let small = downscale(&pixels, probe.width, probe.height, size);
                        let image =
                            Image::gen_image_color(size.0 as i32, size.1 as i32, Color::BLACK);
                        Some(match d.load_texture_from_image(thread, &image) {
                            Ok(mut texture) => match texture.update_texture(&small) {
                                Ok(()) => Ok((texture, size)),
                                Err(error) => {
                                    Err(format!("{}: upload refused: {error:?}", job.name))
                                }
                            },
                            Err(error) => Err(format!(
                                "{}: the GPU refused a thumbnail: {error:?}",
                                job.name
                            )),
                        })
                    }
                    _ if job.started.elapsed() > DEADLINE || settle => {
                        Some(Err(format!("{}: FFmpeg did not answer", job.name)))
                    }
                    _ => None,
                }
            }
        };
        let Some(outcome) = outcome else {
            return false;
        };
        let job = self.job.take().expect("checked above");
        if outcome.is_ok() {
            self.made += 1;
        } else {
            self.refused += 1;
        }
        if self.entries.len() >= CAPACITY {
            if let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.seen)
                .map(|(index, _)| index)
            {
                self.entries.swap_remove(oldest);
            }
        }
        self.entries.push(Thumb {
            clip: job.clip,
            source: job.source,
            outcome,
            seen: self.clock,
        });
        // The worker drops here: its decoder and frames go at once.
        true
    }

    /// `thumbs=made/refused/kept`, for the `posters:` line.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{}/{}/{}{}",
            self.made,
            self.refused,
            self.entries.len(),
            if self.job.is_some() { "+1" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_keeps_the_clips_shape_inside_its_box() {
        assert_eq!(thumb_size(1920, 1080), (96, 54));
        assert_eq!(thumb_size(360, 360), (54, 54));
        assert_eq!(thumb_size(180, 320), (30, 54));
        assert_eq!(thumb_size(4000, 10), (96, 2));
    }

    #[test]
    fn a_downscale_averages_its_footprint() {
        // 8x2: the left half black, the right half white; 2x1 out.
        let mut pixels = Vec::new();
        for _ in 0..2 {
            for x in 0..8 {
                let v = if x < 4 { 0 } else { 255 };
                pixels.extend_from_slice(&[v, v, v, 7]);
            }
        }
        assert_eq!(
            downscale(&pixels, 8, 2, (2, 1)),
            vec![0, 0, 0, 255, 255, 255, 255, 255]
        );
        // 1x1 out over both halves: the mean, rounded.
        assert_eq!(downscale(&pixels, 8, 2, (1, 1)), vec![128, 128, 128, 255]);
        // A short buffer draws black, never panics.
        assert_eq!(downscale(&pixels[..8], 8, 2, (1, 1)), vec![0, 0, 0, 0]);
    }
}
