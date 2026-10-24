//! A run's poster recipe, built from the track every time the lane asks.
//!
//! The recipe is the *whole* identity of a poster (`render::poster` in core):
//! nothing that changes the picture may be read by the replay without also
//! being in here, or an edit leaves a stale picture claiming to be current.
//! So the replay reads its tuning from [`Resolved::table`], which this module
//! built from the same values it put in the recipe, rather than from the track.

use musializer_core::render::poster::{
    poster_moment, PosterKey, PosterRecipe, POSTER_RENDERER_VERSION,
};
use musializer_core::scene::settings::SceneSettings;
use musializer_core::scene::SceneId;
use musializer_core::ui::plan_runs::RunSpan;

use crate::scene_host;
use crate::workspace::Track;

/// The service's knobs that reach the picture, so they are in the key too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Analyzer pre-roll before the drawn frame, in seconds (A in the design
    /// note): long enough for the smoothed bands to settle.
    pub preroll_seconds: f64,
    /// The box a poster is fitted into, keeping the export's aspect.
    pub box_size: (u32, u32),
}

/// A recipe and what the replay needs to honour it.
pub struct Resolved {
    pub recipe: PosterRecipe,
    pub key: PosterKey,
    /// The tuning every replay frame draws with: the track's table with the
    /// moment's section snapshot (and the doorway scene's) applied.
    pub table: SceneSettings,
    /// The doorway scene, for a threshold.
    pub nested: Option<SceneId>,
    /// The frame the replay starts on, and the first one it draws.
    pub first_frame: u64,
    pub draw_from: u64,
}

impl Resolved {
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.recipe.poster_size
    }

    #[must_use]
    pub fn aspect(&self) -> f32 {
        let (width, height) = self.recipe.poster_size;
        width as f32 / height.max(1) as f32
    }
}

/// The poster's pixel size: the export's aspect fitted into `box_size`, both
/// edges even and at least 2.
#[must_use]
pub fn poster_size(export: (u32, u32), box_size: (u32, u32)) -> (u32, u32) {
    let aspect = f64::from(export.0.max(1)) / f64::from(export.1.max(1));
    let (box_width, box_height) = (f64::from(box_size.0), f64::from(box_size.1));
    let (width, height) = if box_width / box_height > aspect {
        (box_height * aspect, box_height)
    } else {
        (box_width, box_width / aspect)
    };
    let even = |value: f64| (((value / 2.0).round() as u32) * 2).max(2);
    (even(width), even(height))
}

/// A scene whose poster would cost too much at the service's defaults gets its
/// own limits, rather than holding every other scene back (design note §4).
///
/// Measured on the GPU path with a release build, 256x144, five renders each
/// (`docs/POSTER_FRAMES.md` has the table):
///
/// - **Clawd** keeps light between frames, so its 3 s run-up is *drawn*: ~90
///   immediate-mode frames at ~1.6 ms of CPU each, 184 ms median and 240 ms
///   worst. One second of run-up is ~2.5 of the resting trail's half-lives —
///   the ghosts are there, a little shorter than in the video.
/// - **Fluid Ink** is a fixed-substep CPU simulation, ~10 ms per replayed
///   frame whatever the poster's size: 600 ms median at A = 2.0 s, 155 ms at
///   0.5 s. It gets the short pre-roll, and because one of its replay frames
///   alone is three times the playing budget, it is only replayed while the
///   song is paused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneCap {
    pub preroll_seconds: Option<f64>,
    pub warmup_seconds: Option<f64>,
    pub paused_only: bool,
}

#[must_use]
pub fn scene_cap(id: SceneId) -> SceneCap {
    match id {
        SceneId::Clawd => SceneCap {
            preroll_seconds: None,
            warmup_seconds: Some(1.0),
            paused_only: false,
        },
        SceneId::FluidInk => SceneCap {
            preroll_seconds: Some(1.0),
            warmup_seconds: None,
            paused_only: true,
        },
        _ => SceneCap {
            preroll_seconds: None,
            warmup_seconds: None,
            paused_only: false,
        },
    }
}

/// Whether the scene draws the lyric line itself, so no caption goes over it
/// (`SceneRenderer::draw_overlays`'s rule).
fn draws_its_line(id: SceneId) -> bool {
    id == SceneId::Cadence
        || id == SceneId::Stage
        || musializer_core::scenes::pixel_cats::Staging::of(id).is_some()
}

/// Builds the recipe for `run` of `track`.
#[must_use]
pub fn resolve(track: &Track, run: &RunSpan, tuning: &Tuning) -> Resolved {
    let config = track.render_config;
    let fps = config.fps.max(1);
    let plan = &track.scene_switches;
    let frame_index = poster_moment(plan, &track.lyrics, run, fps, track.duration_seconds);
    let time = frame_index as f64 / f64::from(fps);
    let scene = run.scene;

    // The section the moment falls in, inside the run. `poster_moment` keeps
    // the moment inside the run, so the fallback is only a guard.
    let cues = plan.cues();
    let last = (run.first_section + run.sections).min(cues.len());
    let section = (run.first_section..last)
        .map(|index| &cues[index])
        .find(|cue| cue.start_seconds <= time && time < cue.end_seconds)
        .or_else(|| cues.get(run.first_section));

    let mut table = track.scene_settings;
    if let Some(cue) = section.filter(|cue| cue.settings.captured) {
        table.apply_snapshot(scene, &cue.settings);
    }
    let settings = table.capture(scene).unwrap_or_default();
    // The threshold's doorway scene wears its nearest section's tuning, as
    // `App::frame_settings` gives it.
    let nested = (scene == SceneId::Threshold)
        .then(|| {
            musializer_core::scenes::threshold::beyond_scene(table.get(
                SceneId::Threshold,
                musializer_core::scene::settings::index::threshold::BEYOND,
            ))
        })
        .flatten();
    let nested_settings = nested.map(|inner| {
        if let Some(cue) = plan
            .cue_near(time, inner)
            .filter(|cue| cue.settings.captured)
        {
            table.apply_snapshot(inner, &cue.settings);
        }
        (inner, table.capture(inner).unwrap_or_default())
    });

    // Analyzer pre-roll, then the scene's own run-up for light it keeps
    // between frames; only the run-up is drawn. The moment is never inside a
    // transition, so the export's transition run-up does not apply.
    // A threshold's doorway scene is capped like the scene it is.
    let capped = |id: SceneId| {
        let cap = scene_cap(id);
        (
            cap.preroll_seconds.unwrap_or(tuning.preroll_seconds),
            cap.warmup_seconds
                .unwrap_or_else(|| scene_host::feedback_warmup_seconds(id)),
        )
    };
    let (mut preroll_seconds, mut warmup_seconds) = capped(scene);
    if let Some(inner) = nested {
        let (inner_preroll, inner_warmup) = capped(inner);
        preroll_seconds = preroll_seconds.min(inner_preroll);
        warmup_seconds = warmup_seconds.max(inner_warmup);
    }
    let warmup = (warmup_seconds * f64::from(fps)).ceil() as u64;
    let preroll = (preroll_seconds * f64::from(fps)).ceil() as u64 + warmup;
    let first_frame = frame_index.saturating_sub(preroll);
    let draw_from = frame_index.saturating_sub(warmup);
    let window = (first_frame as f64 / f64::from(fps), time);

    let mut routes = vec![track.scene_routes.scene(scene).clone()];
    if let Some(inner) = nested {
        routes.push(track.scene_routes.scene(inner).clone());
    }
    let lyrics = track
        .lyrics
        .cues()
        .iter()
        .filter(|cue| {
            cue.origin.is_displayable()
                && cue.end_seconds > window.0
                && cue.start_seconds <= window.1
        })
        .cloned()
        .collect();

    let export_size = (config.width, config.height);
    let recipe = PosterRecipe {
        audio: if track.audio_sha256.is_empty() {
            track.file_path.display().to_string()
        } else {
            track.audio_sha256.clone()
        },
        scene,
        seed: track.scene_seed,
        frame_index,
        fps,
        preroll_frames: frame_index - first_frame,
        settings,
        nested: nested_settings,
        routes,
        grade: plan.grade_at(time),
        pixel: plan.pixel_at(time),
        finish: plan.finish,
        caption: (!draws_its_line(scene)).then(|| track.caption_style.clone()),
        lyrics,
        voices: format!("{:?}", track.lyrics.voices()),
        events: (
            track.semantic_events.revision(),
            track.manual_events.revision(),
        ),
        ascii: track.ascii.as_ref().map(|ascii| ascii.sha256.clone()),
        atlas: track.atlas_map().is_some(),
        export_size,
        poster_size: poster_size(export_size, tuning.box_size),
        version: POSTER_RENDERER_VERSION,
    };
    Resolved {
        key: recipe.key(),
        recipe,
        table,
        nested,
        first_frame,
        draw_from,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poster_keeps_the_exports_shape_inside_its_box() {
        assert_eq!(poster_size((1920, 1080), (256, 144)), (256, 144));
        assert_eq!(poster_size((1280, 720), (128, 72)), (128, 72));
        assert_eq!(poster_size((1080, 1920), (256, 144)), (82, 144));
        assert_eq!(poster_size((1080, 1080), (256, 144)), (144, 144));
        assert_eq!(poster_size((1080, 1350), (160, 90)), (72, 90));
    }
}
