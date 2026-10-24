//! The scene plan folded into **runs** (WX-5b).
//!
//! A run is a maximal sequence of consecutive plan sections of the same scene.
//! It breaks on a scene change **and on nothing else**: a section's grade,
//! transition, glide or pixel look may change inside a run, and the lane draws
//! those as marks within it (a hairline per internal cut, a light edge where a
//! transition arrives, the grade bar segmented per section). That is what makes
//! the Door, No Room plan read as 21 shapes instead of a 70-section barcode
//! (`docs/WORKSTATION.md`, WX-0's mockup).
//!
//! Pure and raylib-free so the policy is pinned by unit tests rather than by a
//! picture. The lane folds for **drawing only**: hit testing stays per section
//! (`scene_lane_edit::hit_test`), so no editing gesture depends on this.

use crate::project::scene_switch::SceneSwitchCue;
use crate::scene::SceneId;

/// One run of the plan: consecutive sections of one scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunSpan {
    pub scene: SceneId,
    /// The first section's start.
    pub start_seconds: f64,
    /// The last section's end.
    pub end_seconds: f64,
    /// Index of the run's first section in the plan's cue list.
    pub first_section: usize,
    /// How many sections the run holds; at least one.
    pub sections: usize,
    /// The first section's cue id: the run's identity. Stable across tuning,
    /// grade and transition edits; new when a split or a merge moves the run's
    /// first section, which is exactly when the run is a different thing.
    pub first_cue_id: u64,
}

impl RunSpan {
    /// Internal cuts: section boundaries inside the run.
    #[must_use]
    pub fn internal_cuts(&self) -> usize {
        self.sections.saturating_sub(1)
    }

    /// Whether `seconds` falls inside the run, end-exclusive like a section.
    #[must_use]
    pub fn contains(&self, seconds: f64) -> bool {
        self.start_seconds <= seconds && seconds < self.end_seconds
    }
}

/// Folds the plan's sections into runs, in plan order.
///
/// A section whose scene index does not resolve (the publication bounds it, so
/// this is a corrupt-input path) belongs to no run and ends the one before it,
/// rather than being absorbed into a neighbour it may not match.
#[must_use]
pub fn fold_runs(cues: &[SceneSwitchCue]) -> Vec<RunSpan> {
    let mut runs: Vec<RunSpan> = Vec::new();
    let mut open = false;
    for (index, cue) in cues.iter().enumerate() {
        let Some(scene) = SceneId::from_index(cue.scene_index as usize) else {
            open = false;
            continue;
        };
        match runs.last_mut() {
            Some(run) if open && run.scene == scene => {
                run.sections += 1;
                run.end_seconds = cue.end_seconds;
            }
            _ => {
                runs.push(RunSpan {
                    scene,
                    start_seconds: cue.start_seconds,
                    end_seconds: cue.end_seconds,
                    first_section: index,
                    sections: 1,
                    first_cue_id: cue.id,
                });
                open = true;
            }
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::look::{Grade, Palette};
    use crate::project::transition::{Transition, TransitionKind};

    fn cue(id: u64, start: f64, end: f64, scene: SceneId) -> SceneSwitchCue {
        SceneSwitchCue {
            id,
            start_seconds: start,
            end_seconds: end,
            scene_index: SceneId::ALL.iter().position(|s| *s == scene).unwrap() as u32,
            ..SceneSwitchCue::default()
        }
    }

    /// The fixture the gate seeds (`--run-plan`): C, C (graded, arriving by a
    /// burn), S, S, S (gliding), C, K.
    fn plan() -> Vec<SceneSwitchCue> {
        let mut cues = vec![
            cue(1, 0.0, 1.0, SceneId::Constellation),
            cue(2, 1.0, 2.0, SceneId::Constellation),
            cue(3, 2.0, 3.0, SceneId::Stage),
            cue(4, 3.0, 4.0, SceneId::Stage),
            cue(5, 4.0, 5.0, SceneId::Stage),
            cue(6, 5.0, 6.0, SceneId::Constellation),
            cue(7, 6.0, 8.0, SceneId::Cadence),
        ];
        cues[1].grade = Some(Grade::from_palette(Palette::NightDoor, 1.0));
        cues[1].transition = Transition {
            kind: TransitionKind::Burn,
            seconds: 0.5,
        };
        cues[4].glide = 1.0;
        cues
    }

    #[test]
    fn a_run_breaks_on_a_scene_change_and_on_nothing_else() {
        let runs = fold_runs(&plan());
        let shape: Vec<(SceneId, usize, usize, u64)> = runs
            .iter()
            .map(|run| (run.scene, run.first_section, run.sections, run.first_cue_id))
            .collect();
        assert_eq!(
            shape,
            [
                (SceneId::Constellation, 0, 2, 1),
                (SceneId::Stage, 2, 3, 3),
                (SceneId::Constellation, 5, 1, 6),
                (SceneId::Cadence, 6, 1, 7),
            ]
        );
        assert_eq!(runs.iter().map(RunSpan::internal_cuts).sum::<usize>(), 3);
        assert_eq!((runs[1].start_seconds, runs[1].end_seconds), (2.0, 5.0));
        assert!(runs[1].contains(2.0) && runs[1].contains(4.999) && !runs[1].contains(5.0));
    }

    /// The negative control for the rule above: the same plan with every
    /// section a different scene from its neighbour folds to one run per
    /// section, so a fold that ignored the scene could not pass both.
    #[test]
    fn alternating_scenes_make_one_run_per_section() {
        let mut cues = plan();
        for (index, cue) in cues.iter_mut().enumerate() {
            cue.scene_index = (index % 2) as u32;
        }
        assert_eq!(fold_runs(&cues).len(), cues.len());
    }

    #[test]
    fn an_unresolvable_scene_belongs_to_no_run_and_ends_the_one_before() {
        let mut cues = plan();
        cues[3].scene_index = u32::MAX;
        let runs = fold_runs(&cues);
        // Stage, Stage-bad-Stage: the bad section splits the Stage run in two.
        assert_eq!(runs.len(), 5);
        assert_eq!((runs[1].sections, runs[2].first_section), (1, 4));
        assert!(fold_runs(&[]).is_empty());
    }
}
