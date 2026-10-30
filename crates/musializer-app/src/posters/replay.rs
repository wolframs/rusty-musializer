//! One poster's replay: the export's per-frame sequence over the service's own
//! state.
//!
//! [`Job::advance`] is `with_export_frame` and `draw_offline_frame`
//! (`ui/panels/export.rs`) with every `App` read replaced by an explicit
//! input, because the export's versions read and write the preview's scene,
//! analyzer and plan cursor, and the service may touch none of them:
//!
//! | export | here |
//! | --- | --- |
//! | `app.apply_auto_scene_switch` | none: a run is one scene |
//! | `app.frame_settings` | [`Resolved::table`], the moment's section |
//! | `app.routed_settings` | the same two `RouteTable::apply` calls |
//! | `crate::Analysis` | the job's own analyzer and beat tracker |
//! | `app.scene` | the job's own `SceneInstance` |
//! | the export's target | the board's work canvas, through `Canvas` |
//!
//! The sample cursor, the frame clock, the beat tracking, the frame lanes and
//! the draw are the export's own functions. What differs is where the replay
//! starts: at the moment minus the pre-roll rather than at frame zero, so a
//! poster is close to the export frame and not bit-identical with it.

use std::time::{Duration, Instant};

use musializer_core::audio::beat_tracker::BeatTracker;
use musializer_core::audio::{AudioAnalyzer, AudioAnalyzerConfig, ChannelMode};
use musializer_core::project::frame_lanes::SceneFrameTiming;
use musializer_core::scene::routes::RouteSources;
use musializer_core::scene::{SceneAudioFrame, SceneInstance};
use musializer_core::timing::render_export;
use musializer_runtime::font::Faces;
use musializer_runtime::offscreen::Canvas;
use raylib::prelude::{RaylibDrawHandle, Rectangle};

use super::audio::PosterAudio;
use super::recipe::Resolved;
use crate::scene_host::{self, SceneRenderer};
use crate::workspace::Track;

/// Where a job's time went, for the bench line and `step-worst`.
#[derive(Clone, Copy, Debug, Default)]
pub struct JobTimes {
    /// Analysis, beat tracking, routing and `scene.update` on every frame.
    pub replay: Duration,
    /// The scene draws into the work canvas (CPU-side submission).
    pub draw: Duration,
    pub frames: u64,
    pub drawn: u64,
}

/// A poster being replayed.
pub struct Job {
    pub track_slot: usize,
    pub run_id: super::PosterId,
    pub resolved: Resolved,
    audio: PosterAudio,
    analyzer: Box<AudioAnalyzer>,
    beat: BeatTracker,
    scene: SceneInstance,
    /// The next frame to replay.
    cursor: u64,
    /// The sample the next frame starts hearing from.
    sample_cursor: u64,
    scratch: Vec<f32>,
    pub times: JobTimes,
    pub started: Instant,
}

/// What one [`Job::advance`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// More frames to go.
    Running,
    /// The target frame is in the work canvas.
    Done,
    /// The analyzer could not be configured or the canvas was refused.
    Failed(&'static str),
}

impl Job {
    /// A job for `resolved`, hearing `audio` from the replay's first frame.
    pub fn new(
        track_slot: usize,
        run_id: super::PosterId,
        resolved: Resolved,
        audio: PosterAudio,
        renderer: &mut SceneRenderer,
    ) -> Result<Self, &'static str> {
        let analyzer = AudioAnalyzer::boxed(AudioAnalyzerConfig {
            sample_rate: audio.0.sample_rate,
            channel_count: 1,
            channel_mode: ChannelMode::Select(0),
        })
        .map_err(|_| "the poster analyzer could not be configured")?;
        let recipe = &resolved.recipe;
        let first = resolved.first_frame;
        let sample_cursor = if first == 0 {
            0
        } else {
            render_export::sample_cursor(
                first - 1,
                audio.0.sample_rate,
                recipe.fps,
                audio.0.frame_count(),
            )
            .map_err(|_| "the track's frame rate is out of range")?
        };
        // Cold, as a still's replay starts: light left over from the last
        // poster would be another run's picture.
        renderer.reset_feedback();
        Ok(Self {
            track_slot,
            run_id,
            scene: SceneInstance::new(scene_host::descriptor(recipe.scene), recipe.seed),
            resolved,
            audio,
            analyzer,
            beat: BeatTracker::default(),
            cursor: first,
            sample_cursor,
            scratch: Vec::new(),
            times: JobTimes::default(),
            started: Instant::now(),
        })
    }

    /// Replays one frame; draws it into `canvas` when it is inside the
    /// scene's run-up or is the target.
    #[allow(
        clippy::too_many_arguments,
        reason = "the offline frame's borrowed resources, as `draw_offline_frame` takes them"
    )]
    pub fn advance(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        renderer: &mut SceneRenderer,
        canvas: &mut Canvas,
        fonts: &Faces,
        track: &Track,
    ) -> Advance {
        let replay_started = Instant::now();
        let recipe = &self.resolved.recipe;
        let (fps, index) = (recipe.fps, self.cursor);
        let pcm = &self.audio.0;
        // `RenderJob::take_samples`: each frame hears the samples between the
        // two cursors.
        if index > 0 {
            let next = render_export::sample_cursor(index, pcm.sample_rate, fps, pcm.frame_count())
                .unwrap_or(self.sample_cursor);
            pcm.read(self.sample_cursor, next, &mut self.scratch);
            self.sample_cursor = next;
            if !self.scratch.is_empty() {
                self.analyzer.push_interleaved(&self.scratch);
            }
        }
        let delta = render_export::frame_delta_seconds(index, fps);
        self.analyzer.analyze(delta);
        let spectrum = self.analyzer.spectrum();
        let mut audio_frame = SceneAudioFrame::from_spectrum(spectrum.smooth, spectrum.smear);
        let time_seconds = render_export::frame_time_seconds(index, fps);
        audio_frame.track_beat(&mut self.beat, time_seconds);
        let sources = RouteSources::from_audio(&audio_frame, time_seconds);
        let base = self.resolved.table;
        let mut routed = track
            .scene_routes
            .apply(recipe.scene, &sources, &base)
            .unwrap_or(base);
        if let Some(inner) = self.resolved.nested {
            if let Some(table) = track.scene_routes.apply(inner, &sources, &routed) {
                routed = table;
            }
        }
        let lanes = crate::project_frame_lanes(Some(track), time_seconds);
        let frame = lanes.scene_frame(
            SceneFrameTiming {
                time_seconds,
                duration_seconds: track.duration_seconds,
                delta_seconds: delta,
                frame_index: index,
            },
            audio_frame,
            &routed,
            track.track_dynamics,
        );
        self.times.frames += 1;

        if index < self.resolved.draw_from {
            self.scene.update(&frame);
            self.times.replay += replay_started.elapsed();
            self.cursor += 1;
            return Advance::Running;
        }

        self.times.replay += replay_started.elapsed();
        let draw_started = Instant::now();
        let (width, height) = self.resolved.size();
        let (export_width, _) = recipe.export_size;
        // Physical poster pixels per logical output pixel: the export's own
        // definition of `pixel_scale`, below one here, so fixed-size details
        // shrink with the picture instead of crowding it.
        let pixel_scale = width as f32 / export_width.max(1) as f32;
        let assets = scene_host::TrackAssets {
            atlas_map: track.atlas_map(),
            ascii_grid: track.ascii_grid(),
            caption_style: Some(&track.caption_style),
            scene_plan: Some(&track.scene_switches),
        };
        let scene = &mut self.scene;
        let drew = canvas.draw_into(
            d,
            width as i32,
            height as i32,
            crate::ui::theme::color::background(),
            |d| {
                scene.update(&frame);
                renderer.draw(
                    d,
                    fonts,
                    scene,
                    &frame,
                    assets,
                    Rectangle::new(0.0, 0.0, width as f32, height as f32),
                    pixel_scale,
                );
            },
        );
        self.times.draw += draw_started.elapsed();
        self.times.drawn += 1;
        if !drew {
            return Advance::Failed("the driver refused the poster surface");
        }
        if index >= recipe.frame_index {
            return Advance::Done;
        }
        self.cursor += 1;
        Advance::Running
    }
}
