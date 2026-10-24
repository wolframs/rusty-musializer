//! Poster frames for the plan lane's runs (WX-5b): what a poster is *of*.
//!
//! A poster is one small rendered frame per run of the scene plan, drawn
//! through the export's offline path from this track's own audio. This module
//! holds the two decisions that need no GPU and are therefore pinned by unit
//! tests rather than by a picture:
//!
//! - **which moment** a run is shown at ([`poster_moment`]): deterministic and
//!   independent of the playhead, so nothing chases playback;
//! - **what identifies the picture** ([`PosterRecipe`], [`PosterKey`]): every
//!   input that changes its pixels. Invalidation is the key changing, so no
//!   edit event is needed — an undo, a followed treatment and a tuning drag
//!   all reach a poster the same way.
//!
//! The service that renders them lives in `musializer-app/src/posters.rs`;
//! `docs/POSTER_FRAMES.md` is its design record.

use std::fmt::Write as _;

use crate::project::look::{Finish, Grade};
use crate::project::lyrics::{LyricCue, LyricsDocument};
use crate::project::model::CaptionStyle;
use crate::project::pixel::PixelFrame;
use crate::project::scene_switch::SceneSwitchTimeline;
use crate::project::transition::{fitted_seconds, TransitionKind};
use crate::scene::routes::SceneRoutes;
use crate::scene::settings::SettingsSnapshot;
use crate::scene::SceneId;
use crate::ui::plan_runs::RunSpan;

/// Bumped whenever the service changes how a poster is drawn from the same
/// inputs, so a picture from an older renderer is never offered as current.
pub const POSTER_RENDERER_VERSION: u32 = 1;

/// Every input that changes a poster's pixels.
///
/// Built from the track and the run on every lane frame, which is why it holds
/// values rather than references: a `Debug`-derived struct hashed field by
/// field means a field added here is in the key without anyone remembering to
/// add it to a hash function — the shape of bug that leaves a stale picture
/// claiming to be current.
#[derive(Clone, Debug)]
pub struct PosterRecipe {
    /// The audio's content digest, or its path when the digest was deferred.
    pub audio: String,
    pub scene: SceneId,
    pub seed: u64,
    /// The frame drawn, at [`Self::fps`].
    pub frame_index: u64,
    pub fps: u32,
    /// How many frames before [`Self::frame_index`] the replay starts.
    pub preroll_frames: u64,
    /// The tuning of the section the moment falls in, for the run's scene.
    pub settings: SettingsSnapshot,
    /// For a threshold: the doorway scene and the tuning it wears.
    pub nested: Option<(SceneId, SettingsSnapshot)>,
    /// The scene's routes (and the doorway scene's, for a threshold).
    pub routes: Vec<SceneRoutes>,
    pub grade: Option<Grade>,
    pub pixel: PixelFrame,
    pub finish: Finish,
    /// The caption style, when the scene's frame carries a caption at all.
    pub caption: Option<CaptionStyle>,
    /// Displayable lyric cues the replay can see, and the voice table they
    /// may be drawn in.
    pub lyrics: Vec<LyricCue>,
    pub voices: String,
    /// The two event lanes' revisions: events feed some scenes' frames.
    pub events: (u64, u64),
    pub ascii: Option<String>,
    /// Whether Song Atlas's whole-track terrain was built: without it the
    /// scene draws its live idle ring, a different picture.
    pub atlas: bool,
    /// The export's frame size, which sets the poster's aspect and the pixel
    /// scale fixed-size details are drawn at.
    pub export_size: (u32, u32),
    pub poster_size: (u32, u32),
    pub version: u32,
}

/// A stable 64-bit identity for a [`PosterRecipe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PosterKey(pub u64);

impl PosterRecipe {
    /// FNV-1a over the recipe's `Debug` text, streamed without allocating.
    ///
    /// Stable within a build, which is all an in-memory cache needs; nothing
    /// writes a key to disk.
    #[must_use]
    pub fn key(&self) -> PosterKey {
        let mut hasher = Fnv1a(0xcbf2_9ce4_8422_2325);
        // Writing into a hasher cannot fail.
        let _ = write!(hasher, "{self:?}");
        PosterKey(hasher.0)
    }
}

struct Fnv1a(u64);

impl std::fmt::Write for Fnv1a {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        for byte in text.bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        Ok(())
    }
}

/// The frame a run's poster shows, at `fps`.
///
/// 1. The run's midpoint;
/// 2. if a displayable lyric line overlaps the run, the middle of the one
///    nearest that midpoint instead (clipped into the run), so a run that
///    carries words shows them;
/// 3. pushed out of the containing section's arrival transition and glide,
///    out of a dip that begins before the next section, and out of the outro,
///    so the poster is the section's own picture rather than a blend;
/// 4. snapped to a frame.
///
/// It never reads the playhead, so a poster does not change under playback.
#[must_use]
pub fn poster_moment(
    plan: &SceneSwitchTimeline,
    lyrics: &LyricsDocument,
    run: &RunSpan,
    fps: u32,
    duration_seconds: f64,
) -> u64 {
    let fps = fps.max(1);
    let frame = 1.0 / f64::from(fps);
    let middle = (run.start_seconds + run.end_seconds) * 0.5;
    let mut moment = middle;
    let mut nearest = f64::INFINITY;
    for cue in lyrics.cues() {
        if !cue.origin.is_displayable()
            || cue.end_seconds <= run.start_seconds
            || cue.start_seconds >= run.end_seconds
        {
            continue;
        }
        let start = cue.start_seconds.max(run.start_seconds);
        let end = cue.end_seconds.min(run.end_seconds);
        let centre = (start + end) * 0.5;
        let distance = (centre - middle).abs();
        if distance < nearest {
            nearest = distance;
            moment = centre;
        }
    }

    let cues = plan.cues();
    let last = (run.first_section + run.sections).min(cues.len());
    let section = (run.first_section..last).find(|&index| {
        let cue = &cues[index];
        cue.start_seconds <= moment && moment < cue.end_seconds
    });
    if let Some(index) = section {
        let cue = &cues[index];
        let room = cue.end_seconds - cue.start_seconds;
        let arrival = if cue.transition.kind == TransitionKind::Dip {
            fitted_seconds(cue.transition, room * 2.0) * 0.5
        } else {
            fitted_seconds(cue.transition, room)
        };
        let glide = f64::from(cue.glide).min(room);
        let low = cue.start_seconds + arrival.max(glide);
        let mut high = cue.end_seconds;
        if let Some(next) = cues.get(index + 1) {
            if next.transition.kind == TransitionKind::Dip {
                let next_room = next.end_seconds - next.start_seconds;
                let half = fitted_seconds(next.transition, next_room * 2.0) * 0.5;
                high = high.min(next.start_seconds - half.min(room));
            }
        }
        if !plan.outro.is_cut() && duration_seconds > 0.0 {
            let length = f64::from(plan.outro.seconds).min(duration_seconds);
            high = high.min(duration_seconds - length);
        }
        // Half a frame inside on both sides, so the snap below cannot land
        // back on the excluded edge.
        let (low, high) = (low + frame * 0.5, high - frame * 1.5);
        if low <= high {
            moment = moment.clamp(low, high);
        }
    }

    let last_frame = ((duration_seconds * f64::from(fps)).ceil() as u64).saturating_sub(1);
    ((moment.max(0.0) * f64::from(fps)).floor() as u64).min(last_frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lyrics::CueOrigin;
    use crate::project::scene_switch::SceneSwitchCue;
    use crate::project::transition::Transition;
    use crate::ui::plan_runs::fold_runs;

    fn plan(sections: &[(f64, f64, SceneId, Transition, f32)]) -> SceneSwitchTimeline {
        let cues: Vec<SceneSwitchCue> = sections
            .iter()
            .enumerate()
            .map(
                |(index, &(start, end, scene, transition, glide))| SceneSwitchCue {
                    id: index as u64 + 1,
                    start_seconds: start,
                    end_seconds: end,
                    scene_index: scene.index() as u32,
                    strength: 1.0,
                    transition,
                    glide,
                    ..SceneSwitchCue::default()
                },
            )
            .collect();
        let duration = sections.last().map_or(0.0, |section| section.1);
        let mut timeline = SceneSwitchTimeline::new();
        timeline.replace(&cues, duration, 64).expect("valid plan");
        timeline.enabled = true;
        timeline
    }

    fn lyrics(duration: f64, lines: &[(f64, f64, CueOrigin)]) -> LyricsDocument {
        let mut document = LyricsDocument::new(duration).expect("document");
        for &(start, end, origin) in lines {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: end,
                    text: "line".to_owned(),
                    origin,
                })
                .expect("cue");
        }
        document
    }

    #[test]
    fn a_plain_run_is_shown_at_its_midpoint() {
        let timeline = plan(&[
            (0.0, 10.0, SceneId::Spectrum, Transition::CUT, 0.0),
            (10.0, 30.0, SceneId::Cadence, Transition::CUT, 0.0),
        ]);
        let runs = fold_runs(timeline.cues());
        let words = lyrics(30.0, &[]);
        assert_eq!(poster_moment(&timeline, &words, &runs[0], 30, 30.0), 150);
        assert_eq!(poster_moment(&timeline, &words, &runs[1], 30, 30.0), 600);
    }

    #[test]
    fn a_run_with_words_is_shown_inside_the_nearest_line() {
        let timeline = plan(&[(0.0, 40.0, SceneId::Stage, Transition::CUT, 0.0)]);
        let runs = fold_runs(timeline.cues());
        // The midpoint is 20 s; the line at 24..28 is nearer than 2..4.
        let words = lyrics(
            40.0,
            &[
                (2.0, 4.0, CueOrigin::UserApplied),
                (24.0, 28.0, CueOrigin::UserApplied),
            ],
        );
        assert_eq!(
            poster_moment(&timeline, &words, &runs[0], 30, 40.0),
            26 * 30
        );
        // A proposal nobody accepted is not a line on screen.
        let proposals = lyrics(40.0, &[(19.0, 21.5, CueOrigin::Potential)]);
        assert_eq!(
            poster_moment(&timeline, &proposals, &runs[0], 30, 40.0),
            600
        );
    }

    #[test]
    fn the_moment_leaves_a_transition_and_a_glide() {
        // Section 2 arrives by a 4 s dissolve over 10..14 and glides for 6 s:
        // a moment at 12 has to move to 16 or later.
        let timeline = plan(&[
            (0.0, 10.0, SceneId::Spectrum, Transition::CUT, 0.0),
            (
                10.0,
                14.0 + 0.0,
                SceneId::Clawd,
                Transition::new(TransitionKind::Burn, 4.0),
                0.0,
            ),
        ]);
        let runs = fold_runs(timeline.cues());
        let words = lyrics(14.0, &[]);
        let frame = poster_moment(&timeline, &words, &runs[1], 30, 14.0);
        // The whole section is the transition: nothing to leave to, so the
        // moment stays rather than leaving the run.
        assert_eq!(frame, 360);

        let timeline = plan(&[
            (0.0, 10.0, SceneId::Clawd, Transition::CUT, 0.0),
            (
                10.0,
                20.0,
                SceneId::Clawd,
                Transition::new(TransitionKind::Burn, 4.0),
                6.0,
            ),
        ]);
        let runs = fold_runs(timeline.cues());
        assert_eq!(runs.len(), 1);
        let words = lyrics(20.0, &[(11.0, 13.0, CueOrigin::UserApplied)]);
        let frame = poster_moment(&timeline, &words, &runs[0], 30, 20.0);
        let seconds = frame as f64 / 30.0;
        assert!((16.0..20.0).contains(&seconds), "{seconds}");
        assert!(timeline.transition_at(seconds, 20.0).is_none());
        assert!(timeline
            .glided_settings(&crate::scene::settings::SceneSettings::default(), seconds)
            .is_none());
    }

    #[test]
    fn the_moment_leaves_an_outro_and_a_dip_into_the_next_section() {
        let mut timeline = plan(&[
            (0.0, 10.0, SceneId::Spectrum, Transition::CUT, 0.0),
            (
                10.0,
                20.0,
                SceneId::Stage,
                Transition::new(TransitionKind::Dip, 12.0),
                0.0,
            ),
        ]);
        // The dip's first half reaches back 6 s into the Spectrum run, its
        // second covers 10..16, and the outro takes 17..20: both runs' plain
        // midpoints (5 and 15) are inside one.
        timeline.outro = Transition::new(TransitionKind::Fade, 3.0);
        let runs = fold_runs(timeline.cues());
        let words = lyrics(20.0, &[]);
        for run in &runs {
            let frame = poster_moment(&timeline, &words, run, 30, 20.0);
            let seconds = frame as f64 / 30.0;
            assert!(run.contains(seconds), "{seconds}");
            assert!(
                timeline.transition_at(seconds, 20.0).is_none(),
                "{seconds} is inside a transition"
            );
        }
    }

    #[test]
    fn the_key_moves_with_every_input_and_only_with_them() {
        let recipe = PosterRecipe {
            audio: "abc".to_owned(),
            scene: SceneId::Clawd,
            seed: 7,
            frame_index: 300,
            fps: 30,
            preroll_frames: 60,
            settings: SettingsSnapshot::default(),
            nested: None,
            routes: Vec::new(),
            grade: None,
            pixel: PixelFrame::steady(crate::project::pixel::Pixel::OFF),
            finish: Finish::default(),
            caption: None,
            lyrics: Vec::new(),
            voices: String::new(),
            events: (0, 0),
            ascii: None,
            atlas: false,
            export_size: (1920, 1080),
            poster_size: (256, 144),
            version: POSTER_RENDERER_VERSION,
        };
        assert_eq!(recipe.key(), recipe.clone().key());
        let moved = [
            PosterRecipe {
                frame_index: 301,
                ..recipe.clone()
            },
            PosterRecipe {
                seed: 8,
                ..recipe.clone()
            },
            PosterRecipe {
                audio: "abd".to_owned(),
                ..recipe.clone()
            },
            PosterRecipe {
                events: (0, 1),
                ..recipe.clone()
            },
            PosterRecipe {
                poster_size: (128, 72),
                ..recipe.clone()
            },
        ];
        for other in &moved {
            assert_ne!(recipe.key(), other.key(), "{other:?}");
        }
    }
}
