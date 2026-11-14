//! Video Clip's drawing half (VG, 2026-11-14): the clip frame a video frame
//! shows, fitted to the frame, with the music's touch on it.
//!
//! Which frame is `core::project::clip` arithmetic on the frame's time, so a
//! still and the export's frame at the same time are the same frame. How it
//! gets here depends on who draws ([`ClipRole`]):
//!
//! - an **export**, a still, a contact sheet and a **poster** wait for exactly
//!   that frame (`ClipWorker::wait_for`), each role with a worker of its own
//!   so a poster rendered between two preview frames does not drag the
//!   preview's decoder back and forth;
//! - the **preview** asks and draws the nearest frame it has, and the
//!   `clip:` report line counts every frame it drew that was not the one asked
//!   for, so "the preview showed another frame while scrubbing" is a number.
//!
//! A clip that cannot be drawn — none chosen, gone, changed, unreadable,
//! still opening — draws a card that says which, by name, and the report
//! line says it drew one. Never a black frame: a black frame is a plausible
//! picture of a broken clip.

use std::path::PathBuf;
use std::time::Duration;

use musializer_core::project::clip::{self as model, ClipMoment, ClipRef};
use musializer_core::scene::SceneFrame;
use musializer_core::scenes::clip::ClipState;
use musializer_runtime::clip::{ClipPhase, ClipWorker, Expected, Frame};
use musializer_runtime::font::Faces;
use raylib::prelude::*;

use crate::clips::TrackClip;
use crate::scene_host::TrackAssets;
use crate::ui::widgets;

const SOURCE: &str = include_str!("../../../../resources/shaders/glsl330/clip.fs");
/// How long an exact draw waits for its frame: opening, hashing and probing a
/// large clip, then a restart. A healthy frame takes milliseconds.
const EXACT_DEADLINE: Duration = Duration::from_secs(60);
/// How long the preview waits for the exact frame before drawing the nearest
/// it has: long enough for a frame being decoded (about 4 ms at 1080p), short
/// enough that a scrub never stalls the window.
const PREVIEW_GRACE: Duration = Duration::from_millis(8);
/// Draws a worker may go unused before its FFmpeg is stopped.
const IDLE_DRAWS: u64 = 900;

/// Who is drawing, which decides whether a frame is waited for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipRole {
    /// The window's preview: draws what it has, never waits.
    #[default]
    Preview,
    /// An export, a still or a contact sheet: the exact frame, waited for.
    Export,
    /// A plan-lane poster: the exact frame, on a worker of its own.
    Poster,
}

impl ClipRole {
    fn exact(self) -> bool {
        !matches!(self, ClipRole::Preview)
    }

    fn name(self) -> &'static str {
        match self {
            ClipRole::Preview => "preview",
            ClipRole::Export => "export",
            ClipRole::Poster => "poster",
        }
    }
}

struct Entry {
    clip: ClipRef,
    role: ClipRole,
    source: PathBuf,
    worker: ClipWorker,
    texture: Option<Texture2D>,
    uploaded: Option<usize>,
    last_used: u64,
}

/// What the last frame drew, for the report line.
#[derive(Clone, Debug, Default)]
struct Drawn {
    line: String,
}

pub struct ClipResources {
    thread: RaylibThread,
    shader: Option<Shader>,
    entries: Vec<Entry>,
    draws: u64,
    last: Drawn,
    exact_frames: u64,
    inexact_frames: u64,
    placeholder_frames: u64,
    /// Consecutive inexact preview frames now, and the longest run: how long
    /// the preview trailed a scrub before it caught up.
    inexact_run: u64,
    longest_inexact_run: u64,
}

impl ClipResources {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let shader = rl.load_shader_from_memory(thread, None, Some(SOURCE));
        // raylib answers a failed compile with its default shader; a uniform
        // only this program has tells the two apart.
        let valid = shader.is_shader_valid() && shader.get_shader_location("exposure") >= 0;
        Self {
            thread: thread.clone(),
            shader: valid.then_some(shader),
            entries: Vec::new(),
            draws: 0,
            last: Drawn::default(),
            exact_frames: 0,
            inexact_frames: 0,
            placeholder_frames: 0,
            inexact_run: 0,
            longest_inexact_run: 0,
        }
    }

    /// The `clip:` report line: the last frame, then the run's counts.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.last.line.is_empty() {
            return "none".into();
        }
        format!(
            "{}; frames exact={} inexact={} (longest run {}) placeholder={} shader={}",
            self.last.line,
            self.exact_frames,
            self.inexact_frames,
            self.longest_inexact_run,
            self.placeholder_frames,
            if self.shader.is_some() {
                "compiled"
            } else {
                "FALLBACK"
            }
        )
    }

    /// Stops the decoders nothing has drawn from for a while.
    fn evict(&mut self) {
        let now = self.draws;
        self.entries
            .retain(|entry| now.saturating_sub(entry.last_used) < IDLE_DRAWS);
    }
}

/// The worker for `clip` in `role`, started when there is none.
fn entry<'a>(
    entries: &'a mut Vec<Entry>,
    draws: u64,
    clip: &TrackClip,
    role: ClipRole,
) -> Option<&'a mut Entry> {
    let key = clip.clip_ref()?;
    // A clip the save moved into its bundle is the same bytes elsewhere: its
    // worker reopens it there.
    entries
        .retain(|entry| !(entry.clip == key && entry.role == role && entry.source != clip.source));
    if !entries
        .iter()
        .any(|entry| entry.clip == key && entry.role == role)
    {
        entries.push(Entry {
            clip: key,
            role,
            source: clip.source.clone(),
            worker: ClipWorker::spawn(
                clip.source.clone(),
                Expected {
                    sha256: clip.asset.sha256.clone(),
                    width: clip.asset.width,
                    height: clip.asset.height,
                    frames: clip.asset.frames,
                },
            ),
            texture: None,
            uploaded: None,
            last_used: draws,
        });
    }
    let entry = entries
        .iter_mut()
        .find(|entry| entry.clip == key && entry.role == role)?;
    entry.last_used = draws;
    Some(entry)
}

/// The picture this frame, or why there is none.
enum Outcome {
    Drew {
        index: usize,
        wanted: usize,
        frames: usize,
        size: (u32, u32),
    },
    Card(String, String),
}

#[allow(
    clippy::too_many_arguments,
    reason = "the scene-draw crossing plus the track's clips"
)]
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    resources: &mut ClipResources,
    state: &ClipState,
    frame: &SceneFrame<'_>,
    assets: TrackAssets<'_>,
    boundary: Rectangle,
) {
    resources.draws += 1;
    resources.evict();
    d.draw_rectangle_rec(boundary, Color::BLACK);
    let role = assets.clip_role;
    let moment = match assets.scene_plan {
        Some(plan) => plan.clip_at(frame.time_seconds, assets.base_clip),
        None => assets.base_clip.map(ClipMoment::from_start),
    };
    let outcome = match moment {
        None => Outcome::Card(
            "No clip chosen".into(),
            "Choose one with Import clip in Scenes, or name one in a treatment".into(),
        ),
        Some(moment) => match crate::clips::find(assets.clips, moment.clip) {
            None => Outcome::Card(
                format!("Clip {} is not in this project", moment.clip.short()),
                "Its section names a clip the project holds no file for".into(),
            ),
            Some(clip) => picture(d, resources, clip, moment, role, state, frame, boundary),
        },
    };
    match outcome {
        Outcome::Drew {
            index,
            wanted,
            frames,
            size,
        } => {
            let exact = index == wanted;
            if exact {
                resources.exact_frames += 1;
                resources.inexact_run = 0;
            } else {
                resources.inexact_frames += 1;
                resources.inexact_run += 1;
                resources.longest_inexact_run =
                    resources.longest_inexact_run.max(resources.inexact_run);
            }
            let moment_text = moment.map_or_else(String::new, |moment| {
                format!(
                    "{} from {:.3} at origin {:.3}, after={} fit={}",
                    moment.clip.short(),
                    moment.from_seconds,
                    moment.origin_seconds,
                    moment.after.name(),
                    moment.fit.name()
                )
            });
            let look = state.look();
            resources.last.line = format!(
                "drew frame {index}/{frames} {}{} of {}x{} ({moment_text}) role={} \
                 look zoom={:.3} offset={:.3},{:.3} exposure={:.3} tint={:.2}",
                if exact { "EXACT" } else { "INEXACT, wanted " },
                if exact {
                    String::new()
                } else {
                    format!("{wanted} (still decoding)")
                },
                size.0,
                size.1,
                role.name(),
                look.zoom,
                look.offset[0],
                look.offset[1],
                look.exposure,
                look.tint,
            );
        }
        Outcome::Card(title, detail) => {
            resources.placeholder_frames += 1;
            draw_card(d, fonts, boundary, &title, &detail);
            resources.last.line =
                format!("PLACEHOLDER \"{title}\" ({detail}) role={}", role.name());
        }
    }
}

/// Gets the frame `moment` shows at this time from the clip's worker and
/// draws it, or says why it cannot.
#[allow(clippy::too_many_arguments, reason = "one clip frame's whole context")]
fn picture(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut ClipResources,
    clip: &TrackClip,
    moment: ClipMoment,
    role: ClipRole,
    state: &ClipState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) -> Outcome {
    let name = clip.asset.name.clone();
    // Decided when the project opened; the file is never touched.
    if let Some(error) = &clip.unusable {
        return Outcome::Card(format!("{name}: {error}"), clip.asset.path.clone());
    }
    let source = clip.source.display().to_string();
    let ClipResources {
        thread,
        shader,
        entries,
        draws,
        ..
    } = resources;
    let Some(entry) = entry(entries, *draws, clip, role) else {
        return Outcome::Card(format!("{name}: no usable digest"), source);
    };
    // The exact roles wait out the opening; the preview says it is opening.
    if role.exact() {
        let _ = entry.worker.wait_for(0, Duration::ZERO);
    }
    let probe = loop {
        match entry.worker.phase() {
            ClipPhase::Ready(probe) => break probe,
            ClipPhase::Failed(error) => {
                return Outcome::Card(format!("{name}: {error}"), source);
            }
            ClipPhase::Starting if role.exact() => {
                // Waiting on frame 0 returns once it is ready or failed.
                if entry.worker.wait_for(0, EXACT_DEADLINE).is_err() {
                    if let ClipPhase::Failed(error) = entry.worker.phase() {
                        return Outcome::Card(format!("{name}: {error}"), source);
                    }
                    return Outcome::Card(format!("{name}: FFmpeg did not answer"), source);
                }
            }
            ClipPhase::Starting => {
                return Outcome::Card(
                    format!("Opening {name}"),
                    "checking and probing the file".into(),
                );
            }
        }
    };
    let wanted = moment.frame(&probe.timing, frame.time_seconds);
    let (index, pixels): (usize, Frame) = if role.exact() {
        match entry.worker.wait_for(wanted, EXACT_DEADLINE) {
            Ok(pixels) => (wanted, pixels),
            Err(error) => return Outcome::Card(format!("{name}: {error}"), source),
        }
    } else {
        let got = match entry.worker.wait_for(wanted, PREVIEW_GRACE) {
            Ok(pixels) => Some((wanted, pixels)),
            Err(_) => entry.worker.latest(),
        };
        // A frame the worker read ahead is there at once; one it is decoding
        // gets a few milliseconds, which a played clip's next frame needs and
        // a scrub's far one does not get.
        match got {
            Some(latest) => latest,
            None => {
                if let ClipPhase::Failed(error) = entry.worker.phase() {
                    return Outcome::Card(format!("{name}: {error}"), source);
                }
                return Outcome::Card(
                    format!("Decoding {name}"),
                    format!("frame {wanted} of {}", probe.timing.frame_count()),
                );
            }
        }
    };
    let (width, height) = (probe.width, probe.height);
    if entry.texture.is_none() {
        let image = Image::gen_image_color(width as i32, height as i32, Color::BLACK);
        match d.load_texture_from_image(thread, &image) {
            Ok(texture) => {
                texture.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_BILINEAR);
                texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
                entry.texture = Some(texture);
                entry.uploaded = None;
            }
            Err(error) => {
                return Outcome::Card(
                    format!("{name}: the GPU refused a {width}x{height} texture"),
                    format!("{error:?}"),
                );
            }
        }
    }
    let Some(texture) = entry.texture.as_mut() else {
        return Outcome::Card(format!("{name}: no texture"), source);
    };
    if entry.uploaded != Some(index) {
        if let Err(error) = texture.update_texture(&pixels) {
            return Outcome::Card(format!("{name}: upload refused"), format!("{error:?}"));
        }
        entry.uploaded = Some(index);
    }
    let (src, dst) = model::fit_rects(
        moment.fit,
        [width as f32, height as f32],
        [boundary.width, boundary.height],
    );
    let look = state.look();
    // The music's push and sway, about the frame's centre. A covering clip
    // never shows its edge: the offset stays inside what the zoom added.
    let zoom = look.zoom.max(1.0);
    let margin = [
        (zoom - 1.0) * 0.5 * boundary.width,
        (zoom - 1.0) * 0.5 * boundary.height,
    ];
    let mut offset = [
        look.offset[0] * boundary.width,
        look.offset[1] * boundary.height,
    ];
    if moment.fit == model::ClipFit::Cover {
        offset[0] = offset[0].clamp(-margin[0], margin[0]);
        offset[1] = offset[1].clamp(-margin[1], margin[1]);
    }
    let centre = [boundary.width * 0.5, boundary.height * 0.5];
    let dest = Rectangle::new(
        boundary.x + centre[0] + (dst[0] - centre[0]) * zoom + offset[0],
        boundary.y + centre[1] + (dst[1] - centre[1]) * zoom + offset[1],
        dst[2] * zoom,
        dst[3] * zoom,
    );
    let source_rect = Rectangle::new(src[0], src[1], src[2], src[3]);
    match shader {
        Some(shader) => {
            let location = shader.get_shader_location("exposure");
            shader.set_shader_value(location, look.exposure);
            let location = shader.get_shader_location("tint");
            shader.set_shader_value(location, look.tint);
            d.draw_shader_mode(shader, |mut pass| {
                pass.draw_texture_pro(
                    &*texture,
                    source_rect,
                    dest,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        }
        None => {
            d.draw_texture_pro(
                &*texture,
                source_rect,
                dest,
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
        }
    }
    Outcome::Drew {
        index,
        wanted,
        frames: probe.timing.frame_count(),
        size: (width, height),
    }
}

/// A named card where the clip would be: what is missing and where.
fn draw_card(
    d: &mut RaylibDrawHandle<'_>,
    fonts: &Faces,
    boundary: Rectangle,
    title: &str,
    detail: &str,
) {
    d.draw_rectangle_rec(boundary, Color::new(12, 13, 18, 255));
    let font = fonts.ui();
    let scale = (boundary.height / 720.0).clamp(0.35, 2.0);
    // A film frame: the picture's place, with sprocket holes.
    let frame_w = boundary.width * 0.5;
    let frame_h = frame_w * 9.0 / 16.0;
    let frame = Rectangle::new(
        boundary.x + (boundary.width - frame_w) * 0.5,
        boundary.y + boundary.height * 0.42 - frame_h * 0.5,
        frame_w,
        frame_h,
    );
    let edge = Color::new(92, 98, 118, 255);
    d.draw_rectangle_lines_ex(frame, (2.0 * scale).max(1.0), edge);
    let hole = 10.0 * scale;
    let mut x = frame.x + hole;
    while x + hole < frame.x + frame.width {
        for y in [frame.y - hole * 1.6, frame.y + frame.height + hole * 0.6] {
            d.draw_rectangle_rounded(Rectangle::new(x, y, hole, hole), 0.3, 4, edge);
        }
        x += hole * 2.2;
    }
    let size = 30.0 * scale;
    let width = widgets::measure(font, title, size);
    widgets::draw_text(
        d,
        font,
        title,
        boundary.x + (boundary.width - width) * 0.5,
        frame.y + frame.height * 0.5 - size * 0.9,
        size,
        Color::new(232, 230, 240, 255),
    );
    let small = 18.0 * scale;
    let width = widgets::measure(font, detail, small);
    widgets::draw_text(
        d,
        font,
        detail,
        boundary.x + ((boundary.width - width) * 0.5).max(8.0),
        frame.y + frame.height * 0.5 + size * 0.3,
        small,
        Color::new(150, 156, 176, 255),
    );
}
