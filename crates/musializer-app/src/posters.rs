//! Track-specific poster frames, one per plan-lane run (WX-5b).
//!
//! **This file is the seam between the plan lane and the poster service**
//! (`build/redesign/wx5b-design.md` in the main checkout, §3). The read side
//! below — [`PosterRequest`], [`PosterView`], [`PosterBoard::view`] and
//! [`PosterBoard::describe`] — is what the lane is written against; the service
//! fills in everything behind it (the recipe and key in
//! `musializer_core::render::poster`, the replay, the textures, the cache) and
//! owns this file from the seam commit on.
//!
//! The rules the seam commits both sides to:
//!
//! - **Requests travel without interior mutability.** The lane pushes one
//!   [`PosterRequest`] per drawn run into `Shell`'s own list; the composition
//!   root drains it after the frame ([`PosterBoard::enqueue`]). `view` takes
//!   `&self`, and the board is a `main.rs` local rather than part of `Shell`,
//!   so the borrow `ShellInput` holds while the shell draws is a plain shared
//!   one.
//! - **[`PosterBoard::step`] is the last thing in the workspace drawing pair**,
//!   after the egui pass: nothing then holds the board, no texture mode or
//!   scissor is active, and the default framebuffer is bound. Every offscreen
//!   draw restores it the way `runtime::offscreen::Canvas` does — never a bare
//!   `begin_texture_mode`, because `EndTextureMode` does not restore rlgl's
//!   cached framebuffer size (AGENTS.md). A poster drawn in frame N shows from
//!   frame N+1.
//! - **The service never touches the preview's state**: not `app.scene`, the
//!   analyzer, the preview's `SceneRenderer`, the music stream or the plan's
//!   switch state. A poster is close to the export frame, not bit-identical
//!   with it, and is never written as a still.
//! - **Invalidation is a key changing.** The view is recomputed from the track
//!   and the run every frame; an edit that changes a poster's inputs changes
//!   its key, and the old picture is offered only as [`PosterView::Stale`].

mod audio;
mod recipe;
mod replay;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use musializer_core::render::poster::{PosterKey, POSTER_RENDERER_VERSION};
use musializer_core::ui::plan_runs::RunSpan;
use musializer_runtime::font::Faces;
use musializer_runtime::offscreen::Canvas;
use raylib::consts::TextureFilter;
use raylib::prelude::{
    Color, RaylibDrawHandle, RaylibHandle, RaylibTexture2D, RaylibThread, Rectangle,
    RenderTexture2D, Vector2,
};

pub use audio::PosterAudio;
use recipe::{Resolved, Tuning};
use replay::{Advance, Job};

use crate::scene_host::SceneRenderer;
use crate::workspace::{Track, Workspace};

/// One run the lane drew this frame and wants a poster for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosterRequest {
    pub track_slot: usize,
    pub run: RunSpan,
    /// Inside the lane's visible span this frame. Visible runs are served
    /// first, then by distance from the playhead.
    pub visible: bool,
}

/// What the lane draws for a run.
///
/// The texture is a render target's colour buffer **stored top row first**:
/// draw it as an ordinary texture, with a positive source height. (The
/// service flips it once when the poster is finished, so the lane never has
/// to know it came from a framebuffer.)
#[allow(
    dead_code,
    reason = "the lane reads the reason into its tooltip once it draws posters"
)]
pub enum PosterView<'a> {
    /// The run's poster for its current inputs.
    Ready {
        texture: &'a RenderTexture2D,
        aspect: f32,
    },
    /// The run's last poster, whose inputs have since changed. Drawn at 45 %
    /// over the run's fill until the new one is ready — never at full strength,
    /// because a stale preview confidently lies (CX-5).
    Stale {
        texture: &'a RenderTexture2D,
        aspect: f32,
    },
    /// Never drawn yet: the run draws as it would without posters.
    Pending,
    /// Refused, with the reason for the run's tooltip: no audio, a surface the
    /// driver would not allocate, posters switched off.
    Unavailable(&'static str),
}

/// `--ui-probe posters=`: let a capture finish every requested poster before
/// it is taken, or switch the service off so the lane's fallback is what is
/// photographed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosterProbe {
    Settle,
    Off,
}

/// How many posters stay on the GPU, across tracks. 256x144 RGBA plus its
/// depth buffer is ~0.3 MB, so the whole cache is ~28 MB.
const LRU_CAPACITY: usize = 96;
/// The per-app-frame budget while the song plays, and while it does not.
const BUDGET_PLAYING: Duration = Duration::from_millis(3);
const BUDGET_PAUSED: Duration = Duration::from_millis(8);
/// The analyzer pre-roll and the poster box (see the measurement in
/// `docs/POSTER_FRAMES.md` for why these two).
const DEFAULT_PREROLL_SECONDS: f64 = 2.0;
const DEFAULT_BOX: (u32, u32) = (256, 144);
/// A settle that has not finished after this many replay frames is a bug
/// (a key that never stops moving), not a slow poster.
const SETTLE_FRAME_LIMIT: u64 = 2_000_000;

/// One finished poster.
struct Entry {
    track: PathBuf,
    run_id: u64,
    key: PosterKey,
    aspect: f32,
    canvas: Canvas,
    /// The board clock when the lane last asked for this run.
    seen: u64,
}

/// One requested run, resolved once for the step.
struct Surveyed {
    request: PosterRequest,
    key: Option<PosterKey>,
    /// Taken by the job that renders it.
    resolved: Option<Resolved>,
}

/// The second renderer, loaded on the first step that has work.
enum RendererState {
    NotLoaded,
    Loaded(Box<SceneRenderer>),
    Failed,
}

/// The step-level statistics the `posters:` line reports.
#[derive(Default)]
struct Stats {
    /// Requests the last step saw, by state.
    requested: usize,
    ready: usize,
    stale: usize,
    pending: usize,
    unavailable: usize,
    /// The longest budgeted step that did poster work.
    step_worst: Duration,
    /// The longest step that found no poster work: the bookkeeping (a recipe
    /// and key per requested run) every frame pays once the lane is full.
    idle_worst: Duration,
    /// The longest settle step (unbounded by design).
    settle_worst: Duration,
    /// Posters finished, and jobs dropped because their key moved.
    rendered: u64,
    abandoned: u64,
    /// Finished posters that replaced a picture with an older key: the
    /// stale-then-ready path an edit takes.
    refreshed: u64,
    /// The longest single replay frame, which a budgeted step may overrun by.
    frame_worst: Duration,
    /// Per-poster wall time (replay plus draw) of every finished poster.
    poster_ms: Vec<f64>,
    renderer_load: Option<Duration>,
    /// What loading it added to the process's resident set, in MB (CPU
    /// side; the GPU's share is the buffers it allocates as scenes draw).
    renderer_resident: f64,
}

/// The poster service: the lane's read side, the request queue, the second
/// renderer, the replay and the cache.
pub struct PosterBoard {
    probe: Option<PosterProbe>,
    dump: Option<PathBuf>,
    tuning: Tuning,
    /// `MUSIALIZER_POSTER_BENCH=N`: render every poster N times and print a
    /// `poster bench:` line per render. The measurement behind the defaults.
    bench_repeats: u32,
    requests: Vec<PosterRequest>,
    entries: Vec<Entry>,
    renderer: RendererState,
    /// Where a job draws; a finished frame is copied out of it into its entry.
    work: Canvas,
    job: Option<Job>,
    /// Bench renders left for the current job's key.
    bench_left: u32,
    /// Keys whose replay failed, so they are not retried every frame.
    failed: Vec<PosterKey>,
    /// A refusal that applies to every poster.
    refusal: Option<&'static str>,
    clock: u64,
    stats: Stats,
}

impl Default for PosterBoard {
    fn default() -> Self {
        Self {
            probe: None,
            dump: None,
            tuning: Tuning {
                preroll_seconds: DEFAULT_PREROLL_SECONDS,
                box_size: DEFAULT_BOX,
            },
            bench_repeats: 0,
            requests: Vec::new(),
            entries: Vec::new(),
            renderer: RendererState::NotLoaded,
            work: Canvas::new(),
            job: None,
            bench_left: 0,
            failed: Vec::new(),
            refusal: None,
            clock: 0,
            stats: Stats::default(),
        }
    }
}

impl PosterBoard {
    /// The board with the measurement knobs read from the environment:
    /// `MUSIALIZER_POSTER_PREROLL` (seconds), `MUSIALIZER_POSTER_BOX` (`WxH`)
    /// and `MUSIALIZER_POSTER_BENCH` (repeats). Unset, they are the defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let mut board = Self::default();
        if let Some(seconds) = std::env::var("MUSIALIZER_POSTER_PREROLL")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && (0.0..=30.0).contains(seconds))
        {
            board.tuning.preroll_seconds = seconds;
        }
        if let Some(size) = std::env::var("MUSIALIZER_POSTER_BOX")
            .ok()
            .and_then(|value| {
                let (width, height) = value.split_once('x')?;
                let size = (width.parse::<u32>().ok()?, height.parse::<u32>().ok()?);
                (size.0 >= 8 && size.1 >= 8 && size.0 <= 1024 && size.1 <= 1024).then_some(size)
            })
        {
            board.tuning.box_size = size;
        }
        board.bench_repeats = std::env::var("MUSIALIZER_POSTER_BENCH")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        board
    }

    /// Applies `--ui-probe posters=` and `poster-dump=`.
    pub fn configure(&mut self, probe: Option<PosterProbe>, dump: Option<PathBuf>) {
        self.probe = probe;
        self.dump = dump;
    }

    /// Why no run of `track` can have a poster, if none can.
    fn refused(&self, track: &Track) -> Option<&'static str> {
        if self.probe == Some(PosterProbe::Off) {
            return Some("posters are switched off for this run");
        }
        if let Some(reason) = self.refusal {
            return Some(reason);
        }
        if track.poster_audio.is_none() {
            return Some("the track's audio could not be decoded for a poster");
        }
        None
    }

    fn entry(&self, track: &Path, run_id: u64) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.run_id == run_id && entry.track == track)
    }

    /// What the lane should draw for `run` of the track in `track_slot`.
    pub fn view(&self, track: &Track, _track_slot: usize, run: &RunSpan) -> PosterView<'_> {
        if let Some(reason) = self.refused(track) {
            return PosterView::Unavailable(reason);
        }
        let key = recipe::resolve(track, run, &self.tuning).key;
        match self.entry(&track.file_path, run.first_cue_id) {
            Some(entry) => {
                let Some(texture) = entry.canvas.render_texture() else {
                    return PosterView::Pending;
                };
                if entry.key == key {
                    PosterView::Ready {
                        texture,
                        aspect: entry.aspect,
                    }
                } else {
                    PosterView::Stale {
                        texture,
                        aspect: entry.aspect,
                    }
                }
            }
            None if self.failed.contains(&key) => {
                PosterView::Unavailable("this run's poster could not be drawn")
            }
            None => PosterView::Pending,
        }
    }

    /// Hands the frame's requests to the service.
    pub fn enqueue(&mut self, requests: Vec<PosterRequest>) {
        self.requests = requests;
    }

    /// Does up to one frame's budget of poster work. See the module comment
    /// for where it is called and what it may not touch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the composition root's resources a poster draws with, borrowed for one call"
    )]
    pub fn step(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        thread: &RaylibThread,
        fonts: &Faces,
        workspace: &Workspace,
        playhead_seconds: f64,
        playing: bool,
    ) {
        let started = Instant::now();
        self.clock += 1;
        let settle = self.probe == Some(PosterProbe::Settle);
        let mut worked = false;
        // One recipe per requested run per frame, shared by everything below:
        // the board's bookkeeping then costs one resolve per run on top of
        // the lane's own `view`, instead of three.
        let mut survey = self.survey(workspace);
        if self.probe != Some(PosterProbe::Off) && !d.is_window_minimized() {
            let budget = if playing {
                BUDGET_PLAYING
            } else {
                BUDGET_PAUSED
            };
            // A settle finishes everything, so it waives the paused-only rule.
            let held = playing && !settle;
            let mut frames = 0u64;
            loop {
                if self.job.is_none()
                    && !self.start_next(
                        d,
                        thread,
                        fonts,
                        workspace,
                        &mut survey,
                        playhead_seconds,
                        held,
                    )
                {
                    break;
                }
                // A paused-only scene's job keeps its place while the song
                // plays and resumes when it stops.
                if held
                    && self
                        .job
                        .as_ref()
                        .is_some_and(|job| held_while_playing(job.resolved.recipe.scene, fonts))
                {
                    break;
                }
                worked = true;
                let frame_started = Instant::now();
                self.advance_job(d, thread, fonts, workspace, &survey);
                self.stats.frame_worst = self.stats.frame_worst.max(frame_started.elapsed());
                frames += 1;
                if settle {
                    if frames >= SETTLE_FRAME_LIMIT {
                        eprintln!("posters: settle stopped after {frames} replay frames");
                        break;
                    }
                } else if started.elapsed() >= budget {
                    break;
                }
            }
        }
        self.count(workspace, &survey);
        if !worked {
            self.stats.idle_worst = self.stats.idle_worst.max(started.elapsed());
        }
        if worked {
            let elapsed = started.elapsed();
            let worst = if settle {
                &mut self.stats.settle_worst
            } else {
                &mut self.stats.step_worst
            };
            *worst = (*worst).max(elapsed);
        }
    }

    /// Loads the second renderer before the frame loop, so its ~40 ms of
    /// shader compiles land in startup rather than in a frame the frame budget
    /// counts. Skipped when posters are switched off.
    pub fn preload(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) {
        if self.probe != Some(PosterProbe::Off) {
            let _ = self.renderer(rl, thread);
        }
    }

    /// The second renderer, loaded on first use if [`Self::preload`] did not.
    fn renderer(
        &mut self,
        d: &mut RaylibHandle,
        thread: &RaylibThread,
    ) -> Option<&mut SceneRenderer> {
        if matches!(self.renderer, RendererState::NotLoaded) {
            let started = Instant::now();
            let resident = resident_megabytes();
            self.renderer = match SceneRenderer::load(d, thread) {
                Ok(renderer) => RendererState::Loaded(Box::new(renderer)),
                Err(error) => {
                    eprintln!("posters: the second scene renderer did not load: {error}");
                    self.refusal = Some("the poster renderer did not load");
                    RendererState::Failed
                }
            };
            self.stats.renderer_load = Some(started.elapsed());
            self.stats.renderer_resident = resident_megabytes() - resident;
        }
        match &mut self.renderer {
            RendererState::Loaded(renderer) => Some(renderer),
            _ => None,
        }
    }

    /// Resolves every requested run once: its recipe, or `None` when no
    /// poster can be made for it (no track, posters off, a refusal).
    fn survey(&self, workspace: &Workspace) -> Vec<Surveyed> {
        self.requests
            .iter()
            .map(|request| {
                let resolved = workspace
                    .get(request.track_slot)
                    .filter(|track| self.refused(track).is_none())
                    .map(|track| recipe::resolve(track, &request.run, &self.tuning));
                Surveyed {
                    request: *request,
                    key: resolved.as_ref().map(|resolved| resolved.key),
                    resolved,
                }
            })
            .collect()
    }

    /// Starts the most wanted poster whose picture is missing or stale.
    /// Returns false when there is nothing to do.
    #[allow(
        clippy::too_many_arguments,
        reason = "step's borrowed resources plus this frame's survey and transport"
    )]
    fn start_next(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        thread: &RaylibThread,
        fonts: &Faces,
        workspace: &Workspace,
        survey: &mut [Surveyed],
        playhead_seconds: f64,
        playing: bool,
    ) -> bool {
        if self.refusal.is_some() {
            return false;
        }
        let mut best: Option<(bool, f64, usize)> = None;
        for (index, item) in survey.iter().enumerate() {
            let (Some(key), Some(_)) = (item.key, item.resolved.as_ref()) else {
                continue;
            };
            let request = &item.request;
            let Some(track) = workspace.get(request.track_slot) else {
                continue;
            };
            let current = self
                .entry(&track.file_path, request.run.first_cue_id)
                .is_some_and(|entry| entry.key == key);
            if current || self.failed.contains(&key) {
                continue;
            }
            if playing && held_while_playing(request.run.scene, fonts) {
                continue;
            }
            let run = &request.run;
            let distance = if run.contains(playhead_seconds) {
                0.0
            } else {
                (run.start_seconds - playhead_seconds)
                    .abs()
                    .min((run.end_seconds - playhead_seconds).abs())
            };
            let better = match best {
                None => true,
                Some((visible, best_distance, _)) => {
                    (request.visible, -distance) > (visible, -best_distance)
                }
            };
            if better {
                best = Some((request.visible, distance, index));
            }
        }
        let Some((_, _, index)) = best else {
            return false;
        };
        let item = &mut survey[index];
        let (slot, run_id) = (item.request.track_slot, item.request.run.first_cue_id);
        let Some(resolved) = item.resolved.take() else {
            return false;
        };
        let Some(audio) = workspace
            .get(slot)
            .and_then(|track| track.poster_audio.clone())
        else {
            return false;
        };
        if self.bench_repeats > 0 {
            // Drain the GPU before the clock starts, so the sync measured
            // after the poster is the poster's own GPU time.
            gpu_sync(&self.work);
        }
        let Some(renderer) = self.renderer(d, thread) else {
            return false;
        };
        let key = resolved.key;
        match Job::new(slot, run_id, resolved, audio, renderer) {
            Ok(job) => {
                if self.bench_left == 0 {
                    self.bench_left = self.bench_repeats.max(1);
                }
                self.job = Some(job);
                true
            }
            Err(reason) => {
                eprintln!("posters: {reason}");
                self.failed.push(key);
                true
            }
        }
    }

    /// One replay frame of the running job, and its completion.
    fn advance_job(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        thread: &RaylibThread,
        fonts: &Faces,
        workspace: &Workspace,
        survey: &[Surveyed],
    ) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        // Still wanted, with the same inputs? An edit mid-replay moves the
        // key, and finishing the old picture would only file it as stale.
        // Edits land between frames, so this frame's survey is current.
        let wanted = survey
            .iter()
            .find(|item| {
                item.request.track_slot == job.track_slot
                    && item.request.run.first_cue_id == job.run_id
            })
            .and_then(|item| item.key)
            .filter(|key| *key == job.resolved.key);
        let (Some(key), Some(track)) = (wanted, workspace.get(job.track_slot)) else {
            self.job = None;
            self.bench_left = 0;
            self.stats.abandoned += 1;
            return;
        };
        let RendererState::Loaded(renderer) = &mut self.renderer else {
            self.job = None;
            return;
        };
        match job.advance(d, renderer, &mut self.work, fonts, track) {
            Advance::Running => {}
            Advance::Failed(reason) => {
                eprintln!("posters: {reason}");
                if self.work.refused() {
                    self.refusal = Some(reason);
                }
                self.failed.push(key);
                self.job = None;
                self.bench_left = 0;
            }
            Advance::Done => {
                let job = self.job.take().expect("matched above");
                self.finish(d, thread, job, track);
            }
        }
    }

    /// Copies a finished frame into its entry, flipped so the entry reads top
    /// row first, and writes the dump.
    fn finish(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        thread: &RaylibThread,
        job: Job,
        track: &Track,
    ) {
        let compute = job.started.elapsed();
        let sync = if self.bench_repeats > 0 {
            let started = Instant::now();
            gpu_sync(&self.work);
            Some(started.elapsed())
        } else {
            None
        };
        let resolved = &job.resolved;
        let (width, height) = resolved.size();
        let total = compute + sync.unwrap_or_default();
        self.stats.poster_ms.push(total.as_secs_f64() * 1000.0);
        if let Some(sync) = sync {
            println!(
                "poster bench: scene={} size={width}x{height} preroll={:.1}s frames={} drawn={} \
                 replay={:.2}ms draw={:.2}ms gpu-sync={:.2}ms total={:.2}ms",
                resolved.recipe.scene.stable_name(),
                self.tuning.preroll_seconds,
                job.times.frames,
                job.times.drawn,
                job.times.replay.as_secs_f64() * 1000.0,
                job.times.draw.as_secs_f64() * 1000.0,
                sync.as_secs_f64() * 1000.0,
                total.as_secs_f64() * 1000.0,
            );
        }
        self.bench_left = self.bench_left.saturating_sub(1);
        if self.bench_left > 0 {
            // The next start picks the same run again: its entry is not
            // updated until the last repeat.
            return;
        }

        let index = match self
            .entries
            .iter()
            .position(|entry| entry.track == track.file_path && entry.run_id == job.run_id)
        {
            Some(index) => index,
            None => {
                if self.entries.len() >= LRU_CAPACITY {
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
                self.entries.push(Entry {
                    track: track.file_path.clone(),
                    run_id: job.run_id,
                    key: resolved.key,
                    aspect: resolved.aspect(),
                    canvas: Canvas::new(),
                    seen: self.clock,
                });
                self.entries.len() - 1
            }
        };
        let Some(source) = self.work.texture() else {
            return;
        };
        let entry = &mut self.entries[index];
        let rect = Rectangle::new(0.0, 0.0, width as f32, height as f32);
        // A positive source height reads the work buffer bottom row first, so
        // the entry ends up top row first: an ordinary texture to the lane.
        let copied = entry
            .canvas
            .draw_into(d, width as i32, height as i32, Color::BLACK, |d| {
                musializer_runtime::draw::draw_texture_pro(
                    d,
                    source,
                    rect,
                    rect,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        if !copied {
            self.refusal = Some("the driver refused a poster texture");
            return;
        }
        // The lane draws a 256-pixel poster about 80 logical pixels wide, a
        // threefold minification: without mipmaps a bilinear sample skips
        // texels and thin lines and small type break up. Regenerated after
        // every copy, because only level 0 was drawn.
        if let Some(target) = entry.canvas.render_texture_mut() {
            target.gen_texture_mipmaps();
            target.set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_TRILINEAR);
        }
        if entry.key != resolved.key {
            self.stats.refreshed += 1;
        }
        entry.key = resolved.key;
        entry.aspect = resolved.aspect();
        entry.seen = self.clock;
        self.stats.rendered += 1;
        if let Some(directory) = &self.dump {
            let name = format!(
                "{}-{}-{:016x}.png",
                resolved.recipe.scene.stable_name(),
                job.run_id,
                resolved.key.0
            );
            write_dump(entry, &directory.join(name));
        }
    }

    /// Refreshes the per-state counts and the LRU stamps from this frame's
    /// survey. Uses its keys rather than resolving again: a poster finished
    /// in this step is in its entry by now, under the same key.
    fn count(&mut self, workspace: &Workspace, survey: &[Surveyed]) {
        let mut stats = (0, 0, 0, 0);
        let clock = self.clock;
        for item in survey {
            let (Some(key), Some(track)) = (item.key, workspace.get(item.request.track_slot))
            else {
                stats.3 += 1;
                continue;
            };
            let run_id = item.request.run.first_cue_id;
            match self
                .entries
                .iter_mut()
                .find(|entry| entry.run_id == run_id && entry.track == track.file_path)
            {
                Some(entry) => {
                    entry.seen = clock;
                    if entry.key == key {
                        stats.0 += 1;
                    } else {
                        stats.1 += 1;
                    }
                }
                None if self.failed.contains(&key) => stats.3 += 1,
                None => stats.2 += 1,
            }
        }
        self.stats.requested = survey.len();
        (
            self.stats.ready,
            self.stats.stale,
            self.stats.pending,
            self.stats.unavailable,
        ) = stats;
    }

    /// The `posters:` report line.
    #[must_use]
    pub fn describe(&self) -> String {
        let stats = &self.stats;
        let (width, height) = recipe::poster_size((1920, 1080), self.tuning.box_size);
        let mut times = stats.poster_ms.clone();
        times.sort_by(f64::total_cmp);
        let median = times.get(times.len() / 2).copied().unwrap_or(0.0);
        let worst = times.last().copied().unwrap_or(0.0);
        format!(
            "requested={} ready={} stale={} pending={} unavailable={} lru={}/{LRU_CAPACITY} \
             size={width}x{height} preroll={:.1}s step-worst={:.1}ms settle-worst={:.1}ms \
             frame-worst={:.1}ms idle-worst={:.2}ms budget={}ms/{}ms rendered={} refreshed={} abandoned={} poster-median={median:.1}ms \
             poster-worst={worst:.1}ms isolated={} load={} version={POSTER_RENDERER_VERSION} \
             probe={} dump={}",
            stats.requested,
            stats.ready,
            stats.stale,
            stats.pending,
            stats.unavailable,
            self.entries.len(),
            self.tuning.preroll_seconds,
            stats.step_worst.as_secs_f64() * 1000.0,
            stats.settle_worst.as_secs_f64() * 1000.0,
            stats.frame_worst.as_secs_f64() * 1000.0,
            stats.idle_worst.as_secs_f64() * 1000.0,
            BUDGET_PLAYING.as_millis(),
            BUDGET_PAUSED.as_millis(),
            stats.rendered,
            stats.refreshed,
            stats.abandoned,
            match (&self.renderer, self.refusal) {
                (_, Some(_)) | (RendererState::Failed, _) => "FAILED",
                (RendererState::Loaded(_), None) => "renderer#2",
                (RendererState::NotLoaded, None) => "idle",
            },
            stats
                .renderer_load
                .map_or_else(
                    || "none".to_owned(),
                    |load| format!(
                        "{:.0}ms/+{:.1}MB",
                        load.as_secs_f64() * 1000.0,
                        stats.renderer_resident
                    ),
                ),
            match self.probe {
                None => "none",
                Some(PosterProbe::Settle) => "settle",
                Some(PosterProbe::Off) => "off",
            },
            self.dump
                .as_deref()
                .map_or_else(|| "none".to_owned(), |path| path.display().to_string()),
        )
    }
}

/// Whether a poster of `scene` waits for the song to pause: a scene capped
/// as paused-only, and Cadence until the shared SDF atlas exists, because its
/// first build (~300 ms, once) would otherwise land in a playing frame. The
/// preview pays the same build the first time it shows Cadence, after which
/// Cadence posters run while playing too.
fn held_while_playing(scene: musializer_core::scene::SceneId, fonts: &Faces) -> bool {
    recipe::scene_cap(scene).paused_only
        || (scene == musializer_core::scene::SceneId::Cadence && !fonts.cadence_sdf_built())
}

/// The process's resident set in MB, from `/proc/self/statm`; zero where
/// that is unreadable.
fn resident_megabytes() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|text| text.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |pages| pages * 4096.0 / (1024.0 * 1024.0))
}

/// Waits for the GPU by reading one buffer back. Bench only: the normal path
/// never reads a poster back.
fn gpu_sync(canvas: &Canvas) {
    if let Some(target) = canvas.render_texture() {
        let _ = target.load_image();
    }
}

/// Writes a finished poster as a PNG for `poster-dump=`.
fn write_dump(entry: &Entry, path: &Path) {
    let Some(target) = entry.canvas.render_texture() else {
        return;
    };
    // `export_image` reports nothing, so confirm by looking, as the probe
    // shot does.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(path);
    if let (Ok(image), Some(text)) = (target.load_image(), path.to_str()) {
        image.export_image(text);
    }
    let written = path.is_file();
    if written {
        println!("poster dump:     {}", path.display());
    } else {
        eprintln!("posters: could not write {}", path.display());
    }
}
